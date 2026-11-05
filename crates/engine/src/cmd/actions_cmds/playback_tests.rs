//! Playback Options (`actions.setPlayback`): validation, preferences and headless playback.
use super::*;
use serde_json::json;

fn options(s: &Session) -> (crate::prefs::PlaybackPerformance, u8, bool) {
    let a = &s.prefs().actions;
    (a.performance, a.pause_seconds, a.create_single_history_state)
}

#[test]
fn set_playback_refuses_bad_values_and_changes_nothing() {
    let mut s = Session::new();
    let before = options(&s);
    for bad in [
        json!({"performance": "fast"}),
        json!({"performance": 1}),
        json!({"performance": null}),
        json!({"performance": "pause", "seconds": 0}),
        json!({"performance": "pause", "seconds": 61}),
        json!({"performance": "pause", "seconds": 1.5}),
        json!({"performance": "pause", "seconds": -1}),
        json!({"performance": "pause", "seconds": "5"}),
        json!({"performance": "pause", "seconds": 256}),
        json!({"performance": "pause", "seconds": 1e300}),
        json!({"performance": "step", "singleState": "yes"}),
        json!({"singleState": null}),
        json!([]),
        json!("pause"),
        json!(null),
    ] {
        assert!(s.execute("actions.setPlayback", bad.clone()).is_err(), "{bad} is refused");
        assert_eq!(options(&s), before, "{bad} changes nothing, also not its valid keys");
    }
}

#[test]
fn set_playback_stores_each_value_in_preferences() {
    use crate::prefs::PlaybackPerformance::*;
    let mut s = Session::new();
    assert_eq!(options(&s), (Accelerated, 1, true), "the defaults the reference application ships");
    let r = s.execute("actions.setPlayback", json!({"performance": "pause", "seconds": 60})).unwrap();
    assert_eq!(r, json!({"performance": "pause", "seconds": 60, "singleState": true}));
    s.execute("actions.setPlayback", json!({"seconds": 1})).unwrap();
    assert_eq!(options(&s), (Pause, 1, true), "the range's ends are accepted");
    s.execute("actions.setPlayback", json!({"performance": "step", "singleState": false})).unwrap();
    assert_eq!(options(&s), (StepByStep, 1, false), "the seconds stay while another mode is chosen");
    assert_eq!(s.execute("actions.setPlayback", json!({})).unwrap(), json!({"performance": "step", "seconds": 1, "singleState": false}));
    // A preferences file round trip keeps all three.
    s.execute("actions.setPlayback", json!({"performance": "pause", "seconds": 7})).unwrap();
    let saved = s.prefs_to_json();
    let mut t = Session::new();
    assert!(t.load_prefs_json(&saved).unwrap().is_empty());
    assert_eq!(options(&t), (Pause, 7, false));
    // prefs.set validates the same range and names.
    assert!(t.execute("prefs.set", json!({"path": "actions.pauseSeconds", "value": 0})).is_err());
    assert!(t.execute("prefs.set", json!({"path": "actions.pauseSeconds", "value": 61})).is_err());
    assert!(t.execute("prefs.set", json!({"path": "actions.pauseSeconds", "value": 2.5})).is_err());
    assert!(t.execute("prefs.set", json!({"path": "actions.performance", "value": "slow"})).is_err());
    t.execute("prefs.set", json!({"path": "actions.performance", "value": "accelerated"})).unwrap();
    assert_eq!(options(&t).0, Accelerated);
}

#[test]
fn older_or_hand_edited_preferences_load_within_range() {
    let mut s = Session::new();
    // Before Playback Options: only the history flag.
    assert!(s.load_prefs_json(r#"{"actions":{"createSingleHistoryState":false}}"#).unwrap().is_empty());
    assert_eq!(options(&s), (crate::prefs::PlaybackPerformance::Accelerated, 1, false));
    s.load_prefs_json(r#"{"actions":{"pauseSeconds":200}}"#).unwrap();
    assert_eq!(s.prefs().actions.pause_seconds(), 60, "a number out of range plays at the nearest end");
    s.load_prefs_json(r#"{"actions":{"pauseSeconds":0}}"#).unwrap();
    assert_eq!(s.prefs().actions.pause_seconds(), 1);
}

#[test]
fn set_playback_keeps_the_history_policy_of_a_running_play() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.actions.list.push(Action::new("Change", vec![("actions.setPlayback".to_string(), json!({"singleState": false}))]));
    let played = s.execute("actions.play", json!({"action": "Change"})).unwrap();
    assert!(played.get("failed").is_some(), "an action can't change Playback Options while it plays: {played}");
    assert!(s.prefs().actions.create_single_history_state);
}

/// Headless playback ignores the performance mode: one call runs every step at once, in one
/// history state with Create a single history state on and one per step with it off.
#[test]
fn headless_play_runs_the_whole_action_in_every_mode() {
    for performance in ["accelerated", "step", "pause"] {
        for single in [true, false] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
            s.execute("actions.setPlayback", json!({"performance": performance, "seconds": 60, "singleState": single})).unwrap();
            let steps = ["A", "B", "C"].map(|n| ("layer.new.layer".to_string(), json!({"name": n}))).to_vec();
            s.actions.list.push(Action::new("Three", steps));
            let layers = s.active().unwrap().doc.layers.len();
            let past = s.active().unwrap().history.past_len();
            let t0 = std::time::Instant::now();
            let played = s.execute("actions.play", json!({"action": "Three"})).unwrap();
            assert!(t0.elapsed() < std::time::Duration::from_secs(30), "{performance}: no wait headless");
            assert_eq!(played["ran"], 3, "{performance}/{single}: {played}");
            assert_eq!(s.active().unwrap().doc.layers.len(), layers + 3);
            let added = if single { 1 } else { 3 };
            assert_eq!(s.active().unwrap().history.past_len(), past + added, "{performance}/{single}");
        }
    }
}
