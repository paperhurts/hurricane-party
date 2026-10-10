//! Processes on Linux, read from `/proc` (#187).
//!
//! Not about X at all; here because this is the Linux platform. Both calls
//! answer the question the Windows ones answer, the way Linux keeps it.

use std::path::{Path, PathBuf};

/// Every pid and its parent, as `/proc` has them now.
fn table() -> Vec<(u32, u32)> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|e| {
        let pid: u32 = e.ok()?.file_name().to_str()?.parse().ok()?;
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        Some((pid, parent_in_stat(&stat)?))
    })
    .collect()
}

/// The parent pid in a `/proc/<pid>/stat` line. The command name is in
/// parentheses and may hold spaces and parentheses of its own, so the
/// fields are counted from the last `)`: state, then the parent.
fn parent_in_stat(stat: &str) -> Option<u32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// `pid` and everything it started, parents before children.
fn descendants(pid: u32, table: &[(u32, u32)]) -> Vec<u32> {
    let mut tree = vec![pid];
    let mut i = 0;
    while i < tree.len() {
        let parent = tree[i];
        tree.extend(
            table
                .iter()
                .filter(|(p, pp)| *pp == parent && !tree.contains(p))
                .map(|(p, _)| *p)
                .collect::<Vec<_>>(),
        );
        i += 1;
    }
    tree
}

/// D117 on Linux. yt-dlp's Linux build is a PyInstaller one-file build too,
/// so the pid the app holds is a bootloader with the downloader under it, and
/// that starts ffmpeg. A process group would end them all, but the shell
/// plugin starts a sidecar in the app's own group. So the tree is walked from
/// `/proc`, each process stopped as it is found so that none can start
/// another behind the walk, and then every one of them is killed.
pub fn kill_tree(pid: u32) {
    let mut stopped: Vec<u32> = Vec::new();
    // Twice round: a child started between reading the table and stopping
    // its parent is in the second read.
    for _ in 0..2 {
        for p in descendants(pid, &table()) {
            if !stopped.contains(&p) {
                // SAFETY: a signal to a pid; the worst a stale pid gets is
                // ESRCH.
                unsafe { libc::kill(p as libc::pid_t, libc::SIGSTOP) };
                stopped.push(p);
            }
        }
    }
    for p in stopped {
        // SAFETY: as above.
        unsafe { libc::kill(p as libc::pid_t, libc::SIGKILL) };
    }
}

/// Whether the `flags:` line of a `/proc/<pid>/fdinfo/<fd>` file is an open
/// for writing. The flags are octal; the access mode is the low two bits.
fn writes(fdinfo: &str) -> bool {
    fdinfo
        .lines()
        .find_map(|l| l.strip_prefix("flags:"))
        .and_then(|f| u32::from_str_radix(f.trim(), 8).ok())
        .is_some_and(|f| f & libc::O_ACCMODE as u32 != libc::O_RDONLY as u32)
}

/// #111 on Linux. Linux has no share modes, so nothing refuses an open the
/// way Windows does while a copy is writing. What there is instead: every
/// process's open files under `/proc`. A file another process has open for
/// writing is in use: a copy still landing, an editor saving. Read-only opens
/// are not, any more than they are on Windows to a reader. Only this user's
/// processes can be seen, which are the ones copying into this user's
/// library.
pub fn in_use(path: &Path) -> bool {
    let Ok(want) = std::fs::canonicalize(path) else {
        return false;
    };
    let me = std::process::id();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return false;
    };
    dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| *pid != me)
        .any(|pid| {
            let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
                return false;
            };
            fds.filter_map(|fd| fd.ok()).any(|fd| {
                std::fs::read_link(fd.path()).ok().as_deref() == Some(want.as_path())
                    && std::fs::read_to_string(
                        PathBuf::from(format!("/proc/{pid}/fdinfo")).join(fd.file_name()),
                    )
                    .is_ok_and(|info| writes(&info))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{Duration, Instant};

    #[test]
    fn the_parent_is_read_past_a_name_with_spaces_and_parentheses() {
        assert_eq!(
            parent_in_stat("4242 (yt-dlp (x) 2) S 4200 4242 4200 0 -1"),
            Some(4200)
        );
        assert_eq!(parent_in_stat("1 (systemd) S 0 1 1 0"), Some(0));
        assert_eq!(parent_in_stat("garbage"), None);
    }

    #[test]
    fn the_tree_is_every_descendant_and_nothing_else() {
        let t = [(10, 1), (11, 10), (12, 11), (13, 10), (20, 1), (21, 20)];
        let mut d = descendants(10, &t);
        d.sort();
        assert_eq!(d, vec![10, 11, 12, 13]);
    }

    #[test]
    fn an_open_for_writing_is_told_from_one_for_reading() {
        assert!(!writes("pos:\t0\nflags:\t0100000\nmnt_id:\t30\n"));
        assert!(writes("pos:\t0\nflags:\t0100001\nmnt_id:\t30\n"));
        assert!(writes("pos:\t0\nflags:\t02100002\nmnt_id:\t30\n"));
        assert!(!writes("pos:\t0\n"));
    }

    /// The real call, on a real tree: a shell that starts a grandchild, as
    /// the bootloader starts the downloader. Both are gone after.
    #[test]
    fn the_whole_tree_is_ended() {
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 60 & wait"])
            .spawn()
            .unwrap();
        let pid = child.id();
        let until = Instant::now() + Duration::from_secs(5);
        let grandchild = loop {
            let d = descendants(pid, &table());
            if d.len() > 1 {
                break d[1];
            }
            assert!(Instant::now() < until, "the shell never started sleep");
            std::thread::sleep(Duration::from_millis(20));
        };
        kill_tree(pid);
        child.wait().unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while Path::new(&format!("/proc/{grandchild}")).exists()
            && std::fs::read_to_string(format!("/proc/{grandchild}/stat"))
                .is_ok_and(|s| !s.contains(") Z "))
        {
            assert!(Instant::now() < until, "the grandchild outlived the kill");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The real call: a file another process is writing is in use, and once
    /// it closes the file it is not.
    #[test]
    fn a_file_another_process_writes_is_in_use_until_it_closes_it() {
        let dir = std::env::temp_dir().join("hp-in-use");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("song.mp3");
        std::fs::write(&file, b"x").unwrap();
        assert!(!in_use(&file));

        let mut writer = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("exec 3>>'{}'; read _", file.display()))
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while !in_use(&file) {
            assert!(Instant::now() < until, "the writer was never seen");
            std::thread::sleep(Duration::from_millis(20));
        }
        writer.stdin.take().unwrap().write_all(b"\n").unwrap();
        writer.wait().unwrap();
        assert!(!in_use(&file));

        // This process's own opens are not another's.
        let _mine = std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap();
        assert!(!in_use(&file));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
