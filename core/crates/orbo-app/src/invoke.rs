//! Requests from outside the drag watcher: `orbo://open?f=...` links (Finder Quick Action) and `--open <paths>`
//! (Explorer verb, or a second launch forwarded by the single-instance plugin). They open the ring at the
//! mouse pointer in click mode.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::platform::{Point, RING_CENTER};
use crate::windows::{self, CLICK_MODE, RING_SIZE};

/// Explorer starts one process per selected file; paths arriving within this window share one ring.
const COLLECT: Duration = Duration::from_millis(300);
/// A click farther than this from the ring centre (CSS px) dismisses the ring. Matches ring.js `R.pout + 16`.
const OUTSIDE: f64 = 244.0 + 16.0;

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn clean(paths: Vec<String>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        let p = PathBuf::from(p);
        if p.is_absolute() && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// `orbo://open?f=<encoded path>&f=<...>` to paths. Anything else yields nothing.
pub fn parse_url(url: &str) -> Vec<PathBuf> {
    let Some(rest) = url.strip_prefix("orbo://") else { return Vec::new() };
    let rest = rest.split('#').next().unwrap_or("");
    let (action, query) = rest.split_once('?').unwrap_or((rest, ""));
    if action.trim_end_matches('/') != "open" {
        return Vec::new();
    }
    clean(query.split('&').filter_map(|kv| kv.strip_prefix("f=")).map(percent_decode).filter(|p| !p.is_empty()).collect())
}

/// `--open <path>...` up to the next `--` option. The first element is the executable.
pub fn parse_args(args: &[String]) -> Vec<PathBuf> {
    let Some(i) = args.iter().position(|a| a == "--open") else { return Vec::new() };
    clean(args[i + 1..].iter().take_while(|a| !a.starts_with("--")).cloned().collect())
}

static PENDING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static COLLECTING: AtomicBool = AtomicBool::new(false);

/// Queues paths for the click-mode ring. Calls within `COLLECT` of the first one are merged.
pub fn submit(app: &AppHandle, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    {
        let mut pending = PENDING.lock().unwrap();
        for p in paths {
            if !pending.contains(&p) {
                pending.push(p);
            }
        }
    }
    if COLLECTING.swap(true, SeqCst) {
        return;
    }
    // The pointer is still at the menu entry now; the user may move it while the batch is collected.
    let at = cursor();
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(COLLECT);
        COLLECTING.store(false, SeqCst);
        let files = std::mem::take(&mut *PENDING.lock().unwrap());
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || windows::open_ring_mode(&app2, at, files, Instant::now(), true));
    });
}

static GEN: AtomicU64 = AtomicU64::new(0);

/// Feeds a click-mode ring: pointer position, primary button and Escape are polled here because the
/// non-activating window gets neither mouse moves nor key events reliably. Ends when the ring closes.
pub fn start_click_loop(app: &AppHandle) {
    let gen = GEN.fetch_add(1, SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        let (mut prev_down, mut prev_esc) = (button_down(), escape_down());
        let mut last = (f64::NAN, f64::NAN);
        loop {
            std::thread::sleep(Duration::from_millis(10));
            if GEN.load(SeqCst) != gen || !CLICK_MODE.load(SeqCst) {
                return;
            }
            let Some(center) = *RING_CENTER.lock().unwrap() else { return };
            let (x, y) = to_window(cursor(), center);
            let (down, esc) = (button_down(), escape_down());
            let ring = app.get_webview_window("ring");
            if (x, y) != last {
                last = (x, y);
                if let Some(w) = &ring {
                    let _ = w.emit("ring:pointer", serde_json::json!({ "x": x, "y": y }));
                }
            }
            let outside = (x - RING_SIZE / 2.0).hypot(y - RING_SIZE / 2.0) > OUTSIDE;
            if (esc && !prev_esc) || (down && !prev_down && outside) {
                windows::close_ring(&app, "dismissed");
                return;
            }
            if down && !prev_down {
                if let Some(w) = &ring {
                    let _ = w.emit("ring:click", serde_json::json!({ "x": x, "y": y }));
                }
            }
            prev_down = down;
            prev_esc = esc;
        }
    });
}

#[cfg(target_os = "macos")]
fn cursor() -> Point {
    let p = objc2_app_kit::NSEvent::mouseLocation();
    Point { x: p.x, y: p.y }
}
#[cfg(target_os = "windows")]
fn cursor() -> Point {
    crate::platform::windows::cursor()
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn cursor() -> Point {
    Point::default()
}

/// Pointer in ring window CSS pixels, origin top-left.
#[cfg(target_os = "macos")]
fn to_window(p: Point, c: Point) -> (f64, f64) {
    let h = RING_SIZE / 2.0;
    (h + p.x - c.x, h - (p.y - c.y))
}
#[cfg(not(target_os = "macos"))]
fn to_window(p: Point, c: Point) -> (f64, f64) {
    let h = RING_SIZE / 2.0;
    #[cfg(target_os = "windows")]
    let scale = crate::platform::windows::scale_at(c);
    #[cfg(not(target_os = "windows"))]
    let scale = 1.0;
    (h + (p.x - c.x) / scale, h + (p.y - c.y) / scale)
}

#[cfg(target_os = "macos")]
fn button_down() -> bool {
    objc2_app_kit::NSEvent::pressedMouseButtons() & 1 != 0
}
#[cfg(target_os = "macos")]
fn escape_down() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
    }
    // kCGEventSourceStateCombinedSessionState, kVK_Escape
    unsafe { CGEventSourceKeyState(0, 53) }
}
#[cfg(target_os = "windows")]
fn button_down() -> bool {
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000 != 0 }
}
#[cfg(target_os = "windows")]
fn escape_down() -> bool {
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE};
    unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) as u16 & 0x8000 != 0 }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn button_down() -> bool {
    false
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn escape_down() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_with_encoded_paths() {
        let u = "orbo://open?f=%2fUsers%2fw%2fA%20b.png&f=%2Ftmp%2F%C3%84.jpg";
        assert_eq!(parse_url(u), vec![PathBuf::from("/Users/w/A b.png"), PathBuf::from("/tmp/Ä.jpg")]);
    }

    #[test]
    fn url_rejects_other_actions_and_relative_paths() {
        assert!(parse_url("orbo://settings?f=%2Fa").is_empty());
        assert!(parse_url("https://open?f=%2Fa").is_empty());
        assert!(parse_url("orbo://open?f=rel%2Fa").is_empty());
        assert_eq!(parse_url("orbo://open/?f=%2Fa&f=%2Fa"), vec![PathBuf::from("/a")]);
    }

    #[test]
    fn broken_percent_sequences_stay_literal() {
        assert_eq!(percent_decode("%zz%2"), "%zz%2");
        assert_eq!(percent_decode("a%41"), "aA");
    }

    #[test]
    fn args_after_open() {
        let a: Vec<String> = ["orbo", "--open", "/a b/c.png", "/d.txt", "--settings"].iter().map(|s| s.to_string()).collect();
        assert_eq!(parse_args(&a), vec![PathBuf::from("/a b/c.png"), PathBuf::from("/d.txt")]);
        assert!(parse_args(&["orbo".to_string()]).is_empty());
    }
}
