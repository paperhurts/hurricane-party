//! His line to the player: the public control pipe, used the way any outside
//! program uses it (D22, protocol 1, D151). He says hello asking only for what
//! he uses, asks where the windows are, and then listens for `layout_changed`.
//! When the player is not running, or goes, he hears `Gone`, waits, and tries
//! again, so he can be started before the player or outlive a restart of it.
//!
//! Every request is written before the first event is read. The pipe handle is
//! synchronous, so a write while a read is blocked would wait for the read;
//! asking everything up front keeps the one thread simple.

use hp_control::{LayoutInfo, PIPE_NAME, PROTOCOL_VERSION};
use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::Sender;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Layout(LayoutInfo),
    /// The player is not there (any more).
    Gone,
}

/// How long he waits before knocking again.
pub const RETRY: Duration = Duration::from_secs(2);

const LAYOUT_ID: u64 = 2;

pub fn spawn(tx: Sender<Msg>) {
    std::thread::Builder::new()
        .name("link".into())
        .spawn(move || {
            let mut connected = false;
            loop {
                let outcome = session(&tx, &mut connected);
                if connected {
                    match outcome {
                        Ok(()) => eprintln!("hp-companion: the player went; waiting for it"),
                        Err(e) => eprintln!("hp-companion: lost the player ({e}); waiting for it"),
                    }
                    connected = false;
                }
                if tx.send(Msg::Gone).is_err() {
                    return; // the window side has gone
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
    let mut out = pipe.try_clone().map_err(|e| e.to_string())?;
    let mut lines = BufReader::new(pipe);

    out.write_all(hello().as_bytes())
        .map_err(|e| e.to_string())?;
    let reply = next(&mut lines)?.ok_or("the player closed the pipe at hello")?;
    check_hello(&reply)?;
    *connected = true;
    eprintln!("hp-companion: connected to the player");

    let ask = serde_json::json!({ "id": LAYOUT_ID, "cmd": "layout" });
    out.write_all(format!("{ask}\n").as_bytes())
        .map_err(|e| e.to_string())?;
    while let Some(line) = next(&mut lines)? {
        if let Some(layout) = parse(&line) {
            if tx.send(Msg::Layout(layout)).is_err() {
                return Ok(());
            }
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

/// Hello, asking only for `layout`: he never sees what he did not ask for.
pub fn hello() -> String {
    let h = serde_json::json!({
        "id": 1,
        "cmd": "hello",
        "client": "hp-companion",
        "protocol_version": PROTOCOL_VERSION,
        "want": ["layout"],
    });
    format!("{h}\n")
}

fn check_hello(reply: &str) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(reply).map_err(|e| e.to_string())?;
    if v["ok"] == true {
        Ok(())
    } else {
        Err(format!("hello refused: {}", v["error"]))
    }
}

/// A line that says where the windows are: the answer to `layout`, or a
/// `layout_changed` event. Anything else is `None`.
pub fn parse(line: &str) -> Option<LayoutInfo> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v["event"] == "layout_changed" {
        return serde_json::from_value(v).ok();
    }
    if v["id"] == LAYOUT_ID && v["ok"] == true {
        return serde_json::from_value(v["result"].clone()).ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_asks_for_layout_and_nothing_else() {
        let v: serde_json::Value = serde_json::from_str(&hello()).unwrap();
        assert_eq!(v["cmd"], "hello");
        assert_eq!(v["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(v["want"], serde_json::json!(["layout"]));
        assert!(hello().ends_with('\n'), "NDJSON: one line");
    }

    #[test]
    fn an_event_and_the_answer_both_carry_the_layout() {
        let w = r#"{"id":"main","x":1,"y":2,"w":275,"h":116,"group":true,"shaded":false,"visible":true}"#;
        let event = format!(r#"{{"event":"layout_changed","windows":[{w}],"bonds":[]}}"#);
        let answer = format!(r#"{{"id":2,"ok":true,"result":{{"windows":[{w}],"bonds":[]}}}}"#);
        for line in [event, answer] {
            let l = parse(&line).unwrap();
            assert_eq!(l.windows[0].id, "main");
            assert_eq!((l.windows[0].x, l.windows[0].w), (1, 275));
        }
    }

    #[test]
    fn other_lines_are_not_a_layout() {
        for line in [
            r#"{"event":"state_changed","state":"paused"}"#,
            r#"{"id":2,"ok":false,"error":"no"}"#,
            r#"{"id":7,"ok":true,"result":{"windows":[],"bonds":[]}}"#,
            "not json",
        ] {
            assert!(parse(line).is_none(), "{line}");
        }
    }

    #[test]
    fn a_refused_hello_is_an_error_with_the_players_reason() {
        let e = check_hello(r#"{"id":1,"ok":false,"error":"protocol 9 is not spoken"}"#);
        assert!(e.unwrap_err().contains("protocol 9"));
        assert!(check_hello(r#"{"id":1,"ok":true,"result":{}}"#).is_ok());
    }
}
