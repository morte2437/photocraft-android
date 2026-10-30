use super::*;
use serde_json::json;

fn pixel(s: &Session, layer: usize, x: i32, y: i32) -> Vec<f32> {
    s.active().unwrap().doc.layers[layer].surface().unwrap().pixel(x, y)
}

/// CopyCat's shape: Edit Contents, change the contents, Save. The Save step has no file to
/// write: it updates the smart object, headless too.
#[test]
fn save_in_edit_contents_updates_the_smart_object() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("layer.smartObjects.editContents", json!({})).unwrap();
    let steps = vec![("edit.fill".to_string(), json!({"contents": "color", "color": "#0000ff"})), ("file.save".to_string(), json!({}))];
    s.actions.list.push(Action::new("Edit and save", steps));
    let played = s.execute("actions.play", json!({"action": "Edit and save"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    s.set_active(0);
    let doc = &s.active().unwrap().doc;
    let flat = crate::file_cmds::flattened(doc, doc.pixel_format());
    let p = flat.pixel(2, 2);
    assert!(p[0] < 0.01 && p[2] > 0.99, "the smart object shows the saved contents: {p:?}");
    // With no Edit Contents document, Save stays the shell's (it needs the document's file).
    s.actions.list.push(Action::new("Save", vec![("file.save".to_string(), json!({}))]));
    assert!(s.execute("actions.play", json!({"action": "Save"})).unwrap().get("failed").is_some());
}

#[test]
fn deselect_with_nothing_selected_is_a_noop_in_playback() {
    // #6744: imported actions often start with Deselect; with no selection Photoshop plays on.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let before = s.active().unwrap().history.past_len();
    // Outside playback the command stays disabled, like Photoshop's greyed menu item.
    assert!(s.execute("select.deselect", json!({})).is_err());
    assert!(s.active().unwrap().doc.selection.is_none());
    assert_eq!(s.active().unwrap().history.past_len(), before, "rejection adds no history step");
    let deselect = crate::commands::find("select.deselect").unwrap();
    assert_eq!((deselect.run)(&mut s, &json!({})).unwrap(), Value::Null);
    assert_eq!(s.active().unwrap().history.past_len(), before, "no history step");

    let steps = vec![("select.deselect".to_string(), json!({})), ("layer.new.layer".to_string(), json!({"name": "A"}))];
    s.actions.list.push(Action::new("Clear", steps));
    let played = s.execute("actions.play", json!({"action": 0})).unwrap();
    assert_eq!(played["ran"], 2, "{played}");
    assert_eq!(s.active().unwrap().doc.layers.last().unwrap().name, "A");
}

#[test]
fn unchecked_steps_are_skipped_and_follow_move_and_delete() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let step = |name: &str| ("layer.new.layer".to_string(), json!({"name": name}));
    s.actions.list.push(Action::new("Three", vec![step("A"), step("B"), step("C")]));
    let names = |s: &Session| s.active().unwrap().doc.layers.iter().skip(1).map(|l| l.name.clone()).collect::<Vec<_>>();

    assert_eq!(s.execute("actions.toggle", json!({"action": "Three", "step": 1})).unwrap()["disabled"], json!([1]));
    let played = s.execute("actions.play", json!({"action": 0})).unwrap();
    assert_eq!(played["ran"], 2);
    assert_eq!(names(&s), ["A", "C"], "the unchecked step is skipped");
    assert_eq!(s.execute("actions.get", json!({"action": 0})).unwrap()["disabled"], json!([1]));

    // Moving step 3 to the top keeps B (now step 2) unchecked; deleting the moved step shifts it to 1.
    s.execute("actions.move", json!({"action": 0, "step": 2, "to": 0})).unwrap();
    let ids: Vec<_> = s.actions.list[0].steps.iter().map(|(_, p)| p["name"].as_str().unwrap()).collect();
    assert_eq!((ids, s.actions.list[0].disabled.iter().copied().collect::<Vec<_>>()), (vec!["C", "A", "B"], vec![2]));
    s.execute("actions.delete", json!({"action": 0, "step": 0})).unwrap();
    assert_eq!(s.actions.list[0].steps, [step("A"), step("B")], "deleting removes only that step");
    assert_eq!(s.actions.list[0].disabled.iter().copied().collect::<Vec<_>>(), [1]);

    // The action's own check: all on unless all already are.
    s.execute("actions.toggle", json!({"action": 0})).unwrap();
    assert!(s.actions.list[0].disabled.is_empty());
    s.execute("actions.toggle", json!({"action": 0})).unwrap();
    assert_eq!(s.actions.list[0].disabled.len(), 2);
    s.execute("actions.toggle", json!({"action": 0, "step": 0, "enabled": true})).unwrap();
    assert_eq!(s.actions.list[0].disabled.iter().copied().collect::<Vec<_>>(), [1]);

    for bad in [
        json!({"action": 0, "step": 2}),
        json!({"action": 0, "step": -1}),
        json!({"action": 0, "step": "x"}),
        json!({"action": 0, "step": 0, "enabled": "yes"}),
        json!({"action": 7, "step": 0}),
        json!({}),
    ] {
        assert!(s.execute("actions.toggle", bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(s.actions.list[0].disabled.iter().copied().collect::<Vec<_>>(), [1], "rejected toggles change nothing");
}

#[test]
fn recorded_eye_visibility_replays_on_another_document_and_duplicate_names() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Same"})).unwrap();
    let hidden = s.execute("layer.new.layer", json!({"name": "Same"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("actions.record", json!({"name": "Hide"})).unwrap();
    s.execute("layer.setProps", json!({"layer": hidden, "visible": false, "coalesce": "eye-sweep"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let keep = s.execute("layer.new.layer", json!({"name": "Same"})).unwrap()["layer"].as_u64().unwrap();
    let target = s.execute("layer.new.layer", json!({"name": "Same"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("select.deselectLayers", json!({})).unwrap();
    let result = s.execute("actions.play", json!({"action": "Hide"})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    assert!(s.active().unwrap().doc.layer(photocraft_doc::LayerId(keep)).unwrap().visible);
    assert!(!s.active().unwrap().doc.layer(photocraft_doc::LayerId(target)).unwrap().visible);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.layer(photocraft_doc::LayerId(target)).unwrap().visible);
}

#[test]
fn rename_keeps_calls_and_function_key_and_rejects_bad_names() {
    let mut s = Session::new();
    s.actions.list.push(Action {
        name: "Action 1".into(),
        steps: vec![("layer.new.layer".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    s.actions.list.push(Action {
        name: "Caller".into(),
        steps: vec![("actions.play".into(), json!({"action": "Action 1"}))],
        disabled: Default::default(),
        ..Default::default()
    });
    s.edit_prefs(|p| p.shortcuts.insert(format!("{PLAY_SHORTCUT_PREFIX}Action 1"), "F2".into()));
    assert_eq!(s.execute("actions.rename", json!({"action": 0, "name": "Sepia"})).unwrap()["action"], "Sepia");
    assert_eq!(s.execute("actions.list", json!({})).unwrap()["actions"][0]["name"], "Sepia");
    assert_eq!(s.actions.list[1].steps[0].1["action"], "Sepia");
    assert_eq!(s.prefs().shortcuts.get("actions.play:Sepia").map(String::as_str), Some("F2"));
    assert!(!s.prefs().shortcuts.contains_key("actions.play:Action 1"));
    for bad in [
        json!({"action": 0, "name": ""}),
        json!({"action": 0, "name": "  "}),
        json!({"action": 0, "name": "Caller"}),
        json!({"action": 0, "name": 7}),
        json!({"action": 9, "name": "X"}),
        json!({"action": 0}),
    ] {
        assert!(s.execute("actions.rename", bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(s.actions.list[0].name, "Sepia");
}

#[test]
fn rename_follows_conditional_branches() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    for name in ["B", "Other"] {
        s.actions.list.push(Action { name: name.into(), steps: vec![("layer.new.layer".into(), json!({}))], ..Default::default() });
    }
    s.actions.list.push(Action {
        name: "Main".into(),
        steps: vec![
            ("actions.conditional".into(), json!({"if": "always", "then": "B", "else": "Other"})),
            ("actions.conditional".into(), json!({"if": "never", "then": "Other", "else": "B"})),
        ],
        ..Default::default()
    });
    s.execute("actions.rename", json!({"action": "B", "name": "C"})).unwrap();
    let steps = &s.actions.list[2].steps;
    assert_eq!(steps[0].1, json!({"if": "always", "then": "C", "else": "Other"}));
    assert_eq!(steps[1].1, json!({"if": "never", "then": "Other", "else": "C"}));
    let r = s.execute("actions.play", json!({"action": "Main"})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
}

#[test]
fn delete_action_drops_its_function_key_but_deleting_a_step_keeps_it() {
    let mut s = Session::new();
    let key = format!("{PLAY_SHORTCUT_PREFIX}Action 1");
    s.actions.list.push(Action::new("Action 1", vec![("layer.new.layer".into(), json!({})), ("layer.new.layer".into(), json!({}))]));
    s.edit_prefs(|p| p.shortcuts.insert(key.clone(), "F2".into()));
    s.execute("actions.delete", json!({"action": 0, "step": 0})).unwrap();
    assert_eq!(s.prefs().shortcuts.get(&key).map(String::as_str), Some("F2"), "deleting a step keeps the key");
    assert_eq!(s.execute("actions.delete", json!({"action": 0})).unwrap()["deleted"], "Action 1");
    assert!(!s.prefs().shortcuts.contains_key(&key), "the deleted action's function key is gone");
    s.execute("actions.record", json!({})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].name, "Action 1");
    assert!(!s.prefs().shortcuts.contains_key(&key), "a new action with the same name starts unbound");
}

fn record_red(s: &mut Session, depth: u32) {
    s.execute("file.new", json!({"width": 40, "height": 40, "depth": depth})).unwrap();
    s.execute("actions.record", json!({"name": "Red"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let _ = s.execute("document.pixel", json!({"x": 1, "y": 1}));
    let _ = s.execute("actions.list", json!({}));
    s.execute("actions.stop", json!({})).unwrap();
}

fn translated_square(s: &mut Session, depth: u32) {
    s.execute("file.new", json!({"width": 20, "height": 8, "depth": depth, "background": "transparent"})).unwrap();
    s.edit("red square", |doc, _| {
        doc.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 2, 2, 4), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 4, 0], "interpolation": "nearest"})).unwrap();
    assert_eq!(pixel(s, 0, 4, 2), vec![1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn record_transform_again_replays_one_concrete_transform_at_each_depth() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        translated_square(&mut s, depth);
        let from = s.journal.len();
        s.execute("actions.record", json!({"name": "Again"})).unwrap();
        s.execute("edit.transform.again", json!({})).unwrap();
        let ids: Vec<&str> = s.journal.iter().skip(from).map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["edit.transform", "edit.transform.again"], "both commands stay in the journal during recording");
        let live_count = s.journal.iter().skip(from).filter(|(id, _)| replayable(id)).count();
        s.execute("actions.stop", json!({})).unwrap();
        let recorded = s.execute("actions.get", json!({"action": "Again"})).unwrap();
        let steps = recorded["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 1, "depth {depth}: record the concrete transform once");
        assert_eq!(live_count, 1, "depth {depth}: the live Actions count agrees");
        assert_eq!(steps[0][0], "edit.transform");
        assert_eq!(steps[0][1]["matrix"], json!([1, 0, 0, 1, 4, 0]));
        assert_eq!(steps[0][1]["interpolation"], "nearest");

        translated_square(&mut s, depth);
        let before = s.active().unwrap().history.past_len();
        let played = s.execute("actions.play", json!({"action": "Again"})).unwrap();
        assert!(played.get("failed").is_none(), "depth {depth}: {played}");
        assert_eq!(played["ran"], 1, "depth {depth}: apply the recorded transform once");
        assert_eq!(s.active().unwrap().history.past_len(), before + 1, "depth {depth}: one undo step");
        assert_eq!(pixel(&s, 0, 8, 2), vec![1.0, 0.0, 0.0, 1.0], "depth {depth}: moved four pixels");
        assert_eq!(pixel(&s, 0, 12, 2), vec![0.0; 4], "depth {depth}: no second transform");
        assert!(s.undo());
        assert_eq!(pixel(&s, 0, 4, 2), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(pixel(&s, 0, 8, 2), vec![0.0; 4]);
        assert!(s.redo());
        assert_eq!(pixel(&s, 0, 8, 2), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(pixel(&s, 0, 4, 2), vec![0.0; 4]);
    }
}

#[test]
fn record_transform_again_on_a_copy_replays_in_a_session_without_a_transform() {
    for depth in [8, 16] {
        let mut s = Session::new();
        translated_square(&mut s, depth);
        s.execute("actions.record", json!({"name": "Step"})).unwrap();
        s.execute("edit.transform.againCopy", json!({})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        let got = s.execute("actions.get", json!({"action": "Step"})).unwrap();
        let ids: Vec<&str> = got["steps"].as_array().unwrap().iter().map(|st| st[0].as_str().unwrap()).collect();
        assert_eq!(ids, ["layer.duplicate", "edit.transform"], "depth {depth}: the concrete steps, not the wrapper");
        let action = s.actions.list[0].clone();

        // A new session has no transform to repeat: the concrete steps still play.
        let mut fresh = Session::new();
        fresh.execute("file.new", json!({"width": 20, "height": 8, "depth": depth, "background": "transparent"})).unwrap();
        fresh
            .edit("red square", |doc, _| {
                doc.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(4, 2, 6, 4), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        fresh.actions.list.push(action);
        let played = fresh.execute("actions.play", json!({"action": "Step"})).unwrap();
        assert!(played.get("failed").is_none(), "depth {depth}: {played}");
        assert_eq!(played["ran"], 2, "depth {depth}");
        assert_eq!(fresh.active().unwrap().doc.layers.len(), 2, "depth {depth}: one copy");
        assert_eq!(pixel(&fresh, 0, 4, 2), vec![1.0, 0.0, 0.0, 1.0], "depth {depth}: the original stays");
        assert_eq!(pixel(&fresh, 1, 8, 2), vec![1.0, 0.0, 0.0, 1.0], "depth {depth}: the copy moved four pixels");
        assert_eq!(pixel(&fresh, 1, 4, 2), vec![0.0; 4], "depth {depth}: moved once");
    }
}

#[test]
fn record_drops_the_steps_of_a_command_that_failed_and_was_taken_back() {
    // Transform Again on a Copy of a position-locked layer duplicates it, fails to move the
    // copy and takes the copy back: nothing is left to record.
    let mut s = Session::new();
    translated_square(&mut s, 8);
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.setProps", json!({"layer": id.0, "locks": {"position": true}})).unwrap();
    s.execute("actions.record", json!({"name": "Locked"})).unwrap();
    assert!(s.execute("edit.transform.againCopy", json!({})).is_err());
    assert_eq!(s.execute("actions.stop", json!({})).unwrap()["steps"], 0, "no Duplicate step for a copy that was taken back");

    // Straighten rolls back when its crop is refused after the rotation ran.
    ruled_background(&mut s, 8);
    s.execute("image.analysis.rulerTool", json!({"start": [10, 30], "end": [70, 25]})).unwrap();
    s.authorize = Some(|id, _| if id == "image.crop" { Err(EngineError::Other("refused".into())) } else { Ok(()) });
    s.execute("actions.record", json!({"name": "Refused"})).unwrap();
    assert!(s.execute("image.analysis.straightenLayer", json!({})).is_err());
    assert_eq!(s.active().unwrap().doc.size.width, 80, "the rotation was taken back");
    assert_eq!(s.execute("actions.stop", json!({})).unwrap()["steps"], 0, "no rotation step for a rotation that was taken back");
}

#[test]
fn record_script_that_plays_an_action_keeps_each_step_once() {
    let mut s = Session::new();
    translated_square(&mut s, 8);
    s.execute("actions.record", json!({"name": "Inner"})).unwrap();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 1, 0], "interpolation": "nearest"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();

    // Browse records the concrete steps it ran, not itself (#3057): the script's transform and
    // the action call, each once, in order.
    let script = json!({"steps": [["edit.transform", {"matrix": [1, 0, 0, 1, 2, 0], "interpolation": "nearest"}], ["actions.play", {"action": "Inner"}]]});
    s.execute("actions.record", json!({"name": "Outer"})).unwrap();
    s.execute("file.scripts.browse", script.clone()).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let got = s.execute("actions.get", json!({"action": "Outer"})).unwrap();
    assert_eq!(got["steps"], script["steps"]);
}

#[test]
fn record_explicit_transform_keeps_one_replayable_step() {
    let mut s = Session::new();
    translated_square(&mut s, 8);
    s.execute("actions.record", json!({"name": "Move"})).unwrap();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 4, 0], "interpolation": "nearest"})).unwrap();
    assert_eq!(s.execute("actions.stop", json!({})).unwrap()["steps"], 1);
    let recorded = s.execute("actions.get", json!({"action": "Move"})).unwrap();
    assert_eq!(recorded["steps"][0][0], "edit.transform");

    translated_square(&mut s, 8);
    let before = s.active().unwrap().history.past_len();
    let played = s.execute("actions.play", json!({"action": "Move"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(played["ran"], 1);
    assert_eq!(s.active().unwrap().history.past_len(), before + 1);
    assert_eq!(pixel(&s, 0, 8, 2), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(pixel(&s, 0, 12, 2), vec![0.0; 4]);
}

#[test]
fn recorded_single_layer_transform_replays_after_layer_ids_shift() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 8, "background": "transparent"})).unwrap();
    let source = s.execute("layer.new.layer", json!({"name": "Target"})).unwrap()["layer"].as_u64().unwrap();
    s.edit("source pixel", |doc, _| {
        doc.layer_mut(photocraft_doc::LayerId(source)).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 2, 2, 4), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();

    s.execute("actions.record", json!({"name": "Move target"})).unwrap();
    s.execute("edit.transform", json!({"layer": source, "dx": 4, "interpolation": "nearest"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let recorded = s.execute("actions.get", json!({"action": "Move target"})).unwrap();
    assert_eq!(recorded["steps"][0][1], json!({"dx": 4, "interpolation": "nearest"}));

    // Add a layer before the target, shifting its id; playback must resolve the current
    // selection rather than keep the source document's numeric layer id.
    s.execute("file.new", json!({"width": 20, "height": 8, "background": "transparent"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Inserted"})).unwrap();
    let target = s.execute("layer.new.layer", json!({"name": "Target"})).unwrap()["layer"].as_u64().unwrap();
    assert_ne!(source, target);
    s.edit("target pixel", |doc, _| {
        doc.layer_mut(photocraft_doc::LayerId(target)).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 2, 2, 4), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let target_index = s.active().unwrap().doc.layers.iter().position(|layer| layer.id.0 == target).unwrap();
    let before = s.active().unwrap().history.past_len();
    let played = s.execute("actions.play", json!({"action": "Move target"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(s.active().unwrap().history.past_len(), before + 1);
    assert_eq!(pixel(&s, target_index, 4, 2), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(pixel(&s, target_index, 0, 2), vec![0.0; 4]);
}

/// A Background with a red bar and a slanted ruler line, ready for Straighten.
fn ruled_background(s: &mut Session, depth: u32) {
    s.execute("file.new", json!({"width": 80, "height": 60, "depth": depth})).unwrap();
    s.edit("red bar", |doc, _| {
        doc.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(10, 20, 60, 8), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
}

#[test]
fn record_command_that_runs_commands_replays_it_once() {
    // Straighten Layer runs Image Rotation and Crop itself: only its own call is recorded,
    // so playback rotates once, not three times (rotate, crop, then straighten again).
    for depth in [8, 16] {
        let mut s = Session::new();
        ruled_background(&mut s, depth);
        s.execute("actions.record", json!({"name": "Level"})).unwrap();
        s.execute("image.analysis.rulerTool", json!({"start": [10, 30], "end": [70, 25]})).unwrap();
        s.execute("image.analysis.straightenLayer", json!({})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        let got = s.execute("actions.get", json!({"action": "Level"})).unwrap();
        let ids: Vec<&str> = got["steps"].as_array().unwrap().iter().map(|st| st[0].as_str().unwrap()).collect();
        assert_eq!(ids, ["image.analysis.rulerTool", "image.analysis.straightenLayer"], "depth {depth}");
        let direct = s.active().unwrap().doc.clone();

        ruled_background(&mut s, depth);
        let before = s.active().unwrap().history.past_len();
        let played = s.execute("actions.play", json!({"action": "Level"})).unwrap();
        assert!(played.get("failed").is_none(), "depth {depth}: {played}");
        assert_eq!(played["ran"], 2, "depth {depth}");
        assert_eq!(s.active().unwrap().history.past_len(), before + 1, "depth {depth}: one Straighten step (the ruler is no step)");
        let replayed = s.active().unwrap().doc.clone();
        assert_eq!(replayed.size, direct.size, "depth {depth}: rotated and cropped once");
        let r = photocraft_geom::Rect::new(0, 0, direct.size.width as i32, direct.size.height as i32);
        let px = |d: &photocraft_doc::Document| d.layers[0].surface().unwrap().read_region(r);
        assert_eq!(px(&replayed), px(&direct), "depth {depth}: same pixels as the direct run");
    }
}

#[test]
fn record_keeps_only_replayable_steps() {
    let mut s = Session::new();
    record_red(&mut s, 8);
    let got = s.execute("actions.get", json!({"action": "Red"})).unwrap();
    let ids: Vec<&str> = got["steps"].as_array().unwrap().iter().map(|st| st[0].as_str().unwrap()).collect();
    assert_eq!(ids, ["layer.new.layer", "select.rect", "edit.fill"]);
    assert!(s.journal.iter().all(|(id, _)| !id.starts_with("actions.")));
    let listed = s.execute("actions.list", json!({})).unwrap();
    assert_eq!(listed["actions"][0]["steps"], 3);
    assert!(listed["recording"].is_null());
}

#[test]
fn play_on_another_document_matches_pixels_at_each_depth() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        record_red(&mut s, depth);
        s.execute("file.new", json!({"width": 40, "height": 40, "depth": depth})).unwrap();
        let before = s.active().unwrap().history.past_len();
        let r = s.execute("actions.play", json!({"action": 0})).unwrap();
        assert!(r.get("failed").is_none(), "depth {depth}: {r}");
        assert_eq!(r["ran"], 3, "depth {depth}");
        assert_eq!(s.active().unwrap().doc.layers.len(), 2, "depth {depth}");
        assert_eq!(s.active().unwrap().history.past_len(), before + 3, "depth {depth}: one history step per played step");
        let px = pixel(&s, 1, 5, 5);
        assert!(px.iter().zip([1.0, 0.0, 0.0, 1.0]).all(|(a, b)| (a - b).abs() < 1.0e-5), "depth {depth}: {px:?}");
        assert_eq!(s.actions.playing, 0);
    }
}

#[test]
fn recorded_layer_selection_replays_by_name_on_another_document() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth})).unwrap();
        let original = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.setProps", json!({"name": "Photo"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "LogoBase"})).unwrap();
        s.execute("actions.record", json!({"name": "Duplicate photo"})).unwrap();
        s.execute("layer.select", json!({"layer": original.0, "mode": "replace"})).unwrap();
        // The target name must be captured now, not when recording stops.
        s.execute("layer.setProps", json!({"name": "Retouched"})).unwrap();
        s.execute("layer.duplicate", json!({})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        assert_eq!(s.actions.list[0].steps[0].1, json!({"name":"Photo", "mode":"replace"}));
        // Persisted actions must work in an entirely different session too.
        let saved = serde_json::to_string(&s.actions.list).unwrap();
        let mut replay = Session::new();
        replay.actions.list = serde_json::from_str(&saved).unwrap();
        replay.execute("file.new", json!({"width": 8, "height": 8, "depth": depth})).unwrap();
        replay.execute("layer.setProps", json!({"name": "Photo"})).unwrap();
        let target = replay.active().unwrap().active_layer.unwrap();
        assert_ne!(original, target);
        replay.execute("layer.new.layer", json!({"name": "LogoBase"})).unwrap();
        let result = replay.execute("actions.play", json!({"action": 0})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
        assert_eq!(result["ran"], 3);
        assert_eq!(replay.active().unwrap().doc.layer(target).unwrap().name, "Retouched");
        let active = replay.active().unwrap().active_layer.unwrap();
        assert_eq!(replay.active().unwrap().doc.layer(active).unwrap().name, "Retouched copy");
    }
}

/// #7845: dragging a layer in the Layers panel while recording keeps the drop target by name,
/// so the action plays on another document (whose layer ids differ) instead of failing with
/// "no such layer". The reporter's steps, replayed on a new document.
#[test]
fn recorded_layer_drag_replays_on_another_document() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
        s.execute("actions.record", json!({"name": "Retouch"})).unwrap();
        s.execute("layer.stampVisible", json!({})).unwrap();
        let layer1 = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.new.layerViaCopy", json!({})).unwrap();
        let layer2 = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.select", json!({"layer": layer1.0, "mode": "replace"})).unwrap();
        s.execute("filter.noise.median", json!({"radius": 6})).unwrap();
        s.execute("layer.new.layerViaCopy", json!({})).unwrap();
        let copy = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.moveTo", json!({"layer": copy.0, "target": layer2.0, "position": "above"})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        let (id, step) = s.actions.list[0].steps.last().unwrap();
        assert_eq!((id.as_str(), step), ("layer.moveTo", &json!({"target": "Layer 1 copy", "position": "above"})));
        let names = |s: &Session| s.active().unwrap().doc.layers.iter().rev().map(|l| l.name.clone()).collect::<Vec<_>>();
        let recorded = names(&s);
        let saved = serde_json::to_string(&s.actions.list).unwrap();
        let mut replay = Session::new();
        replay.actions.list = serde_json::from_str(&saved).unwrap();
        replay.execute("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
        let result = replay.execute("actions.play", json!({"action": 0})).unwrap();
        assert!(result.get("failed").is_none(), "{depth}: {result}");
        assert_eq!(names(&replay), recorded, "{depth}: the same stack");
    }
    // Several layers of the drop target's name: the recorded occurrence picks the right one.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
    let upper = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name": "Mover"})).unwrap();
    let mover = s.active().unwrap().active_layer.unwrap();
    s.execute("actions.record", json!({"name": "Drop"})).unwrap();
    s.execute("layer.moveTo", json!({"layer": mover.0, "target": upper.0, "position": "below"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].steps[0].1, json!({"target": "Twin", "targetOccurrence": 1, "position": "below"}));
    // The moved layer shares the target's name: the occurrence counts the other "Twin"s only,
    // so it means the same layer before the move (on replay) as after it (when recorded).
    let build = |s: &mut Session| {
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        for _ in 0..3 {
            s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
        }
        s.active().unwrap().doc.layers.iter().map(|l| l.id).collect::<Vec<_>>()
    };
    let mut s = Session::new();
    let ids = build(&mut s);
    let (lowest, top) = (ids[1], ids[3]);
    s.execute("actions.record", json!({"name": "Twins"})).unwrap();
    s.execute("layer.moveTo", json!({"layer": top.0, "target": lowest.0, "position": "below"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].steps[0].1, json!({"target": "Twin", "targetOccurrence": 0, "position": "below"}));
    let order = |s: &Session| s.active().unwrap().doc.layers.iter().map(|l| l.id).collect::<Vec<_>>();
    let want: Vec<_> = order(&s).iter().map(|id| ids.iter().position(|x| x == id).unwrap()).collect();
    let saved = serde_json::to_string(&s.actions.list).unwrap();
    let mut replay = Session::new();
    replay.actions.list = serde_json::from_str(&saved).unwrap();
    let ids2 = build(&mut replay);
    let result = replay.execute("actions.play", json!({"action": 0})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    let got: Vec<_> = order(&replay).iter().map(|id| ids2.iter().position(|x| x == id).unwrap()).collect();
    assert_eq!(got, want, "the same layer became the lowest");
    // A moved group holding a same-named layer: its contents don't count either, or moving the
    // group past another "Twin" would shift the occurrence between recording and replay.
    let build_group = |s: &mut Session| {
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
        s.execute("layer.groupLayers", json!({})).unwrap();
        let group = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
        let top = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.select", json!({"layer": group.0, "mode": "replace"})).unwrap();
        (group, top)
    };
    let mut g = Session::new();
    let (group, top) = build_group(&mut g);
    g.execute("actions.record", json!({"name": "Group"})).unwrap();
    g.execute("layer.moveTo", json!({"layer": group.0, "target": top.0, "position": "above"})).unwrap();
    g.execute("actions.stop", json!({})).unwrap();
    let recorded = g.actions.list[0].steps[0].1.clone();
    let saved = serde_json::to_string(&g.actions.list).unwrap();
    let mut replay = Session::new();
    replay.actions.list = serde_json::from_str(&saved).unwrap();
    let (group2, _) = build_group(&mut replay);
    let result = replay.execute("actions.play", json!({"action": 0})).unwrap();
    assert!(result.get("failed").is_none(), "{recorded}: {result}");
    let top_layer = |s: &Session| s.active().unwrap().doc.layers.last().map(|l| l.id);
    assert_eq!(top_layer(&replay), Some(group2), "{recorded}: the group went to the top");
    // Several moved layers named like the target are left out too, so the name is unambiguous.
    let mut m = Session::new();
    m.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let a = photocraft_doc::LayerId(m.execute("layer.new.layer", json!({"name": "Twin"})).unwrap()["layer"].as_u64().unwrap());
    let b = photocraft_doc::LayerId(m.execute("layer.new.layer", json!({"name": "Twin"})).unwrap()["layer"].as_u64().unwrap());
    let target = photocraft_doc::LayerId(m.execute("layer.new.layer", json!({"name": "Twin"})).unwrap()["layer"].as_u64().unwrap());
    m.execute("layer.moveTo", json!({"layers": [a.0, b.0], "target": "Twin", "position": "above"})).unwrap();
    assert_eq!(m.active().unwrap().doc.layers.last().map(|l| l.id), Some(b), "both moved above the third Twin");
    assert_eq!(m.active().unwrap().doc.layers.iter().rev().nth(2).map(|l| l.id), Some(target));
    // An ⌥-drag copy of the selected layer beside itself replays by name too.
    let mut c = Session::new();
    c.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let src = photocraft_doc::LayerId(c.execute("layer.new.layer", json!({"name": "Solo"})).unwrap()["layer"].as_u64().unwrap());
    c.execute("actions.record", json!({"name": "Copy"})).unwrap();
    c.execute("layer.moveTo", json!({"layer": src.0, "target": src.0, "position": "above", "copy": true})).unwrap();
    c.execute("actions.stop", json!({})).unwrap();
    assert_eq!(c.actions.list[0].steps[0].1, json!({"target": "Solo", "position": "above", "copy": true}));
    let saved = serde_json::to_string(&c.actions.list).unwrap();
    let mut replay = Session::new();
    replay.actions.list = serde_json::from_str(&saved).unwrap();
    replay.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    replay.execute("layer.new.layer", json!({"name": "Solo"})).unwrap();
    let before = replay.active().unwrap().doc.layers.len();
    let result = replay.execute("actions.play", json!({"action": 0})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    assert_eq!(replay.active().unwrap().doc.layers.len(), before + 1, "one copy");
    // Copying a group holding a layer named like the target: the new copy's contents (made
    // before the step is recorded) don't count, so the occurrence matches the replay.
    let build_copy = |s: &mut Session| {
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap();
        s.execute("layer.groupLayers", json!({})).unwrap();
        let group = s.active().unwrap().active_layer.unwrap();
        let a = photocraft_doc::LayerId(s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap()["layer"].as_u64().unwrap());
        let b = photocraft_doc::LayerId(s.execute("layer.new.layer", json!({"name": "Twin"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("layer.select", json!({"layer": group.0, "mode": "replace"})).unwrap();
        (group, a, b)
    };
    let mut gc = Session::new();
    let (group, a, _) = build_copy(&mut gc);
    gc.execute("actions.record", json!({"name": "Copy group"})).unwrap();
    // Below `a`: the copy's "Twin" then comes before `a` in document order.
    gc.execute("layer.moveTo", json!({"layer": group.0, "target": a.0, "position": "below", "copy": true})).unwrap();
    gc.execute("actions.stop", json!({})).unwrap();
    let saved = serde_json::to_string(&gc.actions.list).unwrap();
    let mut replay = Session::new();
    replay.actions.list = serde_json::from_str(&saved).unwrap();
    let (_, a2, _) = build_copy(&mut replay);
    let result = replay.execute("actions.play", json!({"action": 0})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    let layers = &replay.active().unwrap().doc.layers;
    let at = layers.iter().position(|l| l.id == a2).unwrap();
    let below = at.checked_sub(1).and_then(|i| layers.get(i)).map(|l| l.name.as_str());
    assert_eq!(below, Some("Group 1 copy"), "the copy landed below the same Twin");
    // A drag of several layers keeps its ids (not covered by name).
    s.execute("actions.record", json!({"name": "Several"})).unwrap();
    s.execute("layer.moveTo", json!({"layers": [ids[2].0, ids[3].0], "target": ids[1].0, "position": "above"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[1].steps[0].1["target"], json!(ids[1].0));
    // Bad targets are errors, not panics.
    for bad in [
        json!({"target": "Nobody"}),
        json!({"target": "Twin"}),
        json!({"target": "Twin", "targetOccurrence": 9}),
        json!({"target": "Twin", "targetOccurrence": -1}),
        json!({"target": true}),
    ] {
        assert!(s.execute("layer.moveTo", bad.clone()).is_err(), "{bad}");
    }
}

/// An Alt-drag copy records `inPlace` (#7200), which only matters for an artboard: a plain
/// layer's recorded copy must still export to .atn, while an artboard's must be refused rather
/// than exported as a copy beside the original.
#[test]
fn recorded_in_place_duplicate_keeps_in_place_only_for_artboards() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("actions.record", json!({"name": "Layer copy"})).unwrap();
    s.execute("layer.duplicate", json!({"inPlace": true})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    s.execute("layer.new.artboard", json!({"name": "Board", "rect": [0, 0, 8, 8]})).unwrap();
    s.execute("actions.record", json!({"name": "Board copy"})).unwrap();
    s.execute("layer.duplicate", json!({"inPlace": true})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].steps, vec![("layer.duplicate".to_string(), json!({}))]);
    assert_eq!(s.actions.list[1].steps, vec![("layer.duplicate".to_string(), json!({"inPlace": true}))]);
    assert!(crate::atn_cmds::export("Audit", &[&s.actions.list[0]], &[], false).is_ok());
    assert!(crate::atn_cmds::export("Audit", &[&s.actions.list[1]], &[], false).is_err());
    // Several selected layers are copied together: one artboard among them keeps `inPlace`.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let photo = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name": "Logo"})).unwrap();
    s.execute("layer.select", json!({"layer": photo.0, "mode": "add"})).unwrap();
    s.execute("actions.record", json!({"name": "Layers copy"})).unwrap();
    s.execute("layer.duplicate", json!({"inPlace": true})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.active().unwrap().selected_layers().len(), 2, "both layers were copied");
    s.execute("layer.new.artboard", json!({"name": "Board", "rect": [0, 0, 8, 8]})).unwrap();
    s.execute("layer.select", json!({"layer": photo.0, "mode": "add"})).unwrap();
    s.execute("actions.record", json!({"name": "Mixed copy"})).unwrap();
    s.execute("layer.duplicate", json!({"inPlace": true})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].steps, vec![("layer.duplicate".to_string(), json!({}))]);
    assert_eq!(s.actions.list[1].steps, vec![("layer.duplicate".to_string(), json!({"inPlace": true}))]);
}

#[test]
fn named_selection_preserves_modes_and_duplicate_occurrences() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4})).unwrap();
    s.execute("layer.setProps", json!({"name":"Photo"})).unwrap();
    let first = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.group", json!({"name":"Group"})).unwrap();
    // A duplicate nested in a group must be found even when that group is collapsed.
    let second = s
        .edit("child", |doc, active| {
            let child = photocraft_doc::Layer::raster("Photo", doc.pixel_format());
            let id = child.id;
            if let photocraft_doc::LayerContent::Group(group) = &mut doc.layer_mut(active.unwrap()).unwrap().content {
                group.children.push(child);
                group.expanded = false;
            }
            Ok(id)
        })
        .unwrap();
    s.execute("actions.record", json!({"name":"Select duplicates"})).unwrap();
    s.execute("layer.select", json!({"layer":first.0})).unwrap();
    s.execute("layer.select", json!({"layer":second.0,"mode":"add"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].steps[0].1, json!({"name":"Photo","occurrence":0}));
    assert_eq!(s.actions.list[0].steps[1].1, json!({"name":"Photo","occurrence":1,"mode":"add"}));
    s.execute("layer.select", json!({"layer":first.0})).unwrap();
    let r = s.execute("actions.play", json!({"action":0})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    assert_eq!(s.active().unwrap().selected_layers(), vec![first, second]);
    s.execute("layer.select", json!({"name":"Photo","occurrence":1,"mode":"toggle"})).unwrap();
    assert_eq!(s.active().unwrap().selected_layers(), vec![first]);
    s.execute("layer.select", json!({"name":"Photo","occurrence":1,"mode":"range"})).unwrap();
    assert!(s.active().unwrap().selected_layers().contains(&second));
    // Main's imported .atn steps use a string-valued `layer` and choose the topmost match.
    s.execute("layer.select", json!({"layer":"Photo"})).unwrap();
    assert_eq!(s.active().unwrap().selected_layers(), vec![second]);
    s.execute("layer.select", json!({"name":"Photo","occurrence":0,"mode":"add"})).unwrap();
    s.execute("layer.select", json!({"name":"Photo","occurrence":1,"mode":"remove"})).unwrap();
    assert_eq!(s.active().unwrap().selected_layers(), vec![first]);
    // Removing an unselected layer does not add it; removing the last target keeps it.
    s.execute("layer.select", json!({"layer":"Photo","mode":"remove"})).unwrap();
    s.execute("layer.select", json!({"name":"Photo","occurrence":0,"mode":"remove"})).unwrap();
    assert_eq!(s.active().unwrap().selected_layers(), vec![first]);
    for p in [
        json!({"name":"absent"}),
        json!({"name":"Photo"}),
        json!({"name":1}),
        json!({"name":"Photo","occurrence":-1}),
        json!({"name":"Photo","occurrence":1.5}),
        json!({"name":"Photo","occurrence":99}),
        json!({"name":"Photo","layer":first.0}),
        json!({"layer":first.0,"occurrence":0}),
    ] {
        let before = s.active().unwrap().selected_layers();
        assert!(s.execute("layer.select", p.clone()).is_err(), "{p}");
        assert_eq!(s.active().unwrap().selected_layers(), before);
    }
    s.actions.list.push(Action {
        name: "Missing".into(),
        steps: vec![("layer.select".into(), json!({"name":"absent"})), ("layer.duplicate".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    let before = s.active().unwrap().doc.walk().len();
    let result = s.execute("actions.play", json!({"action":"Missing"})).unwrap();
    assert_eq!(result["ran"], 0);
    assert!(result["failed"]["error"].as_str().unwrap().contains("absent"));
    assert_eq!(s.active().unwrap().doc.walk().len(), before);
}

#[test]
fn play_stops_at_the_failing_step_and_reports_it() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("actions.record", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "One"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    s.actions.list[0].steps.push(("not.a.command".into(), json!({})));
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let r = s.execute("actions.play", json!({"action": "Action 1"})).unwrap();
    assert_eq!(r["ran"], 1);
    assert_eq!(r["failed"]["step"], 1);
    assert_eq!(r["failed"]["id"], "not.a.command");
    assert!(r["failed"]["error"].as_str().unwrap().contains("unknown command"));
    assert_eq!(s.active().unwrap().doc.layers.len(), 2, "the step before the failure is kept");
    assert_eq!(s.actions.playing, 0);
    // A later play still runs.
    assert_eq!(s.execute("actions.play", json!({"action": 0, "from": 1})).unwrap()["ran"], 0);
}

#[test]
fn play_refuses_recursion() {
    let mut s = Session::new();
    s.actions.list.push(Action {
        name: "Loop".into(),
        steps: vec![("actions.play".into(), json!({"action": "Loop"}))],
        disabled: Default::default(),
        ..Default::default()
    });
    let r = s.execute("actions.play", json!({"action": "Loop"})).unwrap();
    assert_eq!(r["ran"], 0);
    assert_eq!(r["failed"]["id"], "actions.play");
    assert!(r["failed"]["error"].as_str().unwrap().contains("recursive action"), "{r}");
    assert_eq!(s.actions.playing, 0);
}

#[test]
fn play_checks_each_step_with_the_authorize_hook() {
    fn deny_file(id: &str, _: &Value) -> Result<()> {
        if id.starts_with("file.") { Err(EngineError::Other(format!("automation command `{id}` is disabled"))) } else { Ok(()) }
    }
    let mut s = Session::new();
    s.authorize = Some(deny_file);
    s.actions.list.push(Action {
        name: "Open".into(),
        steps: vec![("file.open".into(), json!({"path": "/etc/passwd"})), ("layer.new.layer".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    let r = s.execute("actions.play", json!({"action": "Open"})).unwrap();
    assert_eq!(r["ran"], 0);
    assert_eq!(r["failed"]["step"], 0);
    assert_eq!(r["failed"]["id"], "file.open");
    assert!(r["failed"]["error"].as_str().unwrap().contains("disabled"), "{r}");
    assert!(s.documents().is_empty(), "the refused step does not open a document");
}

#[test]
fn append_delete_and_from_step() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    let started = s.execute("actions.record", json!({"name": "Grow"})).unwrap();
    assert_eq!(started["index"], 0);
    s.execute("layer.new.layer", json!({"name": "One"})).unwrap();
    assert!(s.execute("actions.record", json!({})).is_err(), "already recording");
    assert!(s.execute("actions.delete", json!({"action": 0})).is_err(), "delete while recording");
    let stopped = s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(stopped["steps"], 1);
    s.execute("actions.record", json!({"action": "Grow"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Two"})).unwrap();
    assert_eq!(s.execute("actions.stop", json!({})).unwrap()["steps"], 2);
    assert!(s.execute("actions.stop", json!({})).is_err(), "not recording");
    let before = s.active().unwrap().doc.layers.len();
    let r = s.execute("actions.play", json!({"action": "Grow", "from": 1})).unwrap();
    assert_eq!(r["ran"], 1);
    assert_eq!(s.active().unwrap().doc.layers.len(), before + 1);
    assert_eq!(s.execute("actions.delete", json!({"action": "Grow"})).unwrap()["deleted"], "Grow");
    assert!(s.actions.list.is_empty());
    assert!(s.execute("actions.get", json!({"action": "Grow"})).is_err());
}

#[test]
fn delete_one_step_while_recording_keeps_journal_and_remaining_steps() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("actions.record", json!({"name": "Edit"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Kept"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Removed"})).unwrap();
    let journal = s.journal.clone();
    let rev = s.actions.rev;
    let before = s.actions.clone();
    for step in [json!(-1), json!(2), json!(0.5), json!(null), json!("0"), json!(u64::MAX)] {
        assert!(s.execute("actions.delete", json!({"action": 0, "step": step})).is_err());
        assert_eq!(s.actions, before, "invalid deletion is atomic");
    }
    assert_eq!(pending_steps(&s, 0).count(), 2);
    let result = s.execute("actions.delete", json!({"action": 0, "step": 1})).unwrap();
    assert_eq!(result["steps"], 1);
    assert_eq!(s.journal, journal, "deleting a recording does not erase executed commands");
    assert_eq!(s.active().unwrap().doc.layers.len(), 3, "it does not undo document edits");
    assert!(s.actions.recording.is_some());
    assert!(s.actions.rev > rev);
    s.execute("layer.new.layer", json!({"name": "After"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].steps.iter().map(|(_, p)| p["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Kept", "After"]);
    s.execute("actions.delete", json!({"action": "Edit", "step": 0})).unwrap();
    assert_eq!(s.actions.list[0].steps[0].1["name"], "After");
    let saved = serde_json::to_string(&s.actions.list).unwrap();
    s.actions.list = serde_json::from_str(&saved).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert_eq!(s.execute("actions.play", json!({"action": 0})).unwrap()["ran"], 1);
    assert_eq!(s.active().unwrap().doc.layers[1].name, "After");
    s.execute("actions.delete", json!({"action": 0, "step": 0})).unwrap();
    assert_eq!(s.actions.list.len(), 1, "an empty action still exists");
    assert!(s.actions.list[0].steps.is_empty());
}

#[test]
fn nested_call_records_one_named_step_and_tracks_child_edits() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    s.actions.list.push(Action {
        name: "Child".into(),
        steps: vec![("layer.new.layer".into(), json!({"name": "Child layer"}))],
        disabled: Default::default(),
        ..Default::default()
    });
    s.execute("actions.record", json!({"name": "Parent"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Before"})).unwrap();
    s.execute("actions.play", json!({"action": 0})).unwrap();
    assert_eq!(pending_steps(&s, 1).cloned().collect::<Vec<_>>(), [("actions.play".into(), json!({"action": "Child"}))]);
    s.execute("layer.new.layer", json!({"name": "After"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[1].steps.iter().map(|st| st.0.as_str()).collect::<Vec<_>>(), ["layer.new.layer", "actions.play", "layer.new.layer"]);
    s.actions.list[0].steps.push(("layer.new.layer".into(), json!({"name": "Added later"})));
    s.execute("actions.move", json!({"action": "Child", "to": 1})).unwrap();
    let saved = serde_json::to_string(&s.actions.list).unwrap();
    s.actions.list = serde_json::from_str(&saved).unwrap();
    s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    let result = s.execute("actions.play", json!({"action": "Parent"})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    assert_eq!(s.active().unwrap().doc.layers.iter().skip(1).map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Before", "Child layer", "Added later", "After"]);
    assert!(s.actions.playback_stack.is_empty());
}

#[test]
fn nested_failure_cycles_depth_and_authorization_stop_parent() {
    let mut s = Session::new();
    s.actions.list.push(Action {
        name: "A".into(),
        steps: vec![("actions.play".into(), json!({"action": "B"})), ("file.new".into(), json!({"width": 2,"height": 2}))],
        disabled: Default::default(),
        ..Default::default()
    });
    s.actions.list.push(Action {
        name: "B".into(),
        steps: vec![("actions.play".into(), json!({"action": "A"}))],
        disabled: Default::default(),
        ..Default::default()
    });
    let r = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert!(r["failed"]["error"].as_str().unwrap().contains("recursive"));
    assert!(s.documents().is_empty());
    s.actions.list[1].steps = vec![("file.new".into(), json!({"width": 2,"height": 2}))];
    s.authorize = Some(|id, _| if id == "file.new" { Err(EngineError::Other("denied nested file".into())) } else { Ok(()) });
    let r = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert!(r["failed"]["error"].as_str().unwrap().contains("denied nested file"));
    assert!(s.documents().is_empty());
    s.authorize = None;
    s.actions.list.clear();
    for i in 0..17 {
        s.actions.list.push(Action {
            name: format!("N{i}"),
            steps: if i < 16 { vec![("actions.play".into(), json!({"action":format!("N{}", i + 1)}))] } else { vec![] },
            disabled: Default::default(),
            ..Default::default()
        });
    }
    let r = s.execute("actions.play", json!({"action": "N0"})).unwrap();
    assert!(r["failed"]["error"].as_str().unwrap().contains("16 levels"));
    assert_eq!(s.actions.playing, 0);
    assert!(s.actions.playback_stack.is_empty());
    s.actions.list[15].steps.clear();
    assert!(s.execute("actions.play", json!({"action": "N0"})).unwrap().get("failed").is_none());
}

#[test]
fn move_pending_steps_is_atomic_persists_and_changes_playback_order() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    s.execute("actions.record", json!({"name": "Order"})).unwrap();
    for name in ["One", "Two", "Three"] {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
    }
    let before = s.actions.clone();
    for params in [
        json!({"action":0,"step":0,"to":3}),
        json!({"action":0,"step":99,"to":0}),
        json!({"action":0,"step":0.5,"to":0}),
        json!({"action":0,"to":-1}),
        json!({"action":0,"to":null}),
    ] {
        assert!(s.execute("actions.move", params).is_err());
        assert_eq!(s.actions, before);
    }
    let journal = s.journal.clone();
    s.execute("actions.move", json!({"action":0,"step":0,"to":2})).unwrap();
    s.execute("actions.move", json!({"action":0,"step":1,"to":0})).unwrap();
    assert_eq!(s.journal, journal);
    s.execute("layer.new.layer", json!({"name":"Four"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].steps.iter().map(|st| st.1["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Three", "Two", "One", "Four"]);
    s.execute("file.new", json!({"width":4,"height":4})).unwrap();
    s.execute("actions.play", json!({"action":0})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.iter().skip(1).map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Three", "Two", "One", "Four"]);
}

#[test]
fn new_names_do_not_collide_with_named_calls() {
    let mut s = Session::new();
    s.actions.list.push(Action { name: "Action 2".into(), steps: vec![], disabled: Default::default(), ..Default::default() });
    assert_eq!(s.execute("actions.record", json!({})).unwrap()["action"], "Action 3");
    s.execute("actions.stop", json!({})).unwrap();
    assert!(s.execute("actions.record", json!({"name":"Action 2"})).is_err());
    assert!(s.actions.recording.is_none());
}

#[test]
fn moving_action_keeps_recording_attached_to_it() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4})).unwrap();
    for name in ["A", "B", "C"] {
        s.actions.list.push(Action { name: name.into(), steps: vec![], disabled: Default::default(), ..Default::default() });
    }
    s.execute("actions.record", json!({"action":"B"})).unwrap();
    s.execute("layer.new.layer", json!({"name":"before"})).unwrap();
    s.execute("actions.move", json!({"action":"B","to":2})).unwrap();
    s.execute("actions.move", json!({"action":"A","to":2})).unwrap();
    s.execute("layer.new.layer", json!({"name":"after"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[1].name, "B");
    assert_eq!(s.actions.list[1].steps.len(), 2);
}

#[test]
fn bad_params_do_not_panic() {
    let mut s = Session::new();
    let junk = [
        Value::Null,
        json!([]),
        json!("x"),
        json!(1),
        json!({"action": true}),
        json!({"action": -1}),
        json!({"action": 99}),
        json!({"action": ""}),
        json!({"name": 1}),
        json!({"name": ""}),
        json!({"from": "start"}),
        json!({"from": -3}),
        json!({"action": "missing", "from": 4}),
    ];
    for id in ["actions.list", "actions.get", "actions.record", "actions.stop", "actions.play", "actions.delete", "actions.move"] {
        for p in &junk {
            let _ = s.execute(id, p.clone());
        }
    }
    assert_eq!(s.actions.playing, 0);
    assert!(crate::commands::command_specs().iter().any(|c| c.id == "actions.play" && !c.journal && c.menu.is_empty()));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn actions_persist_with_the_preset_store() {
    let dir = std::env::temp_dir().join(format!("pc-actions-{}-persist", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Session::new();
    let warnings = s.attach_preset_store(crate::preset_store::open_dir(&dir));
    assert!(warnings.iter().all(|w| !w.contains("actions")), "{warnings:?}");
    s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    s.execute("actions.record", json!({"name": "Kept"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    drop(s);

    let mut t = Session::new();
    t.attach_preset_store(crate::preset_store::open_dir(&dir));
    let listed = t.execute("actions.list", json!({})).unwrap();
    assert_eq!(listed["actions"][0]["name"], "Kept");
    assert_eq!(listed["actions"][0]["steps"], 1);
    assert_eq!(t.execute("actions.get", json!({"action": "Kept"})).unwrap()["steps"][0][0], "layer.new.layer");

    // An action created before the store finishes loading is kept, and a different one on
    // disk is merged in.
    let mut early = Session::new();
    early.execute("actions.record", json!({"name": "Early"})).unwrap();
    early.execute("actions.stop", json!({})).unwrap();
    early.attach_preset_store(crate::preset_store::open_dir(&dir));
    let names: Vec<String> = early.actions.list.iter().map(|a| a.name.clone()).collect();
    assert!(names.contains(&"Early".into()) && names.contains(&"Kept".into()), "{names:?}");

    std::fs::write(dir.join("actions.json"), b"{").unwrap();
    let mut bad = Session::new();
    let warnings = bad.attach_preset_store(crate::preset_store::open_dir(&dir));
    assert!(warnings.iter().any(|w| w.contains("actions")), "{warnings:?}");
    assert!(bad.actions.list.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// AUTO-219-4: a recording made on one document replays on a second one — a crop, a filter,
/// a selection fill and styled text all act on the document the replay runs in.
#[test]
fn a_recorded_edit_run_replays_on_a_second_document() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    s.execute("actions.record", json!({"name": "Studio"})).unwrap();
    s.execute("image.crop", json!({"x": 8, "y": 8, "width": 48, "height": 48})).unwrap();
    s.execute("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 12, "height": 12})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("type.create", json!({"x": 10, "y": 36, "text": "Hi", "size": 12, "color": "#000000"})).unwrap();
    s.execute("type.setStyle", json!({"fauxBold": true})).unwrap();
    let stopped = s.execute("actions.stop", json!({})).unwrap();
    let steps = stopped["steps"].as_u64().unwrap();
    assert!(steps >= 7, "the recording keeps every family: {stopped}");

    // The second document: a different size, background and name. The replay must act here.
    s.execute("file.new", json!({"width": 100, "height": 80, "background": "black", "name": "Second"})).unwrap();
    let played = s.execute("actions.play", json!({"action": "Studio"})).unwrap();
    assert!(played.get("failed").is_none(), "replay: {played}");
    assert_eq!(played["ran"].as_u64(), Some(steps), "every step ran: {played}");
    let d = &s.active().unwrap().doc;
    assert_eq!((d.size.width, d.size.height), (48, 48), "the crop replayed onto the second document");
    assert_eq!(pixel(&s, 0, 16, 16), vec![1.0, 0.0, 0.0, 1.0], "the selection fill replayed");
    assert_eq!(pixel(&s, 0, 2, 2), vec![0.0, 0.0, 0.0, 1.0], "outside the selection stays black");
    let last = d.layers.last().unwrap();
    match &last.content {
        photocraft_doc::LayerContent::Text(t) => {
            assert_eq!(t.text, "Hi", "the styled text replayed");
            assert!(t.char_runs().iter().any(|r| r.style.faux_bold), "the bold run replayed");
        }
        other => panic!("expected the recorded type layer, got {other:?}"),
    }
    // The first document is untouched.
    let first = &s.documents()[0].doc;
    assert_eq!((first.size.width, first.size.height), (48, 48), "the recording document keeps its own crop");
}

#[test]
fn button_color_is_one_of_the_photoshop_swatches() {
    let mut s = Session::new();
    s.execute("actions.record", json!({"name": "Print"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let set = s.execute("actions.setColor", json!({"action": "Print", "color": "blue"})).unwrap();
    assert_eq!(set["color"], "blue");
    assert_eq!(s.actions.list[0].color, "blue");
    assert_eq!(s.execute("actions.get", json!({"action": 0})).unwrap()["color"], "blue");
    let saved = serde_json::to_string(&s.actions.list).unwrap();
    assert!(saved.contains("blue"));
    s.execute("actions.setColor", json!({"action": 0, "color": ""})).unwrap();
    assert!(s.actions.list[0].color.is_empty());
    assert!(!serde_json::to_string(&s.actions.list).unwrap().contains("color"), "an empty colour is omitted");
    for bad in [json!({"action": 0, "color": "pink"}), json!({"action": 0}), json!({"action": 4, "color": "red"}), json!({"color": "red"})] {
        assert!(s.execute("actions.setColor", bad).is_err());
    }
    assert!(s.actions.list[0].color.is_empty(), "rejected colours change nothing");
}

#[test]
fn arrange_that_cannot_move_is_a_noop_in_playback() {
    // #8453: imported actions make an adjustment layer, then Bring it to Front, where it already is.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let steps = vec![
        ("layer.newAdjustmentLayer.curves".to_string(), json!({})),
        ("layer.arrange.bringToFront".to_string(), json!({})),
        ("layer.new.layer".to_string(), json!({"name": "After"})),
    ];
    s.actions.list.push(Action::new("Repro", steps));
    let before = s.active().unwrap().history.past_len();
    let played = s.execute("actions.play", json!({"action": 0})).unwrap();
    assert_eq!(played["ran"], 3, "{played}");
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(s.active().unwrap().history.past_len(), before + 2, "the no-op adds no history step");

    // The predicate identifies no movement for every layer: top level, above the Background,
    // and inside a group.
    let a = s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].clone();
    s.execute("layer.select", json!({"layer": a})).unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
    let cmds = [("layer.arrange.bringForward", 1), ("layer.arrange.bringToFront", 1), ("layer.arrange.sendBackward", -1), ("layer.arrange.sendToBack", -1)];
    let ids: Vec<_> = s.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l.id).collect();
    let bg = crate::background_cmds::background_id(&s.active().unwrap().doc);
    for id in ids.into_iter().filter(|id| Some(*id) != bg) {
        for (cmd, dir) in cmds {
            let at_end = super::super::ops::arrange_at_end(&s.active().unwrap().doc, id, dir);
            let history = s.active().unwrap().history.past_len();
            let ran = s.execute(cmd, json!({"layer": id.0}));
            if at_end {
                if matches!(cmd, "layer.arrange.bringToFront" | "layer.arrange.sendToBack") {
                    assert_eq!(ran.unwrap(), json!({"moved": false}), "{cmd} on {id:?}");
                } else {
                    assert!(ran.is_err(), "{cmd} on {id:?}: {ran:?}");
                }
                assert_eq!(s.active().unwrap().history.past_len(), history, "a no-op must not add history");
            } else {
                ran.unwrap();
                assert_eq!(s.active().unwrap().history.past_len(), history + 1);
                s.execute("edit.undo", json!({})).unwrap();
            }
        }
    }

    // Arranging the Background still stops playback: Photoshop refuses it too.
    let bg = bg.unwrap();
    s.actions.list.push(Action::new("Bg", vec![("layer.arrange.bringToFront".to_string(), json!({"layer": bg.0}))]));
    let played = s.execute("actions.play", json!({"action": 1})).unwrap();
    assert_eq!(played["ran"], 0, "{played}");
    assert!(played["failed"].is_object(), "{played}");
}
