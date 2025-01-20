use crate::{
    win_utils::{convert_to_full, get_window_hwnds, make_window_transparent},
    AppState, RulesStorage, RulesWindow, TransparencyRule,
};

use slint::{ComponentHandle, VecModel};
use std::{collections::HashMap, fs, os::raw::c_void, rc::Rc, sync::Arc};
use tokio::time::{sleep, Duration, Instant};
use windows::Win32::Foundation::HWND;

/*
  Monitors the current windows specified in the config file. This is setup to target based on the window class rather than title (multiple windows open of X application...)
*/
pub async fn monitor_windows(app_state: Arc<AppState>) {
    // These caches are used to prevent repeated/uneeded api calls. Rather crude.
    let mut window_cache: HashMap<String, Vec<isize>> = HashMap::new();
    let mut transparency_cache: HashMap<isize, u8> = HashMap::new();
    let mut enabled_cache: HashMap<isize, bool> = HashMap::new();
    let mut last_refresh = Instant::now();
    let mut last_config_str = String::new();
    let mut last_state = app_state.is_enabled().await;

    loop {
        let config = app_state.get_config().await;
        let is_enabled = app_state.is_enabled().await;

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
                    if is_enabled {
                        let handle = HWND(raw_handle as isize as *mut c_void);
                        if transparency_cache.get(&raw_handle) != Some(&transparency)
                            && *window_config.is_enabled()
                        {
                            if let Ok(()) = make_window_transparent(handle, &transparency) {
                                transparency_cache.insert(raw_handle, *transparency);
                                enabled_cache.insert(raw_handle, *window_config.is_enabled());
                            }
                        } else if enabled_cache.get(&raw_handle) != Some(window_config.is_enabled())
                        {
                            if let Ok(()) = make_window_transparent(handle, &(255)) {
                                transparency_cache.clear();
                                enabled_cache.insert(raw_handle, *window_config.is_enabled());
                            }
                        }
                    } else if last_state != is_enabled {
                        let handle = HWND(raw_handle as isize as *mut c_void);

                        if let Ok(()) = make_window_transparent(handle, &(255)) {
                            transparency_cache.clear();
                            enabled_cache.insert(raw_handle, *window_config.is_enabled());
                        }
                    }
                }
            }
        }
        last_state = is_enabled;
        sleep(Duration::from_millis(50)).await;
    }
}

/*
  Creates the rules window, this is so the user can see what rules are currently active.
  There is hardcoded minimum of 30%
*/
pub async fn create_rules_window(app_state: Arc<AppState>) -> Result<(), core::fmt::Error> {
    let window = RulesWindow::new().unwrap();
    let window_handle = window.as_weak();

    let mut window_info = {
        let config = app_state.get_config().await;
        config
            .get_windows_non_mut()
            .values()
            .map(|w| w.into())
            .collect::<Vec<TransparencyRule>>()
    };

    // Oh boo hoo its sorted every time the rules window is opened 😢
    window_info.sort_by_key(|rule| rule.process_name.clone());

    window
        .global::<RulesStorage>()
        .set_items(Rc::new(VecModel::from(window_info)).into());

    window.on_submit({
        let app_state = Arc::clone(&app_state);

        move |value: TransparencyRule| {
            // We need to spawn a new future so we can write to the config file live.
            let app_state = Arc::clone(&app_state);
            tokio::spawn(async move {
                let mut config = app_state.get_config_mut().await;
                let key = &format!(
                    "{}|{}",
                    value.process_name.to_string(),
                    value.window_class.to_string()
                );
                println!("{:?}", value);
                let new_transparency = convert_to_full(value.transparency);

                let new_config = config
                    .get_windows()
                    .get_mut(key)
                    .expect("Okay funny guy stop messing with the config file.");
                new_config.set_transparency(new_transparency);
                new_config.set_enabled(value.enabled);

                if let Ok(config_json) = serde_json::to_string_pretty(&*config) {
                    if let Err(e) = fs::write(&app_state.get_config_path(), config_json) {
                        eprintln!("Failed to write config: {}", e);
                    }
                }
            });
        }
    });

    window.on_cancel(move || {
        if let Some(window) = window_handle.upgrade() {
            window.hide().unwrap();
        }
    });

    window.run().unwrap();
    Ok(())
}
