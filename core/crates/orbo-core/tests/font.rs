use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use orbo_core::jobs::{convert_blocking, JobSpec};

fn run(input: &Path, target: &str) -> anyhow::Result<PathBuf> {
    let spec = JobSpec { input: input.to_path_buf(), target: target.into(), options: Default::default() };
    convert_blocking(&spec, 1, &AtomicBool::new(false), &|_| {})
}

fn system_font() -> Option<PathBuf> {
    [
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.exists())
}

fn glyphs(path: &Path) -> u16 {
    ttf_parser::Face::parse(&fs::read(path).unwrap(), 0).unwrap().number_of_glyphs()
}

fn fixture(dir: &Path) -> Option<PathBuf> {
    let src = system_font()?;
    let copy = dir.join("probe.ttf");
    fs::copy(src, &copy).unwrap();
    Some(copy)
}

#[test]
fn ttf_woff_ttf_keeps_glyph_count() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ttf) = fixture(dir.path()) else { return };
    let woff = run(&ttf, "font_woff").unwrap();
    assert_eq!(&fs::read(&woff).unwrap()[..4], b"wOFF");
    fs::remove_file(&ttf).unwrap();
    let back = run(&woff, "font_ttf").unwrap();
    assert_eq!(glyphs(&back), glyphs(&system_font().unwrap()));
}

#[test]
fn ttf_woff2_ttf_keeps_glyph_count() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ttf) = fixture(dir.path()) else { return };
    let woff2 = run(&ttf, "font_woff2").unwrap();
    assert_eq!(&fs::read(&woff2).unwrap()[..4], b"wOF2");
    assert!(fs::metadata(&woff2).unwrap().len() < fs::metadata(&ttf).unwrap().len());
    fs::remove_file(&ttf).unwrap();
    let back = run(&woff2, "font_ttf").unwrap();
    assert_eq!(glyphs(&back), glyphs(&system_font().unwrap()));
}

#[test]
fn woff_to_woff2_and_wrong_flavor_message() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ttf) = fixture(dir.path()) else { return };
    let woff = run(&ttf, "font_woff").unwrap();
    let woff2 = run(&woff, "font_woff2").unwrap();
    assert_eq!(&fs::read(&woff2).unwrap()[..4], b"wOF2");
    let err = run(&woff, "font_otf").unwrap_err().to_string();
    assert!(err.contains("TTF"), "{err}");
}

#[test]
fn specimen_png_has_text() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ttf) = fixture(dir.path()) else { return };
    let png = run(&ttf, "font_specimen").unwrap();
    let img = image::open(&png).unwrap().to_rgb8();
    assert_eq!(img.width(), 1600);
    assert!(img.height() > 400);
    assert!(img.pixels().filter(|p| p.0[0] < 128).count() > 5_000, "glyphs are drawn");
}

#[test]
fn garbage_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("kaputt.woff");
    fs::write(&bad, b"das ist keine Schrift").unwrap();
    assert!(run(&bad, "font_ttf").is_err());
}
