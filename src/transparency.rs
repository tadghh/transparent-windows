use std::os::raw::c_void;

use super::*;
use tokio::time::Duration;
use win_utils::{get_window_hwnds, make_window_transparent};
use windows::Win32::Foundation::HWND;

pub async fn monitor_windows(app_state: Arc<AppState>) {
    let mut window_cache: HashMap<String, Vec<isize>> = HashMap::new();
    let mut transparency_cache: HashMap<isize, u8> = HashMap::new();
    let mut last_refresh = tokio::time::Instant::now();

    loop {
        let config = app_state.config.read().await;

        let should_refresh_handles = window_cache.is_empty()
            || tokio::time::Instant::now().duration_since(last_refresh) > Duration::from_secs(1);

        for (_, window_config) in config.windows.iter() {
            let transparency = window_config.transparency;
            let class = &window_config.window_class;

            if should_refresh_handles {
                let handles = get_window_hwnds(class.clone())
                    .into_iter()
                    .map(|hwnd| hwnd.0 as isize)
                    .collect::<Vec<_>>();
                window_cache.insert(class.clone(), handles);
                last_refresh = tokio::time::Instant::now();
            }

            // Use cached handles
            if let Some(handles) = window_cache.get(class) {
                for &raw_handle in handles {
                    if transparency_cache.get(&raw_handle) != Some(&transparency) {
                        let handle = HWND(raw_handle as isize as *mut c_void);
                        if let Ok(()) = make_window_transparent(handle, &transparency) {
                            transparency_cache.insert(raw_handle, transparency);
                        }
                    }
                }
            }
        }
        drop(config);

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
