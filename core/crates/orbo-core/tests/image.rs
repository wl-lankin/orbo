use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use image::{GenericImageView, Rgba, RgbaImage};
use orbo_core::format::{self, Format};
use orbo_core::jobs::{convert_blocking, CropRect, JobSpec, Options};

fn run(input: &Path, target: &str, options: Options) -> PathBuf {
    let spec = JobSpec { input: input.to_path_buf(), target: target.into(), options };
    convert_blocking(&spec, 1, &AtomicBool::new(false), &|_| {}).unwrap_or_else(|e| panic!("{target}: {e:#}"))
}

fn fmt_of(p: &Path) -> Format {
    format::detect(p).unwrap().0
}

fn dims(p: &Path) -> (u32, u32) {
    image::open(p).unwrap().dimensions()
}

fn png_fixture(dir: &Path) -> PathBuf {
    let mut img = RgbaImage::new(800, 600);
    for (x, y, p) in img.enumerate_pixels_mut() {
        *p = if x < 100 && y < 100 { Rgba([0, 0, 0, 0]) } else { Rgba([(x / 4) as u8, (y / 3) as u8, 160, 255]) };
    }
    let path = dir.join("pic.png");
    img.save(&path).unwrap();
    path
}

fn photo_jpeg(dir: &Path, name: &str) -> PathBuf {
    let mut img = image::RgbImage::new(800, 600);
    let mut s: u32 = 12345;
    for (x, y, p) in img.enumerate_pixels_mut() {
        s = s.wrapping_mul(1664525).wrapping_add(1013904223);
        let n = (s >> 24) as u8 / 8;
        *p = image::Rgb([((x / 4) as u8).wrapping_add(n), ((y / 3) as u8).wrapping_add(n), n.wrapping_mul(8)]);
    }
    let path = dir.join(name);
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 97).encode_image(&img).unwrap();
    std::fs::write(&path, buf).unwrap();
    path
}

fn opts() -> Options {
    Options::default()
}

#[test]
fn raster_formats_from_png() {
    let d = tempfile::tempdir().unwrap();
    let png = png_fixture(d.path());
    for (target, f) in [
        ("jpg", Format::Jpeg),
        ("webp", Format::Webp),
        ("avif", Format::Avif),
        ("gif_image", Format::Gif),
        ("bmp", Format::Bmp),
        ("tiff", Format::Tiff),
        ("tga", Format::Tga),
        ("ppm", Format::Pnm),
        ("qoi", Format::Qoi),
    ] {
        let out = run(&png, target, opts());
        assert_eq!(fmt_of(&out), f, "{target}");
        if target != "avif" && target != "tga" {
            assert_eq!(dims(&out), (800, 600), "{target}");
        }
    }
}

#[test]
fn png_to_ico_has_multiple_sizes() {
    let d = tempfile::tempdir().unwrap();
    let out = run(&png_fixture(d.path()), "ico", opts());
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(fmt_of(&out), Format::Ico);
    assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 7);
    assert_eq!(dims(&out), (256, 256));
}

#[test]
fn png_to_pdf() {
    let d = tempfile::tempdir().unwrap();
    let out = run(&png_fixture(d.path()), "pdf_image", opts());
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.starts_with(b"%PDF"));
    assert!(String::from_utf8_lossy(&bytes).trim_end().ends_with("%%EOF"));
    let text = String::from_utf8_lossy(&bytes);
    let pos: usize = text.split("startxref\n").nth(1).unwrap().lines().next().unwrap().parse().unwrap();
    assert_eq!(&bytes[pos..pos + 4], b"xref");
}

#[test]
fn jpg_to_png_and_svg_to_png() {
    let d = tempfile::tempdir().unwrap();
    let jpg = photo_jpeg(d.path(), "p.jpg");
    let out = run(&jpg, "png", opts());
    assert_eq!(fmt_of(&out), Format::Png);
    assert_eq!(dims(&out), (800, 600));

    let svg = d.path().join("logo.svg");
    std::fs::write(&svg, r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50" viewBox="0 0 100 50"><rect width="50" height="50" fill="#f00"/></svg>"##).unwrap();
    let out = run(&svg, "png", opts());
    let img = image::open(&out).unwrap();
    assert_eq!(img.dimensions(), (2048, 1024));
    assert_eq!(img.get_pixel(10, 10).0[3], 255);
    assert_eq!(img.get_pixel(2000, 10).0[3], 0);
}

#[cfg(target_os = "macos")]
#[test]
fn heic_roundtrip() {
    let d = tempfile::tempdir().unwrap();
    let png = png_fixture(d.path());
    let heic = d.path().join("pic.heic");
    let st = std::process::Command::new("/usr/bin/sips").args(["-s", "format", "heic"]).arg(&png).arg("--out").arg(&heic).output().unwrap();
    assert!(st.status.success() && heic.exists());
    let out = run(&heic, "jpg", opts());
    assert_eq!(fmt_of(&out), Format::Jpeg);
    assert_eq!(dims(&out), (800, 600));
    let back = run(&png, "heic", opts());
    assert_eq!(fmt_of(&back), Format::Heic);
}

#[test]
fn resize_and_percent() {
    let d = tempfile::tempdir().unwrap();
    let png = png_fixture(d.path());
    assert_eq!(dims(&run(&png, "resize", Options { max_edge: Some(400), ..opts() })), (400, 300));
    assert_eq!(dims(&run(&png, "resize", Options { width: Some(200), ..opts() })), (200, 150));
    assert_eq!(dims(&run(&png, "resize", Options { width: Some(100), height: Some(100), ..opts() })), (100, 100));
    // default max_edge 1920 never upscales
    assert_eq!(dims(&run(&png, "resize", opts())), (800, 600));
    let half = run(&png, "half_size", opts());
    assert_eq!(fmt_of(&half), Format::Png);
    assert_eq!(dims(&half), (400, 300));
}

#[test]
fn crop_square_rotate_flip_gray() {
    let d = tempfile::tempdir().unwrap();
    let png = png_fixture(d.path());
    let c = run(&png, "crop", Options { crop: Some(CropRect { x: 10, y: 20, width: 300, height: 200 }), ..opts() });
    assert_eq!(dims(&c), (300, 200));
    assert_eq!(dims(&run(&png, "crop", Options { aspect: Some("16:9".into()), ..opts() })), (800, 450));
    assert_eq!(dims(&run(&png, "square", opts())), (600, 600));
    assert_eq!(dims(&run(&png, "rotate_right", opts())), (600, 800));
    assert_eq!(dims(&run(&png, "rotate_left", opts())), (600, 800));

    let flipped = image::open(run(&png, "flip", opts())).unwrap();
    let orig = image::open(&png).unwrap();
    assert_eq!(flipped.get_pixel(799, 0), orig.get_pixel(0, 0));
    assert_eq!(flipped.get_pixel(0, 300), orig.get_pixel(799, 300));

    let g = image::open(run(&png, "grayscale", opts())).unwrap();
    let p = g.get_pixel(400, 300).0;
    assert!(p[0] == p[1] && p[1] == p[2]);
    assert_eq!(g.get_pixel(5, 5).0[3], 0);
}

#[test]
fn beautify_grows_canvas() {
    let d = tempfile::tempdir().unwrap();
    let png = png_fixture(d.path());
    let out = run(&png, "beautify", opts());
    assert_eq!(fmt_of(&out), Format::Png);
    assert_eq!(dims(&out), (992, 792));
    let img = image::open(&out).unwrap();
    assert_eq!(img.get_pixel(2, 2).0[3], 255);

    let wide = run(&png, "beautify", Options { aspect: Some("16:9".into()), background: Some("#102030".into()), padding_px: Some(40), shadow_px: Some(0), ..opts() });
    let (w, h) = dims(&wide);
    assert_eq!((w, h), (1209, 680));
    assert_eq!(image::open(&wide).unwrap().get_pixel(1, 1).0[..3], [0x10, 0x20, 0x30]);
}

#[test]
fn compress_and_target_size() {
    let d = tempfile::tempdir().unwrap();
    let jpg = photo_jpeg(d.path(), "photo.jpg");
    let orig = std::fs::metadata(&jpg).unwrap().len();
    let out = run(&jpg, "compress_image", opts());
    assert_eq!(fmt_of(&out), Format::Jpeg);
    assert!(std::fs::metadata(&out).unwrap().len() < orig);

    let limit = orig as f64 / 4.0 / 1024.0 / 1024.0;
    let small = run(&jpg, "webp", Options { target_mb: Some(limit), quality: Some(97), ..opts() });
    assert!(std::fs::metadata(&small).unwrap().len() as f64 <= limit * 1024.0 * 1024.0);
}

fn jpeg_with_gps(dir: &Path) -> PathBuf {
    use img_parts::ImageEXIF;
    let jpg = photo_jpeg(dir, "gps_src.jpg");
    let rat = |n| exif::Rational { num: n, denom: 1 };
    let fields = [
        exif::Field { tag: exif::Tag::GPSLatitudeRef, ifd_num: exif::In::PRIMARY, value: exif::Value::Ascii(vec![b"N".to_vec()]) },
        exif::Field { tag: exif::Tag::GPSLatitude, ifd_num: exif::In::PRIMARY, value: exif::Value::Rational(vec![rat(47), rat(39), rat(0)]) },
        exif::Field { tag: exif::Tag::Make, ifd_num: exif::In::PRIMARY, value: exif::Value::Ascii(vec![b"TestCam".to_vec()]) },
    ];
    let mut w = exif::experimental::Writer::new();
    for f in &fields {
        w.push_field(f);
    }
    let mut buf = Cursor::new(Vec::new());
    w.write(&mut buf, false).unwrap();
    let mut parts = img_parts::jpeg::Jpeg::from_bytes(img_parts::Bytes::from(std::fs::read(&jpg).unwrap())).unwrap();
    parts.set_exif(Some(img_parts::Bytes::from(buf.into_inner())));
    let path = dir.join("gps.jpg");
    let mut out = Vec::new();
    parts.encoder().write_to(&mut out).unwrap();
    std::fs::write(&path, out).unwrap();
    path
}

#[test]
fn strip_meta_removes_gps_losslessly() {
    let d = tempfile::tempdir().unwrap();
    let jpg = jpeg_with_gps(d.path());
    let before = orbo_core::analyze::analyze(&jpg).unwrap();
    assert!(before.image.as_ref().unwrap().has_gps);

    let out = run(&jpg, "strip_meta", opts());
    let after = orbo_core::analyze::analyze(&out).unwrap();
    let info = after.image.unwrap();
    assert!(!info.has_gps);
    assert!(info.camera.is_none());
    assert_eq!(fmt_of(&out), Format::Jpeg);
    assert_eq!(dims(&out), (800, 600));
    // pixel data is untouched, so the file is only smaller by the metadata
    assert!(std::fs::metadata(&out).unwrap().len() < std::fs::metadata(&jpg).unwrap().len());
    let a = image::open(&jpg).unwrap().to_rgb8();
    let b = image::open(&out).unwrap().to_rgb8();
    assert_eq!(a.as_raw(), b.as_raw());
}

#[test]
fn intent_targets() {
    let d = tempfile::tempdir().unwrap();
    let png = png_fixture(d.path());
    let big = d.path().join("big.png");
    RgbaImage::from_pixel(3000, 2400, Rgba([10, 200, 30, 255])).save(&big).unwrap();

    let web = run(&big, "web_image", opts());
    assert_eq!(fmt_of(&web), Format::Webp);
    assert_eq!(dims(&web), (1920, 1536));

    let mail = run(&big, "mail_image", opts());
    assert_eq!(fmt_of(&mail), Format::Jpeg);
    assert_eq!(dims(&mail), (1600, 1280));

    let insta = run(&png, "instagram_image", opts());
    assert_eq!(fmt_of(&insta), Format::Jpeg);
    assert_eq!(dims(&insta), (480, 600));
    let insta_big = run(&big, "instagram_image", opts());
    assert_eq!(dims(&insta_big), (1080, 1350));
}

#[test]
fn cancel_aborts() {
    let d = tempfile::tempdir().unwrap();
    let png = png_fixture(d.path());
    let spec = JobSpec { input: png, target: "jpg".into(), options: opts() };
    let r = convert_blocking(&spec, 1, &AtomicBool::new(true), &|_| {});
    assert!(r.is_err());
}
