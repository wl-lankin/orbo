use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};

use tiny_skia::Pixmap;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::*;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::actions::{self, Action};
use crate::render::{self, Fonts, RingModel, OUTER, PANEL};
use crate::{config, log, now_ms};

/// Ring center in screen pixels, read by the self-test driver thread.
pub static RING_X: AtomicI32 = AtomicI32::new(0);
pub static RING_Y: AtomicI32 = AtomicI32::new(0);
pub static RING_SCALE_PERMILLE: AtomicI32 = AtomicI32::new(1000);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Idle,
    Pressed { x: i32, y: i32 },
    Dragging,
    Probe { since: f64 },
    Shown,
}

struct App {
    hwnd: HWND,
    phase: Phase,
    need_release: bool,
    shake: Shake,
    files: Vec<PathBuf>,
    actions: Vec<Action>,
    hot: Option<i32>,
    cx: i32,
    cy: i32,
    size: i32,
    scale: f32,
    trigger_t: f64,
    anim_t0: f64,
    animating: bool,
    first_frame_logged: bool,
    cache: Option<Pixmap>,
    fonts: Fonts,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| match a.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => {
            log("WARNUNG: Zustand gerade belegt (Wiedereintritt), Aufruf verworfen");
            None
        }
    })
}

#[derive(Default)]
struct Shake {
    last_x: Option<i32>,
    dir: i32,
    turn_x: i32,
    turns: Vec<f64>,
}

impl Shake {
    fn reset(&mut self) {
        *self = Shake::default();
    }

    fn feed(&mut self, x: i32, t: f64, scale: f32) -> bool {
        let Some(lx) = self.last_x else {
            self.last_x = Some(x);
            self.turn_x = x;
            return false;
        };
        let dx = x - lx;
        self.last_x = Some(x);
        if dx.abs() < 2 {
            return false;
        }
        let d = dx.signum();
        if self.dir != 0 && d != self.dir {
            if ((x - self.turn_x).abs() as f32) > 18.0 * scale {
                self.turns.push(t);
            }
            self.turn_x = x;
        }
        self.dir = d;
        self.turns.retain(|&s| t - s < 700.0);
        if self.turns.len() >= 3 {
            self.turns.clear();
            return true;
        }
        false
    }
}

fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}

fn primary_down() -> bool {
    // GetAsyncKeyState reports physical buttons, so swapped buttons need the right one.
    let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
    key_down(if swapped { VK_RBUTTON } else { VK_LBUTTON })
}

fn cursor() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

fn monitor_for(p: POINT) -> (RECT, f32) {
    unsafe {
        let mon = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        (mi.rcMonitor, dx as f32 / 96.0)
    }
}

impl App {
    fn tick(&mut self) {
        let down = primary_down();
        let p = cursor();
        let cfg = config();
        let key = key_down(cfg.key);
        if !key {
            self.need_release = false;
        }
        let t = now_ms();

        match self.phase {
            Phase::Idle => {
                if down {
                    self.phase = Phase::Pressed { x: p.x, y: p.y };
                }
            }
            Phase::Pressed { x, y } => {
                let thr = unsafe { GetSystemMetrics(SM_CXDRAG) }.max(4);
                if !down {
                    self.phase = Phase::Idle;
                } else if (p.x - x).abs() > thr || (p.y - y).abs() > thr {
                    self.phase = Phase::Dragging;
                    self.shake.reset();
                }
            }
            _ if !down => {
                if let Phase::Probe { .. } = self.phase {
                    log("Losgelassen, bevor ein Datei-Drag erkannt wurde (kein DragEnter)");
                }
                self.hide();
                self.phase = Phase::Idle;
            }
            Phase::Dragging => {
                let (_, scale) = monitor_for(p);
                let trigger = if key && !self.need_release {
                    Some(cfg.key_name)
                } else if cfg.shake && self.shake.feed(p.x, t, scale) {
                    Some("Schütteln")
                } else {
                    None
                };
                if let Some(name) = trigger {
                    self.show_probe(p, name);
                }
            }
            Phase::Probe { since } => {
                if t - since > 2500.0 {
                    log("Kein DragEnter innerhalb 2,5 s, vermutlich kein Datei-Drag. Sonde entfernt.");
                    self.hide();
                    self.phase = Phase::Dragging;
                    self.need_release = key;
                }
            }
            Phase::Shown => {
                let d = (((p.x - self.cx) as f32).powi(2) + ((p.y - self.cy) as f32).powi(2)).sqrt();
                if d > 300.0 * self.scale {
                    log("Ring abgebrochen (weggezogen), Drag läuft normal weiter");
                    self.hide();
                    self.phase = Phase::Dragging;
                    self.need_release = key;
                    self.shake.reset();
                }
            }
        }
        if self.animating {
            self.frame();
        }
    }

    fn show_probe(&mut self, p: POINT, trigger: &str) {
        let (mon, scale) = monitor_for(p);
        let size = (PANEL * scale).round() as i32;
        self.scale = scale;
        self.size = size;
        self.cx = p.x.clamp(mon.left + size / 2, mon.right - size / 2);
        self.cy = p.y.clamp(mon.top + size / 2, mon.bottom - size / 2);
        self.trigger_t = now_ms();
        self.files.clear();
        self.actions.clear();
        self.hot = None;
        self.cache = None;
        self.phase = Phase::Probe { since: self.trigger_t };
        let pm = render::probe(size as u32, scale);
        self.present(&pm);
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
        log(&format!(
            "Auslöser: {trigger} · unsichtbare Sonde an ({}, {}), Skalierung {:.0} %, warte auf DragEnter",
            self.cx,
            self.cy,
            scale * 100.0
        ));
    }

    fn hide(&mut self) {
        self.animating = false;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<i32> {
        if self.actions.is_empty() {
            return None;
        }
        let (dx, dy) = ((x - self.cx) as f32, (y - self.cy) as f32);
        let d = (dx * dx + dy * dy).sqrt();
        if d < render::INNER * self.scale {
            return Some(-1);
        }
        if d > (OUTER + 14.0) * self.scale {
            return None;
        }
        let mut a = dx.atan2(-dy);
        if a < 0.0 {
            a += std::f32::consts::TAU;
        }
        let n = self.actions.len();
        let step = std::f32::consts::TAU / n as f32;
        Some(((a / step).round() as usize % n) as i32)
    }

    fn set_hot(&mut self, h: Option<i32>) {
        if h != self.hot {
            let label = match h {
                Some(-1) => "Mitte".to_string(),
                Some(i) => format!("Feld {}", self.actions[i as usize].title),
                None => "außerhalb".to_string(),
            };
            log(&format!("Zeiger: {label}"));
            self.hot = h;
            self.cache = None;
            if !self.animating {
                self.frame();
            }
        }
    }

    fn frame(&mut self) {
        if self.phase != Phase::Shown {
            return;
        }
        if self.cache.is_none() {
            let kind = actions::kind_name(&self.files);
            let model = RingModel { actions: &self.actions, hot: self.hot, kind, count: self.files.len(), style: config().style };
            self.cache = Some(render::ring(self.size as u32, self.scale, &model, &self.fonts));
        }
        let t = ((now_ms() - self.anim_t0) / 180.0) as f32;
        let pm = if self.animating && t < 1.0 {
            render::compose(self.cache.as_ref().unwrap(), t)
        } else {
            self.animating = false;
            self.cache.clone().unwrap()
        };
        self.present(&pm);
        if !self.first_frame_logged {
            self.first_frame_logged = true;
            log(&format!("Ring sichtbar {:.1} ms nach Auslöser", now_ms() - self.trigger_t));
        }
    }

    fn present(&self, pm: &Pixmap) {
        let (w, h) = (pm.width() as i32, pm.height() as i32);
        unsafe {
            let screen = GetDC(HWND::default());
            let mem = CreateCompatibleDC(screen);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let Ok(dib) = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, HANDLE::default(), 0) else {
                log("FEHLER: CreateDIBSection");
                return;
            };
            let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
            for (d, s) in dst.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = s[3];
            }
            let old = SelectObject(mem, dib);
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let pos = POINT { x: self.cx - w / 2, y: self.cy - h / 2 };
            let size = SIZE { cx: w, cy: h };
            let src = POINT { x: 0, y: 0 };
            if UpdateLayeredWindow(self.hwnd, screen, Some(&pos), Some(&size), mem, Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA)
                .is_err()
            {
                log("FEHLER: UpdateLayeredWindow");
            }
            SelectObject(mem, old);
            let _ = DeleteObject(dib);
            let _ = DeleteDC(mem);
            ReleaseDC(HWND::default(), screen);
        }
        RING_X.store(self.cx, Ordering::Relaxed);
        RING_Y.store(self.cy, Ordering::Relaxed);
        RING_SCALE_PERMILLE.store((self.scale * 1000.0) as i32, Ordering::Relaxed);
    }
}

// ---- Called by the drop target (same UI thread) ----

pub fn on_drag_enter(files: Vec<PathBuf>, pt: (i32, i32)) -> DROPEFFECT {
    with_app(|a| {
        if !matches!(a.phase, Phase::Probe { .. } | Phase::Shown) {
            return DROPEFFECT_NONE;
        }
        if files.is_empty() {
            log("DragEnter ohne Dateien (kein CF_HDROP), Ring nicht gezeigt");
            a.hide();
            a.phase = Phase::Dragging;
            a.need_release = true;
            return DROPEFFECT_NONE;
        }
        if a.phase != Phase::Shown || files != a.files {
            let waited = now_ms() - a.trigger_t;
            let names: Vec<String> = files.iter().filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
            log(&format!("DragEnter nach {waited:.1} ms · {} Datei(en): {}", files.len(), names.join(", ")));
            a.actions = actions::for_files(&files);
            a.files = files;
            a.phase = Phase::Shown;
            a.cache = None;
            a.hot = None;
            a.first_frame_logged = false;
            a.animating = true;
            a.anim_t0 = now_ms();
        }
        let h = a.hit(pt.0, pt.1);
        a.set_hot(h);
        a.frame();
        if h.is_some() { DROPEFFECT_COPY } else { DROPEFFECT_NONE }
    })
    .unwrap_or(DROPEFFECT_NONE)
}

pub fn on_drag_over(pt: (i32, i32)) -> DROPEFFECT {
    with_app(|a| {
        if a.phase != Phase::Shown {
            return DROPEFFECT_NONE;
        }
        let h = a.hit(pt.0, pt.1);
        a.set_hot(h);
        if h.is_some() { DROPEFFECT_COPY } else { DROPEFFECT_NONE }
    })
    .unwrap_or(DROPEFFECT_NONE)
}

pub fn on_drag_leave() {
    log("DragLeave");
    with_app(|a| a.set_hot(None));
}

pub fn on_drop(files: Vec<PathBuf>, pt: (i32, i32)) -> DROPEFFECT {
    with_app(|a| {
        if a.phase != Phase::Shown {
            return DROPEFFECT_NONE;
        }
        let Some(h) = a.hit(pt.0, pt.1) else {
            log("Drop außerhalb der Felder, ignoriert");
            return DROPEFFECT_NONE;
        };
        let action = a.actions[if h < 0 { 0 } else { h as usize }];
        let files = if files.is_empty() { a.files.clone() } else { files };
        log(&format!("Drop auf {} {} · {} Datei(en)", if h < 0 { "Mitte" } else { "Feld" }, action.title, files.len()));
        a.hide();
        a.phase = Phase::Idle;
        actions::run(action, files, |ok| log(if ok { "Umwandlung ✓" } else { "Umwandlung ✗" }));
        DROPEFFECT_COPY
    })
    .unwrap_or(DROPEFFECT_NONE)
}

// ---- Window and message loop ----

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            with_app(|a| a.tick());
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

pub fn run(selftest: bool) {
    unsafe {
        if OleInitialize(None).is_err() {
            log("FEHLER: OleInitialize");
            return;
        }
        let Some(fonts) = Fonts::load() else {
            log("FEHLER: Systemschrift (segoeui.ttf) nicht gefunden");
            return;
        };
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap();
        let class = w!("OrbitRing");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(HINSTANCE::default(), IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("Orbit"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            HWND::default(),
            HMENU::default(),
            hinstance,
            None,
        )
        .expect("CreateWindowExW");

        let target: IDropTarget = crate::droptarget::DropTarget::new(hwnd).into();
        if let Err(e) = RegisterDragDrop(hwnd, &target) {
            log(&format!("FEHLER: RegisterDragDrop {e}"));
            return;
        }

        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                hwnd,
                phase: Phase::Idle,
                need_release: false,
                shake: Shake::default(),
                files: Vec::new(),
                actions: Vec::new(),
                hot: None,
                cx: 0,
                cy: 0,
                size: 0,
                scale: 1.0,
                trigger_t: 0.0,
                anim_t0: 0.0,
                animating: false,
                first_frame_logged: false,
                cache: None,
                fonts,
            })
        });

        SetTimer(hwnd, 1, 8, None);
        if selftest {
            crate::selftest::start();
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = RevokeDragDrop(hwnd);
        drop(target);
        OleUninitialize();
    }
}
