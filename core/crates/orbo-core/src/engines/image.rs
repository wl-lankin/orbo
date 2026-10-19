//! Image engine: decode, geometry ops, screenshot beautifier and encoders for all raster targets.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Rgb, RgbImage, Rgba, RgbaImage};

use super::{Ctx, Engine, Request};
use crate::format::Format;

pub struct ImageEngine;

impl Engine for ImageEngine {
    fn convert(&self, req: &Request, ctx: &Ctx) -> Result<()> {
        let fmt = output_format(req)?;
        let op = req.str("op");
        (ctx.progress)(0.05);

        if op == Some("strip_metadata") && req.analysis.format == fmt && lossless_strip(req, fmt)? {
            return Ok(());
        }

        let mut img = decode(req)?;
        check_cancel(ctx)?;
        (ctx.progress)(0.2);

        img = transform(req, img, op)?;
        check_cancel(ctx)?;
        (ctx.progress)(0.6);

        if fmt == Format::Heic {
            encode_heic(&img, req)?;
        } else {
            let bytes = encode_to_size(req, &img, fmt)?;
            let bytes = if op == Some("compress") && !geometry_changed(req) {
                // Never hand back a "compressed" file that grew.
                match std::fs::read(req.input) {
                    Ok(orig) if orig.len() <= bytes.len() && req.analysis.format == fmt => orig,
                    _ => bytes,
                }
            } else {
                bytes
            };
            std::fs::write(req.output, bytes).context("Ausgabe konnte nicht geschrieben werden")?;
        }
        (ctx.progress)(0.95);
        Ok(())
    }
}

fn check_cancel(ctx: &Ctx) -> Result<()> {
    if (ctx.cancelled)() {
        bail!("abgebrochen");
    }
    Ok(())
}

fn output_format(req: &Request) -> Result<Format> {
    let fmt = if req.target.keeps_format() {
        req.analysis.format
    } else {
        req.str("format").and_then(Format::from_id).or_else(|| req.target.output_format()).unwrap_or(req.analysis.format)
    };
    match fmt {
        Format::Jpeg | Format::Png | Format::Webp | Format::Avif | Format::Gif | Format::Bmp | Format::Tiff | Format::Tga | Format::Pnm | Format::Qoi | Format::Ico | Format::Pdf => Ok(fmt),
        Format::Heic if cfg!(target_os = "macos") => Ok(fmt),
        Format::Heic => bail!("HEIC als Ausgabe folgt unter Windows"),
        other => bail!("Das Format {} kann nicht geschrieben werden", other.id().to_uppercase()),
    }
}

fn geometry_changed(req: &Request) -> bool {
    let o = req.options;
    o.max_edge.is_some() || o.width.is_some() || o.height.is_some() || o.crop.is_some() || o.aspect.is_some() || o.rotate.is_some() || o.flip_horizontal == Some(true)
}

// ---------------------------------------------------------------- decode

fn decode(req: &Request) -> Result<DynamicImage> {
    match req.analysis.format {
        Format::Svg => return decode_svg(req),
        Format::Heic | Format::Avif | Format::Raw | Format::Psd | Format::Icns => return decode_via_sips(req),
        _ => {}
    }
    match decode_native(req.input) {
        Ok(img) => Ok(img),
        Err(e) => decode_via_sips(req).map_err(|s| if cfg!(target_os = "macos") { s } else { e }),
    }
}

/// Decodes any raster image the system can read, for feeding a vision model.
pub(crate) fn decode_any(path: &Path, format: Format) -> Result<DynamicImage> {
    if format == Format::Svg {
        bail!("SVG-Grafiken kann die KI nicht lesen");
    }
    let sips = |path: &Path| -> Result<DynamicImage> {
        if !cfg!(target_os = "macos") {
            bail!("Dieses Bildformat kann unter Windows noch nicht gelesen werden");
        }
        let tmp = std::env::temp_dir().join(format!("orbo-ai-{}-{}.png", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
        let out = Command::new("/usr/bin/sips").args(["-s", "format", "png"]).arg(path).arg("--out").arg(&tmp).output().context("sips konnte nicht gestartet werden")?;
        let res = if out.status.success() { ImageReader::open(&tmp).map_err(anyhow::Error::from).and_then(|r| Ok(r.with_guessed_format()?.decode()?)) } else { Err(anyhow!("Das Bild konnte nicht gelesen werden")) };
        let _ = std::fs::remove_file(&tmp);
        res
    };
    match format {
        Format::Heic | Format::Avif | Format::Raw | Format::Psd | Format::Icns => sips(path),
        _ => decode_native(path).or_else(|e| sips(path).map_err(|s| if cfg!(target_os = "macos") { s } else { e })),
    }
}

fn decode_native(path: &Path) -> Result<DynamicImage> {
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder)?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok(img)
}

fn decode_via_sips(req: &Request) -> Result<DynamicImage> {
    if !cfg!(target_os = "macos") {
        bail!("Dieses Bildformat (HEIC, AVIF, RAW, PSD) kann unter Windows noch nicht gelesen werden, das folgt");
    }
    let tmp = sibling(req.output, "src.png");
    let status = Command::new("/usr/bin/sips").args(["-s", "format", "png"]).arg(req.input).arg("--out").arg(&tmp).output().context("sips konnte nicht gestartet werden")?;
    let result = if status.status.success() { ImageReader::open(&tmp).map_err(anyhow::Error::from).and_then(|r| Ok(r.with_guessed_format()?.decode()?)) } else { Err(anyhow!("Das Bild konnte nicht gelesen werden")) };
    let _ = std::fs::remove_file(&tmp);
    result
}

fn sibling(output: &Path, suffix: &str) -> PathBuf {
    let mut name = output.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".");
    name.push(suffix);
    output.with_file_name(name)
}

fn decode_svg(req: &Request) -> Result<DynamicImage> {
    let data = std::fs::read(req.input)?;
    let mut opts = resvg::usvg::Options::default();
    opts.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_data(&data, &opts).map_err(|e| anyhow!("SVG konnte nicht gelesen werden: {e}"))?;
    let size = tree.size();
    let longest = size.width().max(size.height()).max(1.0);
    let target = req.options.max_edge.or_else(|| req.f64("max_edge").map(|v| v as u32)).unwrap_or(2048).max(1) as f32;
    let scale = target / longest;
    let w = ((size.width() * scale).round() as u32).max(1);
    let h = ((size.height() * scale).round() as u32).max(1);
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or_else(|| anyhow!("SVG ist zu groß"))?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Ok(DynamicImage::ImageRgba8(pixmap_to_image(&pixmap)))
}

fn pixmap_to_image(p: &tiny_skia::Pixmap) -> RgbaImage {
    let mut out = Vec::with_capacity(p.data().len());
    for px in p.pixels() {
        let c = px.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    RgbaImage::from_raw(p.width(), p.height(), out).expect("buffer size matches")
}

// ---------------------------------------------------------------- transform

fn parse_aspect(s: &str) -> Option<f64> {
    let (a, b) = s.split_once(':')?;
    let (a, b): (f64, f64) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
    (a > 0.0 && b > 0.0).then_some(a / b)
}

fn transform(req: &Request, mut img: DynamicImage, op: Option<&str>) -> Result<DynamicImage> {
    let o = req.options;
    let beautify = op == Some("beautify");

    if let Some(r) = o.crop {
        let x = r.x.min(img.width().saturating_sub(1));
        let y = r.y.min(img.height().saturating_sub(1));
        let w = r.width.min(img.width() - x).max(1);
        let h = r.height.min(img.height() - y).max(1);
        img = img.crop_imm(x, y, w, h);
    } else if !beautify {
        let aspect = o.aspect.as_deref().or_else(|| req.str("aspect")).and_then(parse_aspect);
        if let Some(r) = aspect {
            let (w, h) = (img.width() as f64, img.height() as f64);
            let (nw, nh) = if w / h > r { ((h * r).round(), h) } else { (w, (w / r).round()) };
            let (nw, nh) = (nw.max(1.0) as u32, nh.max(1.0) as u32);
            img = img.crop_imm((img.width() - nw) / 2, (img.height() - nh) / 2, nw, nh);
        }
    }

    let degrees = o.rotate.or_else(|| req.f64("degrees").map(|d| d as u32)).unwrap_or(0) % 360;
    img = match degrees {
        90 => img.rotate90(),
        180 => img.rotate180(),
        270 => img.rotate270(),
        _ => img,
    };
    if o.flip_horizontal == Some(true) || op == Some("flip") {
        img = img.fliph();
    }

    img = resize(req, img);

    match op {
        Some("grayscale") => img = gray_keep_alpha(&img),
        Some("beautify") => img = DynamicImage::ImageRgba8(beautify_image(req, &img)?),
        _ => {}
    }
    Ok(img)
}

fn gray_keep_alpha(original: &DynamicImage) -> DynamicImage {
    let mut out = DynamicImage::ImageLuma8(original.to_luma8()).to_rgba8();
    if original.color().has_alpha() {
        for (d, s) in out.pixels_mut().zip(original.to_rgba8().pixels()) {
            d.0[3] = s.0[3];
        }
    }
    DynamicImage::ImageRgba8(out)
}

fn resize(req: &Request, img: DynamicImage) -> DynamicImage {
    let o = req.options;
    let (w, h) = (img.width(), img.height());
    let scale = |v: f64| (v.round() as u32).max(1);
    let (nw, nh) = if o.width.is_some() || o.height.is_some() {
        match (o.width, o.height) {
            (Some(nw), Some(nh)) => (nw.max(1), nh.max(1)),
            (Some(nw), None) => (nw.max(1), scale(h as f64 * nw as f64 / w as f64)),
            (None, Some(nh)) => (scale(w as f64 * nh as f64 / h as f64), nh.max(1)),
            (None, None) => unreachable!(),
        }
    } else if let Some(m) = o.max_edge.or_else(|| req.f64("max_edge").map(|v| v as u32)) {
        let longest = w.max(h);
        if m == 0 || longest <= m {
            return img;
        }
        let f = m as f64 / longest as f64;
        (scale(w as f64 * f), scale(h as f64 * f))
    } else if let Some(p) = req.f64("percent") {
        (scale(w as f64 * p / 100.0), scale(h as f64 * p / 100.0))
    } else {
        return img;
    };
    if (nw, nh) == (w, h) {
        return img;
    }
    img.resize_exact(nw, nh, FilterType::Lanczos3)
}

// ---------------------------------------------------------------- beautify

fn hex_color(s: &str) -> Option<tiny_skia::Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(tiny_skia::Color::from_rgba8((v >> 16) as u8, (v >> 8) as u8, v as u8, 255))
}

fn gradient_stops(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "sunset" => ("#ff7e5f", "#feb47b"),
        "ocean" => ("#2193b0", "#6dd5ed"),
        "lavender" => ("#a18cd1", "#fbc2eb"),
        "mint" => ("#43e97b", "#38f9d7"),
        "night" => ("#0f2027", "#2c5364"),
        _ => return None,
    })
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let k = r * 0.552_284_8;
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

fn box_blur_axis(src: &[u8], dst: &mut [u8], len: usize, lines: usize, elem: usize, line: usize, r: usize) {
    let d = (2 * r + 1) as u32;
    for l in 0..lines {
        let base = l * line;
        let at = |i: usize| src[base + i * elem] as u32;
        let mut sum: u32 = (0..=r.min(len - 1)).map(at).sum();
        for x in 0..len {
            dst[base + x * elem] = (sum / d) as u8;
            if x + r + 1 < len {
                sum += at(x + r + 1);
            }
            if x >= r {
                sum -= at(x - r);
            }
        }
    }
}

fn blur_mask(mask: &mut Vec<u8>, w: usize, h: usize, r: usize) {
    let mut tmp = vec![0u8; mask.len()];
    for _ in 0..3 {
        box_blur_axis(mask, &mut tmp, w, h, 1, w, r);
        box_blur_axis(&tmp, mask, h, w, w, 1, r);
    }
}

fn beautify_image(req: &Request, img: &DynamicImage) -> Result<RgbaImage> {
    let o = req.options;
    let padding = o.padding_px.or_else(|| req.f64("padding_px").map(|v| v as u32)).unwrap_or(96);
    let corner = o.corner_px.or_else(|| req.f64("corner_px").map(|v| v as u32)).unwrap_or(24) as f32;
    let shadow = o.shadow_px.or_else(|| req.f64("shadow_px").map(|v| v as u32)).unwrap_or(24);
    let bg = o.background.as_deref().or_else(|| req.str("background")).unwrap_or("sunset");

    let (w, h) = (img.width(), img.height());
    let (mut cw, mut ch) = (w + 2 * padding, h + 2 * padding);
    if let Some(r) = o.aspect.as_deref().or_else(|| req.str("aspect")).and_then(parse_aspect) {
        if (cw as f64 / ch as f64) < r {
            cw = (ch as f64 * r).round() as u32;
        } else {
            ch = (cw as f64 / r).round() as u32;
        }
    }
    if cw as u64 * ch as u64 > 120_000_000 {
        bail!("Das Ergebnis wäre zu groß");
    }
    let mut canvas = tiny_skia::Pixmap::new(cw, ch).ok_or_else(|| anyhow!("Das Ergebnis wäre zu groß"))?;

    let mut paint = tiny_skia::Paint { anti_alias: true, ..Default::default() };
    let full = tiny_skia::Rect::from_xywh(0.0, 0.0, cw as f32, ch as f32).ok_or_else(|| anyhow!("Ungültige Größe"))?;
    if let Some((a, b)) = gradient_stops(bg) {
        let stops = vec![tiny_skia::GradientStop::new(0.0, hex_color(a).unwrap()), tiny_skia::GradientStop::new(1.0, hex_color(b).unwrap())];
        paint.shader = tiny_skia::LinearGradient::new(tiny_skia::Point::from_xy(0.0, 0.0), tiny_skia::Point::from_xy(cw as f32, ch as f32), stops, tiny_skia::SpreadMode::Pad, tiny_skia::Transform::identity())
            .ok_or_else(|| anyhow!("Farbverlauf ungültig"))?;
    } else {
        let c = hex_color(bg).ok_or_else(|| anyhow!("Unbekannter Hintergrund: {bg}"))?;
        paint.set_color(c);
    }
    canvas.fill_rect(full, &paint, tiny_skia::Transform::identity(), None);

    let (ix, iy) = (((cw - w) / 2) as f32, ((ch - h) / 2) as f32);

    if shadow > 0 {
        let mut mask_px = tiny_skia::Pixmap::new(cw, ch).unwrap();
        let mut black = tiny_skia::Paint { anti_alias: true, ..Default::default() };
        black.set_color_rgba8(0, 0, 0, 255);
        if let Some(path) = rounded_rect(ix, iy + shadow as f32 / 3.0, w as f32, h as f32, corner) {
            mask_px.fill_path(&path, &black, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
        }
        let mut mask: Vec<u8> = mask_px.data().chunks_exact(4).map(|p| p[3]).collect();
        blur_mask(&mut mask, cw as usize, ch as usize, (shadow as usize / 3).max(1));
        let data: Vec<u8> = mask.iter().flat_map(|&a| [0, 0, 0, (a as f32 * 0.35) as u8]).collect();
        let shadow_px = tiny_skia::Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(cw, ch).unwrap()).unwrap();
        canvas.draw_pixmap(0, 0, shadow_px.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
    }

    let rgba = img.to_rgba8();
    let data: Vec<u8> = rgba
        .pixels()
        .flat_map(|p| {
            let a = p.0[3] as u32;
            let m = |c: u8| ((c as u32 * a + 127) / 255) as u8;
            [m(p.0[0]), m(p.0[1]), m(p.0[2]), p.0[3]]
        })
        .collect();
    let src = tiny_skia::Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(w, h).unwrap()).unwrap();
    let pattern = tiny_skia::Pattern::new(src.as_ref(), tiny_skia::SpreadMode::Pad, tiny_skia::FilterQuality::Nearest, 1.0, tiny_skia::Transform::from_translate(ix, iy));
    let img_paint = tiny_skia::Paint { shader: pattern, anti_alias: true, ..Default::default() };
    let path = rounded_rect(ix, iy, w as f32, h as f32, corner).ok_or_else(|| anyhow!("Ungültige Größe"))?;
    canvas.fill_path(&path, &img_paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);

    Ok(pixmap_to_image(&canvas))
}

// ---------------------------------------------------------------- encode

fn flatten_white(img: &DynamicImage) -> RgbImage {
    if !img.color().has_alpha() {
        return img.to_rgb8();
    }
    let rgba = img.to_rgba8();
    let mut out = RgbImage::new(rgba.width(), rgba.height());
    for (d, s) in out.pixels_mut().zip(rgba.pixels()) {
        let a = s.0[3] as u32;
        let mix = |c: u8| ((c as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
        *d = Rgb([mix(s.0[0]), mix(s.0[1]), mix(s.0[2])]);
    }
    out
}

fn encode_jpeg(img: &DynamicImage, q: u8) -> Result<Vec<u8>> {
    let rgb = flatten_white(img);
    let mut buf = Vec::new();
    JpegEncoder::new_with_quality(&mut buf, q).write_image(rgb.as_raw(), rgb.width(), rgb.height(), ExtendedColorType::Rgb8)?;
    Ok(buf)
}

fn encode_png(img: &DynamicImage) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let enc = PngEncoder::new_with_quality(&mut buf, CompressionType::Best, PngFilter::Adaptive);
    let (w, h) = (img.width(), img.height());
    if img.color().has_alpha() {
        enc.write_image(img.to_rgba8().as_raw(), w, h, ExtendedColorType::Rgba8)?;
    } else {
        enc.write_image(img.to_rgb8().as_raw(), w, h, ExtendedColorType::Rgb8)?;
    }
    Ok(buf)
}

fn encode_webp(img: &DynamicImage, q: u8, lossless: bool) -> Result<Vec<u8>> {
    let rgba = img.to_rgba8();
    let enc = webp::Encoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height());
    let mem = if lossless { enc.encode_lossless() } else { enc.encode(q as f32) };
    Ok(mem.to_vec())
}

fn encode_avif(img: &DynamicImage, q: u8) -> Result<Vec<u8>> {
    let rgba = img.to_rgba8();
    let px: Vec<rgb::RGBA8> = rgba.pixels().map(|p| rgb::RGBA8::new(p.0[0], p.0[1], p.0[2], p.0[3])).collect();
    let res = ravif::Encoder::new()
        .with_quality(q as f32)
        .with_speed(6)
        .encode_rgba(ravif::Img::new(px.as_slice(), rgba.width() as usize, rgba.height() as usize))
        .map_err(|e| anyhow!("AVIF-Kodierung fehlgeschlagen: {e}"))?;
    Ok(res.avif_file)
}

fn encode_ico(img: &DynamicImage) -> Result<Vec<u8>> {
    let longest = img.width().max(img.height());
    let mut sizes: Vec<u32> = [16, 24, 32, 48, 64, 128, 256].into_iter().filter(|&s| s <= longest).collect();
    if sizes.is_empty() {
        sizes.push(16);
    }
    let mut frames = Vec::new();
    for s in sizes {
        let fitted = img.resize(s, s, FilterType::Lanczos3).to_rgba8();
        let mut square = RgbaImage::from_pixel(s, s, Rgba([0, 0, 0, 0]));
        image::imageops::overlay(&mut square, &fitted, ((s - fitted.width()) / 2) as i64, ((s - fitted.height()) / 2) as i64);
        frames.push(IcoFrame::as_png(square.as_raw(), s, s, ExtendedColorType::Rgba8)?);
    }
    let mut buf = Vec::new();
    IcoEncoder::new(&mut buf).encode_images(&frames)?;
    Ok(buf)
}

fn encode_pdf(img: &DynamicImage, q: u8) -> Result<Vec<u8>> {
    let jpg = encode_jpeg(img, q)?;
    let (w, h) = (img.width(), img.height());
    let (pw, ph) = (w as f64 * 72.0 / 150.0, h as f64 * 72.0 / 150.0);
    let content = format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im0 Do Q");

    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    let mut obj = |out: &mut Vec<u8>, head: String, stream: Option<&[u8]>| {
        offsets.push(out.len());
        out.extend_from_slice(head.as_bytes());
        if let Some(s) = stream {
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(s);
            out.extend_from_slice(b"\nendstream");
        }
        out.extend_from_slice(b"\nendobj\n");
    };
    obj(&mut out, "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>".into(), None);
    obj(&mut out, "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(), None);
    obj(&mut out, format!("3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>"), None);
    obj(&mut out, format!("4 0 obj\n<< /Length {} >>", content.len()), Some(content.as_bytes()));
    obj(
        &mut out,
        format!("5 0 obj\n<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>", jpg.len()),
        Some(&jpg),
    );
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    Ok(out)
}

fn encode_other(img: &DynamicImage, f: ImageFormat) -> Result<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    if f == ImageFormat::Pnm {
        DynamicImage::ImageRgb8(flatten_white(img)).write_to(&mut buf, f)?;
    } else {
        DynamicImage::ImageRgba8(img.to_rgba8()).write_to(&mut buf, f)?;
    }
    Ok(buf.into_inner())
}

fn encode_once(req: &Request, img: &DynamicImage, fmt: Format, q: u8) -> Result<Vec<u8>> {
    match fmt {
        Format::Jpeg => encode_jpeg(img, q),
        Format::Png => encode_png(img),
        Format::Webp => encode_webp(img, q, req.bool("lossless") || req.str("op") == Some("strip_metadata")),
        Format::Avif => encode_avif(img, q),
        Format::Gif => encode_other(img, ImageFormat::Gif),
        Format::Bmp => encode_other(img, ImageFormat::Bmp),
        Format::Tiff => encode_other(img, ImageFormat::Tiff),
        Format::Tga => encode_other(img, ImageFormat::Tga),
        Format::Pnm => encode_other(img, ImageFormat::Pnm),
        Format::Qoi => encode_other(img, ImageFormat::Qoi),
        Format::Ico => encode_ico(img),
        Format::Pdf => encode_pdf(img, q),
        _ => bail!("Das Format {} kann nicht geschrieben werden", fmt.id().to_uppercase()),
    }
}

fn default_quality(fmt: Format) -> u8 {
    match fmt {
        Format::Jpeg => 85,
        Format::Webp => 80,
        Format::Avif => 60,
        Format::Pdf => 90,
        _ => 80,
    }
}

fn encode_to_size(req: &Request, img: &DynamicImage, fmt: Format) -> Result<Vec<u8>> {
    let q = req.quality(default_quality(fmt));
    let first = encode_once(req, img, fmt, q)?;
    let Some(mb) = req.options.target_mb else { return Ok(first) };
    let limit = (mb * 1024.0 * 1024.0) as usize;
    if first.len() <= limit || !matches!(fmt, Format::Jpeg | Format::Webp | Format::Avif) {
        return Ok(first);
    }
    // Binary search for the highest quality that fits.
    let (mut lo, mut hi) = (1u8, q.saturating_sub(1).max(1));
    let mut best: Option<Vec<u8>> = None;
    let mut smallest = first;
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let bytes = encode_once(req, img, fmt, mid)?;
        if bytes.len() < smallest.len() {
            smallest = bytes.clone();
        }
        if bytes.len() <= limit {
            best = Some(bytes);
            lo = mid + 1;
        } else {
            if mid == 1 {
                break;
            }
            hi = mid - 1;
        }
    }
    Ok(best.unwrap_or(smallest))
}

fn encode_heic(img: &DynamicImage, req: &Request) -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("HEIC als Ausgabe folgt unter Windows");
    }
    let tmp = sibling(req.output, "tmp.png");
    std::fs::write(&tmp, encode_png(img)?)?;
    let q = req.quality(80);
    let res = Command::new("/usr/bin/sips").args(["-s", "format", "heic", "-s", "formatOptions", &q.to_string()]).arg(&tmp).arg("--out").arg(req.output).output();
    let _ = std::fs::remove_file(&tmp);
    let out = res.context("sips konnte nicht gestartet werden")?;
    if !out.status.success() || !req.output.exists() {
        bail!("HEIC konnte nicht geschrieben werden");
    }
    Ok(())
}

// ---------------------------------------------------------------- lossless metadata strip

const XMP_PREFIX: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

/// Returns Ok(false) if the file needs the decode path instead (e.g. EXIF orientation must be baked in).
fn lossless_strip(req: &Request, fmt: Format) -> Result<bool> {
    if !matches!(fmt, Format::Jpeg | Format::Png) {
        return Ok(false);
    }
    if let Ok(file) = std::fs::File::open(req.input) {
        if let Ok(exif) = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)) {
            let o = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY).and_then(|f| f.value.get_uint(0));
            if o.is_some_and(|o| o != 1) {
                return Ok(false);
            }
        }
    }
    use img_parts::ImageEXIF;
    let data = img_parts::Bytes::from(std::fs::read(req.input)?);
    let mut out = Vec::new();
    if fmt == Format::Jpeg {
        let mut jpeg = img_parts::jpeg::Jpeg::from_bytes(data).map_err(|e| anyhow!("JPEG ist beschädigt: {e}"))?;
        jpeg.set_exif(None);
        jpeg.segments_mut().retain(|s| !(s.marker() == img_parts::jpeg::markers::APP1 && s.contents().starts_with(XMP_PREFIX)) && s.marker() != img_parts::jpeg::markers::APP13);
        jpeg.encoder().write_to(&mut out)?;
    } else {
        let mut png = img_parts::png::Png::from_bytes(data).map_err(|e| anyhow!("PNG ist beschädigt: {e}"))?;
        png.set_exif(None);
        png.chunks_mut().retain(|c| !(c.kind() == *b"iTXt" && c.contents().starts_with(b"XML:com.adobe.xmp")));
        png.encoder().write_to(&mut out)?;
    }
    std::fs::write(req.output, out)?;
    Ok(true)
}
