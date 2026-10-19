use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use lopdf::{dictionary, Document, Object, Stream};
use orbo_core::jobs::{convert_blocking, JobSpec};

fn run(input: &Path, target: &str) -> anyhow::Result<PathBuf> {
    let spec = JobSpec { input: input.to_path_buf(), target: target.into(), options: Default::default() };
    convert_blocking(&spec, 1, &AtomicBool::new(false), &|_| {})
}

const TXT: &str = "Grüße aus Köln\nÄpfel, Öl und Übermut: Maßnahme\nZweite Zeile\n";

fn make_pdf(path: &Path, pages: u32) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    let res = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
    let mut kids = Vec::new();
    for n in 1..=pages {
        let text = format!("BT /F1 24 Tf 72 700 Td (Seite Nummer {n}) Tj ET");
        let content = doc.add_object(Stream::new(dictionary! {}, text.into_bytes()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        });
        kids.push(Object::from(page));
    }
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages as i64, "Resources" => res }));
    let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", cat);
    let info = doc.add_object(dictionary! { "Author" => Object::string_literal("Geheim"), "Title" => Object::string_literal("Vertraulich") });
    doc.trailer.set("Info", info);
    doc.save(path).unwrap();
}

fn page_count(path: &Path) -> usize {
    Document::load(path).unwrap().get_pages().len()
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn sample_pdf(dir: &Path) -> PathBuf {
    let p = dir.join("mappe.pdf");
    make_pdf(&p, 3);
    p
}

fn zip_has(path: &Path, name: &str) -> bool {
    let mut z = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut s = String::new();
    z.by_name(name).map(|mut f| f.read_to_string(&mut s)).is_ok()
}

#[test]
fn pdf_to_txt_has_page_text() {
    let d = tempfile::tempdir().unwrap();
    let out = run(&sample_pdf(d.path()), "pdf_txt").unwrap();
    assert_eq!(out.extension().unwrap(), "txt");
    let t = fs::read_to_string(out).unwrap();
    assert!(t.contains("Seite Nummer 1") && t.contains("Seite Nummer 3"), "{t}");
}

#[test]
fn pdf_pages_become_images() {
    let d = tempfile::tempdir().unwrap();
    let src = sample_pdf(d.path());
    let out = run(&src, "pdf_jpg").unwrap();
    assert_eq!(files_in(&out), ["Seite_001.jpg", "Seite_002.jpg", "Seite_003.jpg"]);
    let img = image::open(out.join("Seite_001.jpg")).unwrap();
    assert_eq!((img.width(), img.height()), (1190, 1684));
    let dark = img.to_luma8().pixels().filter(|p| p.0[0] < 100).count();
    assert!(dark > 200, "page should show rendered text, dark pixels: {dark}");

    let out = run(&src, "pdf_png").unwrap();
    assert_eq!(files_in(&out), ["Seite_001.png", "Seite_002.png", "Seite_003.png"]);
}

#[test]
fn pdf_split_makes_single_page_files() {
    let d = tempfile::tempdir().unwrap();
    let out = run(&sample_pdf(d.path()), "pdf_split").unwrap();
    assert_eq!(files_in(&out), ["Seite_001.pdf", "Seite_002.pdf", "Seite_003.pdf"]);
    for f in files_in(&out) {
        assert_eq!(page_count(&out.join(&f)), 1);
    }
    let t = pdf_extract::extract_text(out.join("Seite_002.pdf")).unwrap();
    assert!(t.contains("Seite Nummer 2") && !t.contains("Nummer 1"), "{t}");
}

#[test]
fn pdf_rotate_turns_every_page() {
    let d = tempfile::tempdir().unwrap();
    let out = run(&sample_pdf(d.path()), "pdf_rotate").unwrap();
    let doc = Document::load(&out).unwrap();
    assert_eq!(doc.get_pages().len(), 3);
    for (_, id) in doc.get_pages() {
        assert_eq!(doc.get_dictionary(id).unwrap().get(b"Rotate").unwrap().as_i64().unwrap(), 90);
    }
    let again = run(&out, "pdf_rotate").unwrap();
    let doc = Document::load(&again).unwrap();
    let id = *doc.get_pages().get(&1).unwrap();
    assert_eq!(doc.get_dictionary(id).unwrap().get(b"Rotate").unwrap().as_i64().unwrap(), 180);
}

#[test]
fn pdf_strip_removes_metadata() {
    let d = tempfile::tempdir().unwrap();
    let src = sample_pdf(d.path());
    assert!(fs::read(&src).unwrap().windows(6).any(|w| w == b"Geheim"));
    let out = run(&src, "pdf_strip").unwrap();
    assert!(!fs::read(&out).unwrap().windows(6).any(|w| w == b"Geheim"));
    let doc = Document::load(&out).unwrap();
    assert!(doc.trailer.get(b"Info").is_err());
    assert_eq!(doc.get_pages().len(), 3);
}

#[test]
fn pdf_compress_never_grows() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("gross.pdf");
    make_pdf(&src, 30);
    let out = run(&src, "pdf_compress").unwrap();
    assert!(fs::metadata(&out).unwrap().len() <= fs::metadata(&src).unwrap().len());
    assert_eq!(page_count(&out), 30);
}

#[test]
fn markdown_and_text_to_pdf() {
    let d = tempfile::tempdir().unwrap();
    let txt = d.path().join("brief.txt");
    fs::write(&txt, TXT).unwrap();
    let out = run(&txt, "doc_pdf").unwrap();
    assert_eq!(out.extension().unwrap(), "pdf");
    let t = pdf_extract::extract_text(&out).unwrap();
    assert!(t.contains("Grüße aus Köln") && t.contains("Maßnahme"), "{t}");

    let md = d.path().join("notiz.md");
    fs::write(&md, "# Überschrift\n\nEin **fetter** Absatz mit Ölfleck.\n\n- eins\n- zwei\n").unwrap();
    let out = run(&md, "doc_pdf").unwrap();
    let t = pdf_extract::extract_text(&out).unwrap();
    assert!(t.contains("Überschrift") && t.contains("fetter") && t.contains("zwei"), "{t}");
    assert!(!t.contains("**"), "{t}");
}

#[test]
fn markdown_to_html_and_text() {
    let d = tempfile::tempdir().unwrap();
    let md = d.path().join("notiz.md");
    fs::write(&md, "# Größe\n\nText mit *Betonung* und [Link](https://example.org).\n\n1. a\n2. b\n").unwrap();
    let html = fs::read_to_string(run(&md, "md_html").unwrap()).unwrap();
    assert!(html.contains("<h1>Größe</h1>") && html.contains("<em>Betonung</em>") && html.contains("href=\"https://example.org\""));
    assert!(html.contains("charset=\"utf-8\""));
}

#[cfg(target_os = "macos")]
mod office {
    use super::*;

    fn docx_from_txt(dir: &Path) -> PathBuf {
        let txt = dir.join("brief.txt");
        fs::write(&txt, TXT).unwrap();
        let out = run(&txt, "docx").unwrap();
        fs::rename(&out, dir.join("echt.docx")).unwrap();
        dir.join("echt.docx")
    }

    #[test]
    fn txt_to_docx_is_a_word_zip() {
        let d = tempfile::tempdir().unwrap();
        let docx = docx_from_txt(d.path());
        assert!(zip_has(&docx, "word/document.xml"));
        let mut z = zip::ZipArchive::new(fs::File::open(&docx).unwrap()).unwrap();
        let mut xml = String::new();
        z.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
        assert!(xml.contains("Grüße aus Köln"));
    }

    #[test]
    fn docx_roundtrips_to_txt_odt_rtf() {
        let d = tempfile::tempdir().unwrap();
        let docx = docx_from_txt(d.path());
        let t = fs::read_to_string(run(&docx, "txt").unwrap()).unwrap();
        assert!(t.contains("Übermut: Maßnahme"), "{t}");
        let odt = run(&docx, "odt").unwrap();
        assert!(zip_has(&odt, "content.xml"));
        let rtf = fs::read_to_string(run(&docx, "rtf").unwrap()).unwrap();
        assert!(rtf.starts_with("{\\rtf"));
        let t = fs::read_to_string(run(&odt, "txt").unwrap()).unwrap();
        assert!(t.contains("Grüße aus Köln"), "{t}");
    }

    #[test]
    fn docx_to_markdown_and_html() {
        let d = tempfile::tempdir().unwrap();
        let docx = docx_from_txt(d.path());
        let md = fs::read_to_string(run(&docx, "md").unwrap()).unwrap();
        assert!(md.contains("Grüße aus Köln") && !md.contains('<'), "{md}");
        let html = fs::read_to_string(run(&docx, "html_doc").unwrap()).unwrap();
        assert!(html.contains("Köln"));
    }

    #[test]
    fn markdown_to_office_formats() {
        let d = tempfile::tempdir().unwrap();
        let md = d.path().join("notiz.md");
        fs::write(&md, "# Überschrift\n\nEin **fetter** Absatz.\n").unwrap();
        let docx = run(&md, "docx").unwrap();
        assert!(zip_has(&docx, "word/document.xml"));
        assert!(zip_has(&run(&md, "odt").unwrap(), "content.xml"));
        assert!(fs::read_to_string(run(&md, "rtf").unwrap()).unwrap().starts_with("{\\rtf"));
        let back = fs::read_to_string(run(&docx, "md").unwrap()).unwrap();
        assert!(back.contains("Überschrift") && back.contains("fetter"), "{back}");
    }

    #[test]
    fn html_to_markdown_keeps_structure() {
        let d = tempfile::tempdir().unwrap();
        let html = d.path().join("seite.html");
        fs::write(
            &html,
            "<html><head><title>x</title><style>p{}</style></head><body><h2>Titel &amp; Ö</h2><p>Ein <b>fetter</b> und <i>schräger</i> Text, <a href=\"https://a.de\">Link</a>.</p><ul><li>eins</li><li>zwei</li></ul><ol><li>a</li><li>b</li></ol></body></html>",
        )
        .unwrap();
        let md = fs::read_to_string(run(&html, "md").unwrap()).unwrap();
        for want in ["## Titel & Ö", "**fetter**", "*schräger*", "[Link](https://a.de)", "- eins", "- zwei", "1. a", "2. b"] {
            assert!(md.contains(want), "{want} fehlt in:\n{md}");
        }
        assert!(!md.contains("p{}"));
    }

    #[test]
    fn txt_to_markdown_is_trivial() {
        let d = tempfile::tempdir().unwrap();
        let txt = d.path().join("a.txt");
        fs::write(&txt, TXT).unwrap();
        assert_eq!(fs::read_to_string(run(&txt, "md").unwrap()).unwrap(), TXT);
    }

    #[test]
    fn office_documents_become_pdf() {
        let d = tempfile::tempdir().unwrap();
        let docx = docx_from_txt(d.path());
        let out = run(&docx, "office_pdf").unwrap();
        let t = pdf_extract::extract_text(&out).unwrap();
        assert!(t.contains("Grüße aus Köln") && t.contains("Maßnahme"), "{t}");
    }
}
