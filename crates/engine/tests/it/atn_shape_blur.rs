use photocraft_doc::LayerContent;
use photocraft_engine::{Session, atn_cmds};
use photocraft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use photocraft_psd::descriptor::{Descriptor, Id, UnicodeString, Value as D};
use serde_json::json;

#[path = "../../../psd/tests/common/csh_fixture.rs"]
mod csh_fixture;

const SHAPE_ID: &str = "original-fixture-square";

/// The recorded display name differs from the installed ID to expose accidental name lookup.
fn descriptor() -> Descriptor {
    Descriptor::new("null")
        .with("Rds ", D::UnitFloat { unit: *b"#Pxl", value: 5.0 })
        .with(
            "customShape",
            D::Descriptor(
                Descriptor::new("customShape")
                    .with("Nm  ", D::Text(UnicodeString::new_nul("Triangle")))
                    .with("Idnt", D::Text(UnicodeString::new_nul(SHAPE_ID))),
            ),
        )
        .with("keyOriginType", D::Integer(9))
}

fn encoded(d: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Shape"),
            items: vec![AtnItem { event: AtnEvent::name("shapeBlur"), descriptor: Some(d), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

fn fixture(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":25,"height":25,"depth":depth,"background":"#000000"})).unwrap();
    s.execute("paint.stroke", json!({"points":[[8,9],[16,13]],"size":3,"hardness":1,"color":"#ffffff"})).unwrap();
    s
}

fn install_square(s: &mut Session) {
    s.execute("shape.presets.new", json!({"name":"Original Square","id":SHAPE_ID,"path":"M 0 0 L 10 0 L 10 10 L 0 10 Z"})).unwrap();
    s.execute("shape.presets.edit", json!({"action":"rename","preset":"Original Square","name":"Renamed Square"})).unwrap();
}

fn pixels(s: &Session) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(&s.active().unwrap().doc).into_px()
}
fn saved(s: &Session) -> Vec<u8> {
    photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap()
}

/// A conflicting display name must not select a different kernel; playback remains undoable.
#[test]
fn custom_shape_uses_installed_id_despite_name_collision_and_undoes() {
    let original = descriptor();
    let imported = atn_cmds::import(&encoded(original.clone())).unwrap();
    assert!(imported.unsupported.is_empty());
    let (out, skipped) = atn_cmds::export("Shapes", &[&imported.actions[0]], &[], false).unwrap();
    assert!(skipped.is_empty());
    assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor, Some(original));
    for depth in [8, 16, 32] {
        let mut builtin = fixture(depth);
        builtin.execute("filter.blur.shapeBlur", json!({"radius":5,"shape":"square"})).unwrap();
        let expected = pixels(&builtin);
        let mut s = fixture(depth);
        install_square(&mut s);
        let before = saved(&s);
        s.actions.list = imported.actions.clone();
        assert_eq!(s.execute("actions.play", json!({"action":"Shape"})).unwrap()["ran"], 1);
        assert_eq!(pixels(&s), expected, "{depth}-bit square custom kernel");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(saved(&s), before);
    }
}

/// A saved Smart Filter carries geometry so reopening does not depend on ambient presets.
#[test]
fn custom_shape_smart_filter_keeps_geometry_across_save_without_preset_library() {
    let mut s = fixture(16);
    install_square(&mut s);
    s.execute("filter.convertForSmartFilters", json!({})).unwrap();
    s.execute("filter.blur.shapeBlur", json!({"radius":5,"shapeId":SHAPE_ID})).unwrap();
    let expected = pixels(&s);
    let doc = photocraft_format::load_from_bytes(&saved(&s)).unwrap();
    let filter = doc
        .walk()
        .into_iter()
        .find_map(|(_, _, l)| match &l.content {
            LayerContent::Smart(sm) => sm.smart_filters.first(),
            _ => None,
        })
        .unwrap();
    assert!(filter.params["shapePath"].is_object());
    let mut reopened = Session::new();
    assert!(!reopened.presets.shapes.iter().flat_map(|g| &g.items).any(|shape| shape.id.as_deref() == Some(SHAPE_ID)));
    reopened.add_document(doc, None);
    reopened.execute("layer.smartFilter.setVisible", json!({"index":0,"visible":false})).unwrap();
    assert_ne!(pixels(&reopened), expected);
    reopened.execute("layer.smartFilter.setVisible", json!({"index":0,"visible":true})).unwrap();
    assert_eq!(pixels(&reopened), expected);
}

/// Exercise the CSH-to-action bridge, including duplicate imports and durable ID lookup.
#[test]
fn imported_csh_id_resolves_in_an_action_after_reimport_and_preference_reload() {
    let mut s = fixture(16);
    let bytes = csh_fixture::triangle_csh();
    let id = photocraft_psd::csh::parse(&bytes).unwrap()[0].id.clone();
    let p = json!({"data":photocraft_paint::tile::b64_encode(&bytes)});
    s.execute("shape.presets.importCsh", p.clone()).unwrap();
    s.execute("shape.presets.importCsh", p).unwrap();
    let matches: Vec<_> = s.presets.shapes.iter().flat_map(|g| &g.items).filter(|shape| shape.id.as_deref() == Some(id.as_str())).collect();
    assert_eq!(matches.len(), 2, "both imports retain the source's opaque ID");
    let path = photocraft_engine::vector_cmds::path_json(&matches[0].path);
    let mut expected = fixture(16);
    expected.execute("filter.blur.shapeBlur", json!({"radius":5,"shapePath":path})).unwrap();
    let prefs = s.presets.to_json(&s);
    let mut s = fixture(16);
    s.load_presets_json(prefs);
    let shape =
        Descriptor::new("customShape").with("Nm  ", D::Text(UnicodeString::new_nul("Old shape name"))).with("Idnt", D::Text(UnicodeString::new_nul(&id)));
    let d = Descriptor::new("null")
        .with("Rds ", D::UnitFloat { unit: *b"#Pxl", value: 5.0 })
        .with("customShape", D::Descriptor(shape))
        .with("keyOriginType", D::Integer(9));
    let imported = atn_cmds::import(&encoded(d)).unwrap();
    assert!(imported.unsupported.is_empty());
    s.actions.list = imported.actions;
    let before = saved(&s);
    assert_eq!(s.execute("actions.play", json!({"action":"Shape"})).unwrap()["ran"], 1);
    assert_eq!(pixels(&s), pixels(&expected));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(saved(&s), before);
    // This original fixture has the same ID but a different path. Keep both presets
    // available by their unique names, while refusing ambiguous action playback.
    let other = csh_fixture::polygon_csh(&[&[(0.25, 0.25), (0.75, 0.25), (0.75, 0.75), (0.25, 0.75)]]);
    assert_eq!(photocraft_psd::csh::parse(&other).unwrap()[0].id, id);
    s.execute("shape.presets.importCsh", json!({"data":photocraft_paint::tile::b64_encode(&other)})).unwrap();
    let result = s.execute("actions.play", json!({"action":"Shape"})).unwrap();
    assert_eq!(result["ran"], 0);
    assert_eq!(result["failed"]["id"], "filter.blur.shapeBlur");
    assert!(result["failed"]["error"].as_str().unwrap().contains("different installed shapes"), "{result}");
    assert_eq!(saved(&s), before);
}

/// Admission failure must leave both pixels and the Undo history untouched.
#[test]
fn missing_or_invalid_custom_shapes_refuse_before_document_or_history_changes() {
    let mut s = fixture(8);
    let before = saved(&s);
    let history = s.active().unwrap().history.past_len();
    for p in [
        json!({"radius":5,"shapeId":"missing","shapeName":"Triangle"}),
        json!({"radius":5,"shapeId":""}),
        json!({"radius":5,"shapeName":false}),
        json!({"radius":5,"shapePath":{"subpaths":[]}}),
        json!({"radius":5,"shapePath":{"subpaths":[{"closed":true,"knots":[[0,0],[1,0],[2,0]]}]}}),
        json!({"radius":5,"shapePath":{"subpaths":[{"knots":[[-1.7e308,0],[1.7e308,0],[0,1.7e308]]}]}}),
        json!({"radius":5,"shapeOriginType":8}),
    ] {
        assert!(s.execute("filter.blur.shapeBlur", p.clone()).is_err(), "{p}");
        assert_eq!(saved(&s), before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
    let imported = atn_cmds::import(&encoded(descriptor())).unwrap();
    s.actions.list = imported.actions;
    // Playback reports a failed step in its successful response; native command errors are not outer errors.
    let result = s.execute("actions.play", json!({"action":"Shape"})).unwrap();
    assert_eq!(result["ran"], 0);
    assert_eq!(result["failed"]["id"], "filter.blur.shapeBlur");
    assert!(result["failed"]["error"].as_str().unwrap().contains("no installed shape"), "{result}");
    assert_eq!(saved(&s), before);
    assert_eq!(s.active().unwrap().history.past_len(), history);
}

/// New IDs survive preferences serialization without requiring them in older stored presets.
#[test]
fn shape_ids_persist_and_old_preferences_without_ids_still_load() {
    let mut s = Session::new();
    install_square(&mut s);
    let prefs = s.prefs_to_json();
    let mut reopened = Session::new();
    reopened.load_prefs_json(&prefs).unwrap();
    assert!(reopened.presets.shapes.iter().flat_map(|g| &g.items).any(|shape| shape.id.as_deref() == Some(SHAPE_ID) && shape.name == "Renamed Square"));
    let before = reopened.prefs_to_json();
    assert!(reopened.execute("shape.presets.new", json!({"id":SHAPE_ID,"path":"M 0 0 L 10 0 L 5 10 Z"})).is_err());
    assert_eq!(reopened.prefs_to_json(), before);
    let mut old: serde_json::Value = serde_json::from_str(&prefs).unwrap();
    for group in old["presets"]["shapes"].as_array_mut().unwrap() {
        for shape in group["items"].as_array_mut().unwrap() {
            shape.as_object_mut().unwrap().remove("id");
        }
    }
    reopened.load_prefs_json(&old.to_string()).unwrap();
    assert!(reopened.presets.shapes.iter().flat_map(|g| &g.items).all(|shape| shape.id.is_none()));
}

/// Neither unknown recording settings nor portable native geometry may be dropped during export.
#[test]
fn unrepresented_shape_fields_stay_whole_and_native_geometry_refuses_atn_export() {
    for (key, value) in [
        ("keyOriginType", D::Integer(8)),
        ("Rds ", D::Double(5.0)),
        ("customShape", D::Descriptor(Descriptor::new("other").with("Idnt", D::Text(UnicodeString::new_nul(SHAPE_ID))))),
        ("extra", D::Boolean(true)),
    ] {
        let mut d = descriptor();
        d.items.retain(|(k, _)| !k.is(key));
        d.items.push((Id::new(key), value));
        let imported = atn_cmds::import(&encoded(d.clone())).unwrap();
        assert_eq!(imported.actions[0].steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        let (out, _) = atn_cmds::export("Shapes", &[&imported.actions[0]], &[], false).unwrap();
        assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor, Some(d));
    }
    let imported = atn_cmds::import(&encoded(descriptor())).unwrap();
    for (key, value) in [("shapePath", json!({"subpaths":[]})), ("shape", json!("star")), ("target", json!("mask")), ("shapeId", json!(""))] {
        let mut a = imported.actions[0].clone();
        a.steps[0].1[key] = value;
        assert!(atn_cmds::export("Shapes", &[&a], &[], false).is_err());
    }
}
