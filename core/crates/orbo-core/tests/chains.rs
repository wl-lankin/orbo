use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use image::{Rgba, RgbaImage};
use orbo_core::catalog::{Catalog, Requirement};
use orbo_core::chains::{self, Chain, ChainStep};
use orbo_core::jobs::{convert_blocking, JobSpec, Options};
use orbo_core::rules::{Rules, RING_SIZE};

// Chains and AI settings are process-wide, so tests that touch them run one at a time.
static LOCK: Mutex<()> = Mutex::new(());

fn guard() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn step(t: &str) -> ChainStep {
    ChainStep { target: t.into(), preset: None }
}

fn chain(id: &str, steps: &[&str], in_ring: bool) -> Chain {
    Chain { id: id.into(), name: format!("Ablauf {id}"), steps: steps.iter().map(|s| step(s)).collect(), in_ring }
}

fn png(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    RgbaImage::from_fn(3000, 2000, |x, y| Rgba([(x % 251) as u8, (y % 241) as u8, 90, 255])).save(&p).unwrap();
    p
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn run(input: &Path, target: &str, cancel: &AtomicBool) -> anyhow::Result<PathBuf> {
    let spec = JobSpec { input: input.to_path_buf(), target: target.into(), options: Options::default() };
    convert_blocking(&spec, 7, cancel, &|f| {
        if f >= 0.3 && cancel.load(Ordering::Relaxed) == false && std::env::var_os("ORBO_TEST_CANCEL_AT").is_some() {
            cancel.store(true, Ordering::Relaxed);
        }
    })
}

fn steps(ids: &[&str]) -> Vec<ChainStep> {
    ids.iter().map(|s| step(s)).collect()
}

#[test]
fn folder_step_only_last() {
    let _g = guard();
    assert!(chains::validate(&steps(&["pdf_image", "pdf_jpg"])).ok, "Ordner am Ende ist erlaubt");
    let v = chains::validate(&steps(&["pdf_jpg", "pdf_image"]));
    assert!(!v.ok);
    assert_eq!(v.error_step, Some(0));
    assert!(v.error.unwrap().contains("letzte Schritt"));
    let v = chains::validate(&steps(&["unpack", "strip_meta"]));
    assert!(!v.ok);
}

#[test]
fn pick_folder_presets_are_rejected() {
    let _g = guard();
    let v = chains::validate(&[ChainStep { target: "unpack".into(), preset: Some("Ordner wählen …".into()) }]);
    assert!(!v.ok);
    assert!(v.error.unwrap().contains("Ordner wählen"));
    assert!(chains::validate(&[ChainStep { target: "unpack".into(), preset: Some("Hier entpacken".into()) }]).ok);
}

#[test]
fn formats_must_fit_across_steps() {
    let _g = guard();
    // resize and strip_meta keep the format; webp changes it.
    assert!(chains::validate(&steps(&["resize", "webp", "strip_meta"])).ok);
    // webp after webp is pointless for every image input
    let v = chains::validate(&steps(&["web_image", "webp"]));
    assert!(!v.ok, "webp nach webp darf nicht gehen");
    assert_eq!(v.error_step, Some(1));
    // image output can never feed an audio step
    let v = chains::validate(&steps(&["webp", "mp3"]));
    assert!(!v.ok);
    assert!(v.error.unwrap().contains("Schritt 2"));
    // same-format steps work for several input formats
    let v = chains::validate(&steps(&["resize", "compress_image"]));
    assert!(v.ok && v.inputs.contains(&"png".to_string()) && v.inputs.contains(&"jpeg".to_string()));
    // a png into "jpg, jpg" fails at step 2 for png, but jpg-then-resize works
    assert!(chains::validate(&steps(&["jpg", "resize"])).ok);
    assert!(!chains::validate(&[]).ok);
    assert!(!chains::validate(&steps(&["gibt_es_nicht"])).ok);
}

#[test]
fn candidates_follow_previous_output() {
    let _g = guard();
    let first = chains::candidates(&[]);
    assert!(first.iter().any(|t| t.id == "webp") && first.iter().any(|t| t.id == "mp3"));
    let after_webp: Vec<_> = chains::candidates(&steps(&["webp"])).iter().map(|t| t.id.clone()).collect();
    assert!(after_webp.contains(&"strip_meta".to_string()));
    assert!(!after_webp.contains(&"mp3".to_string()));
    assert!(!after_webp.contains(&"webp".to_string()));
    assert!(chains::candidates(&steps(&["pdf_image", "pdf_jpg"])).is_empty(), "nach einem Ordner geht es nicht weiter");
}

#[test]
fn target_has_union_of_requirements_and_flags() {
    let _g = guard();
    let c = chains::build_target(&chain("a", &["loudnorm", "mp3"], false)).unwrap();
    assert!(c.chain && c.engine == "chain" && c.group == "Abläufe");
    assert_eq!(c.id, "chain_a");
    assert!(c.requires.contains(&Requirement::Ffmpeg));
    assert!(c.requires.contains(&Requirement::AudioStream));
    assert_eq!(c.requires.iter().filter(|r| **r == Requirement::Ffmpeg).count(), 1);
    assert_eq!(c.output, "mp3");
    assert_eq!(c.accepts, vec!["audio".to_string()]);
    let f = chains::build_target(&chain("b", &["pdf_image", "pdf_jpg"], false)).unwrap();
    assert!(f.produces_folder());
    assert!(chains::build_target(&chain("c", &["webp", "mp3"], false)).is_none());
}

#[test]
fn three_step_run_leaves_only_the_result() {
    let _g = guard();
    chains::configure(vec![chain("web", &["resize", "webp", "strip_meta"], false)]);
    let d = tempfile::tempdir().unwrap();
    let input = png(d.path(), "foto.png");
    let a = orbo_core::analyze(&input).unwrap();
    let cat = Catalog::current();
    let t = cat.get("chain_web").unwrap();
    assert!(t.accepts(&a));
    assert_eq!(t.ext_for(&a), "webp");
    let out = run(&input, "chain_web", &AtomicBool::new(false)).unwrap();
    assert_eq!(out.extension().unwrap(), "webp");
    let res = orbo_core::analyze(&out).unwrap();
    assert_eq!(res.format, orbo_core::Format::Webp);
    let img = res.image.unwrap();
    assert_eq!(img.width.max(img.height), 1920, "erster Schritt hat auf 1920 px verkleinert");
    assert_eq!(names(d.path()), vec!["foto.png".to_string(), out.file_name().unwrap().to_string_lossy().into_owned()]);
    // never overwrites: a second run gets a new name
    let out2 = run(&input, "chain_web", &AtomicBool::new(false)).unwrap();
    assert_ne!(out, out2);
    assert_eq!(names(d.path()).len(), 3);
    chains::configure(vec![]);
}

#[test]
fn failing_step_is_named_and_cleaned_up() {
    let _g = guard();
    orbo_core::ai::configure(Some(orbo_core::ai::AiConfig { url: "http://127.0.0.1:9".into(), model: None, actions: vec!["alt_text".into()], allow_remote: false }));
    chains::configure(vec![chain("fail", &["webp", "ai_alt_text"], false)]);
    let d = tempfile::tempdir().unwrap();
    let input = png(d.path(), "foto.png");
    let err = run(&input, "chain_fail", &AtomicBool::new(false)).unwrap_err().to_string();
    assert!(err.starts_with("Schritt 2 (Bildbeschreibung): "), "{err}");
    assert_eq!(names(d.path()), vec!["foto.png".to_string()], "keine Zwischendateien oder Teilausgaben");
    orbo_core::ai::configure(None);
    chains::configure(vec![]);
}

#[test]
fn cancellation_leaves_no_files() {
    let _g = guard();
    chains::configure(vec![chain("slow", &["resize", "webp", "strip_meta"], false)]);
    let d = tempfile::tempdir().unwrap();
    let input = png(d.path(), "foto.png");
    // Cancelled before the start
    let cancel = AtomicBool::new(true);
    assert!(run(&input, "chain_slow", &cancel).is_err());
    assert_eq!(names(d.path()), vec!["foto.png".to_string()]);
    // Cancelled while running: flag flips as soon as progress passes 30 percent
    std::env::set_var("ORBO_TEST_CANCEL_AT", "1");
    let cancel = AtomicBool::new(false);
    let r = run(&input, "chain_slow", &cancel);
    std::env::remove_var("ORBO_TEST_CANCEL_AT");
    assert!(r.is_err() && cancel.load(Ordering::Relaxed), "Lauf wurde nicht abgebrochen");
    assert_eq!(names(d.path()), vec!["foto.png".to_string()]);
    chains::configure(vec![]);
}

#[test]
fn in_ring_chain_only_takes_free_slots() {
    let _g = guard();
    let d = tempfile::tempdir().unwrap();
    let input = png(d.path(), "foto.png");
    let a = orbo_core::analyze(&input).unwrap();
    let few = Rules::from_yaml("ring:\n  format:\n    png: [jpg, webp]\nrules: []\n").unwrap();

    chains::configure(vec![chain("ring", &["resize", "webp"], true)]);
    let cat = Catalog::current();
    let six: Vec<String> = cat.compatible(&a).iter().filter(|t| !t.chain).take(RING_SIZE).map(|t| t.id.clone()).collect();
    let full = Rules::from_yaml(&format!("ring:\n  format:\n    png: [{}]\nrules: []\n", six.join(", "))).unwrap();
    let ids = |s: &orbo_core::rules::Suggestions| s.ring.iter().map(|x| x.id.clone()).collect::<Vec<_>>();

    let s = few.suggest(&cat, &a);
    assert_eq!(&ids(&s)[..2], &["jpg".to_string(), "webp".to_string()]);
    assert_eq!(ids(&s)[2], "chain_ring", "Ablauf nimmt den ersten freien Platz");
    assert!(s.ring.iter().find(|x| x.id == "chain_ring").unwrap().chain);

    let s = full.suggest(&cat, &a);
    assert_eq!(ids(&s), six);
    assert!(s.more.iter().any(|x| x.id == "chain_ring" && x.group == "Abläufe"), "Ablauf liegt unter Mehr");
    assert_eq!(ids(&Rules::builtin().suggest(&cat, &a)).len(), RING_SIZE);

    chains::configure(vec![chain("ring", &["resize", "webp"], false)]);
    let s = few.suggest(&Catalog::current(), &a);
    assert_eq!(&ids(&s)[..2], &["jpg".to_string(), "webp".to_string()]);
    assert_ne!(ids(&s)[2], "chain_ring");
    chains::configure(vec![]);
}

#[test]
fn chains_work_as_watch_targets() {
    use orbo_core::watch_rules::{fs_len, Io, WatchMatcher, WatchRule};
    let _g = guard();
    chains::configure(vec![chain("watch", &["resize", "webp"], false)]);
    let d = tempfile::tempdir().unwrap();
    let input = png(d.path(), "neu.png");
    let rule = WatchRule { id: "r".into(), folder: d.path().to_path_buf(), target: "chain_watch".into(), preset: None, include_subfolders: false, enabled: true };
    let m = WatchMatcher::new(&[rule], None, false);
    let io = Io { len: &fs_len, sleep: &|_| {}, paused: &|| false };
    let job = m.process(&input, &io).expect("Ablauf wird als Ordnerziel angenommen");
    assert_eq!(job.target, "chain_watch");
    chains::configure(vec![]);
}
