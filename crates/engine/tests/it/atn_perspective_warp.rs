//! The measured Perspective Warp descriptor shape through ATN conversion and action playback.

use craft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D},
};
use photocraft_doc::{LayerContent, LayerId};
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use photocraft_geom::Rect;
use serde_json::{Value, json};

const EVENT: &str = "perspectiveWarpTransform";
const COMMAND: &str = "edit.perspectiveWarp";

fn px(value: f64) -> D {
    D::UnitFloat { unit: *b"#Pxl", value }
}

fn points(points: &[[f64; 2]]) -> D {
    D::List(points.iter().map(|&[x, y]| D::Descriptor(Descriptor::new("Pnt ").with("Hrzn", px(x)).with("Vrtc", px(y)))).collect())
}

fn quad(indices: [i32; 4]) -> D {
    D::Descriptor(Descriptor::new("perspectiveWarpQuad").with("indices", D::List(indices.into_iter().map(D::Integer).collect())))
}

/// Two planes sharing a seam. Vertices appear out of corner order to distinguish exact export.
fn fixture(name: Option<&str>) -> Descriptor {
    let class = Class { class_id: Id::new("Lyr "), name: UnicodeString::new_nul("") };
    let target = match name {
        Some(name) => ReferenceItem::Name { class, name: UnicodeString::new_nul(name) },
        None => ReferenceItem::Enumerated { class, type_id: Id::new("Ordn"), value: Id::new("Trgt") },
    };
    let rect = Descriptor::new("Rctn").with("Top ", px(0.0)).with("Left", px(0.0)).with("Btom", px(24.0)).with("Rght", px(32.0));
    Descriptor::new(EVENT)
        .with("null", D::Reference(vec![target]))
        .with("Md  ", D::Enumerated { type_id: Id::new("perspectiveWarpMode"), value: Id::new("warp") })
        .with("referenceRect", D::Descriptor(rect))
        .with("vertices", points(&[[16.0, 20.0], [4.0, 4.0], [28.0, 4.0], [4.0, 20.0], [16.0, 4.0], [28.0, 20.0]]))
        .with("warpedVertices", points(&[[17.0, 20.0], [5.5, 5.0], [27.0, 6.0], [4.0, 19.0], [18.0, 4.0], [26.0, 17.0]]))
        .with("quads", D::List(vec![quad([1, 4, 0, 3]), quad([4, 2, 5, 0])]))
}

fn imported(d: Descriptor, with_dialog: bool, enabled: bool) -> atn_cmds::Imported {
    let item =
        AtnItem { event: AtnEvent::name(EVENT), dictionary: b"Perspective Warp".to_vec(), descriptor: Some(d), with_dialog, enabled, ..Default::default() };
    let file = AtnFile {
        name: UnicodeString::new_nul("Set"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("Warp"), items: vec![item], ..Default::default() }],
        ..Default::default()
    };
    atn_cmds::import(&atn::write(&file).unwrap()).unwrap()
}

fn saved(action: &Action) -> AtnItem {
    let (bytes, skipped) = atn_cmds::export("Set", &[action], &[], false).unwrap();
    assert!(skipped.is_empty());
    atn::parse(&bytes).unwrap().actions[0].items[0].clone()
}

fn planes() -> Value {
    json!([
        {"src":[[4,4],[16,4],[16,20],[4,20]],"dst":[[5.5,5],[18,4],[17,20],[4,19]]},
        {"src":[[16,4],[28,4],[28,20],[16,20]],"dst":[[18,4],[27,6],[26,17],[17,20]]}
    ])
}

fn value_mut<'a>(d: &'a mut Descriptor, key: &str) -> &'a mut D {
    &mut d.items.iter_mut().find(|(id, _)| id.is(key)).unwrap().1
}

/// Exact export preserves the original indexed mesh, target, rectangle and step flags.
#[test]
fn descriptor_round_trip_preserves_shared_indices_and_flags() {
    for name in [None, Some("Façade")] {
        let d = fixture(name);
        let loaded = imported(d.clone(), true, false);
        assert!(loaded.unsupported.is_empty());
        let action = &loaded.actions[0];
        assert_eq!(action.steps[0].0, COMMAND);
        let params = &action.steps[0].1;
        assert_eq!(params["planes"], planes());
        assert_eq!(params["layer"].as_str(), name);
        assert_eq!(params["target"], "pixels");
        assert_eq!(params["__atnPerspectiveWarpRect"], json!([0, 0, 32, 24]));
        let item = saved(action);
        assert_eq!(item.event.id(), EVENT);
        assert_eq!(item.descriptor.unwrap().to_bytes(), d.to_bytes());
        assert!(item.with_dialog);
        assert!(!item.enabled);
    }
}

/// Unsupported targets, units, modes and geometry retain the original bytes as kept steps.
#[test]
fn unsupported_recordings_are_kept_without_partial_mapping() {
    let d = fixture(None);
    let mut layout = d.clone();
    *value_mut(&mut layout, "Md  ") = D::Enumerated { type_id: Id::new("perspectiveWarpMode"), value: Id::new("layout") };
    let mut wrong_target = d.clone();
    *value_mut(&mut wrong_target, "null") = D::Reference(vec![ReferenceItem::Class(Class { class_id: Id::new("Chnl"), name: UnicodeString::new_nul("") })]);
    let mut bad_index = d.clone();
    *value_mut(&mut bad_index, "quads") = D::List(vec![quad([1, 4, 99, 3])]);
    let mut wrong_units = d.clone();
    let D::Descriptor(rect) = value_mut(&mut wrong_units, "referenceRect") else { panic!("reference rectangle") };
    *value_mut(rect, "Rght") = D::UnitFloat { unit: *b"#Prc", value: 32.0 };
    let mut incompatible_topology = d.clone();
    for key in ["vertices", "warpedVertices"] {
        let D::List(points) = value_mut(&mut incompatible_topology, key) else { panic!("vertex list") };
        points.push(points[4].clone());
    }
    *value_mut(&mut incompatible_topology, "quads") = D::List(vec![quad([1, 4, 0, 3]), quad([6, 2, 5, 0])]);
    let mut duplicate = d.clone();
    duplicate.items.push((Id::new("quads"), d.get("quads").unwrap().clone()));
    for descriptor in [layout, wrong_target, bad_index, wrong_units, incompatible_topology, duplicate] {
        let loaded = imported(descriptor.clone(), false, true);
        assert_eq!(loaded.unsupported.len(), 1, "{descriptor:?}");
        assert_eq!(loaded.actions[0].steps[0].0, "actions.photoshopStep");
        assert_eq!(saved(&loaded.actions[0]).descriptor.unwrap().to_bytes(), descriptor.to_bytes());
    }
}

/// Native export needs a reference rectangle. A stale recorded topology must not be substituted.
#[test]
fn native_export_requires_known_rectangle_and_rejects_stale_source() {
    let mut action = Action::new("Warp", vec![(COMMAND.into(), json!({"planes": planes()}))]);
    assert!(atn_cmds::export("Set", &[&action], &[], false).is_err());
    action.steps[0].1["__atnPerspectiveWarpRect"] = json!([0, 0, 32, 24]);
    let d = saved(&action).descriptor.unwrap();
    let keys: Vec<_> = d.items.iter().map(|(id, _)| id.as_bytes()).collect();
    assert_eq!(keys, [b"null".as_slice(), b"Md  ", b"referenceRect", b"vertices", b"warpedVertices", b"quads"]);
    let mut recorded = imported(d, false, true).actions.remove(0);
    recorded.steps[0].1["planes"][0]["dst"][0][0] = json!(6);
    assert!(atn_cmds::export("Set", &[&recorded], &[], false).is_err());
    action.steps[0].1["interpolation"] = json!("nearest");
    assert!(atn_cmds::export("Set", &[&action], &[], false).is_err());
}

fn session(depth: u8) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":32,"height":24,"depth":depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("Fixture", |doc, _| {
        let layer = doc.layer_mut(id).unwrap();
        layer.name = "Facade".into();
        let surf = layer.surface_mut().unwrap();
        let fmt = surf.format();
        surf.fill_rect(Rect::new(4, 4, 16, 20), &photocraft_raster::from_rgba(&fmt, [0.9, 0.2, 0.1, 1.0]));
        surf.fill_rect(Rect::new(16, 4, 28, 20), &photocraft_raster::from_rgba(&fmt, [0.1, 0.2, 0.9, 1.0]));
        Ok(())
    })
    .unwrap();
    (s, id)
}

/// A named raster is warped, while the active decoy stays unchanged.
#[test]
fn action_playback_matches_the_native_raster_warp() {
    for depth in [8, 16, 32] {
        let (mut actual, id) = session(depth);
        let (mut direct, did) = session(depth);
        let before = actual.active().unwrap().doc.layer(id).unwrap().surface().unwrap().clone();
        for s in [&mut actual, &mut direct] {
            s.execute("layer.new.layer", json!({})).unwrap();
        }
        let decoy = actual.active().unwrap().active_layer.unwrap();
        let decoy_before = actual.active().unwrap().doc.layer(decoy).unwrap().clone();
        actual.actions.list.extend(imported(fixture(Some("Facade")), false, true).actions);
        actual.execute("actions.play", json!({"action":0})).unwrap();
        direct.execute(COMMAND, json!({"planes":planes(),"layer":"Facade","target":"pixels"})).unwrap();
        let result = actual.active().unwrap().doc.layer(id).unwrap().surface().unwrap();
        assert_ne!(*result, before);
        assert_eq!(result, direct.active().unwrap().doc.layer(did).unwrap().surface().unwrap());
        assert_eq!(actual.active().unwrap().doc.layer(decoy), Some(&decoy_before));
        actual.execute("edit.undo", json!({})).unwrap();
        assert_eq!(actual.active().unwrap().doc.layer(id).unwrap().surface(), Some(&before));
    }
}

/// The same active-layer recording adds a native Smart Filter and keeps PSD export available.
#[test]
fn active_smart_object_playback_matches_the_native_filter() {
    let (mut actual, _) = session(8);
    let (mut direct, _) = session(8);
    for s in [&mut actual, &mut direct] {
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    }
    actual.actions.list.extend(imported(fixture(None), false, true).actions);
    actual.execute("actions.play", json!({"action":0})).unwrap();
    direct.execute("layer.smartObjects.perspectiveWarp", json!({"planes":planes()})).unwrap();
    let [actual, direct] = [&actual, &direct].map(|s| {
        let st = s.active().unwrap();
        let LayerContent::Smart(sm) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("smart object") };
        sm
    });
    assert_eq!(actual.cache, direct.cache);
    assert_eq!(actual.smart_filters.len(), 1);
    assert_eq!(actual.smart_filters[0].command, COMMAND);
    assert_eq!(actual.smart_filters[0].params["planes"], planes());
    assert!(photocraft_io::smart_map::item_for_filter(&actual.smart_filters[0]).is_ok());
}
