//! Watching a folder on Linux (#111, D137): inotify, on a thread of the
//! watch's own.
//!
//! **inotify watches one folder, not a tree.** So the watch keeps one per
//! folder under the root, adds them for a folder that appears, and drops
//! them for one that goes or moves out. A folder that appears may already
//! hold files by the time its watch is in (a copy makes the folder and
//! writes into it straight away), so every file found while adding it is
//! reported too.
//!
//! **An eject is not refused.** A watch holds no file open, so unmounting a
//! drive goes through with the app watching it, and the watch hears
//! `IN_UNMOUNT` and ends. There is nothing to let go of, so on Linux a watch
//! never reports `Released`; a drive pulled out ends it the same way.
//!
//! **Stopping is prompt.** Dropping the `TreeWatch` wakes the thread through
//! an eventfd it polls beside the inotify one, and waits for it to close both.
//!
//! **Starting is complete.** Every folder's watch is in before `watch`
//! returns, so nothing written after it is missed.

use crate::platform::{TreeEvent, TreeWatch};
use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// What a folder's watch hears. Writes as well as names, as on Windows, so a
/// file still being copied in is heard growing and is held until it is done.
const MASK: u32 = libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_MODIFY
    | libc::IN_CLOSE_WRITE
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF
    | libc::IN_ONLYDIR;

/// The thread's side of the watch.
struct Watcher {
    fd: libc::c_int,
    root: PathBuf,
    /// Each folder's watch, and where the folder is under the root.
    dirs: HashMap<libc::c_int, PathBuf>,
    root_wd: libc::c_int,
    on_event: Box<dyn FnMut(TreeEvent) + Send>,
}

impl Watcher {
    /// Watch the folder at `rel` under the root and every folder under it,
    /// reporting each file found when `report` is set. Symbolic links are not
    /// followed below the root: a link back up the tree would never end.
    fn add(&mut self, rel: &Path, report: bool) {
        let mut todo = vec![rel.to_path_buf()];
        while let Some(rel) = todo.pop() {
            let full = self.root.join(&rel);
            let Ok(c) = CString::new(full.as_os_str().as_bytes()) else {
                continue;
            };
            let follow = if rel.as_os_str().is_empty() {
                0
            } else {
                libc::IN_DONT_FOLLOW
            };
            // SAFETY: an inotify fd this watcher owns, and a NUL-terminated
            // path that outlives the call.
            let wd = unsafe { libc::inotify_add_watch(self.fd, c.as_ptr(), MASK | follow) };
            if wd < 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() == Some(libc::ENOSPC) {
                    eprintln!(
                        "not watching {}: out of inotify watches (fs.inotify.max_user_watches)",
                        full.display()
                    );
                }
                continue;
            }
            self.dirs.insert(wd, rel.clone());
            let Ok(entries) = std::fs::read_dir(&full) else {
                continue;
            };
            for e in entries.filter_map(|e| e.ok()) {
                let child = rel.join(e.file_name());
                match e.file_type() {
                    Ok(t) if t.is_dir() => todo.push(child),
                    Ok(_) if report => (self.on_event)(TreeEvent::Changed(child)),
                    _ => {}
                }
            }
        }
    }

    /// Stop watching the folder at `rel` and everything under it: it was
    /// moved out of where it was, or away.
    fn drop_under(&mut self, rel: &Path) {
        let gone: Vec<libc::c_int> = self
            .dirs
            .iter()
            .filter(|(_, d)| d.starts_with(rel))
            .map(|(wd, _)| *wd)
            .collect();
        for wd in gone {
            self.dirs.remove(&wd);
            // SAFETY: a watch this watcher added; one already gone is EINVAL.
            unsafe { libc::inotify_rm_watch(self.fd, wd) };
        }
    }

    /// Handle one event. False when the watch has ended.
    fn hear(&mut self, wd: libc::c_int, mask: u32, name: Option<PathBuf>) -> bool {
        if mask & libc::IN_Q_OVERFLOW != 0 {
            (self.on_event)(TreeEvent::Overflow);
            return true;
        }
        if wd == self.root_wd
            && mask
                & (libc::IN_DELETE_SELF | libc::IN_MOVE_SELF | libc::IN_UNMOUNT | libc::IN_IGNORED)
                != 0
        {
            (self.on_event)(TreeEvent::Ended);
            return false;
        }
        if mask & libc::IN_IGNORED != 0 {
            self.dirs.remove(&wd);
            return true;
        }
        let (Some(dir), Some(name)) = (self.dirs.get(&wd), name) else {
            return true;
        };
        let rel = dir.join(name);
        if mask & libc::IN_ISDIR != 0 {
            if mask & (libc::IN_DELETE | libc::IN_MOVED_FROM) != 0 {
                self.drop_under(&rel);
            }
            if mask & (libc::IN_CREATE | libc::IN_MOVED_TO) != 0 {
                self.add(&rel, true);
            }
        }
        (self.on_event)(TreeEvent::Changed(rel));
        true
    }

    /// Read and handle what inotify has. False when the watch has ended.
    fn read(&mut self) -> bool {
        // inotify_event is four u32-sized fields and the name; the buffer is
        // aligned for them.
        let mut buf = [0u32; 16 * 1024];
        // SAFETY: a buffer of the length given, owned by this frame.
        let n = unsafe {
            libc::read(
                self.fd,
                buf.as_mut_ptr().cast(),
                std::mem::size_of_val(&buf),
            )
        };
        if n <= 0 {
            let e = std::io::Error::last_os_error();
            return n < 0 && e.kind() == std::io::ErrorKind::WouldBlock;
        }
        let bytes = &as_bytes(&buf)[..n as usize];
        for (wd, mask, name) in events(bytes) {
            if !self.hear(wd, mask, name) {
                return false;
            }
        }
        true
    }
}

fn as_bytes(buf: &[u32]) -> &[u8] {
    // SAFETY: any u32 is four valid bytes, and the slice covers the same
    // memory for the same lifetime.
    unsafe { std::slice::from_raw_parts(buf.as_ptr().cast(), std::mem::size_of_val(buf)) }
}

/// The events in what one read returned: each a fixed header, then a name
/// padded with NULs to the length the header gives.
fn events(mut bytes: &[u8]) -> Vec<(libc::c_int, u32, Option<PathBuf>)> {
    const HEAD: usize = std::mem::size_of::<libc::inotify_event>();
    let mut out = Vec::new();
    while bytes.len() >= HEAD {
        let field = |i: usize| u32::from_ne_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
        let (wd, mask, len) = (field(0) as libc::c_int, field(1), field(3) as usize);
        let Some(raw) = bytes.get(HEAD..HEAD + len) else {
            break;
        };
        let name = raw.split(|b| *b == 0).next().filter(|n| !n.is_empty());
        out.push((
            wd,
            mask,
            name.map(|n| PathBuf::from(std::ffi::OsString::from_vec(n.to_vec()))),
        ));
        bytes = &bytes[HEAD + len..];
    }
    out
}

/// The watch's handle: dropping it stops the thread and waits for it.
struct Guard {
    stop: libc::c_int,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: an eventfd this guard owns; eight bytes is its one write.
        unsafe { libc::write(self.stop, (&1u64 as *const u64).cast(), 8) };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        // SAFETY: as above, and closed once, here.
        unsafe { libc::close(self.stop) };
    }
}

pub fn watch(root: &Path, on_event: Box<dyn FnMut(TreeEvent) + Send>) -> Result<TreeWatch, String> {
    if !root.is_dir() {
        return Err(format!("{} is not a folder", root.display()));
    }
    // SAFETY: plain calls that return new fds or -1.
    let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    if fd < 0 {
        return Err(format!(
            "couldn't start inotify: {}",
            std::io::Error::last_os_error()
        ));
    }
    let stop = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC) };
    if stop < 0 {
        unsafe { libc::close(fd) };
        return Err(format!(
            "couldn't make an eventfd: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut w = Watcher {
        fd,
        root: root.to_path_buf(),
        dirs: HashMap::new(),
        root_wd: -1,
        on_event,
    };
    w.add(Path::new(""), false);
    match w.dirs.iter().find(|(_, d)| d.as_os_str().is_empty()) {
        Some((wd, _)) => w.root_wd = *wd,
        None => {
            // SAFETY: both fds are this function's, closed once.
            unsafe {
                libc::close(fd);
                libc::close(stop);
            }
            return Err(format!("couldn't watch {}", root.display()));
        }
    }
    let thread = std::thread::Builder::new()
        .name("hp-tree-watch".into())
        .spawn(move || {
            let mut fds = [
                libc::pollfd {
                    fd: w.fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: stop,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            loop {
                // SAFETY: two pollfds this frame owns.
                let r = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
                if r < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    (w.on_event)(TreeEvent::Ended);
                    break;
                }
                if fds[1].revents != 0 {
                    break;
                }
                if fds[0].revents != 0 && !w.read() {
                    break;
                }
            }
            // SAFETY: the inotify fd is the thread's from here, closed once.
            unsafe { libc::close(w.fd) };
        })
        .map_err(|e| {
            // SAFETY: the thread never started, so both fds are still here.
            unsafe {
                libc::close(fd);
                libc::close(stop);
            }
            format!("couldn't start the watch: {e}")
        })?;
    Ok(TreeWatch::new(Box::new(Guard {
        stop,
        thread: Some(thread),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{channel, Receiver};
    use std::time::{Duration, Instant};

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

    fn watching(dir: &Path) -> (TreeWatch, Receiver<TreeEvent>) {
        let (tx, rx) = channel();
        let w = watch(
            dir,
            Box::new(move |ev| {
                let _ = tx.send(ev);
            }),
        )
        .unwrap();
        (w, rx)
    }

    /// The real call, on a real folder: a file dropped in a subfolder made
    /// just before it is reported by its path under the root.
    #[test]
    fn a_file_dropped_in_a_subfolder_is_reported_by_its_path() {
        let dir = scratch("drop");
        let (w, rx) = watching(&dir);
        std::fs::create_dir(dir.join("Album")).unwrap();
        std::fs::write(dir.join("Album").join("song.mp3"), b"x").unwrap();
        assert!(wait_for(
            &rx,
            &TreeEvent::Changed(PathBuf::from("Album/song.mp3"))
        ));
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A folder moved in with files already in it: every file is reported,
    /// and a file written into it after is heard, so its watch went in.
    #[test]
    fn a_folder_moved_in_is_reported_and_watched() {
        let dir = scratch("move-in");
        let outside = scratch("move-in-outside");
        std::fs::create_dir_all(outside.join("Album/Disc 1")).unwrap();
        std::fs::write(outside.join("Album/Disc 1/one.mp3"), b"x").unwrap();
        let (w, rx) = watching(&dir);
        std::fs::rename(outside.join("Album"), dir.join("Album")).unwrap();
        assert!(wait_for(
            &rx,
            &TreeEvent::Changed(PathBuf::from("Album/Disc 1/one.mp3"))
        ));
        std::fs::write(dir.join("Album/Disc 1/two.mp3"), b"x").unwrap();
        assert!(wait_for(
            &rx,
            &TreeEvent::Changed(PathBuf::from("Album/Disc 1/two.mp3"))
        ));
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    /// A folder renamed within the root is heard under its new name after.
    #[test]
    fn a_folder_renamed_is_heard_by_its_new_name() {
        let dir = scratch("rename");
        std::fs::create_dir(dir.join("Old")).unwrap();
        let (w, rx) = watching(&dir);
        std::fs::rename(dir.join("Old"), dir.join("New")).unwrap();
        assert!(wait_for(&rx, &TreeEvent::Changed(PathBuf::from("New"))));
        std::fs::write(dir.join("New/song.mp3"), b"x").unwrap();
        assert!(wait_for(
            &rx,
            &TreeEvent::Changed(PathBuf::from("New/song.mp3"))
        ));
        drop(w);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The root going away ends the watch.
    #[test]
    fn the_root_removed_ends_the_watch() {
        let dir = scratch("gone");
        let (w, rx) = watching(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(wait_for(&rx, &TreeEvent::Ended));
        drop(w);
    }

    /// Dropping the watch returns promptly, and the folder can be deleted
    /// straight after.
    #[test]
    fn dropping_the_watch_stops_it_and_lets_go_of_the_folder() {
        let dir = scratch("stop");
        let w = watch(&dir, Box::new(|_| {})).unwrap();
        let started = Instant::now();
        drop(w);
        assert!(started.elapsed() < Duration::from_secs(2));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_that_is_not_there_is_refused() {
        let dir = std::env::temp_dir().join("hp-tree-no-such-folder");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(watch(&dir, Box::new(|_| {})).is_err());
    }

    /// The eject, on a real stick. Not run by default: it needs a mounted
    /// drive and a person to eject it within a minute from the file manager.
    ///
    ///   HP_EJECT_DIR=/media/$USER/STICK cargo test eject -- --ignored --nocapture
    ///
    /// The eject has to go through, and the watch has to end.
    #[test]
    #[ignore]
    fn an_eject_is_let_through() {
        let dir = PathBuf::from(std::env::var("HP_EJECT_DIR").expect("set HP_EJECT_DIR"));
        let (w, rx) = watching(&dir);
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
        assert_eq!(last, Some(TreeEvent::Ended));
    }
}
