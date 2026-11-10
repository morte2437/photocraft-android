//! #10694: Image Size style scaling survives ATN import, playback and saving.

use photocraft_color::{BlendMode, Color};
use photocraft_doc::{Effect, Effects, FxCommon, FxPaint, LayerId, StrokeFx, StrokePosition};
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use photocraft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Descriptor, Id, UnicodeString, Value as D},
};
use serde_json::{Value, json};

fn recorded(event: &str, descriptor: Option<Descriptor>) -> Vec<u8> {
    let event = if event == "ImgS" { AtnEvent::Code(*b"ImgS") } else { AtnEvent::name(event) };
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Styles"),
        expanded: false,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Half"),
            items: vec![AtnItem { event, dictionary: b"Image Size".to_vec(), descriptor, ..Default::default() }],
            ..Default::default()
        }],
        warnings: vec![],
        encoding: None,
    })
    .unwrap()
}

fn half_size() -> Descriptor {
    Descriptor::new("null").with("Wdth", D::UnitFloat { unit: *b"#Prc", value: 50.0 })
}

fn load(s: &mut Session, bytes: &[u8]) -> Value {
    s.execute("actions.load", json!({"data": photocraft_engine::paint::tile::b64_encode(bytes)})).unwrap()
}

fn save(s: &mut Session) -> Vec<u8> {
    let saved = s.execute("actions.save", json!({"set": "Styles"})).unwrap();
    assert_eq!(saved["skipped"], json!([]));
    photocraft_engine::paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap()
}

fn fixture() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 40, "background": "transparent"})).unwrap();
    let id = LayerId(s.execute("layer.new.layer", json!({"name": "Styled"})).unwrap()["layer"].as_u64().unwrap());
    s.edit("Style", |doc, _| {
        doc.layer_mut(id).unwrap().effects = Effects {
            enabled: true,
            items: vec![Effect::Stroke(StrokeFx {
                common: FxCommon::new(BlendMode::Normal, 1.0),
                size: 20.0,
                position: StrokePosition::Outside,
                paint: FxPaint::Color(Color::rgb(0.0, 0.0, 1.0)),
                overprint: false,
            })],
            ..Default::default()
        };
        Ok(())
    })
    .unwrap();
    (s, id)
}

fn assert_size_and_style(s: &Session, id: LayerId, scale: bool) {
    let doc = &s.active().unwrap().doc;
    assert_eq!((doc.size.width, doc.size.height), (40, 20));
    let Effect::Stroke(stroke) = &doc.layer(id).unwrap().effects.items[0] else { panic!("stroke lost") };
    assert_eq!(stroke.size, if scale { 10.0 } else { 20.0 });
}

/// Recorded true, false and omitted flags retain their meaning through save and playback.
#[test]
fn imported_style_scaling_survives_saved_bytes_and_playback() {
    for event in ["ImgS", "imageSize"] {
        for option in [None, Some(false), Some(true)] {
            let mut descriptor = half_size();
            if let Some(value) = option {
                descriptor = descriptor.with("scaleStyles", D::Boolean(value));
            }
            let (mut s, id) = fixture();
            let loaded = load(&mut s, &recorded(event, Some(descriptor)));
            assert_eq!(loaded["unsupported"], json!([]));
            let scale = option.unwrap_or(false);
            assert_eq!(s.actions.list[0].steps, vec![("image.imageSize".into(), json!({"percent": 50, "scaleStyles": scale}))]);
            let saved = save(&mut s);
            let parsed = atn::parse(&saved).unwrap();
            // Only scaling needs the key: without it the step keeps style sizes.
            assert_eq!(parsed.actions[0].items[0].descriptor.as_ref().unwrap().get("scaleStyles"), scale.then_some(&D::Boolean(true)));
            let imported = atn_cmds::import(&saved).unwrap();
            assert!(imported.unsupported.is_empty());
            assert_eq!(imported.actions[0].steps, s.actions.list[0].steps);
            s.actions.list = imported.actions;
            let result = s.execute("actions.play", json!({"action": "Half"})).unwrap();
            assert_eq!(result["ran"], 1);
            assert!(result.get("failed").is_none(), "{result}");
            assert_size_and_style(&s, id, scale);
        }
    }
}

/// Saving native calls writes their effective default instead of changing style scaling.
#[test]
fn native_default_is_written_explicitly_and_keeps_its_behavior() {
    for option in [None, Some(json!(false)), Some(json!(true)), Some(Value::Null), Some(json!(1)), Some(json!("false"))] {
        let mut params = json!({"percent": 50});
        if let Some(option) = option {
            params["scaleStyles"] = option;
        }
        let scale = params["scaleStyles"].as_bool().unwrap_or(true);
        let action = Action::new("Half", vec![("image.imageSize".into(), params.clone())]);
        let (bytes, skipped) = atn_cmds::export("Styles", &[&action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let parsed = atn::parse(&bytes).unwrap();
        assert_eq!(parsed.actions[0].items[0].descriptor.as_ref().unwrap().get("scaleStyles"), scale.then_some(&D::Boolean(true)), "{params}");
        let (mut direct, direct_id) = fixture();
        direct.execute("image.imageSize", params).unwrap();
        assert_size_and_style(&direct, direct_id, scale);
        let (mut replay, replay_id) = fixture();
        assert_eq!(load(&mut replay, &bytes)["unsupported"], json!([]));
        assert_eq!(replay.actions.list[0].steps[0].1["scaleStyles"], json!(scale));
        let result = replay.execute("actions.play", json!({"action": "Half"})).unwrap();
        assert_eq!(result["ran"], 1);
        assert!(result.get("failed").is_none(), "{result}");
        assert_size_and_style(&replay, replay_id, scale);
    }
}

/// Only an absent descriptor is menu intent. Empty or invalid descriptors remain kept steps.
#[test]
fn malformed_or_empty_steps_are_kept_without_becoming_resizes() {
    for event in ["ImgS", "imageSize"] {
        let descriptors = [
            None,
            Some(Descriptor::new("null")),
            Some(Descriptor::new("null").with("scaleStyles", D::Boolean(false))),
            Some(Descriptor::new("null").with("scaleStyles", D::Boolean(true))),
            Some(half_size().with("scaleStyles", D::Integer(0))),
            Some(half_size().with("scaleStyles", D::Integer(1))),
            Some(half_size().with("scaleStyles", D::Text(UnicodeString::new_nul("false")))),
        ];
        for descriptor in descriptors {
            let source = recorded(event, descriptor);
            let original = atn::parse(&source).unwrap();
            let mut s = Session::new();
            let loaded = load(&mut s, &source);
            let (step, params) = &s.actions.list[0].steps[0];
            if original.actions[0].items[0].descriptor.is_none() {
                assert_eq!(loaded["unsupported"], json!([]));
                assert_eq!(step, atn_cmds::MENU_ITEM);
                assert_eq!(params["id"], "image.imageSize");
                assert_eq!(params["eventKind"], if event == "ImgS" { "code" } else { "name" });
            } else {
                assert_eq!(loaded["unsupported"].as_array().unwrap().len(), 1);
                assert_eq!(step, atn_cmds::PHOTOSHOP_STEP);
            }
            let saved = atn::parse(&save(&mut s)).unwrap();
            assert_eq!(saved.actions[0].items[0].descriptor, original.actions[0].items[0].descriptor);
            assert_eq!(saved.actions[0].items[0].event, original.actions[0].items[0].event);
            assert_eq!(saved.actions[0].items[0].dictionary, original.actions[0].items[0].dictionary);
        }
    }
}

/// A recorded Image Size step without `scaleStyles` (Fit Image records none) saves back identically.
#[test]
fn step_without_scale_styles_saves_back_identically() {
    for event in ["ImgS", "imageSize"] {
        let fit = Descriptor::new("null")
            .with("Wdth", D::UnitFloat { unit: *b"#Pxl", value: 100.0 })
            .with("Hght", D::UnitFloat { unit: *b"#Pxl", value: 78.0 })
            .with("Rslt", D::UnitFloat { unit: *b"#Rsl", value: 72.0 })
            .with("Intr", D::Enumerated { type_id: Id::new("Intp"), value: Id::new("Bcbc") });
        let source = recorded(event, Some(fit));
        let original = atn::parse(&source).unwrap();
        let mut s = Session::new();
        assert_eq!(load(&mut s, &source)["unsupported"], json!([]));
        let saved = atn::parse(&save(&mut s)).unwrap();
        assert_eq!(saved.actions[0].items[0].descriptor, original.actions[0].items[0].descriptor);
    }
}
