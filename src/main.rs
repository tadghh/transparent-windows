// #![windows_subsystem = "windows"]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use anyhow::Result;
use directories::ProjectDirs;
use std::{fs, path::PathBuf, sync::Arc};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

mod transparency;
mod tray;
mod util;
mod win_utils;

use transparency::{create_rules_window, monitor_windows};
use tray::setup_tray;
use util::{AppState, Config, Message};
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

fn load_config() -> (Config, PathBuf) {
    let project_dirs = ProjectDirs::from("com", "windowtransparency", "wintrans")
        .expect("Failed to get project directories");

    let config_dir = project_dirs.config_dir();

    fs::create_dir_all(config_dir).ok();

    let config_path = config_dir.join("config.json");

    if config_path.exists() {
        (
            serde_json::from_str(
                &fs::read_to_string(&config_path)
                    .ok()
                    .expect("failed to read config"),
            )
            .ok()
            .expect("Failed to read config"),
            config_path,
        )
    } else {
        (Config::new(), config_path)
    }
}
