use std::{collections::HashMap, os::raw::c_void, sync::Arc};
use tokio::time::Duration;
use win_utils::{get_window_hwnds, make_window_transparent};
use windows::Win32::Foundation::HWND;

use crate::{win_utils, AppState};

pub async fn monitor_windows(app_state: Arc<AppState>) {
    let mut window_cache: HashMap<String, Vec<isize>> = HashMap::new();
    let mut transparency_cache: HashMap<isize, u8> = HashMap::new();
    let mut last_refresh = tokio::time::Instant::now();
    let mut now;

    loop {
        let config = app_state.config.read().await;

        now = tokio::time::Instant::now();

        if window_cache.is_empty() || now.duration_since(last_refresh) > Duration::from_secs(1) {
            // Clear caches periodically to prevent memory growth
            window_cache.clear();
            transparency_cache.clear();

            // Only cache currently configured windows
            for (_, window_config) in config.windows.iter() {
                let handles = get_window_hwnds(window_config.window_class.clone())
                    .into_iter()
                    .map(|hwnd| hwnd.0 as isize)
                    .collect::<Vec<_>>();

                if !handles.is_empty() {
                    window_cache.insert(window_config.window_class.clone(), handles);
                    // Pre-populate transparency cache
                    for &handle in &window_cache[&window_config.window_class] {
                        transparency_cache.insert(handle, window_config.transparency);
                    }
                }
            }
            last_refresh = now;
        }

        // Only update transparencies for windows that exist
        for (_, window_config) in config.windows.iter() {
            if let Some(handles) = window_cache.get(&window_config.window_class) {
                for &handle in handles {
                    if transparency_cache.get(&handle) != Some(&window_config.transparency) {
                        let hwnd = HWND(handle as *mut c_void);
                        if let Ok(()) = make_window_transparent(hwnd, &window_config.transparency) {
                            transparency_cache.insert(handle, window_config.transparency);
                        }
                    }
                }
            }
        }

        drop(config);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
