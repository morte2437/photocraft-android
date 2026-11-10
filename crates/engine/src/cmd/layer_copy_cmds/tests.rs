use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, LayerContent, LayerId, Pattern};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::Session;

/// Document 0 (`w`×`h`, `depth`) with a red 10×10 square at (4, 6) on layer "paint" above the
/// Background, and document 1 (60×50, 8-bit RGB). Document 0 is active.
fn two_docs(w: u32, h: u32, depth: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "depth": depth})).unwrap();
    let paint = LayerId(s.execute("layer.new.layer", json!({"name": "paint"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("select.rect", json!({"x": 4, "y": 6, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("file.new", json!({"width": 60, "height": 50})).unwrap();
    s.set_active(0);
    (s, paint)
}

fn doc(s: &Session, i: usize) -> &Document {
    &s.documents()[i].doc
}

fn copied(r: &Value) -> LayerId {
    LayerId(r["layers"][0].as_u64().unwrap())
}

#[test]
fn selection_only_copy_preserves_source_and_soft_coverage_across_documents() {
    for depth in [8, 16, 32] {
        let (mut s, paint) = two_docs(40, 30, depth);
        s.execute("select.rect", json!({"x": 7, "y": 8, "width": 3, "height": 4})).unwrap();
        s.execute("layer.setProps", json!({"layer": paint.0, "opacity": 0.4})).unwrap();
        std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).selection.as_mut().unwrap().write_pixel(7, 8, &[0.5]);
        let before = doc(&s, 0).clone();
        let r = s.execute("layer.copyToDocument", json!({"source": 0, "document": 1, "layers": [paint.0], "selectionOnly": true, "offset": [10, 5]})).unwrap();
        let id = copied(&r);
        assert_eq!(bounds(&s, 1, id), Rect::new(17, 13, 20, 17));
        assert_eq!(*doc(&s, 0), before);
        let pixel = doc(&s, 1).layer(id).unwrap().surface().unwrap().rgba(17, 13);
        assert!((doc(&s, 1).layer(id).unwrap().opacity - 0.4).abs() < 1e-6);
        assert_eq!(&pixel[..3], &[1.0, 0.0, 0.0]);
        assert!((pixel[3] - 0.5).abs() <= 1.0 / 255.0, "soft selection coverage survives depth conversion");
        assert!(s.undo());
        assert_eq!(doc(&s, 1).layers.len(), 1);
        assert_eq!(*doc(&s, 0), before);
    }
}

#[test]
fn selection_only_copy_rejects_a_floating_layer_mask() {
    let (mut s, paint) = two_docs(40, 30, 8);
    s.edit("mask", |doc, _| {
        doc.layer_mut(paint).unwrap().mask = Some(photocraft_doc::LayerMask::reveal_all());
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 7, "y": 8, "width": 3, "height": 4})).unwrap();
    s.execute("select.float", json!({"dx": 2, "dy": 0, "copy": true, "target": "mask"})).unwrap();
    let source = doc(&s, 0).clone();
    assert!(s.execute("layer.copyToDocument", json!({"source": 0, "document": 1, "layers": [paint.0], "selectionOnly": true})).is_err());
    assert_eq!(*doc(&s, 0), source);
    assert_eq!(doc(&s, 1).layers.len(), 1);
}

#[test]
fn selection_only_copy_rejects_missing_selection_and_wrong_parameter_type() {
    let (mut s, _) = two_docs(40, 30, 8);
    for option in [json!(true), json!("yes")] {
        assert!(s.execute("layer.copyToDocument", json!({"document": 1, "selectionOnly": option})).is_err());
        assert_eq!(doc(&s, 1).layers.len(), 1);
    }
}

fn bounds(s: &Session, i: usize, id: LayerId) -> Rect {
    crate::layer_multi_cmds::layer_bounds(doc(s, i).layer(id).unwrap()).unwrap()
}

#[test]
fn copies_the_selected_layers_above_the_destinations_active_layer_in_one_step() {
    let (mut s, paint) = two_docs(40, 30, 8);
    let source_before = doc(&s, 0).clone();
    let r = s.execute("layer.copyToDocument", json!({"document": 1})).unwrap();
    assert_eq!(r["document"], 1);
    assert_eq!(s.active_index(), Some(1), "the destination becomes the active document");
    let d = doc(&s, 1);
    let id = copied(&r);
    assert_ne!(id, paint, "a copy has its own id");
    assert_eq!(d.layers.len(), 2);
    assert_eq!(d.layers[1].id, id, "above the Background, the destination's active layer");
    assert_eq!(d.layers[1].name, "paint");
    assert_eq!(s.active().unwrap().active_layer, Some(id));
    assert_eq!(bounds(&s, 1, id), Rect::new(4, 6, 14, 16), "same canvas position by default");
    assert_eq!(d.layers[1].surface().unwrap().read_region(Rect::new(5, 7, 6, 8)), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(*doc(&s, 0), source_before, "the source is untouched");
    // One history step in the destination.
    assert!(s.undo());
    assert_eq!(doc(&s, 1).layers.len(), 1);
}

#[test]
fn converts_to_the_destinations_depth_and_colour_mode() {
    for depth in [8, 16, 32] {
        let (mut s, _) = two_docs(40, 30, depth);
        s.set_active(1);
        s.execute("image.mode.grayscale", json!({})).unwrap();
        s.execute("image.mode.bits16", json!({})).unwrap();
        s.set_active(0);
        let r = s.execute("layer.copyToDocument", json!({"document": 1})).unwrap();
        let id = copied(&r);
        let surf = doc(&s, 1).layer(id).unwrap().surface().unwrap();
        assert_eq!(surf.format(), PixelFormat::new(ColorMode::Grayscale, SampleType::U16, true), "from {depth}-bit RGB");
        let px = surf.read_region(Rect::new(5, 7, 6, 8));
        assert!(px[0] > 0.1 && px[0] < 0.9 && px[1] == 1.0, "red as a mid grey, opaque: {px:?}");
    }
}

#[test]
fn places_the_copies_centred_on_the_canvas_on_a_point_or_offset() {
    let (mut s, paint) = two_docs(40, 30, 8);
    let place = |s: &mut Session, p: Value| {
        s.set_active(0);
        let mut p = p;
        p["document"] = json!(1);
        p["layers"] = json!([paint.0]);
        let r = s.execute("layer.copyToDocument", p).unwrap();
        bounds(s, 1, copied(&r))
    };
    assert_eq!(place(&mut s, json!({"center": true})), Rect::new(25, 20, 35, 30));
    assert_eq!(place(&mut s, json!({"at": [10.0, 40.0]})), Rect::new(5, 35, 15, 45));
    assert_eq!(place(&mut s, json!({"offset": [-4, 20]})), Rect::new(0, 26, 10, 36));
    assert_eq!(doc(&s, 1).layers.len(), 4);
}

#[test]
fn a_copied_background_is_an_ordinary_layer_and_several_layers_keep_their_order() {
    let (mut s, paint) = two_docs(40, 30, 8);
    let bg = doc(&s, 0).layers[0].id;
    let r = s.execute("layer.copyToDocument", json!({"document": 1, "layers": [paint.0, bg]})).unwrap();
    let d = doc(&s, 1);
    let names: Vec<&str> = d.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Background", "Layer 1", "paint"]);
    assert!(!d.layers[1].locks.transparency && !d.layers[1].locks.position);
    assert_eq!(r["layers"].as_array().unwrap().len(), 2);
    assert_eq!(s.active().unwrap().selected_layers().len(), 2, "the copies are selected");
}

#[test]
fn the_sources_patterns_come_along_once() {
    let (mut s, _) = two_docs(40, 30, 8);
    let mut tile = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, true));
    tile.fill_rect(Rect::new(0, 0, 2, 2), &[0.0, 1.0, 0.0, 1.0]);
    let pat = Pattern::new("dots", tile, 2, 2);
    s.edit("add pattern", |d, _| {
        d.patterns.push(pat.clone());
        Ok(())
    })
    .unwrap();
    s.execute("layer.copyToDocument", json!({"document": 1})).unwrap();
    assert_eq!(doc(&s, 1).patterns.iter().filter(|p| p.id == pat.id).count(), 1);
    s.set_active(0);
    s.execute("layer.copyToDocument", json!({"document": 1})).unwrap();
    assert_eq!(doc(&s, 1).patterns.iter().filter(|p| p.id == pat.id).count(), 1, "not twice");
}

#[test]
fn bad_params_and_states_fail_without_changing_anything() {
    let (mut s, _) = two_docs(40, 30, 8);
    let cmd = "layer.copyToDocument";
    for p in [
        json!({}),
        json!({"document": 0}),
        json!({"document": 7}),
        json!({"document": -1}),
        json!({"document": "1"}),
        json!({"document": 1, "source": 9}),
        json!({"document": 1, "layers": [u64::MAX]}),
        json!({"document": 1, "layers": "all"}),
        json!({"document": 1, "layers": []}),
        json!({"document": 1, "at": [1.0]}),
        json!({"document": 1, "at": [f64::MAX, 0.0]}),
        json!({"document": 1, "offset": ["a", 2]}),
        Value::Null,
    ] {
        assert!(s.execute(cmd, p.clone()).is_err(), "{p}");
        assert_eq!(s.active_index(), Some(0), "{p}: the source stays active");
        assert_eq!(doc(&s, 1).layers.len(), 1, "{p}");
    }
    // An Indexed destination has no layers to add to.
    s.set_active(1);
    s.edit("indexed", |d, _| {
        d.mode = ColorMode::Indexed;
        Ok(())
    })
    .unwrap();
    s.set_active(0);
    assert!(s.execute(cmd, json!({"document": 1})).is_err());
    // A single open document: nothing to copy to.
    s.close(1);
    assert!(!s.is_enabled(cmd));
}

#[test]
fn copying_linked_layers_does_not_join_an_unrelated_destination_group() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let src_a = LayerId(s.execute("layer.new.layer", json!({"name": "Source A"})).unwrap()["layer"].as_u64().unwrap());
    let src_b = LayerId(s.execute("layer.new.layer", json!({"name": "Source B"})).unwrap()["layer"].as_u64().unwrap());
    s.edit("link source", |d, _| {
        d.layer_mut(src_a).unwrap().link_group = Some(1);
        d.layer_mut(src_b).unwrap().link_group = Some(1);
        Ok(())
    })
    .unwrap();

    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let dst_a = LayerId(s.execute("layer.new.layer", json!({"name": "Destination A"})).unwrap()["layer"].as_u64().unwrap());
    let dst_b = LayerId(s.execute("layer.new.layer", json!({"name": "Destination B"})).unwrap()["layer"].as_u64().unwrap());
    s.edit("link destination", |d, _| {
        d.layer_mut(dst_a).unwrap().link_group = Some(1);
        d.layer_mut(dst_b).unwrap().link_group = Some(1);
        Ok(())
    })
    .unwrap();
    s.set_active(0);
    let result = s.execute("layer.copyToDocument", json!({"document": 1, "layers": [src_a.0, src_b.0]})).unwrap();
    let ids: Vec<LayerId> = result["layers"].as_array().unwrap().iter().map(|id| LayerId(id.as_u64().unwrap())).collect();
    assert_eq!(ids.len(), 2);
    let d = doc(&s, 1);
    assert_eq!(d.layer(dst_a).unwrap().link_group, Some(1));
    assert_eq!(d.layer(dst_b).unwrap().link_group, Some(1));
    let first_group = d.layer(ids[0]).unwrap().link_group;
    assert_eq!(first_group, d.layer(ids[1]).unwrap().link_group, "the copied pair stays linked");
    assert!(first_group.is_some());
    assert_ne!(first_group, Some(1), "copied links must not merge with existing ones");

    assert!(s.undo());
    assert!(doc(&s, 1).layer(ids[0]).is_none());
    assert!(s.redo());
    assert_eq!(doc(&s, 1).layer(ids[0]).unwrap().link_group, first_group);
}

#[test]
fn copied_link_groups_are_distinct_from_each_other_and_destination() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let a = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    let b = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.edit("link independent source groups", |d, _| {
        d.layer_mut(a).unwrap().link_group = Some(1);
        d.layer_mut(b).unwrap().link_group = Some(2);
        Ok(())
    })
    .unwrap();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.set_active(0);
    let copied = s.execute("layer.copyToDocument", json!({"document": 1, "layers": [a.0, b.0]})).unwrap();
    let ids: Vec<LayerId> = copied["layers"].as_array().unwrap().iter().map(|id| LayerId(id.as_u64().unwrap())).collect();
    let first = doc(&s, 1).layer(ids[0]).unwrap().link_group;
    let second = doc(&s, 1).layer(ids[1]).unwrap().link_group;
    assert!(first.is_some() && second.is_some());
    assert_ne!(first, second);
}

/// One painted layer, `depth`-bit RGB, as the only layer above the Background.
fn one_doc(depth: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth})).unwrap();
    let paint = LayerId(s.execute("layer.new.layer", json!({"name": "paint"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("select.rect", json!({"x": 4, "y": 6, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    (s, paint)
}

#[test]
fn duplicate_to_a_new_document_keeps_size_pixels_and_place_at_every_depth() {
    for depth in [8, 16, 32] {
        let (mut s, paint) = one_doc(depth);
        let source_before = doc(&s, 0).clone();
        let r = s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
        assert_eq!(r["document"], 1);
        assert_eq!(s.active_index(), Some(1), "the new document becomes active");
        let (src, dst) = (doc(&s, 0), doc(&s, 1));
        assert_eq!(*src, source_before, "the source is untouched");
        assert_eq!(dst.size, src.size, "same canvas, as Photoshop");
        assert_eq!((dst.mode, dst.depth, dst.resolution_dpi), (src.mode, src.depth, src.resolution_dpi));
        assert_eq!(dst.name, format!("{} copy", src.name));
        let layers = r["layers"].as_array().unwrap();
        assert_eq!(layers.len(), 1);
        let id = LayerId(layers[0].as_u64().unwrap());
        assert_eq!(r["layer"], id.0);
        assert_ne!(id, paint, "a copy has its own id");
        assert_eq!(dst.layers.len(), 1, "just the duplicated layer");
        assert_eq!(dst.layers[0].name, "paint");
        assert_eq!(bounds(&s, 1, id), Rect::new(4, 6, 14, 16), "same canvas position");
        assert_eq!(
            dst.layers[0].surface().unwrap().read_region(Rect::new(4, 6, 14, 16)),
            src.layer(paint).unwrap().surface().unwrap().read_region(Rect::new(4, 6, 14, 16)),
            "depth {depth}: identical pixels"
        );
        assert_eq!(s.active().unwrap().selected_layers(), vec![id]);
    }
}

#[test]
fn duplicate_to_a_new_document_names_the_copy_and_the_document() {
    let (mut s, _) = one_doc(8);
    let r = s.execute("layer.duplicate", json!({"document": "new", "name": "Ink", "documentName": "Card"})).unwrap();
    assert_eq!(doc(&s, 1).name, "Card");
    assert_eq!(doc(&s, 1).layers[0].name, "Ink");
    assert_eq!(r["layer"].as_u64(), r["layers"][0].as_u64());
}

#[test]
fn duplicate_to_a_new_document_carries_every_selected_layer_in_order() {
    let (mut s, paint) = one_doc(8);
    let bg = doc(&s, 0).layers[0].id;
    s.execute("layer.select", json!({"layer": paint.0, "mode": "add"})).unwrap();
    s.execute("layer.select", json!({"layer": bg, "mode": "add"})).unwrap();
    let r = s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
    let names: Vec<&str> = doc(&s, 1).layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Background", "paint"]);
    assert_eq!(r["layers"].as_array().unwrap().len(), 2);
    assert!(r.get("layer").is_none(), "no single layer to report");
    // A duplicated Background stays a Background: the new document has no destination one.
    let back = &doc(&s, 1).layers[0];
    assert!(back.locks.transparency && back.locks.position);
}

#[test]
fn duplicate_to_a_new_document_cuts_smart_objects_loose_as_a_batch() {
    let (mut s, _) = one_doc(8);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let original = s.active().unwrap().active_layer.unwrap();
    let sibling = LayerId(s.execute("layer.duplicate", json!({})).unwrap()["layer"].as_u64().unwrap());
    let contents = |s: &Session, i: usize, id: LayerId| {
        let LayerContent::Smart(sm) = &doc(s, i).layer(id).unwrap().content else { panic!("smart object") };
        sm.contents_id
    };
    assert_eq!(contents(&s, 0, original), contents(&s, 0, sibling), "instances share contents");
    s.execute("layer.select", json!({"layer": original.0, "mode": "add"})).unwrap();
    s.execute("layer.select", json!({"layer": sibling.0, "mode": "add"})).unwrap();
    let r = s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
    let ids: Vec<LayerId> = r["layers"].as_array().unwrap().iter().map(|id| LayerId(id.as_u64().unwrap())).collect();
    assert_eq!(contents(&s, 1, ids[0]), contents(&s, 1, ids[1]), "sharing survives within the batch");
    assert_ne!(contents(&s, 1, ids[0]), contents(&s, 0, original), "independent of the source");
}

#[test]
fn duplicate_destinations_that_are_not_new_fail_cleanly() {
    let (mut s, _) = one_doc(8);
    for p in [
        json!({"document": 1}),
        json!({"document": 7}),
        json!({"document": true}),
        json!({"document": 1.5}),
        json!({"document": []}),
        json!({"document": {}}),
        json!({"document": "other"}),
        json!({"document": "new", "layer": u64::MAX}),
    ] {
        let before = (doc(&s, 0).clone(), s.documents().len(), s.active_index());
        let err = s.execute("layer.duplicate", p.clone()).unwrap_err().to_string();
        if p == json!({"document": 1}) {
            assert!(err.contains("layer.copyToDocument"), "{p}: {err}");
        }
        assert_eq!(s.active_index(), before.2, "{p}: the source stays active");
        assert_eq!(s.documents().len(), before.1, "{p}: no document created");
        assert_eq!(*doc(&s, 0), before.0, "{p}: the source is untouched");
    }
    // One open document: same-document duplicates still work, and stay enabled.
    assert!(s.is_enabled("layer.duplicate"));
    s.execute("layer.duplicate", json!({})).unwrap();
}

#[test]
fn a_named_duplicate_takes_the_name_in_one_step() {
    let (mut s, paint) = two_docs(40, 30, 8);
    let before = s.active().unwrap().history.past_len();
    let same = LayerId(s.execute("layer.duplicate", json!({"name": "Ink"})).unwrap()["layer"].as_u64().unwrap());
    assert_eq!(doc(&s, 0).layer(same).unwrap().name, "Ink");
    assert_eq!(s.active().unwrap().history.past_len(), before + 1);
    s.undo();
    s.execute("layer.select", json!({"layer": paint.0})).unwrap();
    let r = s.execute("layer.copyToDocument", json!({"document": 1, "name": "Ink"})).unwrap();
    assert_eq!(doc(&s, 1).layer(copied(&r)).unwrap().name, "Ink");
}

#[test]
fn a_copied_group_cannot_nest_the_destination_past_the_group_depth_limit() {
    use photocraft_doc::{Layer, MAX_GROUP_DEPTH};
    let (mut s, paint) = two_docs(40, 30, 8);
    // A group in the source, and a destination whose active layer sits at the deepest legal level.
    s.execute("layer.groupLayers", json!({"layers": [paint.0]})).unwrap();
    let group = s.active().unwrap().active_layer.unwrap();
    s.set_active(1);
    let mut inner = None;
    s.edit("nest", |doc, active| {
        let mut l = Layer::new("deep", LayerContent::Raster(Surface::new(PixelFormat::RGBA8)));
        inner = Some(l.id);
        for _ in 0..MAX_GROUP_DEPTH {
            l = Layer::group("g", vec![l]);
        }
        doc.layers.push(l);
        *active = inner;
        Ok(())
    })
    .unwrap();
    s.set_active(0);
    let before = doc(&s, 1).clone();
    let e = s.execute("layer.copyToDocument", json!({"document": 1, "layers": [group.0]})).unwrap_err();
    assert!(e.to_string().contains("deeper than"), "{e}");
    assert_eq!(*doc(&s, 1), before, "nothing was copied");
}

/// The type layer `id` of document `i`.
fn text(s: &Session, i: usize, id: LayerId) -> &photocraft_doc::TextLayer {
    match &doc(s, i).layer(id).unwrap().content {
        LayerContent::Text(t) => t,
        other => panic!("not type: {other:?}"),
    }
}

/// Bounds and summed alpha of a layer's rendered pixels.
fn ink(s: &Session, i: usize, id: LayerId) -> (Rect, f64) {
    let surf = doc(s, i).layer(id).unwrap().surface().unwrap();
    let b = surf.content_bounds();
    let a = surf.read_region(b).chunks(surf.channels()).map(|px| f64::from(*px.last().unwrap())).sum();
    (b, a)
}

/// Document 0 at `from` ppi with a 24 pt "H" (leading 30 pt, baseline shift 2 pt, first-line
/// indent 6 pt) and document 1 at `to` ppi; the type layer is selected in document 0.
fn type_docs(from: u32, to: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": from})).unwrap();
    let t = LayerId(
        s.execute("type.create", json!({"text": "H", "size": 24, "leading": 30, "baselineShift": 2, "x": 128, "y": 100})).unwrap()["layer"].as_u64().unwrap(),
    );
    s.execute("type.setStyle", json!({"layer": t.0, "firstLineIndent": 6})).unwrap();
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": to})).unwrap();
    s.set_active(0);
    s.execute("layer.select", json!({"layer": t.0})).unwrap();
    (s, t)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-4 * b.abs().max(1.0)
}

/// Checks a copy of `src` (document 0) as `dst` (document 1) at `pt` points (the reference
/// application's rewritten size for 24 pt), then that an edit keeps that size and the pixels.
fn check_rewritten(s: &mut Session, src: LayerId, dst: LayerId, pt: f32, label: &str) {
    let k = pt / 24.0;
    let (from, to) = (text(s, 0, src), text(s, 1, dst));
    assert_eq!(to.transform, from.transform, "{label}: no compensating transform");
    assert!(close(to.size_pt, pt), "{label}: {} pt", to.size_pt);
    let run = &to.runs[0].style;
    assert!(close(run.size_pt, pt), "{label}: run {} pt", run.size_pt);
    assert!(close(run.leading_pt.unwrap(), 30.0 * k), "{label}: leading {:?}", run.leading_pt);
    assert!(close(run.baseline_shift_pt, 2.0 * k), "{label}: baseline shift {}", run.baseline_shift_pt);
    assert_eq!(run.tracking, from.runs[0].style.tracking, "{label}: tracking is in 1/1000 em");
    assert!(close(to.paragraphs[0].style.first_line_indent_pt, 6.0 * k), "{label}: indent {}", to.paragraphs[0].style.first_line_indent_pt);
    let (b0, a0) = ink(s, 0, src);
    assert_eq!(ink(s, 1, dst).0, b0, "{label}: the copy's pixels");
    // Re-laid out at the destination's resolution, the copy renders as the source does.
    s.execute("type.edit", json!({"layer": dst.0, "text": "M"})).unwrap();
    s.execute("type.edit", json!({"layer": dst.0, "text": "H"})).unwrap();
    assert!(close(text(s, 1, dst).size_pt, pt), "{label}: an edit keeps {} pt", text(s, 1, dst).size_pt);
    let (b1, a1) = ink(s, 1, dst);
    assert_eq!(b1, b0, "{label}: after an edit, {b1:?} vs {b0:?}");
    assert!((a1 - a0).abs() <= a0 * 0.005, "{label}: coverage {a1} vs {a0}");
}

/// #9560: a type layer copied between documents of different resolutions keeps its pixels and
/// takes the destination's point size, as in the reference application (measured: 24 pt from
/// 72 ppi is 5.76 pt at 300 ppi, from 300 ppi 100 pt at 72 ppi, with no transform; later edits
/// keep that size). It used to keep 24 pt and re-lay out about 4.2 times larger or smaller on
/// its next edit, and then kept 24 pt behind a compensating transform.
#[test]
fn copied_type_takes_the_destinations_point_size_and_keeps_its_pixels() {
    for (from, to, pt) in [(72, 300, 5.76), (300, 72, 100.0)] {
        let (mut s, t) = type_docs(from, to);
        let id = copied(&s.execute("layer.copyToDocument", json!({"document": 1})).unwrap());
        check_rewritten(&mut s, t, id, pt, &format!("copy {from}→{to}"));
    }
}

/// The same through Copy and Paste in Place of the whole type layer.
#[test]
fn pasted_type_takes_the_destinations_point_size_and_keeps_its_pixels() {
    for (from, to, pt) in [(72, 300, 5.76), (300, 72, 100.0)] {
        let (mut s, t) = type_docs(from, to);
        s.execute("edit.copy", json!({})).unwrap();
        s.set_active(1);
        let id = LayerId(s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap()["layer"].as_u64().unwrap());
        check_rewritten(&mut s, t, id, pt, &format!("paste {from}→{to}"));
    }
}

/// Paragraph text keeps its box (in pixels) and its wrapping: only the point sizes change.
#[test]
fn copied_paragraph_type_keeps_its_box() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": 72})).unwrap();
    let t =
        LayerId(s.execute("type.create", json!({"text": "Hello wrapping world", "size": 24, "box": [20, 20, 120, 150]})).unwrap()["layer"].as_u64().unwrap());
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": 300})).unwrap();
    s.set_active(0);
    s.execute("layer.select", json!({"layer": t.0})).unwrap();
    let id = copied(&s.execute("layer.copyToDocument", json!({"document": 1})).unwrap());
    assert_eq!(text(&s, 1, id).shape, text(&s, 0, t).shape);
    assert!(close(text(&s, 1, id).size_pt, 5.76));
    let (b0, _) = ink(&s, 0, t);
    s.execute("type.edit", json!({"layer": id.0, "text": "Hello wrapping world"})).unwrap();
    assert_eq!(ink(&s, 1, id).0, b0, "the same lines after a re-layout");
}

/// Equal resolutions change nothing; one that isn't a positive number lays out at 72 ppi, so it
/// is treated as 72 rather than producing an infinite or zero size.
#[test]
fn retargeting_type_survives_bad_resolutions() {
    let mut d = Document::new("", photocraft_geom::Size::new(10, 10), ColorMode::Rgb, SampleType::U8);
    d.layers.push(photocraft_doc::Layer::new("t", LayerContent::Text(photocraft_doc::TextLayer { text: "H".into(), size_pt: 24.0, ..Default::default() })));
    let size = |d: &Document| match &d.layers[0].content {
        LayerContent::Text(t) => t.size_pt,
        _ => panic!("not type"),
    };
    d.resolution_dpi = 72.0;
    crate::canvas_geom::retarget_type_resolution(&mut d, 72.0);
    assert_eq!(size(&d), 24.0);
    for bad in [0.0, -5.0, f32::NAN, f32::INFINITY] {
        crate::canvas_geom::retarget_type_resolution(&mut d, bad);
        assert_eq!(size(&d), 24.0, "{bad}");
        d.resolution_dpi = bad;
        crate::canvas_geom::retarget_type_resolution(&mut d, 72.0);
        assert_eq!(size(&d), 24.0, "from {bad}");
    }
    // Absurd ratios stay finite and positive.
    d.resolution_dpi = f32::MAX;
    crate::canvas_geom::retarget_type_resolution(&mut d, f32::MIN_POSITIVE);
    assert!(size(&d).is_finite() && size(&d) > 0.0, "{}", size(&d));
    crate::canvas_geom::retarget_type_resolution(&mut d, f32::MAX);
    assert!(size(&d).is_finite() && size(&d) > 0.0, "{}", size(&d));
}

/// Document 0 at 72 ppi with "Hello" in Paragraph Style 1 (24 pt) and "He" also in Character
/// Style 1 (24 pt, underlined); document 1 at 300 ppi, its own styles made by `dest`.
fn styled_type_docs(dest: impl FnOnce(&mut Session)) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": 72})).unwrap();
    let t = s.execute("type.create", json!({"text": "Hello", "size": 12, "x": 100, "y": 100})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.paragraphStyle.new", json!({"attrs": {"size": 24}, "fromSelection": false, "apply": true, "layer": t})).unwrap();
    s.execute("type.characterStyle.new", json!({"attrs": {"size": 24, "underline": true}, "fromSelection": false, "apply": true, "layer": t, "range": [0, 2]}))
        .unwrap();
    s.execute("file.new", json!({"width": 400, "height": 200, "resolution": 300})).unwrap();
    dest(&mut s);
    s.set_active(0);
    s.execute("layer.select", json!({"layer": t})).unwrap();
    (s, LayerId(t))
}

/// The styles panel state of `range` in layer `id` of document 1.
fn styles_of(s: &mut Session, id: LayerId, range: [usize; 2]) -> (Value, Value) {
    s.set_active(1);
    let c = s.execute("type.characterStyle.list", json!({"layer": id.0, "range": range})).unwrap();
    let p = s.execute("type.paragraphStyle.list", json!({"layer": id.0, "range": range})).unwrap();
    (c, p)
}

/// #10500: type copied to a document of another resolution brings the styles it uses, as in
/// the reference application (measured with Duplicate Layer from 72 to 300 ppi): a style the
/// destination lacks arrives under its name, scaled like the text (24 pt → 5.76 pt), so the
/// copy has no override; it never takes over an unrelated destination style with the same id.
#[test]
fn copied_type_brings_its_styles_scaled() {
    for paste in [false, true] {
        let (mut s, _) = styled_type_docs(|s| {
            s.execute("type.characterStyle.new", json!({"name": "Unrelated", "attrs": {"size": 50}, "fromSelection": false})).unwrap();
            s.execute("type.paragraphStyle.new", json!({"name": "Unrelated", "attrs": {"size": 50}, "fromSelection": false})).unwrap();
        });
        let id = if paste {
            s.execute("edit.copy", json!({})).unwrap();
            s.set_active(1);
            LayerId(s.execute("edit.pasteSpecial.pasteInPlace", json!({})).unwrap()["layer"].as_u64().unwrap())
        } else {
            copied(&s.execute("layer.copyToDocument", json!({"document": 1})).unwrap())
        };
        assert!(close(text(&s, 1, id).runs[0].style.size_pt, 5.76), "paste {paste}");
        let (c, p) = styles_of(&mut s, id, [0, 2]);
        let names = |v: &Value| v["styles"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(names(&c), ["None", "Unrelated", "Character Style 1"], "paste {paste}");
        assert_eq!(names(&p), ["Basic Paragraph", "Unrelated", "Paragraph Style 1"], "paste {paste}");
        let (cs, ps) = (&c["styles"][2], &p["styles"][2]);
        assert!(close(cs["attrs"]["size_pt"].as_f64().unwrap() as f32, 5.76), "{cs}");
        assert!(close(ps["characterAttrs"]["size_pt"].as_f64().unwrap() as f32, 5.76), "{ps}");
        assert_eq!((&c["current"]["character"], &c["current"]["characterOverride"]), (&cs["id"], &json!(false)), "paste {paste}");
        assert_eq!((&p["current"]["paragraph"], &p["current"]["paragraphOverride"]), (&ps["id"], &json!(false)), "paste {paste}");
        // The source keeps its 24 pt styles.
        assert_eq!(doc(&s, 0).text_styles.character[0].attrs["size_pt"], json!(24.0));
    }
}

/// A destination that already has a style of the same name keeps its definition (24 pt), and
/// the copy, at 5.76 pt, shows a local override ("+"), as in the reference application.
#[test]
fn copied_type_uses_the_destinations_same_named_style() {
    let (mut s, _) = styled_type_docs(|s| {
        s.execute("type.characterStyle.new", json!({"name": "Filler", "fromSelection": false})).unwrap();
        s.execute("type.characterStyle.new", json!({"name": "Character Style 1", "attrs": {"size": 24, "underline": true}, "fromSelection": false})).unwrap();
    });
    let id = copied(&s.execute("layer.copyToDocument", json!({"document": 1})).unwrap());
    let (c, _) = styles_of(&mut s, id, [0, 2]);
    assert_eq!(c["styles"].as_array().unwrap().len(), 3, "no second Character Style 1: {c}");
    let existing = &c["styles"][2];
    assert_eq!(existing["attrs"]["size_pt"], json!(24.0));
    assert_eq!((&c["current"]["character"], &c["current"]["characterOverride"]), (&existing["id"], &json!(true)));
    assert!(close(text(&s, 1, id).runs[0].style.size_pt, 5.76));
}

/// Duplicate Layer › New Document brings the styles the type uses (at the same size).
#[test]
fn type_duplicated_to_a_new_document_keeps_its_styles() {
    let (mut s, t) = styled_type_docs(|_| {});
    let r = s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
    let (i, id) = (r["document"].as_u64().unwrap() as usize, LayerId(r["layer"].as_u64().unwrap()));
    s.set_active(i);
    let c = s.execute("type.characterStyle.list", json!({"layer": id.0, "range": [0, 2]})).unwrap();
    let p = s.execute("type.paragraphStyle.list", json!({"layer": id.0, "range": [0, 2]})).unwrap();
    assert_eq!(c["styles"][1]["name"], "Character Style 1");
    assert_eq!(c["styles"][1]["attrs"]["size_pt"], json!(24.0));
    assert_eq!((&c["current"]["character"], &c["current"]["characterOverride"]), (&c["styles"][1]["id"], &json!(false)));
    assert_eq!((&p["current"]["paragraph"], &p["current"]["paragraphOverride"]), (&p["styles"][1]["id"], &json!(false)));
    assert_eq!(text(&s, i, id).runs, text(&s, 0, t).runs);
}

/// Document 0 (RGB, 72 ppi) with "Hello" whose "He" is in Character Style 1 (red); document 1
/// (CMYK, 72 ppi). The type layer is selected in document 0.
fn red_styled_type_docs() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100, "resolution": 72})).unwrap();
    let t = s.execute("type.create", json!({"text": "Hello", "size": 12, "x": 20, "y": 50})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.characterStyle.new", json!({"attrs": {"color": "#ff0000"}, "fromSelection": false, "apply": true, "layer": t, "range": [0, 2]})).unwrap();
    s.execute("file.new", json!({"width": 200, "height": 100, "resolution": 72, "mode": "cmyk"})).unwrap();
    s.set_active(0);
    s.execute("layer.select", json!({"layer": t})).unwrap();
    (s, LayerId(t))
}

/// The colour Character Style 1 of document `i` sets, and whether "He" of `id` overrides it.
fn style_color(s: &mut Session, i: usize, id: LayerId) -> (photocraft_color::Color, bool) {
    s.set_active(i);
    let c = s.execute("type.characterStyle.list", json!({"layer": id.0, "range": [0, 2]})).unwrap();
    let style = c["styles"].as_array().unwrap().iter().find(|x| x["name"] == "Character Style 1").unwrap();
    (serde_json::from_value(style["attrs"]["color"].clone()).unwrap(), c["current"]["characterOverride"].as_bool().unwrap())
}

/// Type copied to a document of another colour mode converts its run colours, and the style it
/// brings converts with the same transform: the run matches its style (no override), so a later
/// change to the style's colour still reaches it. Converting the document itself (Image › Mode,
/// Convert to Profile with and without dither) keeps its styles in step the same way.
#[test]
fn copied_type_styles_convert_their_colours_with_the_type() {
    let (mut s, _) = red_styled_type_docs();
    let id = copied(&s.execute("layer.copyToDocument", json!({"document": 1})).unwrap());
    let run = text(&s, 1, id).runs[0].style.color;
    assert_eq!(run.mode, ColorMode::Cmyk, "the run converted");
    let (style, over) = style_color(&mut s, 1, id);
    assert_eq!(style, run, "the copied style converted with the run");
    assert!(!over, "no false override");
    for (cmd, p) in [
        ("image.mode.cmyk", json!({})),
        ("edit.convertToProfile", json!({"profile": "coated-cmyk", "dither": false})),
        ("edit.convertToProfile", json!({"profile": "coated-cmyk", "dither": true})),
        ("edit.convertToProfile", json!({"profile": "lab-d50"})),
    ] {
        let (mut s, t) = red_styled_type_docs();
        s.execute(cmd, p.clone()).unwrap();
        let run = text(&s, 0, t).runs[0].style.color;
        assert_ne!(run.mode, ColorMode::Rgb, "{cmd} {p}: the run converted");
        let (style, over) = style_color(&mut s, 0, t);
        assert_eq!(style, run, "{cmd} {p}: the style converted with the run");
        assert!(!over, "{cmd} {p}: no false override");
    }
}

/// Duplicate Layer › New Document keeps the source's Basic Paragraph: type that inherits its
/// size from it has no override there, and Clear Override leaves it as it was.
#[test]
fn type_duplicated_to_a_new_document_keeps_the_sources_basic_paragraph() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let t = LayerId(s.execute("type.create", json!({"text": "Hello", "size": 12, "x": 20, "y": 50})).unwrap()["layer"].as_u64().unwrap());
    s.execute("type.paragraphStyle.set", json!({"id": 0, "attrs": {"size": 30}})).unwrap();
    assert!(close(text(&s, 0, t).runs[0].style.size_pt, 30.0), "the type inherits Basic Paragraph");
    let basic = doc(&s, 0).text_styles.basic.clone();
    s.execute("layer.select", json!({"layer": t.0})).unwrap();
    let r = s.execute("layer.duplicate", json!({"document": "new"})).unwrap();
    let (i, id) = (r["document"].as_u64().unwrap() as usize, LayerId(r["layer"].as_u64().unwrap()));
    assert_eq!(doc(&s, i).text_styles.basic, basic);
    s.set_active(i);
    let c = s.execute("type.characterStyle.list", json!({"layer": id.0})).unwrap();
    let p = s.execute("type.paragraphStyle.list", json!({"layer": id.0})).unwrap();
    assert_eq!((&c["current"]["characterOverride"], &p["current"]["paragraphOverride"]), (&json!(false), &json!(false)), "{c} {p}");
    s.execute("type.paragraphStyle.clearOverride", json!({"layer": id.0})).unwrap();
    s.execute("type.characterStyle.clearOverride", json!({"layer": id.0})).unwrap();
    assert!(close(text(&s, i, id).runs[0].style.size_pt, 30.0), "{}", text(&s, i, id).runs[0].style.size_pt);
}
