//! Hurricane Party Planning: prep mode (#163, D140).
//!
//! A storm has a name and there are hours. A person pastes every link they
//! want, one per line, sees what it comes to against the drive, and presses
//! one button. Everything goes into the queue that already survives a power
//! cut (D10), through the same `jobs::enqueue` the library uses; a list still
//! becomes its own playlist (D114). One press is one batch, so its progress
//! can be shown as one thing and outlasts a restart like the jobs do.
//!
//! What is said, and how the lines add up, is `src/lib/prep.ts`. This is the
//! part that needs the database, the network, or the queue.

use crate::db::{self, Db, DbError};
use crate::jobs::{self, NewJob};
use crate::library::{self, Held};
use crate::pipeline::{self, PlaylistProbe};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// The pasted text, kept until it is queued: a crash halfway through pasting
/// forty links with a storm coming must not lose them.
pub const DRAFT_SETTING: &str = "prep.draft";

/// One pasted line, as far as it can be known without the network.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Line {
    /// Its line number in the text, from 1.
    pub n: usize,
    pub text: String,
    /// `video`, `list`, or `not_link`.
    pub kind: &'static str,
    /// The validated link, for a line that is one.
    pub url: Option<String>,
    /// The YouTube video it names, for telling two lines apart (#163).
    pub video_id: Option<String>,
    /// What the library already has of that video, by kind (D139).
    pub held: Held,
}

/// Sort pasted text into lines. Blank lines are skipped; everything else is
/// kept in order, so a line that is not a link says so where it is.
pub fn lines(conn: &Connection, text: &str) -> Vec<Line> {
    text.lines()
        .enumerate()
        .filter_map(|(i, raw)| {
            let t = raw.trim();
            if t.is_empty() {
                return None;
            }
            let Ok(url) = pipeline::validate_url(t) else {
                return Some(Line {
                    n: i + 1,
                    text: t.to_string(),
                    kind: "not_link",
                    url: None,
                    video_id: None,
                    held: Held::default(),
                });
            };
            let video_id = pipeline::youtube_video_id(&url);
            let held = video_id
                .as_deref()
                .map(|id| library::held(conn, id))
                .unwrap_or_default();
            Some(Line {
                n: i + 1,
                text: t.to_string(),
                kind: if pipeline::list_id_of(&url).is_some() {
                    "list"
                } else {
                    "video"
                },
                url: Some(url),
                video_id,
                held,
            })
        })
        .collect()
}

/// What reading a pasted list came to.
#[derive(Debug, Clone, Serialize)]
pub struct Read {
    /// `ok`, `offline` (tried again by the window until it reads), or `failed`.
    pub status: &'static str,
    pub list: Option<PlaylistProbe>,
    pub message: Option<String>,
}

/// Read a list without downloading it, the way the library does (D114, D119),
/// saying whether a failure was the connection.
pub async fn read(app: &AppHandle, url: &str) -> Read {
    match pipeline::probe_playlist(app, url).await {
        Ok(list) => Read {
            status: "ok",
            list: Some(list),
            message: None,
        },
        Err(e) if e.is_offline() => Read {
            status: "offline",
            list: None,
            message: Some("No connection. It reads the list again when there is one.".into()),
        },
        Err(e) => Read {
            status: "failed",
            list: None,
            message: Some(e.to_string()),
        },
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct GoOne {
    pub url: String,
    pub estimate_bytes: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GoList {
    pub name: String,
    pub entries: Vec<GoOne>,
}

/// Everything one press queues.
#[derive(Debug, Clone, Deserialize)]
pub struct Go {
    pub want_video: bool,
    pub singles: Vec<GoOne>,
    pub lists: Vec<GoList>,
    /// The pasted lines this press did not queue (a list still reading, a
    /// line that is not a link), kept as the draft in the same breath, so a
    /// crash straight after the press neither loses them nor queues the rest
    /// twice.
    #[serde(default)]
    pub remaining: String,
}

/// Queue a prep run: one batch, the single links first in the order they were
/// pasted, then each list into a playlist of its own. Returns the batch.
/// Pressing it queues; nothing waits on a download.
pub fn go(app: &AppHandle, run: &Go) -> Result<i64, String> {
    for one in run
        .singles
        .iter()
        .chain(run.lists.iter().flat_map(|l| &l.entries))
    {
        pipeline::validate_url(&one.url).map_err(|e| e.to_string())?;
    }
    let root = pipeline::library_root(app)
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    let (batch_id, playlists) = {
        let state = app.state::<Db>();
        let conn = state.0.lock().unwrap();
        conn.execute(
            "INSERT INTO batches (want_video, created_at) VALUES (?1, ?2)",
            params![run.want_video as i64, db::now()],
        )
        .map_err(|e| e.to_string())?;
        let batch_id = conn.last_insert_rowid();
        let mut playlists = Vec::new();
        for list in &run.lists {
            let name = list.name.trim();
            let name = if name.is_empty() { "Playlist" } else { name };
            playlists.push(crate::playlist::create(&conn, name).map_err(|e| e.to_string())?);
        }
        db::set_setting(&conn, DRAFT_SETTING, &run.remaining).map_err(|e| e.to_string())?;
        (batch_id, playlists)
    };
    let queue = |one: &GoOne, playlist_id: Option<i64>| {
        jobs::enqueue_with(
            app,
            NewJob {
                url: one.url.trim(),
                want_video: run.want_video,
                playlist_id,
                root: root.clone(),
                batch_id: Some(batch_id),
                estimate_bytes: one.estimate_bytes,
            },
        )
        .map_err(|e| e.to_string())
    };
    for one in &run.singles {
        queue(one, None)?;
    }
    for (list, playlist_id) in run.lists.iter().zip(playlists) {
        for one in &list.entries {
            queue(one, Some(playlist_id))?;
        }
    }
    if !run.lists.is_empty() {
        let _ = app.emit("library-changed", ());
    }
    // The radar loop fills beside the downloads, whatever the theme (D140).
    crate::radar::wake(app);
    Ok(batch_id)
}

/// A download of the batch that did not make it, and why.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Failure {
    pub id: i64,
    pub title: String,
    pub error: String,
}

/// A prep run's progress, as one thing.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Progress {
    pub id: i64,
    pub want_video: bool,
    pub created_at: i64,
    pub total: usize,
    pub done: usize,
    pub running: usize,
    pub queued: usize,
    pub paused: usize,
    pub failed: usize,
    /// Waiting for the connection to come back (D141).
    pub offline: usize,
    /// What has been written so far.
    pub written: i64,
    /// What the batch comes to: prep's estimate for each download, and a
    /// finished one's real size. A single link, which prep could not
    /// estimate, counts what its download reports as soon as it reports it.
    pub expected: i64,
    /// Downloads with neither yet: a single link, before its download starts.
    pub unknown: usize,
    pub failures: Vec<Failure>,
}

/// The progress of the latest prep run, if there has been one.
pub fn latest(conn: &Connection) -> Result<Option<Progress>, DbError> {
    let batch: Option<(i64, bool, i64)> = conn
        .query_row(
            "SELECT id, want_video, created_at FROM batches ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?)),
        )
        .ok();
    let Some((id, want_video, created_at)) = batch else {
        return Ok(None);
    };
    let mut p = Progress {
        id,
        want_video,
        created_at,
        total: 0,
        done: 0,
        running: 0,
        queued: 0,
        paused: 0,
        failed: 0,
        offline: 0,
        written: 0,
        expected: 0,
        unknown: 0,
        failures: Vec::new(),
    };
    let mut st = conn.prepare(
        "SELECT id, status, title, url, error, bytes_done, bytes_total, estimate_bytes, not_before
         FROM jobs WHERE batch_id = ?1 ORDER BY id",
    )?;
    type Row = (
        i64,
        String,
        Option<String>,
        String,
        Option<String>,
        i64,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    );
    let rows = st.query_map([id], |r| -> rusqlite::Result<Row> {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
            r.get(8)?,
        ))
    })?;
    for (job, status, title, url, error, done_bytes, total_bytes, estimate, not_before) in
        rows.filter_map(|r| r.ok())
    {
        p.total += 1;
        let real = total_bytes.filter(|b| *b > 0);
        match status.as_str() {
            "done" => {
                p.done += 1;
                p.written += real.or(estimate).unwrap_or(0);
            }
            "running" => {
                p.running += 1;
                p.written += done_bytes;
            }
            "paused" => {
                p.paused += 1;
                p.written += done_bytes;
            }
            "failed" => {
                p.failed += 1;
                p.failures.push(Failure {
                    id: job,
                    title: title.unwrap_or(url),
                    error: error.unwrap_or_default(),
                });
            }
            _ => {
                if not_before.is_some() {
                    p.offline += 1;
                } else {
                    p.queued += 1;
                }
            }
        }
        match (status.as_str(), real, estimate) {
            ("failed", _, _) => {}
            ("done", Some(b), _) | (_, _, Some(b)) | (_, Some(b), None) => p.expected += b,
            _ => p.unknown += 1,
        }
    }
    Ok(Some(p))
}

/// Whether a prep run still has downloads to make, which keeps the radar
/// loop filling whatever the theme (D140). A paused run is on hold, and a
/// failed download is not going anywhere.
pub fn precaching(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT 1 FROM jobs WHERE batch_id IS NOT NULL AND status IN ('queued', 'running') LIMIT 1",
        [],
        |_| Ok(()),
    )
    .is_ok()
}

/// Pause, resume, or retry what failed, across a whole prep run, the way the
/// list actions work on an import (D117). Returns how many jobs it changed.
pub fn act(app: &AppHandle, batch_id: i64, action: &str) -> Result<usize, String> {
    let statuses = match action {
        "pause" => "('queued', 'running')",
        "resume" => "('paused')",
        "retry" => "('failed')",
        other => return Err(format!("{other} is not something a prep run does")),
    };
    let ids: Vec<i64> = {
        let state = app.state::<Db>();
        let conn = state.0.lock().unwrap();
        let mut st = conn
            .prepare(&format!(
                "SELECT id FROM jobs WHERE batch_id = ?1 AND status IN {statuses} ORDER BY id"
            ))
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map([batch_id], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        rows.filter_map(|r| r.ok()).collect()
    };
    for id in &ids {
        match action {
            "pause" => jobs::pause(app, *id),
            "resume" => jobs::resume(app, *id),
            _ => jobs::retry(app, *id),
        }
        .map_err(|e| e.to_string())?;
    }
    Ok(ids.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(db::schema_for_tests()).unwrap();
        conn.execute(
            "INSERT INTO library_roots (id, label, path) VALUES (1, 'lib', 'C:/lib')",
            [],
        )
        .unwrap();
        conn
    }

    /// Each line says what it is where it is: a video, a list, or not a link,
    /// with what the library already has of a video it names.
    #[test]
    fn pasted_lines_are_sorted_where_they_stand() {
        let conn = conn();
        conn.execute(
            "INSERT INTO media (root_id, relpath, kind, title, added_at)
             VALUES (1, 'youtube/Song [dQw4w9WgXcQ].mp3', 'audio', 'Song', 0)",
            [],
        )
        .unwrap();
        let got = lines(
            &conn,
            "https://youtu.be/dQw4w9WgXcQ\n\n  https://www.youtube.com/playlist?list=PLabc  \nmy grocery list\nhttps://vimeo.com/1234567\n",
        );
        let kinds: Vec<(usize, &str)> = got.iter().map(|l| (l.n, l.kind)).collect();
        assert_eq!(
            kinds,
            [(1, "video"), (3, "list"), (4, "not_link"), (5, "video")]
        );
        assert_eq!(got[0].video_id.as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(
            got[0].held,
            Held {
                audio: true,
                video: false
            }
        );
        assert_eq!(got[3].video_id, None);
        assert_eq!(got[1].text, "https://www.youtube.com/playlist?list=PLabc");
    }

    fn job(
        conn: &Connection,
        id: i64,
        status: &str,
        bytes: (i64, Option<i64>),
        estimate: Option<i64>,
    ) {
        conn.execute(
            "INSERT INTO jobs (id, url, status, stage, title, error, bytes_done, bytes_total,
                               estimate_bytes, batch_id, created_at, updated_at)
             VALUES (?1, 'https://youtu.be/x', ?2, 'download', ?3, ?4, ?5, ?6, ?7, 1, 0, 0)",
            params![
                id,
                status,
                format!("track {id}"),
                (status == "failed").then_some("Video unavailable"),
                bytes.0,
                bytes.1,
                estimate
            ],
        )
        .unwrap();
    }

    /// One press is one thing: how many are saved, what has been written, what
    /// the whole comes to as it firms up, and what failed with its reason.
    #[test]
    fn a_prep_run_adds_up_as_one_thing() {
        let conn = conn();
        assert_eq!(latest(&conn).unwrap(), None);
        conn.execute(
            "INSERT INTO batches (id, want_video, created_at) VALUES (1, 0, 42)",
            [],
        )
        .unwrap();
        job(&conn, 1, "done", (5_000, Some(5_000)), Some(4_000)); // firmed to its real size
        job(&conn, 2, "running", (1_000, Some(3_000)), None); // a single link, now known
        job(&conn, 3, "queued", (0, None), Some(6_000)); // still prep's estimate
        job(&conn, 4, "queued", (0, None), None); // not known yet
        job(&conn, 5, "failed", (0, None), Some(9_000)); // left out of the total
        job(&conn, 6, "paused", (500, Some(2_000)), Some(1_500));
        conn.execute("UPDATE jobs SET not_before = 99 WHERE id = 4", [])
            .unwrap();

        let p = latest(&conn).unwrap().unwrap();
        assert_eq!(
            (p.total, p.done, p.running, p.queued, p.paused, p.failed, p.offline),
            (6, 1, 1, 1, 1, 1, 1)
        );
        assert_eq!(p.written, 5_000 + 1_000 + 500);
        assert_eq!(p.expected, 5_000 + 3_000 + 6_000 + 1_500);
        assert_eq!(p.unknown, 1);
        assert_eq!(
            p.failures,
            [Failure {
                id: 5,
                title: "track 5".into(),
                error: "Video unavailable".into()
            }]
        );
        assert!(precaching(&conn));
        conn.execute(
            "UPDATE jobs SET status = 'done' WHERE status IN ('queued', 'running')",
            [],
        )
        .unwrap();
        assert!(!precaching(&conn));
    }
}
