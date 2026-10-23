//! #7671: exported Stroke widths must have the same effect as command execution.
use photocraft_engine::actions_cmds::Action;
use photocraft_engine::{Session, atn_cmds};
use photocraft_psd::{atn, descriptor::Value as D};
use serde_json::{Value, json};

/// Synthetic colours and partial transparency make lost edge pixels observable at every depth.
fn fixture(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 24, "height": 18, "depth": depth})).unwrap();
    s.edit("procedural fixture", |doc, active| {
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let format = surface.format();
        let bytes: Vec<_> =
            (0..432).flat_map(|i| photocraft_raster::from_rgba(&format, [(i % 11) as f32 / 10.0, (i % 7) as f32 / 6.0, (i % 5) as f32 / 4.0, 0.6])).collect();
        surface.write_region(photocraft_doc::Rect::from_xywh(0, 0, 24, 18), &bytes);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 6, "y": 4, "width": 12, "height": 10})).unwrap();
    s
}

/// Compare the flattened document, including alpha, rather than only imported JSON.
fn pixels(s: &Session) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(&s.active().unwrap().doc).px
}

/// Direct execution, native playback and strict binary ATN playback must agree exactly.
fn check_pixels(widths: &[(f64, u32)]) {
    let mut lost = Vec::new();
    for depth in [8, 16, 32] {
        for location in ["inside", "center", "outside"] {
            for &(width, effective) in widths {
                let params = json!({"width": width, "color": "#d03060", "location": location, "opacity": 63, "mode": "multiply"});
                let action = Action::new("Stroke", vec![("edit.stroke".into(), params.clone())]);
                let mut direct = fixture(depth);
                let before = pixels(&direct);
                direct.execute("edit.stroke", params.clone()).unwrap();
                let expected = pixels(&direct);
                assert_ne!(expected, before, "ineffective fixture: {depth}, {params}");

                let mut whole = fixture(depth);
                let mut whole_params = params.clone();
                whole_params["width"] = json!(effective);
                whole.execute("edit.stroke", whole_params).unwrap();
                assert_eq!(pixels(&whole), expected, "command width normalization: {depth}, {params}");
                if width == 2.4 {
                    let mut rounded = fixture(depth);
                    let mut rounded_params = params.clone();
                    rounded_params["width"] = json!(2);
                    rounded.execute("edit.stroke", rounded_params).unwrap();
                    assert_ne!(pixels(&rounded), expected, "2.4 must differ from 2: {depth}, {location}");
                }

                let mut native = fixture(depth);
                native.actions.list.push(action.clone());
                native.execute("actions.play", json!({"action": "Stroke"})).unwrap();
                assert_eq!(pixels(&native), expected);

                let (bytes, skipped) = atn_cmds::export("Widths", &[&action], &[], false).unwrap();
                assert!(skipped.is_empty());
                let imported = atn_cmds::import(&bytes).unwrap();
                assert!(imported.unsupported.is_empty());
                assert!(imported.warnings.is_empty());
                assert_eq!(imported.actions.len(), 1);
                assert_eq!(imported.actions[0].steps.len(), 1);
                assert_eq!(imported.actions[0].steps[0].0, "edit.stroke");
                let imported_width = imported.actions[0].steps[0].1["width"].clone();
                let mut replay = fixture(depth);
                replay.actions.list = imported.actions;
                replay.execute("actions.play", json!({"action": "Stroke"})).unwrap();
                if pixels(&replay) != expected {
                    lost.push(format!("depth {depth}, {location}, width {width} -> {imported_width}, expected {effective}"));
                }
            }
        }
    }
    assert!(lost.is_empty(), "ATN changed Stroke pixels:\n{}", lost.join("\n"));
}

/// In particular 2.4 renders as 3 px, while the old exporter serialized 2 px.
#[test]
fn fractional_stroke_width_preserves_pixels_at_every_depth_and_location() {
    check_pixels(&[(0.5, 1), (1.01, 2), (2.01, 3), (2.4, 3), (2.5, 3), (2.6, 3), (3.1, 4)]);
}

/// Existing whole-pixel strokes remain supported, with their original rendering.
#[test]
fn integer_stroke_width_controls_still_preserve_pixels() {
    check_pixels(&[(1.0, 1), (2.0, 2), (3.0, 3), (4.0, 4)]);
}

/// Export the command's bounded/default width as an integer, retaining the other options.
#[test]
fn descriptor_width_matches_defaults_limits_and_other_stroke_options() {
    for (width, effective) in [
        (None, 1),
        (Some(Value::Null), 1),
        (Some(json!("2.4")), 1),
        (Some(json!(true)), 1),
        (Some(json!([])), 1),
        (Some(json!({})), 1),
        (Some(json!(-100)), 1),
        (Some(json!(0)), 1),
        (Some(json!(249.01)), 250),
        (Some(json!(249.5)), 250),
        (Some(json!(250)), 250),
        (Some(json!(250.1)), 250),
        (Some(json!(1e300)), 250),
    ] {
        let mut params =
            json!({"color": "#d03060", "location": "outside", "opacity": 63, "mode": "multiply", "preserveTransparency": true, "target": "pixels"});
        if let Some(width) = width {
            params["width"] = width;
        }
        let action = Action::new("Stroke", vec![("edit.stroke".into(), params.clone())]);
        let (bytes, skipped) = atn_cmds::export("Widths", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let file = atn::parse(&bytes).unwrap();
        let descriptor = file.actions[0].items[0].descriptor.as_ref().unwrap();
        assert_eq!(descriptor.get("Wdth"), Some(&D::Integer(effective)), "{params}");
        let imported = atn_cmds::import(&bytes).unwrap();
        assert!(imported.unsupported.is_empty());
        assert!(imported.warnings.is_empty());
        let p = &imported.actions[0].steps[0].1;
        assert_eq!(p["width"].as_f64(), Some(f64::from(effective)));
        for key in ["color", "location", "preserveTransparency"] {
            assert_eq!(p[key], params[key]);
        }
        // Import uses the canonical blend-mode label ("Multiply"); execution accepts both cases.
        assert!(p["mode"].as_str().unwrap().eq_ignore_ascii_case(params["mode"].as_str().unwrap()));
        assert_eq!(p["opacity"].as_f64(), Some(63.0));
        let mut direct = fixture(8);
        direct.execute("edit.stroke", params).unwrap();
        let mut replay = fixture(8);
        replay.actions.list = imported.actions;
        replay.execute("actions.play", json!({"action": "Stroke"})).unwrap();
        assert_eq!(pixels(&replay), pixels(&direct));
    }
    // A null parameter object is also a valid call to the command with its default width.
    let action = Action::new("Default", vec![("edit.stroke".into(), Value::Null)]);
    let (bytes, skipped) = atn_cmds::export("Widths", &[&action], &[], false).unwrap();
    assert!(skipped.is_empty());
    let imported = atn_cmds::import(&bytes).unwrap();
    assert!(imported.unsupported.is_empty());
    assert!(imported.warnings.is_empty());
    assert_eq!(imported.actions[0].steps[0].1["width"].as_f64(), Some(1.0));
    let mut direct = fixture(8);
    direct.execute("edit.stroke", Value::Null).unwrap();
    let mut replay = fixture(8);
    replay.actions.list = imported.actions;
    replay.execute("actions.play", json!({"action": "Default"})).unwrap();
    assert_eq!(pixels(&replay), pixels(&direct));
}

/// A custom Stroke converter must retain the plain-event mask/channel target guard.
#[test]
fn stroke_mask_and_channel_targets_are_still_refused_and_reported() {
    for target in [json!("mask"), json!("quickMask"), json!({"channel": 0})] {
        let action = Action::new("Targeted", vec![("edit.stroke".into(), json!({"width": 2.4, "target": target}))]);
        assert!(atn_cmds::export("Widths", &[&action], &[], false).is_err());
        let (bytes, skipped) = atn_cmds::export("Widths", &[&action], &[], true).unwrap();
        assert_eq!(skipped, vec![atn_cmds::Skipped { action: "Targeted".into(), step: 0, id: "edit.stroke".into() }]);
        assert!(atn::parse(&bytes).unwrap().actions[0].items.is_empty());
    }
}
