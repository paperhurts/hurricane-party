pub mod bond;
mod control;
mod db;
mod jobs;
mod library;
mod localimport;
mod pipeline;
pub mod platform;
mod playlist;
mod skins;
mod tray;
mod video;
mod viz;
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
    let clean = pipeline::validate_url(&url).map_err(|e| e.to_string())?;
    // A list with no video in it is not one job (#137): `--no-playlist` has
    // nothing to reduce, so yt-dlp would download the entire list under a
    // single queue row. It is read with `probe_playlist` instead.
    if pipeline::list_id_of(&clean).is_some() && !pipeline::names_a_video(&clean) {
        return Err(
            "That link is a playlist, not a video. Paste it in the URL field to pick from it."
                .to_string(),
        );
    }
    jobs::enqueue(&app, url.trim(), want_video.unwrap_or(false), None).map_err(|e| e.to_string())
}

/// Read a pasted list without downloading anything (#137): its title, and
/// every entry with whether the library already has it.
#[tauri::command]
async fn probe_playlist(
    app: AppHandle,
    url: String,
) -> Result<pipeline::PlaylistProbe, pipeline::PipelineError> {
    pipeline::probe_playlist(&app, url.trim()).await
}

/// Queue the entries a person kept, into a playlist named after the list
/// (#137).
///
/// The playlist is made first and every job carries its id, so the association
/// survives a kill: forty jobs that come back after a power cut still know
/// which list they were for. Returns the playlist and how many jobs went in.
#[tauri::command]
fn enqueue_playlist(
    app: AppHandle,
    name: String,
    urls: Vec<String>,
    want_video: Option<bool>,
) -> Result<QueuedList, String> {
    for url in &urls {
        pipeline::validate_url(url).map_err(|e| e.to_string())?;
    }
    let name = name.trim();
    let name = if name.is_empty() { "Playlist" } else { name };
    let playlist_id = {
        let state = app.state::<Db>();
        let conn = state.0.lock().unwrap();
        playlist::create(&conn, name).map_err(|e| e.to_string())?
    };
    let mut queued = 0usize;
    for url in urls {
        jobs::enqueue(
            &app,
            url.trim(),
            want_video.unwrap_or(false),
            Some(playlist_id),
        )
        .map_err(|e| e.to_string())?;
        queued += 1;
    }
    let _ = app.emit("library-changed", ());
    Ok(QueuedList {
        playlist_id,
        queued,
    })
}

/// What `enqueue_playlist` made.
#[derive(serde::Serialize)]
struct QueuedList {
    playlist_id: i64,
    queued: usize,
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

/// Stop a download and drop it from the queue (D117). Before this, the
/// button that called `cancel_job` was labelled Pause and did neither.
#[tauri::command]
fn cancel_job(app: AppHandle, id: i64) -> Result<(), db::DbError> {
    jobs::cancel(&app, id)
}

/// Stop a download and keep its bytes for later (D117).
#[tauri::command]
fn pause_job(app: AppHandle, id: i64) -> Result<(), db::DbError> {
    jobs::pause(&app, id)
}

/// Pick a paused download up where it stopped (D117).
#[tauri::command]
fn resume_job(app: AppHandle, id: i64) -> Result<(), db::DbError> {
    jobs::resume(&app, id)
}

/// Pause, resume or cancel every unfinished download of one playlist import
/// (D117). `action` is "pause", "resume" or "cancel"; returns how many changed.
#[tauri::command]
fn playlist_jobs(app: AppHandle, playlist_id: i64, action: String) -> Result<usize, String> {
    let action = match action.as_str() {
        "pause" => jobs::ListAction::Pause,
        "resume" => jobs::ListAction::Resume,
        "cancel" => jobs::ListAction::Cancel,
        other => return Err(format!("no such action: {other}")),
    };
    jobs::apply_to_list(&app, playlist_id, action).map_err(|e| e.to_string())
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

// ---- taking things out (#78) -------------------------------------------------

/// The row goes; the file stays. Returns where the file still is, so the
/// library window can offer the separate step.
#[tauri::command]
fn remove_from_library(app: AppHandle, id: i64) -> Result<library::Removed, db::DbError> {
    let removed = {
        let state = app.state::<Db>();
        let mut conn = state.0.lock().unwrap();
        library::remove(&mut conn, id)?
    };
    let _ = app.emit("library-changed", ());
    Ok(removed)
}

/// A checked selection goes together (D84), in one transaction.
#[tauri::command]
fn remove_tracks(app: AppHandle, ids: Vec<i64>) -> Result<Vec<library::Removed>, db::DbError> {
    let removed = {
        let state = app.state::<Db>();
        let mut conn = state.0.lock().unwrap();
        library::remove_many(&mut conn, &ids)?
    };
    if !removed.is_empty() {
        let _ = app.emit("library-changed", ());
    }
    Ok(removed)
}

/// Drop the rows under one root whose files are gone. Returns how many.
#[tauri::command]
fn prune_root(app: AppHandle, root_id: i64) -> Result<usize, db::DbError> {
    let n = {
        let state = app.state::<Db>();
        let mut conn = state.0.lock().unwrap();
        library::prune(&mut conn, root_id)?
    };
    if n > 0 {
        let _ = app.emit("library-changed", ());
    }
    Ok(n)
}

/// The one destructive action. Inside a library root only; the frontend has
/// already shown the path and asked.
#[tauri::command]
fn delete_media_file(app: AppHandle, path: String) -> Result<(), db::DbError> {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    library::delete_file(&conn, &path)
}

// ---- control API (D9/D15) ---------------------------------------------------

/// The webview reporting playback state. Rust has no audio (D5), so this is
/// the only way the control channel can answer `status` truthfully.
#[tauri::command]
fn report_state(app: AppHandle, state: hp_control::PlayerState) {
    control::update_state(&app, state);
}

/// Main's own buttons, seek bar and volume, through the same router as the
/// pipe (D81): whatever is playing gets the command.
#[tauri::command]
fn transport(app: AppHandle, cmd: String, arg: Option<f64>) -> Result<(), String> {
    control::route(&app, &cmd, arg)
}

/// What the channel says is playing, for Main's display on mount (D81);
/// after that Rust pushes every change as `player:current`.
#[tauri::command]
fn transport_state(app: AppHandle) -> hp_control::PlayerState {
    control::current(&app)
}

// ---- viz stream (D15, #6) ---------------------------------------------------

/// One source frame from Main's stream analyser: scalars in the headers, the
/// raw FFT bins as the body. Sixty times a second while anyone is subscribed,
/// so it takes bytes rather than a JSON array of numbers.
#[tauri::command]
fn viz_frame(app: AppHandle, request: tauri::ipc::Request<'_>) -> Result<(), String> {
    viz::on_request(&app, &request)
}

/// Main asks on mount whether anyone is listening and how fast; afterwards
/// the hub pushes changes as `viz:capture`.
#[tauri::command]
fn viz_demand(app: AppHandle) -> viz::Demand {
    viz::demand(&app)
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
        // Restored first: `set_focus` raises and activates but never unminimizes
        // (#39, D59), so a switch into a minimized window succeeded invisibly.
        // All three best-effort: failing to raise the window is not failing to
        // switch the track (D67).
        let _ = w.unminimize();
        let _ = w.show();
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
    let window = tauri::WebviewWindowBuilder::new(
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
    // The window reports its own playback while it lives (D70); when the user
    // closes it, nothing is left to report, so Rust says so on its behalf.
    let gone = app.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            control::video_gone(&gone);
        }
    });
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

/// #86: Main's minimise button. The whole group goes; see `wm::minimize_group`.
#[tauri::command]
fn wm_minimize(app: AppHandle) {
    wm::minimize_group(&app);
}

/// #3: the close button the sprite chrome gave Main's title bar. It asks the
/// window to close rather than exiting here, so D63's `CloseRequested` handler
/// stays the single exit — one place that saves the layout and calls it a day,
/// whether the click landed on this button, the taskbar or Alt+F4. A satellite
/// refuses to close there, and so refuses here.
#[tauri::command]
fn wm_close(app: AppHandle, label: String) {
    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.close();
    }
}

/// The EQ and PL buttons a classic skin puts on Main (D109). They hide and show
/// their window rather than closing it: `wm_close` destroys, and a destroyed
/// satellite cannot come back until the next launch, which is the wrong price
/// for a button a person taps to get the clutter off the screen. Returns
/// whether the window is on screen afterwards, so the button lights without a
/// second call.
#[tauri::command]
fn wm_toggle_visible(app: AppHandle, label: String) -> bool {
    let Some(win) = app.get_webview_window(&label) else {
        return false;
    };
    let shown = win.is_visible().unwrap_or(false);
    let _ = if shown { win.hide() } else { win.show() };
    !shown
}

/// What those buttons show at mount, since nothing pushes visibility.
#[tauri::command]
fn wm_visible(app: AppHandle, label: String) -> bool {
    app.get_webview_window(&label)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// Which store the jar in use was read from, or "" when it was picked as a
/// file or never read (D115).
#[tauri::command]
fn get_cookies_from(app: AppHandle) -> String {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    db::get_setting(&conn, pipeline::COOKIES_FROM_SETTING).unwrap_or_default()
}

/// The `cookies.txt` this app hands yt-dlp, or "" when there is none (D112).
#[tauri::command]
fn get_cookies_file(app: AppHandle) -> String {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    db::get_setting(&conn, pipeline::COOKIES_SETTING).unwrap_or_default()
}

/// Point the app at one, or clear it with an empty string.
///
/// The file is a person's own session, exported from their own browser, and it
/// stays exactly where they put it: this stores the path and nothing else,
/// never the contents, never a copy in the library, never in a log (D112). It
/// must exist when it is set, so a typo fails at the button rather than three
/// minutes later inside a download.
#[tauri::command]
fn set_cookies_file(app: AppHandle, path: String) -> Result<String, String> {
    let p = path.trim();
    if !p.is_empty() {
        let pb = std::path::PathBuf::from(p);
        if !pb.is_absolute() {
            return Err("that path is not absolute".into());
        }
        if !pb.is_file() {
            return Err("there is no file there".into());
        }
    }
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    db::set_setting(&conn, pipeline::COOKIES_SETTING, p).map_err(|e| e.to_string())?;
    Ok(p.to_string())
}

/// Every browser profile a cookie export can read (D113, D115). Built in
/// Rust, so the picker cannot offer a store the export would then refuse —
/// and so a second Chrome profile, which is where a YouTube sign-in often
/// lives, is offered by name instead of hidden behind "chrome".
#[tauri::command]
fn cookie_browsers() -> Vec<pipeline::CookieSource> {
    pipeline::cookie_sources()
}

/// Read a browser's cookies with yt-dlp, keep the jar in the app's own folder
/// and use it from now on (D113).
///
/// This is the one place the app holds a credential rather than a path to
/// one: the person asked for it with a click, the file is theirs, it sits in
/// their own per-user app data, and the app only ever reports how many
/// cookies are in it.
#[tauri::command]
async fn export_cookies_from_browser(
    app: AppHandle,
    browser: String,
) -> Result<pipeline::CookieExport, String> {
    let spec = browser.trim();
    let made = pipeline::export_cookies(&app, spec)
        .await
        .map_err(|e| e.to_string())?;
    {
        let state = app.state::<Db>();
        let conn = state.0.lock().unwrap();
        db::set_setting(&conn, pipeline::COOKIES_SETTING, &made.path).map_err(|e| e.to_string())?;
        // Which store the jar in use came from, so a read that is not kept
        // can say what was kept instead. Unchanged when this one was not.
        if !made.kept {
            let label = pipeline::cookie_sources()
                .into_iter()
                .find(|s| s.spec == spec)
                .map(|s| s.label)
                .unwrap_or_else(|| spec.to_string());
            db::set_setting(&conn, pipeline::COOKIES_FROM_SETTING, &label)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(made)
}

/// The playlist window's ADD button: the library is where tracks come from.
/// A library hidden to the tray (#87) comes back the same way.
#[tauri::command]
fn show_library(app: AppHandle) {
    tray::reveal_library(&app);
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

/// Rename a playlist (D116).
#[tauri::command]
fn rename_playlist(app: AppHandle, id: i64, name: String) -> Result<(), db::DbError> {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    playlist::rename(&conn, id, &name)
}

/// Delete a playlist and keep every track in it (D116).
#[tauri::command]
fn delete_playlist(app: AppHandle, id: i64) -> Result<(), db::DbError> {
    let state = app.state::<Db>();
    let mut conn = state.0.lock().unwrap();
    playlist::delete(&mut conn, id)
}

/// Put a playlist at index `to` in the list (D116).
#[tauri::command]
fn move_playlist(app: AppHandle, id: i64, to: i64) -> Result<(), db::DbError> {
    let state = app.state::<Db>();
    let mut conn = state.0.lock().unwrap();
    playlist::move_to(&mut conn, id, to)
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

/// Shuffle and repeat (#115, D97). The library holds them while it runs and
/// saves them here, so they outlive a relaunch and the control pipe's
/// `status` can answer with them.
#[derive(serde::Serialize)]
struct PlayMode {
    shuffle: bool,
    repeat: String,
}

#[tauri::command]
fn get_play_mode(app: AppHandle) -> PlayMode {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    let (shuffle, repeat) = db::play_mode(&conn);
    PlayMode { shuffle, repeat }
}

#[tauri::command]
fn set_play_mode(app: AppHandle, shuffle: bool, repeat: String) -> Result<(), db::DbError> {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    db::set_play_mode(&conn, shuffle, &repeat)
}

// ---- skins (#107, D6) ------------------------------------------------------
//
// Importing is a click, never a watched folder (D91): the dialog, the
// validation and the refusal all happen where the person who chose the file
// is looking. Rust unpacks and stores; the frontend maps a `.wsz` into a
// manifest and validates it, then asks for it to be written or thrown away.

/// Where imported skins live. Configurable like every other sensitive path,
/// and under the app's own data directory by default.
fn skins_dir(app: &AppHandle) -> std::path::PathBuf {
    let state = app.state::<Db>();
    let configured = {
        let conn = state.0.lock().unwrap();
        db::get_setting(&conn, SKINS_DIR_SETTING)
    };
    match configured {
        Some(p) if !p.is_empty() => std::path::PathBuf::from(p),
        _ => app
            .path()
            .app_data_dir()
            .expect("no app data dir")
            .join("skins"),
    }
}

const SKINS_DIR_SETTING: &str = "skins.dir";
const SKIN_SETTING: &str = "skin.current";

#[tauri::command]
fn import_skin(app: AppHandle, path: String) -> Result<skins::Unpacked, skins::SkinError> {
    skins::unpack(std::path::Path::new(&path), &skins_dir(&app))
}

#[tauri::command]
fn write_skin_manifest(app: AppHandle, id: String, json: String) -> Result<(), skins::SkinError> {
    skins::write_manifest(&skins_dir(&app), &id, &json)
}

#[tauri::command]
fn discard_skin(app: AppHandle, id: String) -> Result<(), skins::SkinError> {
    skins::discard(&skins_dir(&app), &id)
}

/// Every skin a person can pick: the one that ships, first and always (D90),
/// then whatever they have imported.
#[tauri::command]
fn list_skins(app: AppHandle) -> Vec<String> {
    let mut out = vec!["eyewall".to_string()];
    out.extend(skins::installed(&skins_dir(&app)));
    out
}

#[tauri::command]
fn get_skin(app: AppHandle) -> String {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    db::get_setting(&conn, SKIN_SETTING).unwrap_or_else(|| "eyewall".into())
}

/// Wear a skin. The three classic windows hear `skin:changed` and reload,
/// so a switch is immediate rather than a relaunch.
#[tauri::command]
fn set_skin(app: AppHandle, id: String) -> Result<(), db::DbError> {
    {
        let state = app.state::<Db>();
        let conn = state.0.lock().unwrap();
        db::set_setting(&conn, SKIN_SETTING, &id)?;
    }
    let _ = app.emit("skin:changed", &id);
    Ok(())
}

/// The manifest of an imported skin, and the folder its sheets are in, so the
/// webview can ask for them over the asset protocol.
#[derive(serde::Serialize)]
struct SkinOnDisk {
    manifest: String,
    dir: String,
    /// Everything the importer needs to build the manifest again: the art it
    /// maps and the two text files it reads (D107).
    files: Vec<String>,
    pledit: Option<String>,
    viscolor: Option<String>,
}

#[tauri::command]
fn read_skin(app: AppHandle, id: String) -> Result<SkinOnDisk, skins::SkinError> {
    let dir = skins_dir(&app).join(&id);
    let manifest = std::fs::read_to_string(dir.join("manifest.json"))
        .map_err(|e| skins::SkinError::Io(format!("{id}: {e}")))?;
    let art = skins::contents(&dir);
    Ok(SkinOnDisk {
        manifest,
        dir: dir.to_string_lossy().to_string(),
        files: art.files,
        pledit: art.pledit,
        viscolor: art.viscolor,
    })
}

/// The chrome's glow (#108, D100). Every classic window reads it at mount
/// and hears `chrome:glow` when it changes, so the three windows turn over
/// together rather than on their next launch.
#[tauri::command]
fn get_glow(app: AppHandle) -> bool {
    let state = app.state::<Db>();
    let conn = state.0.lock().unwrap();
    db::glow(&conn)
}

#[tauri::command]
fn set_glow(app: AppHandle, on: bool) -> Result<(), db::DbError> {
    {
        let state = app.state::<Db>();
        let conn = state.0.lock().unwrap();
        db::set_glow(&conn, on)?;
    }
    let _ = app.emit("chrome:glow", on);
    Ok(())
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

/// D63, the other half, and #87: the library's × hides it behind a tray icon
/// (D87) rather than closing it, and the music keeps playing. It must not be
/// able to leave the app running with no way out either: if the classic
/// windows are somehow already gone, the library is the last window the user
/// can actually see, the hidden roots would keep the process alive invisibly,
/// and a tray icon is not a way out, so that case still exits.
fn wire_library_close(app: &AppHandle) {
    let Some(win) = app.get_webview_window("library") else {
        return;
    };
    let handle = app.clone();
    win.on_window_event(move |event| {
        let tauri::WindowEvent::CloseRequested { api, .. } = event else {
            return;
        };
        let any_classic_left = wm::CLASSIC
            .iter()
            .any(|id| handle.get_webview_window(wm::label_of(*id)).is_some());
        if !any_classic_left {
            handle.exit(0);
            return;
        }
        api.prevent_close();
        tray::hide_library(&handle);
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(RunnerHandle::default())
        .manage(jobs::Running::default())
        .manage(control::ControlState::default())
        .manage(control::Broadcaster::default())
        .manage(viz::VizHub::default())
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
            let mut conn = db::open(&path).expect("couldn't open the database");

            // D10: anything that was mid-flight when the process died goes back
            // in the queue. This re-enters the runner; it does NOT start over,
            // and it deliberately leaves .part files alone (D26).
            match db::recover_interrupted(&conn) {
                Ok(0) => {}
                Ok(n) => eprintln!("recovered {n} interrupted job(s) from the last run"),
                Err(e) => eprintln!("recovery failed: {e}"),
            }

            // #78: a root the scanner stored in Windows' verbatim form is the
            // same folder the download pipeline stored plain, and it doubled
            // every row. Fold them before anything reads the library.
            match db::normalize_roots(&mut conn) {
                Ok(0) => {}
                Ok(n) => eprintln!("folded {n} verbatim library root(s) into their plain form"),
                Err(e) => eprintln!("root normalization failed: {e}"),
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
            probe_playlist,
            enqueue_playlist,
            list_jobs,
            retry_job,
            cancel_job,
            pause_job,
            resume_job,
            playlist_jobs,
            list_tracks,
            library_path,
            list_playlists,
            create_playlist,
            rename_playlist,
            delete_playlist,
            move_playlist,
            playlist_items,
            add_to_playlist,
            remove_from_playlist,
            reorder_playlist,
            get_concurrency,
            set_concurrency,
            get_play_mode,
            set_play_mode,
            get_glow,
            set_glow,
            import_skin,
            write_skin_manifest,
            discard_skin,
            list_skins,
            get_skin,
            set_skin,
            read_skin,
            add_local_folder,
            list_roots,
            remove_from_library,
            remove_tracks,
            prune_root,
            delete_media_file,
            open_video,
            video_ready,
            report_state,
            transport,
            transport_state,
            viz_frame,
            viz_demand,
            wm_drag_start,
            wm_drag_move,
            wm_drag_end,
            wm_focus,
            wm_raise,
            wm_set_double,
            wm_resize_start,
            wm_resize_move,
            wm_resize_end,
            wm_minimize,
            wm_close,
            wm_toggle_visible,
            wm_visible,
            cookie_browsers,
            export_cookies_from_browser,
            get_cookies_file,
            get_cookies_from,
            set_cookies_file,
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
