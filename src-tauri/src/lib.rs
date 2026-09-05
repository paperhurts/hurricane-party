pub mod bond;
mod control;
mod db;
mod jobs;
mod localimport;
mod pipeline;
pub mod platform;
mod playlist;
mod video;
pub mod wm;

use db::Db;
use jobs::{Job, RunnerHandle};
use tauri::{AppHandle, Emitter, Manager};

// ---- import -----------------------------------------------------------------

/// Phase 1 — show the user what they're about to download before downloading it.
#[tauri::command]
async fn probe_url(
    app: AppHandle,
    url: String,
) -> Result<pipeline::Probed, pipeline::PipelineError> {
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
async fn add_local_folder(
    app: AppHandle,
    path: String,
    label: Option<String>,
) -> Result<localimport::ScanReport, db::DbError> {
    let p = std::path::PathBuf::from(&path);
    let label = label.unwrap_or_else(|| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or(path.clone())
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

    // Serialized, ack wait included: see `video::OpenLock` for the gap this
    // closes. Held to the end of the function on every path.
    let lock = app.state::<video::OpenLock>();
    let _one_at_a_time = lock.0.lock().await;

    if let Some(w) = app.get_webview_window(LABEL) {
        // Already open: tell it to switch, and wait for it to say it has.
        //
        // D67: the emit cannot report a dead webview, so the ack's absence is
        // the only failure signal there is. D68: an event rather than a
        // navigation, so the bundle does not reload and a re-click of the
        // track already showing does not rewind it. Register before the emit
        // so an ack that beats the wait is already in the channel.
        let pending = app.state::<video::SwitchAcks>().expect(id);
        let started = std::time::Instant::now();
        app.emit_to(LABEL, video::SWITCH_EVENT, id)
            .map_err(|e| e.to_string())?;
        pending
            .wait(video::ACK_TIMEOUT)
            .await
            .map_err(|e| e.to_string())?;
        // Measured so the hand test can judge ACK_TIMEOUT's margin.
        eprintln!("video: switch to {id} acked in {:?}", started.elapsed());
        let _ = w.set_focus();
        return Ok(());
    }

    // D68 applied to creation. The label is in the map the instant `build()`
    // returns, long before the page exists, so without this a click in that
    // window would take the branch above and lose its event (D67). Register,
    // build, then wait for the page to say it is up and listening; only then
    // does the lock release and let the next click through.
    let pending = app.state::<video::SwitchAcks>().expect(id);
    let started = std::time::Instant::now();
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
    pending
        .wait(video::MOUNT_TIMEOUT)
        .await
        .map_err(|e| e.to_string())?;
    // Measured so the hand test can judge MOUNT_TIMEOUT's margin.
    eprintln!("video: window up on {id} in {:?}", started.elapsed());
    Ok(())
}

/// The video window confirming it has switched to `id` (D68). Completes the
/// wait in `open_video`; an ack that arrives after the timeout finds nobody and
/// is dropped, which is right, because that click has already reported
/// failure. Internal: not part of the control protocol (D15).
#[tauri::command]
fn video_ready(app: AppHandle, id: i64) {
    app.state::<video::SwitchAcks>().complete(id);
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

/// Playback started from another window: bring the group forward, and back
/// from minimized, without stealing focus from where the user is working.
#[tauri::command]
fn wm_raise(app: AppHandle, label: String) {
    if let Some(id) = wm::id_of(&label) {
        wm::raise_group(&app, id);
    }
}

/// The classic chrome at 1x or 2x (#47). Integer only; fractional chrome
/// scaling is anti-scope.
#[tauri::command]
fn wm_set_double(app: AppHandle, on: bool) {
    wm::set_double(&app, on);
}

/// Corner grip on the playlist: pointerdown. False means nothing to resize.
#[tauri::command]
fn wm_resize_start(app: AppHandle, label: String) -> bool {
    match wm::id_of(&label) {
        Some(id) => wm::resize_start(&app, id),
        None => false,
    }
}

#[tauri::command]
fn wm_resize_move(app: AppHandle) {
    wm::resize_move(&app);
}

#[tauri::command]
fn wm_resize_end(app: AppHandle) {
    wm::resize_end(&app);
}

/// The playlist window's ADD button: the library is where tracks come from.
#[tauri::command]
fn show_library(app: AppHandle) {
    if let Some(w) = app.get_webview_window("library") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
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
fn remove_from_playlist(
    app: AppHandle,
    playlist_id: i64,
    position: i64,
) -> Result<(), db::DbError> {
    let state = app.state::<Db>();
    let mut conn = state.0.lock().unwrap();
    playlist::remove(&mut conn, playlist_id, position)
}

#[tauri::command]
fn reorder_playlist(
    app: AppHandle,
    playlist_id: i64,
    from: i64,
    to: i64,
) -> Result<(), db::DbError> {
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
            // D63: closing the Main window quits the app; the satellites refuse
            // to close at all.
            //
            // This is not a nicety, it is the only way out. Tauri exits when
            // every window is closed, and D41's three hidden roots are windows
            // that are never shown and can never be closed — so that condition
            // could not be met, and the app had no exit path whatsoever.
            // Worse, closing Main from the taskbar destroyed only Main and left
            // eq and playlist behind as undecorated windows with no taskbar
            // button and no title bar: D59's unrecoverable state, reached
            // without a monitor ever being unplugged.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if id == wm::MAIN {
                    wm::save_now(&handle);
                    handle.exit(0);
                } else {
                    // The satellites have no close affordance of their own and
                    // nothing yet to bring them back, so Alt+F4 doing nothing
                    // beats a window that vanishes for good. Reopening them is
                    // v0.4b, with the sprite chrome that offers it.
                    api.prevent_close();
                }
                return;
            }
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

/// D63, the other half: the library window is an ordinary decorated window, so
/// closing it closes it — but it must not be able to leave the app running with
/// no way out either. If the classic windows are somehow already gone, closing
/// the library is the last window the user can actually see, and the hidden
/// roots would keep the process alive invisibly.
fn wire_library_close(app: &AppHandle) {
    let Some(win) = app.get_webview_window("library") else {
        return;
    };
    let handle = app.clone();
    win.on_window_event(move |event| {
        if !matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
            return;
        }
        let any_classic_left = wm::CLASSIC
            .iter()
            .any(|id| handle.get_webview_window(wm::label_of(*id)).is_some());
        if !any_classic_left {
            handle.exit(0);
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(RunnerHandle::default())
        .manage(control::ControlState::default())
        .manage(control::Broadcaster::default())
        .manage(wm::Wm::default())
        .manage(video::SwitchAcks::default())
        .manage(video::OpenLock::default())
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
            wire_library_close(&handle);
            // Last: the windows are only revealed once the bond graph and the
            // ownership topology behind them are real.
            wm::show_classic_windows(&handle)?;
            // D62: the display watchdog. Polls rather than hooking
            // WM_DISPLAYCHANGE, and covers a group already stranded at launch
            // as well as one stranded while running.
            wm::spawn_display_watch(&handle);
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
            video_ready,
            report_state,
            wm_drag_start,
            wm_drag_move,
            wm_drag_end,
            wm_focus,
            wm_raise,
            wm_set_double,
            wm_resize_start,
            wm_resize_move,
            wm_resize_end,
            show_library,
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
