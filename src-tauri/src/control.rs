//! The control channel server: named pipe in, NDJSON, transport out.
//!
//! **Undocumented and unstable until v1.0.** That's the point of shipping it at
//! v0.3 — it proves the pipe while nothing external depends on the shape.
//!
//! The awkward part, flagged honestly in `control-api.md`: the audio graph
//! lives in the webview (D5), so Rust is not the source of truth for playback.
//! A command arrives on the pipe, gets relayed to the frontend as a Tauri
//! event, the frontend acts, and reports state back. Rust caches the last
//! reported state so `status` can answer without a round trip.
//!
//! That hop is the cost of getting `AnalyserNode` and `BiquadFilterNode` for
//! free instead of hand-rolling FFT in Rust. Worth it — but it's why the viz
//! channel (v0.4) needs its latency measured before v1.0 freezes anything.

use hp_control::{hello_result, Command, Event, PlayerState, Request, Response};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

/// What each transport last reported. Not authoritative — a mirror.
///
/// Two of them, because two windows report (D70): the Main window for the
/// `<audio>` element and the video window for its `<video>`. One thing plays
/// at a time (D69), so `status` answers from whichever kind last said
/// "playing"; a "paused" from the other side does not take the channel back.
#[derive(Default)]
pub struct ControlState(pub Arc<Mutex<Mirror>>);

#[derive(Default)]
pub struct Mirror {
    pub audio: PlayerState,
    pub video: PlayerState,
    /// The video window is the transport.
    pub active_video: bool,
    /// What clients last heard, for deriving events by diff.
    told: PlayerState,
}

impl Mirror {
    /// The one state the channel reports.
    pub fn current(&self) -> PlayerState {
        let mut s = if self.active_video {
            self.video.clone()
        } else {
            self.audio.clone()
        };
        s.kind = if self.active_video { "video" } else { "audio" }.into();
        s
    }
}

/// Broadcast an event to every connected client.
#[derive(Default, Clone)]
pub struct Broadcaster(Arc<Mutex<Vec<tokio::sync::mpsc::UnboundedSender<String>>>>);

impl Broadcaster {
    pub fn subscribe(&self) -> tokio::sync::mpsc::UnboundedReceiver<String> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.0.lock().unwrap().push(tx);
        rx
    }
    pub fn send(&self, ev: &Event) {
        let Ok(line) = serde_json::to_string(ev) else {
            return;
        };
        // Dropping closed senders here is the only cleanup: a client that went
        // away is discovered on the next send, not tracked separately.
        self.0
            .lock()
            .unwrap()
            .retain(|tx| tx.send(line.clone()).is_ok());
    }
}

/// Handle one parsed command.
///
/// Transport commands are *relayed*, not executed: this process has no audio.
/// The reply says the command was accepted, not that it has taken effect —
/// which is honest, and is why `status` reads the mirrored state rather than
/// pretending to know synchronously.
fn handle(app: &AppHandle, state: &ControlState, req: &Request) -> Response {
    let cmd = match req.parse() {
        Ok(c) => c,
        Err(e) => return Response::err(req.id, e.to_string()),
    };

    match cmd {
        Command::Hello { client, .. } => {
            eprintln!("hp-control: {client} connected");
            Response::ok(req.id, hello_result(env!("CARGO_PKG_VERSION")))
        }
        Command::Status => {
            let s = state.0.lock().unwrap().current();
            Response::ok(req.id, serde_json::to_value(s).unwrap_or_default())
        }
        // The one command answered here rather than relayed: the pipe is
        // created before the reply, so the client never finds it missing.
        Command::SubscribeViz(params) => match crate::viz::subscribe(app, params) {
            Ok(stream) => Response::ok(req.id, serde_json::json!({ "stream": stream })),
            Err(e) => Response::err(req.id, e),
        },
        other => {
            // D70: a transport command goes to whichever window is the
            // transport. Next and prev always go to Main, which asks the
            // library to step: the library's cursor walks over videos and
            // tracks alike, so they work from either. Targeted, not broadcast,
            // or both windows would obey and D69's one transport becomes two.
            let steps = matches!(other, Command::Next | Command::Prev);
            let to_video = !steps
                && state.0.lock().unwrap().active_video
                && app.get_webview_window("video").is_some();
            let (name, arg) = match other {
                Command::Play => ("play", serde_json::Value::Null),
                Command::Pause => ("pause", serde_json::Value::Null),
                Command::Toggle => ("toggle", serde_json::Value::Null),
                Command::Stop => ("stop", serde_json::Value::Null),
                Command::Next => ("next", serde_json::Value::Null),
                Command::Prev => ("prev", serde_json::Value::Null),
                Command::Seek(p) => ("seek", serde_json::json!(p)),
                Command::Volume(v) => ("volume", serde_json::json!(v)),
                _ => unreachable!("handled above"),
            };
            let _ = app.emit_to(
                if to_video { "video" } else { "main" },
                "control-command",
                serde_json::json!({ "cmd": name, "arg": arg }),
            );
            Response::ok(req.id, serde_json::json!({ "accepted": name }))
        }
    }
}

/// Accept clients on the control pipe, one task each, forever.
///
/// The transport is `platform::pipe` (D9, #20): this file knows it has a byte
/// stream and that the framing is lines, nothing about the OS.
pub fn spawn_server(app: AppHandle, broadcaster: Broadcaster) {
    use crate::platform::pipe;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    tauri::async_runtime::spawn(async move {
        loop {
            // A fresh instance per connection: create it *before* accepting so
            // there is never a window where a client finds no pipe listening.
            let listener = match pipe::listen(hp_control::PIPE_NAME, pipe::ListenOptions::default())
            {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("hp-control: can't create pipe: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };
            let server = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("hp-control: accept failed: {e}");
                    continue;
                }
            };

            let app = app.clone();
            let mut events = broadcaster.subscribe();
            tauri::async_runtime::spawn(async move {
                let (reader, mut writer) = tokio::io::split(server);
                let mut lines = BufReader::new(reader).lines();
                loop {
                    tokio::select! {
                        // Unsolicited events, pushed as they happen.
                        Some(line) = events.recv() => {
                            if writer.write_all(format!("{line}\n").as_bytes()).await.is_err() {
                                break;
                            }
                        }
                        line = lines.next_line() => {
                            let Ok(Some(line)) = line else { break };
                            if line.trim().is_empty() {
                                continue;
                            }
                            let resp = match serde_json::from_str::<Request>(&line) {
                                Ok(req) => {
                                    let st = app.state::<ControlState>();
                                    handle(&app, &st, &req)
                                }
                                Err(e) => Response::err(0, format!("malformed request: {e}")),
                            };
                            let Ok(mut out) = serde_json::to_string(&resp) else { break };
                            out.push('\n');
                            if writer.write_all(out.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                        else => break,
                    }
                }
            });
        }
    });
}

use tauri::Manager;

/// The webview reporting what it's actually doing. Also the point where
/// unsolicited events are derived — by diffing against the previous mirror,
/// so a client isn't spammed with a state_changed on every position tick.
pub fn update_state(app: &AppHandle, incoming: PlayerState) {
    let video = incoming.kind == "video";
    apply(app, |m| {
        if video {
            m.video = incoming;
        } else {
            m.audio = incoming;
        }
        // D69: whichever last started playing is the transport. A pause or a
        // stop from the other side reports itself but does not take it back.
        let reported = if video { &m.video } else { &m.audio };
        if reported.state == "playing" {
            m.active_video = video;
        }
    });
}

/// The video window closed. Its `<video>` is gone with it, so its mirror is
/// stopped; if it was the transport, `status` says stopped rather than going
/// on describing a window that no longer exists (D70).
pub fn video_gone(app: &AppHandle) {
    apply(app, |m| {
        let volume = m.video.volume;
        m.video = PlayerState {
            state: "stopped".into(),
            kind: "video".into(),
            volume,
            ..Default::default()
        };
    });
}

/// Change the mirror, then tell clients what changed about the one state
/// they see. Events come from the diff against what they were last told,
/// so a position tick on the transport that is not active is silent.
fn apply(app: &AppHandle, change: impl FnOnce(&mut Mirror)) {
    let (state_changed, track_changed, now) = {
        let st = app.state::<ControlState>();
        let mut m = st.0.lock().unwrap();
        change(&mut m);
        let now = m.current();
        let sc = m.told.state != now.state;
        let tc = m.told.media_id != now.media_id || m.told.kind != now.kind;
        m.told = now.clone();
        (sc, tc, now)
    };

    let bc = app.state::<Broadcaster>();
    if track_changed {
        bc.send(&Event::NowPlaying {
            kind: now.kind.clone(),
            media_id: now.media_id,
            title: now.title.clone(),
            uploader: now.uploader.clone(),
            duration_s: now.duration_s,
        });
    }
    if state_changed {
        bc.send(&Event::StateChanged { state: now.state });
    }
}
