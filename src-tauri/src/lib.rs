mod audio_engine;
mod commands;
mod device_manager;
mod power_monitor;
mod storage;
mod transcriber;

use parking_lot::Mutex;
use std::sync::Arc;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Emitter, Manager,
};

pub struct AppState {
    pub storage: storage::Storage,
    pub transcriber: Mutex<Option<transcriber::Transcriber>>,
    pub is_paused: Mutex<bool>,
    pub paused_by_system: std::sync::atomic::AtomicBool,
    pub recording_started_at: Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    pub is_transcribing: std::sync::atomic::AtomicBool,
}

// Safety: All fields use interior mutability via parking_lot::Mutex or atomics.
// rusqlite::Connection is !Send but is always accessed behind Mutex<Connection>.
// Transcriber wraps its process handles in Mutex. This is sound.
unsafe impl Send for AppState {}
unsafe impl Sync for AppState {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                window.show().ok();
                window.set_focus().ok();
            }
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            std::fs::create_dir_all(&data_dir).ok();

            let db_path = data_dir.join("whisper_scribe.db");
            let audio_dir = data_dir.join("audio");
            std::fs::create_dir_all(&audio_dir).ok();

            let storage = storage::Storage::new(&db_path).expect("failed to init storage");

            let state = Arc::new(AppState {
                storage,
                transcriber: Mutex::new(None),
                is_paused: Mutex::new(false),
                recording_started_at: Mutex::new(None),
                is_transcribing: std::sync::atomic::AtomicBool::new(false),
                paused_by_system: std::sync::atomic::AtomicBool::new(false),
            });
            app.manage(state.clone());

            // Apply real macOS vibrancy (NSVisualEffectView)
            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "macos")]
                {
                    use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
                    apply_vibrancy(
                        &window,
                        NSVisualEffectMaterial::HudWindow,
                        Some(NSVisualEffectState::Active), // Always active — prevents grey flash on focus loss
                        None,
                    )
                    .ok();
                }
            }

            // Register Cmd+, to show window
            {
                use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
                let shortcut = Shortcut::new(Some(Modifiers::META), Code::Comma);
                let handle = app.handle().clone();
                app.global_shortcut().on_shortcut(shortcut, move |_app, _shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    if let Some(window) = handle.get_webview_window("main") {
                        if window.is_visible().unwrap_or(false) {
                            window.hide().ok();
                        } else {
                            window.show().ok();
                            window.set_focus().ok();
                        }
                    }
                }).ok();
            }

            let app_handle = app.handle().clone();
            let state_clone = state.clone();

            std::thread::spawn(move || {
                start_pipeline(app_handle, state_clone, audio_dir);
            });

            setup_tray(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_timeline,
            commands::search_transcriptions,
            commands::get_status,
            commands::toggle_pause,
            commands::copy_to_clipboard,
            commands::subscribe_audio_level,
            commands::get_slots_by_date_range,
            commands::get_available_dates,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let quit = MenuItemBuilder::with_id("quit", "Quit Whisper Scribe").build(app)?;
    let toggle = MenuItemBuilder::with_id("toggle", "Pause Recording").build(app)?;
    let show = MenuItemBuilder::with_id("show", "Show Window").build(app)?;

    let menu = MenuBuilder::new(app)
        .items(&[&show, &toggle, &quit])
        .build()?;

    let icon = make_tray_icon();
    let _tray = TrayIconBuilder::new()
        .icon(icon)
        .icon_as_template(true)
        .menu(&menu)
        .tooltip("Whisper Scribe")
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "quit" => {
                app.exit(0);
            }
            "toggle" => {
                let state = app.state::<Arc<AppState>>();
                let mut paused = state.is_paused.lock();
                *paused = !*paused;
            }
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    window.show().ok();
                    window.set_focus().ok();
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click { .. } = event {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    if window.is_visible().unwrap_or(false) {
                        window.hide().ok();
                    } else {
                        window.show().ok();
                        window.set_focus().ok();
                    }
                }
            }
        })
        .build(app)?;

    Ok(())
}

/// Tray icon: audio waveform/scribe icon (NOT a microphone — avoids conflict)
/// Rendered as a template image (black on transparent, macOS inverts for dark mode)
fn make_tray_icon() -> Image<'static> {
    let size: u32 = 44; // @2x for retina
    let mut rgba = vec![0u8; (size * size * 4) as usize];

    let set = |rgba: &mut Vec<u8>, x: i32, y: i32, a: u8| {
        if x >= 0 && x < size as i32 && y >= 0 && y < size as i32 {
            let idx = ((y as u32 * size + x as u32) * 4) as usize;
            rgba[idx] = 0;
            rgba[idx + 1] = 0;
            rgba[idx + 2] = 0;
            // Blend: keep the max alpha
            if a > rgba[idx + 3] {
                rgba[idx + 3] = a;
            }
        }
    };

    let cx = size as i32 / 2;

    // Draw a document/page with waveform — "scribe" concept
    // Page outline (rounded rect)
    let page_l = 10i32;
    let page_r = 34i32;
    let page_t = 5i32;
    let page_b = 39i32;
    let r = 3i32;

    // Top and bottom edges
    for x in (page_l + r)..=(page_r - r) {
        for w in 0..2 {
            set(&mut rgba, x, page_t + w, 255);
            set(&mut rgba, x, page_b - w, 255);
        }
    }
    // Left and right edges
    for y in (page_t + r)..=(page_b - r) {
        for w in 0..2 {
            set(&mut rgba, page_l + w, y, 255);
            set(&mut rgba, page_r - w, y, 255);
        }
    }
    // Rounded corners
    for a in 0..=90 {
        let rad = (a as f32).to_radians();
        let dx = (r as f32 * rad.cos()) as i32;
        let dy = (r as f32 * rad.sin()) as i32;
        // top-left
        set(&mut rgba, page_l + r - dx, page_t + r - dy, 255);
        set(&mut rgba, page_l + r - dx + 1, page_t + r - dy, 255);
        // top-right
        set(&mut rgba, page_r - r + dx, page_t + r - dy, 255);
        set(&mut rgba, page_r - r + dx - 1, page_t + r - dy, 255);
        // bottom-left
        set(&mut rgba, page_l + r - dx, page_b - r + dy, 255);
        set(&mut rgba, page_l + r - dx + 1, page_b - r + dy, 255);
        // bottom-right
        set(&mut rgba, page_r - r + dx, page_b - r + dy, 255);
        set(&mut rgba, page_r - r + dx - 1, page_b - r + dy, 255);
    }

    // Audio waveform inside the page (3 bars, centered)
    let wave_bars: [(i32, i32); 5] = [
        (cx - 8, 6),  // short
        (cx - 4, 10), // medium
        (cx, 14),      // tall
        (cx + 4, 10), // medium
        (cx + 8, 6),  // short
    ];
    let wave_cy = 22i32;

    for &(bx, height) in &wave_bars {
        let half = height / 2;
        for y in (wave_cy - half)..=(wave_cy + half) {
            for w in 0..3 {
                set(&mut rgba, bx + w - 1, y, 255);
            }
        }
        // Round the ends
        set(&mut rgba, bx, wave_cy - half - 1, 180);
        set(&mut rgba, bx, wave_cy + half + 1, 180);
    }

    // Small text lines below waveform (representing transcribed text)
    for &y in &[31i32, 34] {
        for x in (page_l + 5)..=(page_r - 5) {
            set(&mut rgba, x, y, 140);
            set(&mut rgba, x, y + 1, 140);
        }
    }

    Image::new_owned(rgba, size, size)
}

fn process_orphaned_segments(app: &tauri::AppHandle, state: &Arc<AppState>, audio_dir: &std::path::Path) {
    let mut segments: Vec<std::path::PathBuf> = std::fs::read_dir(audio_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "wav"))
        .collect();

    segments.sort();

    // Skip the last segment (likely still being written by a previous instance)
    if segments.len() > 1 {
        let to_process = &segments[..segments.len() - 1];
        log::info!("Found {} orphaned segments to process", to_process.len());

        for path in to_process {
            // Check file size — skip empty/header-only files
            let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            if size < 1000 {
                log::info!("Skipping tiny segment: {} ({} bytes)", path.display(), size);
                continue;
            }

            // Check if already transcribed (by matching start_time from filename)
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            // Check if this segment was already transcribed using efficient DB query
            let date_part = stem.strip_prefix("segment_").unwrap_or(stem);
            let ts = chrono::NaiveDateTime::parse_from_str(date_part, "%Y%m%d_%H%M%S")
                .map(|dt| dt.and_utc().to_rfc3339())
                .unwrap_or_default();
            let already_exists = state.storage.has_transcription_near(&ts);

            if already_exists {
                continue;
            }

            log::info!("Transcribing orphaned segment: {}", path.display());
            state.is_transcribing.store(true, std::sync::atomic::Ordering::Relaxed);

            let transcriber_guard = state.transcriber.lock();
            if let Some(t) = transcriber_guard.as_ref() {
                match t.transcribe(path) {
                    Ok(result) => {
                        if result.text.is_empty() {
                            log::info!("Skipping empty/silent segment");
                        } else if let Err(e) = state.storage.append_to_hour_slot(
                            &result.text,
                            &result.start_time,
                            &result.device,
                        ) {
                            log::error!("Failed to store orphaned transcription: {e}");
                        }
                        app.emit("transcription-updated", &true).ok();
                    }
                    Err(e) => log::error!("Failed to transcribe orphaned segment: {e}"),
                }
            }
            state.is_transcribing.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    }

    // Clean up the orphaned segments after processing
    audio_engine::cleanup_old_segments(audio_dir, 6);
}

fn start_pipeline(app: tauri::AppHandle, state: Arc<AppState>, audio_dir: std::path::PathBuf) {
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

    // Process any orphaned segments from previous app runs
    process_orphaned_segments(&app, &state, &audio_dir);

    log::info!("Starting audio engine...");
    let (segment_tx, segment_rx) = crossbeam_channel::unbounded::<std::path::PathBuf>();

    let _engine = match audio_engine::AudioEngine::new(audio_dir.clone(), segment_tx) {
        Ok(engine) => {
            log::info!("Audio engine started");
            *state.recording_started_at.lock() = Some(chrono::Utc::now());
            engine
        }
        Err(e) => {
            log::error!("Failed to start audio engine: {e}");
            return;
        }
    };

    log::info!("Starting power monitor...");
    let _power_monitor = match power_monitor::PowerMonitor::new(state.clone()) {
        Ok(pm) => {
            log::info!("Power monitor active");
            Some(pm)
        }
        Err(e) => {
            log::warn!("Power monitor unavailable: {e}");
            None
        }
    };

    loop {
        match segment_rx.recv() {
            Ok(path) => {
                // Reset timer immediately when segment arrives (new segment already recording)
                *state.recording_started_at.lock() = Some(chrono::Utc::now());

                log::info!("Transcribing segment: {}", path.display());
                state.is_transcribing.store(true, std::sync::atomic::Ordering::Relaxed);
                let transcriber_guard = state.transcriber.lock();
                if let Some(t) = transcriber_guard.as_ref() {
                    match t.transcribe(&path) {
                        Ok(result) => {
                            if result.text.is_empty() {
                                log::info!("Skipping empty/silent segment");
                            } else if let Err(e) = state.storage.append_to_hour_slot(
                                &result.text,
                                &result.start_time,
                                &result.device,
                            ) {
                                log::error!("Failed to store transcription: {e}");
                            }
                            app.emit("transcription-updated", &true).ok();
                        }
                        Err(e) => {
                            log::error!("Transcription failed: {e}");
                        }
                    }
                }

                state.is_transcribing.store(false, std::sync::atomic::Ordering::Relaxed);
                audio_engine::cleanup_old_segments(&audio_dir, 6);
                audio_engine::cleanup_old_audio(&audio_dir, 3600); // Delete WAVs older than 1 hour
                *state.recording_started_at.lock() = Some(chrono::Utc::now());
            }
            Err(_) => break,
        }
    }
}
