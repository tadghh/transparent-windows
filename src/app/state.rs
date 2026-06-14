use crate::{
    TransparencyRule,
    app::config::{Config, config_path, load_config},
    platform::{Opacity, OperatingSystem, Os, RuleSpec, wm},
    transparency::rules::WindowRule,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
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
    enabled: Arc<AtomicBool>,
    runtime: Handle,
    pub shutdown: Arc<tokio::sync::Notify>,
}

impl AppState {
    /// Build the application state, owning config resolution and loading: the
    /// config path is resolved (and its directory created), then the config is
    /// loaded from it (a missing file starts empty; a corrupt one surfaces the
    /// recovery window). Errors only when the OS yields no config location.
    pub fn new(runtime: Handle) -> Result<Self, anyhow::Error> {
        let config_path = config_path()?;
        let config = load_config(&config_path);

        let (config_tx, _) = broadcast::channel(2);
        let (enabled_tx, _) = broadcast::channel(2);

        Ok(Self {
            config_tx,
            enabled_tx,
            config: Arc::new(RwLock::new(config)),
            config_path,
            enabled: Arc::new(AtomicBool::new(true)),
            runtime,
            shutdown: Arc::new(tokio::sync::Notify::new()),
        })
    }

    /// Handle to the background tokio runtime, for spawning from UI callbacks.
    pub fn runtime(&self) -> &Handle {
        &self.runtime
    }

    pub fn spawn_update_rule(&self, value: WindowRule) {
        // `AppState` is all `Arc`/`Handle`/`Sender` fields, so a clone is cheap
        // and owned — no extra `Arc` wrapper needed to move it into the task.
        let app_state = self.clone();

        self.runtime.spawn(async move {
            if let Err(e) = app_state.add_rule(value).await {
                eprintln!("Failed to update window rule: {}", e);
            }
        });
    }

    pub fn spawn_force_rule(&self, value: WindowRule) {
        let app_state = self.clone();

        self.runtime.spawn(async move {
            if let Err(e) = app_state.add_force_rule(value).await {
                eprintln!("Failed to update window rule: {}", e);
            }
        });
    }

    pub async fn get_window_rules(&self) -> Vec<TransparencyRule> {
        let config = self.get_config().await;
        config
            .windows()
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

    pub async fn add_rule(&self, rule: WindowRule) -> Result<(), anyhow::Error> {
        // Mutate under the write lock, then snapshot and release it before any
        // I/O — persisting happens lock-free (see `persist_and_broadcast`). The
        // rule-merge logic itself lives on `Config` so it's unit-testable.
        let snapshot = {
            let mut config = self.get_config_mut().await;
            config.upsert_rule(rule);
            config.clone()
        };

        self.persist_and_broadcast(snapshot).await
    }

    pub async fn add_force_rule(&self, rule: WindowRule) -> Result<(), anyhow::Error> {
        let lookup_class = rule.get_window_class().to_owned();

        // Resolve the parent window class *before* taking the config lock: this is
        // a blocking platform call, and holding the write lock across it would
        // stall every config reader. No parent found → nothing to force.
        let Ok(Some((_, parent_class))) = wm().find_parent_from_child_class(&lookup_class) else {
            return Ok(());
        };

        let snapshot = {
            let mut config = self.get_config_mut().await;
            config.apply_force(rule, &parent_class, lookup_class, wm());
            config.clone()
        };

        self.persist_and_broadcast(snapshot).await
    }

    pub fn subscribe_config_updates(&self) -> broadcast::Receiver<Config> {
        self.config_tx.subscribe()
    }

    pub fn subscribe_enabled_updates(&self) -> broadcast::Receiver<bool> {
        self.enabled_tx.subscribe()
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub async fn enabled(&self) {
        self.set_enable_state(true).await
    }

    pub async fn disable(&self) {
        self.set_enable_state(false).await
    }

    async fn set_enable_state(&self, new_state: bool) {
        self.enabled.store(new_state, Ordering::Relaxed);

        // A broadcast send errors only when there are no receivers (e.g. during
        // shutdown after the monitor dropped its subscription) — nothing to
        // notify, so it is not a logic error and must not panic the toggle.
        let _ = self.enabled_tx.send(new_state);

        let config = self.get_config().await;
        self.sync_compositor(&config, new_state);
    }

    /// The tray's startup menu label, reflecting live OS autostart state (the
    /// autostart `.desktop` file / registry `Run` key is the source of truth).
    pub fn startup_label(&self) -> String {
        format!("Startup: {}", Os::get_autostart_state())
    }

    /// Flip the OS autostart setting and return the updated [`Self::startup_label`], so
    /// the caller reflects what actually took effect even if the write failed.
    pub fn toggle_autostart(&self) -> String {
        if let Err(e) = Os::set_autostart(!Os::get_autostart_state()) {
            eprintln!("Failed to change startup setting: {e}");
        }
        self.startup_label()
    }

    /// Build the active rule set: enabled rules, only when the global toggle is on.
    fn rules_from(config: &Config, enabled: bool) -> Vec<RuleSpec> {
        if !enabled {
            return Vec::new();
        }
        config
            .windows()
            .values()
            .filter(|window| window.is_enabled())
            .map(|window| RuleSpec {
                window_class: window.get_window_class().to_owned(),
                alpha: window.get_alpha(),
            })
            .collect()
    }

    /// Persist the (already-updated) config to disk and notify subscribers. Takes
    /// the config by value so callers release the write lock first; the disk
    /// write runs on a blocking thread (no `tokio::fs` feature) so it never
    /// stalls an async worker or holds the lock across I/O.
    async fn persist_and_broadcast(&self, config: Config) -> Result<(), anyhow::Error> {
        let config_json = serde_json::to_string_pretty(&config)?;
        let path = self.get_config_path();
        tokio::task::spawn_blocking(move || fs::write(path, config_json)).await??;

        // Sync first (borrows), then hand the config to the broadcast by value —
        // no extra clone.
        self.sync_compositor(&config, self.is_enabled());
        let _ = self.config_tx.send(config);

        Ok(())
    }

    fn sync_compositor(&self, config: &Config, enabled: bool) {
        // Only compositor backends (KWin) enforce rules this way; polling
        // backends apply opacity through the monitor loop instead.
        if let Opacity::Compositor(compositor) = wm().opacity()
            && let Err(e) = compositor.sync_rules(&Self::rules_from(config, enabled))
        {
            eprintln!("Failed to sync compositor transparency rules: {e}");
        }
    }

    pub async fn sync_rules_now(&self) {
        let config = self.get_config().await;
        self.sync_compositor(&config, self.is_enabled());
    }
}
