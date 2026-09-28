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
//! - `WS_EX_TOPMOST`: he stays in front, as #192 asks.

use crate::brain::Hand;
use crate::perch::Rect;
use crate::sprite::Bgra;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use windows::core::{w, BOOL};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
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
    GetWindowRect, LoadCursorW, SetCursor, IDC_HAND, WM_CAPTURECHANGED, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_SETCURSOR,
};

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
            Ok(Surface { hwnd, shown: false })
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
