//! v0.1 import pipeline: URL -> audio stream -> MP3 on disk.
//!
//! Two phases, per architecture.md: probe first (no download), then fetch.
//! The probe is not just politeness — it yields the video id, which lets the
//! fetch write to a deterministic path. That means finding the downloaded file
//! afterwards is a directory scan rather than scraping yt-dlp's stdout for it.
//!
//! Sidecars, all bundled (D46/D47/D48):
//!   yt-dlp — the official standalone exe
//!   deno   — JS runtime for yt-dlp's EJS challenges, passed via --js-runtimes
//!   ffmpeg — MP3 extraction (D3)

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("{0}")]
    Sidecar(String),
    #[error("yt-dlp failed ({code}). Last output: {tail}")]
    YtDlp { code: i32, tail: String },
    #[error("ffmpeg failed ({code}). Last output: {tail}")]
    Ffmpeg { code: i32, tail: String },
    #[error("couldn't understand yt-dlp's metadata: {0}")]
    Metadata(String),
    #[error("downloaded, but no file landed at {0}")]
    MissingOutput(String),
    #[error("{0}")]
    BadUrl(String),
    #[error("{0}")]
    Io(String),
}

// Fully qualified: the `Result<T>` alias below shadows std's in this module.
impl serde::Serialize for PipelineError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

type Result<T> = std::result::Result<T, PipelineError>;

/// What the probe found. Shown to the user before anything is downloaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Probed {
    pub id: String,
    pub title: String,
    pub uploader: Option<String>,
    pub duration_s: Option<f64>,
    pub extractor: String,
    /// Best-effort estimate; yt-dlp often can't know before downloading.
    pub filesize_approx: Option<u64>,
}

/// A finished track, ready to play.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub uploader: Option<String>,
    pub duration_s: Option<f64>,
    /// Absolute path to the MP3. The frontend turns this into an asset: URL.
    pub path: String,
    pub filesize: u64,
}

/// Progress for the UI. `stage` distinguishes the two sidecars, which matters
/// because they fail differently and D26 will need to know which one died.
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub url: String,
    pub stage: &'static str, // "probe" | "download" | "extract" | "done"
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub speed_bps: Option<f64>,
    pub eta_s: Option<u64>,
    pub note: Option<String>,
}

/// Briefly borrow the connection. The guard is bound before use so it isn't a
/// temporary in scrutinee position, and it is always dropped before any await.
fn with_db<T>(app: &AppHandle, f: impl FnOnce(&rusqlite::Connection) -> T) -> Option<T> {
    let state = app.state::<crate::db::Db>();
    let guard = state.0.lock().ok()?;
    Some(f(&guard))
}

/// Emit to the UI and, when this belongs to a queued job, persist to the row.
/// The DB write is what survives a kill; the event is just what makes the
/// window move.
fn emit(app: &AppHandle, job_id: Option<i64>, p: Progress) {
    if let Some(id) = job_id {
        with_db(app, |conn| crate::jobs::set_progress(conn, id, &p).ok());
    }
    let _ = app.emit("job-progress", p);
}

/// Where downloads land. Configurable is the rule (CLAUDE.md) — v0.1 has no
/// settings store yet (that's D32, v0.2), so this is the default, not a constant
/// baked into call sites. It sits under APPDATA to stay inside the asset
/// protocol scope declared in tauri.conf.json.
pub fn library_root(app: &AppHandle) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| PipelineError::Io(format!("no app data dir: {e}")))?
        .join("library");
    std::fs::create_dir_all(&dir)
        .map_err(|e| PipelineError::Io(format!("couldn't create {}: {e}", dir.display())))?;
    Ok(dir)
}

/// Keep the last few lines of stderr so a failure can say what actually broke
/// rather than just surfacing an exit code.
struct Tail(Vec<String>);
impl Tail {
    fn new() -> Self {
        Tail(Vec::new())
    }
    fn push(&mut self, line: String) {
        if line.trim().is_empty() {
            return;
        }
        self.0.push(line);
        if self.0.len() > 12 {
            self.0.remove(0);
        }
    }
    fn text(&self) -> String {
        let t = self.0.join(" / ");
        if t.is_empty() {
            "(no output)".into()
        } else {
            t
        }
    }
}

/// Path to the bundled ffmpeg, which Tauri places beside the main executable.
///
/// yt-dlp needs ffmpeg itself for `--embed-metadata` and `--convert-thumbnails`.
/// Without this it searches PATH — which happens to work on a dev box with
/// ffmpeg installed, and silently doesn't on a clean machine. That is exactly
/// the failure this app cannot have the week before a storm.
fn bundled_ffmpeg() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    for name in ["ffmpeg.exe", "ffmpeg"] {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Args every yt-dlp invocation needs.
///
/// `--js-runtimes deno` is D46. Without a JS runtime, yt-dlp warns that
/// "YouTube extraction without a JS runtime has been deprecated" and silently
/// returns fewer formats — a degradation that looks like success.
fn ytdlp_base() -> Vec<String> {
    vec![
        "--js-runtimes".into(),
        "deno".into(),
        "--no-playlist".into(),
        "--no-warnings".into(),
    ]
}

/// Reject anything that isn't a plain http(s) URL, before it reaches argv.
///
/// This is not paranoia about the user attacking themselves. yt-dlp's flag
/// surface includes `--exec` (run an arbitrary command per download),
/// `--config-location` (load a config file that may itself contain `--exec`)
/// and `--batch-file`. "Paste this link to get the song" is an entirely
/// ordinary thing for someone to be told, and a media downloader is precisely
/// the app where people paste a string without reading it. A value beginning
/// with `-` would be parsed as options, not as a URL.
///
/// Belt and braces: this check, plus a `--` terminator at every call site so
/// yt-dlp stops option parsing before the URL regardless.
pub(crate) fn validate_url(raw: &str) -> Result<String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(PipelineError::BadUrl("no URL given".into()));
    }
    let parsed = url::Url::parse(s)
        .map_err(|_| PipelineError::BadUrl(format!("{s:?} isn't a URL")))?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed.to_string()),
        other => Err(PipelineError::BadUrl(format!(
            "only http and https are supported, not {other:?}"
        ))),
    }
}

/// Phase 1 — probe. No download. Cheap enough to run on paste.
pub async fn probe(app: &AppHandle, url: &str, job_id: Option<i64>) -> Result<Probed> {
    let url = &validate_url(url)?;
    emit(
        app,
        job_id,
        Progress {
            url: url.into(),
            stage: "probe",
            bytes_done: 0,
            bytes_total: None,
            speed_bps: None,
            eta_s: None,
            note: Some("asking yt-dlp what this is".into()),
        },
    );

    let mut args = ytdlp_base();
    // `--` ends option parsing: everything after it is a positional argument.
    args.extend(["-J".to_string(), "--flat-playlist".into(), "--".into(), url.to_string()]);

    let (mut rx, _child) = app
        .shell()
        .sidecar("yt-dlp")
        .map_err(|e| PipelineError::Sidecar(format!("yt-dlp sidecar missing: {e}")))?
        .args(args)
        .spawn()
        .map_err(|e| PipelineError::Sidecar(format!("couldn't start yt-dlp: {e}")))?;

    let mut json = String::new();
    let mut tail = Tail::new();
    let mut code = 0;

    while let Some(ev) = rx.recv().await {
        match ev {
            CommandEvent::Stdout(b) => json.push_str(&String::from_utf8_lossy(&b)),
            CommandEvent::Stderr(b) => tail.push(String::from_utf8_lossy(&b).trim().to_string()),
            CommandEvent::Terminated(p) => code = p.code.unwrap_or(-1),
            _ => {}
        }
    }
    if code != 0 {
        return Err(PipelineError::YtDlp { code, tail: tail.text() });
    }

    let v: serde_json::Value =
        serde_json::from_str(json.trim()).map_err(|e| PipelineError::Metadata(e.to_string()))?;

    // A playlist URL yields entries[]; v0.1 takes the first and moves on.
    // Selecting from the list is v0.2's job (architecture.md phase 1).
    let node = v
        .get("entries")
        .and_then(|e| e.as_array())
        .and_then(|a| a.first())
        .unwrap_or(&v);

    let id = node
        .get("id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| PipelineError::Metadata("no id in yt-dlp output".into()))?
        .to_string();

    Ok(Probed {
        title: node
            .get("title")
            .and_then(|x| x.as_str())
            .unwrap_or(&id)
            .to_string(),
        uploader: node
            .get("uploader")
            .and_then(|x| x.as_str())
            .map(str::to_string),
        duration_s: node.get("duration").and_then(|x| x.as_f64()),
        extractor: node
            .get("extractor_key")
            .or_else(|| node.get("extractor"))
            .and_then(|x| x.as_str())
            .unwrap_or("unknown")
            .to_string(),
        filesize_approx: node
            .get("filesize_approx")
            .and_then(|x| x.as_u64())
            .or_else(|| node.get("filesize").and_then(|x| x.as_u64())),
        id,
    })
}

/// One `--progress-template` line. Pipe-delimited and parsed positionally —
/// never scrape the human-readable bar (D4), which changes between releases.
fn parse_progress(line: &str) -> Option<(u64, Option<u64>, Option<f64>, Option<u64>, String)> {
    let rest = line.strip_prefix("HPPROG|")?;
    let f: Vec<&str> = rest.split('|').collect();
    if f.len() < 5 {
        return None;
    }
    // yt-dlp emits "NA" for anything it doesn't know yet.
    let num = |s: &str| s.trim().parse::<f64>().ok();
    Some((
        num(f[0]).unwrap_or(0.0) as u64,
        num(f[1]).map(|v| v as u64),
        num(f[2]),
        num(f[3]).map(|v| v as u64),
        f[4].trim().to_string(),
    ))
}

/// Phase 2 — fetch the best audio stream to a deterministic path.
async fn download_audio(app: &AppHandle, url: &str, probed: &Probed, job_id: Option<i64>) -> Result<PathBuf> {
    let root = library_root(app)?;
    // Grouped by extractor, flat within it (D49).
    //
    // architecture.md's template also nested by `%(uploader)s`, which was
    // dropped: on YouTube the uploader is the *channel*, not the artist, so it
    // produced roughly one folder per file for no navigational gain. It is
    // still captured on `media.uploader`, so nothing is lost — and the app's
    // own browser is a DB query (O6), not a directory listing.
    let outtmpl = root.join("%(extractor)s/%(title)s [%(id)s].%(ext)s");

    let mut args = ytdlp_base();
    // Point yt-dlp at our ffmpeg rather than letting it search PATH.
    if let Some(ff) = bundled_ffmpeg() {
        args.extend(["--ffmpeg-location".to_string(), ff.to_string_lossy().into_owned()]);
    }
    args.extend([
        // Audio only. D3 forbids downloading twice; for an audio-only request
        // architecture.md sanctions skipping video entirely rather than pulling
        // a video stream we'd immediately discard.
        "-f".to_string(),
        "bestaudio/best".into(),
        // Without these the MP3 has no tags at all and every media library on
        // the machine shows a blank Title/Artist. architecture.md asked for
        // them; the first cut dropped them.
        "--embed-metadata".into(),
        // NOT --embed-thumbnail: the intermediate is webm/opus, and yt-dlp
        // hard-errors ("Supported filetypes for thumbnail embedding are: mp3,
        // mkv/mka, ogg/opus/flac, m4a/mp4/m4v/mov") which fails the whole job
        // after a successful download. Write the art out and attach it at the
        // ffmpeg step, where the container is MP3 and supports it.
        "--write-thumbnail".into(),
        "--convert-thumbnails".into(),
        "jpg".into(),
        "--continue".into(), // D26: resume, don't restart
        "--newline".into(),  // without this every progress line concatenates
        "--progress-template".into(),
        "download:HPPROG|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|\
         %(progress.speed)s|%(progress.eta)s|%(progress.status)s"
            .into(),
        "-o".into(),
        outtmpl.to_string_lossy().to_string(),
        "--".into(),
        url.to_string(),
    ]);

    let (mut rx, _child) = app
        .shell()
        .sidecar("yt-dlp")
        .map_err(|e| PipelineError::Sidecar(format!("yt-dlp sidecar missing: {e}")))?
        .args(args)
        .spawn()
        .map_err(|e| PipelineError::Sidecar(format!("couldn't start yt-dlp: {e}")))?;

    let mut tail = Tail::new();
    let mut code = 0;
    // yt-dlp emits progress many times per second. The UI can absorb that, but
    // persisting each one is a SQLite write per tick for every concurrent job.
    // Throttle to ~4Hz; the final state is always written on completion below.
    let mut last_persist = std::time::Instant::now() - std::time::Duration::from_secs(1);

    while let Some(ev) = rx.recv().await {
        match ev {
            CommandEvent::Stdout(b) => {
                let chunk = String::from_utf8_lossy(&b);
                for line in chunk.lines() {
                    if let Some((done, total, speed, eta, _status)) = parse_progress(line) {
                        let persist = last_persist.elapsed()
                            >= std::time::Duration::from_millis(250);
                        if persist {
                            last_persist = std::time::Instant::now();
                        }
                        emit(
                            app,
                            if persist { job_id } else { None },
                            Progress {
                                url: url.into(),
                                stage: "download",
                                bytes_done: done,
                                bytes_total: total,
                                speed_bps: speed,
                                eta_s: eta,
                                note: None,
                            },
                        );
                    }
                }
            }
            CommandEvent::Stderr(b) => tail.push(String::from_utf8_lossy(&b).trim().to_string()),
            CommandEvent::Terminated(p) => code = p.code.unwrap_or(-1),
            _ => {}
        }
    }
    if code != 0 {
        return Err(PipelineError::YtDlp { code, tail: tail.text() });
    }

    // Deterministic path is why the probe ran first: scan for <id>.* rather
    // than parsing the filename back out of yt-dlp's chatter.
    find_by_id(&root, &probed.id)
        .ok_or_else(|| PipelineError::MissingOutput(format!("{} [{}]", root.display(), probed.id)))
}

/// Locate a downloaded file by the `[id]` yt-dlp writes into its name.
///
/// Recursive, because the template nests by extractor and uploader. Skips
/// `.part` files — a partial is not a result, it is the thing `--continue`
/// will finish.
fn find_by_id(root: &Path, id: &str) -> Option<PathBuf> {
    let marker = format!("[{id}]");
    fn walk(dir: &Path, marker: &str, depth: usize) -> Option<PathBuf> {
        if depth > 6 {
            return None; // the template is 3 deep; this is a symlink-loop guard
        }
        let entries = std::fs::read_dir(dir).ok()?;
        let mut dirs = Vec::new();
        let mut best: Option<PathBuf> = None;
        for e in entries.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
                continue;
            }
            // The thumbnail yt-dlp writes shares the stem exactly, so match on
            // extension too or the "downloaded file" turns out to be a JPEG.
            if !is_media_ext(&p) || is_scratch(&p) {
                continue;
            }
            if p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|stem| stem.ends_with(marker))
            {
                // Prefer the finished MP3 if both it and its source are here.
                if p.extension().and_then(|s| s.to_str()) == Some("mp3") {
                    return Some(p);
                }
                best.get_or_insert(p);
            }
        }
        if best.is_some() {
            return best;
        }
        dirs.into_iter().find_map(|d| walk(&d, marker, depth + 1))
    }
    let found = walk(root, &marker, 0)?;
    // The walk starts at the library root, but assert containment anyway: this
    // path is about to be handed to ffmpeg and then to the asset protocol.
    is_within(root, &found).then_some(found)
}

/// Remove the intermediate download and the loose artwork once the MP3 exists.
///
/// Deliberately NOT inside `extract_mp3`: the resume path can arrive at a
/// finished MP3 without extracting anything, and cleanup that only runs on one
/// branch leaves a 64 MB `.webm` sitting next to every interrupted job.
fn tidy_intermediates(mp3: &Path) {
    for ext in ["webm", "m4a", "opus", "ogg", "oga", "mp4", "mkv", "aac", "wav",
                "jpg", "jpeg", "png", "webp", "info.json", "part.mp3"] {
        let stray = mp3.with_extension(ext);
        if stray != mp3 {
            let _ = std::fs::remove_file(stray);
        }
    }
}

/// Audio/video containers only. Excludes the `.jpg` thumbnail, `.info.json`,
/// subtitle sidecars, and `.part` files.
fn is_media_ext(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|s| s.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("mp3" | "m4a" | "webm" | "opus" | "ogg" | "oga" | "mp4"
            | "mkv" | "flac" | "wav" | "aac" | "mov" | "m4v")
    )
}

/// `foo.part.mp3` is a transcode in flight, not a finished track.
fn is_scratch(p: &Path) -> bool {
    p.file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|stem| stem.ends_with(".part"))
}

/// Is `path` inside `root`? Guards the one place an untrusted-ish path reaches
/// a sidecar argument and, later, the webview's asset scope.
fn is_within(root: &Path, path: &Path) -> bool {
    match (root.canonicalize(), path.canonicalize()) {
        (Ok(r), Ok(p)) => p.starts_with(r),
        _ => false,
    }
}

/// Phase 3 — derive the MP3 locally (D3).
async fn extract_mp3(app: &AppHandle, url: &str, src: &Path, probed: &Probed, job_id: Option<i64>) -> Result<PathBuf> {
    let dest = src.with_extension("mp3");
    if dest == src {
        return Ok(dest); // already an mp3
    }

    // Write to a scratch name and rename only on success. A killed ffmpeg
    // otherwise leaves a truncated file with the finished name, and the resume
    // path — which prefers an existing .mp3 — would accept it as done. That is
    // the "resume silently degrades" failure this milestone exists to prevent,
    // and it is invisible: the file plays, it just stops early.
    // `.part.mp3` keeps the media extension so a stray is still recognisable,
    // while `is_scratch` keeps find_by_id from ever returning one.
    let scratch = src.with_extension("part.mp3");

    emit(
        app,
        job_id,
        Progress {
            url: url.into(),
            stage: "extract",
            bytes_done: 0,
            bytes_total: None,
            speed_bps: None,
            eta_s: None,
            note: Some("converting to MP3".into()),
        },
    );

    // yt-dlp wrote the cover next to the audio (--write-thumbnail). MP3 is the
    // first container in this pipeline that can actually hold it.
    let cover = src.with_extension("jpg");
    let has_cover = cover.exists();

    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(), "error".into(),
        "-y".into(),
        "-i".into(), src.to_string_lossy().into_owned(),
    ];
    if has_cover {
        args.extend(["-i".into(), cover.to_string_lossy().into_owned()]);
        // Audio from input 0, artwork from input 1, copied rather than
        // re-encoded. id3v2.3 because some players still ignore 2.4.
        args.extend([
            "-map".into(), "0:a".into(),
            "-map".into(), "1:v".into(),
            "-c:v".into(), "copy".into(),
            "-id3v2_version".into(), "3".into(),
            "-metadata:s:v".into(), "title=Album cover".into(),
            "-metadata:s:v".into(), "comment=Cover (front)".into(),
        ]);
    } else {
        args.extend(["-map".into(), "0:a".into()]);
    }
    args.extend([
        "-map_metadata".into(), "0".into(),
        "-c:a".into(), "libmp3lame".into(),
        "-q:a".into(), "2".into(),
        // Set these explicitly rather than relying on the carried tags: the
        // probe already knows the real title and uploader, and an empty
        // Title column in Explorer is exactly what this change is fixing.
        "-metadata".into(), format!("title={}", probed.title),
        "-metadata".into(), format!("artist={}", probed.uploader.clone().unwrap_or_default()),
        scratch.to_string_lossy().into_owned(),
    ]);

    let (mut rx, _child) = app
        .shell()
        .sidecar("ffmpeg")
        .map_err(|e| PipelineError::Sidecar(format!("ffmpeg sidecar missing: {e}")))?
        .args(args)
        .spawn()
        .map_err(|e| PipelineError::Sidecar(format!("couldn't start ffmpeg: {e}")))?;

    let mut tail = Tail::new();
    let mut code = 0;
    while let Some(ev) = rx.recv().await {
        match ev {
            CommandEvent::Stderr(b) | CommandEvent::Stdout(b) => {
                tail.push(String::from_utf8_lossy(&b).trim().to_string())
            }
            CommandEvent::Terminated(p) => code = p.code.unwrap_or(-1),
            _ => {}
        }
    }
    if code != 0 {
        let _ = std::fs::remove_file(&scratch);
        return Err(PipelineError::Ffmpeg { code, tail: tail.text() });
    }

    // Atomic on the same volume: after this the name either doesn't exist or
    // refers to a complete file. Never both.
    std::fs::rename(&scratch, &dest)
        .map_err(|e| PipelineError::Io(format!("couldn't finalise {}: {e}", dest.display())))?;
    Ok(dest)
}

/// The whole job: probe, fetch, extract, hand back something playable.
///
/// **Resume, not restart (D26).** After a kill, the recovery that's correct
/// depends on where it died — and rather than trust the recorded stage, this
/// derives it from what's actually on disk, which cannot drift out of sync
/// with reality the way a status column can:
///
/// | on disk | recovery |
/// |---|---|
/// | the finished `.mp3` | nothing to do |
/// | the source audio, whole | skip the download, re-extract |
/// | a `.part` | `--continue` picks up mid-file |
/// | nothing | fetch from the start |
///
/// The recorded `stage` still drives the UI; it just isn't the source of truth.
pub async fn import_job(app: &AppHandle, url: &str, job_id: i64) -> Result<Track> {
    let job_id = Some(job_id);
    let url = &validate_url(url)?;
    let probed = probe(app, url, job_id).await?;

    // Give the queue row a human name as soon as we have one, so a resumed job
    // isn't an anonymous URL in the UI.
    if let Some(id) = job_id {
        with_db(app, |conn| {
            crate::jobs::set_identity(conn, id, &probed.title, &probed.id).ok()
        });
    }

    let root = library_root(app)?;

    // Resume derives its recovery from what is on disk, not from the recorded
    // stage: a status column is written by a process that then died, possibly
    // between the write and its effect. The filesystem cannot disagree with
    // itself that way.
    let existing = find_by_id(&root, &probed.id);
    let already_mp3 = existing
        .as_deref()
        .is_some_and(|p| p.extension().and_then(|s| s.to_str()) == Some("mp3"));

    let mp3 = match existing {
        // The finished MP3 survived; the kill happened after conversion.
        Some(p) if already_mp3 => p,
        // A complete source stream survived — the download is done, only the
        // extract needs redoing.
        Some(source) => extract_mp3(app, url, &source, &probed, job_id).await?,
        // Either a `.part` (which `--continue` picks up mid-file) or nothing.
        None => {
            let source = download_audio(app, url, &probed, job_id).await?;
            extract_mp3(app, url, &source, &probed, job_id).await?
        }
    };

    // Runs on every path to a finished MP3, including the resume path that
    // found one already converted and skipped extraction entirely.
    tidy_intermediates(&mp3);

    let filesize = std::fs::metadata(&mp3).map(|m| m.len()).unwrap_or(0);

    emit(
        app,
        job_id,
        Progress {
            url: url.into(),
            stage: "done",
            bytes_done: filesize,
            bytes_total: Some(filesize),
            speed_bps: None,
            eta_s: None,
            note: Some(probed.title.clone()),
        },
    );

    Ok(Track {
        id: probed.id,
        title: probed.title,
        uploader: probed.uploader,
        duration_s: probed.duration_s,
        path: mp3.to_string_lossy().to_string(),
        filesize,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_urls() {
        for u in [
            "https://www.youtube.com/watch?v=jNQXAC9IVRw",
            "http://example.com/a.mp3",
            "  https://bandcamp.com/track/x  ",
        ] {
            assert!(validate_url(u).is_ok(), "should accept {u:?}");
        }
    }

    /// The actual attack: a flag-shaped string pasted in place of a URL.
    /// yt-dlp's --exec runs an arbitrary command per download.
    #[test]
    fn rejects_argv_flag_smuggling() {
        for evil in [
            "--exec=calc.exe",
            r"--config-location=C:\evil.conf",
            "--batch-file=urls.txt",
            "-o/tmp/pwn",
            "--version",
        ] {
            assert!(validate_url(evil).is_err(), "should reject {evil:?}");
        }
    }

    #[test]
    fn rejects_non_http_schemes() {
        for u in ["file:///etc/passwd", "ftp://x/y", "javascript:alert(1)", "data:text/html,x"] {
            assert!(validate_url(u).is_err(), "should reject {u:?}");
        }
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_url("").is_err());
        assert!(validate_url("   ").is_err());
    }

    /// The path handed to ffmpeg and later to the webview's asset scope must
    /// be inside the library root. This is where the old `safe_stem` guard
    /// moved to: nothing builds a path out of an extractor-supplied id any
    /// more, so the check belongs on the resolved path instead.
    #[test]
    fn containment_check_rejects_escapes() {
        let tmp = std::env::temp_dir().join("hp-contain-test");
        let inner = tmp.join("lib");
        std::fs::create_dir_all(&inner).unwrap();
        let good = inner.join("a.mp3");
        std::fs::write(&good, b"x").unwrap();
        let outside = tmp.join("b.mp3");
        std::fs::write(&outside, b"x").unwrap();

        assert!(is_within(&inner, &good));
        assert!(!is_within(&inner, &outside));
        assert!(!is_within(&inner, &inner.join("../b.mp3")));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A transcode killed mid-write leaves `foo.part.mp3`. If the resume path
    /// ever accepted that as finished, the track would play and stop early —
    /// a silent truncation, which is the worst shape this bug can take.
    #[test]
    fn a_partial_transcode_is_never_mistaken_for_a_result() {
        let tmp = std::env::temp_dir().join("hp-scratch-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("Song [abc123].part.mp3"), b"truncated").unwrap();

        assert!(is_scratch(Path::new("Song [abc123].part.mp3")));
        assert!(!is_scratch(Path::new("Song [abc123].mp3")));
        assert!(
            find_by_id(&tmp, "abc123").is_none(),
            "a .part.mp3 must not be returned as the finished track"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Cleanup has to run on every path to a finished MP3, not just the one
    /// that did the extracting — the resume path can arrive at a converted
    /// file without extracting anything.
    #[test]
    fn tidy_removes_intermediates_but_never_the_mp3() {
        let tmp = std::env::temp_dir().join("hp-tidy-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let mp3 = tmp.join("Song [abc123].mp3");
        for f in ["Song [abc123].mp3", "Song [abc123].webm", "Song [abc123].jpg",
                  "Song [abc123].part.mp3", "Keep me [zzz].mp3"] {
            std::fs::write(tmp.join(f), b"x").unwrap();
        }

        tidy_intermediates(&mp3);

        assert!(mp3.exists(), "the finished mp3 must survive");
        assert!(tmp.join("Keep me [zzz].mp3").exists(), "other tracks untouched");
        for gone in ["Song [abc123].webm", "Song [abc123].jpg", "Song [abc123].part.mp3"] {
            assert!(!tmp.join(gone).exists(), "{gone} should have been cleaned up");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Files are located by the `[id]` yt-dlp writes into the name, so the
    /// human-readable part can be anything without breaking resume.
    #[test]
    fn finds_a_file_by_its_id_marker_at_depth() {
        let tmp = std::env::temp_dir().join("hp-find-test");
        let _ = std::fs::remove_dir_all(&tmp);
        let nested = tmp.join("youtube").join("Some Artist");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("A Song [abc123].mp3"), b"x").unwrap();
        // A partial is not a result.
        std::fs::write(nested.join("Other [zzz999].webm.part"), b"x").unwrap();

        let hit = find_by_id(&tmp, "abc123").unwrap();
        assert_eq!(hit.file_name().unwrap(), "A Song [abc123].mp3");
        assert!(find_by_id(&tmp, "zzz999").is_none(), "a .part is not a result");
        assert!(find_by_id(&tmp, "nope").is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Every yt-dlp invocation must terminate option parsing before the URL.
    #[test]
    fn terminator_is_added_per_call_site_not_in_base() {
        assert!(!ytdlp_base().contains(&"--".to_string()));
    }

    /// The progress template is pipe-delimited and parsed positionally — never
    /// scrape the human-readable bar (D4).
    #[test]
    fn parses_a_progress_line() {
        let (done, total, speed, eta, status) =
            parse_progress("HPPROG|1024|223779|52318.4|3|downloading").unwrap();
        assert_eq!(done, 1024);
        assert_eq!(total, Some(223779));
        assert!(speed.unwrap() > 52318.0);
        assert_eq!(eta, Some(3));
        assert_eq!(status, "downloading");
    }

    /// yt-dlp emits "NA" for anything it doesn't know yet — most importantly
    /// total_bytes, which is absent until the transfer is under way. That must
    /// read as "unknown", not as zero, or the UI draws a false 0%.
    #[test]
    fn unknown_fields_stay_unknown() {
        let (done, total, speed, eta, _) =
            parse_progress("HPPROG|4096|NA|NA|NA|downloading").unwrap();
        assert_eq!(done, 4096);
        assert_eq!(total, None, "unknown total must not become 0");
        assert_eq!(speed, None);
        assert_eq!(eta, None);
    }

    #[test]
    fn ignores_non_progress_output() {
        assert!(parse_progress("[download] Destination: t.webm").is_none());
        assert!(parse_progress("").is_none());
        assert!(parse_progress("HPPROG|too|few").is_none());
    }
}
