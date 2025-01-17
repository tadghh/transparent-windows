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

use transparency::create_rules_window;
use tray::setup_tray;
use util::{AppState, Config, Message, WindowConfig};
use win_utils::create_percentage_window;

slint::include_modules!();

#[tokio::main]
async fn main() -> Result<()> {
    let (config, config_path) = load_config();

    let (tx, mut rx): (UnboundedSender<Message>, UnboundedReceiver<Message>) =
        mpsc::unbounded_channel();

    let _tray = setup_tray(tx);

    let app_state = Arc::new(AppState::new(config, config_path));
    let clone_state = app_state.clone();

    tokio::spawn(async move {
        transparency::monitor_windows(clone_state).await;
    });

    loop {
        match rx.recv().await {
            Some(event) => match event {
                Message::Quit => {
                    return Ok(());
                }
                Message::Rules => {
                    let config = {
                        let config_read = app_state.get_config().read().await;
                        (*config_read).clone()
                    };

                    if let Ok(new_config) = create_rules_window(config) {
                        let mut config_write = app_state.get_config().write().await;

                        *config_write = new_config;
                        if let Ok(config_json) = serde_json::to_string_pretty(&*config_write) {
                            fs::write(&app_state.get_config_path(), config_json)?
                        }
                    }
                }
                Message::Add => match win_utils::get_window_under_cursor() {
                    Ok(window) => match create_percentage_window(window.clone()) {
                        Some(num) => {
                            let window_config =
                                WindowConfig::new(window.process_name, window.class_name, num);

                            {
                                let mut config = app_state.get_config().write().await;

                                config
                                    .get_windows()
                                    .insert(window_config.get_key(), window_config);

                                drop(config);
                            }

                            let config = app_state.get_config().read().await;
                            if let Ok(config_json) = serde_json::to_string_pretty(&*config) {
                                fs::write(&app_state.get_config_path(), config_json)?
                            }
                        }
                        None => println!("No percentage value rec."),
                    },
                    Err(err) => {
                        println!("{:?}", err);
                    }
                },
            },
            None => todo!(),
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
