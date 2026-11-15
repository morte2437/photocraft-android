//! #10723: Layer › New › Frame from Layers, recorded as a Make of class `framedGroupSection`
//! (`docs/atn.md`, "Recorded through the menus").

use super::*;
use photocraft_doc::{Layer, LayerId};
use photocraft_geom::Rect;

const FRAME: &str = "layer.new.frameFromLayers";
const SOURCE: &str = "_atnFrameSectionDescriptor";

/// `From`: the layer `name`, by name.
fn by_name(name: &str) -> D {
    D::Reference(vec![ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul(name) }])
}

/// The measured step, session ids included, built here independently of the converter.
fn recorded(from: D, name: &str) -> Descriptor {
    Descriptor::new("null")
        .with("null", D::Reference(vec![ReferenceItem::Class(class("framedGroupSection"))]))
        .with("From", from)
        .with("layerSectionStart", D::Integer(4))
        .with("layerSectionEnd", D::Integer(5))
        .with("Nm  ", D::Text(UnicodeString::new_nul(name)))
}

/// `d` without the keys `drop`.
fn without(mut d: Descriptor, drop: &[&str]) -> Descriptor {
    d.items.retain(|(k, _)| !drop.iter().any(|key| k.is(key)));
    d
}

/// The step PhotoCraft writes: the measured keys without the session ids.
fn canonical(from: D, name: Option<&str>) -> Descriptor {
    let d = without(recorded(from, name.unwrap_or("")), &["layerSectionStart", "layerSectionEnd"]);
    if name.is_some() { d } else { without(d, &["Nm  "]) }
}

/// The target layer, the `From` of a step recorded in PhotoCraft without a layer of its own.
fn target_layer() -> D {
    D::Reference(vec![ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Trgt") }])
}

/// Load the Make steps holding `ds` as action "A" through `actions.load`.
fn load(ds: Vec<Descriptor>) -> (Session, Value) {
    let items = ds.into_iter().map(|d| AtnItem { event: AtnEvent::Code(*b"Mk  "), dictionary: b"Make".to_vec(), descriptor: Some(d), ..Default::default() });
    let file = AtnFile {
        name: UnicodeString::new_nul("Frames"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items: items.collect(), ..Default::default() }],
        ..Default::default()
    };
    let mut s = Session::new();
    let r = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&atn::write(&file).unwrap())})).unwrap();
    (s, r)
}

/// The descriptor of the first step of action "A" as `actions.save` writes it.
fn saved(s: &mut Session) -> Option<Vec<u8>> {
    let r = s.execute("actions.save", json!({"action": "A"})).unwrap();
    assert_eq!(r["skipped"], json!([]), "{r}");
    let mut items = atn::parse(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap().actions.remove(0).items;
    assert_eq!(items[0].event, AtnEvent::Code(*b"Mk  "));
    items.remove(0).descriptor.as_ref().map(Descriptor::to_bytes)
}

/// The descriptor a native Frame from Layers step with `p` saves as, if it saves.
fn exported(p: &Value) -> Option<Descriptor> {
    export_item(FRAME, p, "S").and_then(|item| item.descriptor)
}

/// A 100 × 80 document with "Photo" (pixels at (10, 15)–(40, 45)) under "Other" (pixels at
/// (60, 5)–(90, 25)), "Other" selected.
fn two_layers(s: &mut Session) -> (LayerId, LayerId) {
    s.execute("file.new", json!({"width": 100, "height": 80})).unwrap();
    let mut add = |name: &str, r: Rect| {
        s.edit(name, |doc, active| {
            let fmt = doc.pixel_format();
            let mut l = Layer::raster(name, fmt);
            l.surface_mut().unwrap().fill_rect(r, &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
            let id = doc.insert_above(None, l);
            *active = Some(id);
            Ok(id)
        })
        .unwrap()
    };
    let photo = add("Photo", Rect::new(10, 15, 40, 45));
    let other = add("Other", Rect::new(60, 5, 90, 25));
    s.execute("layer.select", json!({"layer": other.0, "mode": "replace"})).unwrap();
    (photo, other)
}

/// The frame layers named `name`, as (group id, frame `[x, y, w, h]`).
fn frames(s: &Session, name: &str) -> Vec<(LayerId, [f64; 4])> {
    let doc = &s.active().unwrap().doc;
    doc.walk()
        .into_iter()
        .filter(|(_, _, l)| l.name == name)
        .filter_map(|(_, _, l)| l.frame.as_ref().map(|f| (l.id, [f.transform.m[4], f.transform.m[5], f.transform.m[0], f.transform.m[3]])))
        .collect()
}

/// Recorded steps load with the exact layer and name and save back byte for byte: the measured
/// form, another key order, no session ids, no name, and the target-layer form.
#[test]
fn recorded_steps_load_and_save_back_unchanged() {
    let mut reordered = recorded(by_name("Photo"), "Frame 1");
    reordered.items.reverse();
    let cases = [
        (recorded(by_name("Photo"), "Frame 1"), json!({"layer": "Photo", "name": "Frame 1"})),
        (reordered, json!({"layer": "Photo", "name": "Frame 1"})),
        (without(recorded(by_name("Ébène"), "Cadre"), &["layerSectionStart", "layerSectionEnd"]), json!({"layer": "Ébène", "name": "Cadre"})),
        (without(recorded(by_name("Photo"), ""), &["Nm  "]), json!({"layer": "Photo"})),
        (recorded(by_name("Photo"), ""), json!({"layer": "Photo", "name": ""})),
        (canonical(target_layer(), Some("Frame 1")), json!({"name": "Frame 1"})),
    ];
    for (d, params) in cases {
        let (mut s, r) = load(vec![d.clone()]);
        assert_eq!(r["unsupported"], json!([]), "{r}");
        let (id, p) = &s.actions.list[0].steps[0];
        let mut p = p.clone();
        p.as_object_mut().unwrap().remove(SOURCE);
        assert_eq!((id.as_str(), &p), (FRAME, &params));
        assert_eq!(saved(&mut s), Some(d.to_bytes()));
    }
}

/// A loaded step frames the layer it names, not the selected one: the frame fits that layer's
/// pixels, the other layer stays where it was, and one Undo takes the frame back.
#[test]
fn a_loaded_step_frames_the_named_layer_and_undoes() {
    let (mut s, _) = load(vec![recorded(by_name("Photo"), "Frame 1")]);
    let (photo, other) = two_layers(&mut s);
    let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert_eq!(played["ran"], 1, "{played}");
    assert_eq!(frames(&s, "Frame 1").into_iter().map(|f| f.1).collect::<Vec<_>>(), vec![[10.0, 15.0, 30.0, 30.0]]);
    let doc = &s.active().unwrap().doc;
    assert!(doc.layer(photo).is_none(), "Photo went into the frame");
    assert!(doc.layers.iter().any(|l| l.id == other), "Other stays a top-level layer");
    s.execute("history.undo", json!({})).unwrap();
    assert!(frames(&s, "Frame 1").is_empty());
    let doc = &s.active().unwrap().doc;
    assert!(doc.layers.iter().any(|l| l.id == photo) && doc.layers.iter().any(|l| l.id == other));
}

/// A step that names its layer plays with no layer selected: the command is admitted on the
/// named layer, not the (empty) selection.
#[test]
fn a_loaded_step_plays_with_no_layer_selected() {
    let (mut s, _) = load(vec![recorded(by_name("Photo"), "Frame 1")]);
    let (photo, other) = two_layers(&mut s);
    s.execute("select.deselectLayers", json!({})).unwrap();
    let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert_eq!(played["ran"], 1, "{played}");
    assert_eq!(frames(&s, "Frame 1").into_iter().map(|f| f.1).collect::<Vec<_>>(), vec![[10.0, 15.0, 30.0, 30.0]]);
    // The frame took in Photo, not Other.
    let doc = &s.active().unwrap().doc;
    assert!(doc.layer(photo).is_none(), "Photo went into the frame");
    assert!(doc.layers.iter().any(|l| l.id == other), "Other stays a top-level layer");
}

/// Without a layer of its own the command still frames the selected layers.
#[test]
fn without_a_layer_the_selected_layers_are_framed() {
    let mut s = Session::new();
    let (photo, other) = two_layers(&mut s);
    s.execute(FRAME, json!({"name": "Frame 1"})).unwrap();
    assert_eq!(frames(&s, "Frame 1").into_iter().map(|f| f.1).collect::<Vec<_>>(), vec![[60.0, 5.0, 30.0, 20.0]]);
    let doc = &s.active().unwrap().doc;
    assert!(doc.layer(other).is_none() && doc.layers.iter().any(|l| l.id == photo));
}

/// A step recorded in PhotoCraft writes the measured keys without the session ids: the layer by
/// name (the target layer without one) and `Nm  ` for a name that isn't blank. It loads back to
/// the same settings.
#[test]
fn native_steps_save_the_measured_keys() {
    for (p, d, back) in [
        (json!({"layer": "Photo", "name": "Frame 1"}), canonical(by_name("Photo"), Some("Frame 1")), json!({"layer": "Photo", "name": "Frame 1"})),
        (json!({"layer": "Photo"}), canonical(by_name("Photo"), None), json!({"layer": "Photo"})),
        (json!({"name": "Frame 1"}), canonical(target_layer(), Some("Frame 1")), json!({"name": "Frame 1"})),
        (json!({"name": "  ", "layer": null}), canonical(target_layer(), None), json!({})),
        (json!({}), canonical(target_layer(), None), json!({})),
    ] {
        assert_eq!(exported(&p), Some(d.clone()), "{p}");
        let (s, _) = load(vec![d]);
        assert_eq!(s.actions.list[0].steps[0], (FRAME.to_string(), back), "{p}");
    }
}

/// A native step the event can't hold is refused: a layer by id or selector, a name that isn't
/// text, or the group options the command passes on (colour, mode, opacity), which the step
/// doesn't record.
#[test]
fn settings_the_event_cannot_hold_are_refused() {
    for p in [
        json!({"layer": 3}),
        json!({"layer": {"index": 2}}),
        json!({"layer": {"background": true}}),
        json!({"name": 4}),
        json!({"name": "Frame 1", "color": "red"}),
        json!({"mode": "multiply"}),
        json!({"opacity": 50}),
        json!({"name": "Frame 1", "width": 200}),
    ] {
        assert!(exported(&p).is_none(), "{p}");
    }
}

/// Any other form keeps the reference step: no `From`, another `From`, a repeated or extra key,
/// values of another type, or another `null`.
#[test]
fn other_forms_keep_the_photoshop_step() {
    let measured = recorded(by_name("Photo"), "Frame 1");
    let replaced = |key: &str, v: D| {
        let mut d = measured.clone();
        d.items.iter_mut().filter(|(k, _)| k.is(key)).for_each(|slot| slot.1 = v.clone());
        d
    };
    let forms = [
        without(measured.clone(), &["From"]),
        replaced("From", D::Reference(vec![ReferenceItem::Index { class: class("Lyr "), index: 2 }])),
        replaced("From", D::Reference(vec![ReferenceItem::Name { class: class("Chnl"), name: UnicodeString::new_nul("Photo") }])),
        replaced("From", D::Reference(vec![ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Al  ") }])),
        replaced("From", D::Text(UnicodeString::new_nul("Photo"))),
        replaced("Nm  ", D::Integer(1)),
        replaced("layerSectionEnd", D::Double(5.0)),
        replaced("null", D::Reference(vec![ReferenceItem::Class(class("frame"))])),
        measured.clone().with("Nm  ", D::Text(UnicodeString::new_nul("Again"))),
        measured.clone().with("From", by_name("Other")),
        measured.clone().with("Wdth", D::UnitFloat { unit: PX, value: 200.0 }),
    ];
    for d in forms {
        let (mut s, r) = load(vec![d.clone()]);
        assert_eq!(r["unsupported"].as_array().map(Vec::len), Some(1), "{d:?}");
        assert_eq!(s.actions.list[0].steps[0].0, PHOTOSHOP_STEP, "{d:?}");
        assert_eq!(saved(&mut s), Some(d.to_bytes()));
    }
}

/// A loaded step whose layer or name was changed saves the new settings, not the stale
/// recording; one whose recording is damaged still saves its settings.
#[test]
fn changed_or_damaged_steps_save_their_settings() {
    let (s, _) = load(vec![recorded(by_name("Photo"), "Frame 1")]);
    let mut p = s.actions.list[0].steps[0].1.clone();
    assert!(p.get(SOURCE).is_some(), "{p}");
    p["layer"] = json!("Other");
    assert_eq!(exported(&p), Some(canonical(by_name("Other"), Some("Frame 1"))));
    p["layer"] = json!("Photo");
    p["name"] = json!(" ");
    assert_eq!(exported(&p), Some(canonical(by_name("Photo"), None)));
    p["name"] = json!("Frame 1");
    p[SOURCE] = json!("not base64!");
    assert_eq!(exported(&p), Some(canonical(by_name("Photo"), Some("Frame 1"))));
}
