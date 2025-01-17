use crate::{
    win_utils::{convert_to_full, get_window_hwnds, make_window_transparent},
    AppState, Config, RulesStorage, RulesWindow, TransparencyRule, WindowConfig,
};
use core::fmt::Error;
use slint::{ComponentHandle, VecModel};
use std::{collections::HashMap, os::raw::c_void, rc::Rc, sync::Arc, sync::Mutex};
use tokio::time::{sleep, Duration, Instant};
use windows::Win32::Foundation::HWND;

const MINIMUM_PERCENTAGE: u8 = 30;

/*
  Monitors the current windows specified in the config file. This is setup to target based on the window class rather than title (multiple windows open of X application...)
*/
pub async fn monitor_windows(app_state: Arc<AppState>) {
    let mut window_cache: HashMap<String, Vec<isize>> = HashMap::new();
    let mut transparency_cache: HashMap<isize, u8> = HashMap::new();
    let mut last_refresh = Instant::now();
    let mut last_config_str = String::new();

    loop {
        let config = app_state.get_config().read().await;

        let current_config_str =
            serde_json::to_string(&config.get_windows_non_mut().clone()).unwrap_or_default();

        let should_refresh_handles = window_cache.is_empty()
            || Instant::now().duration_since(last_refresh) > Duration::from_secs(1)
            || current_config_str != last_config_str;

        if should_refresh_handles {
            window_cache.clear();
            last_config_str = current_config_str;
        }

        for (_, window_config) in config.get_windows_non_mut().iter() {
            let transparency = window_config.get_transparency();
            let class = window_config.get_window_class();

            if should_refresh_handles {
                let handles = get_window_hwnds(class.clone())
                    .into_iter()
                    .map(|hwnd| hwnd.0 as isize)
                    .collect::<Vec<_>>();
                window_cache.insert(class.clone(), handles);
                last_refresh = Instant::now();
            }

            if let Some(handles) = window_cache.get(class) {
                for &raw_handle in handles {
                    if transparency_cache.get(&raw_handle) != Some(&transparency) {
                        let handle = HWND(raw_handle as isize as *mut c_void);
                        if let Ok(()) = make_window_transparent(handle, &transparency) {
                            transparency_cache.insert(raw_handle, *transparency);
                        }
                    }
                }
            }
        }

        drop(config);
        sleep(Duration::from_millis(50)).await;
    }
}

/*
  Creates the rules window, this is so the user can see what rules are currently active.
  There is hardcoded minimum of 30%
*/
pub fn create_rules_window(mut config: Config) -> Result<Config, core::fmt::Error> {
    let window = RulesWindow::new().unwrap();
    let window_handle = window.as_weak();

    // Store the latest config in an Arc<Mutex>
    let latest_config = Arc::new(Mutex::new(None::<Config>));

    let window_info: Vec<TransparencyRule> =
        config.get_windows().values().map(|w| w.into()).collect();

    window
        .global::<RulesStorage>()
        .set_items(Rc::new(VecModel::from(window_info)).into());

    window.on_submit({
        let latest_config = Arc::clone(&latest_config);
        move |mut value: TransparencyRule| {
            if value.transparency < MINIMUM_PERCENTAGE as i32 {
                value.transparency = MINIMUM_PERCENTAGE as i32;
            }

            let window_config = WindowConfig::new(
                value.process_name.to_string(),
                value.window_class.to_string(),
                convert_to_full(value.transparency as u8),
            );

            config
                .get_windows()
                .insert(window_config.get_key(), window_config);

            // Store the latest config
            if let Ok(mut latest) = latest_config.lock() {
                *latest = Some(config.clone());
            }
        }
    });

    window.on_cancel(move || {
        if let Some(window) = window_handle.upgrade() {
            window.hide().unwrap();
            drop(window);
        }
    });

    window.run().unwrap();

    let result = match latest_config.lock() {
        Ok(mut guard) => guard.take().ok_or(Error),
        Err(_) => Err(Error),
    };
    result
}
