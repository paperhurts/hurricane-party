//! #87: the library's × hides it to the notification area instead of closing
//! it, and a tray icon brings it back. The icon exists only while the library
//! is hidden (D87), so the tray gains no permanent resident. Closing Main
//! still quits (D63); the tray's Quit is that same exit.

use tauri::{
    menu::{MenuBuilder, MenuEvent},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

/// The one tray icon's id, so it can be found again and removed.
const ID: &str = "library";

/// Hide the library behind a tray icon. If the icon cannot be made there is
/// no way back, so the window stays where it is (D59's rule, applied here).
pub fn hide_library(app: &AppHandle) {
    let Some(win) = app.get_webview_window("library") else {
        return;
    };
    if let Err(e) = install(app) {
        eprintln!("tray: no icon ({e}); the library stays visible");
        return;
    }
    let _ = win.hide();
}

/// Show the library and take the icon down. Also the playlist's LIB button,
/// which must bring a hidden library back the same way.
pub fn reveal_library(app: &AppHandle) {
    let _ = app.remove_tray_by_id(ID);
    if let Some(w) = app.get_webview_window("library") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn install(app: &AppHandle) -> tauri::Result<()> {
    if app.tray_by_id(ID).is_some() {
        return Ok(());
    }
    // The smallest useful menu (#87): the way back, the one transport gesture
    // worth having without a window, and the exit.
    let menu = MenuBuilder::new(app)
        .text("show", "Show library")
        .text("toggle", "Play / Pause")
        .separator()
        .text("quit", "Quit")
        .build()?;
    let mut tray = TrayIconBuilder::with_id(ID)
        .tooltip("hurricane-party")
        .menu(&menu)
        // Left click is the way back; the menu is the right button's.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e: MenuEvent| match e.id().as_ref() {
            "show" => reveal_library(app),
            "toggle" => {
                if let Err(e) = crate::control::route(app, "toggle", None) {
                    eprintln!("tray: play/pause: {e}");
                }
            }
            "quit" => {
                // D63's exit, saved layout and all.
                crate::wm::save_now(app);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                reveal_library(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}
