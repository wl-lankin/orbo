use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{log, now_ms};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Copy)]
pub enum Op {
    /// GDI+ ImageFormat name and optional longest edge.
    Image(&'static str, Option<u32>),
    Zip,
}

#[derive(Clone, Copy)]
pub struct Action {
    pub title: &'static str,
    pub sub: &'static str,
    pub color: [u8; 3],
    pub ext: &'static str,
    pub op: Op,
}

const IMAGE: [u8; 3] = [177, 151, 252];
const OTHER: [u8; 3] = [255, 184, 120];

const IMAGE_EXT: [&str; 7] = ["png", "jpg", "jpeg", "bmp", "gif", "tif", "tiff"];

pub fn kind_name(files: &[PathBuf]) -> &'static str {
    if !files.is_empty() && files.iter().all(|f| is_image(f)) { "Bild" } else { "Datei" }
}

fn is_image(p: &Path) -> bool {
    p.extension().map(|e| IMAGE_EXT.contains(&e.to_string_lossy().to_lowercase().as_str())).unwrap_or(false)
}

pub fn for_files(files: &[PathBuf]) -> Vec<Action> {
    let zip = Action { title: "ZIP", sub: "packen", color: OTHER, ext: "zip", op: Op::Zip };
    if files.is_empty() || !files.iter().all(|f| is_image(f)) {
        return vec![zip];
    }
    let own = if files.len() == 1 {
        files[0].extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
    } else {
        String::new()
    };
    let all = [
        Action { title: "JPG", sub: "überall kompatibel", color: IMAGE, ext: "jpg", op: Op::Image("Jpeg", None) },
        Action { title: "1600 px", sub: "für Mail", color: IMAGE, ext: "jpg", op: Op::Image("Jpeg", Some(1600)) },
        Action { title: "PNG", sub: "verlustfrei", color: IMAGE, ext: "png", op: Op::Image("Png", None) },
        Action { title: "BMP", sub: "unkomprimiert", color: IMAGE, ext: "bmp", op: Op::Image("Bmp", None) },
        Action { title: "TIFF", sub: "für Druck", color: IMAGE, ext: "tiff", op: Op::Image("Tiff", None) },
        Action { title: "GIF", sub: "256 Farben", color: IMAGE, ext: "gif", op: Op::Image("Gif", None) },
    ];
    let mut list: Vec<Action> = all
        .into_iter()
        .filter(|a| a.title.contains("px") || !(a.ext == own || (own == "jpeg" && a.ext == "jpg") || (own == "tif" && a.ext == "tiff")))
        .collect();
    list.truncate(5);
    list.push(zip);
    list
}

pub fn output_path(src: &Path, a: &Action) -> PathBuf {
    let dir = src.parent().unwrap_or(Path::new("."));
    let mut base = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Datei".into());
    if a.title.contains("px") {
        base = format!("{base} ({})", a.title);
    }
    let mut candidate = dir.join(format!("{base}.{}", a.ext));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{base} {n}.{}", a.ext));
        n += 1;
    }
    candidate
}

fn ps_quote(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', "''"))
}

fn command_for(a: &Action, src: &Path, dst: &Path) -> Command {
    match a.op {
        Op::Image(format, max) => {
            // Draw onto a fresh bitmap so transparent pixels become white for formats without alpha.
            let white = matches!(format, "Jpeg" | "Bmp");
            let script = format!(
                "$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Drawing; \
                 $i=[System.Drawing.Image]::FromFile({src}); \
                 $r=[Math]::Min(1.0, {max}/[Math]::Max($i.Width,$i.Height)); \
                 $w=[int][Math]::Max(1,$i.Width*$r); $h=[int][Math]::Max(1,$i.Height*$r); \
                 $b=New-Object System.Drawing.Bitmap($w,$h); $g=[System.Drawing.Graphics]::FromImage($b); \
                 {clear} $g.InterpolationMode='HighQualityBicubic'; $g.DrawImage($i,0,0,$w,$h); \
                 $b.Save({dst},[System.Drawing.Imaging.ImageFormat]::{format}); $g.Dispose(); $b.Dispose(); $i.Dispose()",
                src = ps_quote(src),
                dst = ps_quote(dst),
                max = max.map(|m| m as f64).unwrap_or(1.0e9),
                clear = if white { "$g.Clear([System.Drawing.Color]::White);" } else { "" },
            );
            let mut c = Command::new("powershell.exe");
            c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script]);
            c
        }
        Op::Zip => {
            let dir = src.parent().unwrap_or(Path::new("."));
            let name = src.file_name().map(|n| n.to_os_string()).unwrap_or_default();
            let mut c = Command::new("tar.exe");
            c.arg("-a").arg("-c").arg("-f").arg(dst).arg("-C").arg(dir).arg(name);
            c
        }
    }
}

pub fn run(a: Action, files: Vec<PathBuf>, done: impl FnOnce(bool) + Send + 'static) {
    std::thread::spawn(move || {
        let mut all_ok = true;
        for src in &files {
            let dst = output_path(src, &a);
            let start = now_ms();
            let mut cmd = command_for(&a, src, &dst);
            cmd.creation_flags(CREATE_NO_WINDOW);
            match cmd.output() {
                Ok(out) if out.status.success() && dst.exists() => {
                    log(&format!("Fertig: {} in {:.0} ms", dst.display(), now_ms() - start));
                }
                Ok(out) => {
                    all_ok = false;
                    let err = String::from_utf8_lossy(&out.stderr);
                    log(&format!("FEHLER {} für {}: {} {}", a.title, src.display(), out.status, err.trim()));
                }
                Err(e) => {
                    all_ok = false;
                    log(&format!("FEHLER beim Start für {}: {e}", a.title));
                }
            }
        }
        done(all_ok);
    });
}
