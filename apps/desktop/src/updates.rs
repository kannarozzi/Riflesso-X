//! New-version notices: the app asks GitHub at start and once a day (unless
//! the user turned it off) and shows a notice for a newer release that was
//! neither skipped nor postponed. "Update now" downloads the installer,
//! checks it and starts it; the app then quits so it can be replaced.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use riflesso_updates::Release;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::receiver::Receiver;
use crate::settings::Settings;
use crate::state::AppState;

const FIRST_CHECK: Duration = Duration::from_secs(10);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
const REMIND_AFTER: Duration = Duration::from_secs(3 * 24 * 60 * 60);
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// What the notice shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
    pub page: String,
    /// This is the installed copy, which the installer can update; a portable
    /// copy only gets a link to the download page.
    pub installable: bool,
}

/// The newer release found by the last check, if any.
#[derive(Default)]
pub struct Updates(Mutex<Option<Release>>);

/// Where downloaded installers go; emptied at the next start.
fn download_folder() -> PathBuf {
    std::env::temp_dir().join("riflesso-update")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Checks in the background, at start and then once a day.
pub fn spawn_checks(state: Arc<AppState>, updates: Arc<Updates>) {
    let _ = std::fs::remove_dir_all(download_folder());
    std::thread::Builder::new()
        .name("update-checks".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK);
            loop {
                if state.snapshot().settings.check_updates
                    && let Err(e) = check(&state, &updates, false)
                {
                    tracing::info!("cannot check for updates: {e:#}");
                }
                std::thread::sleep(CHECK_EVERY);
            }
        })
        .expect("spawning update-checks thread");
}

/// Asks GitHub for the latest release and shows the notice when it is newer.
/// Unless `asked` (the user pressed "Check now"), skipped and postponed
/// versions stay quiet. Returns whether a newer version exists.
fn check(state: &AppState, updates: &Updates, asked: bool) -> anyhow::Result<bool> {
    let release = riflesso_updates::latest()?;
    let newer = riflesso_updates::is_newer(&release.version, CURRENT);
    let settings = state.snapshot().settings;
    let quiet = !asked && (release.version == settings.skipped_version || now() < settings.remind_after);
    if newer && !quiet {
        let info = UpdateInfo {
            version: release.version.clone(),
            notes: release.notes.clone(),
            page: release.page.clone(),
            installable: crate::setup::is_installed_copy(),
        };
        tracing::info!("Riflesso X {} is available", info.version);
        state.update(|s| s.update = Some(info));
    }
    *updates.0.lock().unwrap() = newer.then_some(release);
    Ok(newer)
}

/// "Check now" in the settings: returns whether a newer version exists.
#[tauri::command]
pub async fn check_for_updates(state: State<'_, Arc<AppState>>, updates: State<'_, Arc<Updates>>) -> Result<bool, String> {
    let (state, updates) = (state.inner().clone(), updates.inner().clone());
    tauri::async_runtime::spawn_blocking(move || check(&state, &updates, true))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

/// "Update now": downloads and checks the installer, starts it and quits.
/// A portable copy opens the release page instead.
#[tauri::command]
pub async fn install_update(app: AppHandle, updates: State<'_, Arc<Updates>>) -> Result<(), String> {
    let release = updates.0.lock().unwrap().clone().ok_or("no update is available")?;
    if !crate::setup::is_installed_copy() {
        return app.opener().open_url(&release.page, None::<&str>).map_err(|e| e.to_string());
    }
    let installer = tauri::async_runtime::spawn_blocking(move || release.download_installer(&download_folder()))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))?;
    std::process::Command::new(&installer).spawn().map_err(|e| format!("starting the installer: {e}"))?;
    app.exit(0);
    Ok(())
}

/// "Remind me later": quiet for a few days.
#[tauri::command]
pub fn remind_update_later(state: State<'_, Arc<AppState>>, receiver: State<'_, Arc<Receiver>>) -> Result<(), String> {
    dismiss(&state, &receiver, |s| s.remind_after = now() + REMIND_AFTER.as_secs())
}

/// "Skip this version": quiet until a later one comes out.
#[tauri::command]
pub fn skip_update(state: State<'_, Arc<AppState>>, receiver: State<'_, Arc<Receiver>>) -> Result<(), String> {
    let Some(version) = state.snapshot().update.map(|u| u.version) else { return Ok(()) };
    dismiss(&state, &receiver, |s| s.skipped_version = version)
}

fn dismiss(state: &AppState, receiver: &Receiver, change: impl FnOnce(&mut Settings)) -> Result<(), String> {
    let mut settings = state.snapshot().settings;
    change(&mut settings);
    settings.save(&receiver.data_dir).map_err(|e| format!("Cannot save the settings: {e}"))?;
    state.update(|s| {
        s.settings = settings;
        s.update = None;
    });
    Ok(())
}
