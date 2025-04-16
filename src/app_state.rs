use crate::{
    transparency::create_rules_window,
    util::Config,
    win_utils::{self, create_percentage_window},
    window_config::{find_parent_from_child_class, WindowConfig},
    TransparencyRule,
};
use std::{fs, path::PathBuf, sync::Arc};
use tokio::sync::{broadcast, RwLock};

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
        let (config_tx, _) = broadcast::channel(2);
        let (enabled_tx, _) = broadcast::channel(2);

        Self {
            config_tx,
            enabled_tx,
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

        // Check if we need to update any existing config with old_class that matches this one
        for existing_config in config.get_windows().values_mut() {
            if let Some(old_class) = existing_config.get_old_classname() {
                if existing_config.get_name() == window_config.get_name()
                    && window_config.get_window_class() == old_class
                {
                    // Update the existing config
                    existing_config.set_enabled(window_config.is_enabled());
                    existing_config.set_transparency(window_config.get_transparency());

                    let config_json = serde_json::to_string_pretty(&config.to_owned())?;

                    self.config_tx.send(config.to_owned())?;
                    fs::write(self.get_config_path(), config_json)?;

                    return Ok(());
                }
            }
        }

        // If no existing config needed updating, insert the new one
        config
            .get_windows()
            .insert(window_config.get_key(), window_config);

        // Save the updated config
        let config_json = serde_json::to_string_pretty(&config.to_owned())?;

        self.config_tx.send(config.to_owned())?;
        fs::write(self.get_config_path(), config_json)?;

        Ok(())
    }

    pub async fn add_force_config(
        &self,
        mut window_config: WindowConfig,
    ) -> Result<(), anyhow::Error> {
        let mut config = self.get_config_mut().await;

        let lookup_class = window_config.get_window_class().to_owned();

        // Try to find parent class
        if let Ok(Some(parent_info)) = find_parent_from_child_class(&lookup_class) {
            let parent_class = parent_info.1;

            if window_config.is_forced() {
                self.remove_existing_config(&mut config, &window_config);
                window_config.set_window_class(&parent_class);
                if window_config.is_enabled() {
                    window_config.refresh_config();
                }
                window_config.set_old_classname(Some(lookup_class));

                config
                    .get_windows()
                    .insert(window_config.get_key(), window_config.clone());
            } else {
                for existing_config in config.get_windows().values_mut() {
                    if let Some(old_class) = existing_config.get_old_classname() {
                        if existing_config.get_name() == window_config.get_name()
                            && window_config.get_window_class() == old_class
                        {
                            if !window_config.is_forced() {
                                existing_config.set_enabled(false);
                                existing_config.unforce_windows_config();
                                existing_config.set_window_class(window_config.get_window_class());
                            } else {
                                existing_config.set_enabled(window_config.is_enabled());
                            }

                            existing_config.set_transparency(window_config.get_transparency());
                            existing_config.set_forced(window_config.is_forced());
                        }
                    }
                }
            }

            let config_json = serde_json::to_string_pretty(&config.to_owned())?;
            self.config_tx.send(config.to_owned())?;

            fs::write(self.get_config_path(), config_json)?;
        }

        Ok(())
    }

    fn remove_existing_config(&self, config: &mut Config, window_config: &WindowConfig) {
        config.get_windows().remove(&window_config.get_key());

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

    pub async fn enabled(&self) {
        self.set_enable_state(true).await
    }

    pub async fn disable(&self) {
        self.set_enable_state(false).await
    }

    async fn set_enable_state(&self, new_state: bool) {
        *self.enabled.write().await = new_state;

        self.enabled_tx
            .send(new_state)
            .expect("enabled sender failed");
    }
}
