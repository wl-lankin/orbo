use std::cell::Cell;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::UNIX_EPOCH;

use anyhow::{anyhow, bail, Context, Result};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use super::data::civil_from_days;
use super::{Ctx, Engine, Request};
use crate::format::{Format, Kind};

pub struct ArchiveEngine;

const JUNK: [&str; 3] = [".DS_Store", "Thumbs.db", "__MACOSX"];

impl Engine for ArchiveEngine {
    fn convert(&self, req: &Request, ctx: &Ctx) -> Result<()> {
        let res = match req.str("op") {
            Some("pack") => match req.str("format") {
                Some("zip") => pack_zip(req, ctx),
                Some("tgz") => pack_tgz(req, ctx),
                other => bail!("Unbekanntes Archivformat: {}", other.unwrap_or("-")),
            },
            Some("unpack") => unpack(req, ctx),
            Some("repack") => repack(req, ctx),
            other => bail!("Unbekannte Archivaktion: {}", other.unwrap_or("-")),
        };
        if res.is_err() && req.output.is_dir() {
            let _ = fs::remove_dir_all(req.output);
        }
        res
    }
}

fn check_cancel(ctx: &Ctx) -> Result<()> {
    if (ctx.cancelled)() {
        bail!("abgebrochen");
    }
    Ok(())
}

fn is_junk(name: &std::ffi::OsStr) -> bool {
    JUNK.iter().any(|j| name == *j)
}

/// Rejects absolute paths, drive prefixes and `..` components.
fn safe_relative(name: &str) -> Result<PathBuf> {
    let unsafe_path = || anyhow!("Das Archiv enthält den unsicheren Pfad \"{name}\" und wird aus Sicherheitsgründen nicht entpackt");
    if name.starts_with('/') || name.starts_with('\\') || name.as_bytes().get(1) == Some(&b':') {
        return Err(unsafe_path());
    }
    let mut out = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return Err(unsafe_path()),
            p => out.push(p),
        }
    }
    Ok(out)
}

fn zip_time(secs: i64) -> Option<DateTime> {
    let (y, mo, d) = civil_from_days(secs.div_euclid(86400));
    let rem = secs.rem_euclid(86400);
    DateTime::from_date_and_time(u16::try_from(y).ok()?, mo as u8, d as u8, (rem / 3600) as u8, (rem % 3600 / 60) as u8, (rem % 60) as u8).ok()
}

fn file_options(size: u64, mode: Option<u32>, mtime: Option<i64>) -> SimpleFileOptions {
    let mut o = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).large_file(size >= u32::MAX as u64);
    if let Some(m) = mode {
        o = o.unix_permissions(m & 0o7777);
    }
    if let Some(t) = mtime.and_then(zip_time) {
        o = o.last_modified_time(t);
    }
    o
}

#[cfg(unix)]
fn mode_of(meta: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode())
}

#[cfg(not(unix))]
fn mode_of(_: &fs::Metadata) -> Option<u32> {
    None
}

fn mtime_of(meta: &fs::Metadata) -> Option<i64> {
    meta.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
}

/// (path on disk, name inside the archive, is_dir)
fn collect_entries(req: &Request) -> Result<Vec<(PathBuf, String, bool)>> {
    let input = req.input;
    if req.analysis.kind != Kind::Folder && !input.is_dir() {
        let name = input.file_name().ok_or_else(|| anyhow!("Ungültiger Dateiname"))?.to_string_lossy().to_string();
        return Ok(vec![(input.to_path_buf(), name, false)]);
    }
    let root = input.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Ordner".into());
    let mut out = vec![(input.to_path_buf(), root.clone(), true)];
    for e in WalkDir::new(input).min_depth(1).sort_by_file_name().into_iter().filter_entry(|e| !is_junk(e.file_name())) {
        let e = e.context("Ordner konnte nicht gelesen werden")?;
        if e.path_is_symlink() {
            continue;
        }
        let rel = e.path().strip_prefix(input)?;
        let name = std::iter::once(root.clone()).chain(rel.components().map(|c| c.as_os_str().to_string_lossy().to_string())).collect::<Vec<_>>().join("/");
        out.push((e.path().to_path_buf(), name, e.file_type().is_dir()));
    }
    Ok(out)
}

fn total_bytes(entries: &[(PathBuf, String, bool)]) -> u64 {
    entries.iter().filter(|e| !e.2).filter_map(|e| fs::metadata(&e.0).ok()).map(|m| m.len()).sum::<u64>().max(1)
}

fn pack_zip(req: &Request, ctx: &Ctx) -> Result<()> {
    let entries = collect_entries(req)?;
    let total = total_bytes(&entries);
    let mut done = 0u64;
    let mut zw = ZipWriter::new(BufWriter::new(File::create(req.output).context("Archiv konnte nicht angelegt werden")?));
    for (path, name, is_dir) in &entries {
        check_cancel(ctx)?;
        let meta = fs::metadata(path)?;
        let opts = file_options(meta.len(), mode_of(&meta), mtime_of(&meta));
        if *is_dir {
            zw.add_directory(name.as_str(), opts)?;
        } else {
            zw.start_file(name.as_str(), opts)?;
            io::copy(&mut File::open(path)?, &mut zw)?;
            done += meta.len();
            (ctx.progress)(0.05 + 0.9 * done as f32 / total as f32);
        }
    }
    zw.finish()?.flush()?;
    Ok(())
}

fn pack_tgz(req: &Request, ctx: &Ctx) -> Result<()> {
    let entries = collect_entries(req)?;
    let total = total_bytes(&entries);
    let mut done = 0u64;
    let gz = flate2::write::GzEncoder::new(BufWriter::new(File::create(req.output).context("Archiv konnte nicht angelegt werden")?), flate2::Compression::default());
    let mut tb = tar::Builder::new(gz);
    tb.follow_symlinks(false);
    for (path, name, is_dir) in &entries {
        check_cancel(ctx)?;
        if *is_dir {
            tb.append_dir(name, path)?;
        } else {
            tb.append_path_with_name(path, name)?;
            done += fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            (ctx.progress)(0.05 + 0.9 * done as f32 / total as f32);
        }
    }
    tb.into_inner()?.finish()?.flush()?;
    Ok(())
}

struct Counted<R> {
    inner: R,
    pos: Rc<Cell<u64>>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.pos.set(self.pos.get() + n as u64);
        Ok(n)
    }
}

/// Opens a tar or tar.gz as a streaming reader and reports the compressed bytes read so far.
fn open_tar(path: &Path, format: Format) -> Result<(tar::Archive<Box<dyn Read>>, Rc<Cell<u64>>, u64)> {
    let file = File::open(path).context("Archiv konnte nicht geöffnet werden")?;
    let size = file.metadata()?.len().max(1);
    let pos = Rc::new(Cell::new(0));
    let counted = Counted { inner: BufReader::new(file), pos: pos.clone() };
    let reader: Box<dyn Read> = if format == Format::Tgz { Box::new(flate2::read::GzDecoder::new(counted)) } else { Box::new(counted) };
    Ok((tar::Archive::new(reader), pos, size))
}

fn unpack(req: &Request, ctx: &Ctx) -> Result<()> {
    fs::create_dir_all(req.output).context("Zielordner konnte nicht angelegt werden")?;
    match req.analysis.format {
        Format::Zip => unpack_zip(req, ctx),
        Format::Tar | Format::Tgz => unpack_tar(req, ctx),
        Format::Gz => unpack_gz(req),
        other => bail!("{} kann nicht entpackt werden", other.id()),
    }
}

fn unpack_zip(req: &Request, ctx: &Ctx) -> Result<()> {
    let mut ar = ZipArchive::new(BufReader::new(File::open(req.input)?)).map_err(|e| anyhow!("Das ZIP-Archiv ist beschädigt: {e}"))?;
    let n = ar.len().max(1);
    for i in 0..ar.len() {
        check_cancel(ctx)?;
        let mut entry = ar.by_index(i).map_err(|e| anyhow!("Das ZIP-Archiv ist beschädigt: {e}"))?;
        let rel = safe_relative(entry.name())?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let mode = entry.unix_mode();
        if mode.is_some_and(|m| m & 0o170000 == 0o120000) {
            continue;
        }
        let dest = req.output.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&dest)?;
        } else {
            if let Some(p) = dest.parent() {
                fs::create_dir_all(p)?;
            }
            io::copy(&mut entry, &mut File::create(&dest)?)?;
            set_mode(&dest, mode);
        }
        (ctx.progress)(0.05 + 0.9 * (i + 1) as f32 / n as f32);
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: Option<u32>) {
    use std::os::unix::fs::PermissionsExt;
    if let Some(m) = mode {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(m & 0o777 | 0o600));
    }
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: Option<u32>) {}

fn unpack_tar(req: &Request, ctx: &Ctx) -> Result<()> {
    let (mut ar, pos, size) = open_tar(req.input, req.analysis.format)?;
    for entry in ar.entries().map_err(|e| anyhow!("Das Archiv ist beschädigt: {e}"))? {
        check_cancel(ctx)?;
        let mut entry = entry.map_err(|e| anyhow!("Das Archiv ist beschädigt: {e}"))?;
        let name = entry.path()?.to_string_lossy().to_string();
        let rel = safe_relative(&name)?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        let dest = req.output.join(&rel);
        if kind.is_dir() {
            fs::create_dir_all(&dest)?;
        } else {
            if let Some(p) = dest.parent() {
                fs::create_dir_all(p)?;
            }
            let mode = entry.header().mode().ok();
            io::copy(&mut entry, &mut File::create(&dest)?)?;
            set_mode(&dest, mode);
        }
        (ctx.progress)(0.05 + 0.9 * (pos.get() as f32 / size as f32).min(1.0));
    }
    Ok(())
}

fn unpack_gz(req: &Request) -> Result<()> {
    let name = req.input.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let stem = match name.strip_suffix(".gz").or_else(|| name.strip_suffix(".GZ")) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => req.input.file_stem().map(|s| s.to_string_lossy().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "datei".into()),
    };
    let mut dec = flate2::read::GzDecoder::new(BufReader::new(File::open(req.input)?));
    let mut out = BufWriter::new(File::create(req.output.join(stem))?);
    io::copy(&mut dec, &mut out).map_err(|e| anyhow!("Die GZ-Datei ist beschädigt: {e}"))?;
    out.flush()?;
    Ok(())
}

fn repack(req: &Request, ctx: &Ctx) -> Result<()> {
    let (mut ar, pos, size) = open_tar(req.input, req.analysis.format)?;
    let mut zw = ZipWriter::new(BufWriter::new(File::create(req.output).context("Archiv konnte nicht angelegt werden")?));
    for entry in ar.entries().map_err(|e| anyhow!("Das Archiv ist beschädigt: {e}"))? {
        check_cancel(ctx)?;
        let mut entry = entry.map_err(|e| anyhow!("Das Archiv ist beschädigt: {e}"))?;
        let name = entry.path()?.to_string_lossy().to_string();
        let rel = safe_relative(&name)?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        let zip_name = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/");
        let opts = file_options(entry.size(), entry.header().mode().ok(), entry.header().mtime().ok().map(|t| t as i64));
        if kind.is_dir() {
            zw.add_directory(zip_name, opts)?;
        } else {
            zw.start_file(zip_name, opts)?;
            io::copy(&mut entry, &mut zw)?;
        }
        (ctx.progress)(0.05 + 0.9 * (pos.get() as f32 / size as f32).min(1.0));
    }
    zw.finish()?.flush()?;
    Ok(())
}
