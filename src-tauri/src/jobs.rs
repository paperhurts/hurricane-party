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
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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
    /// The folder it was queued for (#154, D136); None is the app's own.
    pub download_root: Option<String>,
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
        download_root: r.get("download_root").unwrap_or(None),
    })
}

/// Wakes the runner when work arrives or a slot frees, so the loop isn't a
/// busy poll.
pub struct RunnerHandle {
    pub notify: Arc<Notify>,
    pub active: Arc<AtomicUsize>,
    /// The jobs whose run has not returned yet (#162). A paused job's row
    /// says `paused` the moment Pause is pressed, but its run is still
    /// unwinding the child it killed; changing what the job is before that
    /// run has returned would race it.
    pub in_flight: Arc<Mutex<HashSet<i64>>>,
}

impl Default for RunnerHandle {
    fn default() -> Self {
        Self {
            notify: Arc::new(Notify::new()),
            active: Arc::new(AtomicUsize::new(0)),
            in_flight: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

pub fn enqueue(
    app: &AppHandle,
    url: &str,
    want_video: bool,
    playlist_id: Option<i64>,
) -> Result<i64, DbError> {
    // The folder it is for is decided now (#154, D136): a person who picks
    // another folder later sends the downloads they queue after that there,
    // and this one still lands, or resumes, where it was going.
    let root = pipeline::library_root(app)
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    enqueue_in(app, url, want_video, playlist_id, root)
}

/// Queue a download for a given folder: the one a finished video is in, when
/// its audio is made from it (#162), so the audio path finds the video there
/// and extracts rather than fetching again (D80).
pub fn enqueue_in(
    app: &AppHandle,
    url: &str,
    want_video: bool,
    playlist_id: Option<i64>,
    root: Option<String>,
) -> Result<i64, DbError> {
    enqueue_with(
        app,
        NewJob {
            url,
            want_video,
            playlist_id,
            root,
            batch_id: None,
            estimate_bytes: None,
        },
    )
}

/// A download to queue, with everything a caller may know about it.
pub struct NewJob<'a> {
    pub url: &'a str,
    pub want_video: bool,
    pub playlist_id: Option<i64>,
    /// The folder it is for (D136).
    pub root: Option<String>,
    /// The prep run that queued it, and what prep estimated it would take
    /// (#163, D140).
    pub batch_id: Option<i64>,
    pub estimate_bytes: Option<i64>,
}

pub fn enqueue_with(app: &AppHandle, job: NewJob) -> Result<i64, DbError> {
    let id = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let t = db::now();
        conn.execute(
            "INSERT INTO jobs (url, want_video, want_audio, status, stage, playlist_id,
                               created_at, updated_at, download_root, batch_id, estimate_bytes)
             VALUES (?1, ?2, ?3, 'queued', 'probe', ?5, ?4, ?4, ?6, ?7, ?8)",
            params![
                job.url,
                job.want_video as i64,
                !job.want_video as i64,
                t,
                job.playlist_id,
                job.root,
                job.batch_id,
                job.estimate_bytes
            ],
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
         WHERE (j.status != 'done' OR j.updated_at > ?1)
           AND NOT (j.status = 'done' AND j.dismissed)
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

/// What the runner found in the queue.
enum Claim {
    Job(Box<Job>),
    /// Queued downloads, all for a folder that is not there right now.
    Waiting,
    Empty,
}

/// Which queued job to start, oldest first, among those whose folder is there
/// (#154, D136), and which are waiting on a folder that is not. A job with no
/// folder recorded was queued for the app's own.
fn first_ready(
    queued: &[(i64, Option<String>)],
    default_root: &Path,
    present: impl Fn(&Path) -> bool,
) -> (Option<i64>, Vec<(i64, PathBuf)>) {
    let mut waiting = Vec::new();
    for (id, root) in queued {
        let dir = root
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| default_root.to_path_buf());
        if present(&dir) {
            return (Some(*id), waiting);
        }
        waiting.push((*id, dir));
    }
    (None, waiting)
}

/// The words on a download that is waiting for its folder.
fn waiting_for(dir: &Path) -> String {
    format!(
        "Waiting for the download folder: {} is not there. It carries on when the folder is back, or pick another folder in the library.",
        dir.display()
    )
}

/// Claim one queued job whose folder is there, flipping it to `running` so
/// two runner iterations can't take the same row, and mark the ones ahead of
/// it that are waiting on a folder that is not.
fn claim_next(conn: &Connection, default_root: &Path) -> Result<Claim, DbError> {
    let all: Vec<(i64, Option<String>, Option<i64>)> = {
        let mut st = conn.prepare(
            "SELECT id, download_root, not_before FROM jobs WHERE status = 'queued' ORDER BY created_at",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    if all.is_empty() {
        return Ok(Claim::Empty);
    }
    // A download waiting out a lost connection is not tried before its time
    // (D141). The runner looks again every ten seconds while any wait.
    let now = db::now();
    let queued: Vec<(i64, Option<String>)> = all
        .into_iter()
        .filter(|(_, _, not_before)| not_before.is_none_or(|t| t <= now))
        .map(|(id, root, _)| (id, root))
        .collect();
    if queued.is_empty() {
        return Ok(Claim::Waiting);
    }
    let (ready, waiting) = first_ready(&queued, default_root, |p| p.is_dir());
    for (id, dir) in &waiting {
        conn.execute(
            "UPDATE jobs SET error = ?2 WHERE id = ?1 AND status = 'queued' AND IFNULL(error, '') != ?2",
            params![id, waiting_for(dir)],
        )?;
    }
    let Some(id) = ready else {
        return Ok(Claim::Waiting);
    };
    let t = db::now();
    let claimed: Option<i64> = conn
        .query_row(
            "UPDATE jobs SET status = 'running', updated_at = ?1, attempts = attempts + 1, error = NULL,
                             not_before = NULL
             WHERE id = ?2 AND status = 'queued'
             RETURNING id",
            params![t, id],
            |r| r.get(0),
        )
        .ok();
    match claimed {
        None => Ok(Claim::Empty),
        Some(id) => Ok(Claim::Job(Box::new(conn.query_row(
            "SELECT * FROM jobs WHERE id = ?1",
            [id],
            row_to_job,
        )?))),
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

/// How long a download that lost its connection waits before it tries again
/// (D141).
pub const OFFLINE_RETRY_SECS: i64 = 60;

/// The words on a download that is waiting for the connection to come back.
pub const OFFLINE_WAIT: &str =
    "No connection. It tries again every minute until there is one, and carries on from where it stopped.";

/// Put a download that failed for want of a connection back in the queue,
/// not to be tried for a minute (D141). A storm takes the connection for
/// hours; a download that failed for good would need a person to notice and
/// press Retry for each one, after the power is back.
fn hold_offline(conn: &Connection, id: i64) -> Result<(), DbError> {
    let now = db::now();
    conn.execute(
        "UPDATE jobs SET status = 'queued', error = ?2, not_before = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, OFFLINE_WAIT, now + OFFLINE_RETRY_SECS, now],
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
            "UPDATE jobs SET status = 'queued', error = NULL, not_before = NULL, updated_at = ?2 WHERE id = ?1",
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
            "UPDATE jobs SET status = 'queued', error = NULL, not_before = NULL, updated_at = ?2
             WHERE id = ?1 AND status = 'paused'",
            params![id, db::now()],
        )?;
    }
    app.state::<RunnerHandle>().notify.notify_one();
    let _ = app.emit("jobs-changed", ());
    Ok(())
}

/// Clear finished downloads from the Downloads list, before the five minutes
/// they would otherwise stay. Only a finished one: anything else still has
/// something to do, and its own Cancel or Dismiss. The row is kept, hidden.
pub fn dismiss(app: &AppHandle, ids: &[i64]) -> Result<usize, DbError> {
    let n = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        dismiss_in(&conn, ids)?
    };
    if n > 0 {
        let _ = app.emit("jobs-changed", ());
    }
    Ok(n)
}

fn dismiss_in(conn: &Connection, ids: &[i64]) -> Result<usize, DbError> {
    let mut n = 0;
    for id in ids {
        n += conn.execute(
            "UPDATE jobs SET dismissed = 1 WHERE id = ?1 AND status = 'done' AND dismissed = 0",
            [id],
        )?;
    }
    Ok(n)
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

/// What asking for a download to be audio only did (#162, D138).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AudioOnly {
    pub id: i64,
    /// `switched`, `already_audio`, `finished` (it landed first, as a video),
    /// or `gone` (cancelled, or never there).
    pub outcome: &'static str,
    /// What its video download had already written, named so the space can
    /// be got back by hand. Nothing here deletes them: a `.part` is kept on
    /// purpose (D26), and deleting stays a person's (D83).
    pub leftovers: Vec<Leftover>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Leftover {
    pub path: String,
    pub bytes: u64,
}

/// What `switch_idle` did, and for a job it switched, its video id and the
/// folder it was queued for, to look for what its video download left.
type Idle = (&'static str, Option<(String, Option<String>)>);

/// Change a job that is not running into an audio download. `None` when it
/// is running, which only its run can let go of.
fn switch_idle(conn: &Connection, id: i64) -> Result<Option<Idle>, DbError> {
    let row: Option<(String, bool, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT status, want_video, video_id, download_root FROM jobs WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?, r.get(3)?)),
        )
        .ok();
    let Some((status, want_video, video_id, root)) = row else {
        return Ok(Some(("gone", None)));
    };
    if !want_video {
        return Ok(Some(("already_audio", None)));
    }
    match status.as_str() {
        "done" => return Ok(Some(("finished", None))),
        "running" => return Ok(None),
        _ => {}
    }
    // Stage is kept, as a retry keeps it: the audio path works out what is
    // already on disk for itself (`import_job`), and a finished video there
    // is a source to extract from rather than something to fetch again (D80).
    let changed = conn.execute(
        "UPDATE jobs SET want_video = 0, want_audio = 1, updated_at = ?2
         WHERE id = ?1 AND want_video = 1 AND status IN ('queued', 'paused', 'failed')",
        params![id, db::now()],
    )?;
    if changed == 0 {
        // Claimed between the read and the write: it is running now.
        return Ok(None);
    }
    Ok(Some(("switched", video_id.map(|v| (v, root)))))
}

/// What a video download of `video_id` left under `root`: its `.part` files,
/// and the separate video and audio streams yt-dlp writes before it merges
/// them (`Song [id].f137.mp4`). Never the finished video, which the audio
/// download extracts from.
fn leftovers(root: &Path, video_id: &str) -> Vec<Leftover> {
    let marker = format!("[{video_id}]");
    walkdir::WalkDir::new(root)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let (_, after) = name.split_once(&marker)?;
            let fragment = after
                .strip_prefix(".f")
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
            (fragment || after.ends_with(".part")).then(|| Leftover {
                path: e.path().to_string_lossy().into_owned(),
                bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
            })
        })
        .collect()
}

fn in_flight(app: &AppHandle, id: i64) -> bool {
    app.state::<RunnerHandle>()
        .in_flight
        .lock()
        .unwrap()
        .contains(&id)
}

/// Make a video download an audio one (#162, D138): the way out a storage
/// warning offers. A running download is paused the way Pause pauses it, its
/// run is let finish unwinding, and then it is switched and resumed, so two
/// runs never write the same `.part` (D117). One that finished first stays a
/// video, and says so.
pub async fn make_audio(app: &AppHandle, id: i64) -> Result<AudioOnly, DbError> {
    let first = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        switch_idle(&conn, id)?
    };
    let (outcome, found, resumed) = match first {
        Some((outcome, found)) => (outcome, found, false),
        None => {
            pause(app, id)?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while in_flight(app, id) && std::time::Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            if in_flight(app, id) {
                return Err(DbError::Io(
                    "the download is paused, but did not stop in time to switch; press Audio only again"
                        .into(),
                ));
            }
            let second = {
                let db = app.state::<Db>();
                let conn = db.0.lock().unwrap();
                switch_idle(&conn, id)?
            };
            match second {
                Some((outcome, found)) => (outcome, found, outcome == "switched"),
                None => return Err(DbError::Io("the download would not stop".into())),
            }
        }
    };
    let leftovers = match found {
        Some((video_id, root)) => {
            let root = root
                .map(PathBuf::from)
                .or_else(|| pipeline::default_root(app).ok());
            root.map(|r| leftovers(&r, &video_id)).unwrap_or_default()
        }
        None => Vec::new(),
    };
    if resumed {
        resume(app, id)?;
    } else if outcome == "switched" {
        app.state::<RunnerHandle>().notify.notify_one();
        let _ = app.emit("jobs-changed", ());
    }
    Ok(AudioOnly {
        id,
        outcome,
        leftovers,
    })
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

    // The folder it landed in (#154): the app's own is "Default", a chosen one
    // is named after itself.
    let root = std::path::PathBuf::from(&track.root);
    let is_default = pipeline::default_root(app).is_ok_and(|d| d == root);
    let label = if is_default {
        "Default".to_string()
    } else {
        root.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Downloads".to_string())
    };
    let root_id = db::ensure_root(&conn, &label, &root.to_string_lossy())?;

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

    let root = match job.download_root.as_deref() {
        Some(r) => std::path::PathBuf::from(r),
        None => match pipeline::default_root(&app) {
            Ok(r) => r,
            Err(e) => {
                let db = app.state::<Db>();
                let conn = db.0.lock().unwrap();
                let _ = fail(&conn, job.id, &e.to_string());
                let _ = app.emit("jobs-changed", ());
                return;
            }
        },
    };
    let result = pipeline::import_job(&app, &job.url, job.id, job.want_video, &root).await;

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
            // Its folder may have just become a root (D136): watch it (#111).
            crate::watch::nudge(&app);
        }
        Err(e) => {
            let conn = db.0.lock().unwrap();
            // A killed child fails its stage; if a person paused or cancelled
            // the job, that is what happened, not an error (D117).
            if !stopped_on_purpose(&conn, job.id) {
                if e.is_offline() {
                    let _ = hold_offline(&conn, job.id);
                } else {
                    let _ = fail(&conn, job.id, &e.to_string());
                }
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
    let in_flight = handle.in_flight.clone();

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

            let default_root = pipeline::default_root(&app).ok();
            let claimed = {
                let db = app.state::<Db>();
                let conn = db.0.lock().unwrap();
                match &default_root {
                    Some(d) => claim_next(&conn, d).unwrap_or(Claim::Empty),
                    None => Claim::Empty,
                }
            };

            match claimed {
                Claim::Waiting => {
                    let _ = app.emit("jobs-changed", ());
                    // A drive coming back is not an event anything sends, so
                    // look again every so often, and at once when a person
                    // picks another folder or queues something.
                    tokio::select! {
                        _ = notify.notified() => {}
                        _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {}
                    }
                }
                Claim::Job(job) => {
                    let job = *job;
                    active.fetch_add(1, Ordering::SeqCst);
                    in_flight.lock().unwrap().insert(job.id);
                    let app2 = app.clone();
                    let active2 = active.clone();
                    let notify2 = notify.clone();
                    let in_flight2 = in_flight.clone();
                    tauri::async_runtime::spawn(async move {
                        let id = job.id;
                        run_one(app2, job).await;
                        in_flight2.lock().unwrap().remove(&id);
                        active2.fetch_sub(1, Ordering::SeqCst);
                        notify2.notify_one(); // a slot freed
                    });
                }
                Claim::Empty => {
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

    fn job(conn: &Connection, id: i64, status: &str, want_video: bool, video_id: Option<&str>) {
        conn.execute(
            "INSERT INTO jobs (id, url, want_video, want_audio, status, stage, video_id, created_at, updated_at)
             VALUES (?1, 'https://youtu.be/x', ?2, ?3, ?4, 'download', ?5, 0, 0)",
            params![id, want_video as i64, !want_video as i64, status, video_id],
        )
        .unwrap();
    }

    /// D141: a download that lost its connection waits out its minute in the
    /// queue, is not claimed before then, and is claimed after.
    #[test]
    fn a_download_without_a_connection_waits_its_minute_and_then_goes() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::schema_for_tests()).unwrap();
        let here = std::env::temp_dir();
        conn.execute(
            "INSERT INTO jobs (id, url, status, stage, created_at, updated_at, download_root)
             VALUES (1, 'https://youtu.be/x', 'running', 'download', 0, 0, ?1)",
            [here.to_string_lossy()],
        )
        .unwrap();
        hold_offline(&conn, 1).unwrap();
        let (status, error): (String, String) = conn
            .query_row("SELECT status, error FROM jobs WHERE id = 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((status.as_str(), error.as_str()), ("queued", OFFLINE_WAIT));
        assert!(matches!(claim_next(&conn, &here).unwrap(), Claim::Waiting));

        conn.execute(
            "UPDATE jobs SET not_before = ?1 WHERE id = 1",
            [db::now() - 1],
        )
        .unwrap();
        match claim_next(&conn, &here).unwrap() {
            Claim::Job(j) => assert_eq!((j.id, j.error), (1, None)),
            _ => panic!("the wait was over, and it was not claimed"),
        }
    }

    /// A finished download can be cleared from the list at once; one still
    /// under way cannot, and the row itself is kept.
    #[test]
    fn a_finished_download_is_cleared_from_the_list_and_kept() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::schema_for_tests()).unwrap();
        let t = db::now();
        for (id, status) in [(1, "done"), (2, "done"), (3, "queued")] {
            conn.execute(
                "INSERT INTO jobs (id, url, status, stage, created_at, updated_at)
                 VALUES (?1, 'https://youtu.be/x', ?2, 'verify', ?3, ?3)",
                params![id, status, t],
            )
            .unwrap();
        }
        assert_eq!(list(&conn).unwrap().len(), 3);
        assert_eq!(dismiss_in(&conn, &[1, 3, 99]).unwrap(), 1);
        let shown: Vec<i64> = list(&conn).unwrap().iter().map(|j| j.id).collect();
        assert_eq!(shown, [3, 2]);
        assert_eq!(dismiss_in(&conn, &[1]).unwrap(), 0);
        let kept: i64 = conn
            .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(kept, 3);
    }

    /// #162: a waiting, paused or failed video download becomes an audio
    /// one where it stands; a running one is left to its run, a finished one
    /// stays what it is, and asking twice is harmless.
    #[test]
    fn a_video_download_not_running_becomes_audio_where_it_stands() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::schema_for_tests()).unwrap();
        job(&conn, 1, "queued", true, None);
        job(&conn, 2, "paused", true, Some("abc123"));
        job(&conn, 3, "running", true, Some("run"));
        job(&conn, 4, "done", true, Some("done"));
        job(&conn, 5, "queued", false, None);

        assert_eq!(switch_idle(&conn, 1).unwrap(), Some(("switched", None)));
        assert_eq!(
            switch_idle(&conn, 2).unwrap(),
            Some(("switched", Some(("abc123".to_string(), None))))
        );
        assert_eq!(switch_idle(&conn, 3).unwrap(), None);
        assert_eq!(switch_idle(&conn, 4).unwrap(), Some(("finished", None)));
        assert_eq!(
            switch_idle(&conn, 5).unwrap(),
            Some(("already_audio", None))
        );
        assert_eq!(
            switch_idle(&conn, 1).unwrap(),
            Some(("already_audio", None))
        );
        assert_eq!(switch_idle(&conn, 99).unwrap(), Some(("gone", None)));

        let (want_video, want_audio, status): (i64, i64, String) = conn
            .query_row(
                "SELECT want_video, want_audio, status FROM jobs WHERE id = 2",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((want_video, want_audio, status.as_str()), (0, 1, "paused"));
    }

    /// The video's partial download is named, never the finished video the
    /// audio download will extract from, and never another video's files.
    #[test]
    fn what_a_video_download_left_is_its_parts_and_streams() {
        let root = std::env::temp_dir().join("hp-162-leftovers");
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("youtube");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, len) in [
            ("Song [abc123].f137.mp4", 7usize),
            ("Song [abc123].f251.webm.part", 3),
            ("Song [abc123].mp4.part", 2),
            ("Song [abc123].mp4", 11),
            ("Song [abc123].jpg", 1),
            ("Other [zzz999].f137.mp4", 5),
        ] {
            std::fs::write(dir.join(name), vec![0u8; len]).unwrap();
        }
        let mut got: Vec<(String, u64)> = leftovers(&root, "abc123")
            .into_iter()
            .map(|l| {
                let name = Path::new(&l.path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                (name, l.bytes)
            })
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                ("Song [abc123].f137.mp4".to_string(), 7),
                ("Song [abc123].f251.webm.part".to_string(), 3),
                ("Song [abc123].mp4.part".to_string(), 2),
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_download_waits_for_its_folder_and_the_rest_carry_on() {
        let default = Path::new("/app/library");
        let usb = "/mnt/usb/music".to_string();
        let there = |p: &Path| p != Path::new("/mnt/usb/music");
        // Oldest first: the one for the missing drive waits, the next runs.
        let queued = vec![(1, Some(usb.clone())), (2, None), (3, Some(usb.clone()))];
        let (ready, waiting) = first_ready(&queued, default, there);
        assert_eq!(ready, Some(2));
        assert_eq!(waiting, vec![(1, PathBuf::from(&usb))]);
        // Nothing can run: every one is waiting, and says for what.
        let (ready, waiting) = first_ready(&[(1, Some(usb.clone()))], default, there);
        assert_eq!(ready, None);
        assert_eq!(waiting.len(), 1);
        assert!(waiting_for(&waiting[0].1).contains("music"));
        // The drive is back.
        assert_eq!(first_ready(&queued, default, |_| true).0, Some(1));
    }

    #[test]
    fn claiming_skips_a_missing_folder_marks_it_and_clears_it_when_it_runs() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::db::schema_for_tests()).unwrap();
        let here = std::env::temp_dir();
        let gone = here.join("hp-154-no-such-drive");
        for (id, root) in [
            (1, gone.to_string_lossy().into_owned()),
            (2, here.to_string_lossy().into_owned()),
        ] {
            conn.execute(
                "INSERT INTO jobs (id, url, status, stage, created_at, updated_at, download_root)
                 VALUES (?1, 'https://example.com', 'queued', 'probe', ?1, ?1, ?2)",
                params![id, root],
            )
            .unwrap();
        }
        match claim_next(&conn, &here).unwrap() {
            Claim::Job(j) => assert_eq!(j.id, 2),
            _ => panic!("the job whose folder is there runs"),
        }
        let (status, error): (String, Option<String>) = conn
            .query_row("SELECT status, error FROM jobs WHERE id = 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(status, "queued");
        assert!(error
            .unwrap()
            .starts_with("Waiting for the download folder"));
        assert!(matches!(claim_next(&conn, &here).unwrap(), Claim::Waiting));
        // Its folder comes back: it runs, and the waiting words go.
        std::fs::create_dir_all(&gone).unwrap();
        match claim_next(&conn, &here).unwrap() {
            Claim::Job(j) => assert_eq!((j.id, j.error), (1, None)),
            _ => panic!("the job runs once its folder is back"),
        }
        std::fs::remove_dir_all(&gone).ok();
        assert!(matches!(claim_next(&conn, &here).unwrap(), Claim::Empty));
    }

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
