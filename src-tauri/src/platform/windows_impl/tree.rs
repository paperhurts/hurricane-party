//! Watching a folder on Windows (#111, D137).
//!
//! `ReadDirectoryChangesW` on the folder's handle, overlapped, on a thread of
//! the watch's own. Two things make it more than a loop.
//!
//! **An open handle refuses an eject.** A library root is exactly the folder
//! that lives on a drive a person unplugs (D28), and Windows answers *Eject*
//! with "this device is currently in use" while any handle on the volume is
//! open. So the handle is registered with `CM_Register_Notification`, and the
//! watch closes it and reports `Released` when Windows says the drive is
//! wanted. Measured on a mounted image (2026-09-13), an eject does not ask
//! "may this device be removed" first: it announces a volume lock, and the
//! lock fails while a handle is open. A mounted image is dismounted anyway; a
//! USB drive stops there and says it is in use. So the watch lets go on the
//! lock, on a dismount, on the removal query, and on a removal that has
//! already happened, which leaves a read pending on a volume that is gone.
//!
//! **Stopping is prompt.** Dropping the `TreeWatch` signals the thread, which
//! cancels its read and closes the handle before the drop returns.

use crate::platform::{TreeEvent, TreeWatch};
use std::ffi::{c_void, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Register_Notification, CM_Unregister_Notification, CM_NOTIFY_ACTION,
    CM_NOTIFY_ACTION_DEVICECUSTOMEVENT, CM_NOTIFY_ACTION_DEVICEQUERYREMOVE,
    CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER,
    CM_NOTIFY_FILTER_0, CM_NOTIFY_FILTER_0_1, CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE, CR_SUCCESS,
    HCMNOTIFICATION,
};
use windows::Win32::Foundation::{CloseHandle, ERROR_NOTIFY_ENUM_DIR, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
    FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
    FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Threading::{
    CreateEventW, ResetEvent, SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::UI::WindowsAndMessaging::{GUID_IO_VOLUME_DISMOUNT, GUID_IO_VOLUME_LOCK};

/// How long an eject waits for the watch to let go. The thread is woken
/// straight away and closes one handle, so this is a ceiling for a thread
/// that is stuck, not a delay anyone sees.
const LET_GO_MS: u32 = 5_000;

/// A manual-reset event that can cross threads. `HANDLE` holds a raw pointer
/// and so is not `Send`; an event is safe to signal from any thread.
struct Event(HANDLE);

// SAFETY: an event handle is a kernel object reference, usable from any
// thread, and nothing here closes it before the last owner is dropped.
unsafe impl Send for Event {}
unsafe impl Sync for Event {}

impl Event {
    fn new() -> Result<Event, String> {
        // SAFETY: no name and no security attributes; the handle is owned by
        // the returned value and closed when it drops.
        unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
            .map(Event)
            .map_err(|e| format!("couldn't make an event: {e}"))
    }

    fn set(&self) {
        // SAFETY: a live event handle, owned by self.
        unsafe {
            let _ = SetEvent(self.0);
        }
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: owned, and closed exactly once, here.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// What the watch thread, the `TreeWatch` guard and the device callback share.
struct Signals {
    /// Set when the `TreeWatch` is dropped: stop for good.
    stop: Event,
    /// Set by the device callback: Windows wants the drive.
    release: Event,
    /// Set by the thread once the folder's handle is closed. The device
    /// callback waits for it before it lets Windows go ahead.
    let_go: Event,
}

/// Held inside the `TreeWatch`. Dropping it stops the thread and waits.
struct Guard {
    signals: Arc<Signals>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.signals.stop.set();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub(super) fn watch(
    root: &Path,
    on_event: Box<dyn FnMut(TreeEvent) + Send>,
) -> Result<TreeWatch, String> {
    let signals = Arc::new(Signals {
        stop: Event::new()?,
        release: Event::new()?,
        let_go: Event::new()?,
    });
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: a NUL-terminated path that outlives the call. BACKUP_SEMANTICS
    // is what lets CreateFile open a folder at all; OVERLAPPED is what lets
    // the thread wait on its read and a stop at once. Every share flag, so
    // the watch never stands in the way of a copy, a rename or a delete.
    let dir = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            None,
        )
    }
    .map_err(|e| format!("can't watch {}: {e}", root.display()))?;

    // The raw value crosses to the thread, which owns the handle from here.
    let raw = dir.0 as usize;
    let thread_signals = signals.clone();
    let thread = std::thread::Builder::new()
        .name("hp-watch-tree".into())
        .spawn(move || run(HANDLE(raw as *mut c_void), thread_signals, on_event));
    match thread {
        Ok(t) => Ok(TreeWatch::new(Box::new(Guard {
            signals,
            thread: Some(t),
        }))),
        Err(e) => {
            // SAFETY: the thread never started, so the handle is still ours.
            unsafe {
                let _ = CloseHandle(dir);
            }
            Err(format!("couldn't start watching {}: {e}", root.display()))
        }
    }
}

/// The watch thread: read changes until told to stop, told to let go, or
/// the folder cannot be read any more.
fn run(dir: HANDLE, signals: Arc<Signals>, mut on_event: Box<dyn FnMut(TreeEvent) + Send>) {
    let notify = register(dir, &signals);
    let filter = FILE_NOTIFY_CHANGE_FILE_NAME
        | FILE_NOTIFY_CHANGE_DIR_NAME
        | FILE_NOTIFY_CHANGE_SIZE
        | FILE_NOTIFY_CHANGE_LAST_WRITE;
    // 64 KB, in u32s: the buffer has to be DWORD-aligned. A burst larger
    // than it arrives as an overflow, and the caller looks at everything.
    let mut buf = vec![0u32; 16 * 1024];
    let mut released = false;

    match Event::new() {
        Err(_) => on_event(TreeEvent::Ended),
        Ok(io) => loop {
            let mut ov = OVERLAPPED {
                hEvent: io.0,
                ..Default::default()
            };
            // SAFETY: the buffer and the OVERLAPPED both live until the read
            // has completed or been cancelled and waited out below.
            let started = unsafe {
                let _ = ResetEvent(io.0);
                ReadDirectoryChangesW(
                    dir,
                    buf.as_mut_ptr().cast(),
                    (buf.len() * 4) as u32,
                    true,
                    filter,
                    None,
                    Some(&mut ov),
                    None,
                )
            };
            if started.is_err() {
                on_event(TreeEvent::Ended);
                break;
            }
            // SAFETY: three live event handles.
            let which = unsafe {
                WaitForMultipleObjects(&[io.0, signals.stop.0, signals.release.0], false, INFINITE)
            };
            if which != WAIT_OBJECT_0 {
                // A stop or a release. Cancel the read and wait until it has
                // finished cancelling, so nothing writes into the buffer
                // after this thread has gone.
                // SAFETY: the OVERLAPPED is the one the read was issued with.
                unsafe {
                    let _ = CancelIoEx(dir, Some(&ov));
                    let mut n = 0u32;
                    let _ = GetOverlappedResult(dir, &ov, &mut n, true);
                }
                released = which.0 == WAIT_OBJECT_0.0 + 2;
                break;
            }
            let mut n = 0u32;
            // SAFETY: the read has completed; this only collects its result.
            match unsafe { GetOverlappedResult(dir, &ov, &mut n, false) } {
                Ok(()) if n == 0 => on_event(TreeEvent::Overflow),
                Ok(()) => {
                    // SAFETY: the system wrote `n` bytes into the buffer.
                    let bytes = unsafe {
                        std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), n as usize)
                    };
                    for rel in changed_paths(bytes) {
                        on_event(TreeEvent::Changed(rel));
                    }
                }
                Err(e) if e.code() == ERROR_NOTIFY_ENUM_DIR.to_hresult() => {
                    on_event(TreeEvent::Overflow)
                }
                Err(_) => {
                    on_event(TreeEvent::Ended);
                    break;
                }
            }
        },
    }

    // SAFETY: the handle this thread was given, closed exactly once.
    unsafe {
        let _ = CloseHandle(dir);
    }
    signals.let_go.set();
    if released {
        on_event(TreeEvent::Released);
    }
    if let Some(h) = notify {
        // Not from the callback's thread, which would deadlock: this waits
        // for a callback in flight to return, and the one waiting on
        // `let_go` just has. After it, no callback can hold `signals`.
        // SAFETY: a registration this thread made.
        unsafe {
            let _ = CM_Unregister_Notification(h);
        }
    }
}

/// Ask to hear when the device under `dir` is about to be removed. A folder
/// on a fixed disk registers too and never hears anything. A registration
/// that fails leaves the watch working and an eject refused, as before.
fn register(dir: HANDLE, signals: &Arc<Signals>) -> Option<HCMNOTIFICATION> {
    let filter = CM_NOTIFY_FILTER {
        cbSize: std::mem::size_of::<CM_NOTIFY_FILTER>() as u32,
        FilterType: CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE,
        u: CM_NOTIFY_FILTER_0 {
            DeviceHandle: CM_NOTIFY_FILTER_0_1 { hTarget: dir },
        },
        ..Default::default()
    };
    let mut h = HCMNOTIFICATION::default();
    // SAFETY: the context is the `Signals` the watch thread keeps alive until
    // it has unregistered, and unregistering waits out any callback.
    let r = unsafe {
        CM_Register_Notification(
            &filter,
            Some(Arc::as_ptr(signals).cast()),
            Some(on_device),
            &mut h,
        )
    };
    (r == CR_SUCCESS).then_some(h)
}

/// Windows says the drive is wanted: let go of it, then answer. Never a veto.
/// The rows stay; the root is missing until the drive is back.
unsafe extern "system" fn on_device(
    _notify: HCMNOTIFICATION,
    context: *const c_void,
    action: CM_NOTIFY_ACTION,
    data: *const CM_NOTIFY_EVENT_DATA,
    _size: u32,
) -> u32 {
    let wanted = if action == CM_NOTIFY_ACTION_DEVICECUSTOMEVENT && !data.is_null() {
        // SAFETY: a custom event on a handle registration carries the
        // handle arm of the union, which is all this reads.
        let guid = unsafe { (*data).u.DeviceHandle.EventGuid };
        guid == GUID_IO_VOLUME_LOCK || guid == GUID_IO_VOLUME_DISMOUNT
    } else {
        action == CM_NOTIFY_ACTION_DEVICEQUERYREMOVE
            || action == CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE
    };
    if wanted {
        // SAFETY: see `register`.
        let signals = unsafe { &*context.cast::<Signals>() };
        signals.release.set();
        // SAFETY: a live event handle.
        unsafe {
            let _ = WaitForSingleObject(signals.let_go.0, LET_GO_MS);
        }
    }
    0 // ERROR_SUCCESS
}

/// The paths in a buffer of `FILE_NOTIFY_INFORMATION` records, relative to
/// the watched folder: a u32 offset to the next record (0 on the last), a u32
/// action, a u32 name length in bytes, then the name in UTF-16. The action is
/// not kept, since the caller looks at what is there now.
fn changed_paths(bytes: &[u8]) -> Vec<PathBuf> {
    let u32_at = |i: usize| {
        bytes
            .get(i..i + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let mut out = Vec::new();
    let mut at = 0usize;
    while let (Some(next), Some(len)) = (u32_at(at), u32_at(at + 8)) {
        let Some(name) = bytes.get(at + 12..at + 12 + len as usize) else {
            break;
        };
        let wide: Vec<u16> = name
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        out.push(PathBuf::from(OsString::from_wide(&wide)));
        if next == 0 {
            break;
        }
        at += next as usize;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Receiver};
    use std::time::{Duration, Instant};

    fn record(next: u32, action: u32, name: &str) -> Vec<u8> {
        let wide: Vec<u8> = name.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let mut r = Vec::new();
        r.extend(next.to_le_bytes());
        r.extend(action.to_le_bytes());
        r.extend((wide.len() as u32).to_le_bytes());
        r.extend(wide);
        r
    }

    #[test]
    fn a_buffer_of_records_reads_as_paths_in_order() {
        // Records are DWORD-aligned, so the first is padded to its offset.
        let padded = record(0, 1, "Albums\\Song.mp3").len().div_ceil(4) * 4;
        let mut bytes = record(padded as u32, 1, "Albums\\Song.mp3");
        bytes.resize(padded, 0);
        bytes.extend(record(0, 2, "old.flac"));
        assert_eq!(
            changed_paths(&bytes),
            [PathBuf::from("Albums\\Song.mp3"), PathBuf::from("old.flac")]
        );
        // A record cut short ends the list rather than reading past it.
        assert_eq!(changed_paths(&bytes[..20]), Vec::<PathBuf>::new());
        assert_eq!(changed_paths(&[]), Vec::<PathBuf>::new());
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hp-tree-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wait_for(rx: &Receiver<TreeEvent>, want: &TreeEvent) -> bool {
        let until = Instant::now() + Duration::from_secs(5);
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(ev) if &ev == want => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
        false
    }

    /// The real call, on a real folder: a file dropped in a subfolder is
    /// reported by its path under the root.
    #[test]
    fn a_file_dropped_in_a_subfolder_is_reported_by_its_path() {
        let dir = scratch("drop");
        let (tx, rx) = channel();
        let w = watch(
            &dir,
            Box::new(move |ev| {
                let _ = tx.send(ev);
            }),
        )
        .unwrap();
        std::fs::create_dir(dir.join("Album")).unwrap();
        std::fs::write(dir.join("Album").join("song.mp3"), b"x").unwrap();
        assert!(wait_for(
            &rx,
            &TreeEvent::Changed(PathBuf::from("Album\\song.mp3"))
        ));
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Dropping the watch returns, and the folder is free: it can be deleted
    /// straight after, which an open handle without FILE_SHARE_DELETE, or a
    /// thread still reading, would get in the way of.
    #[test]
    fn dropping_the_watch_stops_it_and_lets_go_of_the_folder() {
        let dir = scratch("stop");
        let w = watch(&dir, Box::new(|_| {})).unwrap();
        let started = Instant::now();
        drop(w);
        assert!(started.elapsed() < Duration::from_secs(2));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The eject, on a real drive. Not run by default: it needs a removable
    /// drive and a person, or a script, to eject it within a minute.
    ///
    ///   $env:HP_EJECT_DIR = "E:\"; cargo test eject -- --ignored --nocapture
    ///
    /// The watch has to say it let go, and the eject has to go through.
    #[test]
    #[ignore]
    fn an_eject_is_let_through() {
        let dir = PathBuf::from(std::env::var("HP_EJECT_DIR").expect("set HP_EJECT_DIR"));
        let (tx, rx) = channel();
        let w = watch(
            &dir,
            Box::new(move |ev| {
                let _ = tx.send(ev);
            }),
        )
        .unwrap();
        println!("watching {}: eject it now", dir.display());
        let until = Instant::now() + Duration::from_secs(60);
        let mut last = None;
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            match rx.recv_timeout(left) {
                Ok(TreeEvent::Changed(_)) | Ok(TreeEvent::Overflow) => continue,
                Ok(ev) => {
                    last = Some(ev);
                    break;
                }
                Err(_) => break,
            }
        }
        drop(w);
        assert_eq!(last, Some(TreeEvent::Released));
    }

    #[test]
    fn a_folder_that_is_not_there_is_refused() {
        let dir = std::env::temp_dir().join("hp-tree-no-such-folder");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(watch(&dir, Box::new(|_| {})).is_err());
    }
}
