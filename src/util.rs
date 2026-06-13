use crate::{ConfigWindow, window_config::WindowConfig};
use anyhow::anyhow;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use serde_json::from_str;
use slint::ComponentHandle;
use std::{
    collections::HashMap,
    fs::{self, create_dir_all},
    path::PathBuf,
    sync::{Arc, Mutex},
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

/// Shows the config-recovery window. Must be called on the Slint event-loop.
pub fn show_config_error_window(config_path: PathBuf) {
    let config_path = match config_path.into_os_string().into_string() {
        Ok(path) => path,
        Err(os_str) => {
            eprintln!("Invalid UTF-8 in config path: {:?}", os_str);
            return;
        }
    };

    let window = match ConfigWindow::new() {
        Ok(window) => window,
        Err(e) => {
            eprintln!("Failed to create config error window: {e}");
            return;
        }
    };
    let window_handle = window.as_weak();

    let config_clone = Arc::new(Mutex::new(config_path));
    let action_taken = Arc::new(Mutex::new(false));

    let action_taken_clone = action_taken.clone();
    let config_clone_submit = config_clone.clone();
    let config_clone_cancel = config_clone.clone();

    window.on_submit(move |value| match value {
        crate::Action::Edit => {
            *action_taken.lock().unwrap() = true;
            match config_clone_submit.lock() {
                Ok(path) => {
                    _ = crate::platform::wm().open_path(path.as_str());
                }
                Err(_) => {
                    _ = anyhow!("AHHHHH");
                }
            };
        }
        crate::Action::Reset => {
            match action_taken.lock() {
                Ok(mut action_state) => {
                    *action_state = true;
                }
                Err(e) => {
                    _ = anyhow!("AHHHHH {}", e);
                }
            }

            if let Ok(config_json) = serde_json::to_string_pretty(&[serde_json::json!({})])
                && let Ok(config_clone) = config_clone.lock()
            {
                fs::write(config_clone.as_str(), config_json).expect("better not.");
            } else {
                _ = anyhow!("AHHHHH failed to lock/write config!!!");
            }
        }
    });

    window.on_cancel(move || {
        // TODO bleh, hopefully we dont unwrap; this is unlikely
        if !*action_taken_clone.lock().unwrap()
            && let Ok(config_clone) = config_clone_cancel.lock()
            && let Ok(config_json) = serde_json::to_string_pretty(&[serde_json::json!({})])
        {
            fs::write(config_clone.as_str(), config_json).expect("better not.");
        }

        if let Some(handle) = window_handle.upgrade() {
            _ = handle.hide();
        }
    });

    if let Err(e) = window.show() {
        eprintln!("Failed to show config error window: {e}");
        return;
    }
    crate::ui::keep_alive("config_error", window);
}

/// Loads the config. The returned bool is true when the file existed but failed
/// to parse — the caller should surface the recovery window via
/// [`show_config_error_window`].
pub fn load_config() -> (Config, PathBuf, bool) {
    let project_dirs = ProjectDirs::from("com", "windowtransparency", "winalpha")
        .expect("Failed to get project config directories.");

    let config_dir = project_dirs.config_dir();

    create_dir_all(config_dir).ok();

    // TODO: toucou
    let config_path = config_dir.join("config.json");
    if config_path.exists()
        && let Ok(config_data) = fs::read_to_string(&config_path)
    {
        if let Ok(existing) = from_str::<Config>(&config_data) {
            (existing, config_path, false)
        } else {
            (Config::new(), config_path, true)
        }
    } else {
        (Config::new(), config_path, false)
    }
}
