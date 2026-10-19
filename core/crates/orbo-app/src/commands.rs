use std::path::{Path, PathBuf};

use orbo_core::analyze::Analysis;
use orbo_core::catalog::{kind_id, Catalog, Target};
use orbo_core::format::Kind;
use orbo_core::jobs::{JobSpec, MergeSpec, Options};
use orbo_core::rules::{Rules, Suggestion};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::{AppState, DialogRequest};
use crate::windows;

#[derive(Serialize)]
pub struct FileInfo {
    path: PathBuf,
    name: String,
    format: String,
    kind: String,
    size: u64,
    meta: String,
    width: Option<u32>,
    height: Option<u32>,
    duration_s: Option<f64>,
}

fn file_info(a: &Analysis) -> FileInfo {
    let name = a.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut parts: Vec<String> = Vec::new();
    let (mut width, mut height, mut duration_s) = (None, None, None);
    if let Some(i) = &a.image {
        width = Some(i.width);
        height = Some(i.height);
        parts.push(format!("{} × {}", i.width, i.height));
        if i.animated {
            parts.push("animiert".into());
        } else if i.has_alpha {
            parts.push("Transparenz".into());
        }
        if i.has_gps {
            parts.push("GPS".into());
        }
    }
    if let Some(m) = &a.media {
        duration_s = Some(m.duration_s);
        let secs = m.duration_s.round() as u64;
        parts.push(format!("{}:{:02} min", secs / 60, secs % 60));
        if let Some(v) = &m.video {
            width = width.or(Some(v.width));
            height = height.or(Some(v.height));
            parts.push(format!("{} {}p", v.codec.to_uppercase(), v.height));
        } else if let Some(au) = &m.audio {
            parts.push(format!("{} kHz", au.sample_rate / 1000));
        }
    }
    if let Some(t) = &a.table {
        parts.push(format!("{} Spalten", t.columns));
    }
    if let Some(n) = a.entries {
        parts.push(format!("{n} Einträge"));
    }
    FileInfo {
        path: a.path.clone(),
        name,
        format: a.format.id(),
        kind: kind_id(a.kind).to_string(),
        size: a.size,
        meta: parts.join(" · "),
        width,
        height,
        duration_s,
    }
}

/// Rough size of the result in bytes for the ring's live estimate. Deliberately simple.
fn estimate(t: &Target, a: &Analysis) -> Option<u64> {
    if t.chain || t.merge {
        return None;
    }
    if t.keeps_format() && matches!(a.kind, Kind::Audio | Kind::Video) {
        return match t.param_str("op") {
            Some("trim") => None,
            _ if t.param_f64("channels") == Some(1.0) => Some(a.size / 2),
            _ if t.param_f64("speed").is_some() => t.param_f64("speed").map(|s| (a.size as f64 / s) as u64),
            _ => Some(a.size),
        };
    }
    if t.keeps_format() && a.kind == Kind::Image {
        let size = a.size as f64;
        return match t.param_str("op") {
            Some("compress") => Some((size * if a.format == orbo_core::Format::Png { 0.85 } else { 0.6 }) as u64),
            Some("resize") => t.param_f64("percent").map(|p| (size * (p / 100.0).powi(2)) as u64),
            Some("strip_metadata") | Some("rotate") | Some("flip") | Some("grayscale") => Some(a.size),
            _ => None,
        };
    }
    let q = t.param_f64("quality").unwrap_or(80.0) / 100.0;
    let out_kind = if t.produces_folder() { None } else { t.output_format().map(|f| f.kind()) };
    match (a.kind, out_kind) {
        (Kind::Image, Some(Kind::Image)) | (Kind::Image, Some(Kind::Document)) => {
            let i = a.image.as_ref()?;
            let mut px = i.width as f64 * i.height as f64;
            if let Some(m) = t.param_f64("max_edge") {
                let s = (m / i.width.max(i.height).max(1) as f64).min(1.0);
                px *= s * s;
            }
            if let Some(p) = t.param_f64("percent") {
                px *= (p / 100.0).powi(2);
            }
            let bpp = match t.ext_for(a).as_str() {
                "jpg" | "pdf" => 0.08 + 0.5 * q * q,
                "webp" => 0.05 + 0.35 * q * q,
                "avif" => 0.03 + 0.25 * q * q,
                "heic" => 0.04 + 0.3 * q * q,
                "png" => 1.6,
                "gif" => 0.9,
                "bmp" | "tga" | "ppm" | "qoi" => 3.0,
                "tiff" => 3.0,
                "ico" => 0.05,
                _ => return None,
            };
            Some((px * bpp / 8.0 * 8.0) as u64 / 8 * 8)
        }
        (Kind::Video | Kind::Audio, Some(Kind::Audio)) => {
            let m = a.media.as_ref()?;
            let k = t.param_f64("audio_bitrate_k").unwrap_or(match t.ext_for(a).as_str() {
                "wav" | "aiff" => 1411.0,
                "flac" | "m4a" if t.param_str("audio") == Some("alac") => 900.0,
                "flac" => 800.0,
                _ => 192.0,
            });
            let dur = t.param_f64("clip_s").map(|c| c.min(m.duration_s)).unwrap_or(m.duration_s);
            Some((dur * k * 125.0) as u64)
        }
        (Kind::Video, Some(Kind::Video)) => {
            let m = a.media.as_ref()?;
            if let Some(mb) = t.param_f64("target_mb") {
                return Some(((mb * 1_000_000.0) as u64).min(a.size));
            }
            let ratio = if t.param_str("video") == Some("copy") || t.param_bool("remux_if_possible") {
                1.0
            } else if t.param_str("video") == Some("hevc") {
                0.45
            } else {
                match t.param_f64("max_height") {
                    Some(h) if m.video.as_ref().map(|v| v.height as f64 > h).unwrap_or(false) => (h / m.video.as_ref().unwrap().height as f64).powi(2) * 0.8,
                    _ => 0.7,
                }
            };
            Some((a.size as f64 * ratio) as u64)
        }
        _ => None,
    }
}

trait ParamBool {
    fn param_bool(&self, key: &str) -> bool;
}
impl ParamBool for Target {
    fn param_bool(&self, key: &str) -> bool {
        self.params.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
    }
}

fn target_json(s: &Suggestion, a: &Analysis, files: usize) -> Value {
    let cat = Catalog::current();
    let t = cat.get(&s.id);
    let kind = t
        .map(|t| if t.produces_folder() { "folder".to_string() } else { t.output_format().map(|f| kind_id(f.kind()).to_string()).unwrap_or_default() })
        .unwrap_or_default();
    // The ring's lens shows the sub line, so a merge says how many files become one.
    let sub = if s.merge { format!("{files} Dateien zu einer") } else { s.sub.clone() };
    json!({
        "id": s.id, "title": s.title, "sub": sub, "ext": s.ext, "intent": s.intent, "ai": s.ai, "group": s.group,
        "kind": kind, "chain": s.chain, "merge": s.merge,
        "dialog": t.map(|t| t.dialog).unwrap_or(false),
        "controls": t.map(|t| t.controls.clone()).unwrap_or_default(),
        "presets": t.map(|t| t.presets.iter().map(|p| p.label.clone()).collect::<Vec<_>>()).unwrap_or_default(),
        "size_estimate": t.and_then(|t| estimate(t, a)),
    })
}

pub fn suggest_value(paths: &[PathBuf]) -> Result<Value, String> {
    if paths.is_empty() {
        return Err("Keine Datei ausgewählt".into());
    }
    let analyses: Vec<Analysis> = paths.iter().map(|p| orbo_core::analyze(p)).collect::<anyhow::Result<_>>().map_err(|e| format!("{e:#}"))?;
    let first = &analyses[0];
    for a in &analyses {
        eprintln!("[orbo] suggest {} → {} / {}", a.path.display(), a.format.id(), kind_id(a.kind));
    }
    // With several files only targets that work for all of them stay; merge targets appear from two files on.
    let s = Rules::builtin().suggest_many(&Catalog::current(), &analyses);
    let n = analyses.len();
    let ring: Vec<Value> = s.ring.iter().map(|x| target_json(x, first, n)).collect();
    let more: Vec<Value> = s.more.iter().map(|x| target_json(x, first, n)).collect();
    Ok(json!({
        "files": analyses.iter().map(file_info).collect::<Vec<_>>(),
        "default": s.default,
        "ring": ring,
        "more": more,
    }))
}

#[tauri::command]
pub async fn suggest(paths: Vec<PathBuf>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || suggest_value(&paths)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn convert(app: AppHandle, paths: Vec<PathBuf>, target: String, preset: Option<String>, options: Option<Options>) -> Result<Vec<u64>, String> {
    eprintln!("[orbo] convert {target} {paths:?}");
    let state = app.state::<AppState>();
    let catalog = Catalog::current();
    let t = catalog.get(&target).ok_or_else(|| format!("Unbekanntes Ziel: {target}"))?;
    let mut options = options.unwrap_or_default();
    if options.preset.is_none() {
        options.preset = preset;
    }
    if t.merge && paths.len() < 2 {
        return Err(format!("„{}“ braucht mindestens zwei Dateien", t.title));
    }
    let params = orbo_core::jobs::effective_params(t, &options);
    if params.get("pick_folder").and_then(|v| v.as_bool()).unwrap_or(false) && options.out_dir.is_none() {
        use tauri_plugin_dialog::DialogExt;
        let start = paths.first().and_then(|p| p.parent()).map(|p| p.to_path_buf());
        let mut dlg = app.dialog().file().set_title("Zielordner auswählen");
        if let Some(s) = start {
            dlg = dlg.set_directory(s);
        }
        #[cfg(target_os = "macos")]
        let _ = app.run_on_main_thread(crate::platform::macos::activate_app);
        match tauri::async_runtime::spawn_blocking(move || dlg.blocking_pick_folder()).await.ok().flatten() {
            Some(fp) => options.out_dir = Some(fp.into_path().map_err(|e| e.to_string())?),
            None => return Ok(Vec::new()),
        }
    }
    let settings = state.settings.read().unwrap().clone();
    apply_output_settings(&settings, t, &mut options);
    Ok(submit_jobs(&state, paths, &target, t, &options))
}

/// Output folder and naming from the settings; shared by ring drops and watched folders.
pub fn apply_output_settings(settings: &crate::settings::Settings, t: &Target, options: &mut Options) {
    if settings.output.mode == "folder" {
        if let Some(dir) = settings.output.folder.clone() {
            options.out_dir = Some(dir);
        }
    }
    if settings.output.naming == "suffix" && t.suffix.is_empty() && options.name_suffix.is_none() {
        options.name_suffix = Some(format!(" ({})", t.title));
    }
}

pub fn submit_jobs(state: &AppState, paths: Vec<PathBuf>, target: &str, t: &Target, options: &Options) -> Vec<u64> {
    if t.merge {
        let bytes = paths.iter().map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
        let id = state.queue.submit_merge(MergeSpec { inputs: paths, target: target.to_string(), options: options.clone() });
        state.jobs.lock().unwrap().insert(id, (t.title.clone(), bytes));
        return vec![id];
    }
    let mut ids = Vec::new();
    for p in paths {
        let input_bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        let id = state.queue.submit(JobSpec { input: p.clone(), target: target.to_string(), options: options.clone() });
        state.jobs.lock().unwrap().insert(id, (t.title.clone(), input_bytes));
        ids.push(id);
    }
    ids
}

#[tauri::command]
pub fn cancel_job(state: State<AppState>, id: u64) {
    state.queue.cancel(id);
}

#[tauri::command]
pub fn ring_hide(app: AppHandle) {
    windows::hide_ring(&app, 0);
}

#[tauri::command]
pub fn open_dialog(app: AppHandle, state: State<AppState>, paths: Vec<PathBuf>, target: String, preset: Option<String>) -> Result<(), String> {
    *state.dialog.lock().unwrap() = Some(DialogRequest { paths, target, preset });
    windows::hide_ring(&app, 0);
    windows::open_dialog(&app);
    Ok(())
}

#[tauri::command]
pub async fn dialog_context(app: AppHandle) -> Result<Value, String> {
    let req = app.state::<AppState>().dialog.lock().unwrap().clone().ok_or("Kein Dialog offen")?;
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = Catalog::current();
        let t = catalog.get(&req.target).ok_or("Unbekanntes Ziel")?;
        let first = req.paths.first().ok_or("Keine Datei ausgewählt")?;
        let a = orbo_core::analyze(first).map_err(|e| format!("{e:#}"))?;
        let preview = preview_path(&a);
        Ok(json!({
            "paths": req.paths,
            "target": { "id": t.id, "title": t.title, "sub": t.sub, "controls": t.controls,
                        "presets": t.presets.iter().map(|p| p.label.clone()).collect::<Vec<_>>(), "params": t.params },
            "file": file_info(&a),
            "preview": preview,
            "preset": req.preset,
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Path the dialog can show directly. Formats WebKit cannot display get a temporary PNG.
fn preview_path(a: &Analysis) -> Option<PathBuf> {
    use orbo_core::format::Format;
    match a.kind {
        Kind::Video | Kind::Audio => Some(a.path.clone()),
        Kind::Image => match a.format {
            Format::Png | Format::Jpeg | Format::Gif | Format::Webp | Format::Bmp | Format::Svg | Format::Tiff | Format::Heic | Format::Avif
                if cfg!(target_os = "macos") =>
            {
                Some(a.path.clone())
            }
            Format::Png | Format::Jpeg | Format::Gif | Format::Webp | Format::Bmp | Format::Svg => Some(a.path.clone()),
            _ => {
                let dir = std::env::temp_dir().join("orbo-preview");
                let _ = std::fs::create_dir_all(&dir);
                let spec = JobSpec {
                    input: a.path.clone(),
                    target: "png".into(),
                    options: Options { out_dir: Some(dir), max_edge: Some(2400), ..Default::default() },
                };
                orbo_core::jobs::convert_blocking(&spec, 0, &std::sync::atomic::AtomicBool::new(false), &|_| {}).ok()
            }
        },
        _ => None,
    }
}

#[tauri::command]
pub fn dialog_close(app: AppHandle) {
    if let Some(w) = app.get_webview_window("dialog") {
        let _ = w.close();
    }
}

#[tauri::command]
pub fn reveal(path: PathBuf) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("/usr/bin/open").arg("-R").arg(&path).spawn();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn();
    }
}

#[tauri::command]
pub fn undo(path: PathBuf) -> Result<(), String> {
    if !Path::new(&path).exists() {
        return Err("Die Datei ist nicht mehr vorhanden".into());
    }
    trash::delete(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn toast_hide(app: AppHandle) {
    windows::hide_toast(&app);
}

#[tauri::command]
pub fn settings_get(state: State<AppState>) -> crate::settings::Settings {
    state.settings.read().unwrap().clone()
}

#[tauri::command]
pub fn settings_set(app: AppHandle, state: State<AppState>, settings: crate::settings::Settings) {
    let (style_changed, auto_was) = {
        let cur = state.settings.read().unwrap();
        (cur.style != settings.style, cur.auto_update)
    };
    let mut settings = settings;
    // Owned by the updater; a stale copy from a window must not overwrite it.
    settings.last_updated_to = state.settings.read().unwrap().last_updated_to.clone();
    *state.settings.write().unwrap() = settings.clone();
    crate::settings::save(&settings);
    crate::settings::apply_ai(&settings);
    crate::settings::apply_chains(&settings);
    crate::watch::rebuild(&app);
    let _ = app.emit("settings", &settings);
    if style_changed {
        windows::apply_ring_style(&app, &settings.style);
    }
    windows::update_tray(&app);
    if settings.auto_update && !auto_was {
        crate::updates::check_in_background(&app);
    }
}

#[tauri::command]
pub fn open_settings(app: AppHandle) {
    windows::open_settings(&app);
}

#[tauri::command]
pub fn catalog() -> Value {
    json!(Catalog::builtin()
        .targets
        .iter()
        .map(|t| json!({ "id": t.id, "title": t.title, "sub": t.sub, "group": t.group, "controls": t.controls, "dialog": t.dialog,
                          "presets": t.presets.iter().map(|p| p.label.clone()).collect::<Vec<_>>() }))
        .collect::<Vec<_>>())
}

#[tauri::command]
pub fn debug_log(msg: String) {
    eprintln!("[ui] {msg}");
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Nur Webadressen (http und https) sind zulässig".into());
    }
    #[cfg(target_os = "macos")]
    std::process::Command::new("/usr/bin/open").arg(&url).spawn().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    std::process::Command::new("explorer").arg(&url).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn request_folder_access() -> Value {
    tauri::async_runtime::spawn_blocking(|| {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default();
        // Reading a protected folder makes macOS show its permission prompt (and waits for the answer).
        let check = |name: &str| std::fs::read_dir(home.join(name)).map(|mut d| d.next().is_some() || true).unwrap_or(false);
        json!({ "desktop": check("Desktop"), "documents": check("Documents"), "downloads": check("Downloads") })
    })
    .await
    .unwrap_or_else(|_| json!({}))
}

#[tauri::command]
pub fn open_privacy_settings() {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders")
        .spawn();
}

#[tauri::command]
pub fn onboarding_sample() -> Result<PathBuf, String> {
    let dir = crate::settings::app_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("Orbo Beispiel.png");
    std::fs::write(&path, include_bytes!("../icons/icon.png")).map_err(|e| e.to_string())?;
    reveal(path.clone());
    Ok(path)
}

#[tauri::command]
pub fn onboarding_done(app: AppHandle, state: State<AppState>) {
    let settings = {
        let mut s = state.settings.write().unwrap();
        s.onboarded = true;
        s.clone()
    };
    crate::settings::save(&settings);
    if let Some(w) = app.get_webview_window("onboarding") {
        let _ = w.close();
    }
}

#[tauri::command]
pub fn open_onboarding(app: AppHandle) {
    windows::open_onboarding(&app);
}

#[tauri::command]
pub fn notch_hide(app: AppHandle) {
    windows::hide_notch(&app, 0);
}


/// Optional donation page; Orbo itself is free.
pub const SUPPORT_URL: &str = "https://wolfgang-linz.de/orbo";

#[tauri::command]
pub fn support() -> Result<(), String> {
    open_url(SUPPORT_URL.to_string())
}

/// Targets a watched folder can use: no AI, no merge (a watcher sees one file at a time) and nothing that needs a per-file folder prompt.
#[tauri::command]
pub fn watch_targets() -> Value {
    let picks = |p: &serde_json::Value| p.get("pick_folder").and_then(|v| v.as_bool()).unwrap_or(false);
    json!(Catalog::current()
        .targets
        .iter()
        .filter(|t| !t.ai && !t.merge && !picks(&t.params))
        .map(|t| json!({ "id": t.id, "title": t.title, "sub": t.sub, "group": t.group, "chain": t.chain,
                          "presets": t.presets.iter().filter(|p| !picks(&p.params)).map(|p| p.label.clone()).collect::<Vec<_>>() }))
        .collect::<Vec<_>>())
}

/// Per rule: does the folder exist and can Orbo read it (macOS asks for protected folders on first read).
#[tauri::command]
pub async fn watch_status(app: AppHandle) -> Value {
    let rules = app.state::<AppState>().settings.read().unwrap().watch.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default();
        let protected: Vec<PathBuf> = ["Desktop", "Documents", "Downloads"].iter().map(|n| home.join(n)).collect();
        json!(rules
            .iter()
            .map(|r| json!({
                "id": r.id,
                "exists": r.folder.is_dir(),
                "readable": std::fs::read_dir(&r.folder).is_ok(),
                "protected": cfg!(target_os = "macos") && protected.iter().any(|p| r.folder.starts_with(p)),
            }))
            .collect::<Vec<_>>())
    })
    .await
    .unwrap_or_else(|_| json!([]))
}

#[tauri::command]
pub fn context_menu_status() -> crate::contextmenu::Status {
    crate::contextmenu::status()
}

#[tauri::command]
pub fn context_menu_install() -> Result<crate::contextmenu::Status, String> {
    crate::contextmenu::install()?;
    Ok(crate::contextmenu::status())
}

#[tauri::command]
pub fn context_menu_remove() -> Result<crate::contextmenu::Status, String> {
    crate::contextmenu::remove()?;
    Ok(crate::contextmenu::status())
}

/// Runs blocking setup work (login shell, child processes, file edits) off the main thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

/// `id` is one of "cli", "claude_code", "claude_desktop", "codex".
#[tauri::command]
pub async fn integration_status(id: String) -> Result<crate::cli_setup::Status, String> {
    blocking(move || Ok(crate::cli_setup::status(&id))).await
}

#[tauri::command]
pub async fn integration_install(id: String) -> Result<crate::cli_setup::Status, String> {
    blocking(move || crate::cli_setup::install(&id)).await
}

#[tauri::command]
pub async fn integration_remove(id: String) -> Result<crate::cli_setup::Status, String> {
    blocking(move || crate::cli_setup::remove(&id)).await
}

fn step_target_json(t: &Target) -> Value {
    let picks = |p: &serde_json::Value| p.get("pick_folder").and_then(|v| v.as_bool()).unwrap_or(false);
    json!({ "id": t.id, "title": t.title, "sub": t.sub, "group": t.group, "ai": t.ai,
            "kind": if t.produces_folder() { "folder".to_string() } else { t.output_format().map(|f| kind_id(f.kind()).to_string()).unwrap_or_default() },
            "presets": t.presets.iter().filter(|p| !picks(&p.params)).map(|p| p.label.clone()).collect::<Vec<_>>() })
}

/// Checks a chain in the builder: structure, folder only last, formats between steps.
#[tauri::command]
pub fn chain_validate(steps: Vec<orbo_core::chains::ChainStep>) -> Value {
    json!(orbo_core::chains::validate(&steps))
}

/// Targets that can be added after `steps`; with no steps, everything that can start a chain.
#[tauri::command]
pub fn chain_candidates(steps: Vec<orbo_core::chains::ChainStep>) -> Value {
    json!(orbo_core::chains::candidates(&steps).into_iter().map(step_target_json).collect::<Vec<_>>())
}

/// All built-in targets with title, for showing step names of saved chains.
#[tauri::command]
pub fn chain_step_info() -> Value {
    json!(Catalog::builtin().targets.iter().map(step_target_json).collect::<Vec<_>>())
}
