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
    #[error("{}", ytdlp_message(*.code, .tail, *.cookies))]
    YtDlp {
        code: i32,
        tail: String,
        /// Whether the app had a cookies file when this failed, which changes
        /// what a sign-in refusal means: no file is "give me one", a file is
        /// "the one you gave me has no session in it" (D113).
        cookies: bool,
    },
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

/// What a yt-dlp failure says to a person (D112). A failure we recognise leads
/// with what to do about it and keeps yt-dlp's own words after, because the
/// tail is what makes a bug report answerable.
fn ytdlp_message(code: i32, tail: &str, cookies: bool) -> String {
    match explain(tail, cookies) {
        Some(why) => format!(
            "{why}

yt-dlp ({code}) said: {tail}"
        ),
        None => format!("yt-dlp failed ({code}). Last output: {tail}"),
    }
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
    /// "audio" | "video" — decides whether playback needs the video window (D13).
    pub kind: String,
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

/// The browsers yt-dlp can read a cookie store from on Windows (D113).
///
/// An allowlist because the value reaches argv: nothing a person picks goes
/// near yt-dlp's flag surface, which is the same rule `validate_url` keeps
/// about `--exec`. Safari is macOS-only and left out.
pub const BROWSERS: [&str; 7] = [
    "firefox", "brave", "chrome", "chromium", "edge", "opera", "vivaldi",
];

/// One cookie store a person can read from: a browser, and which of its
/// profiles (D115).
///
/// `--cookies-from-browser chrome` means `chrome:Default`, which is the trap
/// this type exists to remove: a second Chrome profile is where a YouTube
/// sign-in often lives, and reading the first one returns a jar full of real
/// cookies with no session in it.
#[derive(Debug, Clone, Serialize)]
pub struct CookieSource {
    /// The browser, always one of `BROWSERS`.
    pub browser: String,
    /// The profile directory (Chromium) or profile name (Firefox), when the
    /// browser has more than the one.
    pub profile: Option<String>,
    /// What the picker shows: the browser, and the profile's own name when it
    /// has one worth reading.
    pub label: String,
    /// What goes to `--cookies-from-browser`, built here so nothing a person
    /// types ever reaches argv.
    pub spec: String,
}

/// Every profile of every browser installed, in the order `BROWSERS` lists
/// them (D115). Directory listings and one small JSON file: no cookie
/// database is opened, and no cookie is read.
pub fn cookie_sources() -> Vec<CookieSource> {
    let mut out = Vec::new();
    for browser in BROWSERS {
        match browser {
            "firefox" => out.extend(firefox_profiles()),
            _ => out.extend(chromium_profiles(browser)),
        }
    }
    out
}

fn source(browser: &str, profile: Option<String>, name: Option<String>) -> CookieSource {
    let label = match (&profile, &name) {
        (Some(_), Some(n)) => format!("{browser} — {n}"),
        (Some(p), None) => format!("{browser} — {p}"),
        _ => browser.to_string(),
    };
    let spec = match &profile {
        Some(p) => format!("{browser}:{p}"),
        None => browser.to_string(),
    };
    CookieSource {
        browser: browser.to_string(),
        profile,
        label,
        spec,
    }
}

/// Chromium keeps one folder per profile under `User Data`, and the display
/// names in `Local State`. A profile with no `Cookies` file has never stored
/// one, so it is not a source.
fn chromium_profiles(browser: &str) -> Vec<CookieSource> {
    let Some(root) = chromium_root(browser) else {
        return Vec::new();
    };
    let names: std::collections::HashMap<String, String> =
        std::fs::read_to_string(root.join("Local State"))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| {
                v.get("profile")?.get("info_cache")?.as_object().map(|o| {
                    o.iter()
                        .filter_map(|(dir, info)| {
                            Some((dir.clone(), info.get("name")?.as_str()?.to_string()))
                        })
                        .collect()
                })
            })
            .unwrap_or_default();

    let mut found: Vec<CookieSource> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return found;
    };
    for e in entries.filter_map(|e| e.ok()) {
        if !e.path().is_dir() {
            continue;
        }
        let dir = e.file_name().to_string_lossy().into_owned();
        // Chromium moved the file under `Network/` and still reads the old
        // place on older profiles.
        let has_cookies = e.path().join("Cookies").is_file()
            || e.path().join("Network").join("Cookies").is_file();
        if !has_cookies {
            continue;
        }
        found.push(source(browser, Some(dir.clone()), names.get(&dir).cloned()));
    }
    found.sort_by(|a, b| a.spec.cmp(&b.spec));
    found
}

fn chromium_root(browser: &str) -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let rel = match browser {
        "chrome" => r"Google\Chrome\User Data",
        "chromium" => r"Chromium\User Data",
        "brave" => r"BraveSoftware\Brave-Browser\User Data",
        "edge" => r"Microsoft\Edge\User Data",
        "vivaldi" => r"Vivaldi\User Data",
        "opera" => r"Opera Software\Opera Stable",
        _ => return None,
    };
    let p = local.join(rel);
    p.is_dir().then_some(p)
}

/// Firefox lists its profiles in `profiles.ini`, by name and path. yt-dlp
/// takes either; the name is what a person recognises.
fn firefox_profiles() -> Vec<CookieSource> {
    let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) else {
        return Vec::new();
    };
    let root = appdata.join(r"Mozilla\Firefox");
    let Ok(ini) = std::fs::read_to_string(root.join("profiles.ini")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    let mut path: Option<String> = None;
    let flush =
        |name: &mut Option<String>, path: &mut Option<String>, out: &mut Vec<CookieSource>| {
            if let (Some(n), Some(p)) = (name.take(), path.take()) {
                let dir = root.join(p.replace('/', "\\"));
                if dir.join("cookies.sqlite").is_file() {
                    out.push(source(
                        "firefox",
                        Some(dir.to_string_lossy().into_owned()),
                        Some(n),
                    ));
                }
            }
        };
    for line in ini.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            flush(&mut name, &mut path, &mut out);
        } else if let Some(v) = line.strip_prefix("Name=") {
            name = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("Path=") {
            path = Some(v.to_string());
        }
    }
    flush(&mut name, &mut path, &mut out);
    out
}

/// The cookie database behind one source, and the query that lists the names
/// in it for youtube.com. Firefox and Chromium keep different schemas.
fn store_db(s: &CookieSource) -> Option<(PathBuf, &'static str)> {
    if s.browser == "firefox" {
        let dir = PathBuf::from(s.profile.as_deref()?);
        let db = dir.join("cookies.sqlite");
        return db.is_file().then_some((
            db,
            "SELECT name FROM moz_cookies WHERE host LIKE '%youtube.com'",
        ));
    }
    let root = chromium_root(&s.browser)?;
    let prof = root.join(s.profile.as_deref()?);
    // Chromium moved the file under `Network/` and older profiles still have
    // it beside the rest.
    for p in [prof.join("Network").join("Cookies"), prof.join("Cookies")] {
        if p.is_file() {
            return Some((
                p,
                "SELECT name FROM cookies WHERE host_key LIKE '%youtube.com'",
            ));
        }
    }
    None
}

/// Which stores on this machine hold a YouTube sign-in (D115).
///
/// Called at exactly one moment: an export came back with no session in it,
/// which is when "where is the sign-in, then?" is the only useful thing left
/// to say. It copies each cookie database to a temporary file (the browser
/// keeps the original locked), reads cookie **names** for youtube.com, and
/// deletes the copy. No cookie value is read, nothing is kept, and a store
/// that will not copy is skipped rather than guessed at.
pub fn stores_with_session() -> Vec<String> {
    const SESSION: [&str; 3] = ["SID", "__Secure-1PSID", "LOGIN_INFO"];
    let tmp = std::env::temp_dir();
    let mut out = Vec::new();
    for (i, s) in cookie_sources().into_iter().enumerate() {
        let Some((db, query)) = store_db(&s) else {
            continue;
        };
        let copy = tmp.join(format!("hp-cookie-peek-{}-{i}.db", std::process::id()));
        if std::fs::copy(&db, &copy).is_err() {
            continue;
        }
        let found = rusqlite::Connection::open(&copy)
            .and_then(|conn| {
                let mut st = conn.prepare(query)?;
                let mut rows = st.query([])?;
                let mut hit = false;
                while let Some(r) = rows.next()? {
                    let name: String = r.get(0)?;
                    if SESSION.contains(&name.as_str()) {
                        hit = true;
                        break;
                    }
                }
                Ok(hit)
            })
            .unwrap_or(false);
        let _ = std::fs::remove_file(&copy);
        if found {
            out.push(s.label);
        }
    }
    out
}

/// What an export produced: where the jar landed, and how many cookies are in
/// it — the count is what tells a person it worked, and it is all this app
/// ever says about the contents.
#[derive(Debug, Clone, Serialize)]
pub struct CookieExport {
    pub path: String,
    pub count: usize,
    /// Whether a YouTube sign-in is among them (D113). A jar full of a
    /// browser's ordinary cookies still fails every age gate, and finding
    /// that out at the export beats finding it out per download.
    pub youtube: bool,
    /// When it has none: the stores on this machine that do (D115). The
    /// owner read three browsers in turn and the app never said "that one" —
    /// which it can, since it already knows what the stores are.
    pub elsewhere: Vec<String>,
}

/// Read a browser's cookie store with yt-dlp and write the jar into the app's
/// own data folder (D113).
///
/// yt-dlp dumps the jar it loaded whenever `--cookies` names a file, so this
/// is one run of the tool that already ships. It needs a URL to accept the
/// job, and the URL it gets is `https://cookies.invalid/`: `.invalid` is
/// reserved by RFC 2606 and can never resolve, so the run cannot reach anyone
/// — the extraction fails, the jar is written anyway, and the exit code is
/// ignored on purpose. What counts as success is a file with cookies in it.
pub async fn export_cookies(app: &AppHandle, spec: &str) -> Result<CookieExport> {
    // A spec, not a browser: `chrome:Profile 1` (D115). Checked against what
    // `cookie_sources` found rather than parsed permissively — the value
    // reaches argv, so it may only ever be one this machine offered.
    let known = cookie_sources();
    let Some(source) = known.into_iter().find(|s| s.spec == spec) else {
        return Err(PipelineError::BadUrl(format!(
            "{spec} is not a cookie store this machine offers"
        )));
    };
    let browser = source.browser.as_str();
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| PipelineError::Io(format!("no app data dir: {e}")))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| PipelineError::Io(format!("couldn't create {}: {e}", dir.display())))?;
    let out = dir.join("cookies.txt");

    let args: Vec<String> = vec![
        "--cookies-from-browser".into(),
        source.spec.clone(),
        "--cookies".into(),
        out.to_string_lossy().into_owned(),
        "--simulate".into(),
        "--skip-download".into(),
        "--no-warnings".into(),
        "--".into(),
        "https://cookies.invalid/".into(),
    ];
    let (mut rx, _child) = app
        .shell()
        .sidecar("yt-dlp")
        .map_err(|e| PipelineError::Sidecar(format!("yt-dlp sidecar missing: {e}")))?
        .args(args)
        .spawn()
        .map_err(|e| PipelineError::Sidecar(format!("couldn't start yt-dlp: {e}")))?;

    let mut tail = Tail::new();
    while let Some(ev) = rx.recv().await {
        match ev {
            CommandEvent::Stdout(b) | CommandEvent::Stderr(b) => {
                tail.push(String::from_utf8_lossy(&b).trim().to_string())
            }
            _ => {}
        }
    }

    let count = count_cookies(&out);
    if count == 0 {
        // Nothing usable: leave no half-written jar behind for yt-dlp to send.
        let _ = std::fs::remove_file(&out);
        return Err(PipelineError::Io(explain_export(browser, &tail.text())));
    }
    let youtube = jar_has_youtube_session(&out);
    Ok(CookieExport {
        path: out.to_string_lossy().into_owned(),
        count,
        youtube,
        // Only when there is bad news to explain: see `stores_with_session`.
        elsewhere: if youtube {
            Vec::new()
        } else {
            stores_with_session()
                .into_iter()
                .filter(|l| *l != source.label)
                .collect()
        },
    })
}

/// Cookies in a Netscape jar: every line that is not blank and not a comment.
/// Reading the file this way is deliberate — the count is the only thing this
/// process ever learns about it.
fn count_cookies(path: &Path) -> usize {
    let Ok(text) = std::fs::read_to_string(path) else {
        return 0;
    };
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .count()
}

/// Whether a jar carries a YouTube sign-in (D113).
///
/// By cookie **name**, never by value: the names below are the first-party
/// session cookies YouTube treats as signed in, and the only thing this
/// process wants to know is whether one of them is there. `__Secure-3PSID`
/// deliberately does not count — it is the cross-site variant, and a jar that
/// has it without the first-party set still gets the age gate, which is
/// exactly what happened on the owner's machine.
fn jar_has_youtube_session(path: &Path) -> bool {
    const SESSION: [&str; 3] = ["SID", "__Secure-1PSID", "LOGIN_INFO"];
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() >= 7 && f[0].contains("youtube.com")).then(|| f[5])
        })
        .any(|name| SESSION.contains(&name))
}

/// Why a browser would not give up its cookies, in words a person can act on
/// (D113). Both of these were seen on the owner's machine before any of this
/// was written.
fn explain_export(browser: &str, tail: &str) -> String {
    if tail.contains("Could not copy") && tail.contains("cookie database") {
        format!(
            "{browser} is running, so its cookie database is locked. Close it completely — check the tray — and try again."
        )
    } else if tail.contains("DPAPI") || tail.contains("App-Bound") {
        format!(
            "{browser} encrypts its cookies in a way yt-dlp cannot read (Chromium's App-Bound Encryption). Firefox works, or export a cookies.txt yourself and pick it with Cookies."
        )
    } else if tail.contains("could not find") {
        format!("No {browser} profile with cookies was found on this machine.")
    } else {
        format!("{browser} gave up no cookies. yt-dlp said: {tail}")
    }
}

/// Where a person keeps the `cookies.txt` they exported from their own
/// browser, when they have pointed the app at one (D112). Unset, which is the
/// default, means yt-dlp runs with no authentication at all.
pub const COOKIES_SETTING: &str = "ytdlp.cookies";

/// That file, if it is set and still there.
///
/// A path that has gone — an unplugged drive, a file they deleted — is no
/// authentication rather than an error: the next age-gated video says what it
/// needs, and everything else keeps working. The file is never read here,
/// never copied, and never logged; only its path is stored, and only yt-dlp
/// opens it.
pub fn cookies_file(app: &AppHandle) -> Option<PathBuf> {
    let state = app.state::<crate::Db>();
    let raw = {
        let conn = state.0.lock().unwrap();
        crate::db::get_setting(&conn, COOKIES_SETTING)
    }?;
    let p = PathBuf::from(raw.trim());
    // Absolute, so nothing resolves against the working directory, and a
    // value that begins with `-` can never reach argv looking like a flag.
    (p.is_absolute() && p.is_file()).then_some(p)
}

/// Args every yt-dlp invocation needs, plus the person's cookies when they
/// have set some (D112).
fn ytdlp_base(app: &AppHandle) -> Vec<String> {
    ytdlp_args_for(app, false)
}

/// The same, for the one call that wants a list expanded rather than reduced
/// to its first video (#137).
fn ytdlp_args_for(app: &AppHandle, playlists: bool) -> Vec<String> {
    ytdlp_args(cookies_file(app).as_deref(), playlists)
}

/// The same list without the lookup, so its shape can be tested without an app.
///
/// `--js-runtimes deno` is D46. Without a JS runtime, yt-dlp warns that
/// "YouTube extraction without a JS runtime has been deprecated" and silently
/// returns fewer formats — a degradation that looks like success.
///
/// `--no-playlist` is on every call but the list probe: a job downloads the one
/// video it was queued for, so its file keeps the `[id]` a resume depends on
/// (D49), and a pasted `watch?v=…&list=…` never turns into forty downloads
/// nobody asked for.
fn ytdlp_args(cookies: Option<&Path>, playlists: bool) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--js-runtimes".into(),
        "deno".into(),
        "--no-warnings".into(),
    ];
    if !playlists {
        args.push("--no-playlist".into());
    }
    if let Some(c) = cookies {
        args.push("--cookies".into());
        args.push(c.to_string_lossy().into_owned());
    }
    args
}

/// The yt-dlp failures a person can do something about, in a line rather than
/// the wall of text yt-dlp writes (D112). Everything else keeps yt-dlp's own
/// words: a message nobody has read yet beats a wrong guess at what it means.
pub(crate) fn explain(tail: &str, cookies: bool) -> Option<String> {
    // Two different problems wearing one error. Without a jar, the app needs
    // one; with a jar, the jar is signed out — and telling someone to do the
    // thing they have already done is the worst answer available (D113).
    const GET: &str = "Point the app at a cookies.txt exported from a browser you are signed in with, with Cookies in the library header, then retry.";
    const STALE: &str = "The cookies this app has carry no YouTube sign-in, or no longer do. Sign in to YouTube in that browser, read them again with From a browser, and retry.";
    let cookies_hint = if cookies { STALE } else { GET };
    let (why, needs_cookies) = if tail.contains("Sign in to confirm your age") {
        (
            "YouTube wants a signed-in session for this one: it is age-restricted.",
            true,
        )
    } else if tail.contains("confirm you") && tail.contains("not a bot") {
        (
            "YouTube asked this download to prove it is not a bot.",
            true,
        )
    } else if tail.contains("members-only") || tail.contains("available to this channel's members")
    {
        (
            "That video is members-only, so it needs an account that has it.",
            true,
        )
    } else if tail.contains("Private video") {
        ("That video is private, so nothing can fetch it.", false)
    } else if tail.contains("Video unavailable") {
        ("YouTube says that video is unavailable.", false)
    } else {
        return None;
    };
    Some(if needs_cookies {
        format!("{why} {cookies_hint}")
    } else {
        why.to_string()
    })
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
    let parsed =
        url::Url::parse(s).map_err(|_| PipelineError::BadUrl(format!("{s:?} isn't a URL")))?;
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

    let mut args = ytdlp_base(app);
    // `--` ends option parsing: everything after it is a positional argument.
    args.extend([
        "-J".to_string(),
        "--flat-playlist".into(),
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
        return Err(PipelineError::YtDlp {
            code,
            tail: tail.text(),
            cookies: cookies_file(app).is_some(),
        });
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

/// A list a person pasted, read without downloading anything (#137).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistProbe {
    /// The `list=` id, which is also how a second import recognises it.
    pub id: String,
    pub title: String,
    pub uploader: Option<String>,
    pub items: Vec<PlaylistItem>,
}

/// One entry of a flat playlist. `duration_s` is missing more often than not —
/// `--flat-playlist` is one request for the whole list and yt-dlp does not
/// visit each video to fill it in, which is the entire point of using it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistItem {
    pub id: String,
    pub title: String,
    pub url: String,
    pub duration_s: Option<f64>,
    /// A file whose name already carries this id is in the library, so the
    /// picker can say so rather than queueing a second copy.
    pub have: bool,
}

/// The `list=` id in a URL, if there is one.
///
/// Parsed rather than pattern-matched on the whole URL because the id is what
/// everything else here keys on: whether the list is real, what the playlist
/// gets named, and which entries to ask for.
pub fn list_id_of(url: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == "list")
        .map(|(_, v)| v.to_string())
        .filter(|v| !v.is_empty())
}

/// Whether a URL names one video rather than only a list (#137).
///
/// This matters because `--no-playlist` means "when the URL is a video *and* a
/// list, take the video". Handed a bare `playlist?list=…` there is no video to
/// take, and yt-dlp downloads the whole list — one queued job that quietly
/// becomes forty downloads. So a URL with a list and no video is refused as a
/// job and read as a list instead.
pub fn names_a_video(url: &str) -> bool {
    let Some((_, query)) = url.split_once('?') else {
        return true;
    };
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .any(|(k, v)| k == "v" && !v.is_empty())
}

/// Whether a list is one YouTube generates on the fly rather than one somebody
/// made (#137).
///
/// `RD…` is radio: My Mix, a song radio, an artist radio. They are endless,
/// personalised, and different the next time you ask — there is nothing to
/// snapshot, so importing one is offering a person something this app cannot
/// keep its side of.
pub fn is_generated_list(list_id: &str) -> bool {
    list_id.starts_with("RD")
}

/// Read a list with one yt-dlp call and no downloads (#137, architecture.md
/// phase 1).
///
/// `--flat-playlist` is what makes this one request instead of one per video:
/// yt-dlp returns the entries without visiting them. `--no-playlist` is
/// dropped here and nowhere else — the per-item jobs keep it, so a job always
/// downloads exactly the one video it was queued for (D49's `[id]` in the file
/// name depends on that).
pub async fn probe_playlist(app: &AppHandle, url: &str) -> Result<PlaylistProbe> {
    let url = validate_url(url)?;
    let list_id = list_id_of(&url).unwrap_or_default();
    if is_generated_list(&list_id) {
        return Err(PipelineError::BadUrl(format!(
            "{list_id} is a mix YouTube makes up as it goes, not a list someone saved. There is nothing to snapshot; the video itself will import."
        )));
    }

    let mut args = ytdlp_args_for(app, true);
    args.extend([
        "-J".to_string(),
        "--flat-playlist".into(),
        "--".into(),
        url.clone(),
    ]);
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
        return Err(PipelineError::YtDlp {
            code,
            tail: tail.text(),
            cookies: cookies_file(app).is_some(),
        });
    }
    let v: serde_json::Value =
        serde_json::from_str(json.trim()).map_err(|e| PipelineError::Metadata(e.to_string()))?;
    let have =
        |id: &str| with_db(app, |conn| crate::library::have_video_id(conn, id)).unwrap_or(false);
    Ok(playlist_from(&v, &list_id, have))
}

/// The mapping, split out so the shape of yt-dlp's answer can be tested
/// without running it.
fn playlist_from(
    v: &serde_json::Value,
    list_id: &str,
    have: impl Fn(&str) -> bool,
) -> PlaylistProbe {
    let str_of = |node: &serde_json::Value, k: &str| {
        node.get(k)
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .filter(|s| !s.is_empty())
    };
    let entries = v.get("entries").and_then(|e| e.as_array());
    let items = entries
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let id = str_of(e, "id")?;
                    Some(PlaylistItem {
                        title: str_of(e, "title").unwrap_or_else(|| id.clone()),
                        // Built from the id rather than taken from `url`: an
                        // entry's own URL can carry the list back with it, and
                        // a job must name one video and nothing else.
                        url: format!("https://www.youtube.com/watch?v={id}"),
                        duration_s: e.get("duration").and_then(|x| x.as_f64()),
                        have: have(&id),
                        id,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    PlaylistProbe {
        id: str_of(v, "id").unwrap_or_else(|| list_id.to_string()),
        title: str_of(v, "title")
            .or_else(|| str_of(v, "playlist_title"))
            .unwrap_or_else(|| "Playlist".to_string()),
        uploader: str_of(v, "uploader").or_else(|| str_of(v, "channel")),
        items,
    }
}

/// The five fields of one progress line, in template order: bytes downloaded,
/// total bytes, speed in bytes/s, ETA in seconds, status. A tuple rather than a
/// struct because every call site destructures it on the spot.
type ProgressLine = (u64, Option<u64>, Option<f64>, Option<u64>, String);

/// One `--progress-template` line. Pipe-delimited and parsed positionally —
/// never scrape the human-readable bar (D4), which changes between releases.
fn parse_progress(line: &str) -> Option<ProgressLine> {
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
async fn download_media(
    app: &AppHandle,
    url: &str,
    probed: &Probed,
    job_id: Option<i64>,
    want_video: bool,
) -> Result<PathBuf> {
    let root = library_root(app)?;
    // Grouped by extractor, flat within it (D49).
    //
    // architecture.md's template also nested by `%(uploader)s`, which was
    // dropped: on YouTube the uploader is the *channel*, not the artist, so it
    // produced roughly one folder per file for no navigational gain. It is
    // still captured on `media.uploader`, so nothing is lost — and the app's
    // own browser is a DB query (O6), not a directory listing.
    let outtmpl = root.join("%(extractor)s/%(title)s [%(id)s].%(ext)s");

    let mut args = ytdlp_base(app);
    // Point yt-dlp at our ffmpeg rather than letting it search PATH.
    if let Some(ff) = bundled_ffmpeg() {
        args.extend([
            "--ffmpeg-location".to_string(),
            ff.to_string_lossy().into_owned(),
        ]);
    }
    args.extend([
        "-f".to_string(),
        if want_video {
            // D3: one download. The MP3, if ever wanted, is derived from this
            // file locally rather than fetched a second time.
            "bv*+ba/b".to_string()
        } else {
            // Audio-only request: architecture.md sanctions skipping video
            // entirely rather than pulling a stream we'd immediately discard.
            "bestaudio/best".to_string()
        },
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
        "--merge-output-format".into(),
        "mp4".into(),
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
                        let persist =
                            last_persist.elapsed() >= std::time::Duration::from_millis(250);
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
        return Err(PipelineError::YtDlp {
            code,
            tail: tail.text(),
            cookies: cookies_file(app).is_some(),
        });
    }

    // Deterministic path is why the probe ran first: scan for <id>.* rather
    // than parsing the filename back out of yt-dlp's chatter.
    downloaded_output(&root, &probed.id, want_video)
        .ok_or_else(|| PipelineError::MissingOutput(format!("{} [{}]", root.display(), probed.id)))
}

/// The file a download just produced, by the kind it was asked for. The
/// video path must find its MP4 even when an earlier audio import left an
/// MP3 with the same id beside it (#22): the general lookup prefers the MP3,
/// and reporting that path made the video's row collide with the audio's,
/// leaving the MP4 on disk with no row in the library.
fn downloaded_output(root: &Path, id: &str, want_video: bool) -> Option<PathBuf> {
    if want_video {
        find_video_by_id(root, id)
    } else {
        find_by_id(root, id)
    }
}

/// Locate a downloaded file by the `[id]` yt-dlp writes into its name.
///
/// Recursive, because the template nests by extractor and uploader. Skips
/// `.part` files — a partial is not a result, it is the thing `--continue`
/// will finish.
/// Any media file carrying `[id]`, the finished MP3 first. What the audio
/// path resumes from: an MP3 means done, anything else is a source to extract
/// from, which includes a video a previous import finished (D3, D80).
fn find_by_id(root: &Path, id: &str) -> Option<PathBuf> {
    find_by_id_where(root, id, |_| true)
}

/// A finished *video* carrying `[id]`: a video container, never the MP3 an
/// audio-only import left behind (#22). The identity of a finished import is
/// (source id, kind), not the id alone (D80). `.webm` is left out on purpose:
/// the audio path's intermediate is a webm, and a dead job leaves one behind
/// (D26), so a webm here would be taken for a finished video and served as
/// one, silent picture and all. The video path merges to mp4 (D3), so the
/// cost is at most one re-download of a single-stream fallback.
fn find_video_by_id(root: &Path, id: &str) -> Option<PathBuf> {
    find_by_id_where(root, id, is_video_container)
}

/// The containers the video path produces, and only those.
fn is_video_container(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("mp4" | "mkv" | "mov" | "m4v")
    )
}

fn find_by_id_where(root: &Path, id: &str, keep: fn(&Path) -> bool) -> Option<PathBuf> {
    let marker = format!("[{id}]");
    fn walk(dir: &Path, marker: &str, keep: fn(&Path) -> bool, depth: usize) -> Option<PathBuf> {
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
            if !is_media_ext(&p) || is_scratch(&p) || !keep(&p) {
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
        dirs.into_iter()
            .find_map(|d| walk(&d, marker, keep, depth + 1))
    }
    let found = walk(root, &marker, keep, 0)?;
    // The walk starts at the library root, but assert containment anyway: this
    // path is about to be handed to ffmpeg and then to the asset protocol.
    is_within(root, &found).then_some(found)
}

/// Remove the intermediate download and the loose artwork, keeping `keep`.
///
/// Deliberately NOT inside `extract_mp3`: the resume path can arrive at a
/// finished MP3 without extracting anything, and the video path never extracts
/// at all. Cleanup that only runs on one branch leaves a 64 MB `.webm` next to
/// every interrupted job, and a stray cover next to every video.
///
/// Video containers are not on the list. A finished video next to an MP3 is
/// not scratch, it is the other kind's result, and the audio path derives its
/// MP3 *from* it (D3) — so stripping `.mp4` here deleted the user's video the
/// moment they asked for the audio as well (#22, D80). The audio path's own
/// intermediate is a webm or an m4a, which are still stripped.
fn tidy_intermediates(keep: &Path) {
    for ext in [
        "webm",
        "m4a",
        "opus",
        "ogg",
        "oga",
        "aac",
        "wav",
        "jpg",
        "jpeg",
        "png",
        "webp",
        "info.json",
        "part.mp3",
    ] {
        let stray = keep.with_extension(ext);
        // The guard is what makes this safe to call with either an .mp3 or an
        // .mp4 as the thing being kept: the result's own extension is in the
        // list, and skipping self is the only reason it survives.
        if stray != keep {
            let _ = std::fs::remove_file(stray);
        }
    }
}

/// Audio/video containers only. Excludes the `.jpg` thumbnail, `.info.json`,
/// subtitle sidecars, and `.part` files.
fn is_media_ext(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some(
            "mp3"
                | "m4a"
                | "webm"
                | "opus"
                | "ogg"
                | "oga"
                | "mp4"
                | "mkv"
                | "flac"
                | "wav"
                | "aac"
                | "mov"
                | "m4v"
        )
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
async fn extract_mp3(
    app: &AppHandle,
    url: &str,
    src: &Path,
    probed: &Probed,
    job_id: Option<i64>,
) -> Result<PathBuf> {
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
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-i".into(),
        src.to_string_lossy().into_owned(),
    ];
    if has_cover {
        args.extend(["-i".into(), cover.to_string_lossy().into_owned()]);
        // Audio from input 0, artwork from input 1, copied rather than
        // re-encoded. id3v2.3 because some players still ignore 2.4.
        args.extend([
            "-map".into(),
            "0:a".into(),
            "-map".into(),
            "1:v".into(),
            "-c:v".into(),
            "copy".into(),
            "-id3v2_version".into(),
            "3".into(),
            "-metadata:s:v".into(),
            "title=Album cover".into(),
            "-metadata:s:v".into(),
            "comment=Cover (front)".into(),
        ]);
    } else {
        args.extend(["-map".into(), "0:a".into()]);
    }
    args.extend([
        "-map_metadata".into(),
        "0".into(),
        "-c:a".into(),
        "libmp3lame".into(),
        "-q:a".into(),
        "2".into(),
        // Set these explicitly rather than relying on the carried tags: the
        // probe already knows the real title and uploader, and an empty
        // Title column in Explorer is exactly what this change is fixing.
        "-metadata".into(),
        format!("title={}", probed.title),
        "-metadata".into(),
        format!("artist={}", probed.uploader.clone().unwrap_or_default()),
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
        return Err(PipelineError::Ffmpeg {
            code,
            tail: tail.text(),
        });
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
pub async fn import_job(
    app: &AppHandle,
    url: &str,
    job_id: i64,
    want_video: bool,
) -> Result<Track> {
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
    if want_video {
        // A video is finished when it's downloaded — there is no transcode
        // step, so `already_mp3` reasoning doesn't apply. Only a video
        // container counts as finished: after an audio-only import the id's
        // file is an MP3, and taking it as the video "done" is #22. The MP3's
        // source was tidied away, so this really does download again (D80).
        let file = match find_video_by_id(&root, &probed.id) {
            Some(p) => p,
            None => download_media(app, url, &probed, job_id, true).await?,
        };
        // Same cleanup the audio path gets. This branch returns early, so
        // without an explicit call the cover art yt-dlp wrote is left behind —
        // the identical shape of bug as the extract-path-only cleanup.
        tidy_intermediates(&file);

        let filesize = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
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
        return Ok(Track {
            id: probed.id,
            title: probed.title,
            uploader: probed.uploader,
            duration_s: probed.duration_s,
            path: file.to_string_lossy().to_string(),
            filesize,
            kind: "video".into(),
        });
    }

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
            let source = download_media(app, url, &probed, job_id, false).await?;
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
        kind: "audio".into(),
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
        for u in [
            "file:///etc/passwd",
            "ftp://x/y",
            "javascript:alert(1)",
            "data:text/html,x",
        ] {
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

    /// Cleanup has to run on every path to a finished file, not just the one
    /// that did the extracting: the resume path can arrive at a converted file
    /// without extracting, and the video path never extracts at all.
    #[test]
    fn tidy_removes_intermediates_but_never_the_mp3() {
        let tmp = std::env::temp_dir().join("hp-tidy-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let mp3 = tmp.join("Song [abc123].mp3");
        for f in [
            "Song [abc123].mp3",
            "Song [abc123].webm",
            "Song [abc123].jpg",
            "Song [abc123].part.mp3",
            "Keep me [zzz].mp3",
        ] {
            std::fs::write(tmp.join(f), b"x").unwrap();
        }

        tidy_intermediates(&mp3);

        assert!(mp3.exists(), "the finished mp3 must survive");
        assert!(
            tmp.join("Keep me [zzz].mp3").exists(),
            "other tracks untouched"
        );
        for gone in [
            "Song [abc123].webm",
            "Song [abc123].jpg",
            "Song [abc123].part.mp3",
        ] {
            assert!(
                !tmp.join(gone).exists(),
                "{gone} should have been cleaned up"
            );
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The video path keeps an .mp4 rather than an .mp3, and calls the same
    /// cleanup. The result's own extension is in the strip list, so "skip
    /// self" is the only thing stopping it deleting what it just produced.
    #[test]
    fn tidy_keeps_a_video_result_and_strips_its_cover() {
        let tmp = std::env::temp_dir().join("hp-tidy-video-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let mp4 = tmp.join("Clip [abc123].mp4");
        for f in [
            "Clip [abc123].mp4",
            "Clip [abc123].jpg",
            "Clip [abc123].webm",
        ] {
            std::fs::write(tmp.join(f), b"x").unwrap();
        }

        tidy_intermediates(&mp4);

        assert!(
            mp4.exists(),
            "the video result must survive its own cleanup"
        );
        assert!(!tmp.join("Clip [abc123].jpg").exists());
        assert!(!tmp.join("Clip [abc123].webm").exists());
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
        assert!(
            find_by_id(&tmp, "zzz999").is_none(),
            "a .part is not a result"
        );
        assert!(find_by_id(&tmp, "nope").is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// #22: after an audio-only import the id's only file is the MP3, and the
    /// video branch used to take it as the finished video. A finished video is
    /// a video container, and a webm is not one (it is the audio path's
    /// intermediate, left behind by a dead job).
    #[test]
    fn a_video_lookup_ignores_the_mp3_and_the_audio_intermediate() {
        let tmp = std::env::temp_dir().join("hp-find-video-test");
        let _ = std::fs::remove_dir_all(&tmp);
        let dir = tmp.join("youtube");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Song [abc123].mp3"), b"x").unwrap();
        std::fs::write(dir.join("Song [abc123].webm"), b"x").unwrap();
        assert!(
            find_video_by_id(&tmp, "abc123").is_none(),
            "an mp3 and a webm are not a finished video"
        );
        assert_eq!(
            find_by_id(&tmp, "abc123").unwrap().file_name().unwrap(),
            "Song [abc123].mp3",
            "the audio lookup still prefers the finished mp3"
        );

        std::fs::write(dir.join("Song [abc123].mp4"), b"x").unwrap();
        assert_eq!(
            find_video_by_id(&tmp, "abc123")
                .unwrap()
                .file_name()
                .unwrap(),
            "Song [abc123].mp4"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// D3 says the MP3 derives from the video; D80 says the video is then a
    /// finished import of its own kind. Cleanup after the extract must leave
    /// it, while still stripping the audio path's own intermediates.
    #[test]
    fn tidy_after_an_audio_import_keeps_a_finished_video() {
        let tmp = std::env::temp_dir().join("hp-tidy-keep-video-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let mp3 = tmp.join("Clip [abc123].mp3");
        for f in [
            "Clip [abc123].mp3",
            "Clip [abc123].mp4",
            "Clip [abc123].mkv",
            "Clip [abc123].m4a",
            "Clip [abc123].jpg",
        ] {
            std::fs::write(tmp.join(f), b"x").unwrap();
        }

        tidy_intermediates(&mp3);

        assert!(mp3.exists());
        assert!(
            tmp.join("Clip [abc123].mp4").exists(),
            "the video is a result, not scratch"
        );
        assert!(tmp.join("Clip [abc123].mkv").exists());
        assert!(
            !tmp.join("Clip [abc123].m4a").exists(),
            "the audio intermediate goes"
        );
        assert!(!tmp.join("Clip [abc123].jpg").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Found by hand test: the video downloaded, then the job reported the
    /// MP3's path, because the general lookup prefers it. The row collided
    /// with the audio row and the MP4 never reached the library.
    #[test]
    fn a_video_download_reports_its_mp4_even_beside_an_older_mp3() {
        let tmp = std::env::temp_dir().join("hp-downloaded-output-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("Song [abc123].mp3"), b"x").unwrap();
        std::fs::write(tmp.join("Song [abc123].mp4"), b"x").unwrap();
        assert_eq!(
            downloaded_output(&tmp, "abc123", true)
                .unwrap()
                .file_name()
                .unwrap(),
            "Song [abc123].mp4"
        );
        assert_eq!(
            downloaded_output(&tmp, "abc123", false)
                .unwrap()
                .file_name()
                .unwrap(),
            "Song [abc123].mp3"
        );
        assert!(downloaded_output(&tmp, "nope", true).is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The audio path resumes from whatever source is there, and a finished
    /// video from an earlier import is a source (D3): no second download.
    #[test]
    fn the_audio_lookup_offers_a_finished_video_as_the_source() {
        let tmp = std::env::temp_dir().join("hp-find-source-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("Clip [abc123].mp4"), b"x").unwrap();
        assert_eq!(
            find_by_id(&tmp, "abc123").unwrap().file_name().unwrap(),
            "Clip [abc123].mp4"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Every yt-dlp invocation must terminate option parsing before the URL.
    #[test]
    fn terminator_is_added_per_call_site_not_in_base() {
        assert!(!ytdlp_args(None, false).contains(&"--".to_string()));
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

    #[test]
    fn a_list_is_found_by_its_query_parameter() {
        assert_eq!(
            list_id_of("https://www.youtube.com/watch?v=Hj_G0SYZMjE&list=PLWtysTk&index=3")
                .as_deref(),
            Some("PLWtysTk")
        );
        assert_eq!(
            list_id_of("https://www.youtube.com/playlist?list=OLAK5uy_k").as_deref(),
            Some("OLAK5uy_k")
        );
        assert_eq!(
            list_id_of("https://www.youtube.com/watch?v=Hj_G0SYZMjE"),
            None
        );
        assert_eq!(list_id_of("https://www.youtube.com/watch?v=x&list="), None);
        assert_eq!(list_id_of("https://example.com/no-query"), None);
    }

    #[test]
    fn a_bare_list_url_names_no_video() {
        assert!(names_a_video("https://www.youtube.com/watch?v=x&list=PL1"));
        assert!(!names_a_video("https://www.youtube.com/playlist?list=PL1"));
        // No query at all is a plain URL, which is a job like any other.
        assert!(names_a_video("https://example.com/song.mp3"));
        assert!(!names_a_video("https://www.youtube.com/watch?v=&list=PL1"));
    }

    #[test]
    fn a_mix_is_not_a_list_anyone_saved() {
        // The case the owner hit first: watch?v=...&list=RDMM.
        assert!(is_generated_list("RDMM"));
        assert!(is_generated_list("RDCLAK5uy_k"));
        assert!(!is_generated_list("PLWtysTkuEQDPa2kda8p6BYFQLCUz_cElx"));
        assert!(!is_generated_list("OLAK5uy_k"));
        assert!(!is_generated_list("UUabcdef"));
    }

    #[test]
    fn a_flat_playlist_becomes_items_this_app_can_queue() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{
              "_type": "playlist",
              "id": "PL123",
              "title": "Storm Prep",
              "uploader": "paperhurts",
              "entries": [
                {"id": "aaaaaaaaaaa", "title": "One", "duration": 213.0,
                 "url": "https://www.youtube.com/watch?v=aaaaaaaaaaa&list=PL123"},
                {"id": "bbbbbbbbbbb", "duration": null},
                {"title": "no id at all"}
              ]
            }"#,
        )
        .unwrap();
        let probe = playlist_from(&v, "PL123", |id| id == "aaaaaaaaaaa");
        assert_eq!(probe.title, "Storm Prep");
        assert_eq!(probe.uploader.as_deref(), Some("paperhurts"));
        // An entry with no id is not something that can be queued.
        assert_eq!(probe.items.len(), 2);
        // The URL is rebuilt from the id: an entry's own URL carries the list
        // back with it, and a job must name one video and nothing else.
        assert_eq!(
            probe.items[0].url,
            "https://www.youtube.com/watch?v=aaaaaaaaaaa"
        );
        assert_eq!(probe.items[0].duration_s, Some(213.0));
        assert!(probe.items[0].have, "the library already has this one");
        // A flat entry often has no title and no duration; the id stands in.
        assert_eq!(probe.items[1].title, "bbbbbbbbbbb");
        assert_eq!(probe.items[1].duration_s, None);
        assert!(!probe.items[1].have);
    }

    #[test]
    fn looking_for_a_sign_in_elsewhere_names_only_known_stores() {
        // Whatever this machine holds, every answer is the label of a store
        // `cookie_sources` offers — never a path, never a cookie (D115).
        let labels: Vec<String> = cookie_sources().into_iter().map(|s| s.label).collect();
        let found = stores_with_session();
        for l in &found {
            assert!(labels.contains(l), "{l}");
        }
        // And it leaves nothing behind in the temporary directory.
        let left = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(&format!("hp-cookie-peek-{}-", std::process::id()))
            })
            .count();
        assert_eq!(left, 0);
    }

    #[test]
    fn a_cookie_source_builds_its_own_spec_and_label() {
        let plain = source("firefox", None, None);
        assert_eq!(plain.spec, "firefox");
        assert_eq!(plain.label, "firefox");
        // A Chromium profile directory with a display name shows the name and
        // sends the directory, which is what yt-dlp takes.
        let named = source("chrome", Some("Profile 1".into()), Some("Kiddo".into()));
        assert_eq!(named.spec, "chrome:Profile 1");
        assert_eq!(named.label, "chrome — Kiddo");
        // No display name: the directory is the label too.
        let bare = source("chrome", Some("Profile 2".into()), None);
        assert_eq!(bare.label, "chrome — Profile 2");
        assert_eq!(bare.spec, "chrome:Profile 2");
    }

    #[test]
    fn every_source_this_machine_offers_names_a_browser_on_the_list() {
        // Whatever is installed here, nothing invented reaches argv: a spec is
        // always "<browser>" or "<browser>:<profile>" with the browser on the
        // allowlist (D115).
        for s in cookie_sources() {
            assert!(BROWSERS.contains(&s.browser.as_str()), "{}", s.browser);
            let head = s.spec.split(':').next().unwrap();
            assert_eq!(head, s.browser);
            assert!(!s.label.is_empty());
        }
    }

    #[test]
    fn only_browsers_on_the_list_reach_argv() {
        // The value lands in argv, so it is an allowlist, not a sanitiser.
        assert!(BROWSERS.contains(&"firefox"));
        assert!(!BROWSERS.contains(&"--exec"));
        assert!(!BROWSERS.contains(&"safari"));
    }

    #[test]
    fn a_jar_is_counted_by_its_cookie_lines() {
        let dir = std::env::temp_dir().join(format!("hp-jar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("cookies.txt");
        std::fs::write(
            &p,
            "# Netscape HTTP Cookie File
# This file is generated by yt-dlp

.youtube.com	TRUE	/	TRUE	0	PREF	x
.youtube.com	TRUE	/	TRUE	0	SID	y
",
        )
        .unwrap();
        assert_eq!(count_cookies(&p), 2);
        assert_eq!(count_cookies(&dir.join("nothing.txt")), 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_browser_that_refuses_says_which_wall_it_is() {
        let locked = explain_export(
            "brave",
            "ERROR: Could not copy Chrome cookie database. See ...",
        );
        assert!(locked.contains("running"), "{locked}");
        let dpapi = explain_export("chrome", "ERROR: Failed to decrypt with DPAPI. See ...");
        assert!(dpapi.contains("App-Bound"), "{dpapi}");
        // Anything else keeps yt-dlp's words rather than guessing.
        let odd = explain_export("opera", "ERROR: something nobody has seen");
        assert!(odd.contains("something nobody has seen"), "{odd}");
    }

    #[test]
    fn cookies_become_one_flag_and_its_path() {
        let none = ytdlp_args(None, false);
        assert!(!none.contains(&"--cookies".to_string()));
        let with = ytdlp_args(Some(Path::new(r"C:\keys\cookies.txt")), false);
        let at = with
            .iter()
            .position(|a| a == "--cookies")
            .expect("the flag");
        // Its own argv entry, so a path with spaces stays one argument.
        assert_eq!(with[at + 1], r"C:\keys\cookies.txt");
        assert_eq!(with.len(), none.len() + 2);
    }

    #[test]
    fn a_failure_a_person_can_act_on_says_what_to_do_first() {
        let tail = "ERROR: [youtube] aAkI4EKKHMw: Sign in to confirm your age. Use --cookies-from-browser or --cookies";
        let said = ytdlp_message(1, tail, false);
        assert!(
            said.starts_with("YouTube wants a signed-in session"),
            "{said}"
        );
        assert!(
            said.contains("Cookies"),
            "the way out is in the message: {said}"
        );
        // yt-dlp's own words survive: a bug report needs them.
        assert!(said.contains(tail), "{said}");
    }

    #[test]
    fn a_sign_in_refusal_says_something_different_once_cookies_are_set() {
        let tail = "ERROR: [youtube] DgYSM91vJko: Sign in to confirm your age.";
        // Nothing set: ask for a jar.
        let without = ytdlp_message(1, tail, false);
        assert!(
            without.contains("Point the app at a cookies.txt"),
            "{without}"
        );
        // A jar set and still refused: the jar is the problem, and telling
        // someone to do what they have already done is the worst answer.
        let with = ytdlp_message(1, tail, true);
        assert!(with.contains("carry no YouTube sign-in"), "{with}");
        assert!(!with.contains("Point the app at a cookies.txt"), "{with}");
    }

    #[test]
    fn a_jar_is_signed_in_only_with_a_first_party_session() {
        let dir = std::env::temp_dir().join(format!("hp-sess-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("cookies.txt");
        let row = |domain: &str, name: &str| {
            format!(
                "{domain}	TRUE	/	TRUE	0	{name}	x
"
            )
        };
        // What the owner's export actually held: third-party only, which
        // YouTube still treats as signed out.
        std::fs::write(
            &p,
            format!(
                "# Netscape HTTP Cookie File
{}{}{}",
                row(".youtube.com", "__Secure-3PSID"),
                row(".youtube.com", "VISITOR_INFO1_LIVE"),
                row(".google.com", "SID"),
            ),
        )
        .unwrap();
        assert!(!jar_has_youtube_session(&p), "3P alone is not a sign-in");
        std::fs::write(&p, row(".youtube.com", "__Secure-1PSID")).unwrap();
        assert!(jar_has_youtube_session(&p));
        std::fs::write(
            &p,
            "# only a comment
",
        )
        .unwrap();
        assert!(!jar_has_youtube_session(&p));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failure_we_do_not_know_keeps_yt_dlps_words() {
        let said = ytdlp_message(2, "ERROR: unable to rename file: [WinError 32]", false);
        assert!(said.starts_with("yt-dlp failed (2)"), "{said}");
        assert!(said.contains("WinError 32"), "{said}");
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
