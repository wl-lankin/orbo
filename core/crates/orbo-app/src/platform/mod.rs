#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "macos")]
pub use macos::Point;

#[cfg(target_os = "windows")]
pub mod windows;

/// Screen point in physical pixels (virtual screen coordinates).
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone, Copy, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Key,
    Shake,
}

#[derive(Debug, Clone)]
pub enum DragSignal {
    Triggered { at: Point, trigger: Trigger, files: Vec<std::path::PathBuf> },
    /// Dragged far away from the ring; the drag itself continues normally.
    Cancelled,
    /// A file drag reached the top center of a screen (notch portal, macOS only).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    NotchEnter { at: Point, files: Vec<std::path::PathBuf> },
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    NotchLeave,
    Released,
}

/// Ring center while the ring is visible, set by the app after placing the window.
pub static RING_CENTER: std::sync::Mutex<Option<Point>> = std::sync::Mutex::new(None);
// Just outside the presets arc (244 pt): the window corners then never swallow a drop.
// On Windows the value is in logical pixels and scaled with the monitor's DPI.
pub const CANCEL_DISTANCE: f64 = 256.0;

/// Whether the notch portal is currently shown.
pub static NOTCH_OPEN: std::sync::Mutex<bool> = std::sync::Mutex::new(false);
