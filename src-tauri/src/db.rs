//! SQLite: library, playlists, job queue.
//!
//! Raw `rusqlite`, not `tauri-plugin-sql`. The plugin exists to let the
//! frontend run SQL over IPC, which is precisely what we don't want — same
//! reasoning that removed the webview's shell permissions. SQL stays in Rust.
//!
//! **WAL is the point** (D10). The job queue has to survive a hard power loss,
//! which is the whole reason this app exists rather than a folder of files.

use rusqlite::{Connection, OptionalExtension};
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Db(pub Mutex<Connection>);

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(String),
}

impl serde::Serialize for DbError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// The schema from architecture.md. One migration — there is no deployed data
/// yet, so this ships whole rather than as a chain. The columns that exist
/// purely to avoid a painful retrofit (`profile_id` per O9, `library_roots` +
/// `relpath` per D28, `media.title` per D34) are the reason it was worth
/// getting right before the first migration was written.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS library_roots (
  id            INTEGER PRIMARY KEY,
  label         TEXT NOT NULL,
  path          TEXT UNIQUE NOT NULL,
  is_removable  INTEGER NOT NULL DEFAULT 0,
  last_seen_at  INTEGER
);

CREATE TABLE IF NOT EXISTS sources (
  id            INTEGER PRIMARY KEY,
  url           TEXT UNIQUE NOT NULL,
  extractor     TEXT NOT NULL,
  title         TEXT,
  uploader      TEXT,
  upload_date   TEXT,
  duration_s    INTEGER,
  thumb_path    TEXT,
  info_json     TEXT,
  added_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS eq_presets (
  id            INTEGER PRIMARY KEY,
  name          TEXT NOT NULL,
  preamp_db     REAL NOT NULL DEFAULT 0,
  bands_db      TEXT NOT NULL,
  is_builtin    INTEGER DEFAULT 0,
  created_at    INTEGER NOT NULL
);

-- (root_id, relpath), never an absolute path (D28): an external drive that
-- comes back as a different letter must not orphan the library.
CREATE TABLE IF NOT EXISTS media (
  id            INTEGER PRIMARY KEY,
  source_id     INTEGER REFERENCES sources(id) ON DELETE CASCADE,
  root_id       INTEGER NOT NULL REFERENCES library_roots(id),
  relpath       TEXT NOT NULL,
  kind          TEXT NOT NULL CHECK (kind IN ('audio','video')),
  title         TEXT NOT NULL,
  uploader      TEXT,
  duration_s    REAL,
  container     TEXT,
  bitrate_kbps  INTEGER,
  filesize      INTEGER,
  sha256        TEXT,
  verified_at   INTEGER,
  eq_preset_id  INTEGER REFERENCES eq_presets(id),
  added_at      INTEGER NOT NULL,
  UNIQUE (root_id, relpath)
);

CREATE TABLE IF NOT EXISTS playlists (
  id            INTEGER PRIMARY KEY,
  name          TEXT NOT NULL,
  is_smart      INTEGER DEFAULT 0,
  rule_json     TEXT,
  profile_id    INTEGER NOT NULL DEFAULT 1,
  created_at    INTEGER NOT NULL
);

-- The unique (playlist_id, position) invariant is real and worth keeping, but
-- SQLite has no deferred UNIQUE, so a reorder cannot be a naive sequence of
-- UPDATEs. See `playlist::reorder` for the two-phase form.
CREATE TABLE IF NOT EXISTS playlist_items (
  playlist_id   INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
  media_id      INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
  position      INTEGER NOT NULL,
  PRIMARY KEY (playlist_id, position)
);

-- Survives power loss. This is the point (D10).
CREATE TABLE IF NOT EXISTS jobs (
  id            INTEGER PRIMARY KEY,
  url           TEXT NOT NULL,
  want_video    INTEGER NOT NULL DEFAULT 0,
  want_audio    INTEGER NOT NULL DEFAULT 1,
  status        TEXT NOT NULL CHECK (status IN ('queued','running','done','failed','paused')),
  stage         TEXT NOT NULL DEFAULT 'probe'
                CHECK (stage IN ('probe','download','extract','verify')),
  title         TEXT,
  video_id      TEXT,
  outtmpl       TEXT,
  progress      REAL NOT NULL DEFAULT 0,
  bytes_done    INTEGER NOT NULL DEFAULT 0,
  bytes_total   INTEGER,
  error         TEXT,
  attempts      INTEGER NOT NULL DEFAULT 0,
  playlist_id   INTEGER REFERENCES playlists(id) ON DELETE SET NULL,
  profile_id    INTEGER NOT NULL DEFAULT 1,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS play_history (
  media_id      INTEGER REFERENCES media(id) ON DELETE CASCADE,
  played_at     INTEGER NOT NULL,
  completed     INTEGER,
  profile_id    INTEGER NOT NULL DEFAULT 1
);

-- Settings live here, not a side file (D32), so a hard power loss cannot
-- desync them from the library they describe.
CREATE TABLE IF NOT EXISTS settings (
  key           TEXT PRIMARY KEY,
  value         TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS window_layout (
  window_id     TEXT PRIMARY KEY,
  x INTEGER NOT NULL, y INTEGER NOT NULL,
  w INTEGER NOT NULL, h INTEGER NOT NULL,
  shaded        INTEGER NOT NULL DEFAULT 0,
  visible       INTEGER NOT NULL DEFAULT 1,
  monitor_id    TEXT
);

CREATE TABLE IF NOT EXISTS window_bonds (
  a TEXT NOT NULL, b TEXT NOT NULL,
  edge TEXT NOT NULL,
  span_start INTEGER NOT NULL, span_end INTEGER NOT NULL,
  PRIMARY KEY (a, b)
);

CREATE INDEX IF NOT EXISTS idx_jobs_status  ON jobs(status, created_at);
CREATE INDEX IF NOT EXISTS idx_media_root   ON media(root_id);
CREATE INDEX IF NOT EXISTS idx_items_plist  ON playlist_items(playlist_id, position);
"#;

pub fn open(path: &PathBuf) -> Result<Connection, DbError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| DbError::Io(e.to_string()))?;
    }
    let conn = Connection::open(path)?;

    // WAL is what makes D10 true rather than aspirational: a reader never
    // blocks the writer, and a hard kill leaves a recoverable log rather than
    // a truncated database.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    // NORMAL is the right pairing with WAL — FULL fsyncs every commit and the
    // job queue commits on every progress tick.
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;

    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

/// Crash recovery, run once at launch (D10, refined by D26).
///
/// `running` -> `queued` means "re-enter the runner", NOT "start over". The row
/// keeps its `stage`, `video_id` and `outtmpl`, and the runner picks the
/// recovery that matches where it died. Critically, this does **not** touch
/// `.part` files on disk — deleting them here would silently convert resume
/// into restart, which is the exact failure this milestone exists to prevent.
pub fn recover_interrupted(conn: &Connection) -> Result<usize, DbError> {
    let n = conn.execute(
        "UPDATE jobs SET status = 'queued', updated_at = ?1 WHERE status = 'running'",
        [now()],
    )?;
    Ok(n)
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Ensure a row for the default library root and return its id.
pub fn ensure_root(conn: &Connection, label: &str, path: &str) -> Result<i64, DbError> {
    conn.execute(
        "INSERT INTO library_roots (label, path, last_seen_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(path) DO UPDATE SET last_seen_at = excluded.last_seen_at",
        rusqlite::params![label, path, now()],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM library_roots WHERE path = ?1",
        [path],
        |r| r.get(0),
    )?)
}

/// A path as Windows shows it, not as `canonicalize` returns it.
///
/// On Windows `canonicalize` yields the verbatim form: `\\?\C:\x`, or
/// `\\?\UNC\server\share` for a network path. The scanner stored that, the
/// download pipeline stored the plain form, and `library_roots.path` is UNIQUE
/// on the string, so the same folder added by both routes became two roots
/// and every file in it two rows (#78). Everything that stores a root goes
/// through here first. Plain string work, no platform gate: the prefix never
/// occurs elsewhere, so stripping it is a no-op there.
pub fn plain_path(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.to_string()
    }
}

/// Fold roots stored in verbatim form into their plain twins. Run on open.
///
/// A verbatim root with no twin is simply renamed. One with a twin gives it
/// every row the twin does not already have; the rest are the same file
/// twice, so the copy's playlist entries and history move to the survivor and
/// the copy goes. Returns how many roots were touched.
pub fn normalize_roots(conn: &mut Connection) -> Result<usize, DbError> {
    let roots: Vec<(i64, String)> = {
        let mut st = conn.prepare("SELECT id, path FROM library_roots ORDER BY id")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let tx = conn.transaction()?;
    let mut touched = 0;
    for (id, path) in &roots {
        let plain = plain_path(path);
        if plain == *path {
            continue;
        }
        let twin: Option<i64> = tx
            .query_row(
                "SELECT id FROM library_roots WHERE path = ?1 AND id != ?2",
                rusqlite::params![plain, id],
                |r| r.get(0),
            )
            .optional()?;
        match twin {
            None => {
                tx.execute(
                    "UPDATE library_roots SET path = ?1 WHERE id = ?2",
                    rusqlite::params![plain, id],
                )?;
            }
            Some(twin) => {
                // Rows the twin does not have move over as they are; OR IGNORE
                // leaves behind exactly the ones it does.
                tx.execute(
                    "UPDATE OR IGNORE media SET root_id = ?1 WHERE root_id = ?2",
                    rusqlite::params![twin, id],
                )?;
                let dups: Vec<(i64, i64)> = {
                    let mut st = tx.prepare(
                        "SELECT d.id, s.id FROM media d
                         JOIN media s ON s.root_id = ?1 AND s.relpath = d.relpath
                         WHERE d.root_id = ?2",
                    )?;
                    let rows = st.query_map(rusqlite::params![twin, id], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
                    })?;
                    rows.filter_map(|r| r.ok()).collect()
                };
                for (dup, keep) in dups {
                    tx.execute(
                        "UPDATE playlist_items SET media_id = ?1 WHERE media_id = ?2",
                        rusqlite::params![keep, dup],
                    )?;
                    tx.execute(
                        "UPDATE play_history SET media_id = ?1 WHERE media_id = ?2",
                        rusqlite::params![keep, dup],
                    )?;
                    tx.execute("DELETE FROM media WHERE id = ?1", [dup])?;
                }
                tx.execute("DELETE FROM library_roots WHERE id = ?1", [id])?;
            }
        }
        touched += 1;
    }
    tx.commit()?;
    Ok(touched)
}

pub fn get_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get::<_, String>(0)
    })
    .ok()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

/// Download concurrency (O12): default 2, clamped to 1-4. Higher gets you
/// throttled, not faster.
pub fn concurrency(conn: &Connection) -> usize {
    get_setting(conn, "download.concurrency")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(2)
        .clamp(1, 4)
}

/// The schema, exposed for in-memory test fixtures.
#[cfg(test)]
pub fn schema_for_tests() -> &'static str {
    SCHEMA
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_path_strips_the_verbatim_prefix_and_nothing_else() {
        assert_eq!(plain_path(r"\\?\C:\dev\mp3"), r"C:\dev\mp3");
        assert_eq!(plain_path(r"\\?\UNC\nas\music"), r"\\nas\music");
        assert_eq!(plain_path(r"C:\dev\mp3"), r"C:\dev\mp3");
        assert_eq!(plain_path("/home/x/music"), "/home/x/music");
    }

    fn fresh() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn
    }

    fn root(conn: &Connection, id: i64, path: &str) {
        conn.execute(
            "INSERT INTO library_roots (id, label, path) VALUES (?1, 'r', ?2)",
            rusqlite::params![id, path],
        )
        .unwrap();
    }

    fn track(conn: &Connection, root_id: i64, rel: &str) -> i64 {
        conn.execute(
            "INSERT INTO media (root_id, relpath, kind, title, added_at)
             VALUES (?1, ?2, 'audio', ?2, 0)",
            rusqlite::params![root_id, rel],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn a_verbatim_root_with_no_twin_is_renamed_in_place() {
        let mut conn = fresh();
        root(&conn, 1, r"\\?\C:\dev\mp3");
        let id = track(&conn, 1, "a.mp3");
        assert_eq!(normalize_roots(&mut conn).unwrap(), 1);
        let path: String = conn
            .query_row("SELECT path FROM library_roots WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(path, r"C:\dev\mp3");
        let still: i64 = conn
            .query_row("SELECT root_id FROM media WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(still, 1);
        // Idempotent: a second run finds nothing to do.
        assert_eq!(normalize_roots(&mut conn).unwrap(), 0);
    }

    /// The shape the live database was in (#78): the download root stored
    /// plain by the pipeline, the same folder stored verbatim by "Add folder",
    /// and a playlist pointing at the verbatim copy of a file both had.
    #[test]
    fn a_verbatim_twin_is_folded_into_the_plain_root() {
        let mut conn = fresh();
        root(&conn, 2, r"\\?\C:\lib");
        root(&conn, 3, r"C:\lib");
        let dup = track(&conn, 2, "both.mp3");
        let only_verbatim = track(&conn, 2, "only-in-2.mp3");
        let keep = track(&conn, 3, "both.mp3");
        let pid = crate::playlist::create(&conn, "p").unwrap();
        crate::playlist::add(&conn, pid, dup).unwrap();
        crate::playlist::add(&conn, pid, only_verbatim).unwrap();

        assert_eq!(normalize_roots(&mut conn).unwrap(), 1);

        let roots: i64 = conn
            .query_row("SELECT COUNT(*) FROM library_roots", [], |r| r.get(0))
            .unwrap();
        assert_eq!(roots, 1);
        let mut rows: Vec<(i64, i64, String)> = conn
            .prepare("SELECT id, root_id, relpath FROM media ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        rows.sort();
        assert_eq!(
            rows,
            [
                (only_verbatim, 3, "only-in-2.mp3".to_string()),
                (keep, 3, "both.mp3".to_string()),
            ]
        );
        // The playlist kept both members; the duplicate now names the survivor.
        let members: Vec<i64> = conn
            .prepare("SELECT media_id FROM playlist_items WHERE playlist_id = ?1 ORDER BY position")
            .unwrap()
            .query_map([pid], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(members, [keep, only_verbatim]);
    }
}
