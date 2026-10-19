//! Command line tool and AI integrations, set up only when the user clicks the matching button in the settings.
//! The tool itself ships inside the app (macOS: Contents/MacOS/orbo-cli, Windows: <install dir>\cli\orbo.exe).

use std::path::{Path, PathBuf};
use std::process::Stdio;
#[cfg(unix)]
use std::time::{Duration, Instant};

use orbo_core::integrations as cfg;
use serde::Serialize;

#[derive(Serialize, Clone, Default)]
pub struct Status {
    pub id: String,
    pub supported: bool,
    /// The bundled command line tool was found next to the app.
    pub available: bool,
    pub installed: bool,
    /// Set up, but pointing at another location than this app.
    pub stale: bool,
    /// Text to copy (shell command or profile line), with a short explanation in `note`.
    pub manual: Option<String>,
    pub note: Option<String>,
}

/// Path of the bundled command line tool.
pub fn bundled_cli() -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let dir = me.parent()?.to_path_buf();
    let p = if cfg!(windows) { dir.join("cli").join("orbo.exe") } else { dir.join("orbo-cli") };
    p.is_file().then_some(p)
}

fn need_cli() -> Result<PathBuf, String> {
    bundled_cli().ok_or_else(|| "Das Kommandozeilen-Tool fehlt in dieser Orbo-Installation. Installiere Orbo neu oder aktualisiere die App.".to_string())
}

fn same_path(a: &str, b: &Path) -> bool {
    let norm = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    norm(Path::new(a)) == norm(b)
}

pub fn status(id: &str) -> Status {
    let mut st = Status { id: id.into(), supported: cfg!(any(target_os = "macos", target_os = "windows")), ..Default::default() };
    let cli = bundled_cli();
    st.available = cli.is_some();
    if !st.supported {
        return st;
    }
    let registered = match id {
        "cli" => return cli_status(st, cli.as_deref()),
        "claude_code" => cfg::json_registered_command(&cfg::claude_code_config_path()),
        "claude_desktop" => cfg::json_registered_command(&cfg::claude_desktop_config_path()),
        "codex" => cfg::codex_registered_command(&cfg::codex_config_path()),
        _ => None,
    };
    if let Some(cmd) = registered {
        st.installed = true;
        st.stale = cli.as_deref().map_or(true, |c| !same_path(&cmd, c));
    }
    st
}

pub fn install(id: &str) -> Result<Status, String> {
    let cli = need_cli()?;
    let (note, manual) = match id {
        "cli" => return cli_install(&cli),
        "claude_code" => match find_claude() {
            Some(claude) => {
                let _ = run(&claude, &["mcp", "remove", "--scope", "user", "orbo"]);
                run(&claude, &["mcp", "add", "--scope", "user", "orbo", "--", &cli.to_string_lossy(), "mcp"])?;
                ("In Claude Code eingerichtet. Neue Sitzungen kennen die Orbo-Werkzeuge.".to_string(), None)
            }
            None => (
                "Die Befehlszeile von Claude Code wurde nicht gefunden. Führe diesen Befehl in einem Terminal aus.".to_string(),
                Some(format!("claude mcp add --scope user orbo -- \"{}\" mcp", cli.display())),
            ),
        },
        "claude_desktop" => {
            cfg::claude_desktop_install(&cfg::claude_desktop_config_path(), &cli).map_err(|e| format!("{e:#}"))?;
            ("In Claude Desktop eingerichtet. Beende Claude Desktop vollständig und starte es neu.".to_string(), None)
        }
        "codex" => {
            cfg::codex_install(&cfg::codex_config_path(), &cli).map_err(|e| format!("{e:#}"))?;
            ("In Codex eingerichtet. Neue Codex-Sitzungen kennen die Orbo-Werkzeuge.".to_string(), None)
        }
        _ => return Err("Unbekannte Einrichtung".into()),
    };
    let mut st = status(id);
    st.note = Some(note);
    st.manual = manual;
    Ok(st)
}

pub fn remove(id: &str) -> Result<Status, String> {
    let (note, manual) = match id {
        "cli" => return cli_remove(),
        "claude_code" => match find_claude() {
            Some(claude) => {
                run(&claude, &["mcp", "remove", "--scope", "user", "orbo"])?;
                (None, None)
            }
            None => (
                Some("Die Befehlszeile von Claude Code wurde nicht gefunden. Führe diesen Befehl in einem Terminal aus.".to_string()),
                Some("claude mcp remove --scope user orbo".to_string()),
            ),
        },
        "claude_desktop" => {
            cfg::claude_desktop_remove(&cfg::claude_desktop_config_path()).map_err(|e| format!("{e:#}"))?;
            (Some("Aus Claude Desktop entfernt. Starte Claude Desktop neu.".to_string()), None)
        }
        "codex" => {
            cfg::codex_remove(&cfg::codex_config_path()).map_err(|e| format!("{e:#}"))?;
            (None, None)
        }
        _ => return Err("Unbekannte Einrichtung".into()),
    };
    let mut st = status(id);
    st.note = note;
    st.manual = manual;
    Ok(st)
}

fn run(program: &Path, args: &[&str]) -> Result<(), String> {
    let out = orbo_core::tools::command(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{} konnte nicht gestartet werden: {e}", program.display()))?;
    if out.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr }).trim().to_string();
    Err(format!("Der Befehl ist fehlgeschlagen: {}", if text.is_empty() { "keine Meldung".into() } else { text }))
}

/// Runs a script in the user's login shell (macOS GUI apps do not inherit the shell PATH). Gives up after `timeout`.
#[cfg(unix)]
fn login_shell(script: &str, timeout: Duration) -> Option<String> {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into());
    let mut child = std::process::Command::new(shell).args(["-lic", script]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut s);
        s
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    reader.join().ok()
}

fn find_claude() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default());
    #[cfg(unix)]
    {
        let line = login_shell("echo __ORBO__$(command -v claude)", Duration::from_secs(5))
            .and_then(|o| o.lines().rev().find_map(|l| l.strip_prefix("__ORBO__").map(str::to_string)));
        if let Some(p) = line.map(PathBuf::from).filter(|p| p.is_file()) {
            return Some(p);
        }
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend([home.join(".local/bin"), home.join(".claude/local"), PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")]);
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] };
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

/// Adds `dir` to a semicolon separated PATH value unless it is already there (case-insensitive, trailing backslash ignored).
#[cfg_attr(not(windows), allow(dead_code))]
fn path_with(current: &str, dir: &str) -> String {
    if path_has(current, dir) {
        return current.to_string();
    }
    if current.trim().is_empty() { dir.to_string() } else { format!("{};{}", current.trim_end_matches(';'), dir) }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn path_without(current: &str, dir: &str) -> String {
    let key = |s: &str| s.trim().trim_end_matches(['\\', '/']).to_lowercase();
    current.split(';').filter(|e| !e.trim().is_empty() && key(e) != key(dir)).collect::<Vec<_>>().join(";")
}

#[cfg_attr(not(windows), allow(dead_code))]
fn path_has(current: &str, dir: &str) -> bool {
    let key = |s: &str| s.trim().trim_end_matches(['\\', '/']).to_lowercase();
    current.split(';').any(|e| key(e) == key(dir))
}

#[cfg(target_os = "macos")]
fn link_path() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/bin/orbo")
}

#[cfg(target_os = "macos")]
fn cli_status(mut st: Status, cli: Option<&Path>) -> Status {
    let link = link_path();
    if std::fs::symlink_metadata(&link).is_ok() {
        let target = std::fs::read_link(&link).ok();
        st.installed = true;
        st.stale = match (&target, cli) {
            (Some(t), Some(c)) => !same_path(&t.to_string_lossy(), c) || !link.exists(),
            _ => true,
        };
        if !st.stale {
            apply_path_hint(&mut st);
        }
    }
    st
}

/// Checks the login shell PATH and, if `~/.local/bin` is missing, prepares the line to add. Never edits profiles.
#[cfg(target_os = "macos")]
fn apply_path_hint(st: &mut Status) {
    let home = std::env::var("HOME").unwrap_or_default();
    let Some(out) = login_shell("echo __ORBO__$PATH", Duration::from_secs(5)) else { return };
    let Some(path) = out.lines().rev().find_map(|l| l.strip_prefix("__ORBO__")) else { return };
    let want = format!("{home}/.local/bin");
    if path.split(':').any(|p| p.trim_end_matches('/') == want) {
        return;
    }
    let shell = std::env::var("SHELL").unwrap_or_default();
    let (file, line) = match shell.rsplit('/').next().unwrap_or("") {
        "bash" => ("~/.bash_profile", "export PATH=\"$HOME/.local/bin:$PATH\""),
        "fish" => ("~/.config/fish/config.fish", "fish_add_path $HOME/.local/bin"),
        "zsh" | "" => ("~/.zprofile", "export PATH=\"$HOME/.local/bin:$PATH\""),
        _ => ("~/.profile", "export PATH=\"$HOME/.local/bin:$PATH\""),
    };
    st.manual = Some(line.to_string());
    st.note = Some(format!("Der Ordner ~/.local/bin ist noch nicht in deinem PATH. Füge diese Zeile in {file} ein und öffne danach ein neues Terminalfenster."));
}

#[cfg(target_os = "macos")]
fn cli_install(cli: &Path) -> Result<Status, String> {
    let link = link_path();
    if let Ok(meta) = std::fs::symlink_metadata(&link) {
        if !meta.file_type().is_symlink() {
            return Err(format!("{} existiert bereits und ist keine Verknüpfung. Orbo lässt die Datei unverändert.", link.display()));
        }
        std::fs::remove_file(&link).map_err(|e| format!("Die alte Verknüpfung konnte nicht entfernt werden: {e}"))?;
    }
    std::fs::create_dir_all(link.parent().unwrap()).map_err(|e| format!("~/.local/bin konnte nicht angelegt werden: {e}"))?;
    std::os::unix::fs::symlink(cli, &link).map_err(|e| format!("Die Verknüpfung konnte nicht angelegt werden: {e}"))?;
    let mut st = status("cli");
    if st.note.is_none() {
        st.note = Some("Eingerichtet. Öffne ein neues Terminalfenster und gib „orbo targets“ ein.".into());
    }
    Ok(st)
}

#[cfg(target_os = "macos")]
fn cli_remove() -> Result<Status, String> {
    let link = link_path();
    if std::fs::read_link(&link).is_ok() {
        std::fs::remove_file(&link).map_err(|e| format!("Die Verknüpfung konnte nicht entfernt werden: {e}"))?;
    }
    Ok(status("cli"))
}

#[cfg(target_os = "windows")]
mod winpath {
    use windows::core::w;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::Registry::{RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_EXPAND_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ};
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The user PATH as stored (variables like %USERPROFILE% stay unexpanded). Empty if the value does not exist yet.
    pub fn read() -> Result<String, String> {
        let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
        let mut bytes = 0u32;
        let err = unsafe { RegGetValueW(HKEY_CURRENT_USER, w!("Environment"), w!("Path"), flags, None, None, Some(&mut bytes)) };
        if err.0 == 2 {
            return Ok(String::new());
        }
        if err.0 != 0 {
            return Err(format!("Der PATH konnte nicht gelesen werden (Code {})", err.0));
        }
        let mut buf = vec![0u16; (bytes as usize + 1) / 2 + 1];
        let mut size = (buf.len() * 2) as u32;
        let err = unsafe { RegGetValueW(HKEY_CURRENT_USER, w!("Environment"), w!("Path"), flags, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size)) };
        if err.0 != 0 {
            return Err(format!("Der PATH konnte nicht gelesen werden (Code {})", err.0));
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(String::from_utf16_lossy(&buf[..len]))
    }

    pub fn write(value: &str) -> Result<(), String> {
        let v = wide(value);
        let err = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, w!("Environment"), w!("Path"), REG_EXPAND_SZ.0, Some(v.as_ptr() as *const _), (v.len() * 2) as u32) };
        if err.0 != 0 {
            return Err(format!("Der PATH konnte nicht gespeichert werden (Code {})", err.0));
        }
        // Lets Explorer and new terminals pick the change up without a logoff.
        let env = wide("Environment");
        unsafe {
            let _ = SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, WPARAM(0), LPARAM(env.as_ptr() as isize), SMTO_ABORTIFHUNG, 3000, None);
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn cli_dir(cli: &Path) -> String {
    cli.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(target_os = "windows")]
fn cli_status(mut st: Status, cli: Option<&Path>) -> Status {
    if let (Some(c), Ok(path)) = (cli, winpath::read()) {
        st.installed = path_has(&path, &cli_dir(c));
    }
    st
}

#[cfg(target_os = "windows")]
fn cli_install(cli: &Path) -> Result<Status, String> {
    let path = winpath::read()?;
    winpath::write(&path_with(&path, &cli_dir(cli)))?;
    let mut st = status("cli");
    st.note = Some("Eingerichtet. Öffne ein neues Terminal- oder PowerShell-Fenster und gib „orbo targets“ ein.".into());
    Ok(st)
}

#[cfg(target_os = "windows")]
fn cli_remove() -> Result<Status, String> {
    if let Some(cli) = bundled_cli() {
        let path = winpath::read()?;
        winpath::write(&path_without(&path, &cli_dir(&cli)))?;
    }
    Ok(status("cli"))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn cli_status(st: Status, _cli: Option<&Path>) -> Status {
    st
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn cli_install(_cli: &Path) -> Result<Status, String> {
    Err("Auf diesem System nicht verfügbar".into())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn cli_remove() -> Result<Status, String> {
    Ok(status("cli"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_edit_avoids_duplicates() {
        let p = r"C:\Windows;%USERPROFILE%\bin";
        let added = path_with(p, r"C:\Program Files\Orbo\cli");
        assert_eq!(added, r"C:\Windows;%USERPROFILE%\bin;C:\Program Files\Orbo\cli");
        assert_eq!(path_with(&added, r"c:\program files\orbo\cli\"), added);
        assert_eq!(path_without(&added, r"C:\Program Files\Orbo\cli"), p);
        assert_eq!(path_with("", r"C:\x"), r"C:\x");
        assert_eq!(path_without(r"C:\x;", r"C:\x"), "");
    }

    #[test]
    fn unknown_id_is_rejected() {
        assert!(remove("nope").is_err());
    }
}
