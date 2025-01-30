use crate::{app_state::AppState, util::Config, win_utils::make_window_transparent};

use core::time::Duration;
use std::{os::raw::c_void, sync::Arc, thread::sleep, time::Instant};

use rustc_hash::FxHashMap;
use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::IsWindow};

#[derive(Eq, PartialEq, Clone, Debug)]
struct WindowHandleState {
    handle: isize,
    transparency: u8,
    enabled: bool,
}

impl WindowHandleState {
    pub fn new(handle: isize, transparency: u8, enabled: bool) -> Self {
        Self {
            handle,
            transparency,
            enabled,
        }
    }

    pub fn get_handle(&self) -> HWND {
        HWND(self.handle as *mut c_void)
    }

    pub fn get_transparency(&self) -> u8 {
        if self.enabled {
            self.transparency
        } else {
            255
        }
    }

    pub fn update_state(&mut self, transparency: u8, enabled: bool) {
        self.transparency = transparency;
        self.enabled = enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/*
  Monitors the current windows specified in the config file. This is setup to target based on the window class rather than title (multiple windows open of X application...)
*/
#[inline(always)]
pub async fn monitor_windows(app_state: Arc<AppState>) {
    let mut window_cache = FxHashMap::with_capacity_and_hasher(8, Default::default());

    let refresh_interval = Duration::from_secs(1);
    let sleep_duration = Duration::from_millis(250);

    let mut last_refresh = Instant::now();
    let mut last_state = app_state.is_enabled().await;
    let mut config = app_state.get_config().await;
    let mut is_enabled = app_state.is_enabled().await;

    let mut config_rx = app_state.subscribe_config_updates();
    let mut enabled_rx = app_state.subscribe_enabled_updates();

    loop {
        if let Ok(new_config) = config_rx.try_recv() {
            config = new_config;
        }

        if let Ok(state) = enabled_rx.try_recv() {
            is_enabled = state;
        }

        if is_enabled {
            let now = Instant::now();
            if now.duration_since(last_refresh) > refresh_interval {
                refresh_window_cache(&config, &mut window_cache);
                last_refresh = now;
            }

            update_windows(&config, &mut window_cache);
        } else if last_state != is_enabled {
            reset_windows(&mut window_cache);
        }

        last_state = is_enabled;
        sleep(sleep_duration);
    }
}

#[inline(always)]
fn refresh_window_cache(config: &Config, cache: &mut FxHashMap<String, Vec<WindowHandleState>>) {
    for cfg in config.get_windows_non_mut().values() {
        let handles = cfg.get_window_hwnds();
        if handles.is_empty() {
            continue;
        }

        let key = cfg.get_cache_key();
        let states = cache.entry(key).or_insert_with(Vec::new);

        for &handle in &handles {
            if !states.iter().any(|state| state.handle == handle) {
                states.push(WindowHandleState::new(handle, 1, false));
            }
        }
        states.retain(|state| handles.contains(&state.handle));
    }

    // Clean up invalid windows and empty entries
    cache.values_mut().for_each(|states| {
        states.retain(|state| unsafe { IsWindow(Some(state.get_handle())).as_bool() });
    });
    cache.retain(|_, states| !states.is_empty());
}

#[inline(always)]
fn update_windows(config: &Config, window_cache: &mut FxHashMap<String, Vec<WindowHandleState>>) {
    for window_config in config.get_windows_non_mut().values() {
        if let Some(handle_states) = window_cache.get_mut(&window_config.get_cache_key()) {
            let mut new_transparency = window_config.get_transparency();
            let new_state = window_config.is_enabled();

            for state in handle_states.iter_mut() {
                if state.get_transparency() != window_config.get_transparency()
                    || state.is_enabled() != new_state
                {
                    if new_state == false {
                        new_transparency = 255;
                    }
                    if make_window_transparent(state.get_handle(), new_transparency).is_ok() {
                        state.update_state(window_config.get_transparency(), new_state);
                    }
                }
            }
        }
    }
}

#[inline(always)]
fn reset_windows(window_cache: &mut FxHashMap<String, Vec<WindowHandleState>>) {
    window_cache
        .values_mut()
        .flat_map(|handles| handles.iter_mut())
        .for_each(|handle| {
            if make_window_transparent(handle.get_handle(), 255).is_ok() {
                handle.update_state(255, false);
            }
        });
}
