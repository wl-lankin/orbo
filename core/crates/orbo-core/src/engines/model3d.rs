//! 3D models: triangle meshes between STL, OBJ, PLY, glTF/GLB, 3MF (and DAE as input).
//! Coordinates are never rescaled. Materials, colors, textures and animations are dropped.
//!
//! params: `to` = stl | obj | ply | glb | 3mf | png (png renders a preview image)

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::{Ctx, Engine, Request};
use crate::format::Format;

pub struct Engine_;

#[derive(Default)]
struct Mesh {
    pos: Vec<[f32; 3]>,
    tri: Vec<[u32; 3]>,
}

impl Mesh {
    fn triangle(&self, t: &[u32; 3]) -> [[f32; 3]; 3] {
        [self.pos[t[0] as usize], self.pos[t[1] as usize], self.pos[t[2] as usize]]
    }

    fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for p in &self.pos {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        (min, max)
    }

    fn append(&mut self, other: Mesh, m: Option<&[f32; 12]>) {
        let base = self.pos.len() as u32;
        self.pos.extend(other.pos.into_iter().map(|p| m.map_or(p, |m| apply12(m, p))));
        self.tri.extend(other.tri.into_iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
    }
}

/// Merges identical positions so triangle soups (STL) become indexed meshes.
struct Welder {
    map: HashMap<[u32; 3], u32>,
}

impl Welder {
    fn new() -> Welder {
        Welder { map: HashMap::new() }
    }

    fn add(&mut self, mesh: &mut Mesh, p: [f32; 3]) -> u32 {
        // -0.0 and 0.0 must weld together
        let key = [(p[0] + 0.0).to_bits(), (p[1] + 0.0).to_bits(), (p[2] + 0.0).to_bits()];
        *self.map.entry(key).or_insert_with(|| {
            mesh.pos.push(p);
            (mesh.pos.len() - 1) as u32
        })
    }
}

impl Engine for Engine_ {
    fn convert(&self, req: &Request, ctx: &Ctx) -> Result<()> {
        let to = req.str("to").ok_or_else(|| anyhow!("Zielformat fehlt"))?;
        let from = req.analysis.format;
        (ctx.progress)(0.05);
        let mesh = read_mesh(req.input, from)?;
        if mesh.tri.is_empty() {
            bail!("Das Modell enthält keine Dreiecke, die sich umwandeln ließen");
        }
        (ctx.progress)(0.5);
        if (ctx.cancelled)() {
            bail!("abgebrochen");
        }
        match to {
            "stl" => fs::write(req.output, write_stl(&mesh)).map_err(Into::into),
            "obj" => fs::write(req.output, write_obj(&mesh)).map_err(Into::into),
            "ply" => fs::write(req.output, write_ply(&mesh)).map_err(Into::into),
            "glb" => fs::write(req.output, write_glb(&mesh)).map_err(Into::into),
            "3mf" => write_3mf(&mesh, req.output),
            "png" => {
                let z_up = matches!(from, Format::Stl | Format::Ply | Format::ThreeMf);
                render_preview(&mesh, z_up).save(req.output).map_err(Into::into)
            }
            other => bail!("Ausgabeformat {other} wird von der 3D-Umwandlung nicht unterstützt"),
        }
        .context("Ausgabe konnte nicht geschrieben werden")?;
        (ctx.progress)(0.95);
        Ok(())
    }
}

// ---------- reading ----------

fn read_mesh(path: &Path, from: Format) -> Result<Mesh> {
    let mesh = match from {
        Format::Stl => read_stl(&fs::read(path)?),
        Format::Obj => read_obj(&String::from_utf8_lossy(&fs::read(path)?)),
        Format::Ply => read_ply(&fs::read(path)?),
        Format::Glb | Format::Gltf => read_gltf(path),
        Format::ThreeMf => read_3mf(path),
        Format::Dae => read_dae(&fs::read(path)?),
        other => bail!("Das Format {} kann nicht gelesen werden", other.id()),
    }?;
    if mesh.pos.iter().any(|p| p.iter().any(|c| !c.is_finite())) {
        bail!("Das Modell enthält ungültige Koordinaten");
    }
    Ok(mesh)
}

fn f32le(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn read_stl(data: &[u8]) -> Result<Mesh> {
    let binary_len = (data.len() >= 84).then(|| 84 + 50 * u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize);
    let looks_ascii = data.starts_with(b"solid") && binary_len != Some(data.len());
    let mut mesh = Mesh::default();
    let mut w = Welder::new();
    if looks_ascii {
        let text = String::from_utf8_lossy(data);
        let mut tokens = text.split_whitespace();
        let mut corner = Vec::with_capacity(3);
        while let Some(t) = tokens.next() {
            if t != "vertex" {
                continue;
            }
            let mut p = [0f32; 3];
            for c in &mut p {
                *c = tokens.next().and_then(|v| v.parse().ok()).ok_or_else(|| anyhow!("Ungültige STL-Datei (Koordinate fehlt)"))?;
            }
            corner.push(w.add(&mut mesh, p));
            if corner.len() == 3 {
                mesh.tri.push([corner[0], corner[1], corner[2]]);
                corner.clear();
            }
        }
        return Ok(mesh);
    }
    if data.len() < 84 {
        bail!("Ungültige STL-Datei (zu kurz)");
    }
    let count = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;
    let count = count.min((data.len() - 84) / 50);
    for i in 0..count {
        let base = 84 + i * 50 + 12;
        let mut ids = [0u32; 3];
        for (k, id) in ids.iter_mut().enumerate() {
            let o = base + k * 12;
            *id = w.add(&mut mesh, [f32le(data, o), f32le(data, o + 4), f32le(data, o + 8)]);
        }
        mesh.tri.push(ids);
    }
    Ok(mesh)
}

fn read_obj(text: &str) -> Result<Mesh> {
    let mut mesh = Mesh::default();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let v: Vec<f32> = it.take(3).filter_map(|s| s.parse().ok()).collect();
                if v.len() < 3 {
                    bail!("Ungültige OBJ-Datei (Punkt unvollständig)");
                }
                mesh.pos.push([v[0], v[1], v[2]]);
            }
            Some("f") => {
                let n = mesh.pos.len() as i64;
                let mut ids = Vec::new();
                for s in it {
                    let i: i64 = s.split('/').next().unwrap_or("").parse().map_err(|_| anyhow!("Ungültige OBJ-Datei (Fläche)"))?;
                    let idx = if i < 0 { n + i } else { i - 1 };
                    if idx < 0 || idx >= n {
                        bail!("Ungültige OBJ-Datei (Flächenindex außerhalb)");
                    }
                    ids.push(idx as u32);
                }
                for k in 1..ids.len().saturating_sub(1) {
                    mesh.tri.push([ids[0], ids[k], ids[k + 1]]);
                }
            }
            _ => {}
        }
    }
    Ok(mesh)
}

#[derive(Clone, Copy)]
enum Scalar {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
}

impl Scalar {
    fn parse(s: &str) -> Result<Scalar> {
        Ok(match s {
            "char" | "int8" => Scalar::I8,
            "uchar" | "uint8" => Scalar::U8,
            "short" | "int16" => Scalar::I16,
            "ushort" | "uint16" => Scalar::U16,
            "int" | "int32" => Scalar::I32,
            "uint" | "uint32" => Scalar::U32,
            "float" | "float32" => Scalar::F32,
            "double" | "float64" => Scalar::F64,
            _ => bail!("Ungültige PLY-Datei (Datentyp {s})"),
        })
    }

    fn size(self) -> usize {
        match self {
            Scalar::I8 | Scalar::U8 => 1,
            Scalar::I16 | Scalar::U16 => 2,
            Scalar::I32 | Scalar::U32 | Scalar::F32 => 4,
            Scalar::F64 => 8,
        }
    }
}

struct PlyProp {
    name: String,
    ty: Scalar,
    list_count: Option<Scalar>,
}

struct PlyElement {
    name: String,
    count: usize,
    props: Vec<PlyProp>,
}

enum PlyData<'a> {
    Ascii(std::str::SplitAsciiWhitespace<'a>),
    Binary { data: &'a [u8], pos: usize, big: bool },
}

impl PlyData<'_> {
    fn read(&mut self, ty: Scalar) -> Result<f64> {
        let short = || anyhow!("Ungültige PLY-Datei (unerwartetes Ende)");
        match self {
            PlyData::Ascii(it) => it.next().ok_or_else(short)?.parse().map_err(|_| anyhow!("Ungültige PLY-Datei (Zahl)")),
            PlyData::Binary { data, pos, big } => {
                let n = ty.size();
                let slice = data.get(*pos..*pos + n).ok_or_else(short)?;
                *pos += n;
                let mut b = [0u8; 8];
                b[..n].copy_from_slice(slice);
                if *big {
                    b[..n].reverse();
                }
                Ok(match ty {
                    Scalar::I8 => b[0] as i8 as f64,
                    Scalar::U8 => b[0] as f64,
                    Scalar::I16 => i16::from_le_bytes([b[0], b[1]]) as f64,
                    Scalar::U16 => u16::from_le_bytes([b[0], b[1]]) as f64,
                    Scalar::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
                    Scalar::U32 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
                    Scalar::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
                    Scalar::F64 => f64::from_le_bytes(b),
                })
            }
        }
    }
}

fn read_ply(data: &[u8]) -> Result<Mesh> {
    let marker = b"end_header";
    let at = data.windows(marker.len()).position(|w| w == marker).ok_or_else(|| anyhow!("Ungültige PLY-Datei (Kopf fehlt)"))?;
    let mut body = at + marker.len();
    if data.get(body) == Some(&b'\r') {
        body += 1;
    }
    if data.get(body) == Some(&b'\n') {
        body += 1;
    }
    let header = String::from_utf8_lossy(&data[..at]);
    let mut format = "";
    let mut elements: Vec<PlyElement> = Vec::new();
    for line in header.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.as_slice() {
            ["format", f, ..] => format = f,
            ["element", name, count] => elements.push(PlyElement {
                name: name.to_string(),
                count: count.parse().map_err(|_| anyhow!("Ungültige PLY-Datei (Anzahl)"))?,
                props: Vec::new(),
            }),
            ["property", "list", c, t, name] => {
                let el = elements.last_mut().ok_or_else(|| anyhow!("Ungültige PLY-Datei"))?;
                el.props.push(PlyProp { name: name.to_string(), ty: Scalar::parse(t)?, list_count: Some(Scalar::parse(c)?) });
            }
            ["property", t, name] => {
                let el = elements.last_mut().ok_or_else(|| anyhow!("Ungültige PLY-Datei"))?;
                el.props.push(PlyProp { name: name.to_string(), ty: Scalar::parse(t)?, list_count: None });
            }
            _ => {}
        }
    }
    let mut src = match format {
        "ascii" => PlyData::Ascii(std::str::from_utf8(&data[body..]).map_err(|_| anyhow!("Ungültige PLY-Datei (Text)"))?.split_ascii_whitespace()),
        "binary_little_endian" => PlyData::Binary { data, pos: body, big: false },
        "binary_big_endian" => PlyData::Binary { data, pos: body, big: true },
        _ => bail!("Ungültige PLY-Datei (Format {format})"),
    };

    let mut mesh = Mesh::default();
    for el in &elements {
        let col = |n: &str| el.props.iter().position(|p| p.name == n);
        let (cx, cy, cz) = (col("x"), col("y"), col("z"));
        let is_vertex = el.name == "vertex" && cx.is_some() && cy.is_some() && cz.is_some();
        let is_face = el.name == "face";
        let mut face_list_used = false;
        for _ in 0..el.count {
            let mut xyz = [0f64; 3];
            for (pi, prop) in el.props.iter().enumerate() {
                match prop.list_count {
                    None => {
                        let v = src.read(prop.ty)?;
                        if is_vertex {
                            if Some(pi) == cx {
                                xyz[0] = v;
                            } else if Some(pi) == cy {
                                xyz[1] = v;
                            } else if Some(pi) == cz {
                                xyz[2] = v;
                            }
                        }
                    }
                    Some(ct) => {
                        let n = src.read(ct)? as usize;
                        let keep = is_face && !face_list_used && (prop.name == "vertex_indices" || prop.name == "vertex_index");
                        let mut ids = Vec::with_capacity(n);
                        for _ in 0..n {
                            ids.push(src.read(prop.ty)? as u32);
                        }
                        if keep {
                            face_list_used = true;
                            for k in 1..n.saturating_sub(1) {
                                mesh.tri.push([ids[0], ids[k], ids[k + 1]]);
                            }
                        }
                    }
                }
            }
            face_list_used = false;
            if is_vertex {
                mesh.pos.push([xyz[0] as f32, xyz[1] as f32, xyz[2] as f32]);
            }
        }
    }
    if mesh.tri.iter().any(|t| t.iter().any(|&i| i as usize >= mesh.pos.len())) {
        bail!("Ungültige PLY-Datei (Flächenindex außerhalb)");
    }
    Ok(mesh)
}

fn mat4_mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut o = [[0f32; 4]; 4];
    for c in 0..4 {
        for r in 0..4 {
            o[c][r] = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    o
}

fn gltf_node(node: gltf::Node, parent: &[[f32; 4]; 4], buffers: &[gltf::buffer::Data], mesh: &mut Mesh, depth: usize) {
    if depth > 64 {
        return;
    }
    let m = mat4_mul(parent, &node.transform().matrix());
    if let Some(gm) = node.mesh() {
        for prim in gm.primitives() {
            let reader = prim.reader(|b| Some(&buffers[b.index()]));
            let Some(positions) = reader.read_positions() else { continue };
            let base = mesh.pos.len() as u32;
            for p in positions {
                mesh.pos.push([
                    m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
                    m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
                    m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2],
                ]);
            }
            let count = mesh.pos.len() as u32 - base;
            let idx: Vec<u32> = match reader.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..count).collect(),
            };
            if idx.iter().any(|&i| i >= count) {
                continue;
            }
            use gltf::mesh::Mode;
            match prim.mode() {
                Mode::Triangles => mesh.tri.extend(idx.chunks_exact(3).map(|t| [base + t[0], base + t[1], base + t[2]])),
                Mode::TriangleStrip => {
                    for k in 2..idx.len() {
                        let (a, b) = if k % 2 == 0 { (idx[k - 2], idx[k - 1]) } else { (idx[k - 1], idx[k - 2]) };
                        mesh.tri.push([base + a, base + b, base + idx[k]]);
                    }
                }
                Mode::TriangleFan => {
                    for k in 2..idx.len() {
                        mesh.tri.push([base + idx[0], base + idx[k - 1], base + idx[k]]);
                    }
                }
                _ => {}
            }
        }
    }
    for child in node.children() {
        gltf_node(child, &m, buffers, mesh, depth + 1);
    }
}

fn read_gltf(path: &Path) -> Result<Mesh> {
    let g = gltf::Gltf::open(path).map_err(|e| anyhow!("Ungültige glTF-Datei: {e}"))?;
    let buffers = gltf::import_buffers(&g.document, path.parent(), g.blob.clone()).map_err(|e| anyhow!("glTF-Daten konnten nicht geladen werden: {e}"))?;
    let identity = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let mut mesh = Mesh::default();
    match g.document.default_scene().or_else(|| g.document.scenes().next()) {
        Some(scene) => {
            for node in scene.nodes() {
                gltf_node(node, &identity, &buffers, &mut mesh, 0);
            }
        }
        None => {
            for node in g.document.nodes() {
                gltf_node(node, &identity, &buffers, &mut mesh, 63);
            }
        }
    }
    Ok(mesh)
}

/// 3MF transforms are 12 numbers: three matrix rows plus a translation, applied to row vectors.
fn apply12(m: &[f32; 12], p: [f32; 3]) -> [f32; 3] {
    [
        p[0] * m[0] + p[1] * m[3] + p[2] * m[6] + m[9],
        p[0] * m[1] + p[1] * m[4] + p[2] * m[7] + m[10],
        p[0] * m[2] + p[1] * m[5] + p[2] * m[8] + m[11],
    ]
}

/// Applies `a` first, then `b`.
fn compose12(a: &[f32; 12], b: &[f32; 12]) -> [f32; 12] {
    let mut o = [0f32; 12];
    for row in 0..4 {
        let r = [a[row * 3], a[row * 3 + 1], a[row * 3 + 2]];
        for c in 0..3 {
            o[row * 3 + c] = r[0] * b[c] + r[1] * b[3 + c] + r[2] * b[6 + c] + if row == 3 { b[9 + c] } else { 0.0 };
        }
    }
    o
}

const IDENTITY12: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

fn parse12(s: Option<&String>) -> [f32; 12] {
    let v: Vec<f32> = s.map(|s| s.split_whitespace().filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
    v.try_into().unwrap_or(IDENTITY12)
}

fn attrs(e: &BytesStart) -> HashMap<String, String> {
    e.attributes()
        .flatten()
        .map(|a| (a.key.local_name().into_inner().to_string(), a.value.to_string()))
        .collect()
}

#[derive(Default)]
struct Object3mf {
    mesh: Mesh,
    components: Vec<(u32, [f32; 12])>,
}

fn read_3mf(path: &Path) -> Result<Mesh> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path)?).map_err(|_| anyhow!("Ungültige 3MF-Datei (kein ZIP-Archiv)"))?;
    let name = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .find(|n| n.eq_ignore_ascii_case("3D/3dmodel.model"))
        .or_else(|| (0..zip.len()).filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string())).find(|n| n.ends_with(".model")))
        .ok_or_else(|| anyhow!("Ungültige 3MF-Datei (Modell fehlt)"))?;
    let mut xml = String::new();
    zip.by_name(&name)?.read_to_string(&mut xml)?;

    let mut reader = Reader::from_str(&xml);
    let mut objects: HashMap<u32, Object3mf> = HashMap::new();
    let mut order = Vec::new();
    let mut items: Vec<(u32, [f32; 12])> = Vec::new();
    let mut current: Option<u32> = None;
    loop {
        let ev = reader.read_event().map_err(|e| anyhow!("Ungültige 3MF-Datei (XML): {e}"))?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let a = attrs(e);
                let num = |k: &str| a.get(k).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
                match e.local_name().into_inner() {
                    "object" => {
                        let id = a.get("id").and_then(|v| v.parse().ok()).unwrap_or(0);
                        current = Some(id);
                        order.push(id);
                        objects.entry(id).or_default();
                    }
                    "vertex" => {
                        if let Some(o) = current.and_then(|id| objects.get_mut(&id)) {
                            o.mesh.pos.push([num("x"), num("y"), num("z")]);
                        }
                    }
                    "triangle" => {
                        if let Some(o) = current.and_then(|id| objects.get_mut(&id)) {
                            let id = |k: &str| a.get(k).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
                            o.mesh.tri.push([id("v1"), id("v2"), id("v3")]);
                        }
                    }
                    "component" => {
                        if let Some(o) = current.and_then(|id| objects.get_mut(&id)) {
                            let oid = a.get("objectid").and_then(|v| v.parse().ok()).unwrap_or(0);
                            o.components.push((oid, parse12(a.get("transform"))));
                        }
                    }
                    "item" => {
                        let oid = a.get("objectid").and_then(|v| v.parse().ok()).unwrap_or(0);
                        items.push((oid, parse12(a.get("transform"))));
                    }
                    _ => {}
                }
            }
            Event::End(e) if e.local_name().as_ref() == "object" => current = None,
            Event::Eof => break,
            _ => {}
        }
    }
    if items.is_empty() {
        items = order.iter().map(|&id| (id, IDENTITY12)).collect();
    }

    fn collect(id: u32, m: &[f32; 12], objects: &HashMap<u32, Object3mf>, out: &mut Mesh, depth: usize) {
        let Some(o) = objects.get(&id) else { return };
        if depth > 16 {
            return;
        }
        let valid = o.mesh.tri.iter().all(|t| t.iter().all(|&i| (i as usize) < o.mesh.pos.len()));
        if valid && !o.mesh.tri.is_empty() {
            out.append(Mesh { pos: o.mesh.pos.clone(), tri: o.mesh.tri.clone() }, Some(m));
        }
        for (cid, cm) in &o.components {
            collect(*cid, &compose12(cm, m), objects, out, depth + 1);
        }
    }
    let mut mesh = Mesh::default();
    for (id, m) in &items {
        collect(*id, m, &objects, &mut mesh, 0);
    }
    Ok(mesh)
}

#[derive(Default)]
struct DaeSource {
    values: Vec<f32>,
    stride: usize,
}

struct DaePrim {
    kind: String,
    inputs: Vec<(String, usize, String)>,
    vcount: Vec<usize>,
    p: Vec<Vec<usize>>,
}

fn dae_finish(prim: &DaePrim, sources: &HashMap<String, DaeSource>, vertices: &HashMap<String, String>, mesh: &mut Mesh, welded: &mut HashMap<(String, usize), u32>) {
    let stride = prim.inputs.iter().map(|i| i.1 + 1).max().unwrap_or(1);
    let Some((_, voff, vsrc)) = prim.inputs.iter().find(|i| i.0 == "VERTEX") else { return };
    let Some(src_id) = vertices.get(vsrc) else { return };
    let Some(src) = sources.get(src_id) else { return };
    let comps = src.stride.max(3);
    let mut vertex_of = |idx: usize, mesh: &mut Mesh| -> Option<u32> {
        if let Some(&v) = welded.get(&(src_id.clone(), idx)) {
            return Some(v);
        }
        let b = idx * comps;
        let p = src.values.get(b..b + 3)?;
        mesh.pos.push([p[0], p[1], p[2]]);
        let v = (mesh.pos.len() - 1) as u32;
        welded.insert((src_id.clone(), idx), v);
        Some(v)
    };
    let mut polygon = |indices: &[usize], mesh: &mut Mesh| {
        let ids: Vec<u32> = indices.chunks_exact(stride).filter_map(|c| vertex_of(c[*voff], mesh)).collect();
        for k in 1..ids.len().saturating_sub(1) {
            mesh.tri.push([ids[0], ids[k], ids[k + 1]]);
        }
    };
    match prim.kind.as_str() {
        "triangles" => {
            for p in &prim.p {
                for tri in p.chunks_exact(stride * 3) {
                    polygon(tri, mesh);
                }
            }
        }
        "polylist" => {
            let mut at = 0;
            for p in &prim.p {
                for &n in &prim.vcount {
                    if let Some(poly) = p.get(at..at + n * stride) {
                        polygon(poly, mesh);
                    }
                    at += n * stride;
                }
            }
        }
        "polygons" => {
            for p in &prim.p {
                polygon(p, mesh);
            }
        }
        _ => {}
    }
}

fn read_dae(data: &[u8]) -> Result<Mesh> {
    let xml = String::from_utf8_lossy(data);
    let mut reader = Reader::from_str(&xml);
    let mut sources: HashMap<String, DaeSource> = HashMap::new();
    let mut vertices: HashMap<String, String> = HashMap::new();
    let mut mesh = Mesh::default();
    let mut welded = HashMap::new();
    let mut cur_source = String::new();
    let mut cur_vertices = String::new();
    let mut prim: Option<DaePrim> = None;
    let mut text_for: Option<&'static str> = None;
    let mut text = String::new();
    loop {
        let ev = reader.read_event().map_err(|e| anyhow!("Ungültige DAE-Datei (XML): {e}"))?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let a = attrs(e);
                let name = e.local_name().into_inner().to_string();
                match name.as_str() {
                    "source" => cur_source = a.get("id").cloned().unwrap_or_default(),
                    "float_array" => {
                        sources.entry(cur_source.clone()).or_default().stride = 3;
                        text_for = Some("float_array");
                        text.clear();
                    }
                    "accessor" => {
                        sources.entry(cur_source.clone()).or_default().stride = a.get("stride").and_then(|v| v.parse().ok()).unwrap_or(1);
                    }
                    "vertices" => cur_vertices = a.get("id").cloned().unwrap_or_default(),
                    "input" => {
                        let sem = a.get("semantic").cloned().unwrap_or_default();
                        let src = a.get("source").map(|s| s.trim_start_matches('#').to_string()).unwrap_or_default();
                        if let Some(p) = prim.as_mut() {
                            p.inputs.push((sem, a.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0), src));
                        } else if sem == "POSITION" && !cur_vertices.is_empty() {
                            vertices.insert(cur_vertices.clone(), src);
                        }
                    }
                    "triangles" | "polylist" | "polygons" => prim = Some(DaePrim { kind: name, inputs: Vec::new(), vcount: Vec::new(), p: Vec::new() }),
                    "vcount" => {
                        text_for = Some("vcount");
                        text.clear();
                    }
                    "p" => {
                        text_for = Some("p");
                        text.clear();
                    }
                    _ => {}
                }
            }
            Event::Text(t) => {
                if text_for.is_some() {
                    text.push_str(t.as_ref());
                }
            }
            Event::End(e) => {
                let name = e.local_name().into_inner().to_string();
                match (name.as_str(), text_for) {
                    ("float_array", Some(_)) => {
                        let values = text.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                        sources.entry(cur_source.clone()).or_default().values = values;
                        text_for = None;
                    }
                    ("vcount", Some(_)) => {
                        if let Some(p) = prim.as_mut() {
                            p.vcount = text.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                        }
                        text_for = None;
                    }
                    ("p", Some(_)) => {
                        if let Some(p) = prim.as_mut() {
                            p.p.push(text.split_whitespace().filter_map(|v| v.parse().ok()).collect());
                        }
                        text_for = None;
                    }
                    ("triangles" | "polylist" | "polygons", _) => {
                        if let Some(p) = prim.take() {
                            dae_finish(&p, &sources, &vertices, &mut mesh, &mut welded);
                        }
                    }
                    ("vertices", _) => cur_vertices.clear(),
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(mesh)
}

// ---------- writing ----------

fn face_normal(t: &[[f32; 3]; 3]) -> [f32; 3] {
    let u = [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]];
    let v = [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]];
    let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > 0.0 {
        [n[0] / len, n[1] / len, n[2] / len]
    } else {
        [0.0; 3]
    }
}

fn write_stl(mesh: &Mesh) -> Vec<u8> {
    let mut out = Vec::with_capacity(84 + mesh.tri.len() * 50);
    let mut header = [0u8; 80];
    header[..16].copy_from_slice(b"Orbo binary STL ");
    out.extend_from_slice(&header);
    out.extend_from_slice(&(mesh.tri.len() as u32).to_le_bytes());
    for t in &mesh.tri {
        let tri = mesh.triangle(t);
        for c in face_normal(&tri).into_iter().chain(tri.into_iter().flatten()) {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    out
}

fn write_obj(mesh: &Mesh) -> String {
    let mut s = String::from("# Erstellt mit Orbo\n");
    for p in &mesh.pos {
        s.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
    }
    for t in &mesh.tri {
        s.push_str(&format!("f {} {} {}\n", t[0] + 1, t[1] + 1, t[2] + 1));
    }
    s
}

fn write_ply(mesh: &Mesh) -> Vec<u8> {
    let mut out = format!(
        "ply\nformat binary_little_endian 1.0\ncomment Erstellt mit Orbo\nelement vertex {}\nproperty float x\nproperty float y\nproperty float z\nelement face {}\nproperty list uchar uint vertex_indices\nend_header\n",
        mesh.pos.len(),
        mesh.tri.len()
    )
    .into_bytes();
    for p in &mesh.pos {
        for c in p {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    for t in &mesh.tri {
        out.push(3);
        for i in t {
            out.extend_from_slice(&i.to_le_bytes());
        }
    }
    out
}

fn write_glb(mesh: &Mesh) -> Vec<u8> {
    let pos_len = mesh.pos.len() * 12;
    let idx_len = mesh.tri.len() * 12;
    let (min, max) = mesh.bounds();
    let json = serde_json::json!({
        "asset": { "version": "2.0", "generator": "Orbo" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 }, "indices": 1, "mode": 4, "material": 0 }] }],
        "materials": [{ "pbrMetallicRoughness": { "baseColorFactor": [0.8, 0.8, 0.8, 1.0], "metallicFactor": 0.1, "roughnessFactor": 0.8 } }],
        "buffers": [{ "byteLength": pos_len + idx_len }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": pos_len, "target": 34962 },
            { "buffer": 0, "byteOffset": pos_len, "byteLength": idx_len, "target": 34963 }
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": mesh.pos.len(), "type": "VEC3", "min": min, "max": max },
            { "bufferView": 1, "componentType": 5125, "count": mesh.tri.len() * 3, "type": "SCALAR" }
        ]
    });
    let mut json = serde_json::to_vec(&json).unwrap_or_default();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let mut bin = Vec::with_capacity(pos_len + idx_len);
    for p in &mesh.pos {
        for c in p {
            bin.extend_from_slice(&c.to_le_bytes());
        }
    }
    for t in &mesh.tri {
        for i in t {
            bin.extend_from_slice(&i.to_le_bytes());
        }
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

fn write_3mf(mesh: &Mesh, out: &Path) -> Result<()> {
    let mut model = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n<resources>\n<object id=\"1\" type=\"model\">\n<mesh>\n<vertices>\n",
    );
    for p in &mesh.pos {
        model.push_str(&format!("<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>\n", p[0], p[1], p[2]));
    }
    model.push_str("</vertices>\n<triangles>\n");
    for t in &mesh.tri {
        model.push_str(&format!("<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>\n", t[0], t[1], t[2]));
    }
    model.push_str("</triangles>\n</mesh>\n</object>\n</resources>\n<build>\n<item objectid=\"1\"/>\n</build>\n</model>\n");

    let mut zip = zip::ZipWriter::new(fs::File::create(out)?);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let files: [(&str, &str); 3] = [
        (
            "[Content_Types].xml",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/></Types>",
        ),
        (
            "_rels/.rels",
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/></Relationships>",
        ),
        ("3D/3dmodel.model", &model),
    ];
    for (name, content) in files {
        zip.start_file(name, opts)?;
        zip.write_all(content.as_bytes())?;
    }
    zip.finish()?;
    Ok(())
}

// ---------- preview ----------

/// Orthographic isometric-style view with a z-buffer and flat shading, rendered at 2x and averaged down.
fn render_preview(mesh: &Mesh, z_up: bool) -> image::RgbImage {
    const OUT: u32 = 1024;
    const SS: usize = 2;
    let size = OUT as usize * SS;
    let (yaw, elev) = (35f32.to_radians(), 30f32.to_radians());
    let (cy, sy_, ce, se) = (yaw.cos(), yaw.sin(), elev.cos(), elev.sin());

    // Rotated frame: x right, y into the screen (before elevation), z up.
    let rot = |p: [f32; 3]| -> [f32; 3] {
        let (x, y, z) = if z_up { (p[0], p[1], p[2]) } else { (p[0], -p[2], p[1]) };
        [x * cy - y * sy_, x * sy_ + y * cy, z]
    };
    let rotated: Vec<[f32; 3]> = mesh.pos.iter().map(|&p| rot(p)).collect();
    let screen = |r: [f32; 3]| (r[0], r[1] * se + r[2] * ce, r[1] * ce - r[2] * se);

    let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for r in &rotated {
        let (x, y, _) = screen(*r);
        x0 = x0.min(x);
        x1 = x1.max(x);
        y0 = y0.min(y);
        y1 = y1.max(y);
    }
    let extent = (x1 - x0).max(y1 - y0).max(f32::EPSILON);
    let scale = size as f32 * 0.88 / extent;
    let (ox, oy) = (size as f32 / 2.0 - (x0 + x1) / 2.0 * scale, size as f32 / 2.0 + (y0 + y1) / 2.0 * scale);

    let mut color = vec![[255u8; 3]; size * size];
    let mut zbuf = vec![f32::INFINITY; size * size];
    let view = [0.0, ce, -se];
    let light = {
        let l = [-0.5f32, -0.7, 0.9];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        [l[0] / n, l[1] / n, l[2] / n]
    };
    let base = [150.0f32, 168.0, 196.0];

    for t in &mesh.tri {
        let r = [rotated[t[0] as usize], rotated[t[1] as usize], rotated[t[2] as usize]];
        let mut n = face_normal(&r);
        if n == [0.0; 3] {
            continue;
        }
        if n[0] * view[0] + n[1] * view[1] + n[2] * view[2] > 0.0 {
            n = [-n[0], -n[1], -n[2]];
        }
        let lambert = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.0);
        let shade = 0.28 + 0.72 * lambert;
        let rgb = [(base[0] * shade) as u8, (base[1] * shade) as u8, (base[2] * shade) as u8];

        let pts: Vec<(f32, f32, f32)> = r
            .iter()
            .map(|&p| {
                let (x, y, d) = screen(p);
                (x * scale + ox, oy - y * scale, d)
            })
            .collect();
        let area = (pts[1].0 - pts[0].0) * (pts[2].1 - pts[0].1) - (pts[2].0 - pts[0].0) * (pts[1].1 - pts[0].1);
        if area.abs() < 1e-6 {
            continue;
        }
        let min_x = pts.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
        let max_x = (pts.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(size - 1);
        let min_y = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
        let max_y = (pts.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(size - 1);
        for py in min_y..=max_y {
            for px in min_x..=max_x {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let w0 = ((pts[1].0 - fx) * (pts[2].1 - fy) - (pts[2].0 - fx) * (pts[1].1 - fy)) / area;
                let w1 = ((pts[2].0 - fx) * (pts[0].1 - fy) - (pts[0].0 - fx) * (pts[2].1 - fy)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                    continue;
                }
                let depth = w0 * pts[0].2 + w1 * pts[1].2 + w2 * pts[2].2;
                let i = py * size + px;
                if depth < zbuf[i] {
                    zbuf[i] = depth;
                    color[i] = rgb;
                }
            }
        }
    }

    let mut img = image::RgbImage::new(OUT, OUT);
    for y in 0..OUT as usize {
        for x in 0..OUT as usize {
            let mut sum = [0u32; 3];
            for dy in 0..SS {
                for dx in 0..SS {
                    let c = color[(y * SS + dy) * size + x * SS + dx];
                    for k in 0..3 {
                        sum[k] += c[k] as u32;
                    }
                }
            }
            let n = (SS * SS) as u32;
            img.put_pixel(x as u32, y as u32, image::Rgb([(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]));
        }
    }
    img
}
