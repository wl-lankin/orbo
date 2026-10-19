//! Document engine: PDF tools (lopdf, pdf-extract, hayro) and text/office conversions.
//!
//! Params (`op`):
//!   pdf_txt                    text of all pages
//!   pdf_images  format         every page as jpg/png into a folder (hayro, pure Rust)
//!   pdf_split                  one single-page PDF per page into a folder
//!   pdf_rotate                 all pages 90 degrees clockwise
//!   pdf_strip                  drops Info dictionary and XMP metadata
//!   pdf_compress               recompresses streams and prunes unused objects
//!   convert     to             txt, md, html, rtf, docx, odt (office formats via macOS textutil)
//!   to_pdf                     plain-text PDF (cupsfilter on macOS, built-in writer otherwise)

use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use lopdf::{dictionary, Document, Object, Stream};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use super::{Ctx, Engine, Request};
use crate::format::Format;

pub struct DocumentEngine;

impl Engine for DocumentEngine {
    fn convert(&self, req: &Request, ctx: &Ctx) -> Result<()> {
        (ctx.progress)(0.05);
        let res = match req.str("op") {
            Some("pdf_txt") => pdf_txt(req),
            Some("pdf_images") => pdf_images(req, ctx),
            Some("pdf_split") => pdf_split(req, ctx),
            Some("pdf_rotate") => pdf_edit(req, rotate),
            Some("pdf_strip") => pdf_edit(req, strip),
            Some("pdf_compress") => pdf_compress(req),
            Some("convert") => convert(req),
            Some("to_pdf") => to_pdf(req),
            other => bail!("Unbekannte Dokumentaktion: {}", other.unwrap_or("-")),
        };
        if res.is_err() && req.output.is_dir() {
            let _ = fs::remove_dir_all(req.output);
        }
        (ctx.progress)(0.95);
        res
    }
}

fn check_cancel(ctx: &Ctx) -> Result<()> {
    if (ctx.cancelled)() {
        bail!("abgebrochen");
    }
    Ok(())
}

// ---------- PDF ----------

pub(crate) fn load_pdf(path: &Path) -> Result<Document> {
    let doc = Document::load(path).map_err(|e| anyhow!("PDF konnte nicht gelesen werden: {e}"))?;
    if doc.is_encrypted() {
        bail!("Die PDF ist passwortgeschützt");
    }
    Ok(doc)
}

pub(crate) fn save_pdf(doc: &mut Document, path: &Path) -> Result<()> {
    doc.save(path).map_err(|e| anyhow!("PDF konnte nicht geschrieben werden: {e}"))?;
    Ok(())
}

fn pdf_txt(req: &Request) -> Result<()> {
    // pdf-extract panics on some malformed fonts
    let text = catch_unwind(AssertUnwindSafe(|| pdf_extract::extract_text(req.input)))
        .map_err(|_| anyhow!("Der Text dieser PDF konnte nicht gelesen werden"))?
        .map_err(|e| anyhow!("Der Text dieser PDF konnte nicht gelesen werden: {e}"))?;
    let text = text.replace("\r\n", "\n");
    if text.trim().is_empty() {
        bail!("Diese PDF enthält keinen Text (vermutlich ein Scan)");
    }
    fs::write(req.output, text.trim().to_string() + "\n").context("Ausgabe konnte nicht geschrieben werden")
}

fn pdf_images(req: &Request, ctx: &Ctx) -> Result<()> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    use hayro::vello_cpu::color::palette::css::WHITE;
    use hayro::{render, PixmapSettings, RenderCache, RenderSettings};

    let fmt = req.str("format").unwrap_or("jpg");
    let data = fs::read(req.input).context("Datei konnte nicht gelesen werden")?;
    let pdf = Pdf::new(data).map_err(|_| anyhow!("PDF konnte nicht gelesen werden (passwortgeschützt oder beschädigt)"))?;
    let pages = pdf.pages();
    if pages.is_empty() {
        bail!("Die PDF hat keine Seiten");
    }
    fs::create_dir_all(req.output).context("Ordner konnte nicht angelegt werden")?;

    let cache = RenderCache::new();
    let settings = InterpreterSettings::default();
    let pixmap_settings = PixmapSettings { x_scale: 2.0, y_scale: 2.0, bg_color: WHITE };
    let total = pages.len();
    for (i, page) in pages.iter().enumerate() {
        check_cancel(ctx)?;
        let pixmap = catch_unwind(AssertUnwindSafe(|| render(page, &cache, &settings, &RenderSettings::default(), &pixmap_settings)))
            .map_err(|_| anyhow!("Seite {} konnte nicht gezeichnet werden", i + 1))?;
        let (w, h) = (pixmap.width() as u32, pixmap.height() as u32);
        let rgba = pixmap.data_as_u8_slice();
        let name = format!("Seite_{:03}.{fmt}", i + 1);
        let path = req.output.join(name);
        if fmt == "png" {
            image::save_buffer(&path, rgba, w, h, image::ColorType::Rgba8)?;
        } else {
            // The white background makes every pixel opaque, so dropping alpha is lossless.
            let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            let file = fs::File::create(&path)?;
            image::codecs::jpeg::JpegEncoder::new_with_quality(file, req.quality(88)).encode(&rgb, w, h, image::ExtendedColorType::Rgb8)?;
        }
        (ctx.progress)(0.1 + 0.85 * (i + 1) as f32 / total as f32);
    }
    Ok(())
}

fn pdf_split(req: &Request, ctx: &Ctx) -> Result<()> {
    let doc = load_pdf(req.input)?;
    let count = doc.get_pages().len() as u32;
    if count == 0 {
        bail!("Die PDF hat keine Seiten");
    }
    fs::create_dir_all(req.output).context("Ordner konnte nicht angelegt werden")?;
    for n in 1..=count {
        check_cancel(ctx)?;
        let mut single = doc.clone();
        let others: Vec<u32> = (1..=count).filter(|&p| p != n).collect();
        single.delete_pages(&others);
        single.prune_objects();
        save_pdf(&mut single, &req.output.join(format!("Seite_{n:03}.pdf")))?;
        (ctx.progress)(0.1 + 0.85 * n as f32 / count as f32);
    }
    Ok(())
}

fn pdf_edit(req: &Request, edit: fn(&mut Document) -> Result<()>) -> Result<()> {
    let mut doc = load_pdf(req.input)?;
    edit(&mut doc)?;
    save_pdf(&mut doc, req.output)
}

fn inherited_rotation(doc: &Document, page: lopdf::ObjectId) -> i64 {
    let mut id = page;
    for _ in 0..32 {
        let Ok(dict) = doc.get_dictionary(id) else { return 0 };
        if let Ok(r) = dict.get(b"Rotate").and_then(Object::as_i64) {
            return r;
        }
        match dict.get(b"Parent").and_then(Object::as_reference) {
            Ok(parent) => id = parent,
            Err(_) => return 0,
        }
    }
    0
}

fn rotate(doc: &mut Document) -> Result<()> {
    for (_, id) in doc.get_pages() {
        let next = (inherited_rotation(doc, id) + 90).rem_euclid(360);
        doc.get_dictionary_mut(id).map_err(|e| anyhow!("Seite beschädigt: {e}"))?.set("Rotate", next);
    }
    Ok(())
}

fn strip(doc: &mut Document) -> Result<()> {
    if let Ok(Object::Reference(id)) = doc.trailer.get(b"Info").cloned() {
        doc.objects.remove(&id);
    }
    doc.trailer.remove(b"Info");
    if let Ok(cat) = doc.catalog_mut() {
        cat.remove(b"Metadata");
        cat.remove(b"PieceInfo");
    }
    for (_, id) in doc.get_pages() {
        if let Ok(page) = doc.get_dictionary_mut(id) {
            page.remove(b"Metadata");
            page.remove(b"PieceInfo");
        }
    }
    doc.prune_objects();
    Ok(())
}

fn pdf_compress(req: &Request) -> Result<()> {
    let mut doc = load_pdf(req.input)?;
    doc.prune_objects();
    doc.delete_zero_length_streams();
    doc.renumber_objects();
    doc.compress();
    save_pdf(&mut doc, req.output)?;
    let before = fs::metadata(req.input)?.len();
    if fs::metadata(req.output)?.len() >= before {
        fs::copy(req.input, req.output).context("Ausgabe konnte nicht geschrieben werden")?;
    }
    Ok(())
}

// ---------- text and office conversions ----------

fn convert(req: &Request) -> Result<()> {
    let to = req.str("to").ok_or_else(|| anyhow!("Zielformat fehlt"))?;
    let from = req.analysis.format;
    let tmp = Tmp::new(req.output);
    match (from, to) {
        (Format::Txt, "md") => {
            fs::write(req.output, read_text(req.input)?).context("Ausgabe konnte nicht geschrieben werden")
        }
        (Format::Md, "txt") => fs::write(req.output, markdown_to_text(&read_text(req.input)?)).context("Ausgabe konnte nicht geschrieben werden"),
        (Format::Md, "html") => fs::write(req.output, markdown_to_html(&read_text(req.input)?)).context("Ausgabe konnte nicht geschrieben werden"),
        (Format::Md, _) => {
            let html = tmp.path("html");
            fs::write(&html, markdown_to_html(&read_text(req.input)?))?;
            textutil(&html, Format::Html, to, req.output)
        }
        (Format::Html, "md") => fs::write(req.output, html_to_markdown(&read_text(req.input)?)).context("Ausgabe konnte nicht geschrieben werden"),
        (_, "md") => {
            let html = tmp.path("html");
            textutil(req.input, from, "html", &html)?;
            fs::write(req.output, html_to_markdown(&read_text(&html)?)).context("Ausgabe konnte nicht geschrieben werden")
        }
        _ => textutil(req.input, from, to, req.output),
    }
}

/// Scratch files next to the output; removed on drop.
struct Tmp {
    base: PathBuf,
    used: std::cell::RefCell<Vec<PathBuf>>,
}

impl Tmp {
    fn new(output: &Path) -> Tmp {
        Tmp { base: output.to_path_buf(), used: Default::default() }
    }

    fn path(&self, ext: &str) -> PathBuf {
        let mut name = self.base.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".tmp.{ext}"));
        let p = self.base.with_file_name(name);
        self.used.borrow_mut().push(p.clone());
        p
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        for p in self.used.borrow().iter() {
            let _ = fs::remove_file(p);
        }
    }
}

fn textutil_format(f: Format) -> Option<&'static str> {
    Some(match f {
        Format::Txt => "txt",
        Format::Html => "html",
        Format::Rtf => "rtf",
        Format::Doc => "doc",
        Format::Docx => "docx",
        Format::Odt => "odt",
        _ => return None,
    })
}

fn textutil(input: &Path, from: Format, to: &str, output: &Path) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("Diese Umwandlung ist nur auf dem Mac möglich");
    }
    let in_fmt = textutil_format(from).ok_or_else(|| anyhow!("{} wird nicht unterstützt", from.id()))?;
    let mut cmd = Command::new("/usr/bin/textutil");
    cmd.args(["-convert", to, "-format", in_fmt]);
    if in_fmt == "txt" || to == "txt" {
        cmd.args(["-encoding", "UTF-8"]);
    }
    cmd.arg(input).arg("-output").arg(output);
    let out = cmd.output().context("textutil konnte nicht gestartet werden")?;
    if !out.status.success() || !output.exists() {
        let msg = String::from_utf8_lossy(&out.stderr);
        bail!("Das Dokument konnte nicht umgewandelt werden: {}", msg.trim());
    }
    Ok(())
}

/// UTF-8 with or without BOM; anything else is decoded as Latin-1.
fn read_text(path: &Path) -> Result<String> {
    let bytes = fs::read(path).context("Datei konnte nicht gelesen werden")?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&bytes);
    Ok(match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    })
}

// ---------- Markdown and HTML ----------

fn md_options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

fn markdown_to_html(md: &str) -> String {
    let mut body = String::new();
    pulldown_cmark::html::push_html(&mut body, Parser::new_ext(md, md_options()));
    format!("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title></title>\n</head>\n<body>\n{body}</body>\n</html>\n")
}

fn markdown_to_text(md: &str) -> String {
    let mut out = String::new();
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    let blank = |out: &mut String| {
        if !out.is_empty() && !out.ends_with("\n\n") {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
    };
    for ev in Parser::new_ext(md, md_options()) {
        match ev {
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak => out.push('\n'),
            Event::Rule => {
                blank(&mut out);
                out.push_str("----------\n\n");
            }
            Event::Start(Tag::List(start)) => list_stack.push(start),
            Event::End(TagEnd::List(_)) => {
                list_stack.pop();
                if list_stack.is_empty() {
                    blank(&mut out);
                }
            }
            Event::Start(Tag::Item) => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&"  ".repeat(list_stack.len().saturating_sub(1)));
                match list_stack.last_mut() {
                    Some(Some(n)) => {
                        out.push_str(&format!("{n}. "));
                        *n += 1;
                    }
                    _ => out.push_str("- "),
                }
            }
            Event::End(TagEnd::Item) => {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::BlockQuote(_) | TagEnd::Table) => {
                if list_stack.is_empty() {
                    blank(&mut out);
                }
            }
            Event::End(TagEnd::TableRow | TagEnd::TableHead) => out.push('\n'),
            Event::End(TagEnd::TableCell) => out.push('\t'),
            _ => {}
        }
    }
    out.trim_end().to_string() + "\n"
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let decoded = rest.find(';').filter(|&e| e <= 10).and_then(|e| {
            let name = &rest[1..e];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            };
            ch.map(|c| (c, e + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let i = lower.find(&format!("{name}="))? + name.len() + 1;
    let rest = &tag[i..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let end = rest[1..].find(quote)?;
        Some(decode_entities(&rest[1..1 + end]))
    } else {
        Some(rest.split(|c: char| c.is_whitespace() || c == '>').next()?.to_string())
    }
}

/// Small HTML to Markdown converter for what textutil and simple pages produce:
/// headings, paragraphs, bold, italic, lists, links.
fn html_to_markdown(html: &str) -> String {
    let mut out = String::new();
    let mut skip_depth = 0usize;
    let mut lists: Vec<(bool, u32)> = Vec::new();
    let mut links: Vec<Option<String>> = Vec::new();
    let mut pre = false;
    let mut rest = html;

    let ensure_blank = |out: &mut String| {
        let t = out.trim_end_matches(' ').len();
        out.truncate(t);
        if out.is_empty() {
            return;
        }
        while !out.ends_with("\n\n") {
            out.push('\n');
        }
    };
    let ensure_line = |out: &mut String| {
        let t = out.trim_end_matches(' ').len();
        out.truncate(t);
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
    };

    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            if skip_depth == 0 {
                push_text(&mut out, &decode_entities(rest), pre);
            }
            break;
        };
        if skip_depth == 0 {
            push_text(&mut out, &decode_entities(&rest[..lt]), pre);
        }
        rest = &rest[lt..];
        if let Some(r) = rest.strip_prefix("<!--") {
            rest = r.find("-->").map(|e| &r[e + 3..]).unwrap_or("");
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        let closing = tag.starts_with('/');
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();

        match (name.as_str(), closing) {
            ("head" | "style" | "script" | "title", false) => skip_depth += 1,
            ("head" | "style" | "script" | "title", true) => skip_depth = skip_depth.saturating_sub(1),
            _ if skip_depth > 0 => {}
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => {
                ensure_blank(&mut out);
                let level = name[1..].parse::<usize>().unwrap_or(1);
                out.push_str(&"#".repeat(level));
                out.push(' ');
            }
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "div" | "blockquote" | "table", true) => ensure_blank(&mut out),
            ("p" | "div" | "blockquote" | "table", false) => ensure_blank(&mut out),
            ("pre", false) => {
                ensure_blank(&mut out);
                pre = true;
            }
            ("pre", true) => {
                pre = false;
                ensure_blank(&mut out);
            }
            ("br", _) => out.push_str("  \n"),
            ("hr", _) => {
                ensure_blank(&mut out);
                out.push_str("---\n\n");
            }
            ("tr", true) => ensure_line(&mut out),
            ("td" | "th", true) => out.push_str(" | "),
            ("b" | "strong", _) => out.push_str("**"),
            ("i" | "em", _) => out.push('*'),
            ("ul", false) => lists.push((false, 0)),
            ("ol", false) => lists.push((true, 0)),
            ("ul" | "ol", true) => {
                lists.pop();
                if lists.is_empty() {
                    ensure_blank(&mut out);
                } else {
                    ensure_line(&mut out);
                }
            }
            ("li", false) => {
                ensure_line(&mut out);
                let depth = lists.len().saturating_sub(1);
                out.push_str(&"  ".repeat(depth));
                match lists.last_mut() {
                    Some((true, n)) => {
                        *n += 1;
                        out.push_str(&format!("{n}. "));
                    }
                    _ => out.push_str("- "),
                }
            }
            ("li", true) => ensure_line(&mut out),
            ("a", false) => {
                let href = attr(tag, "href");
                if href.is_some() {
                    out.push('[');
                }
                links.push(href);
            }
            ("a", true) => {
                if let Some(Some(href)) = links.pop() {
                    out.push_str(&format!("]({href})"));
                }
            }
            _ => {}
        }
    }
    tidy_markdown(&out)
}

fn push_text(out: &mut String, text: &str, pre: bool) {
    if pre {
        out.push_str(text);
        return;
    }
    for c in text.chars() {
        if c.is_whitespace() {
            if !out.is_empty() && !out.ends_with([' ', '\n']) {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
}

/// Drops empty emphasis pairs (textutil wraps whitespace in bold or italic) and collapses blank lines.
fn tidy_markdown(s: &str) -> String {
    let text = s.replace("****", "");
    let mut lines: Vec<String> = text.lines().map(|l| l.trim_end().to_string()).collect();
    let mut blank = 0;
    lines.retain(|l| {
        if l.is_empty() {
            blank += 1;
            blank < 2
        } else {
            blank = 0;
            true
        }
    });
    lines.join("\n").trim().to_string() + "\n"
}

// ---------- PDF from text ----------

fn to_pdf(req: &Request) -> Result<()> {
    let from = req.analysis.format;
    let tmp = Tmp::new(req.output);
    let text = match from {
        Format::Txt => read_text(req.input)?,
        Format::Md => markdown_to_text(&read_text(req.input)?),
        Format::Rtf | Format::Doc | Format::Docx | Format::Odt | Format::Html => {
            let t = tmp.path("txt");
            textutil(req.input, from, "txt", &t)?;
            read_text(&t)?
        }
        other => bail!("{} kann nicht in eine PDF umgewandelt werden", other.id()),
    };
    if text.trim().is_empty() {
        bail!("Das Dokument enthält keinen Text");
    }
    if cfg!(target_os = "macos") {
        let src = tmp.path("txt");
        fs::write(&src, &text)?;
        let out = Command::new("/usr/sbin/cupsfilter").arg(&src).output().context("cupsfilter konnte nicht gestartet werden")?;
        if !out.status.success() || !out.stdout.starts_with(b"%PDF") {
            bail!("Die PDF konnte nicht erzeugt werden");
        }
        fs::write(req.output, &out.stdout).context("Ausgabe konnte nicht geschrieben werden")
    } else {
        write_text_pdf(&text, req.output)
    }
}

/// Windows-1252 byte for a character, as used by the standard PDF fonts' WinAnsiEncoding.
fn win_ansi(c: char) -> u8 {
    match c {
        '\u{20}'..='\u{7E}' | '\u{A0}'..='\u{FF}' => c as u8,
        '€' => 0x80,
        '‚' => 0x82,
        '„' => 0x84,
        '…' => 0x85,
        '‘' => 0x91,
        '’' => 0x92,
        '“' => 0x93,
        '”' => 0x94,
        '•' => 0x95,
        '–' => 0x96,
        '—' => 0x97,
        _ => b'?',
    }
}

/// Monospace A4 text PDF; Courier makes wrapping exact without font metrics.
fn write_text_pdf(text: &str, output: &Path) -> Result<()> {
    const COLS: usize = 80;
    const LINES: usize = 60;
    let mut lines: Vec<Vec<u8>> = Vec::new();
    for raw in text.replace('\r', "").replace('\t', "    ").split('\n') {
        let bytes: Vec<u8> = raw.chars().map(win_ansi).collect();
        if bytes.is_empty() {
            lines.push(Vec::new());
        }
        for chunk in bytes.chunks(COLS) {
            lines.push(chunk.to_vec());
        }
    }

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier", "Encoding" => "WinAnsiEncoding" });
    let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
    let mut kids: Vec<Object> = Vec::new();
    for page_lines in lines.chunks(LINES) {
        let mut content = b"BT\n/F1 10 Tf\n12 TL\n56 790 Td\n".to_vec();
        for l in page_lines {
            content.push(b'(');
            for &b in l {
                if matches!(b, b'(' | b')' | b'\\') {
                    content.push(b'\\');
                }
                content.push(b);
            }
            content.extend_from_slice(b") Tj T*\n");
        }
        content.extend_from_slice(b"ET");
        let content_id = doc.add_object(Stream::new(dictionary! {}, content));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        });
        kids.push(page_id.into());
    }
    let count = kids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count, "Resources" => resources_id }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    doc.compress();
    save_pdf(&mut doc, output)
}
