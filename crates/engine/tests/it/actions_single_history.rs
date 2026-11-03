//! Public playback history semantics, independent of the Actions panel implementation.
use photocraft_engine::{Session, actions_cmds::Action};
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4})).unwrap();
    s.actions.list.push(Action::new(
        "Two layers",
        ["First", "Second"].into_iter().map(|name| ("layer.new.layer".into(), json!({"name":name}))).collect(),
    ));
    s
}

/// Default playback is one named edit, including the complete Undo and Redo states.
#[test]
fn default_playback_is_one_named_history_state() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let count = s.active().unwrap().history.past_len();
    let result = s.execute("actions.play", json!({"action":"Two layers"})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    assert_eq!(s.active().unwrap().history.past_len(), count + 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Action: Two layers"));
    let after = s.active().unwrap().doc.clone();
    assert_eq!(after.layers.len(), before.layers.len() + 2);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), before.layers.len());
    assert_eq!(s.active().unwrap().history.redo_label(), Some("Action: Two layers"));
    assert!(s.redo());
    assert_eq!(s.active().unwrap().doc.layers.len(), after.layers.len());
    assert_eq!(s.active().unwrap().doc.layers.iter().map(|l| &l.name).collect::<Vec<_>>(), after.layers.iter().map(|l| &l.name).collect::<Vec<_>>());
}

/// Disabling the persisted option retains the existing one-entry-per-step behavior.
#[test]
fn disabled_single_history_keeps_each_step() {
    let mut s = session();
    s.execute("actions.setHistory", json!({"singleState":false})).unwrap();
    let count = s.active().unwrap().history.past_len();
    let layers = s.active().unwrap().doc.layers.len();
    s.execute("actions.play", json!({"action":0})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), count + 2);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), layers + 1);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), layers);
}

/// Failure commits the completed prefix; an inner play uses its outer action's name and key.
#[test]
fn failures_and_nested_calls_keep_the_outer_history_entry() {
    let mut s = session();
    let layers = s.active().unwrap().doc.layers.len();
    s.actions.list.push(Action::new("Outer", vec![
        ("layer.new.layer".into(), json!({"name":"Outer layer"})),
        ("actions.play".into(), json!({"action":0})),
        ("not.a.command".into(), json!({})),
        ("layer.new.layer".into(), json!({"name":"Must not run"})),
    ]));
    let before = s.active().unwrap().history.past_len();
    let result = s.execute("actions.play", json!({"action":"Outer"})).unwrap();
    assert_eq!(result["failed"]["step"], 2);
    assert_eq!(s.active().unwrap().doc.layers.len(), layers + 3);
    assert_eq!(s.active().unwrap().history.past_len(), before + 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Action: Outer"));
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), layers);
    assert!(s.redo());
    assert_eq!(s.active().unwrap().doc.layers.len(), layers + 3);
}

/// No-op playback preserves redo, and separate plays never coalesce into one another.
#[test]
fn no_op_and_repeated_plays_preserve_history_boundaries() {
    let mut s = session();
    s.execute("actions.play", json!({"action":0})).unwrap();
    assert!(s.undo());
    let entries = s.active().unwrap().history.entries();
    let redo = s.active().unwrap().history.redo_label().map(str::to_owned);
    s.actions.list.push(Action::new("Empty", vec![]));
    s.execute("actions.play", json!({"action":"Empty"})).unwrap();
    assert_eq!(s.active().unwrap().history.entries(), entries);
    assert_eq!(s.active().unwrap().history.redo_label(), redo.as_deref());
    assert!(s.redo());
    let count = s.active().unwrap().history.past_len();
    let layers = s.active().unwrap().doc.layers.len();
    s.execute("actions.play", json!({"action":0})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), count + 1);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), layers);
}

/// Older preference files default on; an explicit off survives the public save/load path.
#[test]
fn action_history_setting_is_persisted_and_rejects_wrong_types() {
    let mut s = Session::new();
    assert!(s.prefs().actions.create_single_history_state);
    assert!(s.execute("actions.setHistory", json!({"singleState":"false"})).is_err());
    assert!(s.prefs().actions.create_single_history_state);
    s.execute("actions.setHistory", json!({"singleState":false})).unwrap();
    let mut restored = Session::new();
    assert!(restored.load_prefs_json(&s.prefs_to_json()).unwrap().is_empty());
    assert!(!restored.prefs().actions.create_single_history_state);
    restored.load_prefs_json("{}").unwrap();
    assert!(restored.prefs().actions.create_single_history_state);
}
