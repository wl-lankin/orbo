//! Windows: system-wide file drag detection without hooks or special rights, overlay window tweaks and a log file.
//!
//! Windows has no global drag pasteboard. While the primary button is held and the trigger fires, an invisible
//! probe window (alpha 1/255) appears under the cursor. If an OLE drag with files is running, OLE sends the probe
//! `DragEnter` with `CF_HDROP` within a few milliseconds; only then the ring is shown. The probe never accepts a
//! drop and is hidden as soon as the ring is up, so the ring's own (Tauri) drop target receives the drop.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::{implement, w, Ref};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, POINTL, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, GetStockObject, MonitorFromPoint, BLACK_BRUSH, HBRUSH, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{
    IDropTarget, IDropTarget_Impl, OleInitialize, RegisterDragDrop, ReleaseStgMedium, CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_LBUTTON, VK_MENU, VK_RBUTTON, VK_SHIFT};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetCursorPos, GetSystemMetrics, GetWindowLongPtrW, RegisterClassW, SetLayeredWindowAttributes,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_EXSTYLE, HWND_TOPMOST, LWA_ALPHA, MA_NOACTIVATE, SM_CXDRAG, SM_SWAPBUTTON,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, WM_MOUSEACTIVATE, WNDCLASSW, WS_EX_APPWINDOW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use super::{DragSignal, Point, Trigger, CANCEL_DISTANCE, RING_CENTER};

// ---- Screen helpers (physical pixels, per-monitor DPI) ----

pub fn cursor() -> Point {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    Point { x: p.x as f64, y: p.y as f64 }
}

/// Monitor rectangle, work area and scale factor (1.0 = 96 dpi) of the monitor containing `p`.
fn monitor_at(p: Point) -> (RECT, RECT, f64) {
    unsafe {
        let mon = MonitorFromPoint(POINT { x: p.x as i32, y: p.y as i32 }, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        (mi.rcMonitor, mi.rcWork, dx.max(48) as f64 / 96.0)
    }
}

pub fn scale_at(p: Point) -> f64 {
    monitor_at(p).2
}

fn hwnd(h: isize) -> HWND {
    HWND(h as *mut std::ffi::c_void)
}

fn set_rect(h: isize, x: i32, y: i32, w: i32, hgt: i32) {
    unsafe {
        let _ = SetWindowPos(hwnd(h), Some(HWND_TOPMOST), x, y, w, hgt, SWP_NOACTIVATE);
    }
}

/// Places a window of `size` logical pixels centered on `at`, clamped to the monitor containing `at`.
/// Returns the window center actually used (physical pixels).
pub fn place_centered(h: isize, at: Point, size: f64) -> Point {
    let (mon, _, scale) = monitor_at(at);
    let px = (size * scale).round() as i32;
    let half = px / 2;
    let cx = (at.x as i32).clamp(mon.left + half, (mon.right - half).max(mon.left + half));
    let cy = (at.y as i32).clamp(mon.top + half, (mon.bottom - half).max(mon.top + half));
    // Moving onto a monitor with another DPI makes the window resize itself (WM_DPICHANGED); the second call wins.
    set_rect(h, cx - half, cy - half, px, px);
    set_rect(h, cx - half, cy - half, px, px);
    Point { x: cx as f64, y: cy as f64 }
}

/// Moves a window far outside every monitor, keeping its size.
pub fn park(h: isize) {
    unsafe {
        let _ = SetWindowPos(hwnd(h), Some(HWND_TOPMOST), -32000, -32000, 0, 0, SWP_NOACTIVATE | SWP_NOSIZE);
    }
}

/// Places a window at the bottom center of the work area (above the taskbar) of the monitor under the cursor.
pub fn place_bottom_center(h: isize, width: f64, height: f64) {
    let (_, work, scale) = monitor_at(cursor());
    let (w, hgt) = ((width * scale).round() as i32, (height * scale).round() as i32);
    let x = work.left + (work.right - work.left - w) / 2;
    let y = work.bottom - hgt - (16.0 * scale) as i32;
    set_rect(h, x, y, w, hgt);
    set_rect(h, x, y, w, hgt);
}

/// Topmost tool window that never takes focus and has no taskbar or Alt+Tab entry.
pub fn make_overlay(h: isize) {
    unsafe {
        let ex = GetWindowLongPtrW(hwnd(h), GWL_EXSTYLE);
        let ex = (ex | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0) as isize) & !(WS_EX_APPWINDOW.0 as isize);
        SetWindowLongPtrW(hwnd(h), GWL_EXSTYLE, ex);
    }
}

pub fn show_without_activating(h: isize) {
    make_overlay(h);
    unsafe {
        let _ = ShowWindow(hwnd(h), SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(hwnd(h), Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
}

pub fn hide(h: isize) {
    unsafe {
        let _ = ShowWindow(hwnd(h), SW_HIDE);
    }
}

/// Mica needs Windows 11 (build 22000 or newer).
pub fn is_windows_11() -> bool {
    windows_version::OsVersion::current().build >= 22000
}

// ---- Probe window and its drop target ----

#[derive(Default)]
struct Probe {
    armed: bool,
    files: Option<Vec<PathBuf>>,
    /// An OLE drag without files (text, links) reached the probe.
    rejected: bool,
}

static PROBE: Mutex<Probe> = Mutex::new(Probe { armed: false, files: None, rejected: false });
static PROBE_HWND: AtomicIsize = AtomicIsize::new(0);
const PROBE_SIZE: f64 = 160.0;

unsafe fn files_from(obj: &IDataObject) -> Option<Vec<PathBuf>> {
    let fmt = FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
    let mut medium = obj.GetData(&fmt).ok()?;
    let hdrop = HDROP(medium.u.hGlobal.0);
    let count = DragQueryFileW(hdrop, u32::MAX, None);
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let len = DragQueryFileW(hdrop, i, None) as usize;
        let mut buf = vec![0u16; len + 1];
        DragQueryFileW(hdrop, i, Some(&mut buf));
        out.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
    }
    ReleaseStgMedium(&mut medium);
    Some(out)
}

#[implement(IDropTarget)]
struct ProbeTarget;

impl IDropTarget_Impl for ProbeTarget_Impl {
    fn DragEnter(&self, data: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        let mut probe = PROBE.lock().unwrap();
        let mut e = DROPEFFECT_NONE;
        if probe.armed {
            match data.as_ref().and_then(|d| unsafe { files_from(d) }).filter(|f| !f.is_empty()) {
                Some(files) => {
                    probe.files = Some(files);
                    e = DROPEFFECT_COPY;
                }
                None => probe.rejected = true,
            }
        }
        unsafe { *effect = e };
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        let p = PROBE.lock().unwrap();
        unsafe { *effect = if p.armed && p.files.is_some() { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn Drop(&self, _data: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        // Released before the ring was up: refuse, so the source never moves or deletes anything.
        eprintln!("[orbo] Drop auf der Sonde, abgelehnt");
        unsafe { *effect = DROPEFFECT_NONE };
        Ok(())
    }
}

unsafe extern "system" fn probe_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    DefWindowProcW(h, msg, wp, lp)
}

/// Creates the hidden probe window. Must run on the main (UI) thread, which owns the OLE apartment.
pub fn init_main_thread() {
    unsafe {
        let _ = OleInitialize(None);
        let Ok(module) = GetModuleHandleW(None) else { return };
        let class = w!("OrboDragProbe");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(probe_proc),
            hInstance: module.into(),
            lpszClassName: class,
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let Ok(h) = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("Orbo"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(module.into()),
            None,
        ) else {
            eprintln!("[orbo] FEHLER: Sondenfenster konnte nicht erstellt werden");
            return;
        };
        // Alpha 1 keeps the window invisible but still hit-testable for OLE.
        let _ = SetLayeredWindowAttributes(h, COLORREF(0), 1, LWA_ALPHA);
        let target: IDropTarget = ProbeTarget.into();
        if let Err(e) = RegisterDragDrop(h, &target) {
            eprintln!("[orbo] FEHLER: RegisterDragDrop für die Sonde: {e}");
            return;
        }
        // The window lives as long as the app; OLE holds its own reference to the target.
        std::mem::forget(target);
        PROBE_HWND.store(h.0 as isize, Ordering::SeqCst);
    }
}

/// Main thread.
fn show_probe(at: Point) {
    let h = PROBE_HWND.load(Ordering::SeqCst);
    if h == 0 {
        return;
    }
    *PROBE.lock().unwrap() = Probe { armed: true, files: None, rejected: false };
    let px = (PROBE_SIZE * scale_at(at)).round() as i32;
    unsafe {
        let _ = SetWindowPos(
            hwnd(h),
            Some(HWND_TOPMOST),
            at.x as i32 - px / 2,
            at.y as i32 - px / 2,
            px,
            px,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

/// Main thread.
fn hide_probe() {
    PROBE.lock().unwrap().armed = false;
    let h = PROBE_HWND.load(Ordering::SeqCst);
    if h != 0 {
        hide(h);
    }
}

// ---- Drag watcher ----

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}

fn trigger_key(key: &str) -> VIRTUAL_KEY {
    match key {
        "control" | "command" => VK_CONTROL,
        "shift" => VK_SHIFT,
        _ => VK_MENU,
    }
}

fn primary_down() -> bool {
    // GetAsyncKeyState reports physical buttons, so swapped buttons need the right one.
    let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
    key_down(if swapped { VK_RBUTTON } else { VK_LBUTTON })
}

#[derive(Default)]
struct Shake {
    last_x: Option<f64>,
    dir: f64,
    turn_x: f64,
    turns: Vec<Instant>,
}

impl Shake {
    fn feed(&mut self, x: f64, now: Instant, scale: f64) -> bool {
        let Some(lx) = self.last_x else {
            self.last_x = Some(x);
            self.turn_x = x;
            return false;
        };
        let dx = x - lx;
        self.last_x = Some(x);
        if dx.abs() < 1.5 * scale {
            return false;
        }
        let d = dx.signum();
        if self.dir != 0.0 && d != self.dir {
            if (x - self.turn_x).abs() > 18.0 * scale {
                self.turns.push(now);
            }
            self.turn_x = x;
        }
        self.dir = d;
        self.turns.retain(|t| now.duration_since(*t) < Duration::from_millis(700));
        if self.turns.len() >= 3 {
            self.turns.clear();
            return true;
        }
        false
    }
}

pub struct WatchSettings {
    pub key: String,
    pub shake: bool,
    pub paused: bool,
}

#[derive(Clone, Copy)]
enum Phase {
    Pressed { x: f64, y: f64 },
    Dragging,
    Probing { since: Instant, at: Point, trigger: Trigger },
    Shown,
}

type MainThread = Box<dyn Fn(Box<dyn FnOnce() + Send>) + Send>;

/// Runs forever on its own thread. Polls the primary button (30 Hz idle, 120 Hz while pressed).
/// `main` runs a closure on the UI thread (probe window handling).
pub fn watch(
    settings: impl Fn() -> WatchSettings + Send + 'static,
    main: impl Fn(Box<dyn FnOnce() + Send>) + Send + 'static,
    mut signal: impl FnMut(DragSignal) + Send + 'static,
) {
    let main: MainThread = Box::new(main);
    std::thread::spawn(move || {
        let mut phase: Option<Phase> = None;
        let mut need_release = false;
        let mut shake = Shake::default();
        loop {
            let down = primary_down();
            let Some(current) = phase else {
                if down {
                    let p = cursor();
                    phase = Some(Phase::Pressed { x: p.x, y: p.y });
                    need_release = false;
                    shake = Shake::default();
                } else {
                    std::thread::sleep(Duration::from_millis(33));
                }
                continue;
            };
            std::thread::sleep(Duration::from_millis(8));
            if !down {
                match current {
                    Phase::Probing { .. } => main(Box::new(hide_probe)),
                    Phase::Shown => signal(DragSignal::Released),
                    _ => {}
                }
                phase = None;
                continue;
            }
            let WatchSettings { key, shake: shake_on, paused } = settings();
            if paused {
                if let Phase::Probing { .. } = current {
                    main(Box::new(hide_probe));
                    phase = Some(Phase::Dragging);
                }
                continue;
            }
            let p = cursor();
            let key_down = key_down(trigger_key(&key));
            if !key_down {
                need_release = false;
            }
            match current {
                Phase::Pressed { x, y } => {
                    let thr = unsafe { GetSystemMetrics(SM_CXDRAG) }.max(4) as f64;
                    if (p.x - x).abs() > thr || (p.y - y).abs() > thr {
                        phase = Some(Phase::Dragging);
                        shake = Shake::default();
                    }
                }
                Phase::Dragging => {
                    let trigger = if key_down && !need_release {
                        Some(Trigger::Key)
                    } else if shake_on && shake.feed(p.x, Instant::now(), scale_at(p)) {
                        Some(Trigger::Shake)
                    } else {
                        None
                    };
                    if let Some(trigger) = trigger {
                        *PROBE.lock().unwrap() = Probe::default();
                        phase = Some(Phase::Probing { since: Instant::now(), at: p, trigger });
                        main(Box::new(move || show_probe(p)));
                    }
                }
                Phase::Probing { since, at, trigger } => {
                    let (files, rejected) = {
                        let mut pr = PROBE.lock().unwrap();
                        (pr.files.take(), pr.rejected)
                    };
                    if let Some(files) = files {
                        eprintln!("[orbo] Sonde: Datei-Drag nach {:.0} ms erkannt", since.elapsed().as_secs_f64() * 1000.0);
                        phase = Some(Phase::Shown);
                        // The ring is shown first (queued by `signal`), then the probe leaves.
                        signal(DragSignal::Triggered { at, trigger, files });
                        main(Box::new(hide_probe));
                    } else if rejected || since.elapsed() > Duration::from_millis(1500) {
                        eprintln!(
                            "[orbo] Sonde: {}",
                            if rejected { "Drag ohne Dateien, kein Ring" } else { "kein Datei-Drag erkannt (Zeitlimit)" }
                        );
                        main(Box::new(hide_probe));
                        phase = Some(Phase::Dragging);
                        need_release = key_down;
                        shake = Shake::default();
                    }
                }
                Phase::Shown => {
                    let center = *RING_CENTER.lock().unwrap();
                    if let Some(c) = center {
                        let limit = CANCEL_DISTANCE * scale_at(c);
                        if ((p.x - c.x).powi(2) + (p.y - c.y).powi(2)).sqrt() > limit {
                            phase = Some(Phase::Dragging);
                            need_release = key_down;
                            shake = Shake::default();
                            signal(DragSignal::Cancelled);
                        }
                    }
                }
            }
        }
    });
}

// ---- Log file ----

/// Sends everything written to stdout/stderr (eprintln) to %APPDATA%\Orbo\orbo.log, and to the console
/// when Orbo was started from one. A release build has no console of its own.
pub fn init_logging(dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("orbo.log");
    let _ = std::fs::rename(&path, dir.join("orbo.old.log"));
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else { return };
    let Ok((mut reader, writer)) = std::io::pipe() else { return };
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{AttachConsole, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
    let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_ok();
    let mut console = if attached { std::fs::OpenOptions::new().write(true).open("CONOUT$").ok() } else { None };
    if console.is_some() {
        unsafe {
            let _ = windows::Win32::System::Console::SetConsoleOutputCP(65001);
        }
    }
    let raw = HANDLE(writer.as_raw_handle());
    unsafe {
        let _ = SetStdHandle(STD_ERROR_HANDLE, raw);
        let _ = SetStdHandle(STD_OUTPUT_HANDLE, raw);
    }
    std::mem::forget(writer);
    let _ = writeln!(file, "──── Orbo {} gestartet ────", env!("CARGO_PKG_VERSION"));
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut written: u64 = 0;
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            // Keep the file small if logging runs for days.
            if written > 20 * 1024 * 1024 {
                if let Ok(f) = std::fs::File::create(&path) {
                    file = f;
                    written = 0;
                }
            }
            let _ = file.write_all(&buf[..n]);
            written += n as u64;
            if let Some(c) = console.as_mut() {
                let _ = c.write_all(&buf[..n]);
            }
        }
    });
}
