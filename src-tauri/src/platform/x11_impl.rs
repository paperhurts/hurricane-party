//! Linux under X11 (and XWayland, D181): the spike's platform.
//!
//! Only what a stage of `spike-linux.md` needs in order to be measured is
//! real; everything else is the stub's answer. Stage 1's drag reads the screen
//! cursor, so that is the first call here.
//!
//! Xlib is loaded at run time through `x11-dl`, and each thread opens its own
//! connection: Xlib is not thread-safe without `XInitThreads`, and the window
//! engine calls in from command threads, as it does Win32 on Windows.

use super::stub::StubPlatform;
use super::{DiskSpace, NativeWindow, TreeEvent, TreeWatch, Volume, WindowPlatform};
use std::ptr;
use x11_dl::xlib;

pub struct X11Platform;

thread_local! {
    /// This thread's Xlib and display, opened on first use and kept for the
    /// life of the thread. None when there is no X server to talk to.
    static X: Option<(xlib::Xlib, *mut xlib::Display)> = {
        let x = xlib::Xlib::open().ok();
        x.and_then(|x| {
            let d = unsafe { (x.XOpenDisplay)(ptr::null()) };
            (!d.is_null()).then_some((x, d))
        })
    };
}

impl WindowPlatform for X11Platform {
    fn assert_dpi_aware(&self) -> String {
        "n/a (X11: one scale for the whole screen)".to_string()
    }

    fn owner_of(&self, w: NativeWindow) -> NativeWindow {
        StubPlatform.owner_of(w)
    }

    fn set_owner(&self, w: NativeWindow, owner: NativeWindow) -> NativeWindow {
        StubPlatform.set_owner(w, owner)
    }

    fn raise_no_activate(&self, w: NativeWindow) {
        StubPlatform.raise_no_activate(w)
    }

    /// Root coordinates in X's own pixels, which under XWayland are the
    /// physical pixels the bond math is in.
    fn cursor_pos(&self) -> (i32, i32) {
        X.with(|x| {
            let Some((x, d)) = x else {
                return (0, 0);
            };
            let (mut root_ret, mut child) = (0, 0);
            let (mut rx, mut ry, mut wx, mut wy, mut mask) = (0, 0, 0, 0, 0);
            unsafe {
                let root = (x.XDefaultRootWindow)(*d);
                (x.XQueryPointer)(
                    *d,
                    root,
                    &mut root_ret,
                    &mut child,
                    &mut rx,
                    &mut ry,
                    &mut wx,
                    &mut wy,
                    &mut mask,
                );
            }
            (rx, ry)
        })
    }

    fn set_topmost(&self, w: NativeWindow, on: bool) {
        StubPlatform.set_topmost(w, on)
    }

    fn is_minimized(&self, w: NativeWindow) -> bool {
        StubPlatform.is_minimized(w)
    }

    fn restore_no_activate(&self, w: NativeWindow) {
        StubPlatform.restore_no_activate(w)
    }

    fn show_minimized_no_activate(&self, w: NativeWindow) {
        StubPlatform.show_minimized_no_activate(w)
    }

    fn kill_tree(&self, pid: u32) {
        StubPlatform.kill_tree(pid)
    }

    fn spawn_quiet(
        &self,
        exe: &std::path::Path,
        args: &[&str],
    ) -> std::io::Result<std::process::Child> {
        StubPlatform.spawn_quiet(exe, args)
    }

    fn ask_companion_to_leave(&self) {
        StubPlatform.ask_companion_to_leave()
    }

    fn companion_is_running(&self) -> bool {
        StubPlatform.companion_is_running()
    }

    fn run_quiet(
        &self,
        exe: &std::path::Path,
        args: &[&std::ffi::OsStr],
    ) -> std::io::Result<std::process::Output> {
        StubPlatform.run_quiet(exe, args)
    }

    fn open_folder(&self, path: &std::path::Path) -> Result<(), String> {
        StubPlatform.open_folder(path)
    }

    fn watch_tree(
        &self,
        root: &std::path::Path,
        on_event: Box<dyn FnMut(TreeEvent) + Send>,
    ) -> Result<TreeWatch, String> {
        StubPlatform.watch_tree(root, on_event)
    }

    fn in_use(&self, path: &std::path::Path) -> bool {
        StubPlatform.in_use(path)
    }

    fn disk_space(&self, path: &std::path::Path) -> Option<DiskSpace> {
        StubPlatform.disk_space(path)
    }

    fn volume_of(&self, path: &std::path::Path) -> Option<Volume> {
        StubPlatform.volume_of(path)
    }

    fn mounts_of(&self, id: &str) -> Vec<std::path::PathBuf> {
        StubPlatform.mounts_of(id)
    }
}
