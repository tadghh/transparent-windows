//! Unit tests for the opacity conversions (declared via `#[path]` from
//! `platform/mod.rs`, so `super::*` reaches the module's conversion helpers).

use super::{alpha_to_percent, percent_to_alpha};

#[test]
fn percent_to_alpha_maps_the_endpoints() {
    assert_eq!(percent_to_alpha(100), 255, "100% is fully opaque");
    assert_eq!(percent_to_alpha(50), 128);
    assert_eq!(percent_to_alpha(30), 77, "30% is the minimum");
}

#[test]
fn percent_to_alpha_clamps_to_the_minimum_floor() {
    // Anything below the 30% floor (including 0 and negatives) is pulled up to
    // the 30% alpha, so a rule can never request a fully-invisible window.
    let floor = percent_to_alpha(30);
    assert_eq!(percent_to_alpha(0), floor);
    assert_eq!(percent_to_alpha(-50), floor);
    assert_eq!(percent_to_alpha(29), floor);
}

#[test]
fn percent_to_alpha_clamps_above_one_hundred() {
    assert_eq!(percent_to_alpha(101), 255);
    assert_eq!(percent_to_alpha(10_000), 255);
}

#[test]
fn alpha_to_percent_maps_the_endpoints() {
    assert_eq!(alpha_to_percent(255), 100);
    assert_eq!(alpha_to_percent(128), 50);
    assert_eq!(alpha_to_percent(0), 0);
}

#[test]
fn percent_alpha_round_trips_within_range() {
    // For percents at or above the floor, percent -> alpha -> percent is stable.
    for percent in [30, 50, 75, 100] {
        let alpha = percent_to_alpha(percent);
        assert_eq!(
            alpha_to_percent(alpha) as i32,
            percent,
            "round trip drifted at {percent}%"
        );
    }
}
