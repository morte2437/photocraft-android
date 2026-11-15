//! Public load/save/playback witnesses for the measured Shear action mapping.
use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use craft_psd::descriptor::{Descriptor, Id, UnicodeString, Value as D};
use photocraft_engine::{
    Session,
    actions_cmds::Action,
    atn_cmds::{Imported, export, import},
};
use serde_json::{Value, json};

fn desc(class: &str) -> Descriptor {
    Descriptor::new(class)
}
fn enm(ty: &str, value: &str) -> D {
    D::Enumerated { type_id: Id::new(ty), value: Id::new(value) }
}
/// Four-byte recorded IDs retain their code form; longer IDs exercise string-event lookup.
fn item(event: &str, label: &str, descriptor: Option<Descriptor>) -> AtnItem {
    let event = match event.as_bytes() {
        [a, b, c, d] => AtnEvent::Code([*a, *b, *c, *d]),
        _ => AtnEvent::name(event),
    };
    AtnItem { event, dictionary: label.as_bytes().to_vec(), descriptor, ..Default::default() }
}
fn one_action(items: Vec<AtnItem>) -> AtnFile {
    AtnFile {
        name: UnicodeString::new_nul("Saves"),
        expanded: false,
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        warnings: vec![],
        ..Default::default()
    }
}
fn b64(bytes: &[u8]) -> String {
    photocraft_engine::paint::tile::b64_encode(bytes)
}
fn mapped_recording(event: &str, descriptor: Descriptor) -> Imported {
    import(&atn::write(&one_action(vec![item(event, event, Some(descriptor))])).unwrap()).unwrap()
}
/// Save one real native action and parse its public output, rather than calling a private converter.
fn export_item(id: &str, params: &Value, set: &str) -> Option<AtnItem> {
    let action = Action::new("A", vec![(id.into(), params.clone())]);
    let bytes = export(set, &[&action], &[], false).ok()?.0;
    atn::parse(&bytes).ok()?.actions.first()?.items.first().cloned()
}

/// The measured Shear list uses ordered horizontal/vertical doubles inside point objects.
fn shear_descriptor(points: &[[f64; 2]], undefined: &str) -> Descriptor {
    let list = points.iter().map(|p| D::Descriptor(desc("Pnt ").with("Hrzn", D::Double(p[0])).with("Vrtc", D::Double(p[1])))).collect();
    desc("null").with("ShrP", D::List(list)).with("UndA", enm("UndA", undefined)).with("ShrS", D::Integer(0)).with("ShrE", D::Integer(points.len() as i32 - 1))
}

/// Both event identities, changed curves and missing-key defaults save every measured setting in order.
#[test]
fn shear_steps_roundtrip_curves_defaults_and_flags() {
    let factory = shear_descriptor(&[[0.0, 1.0], [0.0, 128.0]], "WrpA");
    for event in ["Shr ", "shear"] {
        for d in [factory.clone(), shear_descriptor(&[[-32.0, 1.0], [16.0, 64.5], [0.0, 128.0]], "RptE")] {
            let loaded = mapped_recording(event, d.clone());
            assert!(loaded.unsupported.is_empty());
            let action = &loaded.actions[0];
            assert_eq!(action.steps[0].0, "filter.distort.shear");
            let bytes = export("S", &[action], &[], false).unwrap().0;
            let saved = atn::parse(&bytes).unwrap();
            assert_eq!(saved.actions[0].items[0].event, AtnEvent::Code(*b"Shr "));
            assert_eq!(saved.actions[0].items[0].descriptor, Some(d));
            assert_eq!(import(&bytes).unwrap().actions[0].steps, action.steps);
        }
    }
    let loaded = mapped_recording("Shr ", desc("null"));
    assert!(loaded.unsupported.is_empty());
    assert_eq!(loaded.actions[0].steps[0].1, json!({"points":[[0,0],[1,0]],"undefinedAreas":"wrap"}));
    assert_eq!(export_item("filter.distort.shear", &loaded.actions[0].steps[0].1, "S").unwrap().descriptor, Some(factory));
    let mut step = item("Shr ", "Shear", Some(shear_descriptor(&[[0.0, 1.0], [0.0, 128.0]], "RptE")));
    step.enabled = false;
    step.with_dialog = true;
    step.dialog_options = 2; // The wire mode for showing the enabled dialog toggle.
    let loaded = import(&atn::write(&one_action(vec![step.clone()])).unwrap()).unwrap();
    let bytes = export("S", &[&loaded.actions[0]], &[], false).unwrap().0;
    let saved = atn::parse(&bytes).unwrap();
    assert_eq!(saved.actions[0].items[0], step);
}

/// Clamp hostile coordinates, but retain finite fractional wire values that division by127 cannot invert.
#[test]
fn shear_clamps_points_and_preserves_fractional_wire_values() {
    let raw = shear_descriptor(&[[-10000.0, -1000.0], [10000.0, 1000.0]], "WrpA");
    let loaded = mapped_recording("Shr ", raw);
    assert!(loaded.unsupported.is_empty());
    let p = &loaded.actions[0].steps[0].1;
    assert_eq!(p["points"], json!([[0, -1], [1, 63.0 / 64.0]]));
    assert_eq!(export_item("filter.distort.shear", p, "S").unwrap().descriptor, Some(shear_descriptor(&[[-64.0, 1.0], [63.0, 128.0]], "WrpA")));

    let d = shear_descriptor(&[[0.0, 1.0], [7.97, 4.975609756097561], [0.0, 128.0]], "RptE");
    let loaded = mapped_recording("shear", d.clone());
    assert!(loaded.unsupported.is_empty());
    let mut p = loaded.actions[0].steps[0].1.clone();
    assert!(p.get("_atnShearPoints").is_some(), "the measured inverse loses a wire bit");
    assert_eq!(export_item("filter.distort.shear", &p, "S").unwrap().descriptor, Some(d));
    let mut edited = p.clone();
    let position = p["points"][1][0].as_f64().unwrap();
    edited["points"][1][0] = json!(position + 1e-12);
    assert_ne!(edited["points"][1][0], p["points"][1][0]);
    assert_eq!(edited["points"][1][0].as_f64().unwrap() as f32, position as f32);
    assert!(export_item("filter.distort.shear", &edited, "S").is_none(), "stored wire metadata rejects even an edit below native precision");
    p["points"][1][1] = json!(0.25);
    assert!(export_item("filter.distort.shear", &p, "S").is_none(), "stale wire metadata must not replace a native edit");
}

/// The adapter's engineering budget admits its boundary and preserves larger recordings intact.
#[test]
fn shear_point_budget_preserves_recordings_and_refuses_oversized_native_exports() {
    const LIMIT: usize = 4096;
    for count in [LIMIT, LIMIT + 1] {
        let grid: Vec<_> = (0..count).map(|i| [0.0, 1.0 + 127.0 * i as f64 / (count - 1) as f64]).collect();
        let label = if count == LIMIT { "Shear" } else { "Budget curve" };
        let mut source = item("Shr ", label, Some(shear_descriptor(&grid, "WrpA")));
        source.enabled = false;
        let loaded = import(&atn::write(&one_action(vec![source.clone()])).unwrap()).unwrap();
        let action = &loaded.actions[0];
        if count == LIMIT {
            assert!(loaded.unsupported.is_empty());
            assert_eq!(action.steps[0].0, "filter.distort.shear");
            assert_eq!(action.steps[0].1["points"].as_array().unwrap().len(), LIMIT);
        } else {
            assert_eq!(loaded.unsupported.len(), 1);
            assert_eq!(action.steps[0].0, "actions.photoshopStep");
        }
        let saved = export("S", &[action], &[], false).unwrap().0;
        assert!(atn::parse(&saved).unwrap().actions[0].items[0] == source, "{count}-point item changed on save");

        let native = json!({"points":grid.iter().map(|p| [(p[1] - 1.0) / 127.0, p[0] / 64.0]).collect::<Vec<_>>()});
        let before = native.clone();
        assert_eq!(export_item("filter.distort.shear", &native, "S").is_some(), count == LIMIT);
        assert_eq!(native, before, "export never edits the caller's curve");
        if count > LIMIT {
            let metadata = json!({"points":[[0,0],[1,0]],"_atnShearPoints":grid});
            assert!(export_item("filter.distort.shear", &metadata, "S").is_none());
        }
    }
}

/// Unmeasured subranges, malformed points and repeated settings remain whole, including their flags.
#[test]
fn shear_unrepresentable_steps_stay_kept_without_partial_playback() {
    let full = shear_descriptor(&[[0.0, 1.0], [8.0, 64.5], [0.0, 128.0]], "WrpA");
    let mut missing_end = full.clone();
    missing_end.items.pop(); // Missing ShrE means1, not the supplied list's last index2.
    let mut invalid_start = full.clone();
    invalid_start.items[2].1 = D::Integer(1);
    let mut invalid_end = full.clone();
    invalid_end.items[3].1 = D::Integer(100);
    let mut wrong_type = full.clone();
    wrong_type.items[2].1 = D::Double(0.0);
    for d in [
        missing_end,
        invalid_start,
        invalid_end,
        wrong_type,
        full.clone().with("ShrS", D::Integer(0)),
        full.clone().with("extra", D::Boolean(true)),
        desc("null").with("ShrP", D::List(vec![])),
        desc("null").with("ShrP", D::List(vec![D::Integer(7)])),
        desc("null").with("ShrP", D::List(vec![D::Descriptor(desc("Pnt ").with("Hrzn", D::Double(0.0)))])),
        desc("null").with("UndA", enm("UndA", "Trns")),
        desc("null").with("UndA", enm("Other", "WrpA")),
        shear_descriptor(&[[0.0, 128.0], [0.0, 1.0]], "WrpA"),
        shear_descriptor(&[[0.0, 1.0], [8.0, 1.0], [0.0, 128.0]], "WrpA"),
    ] {
        let step = item("Shr ", "Shear", Some(d));
        let loaded = import(&atn::write(&one_action(vec![step.clone()])).unwrap()).unwrap();
        assert_eq!(loaded.unsupported.len(), 1, "{step:?}");
        let bytes = export("S", &[&loaded.actions[0]], &[], false).unwrap().0;
        assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0], step);
        let mut s = Session::new();
        s.execute("file.new", json!({"width":8,"height":8})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let past = s.active().unwrap().history.past_len();
        let name = loaded.actions[0].name.clone();
        s.actions.list.extend(loaded.actions);
        let report = s.execute("actions.play", json!({"action":name})).unwrap();
        assert_eq!(report["ran"], 0);
        assert!(report.get("failed").is_some());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), past);
    }
}

/// Native exports follow points-before-amount and refuse settings that the measured curve cannot carry.
#[test]
fn shear_native_exports_keep_effective_defaults_and_refuse_lossy_options() {
    for p in [
        json!({}),
        json!({"amount":25}),
        json!({"points":[[0,0],[1,0.5]],"amount":-80,"undefinedAreas":"repeat"}),
        json!({"points":[[0,0],[0.009,0.125],[1,0]]}),
    ] {
        let item = export_item("filter.distort.shear", &p, "S").unwrap();
        let loaded = mapped_recording("Shr ", item.descriptor.unwrap());
        assert!(loaded.unsupported.is_empty());
        let steps = &loaded.actions[0].steps;
        assert_eq!(
            photocraft_engine::filters_ext::params_for("filter.distort.shear", &steps[0].1),
            photocraft_engine::filters_ext::params_for("filter.distort.shear", &p)
        );
    }
    for p in [
        json!({"points":[]}),
        json!({"points":[[0,0],[1,"bad"]]}),
        json!({"points":[[1,0],[0,0]]}),
        json!({"points":[[0,0],[1,1]]}),
        json!({"amount":100}),
        json!({"undefinedAreas":"transparent"}),
        json!({"target":"mask"}),
        json!({"layer":1}),
        json!({"other":true}),
        json!({"points":[[0,0],[1,0]],"_atnShearPoints":[[0,1],[8,128]]}),
    ] {
        assert!(export_item("filter.distort.shear", &p, "S").is_none(), "{p} exported");
    }
}

/// A constant measured curve shifts pixels right by two, with independent wrap/repeat border and Undo oracles.
#[test]
fn shear_action_moves_real_pixels_and_undoes_at_each_depth() {
    for depth in [8, 16, 32] {
        for edge in ["WrpA", "RptE"] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":32,"height":8,"depth":depth,"background":"transparent"})).unwrap();
            s.edit("Pattern", |doc, _| {
                let surface = doc.layers[0].surface_mut().unwrap();
                let data: Vec<f32> = (0..8).flat_map(|y| (0..32).flat_map(move |x| [x as f32 / 31.0, y as f32 / 7.0, (x % 3) as f32 / 2.0, 1.0])).collect();
                surface.write_region(photocraft_geom::Rect::new(0, 0, 32, 8), &data);
                Ok(())
            })
            .unwrap();
            let before = s.active().unwrap().doc.clone();
            let pixels = photocraft_compose::flatten(&before);
            let bytes = atn::write(&one_action(vec![item("Shr ", "Shear", Some(shear_descriptor(&[[8.0, 1.0], [8.0, 128.0]], edge)))])).unwrap();
            let loaded = s.execute("actions.load", json!({"data":b64(&bytes)})).unwrap();
            assert_eq!(loaded["unsupported"], json!([]));
            let played = s.execute("actions.play", json!({"action":loaded["actions"][0]})).unwrap();
            assert_eq!(played["ran"], 1, "{played}");
            let output = photocraft_compose::flatten(&s.active().unwrap().doc);
            assert_ne!(output, pixels);
            for y in 0..8 {
                for x in 0..32 {
                    let source = if edge == "WrpA" { (x - 2i32).rem_euclid(32) } else { (x - 2i32).max(0) };
                    for (got, want) in output.get(x, y).into_iter().zip(pixels.get(source, y)) {
                        assert!((got - want).abs() < 1e-5, "depth{depth} {edge} ({x},{y}):{got} != {want}");
                    }
                }
            }
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc, before);
        }
    }
}
