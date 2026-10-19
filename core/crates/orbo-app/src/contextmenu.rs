//! Right-click entry "Mit Orbo umwandeln": a Finder Quick Action on macOS, Explorer shell verbs on Windows.
//! Installed and removed only on request from the settings.

use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct Status {
    pub supported: bool,
    pub installed: bool,
    /// "finder" or "explorer"
    pub kind: &'static str,
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Status;
    use std::path::PathBuf;
    use std::process::Command;

    const NAME: &str = "Mit Orbo umwandeln.workflow";
    const INFO: &str = include_str!("../resources/macos/Mit Orbo umwandeln.workflow/Contents/Info.plist");
    const WFLOW: &str = include_str!("../resources/macos/Mit Orbo umwandeln.workflow/Contents/document.wflow");

    fn target() -> PathBuf {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Library/Services").join(NAME)
    }

    /// Writes the workflow bundle into `dir`.
    pub fn write_bundle(dir: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir.join("Contents"))?;
        std::fs::write(dir.join("Contents/Info.plist"), INFO)?;
        std::fs::write(dir.join("Contents/document.wflow"), WFLOW)
    }

    fn refresh() {
        // Makes the Services menu pick the change up without logging out.
        let _ = Command::new("/System/Library/CoreServices/pbs").arg("-update").status();
    }

    pub fn status() -> Status {
        Status { supported: true, installed: target().join("Contents/document.wflow").exists(), kind: "finder" }
    }

    pub fn install() -> Result<(), String> {
        let t = target();
        let _ = std::fs::remove_dir_all(&t);
        write_bundle(&t).map_err(|e| format!("Der Finder-Eintrag konnte nicht angelegt werden: {e}"))?;
        refresh();
        Ok(())
    }

    pub fn remove() -> Result<(), String> {
        let t = target();
        if t.exists() {
            std::fs::remove_dir_all(&t).map_err(|e| format!("Der Finder-Eintrag konnte nicht entfernt werden: {e}"))?;
        }
        refresh();
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn bundle_is_written_and_valid() {
            let dir = std::env::temp_dir().join(format!("orbo-wf-{}.workflow", std::process::id()));
            super::write_bundle(&dir).unwrap();
            for f in ["Contents/Info.plist", "Contents/document.wflow"] {
                let ok = std::process::Command::new("/usr/bin/plutil").arg("-lint").arg(dir.join(f)).status().unwrap().success();
                assert!(ok, "{f}");
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use super::Status;
    use windows::core::{w, PCWSTR};
    use windows::Win32::System::Registry::{RegDeleteTreeW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};

    const KEYS: [&str; 2] = [r"Software\Classes\*\shell\Orbo", r"Software\Classes\Directory\shell\Orbo"];

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn set(sub: &str, name: PCWSTR, value: &str) -> Result<(), String> {
        let (sub, v) = (wide(sub), wide(value));
        let err = unsafe {
            RegSetKeyValueW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr()), name, REG_SZ.0, Some(v.as_ptr() as *const _), (v.len() * 2) as u32)
        };
        if err.0 == 0 { Ok(()) } else { Err(format!("Registrierung fehlgeschlagen (Code {})", err.0)) }
    }

    pub fn status() -> Status {
        let sub = wide(KEYS[0]);
        let err = unsafe { RegGetValueW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr()), w!("MUIVerb"), RRF_RT_REG_SZ, None, None, None) };
        Status { supported: true, installed: err.0 == 0, kind: "explorer" }
    }

    pub fn install() -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        for key in KEYS {
            set(key, w!("MUIVerb"), "Mit Orbo umwandeln")?;
            set(key, w!("Icon"), &format!("\"{exe}\",0"))?;
            set(&format!(r"{key}\command"), PCWSTR::null(), &format!("\"{exe}\" --open \"%1\""))?;
        }
        Ok(())
    }

    pub fn remove() -> Result<(), String> {
        for key in KEYS {
            let sub = wide(key);
            let err = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr())) };
            // 2 = key not found, which is the state we want.
            if err.0 != 0 && err.0 != 2 {
                return Err(format!("Der Explorer-Eintrag konnte nicht entfernt werden (Code {})", err.0));
            }
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    use super::Status;
    pub fn status() -> Status {
        Status { supported: false, installed: false, kind: "none" }
    }
    pub fn install() -> Result<(), String> {
        Err("Auf diesem System nicht verfügbar".into())
    }
    pub fn remove() -> Result<(), String> {
        Ok(())
    }
}

pub use imp::{install, remove, status};
