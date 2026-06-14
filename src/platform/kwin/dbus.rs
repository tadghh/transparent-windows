//! KWin D-Bus transport — the only zbus-touching code in the crate.
//!
//! [`KwinDbus`] owns the session connection and exposes typed calls for the
//! pieces of KWin we drive (script load/unload, `queryWindowInfo`), plus it
//! hosts the `org.winalpha.Hover` service that the probe script calls back into.
//! The rest of the backend talks to KWin only through this struct.

use crate::platform::WindowInfo;
use anyhow::{Result, anyhow};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use zbus::{
    blocking::{Connection, connection::Builder},
    zvariant::OwnedValue,
};

const SERVICE: &str = "org.kde.KWin";
const SCRIPTING_PATH: &str = "/Scripting";
const SCRIPTING_IFACE: &str = "org.kde.kwin.Scripting";
const KWIN_PATH: &str = "/KWin";
const KWIN_IFACE: &str = "org.kde.KWin";
const PEER_IFACE: &str = "org.freedesktop.DBus.Peer";

// Our own D-Bus interface that the probe script calls back into with the hovered
// window. The probe script (see `super::script`) is *generated* from these, so the
// only remaining duplication is the `#[zbus::interface]` / `#[zbus(name)]`
// attributes below — those need string literals, and a test pins them to these.
pub(super) const HOVER_OBJECT: &str = "/Hover";
pub(super) const HOVER_METHOD: &str = "Report";

/// Well-known bus name we claim for the hover callback, derived from the app id.
pub(super) fn hover_service() -> String {
    format!("org.{}", crate::identity::APP_ID)
}

/// Interface name the probe script invokes; must equal the `#[zbus::interface]`
/// literal on [`HoverSink`].
pub(super) fn hover_iface() -> String {
    format!("{}.Hover", hover_service())
}

pub(super) struct KwinDbus {
    conn: Connection,
    /// Latest window the probe script reported over D-Bus, read by the picker.
    latest_hover: Arc<Mutex<Option<WindowInfo>>>,
}

/// The D-Bus object the KWin probe script calls back into with the window under
/// the cursor. The connection's internal task dispatches `Report` onto the
/// shared slot, which the picker reads.
struct HoverSink {
    latest: Arc<Mutex<Option<WindowInfo>>>,
}

// These two literals can't reference the helpers above (attribute macros need
// literals); the `hover_identity` test keeps them equal to `hover_iface()` /
// `HOVER_METHOD`. The explicit method name also stops zbus PascalCasing `report`.
#[zbus::interface(name = "org.winalpha.Hover")]
impl HoverSink {
    #[zbus(name = "Report")]
    fn report(&self, class: String, caption: String) {
        // The caption is the friendlier label; fall back to the class.
        let process_name = if caption.is_empty() {
            class.clone()
        } else {
            caption
        };
        match self.latest.lock() {
            Ok(mut slot) => {
                *slot = Some(WindowInfo {
                    class_name: class,
                    process_name,
                });
            }
            Err(e) => eprintln!("Hover slot lock poisoned: {e}"),
        }
    }
}

impl KwinDbus {
    /// Connect to the session bus, serve the hover interface, and verify KWin is
    /// present, so we fail fast and let the caller fall back to X11 on non-KWin
    /// sessions.
    pub(super) fn connect() -> Result<Self> {
        let latest_hover = Arc::new(Mutex::new(None));

        // Serve our hover interface and claim a well-known name so the probe
        // script can call back into it. If that setup fails we still want KWin —
        // a plain connection works for everything but the live hover preview.
        let conn = match serve_hover(latest_hover.clone()) {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("hover D-Bus service unavailable ({e}); picker preview disabled");
                Connection::session().map_err(|e| anyhow!("D-Bus session bus unavailable: {e}"))?
            }
        };
        let dbus = Self { conn, latest_hover };

        dbus.call(KWIN_PATH, PEER_IFACE, "Ping", &())
            .map_err(|e| anyhow!("KWin not reachable on D-Bus: {e}"))?;

        Ok(dbus)
    }

    /// Call a method on KWin's D-Bus service. Every call targets [`SERVICE`]; only
    /// the object path, interface, method, and body vary.
    fn call<B>(
        &self,
        path: &str,
        iface: &str,
        method: &str,
        body: &B,
    ) -> zbus::Result<zbus::Message>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.conn
            .call_method(Some(SERVICE), path, Some(iface), method, body)
    }

    /// Unload a KWin script by plugin name (no-op if it wasn't loaded).
    pub(super) fn unload_script(&self, plugin: &str) {
        let _ = self.call(SCRIPTING_PATH, SCRIPTING_IFACE, "unloadScript", &(plugin,));
    }

    /// Load and start a KWin script.
    pub(super) fn load_and_start_script(&self, path: &str, plugin: &str) {
        if self
            .call(
                SCRIPTING_PATH,
                SCRIPTING_IFACE,
                "loadScript",
                &(path, plugin),
            )
            .is_ok()
        {
            let _ = self.call(SCRIPTING_PATH, SCRIPTING_IFACE, "start", &());
        }
    }

    /// Blocking native pick via KWin's `queryWindowInfo` click-to-select. A
    /// cancelled pick (Escape) returns an error/empty reply, treated as "no
    /// selection". `queryWindowInfo` doesn't expose a pid on current KWin, so we
    /// key on the resource class (also what the opacity script matches on).
    pub(super) fn query_window_info(&self) -> Result<Option<WindowInfo>> {
        let reply = match self.call(KWIN_PATH, KWIN_IFACE, "queryWindowInfo", &()) {
            Ok(reply) => reply,
            Err(_) => return Ok(None),
        };

        let body = reply.body();
        let info: HashMap<String, OwnedValue> = body
            .deserialize()
            .map_err(|e| anyhow!("could not decode queryWindowInfo reply: {e}"))?;

        let class = info
            .get("resourceClass")
            .and_then(owned_to_string)
            .filter(|class| !class.is_empty());

        Ok(class.map(|class| WindowInfo {
            class_name: class.clone(),
            process_name: class,
        }))
    }

    /// The most recent window the probe script reported over D-Bus, if any.
    pub(super) fn hover(&self) -> Option<WindowInfo> {
        match self.latest_hover.lock() {
            Ok(slot) => slot.clone(),
            Err(e) => {
                eprintln!("Hover slot lock poisoned: {e}");
                None
            }
        }
    }

    /// Forget any reported window. Called before a fresh pick so a window left
    /// over from a previous one isn't shown.
    pub(super) fn clear_hover(&self) {
        match self.latest_hover.lock() {
            Ok(mut slot) => *slot = None,
            Err(e) => eprintln!("Hover slot lock poisoned: {e}"),
        }
    }
}

/// Build a session-bus connection that owns the hover service and serves the
/// [`HoverSink`] at [`HOVER_OBJECT`], so the probe script's `callDBus` reaches us.
fn serve_hover(latest: Arc<Mutex<Option<WindowInfo>>>) -> zbus::Result<Connection> {
    Builder::session()?
        .name(hover_service())?
        .serve_at(HOVER_OBJECT, HoverSink { latest })?
        .build()
}

/// Extract a `String` from a D-Bus variant value, if it holds one.
fn owned_to_string(value: &OwnedValue) -> Option<String> {
    String::try_from(value.try_clone().ok()?).ok()
}
