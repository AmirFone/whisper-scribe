use crate::audio_dir;
use crate::audio_engine;
use crate::events;
use crate::power_monitor;
use crate::state::AppState;
use crate::transcriber;
use crossbeam_channel::unbounded;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

/// Spawn the full transcription pipeline on the current thread:
///   1. Initialize the MLX daemon
///   2. Process orphan WAVs from previous runs
///   3. Start the audio engine + power monitor + cleanup timer
///   4. Loop: receive new segments from the audio engine and transcribe them
pub fn start(app: AppHandle, state: Arc<AppState>, audio_dir: PathBuf) {
    log::info!("Initializing transcriber...");
    match transcriber::Transcriber::new() {
        Ok(t) => {
            *state.transcriber.lock() = Some(t);
            log::info!("Transcriber ready");
        }
        Err(e) => {
            log::error!("Failed to init transcriber: {e}");
            return;
        }
    }

    process_orphans(&app, &state, &audio_dir);

    log::info!("Starting audio engine...");
    let (segment_tx, segment_rx) = unbounded::<PathBuf>();
    let _engine = match audio_engine::AudioEngine::new(
        audio_dir.clone(),
        segment_tx,
        state.pause_flag(),
        state.audio_level_arc(),
        state.segment_started_at_arc(),
        state.audio_disk_error_arc(),
    ) {
        Ok(engine) => {
            log::info!("Audio engine started");
            engine
        }
        Err(e) => {
            log::error!("Failed to start audio engine: {e}");
            return;
        }
    };

    log::info!("Starting power monitor...");
    let _power_monitor = power_monitor::PowerMonitor::new(state.clone());
    log::info!("Power monitor active");

    audio_dir::spawn_cleanup_timer(audio_dir.clone());

    while let Ok(path) = segment_rx.recv() {
        // A segment that was captured before pause can still arrive here; the
        // audio callback stops producing new ones while paused, but a mid-flight
        // one may already be in the channel. Skip transcription while paused
        // and leave the WAV on disk — cleanup will reap it, or the next run's
        // orphan scan will pick it up if pause is released after restart.
        if state.is_paused() {
            log::info!("Skipping segment (paused): {}", path.display());
            continue;
        }
        log::info!("Transcribing segment: {}", path.display());
        transcribe_and_store(&app, &state, &path);
    }
}

/// Transcribe a single WAV and append the result to its hour slot. The audio
/// engine sets `recording_started_at` itself — we don't touch it here.
fn transcribe_and_store(app: &AppHandle, state: &Arc<AppState>, path: &Path) {
    state.is_transcribing.store(true, Ordering::Relaxed);

    let result = {
        let guard = state.transcriber.lock();
        guard.as_ref().map(|t| t.transcribe(path))
    };

    match result {
        Some(Ok(r)) if r.text.is_empty() => {
            log::info!("Skipping empty/silent segment");
        }
        Some(Ok(r)) => {
            if let Err(e) = state.storage.insert_transcription(&r.text, &r.start_time, &r.device) {
                log::error!("Failed to store transcription: {e}");
            }
            app.emit(events::TIMELINE_UPDATED, &()).ok();
        }
        Some(Err(e)) => log::error!("Transcription failed: {e}"),
        None => log::warn!("Transcriber unavailable — segment dropped"),
    }

    state.is_transcribing.store(false, Ordering::Relaxed);
}

/// Process WAVs that the previous app run never transcribed (e.g. the app was
/// killed mid-segment). Skips the latest file (likely still being written by
/// another instance) and any file too small to contain real audio.
fn process_orphans(app: &AppHandle, state: &Arc<AppState>, dir: &Path) {
    let orphans = audio_dir::find_orphan_segments(dir);
    if orphans.is_empty() {
        return;
    }

    log::info!("Found {} orphan segments to process", orphans.len());

    for path in &orphans {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size < audio_dir::MIN_SEGMENT_BYTES {
            log::info!("Skipping tiny segment: {} ({} bytes)", path.display(), size);
            continue;
        }

        match orphan_status(state, path) {
            OrphanStatus::AlreadyProcessed => continue,
            OrphanStatus::NonCanonical => {
                log::warn!(
                    "Skipping non-canonical WAV (filename does not match segment grammar): {}",
                    path.display()
                );
                continue;
            }
            OrphanStatus::Unprocessed => {
                log::info!("Transcribing orphan segment: {}", path.display());
                transcribe_and_store(app, state, path);
            }
        }
    }

    audio_dir::cleanup_old_segments(dir, audio_dir::MAX_RETAINED_SEGMENTS);
}

enum OrphanStatus {
    Unprocessed,
    AlreadyProcessed,
    NonCanonical,
}

fn orphan_status(state: &Arc<AppState>, path: &Path) -> OrphanStatus {
    let Some(ts) = audio_dir::parse_segment_timestamp(path) else {
        return OrphanStatus::NonCanonical;
    };
    if state.storage.is_segment_processed(&ts) {
        OrphanStatus::AlreadyProcessed
    } else {
        OrphanStatus::Unprocessed
    }
}
