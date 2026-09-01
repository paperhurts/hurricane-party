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

pub fn enqueue(app: &AppHandle, url: &str, want_video: bool) -> Result<i64, DbError> {
    let id = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let t = db::now();
        conn.execute(
            "INSERT INTO jobs (url, want_video, want_audio, status, stage, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'queued', 'probe', ?4, ?4)",
            params![url, want_video as i64, !want_video as i64, t],
        )?;
        conn.last_insert_rowid()
    };
    app.state::<RunnerHandle>().notify.notify_one();
    let _ = app.emit("jobs-changed", ());
    Ok(id)
}

pub fn list(conn: &Connection) -> Result<Vec<Job>, DbError> {
    let mut st = conn.prepare(
        "SELECT * FROM jobs
         WHERE status != 'done' OR updated_at > ?1
         ORDER BY CASE status WHEN 'running' THEN 0 WHEN 'queued' THEN 1
                              WHEN 'failed' THEN 2 ELSE 3 END, created_at DESC
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

pub fn cancel(app: &AppHandle, id: i64) -> Result<(), DbError> {
    {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'paused', updated_at = ?2 WHERE id = ?1",
            params![id, db::now()],
        )?;
    }
    let _ = app.emit("jobs-changed", ());
    Ok(())
}

/// Record a finished download in the library.
fn record_media(app: &AppHandle, track: &pipeline::Track) -> Result<(), DbError> {
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
    Ok(())
}

/// Run one job to completion. Errors are recorded, never propagated — a failed
/// job must not take the runner down with it.
async fn run_one(app: AppHandle, job: Job) {
    let _ = app.emit("jobs-changed", ());

    let result = pipeline::import_job(&app, &job.url, job.id, job.want_video).await;

    let db = app.state::<Db>();
    match result {
        Ok(track) => {
            if let Err(e) = record_media(&app, &track) {
                let conn = db.0.lock().unwrap();
                let _ = fail(&conn, job.id, &format!("downloaded, but not recorded: {e}"));
            } else {
                let conn = db.0.lock().unwrap();
                let _ = finish(&conn, job.id);
            }
            let _ = app.emit("library-changed", ());
        }
        Err(e) => {
            let conn = db.0.lock().unwrap();
            let _ = fail(&conn, job.id, &e.to_string());
        }
    }
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
