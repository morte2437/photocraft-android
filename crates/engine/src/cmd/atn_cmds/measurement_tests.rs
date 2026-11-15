//! #10723: Image › Analysis › Set Measurement Scale › Custom steps, recorded as a Set of the
//! document's `measurementScale` property (`docs/atn.md`, "Recorded through the menus").

use super::*;

const SET_SCALE: &str = "image.analysis.setMeasurementScale";
const SOURCE: &str = "_atnMeasurementScaleDescriptor";

/// The `T   ` object of the measured step, built here independently of the converter.
fn scale(class: &str, pixels: i32, logical: f64, units: &str) -> Descriptor {
    Descriptor::new(class)
        .with("pixelLength", D::Integer(pixels))
        .with("logicalLength", D::Double(logical))
        .with("logicalUnits", D::Text(UnicodeString::new_nul(units)))
}

/// The measured Set: `null` is the `measurementScale` property of the target document.
fn recorded(t: Descriptor) -> Descriptor {
    let target = D::Reference(vec![
        ReferenceItem::Property { class: class("Prpr"), key: Id::new("measurementScale") },
        ReferenceItem::Enumerated { class: class("Dcmn"), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
    ]);
    Descriptor::new("null").with("null", target).with("T   ", D::Descriptor(t))
}

fn set_item(d: Descriptor) -> AtnItem {
    AtnItem { event: AtnEvent::Code(*b"setd"), dictionary: b"Set".to_vec(), descriptor: Some(d), ..Default::default() }
}

/// Load `items` as action "A" through `actions.load`.
fn load(items: Vec<AtnItem>) -> (Session, Value) {
    let file = AtnFile {
        name: UnicodeString::new_nul("Analysis"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        ..Default::default()
    };
    let mut s = Session::new();
    let r = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&atn::write(&file).unwrap())})).unwrap();
    (s, r)
}

/// Action "A" written by `actions.save` and parsed back.
fn saved(s: &mut Session) -> Vec<AtnItem> {
    let r = s.execute("actions.save", json!({"action": "A"})).unwrap();
    assert_eq!(r["skipped"], json!([]), "{r}");
    atn::parse(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap().actions.remove(0).items
}

/// The recorded step loads as a custom scale and saves back byte for byte, whatever class its
/// `T   ` object was recorded with and in whatever key order.
#[test]
fn a_recorded_scale_step_loads_and_saves_back_unchanged() {
    let mut reordered = scale("measurementScale", 50, 2.5, "cm");
    reordered.items.reverse();
    for (t, pixels, logical, units) in
        [(scale("measurementScale", 120, 3.0, "mm"), 120, 3.0, "mm"), (scale("null", 100, 25.4, "inches"), 100, 25.4, "inches"), (reordered, 50, 2.5, "cm")]
    {
        let d = recorded(t);
        let (mut s, r) = load(vec![set_item(d.clone())]);
        assert_eq!(r["unsupported"], json!([]), "{r}");
        let (id, p) = &s.actions.list[0].steps[0];
        assert_eq!(id, SET_SCALE);
        assert_eq!(
            (&p["preset"], p["pixelLength"].as_f64(), p["logicalLength"].as_f64(), &p["units"]),
            (&json!("custom"), Some(f64::from(pixels)), Some(logical), &json!(units)),
            "{p}"
        );
        let back = saved(&mut s);
        assert_eq!((&back[0].event, back[0].descriptor.as_ref().map(Descriptor::to_bytes)), (&AtnEvent::Code(*b"setd"), Some(d.to_bytes())));
    }
}

/// A loaded step sets the document's scale, as one undoable edit.
#[test]
fn a_loaded_scale_step_plays_and_undoes() {
    let (mut s, _) = load(vec![set_item(recorded(scale("measurementScale", 120, 3.0, "mm")))]);
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert_eq!(played["ran"], 1, "{played}");
    let read = s.execute(SET_SCALE, json!({})).unwrap();
    assert_eq!(
        (&read["pixelLength"], &read["logicalLength"], &read["units"], &read["default"]),
        (&json!(120.0), &json!(3.0), &json!("mm"), &json!(false)),
        "{read}"
    );
    s.execute("history.undo", json!({})).unwrap();
    assert_eq!(s.execute(SET_SCALE, json!({})).unwrap()["default"], json!(true));
}

/// The step PhotoCraft writes for a native custom scale is the measured form, with `T   ` of
/// class `measurementScale`, and it loads back without a stored descriptor.
#[test]
fn a_native_scale_saves_the_measured_form() {
    let p = json!({"preset": "custom", "pixelLength": 120, "logicalLength": 3, "units": "mm"});
    let d = export_item(SET_SCALE, &p, "S").and_then(|item| item.descriptor).unwrap();
    assert_eq!(d, recorded(scale("measurementScale", 120, 3.0, "mm")));
    let (s, _) = load(vec![set_item(d)]);
    assert_eq!(s.actions.list[0].steps[0].1, p);
    // `logicalUnits` is the command's other spelling of `units`, and the preset defaults to custom.
    let other = json!({"pixelLength": 120, "logicalLength": 3, "logicalUnits": "mm"});
    assert_eq!(export_item(SET_SCALE, &other, "S").and_then(|item| item.descriptor), Some(recorded(scale("measurementScale", 120, 3.0, "mm"))));
}

/// A native step the event can't hold is refused: reading the scale, the default preset, a
/// value left to the current scale, a fractional pixel length (the event stores a long), blank
/// units, or another key.
#[test]
fn settings_the_event_cannot_hold_are_refused() {
    for p in [
        json!({}),
        json!({"preset": "default"}),
        json!({"preset": "query"}),
        json!({"preset": "custom", "pixelLength": 120, "logicalLength": 3}),
        json!({"preset": "custom", "logicalLength": 3, "units": "mm"}),
        json!({"preset": "custom", "pixelLength": 120.5, "logicalLength": 3, "units": "mm"}),
        json!({"preset": "custom", "pixelLength": 0, "logicalLength": 3, "units": "mm"}),
        json!({"preset": "custom", "pixelLength": 120, "logicalLength": -1, "units": "mm"}),
        json!({"preset": "custom", "pixelLength": 120, "logicalLength": 3, "units": "  "}),
        json!({"preset": "custom", "pixelLength": 120, "logicalLength": 3, "units": "mm", "layer": 2}),
    ] {
        assert!(export_item(SET_SCALE, &p, "S").is_none(), "{p}");
    }
}

/// Any other form keeps the reference step: another target, a missing or extra key, values of
/// another type, or values the native command refuses.
#[test]
fn other_forms_keep_the_photoshop_step() {
    let with = |key: &str, v: D| {
        let mut t = scale("measurementScale", 120, 3.0, "mm");
        if let Some(slot) = t.items.iter_mut().find(|(k, _)| k.is(key)) {
            slot.1 = v;
        }
        recorded(t)
    };
    let mut missing = scale("measurementScale", 120, 3.0, "mm");
    missing.items.pop();
    let mut layer_target = recorded(scale("measurementScale", 120, 3.0, "mm"));
    layer_target.items[0].1 = D::Reference(vec![
        ReferenceItem::Property { class: class("Prpr"), key: Id::new("measurementScale") },
        ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
    ]);
    let forms = [
        recorded(missing),
        recorded(scale("measurementScale", 120, 3.0, "mm").with("Scl ", D::Double(1.0))),
        recorded(scale("measurementScale", 120, 3.0, "mm")).with("Rslt", D::Double(72.0)),
        layer_target,
        with("pixelLength", D::Double(120.0)),
        with("logicalLength", D::Integer(3)),
        with("logicalLength", D::Double(0.0)),
        with("pixelLength", D::Integer(-4)),
        with("logicalUnits", D::Text(UnicodeString::new_nul(" "))),
        with("logicalUnits", D::Integer(1)),
    ];
    for d in forms {
        let (mut s, r) = load(vec![set_item(d.clone())]);
        assert_eq!(r["unsupported"].as_array().map(Vec::len), Some(1), "{d:?}");
        assert_eq!(s.actions.list[0].steps[0].0, PHOTOSHOP_STEP, "{d:?}");
        assert_eq!(saved(&mut s)[0].descriptor.as_ref().map(Descriptor::to_bytes), Some(d.to_bytes()));
    }
}

/// A loaded step whose scale was changed saves the new scale, not the stale recording; one whose
/// recording is damaged still saves its settings.
#[test]
fn changed_or_damaged_steps_save_their_settings() {
    let (s, _) = load(vec![set_item(recorded(scale("null", 100, 25.4, "inches")))]);
    let mut p = s.actions.list[0].steps[0].1.clone();
    assert!(p.get(SOURCE).is_some(), "{p}");
    p["units"] = json!("cm");
    assert_eq!(export_item(SET_SCALE, &p, "S").and_then(|item| item.descriptor), Some(recorded(scale("measurementScale", 100, 25.4, "cm"))));
    p[SOURCE] = json!("not base64!");
    assert_eq!(export_item(SET_SCALE, &p, "S").and_then(|item| item.descriptor), Some(recorded(scale("measurementScale", 100, 25.4, "cm"))));
}
