pub mod decode;
pub mod i18n;
pub mod master;
pub mod merge;
pub mod player;
pub mod scan;
pub mod sync;
pub mod wav;

#[cfg(test)]
mod testutil;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;
use i18n::{t, Msg};

#[derive(Default)]
struct AppState {
    scan: Mutex<Option<Arc<scan::Scan>>>,
    sync_plan: Mutex<Option<Arc<sync::SyncPlan>>>,
    master_plan: Mutex<Option<Arc<master::MasterPlan>>>,
    player: OnceLock<player::Player>,
    cancel: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
}

#[derive(Clone, Serialize)]
struct Beat {
    op: &'static str,
    elapsed_ms: u64,
}

/// Sends `work-heartbeat` every 0.5 s while alive, so the interface can show
/// that the backend really is still computing.
struct Heartbeat(Arc<AtomicBool>);

impl Heartbeat {
    fn start(app: &AppHandle, op: &'static str) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, app) = (stop.clone(), app.clone());
        std::thread::spawn(move || {
            let t = Instant::now();
            while !flag.load(Ordering::SeqCst) {
                let _ = app.emit("work-heartbeat", Beat { op, elapsed_ms: t.elapsed().as_millis() as u64 });
                std::thread::sleep(Duration::from_millis(500));
            }
        });
        Heartbeat(stop)
    }
}

impl Drop for Heartbeat {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn player<'a>(app: &AppHandle, state: &'a AppState) -> &'a player::Player {
    state.player.get_or_init(|| {
        let app = app.clone();
        player::Player::start(move |p| {
            let _ = app.emit("player-position", p);
        })
    })
}

#[tauri::command]
async fn scan_paths(app: AppHandle, state: State<'_, AppState>, paths: Vec<String>) -> Result<scan::Scan, String> {
    let _beat = Heartbeat::start(&app, "scan");
    let inputs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let result = tauri::async_runtime::spawn_blocking(move || scan::scan(&inputs, &scan::Options::default()))
        .await
        .map_err(|e| e.to_string())??;
    *state.scan.lock().map_err(|e| e.to_string())? = Some(Arc::new(result.clone()));
    Ok(result)
}

#[tauri::command]
async fn pick_folder(app: AppHandle, title: String) -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || app.dialog().file().set_title(title).blocking_pick_folder())
        .await
        .map_err(|e| e.to_string())?;
    Ok(picked.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
}

#[tauri::command]
async fn merge_recordings(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<usize>,
    out_dir: String,
) -> Result<merge::Summary, String> {
    let scan = state
        .scan
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or(t(Msg::ScanFirst))?;
    if ids.is_empty() {
        return Err(t(Msg::NoRecordingsSelected).into());
    }
    if out_dir.trim().is_empty() {
        return Err(t(Msg::NoOutDir).into());
    }
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err(t(Msg::Busy).into());
    }
    let _beat = Heartbeat::start(&app, "merge");
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();
    let busy = state.busy.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let recs: Vec<&scan::Recording> = scan.recordings.iter().filter(|r| ids.contains(&r.id)).collect();
        let mut last: Option<Instant> = None;
        merge::run(&recs, Path::new(&out_dir), &cancel, |p| {
            if p.milestone || last.map_or(true, |t| t.elapsed() >= Duration::from_millis(80)) {
                last = Some(Instant::now());
                let _ = app.emit("merge-progress", p);
            }
        })
    })
    .await;
    busy.store(false, Ordering::SeqCst);
    result.map_err(|e| e.to_string())?
}

/// Second function: find synchronous stretches between tracks of different recorders.
#[tauri::command]
async fn analyze_tracks(app: AppHandle, state: State<'_, AppState>, paths: Vec<String>) -> Result<sync::SyncPlan, String> {
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err(t(Msg::Busy).into());
    }
    let _beat = Heartbeat::start(&app, "sync-analyze");
    player(&app, &state).pause();
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();
    let busy = state.busy.clone();
    let inputs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let events = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let app = events;
        let mut last: Option<Instant> = None;
        sync::analyze(&inputs, &cancel, &mut |p| {
            if p.done >= p.total || last.map_or(true, |t| t.elapsed() >= Duration::from_millis(100)) {
                last = Some(Instant::now());
                let _ = app.emit("sync-progress", p);
            }
        })
    })
    .await;
    busy.store(false, Ordering::SeqCst);
    let plan = result.map_err(|e| e.to_string())??;
    player(&app, &state).set_mix(player::Mix::from_plan(&plan));
    *state.sync_plan.lock().map_err(|e| e.to_string())? = Some(Arc::new(plan.clone()));
    Ok(plan)
}

#[derive(Serialize)]
struct ClipsUpdate {
    clips: Vec<sync::Clip>,
    items: Vec<sync::Item>,
    edited: bool,
}

fn edit_plan(app: &AppHandle, state: &AppState, change: impl FnOnce(&mut sync::SyncPlan) -> Result<(), String>) -> Result<ClipsUpdate, String> {
    let mut guard = state.sync_plan.lock().map_err(|e| e.to_string())?;
    let arc = guard.as_mut().ok_or(t(Msg::AnalyzeTracksFirst))?;
    let plan = Arc::make_mut(arc);
    change(plan)?;
    player(app, state).set_mix(player::Mix::from_plan(plan));
    Ok(ClipsUpdate { clips: plan.clips.clone(), items: plan.items.clone(), edited: plan.edited })
}

/// Takes the edited clips of the timeline, returns the resulting output files.
#[tauri::command]
fn sync_set_clips(app: AppHandle, state: State<'_, AppState>, clips: Vec<sync::Clip>) -> Result<ClipsUpdate, String> {
    edit_plan(&app, &state, |plan| sync::apply_clips(plan, clips))
}

#[tauri::command]
fn sync_reset_clips(app: AppHandle, state: State<'_, AppState>) -> Result<ClipsUpdate, String> {
    edit_plan(&app, &state, sync::reset_clips)
}

#[tauri::command]
fn sync_peaks(state: State<'_, AppState>, track: usize, t0: f64, t1: f64, buckets: usize) -> Result<Vec<u8>, String> {
    let guard = state.sync_plan.lock().map_err(|e| e.to_string())?;
    let plan = guard.as_ref().ok_or(t(Msg::AnalyzeTracksFirst))?;
    Ok(sync::peaks(plan, track, t0, t1, buckets))
}

#[tauri::command]
fn player_play(app: AppHandle, state: State<'_, AppState>, t: f64) {
    player(&app, &state).play(t);
}

#[tauri::command]
fn player_pause(app: AppHandle, state: State<'_, AppState>) {
    player(&app, &state).pause();
}

#[tauri::command]
fn player_seek(app: AppHandle, state: State<'_, AppState>, t: f64) {
    player(&app, &state).seek(t);
}

#[tauri::command]
fn player_solo(app: AppHandle, state: State<'_, AppState>, muted: Vec<String>, solo: Vec<String>) {
    player(&app, &state).set_solo(muted, solo);
}

/// "Wo speichern?": picks a folder, starting next to the sources, and returns
/// the step's subfolder inside it (or the folder itself if it already has that name).
#[tauri::command]
async fn pick_output_dir(app: AppHandle, title: String, start: String, sub: String) -> Result<Option<String>, String> {
    let start_dir = PathBuf::from(&start);
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = app.dialog().file().set_title(title);
        if start_dir.is_dir() {
            dialog = dialog.set_directory(&start_dir);
        }
        dialog.blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(picked.and_then(|p| p.into_path().ok()).map(|dir| {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let out = if sub.is_empty() || name == sub { dir } else { dir.join(&sub) };
        out.to_string_lossy().into_owned()
    }))
}

#[tauri::command]
async fn write_sync(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<usize>,
    out_dir: String,
) -> Result<merge::Summary, String> {
    let plan = state
        .sync_plan
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or(t(Msg::AnalyzeTracksFirst))?;
    if ids.is_empty() {
        return Err(t(Msg::NoSectionsSelected).into());
    }
    if out_dir.trim().is_empty() {
        return Err(t(Msg::NoOutDir).into());
    }
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err(t(Msg::Busy).into());
    }
    let _beat = Heartbeat::start(&app, "sync-write");
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();
    let busy = state.busy.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut last: Option<Instant> = None;
        sync::write(&plan, &ids, Path::new(&out_dir), &cancel, |p| {
            if p.milestone || last.map_or(true, |t| t.elapsed() >= Duration::from_millis(80)) {
                last = Some(Instant::now());
                let _ = app.emit("sync-write-progress", p);
            }
        })
    })
    .await;
    busy.store(false, Ordering::SeqCst);
    result.map_err(|e| e.to_string())?
}

/// Third function: measure loudness, master to -16 LUFS as MP3.
#[tauri::command]
async fn analyze_master(app: AppHandle, state: State<'_, AppState>, paths: Vec<String>) -> Result<master::MasterPlan, String> {
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err(t(Msg::Busy).into());
    }
    let _beat = Heartbeat::start(&app, "master-analyze");
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();
    let busy = state.busy.clone();
    let inputs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut last: Option<Instant> = None;
        master::analyze(&inputs, &cancel, &mut |p| {
            if p.done >= p.total || last.map_or(true, |t| t.elapsed() >= Duration::from_millis(100)) {
                last = Some(Instant::now());
                let _ = app.emit("master-progress", p);
            }
        })
    })
    .await;
    busy.store(false, Ordering::SeqCst);
    let plan = result.map_err(|e| e.to_string())??;
    *state.master_plan.lock().map_err(|e| e.to_string())? = Some(Arc::new(plan.clone()));
    Ok(plan)
}

#[tauri::command]
async fn write_master(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<usize>,
    out_dir: String,
) -> Result<master::MasterSummary, String> {
    let plan = state
        .master_plan
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or(t(Msg::AnalyzeFilesFirst))?;
    if ids.is_empty() {
        return Err(t(Msg::NoFilesSelected).into());
    }
    if out_dir.trim().is_empty() {
        return Err(t(Msg::NoOutDir).into());
    }
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err(t(Msg::Busy).into());
    }
    let _beat = Heartbeat::start(&app, "master-write");
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();
    let busy = state.busy.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        master::write(&plan, &ids, Path::new(&out_dir), &cancel, |p| {
            let _ = app.emit("master-write-progress", p);
        })
    })
    .await;
    busy.store(false, Ordering::SeqCst);
    result.map_err(|e| e.to_string())?
}

/// Language of the backend's messages ("de", "en", "fr", "it"); the interface
/// calls this at startup and on every switch.
#[tauri::command]
fn set_language(lang: String) {
    i18n::set(&lang);
}

#[tauri::command]
fn cancel_merge(state: State<'_, AppState>) {
    state.cancel.store(true, Ordering::SeqCst);
}

/// Opens a folder, or with `select` reveals a file, in Finder / Explorer.
#[tauri::command]
fn reveal(path: String, select: bool) -> Result<(), String> {
    let p = Path::new(&path);
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        if select {
            c.arg("-R");
        }
        c.arg(p);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("explorer");
        if select {
            c.arg(format!("/select,{}", p.display()));
        } else {
            c.arg(p);
        }
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(if select { p.parent().unwrap_or(p) } else { p });
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Opens a web link from the info panel in the default browser (https only).
#[tauri::command]
fn open_link(url: String) -> Result<(), String> {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_whitespace()) {
        return Err(t(Msg::InvalidLink).into());
    }
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(&url).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            scan_paths,
            pick_folder,
            merge_recordings,
            analyze_tracks,
            write_sync,
            analyze_master,
            write_master,
            sync_set_clips,
            sync_reset_clips,
            sync_peaks,
            player_play,
            player_pause,
            player_seek,
            player_solo,
            pick_output_dir,
            cancel_merge,
            set_language,
            reveal,
            open_link
        ])
        .run(tauri::generate_context!())
        .expect("PrepareAudio konnte nicht gestartet werden");
}
