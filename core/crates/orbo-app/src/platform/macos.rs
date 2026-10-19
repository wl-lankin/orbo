//! macOS: system-wide drag detection without special permissions, and native window tweaks.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::AnyClass;
use objc2::{msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSEvent, NSEventModifierFlags, NSPasteboard, NSPasteboardNameDrag, NSPasteboardTypeFileURL, NSScreen, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindow,
    NSWindowCollectionBehavior, NSWindowOrderingMode,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSURL};

use super::{DragSignal, Trigger, CANCEL_DISTANCE, RING_CENTER};

/// Screen point in AppKit coordinates (origin bottom-left of the primary screen, in points).
#[derive(Debug, Clone, Copy, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

// ---- Drag watcher ----

fn trigger_flag(key: &str) -> NSEventModifierFlags {
    match key {
        "control" => NSEventModifierFlags::Control,
        "shift" => NSEventModifierFlags::Shift,
        "command" => NSEventModifierFlags::Command,
        "fn" => NSEventModifierFlags::Function,
        _ => NSEventModifierFlags::Option,
    }
}

fn drag_pasteboard() -> Retained<NSPasteboard> {
    NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameDrag })
}

fn file_urls(pb: &NSPasteboard) -> Vec<PathBuf> {
    let Some(items) = pb.pasteboardItems() else { return Vec::new() };
    let mut out = Vec::new();
    for item in items.iter() {
        if let Some(s) = item.stringForType(unsafe { NSPasteboardTypeFileURL }) {
            if let Some(url) = NSURL::URLWithString(&s) {
                if let Some(p) = url.path() {
                    out.push(PathBuf::from(p.to_string()));
                }
            }
        }
    }
    out
}

#[derive(Default)]
struct Shake {
    last_x: Option<f64>,
    dir: f64,
    turn_x: f64,
    turns: Vec<Instant>,
}

impl Shake {
    fn feed(&mut self, x: f64, now: Instant) -> bool {
        let Some(lx) = self.last_x else {
            self.last_x = Some(x);
            self.turn_x = x;
            return false;
        };
        let dx = x - lx;
        self.last_x = Some(x);
        if dx.abs() < 1.5 {
            return false;
        }
        let d = dx.signum();
        if self.dir != 0.0 && d != self.dir {
            if (x - self.turn_x).abs() > 18.0 {
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

/// Runs forever on its own thread. Polls the primary button (30 Hz idle, 120 Hz while pressed) and
/// the drag pasteboard's change count, which every drag source bumps when a drag starts.
/// Screen frames (x, y, width, height in AppKit points), refreshed on the main thread.
static SCREENS: std::sync::Mutex<Vec<(f64, f64, f64, f64)>> = std::sync::Mutex::new(Vec::new());

/// Must run on the main thread.
pub fn refresh_screens() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let screens = NSScreen::screens(mtm);
    let frames: Vec<_> = screens
        .iter()
        .map(|s| {
            let f = s.frame();
            (f.origin.x, f.origin.y, f.size.width, f.size.height)
        })
        .collect();
    *SCREENS.lock().unwrap() = frames;
}

fn screen_at(p: Point) -> Option<(f64, f64, f64, f64)> {
    SCREENS.lock().unwrap().iter().copied().find(|(x, y, w, h)| p.x >= *x && p.x <= x + w && p.y >= *y && p.y <= y + h)
}

pub struct WatchSettings {
    pub key: String,
    pub shake: bool,
    pub paused: bool,
    pub notch: bool,
}

pub fn watch(settings: impl Fn() -> WatchSettings + Send + 'static, mut signal: impl FnMut(DragSignal) + Send + 'static) {
    std::thread::spawn(move || {
        let pb = drag_pasteboard();
        let mut baseline = pb.changeCount();
        let mut pressed = false;
        let mut file_drag = false;
        let mut other_drag = false;
        let mut triggered = false;
        let mut need_release = false;
        let mut shake = Shake::default();
        let mut notch_open = false;
        loop {
            let down = NSEvent::pressedMouseButtons() & 1 != 0;
            if !pressed {
                if down {
                    pressed = true;
                    file_drag = false;
                    other_drag = false;
                    triggered = false;
                    need_release = false;
                    shake = Shake::default();
                } else {
                    // Only move the baseline while the button is up, so a drag that starts between two idle polls is still seen.
                    baseline = pb.changeCount();
                    std::thread::sleep(Duration::from_millis(33));
                    continue;
                }
            }
            std::thread::sleep(Duration::from_millis(8));
            if !down {
                pressed = false;
                baseline = pb.changeCount();
                if triggered || notch_open {
                    notch_open = false;
                    signal(DragSignal::Released);
                }
                continue;
            }
            if other_drag {
                continue;
            }
            let WatchSettings { key, shake: shake_on, paused, notch } = settings();
            if paused {
                continue;
            }
            let loc = NSEvent::mouseLocation();
            let p = Point { x: loc.x, y: loc.y };
            if !file_drag {
                if pb.changeCount() == baseline {
                    continue;
                }
                let Some(types) = pb.types() else { continue };
                // The source clears the pasteboard (bumping changeCount) before writing; empty means "not yet".
                if types.count() == 0 {
                    continue;
                }
                if !types.containsObject(unsafe { NSPasteboardTypeFileURL }) {
                    other_drag = true;
                    continue;
                }
                file_drag = true;
            }
            if notch && !triggered {
                if let Some((sx, sy, sw, sh)) = screen_at(p) {
                    let from_top = sy + sh - p.y;
                    let near_center = (p.x - (sx + sw / 2.0)).abs() < 260.0;
                    // Open before the very top edge, where macOS would show the Mission Control spaces bar.
                    if !notch_open && from_top < 60.0 && near_center {
                        notch_open = true;
                        signal(DragSignal::NotchEnter { at: p, files: file_urls(&pb) });
                    } else if notch_open && from_top > 200.0 {
                        notch_open = false;
                        signal(DragSignal::NotchLeave);
                    }
                }
                if notch_open {
                    continue;
                }
            }
            let key_down = NSEvent::modifierFlags_class().contains(trigger_flag(&key));
            if !key_down {
                need_release = false;
            }
            if triggered {
                let center = *RING_CENTER.lock().unwrap();
                if let Some(c) = center {
                    if ((p.x - c.x).powi(2) + (p.y - c.y).powi(2)).sqrt() > CANCEL_DISTANCE {
                        triggered = false;
                        need_release = key_down;
                        shake = Shake::default();
                        signal(DragSignal::Cancelled);
                        continue;
                    }
                }
                continue;
            }
            let trigger = if key_down && !need_release {
                Some(Trigger::Key)
            } else if shake_on && shake.feed(p.x, Instant::now()) {
                Some(Trigger::Shake)
            } else {
                None
            };
            if let Some(t) = trigger {
                triggered = true;
                signal(DragSignal::Triggered { at: p, trigger: t, files: file_urls(&pb) });
            }
        }
    });
}

// ---- Windows ----

/// # Safety
/// `ptr` must be the NSWindow pointer Tauri hands out, and this must run on the main thread.
unsafe fn ns_window<'a>(ptr: *mut std::ffi::c_void) -> &'a NSWindow {
    &*(ptr as *const NSWindow)
}

/// Floating, non-activating overlay on every Space, also above full-screen apps.
pub fn make_overlay(ptr: *mut std::ffi::c_void) {
    let w = unsafe { ns_window(ptr) };
    // NSPopUpMenuWindowLevel
    w.setLevel(101);
    w.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    w.setHasShadow(false);
}

/// Places a window of `size` points centered on `at`, clamped to the screen containing `at`.
/// Returns the window center actually used.
pub fn place_centered(ptr: *mut std::ffi::c_void, at: Point, size: f64) -> Point {
    let mtm = MainThreadMarker::new().expect("main thread");
    let w = unsafe { ns_window(ptr) };
    let screens = NSScreen::screens(mtm);
    let mut frame = None;
    for s in screens.iter() {
        let f = s.frame();
        if at.x >= f.origin.x && at.x <= f.origin.x + f.size.width && at.y >= f.origin.y && at.y <= f.origin.y + f.size.height {
            frame = Some(f);
        }
    }
    let f = frame.or_else(|| screens.iter().next().map(|s| s.frame())).unwrap_or(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0)));
    let half = size / 2.0;
    let cx = at.x.clamp(f.origin.x + half, f.origin.x + f.size.width - half);
    let cy = at.y.clamp(f.origin.y + half, f.origin.y + f.size.height - half);
    w.setFrame_display(NSRect::new(NSPoint::new(cx - half, cy - half), NSSize::new(size, size)), true);
    Point { x: cx, y: cy }
}

/// Places a window flush with the top edge of the screen containing `at`, horizontally centered.
pub fn place_top_center(ptr: *mut std::ffi::c_void, at: Point, width: f64, height: f64) {
    let w = unsafe { ns_window(ptr) };
    let (sx, sy, sw, sh) = screen_at(at).unwrap_or((0.0, 0.0, 1440.0, 900.0));
    w.setFrame_display(NSRect::new(NSPoint::new(sx + (sw - width) / 2.0, sy + sh - height), NSSize::new(width, height)), true);
}

/// Places a window at the bottom center of the main screen's visible area (above the Dock).
pub fn place_bottom_center(ptr: *mut std::ffi::c_void, width: f64, height: f64) {
    let mtm = MainThreadMarker::new().expect("main thread");
    let w = unsafe { ns_window(ptr) };
    let Some(screen) = NSScreen::mainScreen(mtm) else { return };
    let v = screen.visibleFrame();
    let x = v.origin.x + (v.size.width - width) / 2.0;
    w.setFrame_display(NSRect::new(NSPoint::new(x, v.origin.y + 16.0), NSSize::new(width, height)), true);
}

pub fn show_without_activating(ptr: *mut std::ffi::c_void) {
    unsafe { ns_window(ptr) }.orderFrontRegardless();
}

pub fn hide(ptr: *mut std::ffi::c_void) {
    unsafe { ns_window(ptr) }.orderOut(None);
}

thread_local! {
    static RING_BACKDROP: RefCell<Option<Retained<NSView>>> = const { RefCell::new(None) };
}

fn set_layer_radius(view: &NSView, radius: f64) {
    view.setWantsLayer(true);
    unsafe {
        let layer: *mut objc2::runtime::AnyObject = msg_send![view, layer];
        if !layer.is_null() {
            let _: () = msg_send![layer, setCornerRadius: radius];
            let _: () = msg_send![layer, setMasksToBounds: true];
        }
    }
}

/// Puts a round native backdrop (HUD vibrancy or Liquid Glass) of `radius` behind the web content,
/// centered in the window. Replaces a previous one.
pub fn set_ring_backdrop(ptr: *mut std::ffi::c_void, style: &str, radius: f64) {
    let mtm = MainThreadMarker::new().expect("main thread");
    let w = unsafe { ns_window(ptr) };
    let Some(content) = w.contentView() else { return };
    RING_BACKDROP.with(|b| {
        if let Some(old) = b.borrow_mut().take() {
            old.removeFromSuperview();
        }
    });
    let bounds = content.bounds();
    let frame = NSRect::new(
        NSPoint::new(bounds.size.width / 2.0 - radius, bounds.size.height / 2.0 - radius),
        NSSize::new(radius * 2.0, radius * 2.0),
    );

    let glass_class = if style == "glass" { AnyClass::get(c"NSGlassEffectView") } else { None };
    let view: Retained<NSView> = if let Some(cls) = glass_class {
        unsafe {
            let obj: *mut objc2::runtime::AnyObject = msg_send![cls, alloc];
            let obj: *mut objc2::runtime::AnyObject = msg_send![obj, initWithFrame: frame];
            let _: () = msg_send![obj, setCornerRadius: radius];
            Retained::from_raw(obj as *mut NSView).expect("NSGlassEffectView")
        }
    } else {
        let v = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
        v.setMaterial(if style == "glass" { NSVisualEffectMaterial::Popover } else { NSVisualEffectMaterial::HUDWindow });
        v.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        v.setState(NSVisualEffectState::Active);
        set_layer_radius(&v, radius);
        Retained::into_super(v)
    };

    // The web view must stay on top of the backdrop. Tauri may install it as the content view itself.
    let content_is_webview = content.class().name().to_string_lossy().contains("WebView");
    if content_is_webview {
        let container = NSView::initWithFrame(NSView::alloc(mtm), content.frame());
        w.setContentView(Some(&container));
        container.addSubview(&view);
        content.setFrame(container.bounds());
        container.addSubview(&content);
    } else {
        content.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, None);
    }
    RING_BACKDROP.with(|b| *b.borrow_mut() = Some(view));
}

/// Shows the dock-less app's window in front (settings, dialog).
pub fn activate_app() {
    if let Some(mtm) = MainThreadMarker::new() {
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
}

#[allow(dead_code)]
fn _assert_main_thread_only() {
    fn f<T: MainThreadOnly>() {}
    f::<NSWindow>();
}
