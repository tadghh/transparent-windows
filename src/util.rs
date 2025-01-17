use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WindowConfig {
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

    pub fn get_name(self) -> String {
        self.process_name
    }

    pub fn get_transparency(&self) -> &u8 {
        &self.transparency
    }

    pub fn get_window_class(&self) -> &String {
        &self.window_class
    }
}

pub struct AppState {
    config: Arc<RwLock<Config>>,
    config_path: PathBuf,
}

impl AppState {
    pub fn new(config: Config, config_path: PathBuf) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_path,
        }
    }

    pub fn get_config(&self) -> &Arc<RwLock<Config>> {
        &self.config
    }

    pub fn get_config_path(&self) -> &PathBuf {
        &self.config_path
    }
}
pub enum Message {
    Quit,
    Add,
    Rules,
}
