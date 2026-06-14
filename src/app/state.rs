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
use tracing::{debug, error, instrument};

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
        self.spawn_rule_task(async move { app_state.add_rule(value).await });
    }

    pub fn spawn_force_rule(&self, value: WindowRule) {
        let app_state = self.clone();
        self.spawn_rule_task(async move { app_state.add_force_rule(value).await });
    }

    pub fn spawn_remove_rule(&self, value: WindowRule) {
        let app_state = self.clone();
        self.spawn_rule_task(async move { app_state.remove_rule(value).await });
    }

    /// Run a rule-mutating task on the background runtime, surfacing any failure
    /// to the user *and* logging it. Every rule operation shares this: a swallowed
    /// `error!` is invisible in release (tracing is stripped), and these failures
    /// silently lose the user's change, so they must reach the UI.
    fn spawn_rule_task(
        &self,
        task: impl std::future::Future<Output = Result<(), anyhow::Error>> + Send + 'static,
    ) {
        self.runtime.spawn(async move {
            if let Err(e) = task.await {
                error!(error = %e, "failed to update window rule");
                crate::app::ui::report_error(
                    "Couldn't save the transparency rule — the config file may not be writable.",
                );
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

    #[instrument(skip(self), fields(class = %rule.get_window_class()))]
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

    #[instrument(skip(self), fields(class = %rule.get_window_class()))]
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

    #[instrument(skip(self), fields(class = %rule.get_window_class()))]
    pub async fn remove_rule(&self, rule: WindowRule) -> Result<(), anyhow::Error> {
        // On polling backends, restore the rule's live windows to opaque now:
        // once it's gone from the config the monitor loop won't visit it again to
        // reset them. Compositor backends are covered by the rule re-sync inside
        // `persist_and_broadcast`.
        rule.unforce(wm());

        let snapshot = {
            let mut config = self.get_config_mut().await;
            config.remove_rule(&rule);
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

    /// The tray's active menu label, reflecting whether transparency rules are
    /// currently being applied.
    pub fn active_label(&self) -> String {
        format!("Active: {}", self.is_enabled())
    }

    /// Flip the enabled state and return the updated [`Self::active_label`], so
    /// the caller can relabel its menu item to match.
    pub async fn toggle_active(&self) -> String {
        self.set_enable_state(!self.is_enabled()).await;
        self.active_label()
    }

    async fn set_enable_state(&self, new_state: bool) {
        debug!(enabled = new_state, "transparency enabled state changed");
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
        let target = !Os::get_autostart_state();
        debug!(autostart = target, "toggling OS autostart");
        if let Err(e) = Os::set_autostart(target) {
            error!(error = %e, "failed to change startup setting");
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
            error!(error = %e, "failed to sync compositor transparency rules");
        }
    }

    #[instrument(skip_all)]
    pub async fn sync_rules_now(&self) {
        let config = self.get_config().await;
        self.sync_compositor(&config, self.is_enabled());
    }
}
