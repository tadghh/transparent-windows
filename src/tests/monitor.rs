//! Unit tests for the window monitor (declared via `#[path]` from `monitor.rs`,
//! so this stays the `crate::monitor::tests` module and `super::*` resolves to
//! the monitor module's private internals).

use super::*;
use crate::{
    platform::{Opacity, PickHover, WindowInfo, WindowManager},
    transparency::rules::WindowRule,
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

impl PollingOpacity for MockManager {
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
}

impl WindowManager for MockManager {
    fn find_parent_from_child_class(
        &self,
        _child_class: &str,
    ) -> Result<Option<(WindowHandle, String)>> {
        Ok(None)
    }
    fn opacity(&self) -> Opacity<'_> {
        Opacity::Polling(self)
    }
    fn pick_window(&self, _on_hover: &(dyn Fn(PickHover) + Sync)) -> Result<Option<WindowInfo>> {
        Ok(None)
    }
}

/// A config with `rules` enabled rules (`proc{i}` / `class{i}`), each at `alpha`.
fn seed_config(rules: usize, alpha: u8) -> Config {
    let mut config = Config::default();
    for r in 0..rules {
        let info = WindowInfo {
            class_name: format!("class{r}"),
            process_name: format!("proc{r}"),
        };
        let rule = WindowRule::new(&info, alpha);
        config.windows_mut().insert(rule.get_key(), rule);
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
        .windows_mut()
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

/// Churn at 1000 windows: unlike the steady-state benches (which re-scan an
/// unchanging world and so only measure the idle 100%-cache-hit path), each
/// iteration closes a fraction of every class's windows and opens fresh ones.
/// That forces the cache to drop the gone handles, track the new ones, and issue
/// real `set_window_alpha` writes for them — i.e. the path the cache actually
/// exists to optimise, not the no-op scan.
#[bench]
fn churn_01000_windows(b: &mut test::Bencher) {
    const RULES: usize = 50;
    const PER_RULE: usize = 20;
    const CHURN_PER_RULE: usize = 4; // 20% of each class turns over per cycle

    let mut config = seed_config(RULES, 128);
    let mock = seed_mock(RULES, PER_RULE);
    let mut cache = HashMap::new();
    run_cycle(&mock, &mut config, &mut cache); // warm the cache first

    // Mirror the mock's live handles (same numbering as `seed_mock`) so we can
    // rotate a slice of each class without disturbing the rest.
    let mut live: Vec<Vec<WindowHandle>> = (0..RULES)
        .map(|r| {
            (0..PER_RULE)
                .map(|w| WindowHandle((r * PER_RULE + w + 1) as u64))
                .collect()
        })
        .collect();
    let mut next: u64 = (RULES * PER_RULE + 1) as u64;

    b.iter(|| {
        for (r, handles) in live.iter_mut().enumerate() {
            // Replace the first CHURN_PER_RULE handles with brand-new ones: those
            // windows "closed" and equally many "opened".
            for slot in handles.iter_mut().take(CHURN_PER_RULE) {
                *slot = WindowHandle(next);
                next += 1;
            }
            mock.set_windows(&format!("class{r}"), handles.clone());
        }
        run_cycle(&mock, &mut config, &mut cache);
    });
}
