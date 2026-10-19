use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use lopdf::Document;
use orbo_core::jobs::{convert_blocking, JobSpec};

fn run(input: &Path, target: &str) -> anyhow::Result<PathBuf> {
    let spec = JobSpec { input: input.to_path_buf(), target: target.into(), options: Default::default() };
    convert_blocking(&spec, 1, &AtomicBool::new(false), &|_| {})
}

const MD: &str = "# Das Buch\n\nVorwort mit Grüßen.\n\n## Erstes Kapitel\n\nÄpfel, Öl und Übermut: Maßnahme.\n\n- Punkt eins\n- Punkt zwei\n\n## Zweites Kapitel\n\nNoch ein *wichtiger* Absatz über Käse.\n";

fn zip_names(path: &Path) -> Vec<String> {
    let z = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    z.file_names().map(String::from).collect()
}

fn zip_text(path: &Path, name: &str) -> String {
    let mut z = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
    let mut z = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, data) in files {
        z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap();
}

#[test]
fn markdown_to_epub_and_back_keeps_text() {
    let dir = tempfile::tempdir().unwrap();
    let md = dir.path().join("buch.md");
    fs::write(&md, MD).unwrap();
    let epub = run(&md, "epub").unwrap();
    assert_eq!(epub.extension().unwrap(), "epub");

    let mut z = zip::ZipArchive::new(fs::File::open(&epub).unwrap()).unwrap();
    let first = z.by_index(0).unwrap();
    assert_eq!(first.name(), "mimetype");
    assert_eq!(first.compression(), zip::CompressionMethod::Stored);
    drop(first);
    let names = zip_names(&epub);
    assert!(names.contains(&"META-INF/container.xml".to_string()));
    assert!(names.contains(&"OEBPS/nav.xhtml".to_string()));
    assert!(names.contains(&"OEBPS/ch003.xhtml".to_string()), "{names:?}");
    assert!(zip_text(&epub, "OEBPS/content.opf").contains("<dc:title>Das Buch</dc:title>"));

    let txt = fs::read_to_string(run(&epub, "book_txt").unwrap()).unwrap();
    assert!(txt.contains("Erstes Kapitel"));
    assert!(txt.contains("Äpfel, Öl und Übermut: Maßnahme."));
    assert!(txt.contains("- Punkt zwei"));
    assert!(txt.contains("wichtiger Absatz über Käse"));

    let back = fs::read_to_string(run(&epub, "book_md").unwrap()).unwrap();
    assert!(back.contains("## Erstes Kapitel"));
    assert!(back.contains("*wichtiger*"));

    let html = fs::read_to_string(run(&epub, "book_html").unwrap()).unwrap();
    assert!(html.contains("<h2>Zweites Kapitel</h2>") && html.contains("Käse"));
}

#[test]
fn epub_to_pdf_has_pages() {
    let dir = tempfile::tempdir().unwrap();
    let md = dir.path().join("lang.md");
    let mut text = String::from("# Lang\n\n");
    for c in 1..=3 {
        text.push_str(&format!("## Kapitel {c}\n\n"));
        for _ in 0..60 {
            text.push_str("Dies ist ein längerer Satz mit Umlauten wie ä, ö, ü und ß, damit Zeilen umbrochen werden müssen.\n\n");
        }
    }
    fs::write(&md, text).unwrap();
    let epub = run(&md, "epub").unwrap();
    let pdf = run(&epub, "book_pdf").unwrap();
    let doc = Document::load(&pdf).unwrap();
    assert!(doc.get_pages().len() >= 6, "{}", doc.get_pages().len());
}

#[test]
fn txt_and_html_become_epub() {
    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("notiz.txt");
    fs::write(&txt, "Grüße aus Köln\n\n# kein Titel\nZweite Zeile\n").unwrap();
    let epub = run(&txt, "epub").unwrap();
    let out = fs::read_to_string(run(&epub, "book_txt").unwrap()).unwrap();
    assert!(out.contains("Grüße aus Köln") && out.contains("# kein Titel"), "{out}");

    let html = dir.path().join("seite.html");
    fs::write(&html, "<html><head><title>X</title><style>p{}</style></head><body><h1>Titel &amp; Co</h1><p>Ein <b>fetter</b> Absatz&nbsp;mit &uuml;.</p><script>var a=1;</script></body></html>").unwrap();
    let epub = run(&html, "epub").unwrap();
    let md = fs::read_to_string(run(&epub, "book_md").unwrap()).unwrap();
    assert!(md.contains("# Titel & Co") && md.contains("**fetter**") && !md.contains("var a"), "{md}");
}

#[test]
fn fb2_to_epub_and_txt() {
    let dir = tempfile::tempdir().unwrap();
    let fb2 = dir.path().join("roman.fb2");
    fs::write(
        &fb2,
        r#"<?xml version="1.0" encoding="utf-8"?>
<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0">
<description><title-info><author><first-name>Anna</first-name><last-name>Müller</last-name></author><book-title>Größe</book-title><lang>de</lang></title-info></description>
<body>
<section><title><p>Anfang</p></title><p>Erster Absatz mit <emphasis>Betonung</emphasis>.</p><empty-line/><p>Zweiter Absatz.</p></section>
<section><title><p>Ende</p></title><poem><stanza><v>Zeile eins</v><v>Zeile zwei</v></stanza></poem></section>
</body></FictionBook>"#,
    )
    .unwrap();
    let epub = run(&fb2, "epub").unwrap();
    let opf = zip_text(&epub, "OEBPS/content.opf");
    assert!(opf.contains("Größe") && opf.contains("Anna Müller"));
    let txt = fs::read_to_string(run(&fb2, "book_txt").unwrap()).unwrap();
    assert!(txt.contains("Anfang") && txt.contains("Erster Absatz mit Betonung.") && txt.contains("Zeile zwei"), "{txt}");
}

#[test]
fn fb2_windows_1251_is_decoded() {
    let dir = tempfile::tempdir().unwrap();
    let fb2 = dir.path().join("rus.fb2");
    let mut bytes = b"<?xml version=\"1.0\" encoding=\"windows-1251\"?><FictionBook><description><title-info><book-title>T</book-title></title-info></description><body><section><p>".to_vec();
    bytes.extend_from_slice(&[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2]);
    bytes.extend_from_slice(b"</p></section></body></FictionBook>");
    fs::write(&fb2, bytes).unwrap();
    let txt = fs::read_to_string(run(&fb2, "book_txt").unwrap()).unwrap();
    assert!(txt.contains("Привет"), "{txt}");
}

fn mobi_file(text: &str, encryption: u16) -> Vec<u8> {
    let mut rec0 = vec![0u8; 16 + 232];
    rec0[0..2].copy_from_slice(&1u16.to_be_bytes());
    rec0[4..8].copy_from_slice(&(text.len() as u32).to_be_bytes());
    rec0[8..10].copy_from_slice(&1u16.to_be_bytes());
    rec0[10..12].copy_from_slice(&4096u16.to_be_bytes());
    rec0[12..14].copy_from_slice(&encryption.to_be_bytes());
    rec0[16..20].copy_from_slice(b"MOBI");
    rec0[20..24].copy_from_slice(&232u32.to_be_bytes());
    rec0[28..32].copy_from_slice(&65001u32.to_be_bytes());
    rec0[0xA8..0xAC].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
    let name = b"Mobi-Buch";
    let name_off = rec0.len() as u32;
    rec0[0x54..0x58].copy_from_slice(&name_off.to_be_bytes());
    rec0[0x58..0x5C].copy_from_slice(&(name.len() as u32).to_be_bytes());
    rec0.extend_from_slice(name);
    let rec1 = text.as_bytes();
    let mut out = vec![0u8; 78];
    out[60..68].copy_from_slice(b"BOOKMOBI");
    out[76..78].copy_from_slice(&2u16.to_be_bytes());
    let off0 = 78 + 16 + 2;
    let off1 = off0 + rec0.len();
    for off in [off0, off1] {
        out.extend_from_slice(&(off as u32).to_be_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
    }
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&rec0);
    out.extend_from_slice(rec1);
    out
}

#[test]
fn mobi_reads_text_and_rejects_drm() {
    let dir = tempfile::tempdir().unwrap();
    let mobi = dir.path().join("buch.mobi");
    fs::write(&mobi, mobi_file("<html><body><h1>Eins</h1><p>Grüße aus dem Süden.</p><mbp:pagebreak/><h1>Zwei</h1><p>Weiter.</p></body></html>", 0)).unwrap();
    let epub = run(&mobi, "epub").unwrap();
    assert!(zip_names(&epub).contains(&"OEBPS/ch002.xhtml".to_string()));
    let txt = fs::read_to_string(run(&mobi, "book_txt").unwrap()).unwrap();
    assert!(txt.contains("Grüße aus dem Süden.") && txt.contains("Zwei"), "{txt}");

    let drm = dir.path().join("geschuetzt.mobi");
    fs::write(&drm, mobi_file("<p>x</p>", 2)).unwrap();
    let err = run(&drm, "book_txt").unwrap_err().to_string();
    assert!(err.contains("DRM"), "{err}");
}

#[test]
fn docx_becomes_epub_with_headings() {
    let dir = tempfile::tempdir().unwrap();
    let docx = dir.path().join("bericht.docx");
    let body = r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Bericht</w:t></w:r></w:p>
<w:p><w:r><w:t>Straße und Grüße.</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Schluss</w:t></w:r></w:p>
<w:p><w:r><w:t>Ende.</w:t></w:r></w:p></w:body></w:document>"#;
    make_zip(&docx, &[("word/document.xml", body.as_bytes())]);
    let epub = run(&docx, "epub").unwrap();
    assert!(zip_names(&epub).contains(&"OEBPS/ch002.xhtml".to_string()));
    let txt = fs::read_to_string(run(&epub, "book_txt").unwrap()).unwrap();
    assert!(txt.contains("Straße und Grüße.") && txt.contains("Schluss"));
}

#[test]
fn cbz_becomes_pdf_in_natural_order() {
    let dir = tempfile::tempdir().unwrap();
    let cbz = dir.path().join("comic.cbz");
    let png = |w: u32, h: u32| {
        let mut buf = Cursor::new(Vec::new());
        image::RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30])).write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    };
    let jpg = {
        let mut buf = Cursor::new(Vec::new());
        image::RgbImage::from_pixel(40, 60, image::Rgb([10, 90, 200])).write_to(&mut buf, image::ImageFormat::Jpeg).unwrap();
        buf.into_inner()
    };
    make_zip(&cbz, &[("p10.png", &png(30, 30)), ("p2.png", &png(20, 50)), ("p1.jpg", &jpg), ("notiz.txt", b"x")]);
    let pdf = run(&cbz, "cbz_pdf").unwrap();
    let doc = Document::load(&pdf).unwrap();
    let pages = doc.get_pages();
    assert_eq!(pages.len(), 3);
    let first = doc.get_object(pages[&1]).unwrap().as_dict().unwrap();
    let mb = first.get(b"MediaBox").unwrap().as_array().unwrap();
    assert!((mb[2].as_float().unwrap() - 40.0 * 0.48).abs() < 0.1);
}

fn cues(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

#[test]
fn ass_to_srt_and_vtt_keeps_timings() {
    let dir = tempfile::tempdir().unwrap();
    let ass = dir.path().join("film.ass");
    fs::write(
        &ass,
        "[Script Info]\nTitle: x\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:01:02.50,0:01:05.00,Default,,0,0,0,,{\\i1}Grüße{\\i0}, wie geht's?\\NZweite Zeile\nComment: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,ignoriert\nDialogue: 0,0:00:01.00,0:00:02.25,Default,,0,0,0,,Erste\n",
    )
    .unwrap();
    let srt = cues(&run(&ass, "sub_srt").unwrap());
    assert_eq!(srt, "1\n00:00:01,000 --> 00:00:02,250\nErste\n\n2\n00:01:02,500 --> 00:01:05,000\nGrüße, wie geht's?\nZweite Zeile\n\n");
    let vtt = cues(&run(&ass, "sub_vtt").unwrap());
    assert!(vtt.starts_with("WEBVTT\n\n00:00:01.000 --> 00:00:02.250\nErste"));
    let sbv = cues(&run(&ass, "sub_sbv").unwrap());
    assert!(sbv.starts_with("0:00:01.000,0:00:02.250\nErste\n"));
}

#[test]
fn srt_to_ass_and_back() {
    let dir = tempfile::tempdir().unwrap();
    let srt = dir.path().join("serie.srt");
    fs::write(&srt, "1\n00:00:03,400 --> 00:00:05,000\n<i>Hallo</i> Welt\nzweite Zeile\n\n2\n01:00:00,000 --> 01:00:01,500\nSpät\n").unwrap();
    let ass = run(&srt, "sub_ass").unwrap();
    let text = cues(&ass);
    assert!(text.contains("Dialogue: 0,0:00:03.40,0:00:05.00,Default,,0,0,0,,Hallo Welt\\Nzweite Zeile"), "{text}");
    assert!(text.contains("1:00:00.00,1:00:01.50"));
    let back = cues(&run(&ass, "sub_srt").unwrap());
    assert!(back.contains("00:00:03,400 --> 00:00:05,000\nHallo Welt\nzweite Zeile"), "{back}");

    let sbv = run(&srt, "sub_sbv").unwrap();
    let again = cues(&run(&sbv, "sub_srt").unwrap());
    assert!(again.contains("1\n00:00:03,400 --> 00:00:05,000") && again.contains("2\n01:00:00,000 --> 01:00:01,500\nSpät"), "{again}");
}
