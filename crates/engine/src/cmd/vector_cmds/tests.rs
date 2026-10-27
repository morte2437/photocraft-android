use serde_json::json;

use super::*;

#[test]
fn stroke_validation_rejects_hostile_values_without_editing_history() {
    let mut s = session(100, 80, 8);
    let id = s.execute("shape.create", json!({"rect": [20, 20, 50, 30], "stroke": {"width": 4}})).unwrap()["layer"].as_u64().unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    let invalid = [
        json!(true),
        json!([]),
        json!({"width": -1}),
        json!({"width": 1e100}),
        json!({"opacity": 101}),
        json!({"cap": "invalid"}),
        json!({"cap": 2}),
        json!({"join": "invalid"}),
        json!({"align": "invalid"}),
        json!({"miterLimit": 0}),
        json!({"miterLimit": 501}),
        json!({"dashes": [4, "x"]}),
        json!({"dashes": [-1, 2]}),
        json!({"dashes": [0, 0]}),
        json!({"dashes": vec![1; 33]}),
        json!({"dashOffset": 1e100}),
        json!({"width": "4"}),
        json!({"width": 0.00001, "dashes": [0.01, 0.01]}),
        json!({"color": {"gradient": {"offset": [0.5]}}}),
        json!({"color": {"gradient": {"offset": "centre"}}}),
        // Stop lists past the gradient stop edits' 1024 cap are refused, not cut.
        json!({"color": {"gradient": {"stops": vec![json!([0.5, "#ff0000"]); 1025]}}}),
        json!({"color": {"gradient": {"opacityStops": vec![json!([0.5, 1.0]); 1025]}}}),
        json!({"color": {"gradient": {"midpoints": vec![0.5; 1025]}}}),
        // A stop list that is not an array is an error, not the default black-to-white.
        json!({"color": {"gradient": {"stops": "red"}}}),
        json!({"color": {"gradient": {"opacityStops": {}}}}),
        json!({"color": {"gradient": {"midpoints": 1}}}),
    ];
    for stroke in invalid {
        assert!(s.execute("shape.edit", json!({"layer": id, "stroke": stroke})).is_err(), "{stroke}");
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
        assert!(s.execute("shape.create", json!({"rect": [20, 20, 50, 30], "stroke": stroke})).is_err(), "{stroke}");
        assert_eq!(s.active().unwrap().doc, before, "{stroke}: a rejected create adds no layer");
        assert_eq!(s.active().unwrap().history.past_len(), history, "{stroke}: nor a history state");
    }
    assert!(s.execute("shape.edit", json!({"layer": id, "rect": [1e100, 0, 20, 20]})).is_err());
    assert_eq!(s.active().unwrap().doc, before);
    // The cap itself is accepted.
    let at_cap = json!({"color": {"gradient": {"opacityStops": vec![json!([0.5, 1.0]); 1024], "midpoints": vec![0.5; 1024]}}});
    s.execute("shape.edit", json!({"layer": id, "stroke": at_cap})).unwrap();
}

/// A gradient stop given as a stored `Color` keeps its colour model: a CMYK stop stays CMYK.
#[test]
fn gradient_stops_take_a_stored_color_and_keep_its_model() {
    let mut s = session(100, 80, 8);
    let id = s.execute("shape.create", json!({"rect": [20, 20, 50, 30], "stroke": {"width": 6}})).unwrap()["layer"].as_u64().unwrap();
    let cmyk = Color { mode: photocraft_doc::ColorMode::Cmyk, c: [0.1, 0.2, 0.3, 0.9], alpha: 0.75 };
    let stops = json!([[0.0, cmyk], [1.0, "#ffffff"]]);
    s.execute("shape.edit", json!({"layer": id, "stroke": {"color": {"gradient": {"stops": stops}}}})).unwrap();
    let doc = &s.active().unwrap().doc;
    let LayerContent::Shape(sh) = &doc.layer(LayerId(id)).unwrap().content else { panic!("a shape layer") };
    let Fill::Gradient { stops, .. } = &sh.stroke.as_ref().unwrap().paint else { panic!("a gradient stroke") };
    assert_eq!(stops[0], (0.0, cmyk));
    assert_eq!(stops[1], (1.0, Color::WHITE), "hex stops still parse as before");
    // A stored colour that isn't finite or has alpha past 1 is not a colour: the stop is dropped,
    // as an unparseable hex stop is.
    let bad = json!({"mode": "Cmyk", "c": [0.1, 0.2, 0.3, 0.9], "alpha": 2.0});
    s.execute("shape.edit", json!({"layer": id, "stroke": {"color": {"gradient": {"stops": [[0.0, bad], [1.0, "#000000"]]}}}})).unwrap();
    let doc = &s.active().unwrap().doc;
    let LayerContent::Shape(sh) = &doc.layer(LayerId(id)).unwrap().content else { panic!("a shape layer") };
    let Fill::Gradient { stops, .. } = &sh.stroke.as_ref().unwrap().paint else { panic!("a gradient stroke") };
    assert_eq!(stops.as_slice(), [(1.0, Color::BLACK)]);
}

/// A gradient stroke keeps every gradient field through `shape.edit`, and `shape.info` reports
/// them, so the shape swatches can change one setting without dropping the rest.
#[test]
fn gradient_stroke_round_trips_every_gradient_field() {
    let mut s = session(100, 80, 16);
    let id = s.execute("shape.create", json!({"rect": [20, 20, 50, 30], "stroke": {"width": 6}})).unwrap()["layer"].as_u64().unwrap();
    let gradient = json!({"gradient": {
        "stops": [[0.0, [0.25, 0.5, 0.75, 1.0]], [1.0, [1.0, 0.125, 0.0, 0.5]]],
        "angle": 30.0, "scale": 50.0, "style": "radial", "reverse": true,
        "opacityStops": [[1.0, 0.25], [0.0, 1.0]], "midpoints": [0.25], "offset": [0.125, -0.25], "dither": true, "align": false,
    }});
    let r = s.execute("shape.edit", json!({"layer": id, "stroke": {"color": gradient}})).unwrap();
    let Some(Fill::Gradient { stops, angle, scale, style, reverse, opacity_stops, midpoints, offset, dither, align }) = shape(&s, id).stroke.map(|st| st.paint)
    else {
        panic!("a gradient stroke")
    };
    assert_eq!(stops, vec![(0.0, Color::rgba(0.25, 0.5, 0.75, 1.0)), (1.0, Color::rgba(1.0, 0.125, 0.0, 0.5))]);
    assert_eq!((angle, scale, style, reverse), (30.0, 0.5, GradientStyle::Radial, true));
    assert_eq!(opacity_stops, vec![(0.0, 1.0), (1.0, 0.25)], "opacity stops are sorted");
    assert_eq!((midpoints, offset, dither, align), (vec![0.25], (0.125, -0.25), true, false));
    let g = &r["stroke"]["fill"]["gradient"];
    assert_eq!((g["style"].as_str(), g["midpoints"].clone(), g["align"].as_bool()), (Some("radial"), json!([0.25]), Some(false)));
    // A partial gradient keeps the defaults it always had.
    s.execute("shape.edit", json!({"layer": id, "stroke": {"color": {"gradient": {"stops": [[0, "#000000"], [1, "#ffffff"]]}}}})).unwrap();
    let Some(Fill::Gradient { opacity_stops, midpoints, offset, dither, align, .. }) = shape(&s, id).stroke.map(|st| st.paint) else {
        panic!("a gradient stroke")
    };
    assert_eq!((opacity_stops, midpoints, offset, dither, align), (Vec::new(), Vec::new(), (0.0, 0.0), false, true));
}

#[test]
fn complete_stroke_edits_preserve_paint_and_geometry_and_undo_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = session(100, 80, depth);
        let id = s
            .execute("shape.create", json!({"kind": "polygon", "sides": 3, "rect": [20, 20, 50, 30], "stroke": {"width": 4, "color": "#1122dd"}}))
            .unwrap()["layer"]
            .as_u64()
            .unwrap();
        let original = shape(&s, id);
        let history = s.active().unwrap().history.past_len();
        let patch = json!({"align":"outside", "cap":"square", "join":"bevel", "miterLimit":2, "opacity":35, "dashes":[4,2,1,3], "dashOffset":-0.5});
        s.execute("shape.edit", json!({"layer": id, "stroke": patch})).unwrap();
        let changed = shape(&s, id);
        assert_eq!(changed.path, original.path);
        assert_eq!(changed.stroke.as_ref().unwrap().paint, original.stroke.as_ref().unwrap().paint);
        assert_eq!(changed.stroke.as_ref().unwrap().dashes, [4.0, 2.0, 1.0, 3.0]);
        assert_ne!(changed.cache, original.cache);
        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(shape(&s, id), original);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(shape(&s, id), changed);
    }
}

#[test]
fn stroke_edits_respect_shape_locks() {
    let mut s = session(80, 60, 8);
    let id = s.execute("shape.create", json!({"rect":[10,10,40,30], "stroke":{"width":3}})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.lockLayers", json!({"all":true})).unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute("shape.edit", json!({"layer":id,"stroke":{"dashes":[4,2]}})).is_err());
    assert_eq!(s.active().unwrap().doc, before);
}

#[test]
fn adding_components_validates_the_existing_strokes_combined_workload() {
    let mut s = session(80, 60, 8);
    let id = s.execute("shape.create", json!({"rect":[0,0,1,1], "stroke":{"width":0.001,"dashes":[1,1]}})).unwrap()["layer"].as_u64().unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    assert!(s.execute("shape.create", json!({"addTo":id,"rect":[20,20,100,80]})).is_err());
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), history);
}

fn session(w: u32, h: u32, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "white", "depth": depth})).unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

fn shape(s: &Session, id: u64) -> ShapeLayer {
    match &doc(s).layer(LayerId(id)).unwrap().content {
        LayerContent::Shape(sh) => sh.clone(),
        other => panic!("not a shape: {}", other.kind_name()),
    }
}

fn alpha_sum(sh: &ShapeLayer) -> f64 {
    let c = sh.cache.as_ref().unwrap();
    let r = c.content_bounds();
    let fmt = c.format();
    c.read_region(r).chunks_exact(fmt.channels()).map(|p| f64::from(photocraft_raster::to_rgba(&fmt, p)[3])).sum()
}

fn selection_mask(s: &Session) -> Vec<f32> {
    let d = doc(s);
    sel::mask_from_surface(d.selection.as_ref(), d.bounds())
}

#[test]
fn shape_create_every_kind_and_undo() {
    for depth in [8, 16, 32] {
        let mut s = session(200, 150, depth);
        let cases = [
            (json!({"kind": "rect", "rect": [10, 10, 50, 40]}), 2000.0),
            (json!({"kind": "roundedRect", "rect": [10, 10, 50, 40], "radii": 10}), 2000.0 - (4.0 - std::f64::consts::PI) * 100.0),
            (json!({"kind": "ellipse", "rect": [20, 20, 100, 60]}), std::f64::consts::PI * 50.0 * 30.0),
            (json!({"kind": "polygon", "rect": [0, 0, 100, 100], "sides": 6}), 1.5 * 3f64.sqrt() * 2500.0),
            (json!({"kind": "star", "rect": [0, 0, 100, 100], "sides": 5, "starRatio": 0.5}), 0.0),
            (json!({"kind": "line", "from": [10, 100], "to": [110, 100], "weight": 4}), 400.0),
            (json!({"kind": "path", "path": {"subpaths": [{"closed": true, "knots": [[0, 0], [60, 0], [0, 60]]}]}}), 1800.0),
        ];
        let before = doc(&s).layer_count();
        for (i, (p, area)) in cases.iter().enumerate() {
            let r = s.execute("shape.create", p.clone()).unwrap();
            let id = r["layer"].as_u64().unwrap();
            let sh = shape(&s, id);
            assert_eq!(sh.cache.as_ref().unwrap().format(), doc(&s).pixel_format());
            let a = alpha_sum(&sh);
            if *area > 0.0 {
                assert!((a - area).abs() / area < 0.01, "case {i} depth {depth}: {a} vs {area}");
            } else {
                assert!(a > 1000.0);
            }
            assert_eq!(r["kind"], p["kind"], "{r}");
            assert_eq!(s.active().unwrap().active_layer, Some(LayerId(id)));
        }
        assert_eq!(doc(&s).layer_count(), before + cases.len());
        for _ in 0..cases.len() {
            assert!(s.undo());
        }
        assert_eq!(doc(&s).layer_count(), before);
        assert!(s.redo());
        assert_eq!(doc(&s).layer_count(), before + 1);
    }
}

#[test]
fn shape_create_fill_stroke_and_composite() {
    let mut s = session(100, 100, 8);
    let r = s
        .execute(
            "shape.create",
            json!({"kind": "rect", "rect": [20, 20, 60, 60], "fill": "#ff0000", "stroke": {"width": 4, "color": "#0000ff", "align": "outside"}}),
        )
        .unwrap();
    assert_eq!(r["fill"], "#ff0000");
    assert_eq!(r["stroke"]["align"], "outside");
    assert_eq!(r["bounds"], json!([16, 16, 68, 68]));
    let f = photocraft_compose::flatten(doc(&s));
    let px = |x: i32, y: i32| f.get(x, y);
    assert!(px(50, 50)[0] > 0.99 && px(50, 50)[2] < 0.01);
    assert!(px(18, 50)[2] > 0.99 && px(18, 50)[0] < 0.01);
    assert!(px(10, 10)[1] > 0.99);
    // No fill: only the stroke.
    let r = s.execute("shape.create", json!({"kind": "ellipse", "rect": [5, 5, 40, 40], "fill": null, "stroke": {"width": 2}})).unwrap();
    let sh = shape(&s, r["layer"].as_u64().unwrap());
    assert!(sh.fill.is_none());
    assert!((alpha_sum(&sh) - std::f64::consts::PI * 40.0 * 2.0).abs() < 3.0, "{}", alpha_sum(&sh));
    assert!(s.execute("shape.create", json!({"kind": "rect"})).is_err());
    assert!(s.execute("shape.create", json!({"kind": "blob", "rect": [0, 0, 1, 1]})).is_err());
}

#[test]
fn shape_edit_live_params_and_undo() {
    let mut s = session(200, 200, 8);
    let id = s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 50, 50]})).unwrap()["layer"].as_u64().unwrap();
    let r = s.execute("shape.edit", json!({"layer": id, "rect": [10, 10, 100, 50]})).unwrap();
    assert_eq!(r["bounds"], json!([10, 10, 100, 50]));
    assert!((alpha_sum(&shape(&s, id)) - 5000.0).abs() < 1.0);
    // Radii turn it into a rounded rectangle.
    let r = s.execute("shape.edit", json!({"layer": id, "radii": [20, 0, 20, 0]})).unwrap();
    assert_eq!(r["kind"], "roundedRect");
    assert_eq!(r["live"]["radii"], json!([20.0, 0.0, 20.0, 0.0]));
    // Move keeps the live shape; a rotation drops it.
    let r = s.execute("shape.edit", json!({"layer": id, "move": [5, 7]})).unwrap();
    assert_eq!(r["live"]["rect"], json!([15.0, 17.0, 100.0, 50.0]));
    let r = s.execute("shape.edit", json!({"layer": id, "transform": [0.0, 1.0, -1.0, 0.0, 150.0, 0.0]})).unwrap();
    assert_eq!(r["kind"], "path");
    // Fill/stroke edits merge.
    s.execute("shape.edit", json!({"layer": id, "stroke": {"width": 3}})).unwrap();
    let r = s
        .execute(
            "shape.edit",
            json!({"layer": id, "stroke": {"cap": "round", "dashes": [2, 1]}, "fill": {"gradient": {"stops": [[0, "#000000"], [1, "#ffffff"]], "angle": 0}}}),
        )
        .unwrap();
    assert_eq!(r["stroke"]["width"], 3.0);
    assert_eq!(r["stroke"]["cap"], "round");
    assert!(r["fill"]["gradient"].is_object());
    s.execute("shape.edit", json!({"layer": id, "stroke": null, "name": "Box"})).unwrap();
    assert!(shape(&s, id).stroke.is_none());
    assert_eq!(doc(&s).layer(LayerId(id)).unwrap().name, "Box");
    // Undo walks back one edit at a time.
    s.undo();
    assert!(shape(&s, id).stroke.is_some());
    // Editing a non-shape layer fails.
    let bg = doc(&s).layers[0].id.0;
    assert!(s.execute("shape.edit", json!({"layer": bg, "fill": "#000000"})).is_err());
}

#[test]
fn reapplying_unchanged_live_rectangle_keeps_nonuniformly_scaled_corners() {
    for depth in [8, 16, 32] {
        let mut s = session(128, 128, depth);
        let id = s.execute("shape.create", json!({"kind":"roundedRect","rect":[10,10,20,20],"radius":4})).unwrap()["layer"].as_u64().unwrap();
        s.execute("shape.edit", json!({"layer":id,"transform":[2,0,0,1,0,0]})).unwrap();
        let scaled = shape(&s, id);
        assert_eq!(
            scaled.live.as_ref().and_then(|l| match l {
                LiveShape::Rect { rect, .. } => Some(*rect),
                _ => None,
            }),
            Some([20.0, 10.0, 40.0, 20.0])
        );
        // The transformed path has an anisotropic top-left corner.
        let path = scaled.path.clone();
        s.execute("shape.edit", json!({"layer":id,"rect":[20,10,40,20]})).unwrap();
        assert_eq!(shape(&s, id).path, path, "depth {depth}: no-op rectangle edit must preserve all control points");
        s.execute("shape.edit", json!({"layer":id,"kind":"roundedRect","rect":[20,10,40,20],"radius":4})).unwrap();
        assert_eq!(shape(&s, id).path, path, "depth {depth}: explicitly unchanged radius preserves scaled outline");
        // A real radius edit must still rebuild the outline, and Undo must restore it.
        s.execute("shape.edit", json!({"layer":id,"radii":[6,6,6,6]})).unwrap();
        assert_ne!(shape(&s, id).path, path, "depth {depth}: changed radii must take effect");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(shape(&s, id).path, path);
    }
}

#[test]
fn shape_add_to_with_path_ops() {
    let mut s = session(100, 100, 8);
    let id = s.execute("shape.create", json!({"kind": "rect", "rect": [0, 0, 60, 60]})).unwrap()["layer"].as_u64().unwrap();
    let r = s.execute("shape.create", json!({"kind": "rect", "rect": [30, 30, 60, 60], "addTo": id, "op": "subtract"})).unwrap();
    assert_eq!(r["path"]["subpaths"][1]["op"], "subtract");
    assert!((alpha_sum(&shape(&s, id)) - (3600.0 - 900.0)).abs() < 1.0);
    s.execute("layer.combineShapes.intersectShapeAreas", json!({"layer": id})).unwrap();
    assert!((alpha_sum(&shape(&s, id)) - 900.0).abs() < 1.0);
    s.execute("layer.combineShapes.excludeOverlappingShapes", json!({"layer": id})).unwrap();
    assert!((alpha_sum(&shape(&s, id)) - 5400.0).abs() < 1.0);
    // Merge bakes the xor into plain outlines with the same area.
    let r = s.execute("layer.combineShapes.mergeShapeComponents", json!({"layer": id})).unwrap();
    let merged = shape(&s, id);
    assert!(merged.path.subpaths.len() >= 2, "{r}");
    assert!((alpha_sum(&merged) - 5400.0).abs() < 20.0, "{}", alpha_sum(&merged));
}

#[test]
fn shape_rasterize_keeps_pixels() {
    let mut s = session(64, 64, 16);
    let id = s.execute("shape.create", json!({"kind": "ellipse", "rect": [4, 4, 40, 30]})).unwrap()["layer"].as_u64().unwrap();
    let before = shape(&s, id).cache.unwrap();
    let flat = photocraft_compose::flatten(doc(&s));
    s.execute("layer.rasterize.shape", json!({"layer": id})).unwrap();
    let l = doc(&s).layer(LayerId(id)).unwrap();
    assert!(matches!(l.content, LayerContent::Raster(_)));
    assert_eq!(l.surface().unwrap(), &before);
    assert_eq!(photocraft_compose::flatten(doc(&s)).px, flat.px);
    assert!(s.execute("shape.rasterize", json!({"layer": id})).is_err());
}

#[test]
fn paths_set_rename_delete_list() {
    let mut s = session(100, 100, 8);
    let tri = json!({"subpaths": [{"knots": [[10, 10], [90, 10], [50, 90]]}]});
    s.execute("path.set", json!({"path": tri})).unwrap();
    assert!(doc(&s).work_path.is_some());
    // Saving the work path (rename) moves it into the saved paths.
    s.execute("path.rename", json!({"to": "Triangle"})).unwrap();
    assert!(doc(&s).work_path.is_none());
    assert_eq!(doc(&s).paths[0].name, "Triangle");
    s.execute("path.set", json!({"name": "Triangle", "op": "subtract", "path": {"subpaths": [{"knots": [[40, 20], [60, 20], [60, 40], [40, 40]]}]}})).unwrap();
    assert_eq!(doc(&s).paths[0].path.subpaths.len(), 2);
    assert_eq!(doc(&s).paths[0].path.subpaths[1].op, PathOp::Subtract);
    let l = s.execute("path.list", json!({})).unwrap();
    assert_eq!(l["paths"][0]["knots"], 7);
    let info = s.execute("path.info", json!({"name": "Triangle"})).unwrap();
    assert_eq!(info["path"]["subpaths"][1]["op"], "subtract");
    s.execute("path.rename", json!({"name": "Triangle", "to": "Tri"})).unwrap();
    assert!(s.execute("path.delete", json!({"name": "Triangle"})).is_err());
    s.execute("path.delete", json!({"name": "Tri"})).unwrap();
    assert!(doc(&s).paths.is_empty());
    s.undo();
    assert_eq!(doc(&s).paths[0].name, "Tri");
    assert!(s.execute("path.set", json!({"path": {"subpaths": [{"knots": [[0, 0], [1]]}]}})).is_err());
}

#[test]
fn path_to_selection_matches_analytic_shape() {
    let mut s = session(200, 200, 8);
    // Circle of radius 60 at (100, 100) as a work path.
    let c = vector::shapes::ellipse(40.0, 40.0, 120.0, 120.0);
    s.execute("path.set", json!({"path": path_json(&c)})).unwrap();
    s.execute("path.toSelection", json!({})).unwrap();
    let m = selection_mask(&s);
    let area: f64 = m.iter().map(|v| f64::from(*v)).sum();
    let want = std::f64::consts::PI * 3600.0;
    assert!((area - want).abs() / want < 0.005, "{area} vs {want}");
    // Pixel-level: centres well inside/outside are exact; edge pixels match the analytic coverage.
    let at = |x: usize, y: usize| m[y * 200 + x];
    assert_eq!(at(100, 100), 1.0);
    assert_eq!(at(10, 10), 0.0);
    for (x, y) in [(160usize, 100usize), (100, 40), (142, 142)] {
        let d = ((x as f64 + 0.5 - 100.0).powi(2) + (y as f64 + 0.5 - 100.0).powi(2)).sqrt();
        let analytic = (60.0 - d + 0.5).clamp(0.0, 1.0);
        assert!((f64::from(at(x, y)) - analytic).abs() < 0.2, "({x},{y}): {} vs {analytic}", at(x, y));
    }
    // Modes: subtract a square, then intersect.
    s.execute("path.set", json!({"name": "Sq", "path": {"subpaths": [{"knots": [[100, 0], [200, 0], [200, 200], [100, 200]]}]}})).unwrap();
    s.execute("path.toSelection", json!({"name": "Sq", "mode": "subtract"})).unwrap();
    let half: f64 = selection_mask(&s).iter().map(|v| f64::from(*v)).sum();
    assert!((half - want / 2.0).abs() / want < 0.005);
    // Non-anti-aliased selections are binary; feather softens.
    s.execute("path.toSelection", json!({"antiAlias": false})).unwrap();
    assert!(selection_mask(&s).iter().all(|v| *v == 0.0 || *v == 1.0));
    s.execute("path.toSelection", json!({"feather": 8})).unwrap();
    let m = selection_mask(&s);
    assert!(m[100 * 200 + 160] > 0.2 && m[100 * 200 + 160] < 0.8);
}

#[test]
fn path_make_selection_rejects_invalid_options_without_changing_history() {
    let mut s = session(80, 80, 8);
    s.execute("path.set", json!({"path": {"subpaths": [{"knots": [[12, 12], [60, 12], [60, 60], [12, 60]]}]}})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 20, "width": 12, "height": 12})).unwrap();
    let before = doc(&s).clone();
    let history = s.active().unwrap().history.past_len();
    for opts in [
        json!({"feather": -1}),
        json!({"feather": 1001}),
        json!({"feather": 1e100}),
        json!({"feather": "4"}),
        json!({"feather": true}),
        json!({"antiAlias": "false"}),
        json!({"antiAlias": 1}),
        json!({"mode": "union"}),
        json!({"mode": 42}),
    ] {
        assert!(s.execute("path.toSelection", opts.clone()).is_err(), "accepted invalid Make Selection options: {opts}");
        assert_eq!(*doc(&s), before, "invalid Make Selection modified the document: {opts}");
        assert_eq!(s.active().unwrap().history.past_len(), history, "invalid Make Selection changed history: {opts}");
    }
}

#[test]
fn open_pen_paths_make_the_same_selection_as_closed_paths() {
    // The pen's unfinished work path should be implicitly closed when rasterized into a selection.
    let mut s = session(80, 80, 8);
    let knots = json!([[12, 12], [52, 12], [52, 52], [12, 52]]);
    for (aa, feather) in [(false, 0.0), (true, 0.0), (false, 3.0), (true, 3.0)] {
        let mut masks = Vec::new();
        for closed in [false, true] {
            s.execute("path.set", json!({"path": {"subpaths": [{"closed": closed, "knots": knots}]}})).unwrap();
            s.execute("path.toSelection", json!({"antiAlias": aa, "feather": feather, "mode": "replace"})).unwrap();
            masks.push(selection_mask(&s));
        }
        assert_eq!(masks[0], masks[1], "open and closed paths differ (antiAlias={aa}, feather={feather})");
        assert!(masks[0][32 * 80 + 32] > 0.99);
        assert_eq!(masks[0][4 * 80 + 4], 0.0);
        if feather > 0.0 {
            assert!(masks[0].iter().any(|v| *v > 0.0 && *v < 1.0), "feather did not soften the selection");
        }
    }

    // All boolean operations should use the same selection semantics for an open Pen path.
    for mode in ["replace", "add", "subtract", "intersect"] {
        s.execute("select.rect", json!({"x": 30, "y": 30, "width": 34, "height": 25})).unwrap();
        s.execute("path.set", json!({"path": {"subpaths": [{"closed": false, "knots": knots}]}})).unwrap();
        s.execute("path.toSelection", json!({"mode": mode, "antiAlias": false})).unwrap();
        let mask = selection_mask(&s);
        let at = |x: usize, y: usize| mask[y * 80 + x];
        assert_eq!(at(20, 20), if matches!(mode, "replace" | "add") { 1.0 } else { 0.0 }, "{mode}: path-only");
        assert_eq!(at(60, 40), if matches!(mode, "add" | "subtract") { 1.0 } else { 0.0 }, "{mode}: previous-only");
        assert_eq!(at(40, 40), if mode == "subtract" { 0.0 } else { 1.0 }, "{mode}: overlap");
    }
}

#[test]
fn selection_to_work_path_roundtrip_iou() {
    let mut s = session(240, 180, 8);
    // An anti-aliased donut plus a rectangle, from a path, as the selection.
    let mut p = vector::shapes::ellipse(20.0, 20.0, 140.0, 140.0);
    p.subpaths.push(vector::shapes::ellipse(60.0, 60.0, 60.0, 60.0).subpaths.remove(0).with_op(PathOp::Subtract));
    p.subpaths.push(vector::shapes::rect(170.0, 30.0, 50.0, 120.0).subpaths.remove(0));
    s.execute("path.set", json!({"name": "src", "path": path_json(&p)})).unwrap();
    s.execute("path.toSelection", json!({"name": "src"})).unwrap();
    let before = selection_mask(&s);
    for tol in [1.0, 2.0] {
        let r = s.execute("select.toWorkPath", json!({"tolerance": tol})).unwrap();
        assert_eq!(r["subpaths"], 3);
        s.execute("path.toSelection", json!({})).unwrap();
        let after = selection_mask(&s);
        let (mut i, mut u) = (0.0f64, 0.0f64);
        for (a, b) in before.iter().zip(&after) {
            i += f64::from(a.min(*b));
            u += f64::from(a.max(*b));
        }
        assert!(i / u > 0.98, "tolerance {tol}: IoU {}", i / u);
        // Restore the original selection for the next tolerance.
        s.execute("path.toSelection", json!({"name": "src"})).unwrap();
    }
    // Selection → shape layer.
    let r = s.execute("select.convertToShape", json!({"fill": "#00ff00"})).unwrap();
    assert_eq!(r["fill"], "#00ff00");
    assert_eq!(r["path"]["subpaths"].as_array().unwrap().len(), 3);

    // tolerance and simplifyTolerance parameter
    s.execute("path.toSelection", json!({"name": "src"})).unwrap();
    let r_fine = s.execute("select.convertToShape", json!({"tolerance": 0.5, "name": "Fine Shape"})).unwrap();
    assert_eq!(r_fine["name"], "Fine Shape");
    let knots = |r: &Value| r["path"]["subpaths"].as_array().unwrap().iter().map(|sp| sp["knots"].as_array().unwrap().len()).sum::<usize>();
    let fine_knots = knots(&r_fine);

    s.execute("path.toSelection", json!({"name": "src"})).unwrap();
    let r_coarse = s.execute("select.convertToShape", json!({"simplifyTolerance": 5.0, "name": "Coarse Shape"})).unwrap();
    assert_eq!(r_coarse["name"], "Coarse Shape");
    let coarse_knots = knots(&r_coarse);
    assert!(fine_knots > coarse_knots, "fine tolerance {fine_knots} must have more knots than coarse {coarse_knots}");

    // Undo restores state
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.last().unwrap().name, "Fine Shape");
}

#[test]
fn convert_to_shape_requires_selection() {
    let mut s = session(100, 100, 8);
    assert!(!s.is_enabled("select.convertToShape"));
    assert!(s.execute("select.convertToShape", json!({})).is_err());
}

#[test]
fn fill_and_stroke_path_on_pixel_layer() {
    let mut s = session(100, 100, 16);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("path.set", json!({"path": {"subpaths": [{"knots": [[10, 10], [60, 10], [60, 60], [10, 60]]}]}})).unwrap();
    s.execute("path.fill", json!({"color": "#ff0000", "opacity": 50})).unwrap();
    let d = doc(&s);
    let l = d.layer(s.active().unwrap().active_layer.unwrap()).unwrap();
    let px = l.surface().unwrap().rgba(30, 30);
    assert!((px[0] - 1.0).abs() < 1e-3 && (px[3] - 0.5).abs() < 0.01, "{px:?}");
    assert_eq!(l.surface().unwrap().rgba(80, 80)[3], 0.0);
    // Stroke with a hard 4 px pencil along the square outline.
    s.execute("path.stroke", json!({"tool": "pencil", "size": 4, "color": "#0000ff", "opacity": 100})).unwrap();
    let d = doc(&s);
    let surf = d.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().clone();
    assert!(surf.rgba(10, 35)[2] > 0.9);
    assert!(surf.rgba(60, 35)[2] > 0.9);
    assert!(surf.rgba(35, 35)[2] < 0.1);
    assert!(s.execute("path.stroke", json!({"tool": "airbrush"})).is_err());
    // Filling needs a pixel layer.
    s.execute("shape.create", json!({"kind": "rect", "rect": [0, 0, 5, 5]})).unwrap();
    assert!(s.execute("path.fill", json!({})).is_err());
}

/// #2978: Simulate Pressure tapers the stroke at both ends (the brush's size follows pen
/// pressure); without it the stroke is as wide near its ends as in the middle.
#[test]
fn stroke_path_simulate_pressure_tapers_both_ends() {
    let coverage = |simulate: bool, x: i32| -> usize {
        let mut s = session(200, 60, 8);
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("path.set", json!({"path": {"subpaths": [{"closed": false, "knots": [[20, 30], [180, 30]]}]}})).unwrap();
        let p = json!({"tool": "brush", "size": 20, "hardness": 1.0, "color": "#000000", "opacity": 100, "simulatePressure": simulate});
        s.execute("path.stroke", p).unwrap();
        let d = doc(&s);
        let surf = d.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap();
        (0..60).filter(|&y| surf.rgba(x, y)[3] > 0.5).count()
    };
    let (end, middle) = (coverage(true, 26), coverage(true, 100));
    assert!(middle >= 18 && end * 3 < middle, "simulated: {end} px near the start, {middle} px in the middle");
    let (end, middle) = (coverage(false, 26), coverage(false, 100));
    assert!(middle >= 18 && end + 2 >= middle, "plain: {end} px near the start, {middle} px in the middle");
}

/// #2978: Stroke Path runs the other brush-based tools along the path (here the Blur tool) as one
/// history step, and only the path.stroke call is journalled, so an action replays it once.
#[test]
fn stroke_path_with_the_blur_tool_is_one_step() {
    let mut s = session(100, 60, 8);
    s.execute("layer.new.layer", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("path.set", json!({"path": {"subpaths": [{"knots": [[0, 0], [100, 0], [100, 60], [0, 60]]}]}})).unwrap();
    s.execute("path.fill", json!({"color": "#ffffff"})).unwrap();
    s.execute("path.set", json!({"path": {"subpaths": [{"knots": [[0, 0], [50, 0], [50, 60], [0, 60]]}]}})).unwrap();
    s.execute("path.fill", json!({"color": "#000000"})).unwrap();
    let px = |s: &Session, x: i32| doc(s).layer(id).unwrap().surface().unwrap().rgba(x, 30)[0];
    assert!(px(&s, 49) < 0.01 && px(&s, 50) > 0.99);
    // Two subpaths along the edge: still one step.
    let edge = json!({"subpaths": [{"closed": false, "knots": [[50, 5], [50, 28]]}, {"closed": false, "knots": [[50, 28], [50, 55]]}]});
    s.execute("path.set", json!({"path": edge})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    let journal = s.journal.len();
    s.execute("path.stroke", json!({"tool": "blur", "size": 10, "strength": 100})).unwrap();
    assert!(px(&s, 49) > 0.02 && px(&s, 50) < 0.98, "the edge is blurred: {} {}", px(&s, 49), px(&s, 50));
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    let ids: Vec<&str> = s.journal[journal..].iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(ids, ["path.stroke"]);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(px(&s, 49) < 0.01 && px(&s, 50) > 0.99);
    assert!(s.execute("path.stroke", json!({"tool": "blur", "simulatePressure": "yes"})).is_err());
    assert!(s.execute("path.stroke", json!({"tool": "airbrush"})).is_err());
}

#[test]
fn fill_path_stays_inside_the_selection() {
    let alpha = |s: &Session, x, y| {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)[3]
    };
    for depth in [8, 16, 32] {
        // A hard rectangle, an ellipse and a feathered rectangle, all over the left half.
        for extra in [json!({}), json!({"ellipse": true}), json!({"feather": 4})] {
            let mut s = session(100, 100, depth);
            s.execute("layer.new.layer", json!({})).unwrap();
            let mut rect = json!({"x": 0, "y": 0, "width": 50, "height": 100});
            rect.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            s.execute("select.rect", rect).unwrap();
            s.execute("path.set", json!({"path": {"subpaths": [{"closed": true, "knots": [[10, 10], [90, 10], [90, 90], [10, 90]]}]}})).unwrap();
            s.execute("path.fill", json!({"color": "#ff0000", "opacity": 100})).unwrap();
            let mask = selection_mask(&s);
            assert!(alpha(&s, 30, 50) > 0.99, "{depth} {extra}: inside the selection");
            assert_eq!(alpha(&s, 70, 50), 0.0, "{depth} {extra}: outside the selection");
            // At every pixel the fill's alpha is at most the selection's coverage there.
            for (x, y) in [(46, 50), (49, 50), (51, 50), (25, 12), (12, 25)] {
                let m = mask[(y * 100 + x) as usize];
                assert!(alpha(&s, x, y) <= m + 0.01, "{depth} {extra} ({x},{y}): alpha {} > selection {m}", alpha(&s, x, y));
            }
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(alpha(&s, 30, 50), 0.0, "{depth} {extra}: undo restores");
        }
    }
    // Without a selection the whole path fills, as before.
    let mut s = session(100, 100, 8);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("path.set", json!({"path": {"subpaths": [{"closed": true, "knots": [[10, 10], [90, 10], [90, 90], [10, 90]]}]}})).unwrap();
    s.execute("path.fill", json!({"color": "#ff0000"})).unwrap();
    assert!(alpha(&s, 30, 50) > 0.99 && alpha(&s, 70, 50) > 0.99);
}

#[test]
fn clipping_path_requires_saved_path_and_tracks_rename_delete() {
    let mut s = session(60, 60, 8);
    assert!(s.execute("path.clippingPath.set", json!({})).is_err());
    assert!(s.execute("path.clippingPath.set", json!({"name":"missing"})).is_err());
    s.execute("path.set", json!({"name":"Cutout","path":{"subpaths":[{"knots":[[1,1],[20,1],[20,20]]}]}})).unwrap();
    assert!(s.execute("path.clippingPath.set", json!({"name":"Cutout","flatness":-1})).is_err());
    assert!(s.execute("path.clippingPath.set", json!({"name":"Cutout","flatness":"bad"})).is_err());
    s.execute("path.clippingPath.set", json!({"name":"Cutout","flatness":2.5})).unwrap();
    assert_eq!(doc(&s).clipping_path.as_ref().unwrap().name, "Cutout");
    assert_eq!(doc(&s).clipping_path.as_ref().unwrap().flatness, 2.5);
    s.execute("path.rename", json!({"name":"Cutout","to":"Final"})).unwrap();
    assert_eq!(doc(&s).clipping_path.as_ref().unwrap().name, "Final");
    s.execute("path.clippingPath.clear", json!({})).unwrap();
    assert!(doc(&s).clipping_path.is_none());
    assert!(s.undo());
    assert_eq!(doc(&s).clipping_path.as_ref().unwrap().name, "Final");
    s.execute("path.delete", json!({"name":"Final"})).unwrap();
    assert!(doc(&s).clipping_path.is_none());
}

#[test]
fn transform_work_saved_and_shape_paths() {
    let mut s = session(100, 100, 8);
    let path = json!({"subpaths":[{"knots":[[4,5],[20,5],[20,20]]}]});
    s.execute("path.set", json!({"path":path})).unwrap();
    assert!(s.execute("path.transform", json!({})).is_err());
    assert!(s.execute("path.transform", json!({"matrix":[1,0,0,1,"bad",0]})).is_err());
    assert!(s.execute("path.transform", json!({"matrix":[1,0,0,1,1e30,0]})).is_err());
    s.execute("path.transform", json!({"matrix":[1,0,0,1,10,7]})).unwrap();
    assert_eq!(doc(&s).work_path.as_ref().unwrap().subpaths[0].knots[0].anchor, Point::new(14.0, 12.0));
    s.execute("path.set", json!({"name":"Saved","path":path})).unwrap();
    s.execute("path.transform", json!({"name":"Saved","matrix":[2,0,0,2,0,0]})).unwrap();
    assert_eq!(doc(&s).paths[0].path.subpaths[0].knots[0].anchor, Point::new(8.0, 10.0));
    s.execute("path.transform", json!({"name":"Saved","translateX":3,"translateY":-2})).unwrap();
    assert_eq!(doc(&s).paths[0].path.subpaths[0].knots[0].anchor, Point::new(11.0, 8.0));
    let id = s.execute("shape.create", json!({"kind":"rect","rect":[10,10,20,20]})).unwrap()["layer"].as_u64().unwrap();
    s.execute("path.transform", json!({"name":"layer","layer":id,"matrix":[1,0,0,1,5,0]})).unwrap();
    assert_eq!(shape(&s, id).path.subpaths[0].knots[0].anchor.x, 15.0);
    assert!(shape(&s, id).cache.is_some());
}

/// Distort / Perspective and Warp of a path (#3027): the anchors land on the quad's corners, one
/// undo step; hostile quads and warps are refused without a step.
#[test]
fn transform_path_distort_and_warp() {
    let mut s = session(100, 100, 8);
    s.execute("path.set", json!({"path":{"subpaths":[{"closed":true,"knots":[[10,10],[30,10],[30,20],[10,20]]}]}})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    for bad in [
        json!({"quad":[[0,0],[0,0],[0,0],[0,0]]}),
        json!({"quad":[[10,10],[40,5],[45,30]]}),
        json!({"quad":[[10,10],[40,5],[45,30],[5,"x"]]}),
        json!({"quad":[[10,10],[40,5],[45,30],[5,1e30]]}),
        json!({"quad":[[10,10],[40,5],[45,30],[5,25]],"rect":[10,10,10,20]}),
        json!({"style":"nope"}),
        json!({"style":"arc","rect":[0,0,1e30,1]}),
    ] {
        assert!(s.execute("path.transform", bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    let quad = [[12.0, 8.0], [40.0, 5.0], [45.0, 30.0], [5.0, 25.0]];
    s.execute("path.transform", json!({ "quad": quad })).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one step");
    let anchors: Vec<[f64; 2]> = doc(&s).work_path.as_ref().unwrap().subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
    assert_eq!(anchors.len(), 4);
    for (a, q) in anchors.iter().zip(quad) {
        assert!((a[0] - q[0]).abs() < 1e-9 && (a[1] - q[1]).abs() < 1e-9, "{anchors:?}");
    }
    s.execute("path.transform", json!({"style":"arc","bend":50})).unwrap();
    assert!(doc(&s).work_path.as_ref().unwrap().subpaths[0].knots.len() > 4, "the warp bends the edges");
}

#[test]
fn copy_and_paste_shape_fill_and_stroke() {
    let mut s = session(100, 100, 8);
    assert!(!s.is_enabled("path.style.copyFill"));
    assert!(!s.is_enabled("path.style.pasteFill"));
    assert!(s.execute("path.style.copyFill", json!({})).is_err());
    assert!(s.execute("path.style.pasteFill", json!({})).is_err());
    let source =
        s.execute("shape.create", json!({"kind":"rect","rect":[10,10,20,20],"fill":"#ff0000","stroke":{"width":3,"color":"#0000ff"}})).unwrap()["layer"]
            .as_u64()
            .unwrap();
    assert!(s.is_enabled("path.style.copyFill"));
    assert!(s.is_enabled("path.style.copyStroke"));
    s.execute("path.style.copyFill", json!({})).unwrap();
    s.execute("path.style.copyStroke", json!({})).unwrap();
    let target = s.execute("shape.create", json!({"kind":"rect","rect":[40,40,20,20],"fill":"#00ff00"})).unwrap()["layer"].as_u64().unwrap();
    assert!(s.is_enabled("path.style.pasteFill"));
    assert!(s.is_enabled("path.style.pasteStroke"));
    s.execute("path.style.pasteFill", json!({})).unwrap();
    s.execute("path.style.pasteStroke", json!({})).unwrap();
    assert_eq!(shape(&s, source).fill, shape(&s, target).fill);
    assert_eq!(shape(&s, source).stroke, shape(&s, target).stroke);
    assert!(s.undo());
    assert!(shape(&s, target).stroke.is_none());
}

#[test]
fn path_fill_handles_extreme_coordinates_and_feather() {
    let mut s = session(48, 32, 8);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute(
        "path.set",
        json!({
            "name": "P",
            "path": {"subpaths": [{
                "closed": true,
                "knots": [[0.0, 0.0], [3.0e9, 0.0], [3.0e9, 1.0e9]]
            }]}
        }),
    )
    .unwrap();
    s.execute("path.fill", json!({"name": "P", "color": "#ff0000"})).unwrap();
    s.execute("path.fill", json!({"name": "P", "color": "#ff0000", "feather": 1_073_741_824.0})).unwrap();

    let layer = doc(&s).layer(s.active().unwrap().active_layer.unwrap()).unwrap();
    assert_eq!(layer.surface().unwrap().tile_count(), 1);
}

#[test]
fn rasterizing_a_vector_mask_keeps_what_the_masks_show() {
    // #992: the vector mask's feather (and a pixel mask's feather or density) shape the rendered
    // edge, so the rasterized pixel mask must hold the mask the compositor showed.
    // (vector feather, vector enabled, pixel mask (feather, density))
    let cases = [
        (8.0, true, None),
        (8.0, true, Some((0.0, 1.0))),
        (0.0, true, Some((4.0, 1.0))),
        (0.0, true, Some((0.0, 0.5))),
        (6.0, true, Some((3.0, 0.7))),
        // Controls: hard masks were already exact; a disabled vector mask shows nothing.
        (0.0, true, None),
        (0.0, true, Some((0.0, 1.0))),
        (8.0, false, Some((4.0, 1.0))),
    ];
    for depth in [8, 16, 32] {
        for (vf, enabled, pm) in cases {
            let what = format!("{depth}-bit, vector feather {vf} enabled {enabled}, pixel mask {pm:?}");
            let mut s = session(64, 64, depth);
            s.execute("layer.new.layer", json!({})).unwrap();
            let id = s.active().unwrap().active_layer.unwrap();
            s.execute("path.set", json!({"name": "all", "path": {"subpaths": [{"knots": [[0, 0], [64, 0], [64, 64], [0, 64]]}]}})).unwrap();
            s.execute("path.fill", json!({"name": "all", "color": "#000000"})).unwrap();
            if let Some((feather, density)) = pm {
                s.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 64})).unwrap();
                s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
                s.edit("mask settings", |doc, _| {
                    let m = doc.layer_mut(id).unwrap().mask.as_mut().unwrap();
                    (m.feather, m.density) = (feather, density);
                    Ok(())
                })
                .unwrap();
            }
            s.execute("path.set", json!({"path": {"subpaths": [{"knots": [[16, 16], [48, 16], [48, 48], [16, 48]]}]}})).unwrap();
            s.execute("layer.vectorMask.currentPath", json!({})).unwrap();
            s.execute("layer.vectorMask.edit", json!({"feather": vf, "enabled": enabled})).unwrap();
            let before = photocraft_compose::flatten(doc(&s));
            let steps = s.active().unwrap().history.past_len();
            s.execute("layer.rasterize.vectorMask", json!({})).unwrap();
            let l = doc(&s).layer(id).unwrap();
            assert!(l.vector_mask.is_none() && l.mask.is_some(), "{what}");
            // An integer mask stores the soft values rounded to its depth.
            let tol = match l.mask.as_ref().unwrap().surface.format().sample {
                photocraft_color::SampleType::U8 => 1.5 / 255.0,
                _ => 1e-3,
            };
            assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{what}: one history step");
            let after = photocraft_compose::flatten(doc(&s));
            let worst = before.px.iter().zip(&after.px).flat_map(|(a, b)| (0..4).map(move |c| (a[c] - b[c]).abs())).fold(0.0f32, f32::max);
            assert!(worst <= tol, "{what}: the composite changed by {worst}");
            s.undo();
            assert!(doc(&s).layer(id).unwrap().vector_mask.is_some(), "{what}: undo brings the vector mask back");
        }
    }
}

#[test]
fn vector_mask_commands_and_compositing() {
    let mut s = session(100, 100, 8);
    s.execute("layer.new.layer", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap().0;
    // Fill the layer black via a full-canvas path fill (works regardless of edit.fill).
    s.execute("path.set", json!({"name": "all", "path": {"subpaths": [{"knots": [[0, 0], [100, 0], [100, 100], [0, 100]]}]}})).unwrap();
    s.execute("path.fill", json!({"name": "all", "color": "#000000"})).unwrap();
    // Reveal All: nothing hidden.
    s.execute("layer.vectorMask.revealAll", json!({})).unwrap();
    let f = photocraft_compose::flatten(doc(&s));
    assert!(f.get(80, 80)[0] < 0.01);
    // Current path → only the left half shows the black layer.
    s.execute("path.set", json!({"path": {"subpaths": [{"knots": [[0, 0], [50, 0], [50, 100], [0, 100]]}]}})).unwrap();
    s.execute("layer.vectorMask.currentPath", json!({})).unwrap();
    let f = photocraft_compose::flatten(doc(&s));
    assert!(f.get(20, 50)[0] < 0.01 && f.get(80, 50)[0] > 0.99);
    // Disable → all black again; toggling re-enables.
    s.execute("layer.vectorMask.enabled", json!({})).unwrap();
    assert!(photocraft_compose::flatten(doc(&s)).get(80, 50)[0] < 0.01);
    s.execute("layer.vectorMask.enabled", json!({})).unwrap();
    // Density 50%: the hidden half shows half grey.
    let r = s.execute("layer.vectorMask.edit", json!({"density": 50})).unwrap();
    assert_eq!(r["density"], 50.0);
    assert!((photocraft_compose::flatten(doc(&s)).get(80, 50)[0] - 0.5).abs() < 0.01);
    s.execute("layer.vectorMask.edit", json!({"density": 100})).unwrap();
    // Hide All.
    s.execute("layer.vectorMask.hideAll", json!({})).unwrap();
    assert!(photocraft_compose::flatten(doc(&s)).get(20, 50)[0] > 0.99);
    // Rasterize into the pixel mask: same composite, no vector mask left.
    s.execute("layer.vectorMask.currentPath", json!({})).unwrap();
    let before = photocraft_compose::flatten(doc(&s));
    s.execute("layer.rasterize.vectorMask", json!({})).unwrap();
    let l = doc(&s).layer(LayerId(id)).unwrap();
    assert!(l.vector_mask.is_none() && l.mask.is_some());
    assert_eq!(photocraft_compose::flatten(doc(&s)).px, before.px);
    // Delete / undo.
    s.undo();
    s.execute("layer.vectorMask.delete", json!({})).unwrap();
    assert!(doc(&s).layer(LayerId(id)).unwrap().vector_mask.is_none());
    assert!(s.execute("layer.vectorMask.delete", json!({})).is_err());
    s.undo();
    assert!(doc(&s).layer(LayerId(id)).unwrap().vector_mask.is_some());
    // Shape layers cannot take a separate vector mask.
    s.execute("shape.create", json!({"kind": "rect", "rect": [0, 0, 10, 10]})).unwrap();
    assert!(s.execute("layer.vectorMask.revealAll", json!({})).is_err());
}

/// #4201: the vector mask's feather takes the pixel mask's range (finite 0..=1000 px); anything
/// else is refused before the edit, so no infinite feather can block a .pcraft save.
#[test]
fn vector_mask_edit_rejects_out_of_range_feather() {
    let mut s = session(32, 32, 8);
    s.execute("layer.new.layer", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.vectorMask.revealAll", json!({})).unwrap();
    let feather = |s: &Session| doc(s).layer(id).unwrap().vector_mask.as_ref().unwrap().feather;
    let steps = s.active().unwrap().history.past_len();
    for bad in [json!(1e39), json!(-1), json!(1001), json!("x"), json!(null)] {
        assert!(s.execute("layer.vectorMask.edit", json!({"feather": bad})).is_err(), "feather {bad}");
        assert_eq!(feather(&s), 0.0, "feather {bad}");
        assert_eq!(s.active().unwrap().history.past_len(), steps, "feather {bad}");
    }
    let r = s.execute("layer.vectorMask.edit", json!({"feather": 12.5})).unwrap();
    assert_eq!(r["feather"], 12.5);
    assert_eq!(feather(&s), 12.5);
    s.execute("layer.vectorMask.edit", json!({"feather": 1000})).unwrap();
    assert_eq!(feather(&s), 1000.0);
}

/// #5846: a shape layer's path is its vector mask; `layer.vectorMask.edit` sets its Density and
/// Feather in one undoable step, re-rendering the fill, which shows beyond the path at 100 −
/// density %. Hostile values are refused before any edit.
#[test]
fn shape_density_and_feather_through_vector_mask_edit() {
    for depth in [8, 16, 32] {
        let mut s = session(200, 200, depth);
        let id = s.execute("shape.create", json!({"kind": "rect", "rect": [50, 50, 100, 100], "fill": "#000000"})).unwrap()["layer"].as_u64().unwrap();
        let info = s.execute("layer.vectorMask.info", json!({})).unwrap();
        assert_eq!((info["shape"].clone(), info["density"].clone(), info["feather"].clone()), (json!(true), json!(100.0), json!(0.0)), "{depth}");
        let steps = s.active().unwrap().history.past_len();
        let r = s.execute("layer.vectorMask.edit", json!({"density": 50})).unwrap();
        assert_eq!(r["density"], 50.0);
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "one history step");
        let sh = shape(&s, id);
        assert_eq!((sh.density, sh.feather), (0.5, 0.0));
        // The shape's own pixels carry the soft fill (half-transparent black beyond the path).
        let cache = sh.cache.as_ref().unwrap();
        assert!((cache.rgba(10, 10)[3] - 0.5).abs() < 0.01, "{depth}: {:?}", cache.rgba(10, 10));
        assert!(cache.rgba(100, 100)[3] > 0.99);
        s.execute("layer.vectorMask.edit", json!({"feather": 10, "density": 100})).unwrap();
        let sh = shape(&s, id);
        assert_eq!((sh.density, sh.feather), (1.0, 10.0));
        let a = |x| sh.cache.as_ref().unwrap().rgba(x, 100)[3];
        assert!(a(30) > 0.0 && a(30) < 0.05 && (a(49) + a(50) - 1.0).abs() < 0.02 && a(70) > 0.95, "{depth}: {} {} {} {}", a(30), a(49), a(50), a(70));
        // Fill and stroke stay editable: the shape is still a shape.
        s.execute("shape.edit", json!({"fill": "#ff0000"})).unwrap();
        assert_eq!(shape(&s, id).feather, 10.0, "shape.edit keeps the feather");
        s.undo();
        s.undo();
        assert_eq!((shape(&s, id).density, shape(&s, id).feather), (0.5, 0.0), "{depth}: undo restores");
    }
}

#[test]
fn shape_vector_mask_edit_fails_gracefully() {
    let mut s = session(64, 64, 8);
    let id = s.execute("shape.create", json!({"kind": "rect", "rect": [8, 8, 30, 30]})).unwrap()["layer"].as_u64().unwrap();
    let before = doc(&s).clone();
    let steps = s.active().unwrap().history.past_len();
    let bad = [
        json!({"density": -1}),
        json!({"density": 100.5}),
        json!({"density": 1e39}),
        json!({"density": "50"}),
        json!({"density": null}),
        json!({"feather": -0.5}),
        json!({"feather": 1001}),
        json!({"feather": [1]}),
        json!({"feather": 5, "path": {"subpaths": []}}),
        json!({"density": 50, "enabled": false}),
        json!({"invert": true}),
        json!({"layer": u64::MAX, "density": 50}),
    ];
    for p in bad {
        assert!(s.execute("layer.vectorMask.edit", p.clone()).is_err(), "{p}");
        assert_eq!(*doc(&s), before, "{p}");
        assert_eq!(s.active().unwrap().history.past_len(), steps, "{p}");
    }
    // A locked shape refuses it too.
    s.execute("layer.lockLayers", json!({"all": true})).unwrap();
    assert!(s.execute("layer.vectorMask.edit", json!({"layer": id, "density": 50})).is_err());
    assert_eq!(shape(&s, id).density, 1.0);
}

#[test]
fn translate_vectors_moves_shapes_and_masks() {
    let mut s = session(100, 100, 8);
    let id = s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 20, 20]})).unwrap()["layer"].as_u64().unwrap();
    s.edit("Move", |d, _| {
        let snap = d.clone();
        let l = d.layer_mut(LayerId(id)).unwrap();
        translate_vectors(&snap, l, 5.0, 0.0);
        Ok(())
    })
    .unwrap();
    let sh = shape(&s, id);
    assert_eq!(sh.cache.unwrap().content_bounds(), Rect::new(15, 10, 35, 30));
    assert_eq!(sh.live, Some(LiveShape::Rect { rect: [15.0, 10.0, 20.0, 20.0], radii: [0.0; 4] }));
}

#[test]
fn all_vector_commands_are_registered() {
    for id in [
        "shape.create",
        "shape.edit",
        "shape.info",
        "shape.rasterize",
        "path.set",
        "path.delete",
        "path.rename",
        "path.list",
        "path.info",
        "path.toSelection",
        "select.toWorkPath",
        "path.fill",
        "path.stroke",
        "layer.vectorMask.add",
        "layer.vectorMask.fromPath",
        "layer.vectorMask.delete",
        "layer.vectorMask.edit",
        "layer.rasterize.vectorMask",
    ] {
        assert!(crate::commands::find(id).is_some(), "{id}");
    }
    let mut ids: Vec<&str> = crate::command_specs().iter().map(|c| c.id).collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate command ids");
}

#[test]
fn path_stroke_rejects_oversized_brush_before_rendering() {
    let mut s = session(32, 32, 8);
    s.execute("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[2, 2], [20, 20]]}]}})).unwrap();
    let result = s.execute("path.stroke", json!({"size": 1e30}));
    assert!(result.is_err(), "path strokes share the bounded paint renderer");
}

#[test]
fn shape_info_bounds_of_curved_shapes_are_exact() {
    // Issue #2537: an exact 600×600 ellipse reported width 601.
    for depth in [8, 16, 32] {
        let mut s = session(1920, 1080, depth);
        for (params, want) in [
            (json!({"kind": "ellipse", "rect": [0, 0, 100, 100], "fill": "#0000ff"}), json!([0, 0, 100, 100])),
            (json!({"kind": "ellipse", "rect": [660, 240, 600, 600], "fill": "#0000ff"}), json!([660, 240, 600, 600])),
            (json!({"kind": "ellipse", "rect": [10.5, 10, 100, 100], "fill": "#0000ff"}), json!([10, 10, 101, 100])),
            (json!({"kind": "rect", "rect": [0, 0, 100, 100], "fill": "#0000ff"}), json!([0, 0, 100, 100])),
        ] {
            let id = s.execute("shape.create", params.clone()).unwrap()["layer"].as_u64().unwrap();
            let info = s.execute("shape.info", json!({"layer": id})).unwrap();
            assert_eq!(info["bounds"], want, "{params} at {depth}-bit");
        }
    }
}

/// A shape layer as a PSD import leaves it: Photoshop's rendering as the cache (deliberately not
/// what PhotoCraft would draw, a block of another colour, so a re-render shows).
fn add_imported_shape(s: &mut Session, r: Rect) -> LayerId {
    s.edit("shape", |doc, active| {
        let fmt = doc.pixel_format();
        let path = photocraft_vector::shapes::rect(f64::from(r.x0), f64::from(r.y0), f64::from(r.width()), f64::from(r.height()));
        let mut cache = photocraft_raster::Surface::new(fmt);
        cache.fill_rect(r, &photocraft_raster::from_rgba(&fmt, [0.9, 0.5, 0.1, 1.0]));
        let sh = ShapeLayer { path, fill: Some(photocraft_doc::Fill::Solid(photocraft_color::Color::BLACK)), cache: Some(cache), ..Default::default() };
        let id = doc.insert_above(None, Layer::new("Rectangle 1", LayerContent::Shape(sh)));
        *active = Some(id);
        Ok(id)
    })
    .unwrap()
}

fn shape_layer(doc: &Document, id: LayerId) -> ShapeLayer {
    match &doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.clone(),
        other => panic!("not a shape layer: {}", other.kind_name()),
    }
}

fn orange_at(sh: &ShapeLayer, x: i32, y: i32) -> bool {
    let c = sh.cache.as_ref().unwrap();
    let px = photocraft_raster::to_rgba(&c.format(), &c.pixel(x, y));
    (px[0] - 0.9).abs() < 0.01 && (px[1] - 0.5).abs() < 0.01
}

#[test]
fn moving_and_exporting_imported_shapes_keeps_their_rendering() {
    for depth in [8, 16, 32] {
        let mut s = session(100, 80, depth);
        let id = add_imported_shape(&mut s, Rect::new(30, 20, 60, 40));
        // The Move tool: the same pixels, moved; the path moves with them.
        s.execute("layer.translate", json!({"layer": id.0, "dx": 5, "dy": -3})).unwrap();
        let sh = shape_layer(doc(&s), id);
        assert_eq!(sh.cache.as_ref().unwrap().content_bounds(), Rect::new(35, 17, 65, 37), "depth {depth}");
        assert!(orange_at(&sh, 40, 20), "depth {depth}: not re-rendered");
        assert_eq!(sh.path.control_bounds().map(|b| (b.0.round(), b.1.round())), Some((35.0, 17.0)), "the path moves too");
        // Artboards to Files moves the board to the origin the same way.
        s.execute("layer.new.artboard", json!({"rect": [20, 10, 60, 40]})).unwrap();
        let board = doc(&s).artboards()[0].0;
        s.edit("into board", |doc, _| {
            let l = doc.remove(id).unwrap();
            let Some(LayerContent::Group(g)) = doc.layer_mut(board).map(|b| &mut b.content) else { panic!("artboard group") };
            g.children.push(l);
            Ok(())
        })
        .unwrap();
        let one = crate::artboard_cmds::artboard_document(doc(&s), board).unwrap().unwrap();
        let sh = shape_layer(&one, id);
        assert_eq!(sh.cache.as_ref().unwrap().content_bounds(), Rect::new(15, 7, 45, 27), "depth {depth}");
        assert!(orange_at(&sh, 20, 10), "depth {depth}: exported as rendered");
    }
}

#[test]
fn a_shape_reaching_past_the_canvas_renders_again_when_moved() {
    let mut s = session(100, 80, 8);
    // Cut at the canvas's right edge (100 px), so its pixels can't simply be shifted back in.
    let id = add_imported_shape(&mut s, Rect::new(80, 20, 120, 40));
    s.execute("layer.translate", json!({"layer": id.0, "dx": -30, "dy": 0})).unwrap();
    let sh = shape_layer(doc(&s), id);
    assert!(!orange_at(&sh, 60, 30), "re-rendered from the path (black fill)");
    assert_eq!(sh.cache.as_ref().unwrap().content_bounds(), Rect::new(50, 20, 90, 40), "the whole shape shows");
}

#[test]
fn shape_create_negative_size_rect_covers_the_ellipse_box() {
    // Issue #4203: a rectangle with a negative width spanned x 50..70 while the ellipse and
    // polygon with the same `rect` spanned 30..50.
    for depth in [8, 16, 32] {
        let mut s = session(100, 60, depth);
        let ell = s.execute("shape.create", json!({"kind": "ellipse", "rect": [50, 10, -20, 30], "fill": "#0000ff"})).unwrap()["bounds"].clone();
        assert_eq!(ell, json!([30, 10, 20, 30]), "{depth}-bit");
        for params in [
            json!({"kind": "rect", "rect": [50, 10, -20, 30], "fill": "#0000ff"}),
            json!({"kind": "roundedRect", "rect": [50, 10, -20, 30], "radii": 5, "fill": "#0000ff"}),
            json!({"kind": "rect", "rect": [50, 40, -20, -30], "fill": "#0000ff"}),
        ] {
            let r = s.execute("shape.create", params.clone()).unwrap();
            assert_eq!(r["bounds"], ell, "{params} at {depth}-bit");
            let sh = shape(&s, r["layer"].as_u64().unwrap());
            let c = sh.cache.as_ref().unwrap();
            let fmt = c.format();
            let alpha = |x: i32, y: i32| photocraft_raster::to_rgba(&fmt, &c.read_region(photocraft_doc::Rect::from_xywh(x, y, 1, 1)))[3];
            assert!(alpha(40, 25) > 0.99, "{params} at {depth}-bit");
            assert!(alpha(60, 25) < 0.01, "{params} at {depth}-bit");
        }
    }
}

/// Two overlapping 40×40 rectangle shape layers ("Rectangle 1" black below, "Rectangle 2" grey
/// above, overlapping by 20×20) with both selected, like the Layers panel in #2400.
fn two_selected_rects(depth: u32) -> (Session, u64, u64) {
    let mut s = session(100, 100, depth);
    let a = s.execute("shape.create", json!({"rect": [10, 10, 40, 40], "fill": "#000000"})).unwrap()["layer"].as_u64().unwrap();
    let b = s.execute("shape.create", json!({"rect": [30, 30, 40, 40], "fill": "#808080"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layer": a})).unwrap();
    s.execute("layer.select", json!({"layer": b, "mode": "add"})).unwrap();
    assert_eq!(s.active().unwrap().selected_layers(), [LayerId(a), LayerId(b)]);
    (s, a, b)
}

#[test]
fn combine_shapes_merges_the_selected_shape_layers_into_the_top_one() {
    for depth in [8, 16, 32] {
        for (cmd, area) in [
            ("layer.combineShapes.unite", 2800.0),
            ("layer.combineShapes.subtractFrontShape", 1200.0),
            ("layer.combineShapes.intersectShapeAreas", 400.0),
            ("layer.combineShapes.excludeOverlappingShapes", 2400.0),
            ("layer.combineShapes.mergeShapeComponents", 2800.0),
        ] {
            let (mut s, a, b) = two_selected_rects(depth);
            let before = doc(&s).clone();
            let top_fill = shape(&s, b).fill;
            let history = s.active().unwrap().history.past_len();
            let r = s.execute(cmd, json!({})).unwrap();
            assert_eq!(r["layer"], b, "{cmd}");
            let d = doc(&s);
            assert!(d.layer(LayerId(a)).is_none(), "{cmd}: the lower shape layer is merged away");
            assert_eq!(d.walk().len(), 2, "{cmd}: Background plus the merged shape");
            let l = d.layer(LayerId(b)).unwrap();
            assert_eq!(l.name, "Rectangle 2");
            let sh = shape(&s, b);
            assert!((alpha_sum(&sh) - area).abs() < 2.0, "{cmd} at {depth} bits: covers {} px, not {area}", alpha_sum(&sh));
            assert_eq!(sh.fill, top_fill, "{cmd}: the top layer's fill");
            assert_eq!(s.active().unwrap().active_layer, Some(LayerId(b)));
            assert_eq!(s.active().unwrap().selected_layers(), [LayerId(b)]);
            assert_eq!(s.active().unwrap().history.past_len(), history + 1, "{cmd}: one history step");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(*doc(&s), before, "{cmd}: undo restores both layers");
        }
    }
}

#[test]
fn combine_shapes_keeps_the_single_layer_behaviour() {
    let (mut s, a, b) = two_selected_rects(8);
    // An explicit layer narrows it to that layer's components, as before.
    s.execute("shape.create", json!({"addTo": b, "rect": [50, 50, 10, 10]})).unwrap();
    s.execute("layer.combineShapes.subtractFrontShape", json!({"layer": b})).unwrap();
    let d = doc(&s);
    assert!(d.layer(LayerId(a)).is_some());
    assert_eq!(shape(&s, b).path.subpaths[1].op, PathOp::Subtract);
    assert!((alpha_sum(&shape(&s, b)) - 1500.0).abs() < 2.0);
    // One selected shape layer: the operation applies to its own components.
    s.execute("layer.select", json!({"layer": b})).unwrap();
    s.execute("layer.combineShapes.unite", json!({})).unwrap();
    assert!(doc(&s).layer(LayerId(a)).is_some());
    assert!((alpha_sum(&shape(&s, b)) - 1600.0).abs() < 2.0);
}

#[test]
fn combine_shapes_ignores_selected_non_shape_layers() {
    let (mut s, a, b) = two_selected_rects(8);
    let bg = doc(&s).walk()[0].2.id;
    s.execute("layer.select", json!({"layer": bg.0, "mode": "add"})).unwrap();
    assert_eq!(s.active().unwrap().selected_layers().len(), 3);
    s.execute("layer.combineShapes.unite", json!({})).unwrap();
    let d = doc(&s);
    assert!(d.layer(bg).is_some(), "the Background is left alone");
    assert!(d.layer(LayerId(a)).is_none());
    assert!((alpha_sum(&shape(&s, b)) - 2800.0).abs() < 2.0);
}

#[test]
fn combine_shapes_fails_gracefully() {
    let (mut s, a, _) = two_selected_rects(8);
    let before = doc(&s).clone();
    let history = s.active().unwrap().history.past_len();
    let bad_params = [json!({"layer": u64::MAX}), json!({"layer": a, "subpath": 99}), json!({"subpath": 99}), json!({"tolerance": "x", "layer": u64::MAX})];
    for cmd in [
        "layer.combineShapes.unite",
        "layer.combineShapes.subtractFrontShape",
        "layer.combineShapes.intersectShapeAreas",
        "layer.combineShapes.excludeOverlappingShapes",
        "layer.combineShapes.mergeShapeComponents",
    ] {
        for p in &bad_params {
            if cmd.ends_with("mergeShapeComponents") && p.get("subpath").is_some() {
                continue; // `subpath` means nothing to Merge Shape Components
            }
            assert!(s.execute(cmd, p.clone()).is_err(), "{cmd} {p}");
            assert_eq!(*doc(&s), before, "{cmd} {p}");
        }
    }
    assert_eq!(s.active().unwrap().history.past_len(), history);
    // Locked layers refuse the merge without touching the document.
    s.execute("layer.lockLayers", json!({"all": true})).unwrap();
    assert!(doc(&s).effective_locks(LayerId(a)).all);
    let before = doc(&s).clone();
    assert!(s.execute("layer.combineShapes.unite", json!({})).is_err());
    assert_eq!(*doc(&s), before);
}

#[test]
fn triangles_and_polygons_take_a_corner_radius_kept_through_edits_and_scaling() {
    let mut s = session(200, 200, 8);
    let r = s.execute("shape.create", json!({"kind": "polygon", "rect": [20, 20, 120, 120], "sides": 3, "cornerRadius": 12})).unwrap();
    let id = r["layer"].as_u64().unwrap();
    let live = |s: &Session| match &s.active().unwrap().doc.layer(LayerId(id)).unwrap().content {
        LayerContent::Shape(sh) => sh.live.clone(),
        _ => None,
    };
    let path_len = |s: &Session| match &s.active().unwrap().doc.layer(LayerId(id)).unwrap().content {
        LayerContent::Shape(sh) => sh.path.subpaths[0].knots.len(),
        _ => 0,
    };
    assert!(matches!(live(&s), Some(LiveShape::Polygon { sides: 3, corner_radius, .. }) if corner_radius == 12.0));
    assert_eq!(path_len(&s), 6, "every vertex rounded");
    // Other edits keep the radius; a radius edit regenerates the path; 0 makes it sharp again.
    s.execute("shape.edit", json!({"layer": id, "sides": 6})).unwrap();
    assert!(matches!(live(&s), Some(LiveShape::Polygon { sides: 6, corner_radius, .. }) if corner_radius == 12.0));
    assert_eq!(path_len(&s), 12);
    s.execute("shape.edit", json!({"layer": id, "cornerRadius": 0})).unwrap();
    assert_eq!(path_len(&s), 6);
    // Hostile values never panic: negative and non-finite radii are clamped or ignored.
    for bad in [json!(-5), json!(1e300), json!("x")] {
        let _ = s.execute("shape.edit", json!({"layer": id, "cornerRadius": bad}));
    }
    assert!(matches!(live(&s), Some(LiveShape::Polygon { corner_radius, .. }) if corner_radius.is_finite() && corner_radius >= 0.0));
    // Scaling the layer by half halves the radius, as for a rectangle's radii.
    s.execute("shape.edit", json!({"layer": id, "cornerRadius": 10})).unwrap();
    s.execute("shape.edit", json!({"layer": id, "transform": [0.5, 0, 0, 0.5, 0, 0]})).unwrap();
    assert!(matches!(live(&s), Some(LiveShape::Polygon { corner_radius, .. }) if (corner_radius - 5.0).abs() < 1e-9));
}
