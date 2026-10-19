use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use orbo_core::catalog::Catalog;
use orbo_core::engines::office::soffice;
use orbo_core::jobs::{convert_blocking, JobSpec};

fn run(input: &Path, target: &str) -> anyhow::Result<PathBuf> {
    let spec = JobSpec { input: input.to_path_buf(), target: target.into(), options: Default::default() };
    convert_blocking(&spec, 1, &AtomicBool::new(false), &|_| {})
}

fn make_zip(path: &Path, files: &[(&str, String)]) {
    let mut z = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, data) in files {
        z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap();
}

fn slide(title: &str, body: &[&str]) -> String {
    let paras: String = body.iter().map(|b| format!("<a:p><a:r><a:t>{b}</a:t></a:r></a:p>")).collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree>
<p:sp><p:nvSpPr><p:cNvPr id="2" name="Titel"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>{title}</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="3" name="Inhalt"/><p:cNvSpPr/><p:nvPr><p:ph idx="1"/></p:nvPr></p:nvSpPr><p:txBody>{paras}</p:txBody></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="4" name="Nummer"/><p:cNvSpPr/><p:nvPr><p:ph type="sldNum"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>7</a:t></a:r></a:p></p:txBody></p:sp>
</p:spTree></p:cSld></p:sld>"#
    )
}

fn sample_pptx(dir: &Path) -> PathBuf {
    let p = dir.join("vortrag.pptx");
    make_zip(
        &p,
        &[
            ("ppt/slides/slide10.xml", slide("Fazit", &["Alles gut"])),
            ("ppt/slides/slide1.xml", slide("Grüße aus Köln", &["Äpfel &amp; Öl", "Übermut: Maßnahme"])),
            ("ppt/slides/slide2.xml", slide("Zahlen", &["Eins", "Zwei"])),
        ],
    );
    p
}

#[test]
fn pptx_text_in_slide_order() {
    let dir = tempfile::tempdir().unwrap();
    let pptx = sample_pptx(dir.path());
    let txt = fs::read_to_string(run(&pptx, "pptx_txt").unwrap()).unwrap();
    assert!(txt.starts_with("Folie 1: Grüße aus Köln\n\n- Äpfel & Öl\n- Übermut: Maßnahme\n"), "{txt}");
    assert!(txt.find("Folie 2: Zahlen").unwrap() < txt.find("Folie 3: Fazit").unwrap());
    assert!(!txt.contains("\n7\n"), "Foliennummer gehört nicht in den Text");
}

#[test]
fn pptx_markdown_has_headings_and_bullets() {
    let dir = tempfile::tempdir().unwrap();
    let md = fs::read_to_string(run(&sample_pptx(dir.path()), "pptx_md").unwrap()).unwrap();
    assert!(md.contains("## Grüße aus Köln\n\n- Äpfel & Öl\n"), "{md}");
    assert!(md.contains("## Fazit"));
}

#[test]
fn pptx_without_slides_fails_clearly() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("leer.pptx");
    make_zip(&p, &[("docProps/app.xml", "<x/>".into())]);
    let err = run(&p, "pptx_txt").unwrap_err().to_string();
    assert!(err.contains("keine Folien"), "{err}");
}

#[test]
fn libreoffice_targets_follow_detection() {
    let dir = tempfile::tempdir().unwrap();
    let pptx = sample_pptx(dir.path());
    let a = orbo_core::analyze::analyze(&pptx).unwrap();
    let ids: Vec<&str> = Catalog::builtin().compatible(&a).iter().map(|t| t.id.as_str()).collect();
    assert!(ids.contains(&"pptx_txt") && ids.contains(&"pptx_md"));
    assert_eq!(ids.contains(&"slides_pdf"), soffice().is_some());
    assert_eq!(ids.contains(&"odp_lo"), soffice().is_some());
}

#[test]
fn libreoffice_converts_pptx_to_pdf() {
    if soffice().is_none() {
        eprintln!("LibreOffice nicht installiert, Test übersprungen");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let pdf = run(&sample_pptx(dir.path()), "slides_pdf").unwrap();
    assert!(fs::read(&pdf).unwrap().starts_with(b"%PDF"));
}
