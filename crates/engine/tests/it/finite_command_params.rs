//! #6424: rejecting an overflowing f32 parameter must leave a saveable document and history.
use photocraft_engine::{EngineError, Session};
use serde_json::{Value, json};

fn fresh(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 12, "mode": mode, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

fn saved(s: &Session) -> Vec<u8> {
    photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap()
}

fn rejected(cases: Vec<(&str, Value)>) {
    let mut failures = Vec::new();
    for (id, p) in cases {
        let mut s = fresh("rgb", 8);
        let before = saved(&s);
        let undo = s.active().unwrap().history.undo_label().map(str::to_owned);
        let result = s.execute(id, p.clone());
        let save = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default());
        if !matches!(result, Err(EngineError::BadParams { .. })) {
            failures.push(format!("{id} {p}: result={result:?}, save error={:?}", save.as_ref().err()));
            continue;
        }
        assert_eq!(save.unwrap(), before, "rejected {id} must not change the document");
        assert_eq!(s.active().unwrap().history.undo_label(), undo.as_deref(), "rejected {id} must not add history");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn overflowing_layer_styles_are_rejected_before_mutation() {
    let mut cases = Vec::new();
    for value in [1e39, -1e39] {
        for (kind, key) in [
            ("dropShadow", "angle"),
            ("innerShadow", "angle"),
            ("outerGlow", "size"),
            ("innerGlow", "size"),
            ("stroke", "size"),
            ("satin", "angle"),
            ("bevelEmboss", "depth"),
            ("gradientOverlay", "angle"),
            ("patternOverlay", "phaseX"),
        ] {
            let id = match kind {
                "dropShadow" => "layer.layerStyle.dropShadow",
                "innerShadow" => "layer.layerStyle.innerShadow",
                "outerGlow" => "layer.layerStyle.outerGlow",
                "innerGlow" => "layer.layerStyle.innerGlow",
                "stroke" => "layer.layerStyle.stroke",
                "satin" => "layer.layerStyle.satin",
                "bevelEmboss" => "layer.layerStyle.bevelEmboss",
                "gradientOverlay" => "layer.layerStyle.gradientOverlay",
                _ => "layer.layerStyle.patternOverlay",
            };
            let params = json!({key: value});
            cases.push((id, params.clone()));
            cases.push(("layer.layerStyle.replace", json!({"effects": [{"kind": kind, "params": params}]})));
        }
        cases.push(("layer.layerStyle.colorOverlay", json!({"color": [value, 0, 0]})));
    }
    rejected(cases);
}

#[test]
fn overflowing_document_colors_are_rejected_before_mutation() {
    let mut cases = Vec::new();
    for id in ["layer.newFillLayer.solidColor", "channel.new", "channel.newSpot", "layer.new.artboard", "count.newGroup"] {
        for value in [1e39, -1e39] {
            for component in 0..4 {
                let mut color = [0.5; 4];
                color[component] = value;
                cases.push((id, json!({"color": color})));
            }
        }
    }
    rejected(cases);
}

#[test]
fn overflowing_type_positions_and_colors_are_rejected_before_mutation() {
    rejected(vec![
        ("type.create", json!({"text": "hello", "x": 1e39})),
        ("type.create", json!({"text": "hello", "y": -1e39})),
        ("type.create", json!({"text": "hello", "color": [1e39, 0, 0]})),
    ]);
}

#[test]
fn valid_color_and_style_edits_still_save_and_undo_at_each_depth() {
    for mode in ["rgb", "gray", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            let mut s = fresh(mode, depth);
            let before = saved(&s);
            s.execute("layer.layerStyle.dropShadow", json!({"angle": -45, "color": [0.1, 0.3, 0.8]})).unwrap();
            let edited = saved(&s);
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(saved(&s), before);
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(saved(&s), edited);
            s.execute("layer.newFillLayer.solidColor", json!({"color": "#4080c0"})).unwrap();
            let bytes = saved(&s);
            photocraft_format::load_from_bytes(&bytes).unwrap();
        }
    }
}

#[test]
fn effect_snapshots_cannot_bypass_numeric_validation() {
    let mut fx = serde_json::to_value(photocraft_engine::layer_style::effect_from_params("dropShadow", &json!({})).unwrap()).unwrap();
    fx["DropShadow"]["angle"] = json!(1e39);
    rejected(vec![(
        "layer.layerStyle.replace",
        json!({"effects": [
            {"kind": "colorOverlay", "params": {"color": "#00ff00"}},
            {"kind": "dropShadow", "fx": fx, "params": {}}
        ]}),
    )]);
}

#[test]
fn direct_effect_helpers_do_not_introduce_infinity() {
    let original = photocraft_engine::layer_style::effect_from_params("dropShadow", &json!({})).unwrap();
    for p in [json!({"angle": 1e39}), json!({"color": [0, -1e39, 0]})] {
        assert!(photocraft_engine::layer_style::effect_from_params("dropShadow", &p).is_none());
        let mut edited = original.clone();
        photocraft_engine::layer_style::overlay_effect(&mut edited, &p);
        assert_eq!(edited, original);
    }
}

#[test]
fn bad_background_color_does_not_partially_change_tool_colors() {
    let mut s = fresh("rgb", 32);
    let before = (s.tools.foreground, s.tools.background);
    let result = s.execute("tools.setColors", json!({"foreground": "#40a0c0", "background": [0, 1e39, 0]}));
    assert!(matches!(result, Err(EngineError::BadParams { .. })));
    assert_eq!((s.tools.foreground, s.tools.background), before);
}

#[test]
fn finite_hdr_colors_are_preserved_without_unit_clamping() {
    let mut s = fresh("rgb", 32);
    let id = s.execute("layer.newFillLayer.solidColor", json!({"color": [4.0, -0.5, 0.25]})).unwrap()["layer"].as_u64().unwrap();
    let bytes = saved(&s);
    let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
    let layer = reopened.layer(photocraft_doc::LayerId(id)).unwrap();
    let photocraft_doc::LayerContent::Fill(photocraft_doc::Fill::Solid(c)) = &layer.content else { panic!("expected a solid fill") };
    assert_eq!(c.to_rgb(), [4.0, -0.5, 0.25]);
}
