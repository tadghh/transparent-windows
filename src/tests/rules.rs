//! Unit tests for the window-rule model (declared via `#[path]` from `rules.rs`,
//! so `super::*` reaches `WindowRule` and its conversions).

use super::*;
use crate::{TransparencyRule, platform::WindowInfo};

fn rule(process: &str, class: &str, alpha: u8) -> WindowRule {
    WindowRule::new(
        &WindowInfo {
            class_name: class.to_owned(),
            process_name: process.to_owned(),
        },
        alpha,
    )
}

#[test]
fn default_rule_is_opaque() {
    // Regression guard: a defaulted rule must be fully opaque, never invisible.
    assert_eq!(WindowRule::default().get_alpha(), 255);
}

#[test]
fn missing_transparency_field_defaults_to_opaque() {
    // Regression guard for the container-level `#[serde(default)]`: a partial /
    // hand-edited rule with no `transparency` must fall back to 255 (opaque), not
    // the `u8` default of 0 (a fully-transparent, invisible window).
    let parsed: WindowRule = serde_json::from_str("{}").expect("empty object is valid");
    assert_eq!(parsed.get_alpha(), 255);
    assert!(!parsed.is_enabled());
    assert!(!parsed.is_forced());
    assert!(parsed.get_old_classname().is_none());

    // A field that *is* present is still honoured.
    let parsed: WindowRule =
        serde_json::from_str(r#"{"transparency": 100}"#).expect("partial object is valid");
    assert_eq!(parsed.get_alpha(), 100);
}

#[test]
fn key_and_cache_key_formats() {
    let rule = rule("firefox", "Navigator", 128);
    assert_eq!(rule.get_key(), "firefox|Navigator");
    assert_eq!(
        rule.get_cache_key(),
        "Navigator",
        "cache key is the class only"
    );
}

#[test]
fn conversion_round_trips_through_the_ui_rule() {
    let original = rule("firefox", "Navigator", 128);

    let ui: TransparencyRule = (&original).into();
    assert_eq!(ui.process_name, "firefox");
    assert_eq!(ui.window_class, "Navigator");
    assert_eq!(ui.transparency, 50, "alpha 128 displays as 50%");

    let back: WindowRule = ui.into();
    assert_eq!(back.get_alpha(), 128, "50% restores to alpha 128");
    assert_eq!(back.get_name(), "firefox");
    assert_eq!(back.get_window_class(), "Navigator");
}

#[test]
fn old_class_maps_to_and_from_empty_string() {
    // The UI rule carries `old_class` as a plain string; the model uses
    // `Option`, with the empty string standing in for `None`.
    let none_ui: TransparencyRule = (&rule("a", "b", 200)).into();
    assert_eq!(none_ui.old_class, "", "no old class -> empty string");

    let mut with_old = rule("a", "b", 200);
    with_old.set_old_classname(Some("Legacy".to_owned()));
    let some_ui: TransparencyRule = (&with_old).into();
    assert_eq!(some_ui.old_class, "Legacy");

    let back_none: WindowRule = none_ui.into();
    assert!(back_none.get_old_classname().is_none());
    let back_some: WindowRule = some_ui.into();
    assert_eq!(back_some.get_old_classname().as_deref(), Some("Legacy"));
}
