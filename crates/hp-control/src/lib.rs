//! hp-control — the public control protocol.
//!
//! **Protocol 1, frozen with v1.0 (#184, D151).** `docs/control-api.md` is the
//! reference, and this crate is its shape in types. From here the protocol
//! grows only by adding: a new command, an optional field, an event, a
//! capability. Anything that would break a client is protocol 2.
//!
//! Two design points inherited from the decision log, both load-bearing:
//!
//! - **D9** — a named pipe, not localhost HTTP. A pipe makes D11's zero-network
//!   guarantee true *by construction* rather than by policy: there is no socket
//!   for anything off-machine to reach, so it cannot be misconfigured open.
//! - **D8/D15** — this is the *only* external surface. Nothing loads into the
//!   player's process; a client runs in its own and speaks NDJSON.
//!
//! The viz channel (binary frames, separate pipe) is in `viz`: what
//! `subscribe_viz` accepts and the frame a subscriber reads. It landed at
//! v0.4b with the analyser, since it's the same data.

use serde::{Deserialize, Serialize};

pub mod viz;
pub use viz::{Depth, Frame, Include, VizParams};

/// Bumped only for breaking changes. Servers reject unknown majors rather than
/// guessing at what a future client meant.
pub const PROTOCOL_VERSION: u32 = 1;

/// The control pipe, by its Windows name. Not `cfg`-gated: a name is a
/// string on every platform, and the app selects its transport behind
/// `platform/pipe.rs`, so this crate carries no platform conditionals at all
/// (#20). Where there are Unix domain sockets instead, `socket_path` says
/// where the socket for a name is.
pub const PIPE_NAME: &str = r"\\.\pipe\hurricane-party";

/// Where the socket for a pipe named here is, on a system with Unix domain
/// sockets (#187): the name after `\\.\pipe\`, with `.sock`, in `dir`.
/// `dir` is the platform's: `$XDG_RUNTIME_DIR` on Linux, so the control
/// socket is `$XDG_RUNTIME_DIR/hurricane-party.sock` (control-api.md) and a
/// viz stream `hurricane-party-viz-7f3a.sock` beside it. Pure, so the
/// player and a client agree on it without either knowing the other's OS.
pub fn socket_path(pipe: &str, dir: &std::path::Path) -> std::path::PathBuf {
    let base = pipe.strip_prefix(r"\\.\pipe\").unwrap_or(pipe);
    dir.join(format!("{base}.sock"))
}

/// Not part of the pipe protocol: the one name the player and its own
/// desktop companion share outside it (#192, D161). The companion holds a
/// named event under this name while it runs; the player's Cap'n Capy box
/// sets it to send him off, however he was started. Here because both
/// programs already depend on this crate, so the name is written once.
pub const COMPANION_LEAVE_EVENT: &str = r"Local\hurricane-party-companion-leave";

/// A request from a client. `id` is echoed back so a client can match replies
/// on a stream that also carries unsolicited events.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Request {
    #[serde(default)]
    pub id: u64,
    pub cmd: String,
    // hello
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u32>,
    /// The capabilities this connection will use (#184). Absent is all of
    /// them, which is what every client written before v1 gets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub want: Option<Vec<String>>,
    // transport
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<f64>,
    // subscribe_viz. All optional: the defaults are the LED wall's numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bands: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_hz: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    // the library (#182)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub q: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playlist_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Response {
    pub id: u64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn ok(id: u64, result: serde_json::Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(id: u64, msg: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(msg.into()),
        }
    }
}

/// Playback state, mirrored from the webview.
///
/// The audio graph lives in the webview (D5), so Rust is not the source of
/// truth here — it caches what the frontend last reported. `control-api.md`
/// flags the latency cost of that hop; it's real, and it's the price of getting
/// `AnalyserNode` and `BiquadFilterNode` for free.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlayerState {
    pub state: String, // "playing" | "paused" | "stopped"
    /// Which transport this describes: `"audio"` or `"video"` (D70). One
    /// thing plays at a time (D69), so `status` is always one of these, and a
    /// client that never looks still gets the right `state`. Additive: absent
    /// on the wire means audio.
    #[serde(default = "kind_audio")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploader: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pos_s: Option<f64>,
    pub volume: f64,
}

fn kind_audio() -> String {
    "audio".into()
}

/// What `status` answers: the transport's state, plus the play order's two
/// switches (#115, D97). Flat on the wire — a client reading `state` and
/// `volume` sees exactly what it saw before — and `shuffle` and `repeat` are
/// additive. They come from the library, which owns the play order (D74), not
/// from the window that reported `player`.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    #[serde(flatten)]
    pub player: PlayerState,
    pub shuffle: bool,
    /// `"off"`, `"one"` or `"all"`.
    pub repeat: String,
}

/// Unsolicited. No `id`, which is how a client tells them from replies.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event")]
pub enum Event {
    #[serde(rename = "now_playing_changed")]
    NowPlaying {
        /// `"audio"` or `"video"` (D70), so a client can tell a track from a
        /// video without a second call.
        kind: String,
        media_id: Option<i64>,
        title: Option<String>,
        uploader: Option<String>,
        duration_s: Option<f64>,
    },
    #[serde(rename = "state_changed")]
    StateChanged { state: String },
    /// Where the windows are and how they are bonded (#181). Sent when that
    /// changes, at most every 50 ms while a drag runs, and always once after
    /// the last change; the `layout` command answers with the same shape.
    #[serde(rename = "layout_changed")]
    LayoutChanged(LayoutInfo),
    /// The analyser's colours, when the skin or the theme changes them (#183):
    /// the 24-entry ramp the skin manifest defines, `#rrggbb`, darkest first,
    /// so an LED wall can change colour scheme with the windows.
    #[serde(rename = "palette_changed")]
    PaletteChanged { viscolor: Vec<String> },
}

/// Every window a client could put something on, and the bond graph between
/// the classic three (#181). Physical pixels, the app's own convention, so a
/// client compositing against these never guesses a scale factor.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutInfo {
    pub windows: Vec<WindowRect>,
    pub bonds: Vec<BondRect>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowRect {
    /// `main`, `eq` or `playlist` in the bond group; `library`, `video` or
    /// `prep` for a decorated window, listed while it exists.
    pub id: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// One of the three classic windows, which bond. A decorated window is
    /// never in `bonds`.
    pub group: bool,
    /// Collapsed to the windowshade strip (D60): `h` is the strip's.
    pub shaded: bool,
    /// Shown and not minimised. A window that is not visible keeps its last
    /// rectangle, which is where it will come back.
    pub visible: bool,
}

/// One playlist, as `playlists` lists it (#182). `count` is what can play
/// now: a track on a drive that is not plugged in is not counted (D143).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistSummary {
    pub id: i64,
    pub name: String,
    pub count: i64,
    /// Fills itself from a rule (D144).
    pub smart: bool,
}

/// One track, as `search` returns it (#182). `id` is what `play` takes as
/// `media_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackSummary {
    pub id: i64,
    pub title: String,
    pub uploader: Option<String>,
    /// `"audio"` or `"video"`.
    pub kind: String,
    pub duration_s: Option<f64>,
}

/// What `search` answers: at most `SEARCH_LIMIT` tracks, newest first, and
/// how many matched in all.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub total: usize,
    pub tracks: Vec<TrackSummary>,
}

/// The most tracks one `search` returns (#182, the owner's call).
pub const SEARCH_LIMIT: usize = 50;

/// How many colours an analyser ramp has: the classic `VISCOLOR.TXT`'s 24,
/// which the skin manifest keeps (#183).
pub const RAMP_LEN: usize = 24;

/// A ramp as the pipe promises it (#183): exactly 24 colours, each `#rrggbb`
/// in lower case. Anything else is refused with the reason, so a malformed
/// ramp never reaches a client.
pub fn ramp(colours: &[String]) -> Result<Vec<String>, String> {
    if colours.len() != RAMP_LEN {
        return Err(format!(
            "a ramp has {RAMP_LEN} colours, not {}",
            colours.len()
        ));
    }
    colours
        .iter()
        .map(|c| {
            let ok =
                c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|d| d.is_ascii_hexdigit());
            if ok {
                Ok(c.to_ascii_lowercase())
            } else {
                Err(format!("{c:?} is not a #rrggbb colour"))
            }
        })
        .collect()
}

/// Two classic windows sharing an edge (D16). `edge` is the side of `a` that
/// `b` sits against; `span` is the shared stretch along it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BondRect {
    pub a: String,
    pub b: String,
    pub edge: String,
    pub span: [i32; 2],
}

/// Commands the player is expected to act on, parsed from the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Hello {
        client: String,
        protocol_version: u32,
        /// What the client asked to use (#184); `None` is everything.
        want: Option<Vec<String>>,
    },
    Status,
    /// Where the windows are now (#181): the `layout_changed` shape, for a
    /// client that connects after the last change.
    Layout,
    /// Every playlist (#182).
    Playlists,
    /// Tracks whose title or artist hold every word, blind to case and
    /// accents, as the library's search box matches (#182).
    Search(String),
    /// Make a playlist the queue and start it from the top (#182).
    QueuePlaylist(i64),
    /// `play` with a `media_id`: play that one track (#182).
    PlayMedia(i64),
    /// The analyser's colours now (#183): the `palette_changed` shape, for a
    /// client that connects after the last change.
    Palette,
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Prev,
    Seek(f64),
    Volume(f64),
    /// Open a per-subscriber pipe and push frames down it (`viz`).
    SubscribeViz(VizParams),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ParseError {
    #[error("unknown command {0:?}")]
    UnknownCommand(String),
    #[error("{cmd:?} needs {field}")]
    MissingField { cmd: String, field: &'static str },
    #[error("{cmd:?}: {field} {why}")]
    InvalidField {
        cmd: String,
        field: &'static str,
        why: String,
    },
    #[error("protocol version {0} is not supported (this server speaks {PROTOCOL_VERSION})")]
    UnsupportedVersion(u32),
}

impl Request {
    pub fn parse(&self) -> Result<Command, ParseError> {
        let missing = |f| ParseError::MissingField {
            cmd: self.cmd.clone(),
            field: f,
        };
        Ok(match self.cmd.as_str() {
            "hello" => {
                let v = self
                    .protocol_version
                    .ok_or_else(|| missing("protocol_version"))?;
                // Reject rather than guess: a client speaking a future major
                // wants a clear no, not silently-wrong behaviour.
                if v != PROTOCOL_VERSION {
                    return Err(ParseError::UnsupportedVersion(v));
                }
                // Asked for by name, so a typo is a refusal that says what
                // exists rather than a connection that quietly can do less.
                let want = match &self.want {
                    None => None,
                    Some(w) if w.is_empty() => {
                        return Err(ParseError::InvalidField {
                            cmd: self.cmd.clone(),
                            field: "want",
                            why: "asks for nothing".into(),
                        })
                    }
                    Some(w) => {
                        if let Some(bad) = w.iter().find(|c| !CAPABILITIES.contains(&c.as_str())) {
                            return Err(ParseError::InvalidField {
                                cmd: self.cmd.clone(),
                                field: "want",
                                why: format!("{bad:?} is not one of {CAPABILITIES:?}"),
                            });
                        }
                        Some(w.clone())
                    }
                };
                Command::Hello {
                    client: self.client.clone().unwrap_or_else(|| "unknown".into()),
                    protocol_version: v,
                    want,
                }
            }
            "status" => Command::Status,
            "layout" => Command::Layout,
            "play" => match self.media_id {
                Some(id) => Command::PlayMedia(id),
                None => Command::Play,
            },
            "playlists" => Command::Playlists,
            "palette" => Command::Palette,
            // An empty search is refused rather than answered with the whole
            // library: a remote that sends nothing has a bug, not a question.
            "search" => match self.q.as_deref().map(str::trim) {
                Some(q) if !q.is_empty() => Command::Search(q.to_string()),
                Some(_) => {
                    return Err(ParseError::InvalidField {
                        cmd: self.cmd.clone(),
                        field: "q",
                        why: "is empty".into(),
                    })
                }
                None => return Err(missing("q")),
            },
            "queue_playlist" => {
                Command::QueuePlaylist(self.playlist_id.ok_or_else(|| missing("playlist_id"))?)
            }
            "pause" => Command::Pause,
            "toggle" => Command::Toggle,
            "stop" => Command::Stop,
            "next" => Command::Next,
            "prev" => Command::Prev,
            "seek" => Command::Seek(self.pos_s.ok_or_else(|| missing("pos_s"))?),
            // Clamped rather than rejected: a client that sends 1.5 means
            // "loud", and refusing is less useful than doing the sane thing.
            "volume" => {
                Command::Volume(self.level.ok_or_else(|| missing("level"))?.clamp(0.0, 1.0))
            }
            // Validated rather than clamped, unlike volume: a rig asking for
            // 200 bands or 45 Hz has a wrong idea of the protocol, and a frame
            // shaped differently from what it asked for would be worse than a
            // refusal it can read.
            "subscribe_viz" => {
                let invalid = |field, why: String| ParseError::InvalidField {
                    cmd: self.cmd.clone(),
                    field,
                    why,
                };
                let d = VizParams::default();
                let bands = match self.bands {
                    None => d.bands,
                    Some(b) if (viz::MIN_BANDS as u32..=viz::MAX_BANDS as u32).contains(&b) => {
                        b as u8
                    }
                    Some(b) => {
                        return Err(invalid(
                            "bands",
                            format!("must be {}..={}, not {b}", viz::MIN_BANDS, viz::MAX_BANDS),
                        ))
                    }
                };
                let rate_hz = match self.rate_hz {
                    None => d.rate_hz,
                    Some(r) if viz::RATES_HZ.contains(&r) => r,
                    Some(r) => {
                        return Err(invalid(
                            "rate_hz",
                            format!("must be one of {:?}, not {r}", viz::RATES_HZ),
                        ))
                    }
                };
                let depth = match &self.depth {
                    None => d.depth,
                    Some(s) => Depth::parse(s)
                        .ok_or_else(|| invalid("depth", format!("must be u8 or f32, not {s:?}")))?,
                };
                let include = match &self.include {
                    None => d.include,
                    Some(names) => Include::parse(names).map_err(|e| invalid("include", e))?,
                };
                Command::SubscribeViz(VizParams {
                    bands,
                    rate_hz,
                    depth,
                    include,
                })
            }
            other => return Err(ParseError::UnknownCommand(other.to_string())),
        })
    }
}

/// Everything this build does, by the word a client asks for it by. Each
/// joined as it was built: "layout" #181, "library" #182, "palette" #183.
pub const CAPABILITIES: [&str; 5] = ["transport", "viz", "layout", "library", "palette"];

impl Command {
    /// The capability a command belongs to (#184), which a connection must
    /// have been granted at `hello`. `hello` itself belongs to none.
    pub fn capability(&self) -> Option<&'static str> {
        Some(match self {
            Command::Hello { .. } => return None,
            Command::Status
            | Command::Play
            | Command::Pause
            | Command::Toggle
            | Command::Stop
            | Command::Next
            | Command::Prev
            | Command::Seek(_)
            | Command::Volume(_) => "transport",
            Command::SubscribeViz(_) => "viz",
            Command::Layout => "layout",
            Command::Playlists
            | Command::Search(_)
            | Command::QueuePlaylist(_)
            | Command::PlayMedia(_) => "library",
            Command::Palette => "palette",
        })
    }
}

impl Event {
    /// The capability an event belongs to (#184): a connection hears only the
    /// events of what it was granted.
    pub fn capability(&self) -> &'static str {
        match self {
            Event::NowPlaying { .. } | Event::StateChanged { .. } => "transport",
            Event::LayoutChanged(_) => "layout",
            Event::PaletteChanged { .. } => "palette",
        }
    }
}

/// The capabilities a `want` grants: those asked for, in `CAPABILITIES`'
/// order, or all of them when nothing was asked.
pub fn granted(want: Option<&[String]>) -> Vec<&'static str> {
    match want {
        None => CAPABILITIES.to_vec(),
        Some(w) => CAPABILITIES
            .iter()
            .copied()
            .filter(|c| w.iter().any(|x| x == c))
            .collect(),
    }
}

/// What `hello` reports. `capabilities` is how a client discovers what this
/// build supports without version-sniffing; `granted` is what this connection
/// may use (#184). Stable from protocol 1 on (#184): changes only by adding.
pub fn hello_result(app_version: &str, granted: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "protocol_version": PROTOCOL_VERSION,
        "app_version": app_version,
        "capabilities": CAPABILITIES,
        "granted": granted,
        "stable": true,
    })
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_pipe_name_is_a_socket_in_the_runtime_dir() {
        let dir = std::path::Path::new("/run/user/1000");
        assert_eq!(
            socket_path(PIPE_NAME, dir),
            dir.join("hurricane-party.sock")
        );
        assert_eq!(
            socket_path(&viz::pipe_name(0x7f3a), dir),
            dir.join("hurricane-party-viz-7f3a.sock")
        );
    }

    use super::*;

    fn req(json: &str) -> Request {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn parses_the_handshake() {
        let r = req(r#"{"id":0,"cmd":"hello","client":"led-bridge","protocol_version":1}"#);
        assert_eq!(
            r.parse().unwrap(),
            Command::Hello {
                client: "led-bridge".into(),
                protocol_version: 1,
                want: None,
            }
        );
    }

    /// #184: a client may ask for part of the protocol by name; a word that
    /// is not a capability, or asking for nothing, is refused with the list.
    #[test]
    fn hello_can_ask_for_part_of_the_protocol() {
        let r =
            req(r#"{"id":0,"cmd":"hello","client":"bars","protocol_version":1,"want":["viz"]}"#);
        assert_eq!(
            r.parse().unwrap(),
            Command::Hello {
                client: "bars".into(),
                protocol_version: 1,
                want: Some(vec!["viz".into()]),
            }
        );
        let typo = req(r#"{"id":0,"cmd":"hello","protocol_version":1,"want":["vis"]}"#)
            .parse()
            .unwrap_err();
        assert!(
            matches!(&typo, ParseError::InvalidField { field: "want", why, .. } if why.contains("\"vis\"") && why.contains("viz")),
            "{typo:?}"
        );
        assert!(matches!(
            req(r#"{"id":0,"cmd":"hello","protocol_version":1,"want":[]}"#)
                .parse()
                .unwrap_err(),
            ParseError::InvalidField { field: "want", .. }
        ));

        // What a want grants, in the protocol's own order.
        assert_eq!(granted(None), CAPABILITIES.to_vec());
        assert_eq!(
            granted(Some(&["palette".into(), "viz".into()])),
            ["viz", "palette"]
        );
    }

    /// #184: every command but hello, and every event, belongs to exactly one
    /// capability, and each capability has something in it.
    #[test]
    fn every_command_and_event_belongs_to_a_capability() {
        let cases = [
            (r#"{"cmd":"status"}"#, "transport"),
            (r#"{"cmd":"next"}"#, "transport"),
            (r#"{"cmd":"seek","pos_s":1}"#, "transport"),
            (r#"{"cmd":"subscribe_viz"}"#, "viz"),
            (r#"{"cmd":"layout"}"#, "layout"),
            (r#"{"cmd":"playlists"}"#, "library"),
            (r#"{"cmd":"search","q":"x"}"#, "library"),
            (r#"{"cmd":"queue_playlist","playlist_id":1}"#, "library"),
            (r#"{"cmd":"play","media_id":1}"#, "library"),
            (r#"{"cmd":"play"}"#, "transport"),
            (r#"{"cmd":"palette"}"#, "palette"),
        ];
        for (json, cap) in cases {
            assert_eq!(req(json).parse().unwrap().capability(), Some(cap), "{json}");
        }
        assert_eq!(
            req(r#"{"cmd":"hello","protocol_version":1}"#)
                .parse()
                .unwrap()
                .capability(),
            None
        );
        for c in CAPABILITIES {
            assert!(cases.iter().any(|(_, k)| *k == c), "{c} has no command");
        }
        assert_eq!(
            Event::StateChanged {
                state: "paused".into()
            }
            .capability(),
            "transport"
        );
        assert_eq!(
            Event::LayoutChanged(LayoutInfo::default()).capability(),
            "layout"
        );
        assert_eq!(
            Event::PaletteChanged { viscolor: vec![] }.capability(),
            "palette"
        );
    }

    /// A future major must get a clear refusal. Guessing is how a protocol
    /// becomes impossible to change later.
    #[test]
    fn rejects_a_protocol_version_it_does_not_speak() {
        let r = req(r#"{"id":0,"cmd":"hello","client":"x","protocol_version":2}"#);
        assert_eq!(r.parse(), Err(ParseError::UnsupportedVersion(2)));
    }

    #[test]
    fn parses_transport_commands() {
        for (json, want) in [
            (r#"{"id":1,"cmd":"toggle"}"#, Command::Toggle),
            (r#"{"id":2,"cmd":"play"}"#, Command::Play),
            (r#"{"id":3,"cmd":"next"}"#, Command::Next),
            (r#"{"id":4,"cmd":"seek","pos_s":42.5}"#, Command::Seek(42.5)),
        ] {
            assert_eq!(req(json).parse().unwrap(), want);
        }
    }

    #[test]
    fn volume_is_clamped_not_refused() {
        assert_eq!(
            req(r#"{"cmd":"volume","level":1.7}"#).parse().unwrap(),
            Command::Volume(1.0)
        );
        assert_eq!(
            req(r#"{"cmd":"volume","level":-2.0}"#).parse().unwrap(),
            Command::Volume(0.0)
        );
        assert_eq!(
            req(r#"{"cmd":"volume","level":0.7}"#).parse().unwrap(),
            Command::Volume(0.7)
        );
    }

    #[test]
    fn a_command_missing_its_argument_says_which_one() {
        let e = req(r#"{"id":9,"cmd":"seek"}"#).parse().unwrap_err();
        assert_eq!(e.to_string(), "\"seek\" needs pos_s");
    }

    #[test]
    fn unknown_commands_are_rejected_by_name() {
        // set_eq is deliberately NOT in the API (architecture.md): the public
        // surface stays small, and adding it later is additive.
        let e = req(r#"{"id":1,"cmd":"set_eq"}"#).parse().unwrap_err();
        assert_eq!(e, ParseError::UnknownCommand("set_eq".into()));
    }

    /// Events carry no `id`; that is how a client separates them from replies
    /// on the same stream.
    #[test]
    fn events_serialise_without_an_id() {
        let e = Event::StateChanged {
            state: "paused".into(),
        };
        let j: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(j["event"], "state_changed");
        assert_eq!(j["state"], "paused");
        assert!(j.get("id").is_none());
    }

    /// `status` is flat: a client written before shuffle and repeat existed
    /// reads the same keys in the same places, and the new two sit beside
    /// them rather than under a new object (#115).
    #[test]
    fn status_is_the_player_state_flat_with_the_play_mode_beside_it() {
        let s = Status {
            player: PlayerState {
                state: "playing".into(),
                kind: "audio".into(),
                media_id: Some(7),
                volume: 0.5,
                ..Default::default()
            },
            shuffle: true,
            repeat: "all".into(),
        };
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(j["state"], "playing");
        assert_eq!(j["kind"], "audio");
        assert_eq!(j["media_id"], 7);
        assert_eq!(j["volume"], 0.5);
        assert_eq!(j["shuffle"], true);
        assert_eq!(j["repeat"], "all");
        assert!(j.get("player").is_none(), "flattened, not nested");
    }

    /// `kind` is additive (D70): a report or a client that never says it
    /// means audio, and the wire always carries it so a client can rely on it.
    #[test]
    fn kind_defaults_to_audio_and_is_always_written() {
        let s: PlayerState = serde_json::from_str(r#"{"state":"playing","volume":1.0}"#).unwrap();
        assert_eq!(s.kind, "audio");
        let v: PlayerState =
            serde_json::from_str(r#"{"state":"playing","kind":"video","volume":0.5}"#).unwrap();
        assert_eq!(v.kind, "video");
        let j: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(j["kind"], "audio");
        let e = Event::NowPlaying {
            kind: "video".into(),
            media_id: Some(7),
            title: None,
            uploader: None,
            duration_s: None,
        };
        let j: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(j["kind"], "video");
        assert_eq!(j["media_id"], 7);
    }

    #[test]
    fn responses_omit_the_field_they_did_not_use() {
        let ok = serde_json::to_string(&Response::ok(1, serde_json::json!({"a":1}))).unwrap();
        assert!(ok.contains("\"ok\":true") && !ok.contains("error"));
        let err = serde_json::to_string(&Response::err(2, "nope")).unwrap();
        assert!(err.contains("\"ok\":false") && !err.contains("result"));
    }

    /// #183: the palette event is the documented shape, `palette` parses, and
    /// a ramp is 24 lower-case `#rrggbb` or nothing.
    #[test]
    fn the_palette_is_24_colours_or_refused() {
        let theme: Vec<String> = (0..24).map(|i| format!("#0A{i:02X}FF")).collect();
        let r = ramp(&theme).unwrap();
        assert_eq!(r[0], "#0a00ff");
        assert_eq!(r[23], "#0a17ff");
        let e = Event::PaletteChanged {
            viscolor: r.clone(),
        };
        let j = serde_json::to_value(&e).unwrap();
        assert_eq!(j["event"], "palette_changed");
        assert_eq!(j["viscolor"].as_array().unwrap().len(), 24);
        assert!(j.get("id").is_none());
        assert_eq!(
            req(r#"{"id":1,"cmd":"palette"}"#).parse().unwrap(),
            Command::Palette
        );

        assert!(ramp(&theme[..23]).unwrap_err().contains("not 23"));
        let mut bad = theme.clone();
        bad[5] = "rgb(1,2,3)".into();
        assert!(ramp(&bad).unwrap_err().contains("rgb(1,2,3)"));
        bad[5] = "#12345".into();
        assert!(ramp(&bad).is_err());
    }

    /// #182: the library commands parse, a bare `play` is still the
    /// transport's, and what they need is asked for by name.
    #[test]
    fn the_library_commands_parse_and_say_what_is_missing() {
        assert_eq!(
            req(r#"{"id":1,"cmd":"playlists"}"#).parse().unwrap(),
            Command::Playlists
        );
        assert_eq!(
            req(r#"{"id":2,"cmd":"search","q":"  cure  "}"#)
                .parse()
                .unwrap(),
            Command::Search("cure".into())
        );
        assert_eq!(
            req(r#"{"id":3,"cmd":"queue_playlist","playlist_id":12}"#)
                .parse()
                .unwrap(),
            Command::QueuePlaylist(12)
        );
        assert_eq!(
            req(r#"{"id":4,"cmd":"play","media_id":89}"#)
                .parse()
                .unwrap(),
            Command::PlayMedia(89)
        );
        assert_eq!(
            req(r#"{"id":5,"cmd":"play"}"#).parse().unwrap(),
            Command::Play
        );
        assert!(matches!(
            req(r#"{"id":6,"cmd":"search"}"#).parse().unwrap_err(),
            ParseError::MissingField { field: "q", .. }
        ));
        assert!(matches!(
            req(r#"{"id":7,"cmd":"search","q":"   "}"#)
                .parse()
                .unwrap_err(),
            ParseError::InvalidField { field: "q", .. }
        ));
        assert!(matches!(
            req(r#"{"id":8,"cmd":"queue_playlist"}"#)
                .parse()
                .unwrap_err(),
            ParseError::MissingField {
                field: "playlist_id",
                ..
            }
        ));
    }

    /// #181: the layout event is flat, as control-api.md draws it: the tag
    /// beside `windows` and `bonds`, spans as two-element arrays, and the
    /// command that asks for it now parses.
    #[test]
    fn the_layout_event_is_the_documented_shape() {
        let e = Event::LayoutChanged(LayoutInfo {
            windows: vec![
                WindowRect {
                    id: "main".into(),
                    x: 420,
                    y: 300,
                    w: 550,
                    h: 232,
                    group: true,
                    shaded: false,
                    visible: true,
                },
                WindowRect {
                    id: "library".into(),
                    x: -1200,
                    y: 80,
                    w: 916,
                    h: 659,
                    group: false,
                    shaded: false,
                    visible: false,
                },
            ],
            bonds: vec![BondRect {
                a: "main".into(),
                b: "playlist".into(),
                edge: "bottom".into(),
                span: [420, 970],
            }],
        });
        let j: serde_json::Value = serde_json::to_value(&e).unwrap();
        assert_eq!(
            j,
            serde_json::json!({
                "event": "layout_changed",
                "windows": [
                    {"id":"main","x":420,"y":300,"w":550,"h":232,"group":true,"shaded":false,"visible":true},
                    {"id":"library","x":-1200,"y":80,"w":916,"h":659,"group":false,"shaded":false,"visible":false}
                ],
                "bonds": [{"a":"main","b":"playlist","edge":"bottom","span":[420,970]}]
            })
        );
        assert!(j.get("id").is_none());
        assert_eq!(
            req(r#"{"id":4,"cmd":"layout"}"#).parse().unwrap(),
            Command::Layout
        );
    }

    #[test]
    fn hello_advertises_what_is_built_what_is_granted_and_that_it_is_stable() {
        let h = hello_result("1.0.0", &["viz"]);
        assert_eq!(
            h["capabilities"],
            serde_json::json!(["transport", "viz", "layout", "library", "palette"])
        );
        assert_eq!(h["granted"], serde_json::json!(["viz"]));
        assert_eq!(h["protocol_version"], 1);
        // #184: the freeze. From here the protocol changes only by adding.
        assert_eq!(h["stable"], true);
    }

    /// A bare subscribe is the LED wall's numbers from control-api.md.
    #[test]
    fn subscribe_viz_defaults_to_the_led_wall() {
        let c = req(r#"{"id":7,"cmd":"subscribe_viz"}"#).parse().unwrap();
        assert_eq!(c, Command::SubscribeViz(VizParams::default()));
        let Command::SubscribeViz(p) = c else {
            unreachable!()
        };
        assert_eq!((p.bands, p.rate_hz, p.depth), (32, 30, Depth::U8));
        assert!(p.include.spectrum && p.include.level && p.include.beat);
    }

    #[test]
    fn subscribe_viz_takes_every_field() {
        let c = req(
            r#"{"id":7,"cmd":"subscribe_viz","bands":128,"rate_hz":60,"depth":"f32","include":["spectrum"]}"#,
        )
        .parse()
        .unwrap();
        assert_eq!(
            c,
            Command::SubscribeViz(VizParams {
                bands: 128,
                rate_hz: 60,
                depth: Depth::F32,
                include: Include {
                    spectrum: true,
                    level: false,
                    beat: false
                },
            })
        );
    }

    /// Refused by name, not clamped: a frame shaped differently from what a
    /// rig asked for is worse than a refusal it can read.
    #[test]
    fn subscribe_viz_refuses_out_of_range_values_by_field() {
        for (json, field) in [
            (r#"{"cmd":"subscribe_viz","bands":4}"#, "bands"),
            (r#"{"cmd":"subscribe_viz","bands":129}"#, "bands"),
            (r#"{"cmd":"subscribe_viz","rate_hz":45}"#, "rate_hz"),
            (r#"{"cmd":"subscribe_viz","depth":"u16"}"#, "depth"),
            (r#"{"cmd":"subscribe_viz","include":["beet"]}"#, "include"),
        ] {
            match req(json).parse() {
                Err(ParseError::InvalidField { field: f, .. }) => assert_eq!(f, field),
                other => panic!("{json}: expected InvalidField, got {other:?}"),
            }
        }
        let e = req(r#"{"cmd":"subscribe_viz","bands":4}"#)
            .parse()
            .unwrap_err();
        assert_eq!(
            e.to_string(),
            "\"subscribe_viz\": bands must be 8..=128, not 4"
        );
    }
}
