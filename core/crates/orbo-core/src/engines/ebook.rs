//! E-book and subtitle engine, pure Rust.
//!
//! Books are read into a small model (title, author, chapters as Markdown) and written back out.
//! Params:
//!   to   epub, txt, md, html, pdf    book conversion (input: epub, fb2, mobi, azw3, txt, md, html, docx)
//!   to   pdf                         with a CBZ input: one page per image
//!   sub  srt, vtt, sbv, ass          subtitle conversion (input: srt, vtt, sbv, ass)

use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use lopdf::{dictionary, Document, Object, Stream};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::{Ctx, Engine, Request};
use crate::format::Format;

pub struct Engine_;

impl Engine for Engine_ {
    fn convert(&self, req: &Request, ctx: &Ctx) -> Result<()> {
        (ctx.progress)(0.05);
        let res = if let Some(to) = req.str("sub") { subtitles(req, to) } else { book(req, ctx) };
        (ctx.progress)(0.95);
        res
    }
}

fn write_out(req: &Request, data: impl AsRef<[u8]>) -> Result<()> {
    fs::write(req.output, data).context("Ausgabe konnte nicht geschrieben werden")
}

/// UTF-8 with or without BOM; anything else is decoded as Latin-1.
fn read_text(path: &Path) -> Result<String> {
    let bytes = fs::read(path).context("Datei konnte nicht gelesen werden")?;
    Ok(decode_text(&bytes))
}

fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

// ---------- model ----------

struct Chapter {
    title: String,
    md: String,
}

struct Book {
    title: String,
    author: String,
    lang: String,
    chapters: Vec<Chapter>,
}

fn file_stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn book(req: &Request, ctx: &Ctx) -> Result<()> {
    let from = req.analysis.format;
    let to = req.str("to").ok_or_else(|| anyhow!("Zielformat fehlt"))?;
    if from == Format::Cbz {
        return cbz_pdf(req, ctx);
    }
    let book = read_book(req.input, from)?;
    if book.chapters.iter().all(|c| c.md.trim().is_empty()) {
        bail!("Das Buch enthält keinen Text");
    }
    (ctx.progress)(0.5);
    match to {
        "epub" => write_epub(&book, req.output),
        "txt" => write_out(req, book_text(&book)),
        "md" => write_out(req, book_markdown(&book)),
        "html" => write_out(req, book_html(&book)),
        "pdf" => write_pdf(&book, req.output),
        other => bail!("Unbekanntes Zielformat: {other}"),
    }
}

fn read_book(path: &Path, from: Format) -> Result<Book> {
    let fallback = file_stem(path);
    match from {
        Format::Epub => read_epub(path, &fallback),
        Format::Fb2 => read_fb2(path, &fallback),
        Format::Mobi | Format::Azw3 => read_mobi(path, &fallback),
        Format::Docx => read_docx(path, &fallback),
        Format::Txt => Ok(from_markdown(&plain_to_markdown(&read_text(path)?), &fallback)),
        Format::Md => Ok(from_markdown(&read_text(path)?, &fallback)),
        Format::Html => Ok(from_markdown(&html_to_md(&read_text(path)?), &fallback)),
        other => bail!("{} kann nicht als Buch gelesen werden", other.id()),
    }
}

/// Running text of an e-book, DOCX, HTML, Markdown or text file, for the AI actions.
pub(crate) fn plain_text(path: &Path, from: Format) -> Result<String> {
    let book = read_book(path, from)?;
    let parts: Vec<String> = book.chapters.iter().map(|c| if c.title.is_empty() { c.md.clone() } else { format!("{}\n\n{}", c.title, c.md) }).collect();
    Ok(parts.join("\n\n"))
}

/// Plain text keeps its paragraphs; leading markers that Markdown would interpret are escaped.
fn plain_to_markdown(text: &str) -> String {
    text.replace("\r\n", "\n")
        .lines()
        .map(|l| {
            let t = l.trim_end();
            let lead = t.trim_start();
            if lead.starts_with(['#', '>', '-', '+', '*', '`']) || lead.chars().next().is_some_and(|c| c.is_ascii_digit()) && lead.contains(". ") {
                format!("\\{}", lead)
            } else {
                t.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn md_options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH
}

/// ATX headings (level, text) outside code fences, with the line index.
fn headings(md: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut fence = false;
    for (i, line) in md.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
        }
        if fence || line.len() - t.len() > 3 {
            continue;
        }
        let level = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&level) && t[level..].starts_with(' ') {
            let text = t[level..].trim().trim_end_matches('#').trim().to_string();
            if !text.is_empty() {
                out.push((i, level, text));
            }
        }
    }
    out
}

fn plain_title(md_heading: &str) -> String {
    md_heading.replace(['*', '_', '`'], "").replace('\\', "")
}

/// Splits Markdown into chapters at its main heading level.
fn from_markdown(md: &str, fallback: &str) -> Book {
    let hs = headings(md);
    let h1 = hs.iter().filter(|h| h.1 == 1).count();
    let h2 = hs.iter().filter(|h| h.1 == 2).count();
    let split_level = if h1 >= 2 || (h1 >= 1 && h2 == 0) || (h1 == 0 && h2 == 0) { 1 } else { 2 };
    let title = hs.iter().find(|h| h.1 == 1).map(|h| plain_title(&h.2)).unwrap_or_else(|| fallback.to_string());
    let lines: Vec<&str> = md.lines().collect();
    let mut cuts: Vec<usize> = hs.iter().filter(|h| h.1 == split_level || (h.1 < split_level)).map(|h| h.0).collect();
    cuts.retain(|&c| c > 0);
    cuts.insert(0, 0);
    cuts.dedup();
    let mut chapters = Vec::new();
    for (n, &start) in cuts.iter().enumerate() {
        let end = cuts.get(n + 1).copied().unwrap_or(lines.len());
        let text = lines[start..end].join("\n");
        if text.trim().is_empty() {
            continue;
        }
        let t = hs.iter().find(|h| h.0 >= start && h.0 < end).map(|h| plain_title(&h.2)).unwrap_or_else(|| title.clone());
        chapters.push(Chapter { title: t, md: text.trim().to_string() });
    }
    Book { title, author: String::new(), lang: String::new(), chapters }
}

// ---------- HTML to Markdown ----------

pub(crate) fn md_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '*' | '_' | '`' | '[' | ']' | '<') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

pub(crate) fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').filter(|&e| e <= 10);
        let decoded = end.and_then(|e| {
            let name = &rest[1..e];
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => '\u{a0}',
                "hellip" => '…',
                "mdash" => '—',
                "ndash" => '–',
                "lsquo" => '‘',
                "rsquo" => '’',
                "ldquo" => '“',
                "rdquo" => '”',
                "bdquo" => '„',
                "laquo" => '«',
                "raquo" => '»',
                "auml" => 'ä',
                "ouml" => 'ö',
                "uuml" => 'ü',
                "Auml" => 'Ä',
                "Ouml" => 'Ö',
                "Uuml" => 'Ü',
                "szlig" => 'ß',
                _ => {
                    let num = name.strip_prefix('#')?;
                    let code = match num.strip_prefix(['x', 'X']) {
                        Some(h) => u32::from_str_radix(h, 16).ok()?,
                        None => num.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
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

struct MdWriter {
    out: String,
    skip: usize,
    pre: bool,
    pending_space: bool,
    lists: Vec<(bool, u32)>,
}

impl MdWriter {
    fn block(&mut self) {
        let t = self.out.trim_end_matches([' ', '\t']).len();
        self.out.truncate(t);
        if !self.out.is_empty() && !self.out.ends_with("\n\n") {
            if !self.out.ends_with('\n') {
                self.out.push('\n');
            }
            self.out.push('\n');
        }
        self.pending_space = false;
    }

    fn line(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        self.pending_space = false;
    }

    fn text(&mut self, raw: &str) {
        if self.skip > 0 || raw.is_empty() {
            return;
        }
        let t = decode_entities(raw);
        if self.pre {
            self.out.push_str(&t);
            return;
        }
        let mut word = String::new();
        for c in t.chars() {
            if c.is_whitespace() && c != '\u{a0}' {
                self.pending_space = true;
                continue;
            }
            if self.pending_space && !self.out.is_empty() && !self.out.ends_with(['\n', ' ']) {
                self.out.push(' ');
            }
            self.pending_space = false;
            let line_start = self.out.is_empty() || self.out.ends_with('\n');
            let line = self.out.rsplit('\n').next().unwrap_or("");
            if (line_start && matches!(c, '#' | '>' | '+' | '-')) || (matches!(c, '.' | ')') && !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit())) {
                self.out.push('\\');
            }
            word.clear();
            word.push(c);
            self.out.push_str(&md_escape(&word));
        }
    }

    fn tag(&mut self, raw: &str) {
        let closing = raw.starts_with('/');
        let body = raw.trim_start_matches('/').trim_end_matches('/');
        let name = body.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("").to_lowercase();
        let name = name.rsplit(':').next().unwrap_or("").to_string();
        match name.as_str() {
            "script" | "style" | "head" | "svg" => {
                if closing {
                    self.skip = self.skip.saturating_sub(1);
                } else if !raw.ends_with('/') {
                    self.skip += 1;
                }
            }
            _ if self.skip > 0 => {}
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.block();
                if !closing {
                    let level = name[1..].parse::<usize>().unwrap_or(1);
                    self.out.push_str(&"#".repeat(level));
                    self.out.push(' ');
                }
            }
            "p" | "div" | "section" | "article" | "table" | "tr" | "blockquote" | "figure" | "body" | "aside" | "header" | "footer" => self.block(),
            "br" => {
                self.out.push_str("  \n");
                self.pending_space = false;
            }
            "ul" | "ol" => {
                if closing {
                    self.lists.pop();
                } else {
                    self.lists.push((name == "ol", 0));
                }
                if self.lists.is_empty() {
                    self.block();
                } else {
                    self.line();
                }
            }
            "li" => {
                self.line();
                if !closing {
                    let depth = self.lists.len().max(1) - 1;
                    self.out.push_str(&"  ".repeat(depth));
                    match self.lists.last_mut() {
                        Some((true, n)) => {
                            *n += 1;
                            self.out.push_str(&format!("{n}. "));
                        }
                        _ => self.out.push_str("- "),
                    }
                }
            }
            "b" | "strong" => {
                self.pending_space = false;
                self.out.push_str("**");
            }
            "i" | "em" => {
                self.pending_space = false;
                self.out.push('*');
            }
            "td" | "th" if closing => self.pending_space = true,
            "pre" => {
                self.block();
                if closing {
                    self.line();
                    self.out.push_str("```");
                    self.block();
                } else {
                    self.out.push_str("```\n");
                }
                self.pre = !closing;
            }
            "hr" => {
                self.block();
                self.out.push_str("---");
                self.block();
            }
            "pagebreak" => self.block(),
            _ => {}
        }
    }
}

pub(crate) fn html_to_md(html: &str) -> String {
    let mut w = MdWriter { out: String::new(), skip: 0, pre: false, pending_space: false, lists: Vec::new() };
    let mut rest = html;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            w.text(rest);
            break;
        };
        w.text(&rest[..lt]);
        rest = &rest[lt..];
        if let Some(r) = rest.strip_prefix("<!--") {
            rest = r.find("-->").map_or("", |e| &r[e + 3..]);
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>").unwrap_or(rest.len());
            w.text(&rest[9..end]);
            rest = rest.get(end + 3..).unwrap_or("");
            continue;
        }
        // '>' inside quoted attribute values does not end the tag
        let mut quote = None;
        let mut gt = None;
        for (i, c) in rest.char_indices().skip(1) {
            match (quote, c) {
                (None, '"' | '\'') => quote = Some(c),
                (Some(q), c) if c == q => quote = None,
                (None, '>') => {
                    gt = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let Some(gt) = gt else { break };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        if tag.starts_with(['!', '?']) {
            continue;
        }
        w.tag(tag);
    }
    tidy(&w.out)
}

fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blanks = 0;
    for line in s.lines() {
        let l = line.trim_end_matches([' ', '\t']);
        let hard = line.ends_with("  ") && !l.is_empty();
        if l.is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        out.push_str(l);
        if hard {
            out.push_str("  ");
        }
        out.push('\n');
    }
    out.trim().to_string()
}

// ---------- readers ----------

fn zip_read(z: &mut ZipArchive<fs::File>, name: &str) -> Result<Vec<u8>> {
    let mut f = z.by_name(name).map_err(|_| anyhow!("Die Datei {name} fehlt im Archiv"))?;
    let mut v = Vec::new();
    f.read_to_end(&mut v)?;
    Ok(v)
}

fn open_zip(path: &Path) -> Result<ZipArchive<fs::File>> {
    let f = fs::File::open(path).context("Datei konnte nicht geöffnet werden")?;
    ZipArchive::new(f).map_err(|_| anyhow!("Die Datei ist kein gültiges Archiv und damit nicht lesbar"))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn resolve(base_dir: &str, href: &str) -> String {
    let href = percent_decode(href.split('#').next().unwrap_or(""));
    let mut parts: Vec<&str> = if href.starts_with('/') { Vec::new() } else { base_dir.split('/').filter(|s| !s.is_empty()).collect() };
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn xml_doc(text: &str) -> Result<roxmltree::Document<'_>> {
    let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    roxmltree::Document::parse_with_options(text, opts).map_err(|e| anyhow!("Das XML ist fehlerhaft: {e}"))
}

fn local<'a>(n: &roxmltree::Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn node_text(n: roxmltree::Node) -> String {
    n.descendants().filter(|d| d.is_text()).map(|d| d.text().unwrap_or("")).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn read_epub(path: &Path, fallback: &str) -> Result<Book> {
    let mut z = open_zip(path)?;
    if let Ok(enc) = zip_read(&mut z, "META-INF/encryption.xml") {
        let enc = String::from_utf8_lossy(&enc);
        let drm = enc.split("Algorithm=\"").skip(1).any(|a| !(a.starts_with("http://www.idpf.org/2008/embedding") || a.starts_with("http://ns.adobe.com/pdf/enc#RC")));
        if drm {
            bail!("Diese E-Book-Datei ist durch DRM geschützt und lässt sich nicht umwandeln");
        }
    }
    let container = zip_read(&mut z, "META-INF/container.xml").map_err(|_| anyhow!("Keine gültige EPUB-Datei: container.xml fehlt"))?;
    let container = String::from_utf8_lossy(&container).into_owned();
    let cdoc = xml_doc(&container)?;
    let opf_path = cdoc
        .descendants()
        .find(|n| local(n) == "rootfile")
        .and_then(|n| n.attribute("full-path"))
        .ok_or_else(|| anyhow!("Keine gültige EPUB-Datei: Paketdatei nicht gefunden"))?
        .to_string();
    let opf = String::from_utf8_lossy(&zip_read(&mut z, &opf_path)?).into_owned();
    let odoc = xml_doc(&opf)?;
    let base = opf_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("").to_string();

    let meta = |name: &str| odoc.descendants().find(|n| local(n) == name && n.parent().is_some_and(|p| local(&p) == "metadata")).map(node_text).unwrap_or_default();
    let manifest: Vec<(String, String)> =
        odoc.descendants().filter(|n| local(n) == "item").filter_map(|n| Some((n.attribute("id")?.to_string(), n.attribute("href")?.to_string()))).collect();
    let mut chapters = Vec::new();
    for item in odoc.descendants().filter(|n| local(n) == "itemref") {
        let Some(id) = item.attribute("idref") else { continue };
        let Some((_, href)) = manifest.iter().find(|(i, _)| i == id) else { continue };
        let name = resolve(&base, href);
        let Ok(bytes) = zip_read(&mut z, &name) else { continue };
        let md = html_to_md(&decode_text(&bytes));
        if md.trim().is_empty() {
            continue;
        }
        let title = headings(&md).first().map(|h| plain_title(&h.2)).unwrap_or_else(|| format!("Kapitel {}", chapters.len() + 1));
        chapters.push(Chapter { title, md });
    }
    if chapters.is_empty() {
        bail!("Das E-Book enthält keinen lesbaren Text");
    }
    let title = Some(meta("title")).filter(|t| !t.is_empty()).unwrap_or_else(|| fallback.to_string());
    Ok(Book { title, author: meta("creator"), lang: meta("language"), chapters })
}

const CP1251_HIGH: &str = "ЂЃ‚ѓ„…†‡€‰Љ‹ЊЌЋЏђ‘’“”•–—\u{98}™љ›њќћџ\u{a0}ЎўЈ¤Ґ¦§Ё©Є«¬\u{ad}®Ї°±Ііґµ¶·ё№є»јЅѕї";

fn decode_fb2(bytes: &[u8]) -> String {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]).to_lowercase();
    if head.contains("windows-1251") || head.contains("cp1251") {
        let high: Vec<char> = CP1251_HIGH.chars().collect();
        return bytes
            .iter()
            .map(|&b| match b {
                0..=0x7f => b as char,
                0x80..=0xbf => high[(b - 0x80) as usize],
                _ => char::from_u32(0x410 + (b - 0xc0) as u32).unwrap_or('?'),
            })
            .collect();
    }
    if head.contains("iso-8859-1") || head.contains("windows-1252") {
        return bytes.iter().map(|&b| b as char).collect();
    }
    decode_text(bytes)
}

fn fb2_inline(n: roxmltree::Node, out: &mut String) {
    for c in n.children() {
        if c.is_text() {
            out.push_str(&md_escape(c.text().unwrap_or("")));
        } else if c.is_element() {
            let mark = match local(&c) {
                "emphasis" => "*",
                "strong" => "**",
                "strikethrough" => "~~",
                "code" => "`",
                _ => "",
            };
            out.push_str(mark);
            fb2_inline(c, out);
            out.push_str(mark);
        }
    }
}

fn fb2_blocks(n: roxmltree::Node, depth: usize, out: &mut String) {
    for c in n.children().filter(|c| c.is_element()) {
        match local(&c) {
            "title" => {
                let mut t = String::new();
                for p in c.children().filter(|p| p.is_element()) {
                    if !t.is_empty() {
                        t.push(' ');
                    }
                    fb2_inline(p, &mut t);
                }
                out.push_str(&format!("{} {}\n\n", "#".repeat(depth.clamp(1, 6)), t.trim()));
            }
            "section" => fb2_blocks(c, depth + 1, out),
            "p" => {
                let mut t = String::new();
                fb2_inline(c, &mut t);
                out.push_str(t.trim());
                out.push_str("\n\n");
            }
            "subtitle" => {
                let mut t = String::new();
                fb2_inline(c, &mut t);
                out.push_str(&format!("**{}**\n\n", t.trim()));
            }
            "empty-line" => out.push_str("&nbsp;\n\n"),
            "text-author" => {
                let mut t = String::new();
                fb2_inline(c, &mut t);
                out.push_str(&format!("\u{2014} {}\n\n", t.trim()));
            }
            "poem" | "epigraph" | "cite" | "stanza" | "annotation" => {
                if local(&c) == "stanza" {
                    let lines: Vec<String> = c
                        .children()
                        .filter(|v| v.is_element() && local(v) == "v")
                        .map(|v| {
                            let mut t = String::new();
                            fb2_inline(v, &mut t);
                            t.trim().to_string()
                        })
                        .collect();
                    out.push_str(&lines.join("  \n"));
                    out.push_str("\n\n");
                } else {
                    fb2_blocks(c, depth + 1, out);
                }
            }
            _ => {}
        }
    }
}

fn read_fb2(path: &Path, fallback: &str) -> Result<Book> {
    let text = decode_fb2(&fs::read(path).context("Datei konnte nicht gelesen werden")?);
    let doc = xml_doc(&text)?;
    let info = doc.descendants().find(|n| local(n) == "title-info");
    let field = |name: &str| info.and_then(|i| i.descendants().find(|n| local(n) == name)).map(node_text).unwrap_or_default();
    let author = info
        .and_then(|i| i.descendants().find(|n| local(n) == "author"))
        .map(|a| ["first-name", "middle-name", "last-name"].iter().filter_map(|k| a.children().find(|c| local(c) == *k).map(node_text)).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let mut chapters = Vec::new();
    for body in doc.descendants().filter(|n| local(n) == "body" && n.attribute("name") != Some("notes")) {
        let sections: Vec<_> = body.children().filter(|c| c.is_element() && local(c) == "section").collect();
        let units = if sections.is_empty() { vec![body] } else { sections };
        for s in units {
            let mut md = String::new();
            fb2_blocks(s, 1, &mut md);
            if md.trim().is_empty() {
                continue;
            }
            let title = headings(&md).first().map(|h| plain_title(&h.2)).unwrap_or_else(|| format!("Kapitel {}", chapters.len() + 1));
            chapters.push(Chapter { title, md: md.trim().to_string() });
        }
    }
    if chapters.is_empty() {
        bail!("Das E-Book enthält keinen lesbaren Text");
    }
    let title = Some(field("book-title")).filter(|t| !t.is_empty()).unwrap_or_else(|| fallback.to_string());
    Ok(Book { title, author, lang: field("lang"), chapters })
}

fn be16(d: &[u8], o: usize) -> Option<usize> {
    Some(u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?) as usize)
}

fn be32(d: &[u8], o: usize) -> Option<usize> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?) as usize)
}

fn palmdoc_decompress(d: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(d.len() * 3);
    let mut i = 0;
    while i < d.len() {
        let c = d[i];
        i += 1;
        match c {
            0 | 0x09..=0x7f => out.push(c),
            1..=8 => {
                let n = (c as usize).min(d.len() - i);
                out.extend_from_slice(&d[i..i + n]);
                i += n;
            }
            0x80..=0xbf => {
                let Some(&c2) = d.get(i) else { break };
                i += 1;
                let v = (((c & 0x3f) as usize) << 8) | c2 as usize;
                let (dist, len) = (v >> 3, (v & 7) + 3);
                if dist == 0 || dist > out.len() {
                    continue;
                }
                for _ in 0..len {
                    out.push(out[out.len() - dist]);
                }
            }
            _ => {
                out.push(b' ');
                out.push(c ^ 0x80);
            }
        }
    }
    out
}

/// Number of trailing bytes in a text record that are not book text (multibyte overlap and indexing data).
fn trailing_size(rec: &[u8], flags: usize) -> usize {
    let mut total = 0;
    for bit in 1..16 {
        if flags & (1 << bit) == 0 {
            continue;
        }
        let end = rec.len().saturating_sub(total);
        let mut value = 0usize;
        let mut shift = 0;
        let mut pos = end;
        while pos > 0 && shift < 28 {
            pos -= 1;
            let b = rec[pos];
            value |= ((b & 0x7f) as usize) << shift;
            shift += 7;
            if b & 0x80 != 0 {
                break;
            }
        }
        total += value;
    }
    if flags & 1 != 0 {
        let end = rec.len().saturating_sub(total);
        if end > 0 {
            total += (rec[end - 1] & 3) as usize + 1;
        }
    }
    total.min(rec.len())
}

fn read_mobi(path: &Path, fallback: &str) -> Result<Book> {
    let d = fs::read(path).context("Datei konnte nicht gelesen werden")?;
    let bad = || anyhow!("Die Datei ist keine lesbare MOBI-Datei");
    if d.len() < 80 || !matches!(&d[60..68], b"BOOKMOBI" | b"TEXtREAd") {
        return Err(bad());
    }
    let n_rec = be16(&d, 76).ok_or_else(bad)?;
    let offsets: Vec<usize> = (0..n_rec).filter_map(|i| be32(&d, 78 + i * 8)).collect();
    if offsets.len() < 2 {
        return Err(bad());
    }
    let rec = |i: usize| -> &[u8] {
        let start = offsets[i].min(d.len());
        let end = offsets.get(i + 1).copied().unwrap_or(d.len()).min(d.len());
        &d[start..end.max(start)]
    };
    let r0 = rec(0);
    let compression = be16(r0, 0).ok_or_else(bad)?;
    let text_records = be16(r0, 8).ok_or_else(bad)?;
    let encryption = be16(r0, 12).unwrap_or(0);
    let is_mobi = r0.get(16..20) == Some(b"MOBI");
    let drm_offset = if is_mobi { be32(r0, 0xA8).unwrap_or(0xFFFF_FFFF) } else { 0xFFFF_FFFF };
    if encryption != 0 || (drm_offset != 0xFFFF_FFFF && drm_offset != 0) {
        bail!("Diese E-Book-Datei ist durch DRM geschützt und lässt sich nicht umwandeln");
    }
    if compression != 1 && compression != 2 {
        bail!("Diese MOBI-Datei nutzt eine nicht unterstützte Kompression (HUFF/CDIC)");
    }
    let mobi_len = if is_mobi { be32(r0, 20).unwrap_or(0) } else { 0 };
    let flags = if is_mobi && mobi_len >= 0xE4 { be32(r0, 0xF0).unwrap_or(0) & 0xFFFF } else { 0 };
    let utf8 = is_mobi && be32(r0, 0x1C) == Some(65001);

    let mut raw = Vec::new();
    for i in 1..=text_records.min(offsets.len() - 1) {
        let r = rec(i);
        let r = &r[..r.len() - trailing_size(r, flags)];
        if compression == 2 {
            raw.extend(palmdoc_decompress(r));
        } else {
            raw.extend_from_slice(r);
        }
    }
    let html: String = if utf8 { String::from_utf8_lossy(&raw).into_owned() } else { raw.iter().map(|&b| b as char).collect() };

    let mut title = String::new();
    let mut author = String::new();
    if is_mobi {
        if let (Some(off), Some(len)) = (be32(r0, 0x54), be32(r0, 0x58)) {
            if let Some(b) = r0.get(off..off + len) {
                title = String::from_utf8_lossy(b).trim().to_string();
            }
        }
        if be32(r0, 0x80).unwrap_or(0) & 0x40 != 0 {
            let exth = 16 + mobi_len;
            if r0.get(exth..exth + 4) == Some(b"EXTH") {
                let count = be32(r0, exth + 8).unwrap_or(0);
                let mut p = exth + 12;
                for _ in 0..count {
                    let (Some(kind), Some(len)) = (be32(r0, p), be32(r0, p + 4)) else { break };
                    if len < 8 {
                        break;
                    }
                    let data = r0.get(p + 8..p + len).unwrap_or(&[]);
                    let s = String::from_utf8_lossy(data).trim().to_string();
                    match kind {
                        100 if author.is_empty() => author = s,
                        503 if !s.is_empty() => title = s,
                        _ => {}
                    }
                    p += len;
                }
            }
        }
    }
    if title.is_empty() {
        title = fallback.to_string();
    }

    let lower = html.to_lowercase();
    let mut pieces = Vec::new();
    let mut last = 0;
    while let Some(i) = lower[last..].find("<mbp:pagebreak") {
        pieces.push(&html[last..last + i]);
        last += i + "<mbp:pagebreak".len();
    }
    pieces.push(&html[last.min(html.len())..]);
    let mut chapters: Vec<Chapter> = Vec::new();
    for p in pieces {
        let md = html_to_md(p);
        if md.trim().is_empty() {
            continue;
        }
        let t = headings(&md).first().map(|h| plain_title(&h.2)).unwrap_or_else(|| format!("Kapitel {}", chapters.len() + 1));
        chapters.push(Chapter { title: t, md });
    }
    if chapters.len() <= 1 {
        let all = chapters.pop().map(|c| c.md).unwrap_or_default();
        chapters = from_markdown(&all, &title).chapters;
    }
    if chapters.is_empty() {
        bail!("Das E-Book enthält keinen lesbaren Text");
    }
    Ok(Book { title, author, lang: String::new(), chapters })
}

fn read_docx(path: &Path, fallback: &str) -> Result<Book> {
    let mut z = open_zip(path)?;
    let xml = String::from_utf8_lossy(&zip_read(&mut z, "word/document.xml")?).into_owned();
    let doc = xml_doc(&xml)?;
    let mut md = String::new();
    for p in doc.descendants().filter(|n| local(n) == "p") {
        let style = p.descendants().find(|n| local(n) == "pStyle").and_then(|n| n.attributes().find(|a| a.name() == "val")).map(|a| a.value().to_lowercase()).unwrap_or_default();
        let level = if style == "title" || style == "titel" {
            1
        } else if style.contains("heading") || style.contains("berschrift") {
            style.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect::<String>().parse::<usize>().unwrap_or(1).clamp(1, 6)
        } else {
            0
        };
        let mut t = String::new();
        for n in p.descendants() {
            match local(&n) {
                "t" => t.push_str(&md_escape(&node_text_raw(n))),
                "tab" => t.push(' '),
                "br" => t.push(' '),
                _ => {}
            }
        }
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        if level > 0 {
            md.push_str(&format!("{} {}\n\n", "#".repeat(level), t));
        } else if p.descendants().any(|n| local(&n) == "numPr") {
            md.push_str(&format!("- {t}\n"));
        } else {
            md.push_str(&format!("\n{t}\n\n"));
        }
    }
    if md.trim().is_empty() {
        bail!("Das Dokument enthält keinen Text");
    }
    Ok(from_markdown(&tidy(&md), fallback))
}

fn node_text_raw(n: roxmltree::Node) -> String {
    n.children().filter(|c| c.is_text()).map(|c| c.text().unwrap_or("")).collect()
}

// ---------- writers: text, markdown, html ----------

fn chapter_text(md: &str) -> String {
    let mut out = String::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
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
            Event::Start(Tag::List(first)) => lists.push(first),
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                if lists.is_empty() {
                    blank(&mut out);
                }
            }
            Event::Start(Tag::Item) => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&"  ".repeat(lists.len().saturating_sub(1)));
                match lists.last_mut() {
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
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::BlockQuote(_) | TagEnd::Table) => blank(&mut out),
            Event::End(TagEnd::TableCell) => out.push('\t'),
            Event::End(TagEnd::TableRow | TagEnd::TableHead) => out.push('\n'),
            _ => {}
        }
    }
    out.trim().to_string()
}

fn book_text(book: &Book) -> String {
    let parts: Vec<String> = book.chapters.iter().map(|c| chapter_text(&c.md)).filter(|t| !t.is_empty()).collect();
    parts.join("\n\n\n") + "\n"
}

fn book_markdown(book: &Book) -> String {
    let parts: Vec<&str> = book.chapters.iter().map(|c| c.md.trim()).filter(|t| !t.is_empty()).collect();
    parts.join("\n\n") + "\n"
}

fn render_html(md: &str) -> String {
    let events = Parser::new_ext(md, md_options()).map(|e| match e {
        Event::Html(t) | Event::InlineHtml(t) => Event::Text(t),
        e => e,
    });
    let mut body = String::new();
    pulldown_cmark::html::push_html(&mut body, events);
    body
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const BOOK_CSS: &str = "body { font-family: serif; line-height: 1.5; margin: 5%; }\nh1, h2, h3 { font-family: sans-serif; line-height: 1.2; }\np { margin: 0 0 0.8em 0; }\npre { white-space: pre-wrap; }\nblockquote { margin-left: 1.5em; font-style: italic; }\n";

fn lang_or_default(book: &Book) -> &str {
    if book.lang.is_empty() {
        "de"
    } else {
        &book.lang
    }
}

fn book_html(book: &Book) -> String {
    let mut s = format!(
        "<!DOCTYPE html>\n<html lang=\"{}\">\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n<style>\n{}</style>\n</head>\n<body>\n",
        xml_escape(lang_or_default(book)),
        xml_escape(&book.title),
        BOOK_CSS
    );
    for c in &book.chapters {
        s.push_str("<section>\n");
        s.push_str(&render_html(&c.md));
        s.push_str("</section>\n");
    }
    s.push_str("</body>\n</html>\n");
    s
}

// ---------- EPUB writer ----------

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((yoe + era * 400 + (m <= 2) as i64), m, d)
}

fn utc_stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86400));
    let t = secs.rem_euclid(86400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", t / 3600, t % 3600 / 60, t % 60)
}

fn book_uuid(book: &Book) -> String {
    use std::hash::{Hash, Hasher};
    let mut a = std::collections::hash_map::DefaultHasher::new();
    book.title.hash(&mut a);
    book.author.hash(&mut a);
    let mut b = std::collections::hash_map::DefaultHasher::new();
    for c in &book.chapters {
        c.md.hash(&mut b);
    }
    let h = format!("{:016x}{:016x}", a.finish(), b.finish());
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn write_epub(book: &Book, output: &Path) -> Result<()> {
    let file = fs::File::create(output).context("Ausgabe konnte nicht geschrieben werden")?;
    let mut z = ZipWriter::new(file);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut put = |name: &str, data: &str, opts: SimpleFileOptions| -> Result<()> {
        z.start_file(name, opts)?;
        z.write_all(data.as_bytes())?;
        Ok(())
    };
    let lang = xml_escape(lang_or_default(book));
    let title = xml_escape(&book.title);
    put("mimetype", "application/epub+zip", stored)?;
    put(
        "META-INF/container.xml",
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n<rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles>\n</container>\n",
        deflated,
    )?;
    put("OEBPS/style.css", BOOK_CSS, deflated)?;

    let uuid = book_uuid(book);
    let mut manifest = String::from("<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n");
    let mut spine = String::new();
    let mut nav = String::new();
    let mut ncx = String::new();
    for (i, c) in book.chapters.iter().enumerate() {
        let name = format!("ch{:03}.xhtml", i + 1);
        let page = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"{lang}\" xml:lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\"/>\n<title>{}</title>\n<link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/>\n</head>\n<body>\n{}</body>\n</html>\n",
            xml_escape(&c.title),
            render_html(&c.md)
        );
        put(&format!("OEBPS/{name}"), &page, deflated)?;
        manifest.push_str(&format!("<item id=\"c{i}\" href=\"{name}\" media-type=\"application/xhtml+xml\"/>\n"));
        spine.push_str(&format!("<itemref idref=\"c{i}\"/>\n"));
        nav.push_str(&format!("<li><a href=\"{name}\">{}</a></li>\n", xml_escape(&c.title)));
        ncx.push_str(&format!(
            "<navPoint id=\"n{i}\" playOrder=\"{}\"><navLabel><text>{}</text></navLabel><content src=\"{name}\"/></navPoint>\n",
            i + 1,
            xml_escape(&c.title)
        ));
    }
    let author = if book.author.is_empty() { String::new() } else { format!("<dc:creator>{}</dc:creator>\n", xml_escape(&book.author)) };
    let opf = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bookid\" xml:lang=\"{lang}\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n<dc:identifier id=\"bookid\">urn:uuid:{uuid}</dc:identifier>\n<dc:title>{title}</dc:title>\n{author}<dc:language>{lang}</dc:language>\n<meta property=\"dcterms:modified\">{}</meta>\n</metadata>\n<manifest>\n{manifest}</manifest>\n<spine toc=\"ncx\">\n{spine}</spine>\n</package>\n",
        utc_stamp()
    );
    put("OEBPS/content.opf", &opf, deflated)?;
    put(
        "OEBPS/nav.xhtml",
        &format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"{lang}\" xml:lang=\"{lang}\">\n<head><meta charset=\"utf-8\"/><title>Inhalt</title></head>\n<body>\n<nav epub:type=\"toc\" id=\"toc\">\n<h1>Inhalt</h1>\n<ol>\n{nav}</ol>\n</nav>\n</body>\n</html>\n"),
        deflated,
    )?;
    put(
        "OEBPS/toc.ncx",
        &format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\">\n<head><meta name=\"dtb:uid\" content=\"urn:uuid:{uuid}\"/></head>\n<docTitle><text>{title}</text></docTitle>\n<navMap>\n{ncx}</navMap>\n</ncx>\n"),
        deflated,
    )?;
    z.finish()?;
    Ok(())
}

// ---------- PDF writer (text layout) ----------

const HELV: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722,
    278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556,
    333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

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
        '\u{2009}' | '\u{202F}' | '\u{2003}' | '\u{2002}' => b' ',
        _ => b'?',
    }
}

fn glyph_width(b: u8, mono: bool) -> f32 {
    if mono {
        return 600.0;
    }
    match b {
        32..=126 => HELV[(b - 32) as usize] as f32,
        0xC0..=0xC5 => HELV[(b'A' - 32) as usize] as f32,
        0xE0..=0xE5 => HELV[(b'a' - 32) as usize] as f32,
        0xC8..=0xCB => HELV[(b'E' - 32) as usize] as f32,
        0xE8..=0xEB => HELV[(b'e' - 32) as usize] as f32,
        0xD2..=0xD6 => HELV[(b'O' - 32) as usize] as f32,
        0xF2..=0xF6 => HELV[(b'o' - 32) as usize] as f32,
        0xD9..=0xDC => HELV[(b'U' - 32) as usize] as f32,
        0xF9..=0xFC => HELV[(b'u' - 32) as usize] as f32,
        _ => 556.0,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Style {
    H1,
    H2,
    H3,
    Body,
    Item,
    Code,
}

impl Style {
    fn size(self) -> f32 {
        match self {
            Style::H1 => 20.0,
            Style::H2 => 15.0,
            Style::H3 => 12.5,
            Style::Body | Style::Item => 11.0,
            Style::Code => 9.5,
        }
    }

    fn font(self) -> &'static str {
        match self {
            Style::H1 | Style::H2 | Style::H3 => "F2",
            Style::Code => "F3",
            _ => "F1",
        }
    }
}

fn pdf_blocks(md: &str) -> Vec<(Style, String)> {
    let mut blocks = Vec::new();
    let mut cur: Option<(Style, String)> = None;
    let mut in_item = 0usize;
    for ev in Parser::new_ext(md, md_options()) {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                let s = match level {
                    pulldown_cmark::HeadingLevel::H1 => Style::H1,
                    pulldown_cmark::HeadingLevel::H2 => Style::H2,
                    _ => Style::H3,
                };
                cur = Some((s, String::new()));
            }
            Event::Start(Tag::Item) => {
                in_item += 1;
                cur = Some((Style::Item, "\u{2022} ".into()));
            }
            Event::Start(Tag::Paragraph) if in_item == 0 => cur = Some((Style::Body, String::new())),
            Event::Start(Tag::CodeBlock(_)) => cur = Some((Style::Code, String::new())),
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => {
                if let Some((_, s)) = cur.as_mut() {
                    s.push_str(&t)
                }
            }
            Event::SoftBreak => {
                if let Some((st, s)) = cur.as_mut() {
                    s.push(if *st == Style::Code { '\n' } else { ' ' })
                }
            }
            Event::HardBreak => {
                if let Some((_, s)) = cur.as_mut() {
                    s.push('\n')
                }
            }
            Event::End(TagEnd::Item) => {
                in_item = in_item.saturating_sub(1);
                if let Some(b) = cur.take() {
                    blocks.push(b);
                }
            }
            Event::End(TagEnd::Heading(_) | TagEnd::CodeBlock) => {
                if let Some(b) = cur.take() {
                    blocks.push(b);
                }
            }
            Event::End(TagEnd::Paragraph) if in_item == 0 => {
                if let Some(b) = cur.take() {
                    blocks.push(b);
                }
            }
            Event::Rule => blocks.push((Style::Body, "* * *".into())),
            _ => {}
        }
    }
    blocks
}

fn wrap(text: &str, style: Style, max: f32) -> Vec<Vec<u8>> {
    let size = style.size();
    let mono = style == Style::Code;
    let scale = if style.font() == "F2" { 1.06 } else { 1.0 };
    let cw = |b: u8| glyph_width(b, mono) * size / 1000.0 * scale;
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line: Vec<u8> = Vec::new();
        let mut w = 0.0f32;
        let words: Vec<Vec<u8>> = if mono {
            vec![para.chars().map(win_ansi).collect()]
        } else {
            para.split(' ').filter(|s| !s.is_empty()).map(|s| s.chars().map(win_ansi).collect()).collect()
        };
        for word in words {
            if !mono && !line.is_empty() {
                let ww: f32 = word.iter().map(|&b| cw(b)).sum();
                if w + cw(b' ') + ww <= max {
                    line.push(b' ');
                    w += cw(b' ');
                } else {
                    lines.push(std::mem::take(&mut line));
                    w = 0.0;
                }
            }
            // a word wider than the line is broken by character
            for &b in &word {
                if w + cw(b) > max && !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                    w = 0.0;
                }
                line.push(b);
                w += cw(b);
            }
        }
        lines.push(line);
    }
    lines
}

fn pdf_string(bytes: &[u8]) -> Vec<u8> {
    let mut v = vec![b'('];
    for &b in bytes {
        if matches!(b, b'(' | b')' | b'\\') {
            v.push(b'\\');
        }
        v.push(b);
    }
    v.push(b')');
    v
}

fn write_pdf(book: &Book, output: &Path) -> Result<()> {
    const W: f32 = 595.0;
    const H: f32 = 842.0;
    const MX: f32 = 66.0;
    const TOP: f32 = 790.0;
    const BOTTOM: f32 = 70.0;
    let mut pages: Vec<Vec<u8>> = Vec::new();
    let mut content: Vec<u8> = Vec::new();
    let mut y = TOP;
    let mut fresh = true;
    let finish_page = |content: &mut Vec<u8>, pages: &mut Vec<Vec<u8>>| {
        if !content.is_empty() {
            pages.push(std::mem::take(content));
        }
    };
    let mut first_block = true;
    for chapter in &book.chapters {
        let blocks = pdf_blocks(&chapter.md);
        if !first_block {
            finish_page(&mut content, &mut pages);
            y = TOP;
            fresh = true;
        }
        first_block = false;
        for (style, text) in blocks {
            let size = style.size();
            let lh = size * 1.38;
            let indent = if style == Style::Item { 14.0 } else { 0.0 };
            let lines = wrap(&text, style, W - 2.0 * MX - indent);
            let gap_before = match style {
                Style::H1 => 8.0,
                Style::H2 | Style::H3 => 10.0,
                _ => 0.0,
            };
            if !fresh {
                y -= gap_before;
            }
            for (n, l) in lines.iter().enumerate() {
                if y - lh < BOTTOM {
                    finish_page(&mut content, &mut pages);
                    y = TOP;
                }
                y -= lh;
                fresh = false;
                if l.is_empty() {
                    continue;
                }
                let x = MX + if style == Style::Item && n > 0 { indent } else { 0.0 };
                content.extend_from_slice(format!("BT /{} {} Tf {:.1} {:.1} Td ", style.font(), size, x, y).as_bytes());
                content.extend(pdf_string(l));
                content.extend_from_slice(b" Tj ET\n");
            }
            y -= match style {
                Style::H1 | Style::H2 | Style::H3 => 6.0,
                Style::Item => 3.0,
                _ => 7.0,
            };
        }
    }
    finish_page(&mut content, &mut pages);
    if pages.is_empty() {
        bail!("Das Buch enthält keinen Text");
    }

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let f1 = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
    let f2 = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica-Bold", "Encoding" => "WinAnsiEncoding" });
    let f3 = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier", "Encoding" => "WinAnsiEncoding" });
    let res = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => f1, "F2" => f2, "F3" => f3 } });
    let total = pages.len();
    let mut kids: Vec<Object> = Vec::new();
    for (n, mut body) in pages.into_iter().enumerate() {
        let label = format!("{} / {}", n + 1, total);
        let lw: f32 = label.bytes().map(|b| glyph_width(b, false)).sum::<f32>() * 9.0 / 1000.0;
        body.extend_from_slice(format!("BT /F1 9 Tf {:.1} 40 Td ({}) Tj ET\n", (W - lw) / 2.0, label).as_bytes());
        let c = doc.add_object(Stream::new(dictionary! {}, body));
        let p = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => c,
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(W), Object::Real(H)],
        });
        kids.push(p.into());
    }
    let count = kids.len() as i64;
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count, "Resources" => res }));
    let info = doc.add_object(dictionary! { "Title" => Object::string_literal(book.title.clone()) });
    let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", cat);
    doc.trailer.set("Info", info);
    doc.compress();
    doc.save(output).map_err(|e| anyhow!("Die PDF konnte nicht geschrieben werden: {e}"))?;
    Ok(())
}

// ---------- CBZ to PDF ----------

fn natural_key(s: &str) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    let mut num = String::new();
    let mut text = String::new();
    let flush = |num: &mut String, text: &mut String, out: &mut Vec<(u64, String)>| {
        if !num.is_empty() || !text.is_empty() {
            out.push((num.parse().unwrap_or(0), std::mem::take(text)));
            num.clear();
        }
    };
    for c in s.to_lowercase().chars() {
        if c.is_ascii_digit() {
            if !text.is_empty() {
                flush(&mut num, &mut text, &mut out);
            }
            num.push(c);
        } else {
            if !num.is_empty() {
                flush(&mut num, &mut text, &mut out);
            }
            text.push(c);
        }
    }
    flush(&mut num, &mut text, &mut out);
    out
}

/// (width, height, components) from the first SOF marker of a baseline or progressive JPEG.
fn jpeg_info(d: &[u8]) -> Option<(u32, u32, u8)> {
    if !d.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut i = 2;
    while i + 9 < d.len() {
        if d[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = d[i + 1];
        if m == 0xFF || m == 0x00 || (0xD0..=0xD8).contains(&m) || m == 0x01 {
            i += if m == 0xFF { 1 } else { 2 };
            continue;
        }
        let len = u16::from_be_bytes([d[i + 2], d[i + 3]]) as usize;
        if matches!(m, 0xC0..=0xCF) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
            let h = u16::from_be_bytes([d[i + 5], d[i + 6]]) as u32;
            let w = u16::from_be_bytes([d[i + 7], d[i + 8]]) as u32;
            return Some((w, h, d[i + 9]));
        }
        i += 2 + len;
    }
    None
}

fn cbz_pdf(req: &Request, ctx: &Ctx) -> Result<()> {
    let mut z = open_zip(req.input)?;
    let mut names: Vec<String> = (0..z.len())
        .filter_map(|i| z.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| {
            let l = n.to_lowercase();
            let base = l.rsplit('/').next().unwrap_or("");
            !l.starts_with("__macosx") && !base.starts_with('.') && ["jpg", "jpeg", "png", "gif", "webp", "bmp"].iter().any(|e| l.ends_with(&format!(".{e}")))
        })
        .collect();
    if names.is_empty() {
        bail!("Das Archiv enthält keine Bilder");
    }
    names.sort_by_key(|n| natural_key(n));

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut kids: Vec<Object> = Vec::new();
    let total = names.len();
    for (n, name) in names.iter().enumerate() {
        if (ctx.cancelled)() {
            bail!("abgebrochen");
        }
        let mut data = Vec::new();
        z.by_name(name)?.read_to_end(&mut data)?;
        let (w, h, stream) = match jpeg_info(&data).filter(|i| i.2 == 1 || i.2 == 3) {
            Some((w, h, comps)) => {
                let cs = if comps == 1 { "DeviceGray" } else { "DeviceRGB" };
                let dict = dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64, "ColorSpace" => cs, "BitsPerComponent" => 8, "Filter" => "DCTDecode" };
                (w, h, Stream::new(dict, data))
            }
            None => {
                let img = match image::load_from_memory(&data) {
                    Ok(i) => i.to_rgb8(),
                    Err(_) => continue,
                };
                let (w, h) = img.dimensions();
                let dict = dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8 };
                (w, h, Stream::new(dict, img.into_raw()))
            }
        };
        let img_id = doc.add_object(stream);
        let (pw, ph) = (w as f32 * 0.48, h as f32 * 0.48);
        let content = format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im0 Do Q");
        let c = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let p = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => c,
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(pw), Object::Real(ph)],
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => img_id } },
        });
        kids.push(p.into());
        (ctx.progress)(0.05 + 0.85 * (n + 1) as f32 / total as f32);
    }
    if kids.is_empty() {
        bail!("Keines der Bilder im Archiv ließ sich lesen");
    }
    let count = kids.len() as i64;
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", cat);
    doc.compress();
    doc.save(req.output).map_err(|e| anyhow!("Die PDF konnte nicht geschrieben werden: {e}"))?;
    Ok(())
}

// ---------- subtitles ----------

#[derive(Debug)]
struct Cue {
    start: u64,
    end: u64,
    text: Vec<String>,
}

/// "HH:MM:SS,mmm", "MM:SS.mmm" or "H:MM:SS.cc" to milliseconds. `centi` reads a two-digit fraction as centiseconds.
fn parse_time(s: &str) -> Option<u64> {
    let s = s.trim().replace(',', ".");
    let (hms, frac) = s.split_once('.').unwrap_or((&s, "0"));
    let parts: Vec<u64> = hms.split(':').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    let secs = match parts.as_slice() {
        [m, s] => m * 60 + s,
        [h, m, s] => h * 3600 + m * 60 + s,
        _ => return None,
    };
    let digits: String = frac.chars().filter(|c| c.is_ascii_digit()).take(3).collect();
    let ms: u64 = format!("{digits:0<3}").parse().ok()?;
    Some(secs * 1000 + ms)
}

fn blocks(text: &str) -> Vec<Vec<&str>> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(line);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn parse_srt_vtt(text: &str) -> Result<Vec<Cue>> {
    let mut cues = Vec::new();
    for b in blocks(text.trim_start_matches('\u{feff}')) {
        let Some(pos) = b.iter().position(|l| l.contains("-->")) else { continue };
        if b[0].starts_with("NOTE") || b[0].starts_with("STYLE") || b[0].starts_with("REGION") {
            continue;
        }
        let (a, e) = b[pos].split_once("-->").unwrap();
        let (Some(start), Some(end)) = (parse_time(a), parse_time(e.split_whitespace().next().unwrap_or(""))) else { continue };
        cues.push(Cue { start, end, text: b[pos + 1..].iter().map(|s| s.to_string()).collect() });
    }
    Ok(cues)
}

fn parse_sbv(text: &str) -> Result<Vec<Cue>> {
    let mut cues = Vec::new();
    for b in blocks(text.trim_start_matches('\u{feff}')) {
        let Some((a, e)) = b[0].split_once(',') else { continue };
        let (Some(start), Some(end)) = (parse_time(a), parse_time(e)) else { continue };
        cues.push(Cue { start, end, text: b[1..].iter().map(|s| s.to_string()).collect() });
    }
    Ok(cues)
}

fn strip_braces(s: &str, open: char, close: char) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        if c == open {
            depth += 1;
        } else if c == close && depth > 0 {
            depth -= 1;
        } else if depth == 0 {
            out.push(c);
        }
    }
    out
}

fn parse_ass(text: &str) -> Result<Vec<Cue>> {
    let mut in_events = false;
    let mut fields: Vec<String> = Vec::new();
    let mut cues = Vec::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_events = l.eq_ignore_ascii_case("[events]");
            continue;
        }
        if !in_events {
            continue;
        }
        if let Some(f) = l.strip_prefix("Format:") {
            fields = f.split(',').map(|s| s.trim().to_lowercase()).collect();
        } else if let Some(d) = l.strip_prefix("Dialogue:") {
            let idx = |name: &str, default: usize| fields.iter().position(|f| f == name).unwrap_or(default);
            let n = fields.len().max(10);
            let parts: Vec<&str> = d.trim_start().splitn(n, ',').collect();
            let (si, ei, ti) = (idx("start", 1), idx("end", 2), idx("text", n - 1));
            let (Some(s), Some(e), Some(t)) = (parts.get(si), parts.get(ei), parts.get(ti)) else { continue };
            let (Some(start), Some(end)) = (parse_time(s), parse_time(e)) else { continue };
            let t = strip_braces(t, '{', '}').replace("\\N", "\n").replace("\\n", "\n").replace("\\h", " ");
            let text: Vec<String> = t.lines().map(|s| s.to_string()).collect();
            cues.push(Cue { start, end, text });
        }
    }
    cues.sort_by_key(|c| c.start);
    Ok(cues)
}

fn stamp(ms: u64, sep: char, pad_hours: bool) -> String {
    let (h, m, s, f) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if pad_hours {
        format!("{h:02}:{m:02}:{s:02}{sep}{f:03}")
    } else {
        format!("{h}:{m:02}:{s:02}{sep}{f:03}")
    }
}

fn plain_cue_text(c: &Cue) -> Vec<String> {
    c.text.iter().map(|l| strip_braces(l, '<', '>')).collect()
}

fn write_cues(cues: &[Cue], to: &str) -> String {
    let mut out = String::new();
    match to {
        "srt" => {
            for (i, c) in cues.iter().enumerate() {
                out.push_str(&format!("{}\n{} --> {}\n{}\n\n", i + 1, stamp(c.start, ',', true), stamp(c.end, ',', true), c.text.join("\n")));
            }
        }
        "vtt" => {
            out.push_str("WEBVTT\n\n");
            for c in cues {
                out.push_str(&format!("{} --> {}\n{}\n\n", stamp(c.start, '.', true), stamp(c.end, '.', true), c.text.join("\n")));
            }
        }
        "sbv" => {
            for c in cues {
                out.push_str(&format!("{},{}\n{}\n\n", stamp(c.start, '.', false), stamp(c.end, '.', false), plain_cue_text(c).join("\n")));
            }
        }
        _ => {
            out.push_str("[Script Info]\nScriptType: v4.00+\nPlayResX: 384\nPlayResY: 288\nWrapStyle: 0\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Arial,20,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");
            for c in cues {
                let t = plain_cue_text(c).join("\\N");
                let ass_time = |ms: u64| format!("{}:{:02}:{:02}.{:02}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000 / 10);
                out.push_str(&format!("Dialogue: 0,{},{},Default,,0,0,0,,{}\n", ass_time(c.start), ass_time(c.end), t));
            }
        }
    }
    out
}

fn subtitles(req: &Request, to: &str) -> Result<()> {
    let text = read_text(req.input)?;
    let cues = match req.analysis.format {
        Format::Srt | Format::Vtt => parse_srt_vtt(&text)?,
        Format::Sbv => parse_sbv(&text)?,
        Format::Ass => parse_ass(&text)?,
        other => bail!("{} ist kein unterstütztes Untertitelformat", other.id()),
    };
    if cues.is_empty() {
        bail!("Die Datei enthält keine Untertitel");
    }
    if !matches!(to, "srt" | "vtt" | "sbv" | "ass") {
        bail!("Unbekanntes Untertitelformat: {to}");
    }
    write_out(req, write_cues(&cues, to))
}
