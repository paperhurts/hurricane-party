//! The storage budget (#162, D138).
//!
//! What the library takes, what is left on the drive downloads go to, and a
//! ceiling a person may set. The sums and rates are SQL over `media`; the
//! drive is the platform's (`disk_space`). What is said about them, and when,
//! is `src/lib/storage.ts`, so the library and prep mode (#163) say it the
//! same way from the same figures.

use crate::db::{self, DbError};
use crate::platform::{self, DiskSpace};
use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Manager};

/// A person's ceiling on the whole library, in bytes. Unset is none (O10).
pub const CEILING_SETTING: &str = "library.ceiling_bytes";

/// The least playing time a kind needs in the library before its bytes a
/// second are trusted for an estimate. One short clip is not a rate.
const RATE_MIN_SECONDS: f64 = 600.0;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Status {
    /// Everything in the library, every root, plugged in or not, music a
    /// person already owned included (the owner's call on #162). A row's size
    /// is what was on disk when it came in.
    pub library_bytes: u64,
    /// The drive the download folder is on. `None` when the folder is not
    /// there (D136): an unplugged drive has no room to report, which is not
    /// the same as no room.
    pub drive: Option<DiskSpace>,
    pub ceiling: Option<u64>,
    /// Bytes a second of each kind, measured from this library's own files,
    /// for estimating what a download will take. `None` until the library
    /// holds enough of that kind to go on.
    pub audio_bps: Option<f64>,
    pub video_bps: Option<f64>,
}

/// What the whole library takes, in bytes.
pub fn library_bytes(conn: &Connection) -> Result<u64, DbError> {
    let n: i64 = conn.query_row(
        "SELECT COALESCE(SUM(filesize), 0) FROM media WHERE filesize > 0",
        [],
        |r| r.get(0),
    )?;
    Ok(n.max(0) as u64)
}

/// Bytes a second for audio and for video, from the rows that know both
/// their size and their length.
pub fn rates(conn: &Connection) -> Result<(Option<f64>, Option<f64>), DbError> {
    let mut st = conn.prepare(
        "SELECT kind, SUM(filesize), SUM(duration_s) FROM media
         WHERE filesize > 0 AND duration_s > 0 GROUP BY kind",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, f64>(2)?,
        ))
    })?;
    let (mut audio, mut video) = (None, None);
    for (kind, bytes, seconds) in rows.filter_map(|r| r.ok()) {
        let rate = (seconds >= RATE_MIN_SECONDS).then(|| bytes as f64 / seconds);
        match kind.as_str() {
            "audio" => audio = rate,
            "video" => video = rate,
            _ => {}
        }
    }
    Ok((audio, video))
}

/// The ceiling a person set, if they set one.
pub fn ceiling(conn: &Connection) -> Option<u64> {
    db::get_setting(conn, CEILING_SETTING)
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|b| *b > 0)
}

/// Set the ceiling, or take it away with `None`.
pub fn set_ceiling(conn: &Connection, bytes: Option<u64>) -> Result<(), DbError> {
    let value = bytes.filter(|b| *b > 0).map(|b| b.to_string());
    db::set_setting(conn, CEILING_SETTING, value.as_deref().unwrap_or(""))
}

/// The whole picture, as the library window's footer shows it.
pub fn status(app: &AppHandle) -> Result<Status, DbError> {
    let (library_bytes, ceiling, (audio_bps, video_bps)) = {
        let state = app.state::<db::Db>();
        let conn = state.0.lock().unwrap();
        (library_bytes(&conn)?, ceiling(&conn), rates(&conn)?)
    };
    // Outside the lock: `library_root` reads a setting of its own.
    let drive = crate::pipeline::library_root(app)
        .ok()
        .and_then(|dir| platform::platform().disk_space(&dir));
    Ok(Status {
        library_bytes,
        drive,
        ceiling,
        audio_bps,
        video_bps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(db::schema_for_tests()).unwrap();
        conn.execute_batch(
            "INSERT INTO library_roots (id, label, path) VALUES
               (1, 'music', 'C:/music'), (2, 'drive', 'Z:/not/plugged/in');",
        )
        .unwrap();
        conn
    }

    fn row(
        conn: &Connection,
        root: i64,
        rel: &str,
        kind: &str,
        bytes: Option<i64>,
        secs: Option<f64>,
    ) {
        conn.execute(
            "INSERT INTO media (root_id, relpath, kind, title, filesize, duration_s, added_at)
             VALUES (?1, ?2, ?3, ?2, ?4, ?5, 0)",
            rusqlite::params![root, rel, kind, bytes, secs],
        )
        .unwrap();
    }

    /// The owner's call: everything, on every root, whether or not its drive
    /// is plugged in. A row that never learnt its size adds nothing.
    #[test]
    fn the_library_counts_every_root_plugged_in_or_not() {
        let conn = library();
        row(&conn, 1, "a.mp3", "audio", Some(5_000_000), Some(300.0));
        row(&conn, 2, "b.mp4", "video", Some(700_000_000), Some(1200.0));
        row(&conn, 1, "c.mp3", "audio", None, None);
        assert_eq!(library_bytes(&conn).unwrap(), 705_000_000);
    }

    /// Estimates come from what this library's files actually take, and only
    /// once there is enough of a kind to call it a rate.
    #[test]
    fn a_rate_is_measured_from_the_library_and_needs_ten_minutes_of_it() {
        let conn = library();
        row(
            &conn,
            1,
            "short.mp4",
            "video",
            Some(50_000_000),
            Some(120.0),
        );
        assert_eq!(rates(&conn).unwrap(), (None, None));

        row(&conn, 1, "a.mp3", "audio", Some(9_600_000), Some(400.0));
        row(&conn, 1, "b.mp3", "audio", Some(4_800_000), Some(200.0));
        row(
            &conn,
            1,
            "film.mp4",
            "video",
            Some(450_000_000),
            Some(1380.0),
        );
        let (audio, video) = rates(&conn).unwrap();
        assert_eq!(audio, Some(24_000.0));
        assert_eq!(video, Some(500_000_000.0 / 1500.0));
    }

    #[test]
    fn a_ceiling_is_a_positive_number_of_bytes_or_none() {
        let conn = library();
        assert_eq!(ceiling(&conn), None);
        set_ceiling(&conn, Some(250_000_000_000)).unwrap();
        assert_eq!(ceiling(&conn), Some(250_000_000_000));
        set_ceiling(&conn, None).unwrap();
        assert_eq!(ceiling(&conn), None);
        set_ceiling(&conn, Some(0)).unwrap();
        assert_eq!(ceiling(&conn), None);
        db::set_setting(&conn, CEILING_SETTING, "lots").unwrap();
        assert_eq!(ceiling(&conn), None);
    }
}
