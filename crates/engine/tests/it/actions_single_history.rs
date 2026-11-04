//! Public playback history semantics, independent of the Actions panel implementation.
use photocraft_engine::{Session, actions_cmds::Action};
use serde_json::json;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4,"background":"#804020"})).unwrap();
    s.actions.list.push(Action::new("Two layers", ["First", "Second"].into_iter().map(|name| ("layer.new.layer".into(), json!({"name":name}))).collect()));
    s
}

/// Start with distinct older source pixels and a nondefault History Brush source.
fn history_control_session(single_state: bool) -> Session {
    let mut s = session();
    s.execute("actions.setHistory", json!({"singleState":single_state})).unwrap();
    s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
    s.execute("edit.fill", json!({"color":"#00ff00"})).unwrap();
    let current = s.active().unwrap().history.past_len();
    s.execute("history.setSource", json!({"state":current})).unwrap();
    s
}

/// An explicit clear stays undoable; later edits forget it and group from its boundary.
#[test]
fn recorded_clear_preserves_its_boundary_and_grouped_suffix() {
    for single_state in [false, true] {
        for prefix in [false, true] {
            for suffix in [false, true] {
                let mut s = history_control_session(single_state);
                let past = s.active().unwrap().history.past_len();
                let mut steps = Vec::new();
                if prefix {
                    steps.push(("edit.fill".into(), json!({"color":"#0000ff"})));
                }
                steps.push(("history.clear".into(), json!({})));
                if suffix {
                    steps.extend([("layer.renameLayer".into(), json!({"name":"Suffix one"})), ("layer.renameLayer".into(), json!({"name":"Suffix two"}))]);
                }
                s.actions.list = vec![Action::new("Clear", steps)];
                let result = s.execute("actions.play", json!({"action":0})).unwrap();
                assert!(result.get("failed").is_none(), "{result}");
                let after = s.active().unwrap().doc.clone();
                if suffix {
                    assert!(!s.active().unwrap().history.is_cleared());
                    assert_eq!(s.active().unwrap().history.past_len(), if single_state { 1 } else { 2 });
                    assert!(s.undo());
                    if !single_state {
                        assert!(s.undo());
                    }
                    assert!(!s.undo(), "the next edit really forgot the cleared older states");
                } else {
                    assert!(s.active().unwrap().history.is_cleared());
                    assert_eq!(s.active().unwrap().history.past_len(), 0);
                    assert!(s.undo(), "Undo Clear History restores rows, not older pixels");
                    assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &after));
                    assert_eq!(s.active().unwrap().history.past_len(), past + usize::from(prefix));
                }
            }
        }
    }
}

/// Both a surviving old row and an intermediate action row remain usable brush sources.
#[test]
fn recorded_source_change_preserves_selected_pixels_and_grouping() {
    for single_state in [false, true] {
        for prefix in [false, true] {
            for current in [false, true] {
                let mut s = history_control_session(single_state);
                let past = s.active().unwrap().history.past_len();
                let before = s.active().unwrap().doc.clone();
                let mut steps = Vec::new();
                if prefix {
                    steps.push(("edit.fill".into(), json!({"color":"#0000ff"})));
                }
                let selected = if current { past + usize::from(prefix) } else { 1 };
                steps.extend([
                    ("history.setSource".into(), json!({"state":selected})),
                    ("edit.fill".into(), json!({"color":"#00ffff"})),
                    ("layer.renameLayer".into(), json!({"name":"After source"})),
                ]);
                s.actions.list = vec![Action::new("Source", steps)];
                let result = s.execute("actions.play", json!({"action":0})).unwrap();
                assert!(result.get("failed").is_none(), "{result}");
                let added = if single_state { 1 } else { 2 + usize::from(prefix) };
                assert_eq!(s.active().unwrap().history.past_len(), past + added);
                let after = s.active().unwrap().doc.clone();
                for _ in 0..added {
                    assert!(s.undo());
                }
                assert_eq!(s.active().unwrap().doc.as_ref(), before.as_ref());
                for _ in 0..added {
                    assert!(s.redo());
                }
                assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
                s.execute("edit.fill", json!({"contents":"history"})).unwrap();
                let pixel = s.active().unwrap().doc.layers[0].surface().unwrap().rgba(0, 0);
                let expected = if !current {
                    [1.0, 0.0, 0.0, 1.0]
                } else if prefix {
                    [0.0, 0.0, 1.0, 1.0]
                } else {
                    [0.0, 1.0, 0.0, 1.0]
                };
                assert_eq!(pixel, expected, "singleState={single_state}, prefix={prefix}, current={current}");
            }
        }
    }
}

/// Purge Histories is irreversible; suffix edits keep their grouped Undo/Redo boundary.
#[test]
fn recorded_purge_histories_does_not_restore_discarded_states() {
    for single_state in [false, true] {
        for prefix in [false, true] {
            let mut s = history_control_session(single_state);
            let mut steps = Vec::new();
            if prefix {
                steps.push(("edit.fill".into(), json!({"color":"#0000ff"})));
            }
            steps.extend([
                ("edit.purge.histories".into(), json!({})),
                ("layer.renameLayer".into(), json!({"name":"Suffix one"})),
                ("layer.renameLayer".into(), json!({"name":"Suffix two"})),
            ]);
            s.actions.list = vec![Action::new("Purge", steps)];
            let result = s.execute("actions.play", json!({"action":0})).unwrap();
            assert!(result.get("failed").is_none(), "{result}");
            let after = s.active().unwrap().doc.clone();
            assert_eq!(s.active().unwrap().history.past_len(), if single_state { 1 } else { 2 });
            assert!(s.undo());
            if !single_state {
                assert!(s.undo());
            }
            assert!(!s.undo(), "Purge Histories cannot be undone");
            assert!(s.redo());
            if !single_state {
                assert!(s.redo());
            }
            assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
        }
    }
}

/// Dialog identity protection is independent of the optional history grouping.
#[test]
fn per_step_playback_also_binds_its_dialog_continuation() {
    use photocraft_engine::actions_cmds;
    let mut s = history_control_session(false);
    let action = s.actions.list[0].clone();
    let recording = actions_cmds::begin_playback(&mut s, &action).unwrap();
    s.execute("layer.renameLayer", json!({"name":"Prefix"})).unwrap();
    let continuation = actions_cmds::suspend_history(&mut s).expect("per-step playback still needs a document/history binding");
    actions_cmds::finish_playback(&mut s, recording, &action, 0);
    s.execute("layer.renameLayer", json!({"name":"Unrelated"})).unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(continuation.resume(&mut s).is_err());
    assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
}

/// Selecting another existing source snapshot invalidates a pause even when its pixels match.
#[test]
fn snapshot_source_identity_is_bound_to_the_dialog_continuation() {
    use photocraft_engine::actions_cmds;
    for single_state in [false, true] {
        let mut s = session();
        s.execute("actions.setHistory", json!({"singleState":single_state})).unwrap();
        let a = s.execute("history.newSnapshot", json!({"name":"Source A"})).unwrap()["snapshot"].clone();
        let b = s.execute("history.newSnapshot", json!({"name":"Source B"})).unwrap()["snapshot"].clone();
        s.execute("history.setSource", json!({"snapshot":a})).unwrap();
        let action = s.actions.list[0].clone();
        let recording = actions_cmds::begin_playback(&mut s, &action).unwrap();
        s.execute("layer.renameLayer", json!({"name":"Prefix"})).unwrap();
        let continuation = actions_cmds::suspend_history(&mut s).unwrap();
        actions_cmds::finish_playback(&mut s, recording, &action, 0);
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.entries();
        s.execute("history.setSource", json!({"snapshot":b})).unwrap();
        assert!(continuation.resume(&mut s).is_err(), "singleState={single_state}");
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
        assert_eq!(s.active().unwrap().history.entries(), history);
    }
}

/// Undo Clear History inside playback restores rows, rather than recording another clear baseline.
#[test]
fn in_action_undo_clear_restores_history_and_keeps_grouped_image_undo() {
    for single_state in [false, true] {
        for prefix in [false, true] {
            let mut s = history_control_session(single_state);
            let before = s.active().unwrap().doc.clone();
            let past = s.active().unwrap().history.past_len();
            let rows = s.active().unwrap().history.entries();
            let mut steps = Vec::new();
            if prefix {
                steps.push(("layer.renameLayer".into(), json!({"name":"Prefix"})));
            }
            steps.extend([
                ("history.clear".into(), json!({})),
                ("edit.undo".into(), json!({})),
                ("edit.undo".into(), json!({})),
                ("edit.redo".into(), json!({})),
            ]);
            s.actions.list = vec![Action::new("Undo Clear", steps)];
            let result = s.execute("actions.play", json!({"action":0})).unwrap();
            assert!(result.get("failed").is_none(), "{result}");
            assert!(!s.active().unwrap().history.is_cleared(), "singleState={single_state}, prefix={prefix}");
            assert_eq!(s.active().unwrap().history.past_len(), past + usize::from(prefix));
            if prefix {
                let after = s.active().unwrap().doc.clone();
                assert!(s.undo());
                assert_eq!(s.active().unwrap().doc.as_ref(), before.as_ref());
                assert!(s.redo());
                assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
            } else {
                assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
                assert_eq!(s.active().unwrap().history.entries(), rows);
            }
        }
    }
}

/// A source held by Clear History becomes active on Undo Clear, with no additional image Undo row.
#[test]
fn cleared_intermediate_source_restores_exact_pixels_after_undo_redo() {
    for single_state in [false, true] {
        for undo_clear in [false, true] {
            let mut s = history_control_session(single_state);
            let before = s.active().unwrap().doc.clone();
            let past = s.active().unwrap().history.past_len();
            s.actions.list = vec![Action::new(
                "Cleared Source",
                vec![
                    ("edit.fill".into(), json!({"color":"#0000ff"})),
                    ("history.setSource".into(), json!({"state":past+1})),
                    ("edit.fill".into(), json!({"color":"#00ffff"})),
                    ("history.clear".into(), json!({})),
                ],
            )];
            let result = s.execute("actions.play", json!({"action":0})).unwrap();
            assert!(result.get("failed").is_none(), "{result}");
            assert!(s.active().unwrap().history.is_cleared());
            assert_eq!(s.active().unwrap().history.past_len(), 0);
            if undo_clear {
                let after = s.active().unwrap().doc.clone();
                assert!(s.undo());
                assert!(!s.active().unwrap().history.is_cleared());
                assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &after));
                let added = if single_state { 1 } else { 2 };
                assert_eq!(s.active().unwrap().history.past_len(), past + added);
                for _ in 0..added {
                    assert!(s.undo());
                }
                assert_eq!(s.active().unwrap().doc.as_ref(), before.as_ref());
                for _ in 0..added {
                    assert!(s.redo());
                }
                assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
            }
            // Before Undo Clear the source is the current cyan document. Afterwards it is the
            // explicitly selected intermediate blue document, including after image Undo/Redo.
            s.execute("edit.fill", json!({"contents":"history"})).unwrap();
            let pixel = s.active().unwrap().doc.layers[0].surface().unwrap().rgba(0, 0);
            let expected = if undo_clear { [0.0, 0.0, 1.0, 1.0] } else { [0.0, 1.0, 1.0, 1.0] };
            assert_eq!(pixel, expected, "singleState={single_state}, undoClear={undo_clear}");
        }
    }
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

#[test]
fn an_open_document_batch_cannot_change_an_active_history_scope_setting() {
    for single in [false, true] {
        let mut s = session();
        s.execute("actions.setHistory", json!({"singleState": single})).unwrap();
        let past = s.active().unwrap().history.past_len();
        let result = s
            .execute(
                "file.automate.batch",
                json!({"source": "open", "action": "Setting boundary", "steps": [
                    ["layer.renameLayer", {"name": "Prefix"}], ["actions.setHistory", {"singleState": false}], ["layer.renameLayer", {"name": "Suffix"}]
                ]}),
            )
            .unwrap();
        if single {
            assert_eq!(result["errors"].as_array().unwrap().len(), 1, "{result}");
            assert!(result.to_string().contains("cannot change action history settings"), "{result}");
            assert!(s.prefs().actions.create_single_history_state);
            assert_eq!(s.active().unwrap().doc.layers[0].name, "Prefix");
            assert_eq!(s.active().unwrap().history.undo_label(), Some("Action: Setting boundary"));
            assert_eq!(s.active().unwrap().history.past_len(), past + 1);
        } else {
            assert!(result["errors"].as_array().unwrap().is_empty(), "{result}");
            assert!(!s.prefs().actions.create_single_history_state);
            assert_eq!(s.active().unwrap().doc.layers[0].name, "Suffix");
            assert_eq!(s.active().unwrap().history.past_len(), past + 2);
        }
        s.execute("actions.setHistory", json!({"singleState": !single})).unwrap();
        assert_eq!(s.prefs().actions.create_single_history_state, !single, "the batch scope ended");
    }
}

#[test]
fn a_real_action_still_refuses_history_setting_changes_with_grouping_on_or_off() {
    for single in [false, true] {
        let mut s = session();
        s.execute("actions.setHistory", json!({"singleState": single})).unwrap();
        s.actions.list = vec![Action::new("Setting", vec![("actions.setHistory".into(), json!({"singleState": !single}))])];
        let result = s.execute("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(result["failed"]["id"], "actions.setHistory");
        assert_eq!(s.prefs().actions.create_single_history_state, single);
    }
}

#[test]
fn an_accepted_clear_survives_an_action_that_returns_to_empty_captured_history() {
    for single in [false, true] {
        let mut s = session();
        s.execute("actions.setHistory", json!({"singleState": single})).unwrap();
        let original = s.active().unwrap().doc.clone();
        assert_eq!(s.active().unwrap().history.past_len(), 0);
        assert!(s.execute("history.clear", json!({})).is_err(), "ordinary empty Clear still refuses");
        s.actions.list = vec![Action::new(
            "Empty capture Clear",
            vec![
                ("edit.fill".into(), json!({"color": "#000000"})),
                ("history.setSource".into(), json!({"state": 1})),
                ("edit.undo".into(), json!({})),
                ("history.clear".into(), json!({})),
            ],
        )];
        let result = s.execute("actions.play", json!({"action": 0})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
        assert!(s.active().unwrap().history.is_cleared());
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &original));
        assert!(s.undo(), "Undo Clear remains available with an empty captured baseline");
        assert!(!s.active().unwrap().history.is_cleared());
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &original));
        s.execute("edit.fill", json!({"contents": "history"})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers[0].surface().unwrap().rgba(0, 0), [0.0, 0.0, 0.0, 1.0], "the exact black source is restored");
    }
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

/// Check both removed ids and the labels attached to every surviving older row.
fn assert_delete_rows(s: &Session, originals: &[(u64, String, std::sync::Arc<photocraft_doc::Document>)], removed: &[usize], case: &str, single_state: bool) {
    for (i, (id, label, _)) in originals.iter().enumerate() {
        let row = s.active().unwrap().history.row_of(*id);
        if removed.contains(&i) {
            assert!(row.is_none(), "{case}, singleState={single_state}: deleted row {id} returned");
        } else {
            let row = row.unwrap_or_else(|| panic!("{case}: surviving row {id} disappeared"));
            let st = s.active().unwrap();
            let labels: Vec<_> = st.history.entries().into_iter().chain(st.history.redo_labels().map(str::to_owned)).collect();
            assert_eq!(&labels[row], label);
        }
    }
}

/// Compare Undo with the unchanged oracle for this deletion and history mode.
fn assert_delete_boundary(
    s: &Session,
    case: &str,
    single_state: bool,
    prefix: bool,
    before: &std::sync::Arc<photocraft_doc::Document>,
    originals: &[(u64, String, std::sync::Arc<photocraft_doc::Document>)],
) {
    let boundary = if case == "linear-past" {
        &originals[0].2
    } else if case == "nonlinear-redo" {
        // Non-linear history appends the retained future after the original current.
        &originals[3].2
    } else if single_state {
        before
    } else if case == "nonlinear-past" && prefix {
        &originals[3].2
    } else {
        &originals[4].2
    };
    assert_eq!(s.active().unwrap().doc.as_ref(), boundary.as_ref(), "{case}, singleState={single_state}, prefix={prefix}");
}

/// Play the real Delete State command, retaining identities of the older rows it did not delete.
fn recorded_delete_case(case: &str) {
    for single_state in [false, true] {
        for prefix in [false, true] {
            if !prefix && case != "nonlinear-past" {
                continue;
            }
            let nonlinear = case.starts_with("nonlinear");
            let mut s = history_control_session(single_state);
            s.execute("history.options", json!({"allowNonLinearHistory":nonlinear})).unwrap();
            s.execute("edit.fill", json!({"color":"#0000ff"})).unwrap();
            s.execute("edit.fill", json!({"color":"#00ffff"})).unwrap();
            let originals: Vec<_> = (0..=s.active().unwrap().history.past_len())
                .map(|i| {
                    let st = s.active().unwrap();
                    (st.history.row_id(i).unwrap(), st.history.entries()[i].clone(), st.history.row_doc(i, &st.doc).unwrap())
                })
                .collect();
            let source = if case.ends_with("past") { 1 } else { 4 };
            s.execute("history.setSource", json!({"state":source})).unwrap();
            if case == "nonlinear-redo" {
                assert!(s.undo());
                assert!(s.undo());
            }
            let before = s.active().unwrap().doc.clone();
            let past = s.active().unwrap().history.past_len();
            let mut steps = Vec::new();
            if prefix {
                steps.push(("edit.fill".into(), json!({"color":"#0000ff"})));
            }
            if case.ends_with("current") {
                steps.push(("history.setSource".into(), json!({"state":past + 1})));
            }
            if case == "nonlinear-redo" {
                steps.push(("history.selectState".into(), json!({"state":3})));
            }
            let deleted = match case {
                "linear-past" | "nonlinear-past" => 1,
                "nonlinear-redo" => 4,
                _ => past + 1,
            };
            steps.extend([
                ("history.deleteState".into(), json!({"state":deleted})),
                ("edit.fill".into(), json!({"color":"#ffff00"})),
                ("edit.fill".into(), json!({"color":"#ff00ff"})),
            ]);
            s.actions.list = vec![Action::new("Delete state", steps)];
            let result = s.execute("actions.play", json!({"action":0})).unwrap();
            assert!(result.get("failed").is_none(), "{case}, singleState={single_state}: {result}");
            let after = s.active().unwrap().doc.clone();
            assert_eq!(after.layers[0].surface().unwrap().rgba(0, 0), [1.0, 0.0, 1.0, 1.0]);
            let removed: Vec<_> = match case {
                "linear-past" => (1..5).collect(),
                "nonlinear-past" => vec![1],
                "nonlinear-redo" => vec![4],
                _ => Vec::new(),
            };
            assert_delete_rows(&s, &originals, &removed, case, single_state);
            let count = if single_state { 1 } else { 2 };
            if single_state {
                assert_eq!(s.active().unwrap().history.past_len(), originals.len() - removed.len());
                assert_eq!(s.active().unwrap().history.undo_label(), Some("Action: Delete state"));
            }
            for _ in 0..count {
                assert!(s.undo());
            }
            assert_delete_boundary(&s, case, single_state, prefix, &before, &originals);
            for _ in 0..count {
                assert!(s.redo());
            }
            assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
            for i in removed {
                assert!(s.active().unwrap().history.row_of(originals[i].0).is_none());
            }
            assert!(s.execute("edit.fill", json!({"contents":"history"})).is_err(), "the selected deleted source must stay unresolved");
            assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
        }
    }
}

#[test]
fn recorded_linear_past_deletion_keeps_its_boundary_and_suffix() {
    recorded_delete_case("linear-past");
}

#[test]
fn recorded_linear_current_deletion_keeps_surviving_rows() {
    recorded_delete_case("linear-current");
}

#[test]
fn recorded_nonlinear_past_deletion_keeps_only_surviving_rows() {
    recorded_delete_case("nonlinear-past");
}

#[test]
fn recorded_nonlinear_current_deletion_keeps_surviving_rows() {
    recorded_delete_case("nonlinear-current");
}

#[test]
fn recorded_nonlinear_redo_deletion_keeps_only_surviving_rows() {
    recorded_delete_case("nonlinear-redo");
}

/// Purge Undo removes one predecessor, while native snapshots and legitimate older rows remain.
fn recorded_user_purge_undo_case(suffix: bool) {
    for single_state in [false, true] {
        for prefix in [false, true] {
            let mut s = history_control_session(single_state);
            let old: Vec<_> = (0..=s.active().unwrap().history.past_len())
                .map(|i| {
                    let st = s.active().unwrap();
                    (st.history.row_id(i).unwrap(), st.history.row_doc(i, &st.doc).unwrap())
                })
                .collect();
            let removed = if prefix { 2 } else { 1 };
            s.execute("history.setSource", json!({"state":removed})).unwrap();
            let mut steps = Vec::new();
            if prefix {
                steps.push(("edit.fill".into(), json!({"color":"#0000ff"})));
            }
            steps.extend([("edit.purge.undo".into(), json!({})), ("history.newSnapshot".into(), json!({"name":"Purge boundary"}))]);
            if suffix {
                steps.extend([("edit.fill".into(), json!({"color":"#ffff00"})), ("edit.fill".into(), json!({"color":"#ff00ff"}))]);
            }
            s.actions.list = vec![Action::new("Purge Undo", steps)];
            let result = s.execute("actions.play", json!({"action":0})).unwrap();
            assert!(result.get("failed").is_none(), "singleState={single_state}, prefix={prefix}, suffix={suffix}: {result}");
            let st = s.active().unwrap();
            let after = st.doc.clone();
            let boundary = st.snapshots.last().unwrap().doc.clone();
            let snapshot = st.snapshots.last().unwrap().id;
            assert_eq!(boundary.layers[0].surface().unwrap().rgba(0, 0), if prefix { [0.0, 0.0, 1.0, 1.0] } else { [0.0, 1.0, 0.0, 1.0] });
            assert!(st.history.row_of(old[removed].0).is_none(), "Purge Undo's removed predecessor returned");
            assert!(st.history.row_of(old[0].0).is_some());
            let count = if single_state { 1 } else { 2 };
            if suffix {
                for _ in 0..count {
                    assert!(s.undo());
                }
                assert_eq!(s.active().unwrap().doc.as_ref(), boundary.as_ref());
            }
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc.as_ref(), old[if prefix { 1 } else { 0 }].1.as_ref(), "the purged pixels cannot be undone back into existence");
            assert!(s.redo());
            assert_eq!(s.active().unwrap().doc.as_ref(), boundary.as_ref());
            if suffix {
                for _ in 0..count {
                    assert!(s.redo());
                }
            }
            assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
            assert!(s.active().unwrap().history.row_of(old[removed].0).is_none());
            assert!(s.execute("edit.fill", json!({"contents":"history"})).is_err());
            assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
            s.execute("edit.fill", json!({"contents":"history", "snapshot":snapshot})).unwrap();
            assert_eq!(
                s.active().unwrap().doc.layers[0].surface().unwrap().rgba(0, 0),
                boundary.layers[0].surface().unwrap().rgba(0, 0),
                "native snapshots outlive Purge Undo"
            );
        }
    }
}

#[test]
fn recorded_user_purge_undo_does_not_restore_its_removed_predecessor() {
    recorded_user_purge_undo_case(false);
}

#[test]
fn recorded_user_purge_undo_groups_only_the_later_suffix() {
    recorded_user_purge_undo_case(true);
}
