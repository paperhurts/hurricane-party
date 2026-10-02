//! Linux, as an X11 client: under XWayland on a Wayland desktop (D181), or on
//! an X session. Measured on GNOME by the spike (`spike-linux.md`, D182).
//!
//! The window calls are real; the rest (processes, folders, drives) are the
//! stub's until the port reaches them.
//!
//! Xlib is loaded at run time through `x11-dl`, and each thread opens its own
//! connection: Xlib is not thread-safe without `XInitThreads`, and the window
//! engine calls in from command threads, as it does Win32 on Windows. GTK's
//! own connection is never touched, so none of this has to run on the main
//! thread, and D54's deadlock cannot happen here.

use super::stub::StubPlatform;
use super::{DiskSpace, NativeWindow, TreeEvent, TreeWatch, Volume, WindowPlatform};
use std::ffi::CString;
use std::os::raw::{c_long, c_uchar, c_ulong};
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

fn with_x<R>(f: impl FnOnce(&xlib::Xlib, *mut xlib::Display) -> R) -> Option<R> {
    X.with(|x| x.as_ref().map(|(x, d)| f(x, *d)))
}

fn atom(x: &xlib::Xlib, d: *mut xlib::Display, name: &str) -> c_ulong {
    let n = CString::new(name).unwrap();
    unsafe { (x.XInternAtom)(d, n.as_ptr(), 0) }
}

/// A window property as bytes, or None when the window does not have it.
fn property(
    x: &xlib::Xlib,
    d: *mut xlib::Display,
    w: c_ulong,
    name: &str,
    ty: c_ulong,
) -> Option<Vec<u8>> {
    let prop = atom(x, d, name);
    let (mut actual, mut format, mut n, mut after) = (0, 0, 0, 0);
    let mut data: *mut c_uchar = ptr::null_mut();
    let ok = unsafe {
        (x.XGetWindowProperty)(
            d,
            w,
            prop,
            0,
            1024,
            0,
            ty,
            &mut actual,
            &mut format,
            &mut n,
            &mut after,
            &mut data,
        )
    };
    if ok != 0 || data.is_null() || actual == 0 {
        return None;
    }
    // Format 32 comes back as C longs, whatever the size of a long.
    let unit = match format {
        8 => 1,
        16 => 2,
        _ => std::mem::size_of::<c_long>(),
    };
    let bytes = unsafe { std::slice::from_raw_parts(data, n as usize * unit).to_vec() };
    unsafe { (x.XFree)(data.cast()) };
    Some(bytes)
}

fn first_long(bytes: &[u8]) -> Option<c_long> {
    bytes
        .get(..std::mem::size_of::<c_long>())
        .map(|b| c_long::from_ne_bytes(b.try_into().unwrap()))
}

/// The X window of this process titled `title`, searched from the root down.
///
/// This is `handle_of` without GTK, whose objects may only be touched on the
/// main thread, while the window engine asks from command threads. None for a
/// window GTK has not realized yet: a window that has never been shown has no
/// X window at all, which is also why D41's hidden roots do not exist here
/// (D182).
pub fn find_window(title: &str) -> Option<c_ulong> {
    let pid = std::process::id() as c_long;
    with_x(|x, d| {
        let utf8 = atom(x, d, "UTF8_STRING");
        let mut stack = vec![unsafe { (x.XDefaultRootWindow)(d) }];
        while let Some(w) = stack.pop() {
            if property(x, d, w, "_NET_WM_NAME", utf8).as_deref() == Some(title.as_bytes())
                && property(x, d, w, "_NET_WM_PID", xlib::XA_CARDINAL).and_then(|b| first_long(&b))
                    == Some(pid)
            {
                return Some(w);
            }
            let (mut r, mut p, mut kids, mut n) = (0, 0, ptr::null_mut(), 0);
            if unsafe { (x.XQueryTree)(d, w, &mut r, &mut p, &mut kids, &mut n) } != 0
                && !kids.is_null()
            {
                stack.extend_from_slice(unsafe { std::slice::from_raw_parts(kids, n as usize) });
                unsafe { (x.XFree)(kids.cast()) };
            }
        }
        None
    })
    .flatten()
}

impl WindowPlatform for X11Platform {
    fn assert_dpi_aware(&self) -> String {
        "n/a (X11: one scale for the whole screen)".to_string()
    }

    /// X11 has no owner the classic windows can use: the hidden roots are
    /// never realized, so there is nothing to own a group (D182). The group's
    /// z-order comes from `raise_no_activate` alone.
    fn owner_of(&self, _w: NativeWindow) -> NativeWindow {
        NativeWindow::NONE
    }

    fn set_owner(&self, _w: NativeWindow, _owner: NativeWindow) -> NativeWindow {
        NativeWindow::NONE
    }

    /// `XRaiseWindow`. Mutter honours it from the app that has the focus,
    /// which is the app the person just clicked, so raising each member on a
    /// click brings the whole group up together (D182).
    fn raise_no_activate(&self, w: NativeWindow) {
        if w.is_none() {
            return;
        }
        with_x(|x, d| unsafe {
            (x.XRaiseWindow)(d, w.0 as c_ulong);
            (x.XFlush)(d);
        });
    }

    /// Root coordinates in X's own pixels, which under XWayland are the
    /// physical pixels the bond math is in.
    fn cursor_pos(&self) -> (i32, i32) {
        with_x(|x, d| {
            let (mut root_ret, mut child) = (0, 0);
            let (mut rx, mut ry, mut wx, mut wy, mut mask) = (0, 0, 0, 0, 0);
            unsafe {
                let root = (x.XDefaultRootWindow)(d);
                (x.XQueryPointer)(
                    d,
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
        .unwrap_or((0, 0))
    }

    /// D61 as EWMH has it: ask the window manager to add or remove
    /// `_NET_WM_STATE_ABOVE`, a client message to the root.
    fn set_topmost(&self, w: NativeWindow, on: bool) {
        if w.is_none() {
            return;
        }
        with_x(|x, d| unsafe {
            let root = (x.XDefaultRootWindow)(d);
            let mut ev: xlib::XEvent = std::mem::zeroed();
            ev.client_message.type_ = xlib::ClientMessage;
            ev.client_message.window = w.0 as c_ulong;
            ev.client_message.message_type = atom(x, d, "_NET_WM_STATE");
            ev.client_message.format = 32;
            ev.client_message.data.set_long(0, on as c_long);
            ev.client_message
                .data
                .set_long(1, atom(x, d, "_NET_WM_STATE_ABOVE") as c_long);
            ev.client_message.data.set_long(2, 0);
            // Source indication 1: a normal application.
            ev.client_message.data.set_long(3, 1);
            (x.XSendEvent)(
                d,
                root,
                0,
                xlib::SubstructureRedirectMask | xlib::SubstructureNotifyMask,
                &mut ev,
            );
            (x.XFlush)(d);
        });
    }

    /// ICCCM: `WM_STATE` is IconicState (3). Not the map state: Mutter keeps
    /// a minimised X window mapped.
    fn is_minimized(&self, w: NativeWindow) -> bool {
        if w.is_none() {
            return false;
        }
        with_x(|x, d| {
            let ty = atom(x, d, "WM_STATE");
            property(x, d, w.0 as c_ulong, "WM_STATE", ty).and_then(|b| first_long(&b)) == Some(3)
        })
        .unwrap_or(false)
    }

    /// ICCCM: mapping an iconic window asks for NormalState.
    fn restore_no_activate(&self, w: NativeWindow) {
        if w.is_none() {
            return;
        }
        with_x(|x, d| unsafe {
            (x.XMapWindow)(d, w.0 as c_ulong);
            (x.XFlush)(d);
        });
    }

    fn show_minimized_no_activate(&self, w: NativeWindow) {
        StubPlatform.show_minimized_no_activate(w)
    }

    /// X's own answer: a window GTK has shown is not placed until Mutter has
    /// mapped it, and until then tao reports the position it last asked for.
    fn placed(&self, w: NativeWindow) -> Option<(i32, i32, u32, u32)> {
        if w.is_none() {
            return None;
        }
        with_x(|x, d| unsafe {
            let mut attrs: xlib::XWindowAttributes = std::mem::zeroed();
            if (x.XGetWindowAttributes)(d, w.0 as c_ulong, &mut attrs) == 0
                || attrs.map_state != xlib::IsViewable
            {
                return None;
            }
            let (mut rx, mut ry, mut child) = (0, 0, 0);
            (x.XTranslateCoordinates)(
                d,
                w.0 as c_ulong,
                (x.XDefaultRootWindow)(d),
                0,
                0,
                &mut rx,
                &mut ry,
                &mut child,
            );
            Some((rx, ry, attrs.width as u32, attrs.height as u32))
        })
        .flatten()
    }

    /// Mutter keeps a client-placed window wholly inside the work area, one
    /// window at a time (D182).
    fn confines_to_work_area(&self) -> bool {
        true
    }

    /// Mutter will not minimise a window that skips the taskbar (D182).
    fn minimises_taskbarless(&self) -> bool {
        false
    }

    /// WebKitGTK's GStreamer cannot play from the asset protocol (D184).
    fn media_over_loopback(&self) -> bool {
        true
    }

    /// X has one scale for the whole screen (D182).
    fn one_scale(&self) -> bool {
        true
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
