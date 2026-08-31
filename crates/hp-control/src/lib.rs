//! hp-control — the public control protocol.
//!
//! **Status at v0.3: undocumented and explicitly unstable.** The shape below is
//! from `docs/control-api.md`, but nothing here is a commitment until v1.0
//! freezes protocol v1. That window is deliberate — it's the time to change
//! your mind before someone's LED rig is calibrated against it.
//!
//! Two design points inherited from the decision log, both load-bearing:
//!
//! - **D9** — a named pipe, not localhost HTTP. A pipe makes D11's zero-network
//!   guarantee true *by construction* rather than by policy: there is no socket
//!   for anything off-machine to reach, so it cannot be misconfigured open.
//! - **D8/D15** — this is the *only* external surface. Nothing loads into the
//!   player's process; a client runs in its own and speaks NDJSON.
//!
//! The viz channel (binary frames, separate pipe) lands at v0.4 with the
//! analyser, since it's the same data.

use serde::{Deserialize, Serialize};

/// Bumped only for breaking changes. Servers reject unknown majors rather than
/// guessing at what a future client meant.
pub const PROTOCOL_VERSION: u32 = 1;

/// Windows. The POSIX paths in control-api.md land when a port does.
#[cfg(windows)]
pub const PIPE_NAME: &str = r"\\.\pipe\hurricane-party";

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
    // transport
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<f64>,
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
        Self { id, ok: true, result: Some(result), error: None }
    }
    pub fn err(id: u64, msg: impl Into<String>) -> Self {
        Self { id, ok: false, result: None, error: Some(msg.into()) }
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

/// Unsolicited. No `id`, which is how a client tells them from replies.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event")]
pub enum Event {
    #[serde(rename = "now_playing_changed")]
    NowPlaying {
        media_id: Option<i64>,
        title: Option<String>,
        uploader: Option<String>,
        duration_s: Option<f64>,
    },
    #[serde(rename = "state_changed")]
    StateChanged { state: String },
}

/// Commands the player is expected to act on, parsed from the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Hello { client: String, protocol_version: u32 },
    Status,
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Prev,
    Seek(f64),
    Volume(f64),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ParseError {
    #[error("unknown command {0:?}")]
    UnknownCommand(String),
    #[error("{cmd:?} needs {field}")]
    MissingField { cmd: String, field: &'static str },
    #[error("protocol version {0} is not supported (this server speaks {PROTOCOL_VERSION})")]
    UnsupportedVersion(u32),
}

impl Request {
    pub fn parse(&self) -> Result<Command, ParseError> {
        let missing = |f| ParseError::MissingField { cmd: self.cmd.clone(), field: f };
        Ok(match self.cmd.as_str() {
            "hello" => {
                let v = self.protocol_version.ok_or_else(|| missing("protocol_version"))?;
                // Reject rather than guess: a client speaking a future major
                // wants a clear no, not silently-wrong behaviour.
                if v != PROTOCOL_VERSION {
                    return Err(ParseError::UnsupportedVersion(v));
                }
                Command::Hello {
                    client: self.client.clone().unwrap_or_else(|| "unknown".into()),
                    protocol_version: v,
                }
            }
            "status" => Command::Status,
            "play" => Command::Play,
            "pause" => Command::Pause,
            "toggle" => Command::Toggle,
            "stop" => Command::Stop,
            "next" => Command::Next,
            "prev" => Command::Prev,
            "seek" => Command::Seek(self.pos_s.ok_or_else(|| missing("pos_s"))?),
            // Clamped rather than rejected: a client that sends 1.5 means
            // "loud", and refusing is less useful than doing the sane thing.
            "volume" => Command::Volume(self.level.ok_or_else(|| missing("level"))?.clamp(0.0, 1.0)),
            other => return Err(ParseError::UnknownCommand(other.to_string())),
        })
    }
}

/// What `hello` reports. `capabilities` is how a client discovers what this
/// build actually supports without version-sniffing.
pub fn hello_result(app_version: &str) -> serde_json::Value {
    serde_json::json!({
        "protocol_version": PROTOCOL_VERSION,
        "app_version": app_version,
        // No "viz" yet — that lands at v0.4 with the analyser.
        "capabilities": ["transport"],
        "stable": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(json: &str) -> Request {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn parses_the_handshake() {
        let r = req(r#"{"id":0,"cmd":"hello","client":"led-bridge","protocol_version":1}"#);
        assert_eq!(
            r.parse().unwrap(),
            Command::Hello { client: "led-bridge".into(), protocol_version: 1 }
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
        assert_eq!(req(r#"{"cmd":"volume","level":1.7}"#).parse().unwrap(), Command::Volume(1.0));
        assert_eq!(req(r#"{"cmd":"volume","level":-2.0}"#).parse().unwrap(), Command::Volume(0.0));
        assert_eq!(req(r#"{"cmd":"volume","level":0.7}"#).parse().unwrap(), Command::Volume(0.7));
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
        let e = Event::StateChanged { state: "paused".into() };
        let j: serde_json::Value = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(j["event"], "state_changed");
        assert_eq!(j["state"], "paused");
        assert!(j.get("id").is_none());
    }

    #[test]
    fn responses_omit_the_field_they_did_not_use() {
        let ok = serde_json::to_string(&Response::ok(1, serde_json::json!({"a":1}))).unwrap();
        assert!(ok.contains("\"ok\":true") && !ok.contains("error"));
        let err = serde_json::to_string(&Response::err(2, "nope")).unwrap();
        assert!(err.contains("\"ok\":false") && !err.contains("result"));
    }

    #[test]
    fn hello_does_not_yet_advertise_viz() {
        let h = hello_result("0.3.0");
        assert_eq!(h["capabilities"], serde_json::json!(["transport"]));
        assert_eq!(h["stable"], false);
    }
}
