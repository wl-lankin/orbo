use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard};

use anyhow::Result;
use orbo_core::ai::{self, AiConfig, Llm};
use orbo_core::catalog::Catalog;
use orbo_core::engines::ai as engine;
use orbo_core::jobs::{convert_blocking, JobSpec, Options};
use orbo_core::rules::Rules;

/// The AI config is process-wide; tests that touch it run one at a time.
static LOCK: Mutex<()> = Mutex::new(());

fn guard() -> MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn cfg(url: &str, model: &str, actions: &[&str]) -> AiConfig {
    AiConfig { url: url.into(), model: Some(model.into()), actions: actions.iter().map(|s| s.to_string()).collect(), allow_remote: false }
}

const ALL: [&str; 4] = ["rename", "alt_text", "summary", "translate"];

struct Mock<F: Fn(&str) -> String + Send + Sync>(F);

impl<F: Fn(&str) -> String + Send + Sync> Llm for Mock<F> {
    fn generate(&self, prompt: &str, _images: &[String]) -> Result<String> {
        Ok((self.0)(prompt))
    }
}

fn never() -> bool {
    false
}

fn ids(path: &Path) -> Vec<String> {
    let a = orbo_core::analyze(path).unwrap();
    Catalog::builtin().compatible(&a).iter().map(|t| t.id.clone()).collect()
}

fn run(input: &Path, target: &str, preset: Option<&str>) -> Result<PathBuf> {
    let spec = JobSpec { input: input.to_path_buf(), target: target.into(), options: Options { preset: preset.map(String::from), ..Default::default() } };
    convert_blocking(&spec, 1, &AtomicBool::new(false), &|_| {})
}

// ---------- pure helpers ----------

#[test]
fn sanitizes_model_answers_into_file_names() {
    assert_eq!(engine::sanitize_name("\"Sonnenuntergang am Kochelsee\"", "jpg", 60), "Sonnenuntergang am Kochelsee");
    assert_eq!(engine::sanitize_name("<think>hm</think>\n**Rechnung: Strom/Gas?**", "pdf", 60), "Rechnung Strom Gas");
    assert_eq!(engine::sanitize_name("```\nMietvertrag.pdf\n```", "pdf", 60), "Mietvertrag");
    assert_eq!(engine::sanitize_name("„Katze auf Sofa“.", "png", 60), "Katze auf Sofa");
    assert_eq!(engine::sanitize_name("...versteckt", "txt", 60), "versteckt");
    assert_eq!(engine::sanitize_name("a\tb\r\nzweite Zeile", "txt", 60), "a b");
    assert_eq!(engine::sanitize_name("<think>nur Gedanken", "txt", 60), "");
    let long = engine::sanitize_name(&"Wort ".repeat(30), "txt", 60);
    assert!(long.chars().count() <= 60 && !long.ends_with(' '), "{long:?}");
    let umlauts = engine::sanitize_name(&"ä".repeat(100), "txt", 60);
    assert_eq!(umlauts.chars().count(), 60);
}

#[test]
fn composes_stem_with_valid_date_only() {
    assert_eq!(engine::compose_stem(Some("2026-10-18"), "Sonnenuntergang"), "2026-10-18 Sonnenuntergang");
    assert_eq!(engine::compose_stem(Some("18.10.2026"), "Sonnenuntergang"), "Sonnenuntergang");
    assert_eq!(engine::compose_stem(Some("2026-13-01"), "X"), "X");
    assert_eq!(engine::compose_stem(None, "X"), "X");
}

#[test]
fn cleans_answers() {
    assert_eq!(ai::clean_answer("<think>a\nb</think>\n\nHallo"), "Hallo");
    assert_eq!(ai::clean_answer("```markdown\n# Titel\n- a\n```"), "# Titel\n- a");
    assert_eq!(ai::clean_answer("  nur Text  "), "nur Text");
    assert_eq!(ai::strip_quotes("„Hallo“"), "Hallo");
}

#[test]
fn rename_text_reads_date_and_name_lines() {
    let llm = Mock(|_| "DATUM: 2026-03-05\nNAME: \"Mietvertrag Wohnung Kochel\"".to_string());
    assert_eq!(engine::rename_text(&llm, "irgendein Text", "pdf").unwrap(), "2026-03-05 Mietvertrag Wohnung Kochel");
    let llm = Mock(|_| "DATUM: -\nNAME: Protokoll".to_string());
    assert_eq!(engine::rename_text(&llm, "x", "txt").unwrap(), "Protokoll");
    let llm = Mock(|_| "Nur ein Name".to_string());
    assert_eq!(engine::rename_text(&llm, "x", "txt").unwrap(), "Nur ein Name");
    let llm = Mock(|_| "".to_string());
    assert!(engine::rename_text(&llm, "x", "txt").is_err());
}

#[test]
fn chunks_respect_the_limit() {
    let text = (0..50).map(|i| format!("Absatz {i} {}", "x".repeat(300))).collect::<Vec<_>>().join("\n\n");
    let chunks = ai::chunk_text(&text, 1000);
    assert!(chunks.len() > 10);
    assert!(chunks.iter().all(|c| c.chars().count() <= 1000));
    assert!(chunks.concat().contains("Absatz 49"));
    let one_long = "Satz eins. ".repeat(500);
    assert!(ai::chunk_text(&one_long, 700).iter().all(|c| c.chars().count() <= 700));
}

#[test]
fn summary_maps_over_chunks_then_combines() {
    let calls = Mutex::new(Vec::<String>::new());
    let llm = Mock(|p| {
        calls.lock().unwrap().push(p.to_string());
        "- Punkt".to_string()
    });
    let text = (0..40).map(|i| format!("Absatz {i} {}", "y".repeat(2000))).collect::<Vec<_>>().join("\n\n");
    let out = engine::summarize(&llm, &text, &never, &|_| {}).unwrap();
    assert_eq!(out, "- Punkt");
    let calls = calls.lock().unwrap();
    // 12 chunk calls plus the final combination
    assert_eq!(calls.len(), 13);
    assert!(calls[12].contains("Notizen zu den Teilen"));
}

#[test]
fn subtitle_translation_changes_only_text_lines() {
    let srt = "1\r\n00:00:01,000 --> 00:00:03,500\r\nHallo Welt\r\n<i>Wie geht's?</i>\r\n\r\n2\r\n00:00:04,000 --> 00:00:05,000\r\n♪\r\n\r\n3\r\n00:00:06,000 --> 00:00:08,000\r\nTschüss\r\n";
    let llm = Mock(|p| {
        let body = p.split("\n\n").last().unwrap_or("");
        body.lines().filter_map(|l| l.split_once('|')).map(|(n, t)| format!("{n}|EN {t}")).collect::<Vec<_>>().join("\n")
    });
    let out = engine::translate_subtitle(&llm, srt, "Englisch", &never, &|_| {}).unwrap();
    let expect = "1\r\n00:00:01,000 --> 00:00:03,500\r\nEN Hallo Welt\r\nEN <i>Wie geht's?</i>\r\n\r\n2\r\n00:00:04,000 --> 00:00:05,000\r\n♪\r\n\r\n3\r\n00:00:06,000 --> 00:00:08,000\r\nEN Tschüss\r\n";
    assert_eq!(out, expect);
}

#[test]
fn vtt_header_and_notes_stay_untouched_and_missing_lines_are_retried() {
    let vtt = "WEBVTT\n\nNOTE Das ist ein Hinweis\n\ncue1\n00:00:01.000 --> 00:00:02.000\nEins\nZwei\n";
    let llm = Mock(|p| {
        if p.contains("Untertitelzeilen") {
            // drops line 2 to force the single-line fallback
            "1|One".to_string()
        } else {
            "Two".to_string()
        }
    });
    let out = engine::translate_subtitle(&llm, vtt, "Englisch", &never, &|_| {}).unwrap();
    assert_eq!(out, "WEBVTT\n\nNOTE Das ist ein Hinweis\n\ncue1\n00:00:01.000 --> 00:00:02.000\nOne\nTwo\n");
}

#[test]
fn translation_honours_cancel() {
    let llm = Mock(|_| "1|x".to_string());
    let srt = "1\n00:00:01,000 --> 00:00:02,000\nHallo\n";
    assert!(engine::translate_subtitle(&llm, srt, "Englisch", &|| true, &|_| {}).is_err());
    assert!(engine::translate_plain(&llm, "Hallo", "Englisch", &|| true, &|_| {}).is_err());
}

// ---------- safety and gating ----------

#[test]
fn recognises_local_urls() {
    for u in ["http://localhost:11434", "http://127.0.0.1:11434", "http://[::1]:11434/", "localhost", "http://127.5.5.5", "HTTP://LocalHost:1"] {
        assert!(ai::is_local_url(u), "{u}");
    }
    for u in ["http://192.168.2.5:11434", "http://example.com", "https://localhost.evil.com", "http://user@evil.com", "http://127.0.0.1.evil.com", "http://10.0.0.1"] {
        assert!(!ai::is_local_url(u), "{u}");
    }
}

#[test]
fn classifies_server_scope() {
    use ai::Scope::*;
    for (u, want) in [
        ("http://localhost:11434", Local),
        ("http://192.168.2.5:11434", Private),
        ("http://10.0.0.1", Private),
        ("http://172.20.1.1", Private),
        ("http://100.92.10.4:11434", Private),
        ("http://enigma:11434", Private),
        ("http://mac-studio.local:11434", Private),
        ("http://[fd00::5]:11434", Private),
        ("http://172.32.0.1", Public),
        ("http://8.8.8.8", Public),
        ("https://ollama.example.com", Public),
        ("http://127.0.0.1.evil.com", Public),
    ] {
        assert_eq!(ai::server_scope(u), want, "{u}");
    }
}

#[test]
fn internet_server_is_refused_unless_allowed() {
    let lan = cfg("http://192.168.2.50:1", "m", &ALL);
    let err = ai::Ollama::connect(&lan, false).err().unwrap().to_string();
    assert!(err.contains("nicht erreichbar"), "own network is allowed: {err}");
    let remote = cfg("http://203.0.113.7:11434", "m", &ALL);
    let err = ai::Ollama::connect(&remote, false).err().unwrap().to_string();
    assert!(err.contains("über das Internet"), "{err}");
    // allowed: the refusal is gone, the (unreachable) address then fails later with the network message
    let mut allowed = AiConfig { url: "http://127.0.0.1:1".into(), ..remote };
    allowed.allow_remote = true;
    let err = ai::Ollama::connect(&allowed, false).err().unwrap().to_string();
    assert!(err.contains("nicht erreichbar"), "{err}");
}

#[test]
fn unreachable_server_gives_a_clear_error_from_a_job() {
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("notiz.txt");
    fs::write(&f, "Hallo").unwrap();
    ai::configure(Some(cfg("http://127.0.0.1:1", "m", &ALL)));
    let err = run(&f, "ai_summary", None).unwrap_err().to_string();
    ai::configure(None);
    assert_eq!(err, "Ollama ist nicht erreichbar. Läuft ollama serve?");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "no leftovers");
}

#[test]
fn targets_are_hidden_without_config_or_action() {
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("a.txt");
    fs::write(&txt, "Hallo Welt").unwrap();
    let png = dir.path().join("b.png");
    image::RgbImage::from_pixel(8, 8, image::Rgb([200, 30, 30])).save(&png).unwrap();

    ai::configure(None);
    assert!(!ids(&txt).iter().any(|i| i.starts_with("ai_")));
    assert!(!ids(&png).iter().any(|i| i.starts_with("ai_")));

    ai::configure(Some(cfg("http://localhost:11434", "m", &["summary"])));
    let t = ids(&txt);
    assert!(t.contains(&"ai_summary".to_string()));
    assert!(!t.contains(&"ai_translate".to_string()) && !t.contains(&"ai_rename".to_string()));
    assert!(!ids(&png).iter().any(|i| i.starts_with("ai_")), "summary does not apply to images");

    ai::configure(Some(cfg("http://localhost:11434", "m", &ALL)));
    let t = ids(&txt);
    for want in ["ai_rename", "ai_summary", "ai_translate"] {
        assert!(t.contains(&want.to_string()), "{want}");
    }
    assert!(!t.contains(&"ai_alt_text".to_string()));
    let p = ids(&png);
    assert!(p.contains(&"ai_alt_text".to_string()) && p.contains(&"ai_rename".to_string()));
    ai::configure(None);
}

#[test]
fn ring_keeps_core_targets_and_ai_lands_under_more() {
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("b.png");
    image::RgbImage::from_pixel(8, 8, image::Rgb([200, 30, 30])).save(&png).unwrap();
    let a = orbo_core::analyze(&png).unwrap();

    ai::configure(None);
    let before = Rules::builtin().suggest(Catalog::builtin(), &a);
    ai::configure(Some(cfg("http://localhost:11434", "m", &ALL)));
    let after = Rules::builtin().suggest(Catalog::builtin(), &a);
    let ring: Vec<&str> = after.ring.iter().map(|s| s.id.as_str()).collect();
    let core: Vec<&str> = before.ring.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(&ring[..core.len().min(6)], &core[..core.len().min(6)], "core ring unchanged");
    assert!(after.more.iter().any(|s| s.id == "ai_alt_text" && s.ai && s.group == "KI"));
    ai::configure(None);
}

#[test]
fn ai_rename_fills_only_free_slots() {
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    // avif has four core entries, so there is room
    let rules = Rules::from_yaml("ring:\n  format:\n    png: [webp, jpg]\nrules:\n  - why: t\n    when: { kind: [image] }\n    fill: [ai_rename]\n").unwrap();
    let png = dir.path().join("b.png");
    image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3])).save(&png).unwrap();
    let a = orbo_core::analyze(&png).unwrap();
    ai::configure(Some(cfg("http://localhost:11434", "m", &ALL)));
    let s = rules.suggest(Catalog::builtin(), &a);
    assert!(s.ring.iter().any(|x| x.id == "ai_rename"));
    ai::configure(None);
    let s = rules.suggest(Catalog::builtin(), &a);
    assert!(!s.ring.iter().any(|x| x.id == "ai_rename"));
    Rules::builtin().validate(Catalog::builtin()).unwrap();
}

// ---------- live (need Ollama; skipped otherwise) ----------

fn has_model(name: &str) -> bool {
    let Ok(mut r) = ureq::get("http://localhost:11434/api/tags").config().timeout_global(Some(std::time::Duration::from_secs(2))).build().call() else { return false };
    let Ok(v) = r.body_mut().read_json::<serde_json::Value>() else { return false };
    v["models"].as_array().map_or(false, |a| a.iter().any(|m| m["name"].as_str() == Some(name)))
}

macro_rules! live {
    ($model:expr) => {
        if !has_model($model) {
            eprintln!("skipped: Ollama or model {} not available", $model);
            return;
        }
    };
}

const TEXT_MODEL: &str = "gemma2:9b";
const VISION_MODEL: &str = "qwen3.8:27b";

fn red_circle(path: &Path) {
    let mut img = image::RgbImage::from_pixel(160, 160, image::Rgb([40, 90, 200]));
    for y in 0..160i32 {
        for x in 0..160i32 {
            if (x - 80).pow(2) + (y - 80).pow(2) < 50 * 50 {
                img.put_pixel(x as u32, y as u32, image::Rgb([220, 30, 30]));
            }
        }
    }
    img.save(path).unwrap();
}

#[test]
fn live_summary() {
    live!(TEXT_MODEL);
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("Bericht.txt");
    fs::write(&f, "Der Gemeinderat von Kochel hat am Montag beschlossen, den Radweg am See bis Ende 2027 zu verbreitern. Die Kosten von 1,2 Millionen Euro trägt zur Hälfte der Freistaat.").unwrap();
    ai::configure(Some(cfg("http://localhost:11434", TEXT_MODEL, &ALL)));
    let out = run(&f, "ai_summary", None);
    ai::configure(None);
    let out = out.unwrap();
    assert_eq!(out.file_name().unwrap().to_string_lossy(), "Bericht (Zusammenfassung).md");
    let text = fs::read_to_string(&out).unwrap();
    assert!(text.to_lowercase().contains("radweg"), "{text}");
    assert!(!text.contains("<think>"));
}

#[test]
fn live_translate_srt() {
    live!(TEXT_MODEL);
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("film.srt");
    let srt = "1\n00:00:01,000 --> 00:00:03,000\nGuten Morgen, wie geht es dir?\n\n2\n00:00:04,000 --> 00:00:06,000\nMir geht es gut, danke.\n";
    fs::write(&f, srt).unwrap();
    ai::configure(Some(cfg("http://localhost:11434", TEXT_MODEL, &ALL)));
    let out = run(&f, "ai_translate", Some("Englisch"));
    ai::configure(None);
    let out = out.unwrap();
    assert_eq!(out.file_name().unwrap().to_string_lossy(), "film (Englisch).srt");
    let text = fs::read_to_string(&out).unwrap();
    let timing = |s: &str| s.lines().filter(|l| l.contains("-->") || l.trim().parse::<u32>().is_ok()).map(String::from).collect::<Vec<_>>();
    assert_eq!(timing(&text), timing(srt), "{text}");
    assert!(!text.contains("Guten Morgen"), "{text}");
    assert!(f.exists());
}

#[test]
fn live_rename_text() {
    live!(TEXT_MODEL);
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("scan0042.txt");
    fs::write(&f, "Mietvertrag\nzwischen Anna Huber (Vermieterin) und Peter Maier (Mieter) über die Wohnung Seestraße 5 in Kochel am See.\nBeginn: 1. April 2026. Datum: 5. März 2026.").unwrap();
    ai::configure(Some(cfg("http://localhost:11434", TEXT_MODEL, &ALL)));
    let out = run(&f, "ai_rename", None);
    ai::configure(None);
    let out = out.unwrap();
    eprintln!("renamed to {out:?}");
    assert_ne!(out, f);
    assert_eq!(out.extension().unwrap(), "txt");
    assert_eq!(fs::read_to_string(&out).unwrap(), fs::read_to_string(&f).unwrap(), "copy");
    assert!(out.file_stem().unwrap().to_string_lossy().chars().count() <= 70);
}

// Slow: the 27b vision model needs minutes per answer on this Mac. Run with --ignored.
#[test]
#[ignore]
fn live_alt_text_and_rename_image() {
    live!(VISION_MODEL);
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("IMG_4821.png");
    red_circle(&f);
    ai::configure(Some(cfg("http://localhost:11434", VISION_MODEL, &ALL)));
    let alt = run(&f, "ai_alt_text", None);
    let ren = run(&f, "ai_rename", None);
    ai::configure(None);
    let alt = alt.unwrap();
    assert_eq!(alt.file_name().unwrap().to_string_lossy(), "IMG_4821 (Beschreibung).txt");
    let text = fs::read_to_string(&alt).unwrap();
    eprintln!("alt text: {text}");
    assert!(text.trim().len() > 10);
    let ren = ren.unwrap();
    eprintln!("renamed to {ren:?}");
    assert_eq!(ren.extension().unwrap(), "png");
    assert_ne!(ren.file_stem().unwrap(), "IMG_4821");
    assert!(f.exists());
}

#[test]
fn live_text_model_cannot_do_images() {
    live!(TEXT_MODEL);
    let _g = guard();
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("x.png");
    red_circle(&f);
    ai::configure(Some(cfg("http://localhost:11434", TEXT_MODEL, &ALL)));
    let err = run(&f, "ai_alt_text", None);
    ai::configure(None);
    let err = err.unwrap_err().to_string();
    assert!(err.starts_with("Das gewählte Modell kann keine Bilder lesen"), "{err}");
}
