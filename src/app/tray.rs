use crate::{
    app::state::AppState,
    platform::{Opacity, wm},
    transparency,
};
use std::sync::Arc;
use tokio::{
    runtime::Handle,
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
};
use tray_item::{IconSource, TIError, TrayItem};

/// A user command originating from the tray menu, sent to `main`'s message loop.
#[derive(Clone)]
pub enum Message {
    Quit,
    Add,
    Rules,
    Enable,
    Disable,
    Startup,
}

// Menu index of the "Startup" item, used to relabel it when autostart toggles.
const STARTUP_ID: u32 = 5;

/// The build-time ARGB pixmap for the Linux tray (see `build.rs`).
#[cfg(unix)]
mod tray_icon_data {
    include!(concat!(env!("OUT_DIR"), "/tray_icon.rs"));
}

/// The tray icon for this platform. Windows loads the `tray-default` icon
/// embedded as a PE resource; Linux/ksni has no such mechanism, so it gets the
/// raw ARGB pixmap decoded from the PNG at build time.
fn tray_icon() -> IconSource {
    #[cfg(windows)]
    {
        IconSource::Resource("tray-default")
    }
    #[cfg(unix)]
    {
        IconSource::Data {
            data: tray_icon_data::ARGB.to_vec(),
            width: tray_icon_data::WIDTH,
            height: tray_icon_data::HEIGHT,
        }
    }
}

fn setup_tray(tx: UnboundedSender<Message>, app_state: &AppState) -> Result<TrayItem, TIError> {
    let mut tray = TrayItem::new(crate::identity::APP_NAME, tray_icon())?;

    add_tray_menu_item(&mut tray, "Add", &tx, Message::Add)?;
    add_tray_menu_item(&mut tray, "Rules", &tx, Message::Rules)?;
    add_tray_menu_item(&mut tray, "Enable", &tx, Message::Enable)?;
    add_tray_menu_item(&mut tray, "Disable", &tx, Message::Disable)?;

    tray.inner_mut().add_separator()?;

    let startup_tx = tx.clone();

    tray.add_menu_item(&app_state.startup_label(), move || {
        if let Err(e) = startup_tx.send(Message::Startup) {
            eprintln!("Failed to send Startup message: {}", e);
        }
    })?;

    tray.inner_mut().add_separator()?;
    add_tray_menu_item(&mut tray, "Quit", &tx, Message::Quit)?;

    Ok(tray)
}

fn add_tray_menu_item(
    tray: &mut TrayItem,
    label: &'static str,
    tx: &UnboundedSender<Message>,
    message: Message,
) -> Result<(), TIError> {
    let tx_clone = tx.clone();
    tray.add_menu_item(label, move || {
        if let Err(e) = tx_clone.send(message.clone()) {
            eprintln!("Failed to send {} message: {}", label, e);
        }
    })
}

/// Spawns the tray on its own thread, owning its message channel end to end. The
/// thread builds the (`!Send`) [`TrayItem`] and runs the dispatch loop until
/// [`Message::Quit`]; `handle` drives async work on the background runtime.
pub fn run(app_state: Arc<AppState>, handle: Handle) {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || message_loop(rx, tx, app_state, handle));
}

/// Sets up the tray, then receives its messages and dispatches them. UI work is
/// marshalled onto the event-loop thread with `invoke_from_event_loop`; async
/// work is driven on the background runtime via `handle`. Runs on its own thread
/// until [`Message::Quit`].
fn message_loop(
    mut rx: UnboundedReceiver<Message>,
    tx: UnboundedSender<Message>,
    app_state: Arc<AppState>,
    handle: Handle,
) {
    let mut tray = match setup_tray(tx, &app_state) {
        Ok(tray) => tray,
        Err(e) => {
            eprintln!("Failed to set up tray: {e}");
            let _ = slint::quit_event_loop();
            return;
        }
    };

    while let Some(event) = rx.blocking_recv() {
        match event {
            Message::Quit => {
                if let Opacity::Compositor(compositor) = wm().opacity() {
                    let _ = compositor.sync_rules(&[]);
                }
                app_state.shutdown.notify_waiters();
                let _ = slint::quit_event_loop();
                break;
            }
            Message::Rules => {
                let rules = handle.block_on(app_state.get_window_rules());
                let state = app_state.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    transparency::show_rules_window(rules, state);
                });
            }
            Message::Add => transparency::start_add_flow(app_state.clone()),
            Message::Enable => handle.block_on(app_state.enabled()),
            Message::Disable => handle.block_on(app_state.disable()),
            Message::Startup => {
                if let Err(e) = tray
                    .inner_mut()
                    .set_menu_item_label(&app_state.toggle_autostart(), STARTUP_ID)
                {
                    eprintln!("Failed to update startup label: {e}");
                }
            }
        }
    }
}
