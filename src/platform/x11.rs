//! X11 / XWayland implementation of [`WindowManager`].
//!
//! Opacity is driven through the `_NET_WM_WINDOW_OPACITY` EWMH property, which a
//! running compositor (picom, KWin, mutter, ...) honours. Without a compositor
//! the property is set but has no visible effect. Native-Wayland windows are not
//! reachable through this mechanism — only X11/XWayland clients respond.

// TODO this is just gross

use super::{CursorPoint, WindowHandle, WindowInfo, WindowManager, linux_common};
use anyhow::{Result, anyhow};
use x11rb::{
    connection::Connection,
    protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, KeyButMask, PropMode, Window},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

/// Fully-opaque value for `_NET_WM_WINDOW_OPACITY`.
const OPAQUE: u32 = 0xFFFF_FFFF;

struct Atoms {
    net_wm_window_opacity: Atom,
    net_client_list: Atom,
    net_wm_pid: Atom,
    wm_state: Atom,
}

pub struct X11Manager {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
}

impl X11Manager {
    pub fn new() -> Result<Self> {
        let (conn, screen_num) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen_num].root;

        let atoms = Atoms {
            net_wm_window_opacity: intern(&conn, b"_NET_WM_WINDOW_OPACITY")?,
            net_client_list: intern(&conn, b"_NET_CLIENT_LIST")?,
            net_wm_pid: intern(&conn, b"_NET_WM_PID")?,
            wm_state: intern(&conn, b"WM_STATE")?,
        };

        Ok(Self { conn, root, atoms })
    }

    /// Top-level managed client windows, per `_NET_CLIENT_LIST`.
    fn client_list(&self) -> Result<Vec<Window>> {
        let reply = self
            .conn
            .get_property(
                false,
                self.root,
                self.atoms.net_client_list,
                AtomEnum::WINDOW,
                0,
                u32::MAX,
            )?
            .reply()?;

        Ok(reply.value32().map(|it| it.collect()).unwrap_or_default())
    }

    /// The class component of a window's `WM_CLASS` (the second NUL-terminated string).
    fn window_class(&self, window: Window) -> Option<String> {
        let reply = self
            .conn
            .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
            .ok()?
            .reply()
            .ok()?;

        // WM_CLASS is "instance\0class\0".
        let mut parts = reply.value.split(|&b| b == 0);
        let _instance = parts.next();
        parts
            .next()
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
    }

    /// A window's owning process id, per `_NET_WM_PID`.
    fn window_pid(&self, window: Window) -> Option<u32> {
        let reply = self
            .conn
            .get_property(
                false,
                window,
                self.atoms.net_wm_pid,
                AtomEnum::CARDINAL,
                0,
                1,
            )
            .ok()?
            .reply()
            .ok()?;

        reply.value32().and_then(|mut it| it.next())
    }

    /// Whether a window carries the ICCCM `WM_STATE` marker of a managed client.
    fn has_wm_state(&self, window: Window) -> bool {
        self.conn
            .get_property(false, window, self.atoms.wm_state, AtomEnum::ANY, 0, 0)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|reply| reply.type_ != x11rb::NONE)
    }

    /// Descend from a (possibly WM-frame) window to the real client window.
    fn find_client_window(&self, window: Window) -> Option<Window> {
        if self.has_wm_state(window) {
            return Some(window);
        }

        let tree = self.conn.query_tree(window).ok()?.reply().ok()?;
        for &child in &tree.children {
            if let Some(found) = self.find_client_window(child) {
                return Some(found);
            }
        }
        None
    }
}

impl WindowManager for X11Manager {
    fn set_window_alpha(&self, handle: WindowHandle, alpha: u8) -> Result<()> {
        let window = handle.0 as Window;
        let cardinal = alpha_to_cardinal(alpha);

        self.conn.change_property32(
            PropMode::REPLACE,
            window,
            self.atoms.net_wm_window_opacity,
            AtomEnum::CARDINAL,
            &[cardinal],
        )?;
        self.conn.flush()?;
        Ok(())
    }

    fn enumerate_windows(&self, process_name: &str, window_class: &str) -> Vec<WindowHandle> {
        let windows = match self.client_list() {
            Ok(windows) => windows,
            Err(_) => return Vec::new(),
        };

        windows
            .into_iter()
            .filter(|&win| {
                let class_ok = self.window_class(win).as_deref() == Some(window_class);
                let process_ok = self
                    .window_pid(win)
                    .and_then(|pid| self.process_name_from_pid(pid).ok())
                    .is_some_and(|name| name == process_name);

                class_ok && process_ok
            })
            .map(|win| WindowHandle(win as u64))
            .collect()
    }

    fn find_parent_from_child_class(
        &self,
        child_class: &str,
    ) -> Result<Option<(WindowHandle, String)>> {
        // `_NET_CLIENT_LIST` already yields top-level client windows, so the
        // "parent" of a matching client is the client itself.
        for win in self.client_list()? {
            if let Some(class) = self.window_class(win) {
                if class == child_class {
                    return Ok(Some((WindowHandle(win as u64), class)));
                }
            }
        }
        Ok(None)
    }

    fn get_cursor_pos(&self) -> Result<CursorPoint> {
        let pointer = self.conn.query_pointer(self.root)?.reply()?;
        Ok(CursorPoint {
            x: pointer.root_x as i32,
            y: pointer.root_y as i32,
        })
    }

    fn is_left_click(&self) -> bool {
        self.conn
            .query_pointer(self.root)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|pointer| u16::from(pointer.mask) & u16::from(KeyButMask::BUTTON1) != 0)
    }

    fn get_window_info_at(&self, _point: CursorPoint) -> Result<WindowInfo> {
        let pointer = self.conn.query_pointer(self.root)?.reply()?;
        if pointer.child == x11rb::NONE {
            return Err(anyhow!("No window found at cursor position."));
        }

        let client = self
            .find_client_window(pointer.child)
            .unwrap_or(pointer.child);

        Ok(WindowInfo {
            class_name: self.window_class(client).unwrap_or_default(),
            process_name: self
                .window_pid(client)
                .and_then(|pid| self.process_name_from_pid(pid).ok())
                .unwrap_or_default(),
        })
    }

    fn is_elevated_at(&self, _point: CursorPoint) -> bool {
        // No UAC analogue on Linux; opacity changes aren't privilege-gated.
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
}

fn intern(conn: &RustConnection, name: &[u8]) -> Result<Atom> {
    Ok(conn.intern_atom(false, name)?.reply()?.atom)
}

/// Map a 0-255 alpha (255 = opaque) to a `_NET_WM_WINDOW_OPACITY` cardinal.
fn alpha_to_cardinal(alpha: u8) -> u32 {
    if alpha == 255 {
        OPAQUE
    } else {
        ((alpha as u64 * OPAQUE as u64) / 255) as u32
    }
}
