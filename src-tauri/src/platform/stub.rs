//! Non-Windows stub.
//!
//! The project is Windows-first and the window engine is not expected to run
//! anywhere else yet. This exists so `cargo test` and any future port compile
//! without `#[cfg(windows)]` leaking into the callers — it is a compile target,
//! not a supported platform.

use super::{DiskSpace, NativeWindow, TreeEvent, TreeWatch, Volume, WindowPlatform};

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

    // Elsewhere `CommandChild::kill`, which the caller always runs too, is
    // what there is.
    fn kill_tree(&self, _pid: u32) {}

    fn open_folder(&self, _path: &std::path::Path) -> Result<(), String> {
        Err("opening a folder is not supported on this platform".into())
    }

    // Elsewhere the library notices a folder when its root is clicked (D95).
    fn watch_tree(
        &self,
        _root: &std::path::Path,
        _on_event: Box<dyn FnMut(TreeEvent) + Send>,
    ) -> Result<TreeWatch, String> {
        Err("watching a folder is not supported on this platform".into())
    }

    fn in_use(&self, _path: &std::path::Path) -> bool {
        false
    }

    // Elsewhere the meter shows what the library takes and no drive.
    fn disk_space(&self, _path: &std::path::Path) -> Option<DiskSpace> {
        None
    }

    // Elsewhere a root on a drive that comes back somewhere else is added
    // again by hand.
    fn volume_of(&self, _path: &std::path::Path) -> Option<Volume> {
        None
    }

    fn mounts_of(&self, _id: &str) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
}
