//! Edit › Color Settings › Spot drives the compositor's application-wide spot dot gain
//! (`photocraft_compose::multichannel::spot_dot_gain`), which the ink preview of Multichannel
//! documents adds to every channel.
//!
//! This test has its own integration-test binary (its own process) for the same reason as
//! `text_gamma.rs`: changing the gain changes how every Multichannel document composites, so a
//! concurrent test could flatten one document under two settings. Keep any other test that
//! changes the spot dot gain in this file.

use photocraft_compose::multichannel::{SPOT_DOT_GAIN, spot_dot_gain};
use photocraft_engine::Session;
use serde_json::json;

/// A Multichannel document with one transparent black ink at 50 %.
fn half_tint(s: &mut Session) {
    s.execute("file.new", json!({"width": 2, "height": 2, "mode": "grayscale", "background": "white"})).unwrap();
    s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
    s.execute("image.mode.multichannel", json!({})).unwrap();
}

#[test]
fn spot_dot_gain_setting() {
    let mut s = Session::new();
    assert_eq!(s.color.settings.spot_dot_gain, SPOT_DOT_GAIN, "Photoshop's default Spot working space: Dot Gain 20 %");
    for bad in [json!(-1), json!(50), json!("20%"), json!(true)] {
        assert!(s.execute("edit.colorSettings", json!({"spotDotGain": bad})).is_err(), "{bad}");
    }
    assert_eq!(spot_dot_gain(), SPOT_DOT_GAIN, "a refused request changes nothing");

    // The ink preview follows the setting: a 50 % dot darkens as the gain rises.
    half_tint(&mut s);
    let paper = |s: &Session| photocraft_compose::flatten(&s.active().unwrap().doc).px[0][0];
    let r = s.execute("edit.colorSettings", json!({"spotDotGain": 0})).unwrap();
    assert_eq!(r["settings"]["spotDotGain"], 0.0);
    assert_eq!(spot_dot_gain(), 0.0);
    let linear = paper(&s);
    s.execute("edit.colorSettings", json!({"spotDotGain": 38})).unwrap();
    assert_eq!(spot_dot_gain(), 38.0);
    let gained = paper(&s);
    assert!(gained < linear - 0.2, "38 % gain prints the 50 % dot much darker: {linear} → {gained}");

    // `prefs.set` and `prefs.reset` on `colorSettings` move the compositor's gain too. Same
    // function, not a second test: tests in one binary run concurrently.
    let in_sync = |s: &Session| spot_dot_gain() == s.color.settings.spot_dot_gain;
    s.execute("prefs.set", json!({"path": "colorSettings.spotDotGain", "value": 25})).unwrap();
    assert_eq!(spot_dot_gain(), 25.0);
    assert!(in_sync(&s));
    s.execute("prefs.reset", json!({"path": "colorSettings.spotDotGain"})).unwrap();
    assert_eq!(spot_dot_gain(), SPOT_DOT_GAIN);
    assert!(in_sync(&s));
    for reset in [json!({"path": "colorSettings"}), json!({})] {
        s.execute("edit.colorSettings", json!({"spotDotGain": 30})).unwrap();
        s.execute("prefs.reset", reset).unwrap();
        assert_eq!(spot_dot_gain(), SPOT_DOT_GAIN);
        assert!(in_sync(&s));
    }
    // Only colour changes move the process-wide gain: another session's unrelated
    // preferences leave this session's value alone.
    s.execute("edit.colorSettings", json!({"spotDotGain": 30})).unwrap();
    let mut other = Session::new();
    other.execute("prefs.set", json!({"path": "unitsAndRulers.pointSize", "value": "traditional"})).unwrap();
    other.execute("prefs.reset", json!({"path": "unitsAndRulers"})).unwrap();
    assert_eq!(spot_dot_gain(), 30.0);
    s.execute("prefs.reset", json!({"path": "colorSettings.spotDotGain"})).unwrap();

    // Saved with the preferences and applied when they load.
    s.execute("edit.colorSettings", json!({"spotDotGain": 38})).unwrap();
    let saved = s.prefs_to_json();
    photocraft_compose::multichannel::set_spot_dot_gain(SPOT_DOT_GAIN);
    let mut again = Session::new();
    again.load_prefs_json(&saved).unwrap();
    assert_eq!(again.color.settings.spot_dot_gain, 38.0);
    assert_eq!(spot_dot_gain(), 38.0);

    // A saved gain out of range is reported and reset, so later Color Settings edits still pass.
    let mut bad: serde_json::Value = serde_json::from_str(&saved).unwrap();
    bad["colorSettings"]["spotDotGain"] = json!(80);
    let mut third = Session::new();
    let invalid = third.load_prefs_json(&bad.to_string()).unwrap();
    assert!(invalid.iter().any(|p| p.path() == "colorSettings.spotDotGain"), "{:?}", invalid.iter().map(|p| p.path()).collect::<Vec<_>>());
    assert_eq!(third.color.settings.spot_dot_gain, SPOT_DOT_GAIN);
    assert_eq!(spot_dot_gain(), SPOT_DOT_GAIN);
    third.execute("edit.colorSettings", json!({"intent": "relative"})).unwrap();
    photocraft_compose::multichannel::set_spot_dot_gain(SPOT_DOT_GAIN);
}
