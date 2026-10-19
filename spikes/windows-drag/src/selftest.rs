//! Drives a real OLE file drag with synthetic input: a test window in this process acts like Explorer
//! (DoDragDrop with a shell data object), a second thread moves the mouse and presses keys.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};
use windows::core::{implement, w, HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{RING_SCALE_PERMILLE, RING_X, RING_Y};
use crate::{config, log};

const SRC_X: i32 = 120;
const SRC_Y: i32 = 420;
const SRC_W: i32 = 200;

static mut TEST_FILE: Option<PathBuf> = None;
static mut DRAGGING: bool = false;

#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

unsafe fn begin_drag() {
    #[allow(static_mut_refs)]
    let Some(file) = TEST_FILE.clone() else { return };
    let item: IShellItem = match SHCreateItemFromParsingName(&HSTRING::from(file.as_os_str()), None) {
        Ok(i) => i,
        Err(e) => return log(&format!("Selbsttest FEHLER: ShellItem {e}")),
    };
    let data: IDataObject = match item.BindToHandler(None, &BHID_DataObject) {
        Ok(d) => d,
        Err(e) => return log(&format!("Selbsttest FEHLER: DataObject {e}")),
    };
    let source: IDropSource = Source.into();
    let mut effect = DROPEFFECT_NONE;
    let hr = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
    log(&format!("Test-Quelle: Drag beendet ({hr:?}, Effekt {})", effect.0));
}

unsafe extern "system" fn src_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => {
            DRAGGING = false;
            LRESULT(0)
        }
        WM_MOUSEMOVE if (wp.0 as u32 & MK_LBUTTON.0) != 0 && !DRAGGING => {
            DRAGGING = true;
            begin_drag();
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(hwnd, &mut ps);
            let brush = CreateSolidBrush(COLORREF(0x00D0_5AC8));
            FillRect(dc, &ps.rcPaint, brush);
            let _ = DeleteObject(brush);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn make_test_file(dir: &Path) -> PathBuf {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let mut pm = Pixmap::new(800, 600).unwrap();
    pm.fill(Color::from_rgba8(80, 60, 200, 255));
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(255, 120, 160, 255));
    paint.anti_alias = true;
    pm.fill_path(&PathBuilder::from_circle(400.0, 300.0, 200.0).unwrap(), &paint, FillRule::Winding, Transform::identity(), None);
    let path = dir.join("test.png");
    pm.save_png(&path).unwrap();
    path
}

fn source_thread(ready: mpsc::Sender<()>) {
    unsafe {
        let _ = OleInitialize(None);
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap();
        let class = w!("OrbitTestSource");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(src_proc), hInstance: hinstance.into(), lpszClassName: class, ..Default::default() });
        let _ = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class,
            w!("Orbit Test-Quelle"),
            WS_POPUP | WS_VISIBLE,
            SRC_X,
            SRC_Y,
            SRC_W,
            SRC_W,
            HWND::default(),
            HMENU::default(),
            hinstance,
            None,
        );
        let _ = ready.send(());
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

// ---- Synthetic input ----

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn mouse(flags: MOUSE_EVENT_FLAGS, x: i32, y: i32) {
    unsafe {
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let nx = ((x as i64 * 65535) / (sw as i64 - 1).max(1)) as i32;
        let ny = ((y as i64 * 65535) / (sh as i64 - 1).max(1)) as i32;
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT { dx: nx, dy: ny, mouseData: 0, dwFlags: flags | MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE, time: 0, dwExtraInfo: 0 },
            },
        };
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn move_to(x: i32, y: i32) {
    mouse(MOUSE_EVENT_FLAGS(0), x, y);
}

fn glide(from: (i32, i32), to: (i32, i32), steps: i32) {
    for i in 1..=steps {
        move_to(from.0 + (to.0 - from.0) * i / steps, from.1 + (to.1 - from.1) * i / steps);
        sleep(15);
    }
}

fn key(vk: VIRTUAL_KEY, up: bool) {
    unsafe {
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 },
            },
        };
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

#[derive(Clone, Copy)]
enum How {
    Key,
    Shake,
}

enum Target {
    /// Sector by index with the ring having `n` sectors.
    Sector(usize, usize),
    Center,
    /// Trigger, drag far away, come back to the source window and release there.
    Cancel,
}

/// Returns the drop position relative to the measured ring center.
fn run_case(how: How, target: Target) {
    let start = (SRC_X + SRC_W / 2, SRC_Y + SRC_W / 2);
    move_to(start.0, start.1);
    sleep(200);
    mouse(MOUSEEVENTF_LEFTDOWN, start.0, start.1);
    sleep(120);
    let mut p = start;
    for _ in 0..30 {
        p.0 += 8;
        move_to(p.0, p.1);
        sleep(12);
    }
    sleep(250);
    RING_X.store(0, Ordering::Relaxed);
    let cfg_key = config().key;
    match how {
        How::Key => {
            key(cfg_key, false);
            sleep(30);
            for _ in 0..4 {
                move_to(p.0, p.1);
                sleep(20);
            }
        }
        How::Shake => {
            for i in 0..10 {
                p.0 += if i % 2 == 0 { 45 } else { -45 };
                for _ in 0..3 {
                    move_to(p.0, p.1);
                    sleep(12);
                }
            }
        }
    }
    // Nudge so the drag loop sends DragEnter to the freshly shown probe.
    move_to(p.0 + 2, p.1);
    sleep(40);
    move_to(p.0, p.1);
    sleep(400);

    let (cx, cy) = (RING_X.load(Ordering::Relaxed), RING_Y.load(Ordering::Relaxed));
    let k = RING_SCALE_PERMILLE.load(Ordering::Relaxed) as f32 / 1000.0;
    if cx == 0 {
        log("Selbsttest: Ring wurde nicht angezeigt");
    }
    let dest = match target {
        Target::Center => (cx, cy),
        Target::Sector(i, n) => {
            let a = i as f32 * std::f32::consts::TAU / n as f32;
            (cx + (120.0 * k * a.sin()) as i32, cy - (120.0 * k * a.cos()) as i32)
        }
        Target::Cancel => (cx + (380.0 * k) as i32, cy),
    };
    glide(p, dest, 20);
    if let How::Key = how {
        key(cfg_key, true);
    }
    for _ in 0..5 {
        move_to(dest.0 + 1, dest.1);
        sleep(20);
        move_to(dest.0, dest.1);
        sleep(20);
    }
    if let Target::Cancel = target {
        sleep(300);
        glide(dest, start, 25);
        sleep(200);
    }
    sleep(400);
    let (rx, ry) = cursor_pos();
    log(&format!("Loslassen bei ({rx}, {ry}), Ring-Mitte ({cx}, {cy})"));
    mouse(MOUSEEVENTF_LEFTUP, if let Target::Cancel = target { start.0 } else { dest.0 }, if let Target::Cancel = target { start.1 } else { dest.1 });
}

fn cursor_pos() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// Checks that injected absolute coordinates land where intended (RDP and scaling can distort them).
fn calibrate() {
    let (sw, sh) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    let mut worst = 0;
    for &(x, y) in &[(300, 300), (600, 500), (220, 520)] {
        move_to(x, y);
        sleep(80);
        let (ax, ay) = cursor_pos();
        worst = worst.max((ax - x).abs().max((ay - y).abs()));
        log(&format!("Kalibrierung: Soll ({x}, {y}) · Ist ({ax}, {ay})"));
    }
    log(&format!("Bildschirm {sw}×{sh} · größte Abweichung {worst} px{}", if worst > 3 { " · ACHTUNG: Eingaben landen ungenau" } else { "" }));
}

fn wait_for(path: &Path, ms: u64) -> bool {
    let mut waited = 0;
    while waited < ms {
        if path.exists() {
            return true;
        }
        sleep(100);
        waited += 100;
    }
    path.exists()
}

fn count(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|r| r.count()).unwrap_or(0)
}

pub fn start() {
    std::thread::spawn(|| {
        let dir = std::env::temp_dir().join("OrbitSpikeTest");
        let file = make_test_file(&dir);
        unsafe {
            TEST_FILE = Some(file.clone());
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || source_thread(tx));
        let _ = rx.recv();

        log("SELBSTTEST: In 6 Sekunden übernimmt der Test für ca. 40 s die Maus.");
        log("           Über RDP: Mac-Maus jetzt AUS dem RDP-Fenster nehmen, sonst schickt der Client ihre Position mit.");
        sleep(6000);
        calibrate();

        // test.png has sectors JPG, 1600 px, BMP, TIFF, GIF, ZIP
        let mut results = Vec::new();

        log("── Fall 1: Taste, Feld oben (JPG)");
        run_case(How::Key, Target::Sector(0, 6));
        results.push(("Taste → JPG", wait_for(&dir.join("test.jpg"), 8000)));

        log("── Fall 2: Schütteln, Feld rechts oben (1600 px)");
        run_case(How::Shake, Target::Sector(1, 6));
        results.push(("Schütteln → 1600 px", wait_for(&dir.join("test (1600 px).jpg"), 8000)));

        log("── Fall 3: Taste, Mitte (Vorschlag JPG)");
        run_case(How::Key, Target::Center);
        results.push(("Mitte → JPG", wait_for(&dir.join("test 2.jpg"), 8000)));

        log("── Fall 4: Taste, weit wegziehen, auf der Quelle loslassen (Abbruch)");
        let before = count(&dir);
        run_case(How::Key, Target::Cancel);
        sleep(2500);
        results.push(("Abbrechen erzeugt nichts", count(&dir) == before));

        let passed = results.iter().filter(|r| r.1).count();
        log("════ SELBSTTEST ERGEBNIS");
        for (name, ok) in &results {
            log(&format!("  {} {name}", if *ok { "✅" } else { "❌" }));
        }
        log(&format!("  {passed} von {} bestanden · Testdateien: {}", results.len(), dir.display()));
        sleep(300);
        std::process::exit(if passed == results.len() { 0 } else { 1 });
    });
}
