#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

mod transparency;
mod tray;
mod util;
mod win_utils;

use transparency::{create_rules_window, monitor_windows};
use tray::setup_tray;
use util::{load_config, AppState, Message};
use win_utils::create_percentage_window;

slint::include_modules!();

#[tokio::main]
async fn main() -> Result<()> {
    let (config, config_path) = load_config();

    let (tx, mut rx): (UnboundedSender<Message>, UnboundedReceiver<Message>) =
        mpsc::unbounded_channel();

    let _tray = setup_tray(tx)?;

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
                    let app_state = Arc::clone(&app_state);
                    if let Err(e) = create_rules_window(app_state).await {
                        eprintln!("Error in rules window: {}", e);
                    }
                }
                Message::Add => {
                    if let Ok(window) = win_utils::get_window_under_cursor() {
                        let app_state = Arc::clone(&app_state);
                        if let Err(e) = create_percentage_window(window, app_state).await {
                            eprintln!("Error in selection window: {}", e);
                        }
                    }
                }
            }
        }
    }
}
