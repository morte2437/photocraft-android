//! Measured layer-style action steps retain their settings and use the existing native commands.
use craft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Class, Descriptor, Id, MAX_DEPTH, ObjectArray, ReferenceItem, UnicodeString, Value as D},
};
use photocraft_doc::{Document, Effect, LayerId};
use photocraft_engine::{
    Session,
    actions_cmds::Action,
    atn_cmds::{export, import},
};
use serde_json::{Value, json};

const SCALE: &str = "layer.layerStyle.scaleEffects";
const CREATE: &str = "layer.layerStyle.createLayer";

fn class(id: &str) -> Class {
    Class { name: UnicodeString::default(), class_id: Id::new(id) }
}

fn scale(value: f64) -> Descriptor {
    Descriptor::new("null").with("Scl ", D::UnitFloat { unit: *b"#Prc", value })
}

fn create() -> Descriptor {
    Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Class(class("Lyr "))])).with(
        "Usng",
        D::Reference(vec![
            ReferenceItem::Property { class: class("Prpr"), key: Id::new("Lefx") },
            ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
        ]),
    )
}

fn item(id: &str, descriptor: Descriptor) -> AtnItem {
    let (event, name) = if id == SCALE { (AtnEvent::name("scaleEffectsEvent"), "Scale Effects") } else { (AtnEvent::Code(*b"Mk  "), "Make") };
    AtnItem { event, dictionary: name.as_bytes().to_vec(), descriptor: Some(descriptor), ..Default::default() }
}

fn file(items: Vec<AtnItem>) -> AtnFile {
    AtnFile {
        name: UnicodeString::new_nul("S"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        ..Default::default()
    }
}

fn action(items: Vec<AtnItem>) -> Action {
    let loaded = import(&atn::write(&file(items)).unwrap()).unwrap();
    assert!(loaded.unsupported.is_empty());
    loaded.actions.into_iter().next().unwrap()
}

fn exported(id: &str, params: Value) -> Option<AtnItem> {
    let a = Action::new("A", vec![(id.into(), params)]);
    let bytes = export("S", &[&a], &[], false).ok()?.0;
    atn::parse(&bytes).ok()?.actions.into_iter().next()?.items.into_iter().next()
}

/// Wire bytes also compare preserved NaN payloads, whose floating values are not reflexively equal.
fn same_item_wire(mut actual: AtnItem, mut expected: AtnItem) {
    assert_eq!(actual.descriptor.take().map(|d| d.to_bytes()), expected.descriptor.take().map(|d| d.to_bytes()));
    assert_eq!(actual, expected);
}

/// The direct control must actually scale the known one-pixel shadow, not merely accept the call.
fn scaled_shadow(s: &Session, source: LayerId, percent: f64) {
    let shadow = s
        .active()
        .unwrap()
        .doc
        .layer(source)
        .unwrap()
        .effects
        .items
        .iter()
        .find_map(|effect| match effect {
            Effect::DropShadow(shadow) => Some(shadow),
            _ => None,
        })
        .unwrap();
    let factor = percent as f32 / 100.0;
    assert_eq!((shadow.distance, shadow.size), (factor, factor));
}

/// A small painted source with two effects makes a real conversion differ from a blank layer.
fn fixture(depth: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":8,"depth":depth,"background":"transparent"})).unwrap();
    s.execute("layer.new.layer", json!({"name":"Source"})).unwrap();
    let source = s.active().unwrap().active_layer.unwrap();
    s.edit("Synthetic pixels", |doc, _| {
        let pixels: Vec<f32> =
            (0..8).flat_map(|y| (0..8).flat_map(move |x| if (2..6).contains(&x) && (2..6).contains(&y) { [0.2, 0.5, 0.9, 1.0] } else { [0.0; 4] })).collect();
        let bounds = doc.bounds();
        doc.layer_mut(source).unwrap().surface_mut().unwrap().write_region(bounds, &pixels);
        Ok(())
    })
    .unwrap();
    s.execute("layer.layerStyle.dropShadow", json!({"blend":"normal","distance":1,"size":1,"opacity":60})).unwrap();
    s.execute("layer.layerStyle.colorOverlay", json!({"color":"#ff8000","opacity":50,"add":true})).unwrap();
    (s, source)
}

/// Compare each complete raster and its style/stack state without assuming cross-session IDs.
fn equivalent(actual: &Session, expected: &Session) {
    let a = &actual.active().unwrap().doc;
    let b = &expected.active().unwrap().doc;
    assert_eq!(a.layers.len(), b.layers.len());
    assert_eq!(a.selection, b.selection);
    for (a, b) in a.layers.iter().zip(&b.layers) {
        assert_eq!((&a.name, a.clipped, a.blend, a.opacity), (&b.name, b.clipped, b.blend, b.opacity));
        assert_eq!(a.effects, b.effects);
        let (a, b) = (a.surface().unwrap(), b.surface().unwrap());
        assert_eq!(a.content_bounds(), b.content_bounds());
        let bounds = a.content_bounds().union(&b.content_bounds());
        assert_eq!(a.read_region(bounds), b.read_region(bounds));
    }
}

/// Undo the expected playback history edits, then redo the complete document state.
fn undo_redo(s: &mut Session, before: &Document, after: &Document, edits: usize) {
    for _ in 0..edits {
        assert!(s.undo());
    }
    assert_eq!(s.active().unwrap().doc.as_ref(), before);
    for _ in 0..edits {
        assert!(s.redo());
    }
    assert_eq!(s.active().unwrap().doc.as_ref(), after);
}

/// Playback must produce actual effect layers, not the blank layer the old Make branch creates.
#[test]
fn create_layers_playback_matches_native_effect_layers_and_undo_at_each_depth() {
    for depth in [8, 16, 32] {
        let (mut native, native_source) = fixture(depth);
        let native_before = native.active().unwrap().doc.clone();
        native.execute(CREATE, json!({})).unwrap();
        let native_after = native.active().unwrap().doc.clone();
        assert_eq!(native_after.layers.len(), native_before.layers.len() + 2);
        assert!(native_after.layer(native_source).unwrap().effects.items.is_empty());
        undo_redo(&mut native, &native_before, &native_after, 1);

        let (mut recorded, source) = fixture(depth);
        let before = recorded.active().unwrap().doc.clone();
        let past = recorded.active().unwrap().history.past_len();
        recorded.actions.list.push(action(vec![item(CREATE, create())]));
        let result = recorded.execute("actions.play", json!({"action":0})).unwrap();
        assert_eq!(result["ran"], 1);
        assert!(result.get("failed").is_none(), "{depth}: {result}");
        let after = recorded.active().unwrap().doc.clone();
        assert_eq!(after.layers.len(), before.layers.len() + 2, "{depth}: Create Layers must split two effects, not create one blank layer");
        assert!(after.layer(source).unwrap().effects.items.is_empty());
        let at = after.layers.iter().position(|l| l.id == source).unwrap();
        assert!(after.layers[at - 1].name.ends_with("Drop Shadow") && !after.layers[at - 1].clipped);
        assert!(after.layers[at + 1].name.ends_with("Color Overlay") && after.layers[at + 1].clipped);
        assert_eq!(recorded.active().unwrap().history.past_len(), past + 1);
        equivalent(&recorded, &native);
        undo_redo(&mut recorded, &before, &after, 1);
    }
}

/// The recorded whole-percent limits and interior values match native effect scaling at all depths.
#[test]
fn scale_effects_playback_matches_native_settings_and_undo_at_each_depth() {
    for single_state in [false, true] {
        for depth in [8, 16, 32] {
            for percent in [1.0, 100.0, 200.0, 1000.0] {
                let (mut native, native_source) = fixture(depth);
                let native_past = native.active().unwrap().history.past_len();
                native.execute(SCALE, json!({"scale":percent})).unwrap();
                scaled_shadow(&native, native_source, percent);
                let edits = native.active().unwrap().history.past_len() - native_past;
                let (mut recorded, _) = fixture(depth);
                recorded.execute("actions.setHistory", json!({"singleState":single_state})).unwrap();
                let before = recorded.active().unwrap().doc.clone();
                let past = recorded.active().unwrap().history.past_len();
                recorded.actions.list.push(action(vec![item(SCALE, scale(percent))]));
                let result = recorded.execute("actions.play", json!({"action":0})).unwrap();
                assert_eq!(result["ran"], 1);
                assert!(result.get("failed").is_none(), "{depth}/{percent}: {result}");
                let after = recorded.active().unwrap().doc.clone();
                // Grouped playback omits the unchanged 100% scale; per-step mode keeps native rows.
                let playback_edits = if single_state { usize::from(percent != 100.0) } else { edits };
                assert_eq!(recorded.active().unwrap().history.past_len(), past + playback_edits, "singleState={single_state}, depth={depth}, scale={percent}");
                equivalent(&recorded, &native);
                undo_redo(&mut recorded, &before, &after, playback_edits);
            }
        }
    }
}

/// Scaling then splitting keeps native pixels with one grouped or two separately reversible edits.
#[test]
fn scale_then_create_layers_matches_native_sequence() {
    for single_state in [false, true] {
        let (mut native, _) = fixture(16);
        native.execute(SCALE, json!({"scale":200})).unwrap();
        native.execute(CREATE, json!({})).unwrap();
        let (mut recorded, _) = fixture(16);
        recorded.execute("actions.setHistory", json!({"singleState":single_state})).unwrap();
        let before = recorded.active().unwrap().doc.clone();
        let past = recorded.active().unwrap().history.past_len();
        recorded.actions.list.push(action(vec![item(SCALE, scale(200.0)), item(CREATE, create())]));
        let result = recorded.execute("actions.play", json!({"action":0})).unwrap();
        assert_eq!(result["ran"], 2);
        assert!(result.get("failed").is_none());
        let edits = if single_state { 1 } else { 2 };
        assert_eq!(recorded.active().unwrap().history.past_len(), past + edits);
        let after = recorded.active().unwrap().doc.clone();
        equivalent(&recorded, &native);
        if !single_state {
            // Per-step playback must also preserve the native intermediate scaled-effects state.
            assert!(native.undo());
            assert!(recorded.undo());
            equivalent(&recorded, &native);
            assert!(native.redo());
            assert!(recorded.redo());
        }
        undo_redo(&mut recorded, &before, &after, edits);
    }
}

/// Unedited wire identity/order/flags survive; a supported scale edit exports its new value.
#[test]
fn layer_style_descriptors_roundtrip_and_edited_scale_does_not_revive_source() {
    for (id, mut d) in [(SCALE, scale(100.0)), (CREATE, create())] {
        d.name = UnicodeString::new_nul("Recorded style step");
        d.class_id = Id::Str(b"null".to_vec());
        d.items.reverse();
        let mut source = item(id, d);
        source.enabled = false;
        source.with_dialog = true;
        source.dialog_options = 2;
        let a = action(vec![source.clone()]);
        assert_eq!(a.steps[0].0, id);
        let bytes = export("S", &[&a], &[], false).unwrap().0;
        let saved = atn::parse(&bytes).unwrap().actions.into_iter().next().unwrap().items;
        assert_eq!(saved.len(), 1);
        same_item_wire(saved.into_iter().next().unwrap(), source);
        let mut damaged = a.steps[0].1.clone();
        damaged["_atnLayerStyleDescriptor"] = json!("not base64");
        assert!(exported(id, damaged).is_none());
    }
    let mut d = scale(100.0);
    d.name = UnicodeString::new_nul("Original");
    let mut a = action(vec![item(SCALE, d)]);
    a.steps[0].1["scale"] = json!(200);
    for skip in [false, true] {
        let (bytes, skipped) = export("S", &[&a], &[], skip).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor, Some(scale(200.0)));
    }
}

/// Native defaults are explicit; unrepresented targets, options and damaged metadata cannot vanish.
#[test]
fn layer_style_native_exports_are_strict_and_make_defaults_explicit() {
    assert_eq!(exported(SCALE, json!({})).unwrap().descriptor, Some(scale(100.0)));
    let mut canonical_create = create();
    // The shared canonical class constructor writes nul-terminated display names;
    // imported empty names remain separate wire data in the round-trip fixtures.
    for (_, value) in &mut canonical_create.items {
        let D::Reference(parts) = value else { panic!("fixture must contain references") };
        for part in parts {
            let class = match part {
                ReferenceItem::Class(class) | ReferenceItem::Property { class, .. } | ReferenceItem::Enumerated { class, .. } => class,
                _ => panic!("fixture must contain class/property/target references"),
            };
            class.name = UnicodeString::new_nul("");
        }
    }
    assert_eq!(exported(CREATE, json!({})).unwrap().descriptor, Some(canonical_create));
    for percent in [1, 100, 200, 1000] {
        assert_eq!(exported(SCALE, json!({"scale":percent})).unwrap().descriptor, Some(scale(f64::from(percent))));
    }
    for value in [json!(0), json!(1001), json!(1.5), Value::Null, json!("100"), json!(true)] {
        assert!(exported(SCALE, json!({"scale":value})).is_none());
    }
    for id in [SCALE, CREATE] {
        for key in ["layer", "layers", "target", "unknown"] {
            let mut p = json!({});
            p[key] = json!(1);
            assert!(exported(id, p).is_none(), "{id}/{key}");
        }
        let invalid = if id == SCALE { scale(100.0) } else { create() }.with("extra", D::Boolean(true));
        let mut p = if id == SCALE { json!({"scale":200}) } else { json!({}) };
        p["_atnLayerStyleDescriptor"] = json!(photocraft_paint::tile::b64_encode(&invalid.to_bytes()));
        assert!(exported(id, p).is_none());
    }
    assert!(exported(CREATE, json!({"scale":100})).is_none());
}

/// Malformed style references remain whole and cannot fall through to ordinary blank-layer creation.
#[test]
fn layer_style_malformed_records_stay_kept_without_an_edit() {
    let mut cases = vec![(SCALE, Descriptor::new("null")), (CREATE, Descriptor::new("null").with("Usng", create().items[1].1.clone()))];
    for value in [D::Integer(100), D::Double(100.0), D::Boolean(true), D::UnitFloat { unit: *b"#Pxl", value: 100.0 }] {
        cases.push((SCALE, Descriptor::new("null").with("Scl ", value)));
    }
    for value in [0.0, 1001.0, 1.5, f64::NAN, f64::INFINITY] {
        cases.push((SCALE, scale(value)));
    }
    for (id, d) in [(SCALE, scale(100.0)), (CREATE, create())] {
        cases.push((id, d.clone().with("extra", D::Boolean(true))));
        let mut duplicate = d.clone();
        duplicate.items.push(duplicate.items[0].clone());
        cases.push((id, duplicate));
    }
    for null in [
        D::Integer(1),
        D::Reference(vec![ReferenceItem::Class(class("Chnl"))]),
        D::Reference(vec![ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") }]),
    ] {
        let mut d = create();
        d.items[0].1 = null;
        cases.push((CREATE, d));
    }
    for using in [
        D::Integer(1),
        D::Boolean(false),
        D::Descriptor(Descriptor::new("unknown")),
        D::Reference(vec![]),
        D::Reference(vec![
            ReferenceItem::Property { class: class("Prpr"), key: Id::new("wrong") },
            ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
        ]),
        D::Reference(vec![
            ReferenceItem::Property { class: class("wrong"), key: Id::new("Lefx") },
            ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
        ]),
        D::Reference(vec![
            ReferenceItem::Property { class: class("Prpr"), key: Id::new("Lefx") },
            ReferenceItem::Enumerated { class: class("Dcmn"), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
        ]),
        D::Reference(vec![
            ReferenceItem::Property { class: class("Prpr"), key: Id::new("Lefx") },
            ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Al  ") },
        ]),
        D::Reference(vec![
            ReferenceItem::Property { class: class("Prpr"), key: Id::new("Lefx") },
            ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("wrong"), value: Id::new("Trgt") },
        ]),
        D::Reference(vec![
            ReferenceItem::Property { class: class("Prpr"), key: Id::new("Lefx") },
            ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul("Source") },
        ]),
        D::Reference(vec![
            ReferenceItem::Property { class: class("Prpr"), key: Id::new("Lefx") },
            ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") },
            ReferenceItem::Class(class("Lyr ")),
        ]),
    ] {
        let mut d = create();
        d.items[1].1 = using;
        cases.push((CREATE, d));
    }
    for (id, d) in cases {
        let source = item(id, d);
        let loaded = import(&atn::write(&file(vec![source.clone()])).unwrap()).unwrap();
        assert_eq!(loaded.unsupported.len(), 1, "{id}: {source:?}");
        assert_eq!(loaded.actions[0].steps[0].0, "actions.photoshopStep");
        let bytes = export("S", &[&loaded.actions[0]], &[], false).unwrap().0;
        let saved = atn::parse(&bytes).unwrap().actions.into_iter().next().unwrap().items;
        assert_eq!(saved.len(), 1);
        same_item_wire(saved.into_iter().next().unwrap(), source);
        let (mut s, _) = fixture(8);
        let before = s.active().unwrap().doc.clone();
        let past = s.active().unwrap().history.past_len();
        s.actions.list.extend(loaded.actions);
        let result = s.execute("actions.play", json!({"action":0})).unwrap();
        assert_eq!(result["ran"], 0);
        assert!(result.get("failed").is_some());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), past);
    }
}

/// A miskeyed effects reference must be kept, rather than playing as an ordinary blank layer.
#[test]
fn miskeyed_effects_reference_stays_whole_without_a_blank_layer_edit() {
    let mut d = create();
    d.items[1].0 = Id::new("Usin");
    let source = item(CREATE, d);
    let loaded = import(&atn::write(&file(vec![source.clone()])).unwrap()).unwrap();
    let (mut s, _) = fixture(8);
    let before = s.active().unwrap().doc.clone();
    let past = s.active().unwrap().history.past_len();
    s.actions.list.extend(loaded.actions.clone());
    let result = s.execute("actions.play", json!({"action": 0})).unwrap();
    // Play before checking import classification: the old path actually creates a blank layer.
    assert!(
        s.active().unwrap().doc == before,
        "miskeyed Lefx reference changed the document: layers {} -> {}, history {} -> {}",
        before.layers.len(),
        s.active().unwrap().doc.layers.len(),
        past,
        s.active().unwrap().history.past_len()
    );
    assert_eq!(s.active().unwrap().history.past_len(), past);
    assert_eq!(result["ran"], 0);
    assert!(result.get("failed").is_some());
    assert_eq!(loaded.unsupported.len(), 1);
    assert_eq!(loaded.actions[0].steps[0].0, "actions.photoshopStep");
    let (bytes, skipped) = export("S", &[&loaded.actions[0]], &[], false).unwrap();
    assert!(skipped.is_empty());
    let saved = atn::parse(&bytes).unwrap().actions.into_iter().next().unwrap().items;
    assert_eq!(saved.len(), 1);
    same_item_wire(saved.into_iter().next().unwrap(), source);
}

/// A normal layer Make, including class-only Make, still creates a blank layer without splitting fx.
#[test]
fn ordinary_layer_make_remains_a_legitimate_control() {
    for using in [None, Some(D::Descriptor(Descriptor::new("Lyr ").with("Nm  ", D::Text(UnicodeString::new_nul("Ordinary")))))] {
        let mut d = Descriptor::new("null").with("null", D::Reference(vec![ReferenceItem::Class(class("Lyr "))]));
        if let Some(u) = using {
            d = d.with("Usng", u);
        }
        let a = action(vec![item(CREATE, d)]);
        assert_eq!(a.steps[0].0, "layer.new.layer");
        let (mut s, source) = fixture(8);
        let before = s.active().unwrap().doc.clone();
        s.actions.list.push(a);
        let result = s.execute("actions.play", json!({"action":0})).unwrap();
        assert_eq!(result["ran"], 1);
        assert!(result.get("failed").is_none());
        let after = s.active().unwrap().doc.clone();
        assert_eq!(after.layers.len(), before.layers.len() + 1);
        assert_eq!(after.layer(source), before.layer(source));
        undo_redo(&mut s, &before, &after, 1);
    }
}

/// Every typed container must retain nested effects intent without changing the document or history.
#[test]
fn nested_effects_references_stay_whole_without_a_blank_layer_edit() {
    let reference = create().items.pop().unwrap().1;
    let nested = Descriptor::new("Lyr ").with("Nm  ", D::Text(UnicodeString::new_nul("Before"))).with("data", reference.clone());
    let mut deepest = reference.clone();
    // Root descriptor depth0, Usng value depth1, options descriptor depth2, data value depth3.
    for _ in 3..MAX_DEPTH {
        deepest = D::List(vec![D::Boolean(false), deepest]);
    }
    let cases = [
        ("reference", reference.clone()),
        ("descriptor", D::Descriptor(nested.clone())),
        ("global object", D::GlobalObject(nested.clone())),
        ("list", D::List(vec![D::Boolean(false), reference])),
        ("object array", D::ObjectArray(ObjectArray { prefix: 16, body: nested.clone() })),
        (
            "mixed containers",
            D::List(vec![D::Boolean(false), D::GlobalObject(Descriptor::new("Lyr ").with("data", D::ObjectArray(ObjectArray { prefix: 16, body: nested })))]),
        ),
        ("parser depth limit", deepest),
    ];
    for (label, value) in cases {
        for key in ["Usng", "Usin"] {
            let d = Descriptor::new("null")
                .with("null", D::Reference(vec![ReferenceItem::Class(class("Lyr "))]))
                .with(key, D::Descriptor(Descriptor::new("Lyr ").with("Nm  ", D::Text(UnicodeString::new_nul("Ordinary"))).with("data", value.clone())));
            let source = item(CREATE, d);
            let loaded = import(&atn::write(&file(vec![source.clone()])).unwrap()).unwrap();
            let (mut s, _) = fixture(8);
            let before = s.active().unwrap().doc.clone();
            let past = s.active().unwrap().history.past_len();
            s.actions.list.extend(loaded.actions.clone());
            let result = s.execute("actions.play", json!({"action": 0})).unwrap();
            // Playback precedes classification so the baseline exposes the actual blank-layer edit.
            assert!(
                s.active().unwrap().doc == before,
                "{label}/{key}: nested Lefx changed the document: layers {} -> {}, history {} -> {}",
                before.layers.len(),
                s.active().unwrap().doc.layers.len(),
                past,
                s.active().unwrap().history.past_len()
            );
            assert_eq!(s.active().unwrap().history.past_len(), past, "{label}/{key}");
            assert_eq!(result["ran"], 0, "{label}/{key}");
            assert!(result.get("failed").is_some(), "{label}/{key}");
            assert_eq!(loaded.unsupported.len(), 1, "{label}/{key}");
            assert_eq!(loaded.actions[0].steps[0].0, "actions.photoshopStep", "{label}/{key}");
            let (bytes, skipped) = export("S", &[&loaded.actions[0]], &[], false).unwrap();
            assert!(skipped.is_empty(), "{label}/{key}");
            let saved = atn::parse(&bytes).unwrap().actions.into_iter().next().unwrap().items;
            assert_eq!(saved.len(), 1, "{label}/{key}");
            same_item_wire(saved.into_iter().next().unwrap(), source);
        }
    }
}

/// Native layer options still play with Lefx as a name or in synthetic opaque metadata.
#[test]
fn ordinary_layer_make_native_export_matches_native_playback() {
    let params = json!({"name": "Lefx", "opacity": 0.375, "mode": "Multiply", "clipToPrevious": true});
    let opaque = D::List(vec![D::RawData(create().to_bytes()), D::Alias(b"Lefx".to_vec()), D::Path(b"Lefx".to_vec()), D::Text(UnicodeString::new_nul("Lefx"))]);
    for metadata in [None, Some(opaque)] {
        let mut source = exported("layer.new.layer", params.clone()).unwrap();
        if let Some(value) = metadata {
            let D::Descriptor(options) = &mut source.descriptor.as_mut().unwrap().items[1].1 else { panic!("native layer options") };
            options.items.push((Id::new("data"), value));
        }
        let a = action(vec![source]);
        assert_eq!(a.steps[0], ("layer.new.layer".into(), params.clone()));
        let (mut native, _) = fixture(8);
        native.execute("layer.new.layer", params.clone()).unwrap();
        let (mut recorded, source) = fixture(8);
        let before = recorded.active().unwrap().doc.clone();
        let past = recorded.active().unwrap().history.past_len();
        recorded.actions.list.push(a);
        let result = recorded.execute("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(result["ran"], 1);
        assert!(result.get("failed").is_none());
        let after = recorded.active().unwrap().doc.clone();
        assert_eq!(after.layers.len(), before.layers.len() + 1);
        assert_eq!(after.layer(source), before.layer(source));
        assert_eq!(recorded.active().unwrap().history.past_len(), past + 1);
        equivalent(&recorded, &native);
        undo_redo(&mut recorded, &before, &after, 1);
    }
}
