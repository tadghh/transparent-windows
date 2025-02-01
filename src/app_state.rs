use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use crate::transparency::create_rules_window;
use crate::util::Config;
use crate::win_utils::{self, create_percentage_window};
use crate::window_config::find_parent_from_child_class;
use crate::window_config::WindowConfig;
use crate::TransparencyRule;

#[derive(Clone)]
pub struct AppState {
    config_tx: broadcast::Sender<Config>,
    enabled_tx: broadcast::Sender<bool>,
    config: Arc<RwLock<Config>>,
    config_path: PathBuf,
    enabled: Arc<RwLock<bool>>,
    pub shutdown: Arc<tokio::sync::Notify>,
}

impl AppState {
    pub fn new(config: Config, config_path: PathBuf) -> Self {
        let (tx, _) = broadcast::channel(2);
        let (txe, _) = broadcast::channel(2);

        Self {
            config_tx: tx,
            enabled_tx: txe,
            config: Arc::new(RwLock::new(config)),
            config_path,
            enabled: Arc::new(RwLock::new(true)),
            shutdown: Arc::new(tokio::sync::Notify::new()),
        }
    }

    pub async fn quit(&self) {
        self.shutdown.notify_waiters();
    }

    pub fn spawn_update_config(&self, value: WindowConfig) {
        let app_state = Arc::new(self.clone());

        tokio::spawn(async move {
            if let Err(e) = app_state.add_window_config(value).await {
                eprintln!("Failed to update window config: {}", e);
            }
        });
    }
    pub fn spawn_force_config(&self, value: WindowConfig) {
        let app_state: Arc<AppState> = Arc::new(self.clone());

        tokio::spawn(async move {
            if let Err(e) = app_state.add_force_config(value).await {
                eprintln!("Failed to update window config: {}", e);
            }
        });
    }

    pub async fn get_window_rules(&self) -> Vec<TransparencyRule> {
        let config = self.get_config().await;
        config
            .get_windows_non_mut()
            .values()
            .map(TransparencyRule::from)
            .collect()
    }

    pub async fn add_window_rule(&self) -> Result<(), anyhow::Error> {
        let window = win_utils::get_window_under_cursor().expect("Non failure, get window cursor");
        create_percentage_window(window, Arc::new(self.clone())).await
    }

    pub async fn get_config(&self) -> Config {
        self.config.read().await.clone()
    }

    pub fn get_config_path(&self) -> String {
        self.config_path
            .clone()
            .into_os_string()
            .into_string()
            .expect("shut")
    }

    pub async fn show_rules_window(&self) -> Result<(), std::fmt::Error> {
        create_rules_window(Arc::new(self.clone())).await
    }

    pub async fn get_config_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, Config> {
        self.config.write().await
    }

    pub async fn add_window_config(
        &self,
        window_config: WindowConfig,
    ) -> Result<(), anyhow::Error> {
        let mut config = self.get_config_mut().await;

        config
            .get_windows()
            .insert(window_config.get_key(), window_config);

        let config_json = serde_json::to_string_pretty(&config.to_owned())?;
        self.config_tx.send(config.to_owned())?;

        fs::write(&self.get_config_path(), config_json)?;

        Ok(())
    }

    pub async fn add_force_config(
        &self,
        mut window_config: WindowConfig,
    ) -> Result<(), anyhow::Error> {
        let mut config = self.get_config_mut().await;

        // Get the class name to use for parent lookup
        let lookup_class = window_config
            .get_old_classname()
            .clone()
            .unwrap_or_else(|| window_config.get_window_class().to_owned());

        // Try to find parent class
        if let Ok(Some(parent_info)) = find_parent_from_child_class(&lookup_class) {
            let parent_class = parent_info.1;

            // Remove existing configuration
            self.remove_existing_config(&mut config, &window_config);

            if window_config.is_forced() {
                window_config.set_window_class(&parent_class);
                window_config.refresh_config();
                window_config.set_old_classname(Some(lookup_class));
            } else {
                window_config.set_window_class(&parent_class);
                window_config.reset_config();
                window_config.set_window_class(&lookup_class);
                window_config.set_old_classname(None);

                // Remove parent configuration
                let parent_key = format!("{}|{}", window_config.get_name(), parent_class);
                config.get_windows().remove(&parent_key);
            }
        }

        // Update configuration
        config
            .get_windows()
            .insert(window_config.get_key(), window_config);

        let config_json = serde_json::to_string_pretty(&config.to_owned())?;
        self.config_tx.send(config.to_owned())?;

        fs::write(&self.get_config_path(), config_json)?;

        Ok(())
    }

    fn remove_existing_config(&self, config: &mut Config, window_config: &WindowConfig) {
        // Remove configuration by original key
        config.get_windows().remove(&window_config.get_key());

        // Remove configuration by old class if it exists
        if let Some(old_class) = window_config.get_old_classname() {
            let key = format!("{}|{}", window_config.get_name(), old_class);
            config.get_windows().remove(&key);
        }
    }

    pub fn subscribe_config_updates(&self) -> broadcast::Receiver<Config> {
        self.config_tx.subscribe()
    }

    pub fn subscribe_enabled_updates(&self) -> broadcast::Receiver<bool> {
        self.enabled_tx.subscribe()
    }

    pub async fn is_enabled(&self) -> bool {
        *self.enabled.read().await
    }

    pub async fn set_enable_state(&self, new_state: bool) {
        *self.enabled.write().await = new_state;

        self.enabled_tx
            .send(new_state)
            .expect("enabled sender failed");
    }
}
