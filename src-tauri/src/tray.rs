use crate::state::AppState;
use std::sync::Arc;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
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
        window.emit("window-shown", ()).ok();
    }
}

fn toggle_window_visibility(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            window.emit("request-hide", ()).ok();
        } else {
            window.show().ok();
            window.set_focus().ok();
            window.emit("window-shown", ()).ok();
        }
    }
}

/// Tray icon: a 3×3 grid of rounded-square outlines with the middle-right
/// cell filled, matching the main app logo's "lit cell" motif. Rendered as a
/// template image (black on transparent — macOS inverts for dark mode).
fn make_icon() -> Image<'static> {
    const SIZE: u32 = 44; // @2x for retina
    const PAD: i32 = 4; // outer padding from edges
    const GAP: i32 = 2; // gap between cells
    // 3 cells + 2 gaps + 2 pads = SIZE  →  cell size = (SIZE - 2*PAD - 2*GAP) / 3
    const CELL: i32 = (SIZE as i32 - 2 * PAD - 2 * GAP) / 3; // = 10
    const STROKE: i32 = 2;
    const CORNER_R: i32 = 2;
    // The filled cell position (row, col) with 0-indexed rows top-to-bottom.
    // Middle row, right column mirrors the logo's lit square.
    const FILLED: (i32, i32) = (1, 2);

    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];

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

    // Paint a rounded rectangle. `filled=true` paints the interior solid; else
    // only the outlined border (thickness = STROKE) with rounded corners.
    let draw_cell = |rgba: &mut Vec<u8>, x0: i32, y0: i32, filled: bool| {
        let x1 = x0 + CELL - 1;
        let y1 = y0 + CELL - 1;

        // Corner-aware hit test: a pixel is "inside" the rounded box if it's
        // inside the axis-aligned box AND, when inside a corner quadrant,
        // within CORNER_R of the corner center.
        let inside = |x: i32, y: i32| -> bool {
            if x < x0 || x > x1 || y < y0 || y > y1 {
                return false;
            }
            let (cx, cy) = if x < x0 + CORNER_R && y < y0 + CORNER_R {
                (x0 + CORNER_R, y0 + CORNER_R)
            } else if x > x1 - CORNER_R && y < y0 + CORNER_R {
                (x1 - CORNER_R, y0 + CORNER_R)
            } else if x < x0 + CORNER_R && y > y1 - CORNER_R {
                (x0 + CORNER_R, y1 - CORNER_R)
            } else if x > x1 - CORNER_R && y > y1 - CORNER_R {
                (x1 - CORNER_R, y1 - CORNER_R)
            } else {
                return true; // center / straight edges
            };
            let dx = x - cx;
            let dy = y - cy;
            dx * dx + dy * dy <= CORNER_R * CORNER_R
        };

        for y in y0..=y1 {
            for x in x0..=x1 {
                if !inside(x, y) {
                    continue;
                }
                if filled {
                    set(rgba, x, y, 255);
                } else {
                    // Border-only: paint pixel if NOT inside the inner
                    // inset-by-STROKE box.
                    let ix0 = x0 + STROKE;
                    let iy0 = y0 + STROKE;
                    let ix1 = x1 - STROKE;
                    let iy1 = y1 - STROKE;
                    let in_inner = x >= ix0 && x <= ix1 && y >= iy0 && y <= iy1 && {
                        let (cx, cy) = if x < ix0 + CORNER_R && y < iy0 + CORNER_R {
                            (ix0 + CORNER_R, iy0 + CORNER_R)
                        } else if x > ix1 - CORNER_R && y < iy0 + CORNER_R {
                            (ix1 - CORNER_R, iy0 + CORNER_R)
                        } else if x < ix0 + CORNER_R && y > iy1 - CORNER_R {
                            (ix0 + CORNER_R, iy1 - CORNER_R)
                        } else if x > ix1 - CORNER_R && y > iy1 - CORNER_R {
                            (ix1 - CORNER_R, iy1 - CORNER_R)
                        } else {
                            (x, y)
                        };
                        let dx = x - cx;
                        let dy = y - cy;
                        dx * dx + dy * dy <= CORNER_R * CORNER_R
                    };
                    if !in_inner {
                        set(rgba, x, y, 255);
                    }
                }
            }
        }
    };

    for row in 0..3 {
        for col in 0..3 {
            let x0 = PAD + col * (CELL + GAP);
            let y0 = PAD + row * (CELL + GAP);
            let filled = (row, col) == FILLED;
            draw_cell(&mut rgba, x0, y0, filled);
        }
    }

    Image::new_owned(rgba, SIZE, SIZE)
}
