//! Unit tests for the KWin backend (declared via `#[path]` from `kwin/mod.rs`,
//! so this is the `crate::platform::kwin::tests` module and `super::{dbus,
//! script}` reach the backend's `pub(super)` identity helpers).

use super::{dbus, script};
use crate::platform::RuleSpec;

/// The `#[zbus::interface]` / `#[zbus(name)]` attributes on `HoverSink` need
/// literals, so they can't use the helpers. Pin those literals to the
/// app-id-derived values here: changing `APP_ID` without updating the attributes
/// (and the probe script, which is generated from the helpers) breaks this test
/// instead of silently breaking the picker at runtime.
#[test]
fn hover_identity_matches_zbus_attribute_literals() {
    assert_eq!(dbus::hover_iface(), "org.winalpha.Hover");
    assert_eq!(dbus::HOVER_METHOD, "Report");
}

/// `ui/hover-info.slint` sets `title: "winalpha-picker"`, which the probe matches
/// on to move the panel. The slint literal can't read `APP_ID`, so pin the two
/// together here.
#[test]
fn picker_caption_matches_slint_title() {
    assert_eq!(script::picker_caption(), "winalpha-picker");
}

#[test]
fn opacity_script_emits_class_and_opacity() {
    let js = script::build_script(&[RuleSpec {
        window_class: "firefox".to_owned(),
        alpha: 128,
    }]);
    // alpha 128 / 255 ≈ 0.5020, formatted to 4 dp.
    assert!(
        js.contains(r#"{"cls":"firefox","op":0.5020}"#),
        "rule not found in generated script:\n{js}"
    );
}

#[test]
fn opacity_script_escapes_quotes_and_backslashes() {
    // A class containing a quote/backslash must be escaped so the generated JS
    // stays valid (and can't break out of the string literal).
    let js = script::build_script(&[RuleSpec {
        window_class: r#"od"d\class"#.to_owned(),
        alpha: 255,
    }]);
    assert!(
        js.contains(r#""cls":"od\"d\\class""#),
        "class not escaped in:\n{js}"
    );
}

#[test]
fn empty_rule_set_emits_empty_array() {
    assert!(script::build_script(&[]).contains("var rules = [];"));
}

#[test]
fn hover_script_substitutes_every_placeholder() {
    let js = script::build_hover_script();
    assert!(
        !js.contains("__"),
        "unsubstituted placeholder remains in hover script:\n{js}"
    );
    // The identities the probe must call back with are filled in from the helpers.
    assert!(js.contains(&script::picker_caption()));
    assert!(js.contains(&dbus::hover_iface()));
    assert!(js.contains(dbus::HOVER_METHOD));
}
