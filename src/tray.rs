use crate::util::Message;

use tokio::sync::mpsc::UnboundedSender;
use tray_item::{IconSource, TIError, TrayItem};

#[allow(unused_must_use)]
pub fn setup_tray(tx: UnboundedSender<Message>) -> Result<TrayItem, TIError> {
    let mut tray = TrayItem::new("Rust Transparency", IconSource::Resource("app-icon"))?;

    let add_tx = tx.clone();
    tray.add_menu_item("Add", move || {
        add_tx.send(Message::Add);
    })?;

    let rules_tx = tx.clone();
    tray.add_menu_item("Rules", move || {
        rules_tx.send(Message::Rules);
    })?;

    tray.inner_mut().add_separator()?;

    let quit_tx = tx.clone();
    tray.add_menu_item("Quit", move || {
        quit_tx.send(Message::Quit);
    })?;

    Ok(tray)
}
