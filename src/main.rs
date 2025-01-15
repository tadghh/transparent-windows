// #![windows_subsystem = "windows"]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use anyhow::Result;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf, sync::Arc};
use tokio::sync::{
    mpsc::{self, UnboundedReceiver, UnboundedSender},
    RwLock,
};
use transparency::create_rules_window;
use tray_item::{IconSource, TrayItem};
use win_utils::{convert_to_human, create_percentage_window};

mod transparency;
mod win_utils;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Config {
    windows: HashMap<String, WindowConfig>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct WindowConfig {
    process_name: String,
    window_class: String,
    transparency: u8,
}

impl From<&WindowConfig> for TransparencyRule {
    fn from(config: &WindowConfig) -> Self {
        TransparencyRule {
            process_name: config.process_name.clone().into(),
            window_class: config.window_class.clone().into(),
            transparency: convert_to_human(config.transparency) as i32,
        }
    }
}

impl WindowConfig {
    pub fn new(process_name: String, window_class: String, transparency: u8) -> Self {
        Self {
            process_name,
            window_class,
            transparency,
        }
    }
    pub fn get_key(&self) -> String {
        format!("{}|{}", self.process_name, self.window_class)
    }
}

struct AppState {
    config: Arc<RwLock<Config>>,
    config_path: PathBuf,
}

enum Message {
    Quit,
    Add,
    Rules,
}

slint::include_modules!();

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
        (
            Config {
                windows: HashMap::new(),
            },
            config_path,
        )
    }
}

fn setup_tray(tx: UnboundedSender<Message>) -> TrayItem {
    let mut tray = TrayItem::new(
        "Tray Example",
        IconSource::Resource("name-of-icon-in-rc-file"),
    )
    .unwrap();

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

#[tokio::main]
async fn main() -> Result<()> {
    let (config, config_path) = load_config();

    let (tx, mut rx): (UnboundedSender<Message>, UnboundedReceiver<Message>) =
        mpsc::unbounded_channel();

    let _tray = setup_tray(tx);

    let app_state = Arc::new(AppState {
        config: Arc::new(RwLock::new(config)),
        config_path,
    });
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
                        let config_read = app_state.config.read().await;
                        (*config_read).clone()
                    };
                    println!("wh");
                    if let Ok(new_config) = create_rules_window(config) {
                        println!("yo0");
                        let mut config_write = app_state.config.write().await;
                        println!("yo");
                        *config_write = new_config;
                        if let Ok(config_json) = serde_json::to_string_pretty(&*config_write) {
                            println!("yo1");
                            if let Err(err) = fs::write(&app_state.config_path, config_json) {
                                println!("Failed to write config: {:?}", err);
                            }
                        }
                    }
                    println!("yo3")
                }
                Message::Add => {
                    println!("Click on a window to make it transparent...");
                    match win_utils::get_window_under_cursor() {
                        Ok(window) => match create_percentage_window(window.clone()) {
                            Some(num) => {
                                let window_config =
                                    WindowConfig::new(window.process_name, window.class_name, num);

                                {
                                    let mut config = app_state.config.write().await;
                                    config
                                        .windows
                                        .insert(window_config.get_key(), window_config);

                                    drop(config);
                                }

                                let config = app_state.config.read().await;
                                if let Ok(config_json) = serde_json::to_string_pretty(&*config) {
                                    if let Err(err) = fs::write(&app_state.config_path, config_json)
                                    {
                                        println!("Failed to write config: {:?}", err);
                                    }
                                }
                            }
                            None => println!("No percentage value rec."),
                        },
                        Err(err) => {
                            println!("{:?}", err);
                        }
                    }
                }
            },
            None => todo!(),
        }
    }
}
