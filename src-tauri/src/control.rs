//! The control channel server: named pipe in, NDJSON, transport out.
//!
//! **Protocol 1, frozen with v1.0** (#184, D151); `docs/control-api.md` is the
//! reference. Each connection says `hello` first and is held to the
//! capabilities it asked for there (`gate`), its events included.
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

use hp_control::{
    hello_result, Command, Event, PlayerState, PlaylistSummary, Request, Response, SearchResult,
    Status, TrackSummary, SEARCH_LIMIT,
};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

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
    /// The analyser's ramp as Main last reported it (#183): 24 `#rrggbb`,
    /// empty until Main has mounted.
    pub palette: Vec<String>,
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
pub struct Broadcaster(Arc<Mutex<Vec<Sub>>>);

/// One connection's feed: each event's line, and the capability it belongs
/// to, so the connection can keep only what it asked for at `hello` (#184).
type Sub = tokio::sync::mpsc::UnboundedSender<(&'static str, String)>;

impl Broadcaster {
    pub fn subscribe(&self) -> tokio::sync::mpsc::UnboundedReceiver<(&'static str, String)> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.0.lock().unwrap().push(tx);
        rx
    }
    /// Whether anyone has connected and not yet been found gone. A client that
    /// left is only noticed on the next send, so this can say yes for a
    /// moment too long, never no too early.
    pub fn has_clients(&self) -> bool {
        !self.0.lock().unwrap().is_empty()
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
            .retain(|tx| tx.send((ev.capability(), line.clone())).is_ok());
    }
}

/// Handle one parsed command.
///
/// Transport commands are *relayed*, not executed: this process has no audio.
/// The reply says the command was accepted, not that it has taken effect —
/// which is honest, and is why `status` reads the mirrored state rather than
/// pretending to know synchronously.
/// #184: hello first, then only what the connection asked for at hello.
/// Enforced at the freeze because loosening a rule later is additive and
/// tightening one is not. `granted` is `None` until hello.
fn gate(granted: Option<&[&'static str]>, cmd: &Command, name: &str) -> Result<(), String> {
    if matches!(cmd, Command::Hello { .. }) {
        return Ok(());
    }
    let Some(g) = granted else {
        return Err(r#"say hello first: {"cmd":"hello", "protocol_version":1}"#.into());
    };
    match cmd.capability() {
        Some(cap) if !g.contains(&cap) => Err(format!(
            "{name:?} needs {cap:?}, and this connection asked for {g:?} at hello"
        )),
        _ => Ok(()),
    }
}

fn handle(
    app: &AppHandle,
    state: &ControlState,
    granted: &mut Option<Vec<&'static str>>,
    req: &Request,
) -> Response {
    let cmd = match req.parse() {
        Ok(c) => c,
        Err(e) => return Response::err(req.id, e.to_string()),
    };

    if let Err(why) = gate(granted.as_deref(), &cmd, &req.cmd) {
        return Response::err(req.id, why);
    }

    match cmd {
        Command::Hello { client, want, .. } => {
            let g = hp_control::granted(want.as_deref());
            eprintln!("hp-control: {client} connected, for {g:?}");
            let result = hello_result(env!("CARGO_PKG_VERSION"), &g);
            *granted = Some(g);
            Response::ok(req.id, result)
        }
        Command::Status => {
            // The transport's state is the mirror's; shuffle and repeat are
            // the library's, saved in settings (D97). One lock, then the other,
            // never both held.
            let player = state.0.lock().unwrap().current();
            let (shuffle, repeat) = {
                let db = app.state::<crate::db::Db>();
                let conn = db.0.lock().unwrap();
                crate::db::play_mode(&conn)
            };
            let s = Status {
                player,
                shuffle,
                repeat,
            };
            Response::ok(req.id, serde_json::to_value(s).unwrap_or_default())
        }
        // The ramp Main last reported (#183); empty until Main has mounted.
        Command::Palette => {
            let viscolor = state.0.lock().unwrap().palette.clone();
            Response::ok(req.id, serde_json::json!({ "viscolor": viscolor }))
        }
        // The library on the pipe (#182). Asking is answered here from the
        // database; playing goes to the library window, which holds the
        // queue (D120), after Rust has made sure there is something to play,
        // so a bad id or an unplugged drive is an error the client can read
        // rather than a silence.
        Command::Playlists => {
            let lists = {
                let db = app.state::<crate::db::Db>();
                let conn = db.0.lock().unwrap();
                crate::playlist::list(&conn)
            };
            match lists {
                Ok(lists) => {
                    let lists: Vec<PlaylistSummary> = lists
                        .into_iter()
                        .map(|p| PlaylistSummary {
                            id: p.id,
                            name: p.name,
                            count: p.count - p.offline,
                            smart: p.smart,
                        })
                        .collect();
                    Response::ok(req.id, serde_json::json!({ "playlists": lists }))
                }
                Err(e) => Response::err(req.id, e.to_string()),
            }
        }
        Command::Search(q) => {
            let found = {
                let db = app.state::<crate::db::Db>();
                let conn = db.0.lock().unwrap();
                crate::playlist::search(&conn, &q, SEARCH_LIMIT)
            };
            match found {
                Ok((total, rows)) => {
                    let result = SearchResult {
                        total,
                        tracks: rows
                            .into_iter()
                            .map(|t| TrackSummary {
                                id: t.id,
                                title: t.title,
                                uploader: t.uploader,
                                kind: t.kind,
                                duration_s: t.duration_s,
                            })
                            .collect(),
                    };
                    Response::ok(req.id, serde_json::to_value(result).unwrap_or_default())
                }
                Err(e) => Response::err(req.id, e.to_string()),
            }
        }
        Command::QueuePlaylist(id) => {
            let found = {
                let db = app.state::<crate::db::Db>();
                let conn = db.0.lock().unwrap();
                crate::playlist::playable(&conn, id)
            };
            match found {
                Ok(None) => Response::err(req.id, format!("no playlist {id}")),
                Ok(Some((name, 0))) => {
                    Response::err(req.id, format!("“{name}” has nothing that can play now"))
                }
                Ok(Some((name, count))) => {
                    match app.emit_to("library", "pipe:queue-playlist", id) {
                        Ok(()) => Response::ok(
                            req.id,
                            serde_json::json!({ "playlist_id": id, "name": name, "count": count }),
                        ),
                        Err(e) => Response::err(req.id, e.to_string()),
                    }
                }
                Err(e) => Response::err(req.id, e.to_string()),
            }
        }
        Command::PlayMedia(id) => {
            let found = {
                let db = app.state::<crate::db::Db>();
                let conn = db.0.lock().unwrap();
                let title: Option<String> = conn
                    .query_row("SELECT title FROM media WHERE id = ?1", [id], |r| r.get(0))
                    .ok();
                title.map(|t| (t, crate::drives::out_for(&conn, id).ok().flatten()))
            };
            match found {
                None => Response::err(req.id, format!("track {id} is not in the library")),
                Some((title, Some(drive))) => Response::err(
                    req.id,
                    format!("“{title}” is on {drive}, which isn't plugged in"),
                ),
                Some((title, None)) => match app.emit_to("library", "pipe:play-media", id) {
                    Ok(()) => Response::ok(
                        req.id,
                        serde_json::json!({ "media_id": id, "title": title }),
                    ),
                    Err(e) => Response::err(req.id, e.to_string()),
                },
            }
        }
        // Answered here, off the UI thread, which is where the window getters
        // behind it have to be asked from (#181).
        Command::Layout => Response::ok(
            req.id,
            serde_json::to_value(crate::layout::current(app)).unwrap_or_default(),
        ),
        // The one command answered here rather than relayed: the pipe is
        // created before the reply, so the client never finds it missing.
        Command::SubscribeViz(params) => match crate::viz::subscribe(app, params) {
            Ok(stream) => Response::ok(req.id, serde_json::json!({ "stream": stream })),
            Err(e) => Response::err(req.id, e),
        },
        other => {
            let (name, arg) = match other {
                Command::Play => ("play", None),
                Command::Pause => ("pause", None),
                Command::Toggle => ("toggle", None),
                Command::Stop => ("stop", None),
                Command::Next => ("next", None),
                Command::Prev => ("prev", None),
                Command::Seek(p) => ("seek", Some(p)),
                Command::Volume(v) => ("volume", Some(v)),
                _ => unreachable!("handled above"),
            };
            match route(app, name, arg) {
                Ok(()) => Response::ok(req.id, serde_json::json!({ "accepted": name })),
                Err(e) => Response::err(req.id, e),
            }
        }
    }
}

/// Send a transport command to whichever window is the transport.
///
/// D70: `play` `pause` `toggle` `stop` `seek` `volume` go to the video window
/// while it is the transport, otherwise to Main. `next` and `prev` always go
/// to Main, which asks the library to step: the library's cursor walks over
/// videos and tracks alike, so they work from either. Targeted, not
/// broadcast, or both windows would obey and D69's one transport becomes two.
///
/// One router for two callers: the pipe, and Main's own buttons (D81), so a
/// press in Main and a `pause` over the pipe do exactly the same thing.
pub fn route(app: &AppHandle, cmd: &str, arg: Option<f64>) -> Result<(), String> {
    let needs_arg = matches!(cmd, "seek" | "volume");
    match cmd {
        "play" | "pause" | "toggle" | "stop" | "next" | "prev" | "seek" | "volume" => {}
        other => return Err(format!("unknown transport command {other:?}")),
    }
    let arg = match (needs_arg, arg) {
        (true, None) => return Err(format!("{cmd:?} needs a value")),
        (true, Some(v)) if cmd == "volume" => Some(v.clamp(0.0, 1.0)),
        (_, a) => a,
    };
    let steps = matches!(cmd, "next" | "prev");
    // Read what the lock knows, then drop it: the raise below is a Win32
    // call on another thread's window, never made under the state lock (D54).
    let (to_video, video_paused) = {
        let st = app.state::<ControlState>();
        let m = st.0.lock().unwrap();
        let to_video = !steps && m.active_video && app.get_webview_window("video").is_some();
        (to_video, m.video.state != "playing")
    };
    app.emit_to(
        if to_video { "video" } else { "main" },
        "control-command",
        serde_json::json!({ "cmd": cmd, "arg": arg }),
    )
    .map_err(|e| e.to_string())?;
    // A resume brings the video forward, as a switch does (`open_video`): the
    // user pressed play to watch it, and it may have gone behind something
    // while it sat paused. Only when the command would start it, so a toggle
    // that pauses does not pull a window forward on its way to stopping.
    // Restored first, as in `open_video`: `set_focus` does not unminimize (#39).
    if to_video && (cmd == "play" || (cmd == "toggle" && video_paused)) {
        if let Some(w) = app.get_webview_window("video") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
        crate::layout::ping(app);
    }
    Ok(())
}

/// The one state the channel reports, for Main's display (D81).
pub fn current(app: &AppHandle) -> PlayerState {
    app.state::<ControlState>().0.lock().unwrap().current()
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
            // Nothing is granted, and no event is sent, until hello (#184).
            let mut granted: Option<Vec<&'static str>> = None;
            tauri::async_runtime::spawn(async move {
                let (reader, mut writer) = tokio::io::split(server);
                let mut lines = BufReader::new(reader).lines();
                loop {
                    tokio::select! {
                        // Unsolicited events, pushed as they happen.
                        Some((cap, line)) = events.recv() => {
                            if !granted.as_ref().is_some_and(|g| g.contains(&cap)) {
                                continue;
                            }
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
                                    handle(&app, &st, &mut granted, &req)
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

/// Main's ramp, checked against what the pipe promises (`hp_control::ramp`)
/// and told to clients only when it changed (#183). The lock is let go
/// before anything is sent.
pub fn update_palette(app: &AppHandle, viscolor: &[String]) -> Result<(), String> {
    let ramp = hp_control::ramp(viscolor)?;
    {
        let st = app.state::<ControlState>();
        let mut m = st.0.lock().unwrap();
        if m.palette == ramp {
            return Ok(());
        }
        m.palette = ramp.clone();
    }
    app.state::<Broadcaster>()
        .send(&Event::PaletteChanged { viscolor: ramp });
    Ok(())
}

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
        // Hand the channel back to the track (D81): what `status` describes,
        // and what Main displays and its buttons act on, is now the paused
        // or stopped track, which is what a press of play would resume.
        m.active_video = false;
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

    // Main displays and drives whatever is playing (D81), so it hears the
    // same state the pipe would: every change, position ticks included.
    let _ = app.emit_to("main", "player:current", &now);

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

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(json: &str) -> Command {
        serde_json::from_str::<Request>(json)
            .unwrap()
            .parse()
            .unwrap()
    }

    /// #184: nothing before hello; after it, only what was asked for, with
    /// the refusal naming the capability; hello again is always allowed.
    #[test]
    fn a_connection_says_hello_first_and_gets_what_it_asked_for() {
        let status = cmd(r#"{"cmd":"status"}"#);
        let why = gate(None, &status, "status").unwrap_err();
        assert!(why.contains("hello first"), "{why}");

        let all = hp_control::granted(None);
        assert!(gate(Some(&all), &status, "status").is_ok());

        let bars = hp_control::granted(Some(&["viz".to_string()]));
        assert!(gate(
            Some(&bars),
            &cmd(r#"{"cmd":"subscribe_viz"}"#),
            "subscribe_viz"
        )
        .is_ok());
        let why = gate(Some(&bars), &cmd(r#"{"cmd":"next"}"#), "next").unwrap_err();
        assert!(
            why.contains("\"transport\"") && why.contains("[\"viz\"]"),
            "{why}"
        );
        let why = gate(Some(&bars), &cmd(r#"{"cmd":"search","q":"x"}"#), "search").unwrap_err();
        assert!(why.contains("\"library\""), "{why}");

        let hello = cmd(r#"{"cmd":"hello","protocol_version":1}"#);
        assert!(gate(None, &hello, "hello").is_ok());
        assert!(gate(Some(&bars), &hello, "hello").is_ok());
    }
}
