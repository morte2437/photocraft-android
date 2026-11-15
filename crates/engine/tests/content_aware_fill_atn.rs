//! #10694: Content-Aware Fill options survive ATN load, save and playback.

use craft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Descriptor, Id, UnicodeString, Value as D},
};
use photocraft_doc::{LayerId, Rect};
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use serde_json::{Value, json};

const FLAGS: [(&str, &str); 4] = [
    ("contentAwareColorAdaptationFill", "colorAdaptation"),
    ("contentAwareScaleFill", "scaleAdaptation"),
    ("contentAwareMirrorFill", "mirror"),
    ("contentAwareRotateFill", "rotationAdaptation"),
];
const ROTATION: &str = "contentAwareRotateFill";

fn descriptor() -> Descriptor {
    Descriptor::new("null").with("Usng", D::Enumerated { type_id: Id::new("FlCn"), value: Id::new("contentAware") })
}

fn recorded(event: AtnEvent, descriptor: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("CAF"),
        expanded: false,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Fill"),
            items: vec![AtnItem { event, dictionary: b"Fill".to_vec(), descriptor: Some(descriptor), ..Default::default() }],
            ..Default::default()
        }],
        warnings: vec![],
        encoding: None,
    })
    .unwrap()
}

fn load(s: &mut Session, bytes: &[u8]) -> Value {
    s.execute("actions.load", json!({"data": photocraft_engine::paint::tile::b64_encode(bytes)})).unwrap()
}

fn save(s: &mut Session) -> Vec<u8> {
    let saved = s.execute("actions.save", json!({"set": "CAF"})).unwrap();
    assert_eq!(saved["skipped"], json!([]));
    photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap()
}

/// Match the qualified native fixture, using the action command's default seed.
fn fixture() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "background": "transparent"})).unwrap();
    let id = s.active().unwrap().doc.layers[0].id;
    s.edit("Asymmetric texture", |doc, _| {
        let mut pixels = Vec::with_capacity(64 * 48 * 4);
        for y in 0..48 {
            for x in 0..64 {
                pixels.extend([
                    ((37 * x + 11 * y + x * y % 17) % 251) as f32 / 250.0,
                    ((x * x + 13 * y + 3 * (x / 7)) % 241) as f32 / 240.0,
                    ((5 * x + 3 * y * y + 19 * (x / 9)) % 239) as f32 / 238.0,
                    1.0,
                ]);
            }
        }
        doc.layer_mut(id).unwrap().surface_mut().unwrap().write_region(Rect::from_xywh(0, 0, 64, 48), &pixels);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 28, "y": 20, "width": 8, "height": 8})).unwrap();
    (s, id)
}

fn pixels(s: &Session, id: LayerId) -> Vec<f32> {
    let doc = &s.active().unwrap().doc;
    doc.layer(id).unwrap().surface().unwrap().read_region(doc.bounds())
}

/// Imported flags retain their values and native pixel results through saved actions.
#[test]
fn typed_flags_survive_saved_descriptors_and_public_playback() {
    for event in [AtnEvent::Code(*b"Fl  "), AtnEvent::name("fill")] {
        for values in [[false, true, true, true], [true, false, false, false], [true, false, false, true]] {
            let mut d = descriptor();
            let mut params = json!({"contents": "contentAware"});
            for ((key, param), value) in FLAGS.into_iter().zip(values) {
                d = d.with(key, D::Boolean(value));
                params[param] = json!(value);
            }
            let (mut direct, direct_id) = fixture();
            direct.execute("edit.fill", params.clone()).unwrap();
            let expected = pixels(&direct, direct_id);
            if values[1] {
                let (mut default, default_id) = fixture();
                default.execute("edit.fill", json!({"contents": "contentAware"})).unwrap();
                assert_ne!(expected, pixels(&default, default_id), "combined options must change the fixture");
            }
            if values[3] {
                let (mut unrotated, unrotated_id) = fixture();
                let mut off = params.clone();
                off["rotationAdaptation"] = json!(false);
                unrotated.execute("edit.fill", off).unwrap();
                assert!(expected != pixels(&unrotated, unrotated_id), "the action fixture must distinguish rotated candidates");
            }
            let (mut replay, replay_id) = fixture();
            assert_eq!(load(&mut replay, &recorded(event.clone(), d))["unsupported"], json!([]));
            assert_eq!(replay.actions.list[0].steps, vec![("edit.fill".into(), params)], "CAF flags must import without loss");
            let saved = save(&mut replay);
            let parsed = atn::parse(&saved).unwrap();
            let saved_d = parsed.actions[0].items[0].descriptor.as_ref().unwrap();
            for ((key, _), value) in FLAGS.into_iter().zip(values) {
                assert_eq!(saved_d.get(key), Some(&D::Boolean(value)));
            }
            assert_eq!(saved_d.get(ROTATION), Some(&D::Boolean(values[3])));
            let imported = atn_cmds::import(&saved).unwrap();
            assert!(imported.unsupported.is_empty());
            assert_eq!(imported.actions[0].steps, replay.actions.list[0].steps);
            replay.actions.list = imported.actions;
            let played = replay.execute("actions.play", json!({"action": "Fill"})).unwrap();
            assert_eq!(played["ran"], 1);
            assert!(played.get("failed").is_none(), "{played}");
            let actual = pixels(&replay, replay_id);
            let difference = actual.iter().zip(&expected).position(|(a, b)| a != b);
            assert_eq!(difference, None, "saved CAF playback must match the native options");
        }
    }
}

/// Omission and null retain native defaults; saved native fills explicitly disable rotation.
#[test]
fn absent_and_null_native_flags_keep_defaults_and_export_no_rotation() {
    let mut s = Session::new();
    assert_eq!(load(&mut s, &recorded(AtnEvent::Code(*b"Fl  "), descriptor()))["unsupported"], json!([]));
    assert_eq!(s.actions.list[0].steps[0].1, json!({"contents": "contentAware", "rotationAdaptation": false}));
    let saved = save(&mut s);
    let imported = atn_cmds::import(&saved).unwrap();
    assert_eq!(imported.actions[0].steps, s.actions.list[0].steps);
    for null in [false, true] {
        let mut params = json!({"contents": "contentAware"});
        if null {
            for (_, param) in FLAGS {
                params[param] = Value::Null;
            }
        }
        let action = Action::new("Defaults", vec![("edit.fill".into(), params)]);
        let (bytes, skipped) = atn_cmds::export("CAF", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let file = atn::parse(&bytes).unwrap();
        let d = file.actions[0].items[0].descriptor.as_ref().unwrap();
        for (key, _) in FLAGS {
            if key != ROTATION {
                assert!(d.get(key).is_none());
            }
        }
        assert_eq!(d.get(ROTATION), Some(&D::Boolean(false)), "the default performs no rotated-source search");
    }
    for (_, param) in FLAGS {
        for value in [json!(1), json!("false")] {
            let mut params = json!({"contents": "contentAware"});
            params[param] = value;
            let action = Action::new("Invalid", vec![("edit.fill".into(), params)]);
            assert!(atn_cmds::export("CAF", &[&action], &[], false).is_err());
        }
    }
}

/// Malformed flags remain intact as unsupported Photoshop steps.
#[test]
fn malformed_flags_remain_kept_steps() {
    for event in [AtnEvent::Code(*b"Fl  "), AtnEvent::name("fill")] {
        let mut bad = Vec::new();
        for (key, _) in FLAGS {
            for value in [D::Integer(0), D::Text(UnicodeString::new_nul("false"))] {
                bad.push(descriptor().with(key, value));
            }
        }
        bad.push(descriptor().with(ROTATION, D::Integer(0)));
        for d in bad {
            let source = recorded(event.clone(), d);
            let original = atn::parse(&source).unwrap();
            let mut s = Session::new();
            let loaded = load(&mut s, &source);
            assert_eq!(loaded["unsupported"].as_array().unwrap().len(), 1);
            assert_eq!(s.actions.list[0].steps[0].0, atn_cmds::PHOTOSHOP_STEP);
            let saved = atn::parse(&save(&mut s)).unwrap();
            assert_eq!(saved.actions[0].items[0].descriptor, original.actions[0].items[0].descriptor);
            assert_eq!(saved.actions[0].items[0].event, original.actions[0].items[0].event);
        }
    }
}
