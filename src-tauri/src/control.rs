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

/// Last state the webview reported. Not authoritative — a mirror.
#[derive(Default)]
pub struct ControlState(pub Arc<Mutex<PlayerState>>);

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
        let Ok(line) = serde_json::to_string(ev) else { return };
        // Dropping closed senders here is the only cleanup: a client that went
        // away is discovered on the next send, not tracked separately.
        self.0.lock().unwrap().retain(|tx| tx.send(line.clone()).is_ok());
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
            let s = state.0.lock().unwrap().clone();
            Response::ok(req.id, serde_json::to_value(s).unwrap_or_default())
        }
        other => {
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
            let _ = app.emit("control-command", serde_json::json!({ "cmd": name, "arg": arg }));
            Response::ok(req.id, serde_json::json!({ "accepted": name }))
        }
    }
}

#[cfg(windows)]
pub fn spawn_server(app: AppHandle, broadcaster: Broadcaster) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::ServerOptions;

    tauri::async_runtime::spawn(async move {
        loop {
            // A fresh instance per connection: create it *before* accepting so
            // there is never a window where a client finds no pipe listening.
            let server = match ServerOptions::new().create(hp_control::PIPE_NAME) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("hp-control: can't create pipe: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };
            if let Err(e) = server.connect().await {
                eprintln!("hp-control: accept failed: {e}");
                continue;
            }

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

#[cfg(not(windows))]
pub fn spawn_server(_app: AppHandle, _broadcaster: Broadcaster) {
    // POSIX socket path is specced in control-api.md; Windows-first per O7.
    eprintln!("hp-control: no server on this platform yet");
}

use tauri::Manager;

/// The webview reporting what it's actually doing. Also the point where
/// unsolicited events are derived — by diffing against the previous mirror,
/// so a client isn't spammed with a state_changed on every position tick.
pub fn update_state(app: &AppHandle, incoming: PlayerState) {
    let (state_changed, track_changed) = {
        let st = app.state::<ControlState>();
        let mut cur = st.0.lock().unwrap();
        let sc = cur.state != incoming.state;
        let tc = cur.media_id != incoming.media_id;
        *cur = incoming.clone();
        (sc, tc)
    };

    let bc = app.state::<Broadcaster>();
    if track_changed {
        bc.send(&Event::NowPlaying {
            media_id: incoming.media_id,
            title: incoming.title.clone(),
            uploader: incoming.uploader.clone(),
            duration_s: incoming.duration_s,
        });
    }
    if state_changed {
        bc.send(&Event::StateChanged { state: incoming.state });
    }
}
