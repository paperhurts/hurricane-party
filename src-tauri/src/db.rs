//! SQLite: library, playlists, job queue.
//!
//! Raw `rusqlite`, not `tauri-plugin-sql`. The plugin exists to let the
//! frontend run SQL over IPC, which is precisely what we don't want — same
//! reasoning that removed the webview's shell permissions. SQL stays in Rust.
//!
//! **WAL is the point** (D10). The job queue has to survive a hard power loss,
//! which is the whole reason this app exists rather than a folder of files.

use rusqlite::Connection;
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
