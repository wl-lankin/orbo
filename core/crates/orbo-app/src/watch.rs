//! Watched folders: turns new files in configured folders into conversion jobs.
//! All decisions live in `orbo_core::watch_rules`; this file wires them to `notify` and the job queue.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use orbo_core::catalog::Catalog;
use orbo_core::jobs::Options;
use orbo_core::watch_rules::{fs_len, Io, WatchMatcher};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

const WORKERS: usize = 4;

struct Active {
    matcher: Arc<WatchMatcher>,
    signature: String,
    // Dropping the watchers closes the channel, which ends the worker threads.
    _watchers: Vec<RecommendedWatcher>,
}

fn active() -> &'static Mutex<Option<Active>> {
    static A: OnceLock<Mutex<Option<Active>>> = OnceLock::new();
    A.get_or_init(|| Mutex::new(None))
}

/// Called for every finished job so Orbo never converts its own results.
pub fn note_output(path: &std::path::Path) {
    if let Some(a) = active().lock().unwrap().as_ref() {
        a.matcher.note_output(path);
    }
}

/// (Re)creates the watchers from the current settings. Cheap when nothing relevant changed.
pub fn rebuild(app: &AppHandle) {
    let settings = app.state::<AppState>().settings.read().unwrap().clone();
    let rules: Vec<_> = settings.watch.iter().filter(|r| r.enabled && !r.folder.as_os_str().is_empty()).cloned().collect();
    let out_folder = (settings.output.mode == "folder").then(|| settings.output.folder.clone()).flatten();
    let title_suffix = settings.output.naming == "suffix";
    let signature = format!("{rules:?}|{out_folder:?}|{title_suffix}");

    let mut slot = active().lock().unwrap();
    if slot.as_ref().is_some_and(|a| a.signature == signature) {
        return;
    }
    let old = slot.take();
    if rules.is_empty() {
        return;
    }

    let matcher = Arc::new(WatchMatcher::new(&rules, out_folder, title_suffix));
    if let Some(o) = &old {
        matcher.inherit(&o.matcher);
    }
    drop(old);

    let (tx, rx) = crossbeam_channel::unbounded::<PathBuf>();
    let mut watchers = Vec::new();
    for r in &rules {
        let tx = tx.clone();
        let handler = move |res: notify::Result<Event>| {
            let Ok(ev) = res else { return };
            // Data changes are covered by the size check; only arrivals start a conversion.
            if matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))) {
                for p in ev.paths {
                    let _ = tx.send(p);
                }
            }
        };
        match notify::recommended_watcher(handler) {
            Ok(mut w) => {
                let mode = if r.include_subfolders { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
                match w.watch(&r.folder, mode) {
                    Ok(()) => watchers.push(w),
                    Err(e) => eprintln!("[orbo] Ordner {} nicht überwachbar: {e}", r.folder.display()),
                }
            }
            Err(e) => eprintln!("[orbo] Überwachung nicht verfügbar: {e}"),
        }
    }
    drop(tx);

    for _ in 0..WORKERS {
        let rx = rx.clone();
        let matcher = matcher.clone();
        let app = app.clone();
        std::thread::spawn(move || {
            for path in rx {
                handle(&app, &matcher, &path);
            }
        });
    }
    *slot = Some(Active { matcher, signature, _watchers: watchers });
}

fn handle(app: &AppHandle, matcher: &WatchMatcher, path: &std::path::Path) {
    let state = app.state::<AppState>();
    let paused = || state.settings.read().unwrap().paused;
    let io = Io { len: &fs_len, sleep: &|d| std::thread::sleep(d), paused: &paused };
    let Some(job) = matcher.process(path, &io) else { return };
    let catalog = Catalog::current();
    let Some(t) = catalog.get(&job.target) else { return };
    let settings = state.settings.read().unwrap().clone();
    let mut options = Options { preset: job.preset.clone(), ..Default::default() };
    crate::commands::apply_output_settings(&settings, t, &mut options);
    eprintln!("[orbo] Ordnerüberwachung: {} → {}", job.input.display(), job.target);
    crate::commands::submit_jobs(&state, vec![job.input], &job.target, t, &options);
}
