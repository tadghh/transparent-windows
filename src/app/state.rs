use crate::{
    TransparencyRule,
    platform::{RuleSpec, wm},
    util::Config,
    window_config::WindowConfig,
};
use std::{fs, path::PathBuf, sync::Arc};
use tokio::{
    runtime::Handle,
    sync::{RwLock, broadcast},
};

#[derive(Clone)]
pub struct AppState {
    config_tx: broadcast::Sender<Config>,
    enabled_tx: broadcast::Sender<bool>,
    config: Arc<RwLock<Config>>,
    config_path: PathBuf,
    enabled: Arc<RwLock<bool>>,
    runtime: Handle,
    pub shutdown: Arc<tokio::sync::Notify>,
}

impl AppState {
    pub fn new(config: Config, config_path: PathBuf, runtime: Handle) -> Self {
        let (config_tx, _) = broadcast::channel(2);
        let (enabled_tx, _) = broadcast::channel(2);

        Self {
            config_tx,
            enabled_tx,
            config: Arc::new(RwLock::new(config)),
            config_path,
            enabled: Arc::new(RwLock::new(true)),
            runtime,
            shutdown: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// Handle to the background tokio runtime, for spawning from UI callbacks.
    pub fn runtime(&self) -> &Handle {
        &self.runtime
    }

    pub fn spawn_update_config(&self, value: WindowConfig) {
        let app_state = Arc::new(self.clone());

        self.runtime.spawn(async move {
            if let Err(e) = app_state.add_window_config(value).await {
                eprintln!("Failed to update window config: {}", e);
            }
        });
    }

    pub fn spawn_force_config(&self, value: WindowConfig) {
        let app_state: Arc<AppState> = Arc::new(self.clone());

        self.runtime.spawn(async move {
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

    pub async fn get_config(&self) -> Config {
        self.config.read().await.clone()
    }

    pub fn get_config_path(&self) -> String {
        self.config_path.to_string_lossy().into_owned()
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
                    existing_config.set_alpha(window_config.get_alpha());

                    self.persist_and_broadcast(&config).await?;
                    return Ok(());
                }
            }
        }

        // If no existing config needed updating, insert the new one
        config
            .get_windows()
            .insert(window_config.get_key(), window_config);

        self.persist_and_broadcast(&config).await
    }

    pub async fn add_force_config(
        &self,
        mut window_config: WindowConfig,
    ) -> Result<(), anyhow::Error> {
        let mut config = self.get_config_mut().await;

        let lookup_class = window_config.get_window_class().to_owned();

        // Try to find parent class
        if let Ok(Some(parent_info)) = wm().find_parent_from_child_class(&lookup_class) {
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

                            existing_config.set_alpha(window_config.get_alpha());
                            existing_config.set_forced(window_config.is_forced());
                        }
                    }
                }
            }

            self.persist_and_broadcast(&config).await?;
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

        let config = self.get_config().await;
        self.sync_compositor(&config, new_state);
    }

    /// Build the active rule set: enabled rules, only when the global toggle is on.
    fn rules_from(config: &Config, enabled: bool) -> Vec<RuleSpec> {
        if !enabled {
            return Vec::new();
        }
        config
            .get_windows_non_mut()
            .values()
            .filter(|window| window.is_enabled())
            .map(|window| RuleSpec {
                window_class: window.get_window_class().to_owned(),
                alpha: window.get_alpha(),
            })
            .collect()
    }

    async fn persist_and_broadcast(&self, config: &Config) -> Result<(), anyhow::Error> {
        let config_json = serde_json::to_string_pretty(config)?;
        fs::write(self.get_config_path(), config_json)?;

        let _ = self.config_tx.send(config.clone());

        let enabled = self.is_enabled().await;
        self.sync_compositor(config, enabled);

        Ok(())
    }

    fn sync_compositor(&self, config: &Config, enabled: bool) {
        if let Err(e) = wm().sync_rules(&Self::rules_from(config, enabled)) {
            eprintln!("Failed to sync compositor transparency rules: {e}");
        }
    }

    pub async fn sync_rules_now(&self) {
        let config = self.get_config().await;
        let enabled = self.is_enabled().await;
        self.sync_compositor(&config, enabled);
    }
}
