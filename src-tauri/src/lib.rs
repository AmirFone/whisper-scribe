mod audio_dir;
mod audio_engine;
mod commands;
mod device_manager;
mod events;
mod pipeline;
mod power_monitor;
mod screen_analyzer;
mod screen_capture;
mod screen_pipeline;
mod shortcuts;
// `state` and `storage` are public so integration tests in `tests/` can pin
// their contract against the real API (see `tests/storage_integration.rs`).
// Every other module stays crate-private.
pub mod state;
pub mod storage;
mod transcriber;
mod tray;

// Thin re-exports for integration tests. The screen_capture module stays
// crate-private; only the testable subset is exposed here.
pub fn screen_capture_cleanup(dir: &std::path::Path, max_age: std::time::Duration) {
    screen_capture::cleanup_old_screenshots(dir, max_age);
}
pub fn screen_capture_has_permission() -> bool {
    screen_capture::has_screen_capture_permission()
}

use state::AppState;
use std::sync::Arc;
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Default to Info in packaged builds — `env_logger::init()` alone uses the
    // `error`-only default, which silently dropped every `log::info!` and
    // `log::warn!` including the Python stderr forwarder, daemon restarts,
    // and VAD/hallucination-filter messages. `RUST_LOG` still overrides at
    // runtime because of `parse_default_env`.
    env_logger::Builder::new()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                window.show().ok();
                window.set_focus().ok();
                window.emit("window-shown", ()).ok();
            }
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("Failed to resolve app data dir: {e}"))?;
            std::fs::create_dir_all(&data_dir)
                .map_err(|e| format!("Failed to create data dir: {e}"))?;

            let db_path = data_dir.join("whisper_scribe.db");
            let audio_dir = data_dir.join("audio");
            std::fs::create_dir_all(&audio_dir)
                .map_err(|e| format!("Failed to create audio dir: {e}"))?;

            let screenshots_dir = data_dir.join("screenshots");
            std::fs::create_dir_all(&screenshots_dir)
                .map_err(|e| format!("Failed to create screenshots dir: {e}"))?;

            let storage = storage::Storage::new(&db_path)
                .map_err(|e| format!("Failed to init storage: {e}"))?;

            let app_state = Arc::new(AppState::new(storage));
            app.manage(app_state.clone());

            #[cfg(target_os = "macos")]
            apply_macos_vibrancy(app);

            shortcuts::register_show_hide(app);

            let app_handle = app.handle().clone();
            let state_clone = app_state.clone();
            std::thread::spawn(move || {
                pipeline::start(app_handle, state_clone, audio_dir);
            });

            let screen_app_handle = app.handle().clone();
            let screen_state = app_state.clone();
            std::thread::spawn(move || {
                screen_pipeline::start(screen_app_handle, screen_state, screenshots_dir);
            });

            tray::setup(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_timeline,
            commands::search_transcriptions,
            commands::get_status,
            commands::toggle_pause,
            commands::subscribe_audio_level,
            commands::get_slots_by_date_range,
            commands::get_available_dates,
            commands::toggle_screen_capture,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(target_os = "macos")]
fn apply_macos_vibrancy(app: &tauri::App) {
    use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
    if let Some(window) = app.get_webview_window("main") {
        apply_vibrancy(
            &window,
            NSVisualEffectMaterial::Dark,
            Some(NSVisualEffectState::Active),
            Some(18.0),
        )
        .ok();
    }
}
