use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use lopdf::{dictionary, Document, Object, Stream};
use orbo_core::chains::{self, ChainStep};
use orbo_core::jobs::{convert_blocking, convert_merge_blocking, JobSpec, MergeSpec, Options};
use orbo_core::output::{natural_cmp, natural_sorted};
use orbo_core::{analyze, tools, Catalog, Rules};

fn pdf(path: &Path, texts: &[&str], media_box: [i64; 4], on_pages_node: bool) {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    let res = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
    let mb: Vec<Object> = media_box.iter().map(|v| (*v).into()).collect();
    let mut kids = Vec::new();
    for t in texts {
        let content = doc.add_object(Stream::new(dictionary! {}, format!("BT /F1 24 Tf 72 100 Td ({t}) Tj ET").into_bytes()));
        let mut page = dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => content };
        if !on_pages_node {
            page.set("MediaBox", mb.clone());
            page.set("Resources", res);
        }
        kids.push(Object::from(doc.add_object(page)));
    }
    let mut node = dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => texts.len() as i64 };
    if on_pages_node {
        node.set("MediaBox", mb);
        node.set("Resources", res);
    }
    doc.objects.insert(pages_id, Object::Dictionary(node));
    let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", cat);
    doc.save(path).unwrap();
}

/// Page texts and media boxes of a PDF, in page order.
fn pages_of(path: &Path) -> Vec<(String, Vec<f32>)> {
    let doc = Document::load(path).unwrap();
    doc.get_pages()
        .values()
        .map(|id| {
            let content = String::from_utf8_lossy(&doc.get_page_content(*id)).into_owned();
            let text = content.split('(').nth(1).and_then(|s| s.split(')').next()).unwrap_or("").to_string();
            let page = doc.get_dictionary(*id).unwrap();
            let mb = page.get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|o| o.as_float().unwrap()).collect();
            (text, mb)
        })
        .collect()
}

fn png(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
    let p = dir.join(name);
    image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, 255])).save(&p).unwrap();
    p
}

fn jpg(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
    let p = dir.join(name);
    image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 200])).save(&p).unwrap();
    p
}

fn merge(inputs: &[PathBuf], target: &str, preset: Option<&str>) -> anyhow::Result<PathBuf> {
    let options = Options { preset: preset.map(String::from), ..Default::default() };
    convert_merge_blocking(&MergeSpec { inputs: inputs.to_vec(), target: target.into(), options }, 1, &AtomicBool::new(false), &|_| {})
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[test]
fn natural_sort_puts_page_2_before_page_10() {
    assert_eq!(natural_cmp("Seite 2.pdf", "Seite 10.pdf"), std::cmp::Ordering::Less);
    let paths: Vec<PathBuf> = ["/x/Seite 10.pdf", "/x/seite 2.pdf", "/a/Seite 1.pdf", "/x/Seite 02b.pdf"].iter().map(PathBuf::from).collect();
    let sorted: Vec<String> = natural_sorted(&paths).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(sorted, ["Seite 1.pdf", "seite 2.pdf", "Seite 02b.pdf", "Seite 10.pdf"]);
}

#[test]
fn merges_pdfs_in_natural_order_and_keeps_page_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("Rechnung 01.pdf");
    let b = dir.path().join("Rechnung 02.pdf");
    let c = dir.path().join("Rechnung 10.pdf");
    pdf(&a, &["R01"], [0, 0, 595, 842], false);
    // MediaBox and Resources sit on the Pages node here, the merge has to carry them to the pages.
    pdf(&b, &["R02a", "R02b"], [0, 0, 400, 300], true);
    pdf(&c, &["R10"], [0, 0, 612, 792], false);
    let out = merge(&[c.clone(), a.clone(), b.clone()], "pdf_merge", None).unwrap();
    assert_eq!(out, dir.path().join("Rechnung 01 (zusammengeführt).pdf"));
    let pages = pages_of(&out);
    let texts: Vec<&str> = pages.iter().map(|p| p.0.as_str()).collect();
    assert_eq!(texts, ["R01", "R02a", "R02b", "R10"]);
    assert_eq!(pages[1].1, [0.0, 0.0, 400.0, 300.0]);
    assert_eq!(pages[3].1, [0.0, 0.0, 612.0, 792.0]);
    // Fonts must survive: the resources of page 2 still resolve.
    let doc = Document::load(&out).unwrap();
    let (_, id) = doc.get_pages().into_iter().nth(1).unwrap();
    assert!(!doc.get_page_fonts(id).unwrap().is_empty());
    assert!(a.is_file() && b.is_file() && c.is_file());
    // Never overwrite.
    let again = merge(&[a, b, c], "pdf_merge", None).unwrap();
    assert_eq!(again, dir.path().join("Rechnung 01 (zusammengeführt) 2.pdf"));
    assert!(out.is_file());
    assert_eq!(names(dir.path()).len(), 5);
}

#[test]
fn images_become_pages_for_every_preset() {
    let dir = tempfile::tempdir().unwrap();
    let a = jpg(dir.path(), "Scan 1.jpg", 300, 200);
    let b = png(dir.path(), "Scan 2.png", 150, 300);
    let c = jpg(dir.path(), "Scan 3.jpg", 600, 100);
    let inputs = [c.clone(), a.clone(), b.clone()];

    let out = merge(&inputs, "images_pdf", None).unwrap();
    assert_eq!(out.file_name().unwrap().to_string_lossy(), "Scan 1 (zusammengeführt).pdf");
    let pages = pages_of(&out);
    assert_eq!(pages.len(), 3);
    assert_eq!(pages[0].1, [0.0, 0.0, 144.0, 96.0]);
    assert_eq!(pages[1].1, [0.0, 0.0, 72.0, 144.0]);
    assert_eq!(pages[2].1, [0.0, 0.0, 288.0, 48.0]);
    // The plain JPEG goes in without re-encoding.
    let doc = Document::load(&out).unwrap();
    let embedded = doc.objects.values().any(|o| matches!(o, Object::Stream(s) if s.dict.get(b"Filter").ok().and_then(|f| f.as_name().ok()) == Some(b"DCTDecode") && s.content == fs::read(&a).unwrap()));
    assert!(embedded);

    for (preset, w, h) in [("A4 hoch", 595.276, 841.89), ("A4 quer", 841.89, 595.276), ("Originalgröße", 144.0, 96.0)] {
        let out = merge(&inputs, "images_pdf", Some(preset)).unwrap();
        let pages = pages_of(&out);
        assert_eq!(pages.len(), 3, "{preset}");
        assert!((pages[0].1[2] - w).abs() < 0.01 && (pages[0].1[3] - h).abs() < 0.01, "{preset}: {:?}", pages[0].1);
    }
}

#[test]
fn pdf_and_images_mixed() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("Mappe 1.pdf");
    pdf(&doc, &["Deckblatt", "Inhalt"], [0, 0, 595, 842], false);
    let img = png(dir.path(), "Mappe 2.png", 200, 100);
    let out = merge(&[img.clone(), doc.clone()], "pdf_merge", None).unwrap();
    let pages = pages_of(&out);
    assert_eq!(pages.len(), 3);
    assert_eq!(pages[0].0, "Deckblatt");
    assert_eq!(pages[2].1, [0.0, 0.0, 96.0, 48.0]);
}

#[test]
fn merge_is_only_offered_for_several_files() {
    let dir = tempfile::tempdir().unwrap();
    let p1 = dir.path().join("a.pdf");
    let p2 = dir.path().join("b.pdf");
    pdf(&p1, &["A"], [0, 0, 595, 842], false);
    pdf(&p2, &["B"], [0, 0, 595, 842], false);
    let i1 = png(dir.path(), "a.png", 20, 20);
    let i2 = jpg(dir.path(), "b.jpg", 20, 20);
    let catalog = Catalog::current();
    let rules = Rules::builtin();
    let ids = |s: &orbo_core::Suggestions| s.ring.iter().chain(&s.more).map(|x| x.id.clone()).collect::<Vec<_>>();
    let merge_ids = ["pdf_merge", "images_pdf", "audio_concat"];

    for single in [&p1, &i1] {
        let a = analyze(single).unwrap();
        let s = rules.suggest(&catalog, &a);
        assert!(ids(&s).iter().all(|id| !merge_ids.contains(&id.as_str())), "{single:?}");
        assert!(catalog.compatible(&a).iter().all(|t| !t.merge));
    }
    let two = |a: &Path, b: &Path| rules.suggest_many(&catalog, &[analyze(a).unwrap(), analyze(b).unwrap()]);

    let s = two(&p1, &p2);
    assert_eq!(s.default.as_deref(), Some("pdf_merge"));
    assert!(s.ring.iter().any(|x| x.id == "pdf_merge" && x.merge));
    assert!(!ids(&s).contains(&"images_pdf".to_string()));

    let s = two(&i1, &i2);
    assert_eq!(s.default.as_deref(), Some("images_pdf"));
    assert!(!ids(&s).contains(&"pdf_merge".to_string()));

    let s = two(&i1, &p1);
    assert_eq!(s.default.as_deref(), Some("pdf_merge"));

    // A file the merge target does not take keeps it out of the list.
    let txt = dir.path().join("n.txt");
    fs::write(&txt, "hallo").unwrap();
    assert!(!ids(&two(&p1, &txt)).contains(&"pdf_merge".to_string()));

    // Single-file conversion of a merge target is refused with a clear message.
    let err = convert_blocking(&JobSpec { input: p1.clone(), target: "pdf_merge".into(), options: Default::default() }, 1, &AtomicBool::new(false), &|_| {}).unwrap_err();
    assert!(format!("{err:#}").contains("mindestens zwei Dateien"));
    let err = merge(&[p1.clone()], "pdf_merge", None).unwrap_err();
    assert!(format!("{err:#}").contains("mindestens zwei Dateien"), "{err:#}");
    let err = merge(&[p1.clone(), txt], "pdf_merge", None).unwrap_err();
    assert!(format!("{err:#}").contains("n.txt"), "{err:#}");
    let err = merge(&[i1, i2], "pdf_merge", None).unwrap_err();
    assert!(format!("{err:#}").contains("PDF"), "{err:#}");
}

#[test]
fn merge_targets_are_not_allowed_in_chains() {
    let v = chains::validate(&[ChainStep { target: "pdf_merge".into(), preset: None }]);
    assert!(!v.ok);
    assert!(chains::candidates(&[]).iter().all(|t| !t.merge));
}

#[test]
fn cancel_leaves_no_temp_files() {
    let dir = tempfile::tempdir().unwrap();
    let a = png(dir.path(), "a.png", 800, 600);
    let b = png(dir.path(), "b.png", 800, 600);
    let c = png(dir.path(), "c.png", 800, 600);
    let cancel = AtomicBool::new(false);
    // Cancel after the first input has been added.
    let progress = |f: f32| {
        if f > 0.05 && f < 1.0 {
            cancel.store(true, Ordering::Relaxed);
        }
    };
    let spec = MergeSpec { inputs: vec![a, b, c], target: "images_pdf".into(), options: Default::default() };
    assert!(convert_merge_blocking(&spec, 7, &cancel, &progress).is_err());
    assert_eq!(names(dir.path()), ["a.png", "b.png", "c.png"]);
}

fn tone(dir: &Path, name: &str, hz: u32, secs: u32) -> PathBuf {
    let out = dir.join(name);
    let status = Command::new(tools::ffmpeg().unwrap())
        .args(["-hide_banner", "-loglevel", "error", "-y", "-nostdin", "-f", "lavfi", "-i", &format!("sine=frequency={hz}:duration={secs}")])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    out
}

#[test]
fn audio_is_joined_end_to_end() {
    if tools::ffmpeg().is_none() || tools::ffprobe().is_none() || tools::pick_encoder(&["libmp3lame"]).is_none() {
        eprintln!("ffmpeg fehlt, Test übersprungen");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let a = tone(dir.path(), "Ton 1.wav", 440, 1);
    let b = tone(dir.path(), "Ton 2.mp3", 660, 2);
    // Keeps the format of the first file (after sorting).
    let out = merge(&[b.clone(), a.clone()], "audio_concat", None).unwrap();
    assert_eq!(out, dir.path().join("Ton 1 (zusammengeführt).wav"));
    let dur = analyze(&out).unwrap().media.unwrap().duration_s;
    assert!((dur - 3.0).abs() < 0.2, "Dauer {dur}");

    let out = merge(&[a.clone(), b.clone()], "audio_concat", Some("MP3")).unwrap();
    assert_eq!(out.extension().unwrap(), "mp3");
    let dur = analyze(&out).unwrap().media.unwrap().duration_s;
    assert!((dur - 3.0).abs() < 0.3, "Dauer {dur}");

    let catalog = Catalog::current();
    let s = Rules::builtin().suggest_many(&catalog, &[analyze(&a).unwrap(), analyze(&b).unwrap()]);
    assert_eq!(s.default.as_deref(), Some("audio_concat"));

    // Cancelling kills ffmpeg and removes the partial file.
    let before = names(dir.path());
    let cancel = AtomicBool::new(false);
    let progress = |_: f32| cancel.store(true, Ordering::Relaxed);
    let spec = MergeSpec { inputs: vec![a, b], target: "audio_concat".into(), options: Options { preset: Some("M4A".into()), ..Default::default() } };
    assert!(convert_merge_blocking(&spec, 9, &cancel, &progress).is_err());
    assert_eq!(names(dir.path()), before);
}
