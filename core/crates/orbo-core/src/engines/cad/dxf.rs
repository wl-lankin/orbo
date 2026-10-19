//! ASCII DXF reader (R12 to 2018). Flattens the drawing into polylines in millimetres.
//!
//! Geometry handling follows the Shaggy reader: bulge arcs, OCS mirroring (extrusion -Z), block
//! transforms with non-uniform scale, ellipse parameters, B-spline evaluation and $INSUNITS.

use std::collections::{HashMap, HashSet};
use std::f64::consts::{PI, TAU};

use anyhow::{bail, Result};

pub type P = (f64, f64);

#[derive(Debug, Clone)]
pub struct Path {
    pub layer: String,
    pub pts: Vec<P>,
    pub closed: bool,
}

#[derive(Debug, Default)]
pub struct Drawing {
    pub paths: Vec<Path>,
}

impl Drawing {
    pub fn bounds(&self) -> Option<(P, P)> {
        let mut it = self.paths.iter().flat_map(|p| p.pts.iter());
        let first = *it.next()?;
        let (mut lo, mut hi) = (first, first);
        for &(x, y) in it {
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x), hi.1.max(y));
        }
        Some((lo, hi))
    }

    /// Layer names in order of first use.
    pub fn layers(&self) -> Vec<&str> {
        let mut seen = HashSet::new();
        self.paths.iter().filter(|p| seen.insert(p.layer.as_str())).map(|p| p.layer.as_str()).collect()
    }
}

/// Maximum deviation of flattened curves from the true curve, in mm.
const TOL_MM: f64 = 0.02;
/// Upper bound for the number of points a drawing may expand to (blocks included).
const MAX_POINTS: usize = 8_000_000;

// ---------------------------------------------------------------- affine

#[derive(Clone, Copy)]
struct Aff {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Aff {
    const ID: Aff = Aff { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    fn scale(sx: f64, sy: f64) -> Aff {
        Aff { a: sx, d: sy, ..Aff::ID }
    }

    fn translate(x: f64, y: f64) -> Aff {
        Aff { e: x, f: y, ..Aff::ID }
    }

    fn rotate(rad: f64) -> Aff {
        let (s, c) = rad.sin_cos();
        Aff { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }

    /// self after other.
    fn then(self, o: Aff) -> Aff {
        Aff {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            e: self.a * o.e + self.c * o.f + self.e,
            f: self.b * o.e + self.d * o.f + self.f,
        }
    }

    fn apply(&self, (x, y): P) -> P {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    fn factor(&self) -> f64 {
        (self.a * self.d - self.b * self.c).abs().sqrt()
    }
}

// ---------------------------------------------------------------- raw groups

struct Ent {
    ty: String,
    g: Vec<(i32, String)>,
}

impl Ent {
    fn s(&self, code: i32) -> Option<&str> {
        self.g.iter().find(|(c, _)| *c == code).map(|(_, v)| v.as_str())
    }

    fn f(&self, code: i32) -> Option<f64> {
        self.s(code).and_then(|v| v.trim().parse().ok())
    }

    fn fd(&self, code: i32, default: f64) -> f64 {
        self.f(code).unwrap_or(default)
    }

    fn i(&self, code: i32) -> i64 {
        self.f(code).map(|v| v as i64).unwrap_or(0)
    }

    fn all(&self, code: i32) -> Vec<f64> {
        self.g.iter().filter(|(c, _)| *c == code).filter_map(|(_, v)| v.trim().parse().ok()).collect()
    }
}

fn read_pairs(text: &str) -> Vec<(i32, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<(i32, String)> = Vec::with_capacity(lines.len() / 2);
    let mut i = 0;
    while i < lines.len() {
        match (lines[i].trim().parse::<i32>(), lines.get(i + 1)) {
            (Ok(code), Some(v)) => {
                out.push((code, v.trim().to_string()));
                i += 2;
            }
            // dwg2dxf leaves line breaks of text values unquoted; glue them back onto the value.
            _ => {
                if let Some(last) = out.last_mut() {
                    last.1.push('\n');
                    last.1.push_str(lines[i].trim());
                }
                i += 1;
            }
        }
    }
    out
}

fn split_ents(pairs: &[(i32, String)]) -> Vec<Ent> {
    let mut out: Vec<Ent> = Vec::new();
    for (code, v) in pairs {
        if *code == 0 {
            out.push(Ent { ty: v.to_uppercase(), g: Vec::new() });
        } else if let Some(e) = out.last_mut() {
            e.g.push((*code, v.clone()));
        }
    }
    out
}

struct Block {
    base: P,
    ents: Vec<Ent>,
}

struct Parsed {
    insunits: i64,
    hidden: HashSet<String>,
    blocks: HashMap<String, Block>,
    entities: Vec<Ent>,
}

fn parse(text: &str) -> Result<Parsed> {
    let pairs = read_pairs(text);
    let mut p = Parsed { insunits: 0, hidden: HashSet::new(), blocks: HashMap::new(), entities: Vec::new() };
    let mut i = 0;
    let mut sections = 0;
    while i < pairs.len() {
        if pairs[i].0 == 0 && pairs[i].1 == "SECTION" && i + 1 < pairs.len() && pairs[i + 1].0 == 2 {
            sections += 1;
            let name = pairs[i + 1].1.to_uppercase();
            let start = i + 2;
            let mut end = start;
            while end < pairs.len() && !(pairs[end].0 == 0 && pairs[end].1 == "ENDSEC") {
                end += 1;
            }
            let body = &pairs[start..end.min(pairs.len())];
            match name.as_str() {
                "HEADER" => {
                    for w in body.windows(2) {
                        if w[0].0 == 9 && w[0].1 == "$INSUNITS" {
                            p.insunits = w[1].1.trim().parse().unwrap_or(0);
                        }
                    }
                }
                "TABLES" => {
                    for e in split_ents(body).iter().filter(|e| e.ty == "LAYER") {
                        let off = e.fd(62, 1.0) < 0.0 || e.i(70) & 1 != 0;
                        if let (true, Some(n)) = (off, e.s(2)) {
                            p.hidden.insert(n.to_string());
                        }
                    }
                }
                "BLOCKS" => {
                    let mut cur: Option<(String, Block)> = None;
                    for e in split_ents(body) {
                        match e.ty.as_str() {
                            "BLOCK" => {
                                let base = (e.fd(10, 0.0), e.fd(20, 0.0));
                                cur = Some((e.s(2).unwrap_or("").to_uppercase(), Block { base, ents: Vec::new() }));
                            }
                            "ENDBLK" => {
                                if let Some((n, b)) = cur.take() {
                                    p.blocks.insert(n, b);
                                }
                            }
                            _ => {
                                if let Some((_, b)) = cur.as_mut() {
                                    b.ents.push(e);
                                }
                            }
                        }
                    }
                }
                "ENTITIES" => p.entities = split_ents(body),
                _ => {}
            }
            i = end;
        } else {
            i += 1;
        }
    }
    if sections == 0 {
        bail!("Das ist keine gültige DXF-Datei");
    }
    Ok(p)
}

fn unit_factor(code: i64) -> f64 {
    match code {
        1 => 25.4,
        2 => 304.8,
        3 => 1_609_344.0,
        5 => 10.0,
        6 => 1000.0,
        7 => 1_000_000.0,
        8 => 2.54e-5,
        9 => 0.0254,
        10 => 914.4,
        11 => 1e-7,
        12 => 1e-6,
        13 => 1e-3,
        14 => 100.0,
        15 => 10_000.0,
        16 => 100_000.0,
        _ => 1.0,
    }
}

// ---------------------------------------------------------------- flattening

fn arc_steps(r: f64, sweep: f64, tol: f64) -> usize {
    let r = r.abs();
    let step = if r <= tol { PI / 4.0 } else { 2.0 * (1.0 - tol / r).acos() };
    let n = (sweep.abs() / step.max(1e-4)).ceil() as usize;
    n.clamp(1, 5000)
}

fn arc_points(c: P, r: f64, a0: f64, sweep: f64, tol: f64) -> Vec<P> {
    let mut n = arc_steps(r, sweep, tol);
    if sweep.abs() >= TAU - 1e-9 {
        n = n.max(16);
    }
    (0..=n)
        .map(|i| {
            let a = a0 + sweep * i as f64 / n as f64;
            (c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect()
}

/// Points of a bulge segment from p1 to p2 (p1 excluded). Positive bulge runs counter-clockwise.
fn bulge_points(p1: P, p2: P, bulge: f64, tol: f64) -> Vec<P> {
    let (dx, dy) = (p2.0 - p1.0, p2.1 - p1.1);
    let chord = dx.hypot(dy);
    if chord < 1e-12 || bulge.abs() < 1e-12 {
        return vec![p2];
    }
    let sweep = 4.0 * bulge.atan();
    let sagitta = bulge.abs() * chord / 2.0;
    let r = (chord * chord / 4.0 + sagitta * sagitta) / (2.0 * sagitta);
    let (nx, ny) = (-dy / chord, dx / chord);
    let off = (r - sagitta) * bulge.signum();
    let c = ((p1.0 + p2.0) / 2.0 + nx * off, (p1.1 + p2.1) / 2.0 + ny * off);
    let a0 = (p1.1 - c.1).atan2(p1.0 - c.0);
    let mut pts = arc_points(c, r, a0, sweep, tol);
    pts.remove(0);
    if let Some(last) = pts.last_mut() {
        *last = p2;
    }
    pts
}

fn poly_points(v: &[(f64, f64, f64)], closed: bool, tol: f64) -> Vec<P> {
    if v.is_empty() {
        return Vec::new();
    }
    let mut pts = vec![(v[0].0, v[0].1)];
    let segs = if closed { v.len() } else { v.len() - 1 };
    for i in 0..segs {
        let (a, b) = (v[i], v[(i + 1) % v.len()]);
        pts.extend(bulge_points((a.0, a.1), (b.0, b.1), a.2, tol));
    }
    if closed && pts.len() > 1 && same(pts[0], *pts.last().unwrap()) {
        pts.pop();
    }
    pts
}

fn same(a: P, b: P) -> bool {
    (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
}

fn de_boor(k: usize, t: f64, deg: usize, knots: &[f64], ctrl: &[(f64, f64, f64)]) -> (f64, f64, f64) {
    let mut d: Vec<(f64, f64, f64)> = (0..=deg).map(|j| ctrl[j + k - deg]).collect();
    for r in 1..=deg {
        for j in (r..=deg).rev() {
            let i = j + k - deg;
            let den = knots[i + deg + 1 - r] - knots[i];
            let a = if den.abs() < 1e-14 { 0.0 } else { (t - knots[i]) / den };
            d[j] = (
                d[j - 1].0 * (1.0 - a) + d[j].0 * a,
                d[j - 1].1 * (1.0 - a) + d[j].1 * a,
                d[j - 1].2 * (1.0 - a) + d[j].2 * a,
            );
        }
    }
    d[deg]
}

fn zip_xy(xs: Vec<f64>, ys: Vec<f64>) -> Vec<P> {
    xs.into_iter().zip(ys).collect()
}

fn spline_points(e: &Ent) -> (Vec<P>, bool) {
    let flags = e.i(70);
    let deg = e.i(71).max(1) as usize;
    let knots = e.all(40);
    let cps = zip_xy(e.all(10), e.all(20));
    let weights = e.all(41);
    let fit = zip_xy(e.all(11), e.all(21));
    let n = cps.len();
    let mut pts: Vec<P> = Vec::new();
    if n > deg && knots.len() == n + deg + 1 {
        let ctrl: Vec<(f64, f64, f64)> = cps
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| {
                let w = weights.get(i).copied().filter(|w| *w > 0.0).unwrap_or(1.0);
                (x * w, y * w, w)
            })
            .collect();
        for k in deg..n {
            let (t0, t1) = (knots[k], knots[k + 1]);
            if t1 - t0 < 1e-12 {
                continue;
            }
            let steps = 16;
            for s in 0..=steps {
                if s == 0 && !pts.is_empty() {
                    continue;
                }
                let t = t0 + (t1 - t0) * s as f64 / steps as f64;
                let (x, y, w) = de_boor(k, t, deg, &knots, &ctrl);
                if w.abs() > 1e-12 {
                    pts.push((x / w, y / w));
                }
            }
        }
    }
    if pts.len() < 2 && fit.len() >= 2 {
        pts = catmull_rom(&fit);
    }
    if pts.len() < 2 {
        pts = cps;
    }
    let closed = flags & 1 != 0 && pts.len() > 2;
    if closed && same(pts[0], *pts.last().unwrap()) {
        pts.pop();
    }
    (pts, closed)
}

fn catmull_rom(f: &[P]) -> Vec<P> {
    let mut out = vec![f[0]];
    for i in 0..f.len() - 1 {
        let p0 = f[i.saturating_sub(1)];
        let (p1, p2) = (f[i], f[i + 1]);
        let p3 = f[(i + 2).min(f.len() - 1)];
        for s in 1..=12 {
            let t = s as f64 / 12.0;
            let q = |a: f64, b: f64, c: f64, d: f64| {
                0.5 * (2.0 * b + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t + (-a + 3.0 * b - 3.0 * c + d) * t * t * t)
            };
            out.push((q(p0.0, p1.0, p2.0, p3.0), q(p0.1, p1.1, p2.1, p3.1)));
        }
    }
    out
}

// ---------------------------------------------------------------- OCS

/// Object coordinate system via the arbitrary axis algorithm; None when it equals the WCS.
struct Ocs {
    ax: (f64, f64, f64),
    ay: (f64, f64, f64),
    n: (f64, f64, f64),
}

fn cross(a: (f64, f64, f64), b: (f64, f64, f64)) -> (f64, f64, f64) {
    (a.1 * b.2 - a.2 * b.1, a.2 * b.0 - a.0 * b.2, a.0 * b.1 - a.1 * b.0)
}

fn unit(v: (f64, f64, f64)) -> (f64, f64, f64) {
    let l = (v.0 * v.0 + v.1 * v.1 + v.2 * v.2).sqrt();
    if l < 1e-12 {
        (0.0, 0.0, 1.0)
    } else {
        (v.0 / l, v.1 / l, v.2 / l)
    }
}

fn ocs_of(e: &Ent) -> Option<Ocs> {
    let n = unit((e.fd(210, 0.0), e.fd(220, 0.0), e.fd(230, 1.0)));
    if (n.0).abs() < 1e-9 && (n.1).abs() < 1e-9 && n.2 > 0.0 {
        return None;
    }
    let ax = if n.0.abs() < 1.0 / 64.0 && n.1.abs() < 1.0 / 64.0 { cross((0.0, 1.0, 0.0), n) } else { cross((0.0, 0.0, 1.0), n) };
    let ax = unit(ax);
    let ay = unit(cross(n, ax));
    Some(Ocs { ax, ay, n })
}

impl Ocs {
    fn map(&self, (x, y): P, z: f64) -> P {
        (x * self.ax.0 + y * self.ay.0 + z * self.n.0, x * self.ax.1 + y * self.ay.1 + z * self.n.1)
    }
}

fn to_wcs(ocs: &Option<Ocs>, pts: Vec<P>, z: f64) -> Vec<P> {
    match ocs {
        Some(o) => pts.into_iter().map(|p| o.map(p, z)).collect(),
        None => pts,
    }
}

// ---------------------------------------------------------------- walking

struct Walk<'a> {
    p: &'a Parsed,
    out: Vec<Path>,
    points: usize,
}

impl<'a> Walk<'a> {
    fn emit(&mut self, pts: Vec<P>, closed: bool, layer: &str, aff: &Aff) -> Result<()> {
        if pts.len() < 2 || pts.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
            return Ok(());
        }
        self.points += pts.len();
        if self.points > MAX_POINTS {
            bail!("Die Zeichnung ist zu komplex (mehr als {MAX_POINTS} Punkte)");
        }
        self.out.push(Path { layer: layer.to_string(), pts: pts.into_iter().map(|p| aff.apply(p)).collect(), closed });
        Ok(())
    }

    fn walk(&mut self, ents: &[Ent], aff: Aff, parent: Option<&str>, depth: usize) -> Result<()> {
        let mut i = 0;
        while i < ents.len() {
            let e = &ents[i];
            i += 1;
            let layer = match (e.s(8).unwrap_or("0"), parent) {
                ("0", Some(p)) => p,
                (l, _) => l,
            };
            // Vertices of an old-style POLYLINE follow as separate entities.
            let mut verts: &[Ent] = &[];
            if e.ty == "POLYLINE" {
                let start = i;
                while i < ents.len() && ents[i].ty == "VERTEX" {
                    i += 1;
                }
                verts = &ents[start..i];
            }
            if self.p.hidden.contains(layer) || layer.eq_ignore_ascii_case("DEFPOINTS") || e.i(60) == 1 {
                continue;
            }
            let tol = TOL_MM / aff.factor().max(1e-9);
            self.entity(e, verts, layer, aff, tol, depth)?;
        }
        Ok(())
    }

    fn entity(&mut self, e: &Ent, verts: &[Ent], layer: &str, aff: Aff, tol: f64, depth: usize) -> Result<()> {
        match e.ty.as_str() {
            "LINE" => {
                let pts = vec![(e.fd(10, 0.0), e.fd(20, 0.0)), (e.fd(11, 0.0), e.fd(21, 0.0))];
                self.emit(pts, false, layer, &aff)
            }
            "LWPOLYLINE" => {
                let mut v: Vec<(f64, f64, f64)> = Vec::new();
                for (c, val) in &e.g {
                    let x: f64 = val.trim().parse().unwrap_or(0.0);
                    match c {
                        10 => v.push((x, 0.0, 0.0)),
                        20 => {
                            if let Some(l) = v.last_mut() {
                                l.1 = x;
                            }
                        }
                        42 => {
                            if let Some(l) = v.last_mut() {
                                l.2 = x;
                            }
                        }
                        _ => {}
                    }
                }
                let closed = e.i(70) & 1 != 0;
                let pts = to_wcs(&ocs_of(e), poly_points(&v, closed, tol), e.fd(38, 0.0));
                self.emit(pts, closed, layer, &aff)
            }
            "POLYLINE" => {
                let flags = e.i(70);
                // Polygon/polyface meshes carry no outline here.
                if flags & (16 | 64) != 0 {
                    return Ok(());
                }
                let v: Vec<(f64, f64, f64)> = verts
                    .iter()
                    .filter(|v| v.i(70) & 16 == 0)
                    .map(|v| (v.fd(10, 0.0), v.fd(20, 0.0), v.fd(42, 0.0)))
                    .collect();
                let closed = flags & 1 != 0;
                let ocs = if flags & 8 != 0 { None } else { ocs_of(e) };
                let pts = to_wcs(&ocs, poly_points(&v, closed, tol), e.fd(30, 0.0));
                self.emit(pts, closed, layer, &aff)
            }
            "CIRCLE" => {
                let r = e.fd(40, 0.0);
                if r <= 0.0 {
                    return Ok(());
                }
                let mut pts = arc_points((e.fd(10, 0.0), e.fd(20, 0.0)), r, 0.0, TAU, tol);
                pts.pop();
                let pts = to_wcs(&ocs_of(e), pts, e.fd(30, 0.0));
                self.emit(pts, true, layer, &aff)
            }
            "ARC" => {
                let r = e.fd(40, 0.0);
                if r <= 0.0 {
                    return Ok(());
                }
                let (a0, a1) = (e.fd(50, 0.0), e.fd(51, 0.0));
                let mut span = (a1 - a0).rem_euclid(360.0);
                if span < 1e-9 && (a1 - a0).abs() > 1e-9 {
                    span = 360.0;
                }
                let pts = arc_points((e.fd(10, 0.0), e.fd(20, 0.0)), r, a0.to_radians(), span.to_radians(), tol);
                let pts = to_wcs(&ocs_of(e), pts, e.fd(30, 0.0));
                self.emit(pts, false, layer, &aff)
            }
            "ELLIPSE" => {
                let c = (e.fd(10, 0.0), e.fd(20, 0.0));
                let major = (e.fd(11, 1.0), e.fd(21, 0.0));
                let ratio = e.fd(40, 1.0);
                let flip = e.fd(230, 1.0) < 0.0;
                let minor = if flip { (major.1 * ratio, -major.0 * ratio) } else { (-major.1 * ratio, major.0 * ratio) };
                let (t0, t1) = (e.fd(41, 0.0), e.fd(42, TAU));
                let mut span = (t1 - t0).rem_euclid(TAU);
                if span < 1e-9 {
                    span = TAU;
                }
                let r = major.0.hypot(major.1);
                let n = arc_steps(r, span, tol).max(if span >= TAU - 1e-9 { 16 } else { 2 });
                let mut pts: Vec<P> = (0..=n)
                    .map(|i| {
                        let t = t0 + span * i as f64 / n as f64;
                        (c.0 + major.0 * t.cos() + minor.0 * t.sin(), c.1 + major.1 * t.cos() + minor.1 * t.sin())
                    })
                    .collect();
                let closed = span >= TAU - 1e-9;
                if closed {
                    pts.pop();
                }
                self.emit(pts, closed, layer, &aff)
            }
            "SPLINE" => {
                let (pts, closed) = spline_points(e);
                self.emit(pts, closed, layer, &aff)
            }
            "INSERT" => self.insert(e, layer, aff, depth),
            // The anonymous block of a dimension holds its lines and arrows in WCS.
            "DIMENSION" => {
                let p = self.p;
                if let (Some(b), true) = (e.s(2).and_then(|n| p.blocks.get(&n.to_uppercase())), depth < 10) {
                    self.walk(&b.ents, aff, Some(layer), depth + 1)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn insert(&mut self, e: &Ent, layer: &str, aff: Aff, depth: usize) -> Result<()> {
        if depth >= 10 {
            return Ok(());
        }
        let p = self.p;
        let Some(block) = e.s(2).and_then(|n| p.blocks.get(&n.to_uppercase())) else { return Ok(()) };
        let ents: &'a [Ent] = &block.ents;
        let base = block.base;
        let (sx, sy) = (e.fd(41, 1.0), e.fd(42, 1.0));
        let rot = e.fd(50, 0.0).to_radians();
        let cols = e.i(70).max(1) as usize;
        let rows = e.i(71).max(1) as usize;
        if cols * rows > 100_000 {
            bail!("Die Zeichnung ist zu komplex (Blockfeld mit {cols} x {rows} Elementen)");
        }
        let (dc, dr) = (e.fd(44, 0.0), e.fd(45, 0.0));
        let ocs = match ocs_of(e) {
            Some(o) => Aff { a: o.ax.0, b: o.ax.1, c: o.ay.0, d: o.ay.1, e: 0.0, f: 0.0 },
            None => Aff::ID,
        };
        let r = Aff::rotate(rot);
        for row in 0..rows {
            for col in 0..cols {
                let off = r.apply((col as f64 * dc, row as f64 * dr));
                let local = Aff::translate(e.fd(10, 0.0) + off.0, e.fd(20, 0.0) + off.1)
                    .then(r)
                    .then(Aff::scale(sx, sy))
                    .then(Aff::translate(-base.0, -base.1));
                self.walk(ents, aff.then(ocs).then(local), Some(layer), depth + 1)?;
            }
        }
        Ok(())
    }
}

fn walk_scaled(parsed: &Parsed, unit: f64) -> Result<Vec<Path>> {
    let mut w = Walk { p: parsed, out: Vec::new(), points: 0 };
    w.walk(&parsed.entities, Aff::scale(unit, unit), None, 0)?;
    Ok(std::mem::take(&mut w.out))
}

/// Many exporters write $INSUNITS = metres (or similar) while the coordinates are millimetres.
/// A declared unit that would turn a part-sized drawing into something larger than 50 m is ignored.
fn plausible_unit(parsed: &Parsed) -> Result<f64> {
    let unit = unit_factor(parsed.insunits);
    if unit <= 1.0 {
        return Ok(unit);
    }
    let raw = Drawing { paths: walk_scaled(parsed, 1.0)? };
    let Some((lo, hi)) = raw.bounds() else { return Ok(unit) };
    let extent = (hi.0 - lo.0).max(hi.1 - lo.1);
    Ok(if extent * unit > 50_000.0 && extent <= 50_000.0 { 1.0 } else { unit })
}

/// Reads a DXF file (bytes) into millimetre polylines.
pub fn read(bytes: &[u8]) -> Result<Drawing> {
    if bytes.starts_with(b"AutoCAD Binary DXF") {
        bail!("Binäre DXF-Dateien werden nicht unterstützt. Bitte als ASCII-DXF speichern");
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    };
    let parsed = parse(&text)?;
    let out = walk_scaled(&parsed, plausible_unit(&parsed)?)?;
    if out.is_empty() {
        bail!("Die Zeichnung enthält keine darstellbare Geometrie (Linien, Bögen, Kreise, Polylinien, Splines)");
    }
    Ok(Drawing { paths: out })
}
