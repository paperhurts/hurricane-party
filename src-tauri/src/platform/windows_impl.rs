//! The Windows implementation of [`WindowPlatform`]. Six calls, D44 + D57.
//!
//! Every `unsafe` block in the window engine is in this file.

use super::{NativeWindow, WindowPlatform};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, GetAwarenessFromDpiAwarenessContext,
    GetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    DPI_AWARENESS_PER_MONITOR_AWARE, DPI_AWARENESS_SYSTEM_AWARE, DPI_AWARENESS_UNAWARE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, IsIconic, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_HWNDPARENT,
    HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE,
};

pub struct Win32Platform;

fn hwnd(w: NativeWindow) -> HWND {
    HWND(w.0 as _)
}

impl WindowPlatform for Win32Platform {
    fn assert_dpi_aware(&self) -> String {
        // SAFETY: all four are read-only queries of the calling thread's own
        // DPI context. None take a handle, so none can touch another thread.
        let (is_v2, name) = unsafe {
            let ctx = GetThreadDpiAwarenessContext();
            let is_v2 =
                AreDpiAwarenessContextsEqual(ctx, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
                    .as_bool();
            // The DPI_AWARENESS enum cannot distinguish v2 from v1, which is
            // exactly the distinction that matters here — hence the context
            // comparison above, with the enum used only to name the shortfall.
            let awareness = GetAwarenessFromDpiAwarenessContext(ctx);
            let name = if is_v2 {
                "PER_MONITOR_AWARE_V2"
            } else if awareness == DPI_AWARENESS_UNAWARE {
                "UNAWARE"
            } else if awareness == DPI_AWARENESS_SYSTEM_AWARE {
                "SYSTEM_AWARE"
            } else if awareness == DPI_AWARENESS_PER_MONITOR_AWARE {
                "PER_MONITOR_AWARE (v1)"
            } else {
                "unknown"
            };
            (is_v2, name)
        };

        assert!(
            is_v2,
            "D37 STARTUP CHECK FAILED: process DPI awareness is {name}, expected \
             PER_MONITOR_AWARE_V2. Every coordinate in this process is suspect: \
             bonds will look flush on one monitor and open a gap on another. \
             Check that the application manifest still declares \
             <dpiAwareness>PerMonitorV2</dpiAwareness>."
        );
        name.to_string()
    }

    fn owner_of(&self, w: NativeWindow) -> NativeWindow {
        // SAFETY: a read of one window long. Does not send a message, so it is
        // safe cross-thread and cannot deadlock under D54.
        NativeWindow(unsafe { GetWindowLongPtrW(hwnd(w), GWLP_HWNDPARENT) })
    }

    fn set_owner(&self, w: NativeWindow, owner: NativeWindow) -> NativeWindow {
        // Despite the name, GWLP_HWNDPARENT sets the OWNER of a top-level
        // window, not its parent. Passing 0 clears it. This is the primitive
        // the entire D41 hidden-root topology is built from.
        // SAFETY: see D54 on the trait — caller must not hold the state lock.
        NativeWindow(unsafe { SetWindowLongPtrW(hwnd(w), GWLP_HWNDPARENT, owner.0) })
    }

    fn raise_no_activate(&self, w: NativeWindow) {
        // SAFETY: NOMOVE|NOSIZE means the four zeros are ignored. NOACTIVATE is
        // load-bearing, not defensive — see D42.
        unsafe {
            let _ = SetWindowPos(
                hwnd(w),
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    fn is_minimized(&self, w: NativeWindow) -> bool {
        // SAFETY: a state query on a window handle; no message is sent.
        unsafe { IsIconic(hwnd(w)) }.as_bool()
    }

    fn restore_no_activate(&self, w: NativeWindow) {
        // SW_SHOWNOACTIVATE rather than SW_RESTORE: the rescue runs from a
        // WM_DISPLAYCHANGE handler, and stealing focus because a monitor was
        // unplugged would be worse than the problem it fixes.
        // SAFETY: see D54 — caller must not hold the state lock.
        unsafe {
            let _ = ShowWindow(hwnd(w), SW_SHOWNOACTIVATE);
        }
    }
}
