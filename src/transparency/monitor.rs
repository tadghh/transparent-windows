use crate::{
    app::{config::Config, state::AppState},
    platform::{Opacity, PollingOpacity, WindowHandle, wm},
};
use core::time::Duration;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

// Poll interval between monitor passes, in milliseconds.
const MONITOR_DELAY: u64 = 120;

#[derive(Eq, PartialEq, Clone, Debug)]
struct WindowHandleState {
    handle: WindowHandle,
    alpha: u8,
    enabled: bool,
}

impl WindowHandleState {
    pub fn new(handle: WindowHandle) -> Self {
        Self {
            handle,
            alpha: 1,
            enabled: false,
        }
    }

    pub fn get_alpha(&self) -> u8 {
        self.alpha
    }

    pub fn update_state(&mut self, alpha: u8, enabled: bool) {
        self.alpha = alpha;
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn reset_to_opaque(&mut self, wm: &dyn PollingOpacity) {
        self.enabled = false;
        self.apply_alpha(wm);
    }

    fn apply_alpha(&self, wm: &dyn PollingOpacity) {
        let alpha = if self.enabled { self.alpha } else { 255 };
        wm.set_window_alpha(self.handle, alpha).ok();
    }

    pub fn update_window(&mut self, wm: &dyn PollingOpacity, new_alpha: u8, enabled: bool) {
        if self.get_alpha() != new_alpha || self.is_enabled() != enabled {
            self.update_state(new_alpha, enabled);
            self.apply_alpha(wm);
        }
    }
}

/// Polling-backend monitor loop: applies the config's opacity rules to matching
/// open windows and keeps them in sync as windows open and close and as the config
/// or enabled-toggle change. Matches on window class (not title) so every window of
/// an application is covered. Returns immediately on compositor backends, which
/// enforce rules themselves.
pub async fn monitor_windows(app_state: Arc<AppState>) {
    let op = match wm().opacity() {
        Opacity::Polling(op) => op,
        Opacity::Compositor(_) => return,
    };

    let refresh_interval = Duration::from_millis(MONITOR_DELAY);
    let mut window_cache = HashMap::with_capacity(8);

    let mut config = app_state.get_config().await;
    let mut is_enabled = app_state.is_enabled();

    let mut application_config = app_state.subscribe_config_updates();

    let mut application_toggle = app_state.subscribe_enabled_updates();

    loop {
        tokio::select! {
            _ = app_state.shutdown.notified() => {
                reset_windows(op, &mut window_cache);
                break;
            }
            Ok(new_config) = application_config.recv() => {
                config = new_config;
            }
            Ok(state) = application_toggle.recv() => {
                if state != is_enabled && is_enabled {
                    reset_windows(op, &mut window_cache);
                }
                is_enabled = state;
            }
            _ = tokio::time::sleep(refresh_interval) => {
                if is_enabled {
                    refresh_window_cache(op, &config, &mut window_cache);
                    update_windows(op, &config, &mut window_cache);
                }
            }
            else => break
        }
    }
}

fn refresh_window_cache(
    wm: &dyn PollingOpacity,
    config: &Config,
    cache: &mut HashMap<String, Vec<WindowHandleState>>,
) {
    for cfg in config.windows().values() {
        let handles = cfg.get_window_handles(wm);
        let key = cfg.get_cache_key();

        if handles.is_empty() {
            if let Some(val) = cache.get_mut(&key) {
                val.clear();
            }
            continue;
        }

        let states = cache.entry(key).or_default();
        states.retain(|state| handles.contains(&state.handle));

        let existing_handles: HashSet<_> = states.iter().map(|state| state.handle).collect();
        for &handle in &handles {
            if !existing_handles.contains(&handle) {
                states.push(WindowHandleState::new(handle));
            }
        }
    }

    cache.retain(|_, states| !states.is_empty());
}

fn update_windows(
    wm: &dyn PollingOpacity,
    config: &Config,
    window_cache: &mut HashMap<String, Vec<WindowHandleState>>,
) {
    for rule in config.windows().values() {
        if let Some(handle_states) = window_cache.get_mut(&rule.get_cache_key()) {
            for state in handle_states.iter_mut() {
                state.update_window(wm, rule.get_alpha(), rule.is_enabled());
            }
        }
    }
}

fn reset_windows(
    wm: &dyn PollingOpacity,
    window_cache: &mut HashMap<String, Vec<WindowHandleState>>,
) {
    window_cache
        .values_mut()
        .flat_map(|handles| handles.iter_mut())
        .for_each(|handle| handle.reset_to_opaque(wm));
}

#[cfg(test)]
#[path = "../tests/monitor.rs"]
mod tests;
