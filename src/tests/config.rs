//! Unit tests for config load/reset (declared via `#[path]` from `config.rs`, so
//! `super::*` reaches `Config`, `load_config`, and the private `reset_config_file`).
//!
//! Note: the corrupt-file path of `load_config` is intentionally not tested here —
//! it pops the Slint recovery window, which needs a graphical backend and so can't
//! run headless. The recoverable logic (`reset_config_file`) is exercised directly.

use super::*;
use crate::{
    platform::{Opacity, PickHover, PollingOpacity, WindowHandle, WindowInfo, WindowManager},
    transparency::rules::WindowRule,
};
use anyhow::Result;
use std::sync::atomic::{AtomicU32, Ordering};

/// A window manager that reports no live windows, so the opacity side-effects in
/// [`Config::apply_force`] are inert — these tests assert only on config state.
struct NoopWm;

impl PollingOpacity for NoopWm {
    fn enumerate_windows(&self, _process: &str, _class: &str) -> Vec<WindowHandle> {
        Vec::new()
    }
    fn set_window_alpha(&self, _handle: WindowHandle, _alpha: u8) -> Result<()> {
        Ok(())
    }
}

impl WindowManager for NoopWm {
    fn find_parent_from_child_class(&self, _child: &str) -> Result<Option<(WindowHandle, String)>> {
        Ok(None)
    }
    fn opacity(&self) -> Opacity<'_> {
        Opacity::Polling(self)
    }
    fn pick_window(&self, _on_hover: &(dyn Fn(PickHover) + Sync)) -> Result<Option<WindowInfo>> {
        Ok(None)
    }
}

/// Build a rule for `process`/`class` at `alpha`, with the given enabled state.
fn rule(process: &str, class: &str, alpha: u8, enabled: bool) -> WindowRule {
    let mut rule = WindowRule::new(
        &WindowInfo {
            class_name: class.to_owned(),
            process_name: process.to_owned(),
        },
        alpha,
    );
    rule.set_enabled(enabled);
    rule
}

/// A rule already forced from `child` onto `parent` (as `apply_force` would leave
/// it): re-keyed under the parent class with the child recorded as `old_class`.
fn forced_rule(process: &str, parent: &str, child: &str, alpha: u8) -> WindowRule {
    let mut rule = rule(process, parent, alpha, true);
    rule.set_forced(true);
    rule.set_old_classname(Some(child.to_owned()));
    rule
}

/// A unique temp path per call, so tests don't collide. Removed by the caller.
fn temp_path(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "winalpha_test_{}_{tag}_{n}.json",
        std::process::id()
    ))
}

#[test]
fn load_missing_file_yields_empty_config() {
    let path = temp_path("missing");
    let _ = fs::remove_file(&path); // ensure absent
    assert!(load_config(&path).windows().is_empty());
}

#[test]
fn load_round_trips_a_saved_config() {
    let path = temp_path("roundtrip");

    let mut config = Config::default();
    let rule = WindowRule::new(
        &WindowInfo {
            class_name: "Navigator".to_owned(),
            process_name: "firefox".to_owned(),
        },
        128,
    );
    let key = rule.get_key();
    config.windows_mut().insert(key.clone(), rule);

    fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

    let loaded = load_config(&path);
    assert_eq!(loaded.windows().len(), 1);
    assert_eq!(loaded.windows().get(&key).unwrap().get_alpha(), 128);

    let _ = fs::remove_file(&path);
}

#[test]
fn reset_produces_a_file_that_load_config_accepts() {
    // Regression guard: `reset_config_file` once wrote `[{}]`, which `load_config`
    // could not parse — so "Reset" left the file corrupt. It must now write a
    // valid empty config that round-trips cleanly.
    let path = temp_path("reset");
    fs::write(&path, "this is not valid json {").unwrap();

    reset_config_file(path.to_str().unwrap());

    // The reset file parses as a Config (not via the recovery fallback) and is empty.
    let raw = fs::read_to_string(&path).unwrap();
    let parsed: Config = serde_json::from_str(&raw).expect("reset file must be valid Config JSON");
    assert!(parsed.windows().is_empty());
    assert!(load_config(&path).windows().is_empty());

    let _ = fs::remove_file(&path);
}

// --- rule transitions (Config::upsert_rule / apply_force) ------------------

#[test]
fn upsert_inserts_a_new_rule() {
    let mut config = Config::default();
    config.upsert_rule(rule("firefox", "Navigator", 128, true));

    assert_eq!(config.windows().len(), 1);
    assert_eq!(
        config
            .windows()
            .get("firefox|Navigator")
            .unwrap()
            .get_alpha(),
        128
    );
}

#[test]
fn upsert_overwrites_a_plain_rule_with_the_same_key() {
    let mut config = Config::default();
    config.upsert_rule(rule("firefox", "Navigator", 128, true));
    config.upsert_rule(rule("firefox", "Navigator", 64, false));

    assert_eq!(config.windows().len(), 1, "same key must not duplicate");
    let stored = config.windows().get("firefox|Navigator").unwrap();
    assert_eq!(stored.get_alpha(), 64);
    assert!(!stored.is_enabled());
}

#[test]
fn upsert_updates_a_forced_rule_in_place_by_old_class() {
    // A rule forced from "Navigator" onto "Parent" is keyed "firefox|Parent". An
    // upsert that targets the original "Navigator" class must update *that* rule
    // in place (matched via its recorded old_class), not insert a second one.
    let mut config = Config::default();
    let forced = forced_rule("firefox", "Parent", "Navigator", 128);
    config.windows_mut().insert(forced.get_key(), forced);

    config.upsert_rule(rule("firefox", "Navigator", 200, false));

    assert_eq!(config.windows().len(), 1);
    let updated = config.windows().get("firefox|Parent").unwrap();
    assert_eq!(updated.get_alpha(), 200);
    assert!(!updated.is_enabled());
    assert!(
        config.windows().get("firefox|Navigator").is_none(),
        "must not create a second rule under the child class"
    );
}

#[test]
fn force_re_keys_a_rule_under_the_parent_class() {
    let mut config = Config::default();
    // A pre-existing plain rule under the child class, which forcing replaces.
    config.windows_mut().insert(
        "firefox|Navigator".to_owned(),
        rule("firefox", "Navigator", 100, true),
    );

    let mut to_force = rule("firefox", "Navigator", 128, true);
    to_force.set_forced(true);
    config.apply_force(to_force, "Parent", "Navigator".to_owned(), &NoopWm);

    assert_eq!(
        config.windows().len(),
        1,
        "child rule replaced, not added to"
    );
    assert!(config.windows().get("firefox|Navigator").is_none());
    let forced = config.windows().get("firefox|Parent").unwrap();
    assert_eq!(forced.get_window_class().as_str(), "Parent");
    assert_eq!(forced.get_old_classname().as_deref(), Some("Navigator"));
    assert!(forced.is_forced());
    assert_eq!(forced.get_alpha(), 128);
}

#[test]
fn unforce_disables_and_restores_the_child_class() {
    // Un-forcing finds the re-keyed rule by its old_class and disables it, putting
    // its class back to the original child class.
    let mut config = Config::default();
    let forced = forced_rule("firefox", "Parent", "Navigator", 128);
    config.windows_mut().insert(forced.get_key(), forced);

    let mut unforce = rule("firefox", "Navigator", 200, true);
    unforce.set_forced(false);
    config.apply_force(unforce, "Parent", "Navigator".to_owned(), &NoopWm);

    let updated = config
        .windows()
        .values()
        .next()
        .expect("rule still present");
    assert!(!updated.is_enabled(), "un-forced rule is disabled");
    assert!(!updated.is_forced());
    assert_eq!(updated.get_window_class().as_str(), "Navigator");
    assert_eq!(updated.get_alpha(), 200);
}
