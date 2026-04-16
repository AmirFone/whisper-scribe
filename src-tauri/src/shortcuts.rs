use tauri::Manager;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// Register Cmd+, to toggle the main window's visibility.
pub fn register_show_hide(app: &tauri::App) {
    let shortcut = Shortcut::new(Some(Modifiers::META), Code::Comma);
    let handle = app.handle().clone();

    app.global_shortcut()
        .on_shortcut(shortcut, move |_app, _shortcut, event| {
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
        })
        .ok();
}
