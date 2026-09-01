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
