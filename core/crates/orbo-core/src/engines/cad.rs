//! CAD engine: DXF and DWG to SVG, PDF, PNG and clean R12 DXF; SVG to DXF.
//!
//! Params (`op`):
//!   svg | pdf | png | dxf   render the drawing (DXF directly, DWG via the external `dwg2dxf`)
//!   dwg_dxf                 DWG to DXF with `dwg2dxf` as is
//!   svg_dxf                 outlines of an SVG as R12 DXF in millimetres
//!
//! `dwg2dxf` (LibreDWG, GPL) is only ever run as an external program and is never bundled.

mod dxf;
mod out;
mod svgin;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use super::{Ctx, Engine, Request};
use crate::format::Format;

pub struct Engine_;

/// The `dwg2dxf` executable, if installed.
pub fn dwg2dxf() -> Option<&'static PathBuf> {
    static P: OnceLock<Option<PathBuf>> = OnceLock::new();
    P.get_or_init(|| crate::tools::find("dwg2dxf")).as_ref()
}

const DWG_TIMEOUT: Duration = Duration::from_secs(180);

fn dwg_to_dxf(input: &Path, output: &Path, ctx: &Ctx) -> Result<()> {
    let exe = dwg2dxf().context("Für DWG-Dateien wird das Programm dwg2dxf (LibreDWG) benötigt, es ist nicht installiert")?;
    let _ = fs::remove_file(output);
    let mut child = crate::tools::command(exe)
        .arg("-y")
        .arg("-o")
        .arg(output)
        .arg(input)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("dwg2dxf konnte nicht gestartet werden")?;
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if (ctx.cancelled)() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(output);
            bail!("abgebrochen");
        }
        if start.elapsed() > DWG_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(output);
            bail!("Die DWG-Datei konnte nicht in angemessener Zeit umgewandelt werden");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    // LibreDWG reports recoverable problems with a non-zero code but still writes a usable file.
    let produced = fs::metadata(output).map(|m| m.len() > 0).unwrap_or(false);
    if !produced {
        let _ = fs::remove_file(output);
        bail!("Die DWG-Datei konnte nicht gelesen werden (dwg2dxf Code {})", status.code().unwrap_or(-1));
    }
    Ok(())
}

/// Scratch file next to the output, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(output: &Path) -> Scratch {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let name = format!(".orbo-cad-{}-{nanos}.dxf", std::process::id());
        Scratch(output.parent().unwrap_or(Path::new(".")).join(name))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn load(req: &Request, ctx: &Ctx) -> Result<dxf::Drawing> {
    match req.analysis.format {
        Format::Dxf => dxf::read(&fs::read(req.input).context("Datei konnte nicht gelesen werden")?),
        Format::Dwg => {
            let tmp = Scratch::new(req.output);
            dwg_to_dxf(req.input, &tmp.0, ctx)?;
            (ctx.progress)(0.5);
            dxf::read(&fs::read(&tmp.0)?)
        }
        other => bail!("{} wird von der CAD-Umwandlung nicht gelesen", other.id()),
    }
}

impl Engine for Engine_ {
    fn convert(&self, req: &Request, ctx: &Ctx) -> Result<()> {
        (ctx.progress)(0.05);
        let op = req.str("op").unwrap_or("");
        match op {
            "dwg_dxf" => dwg_to_dxf(req.input, req.output, ctx)?,
            "svg_dxf" => {
                let d = svgin::read(&fs::read(req.input).context("Datei konnte nicht gelesen werden")?)?;
                out::write_dxf(&d, req.output)?;
            }
            "svg" | "pdf" | "png" | "dxf" => {
                let d = load(req, ctx)?;
                (ctx.progress)(0.7);
                match op {
                    "svg" => out::write_svg(&d, req.output)?,
                    "pdf" => out::write_pdf(&d, req.output)?,
                    "png" => out::write_png(&d, req.output, req.options.max_edge.unwrap_or(2048))?,
                    _ => out::write_dxf(&d, req.output)?,
                }
            }
            other => bail!("Unbekannte CAD-Aktion: {}", if other.is_empty() { "-" } else { other }),
        }
        (ctx.progress)(0.95);
        Ok(())
    }
}
