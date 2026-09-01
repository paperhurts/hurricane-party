//! Non-Windows stub.
//!
//! The project is Windows-first and the window engine is not expected to run
//! anywhere else yet. This exists so `cargo test` and any future port compile
//! without `#[cfg(windows)]` leaking into the callers — it is a compile target,
//! not a supported platform.

use super::{NativeWindow, WindowPlatform};

pub struct StubPlatform;

impl WindowPlatform for StubPlatform {
    fn assert_dpi_aware(&self) -> String {
        "n/a (non-Windows)".to_string()
    }

    fn owner_of(&self, _w: NativeWindow) -> NativeWindow {
        NativeWindow::NONE
    }

    fn set_owner(&self, _w: NativeWindow, _owner: NativeWindow) -> NativeWindow {
        NativeWindow::NONE
    }

    fn raise_no_activate(&self, _w: NativeWindow) {}

    fn cursor_pos(&self) -> (i32, i32) {
        (0, 0)
    }

    fn set_topmost(&self, _w: NativeWindow, _on: bool) {}

    fn is_minimized(&self, _w: NativeWindow) -> bool {
        false
    }

    fn restore_no_activate(&self, _w: NativeWindow) {}
}
