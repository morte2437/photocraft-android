//! #10723: File › Automate › Conditional Mode Change steps, the script event measured in the
//! reference application: class `cjsp`, then `Msge`, `SrcM` (`Cndn` per ticked source mode) and
//! `DstM` (the target mode's object).

use super::stylize_tests::{load_items, saved_items};
use super::*;
use photocraft_color::ColorMode;

const EVENT: &str = "8cba8cd6-cb66-11d1-bc43-0060b0a13dc4";
const ID: &str = "file.automate.conditionalModeChange";
const SOURCE: &str = "_atnConditionalModeChangeDescriptor";

/// The measured `Cndn` value of each source checkbox, in the order a step with every box ticked
/// lists them, with the native mode name.
const MEASURED_SOURCES: [(&str, &str); 8] = [
    ("UBtm", "bitmap"),
    ("UGry", "grayscale"),
    ("UDtn", "duotone"),
    ("UInd", "indexed"),
    ("URGB", "rgb"),
    ("UCMY", "cmyk"),
    ("ULab", "lab"),
    ("UMlt", "multichannel"),
];

/// A measured `DstM` with no items: an object of the target mode's class.
fn target(class: &str) -> D {
    D::Descriptor(Descriptor::new(class))
}

/// The measured descriptor, built here independently of the converter: `Msge`, then `SrcM` with
/// the given `Cndn` values, then `DstM`.
fn recorded(message: &str, sources: &[&str], dst: D) -> Descriptor {
    Descriptor::new("cjsp").with("Msge", D::Text(UnicodeString::new_nul(message))).with("SrcM", src_m(sources)).with("DstM", dst)
}

/// A measured `SrcM`: a list of `Cndn` enums with the given values.
fn src_m(sources: &[&str]) -> D {
    D::List(sources.iter().map(|c| D::Enumerated { type_id: Id::new("Cndn"), value: Id::new(c) }).collect())
}

/// A Conditional Mode Change step as the reference application records it.
fn item(d: Descriptor) -> AtnItem {
    AtnItem { event: AtnEvent::name(EVENT), dictionary: b"Conditional Mode Change".to_vec(), descriptor: Some(d), ..Default::default() }
}

/// `d` without its `Msge`.
fn without_message(mut d: Descriptor) -> Descriptor {
    d.items.retain(|(k, _)| !k.is("Msge"));
    d
}

/// The descriptor `export_item` writes for a native step with `params`, if it saves.
fn exported(params: &Value) -> Option<Descriptor> {
    export_item(ID, params, "S").and_then(|item| item.descriptor)
}

/// Each measured source alone, and all eight in the measured order, load as the native step with
/// those source modes and save back byte for byte with the event and its label.
#[test]
fn every_measured_source_loads_and_saves_back() {
    let mut forms: Vec<(Vec<&str>, Vec<&str>)> = MEASURED_SOURCES.iter().map(|(c, name)| (vec![*c], vec![*name])).collect();
    forms.push((MEASURED_SOURCES.iter().map(|(c, _)| *c).collect(), MEASURED_SOURCES.iter().map(|(_, name)| *name).collect()));
    for (codes, names) in forms {
        let d = recorded("Recorded message", &codes, target("Grys"));
        let mut s = load_items(vec![item(d.clone())]);
        let (id, p) = &s.actions.list[0].steps[0];
        assert_eq!(id, ID, "{codes:?}");
        assert_eq!((&p["from"], &p["to"]), (&json!(names), &json!("grayscale")), "{p}");
        let back = saved_items(&mut s);
        assert_eq!((back.len(), &back[0].event, back[0].dictionary.as_slice()), (1, &AtnEvent::name(EVENT), b"Conditional Mode Change".as_slice()));
        assert_eq!(back[0].descriptor.as_ref().map(Descriptor::to_bytes), Some(d.to_bytes()), "{codes:?}");
    }
}

/// The four measured targets the native command converts to load with that target; the
/// empty-message form in the measured order is what a native step saves, so it keeps no source.
#[test]
fn every_native_target_loads_and_the_canonical_form_carries_only_the_settings() {
    for (class, to) in [("Grys", "grayscale"), ("RGBM", "rgb"), ("CMYM", "cmyk"), ("LbCM", "lab")] {
        let d = recorded("", &["URGB", "ULab"], target(class));
        let s = load_items(vec![item(d.clone())]);
        let (id, p) = &s.actions.list[0].steps[0];
        assert_eq!((id.as_str(), p), (ID, &json!({"from": ["rgb", "lab"], "to": to})));
        assert_eq!(exported(p), Some(d), "{to}");
    }
}

/// A step recorded in PhotoCraft writes the measured descriptor byte for byte: an empty class name,
/// class `cjsp`, then `Msge` TEXT, `SrcM` VlLs of `Cndn` enums and `DstM` Objc of the target's
/// class without items; the step is enabled and without its dialog.
#[test]
fn a_step_recorded_in_photocraft_writes_the_measured_descriptor() {
    let item = export_item(ID, &json!({"from": ["rgb"], "to": "gray"}), "S").unwrap();
    assert_eq!((item.enabled, item.with_dialog, &item.event), (true, false, &AtnEvent::name(EVENT)));
    let mut expected = Vec::new();
    expected.extend_from_slice(b"\0\0\0\0\0\0\0\0cjsp\0\0\0\x03"); // no class name, `cjsp`, 3 items
    expected.extend_from_slice(b"\0\0\0\0MsgeTEXT\0\0\0\x01\0\0"); // one UTF-16 unit: the NUL
    expected.extend_from_slice(b"\0\0\0\0SrcMVlLs\0\0\0\x01enum\0\0\0\0Cndn\0\0\0\0URGB");
    expected.extend_from_slice(b"\0\0\0\0DstMObjc\0\0\0\0\0\0\0\0Grys\0\0\0\0");
    assert_eq!(item.descriptor.as_ref().map(Descriptor::to_bytes), Some(expected));
}

/// A native step saves its source modes in the measured order, once each, whatever order and
/// spelling it holds; a step without `from` (any source mode) saves every box ticked.
#[test]
fn native_sources_save_in_the_measured_order() {
    let all: Vec<&str> = MEASURED_SOURCES.iter().map(|(c, _)| *c).collect();
    for (from, codes) in [
        (json!(["CMYK", "gray", "rgb", "grayscale"]), vec!["UGry", "URGB", "UCMY"]),
        (json!("Multichannel"), vec!["UMlt"]),
        (json!(["lab", "any"]), all.clone()),
        (json!(null), all.clone()),
    ] {
        assert_eq!(exported(&json!({"from": from, "to": "lab"})), Some(recorded("", &codes, target("LbCM"))), "{from}");
    }
    assert_eq!(exported(&json!({"to": "RGB"})), Some(recorded("", &all, target("RGBM"))));
}

/// The reference plays the step alike whatever its `Msge` holds, or without one (measured on a new
/// 400 x 300 RGB document, RGB to Grayscale: Grayscale every time). It converts nothing when the
/// target is the document's mode or the document's mode isn't a ticked source.
#[test]
fn a_loaded_step_plays_like_the_reference() {
    let to_gray = |message: &str| recorded(message, &["URGB"], target("Grys"));
    for (d, expected) in [
        (to_gray("Conditional Mode Change action settings"), ColorMode::Grayscale),
        (to_gray(""), ColorMode::Grayscale),
        (without_message(to_gray("")), ColorMode::Grayscale),
        (to_gray("unrelated text"), ColorMode::Grayscale),
        (recorded("", &["URGB", "UCMY"], target("RGBM")), ColorMode::Rgb),
        (recorded("", &["UGry", "UCMY"], target("Grys")), ColorMode::Rgb),
        (recorded("", &["UGry", "URGB"], target("LbCM")), ColorMode::Lab),
    ] {
        let mut s = load_items(vec![item(d.clone())]);
        assert_eq!(s.actions.list[0].steps[0].0, ID, "{d:?}");
        s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
        assert_eq!(played["ran"], 1, "{played}");
        assert_eq!(s.active().unwrap().doc.mode, expected, "{d:?}");
        assert_eq!(saved_items(&mut s)[0].descriptor.as_ref().map(Descriptor::to_bytes), Some(d.to_bytes()));
    }
}

/// Measured forms the native command can't play, and every other form, keep the reference step as
/// it was: Duotone, Indexed Color, Bitmap and Multichannel targets with their measured settings,
/// the class reference recorded for Grayscale from a Bitmap document, and any malformed `SrcM`,
/// `DstM` or key set.
#[test]
fn other_forms_keep_the_photoshop_step() {
    let duotone = Descriptor::new("DtnM").with(
        "Inks",
        D::List(vec![D::Descriptor(
            Descriptor::new("DtnI")
                .with("Nm  ", D::Text(UnicodeString::new_nul("Black")))
                .with("Clr ", D::Descriptor(Descriptor::new("Grsc").with("Gry ", D::Double(100.0)))),
        )]),
    );
    let indexed = Descriptor::new("IndC")
        .with("Plt ", D::Enumerated { type_id: Id::new("ClrP"), value: Id::new("Exct") })
        .with("FrcC", D::Enumerated { type_id: Id::new("FrcC"), value: Id::new("None") })
        .with("Trns", D::Boolean(true));
    let bitmap = Descriptor::new("BtmM")
        .with("Rslt", D::UnitFloat { unit: *b"#Rsl", value: 72.0 })
        .with("Mthd", D::Enumerated { type_id: Id::new("Mthd"), value: Id::new("Thrh") });
    let class_reference = D::Class(Class { name: UnicodeString::new_nul(""), class_id: Id::new("Grys") });
    let gray = || target("Grys");
    let repeated_dst = recorded("m", &["URGB"], gray()).with("DstM", target("RGBM"));
    let forms = [
        recorded("m", &["URGB"], D::Descriptor(duotone)),
        recorded("m", &["URGB"], D::Descriptor(indexed)),
        recorded("m", &["UGry"], D::Descriptor(bitmap)),
        recorded("m", &["URGB"], target("MltC")),
        recorded("m", &["UBtm"], class_reference),
        recorded("m", &["URGB"], D::Descriptor(Descriptor::new("Grys").with("Rslt", D::UnitFloat { unit: *b"#Rsl", value: 72.0 }))),
        recorded("m", &["URGB"], D::Enumerated { type_id: Id::new("Md  "), value: Id::new("Grys") }),
        recorded("m", &[], gray()),
        recorded("m", &["URGB", "UGry"], gray()),
        recorded("m", &["URGB", "URGB"], gray()),
        recorded("m", &["UXYZ"], gray()),
        recorded("m", &["URGB"], gray()).with("Rslt", D::UnitFloat { unit: *b"#Rsl", value: 72.0 }),
        Descriptor::new("cjsp").with("Msge", D::Integer(1)).with("SrcM", src_m(&["URGB"])).with("DstM", gray()),
        // A repeated `Msge` or `SrcM`, each otherwise in the measured form.
        recorded("m", &["URGB"], gray()).with("Msge", D::Text(UnicodeString::new_nul("again"))),
        Descriptor::new("cjsp").with("SrcM", src_m(&["URGB"])).with("SrcM", src_m(&["UGry"])).with("DstM", gray()),
        Descriptor::new("cjsp").with("Msge", D::Text(UnicodeString::new_nul("m"))).with("DstM", gray()),
        Descriptor::new("cjsp").with("SrcM", D::List(vec![D::Enumerated { type_id: Id::new("Cndn"), value: Id::new("URGB") }])),
        repeated_dst,
        Descriptor::new("cjsp"),
    ];
    let mut wrong_type = recorded("m", &["URGB"], gray());
    if let D::List(items) = &mut wrong_type.items[1].1 {
        items[0] = D::Enumerated { type_id: Id::new("ClrS"), value: Id::new("URGB") };
    }
    for d in forms.into_iter().chain([wrong_type]) {
        let mut s = load_items(vec![item(d.clone())]);
        assert_eq!(s.actions.list[0].steps[0].0, PHOTOSHOP_STEP, "{d:?}");
        let back = saved_items(&mut s);
        assert_eq!((&back[0].event, back[0].descriptor.as_ref().map(Descriptor::to_bytes)), (&AtnEvent::name(EVENT), Some(d.to_bytes())));
        assert_eq!(back[0].dictionary, b"Conditional Mode Change", "{d:?}");
    }
}

/// A native step saves only when the event holds all of it: a target with an object `DstM`, a
/// non-empty list of known source modes, and no other key.
#[test]
fn settings_the_event_cannot_hold_are_refused() {
    for p in [
        json!({"from": ["rgb"], "to": "bitmap"}),
        json!({"from": ["rgb"], "to": "indexed"}),
        json!({"from": ["rgb"], "to": "duotone"}),
        json!({"from": ["rgb"], "to": "multichannel"}),
        json!({"from": ["rgb"], "to": "xyz"}),
        json!({"from": ["rgb"]}),
        json!({"from": ["rgb"], "to": 1}),
        json!({"from": [], "to": "lab"}),
        json!({"from": ["rgb", "sepia"], "to": "lab"}),
        json!({"from": ["rgb", 1], "to": "lab"}),
        json!({"from": 5, "to": "lab"}),
        json!({"from": ["rgb"], "to": "lab", "flatten": true}),
    ] {
        assert_eq!(exported(&p), None, "{p}");
    }
}

/// A loaded step whose settings were changed saves the new settings in the recorded descriptor
/// (its message or lack of one, class and key order); an unchanged one saves the recording. A
/// damaged recorded descriptor only loses its message.
#[test]
fn changed_settings_save_into_the_recorded_descriptor() {
    let mut other_class = recorded("Recorded message", &["UGry"], target("RGBM"));
    other_class.class_id = Id::new("null");
    let mut changed_other_class = recorded("Recorded message", &["URGB", "UCMY"], target("LbCM"));
    changed_other_class.class_id = Id::new("null");
    for (original, changed) in [
        (recorded("Recorded message", &["UGry"], target("RGBM")), recorded("Recorded message", &["URGB", "UCMY"], target("LbCM"))),
        (without_message(recorded("", &["UGry"], target("RGBM"))), without_message(recorded("", &["URGB", "UCMY"], target("LbCM")))),
        (other_class, changed_other_class),
    ] {
        let s = load_items(vec![item(original.clone())]);
        let mut p = s.actions.list[0].steps[0].1.clone();
        assert!(p.get(SOURCE).is_some(), "{p}");
        assert_eq!(exported(&p), Some(original));
        p["from"] = json!(["cmyk", "rgb"]);
        p["to"] = json!("lab");
        assert_eq!(exported(&p), Some(changed));
    }
    // Another key order (`DstM`, `SrcM`, `Msge`) stays that order with the new settings.
    let reordered = |sources: &[&str], dst: &str| {
        Descriptor::new("cjsp").with("DstM", target(dst)).with("SrcM", src_m(sources)).with("Msge", D::Text(UnicodeString::new_nul("Recorded message")))
    };
    let s = load_items(vec![item(reordered(&["UGry"], "RGBM"))]);
    let mut p = s.actions.list[0].steps[0].1.clone();
    assert_eq!((&p["from"], &p["to"]), (&json!(["grayscale"]), &json!("rgb")), "{p}");
    assert_eq!(exported(&p), Some(reordered(&["UGry"], "RGBM")));
    p["from"] = json!(["cmyk", "rgb"]);
    p["to"] = json!("lab");
    assert_eq!(exported(&p), Some(reordered(&["URGB", "UCMY"], "LbCM")));
    let mut missing_dst = recorded("Recorded message", &["UGry"], target("RGBM"));
    missing_dst.items.retain(|(k, _)| !k.is("DstM"));
    for damaged in [
        json!("not base64!"),
        json!(photocraft_paint::tile::b64_encode(b"not a descriptor")),
        json!(photocraft_paint::tile::b64_encode(&missing_dst.to_bytes())),
        json!(7),
        json!(null),
    ] {
        let p = json!({"from": ["gray"], "to": "rgb", SOURCE: damaged});
        assert_eq!(exported(&p), Some(recorded("", &["UGry"], target("RGBM"))), "{p}");
    }
}
