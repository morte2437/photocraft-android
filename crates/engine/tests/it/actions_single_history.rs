//! Public playback history semantics, independent of the Actions panel implementation.
use photocraft_engine::{Session, actions_cmds::Action};
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4,"background":"#804020"})).unwrap();
    s.actions.list.push(Action::new("Two layers", ["First", "Second"].into_iter().map(|name| ("layer.new.layer".into(), json!({"name":name}))).collect()));
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
    assert_eq!(s.active().unwrap().doc.as_ref(), before.as_ref());
    assert_eq!(s.active().unwrap().history.redo_label(), Some("Action: Two layers"));
    assert!(s.redo());
    assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
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
    s.actions.list.push(Action::new(
        "Outer",
        vec![
            ("layer.new.layer".into(), json!({"name":"Outer layer"})),
            ("actions.play".into(), json!({"action":0})),
            ("not.a.command".into(), json!({})),
            ("layer.new.layer".into(), json!({"name":"Must not run"})),
        ],
    ));
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

/// A command's internal purge must not remove the outer action's only Undo snapshot.
#[test]
fn frame_compound_retains_undo_redo_content_and_layer_targets() {
    for single in [true, false] {
        let mut s = session();
        s.execute("layer.new.layer", json!({"name":"Subject"})).unwrap();
        s.execute("actions.setHistory", json!({"singleState":single})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let target = s.active().unwrap().active_layer;
        let selected = s.active().unwrap().selected_layers.clone();
        let count = s.active().unwrap().history.past_len();
        s.actions.list = vec![Action::new("Frame", vec![("layer.new.frameFromLayers".into(), json!({}))])];
        let result = s.execute("actions.play", json!({"action":0})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
        let after = s.active().unwrap().doc.clone();
        assert_ne!(after.as_ref(), before.as_ref());
        assert_eq!(s.active().unwrap().history.past_len(), count + 1);
        if single {
            assert_eq!(s.active().unwrap().history.undo_label(), Some("Action: Frame"));
        }
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc.as_ref(), before.as_ref());
        assert_eq!(s.active().unwrap().active_layer, target);
        assert_eq!(s.active().unwrap().selected_layers, selected);
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
    }
}

/// A real tilted crop's compound transaction remains inside the outer prefix and suffix.
#[test]
fn tilted_crop_between_edits_is_one_action() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let count = s.active().unwrap().history.past_len();
    s.actions.list = vec![Action::new(
        "Crop",
        vec![
            ("layer.renameLayer".into(), json!({"name":"Prefix"})),
            ("image.crop".into(), json!({"x":0,"y":0,"width":3,"height":3,"angle":10})),
            ("layer.new.layer".into(), json!({"name":"Suffix"})),
        ],
    )];
    let result = s.execute("actions.play", json!({"action":0})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    let after = s.active().unwrap().doc.clone();
    assert_eq!(s.active().unwrap().history.past_len(), count + 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Action: Crop"));
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.as_ref(), before.as_ref());
    assert!(s.redo());
    assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
}

/// Command-local rollback removes a refused copy but keeps the action's completed rename.
#[test]
fn again_copy_failure_preserves_the_completed_prefix() {
    let mut s = session();
    s.execute("layer.new.layerFromBackground", json!({"ifPresent":true})).unwrap();
    s.execute("edit.transform", json!({"matrix":[1,0,0,1,1,0]})).unwrap();
    s.execute("layer.setProps", json!({"locks":{"position":true}})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let count = s.active().unwrap().history.past_len();
    s.actions.list =
        vec![Action::new("Prefix", vec![("layer.renameLayer".into(), json!({"name":"Completed prefix"})), ("edit.transform.againCopy".into(), json!({}))])];
    let result = s.execute("actions.play", json!({"action":0})).unwrap();
    assert_eq!(result["failed"]["step"], 1, "{result}");
    let after = s.active().unwrap().doc.clone();
    assert_eq!(after.layers.len(), before.layers.len());
    assert_ne!(after.as_ref(), before.as_ref());
    assert_eq!(s.active().unwrap().history.past_len(), count + 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Action: Prefix"));
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.as_ref(), before.as_ref());
    assert!(s.redo());
    assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
}

/// A nonempty semantic no-op preserves the original document identity, rows, redo and clean state.
#[test]
fn same_name_rename_does_not_record_or_clear_redo() {
    let mut s = session();
    s.execute("layer.new.layer", json!({"name":"Later"})).unwrap();
    assert!(s.undo());
    let st = s.active().unwrap();
    let before = st.doc.clone();
    let entries = st.history.entries();
    let redo: Vec<_> = st.history.redo_labels().map(str::to_string).collect();
    let ids: Vec<_> = (0..entries.len() + redo.len()).map(|i| st.history.row_id(i)).collect();
    let dirty = st.is_dirty();
    let name = st.doc.layer(st.active_layer.unwrap()).unwrap().name.clone();
    s.actions.list = vec![Action::new("No change", vec![("layer.renameLayer".into(), json!({"name":name}))])];
    let result = s.execute("actions.play", json!({"action":0})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    let st = s.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&st.doc, &before));
    assert_eq!(st.history.entries(), entries);
    assert_eq!(st.history.redo_labels().collect::<Vec<_>>(), redo.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!((0..ids.len()).map(|i| st.history.row_id(i)).collect::<Vec<_>>(), ids);
    assert_eq!(st.is_dirty(), dirty);
    assert!(s.redo());
}

/// Stop commits its prefix, while Allow Continue includes the following step in that same entry.
#[test]
fn stop_message_retains_the_completed_prefix() {
    for allow in [false, true] {
        let mut s = session();
        let before = s.active().unwrap().doc.clone();
        let count = s.active().unwrap().history.past_len();
        s.actions.list = vec![Action::new(
            "Stop",
            vec![
                ("layer.renameLayer".into(), json!({"name":"Prefix"})),
                ("actions.stopMessage".into(), json!({"message":"Check","allowContinue":allow})),
                ("layer.renameLayer".into(), json!({"name":"Suffix"})),
            ],
        )];
        let result = s.execute("actions.play", json!({"action":0})).unwrap();
        assert_eq!(result.get("failed").is_none(), allow, "{result}");
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, if allow { "Suffix" } else { "Prefix" });
        assert_eq!(st.history.past_len(), count + 1);
        assert_eq!(st.history.undo_label(), Some("Action: Stop"));
        let after = st.doc.clone();
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc.as_ref(), before.as_ref());
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
    }
}

/// Resumed dialog edits and subsequent steps join the published prefix, without an active scope while paused.
#[test]
fn suspended_history_resumes_the_same_action() {
    use photocraft_engine::actions_cmds;
    let mut s = session();
    let action = s.actions.list[0].clone();
    let before = s.active().unwrap().doc.clone();
    let count = s.active().unwrap().history.past_len();
    let recording = actions_cmds::begin_playback(&mut s, &action).unwrap();
    s.execute("layer.renameLayer", json!({"name":"Prefix"})).unwrap();
    let continuation = actions_cmds::suspend_history(&mut s).unwrap();
    actions_cmds::finish_playback(&mut s, recording, &action, 0);
    assert_eq!(s.active().unwrap().history.past_len(), count + 1);
    continuation.resume(&mut s).unwrap();
    s.execute("layer.renameLayer", json!({"name":"Confirmed dialog"})).unwrap();
    let recording = actions_cmds::begin_playback(&mut s, &action).unwrap();
    s.execute("layer.new.layer", json!({"name":"Suffix"})).unwrap();
    actions_cmds::finish_playback(&mut s, recording, &action, 1);
    let after = s.active().unwrap().doc.clone();
    assert_eq!(s.active().unwrap().history.past_len(), count + 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Action: Two layers"));
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.as_ref(), before.as_ref());
    assert!(s.redo());
    assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
    assert!(continuation.resume(&mut s).is_err(), "the old continuation cannot join the finalized history");
}

/// An unrelated edit, history purge or document switch during a pause cannot be merged into the action.
#[test]
fn suspended_history_refuses_document_or_history_drift() {
    use photocraft_engine::actions_cmds;
    for drift in 0..3 {
        let mut s = session();
        let action = s.actions.list[0].clone();
        let recording = actions_cmds::begin_playback(&mut s, &action).unwrap();
        s.execute("layer.renameLayer", json!({"name":"Prefix"})).unwrap();
        let continuation = actions_cmds::suspend_history(&mut s).unwrap();
        actions_cmds::finish_playback(&mut s, recording, &action, 0);
        match drift {
            0 => {
                s.execute("layer.new.layer", json!({"name":"Unrelated"})).unwrap();
            }
            1 => {
                s.active_mut().unwrap().history.purge_last();
            }
            _ => {
                s.execute("file.new", json!({"width":2,"height":2})).unwrap();
            }
        }
        let before = s.active().unwrap().doc.clone();
        let entries = s.active().unwrap().history.entries();
        assert!(continuation.resume(&mut s).is_err());
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
        assert_eq!(s.active().unwrap().history.entries(), entries);
    }
}

/// Visibility excluded from history remains excluded when it occurs between grouped edits.
#[test]
fn grouping_preserves_non_undoable_visibility() {
    let mut s = session();
    s.execute("history.options", json!({"makeLayerVisibilityChangesUndoable":false})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let layer = s.active().unwrap().active_layer.unwrap();
    s.actions.list = vec![Action::new(
        "Mixed",
        vec![
            ("layer.renameLayer".into(), json!({"name":"Prefix"})),
            ("layer.setProps".into(), json!({"visible":false})),
            ("layer.renameLayer".into(), json!({"name":"Final"})),
        ],
    )];
    let count = s.active().unwrap().history.past_len();
    let result = s.execute("actions.play", json!({"action":0})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    assert_eq!(s.active().unwrap().history.past_len(), count + 1);
    assert!(s.undo());
    let actual = s.active().unwrap().doc.layer(layer).unwrap();
    assert!(!actual.visible);
    assert_eq!(actual.name, before.layer(layer).unwrap().name);
    assert!(s.redo());
    assert!(!s.active().unwrap().doc.layer(layer).unwrap().visible);
    assert_eq!(s.active().unwrap().doc.layer(layer).unwrap().name, "Final");
}

/// Nonlinear redo states remain ordered, and trimming cannot evict the before-action boundary.
#[test]
fn grouping_preserves_nonlinear_history_and_a_one_state_limit() {
    for nonlinear in [false, true] {
        let mut s = session();
        s.execute("history.options", json!({"allowNonLinearHistory":nonlinear})).unwrap();
        s.execute("layer.new.layer", json!({"name":"Future"})).unwrap();
        let future = s.active().unwrap().history.row_id(s.active().unwrap().history.past_len()).unwrap();
        assert!(s.undo());
        if !nonlinear {
            s.active_mut().unwrap().history.max_states = 1;
        }
        let before = s.active().unwrap().doc.clone();
        s.actions.list = vec![Action::new("Many", (0..5).map(|i| ("layer.renameLayer".into(), json!({"name":format!("Name{i}")}))).collect())];
        let result = s.execute("actions.play", json!({"action":0})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
        let after = s.active().unwrap().doc.clone();
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Action: Many"));
        if !nonlinear {
            assert_eq!(s.active().unwrap().history.past_len(), 1);
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc.as_ref(), before.as_ref());
        } else {
            assert!(s.active().unwrap().history.row_of(future).is_some(), "the existing future row keeps its identity");
            assert!(s.undo());
            assert!(s.active().unwrap().doc.layers.iter().any(|layer| layer.name == "Future"));
        }
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
    }
}
