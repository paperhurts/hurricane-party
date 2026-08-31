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

fn emit(app: &AppHandle, p: Progress) {
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

/// Phase 1 — probe. No download. Cheap enough to run on paste.
pub async fn probe(app: &AppHandle, url: &str) -> Result<Probed> {
    emit(
        app,
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
    args.extend(["-J".to_string(), "--flat-playlist".into(), url.to_string()]);

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
async fn download_audio(app: &AppHandle, url: &str, probed: &Probed) -> Result<PathBuf> {
    let root = library_root(app)?;
    let outtmpl = root.join(format!("{}.%(ext)s", probed.id));

    let mut args = ytdlp_base();
    args.extend([
        // Audio only. D3 forbids downloading twice; for an audio-only request
        // architecture.md sanctions skipping video entirely rather than pulling
        // a video stream we'd immediately discard.
        "-f".to_string(),
        "bestaudio/best".into(),
        "--continue".into(), // D26: resume, don't restart
        "--newline".into(),  // without this every progress line concatenates
        "--progress-template".into(),
        "download:HPPROG|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|\
         %(progress.speed)s|%(progress.eta)s|%(progress.status)s"
            .into(),
        "-o".into(),
        outtmpl.to_string_lossy().to_string(),
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

    while let Some(ev) = rx.recv().await {
        match ev {
            CommandEvent::Stdout(b) => {
                let chunk = String::from_utf8_lossy(&b);
                for line in chunk.lines() {
                    if let Some((done, total, speed, eta, _status)) = parse_progress(line) {
                        emit(
                            app,
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
    find_by_stem(&root, &probed.id)
        .ok_or_else(|| PipelineError::MissingOutput(root.join(&probed.id).display().to_string()))
}

fn find_by_stem(dir: &Path, stem: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).find_map(|e| {
        let p = e.path();
        let matches = p.file_stem().and_then(|s| s.to_str()) == Some(stem)
            && p.extension().and_then(|s| s.to_str()) != Some("part");
        matches.then_some(p)
    })
}

/// Phase 3 — derive the MP3 locally (D3).
async fn extract_mp3(app: &AppHandle, url: &str, src: &Path, probed: &Probed) -> Result<PathBuf> {
    let dest = src.with_file_name(format!("{}.mp3", probed.id));
    if dest == src {
        return Ok(dest); // already an mp3
    }

    emit(
        app,
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

    let (mut rx, _child) = app
        .shell()
        .sidecar("ffmpeg")
        .map_err(|e| PipelineError::Sidecar(format!("ffmpeg sidecar missing: {e}")))?
        .args([
            "-hide_banner",
            "-loglevel", "error",
            "-y",
            "-i", &src.to_string_lossy(),
            "-vn",
            "-c:a", "libmp3lame",
            "-q:a", "2",
            &dest.to_string_lossy(),
        ])
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
        return Err(PipelineError::Ffmpeg { code, tail: tail.text() });
    }

    // The source stream was only ever a means to the MP3.
    let _ = std::fs::remove_file(src);
    Ok(dest)
}

/// The whole v0.1 job: probe, fetch, extract, hand back something playable.
pub async fn import(app: &AppHandle, url: &str) -> Result<Track> {
    let probed = probe(app, url).await?;
    let downloaded = download_audio(app, url, &probed).await?;
    let mp3 = extract_mp3(app, url, &downloaded, &probed).await?;

    let filesize = std::fs::metadata(&mp3).map(|m| m.len()).unwrap_or(0);

    emit(
        app,
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

/// Scan the library folder so restarting the app doesn't lose what's on disk.
/// Not persistence — that's v0.2 and SQLite. This is just reading the directory.
pub fn scan(app: &AppHandle) -> Result<Vec<Track>> {
    let root = library_root(app)?;
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&root)
        .map_err(|e| PipelineError::Io(e.to_string()))?
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("mp3") {
            continue;
        }
        let id = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        out.push(Track {
            title: id.clone(),
            id,
            uploader: None,
            duration_s: None,
            filesize: entry.metadata().map(|m| m.len()).unwrap_or(0),
            path: p.to_string_lossy().to_string(),
        });
    }
    Ok(out)
}
