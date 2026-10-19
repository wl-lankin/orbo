//! Merge engine: several inputs become one output (targets with `merge: true`).
//!
//! The job runner sorts the inputs naturally by file name and hands them in as `inputs`.
//!
//! Params (`op`):
//!   merge_pdf     PDFs (and images as extra pages) into one PDF; pages keep their size
//!   images_pdf    one image per page; `page`: original | a4 | a4_landscape
//!   audio_concat  audio files end to end; `format`: keep | mp3 | m4a | wav
//!
//! Limits of the PDF merge: bookmarks, named destinations and form fields of the sources are dropped,
//! the pages themselves (content, links, annotations) are kept.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};

use super::{image as image_engine, media, Ctx};
use crate::analyze::Analysis;
use crate::catalog::Target;
use crate::format::Format;
use crate::jobs::Options;

/// Registered so the catalog stays consistent; merges run through `convert` below, never through the single-input trait.
pub struct MergeEngine;

impl super::Engine for MergeEngine {
    fn convert(&self, req: &super::Request, _ctx: &Ctx) -> Result<()> {
        bail!("„{}“ braucht mindestens zwei Dateien", req.target.title)
    }
}

pub struct MergeRequest<'a> {
    pub inputs: &'a [PathBuf],
    pub analyses: &'a [Analysis],
    /// Temporary output path with the final extension.
    pub output: &'a Path,
    pub target: &'a Target,
    pub params: &'a serde_json::Value,
    pub options: &'a Options,
}

impl MergeRequest<'_> {
    fn str(&self, key: &str) -> Option<&str> {
        self.params.get(key).and_then(|v| v.as_str())
    }
}

/// Extension of the merged file; audio follows the first file unless a preset names a format.
pub fn output_ext(target: &Target, params: &serde_json::Value, analyses: &[Analysis]) -> String {
    if params.get("op").and_then(|v| v.as_str()) == Some("audio_concat") {
        let wanted = params.get("format").and_then(|v| v.as_str()).unwrap_or("keep");
        return media::concat_format(analyses[0].format, wanted).2.to_string();
    }
    target.output_ext().to_string()
}

pub fn convert(req: &MergeRequest, ctx: &Ctx) -> Result<()> {
    match req.str("op") {
        Some("merge_pdf") | Some("images_pdf") => pdf(req, ctx),
        Some("audio_concat") => {
            let bitrate = req.options.audio_bitrate_k.or_else(|| req.params.get("audio_bitrate_k").and_then(|v| v.as_u64()).map(|v| v as u32)).unwrap_or(192);
            media::concat(req.inputs, req.analyses, req.str("format").unwrap_or("keep"), bitrate, req.output, ctx)
        }
        other => bail!("Unbekannte Zusammenführung: {}", other.unwrap_or("-")),
    }
}

fn check_cancel(ctx: &Ctx) -> Result<()> {
    if (ctx.cancelled)() {
        bail!("abgebrochen");
    }
    Ok(())
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

// ---------------------------------------------------------------- PDF

fn pdf(req: &MergeRequest, ctx: &Ctx) -> Result<()> {
    let page = Page::from_param(req.str("page"));
    let quality = req.options.quality.or_else(|| req.params.get("quality").and_then(|v| v.as_u64()).map(|q| q as u8)).unwrap_or(90).clamp(1, 100);
    let total = req.inputs.len();
    let mut merger = Merger::default();
    for (i, (path, a)) in req.inputs.iter().zip(req.analyses).enumerate() {
        check_cancel(ctx)?;
        let doc = if a.format == Format::Pdf {
            super::document::load_pdf(path).with_context(|| name_of(path))?
        } else {
            // Images inside a PDF merge keep their pixel size; the A4 presets belong to images_pdf only.
            let page = if req.str("op") == Some("images_pdf") { page } else { Page::Original };
            image_doc(path, a, page, quality, req.output).with_context(|| name_of(path))?
        };
        merger.add(doc);
        (ctx.progress)(0.05 + 0.85 * (i + 1) as f32 / total as f32);
    }
    check_cancel(ctx)?;
    let mut doc = merger.finish()?;
    super::document::save_pdf(&mut doc, req.output)
}

#[derive(Default)]
struct Merger {
    next_id: u32,
    objects: BTreeMap<ObjectId, Object>,
    pages: Vec<ObjectId>,
}

/// Pages inherit Resources, MediaBox, CropBox and Rotate from their Pages node; moving them under a new
/// node would lose that, so it is copied onto each page first.
fn flatten_inherited(doc: &mut Document) {
    let ids: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    for id in ids {
        for key in [&b"Resources"[..], b"MediaBox", b"CropBox", b"Rotate"] {
            let mut cur = id;
            let mut found = None;
            for _ in 0..32 {
                let Ok(d) = doc.get_dictionary(cur) else { break };
                if let Ok(v) = d.get(key) {
                    found = Some(v.clone());
                    break;
                }
                match d.get(b"Parent").and_then(Object::as_reference) {
                    Ok(parent) => cur = parent,
                    Err(_) => break,
                }
            }
            if let (true, Some(v)) = (cur != id, found) {
                if let Ok(page) = doc.get_dictionary_mut(id) {
                    page.set(key, v);
                }
            }
        }
    }
}

impl Merger {
    fn add(&mut self, mut doc: Document) {
        flatten_inherited(&mut doc);
        doc.renumber_objects_with(self.next_id.max(1));
        let pages: Vec<ObjectId> = doc.get_pages().values().copied().collect();
        self.next_id = doc.objects.keys().map(|k| k.0).max().unwrap_or(0).max(self.next_id) + 1;
        for (id, obj) in doc.objects {
            // The old page trees and catalogs are replaced by a new one in `finish`.
            if matches!(obj.type_name(), Ok(b"Catalog") | Ok(b"Pages")) {
                continue;
            }
            self.objects.insert(id, obj);
        }
        self.pages.extend(pages);
    }

    fn finish(self) -> Result<Document> {
        if self.pages.is_empty() {
            bail!("Die Dateien enthalten keine Seiten");
        }
        let mut doc = Document::with_version("1.5");
        doc.objects = self.objects;
        doc.max_id = self.next_id.saturating_sub(1);
        let pages_id = doc.new_object_id();
        for id in &self.pages {
            if let Ok(page) = doc.get_dictionary_mut(*id) {
                page.set("Parent", pages_id);
            }
        }
        let kids: Vec<Object> = self.pages.iter().map(|id| Object::Reference(*id)).collect();
        let count = kids.len() as i64;
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        doc.prune_objects();
        doc.compress();
        Ok(doc)
    }
}

// ---------------------------------------------------------------- images as pages

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Original,
    A4,
    A4Landscape,
}

impl Page {
    fn from_param(p: Option<&str>) -> Page {
        match p {
            Some("a4") => Page::A4,
            Some("a4_landscape") => Page::A4Landscape,
            _ => Page::Original,
        }
    }
}

/// Pixels per inch assumed for "Originalgröße".
const DPI: f64 = 150.0;
const A4_W: f64 = 595.276;
const A4_H: f64 = 841.89;
/// 10 mm in points
const MARGIN: f64 = 28.346;
/// PDF viewers reject pages beyond 200 inches.
const MAX_PAGE: f64 = 14400.0;

/// (page width, page height, x, y, drawn width, drawn height) in points.
fn layout(page: Page, w: u32, h: u32) -> (f64, f64, f64, f64, f64, f64) {
    let (w, h) = (w.max(1) as f64, h.max(1) as f64);
    match page {
        Page::Original => {
            let k = (72.0 / DPI).min(MAX_PAGE / w).min(MAX_PAGE / h);
            (w * k, h * k, 0.0, 0.0, w * k, h * k)
        }
        Page::A4 | Page::A4Landscape => {
            let (pw, ph) = if page == Page::A4 { (A4_W, A4_H) } else { (A4_H, A4_W) };
            let k = ((pw - 2.0 * MARGIN) / w).min((ph - 2.0 * MARGIN) / h);
            let (dw, dh) = (w * k, h * k);
            (pw, ph, (pw - dw) / 2.0, (ph - dh) / 2.0, dw, dh)
        }
    }
}

/// Width, height and component count of a baseline or progressive 8-bit JPEG, read from its SOF marker.
fn jpeg_info(b: &[u8]) -> Option<(u32, u32, u8)> {
    if b.len() < 4 || b[0] != 0xFF || b[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = b[i + 1];
        if m == 0xFF || m == 0x01 || (0xD0..=0xD8).contains(&m) {
            i += if m == 0xFF { 1 } else { 2 };
            continue;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        if matches!(m, 0xC0..=0xCF) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
            let s = b.get(i + 4..i + 4 + 6)?;
            // Arithmetic coding (SOF9 and up) is rarely supported by PDF viewers.
            if s[0] != 8 || m >= 0xC9 {
                return None;
            }
            return Some((u16::from_be_bytes([s[3], s[4]]) as u32, u16::from_be_bytes([s[1], s[2]]) as u32, s[5]));
        }
        i += 2 + len;
    }
    None
}

/// JPEG data that can go into the PDF as it is: 8-bit gray or RGB, not rotated by EXIF.
fn raw_jpeg(path: &Path) -> Option<(u32, u32, Vec<u8>, bool)> {
    let bytes = fs::read(path).ok()?;
    let (w, h, comps) = jpeg_info(&bytes)?;
    if comps != 1 && comps != 3 {
        return None;
    }
    let mut dec = ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_decoder().ok()?;
    if dec.orientation().ok()? != Orientation::NoTransforms {
        return None;
    }
    Some((w, h, bytes, comps == 1))
}

fn image_doc(path: &Path, a: &Analysis, page: Page, quality: u8, scratch: &Path) -> Result<Document> {
    let raw = if a.format == Format::Jpeg { raw_jpeg(path) } else { None };
    let (w, h, jpeg, gray) = match raw {
        Some(r) => r,
        None => {
            let img: DynamicImage = image_engine::decode_file(path, a.format, scratch)?;
            (img.width(), img.height(), image_engine::encode_jpeg(&img, quality)?, false)
        }
    };
    let (pw, ph, x, y, dw, dh) = layout(page, w, h);

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let image = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64,
            "ColorSpace" => if gray { "DeviceGray" } else { "DeviceRGB" }, "BitsPerComponent" => 8, "Filter" => "DCTDecode",
        },
        jpeg,
    );
    let image_id = doc.add_object(image);
    let content = format!("q {dw:.3} 0 0 {dh:.3} {x:.3} {y:.3} cm /Im0 Do Q");
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
        "MediaBox" => vec![0.into(), 0.into(), Object::Real(pw as f32), Object::Real(ph as f32)],
        "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => image_id } },
    });
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1 }));
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    Ok(doc)
}
