//! SVG to drawing: outlines of all paths, curves flattened, user units converted to millimetres.

use anyhow::{anyhow, bail, Result};
use resvg::usvg::{self, tiny_skia_path::PathSegment, Node};

use super::dxf::{Drawing, Path, P};

const MM_PER_PX: f64 = 25.4 / 96.0;

fn walk(g: &usvg::Group, layer: &str, out: &mut Vec<Path>) {
    for node in g.children() {
        match node {
            Node::Group(sub) => {
                let id = sub.id();
                walk(sub, if id.is_empty() { layer } else { id }, out);
            }
            Node::Path(p) => path(p, layer, out),
            Node::Text(t) => walk(t.flattened(), layer, out),
            Node::Image(_) => {}
        }
    }
}

fn path(p: &usvg::Path, layer: &str, out: &mut Vec<Path>) {
    let t = p.abs_transform();
    let map = |pt: usvg::tiny_skia_path::Point| -> P {
        let (x, y) = (t.sx * pt.x + t.kx * pt.y + t.tx, t.ky * pt.x + t.sy * pt.y + t.ty);
        (x as f64, y as f64)
    };
    let mut cur: Vec<P> = Vec::new();
    let mut last: P = (0.0, 0.0);
    let flush = |cur: &mut Vec<P>, closed: bool, out: &mut Vec<Path>| {
        if cur.len() >= 2 {
            out.push(Path { layer: layer.to_string(), pts: std::mem::take(cur), closed });
        }
        cur.clear();
    };
    for seg in p.data().segments() {
        match seg {
            PathSegment::MoveTo(a) => {
                flush(&mut cur, false, out);
                last = map(a);
                cur.push(last);
            }
            PathSegment::LineTo(a) => {
                last = map(a);
                cur.push(last);
            }
            PathSegment::QuadTo(c, a) => {
                let (c, a) = (map(c), map(a));
                for i in 1..=16 {
                    let u = i as f64 / 16.0;
                    let v = 1.0 - u;
                    cur.push((v * v * last.0 + 2.0 * v * u * c.0 + u * u * a.0, v * v * last.1 + 2.0 * v * u * c.1 + u * u * a.1));
                }
                last = a;
            }
            PathSegment::CubicTo(c1, c2, a) => {
                let (c1, c2, a) = (map(c1), map(c2), map(a));
                for i in 1..=24 {
                    let u = i as f64 / 24.0;
                    let v = 1.0 - u;
                    let w = [v * v * v, 3.0 * v * v * u, 3.0 * v * u * u, u * u * u];
                    cur.push((
                        w[0] * last.0 + w[1] * c1.0 + w[2] * c2.0 + w[3] * a.0,
                        w[0] * last.1 + w[1] * c1.1 + w[2] * c2.1 + w[3] * a.1,
                    ));
                }
                last = a;
            }
            PathSegment::Close => {
                if cur.len() > 2 && (cur[0].0 - last.0).abs() < 1e-9 && (cur[0].1 - last.1).abs() < 1e-9 {
                    cur.pop();
                }
                let start = cur.first().copied();
                flush(&mut cur, true, out);
                if let Some(s) = start {
                    last = s;
                    cur.push(s);
                }
            }
        }
    }
    flush(&mut cur, false, out);
}

pub fn read(bytes: &[u8]) -> Result<Drawing> {
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default()).map_err(|e| anyhow!("SVG konnte nicht gelesen werden: {e}"))?;
    let mut paths = Vec::new();
    walk(tree.root(), "0", &mut paths);
    if paths.is_empty() {
        bail!("Die SVG enthält keine Pfade, die sich als Linien übernehmen lassen");
    }
    let h = tree.size().height() as f64;
    for p in &mut paths {
        for pt in &mut p.pts {
            *pt = (pt.0 * MM_PER_PX, (h - pt.1) * MM_PER_PX);
        }
    }
    Ok(Drawing { paths })
}
