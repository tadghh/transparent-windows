//! KWin (KDE Plasma, Wayland) implementation of [`WindowManager`].
//!
//! Native Wayland forbids a client from polling the global cursor, positioning an
//! overlay, or touching another window's surface — so the X11 cursor-overlay
//! picker and `_NET_WM_WINDOW_OPACITY` simply don't work under KWin/Wayland.
//! Instead this backend talks to KWin over D-Bus (a persistent `zbus` session
//! connection, so the picker's per-frame calls are cheap and don't fork). All
//! the D-Bus plumbing lives in [`dbus`]; the injected JavaScript lives in
//! [`script`]; this file is just the policy that wires them to the trait:
//!
//! * **Picking** uses `org.kde.KWin.queryWindowInfo()`, KWin's own click-to-select
//!   that returns the chosen window's `resourceClass`.
//! * **Opacity** is enforced by a small resident KWin script that sets
//!   `window.opacity` for matching classes and re-applies to newly opened
//!   windows. The script is regenerated and reloaded whenever the rules change.
//! * **Cursor following + hover** for the picker panel is done by a probe script
//!   that moves our window to the cursor (a Wayland client can't move itself) and
//!   reports the window under it by calling back into a small D-Bus interface we
//!   serve (the `org.winalpha.Hover` object in [`dbus`]); it is reran each frame
//!   over the persistent connection.

mod dbus;
mod script;

#[cfg(test)]
#[path = "../../tests/kwin.rs"]
mod tests;

use super::{
    CompositorOpacity, Opacity, PickHover, RuleSpec, WindowHandle, WindowInfo, WindowManager,
};
use anyhow::Result;
use dbus::KwinDbus;
use script::{build_hover_script, build_script, hover_plugin, plugin};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub struct KWinManager {
    dbus: KwinDbus,
}

impl KWinManager {
    /// Connect to KWin (failing fast so the caller can fall back to X11) and drop
    /// the hover probe on disk; it is (re)loaded on each frame during a pick.
    pub fn new() -> Result<Self> {
        let dbus = KwinDbus::connect()?;

        if let Ok(path) = runtime_file(&hover_script_file()) {
            let _ = std::fs::write(path, build_hover_script());
        }

        Ok(Self { dbus })
    }

    /// Run the probe once: moves the picker panel to the cursor and reports the
    /// window under it back over D-Bus. The script is left loaded; the next
    /// call's unload clears it.
    fn run_probe(&self) {
        let path = match runtime_file(&hover_script_file()) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(_) => return,
        };
        let name = hover_plugin();
        self.dbus.unload_script(&name);
        self.dbus.load_and_start_script(&path, &name);
    }
}

impl WindowManager for KWinManager {
    fn find_parent_from_child_class(
        &self,
        _child_class: &str,
    ) -> Result<Option<(WindowHandle, String)>> {
        Ok(None)
    }

    fn opacity(&self) -> Opacity<'_> {
        Opacity::Compositor(self)
    }

    fn pick_window(&self, on_hover: &(dyn Fn(PickHover) + Sync)) -> Result<Option<WindowInfo>> {
        // Start fresh so a window left over from a previous pick isn't shown.
        self.dbus.clear_hover();

        let stop = AtomicBool::new(false);

        // Two scoped workers run while `queryWindowInfo` blocks on KWin's
        // click-to-select: one keeps the panel glued to the cursor (paced to the
        // display rate), the other forwards the latest hovered window (pushed to
        // us over D-Bus by the probe) to the panel at a calmer cadence. The scope
        // joins both before returning, so `on_hover` is only borrowed.
        let result = std::thread::scope(|scope| {
            scope.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    self.run_probe();
                    // Pace to ~display rate; there's no point moving a window
                    // faster than the screen refreshes (and reloading the KWin
                    // script every frame still costs KWin a little work).
                    std::thread::sleep(Duration::from_millis(16));
                }
            });
            scope.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    if let Some(info) = self.dbus.hover() {
                        // Wayland can't report the cursor position or elevation,
                        // so the panel stays fixed and the window never blocks.
                        on_hover(PickHover {
                            info,
                            blocked: false,
                            cursor: None,
                        });
                    }
                    std::thread::sleep(Duration::from_millis(150));
                }
            });

            let result = self.dbus.query_window_info();
            stop.store(true, Ordering::Relaxed);
            result
        });

        // Clear the probe script left loaded by the last move frame.
        self.dbus.unload_script(&hover_plugin());
        result
    }
}

impl CompositorOpacity for KWinManager {
    fn sync_rules(&self, rules: &[RuleSpec]) -> Result<()> {
        let path = runtime_file(&format!("{}-kwin.js", plugin()))?;
        std::fs::write(&path, build_script(rules))?;
        let path = path.to_string_lossy().into_owned();

        // Reload: drop the previous instance (and its windowAdded handler), then
        // load and start the freshly generated script.
        let name = plugin();
        self.dbus.unload_script(&name);
        self.dbus.load_and_start_script(&path, &name);
        Ok(())
    }
}

/// On-disk name of the hover probe, derived from its plugin name so the write in
/// `new` and the read in `run_probe` can't drift.
fn hover_script_file() -> String {
    format!("{}.js", hover_plugin())
}

/// Path of a per-session runtime file (the generated KWin scripts live here).
fn runtime_file(name: &str) -> Result<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    Ok(dir.join(name))
}
