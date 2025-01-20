use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, create_dir_all};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::win_utils::convert_to_human;
use crate::TransparencyRule;

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

pub fn load_config() -> (Config, PathBuf) {
    let project_dirs = ProjectDirs::from("com", "windowtransparency", "wintrans")
        .expect("Failed to get project config directories.");

    let config_dir = project_dirs.config_dir();

    create_dir_all(config_dir).ok();

    let config_path = config_dir.join("config.json");

    if config_path.exists() {
        (
            serde_json::from_str(
                &fs::read_to_string(&config_path)
                    .ok()
                    .expect("Failed to read config path."),
            )
            .ok()
            .expect("Failed to read config file."),
            config_path,
        )
    } else {
        (Config::new(), config_path)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WindowConfig {
    #[serde(default)]
    process_name: String,
    #[serde(default)]
    window_class: String,
    #[serde(default)]
    transparency: u8,
    #[serde(default)]
    enabled: bool,
}
impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            process_name: String::new(),
            window_class: String::new(),
            transparency: 255,
            enabled: false,
        }
    }
}
impl From<&WindowConfig> for TransparencyRule {
    fn from(config: &WindowConfig) -> Self {
        TransparencyRule {
            process_name: config.process_name.clone().into(),
            window_class: config.window_class.clone().into(),
            transparency: convert_to_human(config.transparency) as i32,
            enabled: config.enabled,
        }
    }
}

impl WindowConfig {
    pub fn new(process_name: String, window_class: String, transparency: u8) -> Self {
        Self {
            process_name,
            window_class,
            transparency,
            enabled: true,
        }
    }

    pub fn get_key(&self) -> String {
        format!("{}|{}", self.process_name, self.window_class)
    }

    pub fn get_name(self) -> String {
        self.process_name
    }

    pub fn get_transparency(&self) -> &u8 {
        &self.transparency
    }

    pub fn get_window_class(&self) -> &String {
        &self.window_class
    }

    pub fn set_name(&mut self, new_process_name: String) {
        self.process_name = new_process_name
    }

    pub fn set_transparency(&mut self, new_transparency: u8) {
        self.transparency = new_transparency
    }
    pub fn set_enabled(&mut self, new_state: bool) {
        self.enabled = new_state
    }
    pub fn is_enabled(&self) -> &bool {
        &self.enabled
    }

    pub fn set_window_class(&mut self, new_class_name: String) {
        self.window_class = new_class_name
    }
}

pub struct AppState {
    pub config: Arc<RwLock<Config>>,
    config_path: PathBuf,
    enabled: Arc<RwLock<bool>>,
}

impl AppState {
    pub fn new(config: Config, config_path: PathBuf) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_path,
            enabled: Arc::new(RwLock::new(true)),
        }
    }

    pub async fn get_config(&self) -> Config {
        self.config.read().await.clone()
    }

    pub async fn get_config_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, Config> {
        self.config.write().await
    }
    pub fn get_config_path(&self) -> &PathBuf {
        &self.config_path
    }

    pub async fn is_enabled(&self) -> bool {
        // Lock the RwLock to read the value
        *self.enabled.read().await
    }

    pub async fn set_enable_state(&self, new_state: bool) {
        *self.enabled.write().await = new_state;
    }
}
pub enum Message {
    Quit,
    Add,
    Rules,
    Enable,
    Disable,
}
