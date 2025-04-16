use crate::{app_state::AppState, util::Config, win_utils::set_window_alpha};
use core::time::Duration;
use std::{
    collections::{HashMap, HashSet},
    os::raw::c_void,
    sync::Arc,
};
use windows::Win32::Foundation::HWND;
// Delays between window monitor runs
// new windows, window updates etc.
const MONITOR_DELAY: u64 = 120;

#[derive(Eq, PartialEq, Clone, Debug)]
struct WindowHandleState {
    handle: isize,
    transparency: u8,
    enabled: bool,
}

impl WindowHandleState {
    pub fn new(handle: isize) -> Self {
        Self {
            handle,
            transparency: 1,
            enabled: false,
        }
    }

    pub fn get_handle(&self) -> HWND {
        HWND(self.handle as *mut c_void)
    }

    pub fn get_transparency(&self) -> u8 {
        self.transparency
    }

    pub fn update_state(&mut self, transparency: u8, enabled: bool) {
        self.transparency = transparency;
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn refresh_window(&mut self) {
        self.enabled = false;
        self.apply_alpha();
    }

    fn apply_alpha(&self) {
        let transparency = if self.enabled { self.transparency } else { 255 };
        set_window_alpha(self.get_handle(), transparency).ok();
    }

    pub fn update_window(&mut self, new_transparency: u8, enabled: bool) {
        if self.get_transparency() != new_transparency || self.is_enabled() != enabled {
            self.update_state(new_transparency, enabled);
            self.apply_alpha();
        }
    }
}

/*
  Monitors the current windows specified in the config file. This is setup to target based on the window class rather than title (multiple windows open of X application...)
*/
#[inline(always)]
pub async fn monitor_windows(app_state: Arc<AppState>) {
    let refresh_interval = Duration::from_millis(MONITOR_DELAY);
    let mut window_cache = HashMap::with_capacity(8);

    let mut config = app_state.get_config().await;
    let mut is_enabled = app_state.is_enabled().await;

    // This is the in memory config
    let mut application_config = app_state.subscribe_config_updates();

    // Global application toggle.
    let mut application_toggle = app_state.subscribe_enabled_updates();

    loop {
        tokio::select! {
            _ = app_state.shutdown.notified() => {
                reset_windows(&mut window_cache);
                break;
            }
            Ok(new_config) = application_config.recv() => {
                config = new_config;
            }
            Ok(state) = application_toggle.recv() => {
                if state != is_enabled && is_enabled {
                    reset_windows(&mut window_cache);
                }
                is_enabled = state;
            }
            _ = tokio::time::sleep(refresh_interval) => {
                if is_enabled {
                    refresh_window_cache(&mut config, &mut window_cache);
                    update_windows(&config, &mut window_cache);
                }
            }
            else => break
        }
    }
}

#[inline(always)]
fn refresh_window_cache(config: &mut Config, cache: &mut HashMap<String, Vec<WindowHandleState>>) {
    for cfg in config.get_windows().values_mut() {
        let handles = cfg.get_window_hwnds();
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

#[inline(always)]
fn update_windows(config: &Config, window_cache: &mut HashMap<String, Vec<WindowHandleState>>) {
    for window_config in config.get_windows_non_mut().values() {
        if let Some(handle_states) = window_cache.get_mut(&window_config.get_cache_key()) {
            for state in handle_states.iter_mut() {
                state.update_window(window_config.get_transparency(), window_config.is_enabled());
            }
        }
    }
}

#[inline(always)]
fn reset_windows(window_cache: &mut HashMap<String, Vec<WindowHandleState>>) {
    window_cache
        .values_mut()
        .flat_map(|handles| handles.iter_mut())
        .for_each(|handle| handle.refresh_window());
}
