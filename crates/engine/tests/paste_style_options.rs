//! Paste Layer Style applies group eligibility consistently and preserves the ATN flag.

use craft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Class, Descriptor, Id, UnicodeString, Value as D},
};
use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{BlendRange, Document, Effect, Layer, LayerId};
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds, layer_multi_cmds};
use photocraft_geom::Size;
use serde_json::{Value, json};

const CMD: &str = "layer.layerStyle.pasteLayerStyle";
const KEY: &str = "allowPasteFXOnLayerSet";

struct Scene {
    s: Session,
    source: LayerId,
    raster: LayerId,
    group: LayerId,
    other_group: LayerId,
    child: LayerId,
    background: LayerId,
}

/// The copied style contains effects and blending data, so a partial copy cannot pass.
fn fixture() -> Scene {
    let mut doc = Document::with_background("Style", Size::new(8, 6), ColorMode::Rgb, SampleType::U8, Color::rgb(1.0, 1.0, 1.0));
    let background = doc.layers[0].id;
    let mut source = Layer::raster("Source", doc.pixel_format());
    source.effects.items.push(Effect::default_drop_shadow());
    source.blend = BlendMode::Multiply;
    source.fill_opacity = 0.6;
    source.advanced.blend_interior = true;
    source.blend_if.set(0, [BlendRange::from_bytes([12, 24, 230, 240]), BlendRange::FULL]);
    source.excluded_channels = 2;
    let raster = Layer::raster("Raster", doc.pixel_format());
    let child = Layer::raster("Child", doc.pixel_format());
    let child_id = child.id;
    let group = Layer::group("Group", vec![child]);
    let other = Layer::group("Other group", vec![]);
    let ids = (source.id, raster.id, group.id, other.id);
    doc.layers.extend([source, raster, group, other]);
    let mut s = Session::new();
    s.add_document(doc, None);
    s.select_layer(ids.0).unwrap();
    s.execute("layer.layerStyle.copyLayerStyle", json!({})).unwrap();
    Scene { s, source: ids.0, raster: ids.1, group: ids.2, other_group: ids.3, child: child_id, background }
}

fn select(scene: &mut Scene, ids: Vec<LayerId>) {
    let active = ids.last().copied();
    layer_multi_cmds::set_selection(&mut scene.s, ids, active, active).unwrap();
}

fn layer(s: &Session, id: LayerId) -> &Layer {
    s.active().unwrap().doc.layer(id).unwrap()
}

fn assert_style(actual: &Layer, source: &Layer) {
    assert_eq!(actual.effects, source.effects);
    assert_eq!(actual.blend, source.blend);
    assert_eq!(actual.fill_opacity, source.fill_opacity);
    assert_eq!(actual.advanced, source.advanced);
    assert_eq!(actual.blend_if, source.blend_if);
    assert_eq!(actual.excluded_channels, source.excluded_channels);
}

/// Refusals must not consume style data, modify selection, or remove a redo branch.
fn assert_rejected(s: &mut Session, params: Value) {
    let state = s.active().unwrap();
    let before = state.doc.clone();
    let revision = state.revision;
    let selected = state.selected_layers.clone();
    let active = state.active_layer;
    let history = (state.history.past_len(), state.history.undo_label().map(str::to_owned), state.history.redo_label().map(str::to_owned));
    let clipboard = s.style_clipboard.as_ref().unwrap().effects.clone();
    assert!(s.execute(CMD, params).is_err(), "group restriction must refuse an ineligible target");
    let state = s.active().unwrap();
    assert_eq!(state.doc, before);
    assert_eq!(state.revision, revision);
    assert_eq!(state.selected_layers, selected);
    assert_eq!(state.active_layer, active);
    assert_eq!((state.history.past_len(), state.history.undo_label().map(str::to_owned), state.history.redo_label().map(str::to_owned)), history);
    assert_eq!(s.style_clipboard.as_ref().unwrap().effects, clipboard);
}

/// Every target cardinality uses the same restriction; group selection never expands children.
#[test]
fn false_excludes_explicit_single_and_all_group_targets_atomically() {
    for explicit in [false, true] {
        let mut scene = fixture();
        let group = scene.group;
        select(&mut scene, vec![group]);
        let mut params = json!({"allowGroups": false});
        if explicit {
            params["layer"] = json!(group.0);
        }
        assert_rejected(&mut scene.s, params);
    }
    for background in [false, true] {
        let mut scene = fixture();
        let ids = vec![scene.group, if background { scene.background } else { scene.other_group }];
        select(&mut scene, ids);
        assert_rejected(&mut scene.s, json!({"allowGroups": false}));
    }
}

/// Mixed selections change only eligible explicit members and create one undoable edit.
#[test]
fn mixed_targets_skip_groups_without_expanding_their_children() {
    for selected_child in [false, true] {
        let mut scene = fixture();
        let eligible = if selected_child { scene.child } else { scene.raster };
        let group = scene.group;
        select(&mut scene, vec![group, eligible]);
        let before = scene.s.active().unwrap().doc.clone();
        let history = scene.s.active().unwrap().history.past_len();
        scene.s.execute(CMD, json!({"allowGroups": false})).unwrap();
        assert_style(layer(&scene.s, eligible), layer(&scene.s, scene.source));
        assert_style(layer(&scene.s, group), before.layer(group).unwrap());
        if !selected_child {
            assert_eq!(layer(&scene.s, scene.child), before.layer(scene.child).unwrap());
        }
        assert_eq!(scene.s.active().unwrap().history.past_len(), history + 1);
        let after = scene.s.active().unwrap().doc.clone();
        scene.s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(scene.s.active().unwrap().doc, before);
        scene.s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(scene.s.active().unwrap().doc, after);
    }
}

/// Native legacy/default group support and explicit Background conversion remain available.
#[test]
fn defaults_and_background_keep_legacy_behavior_and_wrong_types_preserve_redo() {
    for value in [None, Some(Value::Null), Some(json!(true))] {
        let mut scene = fixture();
        let child = layer(&scene.s, scene.child).clone();
        let mut p = json!({"layer": scene.group.0});
        if let Some(value) = value {
            p["allowGroups"] = value;
        }
        scene.s.execute(CMD, p).unwrap();
        assert_style(layer(&scene.s, scene.group), layer(&scene.s, scene.source));
        assert_eq!(layer(&scene.s, scene.child), &child);
    }
    let mut scene = fixture();
    scene.s.execute(CMD, json!({"layer": scene.background.0, "allowGroups": false})).unwrap();
    assert_style(layer(&scene.s, scene.background), layer(&scene.s, scene.source));
    assert!(!scene.s.active().unwrap().doc.is_background(scene.background));
    for value in [json!(0), json!("false"), json!([]), json!({})] {
        let mut scene = fixture();
        scene.s.execute("layer.new.layer", json!({})).unwrap();
        scene.s.execute("edit.undo", json!({})).unwrap();
        assert!(scene.s.active().unwrap().history.can_redo());
        assert_rejected(&mut scene.s, json!({"layer": scene.raster.0, "allowGroups": value}));
    }
}

fn recorded(d: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Style"),
        expanded: false,
        warnings: vec![],
        encoding: None,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Paste"),
            items: vec![AtnItem {
                event: AtnEvent::name("pasteEffects"),
                dictionary: b"Paste Layer Style".to_vec(),
                descriptor: Some(d),
                ..Default::default()
            }],
            ..Default::default()
        }],
    })
    .unwrap()
}

fn load(s: &mut Session, bytes: &[u8]) -> Value {
    s.execute("actions.load", json!({"data": photocraft_engine::paint::tile::b64_encode(bytes)})).unwrap()
}

fn save(s: &mut Session) -> Vec<u8> {
    let result = s.execute("actions.save", json!({"set": "Style"})).unwrap();
    assert_eq!(result["skipped"], json!([]));
    photocraft_engine::paint::tile::b64_decode(result["data"].as_str().unwrap()).unwrap()
}

/// Inspect typed saved fields, then play against selected groups and pixels to expose dropped flags.
#[test]
fn atn_group_flag_survives_saved_descriptor_and_public_playback() {
    for allow in [false, true] {
        let mut scene = fixture();
        let ids = vec![scene.group, scene.raster];
        select(&mut scene, ids);
        let group_before = layer(&scene.s, scene.group).clone();
        let child_before = layer(&scene.s, scene.child).clone();
        let bytes = recorded(Descriptor::new("null").with(KEY, D::Boolean(allow)));
        assert_eq!(load(&mut scene.s, &bytes)["unsupported"], json!([]));
        assert_eq!(scene.s.actions.list[0].steps, vec![(CMD.into(), json!({"allowGroups": allow}))]);
        let saved = save(&mut scene.s);
        let parsed = atn::parse(&saved).unwrap();
        assert_eq!(parsed.actions[0].items[0].descriptor.as_ref().unwrap().get(KEY), Some(&D::Boolean(allow)));
        let imported = atn_cmds::import(&saved).unwrap();
        assert_eq!(imported.actions[0].steps, scene.s.actions.list[0].steps);
        scene.s.actions.list = imported.actions;
        let result = scene.s.execute("actions.play", json!({"action": "Paste"})).unwrap();
        assert_eq!(result["ran"], 1);
        assert!(result.get("failed").is_none(), "{result}");
        assert_style(layer(&scene.s, scene.raster), layer(&scene.s, scene.source));
        if allow {
            assert_style(layer(&scene.s, scene.group), layer(&scene.s, scene.source));
        } else {
            assert_eq!(layer(&scene.s, scene.group), &group_before);
        }
        assert_eq!(layer(&scene.s, scene.child), &child_before);
    }
}

/// Omission retains legacy group eligibility and exports its effective Boolean true.
#[test]
fn atn_defaults_are_explicit_and_unsupported_fields_remain_whole_steps() {
    let mut scene = fixture();
    assert_eq!(load(&mut scene.s, &recorded(Descriptor::new("null")))["unsupported"], json!([]));
    assert_eq!(scene.s.actions.list[0].steps[0].1, json!({"allowGroups": true}));
    for value in [None, Some(Value::Null)] {
        let mut params = json!({});
        if let Some(value) = value {
            params["allowGroups"] = value;
        }
        let action = Action::new("Defaults", vec![(CMD.into(), params)]);
        let (bytes, skipped) = atn_cmds::export("Style", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor.as_ref().unwrap().get(KEY), Some(&D::Boolean(true)));
    }
    let bad = [
        Descriptor::new("null").with(KEY, D::Integer(1)),
        Descriptor::new("null").with(KEY, D::Text(UnicodeString::new_nul("true"))),
        Descriptor::new("null").with("null", D::Class(Class { name: UnicodeString::new_nul(""), class_id: Id::new("Lyr ") })),
    ];
    for d in bad {
        let bytes = recorded(d);
        let original = atn::parse(&bytes).unwrap();
        let mut s = Session::new();
        assert_eq!(load(&mut s, &bytes)["unsupported"].as_array().unwrap().len(), 1);
        assert_eq!(s.actions.list[0].steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        let saved = atn::parse(&save(&mut s)).unwrap();
        assert_eq!(saved.actions[0].items[0].descriptor, original.actions[0].items[0].descriptor);
    }
    for (key, value) in [
        ("allowGroups", json!(1)),
        ("allowGroups", json!("true")),
        ("allowGroups", json!([])),
        ("layer", json!(17)),
        ("layer", Value::Null),
        ("layers", json!([17])),
        ("layers", Value::Null),
    ] {
        let mut params = json!({});
        params[key] = value;
        let action = Action::new("Invalid", vec![(CMD.into(), params)]);
        assert!(atn_cmds::export("Style", &[&action], &[], false).is_err());
    }
}
