//! The platform surface.
//!
//! D44 measured this in the v0.0 spike and it came back much smaller than
//! `windows.md` assumed: the whole of stages 0–5 needed **four** Win32 calls.
//! Stage 6 added a fifth and sixth for display-change recovery (D57). That is
//! the entire non-portable surface of the window engine, so it is a file, not a
//! subsystem.
//!
//! The project rule is that platform-specific calls go behind a trait rather
//! than scattering `#[cfg(windows)]` through the callers. The **only** `cfg` in
//! the window engine is the one below, choosing the implementation.
//!
//! All handles are `NativeWindow`, an opaque integer. On Windows it is an HWND;
//! nothing above this module is allowed to know that.
//!
//! The control API's transport (D9) is the other non-portable thing in the
//! app and lives in `pipe`, so the rule covers it too (#20).

pub mod pipe;

/// An opaque OS window handle. HWND on Windows.
///
/// Deliberately not `HWND`: a newtype over `isize` is what lets `bond.rs` and
/// the group logic above it stay cross-platform and unit-testable.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct NativeWindow(pub isize);

impl NativeWindow {
    pub const NONE: NativeWindow = NativeWindow(0);

    pub fn is_none(self) -> bool {
        self.0 == 0
    }
}

/// The whole non-portable surface of the window engine.
///
/// **D54 — never call any of these while holding the window-state lock.** A
/// cross-thread Win32 call on another thread's window sends a message and waits
/// for that thread's message pump; if the pump is blocked on the same lock, the
/// process deadlocks hard. Compute a plan under the lock, drop it, then call.
pub trait WindowPlatform: Send + Sync {
    /// D37: assert per-monitor-v2 DPI awareness. Returns the awareness level as
    /// a human-readable string for the log; **panics** if it is not v2.
    ///
    /// Not a diagnostic print. Tauri ships no `<dpiAware>` manifest element and
    /// tao reaches v2 through a four-rung fallback ladder where every rung below
    /// the top is a silent `let _ =`. A v1 or system-aware fallback still reads
    /// `scale_factor() == 1.5` on a single monitor while breaking every
    /// cross-monitor coordinate in the process.
    fn assert_dpi_aware(&self) -> String;

    /// The current owner of a window, or `NativeWindow::NONE`.
    fn owner_of(&self, w: NativeWindow) -> NativeWindow;

    /// Set the owner of a top-level window; `NativeWindow::NONE` clears it.
    /// Returns the previous owner.
    ///
    /// D41's hidden-root topology is built entirely out of this call. There is
    /// no Tauri API for it at runtime — `WebviewWindowBuilder::owner()` exists
    /// only at construction, and a bond group re-parents constantly.
    fn set_owner(&self, w: NativeWindow, owner: NativeWindow) -> NativeWindow;

    /// D42: force the pending ownership change to take effect now.
    ///
    /// Ownership applies lazily, on next activation, so after a re-parent the
    /// z-order is stale-but-plausible until the user clicks something. Not
    /// activating is the point: a bond break must never steal focus.
    fn raise_no_activate(&self, w: NativeWindow);

    /// The mouse cursor, in physical virtual-desktop coordinates.
    ///
    /// The drag loop asks the OS rather than converting the webview's
    /// `screenX`/`screenY`, which arrive in CSS pixels and would need a scale
    /// factor applied at exactly the boundary where mixing logical and physical
    /// produces bugs that only appear on a second monitor. Reading the cursor
    /// natively means the drag never converts anything.
    ///
    /// Unlike the calls above this takes no window handle, so it sends no
    /// message and cannot deadlock under D54.
    fn cursor_pos(&self) -> (i32, i32);

    /// D61: keep a window above every non-topmost window, without activating it.
    ///
    /// Applied to a whole bond group rather than one window: the classic three
    /// are owned by a hidden root and not by each other (D41), so topmost does
    /// not propagate along ownership the way a raise does. Lifting only the
    /// shaded Main window would leave its bonded neighbours behind other apps.
    fn set_topmost(&self, w: NativeWindow, on: bool);

    /// D57: is this window minimized?
    ///
    /// Losing a display minimizes the group rather than relocating it. The
    /// window lands at `-32000,-32000` with `IsIconic = true` while
    /// `IsVisible` stays true, so visibility is not the signal — this is.
    fn is_minimized(&self, w: NativeWindow) -> bool;

    /// D57: un-minimize without taking focus, so a rescue after
    /// `WM_DISPLAYCHANGE` does not yank the user out of whatever they are doing.
    ///
    /// D59 is why this has to exist at all: `skipTaskbar` + undecorated +
    /// minimized is unrecoverable by the user, and the classic windows are
    /// undecorated by design, so the constraint is permanent.
    fn restore_no_activate(&self, w: NativeWindow);

    /// D117: end a process **and every process it started**.
    ///
    /// The one call here that is not about a window, and it is here for the
    /// same reason the others are: the portable answer is wrong on Windows.
    /// The bundled yt-dlp is a PyInstaller one-file build, so the process the
    /// app spawns is a bootloader whose child does the downloading.
    /// `CommandChild::kill` ends the bootloader and orphans the child, which
    /// keeps writing the `.part` file — and a Resume then starts a second
    /// writer on the same file. Measured on the real binary before this
    /// existed: Pause ended one of the two processes and not the other.
    fn kill_tree(&self, pid: u32);

    /// Open a folder in the OS file manager (#155). Not about a window
    /// either, and here for the same reason as `kill_tree`: there is no
    /// portable call, and the shell's own "open" is the one that gets a path
    /// with spaces or commas right, where handing it to `explorer.exe` on a
    /// command line does not. The caller decides which folder; nothing from
    /// the webview reaches this as a path.
    fn open_folder(&self, path: &std::path::Path) -> Result<(), String>;

    /// Watch a folder and everything under it (#111, D137), calling
    /// `on_event` from a thread of the watch's own. Not about a window
    /// either: the portable crates for this are a dependency the app does
    /// not need, and on Windows a watch has one duty they skip, letting go of
    /// the drive when a person ejects it.
    ///
    /// `on_event` must not block. The watch cannot read the next change
    /// while it runs, and dropping the watch waits for its thread.
    fn watch_tree(
        &self,
        root: &std::path::Path,
        on_event: Box<dyn FnMut(TreeEvent) + Send>,
    ) -> Result<TreeWatch, String>;

    /// Whether another program has this file open (#111). A file still being
    /// copied in is not read: its tags and its length are not there yet.
    fn in_use(&self, path: &std::path::Path) -> bool;

    /// The space on the drive a folder is on (#162, D138): what this process
    /// may still write there, and the drive's size, in bytes. `None` when the
    /// folder is not there, which is an unplugged drive, not a full one.
    fn disk_space(&self, path: &std::path::Path) -> Option<DiskSpace>;
}

/// A drive's room, in bytes (#162).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct DiskSpace {
    /// What this process may still write. Quotas count, so this is the
    /// figure that fills up, not the drive's raw free space.
    pub free: u64,
    pub total: u64,
}

/// What a watched folder says happened (#111).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeEvent {
    /// Something at this path changed: added, removed, renamed to or from,
    /// written. Relative to the watched folder. What it is now is the
    /// caller's to look at; the event says only where.
    Changed(std::path::PathBuf),
    /// More changed than the platform could list: look at the whole folder.
    Overflow,
    /// The OS asked for the drive (an eject), and the watch let go of it so
    /// the eject is not refused on the app's account. The watch has ended.
    Released,
    /// The watch stopped on its own: the folder went away, or reading it
    /// failed.
    Ended,
}

/// A running watch. Dropping it stops the watch, and returns once it has.
pub struct TreeWatch {
    _guard: Box<dyn Send>,
}

impl TreeWatch {
    pub fn new(guard: Box<dyn Send>) -> TreeWatch {
        TreeWatch { _guard: guard }
    }
}

/// The native handle behind a Tauri window.
///
/// Kept here rather than in the window manager so that `wm.rs` — which is
/// otherwise pure logic over `NativeWindow` — needs no `cfg` of its own.
/// `HWND`'s single field is a raw pointer in every version of the `windows`
/// crate, so this does not care whether Tauri's copy matches ours.
#[cfg(windows)]
pub fn handle_of(w: &tauri::WebviewWindow) -> NativeWindow {
    match w.hwnd() {
        Ok(h) => NativeWindow(h.0 as isize),
        Err(_) => NativeWindow::NONE,
    }
}

#[cfg(not(windows))]
pub fn handle_of(_w: &tauri::WebviewWindow) -> NativeWindow {
    NativeWindow::NONE
}

#[cfg(windows)]
mod windows_impl;

#[cfg(not(windows))]
mod stub;

/// The one place the platform is chosen.
#[cfg(windows)]
pub fn platform() -> &'static dyn WindowPlatform {
    &windows_impl::Win32Platform
}

#[cfg(not(windows))]
pub fn platform() -> &'static dyn WindowPlatform {
    &stub::StubPlatform
}
