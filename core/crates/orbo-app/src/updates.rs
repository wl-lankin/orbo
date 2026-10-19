//! Automatic updates through the Tauri updater: a check shortly after start and every six hours, a
//! background download, and the install as soon as Orbo is idle. Errors are only logged, except for
//! a check the user asked for, which reports into the settings window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::state::AppState;

const FIRST_CHECK_AFTER: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const IDLE_POLL: Duration = Duration::from_secs(5);
const MAX_ERROR: usize = 160;

/// What the settings window shows. `state` is one of: idle, checking, current, available,
/// downloading, ready (downloaded, waiting for an idle moment), installing, error.
#[derive(Clone, Serialize)]
pub struct Status {
    state: &'static str,
    version: Option<String>,
    message: Option<String>,
}

impl Default for Status {
    fn default() -> Self {
        Status { state: "idle", version: None, message: None }
    }
}

#[derive(Default)]
pub struct Updates {
    status: Mutex<Status>,
    /// The update the last check found.
    found: Mutex<Option<Update>>,
    /// True while a check, download or install runs, so only one of them is active.
    busy: AtomicBool,
}

struct Busy<'a>(&'a AtomicBool);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn short_error(err: &dyn std::fmt::Display) -> String {
    let line = err.to_string();
    let line = line.lines().next().unwrap_or("Unbekannter Fehler").trim().to_string();
    if line.chars().count() <= MAX_ERROR {
        return line;
    }
    let mut out: String = line.chars().take(MAX_ERROR - 1).collect();
    out.push('…');
    out
}

/// Nothing the user would lose by a restart: no ring, notch or dialog open and no job running.
fn idle(ring_open: bool, notch_open: bool, dialog_open: bool, jobs: usize) -> bool {
    !ring_open && !notch_open && !dialog_open && jobs == 0
}

fn is_idle(app: &AppHandle) -> bool {
    let visible = |label: &str| app.get_webview_window(label).and_then(|w| w.is_visible().ok()).unwrap_or(false);
    let jobs = app.state::<AppState>().jobs.lock().unwrap().len();
    idle(
        crate::platform::RING_CENTER.lock().unwrap().is_some(),
        *crate::platform::NOTCH_OPEN.lock().unwrap(),
        visible("dialog") || visible("onboarding"),
        jobs,
    )
}

fn auto_update_on(app: &AppHandle) -> bool {
    app.state::<AppState>().settings.read().unwrap().auto_update
}

fn set_status(app: &AppHandle, state: &'static str, version: Option<String>, message: Option<String>) {
    let status = Status { state, version, message };
    *app.state::<Updates>().status.lock().unwrap() = status.clone();
    let _ = app.emit("update-state", status);
    crate::windows::refresh_tray_menu(app);
}

/// The version to offer in the tray menu: only when it is not installed automatically.
pub fn offered_version(app: &AppHandle) -> Option<String> {
    let status = app.state::<Updates>().status.lock().unwrap().clone();
    (status.state == "available").then_some(status.version).flatten()
}

async fn find(app: &AppHandle) -> Result<Option<Update>, String> {
    let updater = app.updater().map_err(|e| short_error(&e))?;
    updater.check().await.map_err(|e| short_error(&e))
}

fn remember_update(app: &AppHandle, version: Option<String>) {
    let state = app.state::<AppState>();
    let mut s = state.settings.write().unwrap();
    s.last_updated_to = version;
    crate::settings::save(&s);
}

/// Downloads, optionally waits for an idle moment, installs and restarts.
async fn download_and_install(app: &AppHandle, update: Update, wait_for_idle: bool) {
    let version = update.version.clone();
    set_status(app, "downloading", Some(version.clone()), None);
    let bytes = match update.download(|_, _| {}, || {}).await {
        Ok(b) => b,
        Err(e) => {
            let msg = short_error(&e);
            eprintln!("[orbo] Update {version}: Download fehlgeschlagen: {msg}");
            set_status(app, "error", Some(version), Some(msg));
            return;
        }
    };
    if wait_for_idle {
        set_status(app, "ready", Some(version.clone()), None);
        loop {
            tokio::time::sleep(IDLE_POLL).await;
            if !auto_update_on(app) {
                set_status(app, "available", Some(version), None);
                return;
            }
            if is_idle(app) {
                break;
            }
        }
    }
    set_status(app, "installing", Some(version.clone()), None);
    // The Windows installer ends this process, so the version has to be stored before.
    remember_update(app, Some(version.clone()));
    eprintln!("[orbo] Update {version} wird installiert");
    let result = tauri::async_runtime::spawn_blocking(move || update.install(bytes)).await;
    match result {
        Ok(Ok(())) => app.restart(),
        other => {
            let msg = match other {
                Ok(Err(e)) => short_error(&e),
                Err(e) => short_error(&e),
                Ok(Ok(())) => unreachable!(),
            };
            eprintln!("[orbo] Update {version}: Installation fehlgeschlagen: {msg}");
            remember_update(app, None);
            set_status(app, "error", Some(version), Some(msg));
        }
    }
}

/// One check. With auto-update on, a found update is downloaded and installed from here.
async fn check_cycle(app: &AppHandle, manual: bool) {
    let updates = app.state::<Updates>();
    if updates.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    let _busy = Busy(&updates.busy);
    if manual {
        set_status(app, "checking", None, None);
    }
    match find(app).await {
        Err(msg) => {
            eprintln!("[orbo] Update-Prüfung fehlgeschlagen: {msg}");
            if manual {
                set_status(app, "error", None, Some(msg));
            }
        }
        Ok(None) => {
            *updates.found.lock().unwrap() = None;
            set_status(app, "current", None, None);
        }
        Ok(Some(update)) => {
            eprintln!("[orbo] Update {} gefunden", update.version);
            *updates.found.lock().unwrap() = Some(update.clone());
            if auto_update_on(app) {
                download_and_install(app, update, true).await;
            } else {
                let v = update.version.clone();
                set_status(app, "available", Some(v), None);
            }
        }
    }
}

pub fn check_in_background(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { check_cycle(&app, false).await });
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_AFTER).await;
        loop {
            check_cycle(&app, false).await;
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// Installs the found update right away (auto-update off: tray item or button in Über).
pub fn install_now(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let updates = app.state::<Updates>();
        if updates.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let _busy = Busy(&updates.busy);
        let found = updates.found.lock().unwrap().clone();
        let update = match found {
            Some(u) => u,
            None => match find(&app).await {
                Ok(Some(u)) => u,
                Ok(None) => {
                    set_status(&app, "current", None, None);
                    return;
                }
                Err(msg) => {
                    set_status(&app, "error", None, Some(msg));
                    return;
                }
            },
        };
        download_and_install(&app, update, false).await;
    });
}

#[tauri::command]
pub fn update_check(app: AppHandle) {
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move { check_cycle(&app2, true).await });
}

#[tauri::command]
pub fn update_install(app: AppHandle) {
    install_now(&app);
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> Status {
    app.state::<Updates>().status.lock().unwrap().clone()
}

/// After a restart caused by an update: tells the user once, then forgets it.
pub fn announce_if_updated(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(version) = ({
        let mut s = state.settings.write().unwrap();
        let v = s.last_updated_to.take();
        if v.is_some() {
            crate::settings::save(&s);
        }
        v
    }) else {
        return;
    };
    if version != app.package_info().version.to_string() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        // The toast page has to be loaded before it can show the card.
        std::thread::sleep(Duration::from_millis(2500));
        crate::windows::show_toast(&app);
        std::thread::sleep(Duration::from_millis(300));
        let _ = app.emit_to("toast", "info", serde_json::json!({ "title": format!("Orbo wurde auf Version {version} aktualisiert") }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_needs_everything_quiet() {
        assert!(idle(false, false, false, 0));
        assert!(!idle(true, false, false, 0));
        assert!(!idle(false, true, false, 0));
        assert!(!idle(false, false, true, 0));
        assert!(!idle(false, false, false, 1));
    }

    #[test]
    fn errors_are_one_short_line() {
        assert_eq!(short_error(&"a\nb"), "a");
        assert_eq!(short_error(&"x".repeat(500)).chars().count(), MAX_ERROR);
    }
}
