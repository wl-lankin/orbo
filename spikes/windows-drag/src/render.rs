use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};
use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, GradientStop, LinearGradient, Paint, Path, PathBuilder, Pixmap, PixmapPaint,
    Point, RadialGradient, Shader, SpreadMode, Stroke, Transform,
};

use crate::actions::Action;
use crate::Style;

pub const PANEL: f32 = 420.0;
pub const OUTER: f32 = 180.0;
pub const INNER: f32 = 62.0;

pub struct Fonts {
    pub regular: FontVec,
    pub bold: FontVec,
}

impl Fonts {
    pub fn load() -> Option<Fonts> {
        let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        let read = |names: &[&str]| {
            names.iter().find_map(|n| std::fs::read(format!("{dir}\\Fonts\\{n}")).ok()).and_then(|b| FontVec::try_from_vec(b).ok())
        };
        Some(Fonts { regular: read(&["segoeui.ttf", "arial.ttf"])?, bold: read(&["segoeuib.ttf", "arialbd.ttf"])? })
    }
}

pub struct RingModel<'a> {
    pub actions: &'a [Action],
    pub hot: Option<i32>,
    pub kind: &'a str,
    pub count: usize,
    pub style: Style,
}

fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::from_rgba8(r, g, b, (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn mix(c: [u8; 3], base: [u8; 3], t: f32) -> [u8; 3] {
    let m = |a: u8, b: u8| (a as f32 * t + b as f32 * (1.0 - t)).round() as u8;
    [m(c[0], base[0]), m(c[1], base[1]), m(c[2], base[2])]
}

fn solid(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

/// Angles run clockwise from north; screen y points down.
fn polar(c: f32, r: f32, a: f32) -> (f32, f32) {
    (c + r * a.sin(), c - r * a.cos())
}

fn annular_sector(c: f32, r0: f32, r1: f32, a0: f32, a1: f32) -> Path {
    let steps = (((a1 - a0).abs() / 0.03).ceil() as usize).max(4);
    let mut pb = PathBuilder::new();
    let (x, y) = polar(c, r1, a0);
    pb.move_to(x, y);
    for i in 1..=steps {
        let (x, y) = polar(c, r1, a0 + (a1 - a0) * i as f32 / steps as f32);
        pb.line_to(x, y);
    }
    for i in (0..=steps).rev() {
        let (x, y) = polar(c, r0, a0 + (a1 - a0) * i as f32 / steps as f32);
        pb.line_to(x, y);
    }
    pb.close();
    pb.finish().unwrap()
}

fn circle(c: f32, r: f32) -> Path {
    PathBuilder::from_circle(c, c, r).unwrap()
}

/// A nearly invisible disc: layered windows only receive drag events on pixels with alpha > 0.
pub fn probe(size: u32, scale: f32) -> Pixmap {
    let mut pm = Pixmap::new(size, size).unwrap();
    let c = size as f32 / 2.0;
    pm.fill_path(&circle(c, OUTER * scale), &solid(rgba(0, 0, 0, 1.0 / 255.0)), FillRule::Winding, Transform::identity(), None);
    pm
}

pub fn ring(size: u32, scale: f32, m: &RingModel, fonts: &Fonts) -> Pixmap {
    let mut pm = Pixmap::new(size, size).unwrap();
    let k = scale;
    let c = size as f32 / 2.0;
    let (ro, ri) = (OUTER * k, INNER * k);
    let glass = m.style == Style::Glass;
    let n = m.actions.len().max(1);
    let step = std::f32::consts::TAU / n as f32;
    let base = [20, 22, 38];

    if glass {
        pm.fill_path(&circle(c, ro), &solid(rgba(34, 38, 72, 0.62)), FillRule::Winding, Transform::identity(), None);
    }

    let band_inner = ri + 6.0 * k;
    let band_outer = ro - 2.0 * k;
    for (i, a) in m.actions.iter().enumerate() {
        let mid = i as f32 * step;
        let path = annular_sector(c, band_inner, band_outer, mid - step / 2.0 + 0.02, mid + step / 2.0 - 0.02);
        let is_hot = m.hot == Some(i as i32) || (m.hot == Some(-1) && i == 0);

        let fill = if glass {
            if is_hot {
                solid(rgba(a.color[0], a.color[1], a.color[2], 0.55))
            } else {
                let mut p = Paint::default();
                p.anti_alias = true;
                p.shader = LinearGradient::new(
                    Point::from_xy(c, c - ro),
                    Point::from_xy(c, c + ro),
                    vec![GradientStop::new(0.0, rgba(255, 255, 255, 0.30)), GradientStop::new(1.0, rgba(255, 255, 255, 0.10))],
                    SpreadMode::Pad,
                    Transform::identity(),
                )
                .unwrap();
                p
            }
        } else {
            let col = if is_hot { mix(a.color, base, 0.32) } else { base };
            solid(rgba(col[0], col[1], col[2], 0.94))
        };
        pm.fill_path(&path, &fill, FillRule::Winding, Transform::identity(), None);

        let stroke_paint = if is_hot {
            solid(rgba(a.color[0], a.color[1], a.color[2], 1.0))
        } else if glass {
            let mut p = Paint::default();
            p.anti_alias = true;
            p.shader = LinearGradient::new(
                Point::from_xy(c, c - ro),
                Point::from_xy(c, c + ro),
                vec![GradientStop::new(0.0, rgba(255, 255, 255, 0.80)), GradientStop::new(1.0, rgba(255, 255, 255, 0.15))],
                SpreadMode::Pad,
                Transform::identity(),
            )
            .unwrap();
            p
        } else {
            solid(rgba(255, 255, 255, 0.12))
        };
        let stroke = Stroke { width: if is_hot { 1.8 * k } else { 1.0 * k }, ..Default::default() };
        pm.stroke_path(&path, &stroke_paint, &stroke, Transform::identity(), None);

        if i == 0 {
            let (dx, dy) = polar(c, band_outer - 8.0 * k, mid);
            let dot = PathBuilder::from_circle(dx, dy, 3.0 * k).unwrap();
            pm.fill_path(&dot, &solid(rgba(a.color[0], a.color[1], a.color[2], 1.0)), FillRule::Winding, Transform::identity(), None);
        }

        let (lx, ly) = polar(c, (band_inner + band_outer) / 2.0, mid);
        let max_w = 2.0 * ((band_inner + band_outer) / 2.0) * (step / 2.0).sin() * 0.9;
        draw_block(
            &mut pm,
            &[
                (&fonts.bold, 16.0 * k, a.title, [255, 255, 255, 255]),
                (&fonts.regular, 11.0 * k, a.sub, [255, 255, 255, 190]),
            ],
            lx,
            ly,
            4.0 * k,
            max_w.min(124.0 * k),
        );
    }

    // Center lens
    let lens = circle(c, ri);
    let center_hot = m.hot == Some(-1) && !m.actions.is_empty();
    if glass {
        let mut p = Paint::default();
        p.anti_alias = true;
        p.shader = RadialGradient::new(
            Point::from_xy(c - ri * 0.35, c - ri * 0.45),
            Point::from_xy(c, c),
            ri * 1.3,
            vec![GradientStop::new(0.0, rgba(255, 255, 255, 0.42)), GradientStop::new(1.0, rgba(60, 66, 110, 0.75))],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap_or(Shader::SolidColor(rgba(60, 66, 110, 0.8)));
        pm.fill_path(&lens, &p, FillRule::Winding, Transform::identity(), None);
    } else {
        pm.fill_path(&lens, &solid(rgba(14, 16, 28, 0.97)), FillRule::Winding, Transform::identity(), None);
    }
    let lens_stroke = if center_hot {
        let col = m.actions[0].color;
        solid(rgba(col[0], col[1], col[2], 1.0))
    } else {
        solid(rgba(255, 255, 255, if glass { 0.45 } else { 0.14 }))
    };
    pm.stroke_path(&lens, &lens_stroke, &Stroke { width: if center_hot { 2.0 * k } else { 1.0 * k }, ..Default::default() }, Transform::identity(), None);

    let (head, sub, head_color) = match m.hot {
        Some(h) if !m.actions.is_empty() => {
            let a = &m.actions[if h < 0 { 0 } else { h as usize }];
            (format!("→ {}", a.title), a.sub.to_string(), [a.color[0], a.color[1], a.color[2], 255])
        }
        _ if m.actions.is_empty() => ("…".to_string(), String::new(), [255, 255, 255, 255]),
        _ => (
            if m.count > 1 { format!("{} ×{}", m.kind, m.count) } else { m.kind.to_string() },
            format!("Mitte: {}", m.actions[0].title),
            [255, 255, 255, 255],
        ),
    };
    draw_block(
        &mut pm,
        &[(&fonts.bold, 17.0 * k, &head, head_color), (&fonts.regular, 11.0 * k, &sub, [255, 255, 255, 200])],
        c,
        c,
        4.0 * k,
        ri * 1.7,
    );
    pm
}

/// Scale-in animation frame: easeOutBack from 60 % plus a quick fade.
pub fn compose(src: &Pixmap, t: f32) -> Pixmap {
    let t = t.clamp(0.0, 1.0);
    let (c1, c3) = (1.70158f32, 2.70158f32);
    let ease = 1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2);
    let s = 0.6 + 0.4 * ease;
    let mut out = Pixmap::new(src.width(), src.height()).unwrap();
    let c = src.width() as f32 / 2.0;
    let tr = Transform::from_translate(c, c).pre_scale(s, s).pre_translate(-c, -c);
    let paint = PixmapPaint { opacity: (t * 3.0).min(1.0), blend_mode: BlendMode::SourceOver, quality: FilterQuality::Bilinear };
    out.draw_pixmap(0, 0, src.as_ref(), &paint, tr, None);
    out
}

// ---- Text ----

fn text_width(font: &FontVec, px: f32, s: &str) -> f32 {
    let sf = font.as_scaled(PxScale::from(px));
    let mut w = 0.0;
    let mut prev = None;
    for ch in s.chars() {
        let id = sf.glyph_id(ch);
        if let Some(p) = prev {
            w += sf.kern(p, id);
        }
        w += sf.h_advance(id);
        prev = Some(id);
    }
    w
}

/// Height of a capital letter, used instead of the font's ascent so text looks optically centered.
fn cap_height(font: &FontVec, px: f32) -> f32 {
    let g = font.glyph_id('H').with_scale_and_position(PxScale::from(px), point(0.0, 0.0));
    font.outline_glyph(g).map(|o| o.px_bounds().height()).unwrap_or(px * 0.7)
}

fn draw_line(pm: &mut Pixmap, font: &FontVec, px: f32, s: &str, cx: f32, baseline: f32, color: [u8; 4]) {
    let sf = font.as_scaled(PxScale::from(px));
    let mut x = cx - text_width(font, px, s) / 2.0;
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    let mut prev = None;
    for ch in s.chars() {
        let id = sf.glyph_id(ch);
        if let Some(p) = prev {
            x += sf.kern(p, id);
        }
        let g = id.with_scale_and_position(PxScale::from(px), point(x, baseline));
        if let Some(outline) = font.outline_glyph(g) {
            let b = outline.px_bounds();
            outline.draw(|gx, gy, cov| {
                let px_x = b.min.x as i32 + gx as i32;
                let px_y = b.min.y as i32 + gy as i32;
                if px_x < 0 || px_y < 0 || px_x >= w || px_y >= h {
                    return;
                }
                let i = ((px_y * w + px_x) * 4) as usize;
                let sa = cov.clamp(0.0, 1.0) * color[3] as f32 / 255.0;
                let inv = 1.0 - sa;
                for ch in 0..3 {
                    data[i + ch] = (color[ch] as f32 * sa + data[i + ch] as f32 * inv).round().min(255.0) as u8;
                }
                data[i + 3] = (255.0 * sa + data[i + 3] as f32 * inv).round().min(255.0) as u8;
            });
        }
        x += sf.h_advance(id);
        prev = Some(id);
    }
}

/// Draws lines centered as one block around (cx, cy); shrinks a line if it is wider than max_w.
fn draw_block(pm: &mut Pixmap, lines: &[(&FontVec, f32, &str, [u8; 4])], cx: f32, cy: f32, gap: f32, max_w: f32) {
    let lines: Vec<_> = lines.iter().filter(|l| !l.2.is_empty()).collect();
    let sized: Vec<(f32, f32, f32)> = lines
        .iter()
        .map(|(font, px, s, _)| {
            let w = text_width(font, *px, s);
            let px = if w > max_w { px * max_w / w } else { *px };
            let cap = cap_height(font, px);
            let desc = -font.as_scaled(PxScale::from(px)).descent();
            (px, cap, cap + desc * 0.35)
        })
        .collect();
    let total: f32 = sized.iter().map(|s| s.2).sum::<f32>() + gap * (sized.len().saturating_sub(1)) as f32;
    let mut top = cy - total / 2.0;
    for ((font, _, s, color), (px, cap, box_h)) in lines.iter().zip(sized) {
        draw_line(pm, font, px, s, cx, top + cap, *color);
        top += box_h + gap;
    }
}
