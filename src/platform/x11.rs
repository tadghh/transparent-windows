use super::{
    CursorPicker, CursorPoint, Opacity, OperatingSystem, Os, PickHover, PollingOpacity,
    WindowHandle, WindowInfo, WindowManager, run_cursor_pick,
};
use anyhow::{Result, anyhow};
use std::sync::Arc;
use tokio::sync::Notify;
use tracing::{debug, warn};
use x11rb::{
    connection::Connection,
    protocol::{
        Event,
        xproto::{
            Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, EventMask, KeyButMask,
            PropMode, Window,
        },
    },
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
    /// Fires when the set of open windows may have changed (see
    /// [`spawn_window_change_listener`]). `None` if the listener couldn't be
    /// started, in which case the monitor falls back to periodic polling.
    window_changed: Option<Arc<Notify>>,
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

        // A dedicated connection listens for window open/close so the monitor can
        // react to events instead of polling on a fixed interval. If it can't be
        // set up, fall back to polling (the monitor handles `None`).
        let window_changed = match spawn_window_change_listener() {
            Ok(signal) => {
                debug!("X11 window-change listener active");
                Some(signal)
            }
            Err(e) => {
                warn!(error = %e, "X11 window-change listener unavailable; falling back to polling");
                None
            }
        };

        Ok(Self {
            conn,
            root,
            atoms,
            window_changed,
        })
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

    fn window_class(&self, window: Window) -> Option<String> {
        let reply = self
            .conn
            .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 1024)
            .ok()?
            .reply()
            .ok()?;

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

    /// Whether the given pointer button is currently held, per the root pointer
    /// query mask.
    fn pointer_button_held(&self, button: KeyButMask) -> bool {
        self.conn
            .query_pointer(self.root)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|pointer| u16::from(pointer.mask) & u16::from(button) != 0)
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

impl PollingOpacity for X11Manager {
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

    fn window_change_signal(&self) -> Option<Arc<Notify>> {
        self.window_changed.clone()
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
                    .and_then(|pid| Os::process_name_from_pid(pid).ok())
                    .is_some_and(|name| name == process_name);

                class_ok && process_ok
            })
            .map(|win| WindowHandle(win as u64))
            .collect()
    }
}

impl CursorPicker for X11Manager {
    fn get_cursor_pos(&self) -> Result<CursorPoint> {
        let pointer = self.conn.query_pointer(self.root)?.reply()?;
        Ok(CursorPoint {
            x: pointer.root_x as i32,
            y: pointer.root_y as i32,
        })
    }

    fn is_left_click(&self) -> bool {
        self.pointer_button_held(KeyButMask::BUTTON1)
    }

    fn is_cancel_requested(&self) -> bool {
        // Right-click cancels the pick.
        self.pointer_button_held(KeyButMask::BUTTON3)
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
                .and_then(|pid| Os::process_name_from_pid(pid).ok())
                .unwrap_or_default(),
        })
    }

    fn is_elevated_at(&self, _point: CursorPoint) -> bool {
        false
    }

    fn is_running_as_admin(&self) -> bool {
        false
    }
}

impl WindowManager for X11Manager {
    fn find_parent_from_child_class(
        &self,
        child_class: &str,
    ) -> Result<Option<(WindowHandle, String)>> {
        for win in self.client_list()? {
            if let Some(class) = self.window_class(win)
                && class == child_class
            {
                return Ok(Some((WindowHandle(win as u64), class)));
            }
        }
        Ok(None)
    }

    fn opacity(&self) -> Opacity<'_> {
        Opacity::Polling(self)
    }

    fn pick_window(&self, on_hover: &(dyn Fn(PickHover) + Sync)) -> Result<Option<WindowInfo>> {
        run_cursor_pick(self, on_hover)
    }
}

/// Open a second X connection, subscribe to root-window structure and property
/// changes, and spawn a thread that signals `Notify` whenever the set of open
/// windows may have changed (a window mapped/unmapped/created/destroyed, or the
/// WM rewriting `_NET_CLIENT_LIST`). A separate connection is used so the blocking
/// `wait_for_event` loop never steals replies from the request/reply connection.
/// The signal coalesces bursts; the monitor pairs it with a slow periodic
/// re-apply so a missed event still self-corrects.
fn spawn_window_change_listener() -> Result<Arc<Notify>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;
    let net_client_list = intern(&conn, b"_NET_CLIENT_LIST")?;

    conn.change_window_attributes(
        root,
        &ChangeWindowAttributesAux::new()
            .event_mask(EventMask::PROPERTY_CHANGE | EventMask::SUBSTRUCTURE_NOTIFY),
    )?
    .check()?;
    conn.flush()?;

    let signal = Arc::new(Notify::new());
    let thread_signal = signal.clone();
    std::thread::spawn(move || {
        while let Ok(event) = conn.wait_for_event() {
            let relevant = match event {
                // Property changes are noisy (focus, desktop, …); only the client
                // list signals a window opening or closing.
                Event::PropertyNotify(e) => e.atom == net_client_list,
                Event::MapNotify(_)
                | Event::UnmapNotify(_)
                | Event::CreateNotify(_)
                | Event::DestroyNotify(_)
                | Event::ReparentNotify(_) => true,
                _ => false,
            };
            if relevant {
                thread_signal.notify_one();
            }
        }
        // The loop ends only if the connection drops (e.g. the X server exits),
        // at which point the whole app is going down anyway.
    });

    Ok(signal)
}

fn intern(conn: &RustConnection, name: &[u8]) -> Result<Atom> {
    Ok(conn.intern_atom(false, name)?.reply()?.atom)
}

fn alpha_to_cardinal(alpha: u8) -> u32 {
    super::rescale(alpha as u32, u8::MAX as u32, OPAQUE)
}
