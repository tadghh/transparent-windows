#![windows_subsystem = "windows"]
use anyhow::Result;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf, sync::Arc};
use tokio::sync::{
    mpsc::{self, UnboundedReceiver, UnboundedSender},
    RwLock,
};
use tray_item::{IconSource, TrayItem};
use win_utils::create_percentage_window;

mod transparency;
mod win_utils;

#[derive(Serialize, Deserialize)]
struct Config {
    windows: HashMap<String, WindowConfig>,
}

#[derive(Serialize, Deserialize)]
struct WindowConfig {
    process_name: String,
    window_class: String,
    transparency: u8,
}

struct AppState {
    config: Arc<RwLock<Config>>,
    config_path: PathBuf,
}

enum Message {
    Quit,
    Add,
}

slint::include_modules!();

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize config directory and load configuration
    let project_dirs = ProjectDirs::from("com", "windowtransparency", "wintrans")
        .expect("Failed to get project directories");
    let config_dir = project_dirs.config_dir();
    fs::create_dir_all(config_dir)?;
    let config_path = config_dir.join("config.json");

    let config = if config_path.exists() {
        serde_json::from_str(&fs::read_to_string(&config_path)?)?
    } else {
        Config {
            windows: HashMap::new(),
        }
    };

    let app_state = Arc::new(AppState {
        config: Arc::new(RwLock::new(config)),
        config_path,
    });
    let clone_state = app_state.clone();

    tokio::spawn(async move {
        transparency::monitor_windows(clone_state).await;
    });

    let mut tray = TrayItem::new(
        "Tray Example",
        IconSource::Resource("name-of-icon-in-rc-file"),
    )
    .unwrap();

    tray.add_label("Tray Label").unwrap();

    tray.inner_mut().add_separator().unwrap();
    // Create channels for tray icon events
    let (tx, mut rx): (UnboundedSender<Message>, UnboundedReceiver<Message>) =
        mpsc::unbounded_channel();

    let add_tx = tx.clone();
    tray.add_menu_item("Add", move || {
        add_tx.send(Message::Add).unwrap();
    })
    .unwrap();

    tray.inner_mut().add_separator().unwrap();

    let quit_tx = tx.clone();
    tray.add_menu_item("Quit", move || {
        quit_tx.send(Message::Quit).unwrap();
    })
    .unwrap();

    loop {
        match rx.recv().await {
            Some(event) => {
                match event {
                    Message::Quit => {
                        return Ok(());
                    }
                    Message::Add => {
                        println!("Click on a window to make it transparent...");

                        if let Ok(window) = win_utils::get_window_under_cursor() {
                            let num = match create_percentage_window(window.clone()) {
                                // Remove clone
                                Some(n) => n,
                                None => core::u8::MAX,
                            };

                            let window_config = WindowConfig {
                                process_name: window.process_name,
                                window_class: window.class_name.clone(),
                                transparency: num,
                            };
                            let key = format!(
                                "{}|{}",
                                window_config.process_name, window_config.window_class
                            );

                            {
                                let mut config = app_state.config.write().await;
                                config.windows.insert(key, window_config);

                                drop(config);
                            }

                            let config = app_state.config.read().await;
                            if let Ok(config_json) = serde_json::to_string_pretty(&*config) {
                                if let Err(err) = fs::write(&app_state.config_path, config_json) {
                                    println!("Failed to write config: {:?}", err);
                                }
                            }

                            drop(config);
                        }
                    }
                }
            }
            None => todo!(),
        }
    }
}
