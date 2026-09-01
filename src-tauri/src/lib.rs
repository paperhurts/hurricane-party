pub mod bond;
mod control;
mod db;
mod jobs;
mod localimport;
mod pipeline;
pub mod platform;
mod playlist;
pub mod wm;

use db::Db;
use jobs::{Job, RunnerHandle};
use tauri::{AppHandle, Manager};

// ---- import -----------------------------------------------------------------

/// Phase 1 — show the user what they're about to download before downloading it.
#[tauri::command]
async fn probe_url(app: AppHandle, url: String) -> Result<pipeline::Probed, pipeline::PipelineError> {
    pipeline::probe(&app, url.trim(), None).await
}

/// Queue a URL. Returns immediately; the runner picks it up.
#[tauri::command]
fn enqueue_url(app: AppHandle, url: String, want_video: Option<bool>) -> Result<i64, String> {
    // Validate before it reaches the queue so a bad URL fails at the button,
    // not three seconds later inside a worker.
    pipeline::validate_url(&url).map_err(|e| e.to_string())?;
    jobs::enqueue(&app, url.trim(), want_video.unwrap_or(false)).map_err(|e| e.to_string())
}

// ---- queue ------------------------------------------------------------------

#[tauri::command]
fn list_jobs(app: AppHandle) -> Result<Vec<Job>, db::DbError> {
    let conn = app.state::<Db>();
    let conn = conn.0.lock().unwrap();
    jobs::list(&conn)
}

#[tauri::command]
fn retry_job(app: AppHandle, id: i64) -> Result<(), db::DbError> {
    jobs::retry(&app, id)
}

#[tauri::command]
fn cancel_job(app: AppHandle, id: i64) -> Result<(), db::DbError> {
    jobs::cancel(&app, id)
}

// ---- library ----------------------------------------------------------------

#[tauri::command]
fn list_tracks(app: AppHandle) -> Result<Vec<playlist::MediaRow>, db::DbError> {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    playlist::list_media(&conn)
}

#[tauri::command]
fn library_path(app: AppHandle) -> Result<String, pipeline::PipelineError> {
    Ok(pipeline::library_root(&app)?.to_string_lossy().to_string())
}

// ---- local folder import (D28 roots, D34 titles, D50 tags) -----------------

#[tauri::command]
async fn add_local_folder(app: AppHandle, path: String, label: Option<String>)
    -> Result<localimport::ScanReport, db::DbError>
{
    let p = std::path::PathBuf::from(&path);
    let label = label.unwrap_or_else(|| {
        p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(path.clone())
    });
    // Scanning a big folder blocks on I/O and tag reads, so keep it off the
    // main thread rather than freezing the window.
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || localimport::scan_root(&app2, &p, &label))
        .await
        .map_err(|e| db::DbError::Io(e.to_string()))?
}

#[tauri::command]
fn list_roots(app: AppHandle) -> Result<Vec<localimport::Root>, db::DbError> {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    localimport::list_roots(&conn)
}

// ---- control API (D9/D15) ---------------------------------------------------

/// The webview reporting playback state. Rust has no audio (D5), so this is
/// the only way the control channel can answer `status` truthfully.
#[tauri::command]
fn report_state(app: AppHandle, state: hp_control::PlayerState) {
    control::update_state(&app, state);
}

// ---- video window (D13) -----------------------------------------------------

/// Open (or focus) the video window for a track.
///
/// A real OS window: decorated, resizable, and deliberately NOT part of the
/// bond group — the three classic 275px windows are the only skinned,
/// undecorated ones. It loads its own HTML entry point rather than a route, so
/// the frontend stays plain Vite; v0.4 adds eq.html and playlist.html the same
/// way.
#[tauri::command]
async fn open_video(app: AppHandle, id: i64) -> Result<(), String> {
    const LABEL: &str = "video";

    if let Some(w) = app.get_webview_window(LABEL) {
        // Already open on a different track: point it at the new one rather
        // than stacking up windows.
        let _ = w.eval(&format!("location.search = '?id={id}'"));
        let _ = w.set_focus();
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        LABEL,
        tauri::WebviewUrl::App(format!("video.html?id={id}").into()),
    )
    .title("hurricane-party — video")
    .inner_size(960.0, 560.0)
    .min_inner_size(320.0, 200.0)
    .resizable(true)
    .decorations(true)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---- window manager ---------------------------------------------------------
//
// These are synchronous on purpose. Tauri runs a non-async command on the main
// thread, which is where our windows live — so every SetWindowPos in a drag
// frame is same-thread, and D54's cross-thread deadlock is structurally out of
// reach rather than merely avoided. It also keeps the drag off the async
// runtime, where a scheduling hiccup would show up as a stutter.

/// Title-bar pointerdown. Snapshots the group and the cursor; everything after
/// this is derived from that origin (D40).
#[tauri::command]
fn wm_drag_start(app: AppHandle, label: String) {
    if let Some(id) = wm::id_of(&label) {
        wm::drag_start(&app, id);
    }
}

/// One drag frame. The webview coalesces pointermove to one per display frame,
/// so this is called at roughly refresh rate and has to stay cheap.
#[tauri::command]
fn wm_drag_move(app: AppHandle) {
    wm::drag_move(&app);
}

#[tauri::command]
fn wm_drag_end(app: AppHandle) {
    wm::drag_end(&app);
}

/// What a classic window asks for on mount.
///
/// The push events fire when things change; a window still loading its bundle
/// misses them. This is the pull half of that pair.
#[tauri::command]
fn wm_hello(app: AppHandle, label: String) -> Option<wm::Hello> {
    wm::id_of(&label).map(|id| wm::hello(&app, id))
}

/// D60: title-bar double-click collapses a window to the 275 x 14 strip, and
/// expands it again. D61 handles the always-on-top half.
#[tauri::command]
fn wm_toggle_shade(app: AppHandle, label: String) {
    if let Some(id) = wm::id_of(&label) {
        wm::toggle_shade(&app, id);
    }
}

/// Pointerdown on a bonded edge.
///
/// Returns which gesture the caller actually got. D35: a seam whose neighbours
/// cannot resize is a move handle, so rather than offering a splitter and then
/// doing nothing, the gesture degrades to a group move and says so.
#[tauri::command]
fn wm_seam_down(app: AppHandle, label: String, edge: String) -> &'static str {
    let (Some(id), Some(edge)) = (wm::id_of(&label), wm::edge_from_str(&edge)) else {
        return "none";
    };
    if wm::splitter_start(&app, id, edge) {
        "splitter"
    } else {
        wm::drag_start(&app, id);
        "move"
    }
}

#[tauri::command]
fn wm_splitter_move(app: AppHandle) {
    wm::splitter_move(&app);
}

#[tauri::command]
fn wm_splitter_end(app: AppHandle) {
    wm::splitter_end(&app);
}

/// Double-click on a seam. Breaking a bond in the middle of a chain splits one
/// group into two, so the components are recomputed and each side gets its own
/// hidden root (D41).
#[tauri::command]
fn wm_demagnetize(app: AppHandle, label: String, edge: String) -> bool {
    let (Some(id), Some(edge)) = (wm::id_of(&label), wm::edge_from_str(&edge)) else {
        return false;
    };
    wm::demagnetize(&app, id, edge)
}

/// Pointerdown anywhere in a classic window: raise the whole group (D42).
#[tauri::command]
fn wm_focus(app: AppHandle, label: String) {
    if let Some(id) = wm::id_of(&label) {
        wm::focus_group(&app, Some(id));
    }
}

// ---- playlists --------------------------------------------------------------

#[tauri::command]
fn list_playlists(app: AppHandle) -> Result<Vec<playlist::Playlist>, db::DbError> {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    playlist::list(&conn)
}

#[tauri::command]
fn create_playlist(app: AppHandle, name: String) -> Result<i64, db::DbError> {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    playlist::create(&conn, name.trim())
}

#[tauri::command]
fn playlist_items(app: AppHandle, id: i64) -> Result<Vec<playlist::MediaRow>, db::DbError> {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    playlist::items(&conn, id)
}

#[tauri::command]
fn add_to_playlist(app: AppHandle, playlist_id: i64, media_id: i64) -> Result<(), db::DbError> {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    playlist::add(&conn, playlist_id, media_id)
}

#[tauri::command]
fn remove_from_playlist(app: AppHandle, playlist_id: i64, position: i64) -> Result<(), db::DbError> {
    let state = app.state::<Db>();
    let mut conn = state.0.lock().unwrap();
    playlist::remove(&mut conn, playlist_id, position)
}

#[tauri::command]
fn reorder_playlist(app: AppHandle, playlist_id: i64, from: i64, to: i64) -> Result<(), db::DbError> {
    let state = app.state::<Db>();
    let mut conn = state.0.lock().unwrap();
    playlist::reorder(&mut conn, playlist_id, from, to)
}

// ---- settings ---------------------------------------------------------------

#[tauri::command]
fn get_concurrency(app: AppHandle) -> usize {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    db::concurrency(&conn)
}

#[tauri::command]
fn set_concurrency(app: AppHandle, n: usize) -> Result<(), db::DbError> {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    db::set_setting(&conn, "download.concurrency", &n.clamp(1, 4).to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Focus is a group property (v0.4-brief): when any bonded window has focus, all
/// of them render active.
///
/// The deferred re-check on focus loss is not defensive, it is required.
/// Windows sends `WM_KILLFOCUS` to the old window *before* `WM_SETFOCUS` to the
/// new one, so clicking from main to eq passes through a moment where nothing
/// in the group is focused. Deactivating on that intermediate state flickers
/// the entire group on every click inside it — which is precisely the thing the
/// brief says looks broken immediately.
fn wire_focus_events(app: &AppHandle) {
    for id in wm::CLASSIC {
        let Some(win) = app.get_webview_window(wm::label_of(id)) else {
            continue;
        };
        let handle = app.clone();
        win.on_window_event(move |event| {
            let tauri::WindowEvent::Focused(gained) = event else {
                return;
            };
            if *gained {
                wm::focus_group(&handle, Some(id));
                return;
            }
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                let still_ours = wm::CLASSIC.iter().any(|id| {
                    handle
                        .get_webview_window(wm::label_of(*id))
                        .and_then(|w| w.is_focused().ok())
                        .unwrap_or(false)
                });
                if !still_ours {
                    wm::focus_group(&handle, None);
                }
            });
        });
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(RunnerHandle::default())
        .manage(control::ControlState::default())
        .manage(control::Broadcaster::default())
        .manage(wm::Wm::default())
        .setup(|app| {
            // D37: the gate, and it runs first. Every physical coordinate this
            // process computes after this line depends on the answer, so there
            // is no useful work to do if it is wrong. Panics if awareness is
            // not per-monitor-v2 — deliberately, and permanently.
            eprintln!("DPI awareness: {}", platform::platform().assert_dpi_aware());

            let handle = app.handle().clone();
            let path = handle
                .path()
                .app_data_dir()
                .expect("no app data dir")
                .join("hurricane-party.db");
            let conn = db::open(&path).expect("couldn't open the database");

            // D10: anything that was mid-flight when the process died goes back
            // in the queue. This re-enters the runner; it does NOT start over,
            // and it deliberately leaves .part files alone (D26).
            match db::recover_interrupted(&conn) {
                Ok(0) => {}
                Ok(n) => eprintln!("recovered {n} interrupted job(s) from the last run"),
                Err(e) => eprintln!("recovery failed: {e}"),
            }

            app.manage(Db(std::sync::Mutex::new(conn)));

            // Asset-protocol scope is runtime state and doesn't survive a
            // restart the way the library_roots rows do, so re-grant it or
            // yesterday's imported folder stops playing today.
            localimport::allow_known_roots(&handle);

            jobs::spawn_runner(handle.clone());

            // Undocumented and unstable until v1.0 (control-api.md). Shipping
            // it now proves the pipe while nothing external depends on it.
            let bc = app.state::<control::Broadcaster>().inner().clone();
            control::spawn_server(handle.clone(), bc);

            // The three classic windows, their hidden roots (D41), and the
            // ownership topology. Last in setup because it is the only part
            // that puts pixels on screen.
            // Seed first: a webview starts loading the instant its window is
            // constructed and asks for its bonds before setup has finished.
            wm::seed_state(&handle)?;
            wm::build_classic_windows(&handle)?;
            wm::register(&handle)?;
            wire_focus_events(&handle);
            // Last: the windows are only revealed once the bond graph and the
            // ownership topology behind them are real.
            wm::show_classic_windows(&handle)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            probe_url,
            enqueue_url,
            list_jobs,
            retry_job,
            cancel_job,
            list_tracks,
            library_path,
            list_playlists,
            create_playlist,
            playlist_items,
            add_to_playlist,
            remove_from_playlist,
            reorder_playlist,
            get_concurrency,
            set_concurrency,
            add_local_folder,
            list_roots,
            open_video,
            report_state,
            wm_drag_start,
            wm_drag_move,
            wm_drag_end,
            wm_focus,
            wm_hello,
            wm_toggle_shade,
            wm_seam_down,
            wm_splitter_move,
            wm_splitter_end,
            wm_demagnetize
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
