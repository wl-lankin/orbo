//! Local AI actions through Ollama (see crate::ai). The original is never touched; results are new files.
//!
//! params:
//!   action  rename      copy of the file under a descriptive name (image: vision, others: extracted text)
//!           alt_text    1-2 sentence description of an image
//!           summary     summary of a document, map-reduce over chunks for long ones
//!           translate   text or subtitles into `lang` (SRT/VTT: only the text lines change)

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;

use super::{Ctx, Engine, Request};
use crate::ai::{self, chunk_text, clean_answer, strip_quotes, Llm, Ollama};
use crate::format::{Format, Kind};

pub struct AiEngine;

const NAME_MAX: usize = 60;
const RENAME_TEXT_CHARS: usize = 4000;
const SUMMARY_CHUNK: usize = 6000;
const SUMMARY_MAX_CHUNKS: usize = 12;
const TRANSLATE_CHUNK: usize = 3000;
const SUBTITLE_BATCH_LINES: usize = 40;
const VISION_EDGE: u32 = 1024;

impl Engine for AiEngine {
    fn convert(&self, req: &Request, ctx: &Ctx) -> Result<()> {
        let action = req.str("action").ok_or_else(|| anyhow!("Unbekannte KI-Aktion"))?;
        let cfg = ai::config().filter(|_| ai::action_enabled(Some(action))).ok_or_else(|| anyhow!("Diese KI-Aktion ist in den Einstellungen ausgeschaltet."))?;
        let vision = action == "alt_text" || (action == "rename" && req.analysis.kind == Kind::Image);
        (ctx.progress)(0.05);
        let llm = Ollama::connect(&cfg, vision)?;
        check_cancel(ctx)?;
        let cancelled = || (ctx.cancelled)();
        let stem = req.input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        match action {
            "rename" => {
                let name = if req.analysis.kind == Kind::Image {
                    let img = image_b64(req.input, req.analysis.format)?;
                    rename_image(&llm, &img, exif_date(req.input).as_deref(), &input_ext(req.input))?
                } else {
                    let text = extract_text(req.input, req.analysis.format, RENAME_TEXT_CHARS)?;
                    rename_text(&llm, &text, &input_ext(req.input))?
                };
                std::fs::copy(req.input, req.output).context("Kopie konnte nicht geschrieben werden")?;
                *ctx.final_stem.lock().unwrap() = Some(name);
            }
            "alt_text" => {
                let img = image_b64(req.input, req.analysis.format)?;
                let text = alt_text(&llm, &img)?;
                std::fs::write(req.output, text + "\n").context("Ausgabe konnte nicht geschrieben werden")?;
            }
            "summary" => {
                let text = extract_text(req.input, req.analysis.format, usize::MAX)?;
                let md = summarize(&llm, &text, &cancelled, &|f| (ctx.progress)(0.1 + 0.85 * f))?;
                std::fs::write(req.output, md + "\n").context("Ausgabe konnte nicht geschrieben werden")?;
            }
            "translate" => {
                let lang = req.str("lang").unwrap_or("Englisch");
                let text = extract_text(req.input, req.analysis.format, usize::MAX)?;
                let out = match req.analysis.format {
                    Format::Srt | Format::Vtt => translate_subtitle(&llm, &text, lang, &cancelled, &|f| (ctx.progress)(0.1 + 0.85 * f))?,
                    _ => translate_plain(&llm, &text, lang, &cancelled, &|f| (ctx.progress)(0.1 + 0.85 * f))?,
                };
                std::fs::write(req.output, out).context("Ausgabe konnte nicht geschrieben werden")?;
                *ctx.final_stem.lock().unwrap() = Some(format!("{stem} ({lang})"));
            }
            other => bail!("Unbekannte KI-Aktion: {other}"),
        }
        check_cancel(ctx)
    }
}

fn check_cancel(ctx: &Ctx) -> Result<()> {
    if (ctx.cancelled)() {
        bail!("abgebrochen");
    }
    Ok(())
}

fn input_ext(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
}

// ---------- input ----------

/// Running text of the file, cut to `max` characters.
pub fn extract_text(path: &Path, format: Format, max: usize) -> Result<String> {
    let text = match format {
        Format::Pdf => {
            // pdf-extract panics on some malformed fonts
            let t = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pdf_extract::extract_text(path)))
                .map_err(|_| anyhow!("Der Text dieser PDF konnte nicht gelesen werden"))?
                .map_err(|e| anyhow!("Der Text dieser PDF konnte nicht gelesen werden: {e}"))?;
            t.replace("\r\n", "\n")
        }
        Format::Docx | Format::Epub | Format::Html | Format::Md => super::ebook::plain_text(path, format)?,
        _ => {
            let bytes = std::fs::read(path).context("Datei konnte nicht gelesen werden")?;
            let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&bytes);
            match std::str::from_utf8(bytes) {
                Ok(s) => s.to_string(),
                Err(_) => bytes.iter().map(|&b| b as char).collect(),
            }
        }
    };
    let text = text.trim();
    if text.is_empty() {
        bail!("Die Datei enthält keinen lesbaren Text (vermutlich ein Scan)");
    }
    Ok(match text.char_indices().nth(max) {
        Some((i, _)) => text[..i].to_string(),
        None => text.to_string(),
    })
}

/// Downscaled JPEG as base64 for a vision model.
fn image_b64(path: &Path, format: Format) -> Result<String> {
    let img = super::image::decode_any(path, format)?;
    let img = if img.width().max(img.height()) > VISION_EDGE { img.resize(VISION_EDGE, VISION_EDGE, image::imageops::FilterType::Triangle) } else { img };
    let rgb = image::DynamicImage::ImageRgb8(flatten(&img));
    let mut buf = Vec::new();
    rgb.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Jpeg).context("Bild konnte nicht vorbereitet werden")?;
    Ok(base64::engine::general_purpose::STANDARD.encode(buf))
}

fn flatten(img: &image::DynamicImage) -> image::RgbImage {
    let rgba = img.to_rgba8();
    let mut out = image::RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = p[3] as u32;
        let mix = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        out.put_pixel(x, y, image::Rgb([mix(p[0]), mix(p[1]), mix(p[2])]));
    }
    out
}

/// "YYYY-MM-DD" from the EXIF capture time, if the image has one.
pub fn exif_date(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let exif = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)).ok()?;
    let field = exif.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY).or_else(|| exif.get_field(exif::Tag::DateTime, exif::In::PRIMARY))?;
    let v = field.display_value().to_string();
    valid_date(v.trim_matches('"').get(..10)?).then(|| v.trim_matches('"')[..10].to_string())
}

fn valid_date(s: &str) -> bool {
    let p: Vec<&str> = s.split('-').collect();
    p.len() == 3
        && p[0].len() == 4
        && p[1].len() == 2
        && p[2].len() == 2
        && matches!(p[0].parse::<u32>(), Ok(y) if (1900..=2100).contains(&y))
        && matches!(p[1].parse::<u32>(), Ok(m) if (1..=12).contains(&m))
        && matches!(p[2].parse::<u32>(), Ok(d) if (1..=31).contains(&d))
}

// ---------- names ----------

/// Makes a model answer usable as a file name: one line, no quotes, filesystem-safe, at most `max` characters.
pub fn sanitize_name(raw: &str, ext: &str, max: usize) -> String {
    let answer = clean_answer(raw);
    let line = answer.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let mut line = strip_quotes(strip_quotes(line).trim_end_matches('.')).to_string();
    if !ext.is_empty() {
        let dotted = format!(".{}", ext.to_lowercase());
        if line.to_lowercase().ends_with(&dotted) {
            line.truncate(line.len() - dotted.len());
        }
    }
    let cleaned: String = line.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { ' ' } else { c }).collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = cleaned.trim_start_matches(['#', '-', '.', ' ']).to_string();
    let mut name = cleaned;
    if name.chars().count() > max {
        let cut = name.char_indices().nth(max).map_or(name.len(), |(i, _)| i);
        name.truncate(cut);
        if let Some(sp) = name.rfind(' ') {
            if name[..sp].chars().count() >= max / 2 {
                name.truncate(sp);
            }
        }
    }
    name.trim_end_matches(['.', ' ', '-', ',', ';']).to_string()
}

/// "YYYY-MM-DD Name" when a valid date is known, else "Name".
pub fn compose_stem(date: Option<&str>, name: &str) -> String {
    match date.filter(|d| valid_date(d)) {
        Some(d) => format!("{d} {name}"),
        None => name.to_string(),
    }
}

fn require_name(name: String) -> Result<String> {
    if name.is_empty() {
        bail!("Das Modell hat keinen brauchbaren Namen geliefert");
    }
    Ok(name)
}

pub fn rename_image(llm: &dyn Llm, image_b64: &str, date: Option<&str>, ext: &str) -> Result<String> {
    let prompt = "Gib dieser Bilddatei einen kurzen, beschreibenden deutschen Dateinamen (zwei bis sechs Wörter, höchstens 50 Zeichen), der den Inhalt des Bildes trifft. \
                  Keine Dateiendung, kein Datum, keine Anführungszeichen, keine Sonderzeichen. Antworte nur mit dem Namen.";
    let name = require_name(sanitize_name(&llm.generate(prompt, &[image_b64.to_string()])?, ext, NAME_MAX))?;
    Ok(compose_stem(date, &name))
}

pub fn rename_text(llm: &dyn Llm, text: &str, ext: &str) -> Result<String> {
    let prompt = format!(
        "Hier ist der Anfang eines Dokuments. Schlage einen Dateinamen vor.\n\
         Antworte in genau zwei Zeilen:\n\
         DATUM: das Datum des Dokuments als JJJJ-MM-TT, oder - wenn keines eindeutig genannt wird\n\
         NAME: kurzer, beschreibender deutscher Name (zwei bis sechs Wörter, höchstens 50 Zeichen, keine Dateiendung, keine Anführungszeichen)\n\n\
         Dokument:\n{text}"
    );
    let answer = clean_answer(&llm.generate(&prompt, &[])?);
    let field = |key: &str| answer.lines().find_map(|l| l.trim().strip_prefix(key).map(|v| v.trim().to_string()));
    let date = field("DATUM:").filter(|d| valid_date(d));
    let raw = field("NAME:").unwrap_or_else(|| answer.clone());
    let name = require_name(sanitize_name(&raw, ext, NAME_MAX))?;
    Ok(compose_stem(date.as_deref(), &name))
}

pub fn alt_text(llm: &dyn Llm, image_b64: &str) -> Result<String> {
    let prompt = "Beschreibe dieses Bild als Alternativtext für Screenreader auf Deutsch in einem bis zwei sachlichen Sätzen. \
                  Nenne, was zu sehen ist, ohne Einleitung wie „Das Bild zeigt“. Antworte nur mit dem Text.";
    let text = clean_answer(&llm.generate(prompt, &[image_b64.to_string()])?);
    let text = strip_quotes(&text).to_string();
    if text.is_empty() {
        bail!("Das Modell hat keine Beschreibung geliefert");
    }
    Ok(text)
}

// ---------- summary ----------

pub fn summarize(llm: &dyn Llm, text: &str, cancelled: &dyn Fn() -> bool, progress: &dyn Fn(f32)) -> Result<String> {
    let mut chunks = chunk_text(text, SUMMARY_CHUNK);
    if chunks.len() > SUMMARY_MAX_CHUNKS {
        // Evenly spread sample so the end of a long text is not lost.
        let n = chunks.len();
        chunks = (0..SUMMARY_MAX_CHUNKS).map(|i| chunks[i * n / SUMMARY_MAX_CHUNKS].clone()).collect();
    }
    let final_prompt = |material: &str| {
        format!(
            "Fasse den folgenden Text auf Deutsch zusammen: zuerst ein Satz zum Kern, dann drei bis acht Stichpunkte mit den wichtigsten Inhalten, Zahlen und Vereinbarungen. \
             Sachlich, ohne Einleitung. Antworte in Markdown.\n\n{material}"
        )
    };
    let result = if chunks.len() <= 1 {
        progress(0.1);
        llm.generate(&final_prompt(chunks.first().map(String::as_str).unwrap_or(text)), &[])?
    } else {
        let mut notes = Vec::new();
        for (i, c) in chunks.iter().enumerate() {
            if cancelled() {
                bail!("abgebrochen");
            }
            progress(i as f32 / (chunks.len() + 1) as f32);
            let p = format!("Teil {} von {} eines längeren Textes. Fasse ihn auf Deutsch in fünf bis zehn kurzen Stichpunkten zusammen und behalte Namen, Zahlen und Beschlüsse. Antworte nur mit den Stichpunkten.\n\n{c}", i + 1, chunks.len());
            notes.push(clean_answer(&llm.generate(&p, &[])?));
        }
        if cancelled() {
            bail!("abgebrochen");
        }
        llm.generate(&final_prompt(&format!("Notizen zu den Teilen eines Dokuments:\n\n{}", notes.join("\n\n"))), &[])?
    };
    progress(1.0);
    let md = clean_answer(&result);
    if md.is_empty() {
        bail!("Das Modell hat keine Zusammenfassung geliefert");
    }
    Ok(md)
}

// ---------- translation ----------

pub fn translate_plain(llm: &dyn Llm, text: &str, lang: &str, cancelled: &dyn Fn() -> bool, progress: &dyn Fn(f32)) -> Result<String> {
    let chunks = chunk_text(text, TRANSLATE_CHUNK);
    let mut out = Vec::new();
    for (i, c) in chunks.iter().enumerate() {
        if cancelled() {
            bail!("abgebrochen");
        }
        progress(i as f32 / chunks.len() as f32);
        let p = format!(
            "Übersetze den folgenden Text ins {lang}. Behalte Absätze, Aufzählungen und Markdown-Auszeichnungen bei. Antworte nur mit der Übersetzung, ohne Kommentar.\n\n{c}"
        );
        let t = clean_answer(&llm.generate(&p, &[])?);
        if t.is_empty() {
            bail!("Das Modell hat keine Übersetzung geliefert");
        }
        out.push(t);
    }
    progress(1.0);
    Ok(out.join("\n\n") + "\n")
}

/// Translates the text lines of SRT/VTT; numbering, timings, headers and blank lines stay byte-identical.
pub fn translate_subtitle(llm: &dyn Llm, src: &str, lang: &str, cancelled: &dyn Fn() -> bool, progress: &dyn Fn(f32)) -> Result<String> {
    let mut lines: Vec<String> = src.split('\n').map(String::from).collect();
    let mut todo: Vec<usize> = Vec::new();
    let mut in_cue = false;
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim();
        if t.is_empty() {
            in_cue = false;
        } else if t.contains("-->") {
            in_cue = true;
        } else if in_cue && t.chars().any(|c| c.is_alphabetic()) {
            todo.push(i);
        }
    }
    let batches: Vec<&[usize]> = todo.chunks(SUBTITLE_BATCH_LINES).collect();
    for (bi, batch) in batches.iter().enumerate() {
        if cancelled() {
            bail!("abgebrochen");
        }
        progress(bi as f32 / batches.len() as f32);
        let listing: String = batch.iter().enumerate().map(|(n, &i)| format!("{}|{}\n", n + 1, lines[i].trim())).collect();
        let p = format!(
            "Übersetze die folgenden Untertitelzeilen ins {lang}. Jede Zeile beginnt mit einer Nummer und einem senkrechten Strich. \
             Antworte mit denselben Nummern im selben Format „Nummer|Übersetzung“, genau eine Zeile pro Eingabezeile. \
             Behalte HTML-Tags wie <i> und Sprecherangaben bei. Keine Erklärungen.\n\n{listing}"
        );
        let answer = clean_answer(&llm.generate(&p, &[])?);
        let mut got: Vec<Option<String>> = vec![None; batch.len()];
        for l in answer.lines() {
            if let Some((n, rest)) = l.trim().split_once('|') {
                if let Ok(n) = n.trim().parse::<usize>() {
                    if n >= 1 && n <= got.len() && !rest.trim().is_empty() {
                        got[n - 1] = Some(rest.trim().to_string());
                    }
                }
            }
        }
        for (n, &i) in batch.iter().enumerate() {
            let tr = match got[n].take() {
                Some(t) => t,
                None => {
                    if cancelled() {
                        bail!("abgebrochen");
                    }
                    let p = format!("Übersetze diese Untertitelzeile ins {lang}. Behalte HTML-Tags bei. Antworte nur mit der Übersetzung.\n\n{}", lines[i].trim());
                    let t = clean_answer(&llm.generate(&p, &[])?);
                    let t = t.lines().next().unwrap_or("").trim().to_string();
                    if t.is_empty() {
                        bail!("Das Modell hat eine Untertitelzeile nicht übersetzt");
                    }
                    t
                }
            };
            let cr = if lines[i].ends_with('\r') { "\r" } else { "" };
            lines[i] = format!("{tr}{cr}");
        }
    }
    progress(1.0);
    Ok(lines.join("\n"))
}
