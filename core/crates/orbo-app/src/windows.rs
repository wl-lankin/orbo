use std::time::Duration;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::platform::{Point, RING_CENTER};
use crate::state::AppState;

pub const RING_SIZE: f64 = 520.0;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const RING_BACKDROP_RADIUS: f64 = 180.0;
const TOAST_W: f64 = 460.0;
const TOAST_H: f64 = 260.0;

fn overlay(app: &AppHandle, label: &str, page: &str, w: f64, h: f64) -> tauri::Result<WebviewWindow> {
    #[allow(unused_mut)]
    let mut b = WebviewWindowBuilder::new(app, label, WebviewUrl::App(page.into()))
        .title("Orbo")
        .inner_size(w, h)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .visible_on_all_workspaces(true)
        .accept_first_mouse(true);
    #[cfg(target_os = "windows")]
    {
        // WebView2 paints white until the page is loaded; a transparent default avoids the flash.
        b = b.focusable(false).background_color(tauri::window::Color(0, 0, 0, 0)).initialization_script(WIN_OVERLAY_SCRIPT);
    }
    b.build()
}

/// Windows has no native round backdrop behind the ring (macOS: vibrancy or Liquid Glass), so the page draws one.
#[cfg(target_os = "windows")]
const WIN_OVERLAY_SCRIPT: &str = r#"(() => {
  document.documentElement.classList.add('os-win');
  const css = `
    .os-win #stage::before { content: ""; position: absolute; left: 80px; top: 80px; width: 360px; height: 360px;
      border-radius: 50%; background: rgba(20, 22, 36, .84); box-shadow: 0 0 0 1px rgba(255, 255, 255, .08), 0 12px 40px rgba(0, 0, 0, .35); }
    .os-win body.style-glass #stage::before { background: rgba(70, 74, 96, .62); box-shadow: 0 0 0 1px rgba(255, 255, 255, .22), 0 12px 40px rgba(0, 0, 0, .25); }`;
  const add = () => { const st = document.createElement('style'); st.textContent = css; document.head.appendChild(st); };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', add); else add();
})();"#;

/// Windows-only panel tweaks; key names and wording come from ui/platform.js.
#[cfg(target_os = "windows")]
const WIN_PANEL_SCRIPT: &str = r#"(() => {
  document.documentElement.classList.add('os-win');
  const fix = () => {
    const st = document.createElement('style');
    st.textContent = `.os-win #privacyBtn, .os-win #grantNote { display: none !important; }
      .os-win.solid-bg, .os-win.solid-bg body { background: #1b1d2b !important; }`;
    document.head.appendChild(st);
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', fix); else fix();
})();"#;

pub fn create_overlays(app: &AppHandle) -> tauri::Result<()> {
    let ring = overlay(app, "ring", "ring.html", RING_SIZE, RING_SIZE)?;
    let toast = overlay(app, "toast", "toast.html", TOAST_W, TOAST_H)?;
    let notch = overlay(app, "notch", "notch.html", NOTCH_W, NOTCH_H)?;
    #[cfg(target_os = "macos")]
    {
        use crate::platform::macos;
        let style = app.state::<AppState>().settings.read().unwrap().style.clone();
        let (rp, tp) = (ring.ns_window()?, toast.ns_window()?);
        macos::make_overlay(rp);
        macos::make_overlay(tp);
        macos::make_overlay(notch.ns_window()?);
        macos::set_ring_backdrop(rp, &style, RING_BACKDROP_RADIUS);
        // Ordering the hidden window in once moves window-server setup out of the first real trigger.
        macos::place_centered(rp, Point { x: -5000.0, y: -5000.0 }, RING_SIZE);
        macos::show_without_activating(rp);
        macos::hide(rp);
    }
    #[cfg(target_os = "windows")]
    {
        use crate::platform::windows as win;
        for w in [&ring, &toast, &notch] {
            win::make_overlay(w.hwnd()?.0 as isize);
        }
        // Showing the hidden ring once off screen moves WebView2 setup out of the first real trigger.
        let rh = ring.hwnd()?.0 as isize;
        win::park(rh);
        win::show_without_activating(rh);
        win::hide(rh);
    }
    let _ = (ring, toast, notch);
    Ok(())
}

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn apply_ring_style(app: &AppHandle, style: &str) {
    #[cfg(target_os = "macos")]
    if let Some(w) = app.get_webview_window("ring") {
        let style = style.to_string();
        let w2 = w.clone();
        let _ = w.run_on_main_thread(move || {
            if let Ok(p) = w2.ns_window() {
                crate::platform::macos::set_ring_backdrop(p, &style, RING_BACKDROP_RADIUS);
            }
        });
    }
}

/// True while a ring opened by a context menu entry (clicked, not dragged onto) is up.
pub static CLICK_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Called on the main thread when the user triggers the ring during a drag.
pub fn open_ring(app: &AppHandle, at: Point, files: Vec<std::path::PathBuf>, started: std::time::Instant) {
    open_ring_mode(app, at, files, started, false);
}

/// Called on the main thread. `click` opens the ring for a context menu request: no drag is running,
/// the pointer and clicks come from a polling thread instead of drag events.
pub fn open_ring_mode(app: &AppHandle, at: Point, files: Vec<std::path::PathBuf>, started: std::time::Instant, click: bool) {
    let Some(w) = app.get_webview_window("ring") else { return };
    let mode = if click { "click" } else { "drag" };
    CLICK_MODE.store(click, std::sync::atomic::Ordering::SeqCst);
    if click {
        let _ = w.set_ignore_cursor_events(false);
    }
    #[cfg(target_os = "macos")]
    {
        use crate::platform::macos;
        let Ok(p) = w.ns_window() else { return };
        let center = macos::place_centered(p, at, RING_SIZE);
        *RING_CENTER.lock().unwrap() = Some(center);
        macos::show_without_activating(p);
    }
    #[cfg(target_os = "windows")]
    {
        use crate::platform::windows as win;
        let Ok(h) = w.hwnd() else { return };
        let h = h.0 as isize;
        let center = win::place_centered(h, at, RING_SIZE);
        *RING_CENTER.lock().unwrap() = Some(center);
        win::show_without_activating(h);
    }
    let settings = app.state::<AppState>().settings.read().unwrap().clone();
    let shown_ms = started.elapsed().as_secs_f64() * 1000.0;
    eprintln!("[orbo] Ring sichtbar nach {shown_ms:.1} ms");
    if click {
        crate::invoke::start_click_loop(app);
    }
    let _ = w.emit("ring:open", serde_json::json!({
        "mode": mode,
        "files": files,
        "suggestions": serde_json::Value::Null,
        "style": settings.style,
        "sound": settings.sound,
        "reduce_motion": settings.reduce_motion,
    }));
    // Suggestions need file analysis (ffprobe for media); send them as soon as they are ready.
    if !files.is_empty() {
        let w2 = w.clone();
        let s2 = settings.clone();
        std::thread::spawn(move || {
            if let Ok(v) = crate::commands::suggest_value(&files) {
                let _ = w2.emit("ring:open", serde_json::json!({
                    "mode": mode, "files": files, "suggestions": v, "style": s2.style, "sound": s2.sound, "reduce_motion": s2.reduce_motion,
                }));
            }
        });
    }
}

pub fn close_ring(app: &AppHandle, reason: &str) {
    use std::sync::atomic::Ordering::SeqCst;
    // The drag watcher keeps reporting release and cancel; they mean nothing for a clicked ring.
    if CLICK_MODE.load(SeqCst) {
        if reason != "dismissed" {
            return;
        }
        CLICK_MODE.store(false, SeqCst);
    }
    if let Some(w) = app.get_webview_window("ring") {
        let _ = w.emit("ring:close", serde_json::json!({ "reason": reason }));
    }
    // After a release the drop event may still be on its way to the webview.
    hide_ring(app, if reason == "released" { 450 } else { 220 });
}

pub fn hide_ring(app: &AppHandle, delay_ms: u64) {
    CLICK_MODE.store(false, std::sync::atomic::Ordering::SeqCst);
    *RING_CENTER.lock().unwrap() = None;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(delay_ms));
        let Some(w) = app.get_webview_window("ring") else { return };
        let w2 = w.clone();
        let _ = w.run_on_main_thread(move || {
            // A new trigger may have reopened the ring meanwhile.
            if RING_CENTER.lock().unwrap().is_some() {
                return;
            }
            #[cfg(target_os = "macos")]
            if let Ok(p) = w2.ns_window() {
                crate::platform::macos::hide(p);
            }
            #[cfg(target_os = "windows")]
            if let Ok(h) = w2.hwnd() {
                crate::platform::windows::hide(h.0 as isize);
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let _ = w2.hide();
        });
    });
}

pub fn show_toast(app: &AppHandle) {
    let Some(w) = app.get_webview_window("toast") else { return };
    let w2 = w.clone();
    let _ = w.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        if let Ok(p) = w2.ns_window() {
            crate::platform::macos::place_bottom_center(p, TOAST_W, TOAST_H);
            crate::platform::macos::show_without_activating(p);
        }
        #[cfg(target_os = "windows")]
        if let Ok(h) = w2.hwnd() {
            crate::platform::windows::place_bottom_center(h.0 as isize, TOAST_W, TOAST_H);
            crate::platform::windows::show_without_activating(h.0 as isize);
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = w2.show();
    });
}

pub fn hide_toast(app: &AppHandle) {
    let Some(w) = app.get_webview_window("toast") else { return };
    let w2 = w.clone();
    let _ = w.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        if let Ok(p) = w2.ns_window() {
            crate::platform::macos::hide(p);
        }
        #[cfg(target_os = "windows")]
        if let Ok(h) = w2.hwnd() {
            crate::platform::windows::hide(h.0 as isize);
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = w2.hide();
    });
}

fn panel_window(app: &AppHandle, label: &str, page: &str, w: f64, h: f64) -> tauri::Result<WebviewWindow> {
    if let Some(existing) = app.get_webview_window(label) {
        if label == "dialog" {
            // A new drop replaces the old dialog content.
            let _ = existing.eval("location.reload()");
        }
        return Ok(existing);
    }
    let mut b = WebviewWindowBuilder::new(app, label, WebviewUrl::App(page.into()))
        .title(if label == "settings" { "Orbo Einstellungen" } else { "Orbo" })
        .inner_size(w, h)
        .min_inner_size(w * 0.8, h * 0.8)
        .transparent(true)
        .center()
        .visible(false);
    #[cfg(target_os = "macos")]
    {
        // The panels are designed dark; in light mode the vibrancy would turn pale under white text.
        b = b.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true).theme(Some(tauri::Theme::Dark));
    }
    #[cfg(target_os = "windows")]
    let mica = crate::platform::windows::is_windows_11();
    #[cfg(target_os = "windows")]
    {
        b = b.theme(Some(tauri::Theme::Dark)).initialization_script(WIN_PANEL_SCRIPT);
        if !mica {
            // Without Mica the transparent page would show the desktop through it.
            b = b.initialization_script("document.documentElement.classList.add('solid-bg');");
        }
    }
    let win = b.build()?;
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
        let _ = apply_vibrancy(&win, NSVisualEffectMaterial::UnderWindowBackground, None, Some(12.0));
    }
    #[cfg(target_os = "windows")]
    if mica {
        if let Err(e) = window_vibrancy::apply_mica(&win, Some(true)) {
            eprintln!("[orbo] Mica nicht verfügbar: {e}");
            let _ = win.eval("document.documentElement.classList.add('solid-bg')");
        }
    }
    Ok(win)
}

fn present(app: &AppHandle, win: &WebviewWindow) {
    let _ = win.show();
    let _ = win.set_focus();
    #[cfg(target_os = "macos")]
    {
        let _ = app.run_on_main_thread(crate::platform::macos::activate_app);
    }
    let _ = app;
}

pub fn open_dialog(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Ok(w) = panel_window(&app2, "dialog", "dialog.html", 760.0, 620.0) {
            present(&app2, &w);
        }
    });
}

pub fn open_settings(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Ok(w) = panel_window(&app2, "settings", "settings.html", 720.0, 520.0) {
            present(&app2, &w);
        }
    });
}

fn build_tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let paused = app.state::<AppState>().settings.read().unwrap().paused;
    let settings = MenuItem::with_id(app, "settings", "Einstellungen …", true, Some("CmdOrCtrl+,"))?;
    let pause = CheckMenuItem::with_id(app, "pause", "Pausieren", true, paused, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Orbo beenden", true, Some("CmdOrCtrl+Q"))?;
    let sep = PredefinedMenuItem::separator(app)?;
    match crate::updates::offered_version(app) {
        Some(v) => {
            let update = MenuItem::with_id(app, "update", format!("Update auf {v} installieren"), true, None::<&str>)?;
            Menu::with_items(app, &[&update, &settings, &pause, &sep, &quit])
        }
        None => Menu::with_items(app, &[&settings, &pause, &sep, &quit]),
    }
}

/// Rebuilds the tray menu, which shows "Update installieren" while an update waits for the user.
pub fn refresh_tray_menu(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let (Some(tray), Ok(menu)) = (app2.tray_by_id("main"), build_tray_menu(&app2)) {
            let _ = tray.set_menu(Some(menu));
        }
    });
}

pub fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_tray_menu(app)?;
    // macOS wants a monochrome template image in the menu bar, Windows a colored icon in the notification area.
    #[cfg(target_os = "macos")]
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    #[cfg(not(target_os = "macos"))]
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-color.png"))?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Orbo")
        .menu(&menu)
        .on_menu_event(move |app, ev| match ev.id().as_ref() {
            "settings" => open_settings(app),
            "pause" => {
                let state = app.state::<AppState>();
                let mut s = state.settings.write().unwrap();
                s.paused = !s.paused;
                crate::settings::save(&s);
                let _ = app.emit("settings", &*s);
            }
            "update" => crate::updates::install_now(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

pub fn update_tray(app: &AppHandle) {
    sync_autostart(app);
}

/// Keeps the login item in line with the setting.
pub fn sync_autostart(app: &AppHandle) {
    use tauri_plugin_autostart::ManagerExt;
    let want = app.state::<AppState>().settings.read().unwrap().autostart;
    let mgr = app.autolaunch();
    // Only register the real app bundle, not development builds started from target/.
    #[cfg(target_os = "macos")]
    let is_bundle = std::env::current_exe().map(|p| p.to_string_lossy().contains(".app/Contents/MacOS")).unwrap_or(false);
    #[cfg(not(target_os = "macos"))]
    let is_bundle = std::env::current_exe().map(|p| !p.to_string_lossy().contains(r"\target\")).unwrap_or(false);
    if want && is_bundle {
        // Re-registering keeps the login item pointing at the current app location after a move.
        let _ = mgr.enable();
    } else if mgr.is_enabled().unwrap_or(false) {
        let _ = mgr.disable();
    }
}

pub fn open_onboarding(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Ok(w) = panel_window(&app2, "onboarding", "onboarding.html", 720.0, 560.0) {
            present(&app2, &w);
        }
    });
}

pub const NOTCH_W: f64 = 720.0;
pub const NOTCH_H: f64 = 180.0;

/// Called on the main thread when a file drag reaches the top center of a screen. macOS only.
#[cfg_attr(not(target_os = "macos"), allow(dead_code, unused_variables))]
pub fn open_notch(app: &AppHandle, at: Point, files: Vec<std::path::PathBuf>) {
    let Some(w) = app.get_webview_window("notch") else { return };
    #[cfg(target_os = "macos")]
    {
        let Ok(p) = w.ns_window() else { return };
        crate::platform::macos::place_top_center(p, at, NOTCH_W, NOTCH_H);
        crate::platform::macos::show_without_activating(p);
    }
    *crate::platform::NOTCH_OPEN.lock().unwrap() = true;
    let settings = app.state::<AppState>().settings.read().unwrap().clone();
    let base = serde_json::json!({ "files": files, "suggestions": null, "style": settings.style, "sound": settings.sound, "reduce_motion": settings.reduce_motion });
    let _ = w.emit("notch:open", &base);
    if !files.is_empty() {
        let w2 = w.clone();
        std::thread::spawn(move || {
            if let Ok(v) = crate::commands::suggest_value(&files) {
                let mut payload = base;
                payload["suggestions"] = v;
                let _ = w2.emit("notch:open", payload);
            }
        });
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn close_notch(app: &AppHandle, reason: &str) {
    if let Some(w) = app.get_webview_window("notch") {
        let _ = w.emit("notch:close", serde_json::json!({ "reason": reason }));
    }
    hide_notch(app, if reason == "released" { 450 } else { 250 });
}

pub fn hide_notch(app: &AppHandle, delay_ms: u64) {
    *crate::platform::NOTCH_OPEN.lock().unwrap() = false;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(delay_ms));
        let Some(w) = app.get_webview_window("notch") else { return };
        let w2 = w.clone();
        let _ = w.run_on_main_thread(move || {
            if *crate::platform::NOTCH_OPEN.lock().unwrap() {
                return;
            }
            #[cfg(target_os = "macos")]
            if let Ok(p) = w2.ns_window() {
                crate::platform::macos::hide(p);
            }
            #[cfg(target_os = "windows")]
            if let Ok(h) = w2.hwnd() {
                crate::platform::windows::hide(h.0 as isize);
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let _ = w2.hide();
        });
    });
}

pub fn set_tray_progress(app: &AppHandle, text: Option<String>) {
    if let Some(tray) = app.tray_by_id("main") {
        #[cfg(target_os = "macos")]
        let _ = tray.set_title(text);
        // The Windows notification area has no text next to the icon.
        #[cfg(not(target_os = "macos"))]
        let _ = tray.set_tooltip(Some(text.map(|t| format!("Orbo · {t}")).unwrap_or_else(|| "Orbo".into())));
    }
}
