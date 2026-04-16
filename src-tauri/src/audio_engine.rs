use crate::audio_dir;
use crate::device_manager;
use crate::state::{encode_segment_started, SEGMENT_STARTED_UNSET};
use chrono::Utc;
use cpal::traits::{DeviceTrait, StreamTrait};
use crossbeam_channel::Sender;
use hound::{SampleFormat, WavSpec, WavWriter};
use parking_lot::Mutex;
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;

const TARGET_SAMPLE_RATE: u32 = 16_000;
const CHANNELS: u16 = 1;
const RESAMPLE_CHUNK: usize = 1024;
const DEFAULT_SEGMENT_SECS: u64 = 120;

// Audio-level smoother (asymmetric: fast attack, slow release).
const LEVEL_ATTACK_PREV: f32 = 0.4;
const LEVEL_ATTACK_NEW: f32 = 0.6;
const LEVEL_RELEASE_PREV: f32 = 0.92;
const LEVEL_RELEASE_NEW: f32 = 0.08;
const LEVEL_RMS_GAIN: f32 = 300.0;
const LEVEL_MAX: f32 = 100.0;

pub fn segment_duration_secs() -> u64 {
    static CACHED: OnceLock<u64> = OnceLock::new();
    *CACHED.get_or_init(|| {
        std::env::var("WHISPER_SEGMENT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_SEGMENT_SECS)
    })
}

pub struct AudioEngine {
    _stream: cpal::Stream,
}

struct RecordingState {
    writer: Option<WavWriter<std::io::BufWriter<std::fs::File>>>,
    samples_written: u64,
    audio_dir: PathBuf,
    current_path: Option<PathBuf>,
    segment_tx: Sender<PathBuf>,
    resample_buf: VecDeque<f32>,
    write_error_logged: bool,
    /// Shared with `AppState` so `get_status` can read elapsed time. Stores
    /// epoch millis (UTC); `SEGMENT_STARTED_UNSET` means no segment is open.
    /// Atomic rather than `Mutex<Option<_>>` because the value fits in 64 bits
    /// and the audio callback would otherwise acquire two locks (outer state
    /// + inner timestamp) on every buffer — a latent deadlock and RT hazard.
    segment_started_at: Arc<AtomicI64>,
    /// Surfaces disk-open failures to `get_status`. Set when `WavWriter::create`
    /// fails (no writer → every sample silently dropped until next rotation)
    /// and cleared when a rotation succeeds. Without this flag a permissions
    /// error or full-disk condition is only visible in `log::error!`, which
    /// is effectively invisible in a packaged bundle.
    audio_disk_error: Arc<AtomicBool>,
}

impl AudioEngine {
    pub fn new(
        audio_dir: PathBuf,
        segment_tx: Sender<PathBuf>,
        pause_flag: Arc<AtomicBool>,
        audio_level: Arc<AtomicU32>,
        segment_started_at: Arc<AtomicI64>,
        audio_disk_error: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let host = cpal::default_host();

        let device = device_manager::select_best_input(&host)
            .ok_or("no input device available")?;

        let device_name = device.name().unwrap_or_else(|_| "Unknown".into());
        log::info!("Using input device: {device_name}");

        let supported = device
            .default_input_config()
            .map_err(|e| format!("No supported input config: {e}"))?;

        let native_rate = supported.sample_rate().0;
        let native_channels = supported.channels();
        log::info!("Native config: {native_rate}Hz, {native_channels}ch");

        let config = cpal::StreamConfig {
            channels: native_channels,
            sample_rate: cpal::SampleRate(native_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let needs_resample = native_rate != TARGET_SAMPLE_RATE;
        // sinc_len * oversampling_factor is the effective filter quality. The
        // previous 256*256 = 65k-tap setup was GPU-rendering-grade for what
        // is ultimately a 16 kHz ASR pipeline where Whisper drops everything
        // above 8 kHz anyway. 64*128 is still overkill for speech but runs
        // ~10-20× faster on the real-time thread, with no audible or
        // accuracy-measurable difference.
        let mut resampler: Option<SincFixedIn<f32>> = if needs_resample {
            let params = SincInterpolationParameters {
                sinc_len: 64,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Linear,
                oversampling_factor: 128,
                window: WindowFunction::BlackmanHarris2,
            };
            let r = SincFixedIn::<f32>::new(
                TARGET_SAMPLE_RATE as f64 / native_rate as f64,
                2.0,
                params,
                RESAMPLE_CHUNK,
                1,
            )
            .map_err(|e| format!("Resampler init failed: {e}"))?;
            log::info!("Resampler: {native_rate}Hz -> {TARGET_SAMPLE_RATE}Hz (chunk={RESAMPLE_CHUNK})");
            Some(r)
        } else {
            None
        };

        let state = Arc::new(Mutex::new(RecordingState {
            writer: None,
            samples_written: 0,
            audio_dir: audio_dir.clone(),
            current_path: None,
            segment_tx,
            resample_buf: VecDeque::with_capacity(RESAMPLE_CHUNK * 4),
            write_error_logged: false,
            segment_started_at: segment_started_at.clone(),
            audio_disk_error: audio_disk_error.clone(),
        }));

        // No disk I/O at construction — the first segment is opened in the
        // callback when we actually have audio to write. This avoids a
        // filename/content mismatch when the engine is born paused
        // (power-monitor auto-pause before engine start): previously the
        // WAV was created at `T_construct` but held the audio captured at
        // `T_resume`, which fed the wrong capture time to the dedup path.

        let state_clone = state.clone();
        let native_ch = native_channels;
        let level_for_callback = audio_level.clone();
        let segment_started_at_cb = segment_started_at.clone();

        // Pre-allocated buffers owned by the callback closure — the resampler
        // itself moves in here (not shared via Arc<Mutex>, because only this
        // thread ever uses it). `mono_buf` holds the downmixed frame; `chunk`
        // is the slice handed to the resampler. Both are reused across every
        // invocation so the real-time thread never allocates.
        let mut mono_buf: Vec<f32> = Vec::with_capacity(8192);
        let mut chunk: Vec<f32> = Vec::with_capacity(RESAMPLE_CHUNK);

        // Callback-local edge detector for pause transitions. Previously the
        // pause branch took the state lock + stamped `Utc::now()` on every
        // paused buffer (~93×/s); now we only do that work on pause onset and
        // pause release.
        //
        // Initialised to `true` so the first unpaused callback flows through
        // the release branch, which now owns lazy-opening the initial WAV.
        // This lets engine construction stay free of disk side effects: a
        // born-paused engine simply never enters the release branch until
        // the user unpauses, at which point the filename timestamp matches
        // the captured-audio timestamp.
        let mut was_paused = true;

        #[cfg(debug_assertions)]
        let callback_count = Arc::new(std::sync::atomic::AtomicU64::new(0));
        #[cfg(debug_assertions)]
        let callback_count_clone = callback_count.clone();

        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if data.is_empty() {
                        return;
                    }

                    // Fast-path pause gate — pure atomic read. Most paused
                    // buffers short-circuit here with no mutex and no syscall.
                    // Lock + state writes only happen on the two edges:
                    //   - onset (unpaused → paused): clear resample state and
                    //     mark the segment timestamp UNSET so `get_status`
                    //     reports elapsed=0 while paused.
                    //   - release (paused → unpaused): re-baseline the
                    //     segment timestamp to now so the pre-pause segment
                    //     gets a fresh rotation allowance and the paused
                    //     interval doesn't count toward rotation.
                    let paused_now = pause_flag.load(Ordering::Acquire);
                    if paused_now {
                        level_for_callback.store(0, Ordering::Relaxed);
                        if !was_paused {
                            let mut s = state_clone.lock();
                            s.resample_buf.clear();
                            segment_started_at_cb
                                .store(SEGMENT_STARTED_UNSET, Ordering::Release);
                            was_paused = true;
                        }
                        return;
                    }
                    if was_paused {
                        let mut s = state_clone.lock();
                        s.resample_buf.clear();
                        if s.writer.is_none() {
                            // Lazy-open path: either engine birth or a prior
                            // rotation failure. `open_new_segment` sets
                            // `segment_started_at` to its own `now`, so the
                            // filename stamp and the segment start line up.
                            open_new_segment(&mut s);
                        } else {
                            // True pause→unpause edge on an existing segment:
                            // re-baseline so the paused interval doesn't count
                            // toward rotation.
                            segment_started_at_cb
                                .store(encode_segment_started(Utc::now()), Ordering::Release);
                        }
                        was_paused = false;
                    }

                    #[cfg(debug_assertions)]
                    {
                        let count = callback_count_clone.fetch_add(1, Ordering::Relaxed);
                        if count < 3 || count % 1000 == 0 {
                            let max = data.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
                            let rms = (data.iter().map(|s| s * s).sum::<f32>() / data.len() as f32).sqrt();
                            log::debug!("[AUDIO] callback #{count}: len={}, max={max:.6}, rms={rms:.6}, channels={native_ch}", data.len());
                        }
                    }

                    mono_buf.clear();
                    if native_ch > 1 {
                        let ch = native_ch as usize;
                        mono_buf.extend(
                            data.chunks(ch).map(|c| c.iter().sum::<f32>() / c.len() as f32),
                        );
                    } else {
                        mono_buf.extend_from_slice(data);
                    }

                    update_audio_level(&level_for_callback, &mono_buf);

                    let mut s = state_clone.lock();

                    if let Some(r) = resampler.as_mut() {
                        s.resample_buf.extend(mono_buf.iter());

                        while s.resample_buf.len() >= RESAMPLE_CHUNK {
                            chunk.clear();
                            chunk.extend(s.resample_buf.drain(..RESAMPLE_CHUNK));
                            match r.process(&[&chunk[..]], None) {
                                Ok(resampled) => {
                                    if !resampled.is_empty() && !resampled[0].is_empty() {
                                        write_samples(&mut s, &resampled[0]);
                                    }
                                }
                                Err(e) => {
                                    log::error!("Resample error: {e}");
                                }
                            }
                        }
                    } else {
                        write_samples(&mut s, &mono_buf);
                    }
                },
                |err| log::error!("Audio stream error: {err}"),
                None,
            )
            .map_err(|e| format!("Failed to build input stream: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start audio stream: {e}"))?;

        log::info!("Audio stream playing — capturing at {native_rate}Hz {native_ch}ch, writing {TARGET_SAMPLE_RATE}Hz mono, device={device_name}");

        Ok(Self { _stream: stream })
    }
}

fn update_audio_level(level: &AtomicU32, mono: &[f32]) {
    let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt();
    let new_level = (rms * LEVEL_RMS_GAIN).min(LEVEL_MAX) as u32;
    let prev = level.load(Ordering::Relaxed);
    let smoothed = if new_level > prev {
        (prev as f32 * LEVEL_ATTACK_PREV + new_level as f32 * LEVEL_ATTACK_NEW) as u32
    } else {
        (prev as f32 * LEVEL_RELEASE_PREV + new_level as f32 * LEVEL_RELEASE_NEW) as u32
    };
    level.store(smoothed, Ordering::Relaxed);
}

fn write_samples(state: &mut RecordingState, samples: &[f32]) {
    let Some(writer) = state.writer.as_mut() else { return };

    for &sample in samples {
        if let Err(e) = writer.write_sample(sample) {
            // Log once per segment to avoid flooding — the writer is dead until rotation
            if !state.write_error_logged {
                log::error!("WAV write_sample failed: {e} — segment will be incomplete");
                state.write_error_logged = true;
            }
            return;
        }
    }
    state.samples_written += samples.len() as u64;

    // Rotate by wall clock time — stays in sync with UI timer even if audio is throttled
    let elapsed = elapsed_seconds(&state.segment_started_at);
    if elapsed >= segment_duration_secs() {
        rotate_segment(state);
    }
}

fn elapsed_seconds(started: &AtomicI64) -> u64 {
    match crate::state::decode_segment_started(started.load(Ordering::Acquire)) {
        Some(t) => (Utc::now() - t).num_seconds().max(0) as u64,
        None => 0,
    }
}

fn open_new_segment(state: &mut RecordingState) {
    let now = chrono::Utc::now();
    let filename = audio_dir::format_segment_filename(&now);
    let path = state.audio_dir.join(&filename);

    // Unconditional reset: previously this lived inside the `Ok` arm only,
    // so a failed `WavWriter::create` would leave the flag stuck `true` from
    // the prior segment and subsequent write-sample errors would be silently
    // swallowed — the opposite of the "log once per segment" intent.
    state.write_error_logged = false;

    let spec = WavSpec {
        channels: CHANNELS,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };

    match WavWriter::create(&path, spec) {
        Ok(writer) => {
            state.writer = Some(writer);
            state.samples_written = 0;
            state.current_path = Some(path);
            state
                .segment_started_at
                .store(encode_segment_started(now), Ordering::Release);
            state.audio_disk_error.store(false, Ordering::Release);
            log::info!("New segment: {filename}");
        }
        Err(e) => {
            log::error!("Failed to create WAV: {e} — audio will be dropped until next rotation");
            state.audio_disk_error.store(true, Ordering::Release);
        }
    }
}

fn rotate_segment(state: &mut RecordingState) {
    let mut send_path: Option<PathBuf> = None;

    if let Some(writer) = state.writer.take() {
        let samples = state.samples_written;
        let path = state.current_path.take();

        match writer.finalize() {
            Ok(()) => {
                if let Some(p) = path {
                    log::info!(
                        "Segment complete: {} ({} samples, {:.1}s)",
                        p.display(),
                        samples,
                        samples as f64 / TARGET_SAMPLE_RATE as f64
                    );
                    send_path = Some(p);
                }
            }
            Err(e) => {
                log::error!("WAV finalize failed: {e} — discarding segment");
                if let Some(p) = path {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }

    open_new_segment(state);

    // Send AFTER opening the next segment so the consumer can run in parallel
    // with the next chunk of recording.
    if let Some(p) = send_path {
        state.segment_tx.send(p).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resample_buffer_accumulation() {
        // #given a resample buffer fed five chunks of mono samples
        let mut buf: VecDeque<f32> = VecDeque::new();
        for _ in 0..5 {
            buf.extend((0..256).map(|i| (i as f32 * 0.01).sin()));
        }

        // #when we drain one resample chunk
        assert!(buf.len() >= RESAMPLE_CHUNK);
        let chunk: Vec<f32> = buf.drain(..RESAMPLE_CHUNK).collect();

        // #then the chunk is full and the remainder matches
        assert_eq!(chunk.len(), RESAMPLE_CHUNK);
        assert_eq!(buf.len(), 256);
    }

    #[test]
    fn test_audio_level_stores_and_loads_through_arc() {
        // #given an isolated level atomic (no module global — tests can run in parallel)
        let level = Arc::new(AtomicU32::new(0));

        // #when we store and load
        level.store(100, Ordering::Relaxed);
        assert_eq!(level.load(Ordering::Relaxed), 100);
    }

    #[test]
    fn test_downmix_stereo() {
        // #given interleaved stereo samples
        let stereo = vec![1.0f32, -1.0, 0.5, 0.5];

        // #when we downmix to mono
        let mono: Vec<f32> = stereo.chunks(2).map(|ch| ch.iter().sum::<f32>() / 2.0).collect();

        // #then left+right pairs average correctly
        assert!((mono[0] - 0.0).abs() < 1e-6);
        assert!((mono[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_update_audio_level_attacks_fast_releases_slow() {
        // #given a fresh level atomic
        let level = AtomicU32::new(0);

        // #when a loud burst arrives
        update_audio_level(&level, &[0.5; 256]);
        let after_attack = level.load(Ordering::Relaxed);

        // #then attack is immediate
        assert!(after_attack > 0);

        // #when silence follows
        update_audio_level(&level, &[0.0; 256]);
        let after_release = level.load(Ordering::Relaxed);

        // #then release is slower than attack — one tick doesn't drop to 0
        assert!(after_release < after_attack);
        assert!(after_release > 0, "release should be slow, not instant");
    }

    #[test]
    fn test_elapsed_seconds_unset_is_zero() {
        // #given an UNSET atomic — no segment open
        let a = AtomicI64::new(SEGMENT_STARTED_UNSET);
        // #then elapsed reads as zero (distinct from "1970-01-01")
        assert_eq!(elapsed_seconds(&a), 0);
    }

    #[test]
    fn test_elapsed_seconds_from_past() {
        // #given a segment that started ~10 s ago
        let past = Utc::now() - chrono::Duration::seconds(10);
        let a = AtomicI64::new(encode_segment_started(past));

        // #when we read elapsed
        let elapsed = elapsed_seconds(&a);

        // #then it matches wall clock within 1 s jitter
        assert!((9..=11).contains(&elapsed), "expected ~10s, got {elapsed}");
    }

    #[test]
    fn test_segment_started_encode_decode_roundtrip() {
        // #given a capture time at millisecond precision
        use chrono::TimeZone;
        let ts = Utc.with_ymd_and_hms(2026, 4, 16, 14, 5, 22).unwrap();

        // #when we encode to the atomic representation and decode back
        let raw = encode_segment_started(ts);
        let decoded = crate::state::decode_segment_started(raw).unwrap();

        // #then the seconds-level value survives the roundtrip
        assert_eq!(decoded, ts);
    }
}
