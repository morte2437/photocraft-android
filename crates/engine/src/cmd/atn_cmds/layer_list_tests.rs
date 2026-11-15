use super::*;
use craft_psd::atn::{AtnAction, AtnEvent, AtnFile, AtnItem};
use craft_psd::descriptor::{Descriptor, ReferenceItem, UnicodeString, Value as D};
use serde_json::json;

fn item(event: &str, label: &str, d: Option<Descriptor>) -> AtnItem {
    let event = match event.as_bytes() {
        [a, b, c, dd] => AtnEvent::Code([*a, *b, *c, *dd]),
        _ => AtnEvent::name(event),
    };
    AtnItem { event, dictionary: label.as_bytes().to_vec(), descriptor: d, ..Default::default() }
}

fn two_layers() -> (Session, u64, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("layer.new.layer", json!({"name": "First"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Second"})).unwrap();
    let first = s.active().unwrap().doc.walk()[1].2.id.0;
    let second = s.active().unwrap().doc.walk()[2].2.id.0;
    (s, first, second)
}

fn opacities(s: &Session) -> Vec<f64> {
    s.active().unwrap().doc.walk().iter().skip(1).map(|(_, _, l)| l.opacity as f64).collect()
}

/// The issue's procedure: an opacities step that named both layers, undone, and strictly
/// exported/imported, replays on both layers of the same document — the id keeps exactly
/// the ones it changed (#7246).
#[test]
fn the_layers_list_survives_a_strict_roundtrip() {
    let (mut s, first, second) = two_layers();
    s.actions.list.push(Action::new("Targets", vec![("layer.setProps".into(), json!({"layers": [first, second], "opacity": 0.5}))]));
    s.actions.list[0].set = Some("S".into());

    s.execute("actions.play", json!({"action": "Targets"})).unwrap();
    assert_eq!(opacities(&s), [0.5, 0.5]);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(opacities(&s), [1.0, 1.0]);

    let action = s.actions.list[0].clone();
    let (bytes, skipped) = export("S", &[&action], &[], false).unwrap();
    assert_eq!(skipped, []);
    let file = atn::parse(&bytes).unwrap();
    let items = &file.actions[0].items;
    assert_eq!(items.len(), 2, "one Set per layer: {}", items.len());
    for (item, id) in items.iter().zip([first, second]) {
        assert_eq!(item.event.id(), "setd");
        let d = item.descriptor.as_ref().unwrap();
        let [ReferenceItem::Identifier { class, id: target }] = get_ref(d, "null").unwrap() else { panic!("{d:?}") };
        assert!(class.class_id.is("Lyr "), "{class:?}");
        assert_eq!(*target as u64, id);
        let props = get_obj(d, "T   ").unwrap();
        assert!(props.class_id.is("Lyr "));
        assert_eq!(props.items.len(), 1);
        assert_eq!(props.get("Opct"), Some(&pct(50.0)));
    }

    // Strictly back into the same document, ids unchanged.
    let r = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(bytes.as_slice()), "set": "S"})).unwrap();
    assert_eq!(r["actions"], json!(["Targets"]));
    assert_eq!(r["steps"], 2);
    let played = s.execute("actions.play", json!({"action": "Targets"})).unwrap();
    assert!(played["failed"].is_null(), "{played}");
    assert_eq!(played["ran"], 2);
    assert_eq!(opacities(&s), [0.5, 0.5]);
}

/// Import: a Set on a Photoshop layer id (`Idnt`) becomes the id the `layer` param takes.
#[test]
fn a_layer_id_reference_imports_as_the_layer_id() {
    let descriptor = desc("null")
        .with("null", D::Reference(vec![ReferenceItem::Identifier { class: class("Lyr "), id: 7 }]))
        .with("T   ", D::Descriptor(desc("Lyr ").with("Opct", pct(30.0))));
    let bytes = atn::write(&AtnFile {
        name: UnicodeString::new_nul("S"),
        expanded: true,
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items: vec![item("setd", "Set", Some(descriptor))], ..Default::default() }],
        warnings: Vec::new(),
        encoding: None,
    })
    .unwrap();
    let imported = import(&bytes).unwrap();
    assert_eq!(imported.unsupported, []);
    assert_eq!(imported.actions[0].steps, [("layer.setProps".to_string(), json!({"layer": 7, "opacity": 0.3}))]);

    // Re-exporting the imported step keeps the id: the reference is the same `Idnt` again,
    // not the implicit target layer (#7246, review).
    let (out, skipped) = export("S", &[&imported.actions[0]], &[], false).unwrap();
    assert_eq!(skipped, []);
    let file = atn::parse(&out).unwrap();
    assert_eq!(file.actions[0].items.len(), 1);
    let id_ref = get_ref(file.actions[0].items[0].descriptor.as_ref().unwrap(), "null").unwrap();
    let [ReferenceItem::Identifier { class, id }] = id_ref else { panic!("{id_ref:?}") };
    assert!(class.class_id.is("Lyr "), "{class:?}");
    assert_eq!(*id, 7);
}

/// A one-layer list exports as a single Set by its id, so the step roundtrips by the same
/// layer even though no name rides along.
#[test]
fn a_one_layer_list_exports_one_exact_set() {
    let (_s, first, _) = two_layers();
    let action = Action::new("Targets", vec![("layer.setProps".into(), json!({"layers": [first], "opacity": 0.5}))]);
    let (bytes, skipped) = export("S", &[&action], &[], false).unwrap();
    assert_eq!(skipped, []);
    let file = atn::parse(&bytes).unwrap();
    assert_eq!(file.actions[0].items.len(), 1);
    let steps = import(&bytes).unwrap().actions[0].steps.clone();
    assert_eq!(steps, [("layer.setProps".to_string(), json!({"layer": first, "opacity": 0.5}))]);
}

/// A list the format can't carry exactly (`Idnt` holds u32) refuses: strict export fails,
/// and `skipUnsupported` reports the whole step.
#[test]
fn export_refuses_a_layer_id_beyond_the_idnt_range() {
    let action = Action::new("Targets", vec![("layer.setProps".into(), json!({"layers": [u32::MAX as u64 + 1], "opacity": 0.5}))]);
    let e = export("S", &[&action], &[], false).unwrap_err();
    assert!(e.contains("layer.setProps"), "{e}");
    let (_, skipped) = export("S", &[&action], &[], true).unwrap();
    assert_eq!(skipped, [Skipped { action: "Targets".into(), step: 0, id: "layer.setProps".into() }]);
}

/// A name on a several-layers change can't be exact either: PhotoCraft's own command refuses
/// that step, and strict export refuses it too.
#[test]
fn export_refuses_a_rename_carrying_a_layers_list() {
    let action = Action::new("Targets", vec![("layer.setProps".into(), json!({"layers": [1, 2], "name": "Both"}))]);
    let e = export("S", &[&action], &[], false).unwrap_err();
    assert!(e.contains("layer.setProps"), "{e}");
}

/// A present `layers` that isn't an array ("7,8" etc.) can't be exact either: exporting it as
/// one item would aim the step at the active layer instead (#7246, review).
#[test]
fn export_refuses_a_non_array_layers_value() {
    let action = Action::new("Targets", vec![("layer.setProps".into(), json!({"layers": "7,8", "opacity": 0.5}))]);
    let e = export("S", &[&action], &[], false).unwrap_err();
    assert!(e.contains("layer.setProps"), "{e}");
    let (_, skipped) = export("S", &[&action], &[], true).unwrap();
    assert_eq!(skipped, [Skipped { action: "Targets".into(), step: 0, id: "layer.setProps".into() }]);
}

/// Several explicit layers with several properties: per layer (in list order), per property
/// (the step's exact keys and values, opacity then blend), each pitched to that layer's
/// `Idnt`; the step's enabled/dialog flags ride the expansion; an unrepresentable id anywhere
/// refuses the whole source step, so no partial first-layer Set escapes into the skipped
/// bytes (#7246, coord with #8371).
#[test]
fn a_list_with_properties_exports_exact_targets_in_order() {
    let (s, first, second) = two_layers();
    let mut action = Action::new("Targets", vec![("layer.setProps".into(), json!({"layers": [first, second], "opacity": 0.5, "blend": "Multiply"}))]);
    // The panel's unchecked and dialog columns: the expansion must not lose them.
    action.disabled.insert(0);
    action.dialogs.insert(0);
    let (bytes, skipped) = export("S", &[&action], &[], false).unwrap();
    assert_eq!(skipped, []);
    let file = atn::parse(&bytes).unwrap();
    let items = &file.actions[0].items;
    let expected: Vec<(u64, &str, D)> =
        [first, second].iter().flat_map(|&l| [(l, "Opct", pct(50.0)), (l, "Md  ", enm("BlnM", "Mltp"))].map(|(l, k, v)| (l, k, v)).into_iter()).collect();
    assert_eq!(items.len(), expected.len(), "one property Set per layer, properties split per item");
    for (item, (id, key, value)) in items.iter().zip(expected) {
        assert!(item.event.id() == "setd", "property events stay Sets");
        let d = item.descriptor.as_ref().unwrap();
        let [ReferenceItem::Identifier { class, id: target }] = get_ref(d, "null").unwrap() else { panic!("{d:?}") };
        assert!(class.class_id.is("Lyr "), "{class:?}");
        assert_eq!(*target as u64, id, "each property event pitches its own layer by id");
        let props = get_obj(d, "T   ").unwrap();
        assert_eq!(props.items.len(), 1);
        let (k, v) = props.items.first().unwrap();
        assert!(k.is(key), "{k:?} not {key:?}");
        assert_eq!(v, &value, "{k:?}");
        assert!(!item.enabled, "an unchecked source step stays unchecked through expansion");
        assert!(item.with_dialog, "a dialog source step keeps its dialog through expansion");
    }
    assert!(s.active().unwrap().doc.walk()[1].2.blend == photocraft_color::BlendMode::Normal, "export writes no edit into the document");

    let malformed = Action::new("Targets", vec![("layer.setProps".into(), json!({"layers": [first, "nope"], "opacity": 0.5}))]);
    let e = export("S", &[&malformed], &[], false).unwrap_err();
    assert!(e.contains("layer.setProps"), "{e}");
    let (bytes, skipped) = export("S", &[&malformed], &[], true).unwrap();
    assert_eq!(skipped, [Skipped { action: "Targets".into(), step: 0, id: "layer.setProps".into() }]);
    let file = atn::parse(&bytes).unwrap();
    assert!(file.actions[0].items.is_empty(), "no partial first-layer Set escapes the skipped step");
}

/// A numeric `layer` the reference can't hold refuses as well: exporting it silently would
/// aim the step at the active layer instead (#7246, review).
#[test]
fn export_refuses_a_numeric_layer_beyond_the_idnt_range() {
    for bad in [json!(u32::MAX as u64 + 1), json!(-5)] {
        let action = Action::new("Targets", vec![("layer.setProps".into(), json!({"layer": bad, "opacity": 0.5}))]);
        let e = export("S", &[&action], &[], false).unwrap_err();
        assert!(e.contains("layer.setProps"), "{e}");
    }
    let skipped_action = Action::new("Targets", vec![("layer.setProps".into(), json!({"layer": -5, "opacity": 0.5}))]);
    let (_, skipped) = export("S", &[&skipped_action], &[], true).unwrap();
    assert_eq!(skipped, [Skipped { action: "Targets".into(), step: 0, id: "layer.setProps".into() }]);
}
