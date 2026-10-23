//! #7669: strict ATN export must not replace a native fractional Crystallize cell size.
use photocraft_engine::actions_cmds::Action;
use photocraft_engine::{Session, atn_cmds};
use photocraft_psd::{atn, descriptor::Value as D};
use serde_json::{Value, json};

const CRYSTALLIZE: &str = "filter.pixelate.crystallize";

/// Procedural colours expose cell-boundary changes without relying on external images.
fn fixture(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 24, "height": 18, "depth": depth})).unwrap();
    s.edit("procedural fixture", |doc, active| {
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let format = surface.format();
        let bytes: Vec<_> =
            (0..432).flat_map(|i| photocraft_raster::from_rgba(&format, [(i % 11) as f32 / 10.0, (i % 7) as f32 / 6.0, (i % 5) as f32 / 4.0, 1.0])).collect();
        surface.write_region(photocraft_doc::Rect::from_xywh(0, 0, 24, 18), &bytes);
        Ok(())
    })
    .unwrap();
    s
}

/// Compare the actual flattened document rather than just the imported parameters.
fn pixels(s: &Session) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(&s.active().unwrap().doc).px
}

/// Fractions remain valid native edits; strict integer ATN conversion must refuse them.
#[test]
fn fractional_cell_sizes_remain_native_and_are_refused_by_strict_export() {
    let mut silently_exported = Vec::new();
    for depth in [8, 16, 32] {
        for cell_size in [3.1, 5.4, 5.5, 5.6, 10.25, 299.9] {
            let params = json!({"cellSize": cell_size});
            let action = Action::new("Fractional Crystallize", vec![(CRYSTALLIZE.into(), params.clone())]);
            let mut direct = fixture(depth);
            direct.execute(CRYSTALLIZE, params).unwrap();
            let expected = pixels(&direct);
            let mut native = fixture(depth);
            native.actions.list.push(action.clone());
            native.execute("actions.play", json!({"action": "Fractional Crystallize"})).unwrap();
            assert_eq!(pixels(&native), expected);

            if cell_size == 5.4 {
                let mut rounded = fixture(depth);
                rounded.execute(CRYSTALLIZE, json!({"cellSize": 5})).unwrap();
                assert_ne!(pixels(&rounded), expected, "fixture must expose 5.4 vs 5: {depth}");
            }
            if let Ok((bytes, skipped)) = atn_cmds::export("Crystallize", &[&action], &[], false) {
                let imported = atn_cmds::import(&bytes).unwrap();
                assert!(skipped.is_empty() && imported.unsupported.is_empty() && imported.warnings.is_empty());
                let exported = imported.actions[0].steps[0].1["cellSize"].clone();
                let mut replay = fixture(depth);
                replay.actions.list = imported.actions;
                replay.execute("actions.play", json!({"action": "Fractional Crystallize"})).unwrap();
                let changed = pixels(&replay) != expected;
                if cell_size == 5.4 {
                    assert!(changed, "baseline must expose the reported pixel loss: {depth}");
                }
                silently_exported.push(format!("depth {depth}: {cell_size} -> {exported}, pixels changed: {changed}"));
            }
        }
    }
    assert!(silently_exported.is_empty(), "strict export silently accepted fractional cell sizes:\n{}", silently_exported.join("\n"));
}

/// Integer sizes, bounds and omitted defaults retain their binary descriptor and playback.
#[test]
fn integer_cell_sizes_and_defaults_preserve_playback() {
    for depth in [8, 16, 32] {
        let mut cases = vec![Value::Null, json!({}), json!({"target": "pixels"}), json!({"cellSize": 5.0})];
        cases.extend([3, 5, 10, 100, 300].map(|cell_size| json!({"cellSize": cell_size})));
        for params in cases {
            let action = Action::new("Integer Crystallize", vec![(CRYSTALLIZE.into(), params.clone())]);
            let mut direct = fixture(depth);
            direct.execute(CRYSTALLIZE, params.clone()).unwrap();
            let mut native = fixture(depth);
            native.actions.list.push(action.clone());
            native.execute("actions.play", json!({"action": "Integer Crystallize"})).unwrap();
            assert_eq!(pixels(&native), pixels(&direct));

            let (bytes, skipped) = atn_cmds::export("Crystallize", &[&action], &[], false).unwrap();
            assert!(skipped.is_empty());
            let file = atn::parse(&bytes).unwrap();
            let descriptor = file.actions[0].items[0].descriptor.as_ref().unwrap();
            if let Some(cell_size) = params.get("cellSize").and_then(Value::as_f64) {
                assert!(matches!(descriptor.get("ClSz"), Some(D::Integer(n)) if f64::from(*n) == cell_size));
            }
            let imported = atn_cmds::import(&bytes).unwrap();
            assert!(imported.unsupported.is_empty() && imported.warnings.is_empty());
            assert_eq!(imported.actions.len(), 1);
            assert_eq!(imported.actions[0].steps.len(), 1);
            assert_eq!(imported.actions[0].steps[0].0, CRYSTALLIZE);
            let mut replay = fixture(depth);
            replay.actions.list = imported.actions;
            replay.execute("actions.play", json!({"action": "Integer Crystallize"})).unwrap();
            assert_eq!(pixels(&replay), pixels(&direct), "depth {depth}: {params}");
        }
    }
}

/// Skipping is explicit and reports exactly which edits were lost, preserving supported order.
#[test]
fn skipping_fractional_cell_sizes_reports_action_and_step() {
    let action =
        Action::new("Mixed Crystallize", [5.0, 5.4, 10.0, 10.25].into_iter().map(|cell_size| (CRYSTALLIZE.into(), json!({"cellSize": cell_size}))).collect());
    let error = atn_cmds::export("Crystallize", &[&action], &[], false).unwrap_err();
    assert!(error.contains("step 2 of `Mixed Crystallize`"), "{error}");
    assert!(error.contains(CRYSTALLIZE) && error.contains("skipUnsupported"), "{error}");
    let (bytes, skipped) = atn_cmds::export("Crystallize", &[&action], &[], true).unwrap();
    assert_eq!(
        skipped,
        vec![
            atn_cmds::Skipped { action: "Mixed Crystallize".into(), step: 1, id: CRYSTALLIZE.into() },
            atn_cmds::Skipped { action: "Mixed Crystallize".into(), step: 3, id: CRYSTALLIZE.into() },
        ]
    );
    let imported = atn_cmds::import(&bytes).unwrap();
    assert!(imported.unsupported.is_empty() && imported.warnings.is_empty());
    assert_eq!(imported.actions.len(), 1);
    let steps = &imported.actions[0].steps;
    assert_eq!(steps.len(), 2);
    for (step, cell_size) in steps.iter().zip([5.0, 10.0]) {
        assert_eq!(step.0, CRYSTALLIZE);
        assert_eq!(step.1["cellSize"].as_f64(), Some(cell_size));
    }
}

/// The plain event still cannot carry mask/channel targets or nonnumeric cell sizes.
#[test]
fn non_pixel_targets_and_malformed_fields_are_still_refused() {
    for params in [
        json!({"cellSize": 5, "target": "mask"}),
        json!({"cellSize": 5, "target": "quickMask"}),
        json!({"cellSize": 5, "target": "channel"}),
        json!({"cellSize": 5, "target": null}),
        json!({"cellSize": null}),
        json!({"cellSize": "5.4"}),
        json!({"cellSize": 2147483648_i64}),
        json!({"cellSize": -2147483649_i64}),
        json!({"cellSize": 1e100}),
    ] {
        let action = Action::new("Unsupported Crystallize", vec![(CRYSTALLIZE.into(), params.clone())]);
        assert!(atn_cmds::export("Crystallize", &[&action], &[], false).is_err(), "{params}");
        let (bytes, skipped) = atn_cmds::export("Crystallize", &[&action], &[], true).unwrap();
        assert_eq!(skipped, vec![atn_cmds::Skipped { action: "Unsupported Crystallize".into(), step: 0, id: CRYSTALLIZE.into() }]);
        assert!(atn_cmds::import(&bytes).unwrap().actions[0].steps.is_empty());
    }
}
