//! His window on Windows: a `WS_EX_LAYERED` popup drawn with
//! `UpdateLayeredWindow` from a premultiplied 32-bit DIB, so his outline is his
//! own pixels and not a rectangle.
//!
//! - `WS_EX_NOACTIVATE` and `SW_SHOWNOACTIVATE`: he never takes the keyboard
//!   and never comes up in front of what someone is doing (#191, #192).
//! - `WS_EX_TRANSPARENT`: every click goes through him to whatever is under.
//!   Petting and carrying him (a later PR) will make his own pixels clickable.
//! - `WS_EX_TOOLWINDOW`: no taskbar button and no Alt+Tab entry.
//! - `WS_EX_TOPMOST`: he stays in front, as #192 asks.

use crate::perch::Rect;
use crate::sprite::Bgra;
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
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetMessageW, KillTimer, RegisterClassW, SetTimer,
    SetWindowPos, ShowWindow, UpdateLayeredWindow, HWND_TOPMOST, MA_NOACTIVATE, MSG,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA,
    WM_MOUSEACTIVATE, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

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
    if msg == WM_MOUSEACTIVATE {
        // Belt and braces with WS_EX_NOACTIVATE: a click never activates him.
        return LRESULT(MA_NOACTIVATE as isize);
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
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
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
