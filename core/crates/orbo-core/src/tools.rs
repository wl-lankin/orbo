//! Locates external helper programs (ffmpeg, ffprobe) and caches what they can do.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// A child process without a console window. On Windows a GUI app would otherwise flash one per call.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// Search order: env var ORBO_<NAME>, next to the running executable (symlinks resolved), `bin/` next to it, the parent of a `cli/` folder, then PATH.
pub fn find(name: &str) -> Option<PathBuf> {
    if let Ok(p) = std::env::var(format!("ORBO_{}", name.to_uppercase())) {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(me) = std::env::current_exe() {
        // The CLI is started through a symlink (~/.local/bin/orbo) or from a subfolder of the install dir (Windows: cli/).
        let real = me.canonicalize().unwrap_or(me);
        if let Some(dir) = real.parent() {
            let mut dirs = vec![dir.to_path_buf(), dir.join("bin")];
            if dir.file_name().is_some_and(|n| n == "cli") {
                dirs.extend(dir.parent().map(|up| up.to_path_buf()));
            }
            for candidate in dirs.into_iter().map(|d| d.join(exe(name))) {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    // GUI apps on macOS do not inherit the shell PATH.
    if cfg!(target_os = "macos") {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    }
    dirs.into_iter().map(|d| d.join(exe(name))).find(|p| p.is_file())
}

pub fn ffmpeg() -> Option<&'static PathBuf> {
    static P: OnceLock<Option<PathBuf>> = OnceLock::new();
    P.get_or_init(|| find("ffmpeg")).as_ref()
}

pub fn ffprobe() -> Option<&'static PathBuf> {
    static P: OnceLock<Option<PathBuf>> = OnceLock::new();
    P.get_or_init(|| find("ffprobe")).as_ref()
}

/// Names of all encoders the available ffmpeg build offers (e.g. "h264_videotoolbox", "libmp3lame").
pub fn ffmpeg_encoders() -> &'static HashSet<String> {
    static E: OnceLock<HashSet<String>> = OnceLock::new();
    E.get_or_init(|| {
        let Some(ff) = ffmpeg() else { return HashSet::new() };
        let Ok(out) = command(ff).args(["-hide_banner", "-encoders"]).output() else { return HashSet::new() };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| {
                let mut parts = l.split_whitespace();
                let flags = parts.next()?;
                let name = parts.next()?;
                (flags.len() == 6 && flags.chars().next().map(|c| "VAS".contains(c)).unwrap_or(false)).then(|| name.to_string())
            })
            .collect()
    })
}

/// First encoder from the preference list that this ffmpeg build has.
pub fn pick_encoder(preference: &[&str]) -> Option<String> {
    let have = ffmpeg_encoders();
    preference.iter().find(|e| have.contains(**e)).map(|e| e.to_string())
}
