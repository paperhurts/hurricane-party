//! His window on Windows: a `WS_EX_LAYERED` popup drawn with
//! `UpdateLayeredWindow` from a premultiplied 32-bit DIB, so his outline is his
//! own pixels and not a rectangle.
//!
//! - `WS_EX_NOACTIVATE`, `SW_SHOWNOACTIVATE` and `MA_NOACTIVATE`: he never
//!   takes the keyboard and never comes up in front of what someone is doing,
//!   not even when he is clicked or carried (#191, #192).
//! - **Clickable only where he is.** A layered window drawn with per-pixel
//!   alpha is hit-tested by that alpha: a pixel with alpha 0 lets the mouse
//!   through to whatever is under it. So there is no `WS_EX_TRANSPARENT`: his
//!   outline catches the pointer, for petting and carrying (D156), and the
//!   empty corners of his cell do not.
//! - A press captures the mouse until it is released, so a drag keeps going
//!   when the pointer runs ahead of him, which it does between two frames.
//! - `WS_EX_TOOLWINDOW`: no taskbar button and no Alt+Tab entry.
//! - `WS_EX_TOPMOST`: he stays in front, as #192 asks and the owner kept on
//!   #210 (D166). So another app in front of the window he stands on would
//!   leave him floating on it; `look` sees that, and he falls to the floor.

use crate::brain::{Hand, Seen, Spot};
use crate::perch::Rect;
use crate::sprite::Bgra;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::time::{Duration, Instant};
use windows::core::{w, BOOL};
use windows::Win32::Foundation::{
    COLORREF, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC,
    GetMonitorInfoW, ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetMessageW, KillTimer, RegisterClassW, SetTimer,
    SetWindowPos, ShowWindow, UpdateLayeredWindow, HWND_TOPMOST, MA_NOACTIVATE, MSG,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA,
    WM_MOUSEACTIVATE, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetAncestor, GetWindowLongW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, WindowFromPoint, GA_ROOT, GWL_EXSTYLE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, LoadCursorW, SetCursor, IDC_HAND, WM_CAPTURECHANGED, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_SETCURSOR,
};

/// A player's window not found for his perch is looked for again this often:
/// a layout can be a moment behind the screen.
const LOOK_AGAIN: Duration = Duration::from_secs(1);
/// How far (physical pixels, each edge) a window's own rectangle may be from
/// the layout's and still be the one it describes: rounding, no more.
const SLACK: i32 = 4;

thread_local! {
    /// The pointer on him since the last tick. His window and the loop that
    /// drains this share one thread, so a queue here is all it takes.
    static HANDS: RefCell<Vec<Hand>> = const { RefCell::new(Vec::new()) };
    /// A press is down on him, and he holds the mouse capture for it.
    static PRESSED: Cell<bool> = const { Cell::new(false) };
}

/// A mouse message's position, in screen pixels. The coordinates are his
/// client area's, which for a borderless popup starts at its window's corner;
/// while he holds the capture they run past his edges, negative included.
fn screen_point(h: HWND, lp: LPARAM) -> (i32, i32) {
    let (cx, cy) = (
        (lp.0 & 0xFFFF) as i16 as i32,
        ((lp.0 >> 16) & 0xFFFF) as i16 as i32,
    );
    let mut r = RECT::default();
    // SAFETY: a query about his own window.
    let _ = unsafe { GetWindowRect(h, &mut r) };
    (r.left + cx, r.top + cy)
}

fn heard(hand: Hand) {
    HANDS.with(|q| q.borrow_mut().push(hand));
}

/// Physical pixels everywhere, like the player (D37): the pipe's layout is in
/// physical pixels, and so are his window's position and size.
pub fn init() {
    // SAFETY: a process-wide setting, called once before any window exists.
    // It fails only when a manifest already set it, which is the same answer.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// Whether this is the only Cap'n: a named mutex for the session, held for
/// the life of the process (the handle is never closed; Windows lets go of
/// it when he exits). A second one started by the switch or by hand sees it
/// already there and leaves.
pub fn only_one() -> bool {
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    // SAFETY: creates or opens a named kernel object; the name is a constant.
    unsafe {
        match CreateMutexW(None, false, w!("Local\\hurricane-party-companion")) {
            Ok(_held) => GetLastError() != ERROR_ALREADY_EXISTS,
            Err(_) => true, // cannot tell: better one extra than none
        }
    }
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        // Belt and braces with WS_EX_NOACTIVATE: a click never activates him.
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_SETCURSOR => {
            // A hand over him: he is something to pet and pick up.
            // SAFETY: a stock cursor, set for the pointer over his window.
            if let Ok(hand) = unsafe { LoadCursorW(None, IDC_HAND) } {
                unsafe { SetCursor(Some(hand)) };
            }
            return LRESULT(1);
        }
        WM_LBUTTONDOWN => {
            PRESSED.set(true);
            // SAFETY: capture for his own window while the button is down.
            unsafe { SetCapture(h) };
            let (x, y) = screen_point(h, lp);
            heard(Hand::Down(x, y));
            return LRESULT(0);
        }
        WM_MOUSEMOVE if PRESSED.get() => {
            let (x, y) = screen_point(h, lp);
            heard(Hand::Move(x, y));
            return LRESULT(0);
        }
        WM_LBUTTONUP if PRESSED.get() => {
            // Cleared first, so the WM_CAPTURECHANGED that ReleaseCapture
            // sends is not taken for someone else taking the mouse.
            PRESSED.set(false);
            let (x, y) = screen_point(h, lp);
            heard(Hand::Up(x, y));
            // SAFETY: gives back the capture this window took on the press.
            let _ = unsafe { ReleaseCapture() };
            return LRESULT(0);
        }
        WM_CAPTURECHANGED if PRESSED.get() => {
            PRESSED.set(false);
            heard(Hand::Cancel);
            return LRESULT(0);
        }
        _ => {}
    }
    // SAFETY: the default handling for his own window's messages.
    unsafe { DefWindowProcW(h, msg, wp, lp) }
}

pub struct Surface {
    hwnd: HWND,
    shown: bool,
    /// The player's process: its windows are the ones he stands on.
    player: Option<u32>,
    /// The player's window found for his perch, kept while he is on it.
    perch: Option<Found>,
}

/// The window found for a layout id and rectangle, or none, and when.
struct Found {
    id: String,
    rect: (i32, i32, i32, i32),
    hwnd: Option<HWND>,
    at: Instant,
}

impl Surface {
    pub fn new() -> Result<Surface, String> {
        // SAFETY: registers a class for this process and creates one hidden
        // popup from it on this thread, which then runs its message loop.
        unsafe {
            let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: module.into(),
                lpszClassName: w!("hp-companion"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("hp-companion"),
                w!("Cap'n Capy"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(module.into()),
                None,
            )
            .map_err(|e| e.to_string())?;
            Ok(Surface {
                hwnd,
                shown: false,
                player: None,
                perch: None,
            })
        }
    }

    /// Draw `img` with its top-left at (`x`, `y`), and show the window the
    /// first time without activating it.
    pub fn present(&mut self, img: &Bgra, x: i32, y: i32) -> Result<(), String> {
        // SAFETY: a screen DC and a memory DC with a DIB section of exactly
        // img's size; the pixels are copied in, handed to the layered window,
        // and every GDI object is released before returning.
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: img.w as i32,
                    biHeight: -(img.h as i32), // top-down, as Bgra is
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            let result =
                match CreateDIBSection(Some(mem), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                    Ok(dib) if !bits.is_null() => {
                        std::ptr::copy_nonoverlapping(
                            img.px.as_ptr(),
                            bits as *mut u8,
                            img.px.len(),
                        );
                        let old = SelectObject(mem, dib.into());
                        let blend = BLENDFUNCTION {
                            BlendOp: AC_SRC_OVER as u8,
                            BlendFlags: 0,
                            SourceConstantAlpha: 255,
                            AlphaFormat: AC_SRC_ALPHA as u8,
                        };
                        let at = POINT { x, y };
                        let size = SIZE {
                            cx: img.w as i32,
                            cy: img.h as i32,
                        };
                        let from = POINT { x: 0, y: 0 };
                        let r = UpdateLayeredWindow(
                            self.hwnd,
                            Some(screen),
                            Some(&at),
                            Some(&size),
                            Some(mem),
                            Some(&from),
                            COLORREF(0),
                            Some(&blend),
                            ULW_ALPHA,
                        )
                        .map_err(|e| e.to_string());
                        SelectObject(mem, old);
                        let _ = DeleteObject(dib.into());
                        r
                    }
                    Ok(_) => Err("the DIB came back without pixels".to_string()),
                    Err(e) => Err(e.to_string()),
                };
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            result?;
            if !self.shown {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                let _ = SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                self.shown = true;
            }
        }
        Ok(())
    }

    pub fn hide(&mut self) {
        if self.shown {
            // SAFETY: hides his own window; no activation change.
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.shown = false;
        }
    }

    /// The pointer on him since the last call.
    pub fn hands(&mut self) -> Vec<Hand> {
        HANDS.with(|q| std::mem::take(&mut *q.borrow_mut()))
    }

    /// The player's process, from the pipe. A new one is a new player, with
    /// new windows.
    pub fn player(&mut self, pid: u32) {
        if self.player != Some(pid) {
            self.player = Some(pid);
            self.perch = None;
        }
    }

    /// What the OS says about the player's window he is on (D166): whether
    /// it is minimised, and whether another window is in front of it at the
    /// spot under his feet. Nothing, when that window cannot be found.
    ///
    /// The top-level window at that spot, by hit-testing as a click would, is
    /// in front of his unless it is his, or him, or always on top itself: a
    /// menu, a tooltip, the shell's flyouts, the player's own mini-player
    /// (D61). Those are his layer, not something the player went behind.
    pub fn look(&mut self, spot: &Spot) -> Seen {
        let Some(perch) = self.perch_window(spot) else {
            return Seen::default();
        };
        // SAFETY: queries about windows by handle and about a point on the
        // screen; a handle that has gone answers false, zero or null.
        unsafe {
            if IsIconic(perch).as_bool() {
                return Seen {
                    covered: false,
                    minimised: true,
                };
            }
            let (x, y) = spot.at;
            let top = GetAncestor(WindowFromPoint(POINT { x, y }), GA_ROOT);
            let pinned = GetWindowLongW(top, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0;
            Seen {
                covered: !top.is_invalid() && top != perch && top != self.hwnd && !pinned,
                minimised: false,
            }
        }
    }

    /// The player's window for `spot`, found once while he is on it and kept.
    /// A minimised window has left its rectangle for -32000, so it can only
    /// be found while it is up, which is when he lands on it.
    fn perch_window(&mut self, spot: &Spot) -> Option<HWND> {
        if let Some(f) = &self.perch {
            if f.id == spot.id && f.rect == spot.rect {
                match f.hwnd {
                    // SAFETY: asks whether a handle found earlier is still a window.
                    Some(h) if unsafe { IsWindow(Some(h)) }.as_bool() => return Some(h),
                    None if f.at.elapsed() < LOOK_AGAIN => return None,
                    _ => {}
                }
            }
        }
        let hwnd = self.player.and_then(|pid| player_window(pid, spot.rect));
        self.perch = Some(Found {
            id: spot.id.to_string(),
            rect: spot.rect,
            hwnd,
            at: Instant::now(),
        });
        hwnd
    }
}

/// The player's top-level window that is up at `rect`, the layout's
/// rectangle for it, or the nearest within `SLACK` on every edge. Matched in
/// the player's own process only, so a maximised library is never mistaken
/// for another app maximised beside it.
fn player_window(pid: u32, (x, y, w, h): (i32, i32, i32, i32)) -> Option<HWND> {
    struct Hunt {
        pid: u32,
        want: [i32; 4],
        best: Option<(i32, HWND)>,
    }
    unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the &mut Hunt passed below, alive for the call.
        let hunt = unsafe { &mut *(data.0 as *mut Hunt) };
        let mut pid = 0u32;
        let mut r = RECT::default();
        // SAFETY: queries about a window the enumeration just gave us.
        let up = unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32));
            pid == hunt.pid
                && IsWindowVisible(hwnd).as_bool()
                && !IsIconic(hwnd).as_bool()
                && GetWindowRect(hwnd, &mut r).is_ok()
        };
        if up {
            let [x0, y0, x1, y1] = hunt.want;
            let off = [r.left - x0, r.top - y0, r.right - x1, r.bottom - y1].map(i32::abs);
            let total: i32 = off.iter().sum();
            if off.iter().all(|d| *d <= SLACK) && hunt.best.is_none_or(|(b, _)| total < b) {
                hunt.best = Some((total, hwnd));
            }
        }
        BOOL(1)
    }
    let mut hunt = Hunt {
        pid,
        want: [x, y, x + w, y + h],
        best: None,
    };
    // SAFETY: synchronous enumeration; the callback only writes into `hunt`.
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut hunt as *mut _ as isize));
    }
    hunt.best.map(|(_, hwnd)| hwnd)
}

/// The player's process id, from its end of the control pipe.
pub fn pipe_server(pipe: &std::fs::File) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
    let mut pid = 0u32;
    // SAFETY: a query on a pipe handle the caller holds open for the call.
    unsafe { GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) }.ok()?;
    (pid != 0).then_some(pid)
}

/// Each monitor's work area (the monitor minus its taskbar), physical pixels.
pub fn work_areas() -> Vec<Rect> {
    unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the &mut Vec passed below, alive for the call.
        let out = unsafe { &mut *(data.0 as *mut Vec<Rect>) };
        let mut mi = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: a query about a monitor handle the enumeration just gave us.
        if unsafe { GetMonitorInfoW(m, &mut mi) }.as_bool() {
            let r = mi.rcWork;
            out.push(Rect {
                x: r.left,
                y: r.top,
                w: r.right - r.left,
                h: r.bottom - r.top,
            });
        }
        BOOL(1)
    }
    let mut out: Vec<Rect> = Vec::new();
    // SAFETY: synchronous enumeration; the callback only writes into `out`.
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// The message loop, with `tick` called every `interval_ms` from a thread timer.
pub fn run(interval_ms: u32, mut tick: impl FnMut()) {
    // SAFETY: a thread timer and a plain GetMessage loop on the thread that
    // owns his window; WM_TIMER for our timer is handled here and not
    // dispatched, everything else goes to the window procedure.
    unsafe {
        let id = SetTimer(None, 0, interval_ms, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            if msg.message == WM_TIMER && msg.wParam.0 == id {
                tick();
                continue;
            }
            let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
            windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
        }
        let _ = KillTimer(None, id);
    }
}

/// The player's request to leave (D161): a manual-reset named event he
/// creates at start and checks every tick. Created cleared and reset at once,
/// so a signal left over from an earlier Cap'n never sends this one home.
/// The handle is never closed; Windows destroys the event when the last
/// Cap'n exits, which is how the player knows there is no one to ask.
pub struct Leave(Option<windows::Win32::Foundation::HANDLE>);

impl Leave {
    pub fn new() -> Leave {
        Leave::named(hp_control::COMPANION_LEAVE_EVENT)
    }

    fn named(name: &str) -> Leave {
        use windows::core::HSTRING;
        use windows::Win32::System::Threading::{CreateEventW, ResetEvent};
        // SAFETY: creates, or opens if a stale one survives, a named event
        // this process then owns a handle to for its whole life.
        unsafe {
            match CreateEventW(None, true, false, &HSTRING::from(name)) {
                Ok(h) => {
                    let _ = ResetEvent(h);
                    Leave(Some(h))
                }
                Err(_) => Leave(None),
            }
        }
    }

    /// Whether the player has asked him to go.
    pub fn asked(&self) -> bool {
        use windows::Win32::Foundation::WAIT_OBJECT_0;
        use windows::Win32::System::Threading::WaitForSingleObject;
        match self.0 {
            // SAFETY: a zero-timeout wait on our own event handle.
            Some(h) => (unsafe { WaitForSingleObject(h, 0) }) == WAIT_OBJECT_0,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Leave;

    /// What the player's box does (src-tauri's ask_companion_to_leave).
    fn ask(name: &str) {
        use windows::core::HSTRING;
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
        unsafe {
            let h = OpenEventW(EVENT_MODIFY_STATE, false, &HSTRING::from(name)).unwrap();
            SetEvent(h).unwrap();
            let _ = CloseHandle(h);
        }
    }

    #[test]
    fn he_hears_the_player_ask_and_a_new_cap_n_starts_clear() {
        // A name of this test's own, so it never sends off a real Cap'n.
        let name = format!(r"Local\hp-companion-test-leave-{}", std::process::id());
        let leave = Leave::named(&name);
        assert!(!leave.asked(), "nobody has asked yet");
        ask(&name);
        assert!(leave.asked(), "the box was unticked");
        assert!(leave.asked(), "and it stays asked: he is on his way out");
        let next = Leave::named(&name);
        assert!(
            !next.asked(),
            "a Cap'n starting later clears a stale request"
        );
    }

    use super::{player_window, Surface, SLACK};
    use crate::brain::{Seen, Spot, UNDERFOOT};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{DestroyWindow, SW_SHOWMINNOACTIVE};

    unsafe extern "system" fn plain(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe { super::DefWindowProcW(h, msg, wp, lp) }
    }

    /// A window of this test's own standing in for one of the player's: shown
    /// without activation, off every screen, and never drawn, so nothing
    /// appears on the desktop the test runs on.
    fn a_window((x, y, w, h): (i32, i32, i32, i32)) -> HWND {
        use super::*;
        unsafe {
            let module = GetModuleHandleW(None).unwrap();
            let class = WNDCLASSW {
                lpfnWndProc: Some(plain),
                hInstance: module.into(),
                lpszClassName: w!("hp-companion-test"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("hp-companion-test"),
                w!("test"),
                WS_POPUP,
                x,
                y,
                w,
                h,
                None,
                None,
                Some(module.into()),
                None,
            )
            .unwrap();
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            hwnd
        }
    }

    #[test]
    fn the_players_window_is_found_by_its_rectangle_and_seen_minimised() {
        let pid = std::process::id();
        let rect = (-20000, -20000, 300, 100);
        let hwnd = a_window(rect);
        let (x, y, w, h) = rect;
        assert_eq!(player_window(pid, rect), Some(hwnd));
        assert_eq!(player_window(pid, (x + SLACK, y, w, h)), Some(hwnd));
        assert_eq!(
            player_window(pid, (x, y, w + 40, h)),
            None,
            "another window"
        );
        assert_eq!(player_window(pid + 1, rect), None, "another process");

        let mut surface = Surface::new().unwrap();
        surface.player(pid);
        let spot = Spot {
            id: "library",
            rect,
            at: (x + 10, y + UNDERFOOT),
        };
        assert!(!surface.look(&spot).minimised, "up");
        unsafe {
            let _ = super::ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
        }
        assert_eq!(
            surface.look(&spot),
            Seen {
                covered: false,
                minimised: true
            },
            "found while it was up, so known when it is down"
        );
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        assert_eq!(surface.look(&spot), Seen::default(), "gone: nothing to say");
    }
}
