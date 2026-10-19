//! Fonts: TTF/OTF/WOFF/WOFF2 container conversion and a PNG specimen sheet. Glyph outlines are never touched.
//!
//! params: `to` = ttf | otf | woff | woff2 | png (png renders a specimen sheet)

use std::fs;

use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont};
use anyhow::{anyhow, bail, Context, Result};
use flate2::{write::ZlibEncoder, Compression};
use image::{Rgb, RgbImage};
use std::io::Write;

use super::{Ctx, Engine, Request};

pub struct Engine_;

const SAMPLE: &str = "Zwölf Boxkämpfer jagen Viktor quer über den großen Sylter Deich";
const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789 ÄÖÜ äöüß &@€!?";

impl Engine for Engine_ {
    fn convert(&self, req: &Request, ctx: &Ctx) -> Result<()> {
        let to = req.str("to").ok_or_else(|| anyhow!("Zielformat fehlt"))?;
        (ctx.progress)(0.05);
        let raw = fs::read(req.input).context("Schrift konnte nicht gelesen werden")?;
        let sfnt = to_sfnt(&raw)?;
        (ctx.progress)(0.4);
        let out = match to {
            "ttf" | "otf" => {
                let cff = sfnt.starts_with(b"OTTO");
                if to == "ttf" && cff {
                    bail!("Diese Schrift hat PostScript-Konturen (CFF) und lässt sich nur als OTF speichern");
                }
                if to == "otf" && !cff {
                    bail!("Diese Schrift hat TrueType-Konturen und lässt sich nur als TTF speichern");
                }
                sfnt
            }
            "woff" => woff1(&sfnt)?,
            "woff2" => ttf2woff2::encode(&sfnt, ttf2woff2::BrotliQuality::from(11u8)).map_err(|e| anyhow!("WOFF2 konnte nicht erzeugt werden: {e}"))?,
            "png" => {
                specimen(&sfnt)?.save(req.output).context("Schriftprobe konnte nicht geschrieben werden")?;
                (ctx.progress)(0.95);
                return Ok(());
            }
            other => bail!("Ausgabeformat {other} wird von der Schriftumwandlung nicht unterstützt"),
        };
        fs::write(req.output, out).context("Ausgabe konnte nicht geschrieben werden")?;
        (ctx.progress)(0.95);
        Ok(())
    }
}

/// Plain sfnt bytes (TrueType or OpenType) from any of the four formats.
fn to_sfnt(raw: &[u8]) -> Result<Vec<u8>> {
    // The signature decides, not the extension or the sniffed type.
    let sfnt = if raw.starts_with(b"wOFF") {
        wuff::decompress_woff1(raw).map_err(|e| anyhow!("Ungültige WOFF-Datei: {e:?}"))?
    } else if raw.starts_with(b"wOF2") {
        wuff::decompress_woff2(raw).map_err(|e| anyhow!("Ungültige WOFF2-Datei: {e:?}"))?
    } else {
        raw.to_vec()
    };
    if sfnt.starts_with(b"ttcf") {
        bail!("Schriftsammlungen (TTC) werden nicht unterstützt");
    }
    if !(sfnt.starts_with(&[0, 1, 0, 0]) || sfnt.starts_with(b"OTTO") || sfnt.starts_with(b"true")) {
        bail!("Die Datei ist keine gültige Schrift");
    }
    Ok(sfnt)
}

fn be32(b: &[u8], at: usize) -> Result<u32> {
    b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| anyhow!("Die Schriftdatei ist beschädigt"))
}

fn woff1(sfnt: &[u8]) -> Result<Vec<u8>> {
    let n = sfnt.get(4..6).map(|s| u16::from_be_bytes([s[0], s[1]]) as usize).ok_or_else(|| anyhow!("Die Schriftdatei ist beschädigt"))?;
    let pad4 = |v: usize| (v + 3) & !3;
    let mut tables = Vec::with_capacity(n);
    let mut sfnt_size = 12 + 16 * n;
    for i in 0..n {
        let e = 12 + 16 * i;
        let (checksum, offset, length) = (be32(sfnt, e + 4)?, be32(sfnt, e + 8)? as usize, be32(sfnt, e + 12)? as usize);
        let data = sfnt.get(offset..offset + length).ok_or_else(|| anyhow!("Die Schriftdatei ist beschädigt"))?;
        let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
        z.write_all(data)?;
        let packed = z.finish()?;
        let stored = if packed.len() < data.len() { packed } else { data.to_vec() };
        sfnt_size += pad4(length);
        tables.push((sfnt[e..e + 4].to_vec(), checksum, length, stored));
    }
    let mut offset = 44 + 20 * n;
    let total = offset + tables.iter().map(|t| pad4(t.3.len())).sum::<usize>();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"wOFF");
    out.extend_from_slice(&sfnt[0..4]);
    out.extend_from_slice(&(total as u32).to_be_bytes());
    out.extend_from_slice(&(n as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(sfnt_size as u32).to_be_bytes());
    out.extend_from_slice(&[0, 1, 0, 0]);
    out.extend_from_slice(&[0u8; 20]);
    for (tag, checksum, length, stored) in &tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(stored.len() as u32).to_be_bytes());
        out.extend_from_slice(&(*length as u32).to_be_bytes());
        out.extend_from_slice(&checksum.to_be_bytes());
        offset += pad4(stored.len());
    }
    for (_, _, _, stored) in &tables {
        out.extend_from_slice(stored);
        out.resize(pad4(out.len()), 0);
    }
    Ok(out)
}

fn font_name(sfnt: &[u8]) -> String {
    ttf_parser::Face::parse(sfnt, 0)
        .ok()
        .and_then(|f| {
            let names = f.names();
            [4u16, 1].iter().find_map(|&id| names.into_iter().find(|n| n.name_id == id && n.is_unicode()).and_then(|n| n.to_string()))
        })
        .unwrap_or_else(|| "Schrift".to_string())
}

fn draw_text(img: &mut RgbImage, font: &FontRef, size: f32, x: f32, baseline: f32, text: &str) {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut pen = x;
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            pen += scaled.kern(p, id);
        }
        let glyph = id.with_scale_and_position(PxScale::from(size), ab_glyph::point(pen, baseline));
        pen += scaled.h_advance(id);
        prev = Some(id);
        if let Some(outlined) = font.outline_glyph(glyph) {
            let b = outlined.px_bounds();
            outlined.draw(|gx, gy, cov| {
                let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if px >= 0 && py >= 0 && (px as u32) < img.width() && (py as u32) < img.height() {
                    let bg = img.get_pixel(px as u32, py as u32).0;
                    let mix = |c: u8| (c as f32 * (1.0 - cov)) as u8;
                    img.put_pixel(px as u32, py as u32, Rgb([mix(bg[0]), mix(bg[1]), mix(bg[2])]));
                }
            });
        }
    }
}

fn text_width(font: &FontRef, size: f32, text: &str) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    text.chars().map(|c| scaled.h_advance(font.glyph_id(c))).sum()
}

fn wrap(font: &FontRef, size: f32, text: &str, max: f32) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in text.split(' ') {
        let cur = lines.last_mut().unwrap();
        let candidate = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if cur.is_empty() || text_width(font, size, &candidate) <= max {
            *cur = candidate;
        } else {
            lines.push(word.to_string());
        }
    }
    lines
}

fn specimen(sfnt: &[u8]) -> Result<RgbImage> {
    let font = FontRef::try_from_slice(sfnt).map_err(|_| anyhow!("Die Schrift konnte nicht gelesen werden"))?;
    let name = font_name(sfnt);
    const W: u32 = 1600;
    const MARGIN: f32 = 64.0;
    let max = W as f32 - 2.0 * MARGIN;

    let mut blocks: Vec<(f32, Vec<String>)> = Vec::new();
    blocks.push((72.0, wrap(&font, 72.0, &name, max)));
    blocks.push((28.0, wrap(&font, 28.0, ALPHABET, max)));
    for size in [14.0, 20.0, 32.0, 48.0, 72.0] {
        blocks.push((size, wrap(&font, size, SAMPLE, max)));
    }
    let line_h = |s: f32| s * 1.3;
    let gap = 36.0;
    let height = 2.0 * MARGIN + blocks.iter().map(|(s, l)| line_h(*s) * l.len() as f32 + gap).sum::<f32>();
    let mut img = RgbImage::from_pixel(W, height.ceil() as u32, Rgb([255, 255, 255]));
    let mut y = MARGIN;
    for (size, lines) in &blocks {
        for line in lines {
            y += line_h(*size);
            draw_text(&mut img, &font, *size, MARGIN, y - size * 0.3, line);
        }
        y += gap;
    }
    Ok(img)
}
