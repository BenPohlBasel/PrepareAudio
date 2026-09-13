pub mod master;
pub mod merge;
pub mod scan;
pub mod sync;
pub mod wav;

#[cfg(test)]
mod testutil;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

#[derive(Default)]
struct AppState {
    scan: Mutex<Option<Arc<scan::Scan>>>,
    sync_plan: Mutex<Option<Arc<sync::SyncPlan>>>,
    master_plan: Mutex<Option<Arc<master::MasterPlan>>>,
    cancel: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
}

#[tauri::command]
async fn scan_paths(state: State<'_, AppState>, paths: Vec<String>) -> Result<scan::Scan, String> {
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
        .ok_or("Bitte zuerst einen Ordner scannen.")?;
    if ids.is_empty() {
        return Err("Keine Aufnahmen ausgewählt.".into());
    }
    if out_dir.trim().is_empty() {
        return Err("Kein Zielordner angegeben.".into());
    }
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("Es läuft bereits ein Vorgang.".into());
    }
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
        return Err("Es läuft bereits ein Vorgang.".into());
    }
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();
    let busy = state.busy.clone();
    let inputs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let result = tauri::async_runtime::spawn_blocking(move || {
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
    *state.sync_plan.lock().map_err(|e| e.to_string())? = Some(Arc::new(plan.clone()));
    Ok(plan)
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
        .ok_or("Bitte zuerst Tracks analysieren.")?;
    if ids.is_empty() {
        return Err("Keine Abschnitte ausgewählt.".into());
    }
    if out_dir.trim().is_empty() {
        return Err("Kein Zielordner angegeben.".into());
    }
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("Es läuft bereits ein Vorgang.".into());
    }
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
        return Err("Es läuft bereits ein Vorgang.".into());
    }
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
        .ok_or("Bitte zuerst Dateien analysieren.")?;
    if ids.is_empty() {
        return Err("Keine Dateien ausgewählt.".into());
    }
    if out_dir.trim().is_empty() {
        return Err("Kein Zielordner angegeben.".into());
    }
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("Es läuft bereits ein Vorgang.".into());
    }
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
        return Err("Ungültiger Link".into());
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
            cancel_merge,
            reveal,
            open_link
        ])
        .run(tauri::generate_context!())
        .expect("PrepareAudio konnte nicht gestartet werden");
}
