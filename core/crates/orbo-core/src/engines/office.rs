//! Office engine: LibreOffice (optional, headless) and a pure Rust PPTX text reader.
//!
//! Params:
//!   op  lo    to = pdf, pptx, odp, docx    conversion with `soffice --headless --convert-to`
//!   op  pptx_text   to = txt, md           slide texts from the PPTX XML, no LibreOffice needed

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};

use super::{Ctx, Engine, Request};
use crate::engines::ebook::md_escape;

pub struct Engine_;

impl Engine for Engine_ {
    fn convert(&self, req: &Request, ctx: &Ctx) -> Result<()> {
        (ctx.progress)(0.05);
        let res = match req.str("op") {
            Some("lo") => libreoffice(req, ctx),
            Some("pptx_text") => pptx_text(req),
            other => bail!("Unbekannte Office-Aktion: {}", other.unwrap_or("-")),
        };
        (ctx.progress)(0.95);
        res
    }
}

/// The LibreOffice command line binary, if installed.
pub fn soffice() -> Option<&'static PathBuf> {
    static P: OnceLock<Option<PathBuf>> = OnceLock::new();
    P.get_or_init(find_soffice).as_ref()
}

fn find_soffice() -> Option<PathBuf> {
    if let Some(p) = crate::tools::find("soffice") {
        return Some(p);
    }
    let mut candidates: Vec<PathBuf> = vec!["/Applications/LibreOffice.app/Contents/MacOS/soffice".into()];
    for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Some(dir) = std::env::var_os(var) {
            candidates.push(Path::new(&dir).join("LibreOffice").join("program").join("soffice.exe"));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Scratch directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<TempDir> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("orbo-lo-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).context("Temporärer Ordner konnte nicht angelegt werden")?;
        Ok(TempDir(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn file_url(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let enc: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "/-_.~:".contains(c) { c.to_string() } else { c.to_string().bytes().map(|b| format!("%{b:02X}")).collect() })
        .collect();
    if enc.starts_with('/') {
        format!("file://{enc}")
    } else {
        format!("file:///{enc}")
    }
}

fn libreoffice(req: &Request, ctx: &Ctx) -> Result<()> {
    let exe = soffice().ok_or_else(|| anyhow!("LibreOffice ist nicht installiert"))?;
    let to = req.str("to").ok_or_else(|| anyhow!("Zielformat fehlt"))?;
    let tmp = TempDir::new()?;
    let outdir = tmp.0.join("out");
    let profile = tmp.0.join("profile");
    fs::create_dir_all(&outdir)?;
    let mut child = crate::tools::command(exe)
        .arg("--headless")
        .arg("--norestore")
        .arg(format!("-env:UserInstallation={}", file_url(&profile)))
        .arg("--convert-to")
        .arg(to)
        .arg("--outdir")
        .arg(&outdir)
        .arg(req.input)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("LibreOffice konnte nicht gestartet werden")?;
    let started = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if (ctx.cancelled)() {
            let _ = child.kill();
            let _ = child.wait();
            bail!("abgebrochen");
        }
        if started.elapsed() > Duration::from_secs(900) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("LibreOffice hat zu lange gebraucht");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut stderr = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut stderr);
    }
    let result = fs::read_dir(&outdir)?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(to)));
    let Some(result) = result.filter(|_| status.success()) else {
        let detail = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
        bail!("LibreOffice konnte die Datei nicht umwandeln{}", if detail.is_empty() { String::new() } else { format!(": {detail}") });
    };
    if fs::rename(&result, req.output).is_err() {
        fs::copy(&result, req.output).context("Ausgabe konnte nicht geschrieben werden")?;
    }
    Ok(())
}

// ---------- PPTX text ----------

struct Slide {
    title: String,
    /// (indent level, bullet, text)
    paragraphs: Vec<(usize, bool, String)>,
}

fn local<'a>(n: &roxmltree::Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn paragraph_text(p: roxmltree::Node) -> String {
    let mut s = String::new();
    for n in p.descendants() {
        match local(&n) {
            "t" => s.push_str(n.text().unwrap_or("")),
            "br" => s.push('\n'),
            "tab" => s.push('\t'),
            _ => {}
        }
    }
    s
}

fn read_slide(xml: &str) -> Result<Slide> {
    let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(xml, opts).map_err(|e| anyhow!("Die Folie ist fehlerhaft: {e}"))?;
    let mut slide = Slide { title: String::new(), paragraphs: Vec::new() };
    for shape in doc.descendants().filter(|n| matches!(local(n), "sp" | "graphicFrame")) {
        let ph = shape.descendants().find(|n| local(n) == "ph");
        let ph_type = ph.and_then(|p| p.attribute("type")).unwrap_or("");
        let is_title = matches!(ph_type, "title" | "ctrTitle");
        let is_body = ph.is_some() && !is_title && !matches!(ph_type, "sldNum" | "ftr" | "dt");
        if matches!(ph_type, "sldNum" | "ftr" | "dt") {
            continue;
        }
        for p in shape.descendants().filter(|n| local(n) == "p") {
            let text = paragraph_text(p);
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            if is_title {
                if !slide.title.is_empty() {
                    slide.title.push(' ');
                }
                slide.title.push_str(&text.replace('\n', " "));
            } else {
                let lvl = p.children().find(|n| local(n) == "pPr").and_then(|n| n.attribute("lvl")).and_then(|v| v.parse().ok()).unwrap_or(0);
                let no_bullet = p.descendants().any(|n| local(&n) == "buNone");
                slide.paragraphs.push((lvl, is_body && !no_bullet, text.to_string()));
            }
        }
    }
    Ok(slide)
}

fn slide_number(name: &str) -> Option<u32> {
    name.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse().ok()
}

fn pptx_text(req: &Request) -> Result<()> {
    let to = req.str("to").unwrap_or("txt");
    let file = fs::File::open(req.input).context("Datei konnte nicht geöffnet werden")?;
    let mut z = zip::ZipArchive::new(file).map_err(|_| anyhow!("Die Datei ist keine gültige PPTX-Präsentation"))?;
    let mut names: Vec<(u32, String)> = z.file_names().filter_map(|n| slide_number(n).map(|k| (k, n.to_string()))).collect();
    names.sort();
    if names.is_empty() {
        bail!("Die Präsentation enthält keine Folien");
    }
    let mut out = String::new();
    for (i, (_, name)) in names.iter().enumerate() {
        let mut xml = String::new();
        z.by_name(name)?.read_to_string(&mut xml)?;
        let s = read_slide(&xml)?;
        if to == "md" {
            let title = if s.title.is_empty() { format!("Folie {}", i + 1) } else { md_escape(&s.title) };
            out.push_str(&format!("## {title}\n\n"));
            for (lvl, bullet, t) in &s.paragraphs {
                let t = md_escape(t).replace('\n', "  \n");
                if *bullet {
                    out.push_str(&format!("{}- {t}\n", "  ".repeat(*lvl)));
                } else {
                    out.push_str(&format!("\n{t}\n\n"));
                }
            }
            out.push('\n');
        } else {
            out.push_str(&format!("Folie {}", i + 1));
            if !s.title.is_empty() {
                out.push_str(&format!(": {}", s.title));
            }
            out.push_str("\n\n");
            for (lvl, bullet, t) in &s.paragraphs {
                if *bullet {
                    out.push_str(&format!("{}- {t}\n", "  ".repeat(*lvl)));
                } else {
                    out.push_str(&format!("{t}\n"));
                }
            }
            out.push_str("\n\n");
        }
    }
    let out = out.replace("\n\n\n\n", "\n\n\n");
    fs::write(req.output, out.trim_end().to_string() + "\n").context("Ausgabe konnte nicht geschrieben werden")
}
