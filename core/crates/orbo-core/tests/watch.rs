use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use image::{Rgba, RgbaImage};
use orbo_core::watch_rules::*;

fn rule(folder: &Path, target: &str, sub: bool) -> WatchRule {
    WatchRule { id: "r1".into(), folder: folder.to_path_buf(), target: target.into(), preset: None, include_subfolders: sub, enabled: true }
}

fn png(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    RgbaImage::from_pixel(64, 48, Rgba([200, 30, 30, 255])).save(&path).unwrap();
    path
}

fn run(m: &WatchMatcher, p: &Path) -> Option<WatchJob> {
    let slept = Cell::new(0u32);
    let io = Io { len: &fs_len, sleep: &|_| slept.set(slept.get() + 1), paused: &|| false };
    m.process(p, &io)
}

#[test]
fn stable_wait_needs_unchanged_size() {
    let sizes = Cell::new(0usize);
    let seq = [10u64, 20, 30, 30, 30, 30];
    let next = || {
        let i = sizes.get().min(seq.len() - 1);
        sizes.set(sizes.get() + 1);
        Some(seq[i])
    };
    assert!(wait_stable(next, |_| {}, POLL, STABLE_FOR, MAX_WAIT));
    assert!(sizes.get() >= 6, "returned before three unchanged polls");
    assert!(!wait_stable(|| Some(0), |_| {}, POLL, STABLE_FOR, Duration::from_secs(5)));
    assert!(!wait_stable(|| None, |_| {}, POLL, STABLE_FOR, MAX_WAIT));
    let n = Cell::new(0u64);
    assert!(!wait_stable(|| { n.set(n.get() + 1); Some(n.get()) }, |_| {}, POLL, STABLE_FOR, Duration::from_secs(30)));
}

#[test]
fn ignore_names() {
    for n in ["a.png.crdownload", "a.part", "x.download", "~$doc.docx", ".hidden.png", ".orbo-3-123.webp", "a.tmp", "Thumbs.db"] {
        assert!(ignored_name(n), "{n}");
    }
    for n in ["Bildschirmfoto.png", "report.pdf", "partial.png"] {
        assert!(!ignored_name(n), "{n}");
    }
    assert!(carries_suffix(Path::new("/a/Foto (klein).png"), &[" (klein)".into()]));
    assert!(carries_suffix(Path::new("/a/Foto (klein) 2.png"), &[" (klein)".into()]));
    assert!(!carries_suffix(Path::new("/a/Foto.png"), &[" (klein)".into(), String::new()]));
}

#[test]
fn converts_once_and_ignores_partials_and_own_output() {
    let dir = tempfile::tempdir().unwrap();
    let m = WatchMatcher::new(&[rule(dir.path(), "webp", false)], None, false);

    let shot = png(dir.path(), "shot.png");
    let job = run(&m, &shot).expect("png should be submitted");
    assert_eq!(job.target, "webp");
    assert!(job.input.ends_with("shot.png"));
    assert!(run(&m, &shot).is_none(), "second event for the same file must not submit again");

    let partial = dir.path().join("big.png.crdownload");
    std::fs::write(&partial, b"x").unwrap();
    assert!(run(&m, &partial).is_none());
    assert!(run(&m, &png(dir.path(), ".hidden.png")).is_none());

    // The output of the job shows up as a new file and must be left alone.
    let out = png(dir.path(), "shot.webp");
    m.note_output(&out);
    assert!(run(&m, &out).is_none());
}

#[test]
fn same_format_target_does_not_loop_on_its_output() {
    let dir = tempfile::tempdir().unwrap();
    let m = WatchMatcher::new(&[rule(dir.path(), "compress_image", false)], None, false);
    assert!(run(&m, &png(dir.path(), "a.png")).is_some());
    // Suffix " (klein)" is recognised by name alone.
    assert!(run(&m, &png(dir.path(), "a (klein).png")).is_none());
    // A collision-renamed result without suffix is caught through the output ledger.
    let odd = png(dir.path(), "b 2.png");
    m.note_output(&odd);
    assert!(run(&m, &odd).is_none());
}

#[test]
fn incompatible_disabled_and_nested() {
    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("notes.txt");
    std::fs::write(&txt, "hello").unwrap();
    let m = WatchMatcher::new(&[rule(dir.path(), "webp", false)], None, false);
    assert!(run(&m, &txt).is_none(), "text cannot become WEBP");

    let sub = dir.path().join("album");
    std::fs::create_dir(&sub).unwrap();
    let nested = png(&sub, "n.png");
    assert!(run(&m, &nested).is_none(), "subfolders are off");
    let m = WatchMatcher::new(&[rule(dir.path(), "webp", true)], None, false);
    assert!(run(&m, &nested).is_some());

    let mut off = rule(dir.path(), "webp", true);
    off.enabled = false;
    let m = WatchMatcher::new(&[off], None, false);
    assert!(run(&m, &png(dir.path(), "z.png")).is_none());
}

#[test]
fn output_folder_inside_watched_folder_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("Orbo");
    std::fs::create_dir(&out).unwrap();
    let m = WatchMatcher::new(&[rule(dir.path(), "webp", true)], Some(out.clone()), false);
    assert!(run(&m, &png(&out, "r.png")).is_none());
    assert!(run(&m, &png(dir.path(), "r.png")).is_some());
}

#[test]
fn paused_skips() {
    let dir = tempfile::tempdir().unwrap();
    let m = WatchMatcher::new(&[rule(dir.path(), "webp", false)], None, false);
    let p = png(dir.path(), "p.png");
    let io = Io { len: &fs_len, sleep: &|_| {}, paused: &|| true };
    assert!(m.process(&p, &io).is_none());
    assert!(run(&m, &p).is_some(), "a skipped file is not marked as handled");
}
