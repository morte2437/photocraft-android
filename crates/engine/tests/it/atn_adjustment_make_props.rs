//! #8384: an adjustment layer's Make step keeps the new layer's name (`Nm  `), blend mode
//! (`Md  `) and clipping (`Grup`) on load, playback and save.
use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use craft_psd::descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D};
use photocraft_doc::{BlendMode, LayerContent};
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use serde_json::{Value, json};

const NAME: &str = "Independent named adjustment";

fn text(s: &str) -> D {
    D::Text(UnicodeString::new_nul(s))
}

fn blend(code: &str) -> D {
    D::Enumerated { type_id: Id::new("BlnM"), value: Id::new(code) }
}

fn levels() -> Descriptor {
    Descriptor::new("Lvls").with("presetKind", D::Enumerated { type_id: Id::new("presetKindType"), value: Id::new("presetKindDefault") })
}

/// `Make adjustment layer` with `extra` keys after `Type`, in the order Save Actions writes them.
fn make(extra: &[(&str, D)]) -> Descriptor {
    let mut using = Descriptor::new("AdjL").with("Type", D::Descriptor(levels()));
    for (k, v) in extra {
        using = using.with(k, v.clone());
    }
    let class_ref = ReferenceItem::Class(Class { name: UnicodeString::new_nul(""), class_id: Id::new("AdjL") });
    Descriptor::new("null").with("null", D::Reference(vec![class_ref])).with("Usng", D::Descriptor(using))
}

fn bytes(descriptor: Descriptor) -> Vec<u8> {
    let file = AtnFile {
        name: UnicodeString::new_nul("Props"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Make"),
            items: vec![AtnItem { event: AtnEvent::Code(*b"Mk  "), dictionary: b"Make".to_vec(), descriptor: Some(descriptor), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    };
    atn::write(&file).unwrap()
}

fn doc(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 3, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Pixels"})).unwrap();
    s
}

#[test]
fn make_keeps_name_mode_and_clipping_through_play_and_save_at_both_depths() {
    let cases: [(&[(&str, D)], Value); 5] = [
        (&[("Nm  ", text(NAME))], json!({"name": NAME})),
        (&[("Md  ", blend("Mltp"))], json!({"mode": "Multiply"})),
        (&[("Grup", D::Boolean(true))], json!({"clipToPrevious": true})),
        (&[("Grup", D::Boolean(false))], json!({"clipToPrevious": false})),
        (&[("Nm  ", text(NAME)), ("Md  ", blend("Scrn")), ("Grup", D::Boolean(true))], json!({"name": NAME, "mode": "Screen", "clipToPrevious": true})),
    ];
    for depth in [8, 16] {
        for (extra, params) in &cases {
            let descriptor = make(extra);
            let mut s = doc(depth);
            let base = s.active().unwrap().active_layer.unwrap();
            let loaded = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&bytes(descriptor.clone()))})).unwrap();
            assert_eq!(loaded["unsupported"], json!([]), "{params}");
            let steps = &s.actions.list[0].steps;
            assert_eq!(steps.len(), 1, "{params}");
            assert_eq!(steps[0].0, "layer.newAdjustmentLayer.levels");
            for (k, v) in params.as_object().unwrap() {
                assert_eq!(steps[0].1.get(k), Some(v), "{params}: {}", steps[0].1);
            }

            let played = s.execute("actions.play", json!({"action": "Make"})).unwrap();
            assert!(played.get("failed").is_none(), "{params}: {played}");
            let st = s.active().unwrap();
            let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
            assert!(matches!(layer.content, LayerContent::Adjustment(_)), "{params}");
            assert_eq!(layer.name, if params.get("name").is_some() { NAME } else { "Levels 1" }, "{params}");
            let mode = match params.get("mode").and_then(Value::as_str) {
                Some("Multiply") => BlendMode::Multiply,
                Some("Screen") => BlendMode::Screen,
                _ => BlendMode::Normal,
            };
            assert_eq!(layer.blend, mode, "{params}");
            assert_eq!(layer.clipped, params.get("clipToPrevious") == Some(&json!(true)), "{params}");
            let base = st.doc.layer(base).unwrap();
            assert_eq!((base.name.as_str(), base.blend, base.clipped), ("Pixels", BlendMode::Normal, false), "{params}");

            // Save Actions writes the descriptor the step was loaded from.
            let saved = s.execute("actions.save", json!({"set": "Props"})).unwrap();
            let data = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            let file = atn::parse(&data).unwrap();
            assert_eq!(file.actions[0].items[0].descriptor.as_ref(), Some(&descriptor), "{params}");
        }
    }
}

#[test]
fn unreadable_layer_properties_keep_the_step() {
    for extra in [("Nm  ", D::Integer(2)), ("Md  ", blend("unknownBlend")), ("Md  ", blend("PsTh")), ("Md  ", text("Multiply")), ("Grup", text("true"))] {
        let imported = atn_cmds::import(&bytes(make(std::slice::from_ref(&extra)))).unwrap();
        assert_eq!(imported.unsupported.len(), 1, "{:?}", extra.0);
        assert_eq!(imported.actions[0].steps[0].0, atn_cmds::PHOTOSHOP_STEP, "{:?}", extra.0);
    }
}

#[test]
fn bad_layer_properties_fail_without_a_layer_or_an_export() {
    let mut s = doc(8);
    for params in [
        json!({"name": false}),
        json!({"name": 3}),
        json!({"mode": false}),
        json!({"mode": "unknown"}),
        json!({"mode": "Pass Through"}),
        json!({"clipToPrevious": 1}),
        json!({"clipToPrevious": "true"}),
    ] {
        let layers = s.active().unwrap().doc.layers.len();
        assert!(s.execute("layer.newAdjustmentLayer.levels", params.clone()).is_err(), "{params}");
        assert_eq!(s.active().unwrap().doc.layers.len(), layers, "{params} must not add a layer");
        let action = Action::new("Make", vec![("layer.newAdjustmentLayer.levels".into(), params.clone())]);
        assert!(atn_cmds::export("Props", &[&action], &[], false).is_err(), "{params} must not save as a default layer");
    }
}
