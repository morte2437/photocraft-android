use photocraft_doc::LayerId;
use serde_json::{Value, json};

use crate::jobs::Started;
use crate::{EngineError, Session};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    s
}

fn new_layer(s: &mut Session, name: &str) -> u64 {
    s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap()
}

fn last(s: &Session) -> &(String, Value) {
    s.journal.last().unwrap()
}

fn background(s: &Session) -> u64 {
    s.active().unwrap().doc.layers[0].id.0
}

#[test]
fn the_active_layer_is_implicit() {
    let mut s = session();
    let shade = new_layer(&mut s, "Shade");
    s.execute("layer.setProps", json!({"layer": shade, "opacity": 0.5})).unwrap();
    assert_eq!(last(&s), &("layer.setProps".into(), json!({"opacity": 0.5})), "the active layer's id is dropped");
    assert_eq!(s.active().unwrap().doc.layer(LayerId(shade)).unwrap().opacity, 0.5, "the command still ran on it");
}

#[test]
fn another_layer_is_named() {
    let mut s = session();
    let bg = background(&s);
    new_layer(&mut s, "Shade");
    s.execute("layer.setProps", json!({"layer": bg, "visible": false})).unwrap();
    assert_eq!(last(&s), &("layer.setProps".into(), json!({"layer": "Background", "visible": false})));
}

#[test]
fn selecting_a_layer_is_always_by_name() {
    let mut s = session();
    let shade = new_layer(&mut s, "Shade");
    s.execute("layer.select", json!({"layer": shade})).unwrap();
    assert_eq!(last(&s), &("layer.select".into(), json!({"layer": "Shade"})), "even though it was the active layer");
    s.execute("layer.select", json!({"layer": shade, "mode": "toggle"})).unwrap();
    assert_eq!(last(&s).1["mode"], "toggle", "the other params are kept");
}

#[test]
fn an_ambiguous_name_keeps_the_id() {
    let mut s = session();
    let a = new_layer(&mut s, "Copy");
    let b = new_layer(&mut s, "Copy");
    s.execute("layer.setProps", json!({"layer": a, "opacity": 0.5})).unwrap();
    assert_eq!(last(&s), &("layer.setProps".into(), json!({"layer": a, "opacity": 0.5})), "two layers are called Copy");
    s.execute("layer.setProps", json!({"layer": b, "opacity": 0.5})).unwrap();
    assert_eq!(last(&s).1, json!({"opacity": 0.5}), "the active one is still implicit");
}

#[test]
fn one_of_several_selected_layers_stays_named() {
    let mut s = session();
    let bg = background(&s);
    let shade = new_layer(&mut s, "Shade");
    s.execute("layer.select", json!({"layer": bg, "mode": "add"})).unwrap();
    assert_eq!(s.active().unwrap().selected_layers().len(), 2);
    let active = s.active().unwrap().active_layer.unwrap().0;
    let name = if active == shade { "Shade" } else { "Background" };
    s.execute("layer.setProps", json!({"layer": active, "opacity": 0.5})).unwrap();
    assert_eq!(last(&s).1, json!({"layer": name, "opacity": 0.5}), "without the layer, a layer command acts on the whole selection");
}

#[test]
fn a_name_or_no_document_is_left_alone() {
    let mut s = session();
    new_layer(&mut s, "Shade");
    s.execute("layer.setProps", json!({"layer": "Background", "visible": false})).unwrap();
    assert_eq!(last(&s).1, json!({"layer": "Background", "visible": false}));
    let e = s.execute("layer.setProps", json!({"layer": 999, "visible": false})).unwrap_err();
    assert!(matches!(e, EngineError::UnknownLayer { .. }), "{e}");
}

#[test]
fn a_background_job_journals_the_same_way() {
    let mut s = session();
    let shade = new_layer(&mut s, "Shade");
    let id = match s.start("filter.blur.gaussianBlur", json!({"layer": shade, "radius": 2})).unwrap() {
        Started::Job(id) => id,
        Started::Done(v) => panic!("ran inline: {v}"),
    };
    s.wait_job(id).unwrap();
    assert_eq!(last(&s), &("filter.blur.gaussianBlur".into(), json!({"radius": 2})));
}

#[test]
fn frame_action_keeps_the_layer_it_wraps() {
    let mut s = session();
    let picture = new_layer(&mut s, "Picture");
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("actions.record", json!({"name": "Frame picture"})).unwrap();
    s.execute("layer.new.frame", json!({"x": 1, "y": 1, "width": 20, "height": 20, "layer": picture})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action": "Frame picture"})).unwrap()["steps"].clone();
    assert_eq!(steps[0][1]["layer"], "Picture", "the frame must wrap the recorded source layer");
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    new_layer(&mut s, "Spare");
    let target = new_layer(&mut s, "Picture");
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    assert_ne!(target, picture, "the replay target has a different document-local id");
    let played = s.execute("actions.play", json!({"action": "Frame picture"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(
        s.active().unwrap().doc.walk().iter().any(
            |(_, _, layer)| matches!(&layer.content, photocraft_doc::LayerContent::Group(group) if group.children.iter().any(|child| child.id.0 == target))
        ),
        "the replayed frame wraps Picture"
    );
}

#[test]
fn selection_actions_keep_the_active_layer_as_their_source() {
    let mut s = session();
    let raster = new_layer(&mut s, "Pixels");
    s.execute("actions.record", json!({"name": "Load pixels"})).unwrap();
    s.execute("select.loadSelection", json!({"channel": "transparency", "layer": raster})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action": "Load pixels"})).unwrap()["steps"].clone();
    assert_eq!(steps[0][1]["layer"], json!({"name": "Pixels"}), "missing layer loads merged pixels instead");
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    new_layer(&mut s, "Pixels");
    let played = s.execute("actions.play", json!({"action": "Load pixels"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(
        s.active().unwrap().doc.selection.as_ref().map(|mask| mask.content_bounds()).unwrap_or(photocraft_geom::Rect::EMPTY),
        photocraft_geom::Rect::EMPTY,
        "the empty source layer must not load the opaque merged document"
    );

    let type_id = s.execute("type.create", json!({"x": 2, "y": 20, "text": "I", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    let type_name = s.active().unwrap().doc.layer(LayerId(type_id)).unwrap().name.clone();
    s.execute("actions.record", json!({"name": "Mask text"})).unwrap();
    s.execute("type.createSelection", json!({"layer": type_id, "removeLayer": true})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action": "Mask text"})).unwrap()["steps"].clone();
    assert_eq!(steps[0][1]["layer"], type_name, "without the source, replay cannot remove that type layer");
    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    new_layer(&mut s, "Spare");
    let replay_type = s.execute("type.create", json!({"x": 2, "y": 20, "text": "I", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    assert_ne!(replay_type, type_id);
    let played = s.execute("actions.play", json!({"action": "Mask text"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(s.active().unwrap().doc.selection.as_ref().is_some_and(|mask| !mask.content_bounds().is_empty()), "replay selects the glyphs");
    assert!(s.active().unwrap().doc.layer(LayerId(replay_type)).is_none(), "replay removed the named source type layer");
}

#[test]
fn named_nonactive_smart_layer_passes_admission_and_converts_to_layers() {
    let mut s = session();
    s.execute("type.create", json!({"x": 2, "y": 20, "text": "I", "size": 12})).unwrap();
    let smart = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
    let name = s.active().unwrap().doc.layer(LayerId(smart)).unwrap().name.clone();
    let top = new_layer(&mut s, "Top");
    let restored = s.execute("layer.smartObjects.convertToLayers", json!({"layer": name})).unwrap()["layer"].as_u64().unwrap();
    assert!(s.active().unwrap().doc.layer(LayerId(smart)).is_none());
    assert!(s.active().unwrap().doc.layer(LayerId(top)).is_some());
    assert!(s.active().unwrap().doc.layer(LayerId(restored)).is_some(), "the restored layer is present");
    let error = s.execute("layer.smartObjects.convertToLayers", json!({"layer": "Removed smart"})).unwrap_err();
    assert!(error.to_string().contains("Removed smart"), "missing named layer should be identified: {error}");
}

#[test]
fn named_filter_mask_target_uses_the_recorded_smart_layer() {
    let mut s = session();
    s.execute("type.create", json!({"x": 2, "y": 20, "text": "I", "size": 12})).unwrap();
    let smart = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
    let name = s.active().unwrap().doc.layer(LayerId(smart)).unwrap().name.clone();
    new_layer(&mut s, "Top");
    assert_eq!(crate::channel_cmds::filter_mask_layer(&s, "edit.fill", &json!({"target": "filterMask", "layer": name})), Some(LayerId(smart)));
}

#[test]
fn named_nonactive_layer_is_the_one_converted_to_smart() {
    let mut s = session();
    let target = s.execute("type.create", json!({"x": 2, "y": 20, "text": "I", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    let name = s.active().unwrap().doc.layer(LayerId(target)).unwrap().name.clone();
    new_layer(&mut s, "Top");
    s.execute("actions.record", json!({"name": "Convert source"})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({"layer": target})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action": "Convert source"})).unwrap()["steps"].clone();
    assert_eq!(steps[0][1]["layer"], name, "the journal names the explicit source");

    s.execute("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
    let replay_target = s.execute("type.create", json!({"x": 2, "y": 20, "text": "I", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    let top = new_layer(&mut s, "Top");
    let played = s.execute("actions.play", json!({"action": "Convert source"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(s.active().unwrap().doc.layer(LayerId(replay_target)).is_none(), "replay removes the named source");
    assert!(s.active().unwrap().doc.layer(LayerId(top)).is_some(), "replay keeps the selected top layer");
    assert!(
        s.active().unwrap().doc.walk().iter().any(|(_, _, layer)| matches!(layer.content, photocraft_doc::LayerContent::Smart(_))),
        "replay creates a smart object"
    );
}

#[test]
fn merged_named_layer_keeps_its_id_as_a_selection_source() {
    let mut s = session();
    let source = new_layer(&mut s, "merged");
    new_layer(&mut s, "Top");
    s.execute("select.loadSelection", json!({"channel": "transparency", "layer": source})).unwrap();
    assert_eq!(last(&s).1["layer"], source, "the reserved name must not mean the merged document");
}

#[test]
fn keystrokes_fold_into_the_type_layer_they_made() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 2, "y": 20, "text": "", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    let n = s.journal.len();
    for (i, ch) in "Hey".chars().enumerate() {
        s.execute("type.edit", json!({"layer": id, "replace": {"start": i, "end": i, "text": ch.to_string()}, "coalesce": "t1"})).unwrap();
    }
    // Backspace, then more.
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 2, "end": 3, "text": ""}, "coalesce": "t1"})).unwrap();
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 2, "end": 2, "text": "llo"}, "coalesce": "t1"})).unwrap();
    assert_eq!(s.journal.len(), n, "every keystroke folded into Make Type Layer");
    let (cmd, p) = last(&s);
    assert_eq!(cmd, "type.create");
    assert_eq!(p["text"], "Hello");
    assert!(p.get("replace").is_none() && p.get("coalesce").is_none(), "{p}");
}

#[test]
fn keystrokes_in_an_existing_layer_keep_separate_replacements() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 2, "y": 20, "text": "Hi", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"opacity": 0.9})).unwrap();
    let n = s.journal.len();
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 2, "end": 2, "text": "!"}, "coalesce": "t2"})).unwrap();
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 0, "end": 1, "text": "h"}, "coalesce": "t2"})).unwrap();
    assert_eq!(s.journal.len(), n + 2);
    assert_eq!(s.journal[n].1, json!({"replace": {"start": 2, "end": 2, "text": "!"}, "coalesce": "t2"}));
    assert_eq!(last(&s).1, json!({"replace": {"start": 0, "end": 1, "text": "h"}, "coalesce": "t2"}));
    // A new editing session starts a new step.
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 3, "end": 3, "text": "?"}, "coalesce": "t3"})).unwrap();
    assert_eq!(s.journal.len(), n + 3);
    assert_eq!(last(&s).1, json!({"replace": {"start": 3, "end": 3, "text": "?"}, "coalesce": "t3"}));
}

#[test]
fn action_replay_preserves_styles_between_separate_typing_edits() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 2, "y": 20, "text": "a M z", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.edit", json!({"layer": id, "runs": [{"start": 2, "end": 3, "size": 24}]})).unwrap();
    let replay = s.active().unwrap().doc.as_ref().clone();
    s.execute("actions.record", json!({"name": "Two typed edits"})).unwrap();
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 0, "end": 1, "text": "A"}, "coalesce": "typing"})).unwrap();
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 4, "end": 5, "text": "Z"}, "coalesce": "typing"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let original = s.active().unwrap().doc.layer(LayerId(id)).unwrap();
    let photocraft_doc::LayerContent::Text(original) = &original.content else { panic!("type layer") };
    let original = original.clone();
    assert_eq!(original.text, "A M Z");
    s.add_document(replay, None);
    let played = s.execute("actions.play", json!({"action": "Two typed edits"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    let copy = s.active().unwrap().doc.layer(LayerId(id)).unwrap();
    let photocraft_doc::LayerContent::Text(copy) = &copy.content else { panic!("type layer") };
    assert_eq!(copy.text, original.text);
    assert_eq!(copy.char_runs(), original.char_runs(), "the unchanged middle run keeps its style");
}

#[test]
fn recording_does_not_fold_into_a_step_from_before_recording_started() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 2, "y": 20, "text": "Hi", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("actions.record", json!({"name": "Type more"})).unwrap();
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 2, "end": 2, "text": "!"}, "coalesce": "t2"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action": "Type more"})).unwrap()["steps"].clone();
    assert_eq!(steps, json!([["type.edit", {"replace": {"start": 2, "end": 2, "text": "!"}, "coalesce": "t2"}]]));
}

#[test]
fn keystrokes_into_another_layer_do_not_fold() {
    let mut s = session();
    let first = s.execute("type.create", json!({"x": 2, "y": 10, "text": "One", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    let second = s.execute("type.create", json!({"x": 2, "y": 20, "text": "", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    // The Type tool selects a clicked layer without a command.
    s.select_layer(LayerId(first)).unwrap();
    let n = s.journal.len();
    s.execute("type.edit", json!({"layer": first, "replace": {"start": 3, "end": 3, "text": "!"}, "coalesce": "t4"})).unwrap();
    assert_eq!(s.journal.len(), n + 1, "typing into the first layer is its own step");
    assert_eq!(last(&s).1, json!({"replace": {"start": 3, "end": 3, "text": "!"}, "coalesce": "t4"}));
    assert_eq!(s.journal[n - 1].1["text"], "", "the second Make Type Layer keeps its own text");
    // Programmatic edits (no session key) and other edits stand on their own.
    s.execute("type.edit", json!({"layer": second, "replace": {"start": 0, "end": 0, "text": "x"}})).unwrap();
    assert!(last(&s).1["layer"].is_string() && last(&s).1.get("replace").is_some(), "{}", last(&s).1);
}

#[test]
fn keystrokes_in_another_document_do_not_fold_into_the_previous_document() {
    let mut s = session();
    let first = s.execute("type.create", json!({"x": 2, "y": 20, "text": "A", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    let mut copy = s.active().unwrap().doc.as_ref().clone();
    if let Some(layer) = copy.layer_mut(LayerId(first))
        && let photocraft_doc::LayerContent::Text(text) = &mut layer.content
    {
        text.text = "B".into();
    }
    s.add_document(copy, None);
    let second = first;
    let before = s.journal.len();
    s.execute("type.edit", json!({"layer": second, "replace": {"start": 1, "end": 1, "text": "!"}, "coalesce": "typing"})).unwrap();
    assert_eq!(s.journal.len(), before + 1, "typing in a different document needs a new journal step");
    assert_eq!(s.journal[before - 1].1["text"], "A");
    assert_eq!(last(&s).1["replace"], json!({"start": 1, "end": 1, "text": "!"}));
}

#[test]
fn a_recording_replays_on_a_document_with_other_layer_ids() {
    let mut s = session();
    let bg = background(&s);
    s.execute("actions.record", json!({"name": "Shade"})).unwrap();
    let shade = new_layer(&mut s, "Shade");
    s.execute("layer.setProps", json!({"layer": shade, "opacity": 0.5})).unwrap();
    s.execute("layer.select", json!({"layer": bg})).unwrap();
    s.execute("edit.fill", json!({"layer": bg, "color": "#ff0000"})).unwrap();
    let id = s.execute("type.create", json!({"x": 2, "y": 20, "text": "", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    for (i, ch) in "Hi".chars().enumerate() {
        s.execute("type.edit", json!({"layer": id, "replace": {"start": i, "end": i, "text": ch.to_string()}, "coalesce": "t5"})).unwrap();
    }
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action": "Shade"})).unwrap()["steps"].clone();
    let ids: Vec<&str> = steps.as_array().unwrap().iter().map(|st| st[0].as_str().unwrap()).collect();
    assert_eq!(ids, ["layer.new.layer", "layer.setProps", "layer.select", "edit.fill", "type.create"], "{steps}");
    assert!(steps.as_array().unwrap().iter().all(|st| st[1].get("layer").is_none_or(Value::is_string)), "no layer ids: {steps}");

    // A second document whose layers have other ids and a layer stacked over the background.
    s.execute("file.new", json!({"width": 40, "height": 40, "background": "black"})).unwrap();
    new_layer(&mut s, "Top");
    assert_ne!(background(&s), bg);
    let played = s.execute("actions.play", json!({"action": "Shade"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    let d = &s.active().unwrap().doc;
    let shade = d.walk().into_iter().find(|(_, _, l)| l.name == "Shade").map(|(_, _, l)| l).unwrap();
    assert_eq!(shade.opacity, 0.5, "the new layer, not the one of the first document, got the opacity");
    let text = d.walk().into_iter().find_map(|(_, _, l)| match &l.content {
        photocraft_doc::LayerContent::Text(t) => Some(t.text.clone()),
        _ => None,
    });
    assert_eq!(text.as_deref(), Some("Hi"), "the typed text replayed as one step");
    assert!(s.documents()[0].doc.layer(LayerId(bg)).is_some());
}

#[test]
fn recorded_edit_targets_a_named_nonactive_type_layer_on_another_document() {
    let mut s = session();
    let first = s.execute("type.create", json!({"x": 2, "y": 20, "text": "Old", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": first, "name": "Caption"})).unwrap();
    new_layer(&mut s, "Top");
    s.execute("actions.record", json!({"name": "Edit caption"})).unwrap();
    s.execute("type.edit", json!({"layer": first, "text": "New"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let steps = s.execute("actions.get", json!({"action": "Edit caption"})).unwrap()["steps"].clone();
    assert_eq!(steps, json!([["type.edit", {"layer": "Caption", "text": "New"}]]));

    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    let second = s.execute("type.create", json!({"x": 2, "y": 20, "text": "Other", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": second, "name": "Caption"})).unwrap();
    new_layer(&mut s, "Top");
    assert_ne!(first, second);
    let played = s.execute("actions.play", json!({"action": "Edit caption"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    let layer = s.active().unwrap().doc.layer(LayerId(second)).unwrap();
    let photocraft_doc::LayerContent::Text(text) = &layer.content else { panic!("expected type layer") };
    assert_eq!(text.text, "New");
}
