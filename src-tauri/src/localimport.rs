//! Local folder import.
//!
//! Point at a folder of music you already own; it becomes a **library root**
//! (D28) and its files become `media` rows with `source_id NULL`. This is the
//! first real exercise of two decisions that were made early specifically to
//! avoid a retrofit:
//!
//!   - **D28** — `media` stores `(root_id, relpath)`, never an absolute path.
//!     An external drive that reappears as a different letter must not orphan
//!     the library, and that case is exactly "the storm drive".
//!   - **D34** — `media` carries its own `title`/`uploader`/`duration_s`.
//!     A local file has no `sources` row to borrow them from, so without this
//!     the library browser could not render a row for it at all.
//!
//! Tags are read with `lofty` rather than ffprobe (D50): no fourth sidecar, and
//! no process spawn per file when scanning thousands.

use crate::db::{self, Db, DbError};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::ItemKey;
use lofty::probe::Probe;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};

/// Extensions worth scanning.
///
/// Audio and video both, as of D64. This was audio-only through v0.3 because
/// local video raised questions the milestone had not answered yet — where it
/// plays, what a thumbnail is. The playback question is now settled: D13's
/// decorated video window exists, and the library already routes a
/// `kind = 'video'` row to it. Leaving the scanner audio-only made a downloaded
/// video visible while the identical file found by a folder scan was not.
const AUDIO_EXTS: &[&str] = &[
    "mp3", "m4a", "aac", "flac", "ogg", "oga", "opus", "wav", "wma", "aiff", "aif", "alac",
];

/// Containers the video window can actually play. Deliberately not "anything
/// ffmpeg knows" — a row that opens a window and then fails is worse than a
/// file the library never claimed.
const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "mkv", "webm", "mov", "avi"];

#[derive(Debug, Clone, Serialize)]
pub struct ScanReport {
    pub root: String,
    pub found: usize,
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
}

/// `"audio"`, `"video"`, or `None` for a file the library should not claim.
///
/// The returned string is written straight into `media.kind`, so the two lists
/// and the column cannot drift apart.
fn kind_of(p: &Path) -> Option<&'static str> {
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if AUDIO_EXTS.contains(&ext.as_str()) {
        Some("audio")
    } else if VIDEO_EXTS.contains(&ext.as_str()) {
        Some("video")
    } else {
        // Everything else, including the .part files a killed download leaves
        // behind (D26 keeps those on purpose) and the cover art next to a
        // track.
        None
    }
}

/// What we could learn about a file without opening a subprocess.
struct Tags {
    title: String,
    artist: Option<String>,
    duration_s: Option<f64>,
    bitrate_kbps: Option<u32>,
}

/// Read tags, falling back to the filename for the title.
///
/// A missing tag is normal, not an error: plenty of real libraries are full of
/// untagged rips. Falling back to the stem means the row is still usable, which
/// matters more than being pure about metadata.
fn read_tags(path: &Path) -> Tags {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string();

    let Ok(tagged) = Probe::open(path).and_then(|p| p.read()) else {
        return Tags { title: stem, artist: None, duration_s: None, bitrate_kbps: None };
    };

    let props = tagged.properties();
    let duration_s = Some(props.duration().as_secs_f64()).filter(|d| *d > 0.0);
    let bitrate_kbps = props.audio_bitrate();

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let (title, artist) = match tag {
        Some(t) => (
            t.get_string(ItemKey::TrackTitle)
                .map(str::to_string)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(stem),
            t.get_string(ItemKey::TrackArtist)
                .map(str::to_string)
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    t.get_string(ItemKey::AlbumArtist).map(str::to_string)
                }),
        ),
        None => (stem, None),
    };

    Tags { title, artist, duration_s, bitrate_kbps }
}

/// Register a folder as a library root and pull everything in it into `media`.
///
/// Re-scanning an already-known root is safe and expected — it's how you pick
/// up files added since. Rows are upserted on `(root_id, relpath)`.
pub fn scan_root(app: &AppHandle, root: &Path, label: &str) -> Result<ScanReport, DbError> {
    let root = root
        .canonicalize()
        .map_err(|e| DbError::Io(format!("can't read {}: {e}", root.display())))?;

    let root_id = {
        let state = app.state::<Db>();
        let conn = state.0.lock().unwrap();
        db::ensure_root(&conn, label, &root.to_string_lossy())?
    };

    // The asset protocol scope is declared statically in tauri.conf.json and
    // only covers app data. A folder the user just picked is outside it, so the
    // webview could not load the file for playback without this.
    allow_root(app, &root);

    let mut report = ScanReport {
        root: root.to_string_lossy().to_string(),
        found: 0,
        added: 0,
        updated: 0,
        skipped: 0,
    };

    let state = app.state::<Db>();
    let mut conn = state.0.lock().unwrap();
    let tx = conn.transaction()?;

    for entry in walkdir::WalkDir::new(&root)
        .follow_links(false) // a symlink loop would walk forever
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        let Some(kind) = kind_of(path).filter(|_| entry.file_type().is_file()) else {
            continue;
        };
        report.found += 1;

        let Ok(rel) = path.strip_prefix(&root) else {
            report.skipped += 1;
            continue;
        };
        let relpath = rel.to_string_lossy().to_string();
        let filesize = entry.metadata().map(|m| m.len() as i64).ok();
        let t = read_tags(path);

        // Ask first, because the counts are reported to the user and an UPSERT
        // cannot tell them apart: SQLite reports one changed row whether the
        // conflict clause inserted or updated, so counting `execute`'s return
        // called every re-scan a fresh import. One indexed lookup per file is a
        // cheap price for a message that is true.
        let existed: bool = tx
            .query_row(
                "SELECT 1 FROM media WHERE root_id = ?1 AND relpath = ?2",
                rusqlite::params![root_id, relpath],
                |_| Ok(true),
            )
            .unwrap_or(false);

        // D28: (root_id, relpath), never the absolute path.
        tx.execute(
            "INSERT INTO media (source_id, root_id, relpath, kind, title, uploader,
                                duration_s, container, bitrate_kbps, filesize, added_at)
             VALUES (NULL, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(root_id, relpath) DO UPDATE SET
                kind = excluded.kind, title = excluded.title,
                uploader = excluded.uploader, duration_s = excluded.duration_s,
                filesize = excluded.filesize",
            rusqlite::params![
                root_id,
                relpath,
                kind,
                t.title,
                t.artist,
                t.duration_s,
                path.extension().and_then(|e| e.to_str()),
                t.bitrate_kbps,
                filesize,
                db::now(),
            ],
        )?;
        if existed {
            report.updated += 1;
        } else {
            report.added += 1;
        }
    }

    tx.commit()?;
    drop(conn);
    let _ = app.emit("library-changed", ());
    Ok(report)
}

/// Extend the runtime asset scope to a library root so its files can play.
pub fn allow_root(app: &AppHandle, root: &Path) {
    let scope = app.asset_protocol_scope();
    if let Err(e) = scope.allow_directory(root, true) {
        eprintln!("couldn't grant asset access to {}: {e}", root.display());
    }
}

/// On launch, re-grant every known root. Scope is runtime state and does not
/// survive a restart the way the `library_roots` rows do.
pub fn allow_known_roots(app: &AppHandle) {
    let paths: Vec<String> = {
        let state = app.state::<Db>();
        let Ok(conn) = state.0.lock() else { return };
        let Ok(mut st) = conn.prepare("SELECT path FROM library_roots") else { return };
        let Ok(rows) = st.query_map([], |r| r.get::<_, String>(0)) else { return };
        rows.filter_map(|r| r.ok()).collect()
    };
    for p in paths {
        allow_root(app, Path::new(&p));
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Root {
    pub id: i64,
    pub label: String,
    pub path: String,
    pub count: i64,
    /// False when the drive isn't plugged in. The storm-drive case (D28) is a
    /// missing root, not a broken library.
    pub present: bool,
}

pub fn list_roots(conn: &rusqlite::Connection) -> Result<Vec<Root>, DbError> {
    let mut st = conn.prepare(
        "SELECT r.id, r.label, r.path,
                (SELECT COUNT(*) FROM media m WHERE m.root_id = r.id) AS count
         FROM library_roots r ORDER BY r.id",
    )?;
    let rows = st.query_map([], |r| {
        let path: String = r.get("path")?;
        Ok(Root {
            id: r.get("id")?,
            label: r.get("label")?,
            present: PathBuf::from(&path).is_dir(),
            path,
            count: r.get("count")?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_and_video_are_both_claimed_and_nothing_else_is() {
        // D64. The playback question that kept video out of here is settled --
        // the row routes to D13's video window -- so a folder scan now finds
        // the same file the download pipeline would have.
        assert_eq!(kind_of(Path::new("a/b/track.mp3")), Some("audio"));
        assert_eq!(kind_of(Path::new("a/b/track.FLAC")), Some("audio"));
        assert_eq!(kind_of(Path::new("a/b/clip.mp4")), Some("video"));
        assert_eq!(kind_of(Path::new("a/b/clip.MKV")), Some("video"));

        // Not claimed: cover art, and the .part files a killed download leaves
        // behind on purpose (D26). Claiming either would put a row in the
        // library that cannot be played.
        assert_eq!(kind_of(Path::new("a/b/cover.jpg")), None);
        assert_eq!(kind_of(Path::new("a/b/track.mp3.part")), None);
        assert_eq!(kind_of(Path::new("a/b/notes.txt")), None);
        assert_eq!(kind_of(Path::new("a/b/no-extension")), None);
    }

    #[test]
    fn the_two_extension_lists_do_not_overlap() {
        // m4a is audio and m4v is video, which is exactly the kind of pair that
        // ends up in both lists. An overlap would make kind_of depend on which
        // list is checked first.
        for e in AUDIO_EXTS {
            assert!(!VIDEO_EXTS.contains(e), "{e} is in both lists");
        }
    }

    /// An untagged file is ordinary, not an error — plenty of real libraries
    /// are full of them. The filename has to carry the title so the row is
    /// still usable in the browser.
    #[test]
    fn untagged_file_falls_back_to_its_filename() {
        let tmp = std::env::temp_dir().join("hp-tags-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let f = tmp.join("Some Old Rip.mp3");
        std::fs::write(&f, b"not really an mp3").unwrap();

        let t = read_tags(&f);
        assert_eq!(t.title, "Some Old Rip");
        assert!(t.artist.is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
