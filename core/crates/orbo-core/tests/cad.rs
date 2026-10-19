use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use orbo_core::jobs::{convert_blocking, JobSpec, Options};

fn run(input: &Path, target: &str) -> PathBuf {
    let spec = JobSpec { input: input.to_path_buf(), target: target.into(), options: Options::default() };
    convert_blocking(&spec, 1, &AtomicBool::new(false), &|_| {}).unwrap_or_else(|e| panic!("{target}: {e:#}"))
}

fn dxf(dir: &Path, name: &str, header: &str, blocks: &str, entities: &str) -> PathBuf {
    let text = format!(
        "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n{header}0\nENDSEC\n0\nSECTION\n2\nBLOCKS\n{blocks}0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    );
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p
}

fn lwpoly(layer: &str, closed: bool, v: &[(f64, f64, f64)]) -> String {
    let mut s = format!("0\nLWPOLYLINE\n8\n{layer}\n90\n{}\n70\n{}\n", v.len(), closed as i32);
    for (x, y, b) in v {
        s += &format!("10\n{x}\n20\n{y}\n42\n{b}\n");
    }
    s
}

fn view_box(svg: &str) -> (f64, f64) {
    let i = svg.find("viewBox=\"").expect("viewBox") + 9;
    let v: Vec<f64> = svg[i..].split('"').next().unwrap().split_whitespace().map(|n| n.parse().unwrap()).collect();
    assert_eq!((v[0], v[1]), (0.0, 0.0));
    (v[2], v[3])
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 0.1, "{a} != {b}");
}

fn svg_of(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

fn rect_with_bulge(dir: &Path) -> PathBuf {
    // The bottom edge bulges 25 mm downwards (sagitta = bulge * chord / 2).
    dxf(dir, "rect.dxf", "", "", &lwpoly("Kontur", true, &[(0.0, 0.0, 0.5), (100.0, 0.0, 0.0), (100.0, 50.0, 0.0), (0.0, 50.0, 0.0)]))
}

#[test]
fn svg_true_scale_with_bulge() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(&rect_with_bulge(dir.path()), "cad_svg");
    let svg = svg_of(&out);
    let (w, h) = view_box(&svg);
    near(w, 100.0);
    near(h, 75.0);
    assert!(svg.contains("width=\"100mm\""));
    assert_eq!(svg.matches("<path").count(), 1);
    assert!(svg.contains("<g id=\"Kontur\">"));
}

#[test]
fn circle_and_units() {
    let dir = tempfile::tempdir().unwrap();
    let circle = dxf(dir.path(), "c.dxf", "", "", "0\nCIRCLE\n8\n0\n10\n10\n20\n10\n40\n5\n");
    let (w, h) = view_box(&svg_of(&run(&circle, "cad_svg")));
    near(w, 10.0);
    near(h, 10.0);

    let inch = dxf(dir.path(), "i.dxf", "9\n$INSUNITS\n70\n1\n", "", "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n2\n21\n1\n");
    let (w, h) = view_box(&svg_of(&run(&inch, "cad_svg")));
    near(w, 50.8);
    near(h, 25.4);
}

fn insert_fixture(dir: &Path) -> PathBuf {
    let blocks = "0\nBLOCK\n8\n0\n2\nB\n70\n0\n10\n0\n20\n0\n0\nLINE\n8\n0\n10\n0\n20\n0\n11\n10\n21\n0\n0\nLINE\n8\n0\n10\n10\n20\n0\n11\n10\n21\n5\n0\nENDBLK\n";
    dxf(dir, "ins.dxf", "", blocks, "0\nINSERT\n8\nTeile\n2\nB\n10\n100\n20\n100\n41\n2\n42\n2\n50\n90\n")
}

#[test]
fn insert_scaled_and_rotated() {
    let dir = tempfile::tempdir().unwrap();
    let svg = svg_of(&run(&insert_fixture(dir.path()), "cad_svg"));
    let (w, h) = view_box(&svg);
    near(w, 10.0);
    near(h, 20.0);
    assert_eq!(svg.matches("<path").count(), 2);
    // Block entities on layer 0 take the layer of the INSERT.
    assert!(svg.contains("<g id=\"Teile\">"));
}

#[test]
fn pdf_page_matches_drawing() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(&rect_with_bulge(dir.path()), "cad_pdf");
    let doc = lopdf::Document::load(&out).unwrap();
    let pages = doc.get_pages();
    assert_eq!(pages.len(), 1);
    let page = doc.get_dictionary(*pages.values().next().unwrap()).unwrap();
    let mb: Vec<f64> = page.get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|o| o.as_float().unwrap() as f64).collect();
    let pt = 72.0 / 25.4;
    near(mb[2], 120.0 * pt);
    near(mb[3], 95.0 * pt);
}

#[test]
fn png_has_white_background_and_long_side() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(&rect_with_bulge(dir.path()), "cad_png");
    let img = image::open(&out).unwrap().to_rgba8();
    assert_eq!(img.width().max(img.height()), 2048);
    assert_eq!(img.get_pixel(1, 1).0, [255, 255, 255, 255]);
    assert!(img.pixels().any(|p| p.0[0] < 64));
}

#[test]
fn clean_r12_roundtrip_and_svg_to_dxf() {
    let dir = tempfile::tempdir().unwrap();
    let clean = run(&rect_with_bulge(dir.path()), "cad_dxf_clean");
    assert!(std::fs::read_to_string(&clean).unwrap().contains("AC1009"));
    let again = dir.path().join("clean.dxf");
    std::fs::copy(&clean, &again).unwrap();
    let (w, h) = view_box(&svg_of(&run(&again, "cad_svg")));
    near(w, 100.0);
    near(h, 75.0);

    let svg = dir.path().join("shape.svg");
    std::fs::write(&svg, "<svg xmlns='http://www.w3.org/2000/svg' width='96' height='48' viewBox='0 0 96 48'><path d='M0 0 H96 V48 H0 Z' fill='none' stroke='black'/></svg>").unwrap();
    let out = run(&svg, "svg_to_dxf");
    let back = dir.path().join("back.dxf");
    std::fs::copy(&out, &back).unwrap();
    let (w, h) = view_box(&svg_of(&run(&back, "cad_svg")));
    near(w, 25.4);
    near(h, 12.7);
}

#[test]
fn dwg_needs_converter() {
    let Some(sample) = std::env::var_os("ORBO_DWG_SAMPLE") else { return };
    let sample = Path::new(&sample);
    if orbo_core::engines::cad::dwg2dxf().is_none() || !sample.is_file() {
        eprintln!("dwg2dxf or sample missing, skipped");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let dwg = dir.path().join("sample.dwg");
    std::fs::copy(sample, &dwg).unwrap();
    let svg = svg_of(&run(&dwg, "dwg_svg"));
    let (w, h) = view_box(&svg);
    assert!(w > 0.0 && h > 0.0);
    assert!(svg.matches("<path").count() >= 1);
    assert!(std::fs::metadata(run(&dwg, "dwg_dxf")).unwrap().len() > 0);
}
