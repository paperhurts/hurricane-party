//! Cap'n Capy's switch (#192, D157). He is his own program on the public
//! control pipe (D22), so all the player does is start him, stop him, and
//! remember which. It starts him with `--with-player`, and he leaves by himself
//! the moment the player's pipe closes, so quitting the player, from its
//! close button, the tray or a crash, takes him with it without a line of
//! exit handling here.

use crate::db::{self, Db};
use crate::platform;
use std::path::PathBuf;
use std::process::Child;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

/// The program the player started, while it runs.
#[derive(Default)]
pub struct Companion(Mutex<Option<Child>>);

/// His program's file name, beside the player's own exe in the release zip.
pub fn exe_name() -> String {
    format!("hp-companion{}", std::env::consts::EXE_SUFFIX)
}

/// Where his program is: beside the player (the zip), or in a debug build the
/// companion crate's own debug build, so `cargo build` there is enough to try him.
fn exe_path() -> Option<PathBuf> {
    let mut places = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from))
    {
        places.push(dir.join(exe_name()));
    }
    #[cfg(debug_assertions)]
    places.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/hp-companion/target/debug")
            .join(exe_name()),
    );
    places.into_iter().find(|p| p.is_file())
}

/// Start him, unless the one we started is still running. A second copy
/// started some other way leaves at once on its own (he is single-instance).
pub fn start(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<Companion>();
    let mut held = state.0.lock().unwrap();
    if let Some(child) = held.as_mut() {
        if matches!(child.try_wait(), Ok(None)) {
            return Ok(());
        }
    }
    let exe = exe_path().ok_or_else(|| {
        format!(
            "Cap'n Capy's program ({}) is not beside the player",
            exe_name()
        )
    })?;
    let child = platform::platform()
        .spawn_quiet(&exe, &["--with-player"])
        .map_err(|e| format!("Cap'n Capy would not start: {e}"))?;
    *held = Some(child);
    Ok(())
}

/// Send him off, however he was started (D161): ask whichever Cap'n is
/// running to leave, then end the one we started, if it is still here. He
/// keeps nothing, so ending him is all it takes.
pub fn stop(app: &AppHandle) {
    platform::platform().ask_companion_to_leave();
    let state = app.state::<Companion>();
    let taken = state.0.lock().unwrap().take();
    if let Some(mut child) = taken {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// At launch: start him if the switch was left on. A failure here is quiet;
/// the switch says what went wrong the next time someone touches it.
pub fn at_launch(app: &AppHandle) {
    let on = {
        let state = app.state::<Db>();
        let conn = state.0.lock().unwrap();
        db::companion_on(&conn)
    };
    if on {
        if let Err(e) = start(app) {
            eprintln!("companion: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn his_program_sits_beside_the_player_under_one_name() {
        // On Windows that is hp-companion.exe, the name release-exe.yml
        // copies into the zip beside hurricane-party.exe.
        assert_eq!(
            exe_name().trim_end_matches(std::env::consts::EXE_SUFFIX),
            "hp-companion"
        );
    }
}
