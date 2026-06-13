//! Cross-platform window-manager abstraction.
//!
//! Every operation that touches the OS (setting opacity, enumerating windows,
//! identifying the window under the cursor, autostart, etc.) goes through the
//! [`WindowManager`] trait. One backend is selected per target: [`win32`] on
//! Windows, and on Unix either [`kwin`] (KDE/Wayland) or [`x11`] depending on the
//! session (see [`init`]).

use anyhow::Result;
use std::sync::OnceLock;

#[cfg(windows)]
mod win32;

#[cfg(unix)]
mod kwin;
#[cfg(unix)]
mod linux_common;

// TODO: claude ffs making shit up
#[cfg(unix)]
mod x11;

const MINIMUM_TRANSPARENCY: i32 = 30;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct WindowHandle(pub u64);

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct CursorPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WindowInfo {
    pub class_name: String,
    pub process_name: String,
}

/// A single active transparency rule, handed to backends that enforce opacity
/// themselves (see [`WindowManager::sync_rules`]).
#[derive(Debug, Clone)]
pub struct RuleSpec {
    pub window_class: String,
    /// Target opacity, 0-255 where 255 is fully opaque.
    pub alpha: u8,
}

/// The set of platform operations the app needs. One implementation is selected
/// at compile time (see [`init`]).
pub trait WindowManager: Send + Sync {
    /// Set a window's opacity. `alpha` is 0-255 where 255 is fully opaque.
    fn set_window_alpha(&self, handle: WindowHandle, alpha: u8) -> Result<()>;

    /// All currently-open windows matching both `process_name` and `window_class`.
    fn enumerate_windows(&self, process_name: &str, window_class: &str) -> Vec<WindowHandle>;

    /// Given a child window class, find the top-level parent and its class name.
    fn find_parent_from_child_class(
        &self,
        child_class: &str,
    ) -> Result<Option<(WindowHandle, String)>>;

    /// Current cursor position.
    fn get_cursor_pos(&self) -> Result<CursorPoint>;

    /// Whether the left mouse button is currently pressed.
    fn is_left_click(&self) -> bool;

    /// Identify the window at a screen point.
    fn get_window_info_at(&self, point: CursorPoint) -> Result<WindowInfo>;

    /// Whether the window at `point` belongs to an elevated/admin process.
    fn is_elevated_at(&self, point: CursorPoint) -> bool;

    /// Whether this process is running elevated/admin.
    fn is_running_as_admin(&self) -> bool;

    /// Enable or disable launching the app at login.
    fn set_autostart(&self, enabled: bool) -> Result<()>;

    /// Whether the app is configured to launch at login.
    fn get_autostart_state(&self) -> bool;

    /// Open a file path with the platform's default handler.
    fn open_path(&self, path: &str) -> Result<()>;

    /// Resolve a process id to its executable name (without extension).
    fn process_name_from_pid(&self, pid: u32) -> Result<String>;

    /// Whether this backend selects windows through a native picker (e.g. KWin's
    /// `queryWindowInfo`) instead of the cursor-overlay primitives above. Wayland
    /// backends must use this because the cursor primitives don't work there.
    fn supports_native_pick(&self) -> bool {
        false
    }

    /// Pick a window using the backend's native selector, blocking until the user
    /// chooses. `Ok(None)` means they cancelled. Only called when
    /// [`WindowManager::supports_native_pick`] returns true.
    fn native_pick(&self) -> Result<Option<WindowInfo>> {
        Ok(None)
    }

    /// Like [`WindowManager::native_pick`], but reports the window currently under
    /// the cursor to `on_hover` (possibly from another thread) so the caller can
    /// show a live preview. Defaults to a plain pick with no hover updates.
    fn native_pick_with_hover(
        &self,
        _on_hover: &(dyn Fn(WindowInfo) + Sync),
    ) -> Result<Option<WindowInfo>> {
        self.native_pick()
    }

    /// Apply the full set of active transparency rules at once. Backends that let
    /// the compositor enforce opacity (KWin) implement this; the polling backends
    /// (Windows, X11) leave it a no-op and rely on the monitor loop instead.
    fn sync_rules(&self, _rules: &[RuleSpec]) -> Result<()> {
        Ok(())
    }
}

static MANAGER: OnceLock<Box<dyn WindowManager>> = OnceLock::new();

pub fn init() {
    #[cfg(windows)]
    let manager: Box<dyn WindowManager> = Box::new(win32::Win32Manager::new());
    #[cfg(unix)]
    let manager: Box<dyn WindowManager> = init_unix();

    let _ = MANAGER.set(manager);
}

#[cfg(unix)]
fn init_unix() -> Box<dyn WindowManager> {
    if prefer_kwin() {
        match kwin::KWinManager::new() {
            Ok(manager) => return Box::new(manager),
            Err(e) => eprintln!("KWin backend unavailable ({e}); falling back to X11."),
        }
    }

    Box::new(x11::X11Manager::new().expect("Failed to connect to the X11 display server"))
}

#[cfg(unix)]
fn prefer_kwin() -> bool {
    let wayland = std::env::var("XDG_SESSION_TYPE")
        .map(|value| value.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
        || std::env::var_os("WAYLAND_DISPLAY").is_some();
    let kde = std::env::var("XDG_CURRENT_DESKTOP")
        .map(|value| value.to_uppercase().contains("KDE"))
        .unwrap_or(false);
    wayland && kde
}

pub fn wm() -> &'static dyn WindowManager {
    MANAGER
        .get()
        .expect("platform::init() must be called before platform::wm()")
        .as_ref()
}

/*
  Convert a value from 1 - 100 to its u8 (255) equivalent.
  TODO: this function should be either contextually typed or private... it could be used incorrectly
*/
pub fn convert_to_full(mut value: i32) -> u8 {
    if value < MINIMUM_TRANSPARENCY {
        value = MINIMUM_TRANSPARENCY;
    }
    if value > 100 {
        return 255;
    }
    ((value as f32 / 100.0) * 255.0).round() as u8
}

/*
  Takes a u8 (255) value and converts it to a measurable format (a percentage of 100)
*/
pub fn convert_to_human(value: u8) -> u8 {
    ((value as f32 / 255.0) * 100.0).round() as u8
}
