use crate::{window_config::WindowConfig, ConfigWindow};
use anyhow::{anyhow, Error};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use slint::ComponentHandle;
use std::{
    collections::HashMap,
    fs::{self, create_dir_all},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use windows::{
    core::{w, PCWSTR},
    Win32::UI::Shell::ShellExecuteW,
};

#[derive(Clone)]
pub enum Message {
    Quit,
    Add,
    Rules,
    Enable,
    Disable,
    Startup,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    windows: HashMap<String, WindowConfig>,
}

impl Config {
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
        }
    }

    pub fn get_windows(&mut self) -> &mut HashMap<String, WindowConfig> {
        &mut self.windows
    }

    pub fn get_windows_non_mut(&self) -> &HashMap<String, WindowConfig> {
        &self.windows
    }
}

pub fn create_config_error_window(config_path: PathBuf) -> Result<(), Error> {
    let config_path = config_path
        .into_os_string()
        .into_string()
        .map_err(|os_str| anyhow!("Invalid UTF-8 in path: {:?}", os_str))?;

    let window = ConfigWindow::new()?;
    let window_handle = window.as_weak();

    let config_clone = Arc::new(Mutex::new(config_path));
    let action_taken = Arc::new(Mutex::new(false));
    let action_taken_clone = action_taken.clone();
    let config_clone_submit = config_clone.clone();
    let config_clone_cancel = config_clone.clone();

    window.on_submit(move |value| match value {
        crate::Action::Edit => unsafe {
            // shhh its okay sometimes
            *action_taken.lock().unwrap() = true;
            match config_clone_submit.lock() {
                Ok(path) => {
                    ShellExecuteW(
                        None,
                        w!("open"),
                        PCWSTR::from_raw(
                            path.as_str()
                                .encode_utf16()
                                .chain(Some(0))
                                .collect::<Vec<u16>>()
                                .as_ptr(),
                        ),
                        None,
                        None,
                        windows::Win32::UI::WindowsAndMessaging::SHOW_WINDOW_CMD(1),
                    );
                }
                Err(_) => {
                    _ = anyhow!("AHHHHH");
                }
            };
        },
        crate::Action::Reset => {
            match action_taken.lock() {
                Ok(mut action_state) => {
                    *action_state = true;
                }
                Err(e) => {
                    _ = anyhow!("AHHHHH {}", e);
                    println!("yo");
                }
            }

            if let Ok(config_json) = serde_json::to_string_pretty(&[serde_json::json!({})])
                && let Ok(config_clone) = config_clone.lock()
            {
                fs::write(&config_clone.as_str(), config_json).expect("better not.");
            } else {
                _ = anyhow!("AHHHHH failed to lock/write config!!!");
            }
        }
    });

    window.on_cancel(move || {
        if !*action_taken_clone.lock().unwrap() {
            println!("false");
            if let Ok(config_clone) = config_clone_cancel.lock()
                && let Ok(config_json) = serde_json::to_string_pretty(&[serde_json::json!({})])
            {
                fs::write(config_clone.as_str(), config_json).expect("better not.");
            }
        }

        if let Some(window) = window_handle.upgrade() {
            let _ = window.hide();
        }
    });
    println!("false");
    window.run()?;
    Ok(())
}

pub fn load_config() -> (Config, PathBuf) {
    let project_dirs = ProjectDirs::from("com", "windowtransparency", "winalpha")
        .expect("Failed to get project config directories.");

    let config_dir = project_dirs.config_dir();

    create_dir_all(config_dir).ok();

    let config_path = config_dir.join("config.json");

    if config_path.exists()
        && let Ok(config_data) = fs::read_to_string(&config_path)
    {
        if let Ok(existing) = serde_json::from_str::<Config>(&config_data) {
            (existing, config_path)
        } else {
            match create_config_error_window(config_path) {
                Ok(_) => (),
                Err(e) => eprintln!("Error: {}", e),
            }
            load_config()
        }
    } else {
        (Config::new(), config_path)
    }
}
