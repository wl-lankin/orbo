#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod contextmenu;
mod invoke;
mod platform;
mod settings;
mod state;
mod updates;
mod watch;
mod windows;

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

use orbo_core::jobs::{JobEvent, JobQueue};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

fn forward_job_events(app: AppHandle, events: crossbeam_channel::Receiver<JobEvent>) {
    std::thread::spawn(move || {
        let mut running: HashMap<u64, f32> = HashMap::new();
        for ev in events {
            let state = app.state::<AppState>();
            let mut payload = serde_json::to_value(&ev).unwrap_or_default();
            let id = payload["id"].as_u64().unwrap_or(0);
            let (title, input_bytes) = state.jobs.lock().unwrap().get(&id).cloned().unwrap_or_default();
            payload["title"] = title.into();
            payload["input_bytes"] = input_bytes.into();
            match &ev {
                JobEvent::Started { .. } => {
                    running.insert(id, 0.0);
                    windows::show_toast(&app);
                }
                JobEvent::Progress { fraction, .. } => {
                    running.insert(id, *fraction);
                }
                _ => {
                    if let JobEvent::Finished { output, .. } = &ev {
                        watch::note_output(output);
                    }
                    running.remove(&id);
                    state.jobs.lock().unwrap().remove(&id);
                }
            }
            let tray = if running.is_empty() {
                None
            } else {
                let avg = running.values().sum::<f32>() / running.len() as f32;
                Some(format!("{:.0} %", avg * 100.0))
            };
            windows::set_tray_progress(&app, tray);
            let _ = app.emit("job", payload);
        }
    });
}

#[cfg(target_os = "macos")]
fn start_watcher(app: AppHandle) {
    use crate::platform::{macos, DragSignal};
    let app_for_settings = app.clone();
    let settings = move || {
        let s = app_for_settings.state::<AppState>().settings.read().unwrap().clone();
        macos::WatchSettings { key: s.trigger_key, shake: s.shake, paused: s.paused, notch: s.notch }
    };
    // Screen layout can change (displays plugged in); the watcher thread reads a cached copy.
    let app_for_screens = app.clone();
    std::thread::spawn(move || loop {
        let _ = app_for_screens.run_on_main_thread(macos::refresh_screens);
        std::thread::sleep(std::time::Duration::from_secs(4));
    });
    macos::watch(settings, move |sig| match sig {
        DragSignal::Triggered { at, files, trigger } => {
            let started = std::time::Instant::now();
            eprintln!("[orbo] Auslöser {trigger:?}, Dateien: {files:?}");
            let app2 = app.clone();
            let _ = app.run_on_main_thread(move || windows::open_ring(&app2, at, files, started));
        }
        DragSignal::Cancelled => windows::close_ring(&app, "cancelled"),
        DragSignal::Released => {
            windows::close_ring(&app, "released");
            windows::close_notch(&app, "released");
        }
        DragSignal::NotchEnter { at, files } => {
            let app2 = app.clone();
            let _ = app.run_on_main_thread(move || windows::open_notch(&app2, at, files));
        }
        DragSignal::NotchLeave => windows::close_notch(&app, "cancelled"),
    });
}

#[cfg(target_os = "windows")]
fn start_watcher(app: AppHandle) {
    use crate::platform::{windows as win, DragSignal};
    let app_for_settings = app.clone();
    let settings = move || {
        let s = app_for_settings.state::<AppState>().settings.read().unwrap().clone();
        win::WatchSettings { key: s.trigger_key, shake: s.shake, paused: s.paused }
    };
    let app_for_main = app.clone();
    let main = move |f: Box<dyn FnOnce() + Send>| {
        let _ = app_for_main.run_on_main_thread(f);
    };
    win::watch(settings, main, move |sig| match sig {
        DragSignal::Triggered { at, files, trigger } => {
            let started = std::time::Instant::now();
            eprintln!("[orbo] Auslöser {trigger:?}, Dateien: {files:?}");
            let app2 = app.clone();
            let _ = app.run_on_main_thread(move || windows::open_ring(&app2, at, files, started));
        }
        DragSignal::Cancelled => windows::close_ring(&app, "cancelled"),
        DragSignal::Released => windows::close_ring(&app, "released"),
        DragSignal::NotchEnter { .. } | DragSignal::NotchLeave => {}
    });
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn start_watcher(_app: AppHandle) {}

fn main() {
    #[cfg(target_os = "windows")]
    platform::windows::init_logging(settings::app_dir());
    let queue = JobQueue::new(2);
    let events = queue.events().clone();
    let loaded = settings::load();
    settings::apply_ai(&loaded);
    settings::apply_chains(&loaded);
    let state = AppState {
        settings: RwLock::new(loaded),
        queue,
        jobs: Mutex::new(HashMap::new()),
        dialog: Mutex::new(None),
    };

    tauri::Builder::default()
        // Must be the first plugin. A second launch (Explorer verb, orbo:// link) is forwarded here.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let paths = invoke::parse_args(&argv);
            if paths.is_empty() {
                if !argv.iter().any(|a| a.starts_with("orbo://")) {
                    windows::open_settings(app);
                }
            } else {
                invoke::submit(app, paths);
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .manage(state)
        .manage(updates::Updates::default())
        .invoke_handler(tauri::generate_handler![
            commands::suggest,
            commands::convert,
            commands::cancel_job,
            commands::ring_hide,
            commands::open_dialog,
            commands::dialog_context,
            commands::dialog_close,
            commands::reveal,
            commands::undo,
            commands::toast_hide,
            commands::settings_get,
            commands::settings_set,
            commands::open_settings,
            commands::catalog,
            commands::debug_log,
            commands::open_url,
            commands::request_folder_access,
            commands::open_privacy_settings,
            commands::onboarding_sample,
            commands::onboarding_done,
            commands::open_onboarding,
            commands::notch_hide,
            commands::support,
            commands::context_menu_status,
            commands::context_menu_install,
            commands::context_menu_remove,
            commands::watch_targets,
            commands::watch_status,
            commands::chain_validate,
            commands::chain_candidates,
            commands::chain_step_info,
            updates::update_check,
            updates::update_install,
            updates::update_status,
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let handle = app.handle().clone();
            windows::create_overlays(&handle)?;
            #[cfg(target_os = "windows")]
            platform::windows::init_main_thread();
            windows::create_tray(&handle)?;
            forward_job_events(handle.clone(), events);
            start_watcher(handle.clone());
            watch::rebuild(&handle);
            let args: Vec<String> = std::env::args().collect();
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let h = handle.clone();
                app.deep_link().on_open_url(move |ev| {
                    for u in ev.urls() {
                        invoke::submit(&h, invoke::parse_url(u.as_str()));
                    }
                });
                // On Windows the plugin routes a link in the first launch's arguments through the handler above.
                invoke::submit(&handle, invoke::parse_args(&args));
            }
            windows::sync_autostart(&handle);
            updates::announce_if_updated(&handle);
            updates::spawn(handle.clone());
            if !handle.state::<AppState>().settings.read().unwrap().onboarded && !args.iter().any(|a| a == "--dialog") {
                windows::open_onboarding(&handle);
            }
            if args.iter().any(|a| a == "--onboarding") {
                windows::open_onboarding(&handle);
            }
            if args.iter().any(|a| a == "--settings") {
                windows::open_settings(&handle);
            }
            // Developer shortcut: --dialog <target> <file> opens the options dialog directly.
            if let Some(i) = args.iter().position(|a| a == "--dialog") {
                if let (Some(target), Some(file)) = (args.get(i + 1), args.get(i + 2)) {
                    *handle.state::<AppState>().dialog.lock().unwrap() =
                        Some(state::DialogRequest { paths: vec![file.into()], target: target.clone(), preset: None });
                    windows::open_dialog(&handle);
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Orbo konnte nicht starten")
        .run(|_app, event| {
            // Closing the settings or dialog window must not quit the menu bar app.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
