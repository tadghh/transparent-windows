#![windows_subsystem = "windows"]
#![feature(let_chains)]
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

mod app_state;
mod monitor;
mod transparency;
mod tray;
mod util;
mod win_utils;
mod window_config;
use app_state::AppState;
use monitor::monitor_windows;
use tray::setup_tray;
use util::{load_config, Message};

use win_utils::{change_startup, get_startup_state};
slint::include_modules!();

#[cfg(target_os = "windows")]
#[tokio::main]
async fn main() -> Result<()> {
    let (tx, mut rx): (UnboundedSender<Message>, UnboundedReceiver<Message>) =
        mpsc::unbounded_channel();

    let (config, config_path) = load_config();

    let mut tray = setup_tray(tx.clone())?;

    let app_state = Arc::new(AppState::new(config, config_path));
    let clone_state = app_state.clone();

    tokio::spawn(async move {
        monitor_windows(clone_state).await;
    });

    loop {
        if let Some(event) = rx.recv().await {
            match event {
                Message::Quit => {
                    return Ok(());
                }
                Message::Rules => {
                    if let Err(e) = app_state.show_rules_window().await {
                        eprintln!("Error in rules window: {}", e);
                    }
                }
                Message::Add => {
                    if let Err(e) = app_state.add_window_rule().await {
                        eprintln!("Error in selection window: {}", e);
                    }
                }
                Message::Enable => {
                    app_state.set_enable_state(true).await;
                }
                Message::Disable => {
                    app_state.set_enable_state(false).await;
                }
                Message::Startup => {
                    if !get_startup_state() {
                        tray.inner_mut().set_menu_item_label("Startup - True", 5)?;
                    } else {
                        tray.inner_mut().set_menu_item_label("Startup - False", 5)?;
                    }
                    let _ = change_startup(!get_startup_state());
                }
            }
        }
    }
}
