use crate::get_startup_state;
use crate::util::Message;
use tokio::sync::mpsc::UnboundedSender;
use tray_item::{IconSource, TIError, TrayItem};

// ID for startup menu item
pub const STARTUP_ID: u32 = 5;

pub fn setup_tray(tx: UnboundedSender<Message>) -> Result<TrayItem, TIError> {
    let mut tray = TrayItem::new("WinAlpha", IconSource::Resource("tray-default"))?;

    add_tray_menu_item(&mut tray, "Add", &tx, Message::Add)?;
    add_tray_menu_item(&mut tray, "Rules", &tx, Message::Rules)?;
    add_tray_menu_item(&mut tray, "Enable", &tx, Message::Enable)?;
    add_tray_menu_item(&mut tray, "Disable", &tx, Message::Disable)?;

    tray.inner_mut().add_separator()?;

    let startup_label = format!("Startup: {}", get_startup_state());

    let startup_tx = tx.clone();
    tray.add_menu_item(&startup_label, move || {
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
    tray.add_menu_item(&label, move || {
        if let Err(e) = tx_clone.send(message.clone()) {
            eprintln!("Failed to send {} message: {}", label, e);
        }
    })
}
