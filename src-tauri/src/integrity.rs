//! Integrity checking (#164, D142).
//!
//! `architecture.md` lists this among the constraints the app is built on:
//! "find the corrupt file the day before, not during". A download is
//! fingerprinted as it lands, and the library is read back quietly afterwards,
//! so a file that has changed, gone short or stopped opening is found while
//! there is still a connection to fix it with.
//!
//! Three results, kept apart:
//!
//!   - **missing** — the file is not there. That is #78's path already (the
//!     row stays, a rescan counts it, a person prunes it), so this marks
//!     nothing and says nothing new.
//!   - **changed** — the bytes are not the ones that were hashed. A retag in
//!     another program looks exactly like damage, so nothing is deleted and
//!     nothing is re-downloaded: a person accepts it with one press (the
//!     owner's call) or takes the row out.
//!   - **unreadable** — `lofty` cannot open it, or it has fallen short of the
//!     length the library recorded. No ffmpeg decode: that is a process per
//!     file and minutes per film (D50).
//!
//! The pass never touches a `.part` (D26): it reads only files that have a
//! `media` row.

use crate::db::{self, Db, DbError};
use lofty::file::AudioFile;
use lofty::probe::Probe;
use rusqlite::{params, Connection};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// How long after launch the quiet pass starts. The windows come up first and
/// the radar's own timer waits three seconds; this waits longer again, so
/// nothing a person does in the first moments waits on a disk read.
const START_DELAY: Duration = Duration::from_secs(20);
/// Between files in the quiet pass. A gap is what keeps the read off the head
/// of whatever is playing from the same drive.
const QUIET_GAP: Duration = Duration::from_millis(250);
/// Between files when a person asked for the check themselves.
const NOW_GAP: Duration = Duration::from_millis(20);
/// What one launch reads before it stops for the day: a large library is
/// covered over several launches rather than in one long read (D95's
/// complaint about launch answered by never being at launch, and by stopping).
const LAUNCH_BUDGET_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// How short is short: a file that lost its tail rather than a second of
/// rounding between what wrote it and what reads it.
const SHORT_ENOUGH: f64 = 0.9;
/// Read size. Small enough to interleave with anything else on the drive.
const CHUNK: usize = 256 * 1024;
/// How fast the quiet pass may read. A library holds films: the owner's has a
/// 17 GB one, and hashing it flat out is minutes of the drive at full tilt
/// under whatever is playing from it. At this rate that film takes about ten
/// minutes in the background and nobody hears it. A check a person asked for
/// reads as fast as the drive will go.
const QUIET_BYTES_A_SECOND: u64 = 30 * 1024 * 1024;

/// What one file came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// It is what it was: hashed for the first time, or the hash matched.
    Passed { sha256: String },
    /// Not on disk. #78's path, not this one's.
    Missing,
    /// The bytes are not the ones that were hashed.
    Changed,
    /// `lofty` will not open it, or it has fallen short.
    Unreadable(String),
}

/// The SHA-256 of a file, read a little at a time.
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    hash_file_at(path, None)
}

/// The same, holding the read to `limit` bytes a second when there is one, so
/// a long file cannot take the drive from whatever is playing.
pub fn hash_file_at(path: &Path, limit: Option<u64>) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let per_chunk = limit.map(|bytes| Duration::from_secs_f64(CHUNK as f64 / bytes as f64));
    loop {
        let started = std::time::Instant::now();
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        if let Some(pace) = per_chunk {
            // What the read itself took counts towards the pace: a slow drive
            // is already slow enough.
            if let Some(rest) = pace.checked_sub(started.elapsed()) {
                std::thread::sleep(rest);
            }
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as i64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Look at one file: is it there, does it still open, is it still as long as
/// the library says, and are its bytes the ones that were hashed.
pub fn examine(path: &Path, sha256: Option<&str>, duration_s: Option<f64>) -> Outcome {
    examine_at(path, sha256, duration_s, None)
}

/// The same, at a bounded reading pace (the quiet pass).
pub fn examine_at(
    path: &Path,
    sha256: Option<&str>,
    duration_s: Option<f64>,
    limit: Option<u64>,
) -> Outcome {
    if !path.is_file() {
        return Outcome::Missing;
    }
    let read = Probe::open(path).and_then(|p| p.read());
    let Ok(tagged) = read else {
        return Outcome::Unreadable("it will not open any more".into());
    };
    let now = tagged.properties().duration().as_secs_f64();
    if let Some(was) = duration_s.filter(|d| *d > 1.0) {
        if now > 0.0 && now < was * SHORT_ENOUGH {
            return Outcome::Unreadable(format!(
                "it is {} long now, where it was {}",
                clock(now),
                clock(was)
            ));
        }
    }
    match hash_file_at(path, limit) {
        Err(e) => Outcome::Unreadable(format!("it could not be read: {e}")),
        Ok(got) => match sha256 {
            Some(was) if was != got => Outcome::Changed,
            _ => Outcome::Passed { sha256: got },
        },
    }
}

/// A row to look at.
#[derive(Debug, Clone)]
struct Row {
    id: i64,
    path: PathBuf,
    sha256: Option<String>,
    duration_s: Option<f64>,
    size: i64,
}

/// The rows a pass should look at next: never one already marked, never one
/// on a drive that is not plugged in (D28), those never hashed first, then
/// the ones checked longest ago.
fn next_rows(conn: &Connection, limit: usize) -> Result<Vec<Row>, DbError> {
    let mut st = conn.prepare(
        "SELECT m.id, r.path AS root, m.relpath, m.sha256, m.duration_s, IFNULL(m.filesize, 0) AS size
         FROM media m JOIN library_roots r ON r.id = m.root_id
         WHERE m.integrity IS NULL
         ORDER BY (m.sha256 IS NOT NULL), IFNULL(m.verified_at, 0), m.id
         LIMIT ?1",
    )?;
    let rows = st.query_map([limit as i64], |r| {
        Ok((
            r.get::<_, i64>("id")?,
            r.get::<_, String>("root")?,
            r.get::<_, String>("relpath")?,
            r.get::<_, Option<String>>("sha256")?,
            r.get::<_, Option<f64>>("duration_s")?,
            r.get::<_, i64>("size")?,
        ))
    })?;
    Ok(rows
        .filter_map(|r| r.ok())
        .filter(|(_, root, ..)| Path::new(root).is_dir())
        .map(|(id, root, relpath, sha256, duration_s, size)| Row {
            id,
            path: Path::new(&root).join(relpath),
            sha256,
            duration_s,
            size,
        })
        .collect())
}

/// Write what a look came to. A pass that found nothing wrong moves the row's
/// `verified_at` on, which is also what puts it at the back of the queue.
fn record(conn: &Connection, id: i64, outcome: &Outcome) -> Result<(), DbError> {
    match outcome {
        Outcome::Passed { sha256 } => conn.execute(
            "UPDATE media SET sha256 = ?2, verified_at = ?3, integrity = NULL, integrity_note = NULL
             WHERE id = ?1",
            params![id, sha256, db::now()],
        )?,
        // Missing is #78's, not this pass's: the row is left exactly as it is.
        Outcome::Missing => 0,
        Outcome::Changed => conn.execute(
            "UPDATE media SET integrity = 'changed', integrity_note = ?2, verified_at = ?3 WHERE id = ?1",
            params![
                id,
                "its bytes are not the ones this app hashed. A retag looks like this too.",
                db::now()
            ],
        )?,
        Outcome::Unreadable(why) => conn.execute(
            "UPDATE media SET integrity = 'unreadable', integrity_note = ?2, verified_at = ?3 WHERE id = ?1",
            params![id, why, db::now()],
        )?,
    };
    Ok(())
}

/// What a pass did.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Report {
    pub looked: usize,
    pub passed: usize,
    pub failed: usize,
    pub missing: usize,
    /// True when it stopped on its budget with rows still to look at.
    pub more: bool,
}

/// How a pass is run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pace {
    /// After launch, in the background, until the budget is spent.
    Quiet,
    /// Everything, now, because a person asked (the day before a storm).
    Now,
}

/// Read the library back. Returns what it found.
pub async fn sweep(app: &AppHandle, pace: Pace) -> Report {
    let mut report = Report::default();
    let mut read = 0u64;
    loop {
        let rows = {
            let state = app.state::<Db>();
            let conn = state.0.lock().unwrap();
            match next_rows(&conn, 32) {
                Ok(rows) => rows,
                Err(e) => {
                    eprintln!("integrity: {e}");
                    return report;
                }
            }
        };
        if rows.is_empty() {
            return report;
        }
        for row in rows {
            if pace == Pace::Quiet && read >= LAUNCH_BUDGET_BYTES {
                report.more = true;
                return report;
            }
            // A download has the drive; the check can wait for it.
            while pace == Pace::Quiet && busy(app) {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            let path = row.path.clone();
            let (sha, duration) = (row.sha256.clone(), row.duration_s);
            let limit = (pace == Pace::Quiet).then_some(QUIET_BYTES_A_SECOND);
            let outcome = tauri::async_runtime::spawn_blocking(move || {
                examine_at(&path, sha.as_deref(), duration, limit)
            })
            .await
            .unwrap_or(Outcome::Unreadable("the check itself failed".into()));
            read += row.size.max(0) as u64;
            report.looked += 1;
            match &outcome {
                Outcome::Passed { .. } => report.passed += 1,
                Outcome::Missing => report.missing += 1,
                _ => report.failed += 1,
            }
            {
                let state = app.state::<Db>();
                let conn = state.0.lock().unwrap();
                let _ = record(&conn, row.id, &outcome);
            }
            if report.failed > 0 {
                let _ = app.emit("integrity:changed", ());
            }
            tokio::time::sleep(match pace {
                Pace::Quiet => QUIET_GAP,
                Pace::Now => NOW_GAP,
            })
            .await;
        }
        if pace == Pace::Now {
            let _ = app.emit("integrity:progress", report.clone());
        }
    }
}

/// Whether a download is running, so the quiet pass can leave the drive alone.
fn busy(app: &AppHandle) -> bool {
    let state = app.state::<Db>();
    let Ok(conn) = state.0.lock() else {
        return false;
    };
    conn.query_row(
        "SELECT 1 FROM jobs WHERE status = 'running' LIMIT 1",
        [],
        |_| Ok(()),
    )
    .is_ok()
}

/// The quiet pass: after launch, never at it (D95), and it stops for the day
/// when its budget is spent.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(START_DELAY).await;
        let report = sweep(&app, Pace::Quiet).await;
        if report.looked > 0 {
            eprintln!(
                "integrity: looked at {} file(s), {} failed{}",
                report.looked,
                report.failed,
                if report.more {
                    ", more next launch"
                } else {
                    ""
                }
            );
            let _ = app.emit("integrity:changed", ());
        }
    });
}

/// Fingerprint a download as it lands (#164): its hash, and a length that
/// matches what the site said it would be. A file that came down short is
/// marked where a person will see it rather than passing as good.
pub fn at_import(app: &AppHandle, media_id: i64, path: &Path, expected_s: Option<f64>) {
    let outcome = examine(path, None, expected_s);
    let state = app.state::<Db>();
    let Ok(conn) = state.0.lock() else { return };
    let _ = record(&conn, media_id, &outcome);
}

/// A row a check marked, for the library's line.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Failed {
    pub id: i64,
    pub title: String,
    pub state: String,
    pub note: Option<String>,
    pub path: String,
    /// The link it came from, when a finished download names it: the offer to
    /// fetch it again (the owner's call).
    pub url: Option<String>,
}

pub fn failures(conn: &Connection) -> Result<Vec<Failed>, DbError> {
    let mut st = conn.prepare(
        "SELECT m.id, m.title, m.integrity, m.integrity_note, r.path AS root, m.relpath,
                (SELECT j.url FROM jobs j
                  WHERE j.video_id IS NOT NULL AND m.relpath LIKE '%[' || j.video_id || ']%'
                  ORDER BY j.id DESC LIMIT 1) AS url
         FROM media m JOIN library_roots r ON r.id = m.root_id
         WHERE m.integrity IS NOT NULL ORDER BY m.id",
    )?;
    let rows = st.query_map([], |r| {
        let root: String = r.get("root")?;
        let relpath: String = r.get("relpath")?;
        Ok(Failed {
            id: r.get("id")?,
            title: r.get("title")?,
            state: r.get("integrity")?,
            note: r.get("integrity_note")?,
            path: Path::new(&root)
                .join(relpath)
                .to_string_lossy()
                .into_owned(),
            url: r.get("url")?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Take a file as it is now: hash it again and clear the mark. What a person
/// presses when they retagged it themselves (the owner's call).
pub fn accept(conn: &Connection, id: i64) -> Result<(), DbError> {
    let path: Option<String> = conn
        .query_row(
            "SELECT r.path || '/' || m.relpath FROM media m
             JOIN library_roots r ON r.id = m.root_id WHERE m.id = ?1",
            [id],
            |r| r.get(0),
        )
        .ok();
    let Some(path) = path else {
        return Ok(());
    };
    match hash_file(Path::new(&path)) {
        Ok(sha) => record(conn, id, &Outcome::Passed { sha256: sha }),
        Err(e) => record(
            conn,
            id,
            &Outcome::Unreadable(format!("it could not be read: {e}")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real, tiny sound file: PCM, mono, 8 kHz, so `lofty` opens it and
    /// reads a length from it, which is what half the check is about.
    fn wav(seconds: f64, fill: u8) -> Vec<u8> {
        let rate = 8_000u32;
        let samples = (rate as f64 * seconds) as u32;
        let mut v = Vec::with_capacity(44 + samples as usize);
        v.extend(b"RIFF");
        v.extend((36 + samples).to_le_bytes());
        v.extend(b"WAVEfmt ");
        v.extend(16u32.to_le_bytes());
        v.extend(1u16.to_le_bytes()); // PCM
        v.extend(1u16.to_le_bytes()); // mono
        v.extend(rate.to_le_bytes());
        v.extend(rate.to_le_bytes()); // bytes a second
        v.extend(1u16.to_le_bytes()); // block align
        v.extend(8u16.to_le_bytes()); // bits a sample
        v.extend(b"data");
        v.extend(samples.to_le_bytes());
        v.extend(std::iter::repeat_n(fill, samples as usize));
        v
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hp-164-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn library(dir: &Path) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(db::schema_for_tests()).unwrap();
        conn.execute(
            "INSERT INTO library_roots (id, label, path) VALUES (1, 'lib', ?1),
                                                               (2, 'drive', 'Z:/not/plugged/in')",
            [dir.to_string_lossy()],
        )
        .unwrap();
        conn
    }

    fn row(
        conn: &Connection,
        id: i64,
        root: i64,
        rel: &str,
        sha: Option<&str>,
        verified: Option<i64>,
    ) {
        conn.execute(
            "INSERT INTO media (id, root_id, relpath, kind, title, sha256, verified_at, added_at)
             VALUES (?1, ?2, ?3, 'audio', ?3, ?4, ?5, 0)",
            params![id, root, rel, sha, verified],
        )
        .unwrap();
    }

    /// The same bytes hash the same, and a file that is not there is missing,
    /// which is #78's business and not a failure here.
    #[test]
    fn a_file_is_what_it_was_until_its_bytes_change() {
        let dir = scratch("hash");
        let f = dir.join("song.wav");
        std::fs::write(&f, wav(30.0, 7)).unwrap();
        let first = hash_file(&f).unwrap();
        assert_eq!(first.len(), 64);
        // Held to a pace, it is the same file and the same hash.
        assert_eq!(hash_file_at(&f, Some(1024 * 1024)).unwrap(), first);
        // Never hashed: this is its fingerprint. Hashed before: it matches.
        assert_eq!(
            examine(&f, None, Some(30.0)),
            Outcome::Passed {
                sha256: first.clone()
            }
        );
        assert_eq!(
            examine(&f, Some(&first), Some(30.0)),
            Outcome::Passed {
                sha256: first.clone()
            }
        );

        // Retagged, or damaged: it opens, it is as long as it was, and its
        // bytes are not the ones that were hashed.
        std::fs::write(&f, wav(30.0, 9)).unwrap();
        assert_eq!(examine(&f, Some(&first), Some(30.0)), Outcome::Changed);
        assert_eq!(
            examine(&dir.join("gone.wav"), Some(&first), None),
            Outcome::Missing
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file that will not open, and one that has lost its tail, are both
    /// unreadable, and each says which.
    #[test]
    fn a_file_that_will_not_open_or_has_gone_short_says_so() {
        let dir = scratch("short");
        let f = dir.join("not-really.mp3");
        std::fs::write(&f, b"this is not an mp3").unwrap();
        match examine(&f, None, None) {
            Outcome::Unreadable(why) => assert!(why.contains("will not open"), "{why}"),
            other => panic!("{other:?}"),
        }

        // The download that came down short: it opens, and it is a minute
        // where the library recorded three.
        let cut = dir.join("cut.wav");
        std::fs::write(&cut, wav(60.0, 3)).unwrap();
        match examine(&cut, None, Some(180.0)) {
            Outcome::Unreadable(why) => assert_eq!(why, "it is 1:00 long now, where it was 3:00"),
            other => panic!("{other:?}"),
        }
        // A second of rounding between what wrote it and what reads it is
        // not short.
        assert!(matches!(
            examine(&cut, None, Some(61.0)),
            Outcome::Passed { .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Never a row already marked, never a drive that is not plugged in
    /// (D28), those never hashed first, then the ones checked longest ago.
    #[test]
    fn the_pass_takes_the_never_hashed_first_and_leaves_an_unplugged_drive_alone() {
        let dir = scratch("order");
        let conn = library(&dir);
        row(&conn, 1, 1, "old.mp3", Some("aa"), Some(10));
        row(&conn, 2, 1, "never.mp3", None, None);
        row(&conn, 3, 1, "fresh.mp3", Some("bb"), Some(999));
        row(&conn, 4, 2, "on-the-drive.mp3", None, None);
        row(&conn, 5, 1, "marked.mp3", Some("cc"), Some(1));
        conn.execute("UPDATE media SET integrity = 'changed' WHERE id = 5", [])
            .unwrap();

        let ids: Vec<i64> = next_rows(&conn, 10).unwrap().iter().map(|r| r.id).collect();
        assert_eq!(ids, [2, 1, 3]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What each result writes: a pass moves the row on, a failure is kept on
    /// it with its reason, and accepting takes the file as it now is.
    #[test]
    fn a_failure_stays_on_the_row_until_it_is_accepted() {
        let dir = scratch("record");
        let conn = library(&dir);
        let f = dir.join("song.mp3");
        std::fs::write(&f, b"bytes").unwrap();
        row(&conn, 1, 1, "song.mp3", Some("nottherightone"), Some(1));

        record(&conn, 1, &Outcome::Changed).unwrap();
        let failed = failures(&conn).unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].state, "changed");
        assert!(failed[0].note.as_deref().unwrap().contains("retag"));
        assert!(failed[0].path.ends_with("song.mp3"));

        accept(&conn, 1).unwrap();
        assert!(failures(&conn).unwrap().is_empty());
        let (sha, integrity): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT sha256, integrity FROM media WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(sha.as_deref(), Some(hash_file(&f).unwrap().as_str()));
        assert_eq!(integrity, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
