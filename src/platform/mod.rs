//! Cross-platform window-manager abstraction.
//!
//! Window operations (opacity, enumerating windows, identifying the window under
//! the cursor, picking) go through the [`WindowManager`] trait. One backend is
//! selected per target: `win32` on Windows, and on Unix either `kwin`
//! (KDE/Wayland) or `x11` depending on the session (see [`init`]).
//!
//! Host-OS integration that doesn't vary by window backend (autostart, opening
//! paths, resolving pids) goes through the [`OperatingSystem`] contract, resolved
//! to the single [`Os`] for the build at compile time.

use anyhow::Result;
use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
#[cfg(unix)]
use tracing::{debug, warn};

#[cfg(unix)]
mod kwin;
#[cfg(unix)]
mod linux_common;
#[cfg(windows)]
mod win32;
#[cfg(unix)]
mod x11;

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

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
/// themselves (see [`CompositorOpacity::sync_rules`]).
#[derive(Debug, Clone)]
pub struct RuleSpec {
    pub window_class: String,
    /// Target opacity, 0-255 where 255 is fully opaque.
    pub alpha: u8,
}

/// A live update during [`WindowManager::pick_window`], carrying everything the
/// caller's preview panel needs without it knowing which backend is picking.
#[derive(Debug, Clone, Default)]
pub struct PickHover {
    /// The window currently under the cursor / selector.
    pub info: WindowInfo,
    /// Whether this window can't be made transparent (e.g. it's elevated and we
    /// aren't), so the caller can surface a warning.
    pub blocked: bool,
    /// Cursor position, when the backend can report it (Windows, X11), so the
    /// panel can follow the mouse. `None` on Wayland, where a client can't
    /// position itself and the panel stays fixed.
    pub cursor: Option<CursorPoint>,
}

// ---------------------------------------------------------------------------
// Window-manager contract
// ---------------------------------------------------------------------------

/// The platform operations every window backend provides: opacity enforcement
/// (via the [`Opacity`] selector, since that loop is interleaved with app-level
/// caching) and window picking (which each backend owns end to end). One
/// implementation is selected at runtime (see [`init`]). OS integration that
/// doesn't vary by window backend lives on [`OperatingSystem`] instead.
pub trait WindowManager: Send + Sync {
    /// Given a child window class, find the top-level parent and its class name.
    fn find_parent_from_child_class(
        &self,
        child_class: &str,
    ) -> Result<Option<(WindowHandle, String)>>;
    /// How this backend enforces opacity (see [`Opacity`]).
    fn opacity(&self) -> Opacity<'_>;
    /// Let the user pick a window, blocking until they choose. `Ok(None)` means
    /// they cancelled. The window under the cursor is reported to `on_hover`
    /// (possibly from another thread) so the caller can show a live preview; the
    /// caller owns only that UI, while each backend runs its own pick logic
    /// (cursor polling on Windows/X11, a native selector on KWin).
    fn pick_window(&self, on_hover: &(dyn Fn(PickHover) + Sync)) -> Result<Option<WindowInfo>>;
}

/// A backend's opacity-enforcement strategy. Polling backends (Windows, X11)
/// set per-window alpha on a monitor loop; compositor backends (KWin) hand the
/// full rule set to the compositor, which enforces it.
pub enum Opacity<'a> {
    Polling(&'a dyn PollingOpacity),
    Compositor(&'a dyn CompositorOpacity),
}

/// Opacity by polling: the monitor loop enumerates matching windows and sets
/// their alpha directly. Used by Windows and X11.
pub trait PollingOpacity: Send + Sync {
    /// All currently-open windows matching both `process_name` and `window_class`.
    fn enumerate_windows(&self, process_name: &str, window_class: &str) -> Vec<WindowHandle>;

    /// Set a window's opacity. `alpha` is 0-255 where 255 is fully opaque.
    fn set_window_alpha(&self, handle: WindowHandle, alpha: u8) -> Result<()>;

    /// An optional signal that fires when the set of open windows may have
    /// changed, so the monitor can re-apply opacity in response to windows
    /// opening/closing instead of polling on a fixed interval. `None` (the
    /// default) means the backend has no such signal and the monitor falls back
    /// to periodic polling. The signal is coalescing — bursts collapse into one
    /// wake-up — and is paired with a slow periodic re-apply for robustness.
    fn window_change_signal(&self) -> Option<Arc<tokio::sync::Notify>> {
        None
    }
}

/// Opacity enforced by the compositor: the full rule set is applied at once and
/// the compositor keeps it applied (including to newly opened windows). Used by
/// KWin.
pub trait CompositorOpacity: Send + Sync {
    /// Apply the full set of active transparency rules at once.
    fn sync_rules(&self, rules: &[RuleSpec]) -> Result<()>;
}

// ---------------------------------------------------------------------------
// Host-OS contract
// ---------------------------------------------------------------------------

/// Host-OS integration that has nothing to do with windowing: autostart, opening
/// paths, resolving pids. Unlike [`WindowManager`] (one of several backends chosen
/// at *runtime* on a given OS), there is exactly one OS per build, so this is a
/// compile-time contract on a stateless unit type rather than a `dyn` trait — see
/// [`Os`]. Implementing it forces every OS to supply the whole set (a missing
/// method is a compile error), without any runtime dispatch.
pub trait OperatingSystem {
    /// Enable or disable launching the app at login.
    fn set_autostart(enabled: bool) -> Result<()>;
    /// Whether the app is configured to launch at login.
    fn get_autostart_state() -> bool;
    /// Open a file path with the platform's default handler.
    fn open_path(path: &str) -> Result<()>;
    /// Resolve a process id to its executable name (without extension).
    fn process_name_from_pid(pid: u32) -> Result<String>;
}

/// The host OS for this build, resolved at compile time. Call its contract
/// directly, e.g. `Os::set_autostart(true)?` — no instance, no dispatch.
#[cfg(unix)]
pub use linux_common::Linux as Os;
#[cfg(windows)]
pub use win32::Windows as Os;

// ---------------------------------------------------------------------------
// Backend selection & access
// ---------------------------------------------------------------------------

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
            Ok(manager) => {
                debug!("selected KWin window backend");
                return Box::new(manager);
            }
            Err(e) => {
                warn!(error = %e, "KWin backend unavailable; falling back to X11")
            }
        }
    }

    debug!("selected X11 window backend");
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

// ---------------------------------------------------------------------------
// Cursor-overlay picker (internal, shared by polling backends)
// ---------------------------------------------------------------------------

/// Cursor primitives a polling backend (Windows, X11) exposes so the shared
/// [`run_cursor_pick`] loop can drive an overlay pick. Internal to the platform
/// layer — backends implement it, the loop consumes it, nothing else sees it.
trait CursorPicker: Send + Sync {
    /// Current cursor position.
    fn get_cursor_pos(&self) -> Result<CursorPoint>;
    /// Whether the left mouse button is currently pressed.
    fn is_left_click(&self) -> bool;
    /// Whether the user has asked to cancel the pick (right-click or Escape), so
    /// the shared loop can abort instead of forcing a selection.
    fn is_cancel_requested(&self) -> bool {
        false
    }
    /// Identify the window at a screen point.
    fn get_window_info_at(&self, point: CursorPoint) -> Result<WindowInfo>;
    /// Whether the window at `point` belongs to an elevated/admin process.
    fn is_elevated_at(&self, point: CursorPoint) -> bool;
    /// Whether this process is running elevated/admin.
    fn is_running_as_admin(&self) -> bool;
}

/// The cursor-overlay pick loop, shared by every polling backend so the logic
/// lives once. Polls `picker` for the window under the cursor, reporting each
/// change (and the cursor position, so the overlay can follow) through
/// `on_hover`, and returns the window the user clicks. Runs until a click; the
/// caller drives it on a worker thread.
fn run_cursor_pick(
    picker: &dyn CursorPicker,
    on_hover: &(dyn Fn(PickHover) + Sync),
) -> Result<Option<WindowInfo>> {
    const POLL_INTERVAL: Duration = Duration::from_millis(8);
    const WINDOW_CHECK_INTERVAL: Duration = Duration::from_millis(25);

    let is_admin = picker.is_running_as_admin();
    let mut click_point = CursorPoint::default();
    let mut click_point_old = CursorPoint::default();
    let mut window_info_old = WindowInfo::default();
    let mut blocked = false;
    let mut last_window_check = Instant::now();

    loop {
        let now = Instant::now();

        // Right-click / Escape aborts the pick without forcing a selection.
        if picker.is_cancel_requested() {
            return Ok(None);
        }

        if let Ok(pos) = picker.get_cursor_pos() {
            click_point = pos;
            if pos != click_point_old {
                click_point_old = pos;
                on_hover(PickHover {
                    info: window_info_old.clone(),
                    blocked,
                    cursor: Some(pos),
                });
            }
        }

        if now.duration_since(last_window_check) >= WINDOW_CHECK_INTERVAL {
            last_window_check = now;

            if let Ok(window_info) = picker.get_window_info_at(click_point)
                && window_info_old != window_info
            {
                window_info_old = window_info.clone();
                blocked = picker.is_elevated_at(click_point) && !is_admin;
                on_hover(PickHover {
                    info: window_info,
                    blocked,
                    cursor: Some(click_point),
                });
            }
        }

        if picker.is_left_click() {
            return Ok(Some(
                picker.get_window_info_at(click_point).unwrap_or_default(),
            ));
        }

        std::thread::sleep(POLL_INTERVAL);
    }
}

// ---------------------------------------------------------------------------
// Opacity conversions
// ---------------------------------------------------------------------------

const MINIMUM_TRANSPARENCY: i32 = 30;
const PERCENT_MAX: i32 = 100;

fn rescale(value: u32, from_max: u32, to_max: u32) -> u32 {
    ((value as u64 * to_max as u64 + from_max as u64 / 2) / from_max as u64) as u32
}

pub fn percent_to_alpha(percent: i32) -> u8 {
    let percent = percent.clamp(MINIMUM_TRANSPARENCY, PERCENT_MAX);
    rescale(percent as u32, PERCENT_MAX as u32, u8::MAX as u32) as u8
}

pub fn alpha_to_percent(alpha: u8) -> u8 {
    rescale(alpha as u32, u8::MAX as u32, PERCENT_MAX as u32) as u8
}

#[cfg(test)]
#[path = "../tests/platform.rs"]
mod tests;
