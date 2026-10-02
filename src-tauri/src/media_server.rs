//! The loopback media server, where the webview cannot play from the asset
//! protocol (D185).
//!
//! WebKitGTK plays `<audio>` and `<video>` through GStreamer, and GStreamer
//! fetches only over http(s), `file:` and `blob:`: a custom scheme like
//! `asset://` is refused with MEDIA_ERR_SRC_NOT_SUPPORTED before a pipeline
//! is built, while an `<img>` from the same scheme loads (measured on #187).
//! So on Linux the player's media comes from here instead: a small HTTP/1.1
//! server on 127.0.0.1, on a port the OS picks, answering only requests that
//! carry the secret made at launch, and serving only files the asset
//! protocol's own scope allows. GET and HEAD, with byte ranges, so a track
//! seeks and a long video streams rather than loading whole.
//!
//! It is not the network: nothing is fetched, and nothing listens beyond the
//! loopback address. It is the one local socket beside the control pipe, and
//! carries nothing but the bytes of files the app could already show.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};

/// `http://127.0.0.1:<port>/<secret>`, once the server is up. A file's URL is
/// this, a slash, and its absolute path, percent-encoded as one segment.
static BASE: OnceLock<String> = OnceLock::new();

/// The base URL media is played from, or None where the asset protocol does
/// the job (Windows) or the server could not start.
pub fn base() -> Option<String> {
    BASE.get().cloned()
}

/// 128 bits from the OS's random source. Not std's `RandomState`: two made
/// on one thread are correlated, which a hash key may be and a secret may not.
fn secret() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the OS has no random source");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Start the server, once, where the platform needs it.
pub fn start(app: &AppHandle) {
    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("media: no loopback server, nothing will play: {e}");
            return;
        }
    };
    let Ok(addr) = listener.local_addr() else {
        return;
    };
    let token = secret();
    let _ = BASE.set(format!("http://127.0.0.1:{}/{token}", addr.port()));
    let app = app.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let app = app.clone();
            let token = token.clone();
            std::thread::spawn(move || {
                let _ = serve(&app, stream, &token);
            });
        }
    });
}

/// One request: what was asked for, from which byte to which.
#[derive(Debug, PartialEq)]
pub struct Request {
    pub head: bool,
    pub path: PathBuf,
    pub range: Option<String>,
}

/// Read a request's line and headers. None for anything that is not a GET
/// or HEAD of `/<token>/<path>` with an absolute path: the secret is what
/// keeps another program on this machine from reading through the player.
pub fn parse(lines: &[String], token: &str) -> Option<Request> {
    let mut first = lines.first()?.split_whitespace();
    let head = match first.next()? {
        "GET" => false,
        "HEAD" => true,
        _ => return None,
    };
    let target = first.next()?;
    let rest = target
        .strip_prefix('/')?
        .strip_prefix(token)?
        .strip_prefix('/')?;
    let rest = rest.split(['?', '#']).next()?;
    let decoded = percent_encoding::percent_decode_str(rest)
        .decode_utf8()
        .ok()?;
    let path = PathBuf::from(decoded.as_ref());
    if !path.is_absolute() || path.components().any(|c| c.as_os_str() == "..") {
        return None;
    }
    let range = lines.iter().skip(1).find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim()
            .eq_ignore_ascii_case("range")
            .then(|| v.trim().to_string())
    });
    Some(Request { head, path, range })
}

/// A `Range: bytes=…` header against a file of `len` bytes: the first and
/// last byte to send, inclusive. None for no range or one that cannot be
/// served, which is answered with the whole file or 416.
pub fn byte_range(range: &str, len: u64) -> Option<(u64, u64)> {
    let spec = range.strip_prefix("bytes=")?;
    // One range only; a player asks for one at a time.
    let (a, b) = spec.split(',').next()?.trim().split_once('-')?;
    if len == 0 {
        return None;
    }
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            (len.saturating_sub(n), len - 1)
        }
        (s, "") => (s.parse().ok()?, len - 1),
        (s, e) => (s.parse().ok()?, e.parse::<u64>().ok()?.min(len - 1)),
    };
    (start <= end && start < len).then_some((start, end))
}

/// What a browser should be told a file is, by its extension.
pub fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("mp3") => "audio/mpeg",
        Some("m4a") | Some("aac") => "audio/mp4",
        Some("opus") | Some("ogg") | Some("oga") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("wav") => "audio/wav",
        Some("mp4") | Some("m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        _ => "application/octet-stream",
    }
}

fn serve(app: &AppHandle, stream: TcpStream, token: &str) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        lines.push(line.trim_end().to_string());
        if lines.len() > 64 {
            break;
        }
    }
    let mut out = stream;
    let Some(req) = parse(&lines, token) else {
        return status(&mut out, "404 Not Found");
    };
    // The asset protocol's scope, the same rule an `<img>` from the library
    // gets: app data, and the library roots `localimport` allows (D29).
    if !app.asset_protocol_scope().is_allowed(&req.path) {
        return status(&mut out, "403 Forbidden");
    }
    let Ok(mut file) = std::fs::File::open(&req.path) else {
        return status(&mut out, "404 Not Found");
    };
    let len = file.metadata()?.len();
    let ranged = req.range.as_deref().map(|r| byte_range(r, len));
    let (code, start, end) = match ranged {
        Some(Some((s, e))) => ("206 Partial Content", s, e),
        Some(None) => {
            write!(
                out,
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{len}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
            return Ok(());
        }
        None => ("200 OK", 0, len.saturating_sub(1)),
    };
    let count = if len == 0 { 0 } else { end - start + 1 };
    write!(
        out,
        "HTTP/1.1 {code}\r\nContent-Type: {}\r\nContent-Length: {count}\r\nAccept-Ranges: bytes\r\n",
        content_type(&req.path)
    )?;
    if code.starts_with("206") {
        write!(out, "Content-Range: bytes {start}-{end}/{len}\r\n")?;
    }
    // Main plays with crossorigin="anonymous" so the EQ's audio graph can
    // read the samples (D75); the page's origin is tauri://localhost.
    write!(
        out,
        "Access-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    )?;
    if req.head || count == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::Start(start))?;
    std::io::copy(&mut file.take(count), &mut out)?;
    Ok(())
}

fn status(out: &mut TcpStream, code: &str) -> std::io::Result<()> {
    write!(
        out,
        "HTTP/1.1 {code}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

/// The command the player asks once: where to play media from, or None for
/// the asset protocol.
#[tauri::command]
pub fn media_base() -> Option<String> {
    base()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(line: &str, extra: &[&str]) -> Vec<String> {
        std::iter::once(line)
            .chain(extra.iter().copied())
            .map(String::from)
            .collect()
    }

    /// A path as the page sends it: absolute, percent-encoded as one segment.
    fn enc(p: &Path) -> String {
        percent_encoding::utf8_percent_encode(
            p.to_str().unwrap(),
            percent_encoding::NON_ALPHANUMERIC,
        )
        .to_string()
    }

    #[test]
    fn only_the_secret_and_an_absolute_path_get_a_file() {
        let t = "abc123";
        // Absolute on whatever runs the test: a drive letter on Windows.
        let file = std::env::temp_dir().join("a b[1].mp3");
        let ok = parse(
            &req(
                &format!("GET /abc123/{} HTTP/1.1", enc(&file)),
                &["Range: bytes=0-"],
            ),
            t,
        )
        .unwrap();
        assert_eq!(ok.path, file);
        assert!(!ok.head);
        assert_eq!(ok.range.as_deref(), Some("bytes=0-"));
        let head = format!("HEAD /abc123/{} HTTP/1.1", enc(&file));
        assert!(parse(&req(&head, &[]), t).unwrap().head);
        // No secret, the wrong one, a relative path, a climb, another verb.
        let at = |line: String| parse(&req(&line, &[]), t);
        assert_eq!(at(format!("GET /{} HTTP/1.1", enc(&file))), None);
        assert_eq!(at(format!("GET /nope/{} HTTP/1.1", enc(&file))), None);
        assert_eq!(at("GET /abc123/x.mp3 HTTP/1.1".into()), None);
        let climb = std::env::temp_dir().join("a").join("..").join("passwd");
        assert_eq!(at(format!("GET /abc123/{} HTTP/1.1", enc(&climb))), None);
        assert_eq!(at(format!("POST /abc123/{} HTTP/1.1", enc(&file))), None);
        assert_eq!(parse(&[], t), None);
    }

    #[test]
    fn byte_ranges_are_read_as_a_player_asks() {
        assert_eq!(byte_range("bytes=0-", 1000), Some((0, 999)));
        assert_eq!(byte_range("bytes=100-199", 1000), Some((100, 199)));
        assert_eq!(byte_range("bytes=900-5000", 1000), Some((900, 999)));
        assert_eq!(byte_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(byte_range("bytes=1000-", 1000), None);
        assert_eq!(byte_range("bytes=5-1", 1000), None);
        assert_eq!(byte_range("items=0-1", 1000), None);
        assert_eq!(byte_range("bytes=0-", 0), None);
    }

    #[test]
    fn a_file_is_typed_by_its_extension() {
        assert_eq!(content_type(Path::new("/a/b.MP3")), "audio/mpeg");
        assert_eq!(content_type(Path::new("/a/b.webm")), "video/webm");
        assert_eq!(content_type(Path::new("/a/b.mp4")), "video/mp4");
        assert_eq!(content_type(Path::new("/a/b")), "application/octet-stream");
    }

    #[test]
    fn every_launch_has_its_own_secret() {
        let (a, b) = (secret(), secret());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
