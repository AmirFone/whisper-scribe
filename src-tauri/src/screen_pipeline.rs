use crate::events;
use crate::screen_analyzer::ScreenAnalyzer;
use crate::screen_capture;
use crate::state::AppState;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const CAPTURE_INTERVAL_SECS: u64 = 300; // 5 minutes
const SCREENSHOT_MAX_AGE_SECS: u64 = 3600; // 1 hour

pub fn start(app: AppHandle, state: Arc<AppState>, screenshots_dir: PathBuf) {
    log::info!("Initializing screen analyzer...");
    let analyzer = match ScreenAnalyzer::new() {
        Ok(a) => {
            log::info!("Screen analyzer ready");
            a
        }
        Err(e) => {
            log::error!("Failed to init screen analyzer: {e}");
            return;
        }
    };

    if !screen_capture::has_screen_capture_permission() {
        log::warn!("Screen recording permission not granted at startup — requesting...");
        screen_capture::request_screen_capture_permission();
    }

    loop {
        std::thread::sleep(Duration::from_secs(CAPTURE_INTERVAL_SECS));

        if state.is_paused() || !state.screen_capture_enabled() {
            continue;
        }

        if !screen_capture::has_screen_capture_permission() {
            continue;
        }

        state.is_analyzing_screen.store(true, Ordering::Relaxed);

        screen_capture::cleanup_old_screenshots(
            &screenshots_dir,
            Duration::from_secs(SCREENSHOT_MAX_AGE_SECS),
        );

        match screen_capture::capture_all_screens(&screenshots_dir) {
            Ok(paths) => {
                match analyzer.analyze(&paths) {
                    Ok(text) if text.is_empty() => {
                        log::info!("Screen analysis returned empty text, skipping");
                    }
                    Ok(text) => {
                        let now = chrono::Utc::now();
                        if let Err(e) = state.storage.append_to_screen_slot(&text, &now) {
                            log::error!("Failed to store screen context: {e}");
                        }
                        app.emit(events::SCREEN_CONTEXT_UPDATED, &()).ok();
                    }
                    Err(e) => log::error!("Screen analysis failed: {e}"),
                }
            }
            Err(e) => log::error!("Screen capture failed: {e}"),
        }

        state.is_analyzing_screen.store(false, Ordering::Relaxed);
    }
}
