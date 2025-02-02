use crate::get_startup_state;
use crate::util::Message;
use tokio::sync::mpsc::UnboundedSender;
use tray_item::{IconSource, TIError, TrayItem};

pub fn setup_tray(tx: UnboundedSender<Message>) -> Result<TrayItem, TIError> {
    let mut tray = TrayItem::new("WinAlpha", IconSource::Resource("tray-default"))?;

    let add_tx = tx.clone();
    tray.add_menu_item("Add", move || {
        if let Err(e) = add_tx.send(Message::Add) {
            eprintln!("Failed to send Add message: {}", e);
        }
    })?;

    let rules_tx = tx.clone();
    tray.add_menu_item("Rules", move || {
        if let Err(e) = rules_tx.send(Message::Rules) {
            eprintln!("Failed to send Rules message: {}", e);
        }
    })?;

    tray.inner_mut().add_separator()?;

    let enable_tx = tx.clone();
    tray.add_menu_item("Enable", move || {
        if let Err(e) = enable_tx.send(Message::Enable) {
            eprintln!("Failed to send Enable message: {}", e);
        }
    })?;

    let disable_tx = tx.clone();
    tray.add_menu_item("Disable", move || {
        if let Err(e) = disable_tx.send(Message::Disable) {
            eprintln!("Failed to send Disable message: {}", e);
        }
    })?;

    let startup_string = if get_startup_state() {
        "Startup - True"
    } else {
        "Startup - False"
    };

    let startup_tx = tx.clone();
    tray.add_menu_item(startup_string, move || {
        if let Err(e) = startup_tx.send(Message::Startup) {
            eprintln!("Failed to send Disable message: {}", e);
        }
    })?;

    tray.inner_mut().add_separator()?;

    let quit_tx = tx.clone();
    tray.add_menu_item("Quit", move || {
        if let Err(e) = quit_tx.send(Message::Quit) {
            eprintln!("Failed to send Quit message: {}", e);
        }
    })?;

    Ok(tray)
}
