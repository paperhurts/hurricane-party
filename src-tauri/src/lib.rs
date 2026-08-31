mod pipeline;

use pipeline::{Probed, Track};
use tauri::AppHandle;

/// Phase 1 of the import pipeline — show the user what they're about to
/// download before downloading it (architecture.md).
#[tauri::command]
async fn probe_url(app: AppHandle, url: String) -> Result<Probed, pipeline::PipelineError> {
    pipeline::probe(&app, url.trim()).await
}

/// The whole job: probe, fetch audio, extract MP3.
#[tauri::command]
async fn import_url(app: AppHandle, url: String) -> Result<Track, pipeline::PipelineError> {
    pipeline::import(&app, url.trim()).await
}

/// What's already on disk. v0.1 has no database (that's v0.2), so the library
/// is whatever the folder contains.
#[tauri::command]
fn list_tracks(app: AppHandle) -> Result<Vec<Track>, pipeline::PipelineError> {
    pipeline::scan(&app)
}

/// Surfaced in the UI so "where does this write" is never a guess.
#[tauri::command]
fn library_path(app: AppHandle) -> Result<String, pipeline::PipelineError> {
    Ok(pipeline::library_root(&app)?.to_string_lossy().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            probe_url,
            import_url,
            list_tracks,
            library_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
