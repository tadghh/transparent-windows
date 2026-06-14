use crate::{
    app::{config::Config, state::AppState},
    platform::{Opacity, PollingOpacity, WindowHandle, wm},
};
use core::time::Duration;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tracing::{debug, instrument};

// Poll interval between monitor passes, in milliseconds. Used when the backend
// has no window-change signal (Windows) and must poll to notice new windows.
const MONITOR_DELAY: u64 = 120;

// Slow periodic re-apply, in milliseconds, used when the backend *does* signal
// window changes (X11): events drive the fast response, this only catches missed
// events and externally-reset opacity, so it can be infrequent.
const FALLBACK_DELAY: u64 = 2000;

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
#[instrument(skip_all)]
pub async fn monitor_windows(app_state: Arc<AppState>) {
    let op = match wm().opacity() {
        Opacity::Polling(op) => op,
        Opacity::Compositor(_) => {
            debug!("compositor backend enforces rules; polling monitor not started");
            return;
        }
    };
    // If the backend can signal window open/close (X11), drive re-application off
    // those events and keep only a slow periodic re-apply as a safety net.
    // Without a signal (Windows), fall back to the original fixed polling tick.
    let change_signal = op.window_change_signal();
    let refresh_interval = Duration::from_millis(if change_signal.is_some() {
        FALLBACK_DELAY
    } else {
        MONITOR_DELAY
    });
    debug!(
        poll_ms = refresh_interval.as_millis() as u64,
        event_driven = change_signal.is_some(),
        "polling window monitor started"
    );

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
                // Apply immediately rather than waiting for the next tick, which
                // can be seconds out on the event-driven path.
                if is_enabled {
                    apply_rules(op, &config, &mut window_cache);
                }
            }
            Ok(state) = application_toggle.recv() => {
                if state != is_enabled && is_enabled {
                    reset_windows(op, &mut window_cache);
                }
                is_enabled = state;
            }
            // Event-driven wake (X11): a window opened or closed. A no-op future
            // when the backend has no signal. The `if is_enabled` guard means a
            // disabled monitor parks here instead of waking.
            _ = wait_for_change(&change_signal), if is_enabled => {
                apply_rules(op, &config, &mut window_cache);
            }
            // Periodic re-apply: the primary tick on polling backends, a slow
            // safety net on event-driven ones. Only armed while enabled.
            _ = tokio::time::sleep(refresh_interval), if is_enabled => {
                apply_rules(op, &config, &mut window_cache);
            }
            else => break
        }
    }
}

/// Resolve when the backend signals a possible window change, or never (when the
/// backend has no signal) so that `select!` branch simply stays pending and the
/// periodic tick drives re-application instead.
async fn wait_for_change(signal: &Option<Arc<tokio::sync::Notify>>) {
    match signal {
        Some(notify) => notify.notified().await,
        None => std::future::pending::<()>().await,
    }
}

/// Refresh the window cache and push the current opacity to every matching window.
fn apply_rules(
    op: &dyn PollingOpacity,
    config: &Config,
    cache: &mut HashMap<String, Vec<WindowHandleState>>,
) {
    refresh_window_cache(op, config, cache);
    update_windows(op, config, cache);
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
            if let Some(val) = cache.get_mut(key) {
                val.clear();
            }
            continue;
        }

        let states = cache.entry(key.to_owned()).or_default();
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
        if let Some(handle_states) = window_cache.get_mut(rule.get_cache_key()) {
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
