//! His line to the player: the public control pipe, used the way any outside
//! program uses it (D22, protocol 1, D151). He says hello asking only for what
//! he uses (`layout`, `transport`, `viz`), asks where the windows are, whether
//! anything is playing and for the smallest viz stream there is, then listens.
//! When the player is not running, or goes, he hears `Gone`, waits, and tries
//! again, so he can be started before the player or outlive a restart of it.
//!
//! Every request is written before the first event is read. The pipe handle is
//! synchronous, so a write while a read is blocked would wait for the read;
//! asking everything up front keeps the one thread simple. The viz stream is a
//! pipe of its own (`subscribe_viz`), read on a second thread that sends only
//! the moment each beat starts.

use hp_control::viz::DecodeError;
use hp_control::{Frame, LayoutInfo, PIPE_NAME, PROTOCOL_VERSION};
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::mpsc::Sender;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    /// The player's process id, from its end of the pipe: its windows are
    /// the ones he stands on (D166).
    Player(u32),
    Layout(LayoutInfo),
    Playing(bool),
    /// A beat just started (the flag's rising edge; the player holds it 200 ms).
    Beat,
    /// The player is not there (any more).
    Gone,
    /// Started by the player's switch, and the player has gone: time to go.
    Bye,
}

/// How long he waits before knocking again.
pub const RETRY: Duration = Duration::from_secs(2);
/// Started by the player but never let in: the player did not come up, and
/// there is no one to leave with. He does not wait for ever.
pub const NO_SHOW: Duration = Duration::from_secs(30);

const STATUS_ID: u64 = 2;
const LAYOUT_ID: u64 = 3;
const VIZ_ID: u64 = 4;

/// `with_player`: he came from the player's switch (D157), so the pipe
/// closing means the player has gone and so should he. Otherwise he waits for
/// it, and outlasts a restart.
pub fn spawn(tx: Sender<Msg>, with_player: bool) {
    std::thread::Builder::new()
        .name("link".into())
        .spawn(move || {
            let born = std::time::Instant::now();
            let mut connected = false;
            let mut ever = false;
            loop {
                let outcome = session(&tx, &mut connected);
                ever |= connected;
                if connected {
                    match outcome {
                        Ok(()) => eprintln!("hp-companion: the player went"),
                        Err(e) => eprintln!("hp-companion: lost the player ({e})"),
                    }
                    connected = false;
                }
                let bye = with_player && (ever || born.elapsed() >= NO_SHOW);
                if tx.send(if bye { Msg::Bye } else { Msg::Gone }).is_err() || bye {
                    return;
                }
                std::thread::sleep(RETRY);
            }
        })
        .expect("start the link thread");
}

/// One connection, from hello to the pipe closing. `Ok` when the player
/// closed it; `Err` for a failure, including not being able to connect.
fn session(tx: &Sender<Msg>, connected: &mut bool) -> Result<(), String> {
    let pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(PIPE_NAME)
        .map_err(|e| e.to_string())?;
    let player = crate::platform::pipe_server(&pipe);
    let mut out = pipe.try_clone().map_err(|e| e.to_string())?;
    let mut lines = BufReader::new(pipe);

    out.write_all(hello().as_bytes())
        .map_err(|e| e.to_string())?;
    let reply = next(&mut lines)?.ok_or("the player closed the pipe at hello")?;
    check_hello(&reply)?;
    *connected = true;
    eprintln!("hp-companion: connected to the player");
    if let Some(pid) = player {
        if tx.send(Msg::Player(pid)).is_err() {
            return Ok(());
        }
    }

    out.write_all(asks().as_bytes())
        .map_err(|e| e.to_string())?;
    while let Some(line) = next(&mut lines)? {
        let msg = match parse(&line) {
            Line::Layout(l) => Msg::Layout(l),
            Line::Playing(p) => Msg::Playing(p),
            Line::Stream(name) => {
                watch_beats(name, tx.clone());
                continue;
            }
            Line::Other => continue,
        };
        if tx.send(msg).is_err() {
            return Ok(());
        }
    }
    Ok(())
}

fn next(lines: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut s = String::new();
    match lines.read_line(&mut s) {
        Ok(0) => Ok(None),
        Ok(_) => Ok(Some(s)),
        Err(e) => Err(e.to_string()),
    }
}

/// Hello, asking for the three parts he uses and nothing else.
pub fn hello() -> String {
    let h = serde_json::json!({
        "id": 1,
        "cmd": "hello",
        "client": "hp-companion",
        "protocol_version": PROTOCOL_VERSION,
        "want": ["layout", "transport", "viz"],
    });
    format!("{h}\n")
}

/// Where the transport stands, where the windows are, and the smallest viz
/// stream the protocol offers: 8 bands (the minimum) left out, at 15 Hz, beat
/// only. The beat is held 200 ms, so 15 Hz still sees every one.
pub fn asks() -> String {
    let status = serde_json::json!({ "id": STATUS_ID, "cmd": "status" });
    let layout = serde_json::json!({ "id": LAYOUT_ID, "cmd": "layout" });
    let viz = serde_json::json!({
        "id": VIZ_ID, "cmd": "subscribe_viz", "bands": 8, "rate_hz": 15, "include": ["beat"],
    });
    format!("{status}\n{layout}\n{viz}\n")
}

fn check_hello(reply: &str) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(reply).map_err(|e| e.to_string())?;
    if v["ok"] == true {
        Ok(())
    } else {
        Err(format!("hello refused: {}", v["error"]))
    }
}

#[derive(Debug, PartialEq)]
pub enum Line {
    Layout(LayoutInfo),
    Playing(bool),
    /// The viz stream's pipe name, from `subscribe_viz`.
    Stream(String),
    Other,
}

/// What a line from the control pipe says, as far as he cares.
pub fn parse(line: &str) -> Line {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return Line::Other;
    };
    match v["event"].as_str() {
        Some("layout_changed") => {
            return serde_json::from_value(v)
                .map(Line::Layout)
                .unwrap_or(Line::Other)
        }
        Some("state_changed") => return Line::Playing(v["state"] == "playing"),
        Some(_) => return Line::Other,
        None => {}
    }
    if v["ok"] != true {
        if v["id"] == VIZ_ID {
            eprintln!("hp-companion: no beat to dance to ({})", v["error"]);
        }
        return Line::Other;
    }
    let result = &v["result"];
    match v["id"].as_u64() {
        Some(STATUS_ID) => Line::Playing(result["state"] == "playing"),
        Some(LAYOUT_ID) => serde_json::from_value(result.clone())
            .map(Line::Layout)
            .unwrap_or(Line::Other),
        Some(VIZ_ID) => result["stream"]
            .as_str()
            .map(|s| Line::Stream(s.into()))
            .unwrap_or(Line::Other),
        _ => Line::Other,
    }
}

/// Read the viz stream on a thread of its own and send `Beat` on each rising
/// edge. It ends with the stream: the subscription belongs to that pipe, and a
/// new session subscribes again.
fn watch_beats(name: String, tx: Sender<Msg>) {
    let _ = std::thread::Builder::new()
        .name("beats".into())
        .spawn(move || {
            let Ok(mut pipe) = std::fs::File::open(&name) else {
                eprintln!("hp-companion: could not open the viz stream; no dancing this time");
                return;
            };
            let mut beats = Beats::default();
            let mut buf = [0u8; 4096];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        for _ in 0..beats.feed(&buf[..n]) {
                            if tx.send(Msg::Beat).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        });
}

/// Frames in, beat starts out. Keeps a partial frame between reads, and
/// resyncs on the magic if it ever loses its place.
#[derive(Default)]
pub struct Beats {
    pending: Vec<u8>,
    was: bool,
}

impl Beats {
    /// Take bytes from the stream; return how many beats started in them.
    pub fn feed(&mut self, bytes: &[u8]) -> usize {
        self.pending.extend_from_slice(bytes);
        let mut started = 0;
        loop {
            match Frame::decode(&self.pending) {
                Ok((f, used)) => {
                    self.pending.drain(..used);
                    if f.beat && !self.was {
                        started += 1;
                    }
                    self.was = f.beat;
                }
                Err(DecodeError::Incomplete(_)) => return started,
                Err(_) => {
                    self.pending.remove(0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hp_control::Depth;

    #[test]
    fn hello_asks_for_what_he_uses_and_nothing_else() {
        let v: serde_json::Value = serde_json::from_str(&hello()).unwrap();
        assert_eq!(v["cmd"], "hello");
        assert_eq!(v["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(v["want"], serde_json::json!(["layout", "transport", "viz"]));
        assert!(hello().ends_with('\n'), "NDJSON: one line");
    }

    #[test]
    fn he_asks_for_the_smallest_viz_stream() {
        let viz = asks()
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .nth(2);
        let viz = viz.unwrap();
        assert_eq!(viz["cmd"], "subscribe_viz");
        assert_eq!(viz["include"], serde_json::json!(["beat"]));
        assert_eq!(
            (viz["bands"].as_u64(), viz["rate_hz"].as_u64()),
            (Some(8), Some(15))
        );
    }

    #[test]
    fn an_event_and_the_answer_both_carry_the_layout() {
        let w = r#"{"id":"main","x":1,"y":2,"w":275,"h":116,"group":true,"shaded":false,"visible":true}"#;
        let event = format!(r#"{{"event":"layout_changed","windows":[{w}],"bonds":[]}}"#);
        let answer = format!(r#"{{"id":3,"ok":true,"result":{{"windows":[{w}],"bonds":[]}}}}"#);
        for line in [event, answer] {
            let Line::Layout(l) = parse(&line) else {
                panic!("{line}")
            };
            assert_eq!((l.windows[0].id.as_str(), l.windows[0].w), ("main", 275));
        }
    }

    #[test]
    fn playing_comes_from_status_and_from_state_changed() {
        assert_eq!(
            parse(r#"{"id":2,"ok":true,"result":{"state":"playing"}}"#),
            Line::Playing(true)
        );
        assert_eq!(
            parse(r#"{"event":"state_changed","state":"paused"}"#),
            Line::Playing(false)
        );
        assert_eq!(
            parse(r#"{"event":"state_changed","state":"playing"}"#),
            Line::Playing(true)
        );
    }

    #[test]
    fn the_viz_answer_names_the_stream() {
        let line =
            r#"{"id":4,"ok":true,"result":{"stream":"\\\\.\\pipe\\hurricane-party-viz-7f3a"}}"#;
        assert_eq!(
            parse(line),
            Line::Stream(r"\\.\pipe\hurricane-party-viz-7f3a".into())
        );
    }

    #[test]
    fn other_lines_are_nothing_to_him() {
        for line in [
            r#"{"event":"now_playing_changed","title":"x"}"#,
            r#"{"id":3,"ok":false,"error":"no"}"#,
            r#"{"id":7,"ok":true,"result":{}}"#,
            "not json",
        ] {
            assert_eq!(parse(line), Line::Other, "{line}");
        }
    }

    #[test]
    fn a_refused_hello_is_an_error_with_the_players_reason() {
        let e = check_hello(r#"{"id":1,"ok":false,"error":"protocol 9 is not spoken"}"#);
        assert!(e.unwrap_err().contains("protocol 9"));
        assert!(check_hello(r#"{"id":1,"ok":true,"result":{}}"#).is_ok());
    }

    fn frame(beat: bool) -> Vec<u8> {
        Frame {
            timestamp_us: 1,
            depth: Depth::U8,
            beat,
            level_peak: 0,
            level_rms: 0,
            spectrum: vec![],
        }
        .encode()
    }

    #[test]
    fn a_held_beat_counts_once_and_a_new_one_counts_again() {
        let mut b = Beats::default();
        let stream: Vec<u8> = [false, true, true, true, false, true]
            .iter()
            .flat_map(|&x| frame(x))
            .collect();
        assert_eq!(b.feed(&stream), 2);
    }

    #[test]
    fn frames_split_across_reads_and_junk_are_survived() {
        let mut b = Beats::default();
        let mut stream = vec![0xAA, 0xBB];
        stream.extend(frame(false));
        stream.extend(frame(true));
        let (a, c) = stream.split_at(stream.len() - 5);
        assert_eq!(b.feed(a), 0, "the beat frame is not whole yet");
        assert_eq!(b.feed(c), 1);
    }
}
