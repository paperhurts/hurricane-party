//! Cap'n Capy's switch (#192, D157). He is his own program on the public
//! control pipe (D22), so all the player does is start him, stop him, and
//! remember which. It starts him with `--with-player`, and he leaves by himself
//! the moment the player's pipe closes, so quitting the player, from its
//! close button, the tray or a crash, takes him with it without a line of
//! exit handling here.

use crate::db::{self, Db};
use crate::packs;
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

fn exe() -> Result<PathBuf, String> {
    exe_path().ok_or_else(|| {
        format!(
            "Cap'n Capy's program ({}) is not beside the player",
            exe_name()
        )
    })
}

/// Start the picked companion, unless the one we started is still running. A
/// second copy started some other way leaves at once on its own (only one
/// runs). Cap'n Capy finds his own pack; any other is named by its folder
/// in the companions folder (D162).
pub fn start(app: &AppHandle) -> Result<(), String> {
    let pick = {
        let state = app.state::<Db>();
        let conn = state.0.lock().unwrap();
        db::companion_pick(&conn)
    };
    let pack = if pick == packs::CAPTAIN {
        None
    } else {
        Some(
            packs::folder(&crate::companions_dir(app), &pick)
                .ok_or("that companion's folder is gone; pick another in the list")?,
        )
    };
    let state = app.state::<Companion>();
    let mut held = state.0.lock().unwrap();
    if let Some(child) = held.as_mut() {
        if matches!(child.try_wait(), Ok(None)) {
            return Ok(());
        }
    }
    let exe = exe()?;
    let dir = pack.map(|p| p.to_string_lossy().into_owned());
    let mut args = vec!["--with-player"];
    if let Some(d) = &dir {
        args.extend(["--pack", d.as_str()]);
    }
    let child = platform::platform()
        .spawn_quiet(&exe, &args)
        .map_err(|e| format!("the companion would not start: {e}"))?;
    *held = Some(child);
    Ok(())
}

/// Send off whoever is on screen and start the pick, for a new pick while
/// the box is ticked. Only one companion runs, so the new one waits until the
/// old one has gone, or it would meet him and leave.
pub fn restart(app: &AppHandle) -> Result<(), String> {
    stop(app);
    let p = platform::platform();
    for _ in 0..30 {
        if !p.companion_is_running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    start(app)
}

/// Check a pack with the companion's own loader (`--check`, D162): the
/// pack's name when it is good, and his reason when it is not.
pub fn check(pack: &std::path::Path) -> Result<String, String> {
    let exe = exe()?;
    let out = platform::platform()
        .run_quiet(
            &exe,
            &["--check".as_ref(), "--pack".as_ref(), pack.as_os_str()],
        )
        .map_err(|e| format!("the companion could not check it: {e}"))?;
    let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match (out.status.success(), said.strip_prefix("ok: ")) {
        (true, Some(name)) => Ok(name.to_string()),
        _ => Err(said.strip_prefix("refused: ").unwrap_or(&said).to_string()),
    }
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
