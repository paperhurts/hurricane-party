//! The Windows implementation of [`WindowPlatform`]. Six calls, D44 + D57.
//!
//! Every `unsafe` block in the window engine is in this file. The folder
//! watch (#111) is in `tree`, beside it.

use super::{DiskSpace, NativeWindow, TreeEvent, TreeWatch, Volume, WindowPlatform};
use windows::Win32::Foundation::HWND;
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, GetAwarenessFromDpiAwarenessContext,
    GetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    DPI_AWARENESS_PER_MONITOR_AWARE, DPI_AWARENESS_SYSTEM_AWARE, DPI_AWARENESS_UNAWARE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongPtrW, IsIconic, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    GWLP_HWNDPARENT, HWND_NOTOPMOST, HWND_TOP, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SW_SHOWNOACTIVATE,
};

mod tree;

pub struct Win32Platform;

/// A volume's serial number as eight hex digits, from its NUL-terminated
/// mount (`E:\`). `None` for a volume that will not say, or says zero, which
/// is no id at all.
fn serial(mount: &[u16]) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;
    let mut serial = 0u32;
    // SAFETY: a NUL-terminated mount that outlives the call, and one stack
    // u32 it writes into; every other out-parameter is declined.
    unsafe {
        GetVolumeInformationW(
            PCWSTR(mount.as_ptr()),
            None,
            Some(&mut serial),
            None,
            None,
            None,
        )
    }
    .ok()?;
    (serial != 0).then(|| format!("{serial:08X}"))
}

/// Run `f` with Windows' "there is no disk in the drive" box off for this
/// thread. Asking an empty card reader for its volume puts one on the screen
/// otherwise, in front of whatever the person was doing.
fn quietly<T>(f: impl FnOnce() -> T) -> T {
    use windows::Win32::System::Diagnostics::Debug::{
        SetThreadErrorMode, SEM_FAILCRITICALERRORS, THREAD_ERROR_MODE,
    };
    let mut was = THREAD_ERROR_MODE(0);
    // SAFETY: this thread's own error mode, and a stack value the old one is
    // written into.
    let set = unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, Some(&mut was)) }.is_ok();
    let out = f();
    if set {
        // SAFETY: as above, putting back what was there.
        unsafe {
            let _ = SetThreadErrorMode(was, None);
        }
    }
    out
}

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

    fn cursor_pos(&self) -> (i32, i32) {
        let mut p = POINT::default();
        // SAFETY: writes into a stack POINT. Takes no handle, sends no message.
        // Returns virtual-desktop physical pixels because the process is
        // per-monitor-v2 aware, which D37 asserts at startup.
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        (p.x, p.y)
    }

    fn set_topmost(&self, w: NativeWindow, on: bool) {
        // SAFETY: NOMOVE|NOSIZE means the four zeros are ignored, and
        // NOACTIVATE keeps the mini-player from stealing focus the moment it
        // floats.
        unsafe {
            let _ = SetWindowPos(
                hwnd(w),
                Some(if on { HWND_TOPMOST } else { HWND_NOTOPMOST }),
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

    fn kill_tree(&self, pid: u32) {
        use std::os::windows::process::CommandExt;
        // `taskkill /T` walks the tree the way the process table records it,
        // which is the thing `CommandChild::kill` does not do. No window: a
        // console flashing up on every Pause would be a bug of its own.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    fn open_folder(&self, path: &std::path::Path) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::{w, PCWSTR};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        if !path.is_dir() {
            return Err(format!("{} is not a folder", path.display()));
        }
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: both strings are NUL-terminated and outlive the call; no
        // window handle is passed, so no message is sent to another thread.
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(wide.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        // Anything above 32 is success; the rest are the shell's error codes.
        if result.0 as isize > 32 {
            Ok(())
        } else {
            Err(format!(
                "Windows would not open {} (error {})",
                path.display(),
                result.0 as isize
            ))
        }
    }

    fn watch_tree(
        &self,
        root: &std::path::Path,
        on_event: Box<dyn FnMut(TreeEvent) + Send>,
    ) -> Result<TreeWatch, String> {
        tree::watch(root, on_event)
    }

    fn in_use(&self, path: &std::path::Path) -> bool {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Foundation::ERROR_SHARING_VIOLATION;
        // Share mode 0 asks for the file to ourselves, which Windows refuses
        // while anyone else has it open: a copy still writing, an editor
        // saving. Read access, because an open for attributes alone is never
        // refused on sharing. Anything else that fails the open (gone, no
        // permission) is not "in use": the caller finds that out itself.
        match std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
        {
            Ok(_) => false,
            Err(e) => e.raw_os_error() == Some(ERROR_SHARING_VIOLATION.0 as i32),
        }
    }

    fn disk_space(&self, path: &std::path::Path) -> Option<DiskSpace> {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        if !path.is_dir() {
            return None;
        }
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let (mut free, mut total) = (0u64, 0u64);
        // SAFETY: a NUL-terminated path that outlives the call, and two
        // stack u64s it writes into. Any folder on the drive will do: the
        // call answers for the volume it is on, a mounted folder included.
        unsafe {
            GetDiskFreeSpaceExW(
                PCWSTR(wide.as_ptr()),
                Some(&mut free),
                Some(&mut total),
                None,
            )
        }
        .ok()?;
        (total > 0).then_some(DiskSpace { free, total })
    }

    fn volume_of(&self, path: &std::path::Path) -> Option<Volume> {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::GetVolumePathNameW;
        if !path.is_dir() {
            return None;
        }
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // A mount is never longer than the path it holds.
        let mut mount = vec![0u16; wide.len().max(261)];
        // SAFETY: a NUL-terminated path that outlives the call, and a buffer
        // the call is told the length of by the slice.
        unsafe { GetVolumePathNameW(PCWSTR(wide.as_ptr()), &mut mount) }.ok()?;
        let len = mount.iter().position(|c| *c == 0)?;
        mount.truncate(len + 1);
        let id = quietly(|| serial(&mount))?;
        Some(Volume {
            id,
            mount: std::ffi::OsString::from_wide(&mount[..len]).into(),
        })
    }

    fn mounts_of(&self, id: &str) -> Vec<std::path::PathBuf> {
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
        // DRIVE_REMOVABLE and DRIVE_FIXED. A USB hard drive says fixed.
        // They live in a feature of the crate this app is not built with,
        // for two numbers.
        const REMOVABLE: u32 = 2;
        const FIXED: u32 = 3;
        // SAFETY: no arguments; a bit per drive letter in use.
        let letters = unsafe { GetLogicalDrives() };
        quietly(|| {
            (0..26u8)
                .filter(|i| letters & (1 << i) != 0)
                .filter_map(|i| {
                    let root = format!("{}:\\", (b'A' + i) as char);
                    let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
                    // SAFETY: a NUL-terminated root that outlives the call.
                    let kind = unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) };
                    if kind != REMOVABLE && kind != FIXED {
                        return None;
                    }
                    (serial(&wide)? == id).then(|| root.into())
                })
                .collect()
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    // Only the refusal is testable without a window opening on the desk.
    #[test]
    fn a_path_that_is_not_a_folder_is_refused_before_the_shell_sees_it() {
        let here = std::env::temp_dir().join("hp-no-such-folder-155");
        let why = Win32Platform.open_folder(&here).unwrap_err();
        assert!(why.contains("not a folder"), "{why}");
        let file = std::env::current_exe().unwrap();
        assert!(Win32Platform.open_folder(&file).is_err());
    }

    /// #162: a real folder has a drive with room on it, and a folder that
    /// is not there has none to report.
    #[test]
    fn a_folder_reports_its_drive_and_a_missing_one_reports_nothing() {
        let here = Win32Platform.disk_space(&std::env::temp_dir()).unwrap();
        assert!(here.total > 0 && here.free <= here.total, "{here:?}");
        let gone = std::env::temp_dir().join("hp-no-such-folder-162");
        let _ = std::fs::remove_dir_all(&gone);
        assert_eq!(Win32Platform.disk_space(&gone), None);
    }

    /// #111: a file another handle still has open is in use; once it is
    /// closed it is not; a file that is not there is not in use either.
    #[test]
    fn a_file_still_open_for_writing_is_in_use() {
        let dir = std::env::temp_dir().join("hp-in-use-111");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("copying.mp3");
        let writer = std::fs::File::create(&path).unwrap();
        assert!(Win32Platform.in_use(&path));
        drop(writer);
        assert!(!Win32Platform.in_use(&path));
        assert!(!Win32Platform.in_use(&dir.join("never.mp3")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// D143, on the real calls: a folder knows its drive and where the drive
    /// is mounted, and looking for that drive by its id finds the same mount.
    /// A folder that is not there has no drive, and an id no drive carries is
    /// mounted nowhere.
    #[test]
    fn a_folder_names_its_drive_and_the_drive_is_found_by_that_name() {
        let dir = std::env::temp_dir().join("hp-volume-143");
        std::fs::create_dir_all(&dir).unwrap();
        let v = Win32Platform.volume_of(&dir).unwrap();
        assert_eq!(v.id.len(), 8, "{v:?}");
        assert!(dir.starts_with(&v.mount), "{v:?}");
        assert!(Win32Platform.mounts_of(&v.id).contains(&v.mount), "{v:?}");
        assert!(Win32Platform.mounts_of("no drive").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Win32Platform.volume_of(&dir), None);
    }
}
