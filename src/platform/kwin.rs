//! KWin (KDE Plasma, Wayland) implementation of [`WindowManager`].
//!
//! Native Wayland forbids a client from polling the global cursor, positioning an
//! overlay, or touching another window's surface — so the X11 cursor-overlay
//! picker and `_NET_WM_WINDOW_OPACITY` simply don't work under KWin/Wayland.
//! Instead this backend talks to KWin over D-Bus (a persistent `zbus` session
//! connection, so the picker's per-frame calls are cheap and don't fork):
//!
//! * **Picking** uses `org.kde.KWin.queryWindowInfo()`, KWin's own click-to-select
//!   that returns the chosen window's `resourceClass`.
//! * **Opacity** is enforced by a small resident KWin script (the Scripting
//!   D-Bus interface) that sets `window.opacity` for matching classes and
//!   re-applies to newly opened windows. The script is regenerated and reloaded
//!   whenever the rules change.
//! * **Cursor following** for the picker panel is done by a probe script that
//!   moves our window to the cursor (a Wayland client can't move itself), reran
//!   each frame over the persistent connection.

use super::{CursorPoint, RuleSpec, WindowHandle, WindowInfo, WindowManager, linux_common};
use anyhow::{Result, anyhow};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use zbus::{blocking::Connection, zvariant::OwnedValue};

const SERVICE: &str = "org.kde.KWin";
const SCRIPTING_PATH: &str = "/Scripting";
const SCRIPTING_IFACE: &str = "org.kde.kwin.Scripting";
const KWIN_PATH: &str = "/KWin";
const KWIN_IFACE: &str = "org.kde.KWin";
const PLUGIN: &str = "winalpha";
const HOVER_PLUGIN: &str = "winalpha-hover";

/// Probe script run each frame during a pick: it (1) moves our picker panel to
/// the cursor so it follows the mouse, and (2) reports the top-most window under
/// the cursor (skipping our panel and the desktop) to the journal.
const HOVER_SCRIPT: &str = r#"var p = workspace.cursorPos;
var stack = workspace.stackingOrder || ((typeof workspace.windowList === "function") ? workspace.windowList() : workspace.clientList);
var out = "";
var self = null;
for (var i = stack.length - 1; i >= 0; i--) {
    var w = stack[i];
    if (!w) continue;
    if (("" + w.caption).indexOf("winalpha-picker") >= 0) { self = w; continue; }
    if (!w.resourceClass) continue;
    if (w.minimized) continue;
    var c = "" + w.resourceClass;
    if (c === "plasmashell") continue;
    if (out === "") {
        var g = w.frameGeometry;
        if (g && p.x >= g.x && p.x <= g.x + g.width && p.y >= g.y && p.y <= g.y + g.height) {
            out = c + "\t" + w.caption;
        }
    }
}
if (self) {
    var sg = self.frameGeometry;
    self.frameGeometry = { x: p.x + 16, y: p.y + 16, width: sg.width, height: sg.height };
}
print("WINALPHA_HOVER\t" + out);
"#;

pub struct KWinManager {
    conn: Connection,
}

impl KWinManager {
    /// Connect to the session bus and verify KWin is present, so we fail fast and
    /// let the caller fall back to X11 on non-KWin sessions.
    pub fn new() -> Result<Self> {
        let conn =
            Connection::session().map_err(|e| anyhow!("D-Bus session bus unavailable: {e}"))?;

        conn.call_method(
            Some(SERVICE),
            KWIN_PATH,
            Some("org.freedesktop.DBus.Peer"),
            "Ping",
            &(),
        )
        .map_err(|e| anyhow!("KWin not reachable on D-Bus: {e}"))?;

        // Write the hover probe once; it is (re)loaded on each frame during a pick.
        if let Ok(path) = runtime_file("winalpha-hover.js") {
            let _ = std::fs::write(path, HOVER_SCRIPT);
        }

        Ok(Self { conn })
    }

    /// Unload a KWin script by plugin name (no-op if it wasn't loaded).
    fn script_unload(&self, plugin: &str) {
        let _ = self.conn.call_method(
            Some(SERVICE),
            SCRIPTING_PATH,
            Some(SCRIPTING_IFACE),
            "unloadScript",
            &(plugin,),
        );
    }

    /// Load and start a KWin script.
    fn script_load_start(&self, path: &str, plugin: &str) {
        if self
            .conn
            .call_method(
                Some(SERVICE),
                SCRIPTING_PATH,
                Some(SCRIPTING_IFACE),
                "loadScript",
                &(path, plugin),
            )
            .is_ok()
        {
            let _ = self.conn.call_method(
                Some(SERVICE),
                SCRIPTING_PATH,
                Some(SCRIPTING_IFACE),
                "start",
                &(),
            );
        }
    }

    /// Run the probe once: moves the picker panel to the cursor and prints the
    /// window under it to the journal. Three cheap in-process D-Bus calls (no
    /// process spawns), so the panel can track the mouse smoothly. The script is
    /// left loaded; the next call's unload clears it.
    fn run_probe(&self) {
        let path = match runtime_file("winalpha-hover.js") {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(_) => return,
        };
        self.script_unload(HOVER_PLUGIN);
        self.script_load_start(&path, HOVER_PLUGIN);
    }

    /// Read the most recent window-under-cursor reported by the probe.
    fn read_hover(&self) -> Option<WindowInfo> {
        let output = Command::new("journalctl")
            .args([
                "--user",
                "-o",
                "cat",
                "-g",
                "WINALPHA_HOVER",
                "-n",
                "1",
                "--since",
                "2 seconds ago",
            ])
            .output()
            .ok()?;

        parse_hover(&String::from_utf8_lossy(&output.stdout))
    }
}

impl WindowManager for KWinManager {
    // Opacity is enforced by the resident KWin script (see `sync_rules`), not by
    // per-window calls, so these are inert on KWin.
    fn set_window_alpha(&self, _handle: WindowHandle, _alpha: u8) -> Result<()> {
        Ok(())
    }

    // The polling monitor has nothing to do on KWin — return no handles so it idles.
    fn enumerate_windows(&self, _process_name: &str, _window_class: &str) -> Vec<WindowHandle> {
        Vec::new()
    }

    fn find_parent_from_child_class(
        &self,
        _child_class: &str,
    ) -> Result<Option<(WindowHandle, String)>> {
        Ok(None)
    }

    // Cursor primitives are unavailable on Wayland; picking goes through `native_pick`.
    fn get_cursor_pos(&self) -> Result<CursorPoint> {
        Err(anyhow!("cursor position is not available on Wayland"))
    }

    fn is_left_click(&self) -> bool {
        false
    }

    fn get_window_info_at(&self, _point: CursorPoint) -> Result<WindowInfo> {
        Err(anyhow!("window-at-point is not available on Wayland"))
    }

    fn is_elevated_at(&self, _point: CursorPoint) -> bool {
        false
    }

    fn is_running_as_admin(&self) -> bool {
        false
    }

    fn set_autostart(&self, enabled: bool) -> Result<()> {
        linux_common::set_autostart(enabled)
    }

    fn get_autostart_state(&self) -> bool {
        linux_common::get_autostart_state()
    }

    fn open_path(&self, path: &str) -> Result<()> {
        linux_common::open_path(path)
    }

    fn process_name_from_pid(&self, pid: u32) -> Result<String> {
        linux_common::process_name_from_pid(pid)
    }

    fn supports_native_pick(&self) -> bool {
        true
    }

    fn native_pick(&self) -> Result<Option<WindowInfo>> {
        // Blocks while KWin shows its click-to-select cursor. A cancelled pick
        // (Escape) returns an error/empty reply, which we treat as "no selection".
        let reply = match self.conn.call_method(
            Some(SERVICE),
            KWIN_PATH,
            Some(KWIN_IFACE),
            "queryWindowInfo",
            &(),
        ) {
            Ok(reply) => reply,
            Err(_) => return Ok(None),
        };

        let body = reply.body();
        let info: HashMap<String, OwnedValue> = body
            .deserialize()
            .map_err(|e| anyhow!("could not decode queryWindowInfo reply: {e}"))?;

        // queryWindowInfo doesn't expose a pid on current KWin, so we key on the
        // resource class (also what the opacity script matches on).
        let class = info
            .get("resourceClass")
            .and_then(owned_to_string)
            .filter(|class| !class.is_empty());

        match class {
            Some(class) => Ok(Some(WindowInfo {
                class_name: class.clone(),
                process_name: class,
            })),
            None => Ok(None),
        }
    }

    fn native_pick_with_hover(
        &self,
        on_hover: &(dyn Fn(WindowInfo) + Sync),
    ) -> Result<Option<WindowInfo>> {
        let stop = AtomicBool::new(false);

        // Two scoped workers run while `native_pick` blocks on KWin's
        // click-to-select: one keeps the panel glued to the cursor (paced to the
        // display rate), the other refreshes the hovered-window label at a calmer
        // cadence. Separating them keeps the journal read from stalling movement.
        // The scope joins both before returning, so `on_hover` is only borrowed.
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
                    if let Some(info) = self.read_hover() {
                        on_hover(info);
                    }
                    std::thread::sleep(Duration::from_millis(150));
                }
            });

            let result = self.native_pick();
            stop.store(true, Ordering::Relaxed);
            result
        });

        // Clear the probe script left loaded by the last move frame.
        self.script_unload(HOVER_PLUGIN);
        result
    }

    fn sync_rules(&self, rules: &[RuleSpec]) -> Result<()> {
        let path = runtime_file("winalpha-kwin.js")?;
        std::fs::write(&path, build_script(rules))?;
        let path = path.to_string_lossy().into_owned();

        // Reload: drop the previous instance (and its windowAdded handler), then
        // load and start the freshly generated script.
        self.script_unload(PLUGIN);
        self.script_load_start(&path, PLUGIN);
        Ok(())
    }
}

/// Extract a `String` from a D-Bus variant value, if it holds one.
fn owned_to_string(value: &OwnedValue) -> Option<String> {
    String::try_from(value.try_clone().ok()?).ok()
}

/// Path of a per-session runtime file (the generated KWin scripts live here).
fn runtime_file(name: &str) -> Result<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    Ok(dir.join(name))
}

/// Parse a `WINALPHA_HOVER\t<class>\t<caption>` probe line into window info.
fn parse_hover(line: &str) -> Option<WindowInfo> {
    let mut fields = line.trim_end().splitn(3, '\t');
    let _tag = fields.next()?; // "WINALPHA_HOVER"
    let class = fields.next()?.trim();
    if class.is_empty() {
        return None;
    }
    let caption = fields.next().unwrap_or("").trim();
    Some(WindowInfo {
        class_name: class.to_owned(),
        process_name: if caption.is_empty() {
            class.to_owned()
        } else {
            caption.to_owned()
        },
    })
}

/// Generate the resident KWin script: reset every window to opaque, then apply
/// the per-class opacity rules, and keep applying them to newly opened windows.
fn build_script(rules: &[RuleSpec]) -> String {
    let mut array = String::from("[");
    for (i, rule) in rules.iter().enumerate() {
        if i > 0 {
            array.push(',');
        }
        let opacity = (rule.alpha as f64 / 255.0).clamp(0.0, 1.0);
        let class = rule.window_class.replace('\\', "\\\\").replace('"', "\\\"");
        array.push_str(&format!("{{\"cls\":\"{class}\",\"op\":{opacity:.4}}}"));
    }
    array.push(']');

    format!(
        r#"var rules = {array};
function applyTo(w) {{
    if (!w || !w.resourceClass) return;
    var cls = "" + w.resourceClass;
    for (var i = 0; i < rules.length; i++) {{
        if (cls === rules[i].cls) {{ w.opacity = rules[i].op; return; }}
    }}
}}
var list = (typeof workspace.windowList === "function") ? workspace.windowList() : workspace.clientList;
for (var i = 0; i < list.length; i++) {{ list[i].opacity = 1.0; }}
for (var i = 0; i < list.length; i++) {{ applyTo(list[i]); }}
var added = workspace.windowAdded || workspace.clientAdded;
if (added) added.connect(applyTo);
"#
    )
}
