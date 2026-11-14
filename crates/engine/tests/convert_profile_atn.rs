//! Recorded-shaped Convert to Profile flags survive ATN load, save and native playback.

use std::sync::Arc;

use craft_cms::Builtin;
use craft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Descriptor, Id, UnicodeString, Value as D},
};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, SmartObject, SmartSource};
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use photocraft_geom::{Affine, Size};
use photocraft_raster::Surface;
use serde_json::{Value, json};

const FLAGS: [(&str, &str); 3] = [("Dthr", "dither"), ("Fltt", "flatten"), ("rasterizePlaced", "rasterizePlaced")];

fn descriptor() -> Descriptor {
    Descriptor::new("null")
        .with("T   ", D::Text(UnicodeString::new_nul("sRGB IEC61966-2.1")))
        .with("Inte", D::Enumerated { type_id: Id::new("Inte"), value: Id::new("Clrm") })
        .with("MpBl", D::Boolean(false))
}

fn recorded(d: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Profile"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Convert"),
            items: vec![AtnItem {
                event: AtnEvent::name("convertToProfile"),
                dictionary: b"Convert to Profile".to_vec(),
                descriptor: Some(d),
                ..Default::default()
            }],
            ..Default::default()
        }],
        expanded: false,
        warnings: vec![],
        encoding: None,
    })
    .unwrap()
}

fn load(s: &mut Session, bytes: &[u8]) -> Value {
    s.execute("actions.load", json!({"data": photocraft_engine::paint::tile::b64_encode(bytes)})).unwrap()
}

fn save(s: &mut Session) -> Vec<u8> {
    let saved = s.execute("actions.save", json!({"set": "Profile"})).unwrap();
    assert_eq!(saved["skipped"], json!([]));
    photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap()
}

/// Preserve absolute intent and keep unrepresentable intent settings as complete steps.
#[test]
fn absolute_and_unknown_intents_remain_distinct_with_conversion_flags() {
    for code in ["AClr", "AbsC", "futureIntent"] {
        let mut d = descriptor();
        d.items.retain(|(key, _)| !key.is("Inte"));
        d.items.push((Id::new("Inte"), D::Enumerated { type_id: Id::new("Inte"), value: Id::new(code) }));
        let mut s = intent_fixture();
        let result = load(&mut s, &recorded(d.clone()));
        let saved = atn::parse(&save(&mut s)).unwrap();
        let saved = saved.actions[0].items[0].descriptor.as_ref().unwrap();
        if code == "futureIntent" {
            assert_eq!(result["unsupported"].as_array().unwrap().len(), 1);
            assert_eq!(saved, &d);
        } else {
            assert_eq!(result["unsupported"], json!([]));
            assert_eq!(s.actions.list[0].steps[0].1["intent"], json!("absolute"));
            assert_eq!(saved.get("Inte"), Some(&D::Enumerated { type_id: Id::new("Inte"), value: Id::new("AClr") }));
            let mut absolute = intent_fixture();
            absolute.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "absolute", "bpc": false})).unwrap();
            let mut relative = intent_fixture();
            relative.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "relative", "bpc": false})).unwrap();
            assert_ne!(rendered(&absolute), rendered(&relative), "the profile pair must distinguish rendering intents");
            let played = s.execute("actions.play", json!({"action": "Convert"})).unwrap();
            assert_eq!(played["ran"], 1);
            assert!(played.get("failed").is_none(), "{played}");
            assert_eq!(rendered(&s), rendered(&absolute));
            // Half media white scales linear 128/255 to 64/255, about 137/255 in sRGB.
            // Check the cached pixel before opacity so a shared intent swap cannot pass.
            let pixel = s.active().unwrap().doc.layers[1].surface().unwrap().default_pixel();
            assert!(pixel[..3].iter().all(|value| (value - 137.0 / 255.0).abs() <= 1.0 / 255.0), "{pixel:?}");
        }
    }
}

/// A cached placed layer over black separates flatten order, rasterization and U8 dither.
fn fixture() -> Session {
    let mut doc = Document::new("Linear source", Size::new(256, 256), ColorMode::Rgb, SampleType::U8);
    doc.icc_profile = Some(Builtin::LinearSrgb.profile().to_bytes());
    doc.layers.push(Layer::new("Black", LayerContent::Raster(Surface::with_default(PixelFormat::RGBA8, &[0.0, 0.0, 0.0, 1.0]))));
    let cache = Surface::with_default(PixelFormat::RGBA8, &[128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
    let source = SmartSource::Embedded { file_name: "cached.svg".into(), bytes: Arc::new(b"cached appearance must be used".to_vec()) };
    let mut layer = Layer::new("Half placed", LayerContent::Smart(SmartObject::new(source, Affine::IDENTITY, Some(cache))));
    layer.opacity = 0.5;
    doc.layers.push(layer);
    let mut s = Session::new();
    s.add_document(doc, None);
    s
}

/// A darker media white makes absolute and relative conversion produce different pixels.
fn intent_fixture() -> Session {
    let mut s = fixture();
    let mut profile = Builtin::LinearSrgb.profile().clone();
    profile.white_point = profile.white_point.map(|value| value * 0.5);
    Arc::make_mut(&mut s.active_mut().unwrap().doc).icc_profile = Some(profile.with_encoded_bytes().to_bytes());
    s
}

fn rendered(s: &Session) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(&s.active().unwrap().doc).into_px()
}

/// Assert saved typed fields and independent pixel/model effects, not just paired adapters.
#[test]
fn recorded_rgb_flags_survive_save_reimport_and_public_playback() {
    for values in [[true, true, true], [true, false, true], [true, false, false], [false, false, false]] {
        let mut d = descriptor().with("sdwM", D::Integer(-1));
        let mut params = json!({"profile": "srgb", "intent": "relative", "bpc": false});
        for ((key, param), value) in FLAGS.into_iter().zip(values) {
            d = d.with(key, D::Boolean(value));
            params[param] = json!(value);
        }
        let mut direct = fixture();
        direct.execute("edit.convertToProfile", params.clone()).unwrap();
        let expected = direct.active().unwrap().doc.clone();
        if values[0] {
            let mut control = fixture();
            let mut p = params.clone();
            p["dither"] = json!(false);
            control.execute("edit.convertToProfile", p).unwrap();
            assert_ne!(rendered(&direct), rendered(&control), "ignored dither cannot pass");
        }
        if values[1] {
            let mut wrong_order = fixture();
            let mut p = params.clone();
            p["flatten"] = json!(false);
            wrong_order.execute("edit.convertToProfile", p).unwrap();
            wrong_order.execute("layer.flattenImage", json!({})).unwrap();
            assert!((rendered(&direct)[0][0] - rendered(&wrong_order)[0][0]).abs() > 0.1, "source-first order fixture");
        }
        let mut replay = fixture();
        assert_eq!(load(&mut replay, &recorded(d))["unsupported"], json!([]));
        assert_eq!(replay.actions.list[0].steps, vec![("edit.convertToProfile".into(), params)]);
        let saved = save(&mut replay);
        let parsed = atn::parse(&saved).unwrap();
        let saved_d = parsed.actions[0].items[0].descriptor.as_ref().unwrap();
        assert_eq!(saved_d.get("sdwM"), Some(&D::Integer(-1)));
        for ((key, _), value) in FLAGS.into_iter().zip(values) {
            assert_eq!(saved_d.get(key), Some(&D::Boolean(value)));
        }
        let imported = atn_cmds::import(&saved).unwrap();
        assert!(imported.unsupported.is_empty());
        assert_eq!(imported.actions[0].steps, replay.actions.list[0].steps);
        replay.actions.list = imported.actions;
        let played = replay.execute("actions.play", json!({"action": "Convert"})).unwrap();
        assert_eq!(played["ran"], 1);
        assert!(played.get("failed").is_none(), "{played}");
        assert_eq!(rendered(&replay), rendered(&direct));
        let actual = &replay.active().unwrap().doc;
        assert_eq!(actual.icc_profile, expected.icc_profile);
        assert_eq!(actual.layers.len(), if values[1] { 1 } else { 2 });
        assert_eq!(actual.layers.iter().filter(|l| matches!(l.content, LayerContent::Smart(_))).count(), usize::from(!values[1] && !values[2]));
        for (actual, expected) in actual.layers.iter().zip(&expected.layers) {
            assert_eq!(actual.surface(), expected.surface());
            assert_eq!(actual.opacity, expected.opacity);
        }
    }
}

/// Legacy omission and native nulls have the existing false defaults, serialized explicitly.
#[test]
fn defaults_are_explicit_false_and_assign_profile_is_unchanged() {
    let mut s = fixture();
    assert_eq!(load(&mut s, &recorded(descriptor()))["unsupported"], json!([]));
    let params = &s.actions.list[0].steps[0].1;
    for (_, param) in FLAGS {
        assert_eq!(params[param], false);
    }
    let saved = save(&mut s);
    s.actions.list = atn_cmds::import(&saved).unwrap().actions;
    let mut control = fixture();
    control.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "relative", "bpc": false})).unwrap();
    let played = s.execute("actions.play", json!({"action": "Convert"})).unwrap();
    assert_eq!(played["ran"], 1);
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(rendered(&s), rendered(&control));
    assert!(matches!(s.active().unwrap().doc.layers[1].content, LayerContent::Smart(_)));
    for nulls in [false, true] {
        let mut params = json!({"profile": "srgb"});
        if nulls {
            for (_, param) in FLAGS {
                params[param] = Value::Null;
            }
        }
        let action = Action::new("Defaults", vec![("edit.convertToProfile".into(), params)]);
        let (bytes, skipped) = atn_cmds::export("Profile", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let parsed = atn::parse(&bytes).unwrap();
        let d = parsed.actions[0].items[0].descriptor.as_ref().unwrap();
        for (key, _) in FLAGS {
            assert_eq!(d.get(key), Some(&D::Boolean(false)));
        }
        assert_eq!(d.get("sdwM"), Some(&D::Integer(-1)));
    }
    let assign = Action::new("Assign", vec![("edit.assignProfile".into(), json!({"profile": "srgb"}))]);
    let (bytes, skipped) = atn_cmds::export("Profile", &[&assign], &[], false).unwrap();
    assert!(skipped.is_empty());
    assert_eq!(atn_cmds::import(&bytes).unwrap().actions[0].steps, assign.steps);
    let parsed = atn::parse(&bytes).unwrap();
    let d = parsed.actions[0].items[0].descriptor.as_ref().unwrap();
    for key in ["sdwM", "Dthr", "Fltt", "rasterizePlaced"] {
        assert!(d.get(key).is_none());
    }
}

/// Unrepresented radio values and malformed booleans remain complete kept Photoshop steps.
#[test]
fn unsupported_values_are_preserved_and_invalid_native_values_refuse_export() {
    let mut descriptors = Vec::new();
    for value in [D::Integer(3), D::Integer(4), D::Integer(2), D::Integer(999), D::Boolean(true), D::Double(2.0)] {
        descriptors.push(descriptor().with("sdwM", value));
    }
    for (key, _) in FLAGS {
        for value in [D::Integer(1), D::Text(UnicodeString::new_nul("true"))] {
            descriptors.push(descriptor().with("sdwM", D::Integer(-1)).with(key, value));
        }
    }
    for d in descriptors {
        let bytes = recorded(d);
        let original = atn::parse(&bytes).unwrap();
        let mut s = Session::new();
        assert_eq!(load(&mut s, &bytes)["unsupported"].as_array().unwrap().len(), 1);
        assert_eq!(s.actions.list[0].steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        let saved = atn::parse(&save(&mut s)).unwrap();
        assert_eq!(saved.actions[0].items[0].descriptor, original.actions[0].items[0].descriptor);
        assert_eq!(saved.actions[0].items[0].event, original.actions[0].items[0].event);
    }
    for key in ["dither", "flatten", "rasterizePlaced", "sdwM"] {
        for value in [json!(2), json!("false"), json!([]), json!({})] {
            let mut params = json!({"profile": "srgb"});
            params[key] = value;
            let action = Action::new("Invalid", vec![("edit.convertToProfile".into(), params)]);
            assert!(atn_cmds::export("Profile", &[&action], &[], false).is_err());
        }
    }
    let action = Action::new("Invalid radio null", vec![("edit.convertToProfile".into(), json!({"profile": "srgb", "sdwM": null}))]);
    assert!(atn_cmds::export("Profile", &[&action], &[], false).is_err());
}

/// A step recorded by the reference application (`sdwM` -1, dither on) maps and saves back as recorded.
#[test]
fn reference_recorded_convert_to_profile_maps() {
    let d = Descriptor::new("null")
        .with(
            "null",
            D::Reference(vec![craft_psd::descriptor::ReferenceItem::Enumerated {
                class: craft_psd::descriptor::Class { name: UnicodeString::new_nul(""), class_id: Id::new("Dcmn") },
                type_id: Id::new("Ordn"),
                value: Id::new("Trgt"),
            }]),
        )
        .with("T   ", D::Text(UnicodeString::new_nul("ProPhoto RGB")))
        .with("Inte", D::Enumerated { type_id: Id::new("Inte"), value: Id::new("AClr") })
        .with("Dthr", D::Boolean(true))
        .with("Fltt", D::Boolean(false))
        .with("rasterizePlaced", D::Boolean(false))
        .with("sdwM", D::Integer(-1));
    let mut s = fixture();
    assert_eq!(load(&mut s, &recorded(d))["unsupported"], json!([]));
    assert_eq!(
        s.actions.list[0].steps,
        vec![(
            "edit.convertToProfile".into(),
            json!({"profile": "prophoto-compat", "intent": "absolute", "dither": true, "flatten": false, "rasterizePlaced": false})
        )]
    );
    let saved = atn::parse(&save(&mut s)).unwrap();
    let saved = saved.actions[0].items[0].descriptor.as_ref().unwrap();
    assert_eq!(saved.get("sdwM"), Some(&D::Integer(-1)));
    assert_eq!(saved.get("Dthr"), Some(&D::Boolean(true)));
}
