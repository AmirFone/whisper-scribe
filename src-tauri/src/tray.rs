use crate::state::AppState;
use std::sync::Arc;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{TrayIconBuilder, TrayIconEvent},
    Manager,
};

pub fn setup(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let quit = MenuItemBuilder::with_id("quit", "Quit Whisper Scribe").build(app)?;
    let toggle = MenuItemBuilder::with_id("toggle", "Pause Recording").build(app)?;
    let show = MenuItemBuilder::with_id("show", "Show Window").build(app)?;

    let menu = MenuBuilder::new(app).items(&[&show, &toggle, &quit]).build()?;

    let _tray = TrayIconBuilder::new()
        .icon(make_icon())
        .icon_as_template(true)
        .menu(&menu)
        .tooltip("Whisper Scribe")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "quit" => app.exit(0),
            "toggle" => {
                let state = app.state::<Arc<AppState>>();
                state.toggle_pause();
            }
            "show" => show_window(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { .. } = event {
                toggle_window_visibility(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn show_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        window.show().ok();
        window.set_focus().ok();
    }
}

fn toggle_window_visibility(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            window.hide().ok();
        } else {
            window.show().ok();
            window.set_focus().ok();
        }
    }
}

/// Tray icon: a page-with-waveform glyph rendered as a template image
/// (black on transparent — macOS inverts for dark mode).
fn make_icon() -> Image<'static> {
    const SIZE: u32 = 44; // @2x for retina
    const PAGE_L: i32 = 10;
    const PAGE_R: i32 = 34;
    const PAGE_T: i32 = 5;
    const PAGE_B: i32 = 39;
    const CORNER_R: i32 = 3;

    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    let cx = SIZE as i32 / 2;

    let set = |rgba: &mut Vec<u8>, x: i32, y: i32, a: u8| {
        if x >= 0 && x < SIZE as i32 && y >= 0 && y < SIZE as i32 {
            let idx = ((y as u32 * SIZE + x as u32) * 4) as usize;
            rgba[idx] = 0;
            rgba[idx + 1] = 0;
            rgba[idx + 2] = 0;
            if a > rgba[idx + 3] {
                rgba[idx + 3] = a;
            }
        }
    };

    // Page edges
    for x in (PAGE_L + CORNER_R)..=(PAGE_R - CORNER_R) {
        for w in 0..2 {
            set(&mut rgba, x, PAGE_T + w, 255);
            set(&mut rgba, x, PAGE_B - w, 255);
        }
    }
    for y in (PAGE_T + CORNER_R)..=(PAGE_B - CORNER_R) {
        for w in 0..2 {
            set(&mut rgba, PAGE_L + w, y, 255);
            set(&mut rgba, PAGE_R - w, y, 255);
        }
    }

    // Rounded corners
    for a in 0..=90 {
        let rad = (a as f32).to_radians();
        let dx = (CORNER_R as f32 * rad.cos()) as i32;
        let dy = (CORNER_R as f32 * rad.sin()) as i32;
        set(&mut rgba, PAGE_L + CORNER_R - dx, PAGE_T + CORNER_R - dy, 255);
        set(&mut rgba, PAGE_L + CORNER_R - dx + 1, PAGE_T + CORNER_R - dy, 255);
        set(&mut rgba, PAGE_R - CORNER_R + dx, PAGE_T + CORNER_R - dy, 255);
        set(&mut rgba, PAGE_R - CORNER_R + dx - 1, PAGE_T + CORNER_R - dy, 255);
        set(&mut rgba, PAGE_L + CORNER_R - dx, PAGE_B - CORNER_R + dy, 255);
        set(&mut rgba, PAGE_L + CORNER_R - dx + 1, PAGE_B - CORNER_R + dy, 255);
        set(&mut rgba, PAGE_R - CORNER_R + dx, PAGE_B - CORNER_R + dy, 255);
        set(&mut rgba, PAGE_R - CORNER_R + dx - 1, PAGE_B - CORNER_R + dy, 255);
    }

    // Audio waveform (5 bars)
    let wave_bars: [(i32, i32); 5] = [
        (cx - 8, 6),
        (cx - 4, 10),
        (cx, 14),
        (cx + 4, 10),
        (cx + 8, 6),
    ];
    let wave_cy = 22i32;
    for &(bx, height) in &wave_bars {
        let half = height / 2;
        for y in (wave_cy - half)..=(wave_cy + half) {
            for w in 0..3 {
                set(&mut rgba, bx + w - 1, y, 255);
            }
        }
        set(&mut rgba, bx, wave_cy - half - 1, 180);
        set(&mut rgba, bx, wave_cy + half + 1, 180);
    }

    // Text lines below waveform
    for &y in &[31i32, 34] {
        for x in (PAGE_L + 5)..=(PAGE_R - 5) {
            set(&mut rgba, x, y, 140);
            set(&mut rgba, x, y + 1, 140);
        }
    }

    Image::new_owned(rgba, SIZE, SIZE)
}
