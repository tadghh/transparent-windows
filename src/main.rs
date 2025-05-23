#![windows_subsystem = "windows"]
#![feature(let_chains)]
use anyhow::Result;
use app_state::AppState;
use monitor::monitor_windows;
use std::sync::Arc;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tray::{setup_tray, STARTUP_ID};
use util::{load_config, Message};
use win_utils::{change_startup, get_startup_state};
mod app_state;
mod monitor;
mod transparency;
mod tray;
mod util;
mod win_utils;
mod window_config;

slint::include_modules!();

#[cfg(target_os = "windows")]
#[tokio::main]
async fn main() -> Result<()> {
    let (config, config_path) = load_config();
    let (tx, mut rx): (UnboundedSender<Message>, UnboundedReceiver<Message>) =
        mpsc::unbounded_channel();

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
                    app_state.quit().await;
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
                    app_state.enabled().await;
                }
                Message::Disable => {
                    app_state.disable().await;
                }
                Message::Startup => {
                    _ = change_startup(!get_startup_state());
                    let state_string = format!("Startup - {}", get_startup_state());
                    tray.inner_mut()
                        .set_menu_item_label(&state_string, STARTUP_ID)?;
                }
            }
        }
    }
}
