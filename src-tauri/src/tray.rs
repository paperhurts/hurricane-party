//! #87: the library's × hides it to the notification area instead of closing
//! it, and a tray icon brings it back. The icon exists only while the library
//! is hidden (D87), so the tray gains no permanent resident. Closing Main
//! still quits (D63); the tray's Quit is that same exit.

use std::sync::Mutex;
use std::time::{Duration, Instant};
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

/// When the library last lost focus (D123). LIB needs to know whether the
/// library was in front when it was pressed, and it cannot ask: the press
/// lands on the playlist, which takes focus as the pointer goes down, so by
/// the time the button acts the library is never the focused window. A library
/// that lost focus a moment ago was in front, and that is what this records.
static LIBRARY_LEFT: Mutex<Option<Instant>> = Mutex::new(None);

/// How recent "a moment ago" is: a press and its release, with room for a
/// slow finger, and short of a person who clicked a playlist row and only
/// later reached for LIB.
const IN_FRONT_JUST_NOW: Duration = Duration::from_millis(1000);

/// Record that the library lost focus. Called from its window events.
pub fn library_lost_focus() {
    if let Ok(mut left) = LIBRARY_LEFT.lock() {
        *left = Some(Instant::now());
    }
}

/// What LIB does (D123): hide a library that was in front a moment ago, and
/// bring back any other — covered by another window, minimised, or in the
/// tray. So every press visibly does something, where "bring it forward"
/// alone did nothing a person could see when the library was already there.
pub fn toggle_hides(visible: bool, minimized: bool, left_ago: Option<Duration>) -> bool {
    visible && !minimized && left_ago.is_some_and(|d| d <= IN_FRONT_JUST_NOW)
}

/// The playlist's LIB button, and a classic skin's LOAD LIST.
pub fn toggle_library(app: &AppHandle) {
    let Some(w) = app.get_webview_window("library") else {
        return;
    };
    let visible = w.is_visible().unwrap_or(false);
    let minimized = w.is_minimized().unwrap_or(false);
    let left_ago = LIBRARY_LEFT
        .lock()
        .ok()
        .and_then(|left| *left)
        .map(|t| t.elapsed());
    if toggle_hides(visible, minimized, left_ago) {
        hide_library(app);
    } else {
        reveal_library(app);
    }
}

/// Show the library and take the icon down. Also the second press of LIB,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lib_hides_a_library_that_was_just_in_front_and_brings_back_any_other() {
        let just = Some(Duration::from_millis(200));
        let long_ago = Some(Duration::from_secs(30));
        // In front a moment ago, so the press is "put it away".
        assert!(toggle_hides(true, false, just));
        // Visible but not in front (covered, or never focused): bring it forward.
        assert!(!toggle_hides(true, false, long_ago));
        assert!(!toggle_hides(true, false, None));
        // Minimised or in the tray: always bring it back.
        assert!(!toggle_hides(true, true, just));
        assert!(!toggle_hides(false, false, just));
    }
}
