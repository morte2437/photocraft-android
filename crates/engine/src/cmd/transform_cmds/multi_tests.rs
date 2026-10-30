//! Free Transform of several selected layers by one box (#6501).

use super::*;

/// A 200 x 200 document: a red 40 x 20 block on "Paint" (x 10..50, y 10..30) and a type layer
/// over it, both selected, the type layer active (as after creating it and ⌘-clicking Paint).
fn two_layers() -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    let paint = LayerId(s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap());
    s.edit("paint", |doc, _| {
        doc.layer_mut(paint).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 10, 50, 30), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let text = LayerId(s.execute("type.create", json!({"x": 80, "y": 120, "text": "Hi", "size": 30})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    s.execute("layer.select", json!({"layer": text.0, "mode": "add"})).unwrap();
    (s, paint, text)
}

fn bounds(s: &Session, id: LayerId) -> Rect {
    let st = s.active().unwrap();
    transform_bounds(&st.doc, st.doc.layer(id).unwrap())
}

fn text_matrix(s: &Session, id: LayerId) -> [f64; 6] {
    let st = s.active().unwrap();
    let LayerContent::Text(t) = &st.doc.layer(id).unwrap().content else { panic!("not a type layer") };
    t.transform.m
}

#[test]
fn free_transform_moves_every_selected_layer_in_one_step() {
    let (mut s, paint, text) = two_layers();
    let (p0, t0, m0) = (bounds(&s, paint), bounds(&s, text), text_matrix(&s, text));
    let steps = s.active().unwrap().history.past_len();
    // Before #6501 only the active layer moved, whichever kind it was.
    let r = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 30, 40]})).unwrap();
    assert_eq!(r["layers"], json!([paint.0, text.0]));
    assert_eq!(bounds(&s, paint), p0.translate(30, 40));
    assert_eq!(bounds(&s, text), t0.translate(30, 40));
    let st = s.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 1, "one history step");
    let history = s.execute("document.inspect", json!({})).unwrap()["history"].clone();
    assert_eq!(history.as_array().unwrap().last(), Some(&json!("Free Transform")));
    // The type stays editable: it keeps the transform instead of pixels.
    assert_eq!(text_matrix(&s, text)[4..], [m0[4] + 30.0, m0[5] + 40.0]);
    s.undo();
    assert_eq!((bounds(&s, paint), bounds(&s, text)), (p0, t0), "and one Undo takes both back");
}

#[test]
fn the_default_frame_is_the_selected_layers_together() {
    let (mut s, paint, text) = two_layers();
    let both = bounds(&s, paint).union(&bounds(&s, text));
    let m0 = text_matrix(&s, text);
    // 200 % about the joined frame's centre: each layer scales about that point, not its own.
    let r = s.execute("edit.transform", json!({"scaleX": 200, "scaleY": 200})).unwrap();
    let rect: Vec<f64> = serde_json::from_value(r["rect"].clone()).unwrap();
    assert_eq!(rect, [both.x0, both.y0, both.x1, both.y1].map(f64::from));
    let (cx, cy) = (f64::from(both.x0 + both.x1) / 2.0, f64::from(both.y0 + both.y1) / 2.0);
    let p = bounds(&s, paint);
    assert_eq!((f64::from(p.x0), f64::from(p.y0)), (2.0 * 10.0 - cx, 2.0 * 10.0 - cy));
    let m = text_matrix(&s, text);
    assert!((m[0] - 2.0).abs() < 1e-9 && (m[3] - 2.0).abs() < 1e-9, "{m:?}");
    assert!((m[4] - (2.0 * m0[4] - cx)).abs() < 1e-9 && (m[5] - (2.0 * m0[5] - cy)).abs() < 1e-9, "{m:?}");
}

#[test]
fn an_explicit_layer_still_transforms_that_layer_alone() {
    let (mut s, paint, text) = two_layers();
    let t0 = bounds(&s, text);
    let r = s.execute("edit.transform", json!({"layer": paint.0, "matrix": [1, 0, 0, 1, 5, 0]})).unwrap();
    assert_eq!(r["layers"], json!([paint.0]));
    assert_eq!(bounds(&s, paint), Rect::new(15, 10, 55, 30));
    assert_eq!(bounds(&s, text), t0);
}

#[test]
fn the_layers_param_names_the_layers_and_a_listed_group_carries_its_children() {
    let (mut s, paint, text) = two_layers();
    let t0 = bounds(&s, text);
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    let group = LayerId(s.execute("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
    // The group and the layer in it: the layer moves once, with its group.
    let r = s.execute("edit.transform", json!({"layers": [paint.0, group.0], "matrix": [1, 0, 0, 1, 7, 0]})).unwrap();
    assert_eq!(r["layers"], json!([group.0]));
    assert_eq!(bounds(&s, paint), Rect::new(17, 10, 57, 30));
    assert_eq!(bounds(&s, text), t0, "not listed, not moved");
}

#[test]
fn the_background_stays_out_of_a_selection_of_several() {
    let (mut s, paint, text) = two_layers();
    let bg = s.active().unwrap().doc.layers[0].id;
    s.execute("layer.select", json!({"layer": bg.0, "mode": "add"})).unwrap();
    let r = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 3, 0]})).unwrap();
    assert_eq!(r["layers"], json!([paint.0, text.0]));
    let st = s.active().unwrap();
    assert!(st.doc.is_background(bg), "still the Background");
    assert_eq!(st.doc.layer(bg).unwrap().surface().unwrap().content_bounds(), Rect::new(0, 0, 200, 200));
}

#[test]
fn a_selection_limits_the_pixels_of_every_selected_layer() {
    let (mut s, paint, text) = two_layers();
    let t0 = bounds(&s, text);
    // The left half of the block: only those pixels of Paint move; the type moves whole (#2630).
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 30, "height": 200})).unwrap();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 0, 100]})).unwrap();
    let st = s.active().unwrap();
    let surf = st.doc.layer(paint).unwrap().surface().unwrap();
    assert_eq!(surf.content_bounds(), Rect::new(10, 10, 50, 130));
    assert_eq!(surf.sample_channel(20, 15, 3), 0.0, "the selected part left");
    assert_eq!(surf.sample_channel(40, 15, 3), 1.0, "the rest stayed");
    assert_eq!(surf.sample_channel(20, 115, 3), 1.0, "and landed below");
    assert_eq!(bounds(&s, text), t0.translate(0, 100));
}

#[test]
fn a_locked_layer_among_them_refuses_the_whole_transform() {
    let (mut s, paint, text) = two_layers();
    s.execute("layer.setProps", json!({"layer": text.0, "locks": {"position": true}})).unwrap();
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    s.execute("layer.select", json!({"layer": text.0, "mode": "add"})).unwrap();
    let st = s.active().unwrap();
    let (before, revision) = (st.doc.clone(), st.revision);
    let err = s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 5, 0]})).unwrap_err().to_string();
    assert!(err.contains("locked"), "{err}");
    let st = s.active().unwrap();
    assert!(st.doc == before && st.revision == revision, "nothing moved, not even the unlocked layer");
}

#[test]
fn one_layer_past_the_pixel_budget_leaves_every_layer_as_it_was() {
    let (mut s, paint, _) = two_layers();
    // A 1 px dot, beside the 40 px block: only the block, stretched, outgrows the test budget.
    let dot = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.edit("dot", |doc, _| {
        doc.layer_mut(dot).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 40, 11, 41), &[0.0, 1.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let st = s.active().unwrap();
    let (before, revision) = (st.doc.clone(), st.revision);
    let k = 4e4;
    assert!(s.execute("edit.transform", json!({"layers": [dot.0, paint.0], "matrix": [k, 0, 0, 1, -10.0 * k, 0]})).is_err());
    let st = s.active().unwrap();
    assert!(st.doc == before && st.revision == revision);
}

#[test]
fn distort_with_type_among_the_layers_is_refused() {
    let (mut s, _, _) = two_layers();
    let before = s.active().unwrap().doc.clone();
    let quad = json!({"rect": [0, 0, 100, 100], "quad": [[0, 0], [100, 0], [120, 100], [-20, 100]]});
    assert!(s.execute("edit.transform", quad).is_err());
    assert!(s.active().unwrap().doc == before);
}

#[test]
fn transform_again_repeats_on_every_selected_layer() {
    let (mut s, paint, text) = two_layers();
    let (p0, t0) = (bounds(&s, paint), bounds(&s, text));
    let both = p0.union(&t0);
    let r = [both.x0, both.y0, both.x1, both.y1].map(f64::from);
    let quad = [[r[0] + 10.0, r[1]], [r[2] + 10.0, r[1]], [r[2] + 10.0, r[3]], [r[0] + 10.0, r[3]]];
    // Nearest neighbour: no resampling fringe to blur the bounds.
    s.execute("edit.transform", json!({"rect": r, "quad": quad, "interpolation": "nearest"})).unwrap();
    s.execute("edit.transform.again", json!({})).unwrap();
    assert_eq!(bounds(&s, paint), p0.translate(20, 0));
    assert_eq!(bounds(&s, text), t0.translate(20, 0));
}

#[test]
fn transform_again_on_a_copy_copies_and_moves_the_active_layer_only() {
    let (mut s, paint, text) = two_layers();
    s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 10, 0]})).unwrap();
    let (p1, t1) = (bounds(&s, paint), bounds(&s, text));
    s.execute("edit.transform.againCopy", json!({})).unwrap();
    let st = s.active().unwrap();
    let copy = st.active_layer.unwrap();
    assert!(copy != paint && copy != text);
    assert_eq!(bounds(&s, copy), t1.translate(10, 0), "the copy of the active type layer moved on");
    assert_eq!((bounds(&s, paint), bounds(&s, text)), (p1, t1), "the originals stayed");
}

#[test]
fn a_null_layer_beside_layers_counts_as_absent() {
    let (mut s, paint, text) = two_layers();
    let t0 = bounds(&s, text);
    let r = s.execute("edit.transform", json!({"layers": [paint.0], "layer": null, "matrix": [1, 0, 0, 1, 5, 0]})).unwrap();
    assert_eq!(r["layers"], json!([paint.0]));
    assert_eq!(bounds(&s, paint), Rect::new(15, 10, 55, 30));
    assert_eq!(bounds(&s, text), t0);
}

#[test]
fn bad_layers_params_are_errors_not_panics() {
    let (mut s, paint, _) = two_layers();
    let before = s.active().unwrap().doc.clone();
    let m = json!([1, 0, 0, 1, 5, 0]);
    for p in [
        json!({"layers": [], "matrix": m}),
        json!({"layers": "all", "matrix": m}),
        json!({"layers": [paint.0, "x"], "matrix": m}),
        json!({"layers": [-1], "matrix": m}),
        json!({"layers": [987_654], "matrix": m}),
        json!({"layers": [u64::MAX], "matrix": m}),
        json!({"layers": (0..10_001).collect::<Vec<u64>>(), "matrix": m}),
        json!({"layers": [paint.0], "layer": paint.0, "matrix": m}),
        json!({"layers": [paint.0], "target": "quickMask", "matrix": m}),
        json!({"layers": [paint.0], "target": {"channel": 0}, "matrix": m}),
    ] {
        assert!(s.execute("edit.transform", p.clone()).is_err(), "{p}");
    }
    assert!(s.active().unwrap().doc == before);
}

#[test]
fn straighten_layer_rotates_every_selected_layer() {
    // Photoshop's Straighten Layer turns all the selected layers, not just the active one; it
    // goes through `edit.transform` without a layer, so it takes the selection like Free Transform.
    let (mut s, paint, text) = two_layers();
    let (p0, m0) = (bounds(&s, paint), text_matrix(&s, text));
    s.execute("image.analysis.rulerTool", json!({"start": [10, 30], "end": [70, 25]})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    let r = s.execute("image.analysis.straightenLayer", json!({})).unwrap();
    assert_eq!(r["cropped"], false);
    assert!(bounds(&s, paint) != p0, "the selected pixel layer turned too");
    assert!(text_matrix(&s, text)[1].abs() > 0.05 && m0[1].abs() < 1e-9, "the active type layer turned");
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one history step");
    assert!(s.undo());
    assert_eq!((bounds(&s, paint), text_matrix(&s, text)), (p0, m0), "and one Undo takes both back");

    // With one layer selected, only that layer turns.
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    s.execute("image.analysis.rulerTool", json!({"start": [10, 30], "end": [70, 25]})).unwrap();
    s.execute("image.analysis.straightenLayer", json!({})).unwrap();
    assert!(bounds(&s, paint) != p0);
    assert_eq!(text_matrix(&s, text), m0, "an unselected layer stays put");
}

#[test]
fn a_recorded_transform_of_the_selected_layers_replays_on_the_selection_in_another_document() {
    let (mut s, paint, text) = two_layers();
    s.execute("actions.record", json!({"name": "Move both"})).unwrap();
    // What the canvas box commits: the ids it opened on, listed (#6501).
    s.execute("edit.transform", json!({"layers": [paint.0, text.0], "matrix": [1, 0, 0, 1, 30, 40]})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let step = s.actions.list[0].steps[0].1.clone();
    assert!(step.get("layers").is_none(), "the step plays on the selection, not on these ids: {step}");
    // Another document, its ids shifted by an extra layer under the two that get selected.
    s.execute("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    let spare = LayerId(s.execute("layer.new.layer", json!({"name": "Spare"})).unwrap()["layer"].as_u64().unwrap());
    let a = LayerId(s.execute("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap());
    let b = LayerId(s.execute("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap());
    assert_ne!((a, b), (paint, text), "the ids differ from the recorded ones");
    s.edit("paint", |doc, _| {
        for (id, x) in [(spare, 100), (a, 10), (b, 60)] {
            doc.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(Rect::new(x, 10, x + 20, 30), &[0.0, 0.0, 1.0, 1.0]);
        }
        Ok(())
    })
    .unwrap();
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    let (a0, b0, spare0) = (bounds(&s, a), bounds(&s, b), bounds(&s, spare));
    let r = s.execute("actions.play", json!({"action": 0})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    assert_eq!((bounds(&s, a), bounds(&s, b)), (a0.translate(30, 40), b0.translate(30, 40)), "the selected layers moved");
    assert_eq!(bounds(&s, spare), spare0, "the unselected layer stayed");
}

#[test]
fn a_recorded_transform_of_other_layers_than_the_selection_keeps_their_ids() {
    let (mut s, paint, _) = two_layers();
    s.execute("actions.record", json!({"name": "Paint only"})).unwrap();
    s.execute("edit.transform", json!({"layers": [paint.0], "matrix": [1, 0, 0, 1, 5, 0]})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    assert_eq!(s.actions.list[0].steps[0].1["layers"], json!([paint.0]));
}
