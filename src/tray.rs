use tokio::sync::mpsc::UnboundedSender;
use tray_item::{IconSource, TrayItem};

use crate::util::Message;

pub fn setup_tray(tx: UnboundedSender<Message>) -> TrayItem {
    let mut tray = TrayItem::new("Rust Transparency", IconSource::Resource("app-icon")).unwrap();

    tray.add_label("Tray Label").unwrap();

    tray.inner_mut().add_separator().unwrap();
    let add_tx = tx.clone();
    tray.add_menu_item("Add", move || {
        add_tx.send(Message::Add).unwrap();
    })
    .unwrap();
    let rules_tx = tx.clone();
    tray.add_menu_item("Rules", move || {
        rules_tx.send(Message::Rules).unwrap();
    })
    .unwrap();

    tray.inner_mut().add_separator().unwrap();

    let quit_tx = tx.clone();
    tray.add_menu_item("Quit", move || {
        quit_tx.send(Message::Quit).unwrap();
    })
    .unwrap();
    tray
}
