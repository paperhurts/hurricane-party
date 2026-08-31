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

/// Extensions worth scanning. Deliberately audio-only for v0.3 — video import
/// from a local folder is a different question (thumbnails, playback window)
/// and isn't what this milestone asked for.
const AUDIO_EXTS: &[&str] = &[
    "mp3", "m4a", "aac", "flac", "ogg", "oga", "opus", "wav", "wma", "aiff", "aif", "alac",
];

#[derive(Debug, Clone, Serialize)]
pub struct ScanReport {
    pub root: String,
    pub found: usize,
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
}

fn is_audio(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|e| AUDIO_EXTS.contains(&e.as_str()))
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
        if !entry.file_type().is_file() || !is_audio(path) {
            continue;
        }
        report.found += 1;

        let Ok(rel) = path.strip_prefix(&root) else {
            report.skipped += 1;
            continue;
        };
        let relpath = rel.to_string_lossy().to_string();
        let filesize = entry.metadata().map(|m| m.len() as i64).ok();
        let t = read_tags(path);

        // D28: (root_id, relpath), never the absolute path.
        let n = tx.execute(
            "INSERT INTO media (source_id, root_id, relpath, kind, title, uploader,
                                duration_s, container, bitrate_kbps, filesize, added_at)
             VALUES (NULL, ?1, ?2, 'audio', ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(root_id, relpath) DO UPDATE SET
                title = excluded.title, uploader = excluded.uploader,
                duration_s = excluded.duration_s, filesize = excluded.filesize",
            rusqlite::params![
                root_id,
                relpath,
                t.title,
                t.artist,
                t.duration_s,
                path.extension().and_then(|e| e.to_str()),
                t.bitrate_kbps,
                filesize,
                db::now(),
            ],
        )?;
        if n == 1 {
            report.added += 1;
        } else {
            report.updated += 1;
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
    fn recognises_audio_by_extension() {
        for good in ["a.mp3", "b.FLAC", "c.m4a", "d.opus", "e.Wav"] {
            assert!(is_audio(Path::new(good)), "{good} should be audio");
        }
        for bad in ["a.txt", "b.jpg", "c.mp4", "d.webm", "cover.png", "no-extension"] {
            assert!(!is_audio(Path::new(bad)), "{bad} should not be audio");
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
