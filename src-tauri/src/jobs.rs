//! The durable job queue and its runner.
//!
//! The milestone this serves: kill the app mid-download, relaunch, and it
//! **resumes the bytes** (D26). That requires three things to survive the kill,
//! and all three are easy to break by accident:
//!
//!   1. the job row            — SQLite WAL (D10)
//!   2. the `.part` file       — never cleaned up on startup
//!   3. the output template    — deterministic, so `--continue` finds the part
//!
//! Miss any one and "resume" silently degrades to "restart", which still looks
//! like it works.

use crate::db::{self, Db, DbError};
use crate::pipeline::{self, Progress};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::process::CommandChild;
use tokio::sync::Notify;

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: i64,
    pub url: String,
    pub status: String,
    pub stage: String,
    pub title: Option<String>,
    pub video_id: Option<String>,
    pub progress: f64,
    pub bytes_done: i64,
    pub bytes_total: Option<i64>,
    pub error: Option<String>,
    pub attempts: i64,
    pub want_video: bool,
    /// The list this job was queued as part of (#137). Set once, at enqueue;
    /// the finished track is added to it, so closing the app mid-queue does
    /// not lose which playlist forty downloads were for.
    pub playlist_id: Option<i64>,
    /// That list's name, for the Downloads header that controls it (D117).
    pub playlist_name: Option<String>,
    pub created_at: i64,
}

fn row_to_job(r: &rusqlite::Row) -> rusqlite::Result<Job> {
    Ok(Job {
        id: r.get("id")?,
        url: r.get("url")?,
        status: r.get("status")?,
        stage: r.get("stage")?,
        title: r.get("title")?,
        video_id: r.get("video_id")?,
        progress: r.get("progress")?,
        bytes_done: r.get("bytes_done")?,
        bytes_total: r.get("bytes_total")?,
        error: r.get("error")?,
        attempts: r.get("attempts")?,
        want_video: r.get::<_, i64>("want_video")? != 0,
        playlist_id: r.get("playlist_id")?,
        // Only `list` joins the name in; a claimed job does not need it.
        playlist_name: r.get("playlist_name").unwrap_or(None),
        created_at: r.get("created_at")?,
    })
}

/// Wakes the runner when work arrives or a slot frees, so the loop isn't a
/// busy poll.
pub struct RunnerHandle {
    pub notify: Arc<Notify>,
    pub active: Arc<AtomicUsize>,
}

impl Default for RunnerHandle {
    fn default() -> Self {
        Self {
            notify: Arc::new(Notify::new()),
            active: Arc::new(AtomicUsize::new(0)),
        }
    }
}

pub fn enqueue(
    app: &AppHandle,
    url: &str,
    want_video: bool,
    playlist_id: Option<i64>,
) -> Result<i64, DbError> {
    let id = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let t = db::now();
        conn.execute(
            "INSERT INTO jobs (url, want_video, want_audio, status, stage, playlist_id,
                               created_at, updated_at)
             VALUES (?1, ?2, ?3, 'queued', 'probe', ?5, ?4, ?4)",
            params![url, want_video as i64, !want_video as i64, t, playlist_id],
        )?;
        conn.last_insert_rowid()
    };
    app.state::<RunnerHandle>().notify.notify_one();
    let _ = app.emit("jobs-changed", ());
    Ok(id)
}

pub fn list(conn: &Connection) -> Result<Vec<Job>, DbError> {
    let mut st = conn.prepare(
        "SELECT j.*, p.name AS playlist_name FROM jobs j
         LEFT JOIN playlists p ON p.id = j.playlist_id
         WHERE j.status != 'done' OR j.updated_at > ?1
         -- A paused job keeps a running job's place (D117): pressing Pause
         -- must not move the row out from under the pointer, or the Resume
         -- that replaces the button is somewhere else by the time anyone
         -- reaches for it.
         ORDER BY CASE j.status WHEN 'running' THEN 0 WHEN 'paused' THEN 0
                                WHEN 'queued' THEN 1 WHEN 'failed' THEN 2 ELSE 3 END,
                  j.created_at DESC
         LIMIT 200",
    )?;
    // Finished jobs stay visible for a few minutes so a completed download
    // doesn't vanish the instant it lands.
    let cutoff = db::now() - 300;
    let rows = st.query_map([cutoff], row_to_job)?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Claim one queued job, flipping it to `running` in the same statement so two
/// runner iterations can't take the same row.
fn claim_next(conn: &Connection) -> Result<Option<Job>, DbError> {
    let t = db::now();
    let claimed: Option<i64> = conn
        .query_row(
            "UPDATE jobs SET status = 'running', updated_at = ?1, attempts = attempts + 1
             WHERE id = (SELECT id FROM jobs WHERE status = 'queued'
                         ORDER BY created_at LIMIT 1)
             RETURNING id",
            [t],
            |r| r.get(0),
        )
        .ok();

    match claimed {
        None => Ok(None),
        Some(id) => Ok(Some(conn.query_row(
            "SELECT * FROM jobs WHERE id = ?1",
            [id],
            row_to_job,
        )?)),
    }
}

pub fn set_identity(
    conn: &Connection,
    id: i64,
    title: &str,
    video_id: &str,
) -> Result<(), DbError> {
    conn.execute(
        "UPDATE jobs SET title = ?2, video_id = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, title, video_id, db::now()],
    )?;
    Ok(())
}

pub fn set_progress(conn: &Connection, id: i64, p: &Progress) -> Result<(), DbError> {
    let pct = match p.bytes_total {
        Some(t) if t > 0 => p.bytes_done as f64 / t as f64,
        _ => 0.0,
    };
    conn.execute(
        "UPDATE jobs SET stage = ?2, bytes_done = ?3, bytes_total = ?4,
                         progress = ?5, updated_at = ?6 WHERE id = ?1",
        params![
            id,
            p.stage,
            p.bytes_done as i64,
            p.bytes_total.map(|v| v as i64),
            pct,
            db::now()
        ],
    )?;
    Ok(())
}

fn finish(conn: &Connection, id: i64) -> Result<(), DbError> {
    conn.execute(
        "UPDATE jobs SET status = 'done', stage = 'verify', progress = 1.0,
                         error = NULL, updated_at = ?2 WHERE id = ?1",
        params![id, db::now()],
    )?;
    Ok(())
}

fn fail(conn: &Connection, id: i64, msg: &str) -> Result<(), DbError> {
    conn.execute(
        "UPDATE jobs SET status = 'failed', error = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, msg, db::now()],
    )?;
    Ok(())
}

pub fn retry(app: &AppHandle, id: i64) -> Result<(), DbError> {
    {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        // Stage is preserved deliberately: a retry resumes where it died.
        conn.execute(
            "UPDATE jobs SET status = 'queued', error = NULL, updated_at = ?2 WHERE id = ?1",
            params![id, db::now()],
        )?;
    }
    app.state::<RunnerHandle>().notify.notify_one();
    let _ = app.emit("jobs-changed", ());
    Ok(())
}

/// The child process each running job is waiting on (D117).
///
/// A job runs one child at a time — yt-dlp to probe, yt-dlp to download,
/// ffmpeg to extract — and each stage puts its child here as it starts, so
/// Pause and Cancel have something to stop. Before this existed, "Pause" only
/// wrote a status: the download carried on underneath it and then marked
/// itself done.
#[derive(Default)]
pub struct Running(pub std::sync::Mutex<std::collections::HashMap<i64, CommandChild>>);

/// Stop a job's child and everything it started (D117). The whole tree, not
/// the one process: see `WindowPlatform::kill_tree` for the orphan that
/// `kill` alone leaves behind. Killing a child that has already exited is an
/// error nobody needs to see.
fn stop_child(app: &AppHandle, id: i64) {
    let child = app.state::<Running>().0.lock().unwrap().remove(&id);
    if let Some(c) = child {
        crate::platform::platform().kill_tree(c.pid());
        let _ = c.kill();
    }
}

/// Pause: stop the child and park the row (D117). The `.part` file stays
/// exactly where it is, which is the whole point — `resume` hands the job back
/// to the runner and `--continue` picks up the bytes (D26).
pub fn pause(app: &AppHandle, id: i64) -> Result<(), DbError> {
    {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'paused', updated_at = ?2
             WHERE id = ?1 AND status IN ('queued', 'running')",
            params![id, db::now()],
        )?;
    }
    stop_child(app, id);
    let _ = app.emit("jobs-changed", ());
    Ok(())
}

/// Resume a paused job where it stopped: its stage is kept, as a retry's is.
pub fn resume(app: &AppHandle, id: i64) -> Result<(), DbError> {
    {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'queued', error = NULL, updated_at = ?2
             WHERE id = ?1 AND status = 'paused'",
            params![id, db::now()],
        )?;
    }
    app.state::<RunnerHandle>().notify.notify_one();
    let _ = app.emit("jobs-changed", ());
    Ok(())
}

/// Cancel: stop the child and drop the row (D117). A finished job is not
/// cancellable — it already happened. Any `.part` stays on disk; the file name
/// carries the video id, so importing the same video again resumes those
/// bytes rather than orphaning them for good.
pub fn cancel(app: &AppHandle, id: i64) -> Result<(), DbError> {
    stop_child(app, id);
    {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.execute("DELETE FROM jobs WHERE id = ?1 AND status != 'done'", [id])?;
    }
    let _ = app.emit("jobs-changed", ());
    Ok(())
}

/// What a list-wide action applies to (D117).
#[derive(Clone, Copy)]
pub enum ListAction {
    Pause,
    Resume,
    Cancel,
}

/// The ids a list-wide action touches: everything of that import that has not
/// finished and that the action can change.
fn list_ids(conn: &Connection, playlist_id: i64, action: ListAction) -> Result<Vec<i64>, DbError> {
    let statuses = match action {
        ListAction::Pause => "('queued', 'running')",
        ListAction::Resume => "('paused')",
        ListAction::Cancel => "('queued', 'running', 'paused', 'failed')",
    };
    let mut st = conn.prepare(&format!(
        "SELECT id FROM jobs WHERE playlist_id = ?1 AND status IN {statuses} ORDER BY id"
    ))?;
    let rows = st.query_map([playlist_id], |r| r.get::<_, i64>(0))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Pause, resume or cancel every unfinished job of one playlist import in one
/// press (D117). A 40-video list is 40 rows, and a Pause button that flickers
/// past while each one runs is not a way to stop an import. Returns how many
/// jobs it changed.
pub fn apply_to_list(
    app: &AppHandle,
    playlist_id: i64,
    action: ListAction,
) -> Result<usize, DbError> {
    let ids = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        list_ids(&conn, playlist_id, action)?
    };
    for id in &ids {
        match action {
            ListAction::Pause => pause(app, *id)?,
            ListAction::Resume => resume(app, *id)?,
            ListAction::Cancel => cancel(app, *id)?,
        }
    }
    Ok(ids.len())
}

/// Whether a job that just ended was stopped on purpose rather than failing:
/// its row is gone (cancelled) or parked (paused). Either way the runner must
/// not write "failed" over what a person asked for.
fn stopped_on_purpose(conn: &Connection, id: i64) -> bool {
    match conn.query_row("SELECT status FROM jobs WHERE id = ?1", [id], |r| {
        r.get::<_, String>(0)
    }) {
        Ok(status) => status == "paused",
        Err(_) => true,
    }
}

/// Record a finished download in the library.
fn record_media(app: &AppHandle, track: &pipeline::Track) -> Result<i64, DbError> {
    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();

    let root = pipeline::library_root(app).map_err(|e| DbError::Io(e.to_string()))?;
    let root_id = db::ensure_root(&conn, "Default", &root.to_string_lossy())?;

    // D28: store the path relative to its root so a drive returning under a
    // different letter doesn't orphan every row.
    let relpath = std::path::Path::new(&track.path)
        .strip_prefix(&root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| track.path.clone());

    conn.execute(
        "INSERT INTO media (root_id, relpath, kind, title, uploader, duration_s,
                            container, filesize, added_at)
         VALUES (?1, ?2, ?8, ?3, ?4, ?5, ?9, ?6, ?7)
         ON CONFLICT(root_id, relpath) DO UPDATE SET
            title = excluded.title, uploader = excluded.uploader,
            duration_s = excluded.duration_s, filesize = excluded.filesize",
        params![
            root_id,
            relpath,
            track.title,
            track.uploader,
            track.duration_s,
            track.filesize as i64,
            db::now(),
            track.kind,
            std::path::Path::new(&track.path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mp3"),
        ],
    )?;
    // Read back rather than trusting `last_insert_rowid`: the statement above
    // is an upsert, and on the update path that id belongs to another row.
    let id = conn.query_row(
        "SELECT id FROM media WHERE root_id = ?1 AND relpath = ?2",
        params![root_id, relpath],
        |r| r.get::<_, i64>(0),
    )?;
    Ok(id)
}

/// Run one job to completion. Errors are recorded, never propagated — a failed
/// job must not take the runner down with it.
async fn run_one(app: AppHandle, job: Job) {
    let _ = app.emit("jobs-changed", ());

    let result = pipeline::import_job(&app, &job.url, job.id, job.want_video).await;

    let db = app.state::<Db>();
    match result {
        Ok(track) => {
            match record_media(&app, &track) {
                Err(e) => {
                    let conn = db.0.lock().unwrap();
                    let _ = fail(&conn, job.id, &format!("downloaded, but not recorded: {e}"));
                }
                Ok(media_id) => {
                    let conn = db.0.lock().unwrap();
                    // The list this job was queued for (#137). A track that
                    // lands twice is added once: `playlist::add` is the same
                    // call the library's own button makes.
                    if let Some(pid) = job.playlist_id {
                        if let Err(e) = crate::playlist::add(&conn, pid, media_id) {
                            eprintln!("job {}: downloaded, but not filed: {e}", job.id);
                        }
                    }
                    let _ = finish(&conn, job.id);
                }
            }
            let _ = app.emit("library-changed", ());
        }
        Err(e) => {
            let conn = db.0.lock().unwrap();
            // A killed child fails its stage; if a person paused or cancelled
            // the job, that is what happened, not an error (D117).
            if !stopped_on_purpose(&conn, job.id) {
                let _ = fail(&conn, job.id, &e.to_string());
            }
        }
    }
    app.state::<Running>().0.lock().unwrap().remove(&job.id);
    let _ = app.emit("jobs-changed", ());
}

/// The runner loop. One task, spawning up to `concurrency` jobs at a time.
pub fn spawn_runner(app: AppHandle) {
    let handle = app.state::<RunnerHandle>();
    let notify = handle.notify.clone();
    let active = handle.active.clone();

    tauri::async_runtime::spawn(async move {
        loop {
            let limit = {
                let db = app.state::<Db>();
                let conn = db.0.lock().unwrap();
                db::concurrency(&conn)
            };

            if active.load(Ordering::SeqCst) >= limit {
                notify.notified().await;
                continue;
            }

            let claimed = {
                let db = app.state::<Db>();
                let conn = db.0.lock().unwrap();
                claim_next(&conn).ok().flatten()
            };

            match claimed {
                Some(job) => {
                    active.fetch_add(1, Ordering::SeqCst);
                    let app2 = app.clone();
                    let active2 = active.clone();
                    let notify2 = notify.clone();
                    tauri::async_runtime::spawn(async move {
                        run_one(app2, job).await;
                        active2.fetch_sub(1, Ordering::SeqCst);
                        notify2.notify_one(); // a slot freed
                    });
                }
                None => {
                    // Nothing queued. Wake on enqueue, or poll slowly as a
                    // backstop so a missed notification can't wedge the queue.
                    tokio::select! {
                        _ = notify.notified() => {}
                        _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::schema_for_tests()).unwrap();
        conn.execute(
            "INSERT INTO playlists (id, name, created_at, position) VALUES (7, 'import', 0, 0)",
            [],
        )
        .unwrap();
        // One import in every state a job can be in, plus a stranger.
        for (id, status, pid) in [
            (1, "queued", Some(7)),
            (2, "running", Some(7)),
            (3, "paused", Some(7)),
            (4, "failed", Some(7)),
            (5, "done", Some(7)),
            (6, "queued", None),
        ] {
            conn.execute(
                "INSERT INTO jobs (id, url, status, stage, playlist_id, created_at, updated_at)
                 VALUES (?1, 'https://x', ?2, 'download', ?3, 0, 0)",
                params![id, status, pid],
            )
            .unwrap();
        }
        conn
    }

    #[test]
    fn a_list_action_touches_only_what_it_can_change_in_that_import() {
        let conn = fixture();
        assert_eq!(list_ids(&conn, 7, ListAction::Pause).unwrap(), [1, 2]);
        assert_eq!(list_ids(&conn, 7, ListAction::Resume).unwrap(), [3]);
        // Cancel clears everything unfinished, failed included; a finished
        // job already happened, and job 6 belongs to no list.
        assert_eq!(
            list_ids(&conn, 7, ListAction::Cancel).unwrap(),
            [1, 2, 3, 4]
        );
        assert!(list_ids(&conn, 99, ListAction::Cancel).unwrap().is_empty());
    }

    #[test]
    fn a_paused_or_cancelled_job_is_not_a_failure() {
        let conn = fixture();
        assert!(stopped_on_purpose(&conn, 3), "paused");
        conn.execute("DELETE FROM jobs WHERE id = 1", []).unwrap();
        assert!(stopped_on_purpose(&conn, 1), "cancelled: the row is gone");
        assert!(
            !stopped_on_purpose(&conn, 2),
            "still running means it really failed"
        );
        assert!(!stopped_on_purpose(&conn, 6));
    }
}
