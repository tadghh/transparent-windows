// #![windows_subsystem = "windows"]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use anyhow::Result;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use win_utils::create_percentage_window;

use tray_item::{IconSource, TrayItem};
use windows::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{GetClassNameW, GetWindowTextW, GetWindowThreadProcessId},
};

mod transparency;
mod win_utils;

#[derive(Debug, Serialize, Deserialize)]
struct Config {
    windows: HashMap<String, WindowConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WindowConfig {
    process_name: String,
    window_class: String,
    transparency: u8,
}

struct AppState {
    config: Arc<Mutex<Config>>,
    config_path: PathBuf,
}

#[derive(Debug)]
enum Message {
    Quit,
    Green,
    Red,
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
        config: Arc::new(Mutex::new(config)),
        config_path,
    });

    let mut tray = TrayItem::new(
        "Tray Example",
        IconSource::Resource("name-of-icon-in-rc-file"),
    )
    .unwrap();

    tray.add_label("Tray Label").unwrap();

    tray.add_menu_item("Hello", || {
        println!("Hello!");
    })
    .unwrap();

    tray.inner_mut().add_separator().unwrap();
    // Create channels for tray icon events
    let (tx, mut rx): (UnboundedSender<Message>, UnboundedReceiver<Message>) =
        mpsc::unbounded_channel();

    let red_tx = tx.clone();
    tray.add_menu_item("Red", move || {
        red_tx.send(Message::Red).unwrap();
    })
    .unwrap();
    let add_tx = tx.clone();
    tray.add_menu_item("Add", move || {
        add_tx.send(Message::Add).unwrap();
    })
    .unwrap();

    let green_tx = tx.clone();
    tray.add_menu_item("Green", move || {
        green_tx.send(Message::Green).unwrap();
    })
    .unwrap();

    tray.inner_mut().add_separator().unwrap();

    let quit_tx = tx.clone();
    tray.add_menu_item("Quit", move || {
        quit_tx.send(Message::Quit).unwrap();
    })
    .unwrap();

    let monitor_state = app_state.clone();
    thread::spawn(move || {
        transparency::monitor_windows(monitor_state);
    });

    loop {
        match rx.recv().await {
            Some(event) => {
                match event {
                    Message::Quit => {
                        println!("Quit");
                        break;
                    }
                    Message::Green => todo!(),
                    Message::Red => {
                        println!("Red");
                        tray.set_icon(IconSource::Resource("another-name-from-rc-file"))
                            .unwrap();
                    }
                    Message::Add => {
                        println!("Click on a window to make it transparent...");
                        // Handle adding new window
                        if let Ok(window) = win_utils::get_window_under_cursor() {
                            // For now, let's use a default transparency value of 80%

                            let num = create_percentage_window(window.clone()).unwrap();
                            println!("Set transparency to {:?}", num);
                            // Update configuration
                            let mut config = app_state.config.lock().unwrap();
                            let key = format!("{}|{}", window.process_name, window.class_name);

                            config.windows.insert(
                                key,
                                WindowConfig {
                                    process_name: window.process_name,
                                    window_class: window.class_name.clone(),
                                    transparency: num,
                                },
                            );

                            if let Ok(config_json) = serde_json::to_string_pretty(&*config) {
                                match fs::write(&app_state.config_path, config_json) {
                                    Ok(()) => (),
                                    Err(err) => println!("aaaa {:?}", err),
                                }
                            }

                            println!("Added transparency rule for window: {}", "title");
                        }
                    }
                }
            }
            None => todo!(),
        }
    }

    // Event loop

    Ok(())
}
