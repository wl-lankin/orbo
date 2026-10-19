//! Writers for a flattened drawing: SVG, PDF, PNG and R12 DXF.

use std::fmt::Write as _;
use std::path::Path as FsPath;

use anyhow::{anyhow, bail, Context, Result};
use lopdf::{dictionary, Document, Object, Stream};

use super::dxf::{Drawing, Path};

pub const PDF_MARGIN_MM: f64 = 10.0;
const PT_PER_MM: f64 = 72.0 / 25.4;
/// PDF viewers cap pages at 200 inches.
const MAX_PAGE_PT: f64 = 14400.0;

fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".into()
    } else {
        s.into()
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Extent of the drawing in mm, with a floor so degenerate drawings stay valid.
fn extent(d: &Drawing) -> Result<((f64, f64), (f64, f64))> {
    let (lo, hi) = d.bounds().ok_or_else(|| anyhow!("Die Zeichnung enthält keine darstellbare Geometrie"))?;
    Ok((lo, ((hi.0 - lo.0).max(0.1), (hi.1 - lo.1).max(0.1))))
}

fn line_width(w: f64, h: f64) -> f64 {
    (w.max(h) / 1500.0).clamp(0.1, 1.0)
}

/// SVG at true scale: one user unit is one millimetre, y points down.
pub fn svg_string(d: &Drawing, margin: f64, white: bool) -> Result<String> {
    let (lo, (w, h)) = extent(d)?;
    let (vw, vh) = (w + 2.0 * margin, h + 2.0 * margin);
    let lw = line_width(w, h);
    let mut s = String::new();
    let unit = if white { "" } else { "mm" };
    let _ = write!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}{unit}\" height=\"{}{unit}\" viewBox=\"0 0 {} {}\">\n",
        num(vw),
        num(vh),
        num(vw),
        num(vh)
    );
    if white {
        s.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#fff\"/>\n");
    }
    let _ = write!(s, "<g fill=\"none\" stroke=\"#000\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\n", num(lw));
    for layer in d.layers() {
        let _ = writeln!(s, "<g id=\"{}\">", esc(layer));
        for p in d.paths.iter().filter(|p| p.layer == layer) {
            s.push_str("<path d=\"");
            for (i, &(x, y)) in p.pts.iter().enumerate() {
                let _ = write!(s, "{}{} {}", if i == 0 { "M" } else { " L" }, num(x - lo.0 + margin), num(h - (y - lo.1) + margin));
            }
            if p.closed {
                s.push('Z');
            }
            s.push_str("\"/>\n");
        }
        s.push_str("</g>\n");
    }
    s.push_str("</g>\n</svg>\n");
    Ok(s)
}

pub fn write_svg(d: &Drawing, out: &FsPath) -> Result<()> {
    std::fs::write(out, svg_string(d, 0.0, false)?).context("Ausgabe konnte nicht geschrieben werden")
}

pub fn write_png(d: &Drawing, out: &FsPath, long_side: u32) -> Result<()> {
    let (_, (w, h)) = extent(d)?;
    let svg = svg_string(d, w.max(h) * 0.02, true)?;
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).map_err(|e| anyhow!("Die Zeichnung konnte nicht dargestellt werden: {e}"))?;
    let size = tree.size();
    let scale = long_side.max(16) as f32 / size.width().max(size.height());
    let (pw, ph) = (((size.width() * scale).round() as u32).max(1), ((size.height() * scale).round() as u32).max(1));
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph).ok_or_else(|| anyhow!("Die Zeichnung ist zu groß für die Bildausgabe"))?;
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.save_png(out).map_err(|e| anyhow!("PNG konnte nicht geschrieben werden: {e}"))
}

pub fn write_pdf(d: &Drawing, out: &FsPath) -> Result<()> {
    let (lo, (w, h)) = extent(d)?;
    let (pw_mm, ph_mm) = (w + 2.0 * PDF_MARGIN_MM, h + 2.0 * PDF_MARGIN_MM);
    let k = PT_PER_MM * (MAX_PAGE_PT / (pw_mm.max(ph_mm) * PT_PER_MM)).min(1.0);
    let lw = line_width(w, h) * k;
    let mut c = format!("q\n{} w 1 J 1 j\n0 0 0 RG\n", num(lw));
    for p in &d.paths {
        for (i, &(x, y)) in p.pts.iter().enumerate() {
            let _ = writeln!(c, "{} {} {}", num((x - lo.0 + PDF_MARGIN_MM) * k), num((y - lo.1 + PDF_MARGIN_MM) * k), if i == 0 { "m" } else { "l" });
        }
        c.push_str(if p.closed { "h S\n" } else { "S\n" });
    }
    c.push_str("Q\n");

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Stream::new(dictionary! {}, c.into_bytes()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "MediaBox" => vec![0.into(), 0.into(), Object::Real((pw_mm * k) as f32), Object::Real((ph_mm * k) as f32)],
    });
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 }));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    doc.compress();
    doc.save(out).map_err(|e| anyhow!("PDF konnte nicht geschrieben werden: {e}"))?;
    Ok(())
}

fn r12_layer(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '_' | '-' | '$') { c.to_ascii_uppercase() } else { '_' }).collect();
    if s.is_empty() {
        "0".into()
    } else {
        s.chars().take(31).collect()
    }
}

fn r12_path(s: &mut String, p: &Path, layer: &str) {
    let g = |s: &mut String, code: i32, v: &str| {
        let _ = write!(s, "{code}\n{v}\n");
    };
    if p.pts.len() == 2 && !p.closed {
        g(s, 0, "LINE");
        g(s, 8, layer);
        for (i, &(x, y)) in p.pts.iter().enumerate() {
            g(s, 10 + i as i32, &num(x));
            g(s, 20 + i as i32, &num(y));
            g(s, 30 + i as i32, "0");
        }
        return;
    }
    g(s, 0, "POLYLINE");
    g(s, 8, layer);
    g(s, 66, "1");
    g(s, 70, if p.closed { "1" } else { "0" });
    for &(x, y) in &p.pts {
        g(s, 0, "VERTEX");
        g(s, 8, layer);
        g(s, 10, &num(x));
        g(s, 20, &num(y));
        g(s, 30, "0");
    }
    g(s, 0, "SEQEND");
    g(s, 8, layer);
}

/// Plain R12 ASCII DXF: millimetres, every curve flattened, blocks exploded.
pub fn write_dxf(d: &Drawing, out: &FsPath) -> Result<()> {
    if d.paths.is_empty() {
        bail!("Die Zeichnung enthält keine darstellbare Geometrie");
    }
    let mut s = String::new();
    s.push_str("0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1009\n0\nENDSEC\n");
    s.push_str("0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLTYPE\n70\n1\n0\nLTYPE\n2\nCONTINUOUS\n70\n0\n3\nSolid line\n72\n65\n73\n0\n40\n0.0\n0\nENDTAB\n");
    let layers = d.layers();
    let _ = write!(s, "0\nTABLE\n2\nLAYER\n70\n{}\n", layers.len());
    let mut seen = std::collections::HashSet::new();
    for l in &layers {
        let n = r12_layer(l);
        if seen.insert(n.clone()) {
            let _ = write!(s, "0\nLAYER\n2\n{n}\n70\n0\n62\n7\n6\nCONTINUOUS\n");
        }
    }
    s.push_str("0\nENDTAB\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n");
    for p in &d.paths {
        r12_path(&mut s, p, &r12_layer(&p.layer));
    }
    s.push_str("0\nENDSEC\n0\nEOF\n");
    std::fs::write(out, s).context("Ausgabe konnte nicht geschrieben werden")
}
