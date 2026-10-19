use std::path::Path;

use orbo_core::catalog::Catalog;
use orbo_core::format::{detect, Format};
use orbo_core::output;
use orbo_core::rules::{Rules, RING_SIZE};

fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

fn png_bytes(alpha: bool) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    if alpha {
        image::RgbaImage::from_pixel(40, 30, image::Rgba([10, 20, 30, 0])).write_to(&mut buf, image::ImageFormat::Png).unwrap();
    } else {
        image::RgbImage::from_pixel(40, 30, image::Rgb([10, 20, 30])).write_to(&mut buf, image::ImageFormat::Png).unwrap();
    }
    buf.into_inner()
}

#[test]
fn builtin_catalog_and_rules_are_consistent() {
    let c = Catalog::builtin();
    assert!(c.targets.len() > 60, "Katalog sollte breit sein");
    Rules::builtin().validate(c).unwrap();
    for t in &c.targets {
        assert!(orbo_core::engines::get(&t.engine).is_some(), "Engine fehlt für {}", t.id);
        assert!(!t.title.is_empty() && !t.sub.is_empty(), "Beschriftung fehlt bei {}", t.id);
        for ch in ['\u{2013}', '\u{2014}'] {
            assert!(!t.title.contains(ch) && !t.sub.contains(ch), "Gedankenstrich in {}", t.id);
        }
    }
}

#[test]
fn detects_content_not_extension() {
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "eigentlich_png.jpg", &png_bytes(false));
    let (f, mismatch) = detect(&p).unwrap();
    assert_eq!(f, Format::Png);
    assert!(mismatch);
}

#[test]
fn detects_text_formats() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(detect(&write(d.path(), "a.csv", b"a;b\n1;2\n")).unwrap().0, Format::Csv);
    assert_eq!(detect(&write(d.path(), "ohne_endung", b"[{\"a\":1}]")).unwrap().0, Format::Json);
    assert_eq!(detect(&write(d.path(), "x.txt", b"1\n00:00:01,000 --> 00:00:02,000\nHallo\n")).unwrap().0, Format::Srt);
    assert_eq!(detect(&write(d.path(), "u.vtt", b"WEBVTT\n\n00:01.000 --> 00:02.000\nHi\n")).unwrap().0, Format::Vtt);
    assert_eq!(detect(&write(d.path(), "s", b"<svg xmlns='http://www.w3.org/2000/svg'/>")).unwrap().0, Format::Svg);
    assert_eq!(detect(&write(d.path(), "x.svg", b"<?xml version=\"1.0\"?>\n<svg xmlns='http://www.w3.org/2000/svg'/>")).unwrap().0, Format::Svg);
    assert_eq!(detect(d.path()).unwrap().0, Format::Folder);
}

#[test]
fn ring_is_limited_and_has_a_default() {
    let d = tempfile::tempdir().unwrap();
    for (name, bytes) in [("a.png", png_bytes(true)), ("b.png", png_bytes(false)), ("c.csv", b"x,y\n1,2\n".to_vec())] {
        let (_, s) = orbo_core::suggest(&write(d.path(), name, &bytes)).unwrap();
        assert!(!s.ring.is_empty() && s.ring.len() <= RING_SIZE);
        assert!(s.default.is_some());
        let ids: Vec<&str> = s.ring.iter().chain(&s.more).map(|x| x.id.as_str()).collect();
        let mut dedup = ids.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(ids.len(), dedup.len(), "doppelte Ziele für {name}");
    }
}

#[test]
fn transparency_demotes_jpg_and_opaque_png_prefers_jpg() {
    let d = tempfile::tempdir().unwrap();
    let (_, with_alpha) = orbo_core::suggest(&write(d.path(), "alpha.png", &png_bytes(true))).unwrap();
    let jpg = with_alpha.ring.iter().chain(&with_alpha.more).find(|s| s.id == "jpg").unwrap();
    assert_eq!(jpg.sub, "weißer Hintergrund");
    assert_ne!(with_alpha.default.as_deref(), Some("jpg"));

    let (_, opaque) = orbo_core::suggest(&write(d.path(), "foto.png", &png_bytes(false))).unwrap();
    assert_eq!(opaque.default.as_deref(), Some("jpg"));
}

#[test]
fn same_format_is_only_offered_for_tools() {
    let d = tempfile::tempdir().unwrap();
    let (_, s) = orbo_core::suggest(&write(d.path(), "foto.png", &png_bytes(false))).unwrap();
    let all: Vec<&str> = s.ring.iter().chain(&s.more).map(|x| x.id.as_str()).collect();
    assert!(!all.contains(&"png"), "PNG nach PNG ergibt keinen Sinn");
    assert!(all.contains(&"rotate_right"), "Werkzeuge bleiben im gleichen Format verfügbar");
    let rotate = s.ring.iter().chain(&s.more).find(|x| x.id == "rotate_right").unwrap();
    assert_eq!(rotate.ext, "png", "Werkzeuge behalten das Format der Quelle");
}

#[test]
fn output_names_never_overwrite() {
    let d = tempfile::tempdir().unwrap();
    let first = output::next_free(d.path(), "Bild", "jpg");
    assert_eq!(first.file_name().unwrap(), "Bild.jpg");
    std::fs::write(&first, b"x").unwrap();
    assert_eq!(output::next_free(d.path(), "Bild", "jpg").file_name().unwrap(), "Bild 2.jpg");
    assert_eq!(output::stem_of(Path::new("/a/Archiv.tar.gz")), "Archiv");

    let temp = output::temp_for(&first, 7);
    std::fs::write(&temp, b"neu").unwrap();
    let committed = output::commit(&temp, &first).unwrap();
    assert_eq!(committed.file_name().unwrap(), "Bild 2.jpg");
    assert_eq!(std::fs::read(&first).unwrap(), b"x", "Original bleibt unangetastet");
}

#[cfg(unix)]
#[test]
fn unreadable_files_fall_back_to_extension() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let p = write(d.path(), "Video.mp4", b"not really a video but unreadable");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
    let (a, s) = orbo_core::suggest(&p).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(a.format, Format::Mp4);
    assert!(!a.readable);
    let ids: Vec<&str> = s.ring.iter().map(|x| x.id.as_str()).collect();
    assert!(ids.contains(&"mp3"), "ohne Lesezugriff trotzdem passende Ziele: {ids:?}");
}
