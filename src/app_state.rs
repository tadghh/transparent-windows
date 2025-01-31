use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use crate::monitor::{refresh_config, reset_config};
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
        mut window_config: WindowConfig,
    ) -> Result<(), anyhow::Error> {
        let mut config = self.get_config_mut().await;

        if window_config.is_wide() && window_config.get_old_classname().is_none() {
            let old_key: String = window_config.get_key();
            let old_class: String = window_config.get_window_class().to_owned();

            config.get_windows().remove(&old_key);

            match find_parent_from_child_class(window_config.get_window_class()) {
                Ok(class) => {
                    if let Some(info) = class {
                        window_config.set_window_class(info.1);
                        refresh_config(window_config.clone());
                        config.get_windows().remove(&window_config.get_key());
                        window_config.set_old_classname(Some(old_class));
                    }
                }
                Err(_) => (),
            }
        }

        if !window_config.is_wide() {
            println!("d");
            match find_parent_from_child_class(window_config.get_window_class()) {
                Ok(class) => {
                    if let Some(info) = class {
                        let key = &format!("{}|{}", window_config.get_name(), info.1);
                        println!("hrere {:?}", info);
                        if config.get_windows().contains_key(key) {
                            println!("key yo {:?}", key);
                            let clone_config = window_config.clone();
                            let real_class = clone_config.get_window_class();
                            window_config.set_window_class(info.1);
                            reset_config(window_config.clone());
                            // let handles = window_config.get_window_hwnds();
                            window_config.set_window_class(real_class.to_string());
                            // println!("handles yo {:?}", handles);

                            config.get_windows().remove(key);
                        }
                    } else {
                        println!("2{:?}", class);
                    }
                }
                Err(_) => {
                    println!("2");
                    ()
                }
            }
        }

        config
            .get_windows()
            .insert(window_config.get_key(), window_config);

        let config_json = serde_json::to_string_pretty(&config.to_owned())?;
        fs::write(&self.get_config_path(), config_json)?;

        self.config_tx.send(config.clone())?;

        Ok(())
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
