use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Output {
    /// "beside" = next to the original, "folder" = always into `folder`.
    pub mode: String,
    pub folder: Option<PathBuf>,
    /// "plain" = Name.jpg, "suffix" = Name (JPG).jpg
    pub naming: String,
}

impl Default for Output {
    fn default() -> Self {
        Output { mode: "beside".into(), folder: None, naming: "plain".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Ollama {
    pub enabled: bool,
    pub url: String,
    pub model: Option<String>,
    pub actions: Vec<String>,
    /// Allow a server on another machine; file contents then leave the device.
    pub allow_remote: bool,
}

impl Default for Ollama {
    fn default() -> Self {
        Ollama {
            enabled: false,
            url: "http://localhost:11434".into(),
            model: None,
            actions: vec!["rename".into(), "alt_text".into(), "summary".into(), "translate".into()],
            allow_remote: false,
        }
    }
}

/// Hands the AI settings to the core, which gates the AI targets on them.
pub fn apply_ai(s: &Settings) {
    let o = &s.ollama;
    orbo_core::ai::configure(o.enabled.then(|| orbo_core::ai::AiConfig {
        url: o.url.clone(),
        model: o.model.clone(),
        actions: o.actions.clone(),
        allow_remote: o.allow_remote,
    }));
}

/// Hands the user's chains to the core, which adds them to the catalog.
pub fn apply_chains(s: &Settings) {
    orbo_core::chains::configure(s.chains.clone());
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub style: String,
    pub trigger_key: String,
    pub shake: bool,
    pub notch: bool,
    pub sound: bool,
    pub reduce_motion: bool,
    pub paused: bool,
    pub autostart: bool,
    pub onboarded: bool,
    /// Download and install new versions without asking.
    pub auto_update: bool,
    /// Set just before the restart of an update; shows the "updated" toast once, then is cleared.
    pub last_updated_to: Option<String>,
    pub output: Output,
    pub ollama: Ollama,
    /// Watched folders ("Überwachte Ordner").
    pub watch: Vec<orbo_core::watch_rules::WatchRule>,
    /// User-defined chained conversions ("Abläufe").
    pub chains: Vec<orbo_core::chains::Chain>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            style: "classic".into(),
            trigger_key: "option".into(),
            shake: true,
            notch: false,
            sound: false,
            reduce_motion: false,
            paused: false,
            autostart: true,
            onboarded: false,
            auto_update: true,
            last_updated_to: None,
            output: Output::default(),
            ollama: Ollama::default(),
            watch: Vec::new(),
            chains: Vec::new(),
        }
    }
}

fn dir_for(name: &str) -> PathBuf {
    let base = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if cfg!(target_os = "macos") {
        base.join("Library/Application Support").join(name)
    } else {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or(base).join(name)
    }
}

pub fn app_dir() -> PathBuf {
    orbo_core::appconfig::app_dir()
}

fn path() -> PathBuf {
    app_dir().join("settings.json")
}

pub fn load() -> Settings {
    // Earlier development names; take over their settings once.
    for name in ["Rondo", "Orbit"] {
        let old = dir_for(name).join("settings.json");
        if !path().exists() && old.exists() {
            let _ = std::fs::create_dir_all(dir_for("Orbo"));
            let _ = std::fs::copy(&old, path());
        }
    }
    std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(s: &Settings) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(s) {
        let _ = std::fs::write(p, json);
    }
}
