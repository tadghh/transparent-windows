//! Application config: the on-disk [`Config`] model, resolving and loading it
//! ([`config_path`], [`load_config`]), and the recovery window shown when the
//! stored file can't be parsed.

use crate::{
    ConfigWindow,
    app::ui::WindowExt,
    platform::{OperatingSystem, Os, WindowManager},
    transparency::rules::WindowRule,
};
use anyhow::{Result, anyhow};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use slint::ComponentHandle;
use std::{
    cell::Cell,
    collections::HashMap,
    fs::{self, create_dir_all},
    path::{Path, PathBuf},
    rc::Rc,
};
use tracing::{debug, error, warn};

#[cfg(test)]
#[path = "../tests/config.rs"]
mod tests;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Config {
    windows: HashMap<String, WindowRule>,
}

impl Config {
    pub fn windows(&self) -> &HashMap<String, WindowRule> {
        &self.windows
    }

    /// Mutable access to the whole rule map. Rule mutations in normal flow go
    /// through [`Config::upsert_rule`]/[`Config::apply_force`]; this is for test
    /// setup that needs to seed arbitrary state.
    #[cfg(test)]
    pub fn windows_mut(&mut self) -> &mut HashMap<String, WindowRule> {
        &mut self.windows
    }

    /// Add `rule`, or update the existing rule it supersedes — one whose recorded
    /// `old_class` (the class it was forced away from) matches this rule's class
    /// for the same process. Matching that case updates in place; otherwise the
    /// rule is inserted fresh.
    pub fn upsert_rule(&mut self, rule: WindowRule) {
        for existing in self.windows.values_mut() {
            if let Some(old_class) = existing.get_old_classname()
                && existing.get_name() == rule.get_name()
                && rule.get_window_class() == old_class
            {
                existing.set_enabled(rule.is_enabled());
                existing.set_alpha(rule.get_alpha());
                return;
            }
        }

        self.windows.insert(rule.get_key(), rule);
    }

    /// Apply a force toggle for `rule`, given the already-resolved `parent_class`
    /// (the top-level window class the child belongs to) and the `lookup_class`
    /// the rule was created against. `wm` is used only to apply opacity to live
    /// windows during the transition, so it can be a no-op in tests.
    ///
    /// Forcing re-keys the rule under the parent class and records the original
    /// class as `old_class`; un-forcing finds that re-keyed rule and disables it,
    /// restoring its windows to opaque.
    pub fn apply_force(
        &mut self,
        mut rule: WindowRule,
        parent_class: &str,
        lookup_class: String,
        wm: &dyn WindowManager,
    ) {
        if rule.is_forced() {
            self.remove_rule(&rule);
            rule.set_window_class(parent_class);
            if rule.is_enabled() {
                rule.refresh(wm);
            }
            rule.set_old_classname(Some(lookup_class));

            self.windows.insert(rule.get_key(), rule);
        } else {
            for existing in self.windows.values_mut() {
                if let Some(old_class) = existing.get_old_classname()
                    && existing.get_name() == rule.get_name()
                    && rule.get_window_class() == old_class
                {
                    existing.set_enabled(false);
                    existing.unforce(wm);
                    existing.set_window_class(rule.get_window_class());
                    existing.set_alpha(rule.get_alpha());
                    existing.set_forced(rule.is_forced());
                }
            }
        }
    }

    /// Remove a rule by its key, plus any forced-class alias keyed by its
    /// `old_class`.
    pub fn remove_rule(&mut self, rule: &WindowRule) {
        self.windows.remove(&rule.get_key());

        if let Some(old_class) = rule.get_old_classname() {
            let key = format!("{}|{}", rule.get_name(), old_class);
            self.windows.remove(&key);
        }
    }
}

/// Overwrite the file at `path` with a valid empty config, recovering a corrupt
/// file in place. A serialize/write failure is logged rather than panicking the
/// UI callback.
fn reset_config_file(path: &str) {
    match serde_json::to_string_pretty(&Config::default()) {
        Ok(json) => {
            if let Err(e) = fs::write(path, json) {
                error!(error = %e, path, "failed to reset config");
            }
        }
        Err(e) => error!(error = %e, "failed to serialize empty config"),
    }
}

/// Shows the config-recovery window. Must be called on the Slint event-loop
/// thread (every callback below runs there, so single-thread cell types are
/// enough — no cross-thread locking).
fn show_config_error_window(config_path: PathBuf) {
    let config_path = match config_path.into_os_string().into_string() {
        Ok(path) => path,
        Err(os_str) => {
            error!(?os_str, "invalid UTF-8 in config path");
            return;
        }
    };

    let window = match ConfigWindow::new() {
        Ok(window) => window,
        Err(e) => {
            error!(error = %e, "failed to create config error window");
            return;
        }
    };

    // Tracks whether the user picked Edit or Reset, so closing the window without
    // choosing can fall back to a reset. Shared between both callbacks.
    let action_taken = Rc::new(Cell::new(false));

    let submit_action = action_taken.clone();
    let submit_path = config_path.clone();
    let submit_handle = window.as_weak();
    window.on_submit(move |value| match value {
        crate::Action::Edit => {
            submit_action.set(true);
            if let Err(e) = Os::open_path(&submit_path) {
                error!(error = %e, "failed to open config for editing");
            }
        }
        crate::Action::Reset => {
            submit_action.set(true);
            reset_config_file(&submit_path);
            if let Some(handle) = submit_handle.upgrade() {
                let _ = handle.hide();
            }
        }
    });

    let window_handle = window.as_weak();
    window.on_cancel(move || {
        // Closing without choosing Edit/Reset falls back to resetting the file.
        if !action_taken.get() {
            reset_config_file(&config_path);
        }
        if let Some(handle) = window_handle.upgrade() {
            let _ = handle.hide();
        }
    });

    window.show_keep_alive();
}

/// Resolve the config file's path, creating its directory if missing. Errors when
/// the OS won't yield a config location (no home directory) or the directory
/// can't be created.
pub fn config_path() -> Result<PathBuf> {
    let project_dirs = ProjectDirs::from(
        crate::identity::APP_QUALIFIER,
        crate::identity::APP_ORG,
        crate::identity::APP_ID,
    )
    .ok_or_else(|| anyhow!("could not resolve the OS config directory"))?;

    let config_dir = project_dirs.config_dir();
    create_dir_all(config_dir).map_err(|e| {
        anyhow!(
            "could not create config directory {}: {e}",
            config_dir.display()
        )
    })?;

    Ok(config_dir.join("config.json"))
}

/// Load the config at `path`, always yielding a usable [`Config`]:
/// - missing or unreadable file → a fresh empty config (first run, nothing to do);
/// - present but unparseable → a fresh empty config *and* the recovery window, so
///   the user can edit or reset the broken file instead of it being silently
///   overwritten.
pub fn load_config(path: &Path) -> Config {
    let Ok(data) = fs::read_to_string(path) else {
        return Config::default();
    };

    match serde_json::from_str::<Config>(&data) {
        Ok(config) => {
            debug!(rules = config.windows().len(), "loaded config");
            config
        }
        Err(e) => {
            warn!(error = %e, path = %path.display(), "config is invalid; opening recovery window");
            show_config_error_window(path.to_path_buf());
            Config::default()
        }
    }
}
