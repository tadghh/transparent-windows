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
use tracing::{error, trace};
use tray_item::{IconSource, TIError, TrayItem};

/// A user command originating from the tray menu, sent to `main`'s message loop.
#[derive(Clone, Debug)]
pub enum Message {
    Quit,
    Add,
    Rules,
    Active,
    Startup,
}

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

/// Backend-assigned ids for the self-relabeling tray items, captured when the
/// menu is built so they can be updated when their state toggles. The ids can't
/// be hardcoded: the ksni and Windows backends number separators differently,
/// so a fixed index isn't portable across platforms.
struct MenuIds {
    active: u32,
    startup: u32,
}

fn setup_tray(
    tx: UnboundedSender<Message>,
    app_state: &AppState,
) -> Result<(TrayItem, MenuIds), TIError> {
    let mut tray = TrayItem::new(crate::identity::APP_NAME, tray_icon())?;

    add_tray_menu_item(&mut tray, "Add", &tx, Message::Add)?;
    add_tray_menu_item(&mut tray, "Rules", &tx, Message::Rules)?;

    let active = add_tray_menu_item_with_id(&mut tray, &app_state.active_label(), &tx, Message::Active)?;

    tray.inner_mut().add_separator()?;

    let startup =
        add_tray_menu_item_with_id(&mut tray, &app_state.startup_label(), &tx, Message::Startup)?;

    tray.inner_mut().add_separator()?;
    add_tray_menu_item(&mut tray, "Quit", &tx, Message::Quit)?;

    Ok((tray, MenuIds { active, startup }))
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
            error!(error = %e, label, "failed to send tray message");
        }
    })
}

/// Like [`add_tray_menu_item`], but for the self-relabeling items (`Active`,
/// `Startup`): the label is dynamic and the backend-assigned id is returned so
/// the caller can update the label later.
fn add_tray_menu_item_with_id(
    tray: &mut TrayItem,
    label: &str,
    tx: &UnboundedSender<Message>,
    message: Message,
) -> Result<u32, TIError> {
    let tx = tx.clone();
    tray.inner_mut().add_menu_item_with_id(label, move || {
        if let Err(e) = tx.send(message.clone()) {
            error!(error = %e, "failed to send tray message");
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
    let (mut tray, menu_ids) = match setup_tray(tx, &app_state) {
        Ok(tray) => tray,
        Err(e) => {
            error!(error = %e, "failed to set up tray");
            let _ = slint::quit_event_loop();
            return;
        }
    };

    while let Some(event) = rx.blocking_recv() {
        trace!(?event, "tray event");
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
            Message::Active => {
                let label = handle.block_on(app_state.toggle_active());
                if let Err(e) = tray
                    .inner_mut()
                    .set_menu_item_label(&label, menu_ids.active)
                {
                    error!(error = %e, "failed to update active label");
                }
            }
            Message::Startup => {
                if let Err(e) = tray
                    .inner_mut()
                    .set_menu_item_label(&app_state.toggle_autostart(), menu_ids.startup)
                {
                    error!(error = %e, "failed to update startup label");
                }
            }
        }
    }
}
