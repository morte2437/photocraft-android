//! Catch-up on Stroke End starts unchecked, as in Photoshop (#11887).

use crate::StrokePoint;
use crate::brush::{BrushSettings, Smoothing};
use crate::dynamics::smooth_points;

/// A quick drag from x = 100 to x = 300 in 40 steps.
fn drag() -> Vec<StrokePoint> {
    (0..=40).map(|i| StrokePoint::new(100.0 + f64::from(i) * 5.0, 100.0, 1.0)).collect()
}

#[test]
fn catch_up_on_end_defaults_off_and_the_others_stay() {
    let s = Smoothing::default();
    assert!(!s.catch_up_on_end);
    assert!(!s.pulled_string && s.catch_up && s.adjust_for_zoom);
    assert!(!BrushSettings::default().smoothing.catch_up_on_end);
    let json = serde_json::to_value(&BrushSettings::default().smoothing).unwrap();
    assert_eq!(json["catchUpOnEnd"], false);
}

#[test]
fn default_smoothed_stroke_ends_short_of_the_release_point() {
    let cfg = Smoothing { amount: 0.5, ..Default::default() };
    let out = smooth_points(&cfg, 1.0, &drag());
    let end = out.last().unwrap();
    assert!(end.x < 299.0, "ends at {}", end.x);
}

#[test]
fn catch_up_on_end_on_reaches_the_release_point() {
    let cfg = Smoothing { amount: 0.5, catch_up_on_end: true, ..Default::default() };
    let out = smooth_points(&cfg, 1.0, &drag());
    let end = out.last().unwrap();
    assert_eq!((end.x, end.y), (300.0, 100.0));
}
