//! Drives on Linux (#162, D138, D143): their room, and which drive a folder
//! is on, from `statvfs`, `/proc/self/mountinfo` and `/dev/disk/by-uuid`.
//!
//! A drive's id is its filesystem UUID, which is what udev names it by and
//! what travels with a stick from one mount point to the next. On a FAT or
//! exFAT stick that UUID is the volume serial Windows reads, only written
//! `1A2B-3C4D` rather than `1A2B3C4D`.

use super::super::{DiskSpace, Volume};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Where the udev rules name each filesystem by its UUID.
const BY_UUID: &str = "/dev/disk/by-uuid";

/// Filesystems that are not a drive a library root lives on: a disc is not
/// a root (as on Windows), and the network ones are not asked, since asking
/// one whose server has gone can take a long time.
const NOT_A_DRIVE: &[&str] = &[
    "iso9660",
    "udf",
    "nfs",
    "nfs4",
    "cifs",
    "smb3",
    "sshfs",
    "fuse.sshfs",
];

/// One line of `/proc/self/mountinfo`, as much of it as is asked.
#[derive(Debug, PartialEq)]
struct Mount {
    /// The device, `major:minor`, as `stat` gives it for a file on it.
    dev: (u32, u32),
    /// The folder within the filesystem that is mounted. `/` but for a bind
    /// mount or a btrfs subvolume.
    root: PathBuf,
    /// Where it is mounted.
    at: PathBuf,
    fstype: String,
    /// What is mounted: a device node for a drive.
    source: String,
}

/// mountinfo writes a space, a tab, a newline and a backslash in a path as
/// three octal digits after a backslash.
fn unescape(s: &str) -> PathBuf {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let oct = b
            .get(i + 1..i + 4)
            .filter(|o| o.iter().all(|c| (b'0'..=b'7').contains(c)));
        match (b[i], oct) {
            (b'\\', Some(o)) => {
                out.push((o[0] - b'0') * 64 + (o[1] - b'0') * 8 + (o[2] - b'0'));
                i += 4;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    PathBuf::from(std::ffi::OsStr::from_bytes(&out))
}

/// Parse `/proc/self/mountinfo`. The optional fields between the mount
/// options and the ` - ` are of any number, so the line is split there.
fn parse(mountinfo: &str) -> Vec<Mount> {
    mountinfo
        .lines()
        .filter_map(|line| {
            let (head, tail) = line.split_once(" - ")?;
            let head: Vec<&str> = head.split(' ').collect();
            let mut tail = tail.split(' ');
            let (major, minor) = head.get(2)?.split_once(':')?;
            Some(Mount {
                dev: (major.parse().ok()?, minor.parse().ok()?),
                root: unescape(head.get(3)?),
                at: unescape(head.get(4)?),
                fstype: tail.next()?.to_string(),
                source: unescape(tail.next()?).to_string_lossy().into_owned(),
            })
        })
        .collect()
}

fn mounts() -> Vec<Mount> {
    std::fs::read_to_string("/proc/self/mountinfo")
        .map(|s| parse(&s))
        .unwrap_or_default()
}

/// The mount of a whole filesystem a file on device `dev` at `path` is
/// under: of those mounts of that device, the one with the longest mount
/// point the path is in. Only a whole filesystem, because a root's place on
/// its drive is kept relative to the mount (D143) and found again under
/// `mounts_of`'s, which are whole filesystems too. A path reached only
/// through a bind mount, or on a btrfs subvolume, has no drive to follow; a
/// stick is mounted whole.
fn mount_of<'a>(mounts: &'a [Mount], dev: (u32, u32), path: &Path) -> Option<&'a Mount> {
    mounts
        .iter()
        .filter(|m| m.dev == dev && m.root == Path::new("/") && path.starts_with(&m.at))
        .max_by_key(|m| m.at.as_os_str().len())
}

/// The UUID udev has for a device node, by finding the by-uuid link that
/// resolves to it.
fn uuid_of(source: &Path) -> Option<String> {
    let dev = std::fs::canonicalize(source).ok()?;
    std::fs::read_dir(BY_UUID)
        .ok()?
        .filter_map(|e| e.ok())
        .find(|e| std::fs::canonicalize(e.path()).ok().as_deref() == Some(dev.as_path()))
        .and_then(|e| e.file_name().into_string().ok())
}

/// The space on the drive a folder is on, as `df` has it: `f_bavail` is what
/// a process without root may still write, which is this one.
pub fn disk_space(path: &Path) -> Option<DiskSpace> {
    if !path.is_dir() {
        return None;
    }
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: a zeroed struct for the call to fill, and a NUL-terminated
    // path that outlives it.
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let unit = s.f_frsize;
    let total = s.f_blocks * unit;
    (total > 0).then_some(DiskSpace {
        free: s.f_bavail * unit,
        total,
    })
}

/// The drive a folder is on and where it is mounted. A filesystem with no
/// UUID (a tmpfs, a network share) has no id to follow, as a drive with no
/// serial has none on Windows.
pub fn volume_of(path: &Path) -> Option<Volume> {
    if !path.is_dir() {
        return None;
    }
    let path = std::fs::canonicalize(path).ok()?;
    let dev = std::fs::metadata(&path).ok()?.dev();
    let all = mounts();
    let m = mount_of(&all, (libc::major(dev), libc::minor(dev)), &path)?;
    if NOT_A_DRIVE.contains(&m.fstype.as_str()) {
        return None;
    }
    Some(Volume {
        id: uuid_of(Path::new(&m.source))?,
        mount: m.at.clone(),
    })
}

/// Where a drive with this UUID is mounted now. Where it is mounted more
/// than once, every mount of its whole filesystem; a bind mount of a folder
/// in it is a different place, and `drives::whereabouts` joins the root's
/// folder onto these.
pub fn mounts_of(id: &str) -> Vec<PathBuf> {
    if id.is_empty() || id.contains('/') {
        return Vec::new();
    }
    let Ok(dev) = std::fs::canonicalize(Path::new(BY_UUID).join(id)) else {
        return Vec::new();
    };
    mounts()
        .into_iter()
        .filter(|m| m.root == Path::new("/") && !NOT_A_DRIVE.contains(&m.fstype.as_str()))
        .filter(|m| std::fs::canonicalize(&m.source).ok().as_deref() == Some(dev.as_path()))
        .map(|m| m.at)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = "\
26 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
40 26 0:35 / /tmp rw shared:20 - tmpfs tmpfs rw
512 30 8:17 / /media/me/MY\\040STICK rw,nosuid,nodev,relatime shared:300 - vfat /dev/sdb1 rw,uid=1000
513 26 8:17 /Music /home/me/Music rw,relatime - vfat /dev/sdb1 rw
";

    #[test]
    fn mountinfo_is_read_past_its_optional_fields_and_escapes() {
        let m = parse(INFO);
        assert_eq!(m.len(), 4);
        assert_eq!(
            m[2],
            Mount {
                dev: (8, 17),
                root: "/".into(),
                at: "/media/me/MY STICK".into(),
                fstype: "vfat".into(),
                source: "/dev/sdb1".into(),
            }
        );
        assert_eq!(m[3].root, PathBuf::from("/Music"));
    }

    #[test]
    fn a_path_is_under_the_deepest_mount_of_its_own_device() {
        let m = parse(INFO);
        let at = |dev, p: &str| mount_of(&m, dev, Path::new(p)).map(|m| m.at.clone());
        assert_eq!(at((259, 2), "/home/me/Music2"), Some("/".into()));
        assert_eq!(
            at((8, 17), "/media/me/MY STICK/Album"),
            Some("/media/me/MY STICK".into())
        );
        // Through the bind mount of a folder on the stick: not followed.
        assert_eq!(at((8, 17), "/home/me/Music/Album"), None);
        // `/tmp` is a different device; the path's own device decides.
        assert_eq!(at((259, 2), "/tmp/x"), Some("/".into()));
        assert_eq!(at((9, 9), "/x"), None);
    }

    #[test]
    fn an_escape_is_three_octal_digits_and_nothing_else_is() {
        assert_eq!(unescape("a\\040b\\134c"), PathBuf::from("a b\\c"));
        assert_eq!(unescape("a\\9b\\04"), PathBuf::from("a\\9b\\04"));
    }

    /// The real calls, on the folder the tests run in: it has room, its
    /// drive has an id, and that drive is found again by the id.
    #[test]
    fn the_drive_a_folder_is_on_is_found_again_by_its_id() {
        let here = std::env::current_dir().unwrap();
        let room = disk_space(&here).unwrap();
        assert!(room.total > 0 && room.free <= room.total);
        // A container or a CI runner on an overlay has no UUID to give.
        let Some(v) = volume_of(&here) else { return };
        assert!(here.canonicalize().unwrap().starts_with(&v.mount));
        assert!(mounts_of(&v.id).contains(&v.mount));
        assert!(mounts_of("no-such-uuid").is_empty());
        assert!(mounts_of("../sda1").is_empty());
    }

    #[test]
    fn a_folder_that_is_not_there_has_no_drive() {
        let gone = Path::new("/no/such/folder/hp");
        assert!(disk_space(gone).is_none());
        assert!(volume_of(gone).is_none());
    }
}
