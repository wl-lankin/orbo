//! Read-only access to the settings file of the Orbo app, for the command line tool.

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub fn app_dir() -> PathBuf {
    let base = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    if cfg!(target_os = "macos") {
        base.join("Library/Application Support/Orbo")
    } else {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or(base).join("Orbo")
    }
}

pub fn settings_path() -> PathBuf {
    app_dir().join("settings.json")
}

#[derive(Deserialize)]
#[serde(default)]
struct OllamaSettings {
    enabled: bool,
    url: String,
    model: Option<String>,
    actions: Vec<String>,
    allow_remote: bool,
}

impl Default for OllamaSettings {
    fn default() -> Self {
        OllamaSettings {
            enabled: false,
            url: crate::ai::DEFAULT_URL.into(),
            model: None,
            actions: vec!["rename".into(), "alt_text".into(), "summary".into(), "translate".into()],
            allow_remote: false,
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SettingsFile {
    ollama: OllamaSettings,
    chains: Vec<crate::chains::Chain>,
}

/// Configures chains and AI from the settings file at `path`. Returns false when there is no usable file.
pub fn apply_from(path: &Path) -> bool {
    let Some(parsed) = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<SettingsFile>(&b).ok()) else { return false };
    let o = parsed.ollama;
    crate::ai::configure(o.enabled.then(|| crate::ai::AiConfig { url: o.url, model: o.model, actions: o.actions, allow_remote: o.allow_remote }));
    crate::chains::configure(parsed.chains);
    true
}

/// Same as `apply_from` for the settings file of the installed app.
pub fn apply_app_settings() -> bool {
    apply_from(&settings_path())
}

#[cfg(test)]
mod tests {
    #[test]
    fn missing_file_is_not_an_error() {
        assert!(!super::apply_from(std::path::Path::new("/nonexistent/settings.json")));
    }

    #[test]
    fn reads_ai_and_chains() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, r#"{"style":"glass","ollama":{"enabled":true,"actions":["summary"]},"chains":[{"id":"x","name":"Test","steps":[{"target":"webp"}]}]}"#).unwrap();
        assert!(super::apply_from(&p));
        assert!(crate::ai::action_enabled(Some("summary")));
        assert_eq!(crate::chains::configured().len(), 1);
        crate::ai::configure(None);
        crate::chains::configure(Vec::new());
    }
}
