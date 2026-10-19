//! Registers the Orbo MCP server in the config files of Claude Desktop and Codex.
//! Only the `orbo` entry is touched; everything else in the files stays as it was.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

pub const SERVER_NAME: &str = "orbo";

fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default()
}

pub fn claude_desktop_config_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/Claude/claude_desktop_config.json")
    } else {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(home).join("Claude/claude_desktop_config.json")
    }
}

pub fn codex_config_path() -> PathBuf {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".codex")).join("config.toml")
}

pub fn claude_code_config_path() -> PathBuf {
    home().join(".claude.json")
}

fn backup(path: &Path) -> Result<()> {
    if path.exists() {
        let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(".bak");
        std::fs::copy(path, path.with_file_name(name)).with_context(|| format!("Sicherung von {} fehlgeschlagen", path.display()))?;
    }
    Ok(())
}

fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let dir = path.parent().ok_or_else(|| anyhow!("Ungültiger Pfad"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".orbo-{}.tmp", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

fn read_text(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("{} ist nicht lesbar", path.display())),
    }
}

fn parse_json_object(path: &Path, text: Option<&str>) -> Result<Value> {
    match text {
        None => Ok(json!({})),
        Some(t) if t.trim().is_empty() => Ok(json!({})),
        Some(t) => {
            let v: Value = serde_json::from_str(t).with_context(|| format!("{} ist kein gültiges JSON. Orbo ändert die Datei nicht.", path.display()))?;
            if !v.is_object() {
                bail!("{} hat nicht die erwartete Form. Orbo ändert die Datei nicht.", path.display());
            }
            Ok(v)
        }
    }
}

/// Adds or replaces `mcpServers.orbo` in a Claude Desktop config. Creates the file if needed, keeps a `.bak` of the old one.
pub fn claude_desktop_install(path: &Path, exe: &Path) -> Result<()> {
    let text = read_text(path)?;
    let mut doc = parse_json_object(path, text.as_deref())?;
    let root = doc.as_object_mut().unwrap();
    let servers = root.entry("mcpServers").or_insert_with(|| json!({}));
    let Some(servers) = servers.as_object_mut() else { bail!("„mcpServers“ in {} ist kein Objekt. Orbo ändert die Datei nicht.", path.display()) };
    servers.insert(SERVER_NAME.into(), json!({ "command": exe.to_string_lossy(), "args": ["mcp"] }));
    backup(path)?;
    write_atomic(path, &(serde_json::to_string_pretty(&doc)? + "\n"))
}

pub fn claude_desktop_remove(path: &Path) -> Result<()> {
    let Some(text) = read_text(path)? else { return Ok(()) };
    let mut doc = parse_json_object(path, Some(&text))?;
    let removed = doc.get_mut("mcpServers").and_then(|s| s.as_object_mut()).and_then(|s| s.shift_remove(SERVER_NAME)).is_some();
    if removed {
        backup(path)?;
        write_atomic(path, &(serde_json::to_string_pretty(&doc)? + "\n"))?;
    }
    Ok(())
}

/// The registered command of the `orbo` entry in a JSON config with `mcpServers` (Claude Desktop, Claude Code's ~/.claude.json).
pub fn json_registered_command(path: &Path) -> Option<String> {
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    doc.get("mcpServers")?.get(SERVER_NAME)?.get("command")?.as_str().map(String::from)
}

fn parse_toml(path: &Path, text: &str) -> Result<toml_edit::DocumentMut> {
    text.parse().map_err(|e| anyhow!("{} ist kein gültiges TOML ({e}). Orbo ändert die Datei nicht.", path.display()))
}

/// Adds or replaces the `[mcp_servers.orbo]` table in a Codex config, keeping comments and other tables.
pub fn codex_install(path: &Path, exe: &Path) -> Result<()> {
    let text = read_text(path)?.unwrap_or_default();
    let mut doc = parse_toml(path, &text)?;
    let servers = doc.entry("mcp_servers").or_insert_with(|| {
        let mut t = toml_edit::Table::new();
        t.set_implicit(true);
        toml_edit::Item::Table(t)
    });
    let Some(servers) = servers.as_table_mut() else { bail!("„mcp_servers“ in {} hat nicht die erwartete Form. Orbo ändert die Datei nicht.", path.display()) };
    let mut entry = toml_edit::Table::new();
    entry["command"] = toml_edit::value(exe.to_string_lossy().as_ref());
    let mut args = toml_edit::Array::new();
    args.push("mcp");
    entry["args"] = toml_edit::value(args);
    servers.insert(SERVER_NAME, toml_edit::Item::Table(entry));
    backup(path)?;
    write_atomic(path, &doc.to_string())
}

pub fn codex_remove(path: &Path) -> Result<()> {
    let Some(text) = read_text(path)? else { return Ok(()) };
    let mut doc = parse_toml(path, &text)?;
    let removed = doc.get_mut("mcp_servers").and_then(|s| s.as_table_mut()).and_then(|s| s.remove(SERVER_NAME)).is_some();
    if removed {
        backup(path)?;
        write_atomic(path, &doc.to_string())?;
    }
    Ok(())
}

pub fn codex_registered_command(path: &Path) -> Option<String> {
    let doc = parse_toml(path, &std::fs::read_to_string(path).ok()?).ok()?;
    doc.get("mcp_servers")?.get(SERVER_NAME)?.get("command")?.as_str().map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_desktop_keeps_other_servers_and_backs_up() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Claude/claude_desktop_config.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"theme":"dark","mcpServers":{"other":{"command":"x","args":["a"]}}}"#).unwrap();
        claude_desktop_install(&p, Path::new("/Applications/Orbo.app/Contents/MacOS/orbo-cli")).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"]["orbo"]["args"], json!(["mcp"]));
        assert!(std::fs::read_to_string(dir.path().join("Claude/claude_desktop_config.json.bak")).unwrap().contains("\"other\""));
        assert_eq!(json_registered_command(&p).as_deref(), Some("/Applications/Orbo.app/Contents/MacOS/orbo-cli"));
        claude_desktop_install(&p, Path::new("/new/orbo")).unwrap();
        assert_eq!(json_registered_command(&p).as_deref(), Some("/new/orbo"));
        claude_desktop_remove(&p).unwrap();
        assert!(json_registered_command(&p).is_none());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
    }

    #[test]
    fn claude_desktop_creates_missing_file_and_refuses_broken_json() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("new/claude_desktop_config.json");
        claude_desktop_install(&p, Path::new("/x/orbo")).unwrap();
        assert_eq!(json_registered_command(&p).as_deref(), Some("/x/orbo"));
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "{ nope").unwrap();
        assert!(claude_desktop_install(&bad, Path::new("/x/orbo")).is_err());
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ nope");
    }

    #[test]
    fn codex_keeps_comments_and_tables() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".codex/config.toml");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let original = "# my model\nmodel = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"npx\"\nargs = [\"-y\", \"thing\"] # keep\n\n[projects.\"/tmp/x\"]\ntrust_level = \"trusted\"\n";
        std::fs::write(&p, original).unwrap();
        codex_install(&p, Path::new("/Applications/Orbo.app/Contents/MacOS/orbo-cli")).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.contains("# my model") && t.contains("# keep") && t.contains("[mcp_servers.other]") && t.contains("trust_level"));
        assert!(t.contains("[mcp_servers.orbo]") && t.contains("args = [\"mcp\"]"));
        assert_eq!(std::fs::read_to_string(dir.path().join(".codex/config.toml.bak")).unwrap(), original);
        assert_eq!(codex_registered_command(&p).as_deref(), Some("/Applications/Orbo.app/Contents/MacOS/orbo-cli"));
        codex_install(&p, Path::new("C:\\Program Files\\Orbo\\cli\\orbo.exe")).unwrap();
        assert_eq!(codex_registered_command(&p).as_deref(), Some("C:\\Program Files\\Orbo\\cli\\orbo.exe"));
        assert_eq!(std::fs::read_to_string(&p).unwrap().matches("[mcp_servers.orbo]").count(), 1);
        codex_remove(&p).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(!t.contains("orbo") && t.contains("[mcp_servers.other]") && t.contains("# my model"));
    }

    #[test]
    fn codex_creates_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        codex_install(&p, Path::new("/x/orbo")).unwrap();
        assert_eq!(codex_registered_command(&p).as_deref(), Some("/x/orbo"));
    }
}
