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
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_long, c_uchar, c_ulong};
use std::ptr;
use std::sync::Mutex;
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

/// What each window was last given as its owner, for `owner_of`. X keeps
/// `WM_TRANSIENT_FOR`, but reading it back is a round trip for nothing.
static OWNERS: Mutex<Option<HashMap<isize, isize>>> = Mutex::new(None);

/// How stage 2 groups the z-order, chosen at launch by `SPIKE_Z`:
/// `raise` (the default) raises a group's windows together and sets no owner;
/// `transient` also sets `WM_TRANSIENT_FOR` to the window the engine names as
/// owner, or to GDK's client leader when that window has no X window (the
/// hidden roots are never mapped, so GTK never creates one).
fn z_mode() -> &'static str {
    static MODE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("SPIKE_Z").unwrap_or_else(|_| "raise".into()))
}

fn with_x<R>(f: impl FnOnce(&xlib::Xlib, *mut xlib::Display) -> R) -> Option<R> {
    X.with(|x| x.as_ref().map(|(x, d)| f(x, *d)))
}

fn atom(x: &xlib::Xlib, d: *mut xlib::Display, name: &str) -> c_ulong {
    let n = CString::new(name).unwrap();
    unsafe { (x.XInternAtom)(d, n.as_ptr(), 0) }
}

/// A property as bytes, or None.
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
            d, w, prop, 0, 1024, 0, ty, &mut actual, &mut format, &mut n, &mut after, &mut data,
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

/// The X window of this process titled `title`, searched from the root down:
/// `handle_of` without touching GTK, which is not safe off the main thread.
/// None when there is none, which is every window GTK has not realized.
pub fn find_window(title: &str) -> Option<c_ulong> {
    let pid = std::process::id() as c_long;
    with_x(|x, d| {
        let utf8 = atom(x, d, "UTF8_STRING");
        let mut stack = vec![unsafe { (x.XDefaultRootWindow)(d) }];
        while let Some(w) = stack.pop() {
            let name = property(x, d, w, "_NET_WM_NAME", utf8);
            if name.as_deref() == Some(title.as_bytes())
                && property(x, d, w, "_NET_WM_PID", xlib::XA_CARDINAL)
                    .and_then(|b| first_long(&b))
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

/// GDK's client leader for this process: an unmapped window every toplevel
/// names in `WM_CLIENT_LEADER`.
fn leader(x: &xlib::Xlib, d: *mut xlib::Display, w: c_ulong) -> Option<c_ulong> {
    property(x, d, w, "WM_CLIENT_LEADER", xlib::XA_WINDOW)
        .and_then(|b| first_long(&b))
        .map(|l| l as c_ulong)
}

impl WindowPlatform for X11Platform {
    fn assert_dpi_aware(&self) -> String {
        "n/a (X11: one scale for the whole screen)".to_string()
    }

    fn owner_of(&self, w: NativeWindow) -> NativeWindow {
        let owners = OWNERS.lock().unwrap();
        NativeWindow(owners.as_ref().and_then(|o| o.get(&w.0).copied()).unwrap_or(0))
    }

    fn set_owner(&self, w: NativeWindow, owner: NativeWindow) -> NativeWindow {
        if w.is_none() {
            return NativeWindow::NONE;
        }
        let prev = {
            let mut owners = OWNERS.lock().unwrap();
            owners.get_or_insert_with(HashMap::new).insert(w.0, owner.0)
        };
        if z_mode() == "transient" {
            with_x(|x, d| unsafe {
                let target = if owner.is_none() {
                    leader(x, d, w.0 as c_ulong).unwrap_or(0)
                } else {
                    owner.0 as c_ulong
                };
                if target == 0 {
                    (x.XDeleteProperty)(d, w.0 as c_ulong, atom(x, d, "WM_TRANSIENT_FOR"));
                } else {
                    (x.XSetTransientForHint)(d, w.0 as c_ulong, target);
                }
                (x.XFlush)(d);
            });
        }
        NativeWindow(prev.unwrap_or(0))
    }

    /// `XRaiseWindow`, which a window manager may or may not honour from a
    /// client: that is what stage 2 measures.
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
            ev.client_message.data.set_long(1, atom(x, d, "_NET_WM_STATE_ABOVE") as c_long);
            ev.client_message.data.set_long(2, 0);
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

    /// ICCCM: `WM_STATE` is IconicState (3).
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

    /// ICCCM: mapping an iconic window asks for NormalState. Whether the
    /// window manager also focuses it is the window manager's call.
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
