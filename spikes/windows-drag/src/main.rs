mod actions;
mod app;
mod droptarget;
mod render;
mod selftest;

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_CONTROL, VK_MENU, VK_SHIFT};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Style {
    Classic,
    Glass,
}

pub struct Config {
    pub key: VIRTUAL_KEY,
    pub key_name: &'static str,
    pub shake: bool,
    pub style: Style,
    pub selftest: bool,
}

pub static CONFIG: OnceLock<Config> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();
static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn config() -> &'static Config {
    CONFIG.get().expect("config")
}

pub fn now_ms() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

pub fn log(msg: &str) {
    let line = format!("{:>10.1} ms  {}", now_ms(), msg);
    println!("{line}");
    if let Ok(mut f) = LOG_FILE.lock() {
        if let Some(f) = f.as_mut() {
            let _ = writeln!(f, "{line}");
        }
    }
}

pub fn log_path() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("OrbitSpike").join("log.txt")
}

fn parse_args() -> Config {
    let args: Vec<String> = std::env::args().skip(1).map(|a| a.to_lowercase()).collect();
    let has = |s: &str| args.iter().any(|a| a == s);
    let value = |s: &str| args.iter().position(|a| a == s).and_then(|i| args.get(i + 1)).cloned();
    let (key, key_name) = match value("--taste").as_deref() {
        Some("strg") | Some("ctrl") => (VK_CONTROL, "Strg"),
        Some("shift") | Some("umschalt") => (VK_SHIFT, "Umschalt"),
        _ => (VK_MENU, "Alt"),
    };
    let style = match value("--stil").as_deref() {
        Some("glas") => Style::Glass,
        _ => Style::Classic,
    };
    Config { key, key_name, shake: !has("--ohne-schuetteln"), style, selftest: has("--selbsttest") }
}

fn main() {
    START.get_or_init(Instant::now);
    let path = log_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    *LOG_FILE.lock().unwrap() = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();

    let cfg = parse_args();
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    log(&format!(
        "──── Orbit Machbarkeitstest Windows · Taste {} · Schütteln {} · Stil {:?}{}",
        cfg.key_name,
        if cfg.shake { "an" } else { "aus" },
        cfg.style,
        if cfg.selftest { " · SELBSTTEST" } else { "" }
    ));
    log(&format!("Log-Datei: {}", path.display()));
    let selftest = cfg.selftest;
    let _ = CONFIG.set(cfg);

    if !selftest {
        log("Bereit. Datei im Explorer ziehen, dann Taste drücken oder schütteln. Beenden mit Strg+C.");
    }
    app::run(selftest);
}
