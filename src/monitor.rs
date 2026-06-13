use crate::{
    app_state::AppState,
    platform::{WindowHandle, WindowManager, wm},
    util::Config,
};
use core::time::Duration;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
// Delays between window monitor runs
// new windows, window updates etc.
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

    pub fn reset_to_opaque(&mut self, wm: &dyn WindowManager) {
        self.enabled = false;
        self.apply_alpha(wm);
    }

    fn apply_alpha(&self, wm: &dyn WindowManager) {
        let alpha = if self.enabled { self.alpha } else { 255 };
        wm.set_window_alpha(self.handle, alpha).ok();
    }

    pub fn update_window(&mut self, wm: &dyn WindowManager, new_alpha: u8, enabled: bool) {
        if self.get_alpha() != new_alpha || self.is_enabled() != enabled {
            self.update_state(new_alpha, enabled);
            self.apply_alpha(wm);
        }
    }
}

/*
  Monitors the current windows specified in the config file. This is setup to target based on the window class rather than title (multiple windows open of X application...)
*/
pub async fn monitor_windows(app_state: Arc<AppState>) {
    let wm = wm();

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
                reset_windows(wm, &mut window_cache);
                break;
            }
            Ok(new_config) = application_config.recv() => {
                config = new_config;
            }
            Ok(state) = application_toggle.recv() => {
                if state != is_enabled && is_enabled {
                    reset_windows(wm, &mut window_cache);
                }
                is_enabled = state;
            }
            _ = tokio::time::sleep(refresh_interval) => {
                if is_enabled {
                    refresh_window_cache(wm, &mut config, &mut window_cache);
                    update_windows(wm, &config, &mut window_cache);
                }
            }
            else => break
        }
    }
}

fn refresh_window_cache(
    wm: &dyn WindowManager,
    config: &mut Config,
    cache: &mut HashMap<String, Vec<WindowHandleState>>,
) {
    for cfg in config.get_windows().values_mut() {
        let handles = cfg.get_window_hwnds(wm);
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
    wm: &dyn WindowManager,
    config: &Config,
    window_cache: &mut HashMap<String, Vec<WindowHandleState>>,
) {
    for window_config in config.get_windows_non_mut().values() {
        if let Some(handle_states) = window_cache.get_mut(&window_config.get_cache_key()) {
            for state in handle_states.iter_mut() {
                state.update_window(wm, window_config.get_alpha(), window_config.is_enabled());
            }
        }
    }
}

fn reset_windows(
    wm: &dyn WindowManager,
    window_cache: &mut HashMap<String, Vec<WindowHandleState>>,
) {
    window_cache
        .values_mut()
        .flat_map(|handles| handles.iter_mut())
        .for_each(|handle| handle.reset_to_opaque(wm));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        platform::{CursorPoint, WindowInfo},
        window_config::WindowConfig,
    };
    use anyhow::Result;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering::Relaxed},
    };

    /// In-memory [`WindowManager`] for tests and benchmarks. Holds a fake set of
    /// windows keyed by class and counts the OS calls the monitor makes, so the
    /// hot path can be exercised with zero real windows. Every method that isn't
    /// driven by the monitor cycle is an inert stub.
    struct MockManager {
        windows: Mutex<HashMap<String, Vec<WindowHandle>>>,
        set_alpha_calls: AtomicUsize,
        enumerate_calls: AtomicUsize,
    }

    impl MockManager {
        fn new() -> Self {
            Self {
                windows: Mutex::new(HashMap::new()),
                set_alpha_calls: AtomicUsize::new(0),
                enumerate_calls: AtomicUsize::new(0),
            }
        }

        fn set_windows(&self, class: &str, handles: Vec<WindowHandle>) {
            self.windows
                .lock()
                .unwrap()
                .insert(class.to_owned(), handles);
        }

        fn set_alpha_calls(&self) -> usize {
            self.set_alpha_calls.load(Relaxed)
        }

        fn reset_counts(&self) {
            self.set_alpha_calls.store(0, Relaxed);
            self.enumerate_calls.store(0, Relaxed);
        }
    }

    impl WindowManager for MockManager {
        fn set_window_alpha(&self, _handle: WindowHandle, _alpha: u8) -> Result<()> {
            self.set_alpha_calls.fetch_add(1, Relaxed);
            Ok(())
        }

        fn enumerate_windows(&self, _process_name: &str, window_class: &str) -> Vec<WindowHandle> {
            self.enumerate_calls.fetch_add(1, Relaxed);
            self.windows
                .lock()
                .unwrap()
                .get(window_class)
                .cloned()
                .unwrap_or_default()
        }

        fn find_parent_from_child_class(
            &self,
            _child_class: &str,
        ) -> Result<Option<(WindowHandle, String)>> {
            Ok(None)
        }
        fn get_cursor_pos(&self) -> Result<CursorPoint> {
            Ok(CursorPoint::default())
        }
        fn is_left_click(&self) -> bool {
            false
        }
        fn get_window_info_at(&self, _point: CursorPoint) -> Result<WindowInfo> {
            Ok(WindowInfo::default())
        }
        fn is_elevated_at(&self, _point: CursorPoint) -> bool {
            false
        }
        fn is_running_as_admin(&self) -> bool {
            false
        }
        fn set_autostart(&self, _enabled: bool) -> Result<()> {
            Ok(())
        }
        fn get_autostart_state(&self) -> bool {
            false
        }
        fn open_path(&self, _path: &str) -> Result<()> {
            Ok(())
        }
        fn process_name_from_pid(&self, _pid: u32) -> Result<String> {
            Ok(String::new())
        }
    }

    /// A config with `rules` enabled rules (`proc{i}` / `class{i}`), each at `alpha`.
    fn seed_config(rules: usize, alpha: u8) -> Config {
        let mut config = Config::new();
        for r in 0..rules {
            let info = WindowInfo {
                class_name: format!("class{r}"),
                process_name: format!("proc{r}"),
            };
            let window_config = WindowConfig::new(&info, alpha);
            config
                .get_windows()
                .insert(window_config.get_key(), window_config);
        }
        config
    }

    /// A mock holding `windows_per_rule` distinct handles for each rule's class.
    fn seed_mock(rules: usize, windows_per_rule: usize) -> MockManager {
        let mock = MockManager::new();
        let mut next: u64 = 1;
        for r in 0..rules {
            let handles = (0..windows_per_rule)
                .map(|_| {
                    let handle = WindowHandle(next);
                    next += 1;
                    handle
                })
                .collect();
            mock.set_windows(&format!("class{r}"), handles);
        }
        mock
    }

    fn run_cycle(
        mock: &MockManager,
        config: &mut Config,
        cache: &mut HashMap<String, Vec<WindowHandleState>>,
    ) {
        refresh_window_cache(mock, config, cache);
        update_windows(mock, config, cache);
    }

    #[test]
    fn cache_eliminates_redundant_set_alpha_calls() {
        let mut config = seed_config(10, 128);
        let mock = seed_mock(10, 20);
        let mut cache = HashMap::new();

        // Cold cycle: every matching window has its alpha applied exactly once.
        run_cycle(&mock, &mut config, &mut cache);
        assert_eq!(mock.set_alpha_calls(), 10 * 20);

        // Warm cycle: nothing changed, so the cache suppresses all OS writes.
        mock.reset_counts();
        run_cycle(&mock, &mut config, &mut cache);
        assert_eq!(
            mock.set_alpha_calls(),
            0,
            "an unchanged cycle must not touch the OS"
        );
    }

    #[test]
    fn only_changed_rule_reapplies() {
        let mut config = seed_config(5, 128);
        let mock = seed_mock(5, 10);
        let mut cache = HashMap::new();

        run_cycle(&mock, &mut config, &mut cache);
        mock.reset_counts();

        // Bump a single rule's alpha; only its windows should be re-applied.
        config
            .get_windows()
            .get_mut("proc2|class2")
            .unwrap()
            .set_alpha(64);

        run_cycle(&mock, &mut config, &mut cache);
        assert_eq!(mock.set_alpha_calls(), 10);
    }

    #[test]
    fn closed_windows_do_not_panic() {
        // Regression guard for the old `unwrap()` on a vanished window handle.
        let mut config = seed_config(1, 128);
        let mock = seed_mock(1, 5);
        let mut cache = HashMap::new();

        run_cycle(&mock, &mut config, &mut cache);

        // Every window of the rule disappears between cycles.
        mock.set_windows("class0", Vec::new());
        run_cycle(&mock, &mut config, &mut cache);

        assert!(
            cache.is_empty(),
            "cache should drop rules with no live windows"
        );
    }

    /// Generates a steady-state (warm-cache) bench at a given scale. This is the
    /// common case: every 120 ms the loop enumerates windows and diffs them
    /// against the cache, applying opacity only where something changed (here,
    /// nothing — so no `set_window_alpha` calls). Total windows = `rules * per`.
    macro_rules! steady_state_bench {
        ($name:ident, $rules:expr, $per:expr) => {
            #[bench]
            fn $name(b: &mut test::Bencher) {
                let mut config = seed_config($rules, 128);
                let mock = seed_mock($rules, $per);
                let mut cache = HashMap::new();
                run_cycle(&mock, &mut config, &mut cache); // warm the cache
                b.iter(|| run_cycle(&mock, &mut config, &mut cache));
            }
        };
    }

    //                    name                    rules  windows/rule  (= total)
    steady_state_bench!(steady_00100_windows, 5, 20); //         100
    steady_state_bench!(steady_01000_windows, 50, 20); //       1000
    steady_state_bench!(steady_05000_windows, 250, 20); //      5000
    steady_state_bench!(steady_10000_windows, 500, 20); //     10000

    /// Cold-cache counterpart at 1000 windows: the cache is rebuilt from scratch
    /// every iteration, so every window also takes an opacity write. The gap
    /// between this and `steady_01000_windows` is the work the cache saves on a
    /// normal cycle.
    #[bench]
    fn cold_01000_windows(b: &mut test::Bencher) {
        let mut config = seed_config(50, 128);
        let mock = seed_mock(50, 20);
        b.iter(|| {
            let mut cache = HashMap::new();
            run_cycle(&mock, &mut config, &mut cache);
        });
    }
}
