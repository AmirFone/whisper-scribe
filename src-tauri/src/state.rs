use crate::{storage, transcriber};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::Arc;

/// Sentinel value meaning "no segment is open" in the `segment_started_at`
/// atomic. Chosen because real capture times are bounded by the chrono
/// representable range; `i64::MIN` cannot collide with any real epoch-millis.
pub const SEGMENT_STARTED_UNSET: i64 = i64::MIN;

pub fn encode_segment_started(ts: DateTime<Utc>) -> i64 {
    ts.timestamp_millis()
}

pub fn decode_segment_started(raw: i64) -> Option<DateTime<Utc>> {
    if raw == SEGMENT_STARTED_UNSET {
        None
    } else {
        DateTime::from_timestamp_millis(raw)
    }
}

/// Why recording is currently paused (if at all). Distinguishing manual from
/// system-initiated pauses lets the power monitor auto-resume on screen unlock
/// without overriding an explicit user pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    None,
    Manual,
    System,
}

impl PauseReason {
    pub fn is_paused(self) -> bool {
        !matches!(self, Self::None)
    }
}

pub struct AppState {
    pub storage: storage::Storage,
    pub transcriber: Mutex<Option<transcriber::Transcriber>>,
    /// Source of truth for pause state. The `pause_flag` mirrors
    /// `pause_state.is_paused()` and is the hot-path read for the audio callback.
    pause_state: Mutex<PauseReason>,
    /// Mirror of `pause_state.is_paused()`. Shared with the cpal callback via
    /// `pause_flag()` so the real-time thread can check pause without a mutex.
    pause_flag: Arc<AtomicBool>,
    pub is_transcribing: AtomicBool,
    /// Smoothed audio level (0..=100). Written by the cpal callback, read by
    /// UI status polls and the audio-level channel. Owned here so that tests
    /// can construct engines with isolated levels — no more module globals.
    audio_level: Arc<AtomicU32>,
    /// Wall-clock start of the currently-open WAV segment, encoded as epoch
    /// millis (`SEGMENT_STARTED_UNSET` = no segment open). An atomic, not a
    /// mutex, because the value fits in 64 bits and the audio callback would
    /// otherwise acquire an inner lock on every paused buffer (~93×/s at
    /// 48 kHz / 512 frames) — a real-time hazard and a latent deadlock trap
    /// with the outer `RecordingState` lock.
    segment_started_at: Arc<AtomicI64>,
    /// Set by the audio engine when `WavWriter::create` fails and the writer
    /// is stuck at `None`. In that state the callback silently drops every
    /// sample until the next rotation attempt. `get_status` surfaces this so
    /// the UI can warn the user instead of the disk failure being invisible.
    audio_disk_error: Arc<AtomicBool>,
    pub is_analyzing_screen: AtomicBool,
    screen_capture_enabled: AtomicBool,
}

impl AppState {
    pub fn new(storage: storage::Storage) -> Self {
        Self {
            storage,
            transcriber: Mutex::new(None),
            pause_state: Mutex::new(PauseReason::None),
            pause_flag: Arc::new(AtomicBool::new(false)),
            is_transcribing: AtomicBool::new(false),
            audio_level: Arc::new(AtomicU32::new(0)),
            segment_started_at: Arc::new(AtomicI64::new(SEGMENT_STARTED_UNSET)),
            audio_disk_error: Arc::new(AtomicBool::new(false)),
            is_analyzing_screen: AtomicBool::new(false),
            screen_capture_enabled: AtomicBool::new(true),
        }
    }

    pub fn audio_level(&self) -> u32 {
        self.audio_level.load(Ordering::Relaxed)
    }

    pub fn audio_level_arc(&self) -> Arc<AtomicU32> {
        self.audio_level.clone()
    }

    pub fn segment_started_at(&self) -> Option<DateTime<Utc>> {
        decode_segment_started(self.segment_started_at.load(Ordering::Acquire))
    }

    pub fn segment_started_at_arc(&self) -> Arc<AtomicI64> {
        self.segment_started_at.clone()
    }

    pub fn audio_disk_error(&self) -> bool {
        self.audio_disk_error.load(Ordering::Acquire)
    }

    pub fn audio_disk_error_arc(&self) -> Arc<AtomicBool> {
        self.audio_disk_error.clone()
    }

    pub fn pause_reason(&self) -> PauseReason {
        *self.pause_state.lock()
    }

    /// Fast lock-free check for hot paths (audio callback, pipeline loop).
    pub fn is_paused(&self) -> bool {
        self.pause_flag.load(Ordering::Acquire)
    }

    /// Shared atomic for wiring into the cpal callback closure. The callback
    /// never takes the mutex — it only reads this flag.
    pub fn pause_flag(&self) -> Arc<AtomicBool> {
        self.pause_flag.clone()
    }

    /// Set the pause reason and update the mirror flag atomically-enough that
    /// any hot-path reader observing the flag sees a consistent state.
    pub fn set_pause(&self, reason: PauseReason) {
        let mut p = self.pause_state.lock();
        *p = reason;
        self.pause_flag.store(p.is_paused(), Ordering::Release);
    }

    pub fn screen_capture_enabled(&self) -> bool {
        self.screen_capture_enabled.load(Ordering::Acquire)
    }

    pub fn toggle_screen_capture(&self) -> bool {
        let prev = self.screen_capture_enabled.load(Ordering::Acquire);
        let new = !prev;
        self.screen_capture_enabled.store(new, Ordering::Release);
        new
    }

    /// Toggle between `None` and `Manual`. Any system-initiated pause gets
    /// cleared by an explicit toggle — user action wins.
    pub fn toggle_pause(&self) -> bool {
        let mut p = self.pause_state.lock();
        *p = match *p {
            PauseReason::None => PauseReason::Manual,
            _ => PauseReason::None,
        };
        let paused = p.is_paused();
        self.pause_flag.store(paused, Ordering::Release);
        paused
    }
}

// Compile-time assertion that `AppState` stays `Send + Sync`. Tauri requires
// both to manage the state across its async command executor + audio thread.
// If a future field (e.g. an `Rc<_>` or a non-Sync type) breaks the bound,
// this trips at compile time with a clear message, instead of surfacing as
// an obscure `tauri::Manager::manage` error at app start.
const _ASSERT_APP_STATE_SEND_SYNC: fn() = || {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<AppState>();
    assert_sync::<AppState>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_state() -> (AppState, TempDir) {
        let dir = TempDir::new().unwrap();
        let storage = storage::Storage::new(&dir.path().join("t.db")).unwrap();
        (AppState::new(storage), dir)
    }

    #[test]
    fn test_pause_flag_mirrors_state() {
        // #given a fresh state
        let (state, _dir) = make_state();
        assert!(!state.is_paused());

        // #when we toggle on
        let paused = state.toggle_pause();

        // #then both the flag and the reason agree
        assert!(paused);
        assert!(state.is_paused());
        assert_eq!(state.pause_reason(), PauseReason::Manual);

        // #when we toggle off
        let paused = state.toggle_pause();
        assert!(!paused);
        assert!(!state.is_paused());
        assert_eq!(state.pause_reason(), PauseReason::None);
    }

    #[test]
    fn test_set_pause_updates_flag() {
        // #given a fresh state
        let (state, _dir) = make_state();

        // #when system sets a system-pause
        state.set_pause(PauseReason::System);

        // #then the flag reflects paused
        assert!(state.is_paused());
        assert_eq!(state.pause_reason(), PauseReason::System);

        // #when we clear to None
        state.set_pause(PauseReason::None);
        assert!(!state.is_paused());
    }

    #[test]
    fn test_screen_capture_enabled_default() {
        // #given a fresh state
        let (state, _dir) = make_state();

        // #then screen capture is enabled by default
        assert!(state.screen_capture_enabled());
    }

    #[test]
    fn test_toggle_screen_capture() {
        // #given a fresh state with screen capture enabled
        let (state, _dir) = make_state();
        assert!(state.screen_capture_enabled());

        // #when we toggle off
        let enabled = state.toggle_screen_capture();

        // #then it returns false and reads false
        assert!(!enabled);
        assert!(!state.screen_capture_enabled());

        // #when we toggle back on
        let enabled = state.toggle_screen_capture();
        assert!(enabled);
        assert!(state.screen_capture_enabled());
    }

    #[test]
    fn test_is_analyzing_screen_default() {
        // #given a fresh state
        let (state, _dir) = make_state();

        // #then is_analyzing_screen starts false
        assert!(!state.is_analyzing_screen.load(Ordering::Relaxed));
    }

    #[test]
    fn test_is_analyzing_screen_can_be_set() {
        // #given a fresh state
        let (state, _dir) = make_state();

        // #when we set it to true
        state.is_analyzing_screen.store(true, Ordering::Relaxed);

        // #then it reads true
        assert!(state.is_analyzing_screen.load(Ordering::Relaxed));
    }
}
