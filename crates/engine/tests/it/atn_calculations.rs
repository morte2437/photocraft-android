//! Recorded Calculations Make forms, using the measured reference and ScriptListener types.
use photocraft_engine::actions_cmds::Action;
use photocraft_engine::{Session, atn_cmds};
use photocraft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use photocraft_psd::descriptor::{Class, Descriptor, Id, ReferenceItem, UnicodeString, Value as D};
use serde_json::{Value, json};

fn class(id: &str) -> Class {
    Class { name: UnicodeString::new_nul(""), class_id: Id::new(id) }
}

fn channel(ty: &str, value: &str) -> Vec<ReferenceItem> {
    vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: Id::new(ty), value: Id::new(value) }]
}

fn named(cls: &str, name: &str) -> ReferenceItem {
    ReferenceItem::Name { class: class(cls), name: UnicodeString::new_nul(name) }
}

fn make(a: Vec<ReferenceItem>, b: Vec<ReferenceItem>) -> Descriptor {
    let using = Descriptor::new("Clcl")
        .with("T   ", D::Reference(a))
        .with("Clcl", D::Enumerated { type_id: Id::new("Clcn"), value: Id::new("Add ") })
        .with("Scl ", D::Double(1.0))
        .with("Ofst", D::Integer(0))
        .with("Src2", D::Reference(b));
    Descriptor::new("null").with("Nw  ", D::Class(class("Chnl"))).with("Usng", D::Descriptor(using))
}

fn bytes(d: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Calculations"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Calculate"),
            items: vec![AtnItem { event: AtnEvent::Code(*b"Mk  "), descriptor: Some(d), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

fn roundtrip(d: Descriptor) -> Action {
    let imported = atn_cmds::import(&bytes(d.clone())).unwrap();
    assert!(imported.unsupported.is_empty());
    let action = imported.actions[0].clone();
    assert_eq!(action.steps[0].0, "image.calculations");
    let (saved, skipped) = atn_cmds::export("Calculations", &[&action], &[], false).unwrap();
    assert!(skipped.is_empty());
    assert_eq!(atn::parse(&saved).unwrap().actions[0].items[0].descriptor, Some(d));
    action
}

fn fixture(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 3, "height": 1, "depth": depth, "background": "transparent"})).unwrap();
    s.edit("pattern", |doc, active| {
        let area = doc.bounds();
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let format = surface.format();
        let pixels: Vec<f32> = [[0.125, 0.25, 0.5, 1.0], [0.25, 0.5, 0.125, 1.0], [0.5, 0.125, 0.25, 1.0]]
            .into_iter()
            .flat_map(|v| photocraft_raster::from_rgba(&format, v))
            .collect();
        surface.write_region(area, &pixels);
        Ok(())
    })
    .unwrap();
    s
}

fn alpha(s: &Session, index: usize) -> Vec<f32> {
    let doc = &s.active().unwrap().doc;
    doc.channels[index].surface.read_region(doc.bounds())
}

fn play(s: &mut Session, action: Action) -> Value {
    s.actions.list = vec![action];
    s.execute("actions.play", json!({"action": "Calculate"})).unwrap()
}

fn saved(s: &Session) -> Vec<u8> {
    photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap()
}

#[test]
fn atn_calculations_add_creates_alpha_with_expected_pixels_and_undo() {
    let action = roundtrip(make(channel("Chnl", "Rd  "), channel("Chnl", "Grn ")));
    for depth in [8, 16, 32] {
        let mut s = fixture(depth);
        let before = saved(&s);
        let original = photocraft_compose::flatten(&s.active().unwrap().doc).into_px();
        assert_eq!(play(&mut s, action.clone())["ran"], 1);
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.channels.len(), 1);
        assert_eq!(doc.channels[0].name, "Alpha 1");
        assert_eq!(doc.channels[0].surface.format().sample, doc.pixel_format().sample);
        for (got, rgba) in alpha(&s, 0).iter().zip(&original) {
            assert!((got - (rgba[0] + rgba[1]).min(1.0)).abs() < 2.0 / 255.0, "{depth}: {got} {rgba:?}");
        }
        assert_eq!(photocraft_compose::flatten(&s.active().unwrap().doc).px, original);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(saved(&s), before);
    }
    let mut d = make(channel("Chnl", "Rd  "), channel("Chnl", "Grn "));
    let D::Descriptor(using) = &mut d.items[1].1 else { panic!() };
    using.items[1].1 = D::Enumerated { type_id: Id::new("Clcn"), value: Id::new("Sbtr") };
    let mut s = fixture(32);
    assert_eq!(play(&mut s, roundtrip(d))["ran"], 1);
    assert_eq!(alpha(&s, 0), vec![0.125, 0.25, 0.0], "source1 is subtracted from source2");
}

#[test]
fn atn_calculations_target_is_resolved_on_replay_and_literal_alpha_is_distinct() {
    let action = roundtrip(make(channel("Ordn", "Trgt"), channel("Chnl", "Grn ")));
    let mut results = Vec::new();
    for component in ["red", "blue"] {
        let mut s = fixture(32);
        s.execute("channel.target", json!({"channel": component})).unwrap();
        assert_eq!(play(&mut s, action.clone())["ran"], 1);
        results.push(alpha(&s, 0));
    }
    assert_eq!(results[0], vec![0.375, 0.75, 0.625]);
    assert_eq!(results[1], vec![0.75, 0.625, 0.375]);

    let mut s = fixture(32);
    s.execute("channel.new", json!({"name": "red", "fill": "white"})).unwrap();
    assert_eq!(play(&mut s, action)["ran"], 1);
    assert_eq!(alpha(&s, 1), vec![1.0; 3]);
    let literal = roundtrip(make(vec![named("Chnl", "red")], channel("Chnl", "Grn ")));
    s.execute("channel.target", json!({"channel": "blue"})).unwrap();
    assert_eq!(play(&mut s, literal)["ran"], 1);
    assert_eq!(alpha(&s, 2), vec![1.0; 3], "named red must read the alpha, not the red component");
    // The old string remains a literal alpha name; only the typed selector has target semantics.
    s.execute("channel.rename", json!({"channel": 0, "name": "targeted"})).unwrap();
    s.execute("image.calculations", json!({"source1":{"channel":"targeted"},"source2":{"channel":{"color":"green"}},"blending":"add"})).unwrap();
    assert_eq!(alpha(&s, 3), vec![1.0; 3]);
}

#[test]
fn atn_calculations_named_layer_is_not_the_merged_sentinel() {
    let mut a = channel("Chnl", "Rd  ");
    a.push(named("Lyr ", "merged"));
    let action = roundtrip(make(a, channel("Chnl", "Grn ")));
    let mut s = fixture(32);
    s.execute("layer.setProps", json!({"name": "merged"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "top"})).unwrap();
    s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
    assert_eq!(play(&mut s, action)["ran"], 1);
    assert_eq!(alpha(&s, 0), vec![0.125, 0.25, 0.5], "red from named layer plus merged green");
}

#[test]
fn atn_calculations_explicit_merged_transparency_reads_composite_alpha() {
    let mut source = channel("Chnl", "Trsp");
    source.push(ReferenceItem::Enumerated { class: class("Lyr "), type_id: Id::new("Ordn"), value: Id::new("Mrgd") });
    let mut d = make(source, channel("Chnl", "Rd  "));
    let D::Descriptor(using) = &mut d.items[1].1 else { panic!() };
    using.items[1].1 = D::Enumerated { type_id: Id::new("Clcn"), value: Id::new("Nrml") };
    let action = roundtrip(d);
    let mut s = fixture(32);
    for (index, alphas) in [[0.5, 0.0, 0.25], [0.25, 0.5, 0.0]].into_iter().enumerate() {
        if index == 1 {
            s.execute("layer.new.layer", json!({"name":"top"})).unwrap();
        }
        s.edit("alpha pattern", |doc, active| {
            let area = doc.bounds();
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            let format = surface.format();
            let pixels: Vec<_> = alphas.into_iter().flat_map(|a| photocraft_raster::from_rgba(&format, [0.5, 0.25, 0.125, a])).collect();
            surface.write_region(area, &pixels);
            Ok(())
        })
        .unwrap();
    }
    let before = saved(&s);
    assert_eq!(play(&mut s, action)["ran"], 1);
    assert_eq!(alpha(&s, 0), vec![0.625, 0.5, 0.25], "top alpha plus base alpha times one minus top alpha");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(saved(&s), before);
    for channel in ["transparency", "Transparency", "TRANSPARENCY"] {
        s.execute("image.calculations",json!({"source1Layer":"merged","source1Channel":channel,"source2Layer":"merged","source2Channel":channel,"blending":"multiply","maskLayer":"merged"})).unwrap();
        assert_eq!(alpha(&s, 0), vec![0.390625, 0.25, 0.0625], "both flat sources retain merged alpha; a channel-less merged mask remains absent");
        s.execute("edit.undo", json!({})).unwrap();
    }
    s.execute("image.calculations", json!({"source1":{"channel":"transparency"},"source2":{"channel":{"color":"red"}},"blending":"normal"})).unwrap();
    assert_eq!(alpha(&s, 0), vec![0.25, 0.5, 0.0], "omitting the layer still reads the active layer's alpha");
}

#[test]
fn atn_calculations_unsupported_targets_and_missing_sources_leave_history_unchanged() {
    let p = json!({"source1":{"channel":{"targeted":true}},"source2":{"channel":{"color":"green"}},"blending":"add"});
    let mut s = fixture(32);
    let before = saved(&s);
    let label = s.active().unwrap().history.undo_label().map(str::to_owned);
    assert!(s.execute("image.calculations", p.clone()).is_err(), "composite is not one channel");
    assert_eq!(saved(&s), before);
    assert_eq!(s.active().unwrap().history.undo_label(), label.as_deref());
    s.execute("channel.target", json!({"channel": "red"})).unwrap();
    s.execute("channel.target", json!({"channel": "blue", "extend": true})).unwrap();
    assert!(s.execute("image.calculations", p).is_err(), "multiple colours are not one channel");
    for channel in [json!({"name":"missing"}), json!({"color":"cyan"})] {
        assert!(s.execute("image.calculations", json!({"source1":{"channel":channel},"source2":{"channel":{"color":"green"}},"blending":"add"})).is_err());
    }
    assert_eq!(saved(&s), before);
    assert_eq!(s.active().unwrap().history.undo_label(), label.as_deref());
}

#[test]
fn atn_calculations_group_transparency_uses_the_canvas_and_undoes() {
    let mut group = fixture(32);
    group.execute("layer.new.group", json!({"name":"source"})).unwrap();
    let before = saved(&group);
    let label = group.active().unwrap().history.undo_label().map(str::to_owned);
    group
        .execute(
            "image.calculations",
            json!({"source1":{"layer":{"name":"source"},"channel":"transparency"},"source2":{"channel":{"color":"green"}},"blending":"add"}),
        )
        .unwrap();
    assert_eq!(alpha(&group, 0), vec![1.0; 3]);
    group.execute("edit.undo", json!({})).unwrap();
    assert_eq!(saved(&group), before);
    assert_eq!(group.active().unwrap().history.undo_label(), label.as_deref());
}

#[test]
fn atn_calculations_unknown_wire_forms_are_kept_and_unrepresentable_native_options_refuse() {
    let original = make(channel("Chnl", "Rd  "), channel("Chnl", "Grn "));
    let mut variants = vec![original.clone().with("extra", D::Boolean(true)), original.clone().with("Nw  ", D::Class(class("Chnl")))];
    let D::Descriptor(using) = original.get("Usng").unwrap() else { panic!() };
    for (key, value) in [("Scl ", D::Integer(1)), ("Ofst", D::Double(0.0)), ("extra", D::Boolean(true))] {
        let mut d = using.clone();
        d.items.retain(|(id, _)| !id.is(key));
        d = d.with(key, value);
        variants.push(Descriptor::new("null").with("Nw  ", D::Class(class("Chnl"))).with("Usng", D::Descriptor(d)));
    }
    let mut omitted_blend = using.clone();
    omitted_blend.items.retain(|(id, _)| !id.is("Clcl"));
    variants.push(Descriptor::new("null").with("Nw  ", D::Class(class("Chnl"))).with("Usng", D::Descriptor(omitted_blend)));
    let mut bad_ref = channel("Chnl", "Rd  ");
    bad_ref.push(ReferenceItem::Index { class: class("Dcmn"), index: 1 });
    variants.push(make(bad_ref, channel("Chnl", "Grn ")));
    for r in [channel("Ordn", "Rd  "), channel("Chnl", "nope"), channel("Chnl", "Trsp")] {
        variants.push(make(r, channel("Chnl", "Grn ")));
    }
    for d in variants {
        let imported = atn_cmds::import(&bytes(d.clone())).unwrap();
        assert_eq!(imported.actions[0].steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        let (saved, _) = atn_cmds::export("Calculations", &[&imported.actions[0]], &[], false).unwrap();
        assert_eq!(atn::parse(&saved).unwrap().actions[0].items[0].descriptor, Some(d));
    }
    let valid = roundtrip(original).steps[0].1.clone();
    for (key, value) in [
        ("result", json!("selection")),
        ("mask", json!({"channel":"red"})),
        ("opacity", json!(50)),
        ("offset", json!(0.5)),
        ("scale", json!(3)),
        ("name", json!("custom")),
    ] {
        let mut p = valid.clone();
        p[key] = value;
        let a = Action::new("Bad", vec![("image.calculations".into(), p)]);
        assert!(atn_cmds::export("Calculations", &[&a], &[], false).is_err(), "{key}");
    }
}

#[test]
fn atn_calculations_make_routing_retains_mask_creation() {
    let d = Descriptor::new("null")
        .with("Nw  ", D::Class(class("Chnl")))
        .with("At  ", D::Reference(vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: Id::new("Chnl"), value: Id::new("Msk ") }]))
        .with("Usng", D::Enumerated { type_id: Id::new("UsrM"), value: Id::new("RvlA") });
    let imported = atn_cmds::import(&bytes(d)).unwrap();
    assert!(imported.unsupported.is_empty());
    assert_eq!(imported.actions[0].steps[0].0, "layer.layerMask.revealAll");
    let mut s = fixture(32);
    assert_eq!(play(&mut s, imported.actions[0].clone())["ran"], 1);
    let doc = &s.active().unwrap().doc;
    assert!(doc.channels.is_empty());
    assert!(doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().mask.is_some());
}

/// Non-default valid scale and offset retain their wire types and affect the calculated pixels.
#[test]
fn atn_calculations_valid_scale_and_offset_roundtrip_and_pixels() {
    for (scale, offset) in [(2.0, 0), (1.0, 32), (2.0, -32)] {
        let mut d = make(channel("Chnl", "Rd  "), channel("Chnl", "Grn "));
        let D::Descriptor(using) = &mut d.items[1].1 else { panic!("Calculations settings") };
        using.items.iter_mut().find(|(k, _)| k.is("Scl ")).unwrap().1 = D::Double(scale);
        using.items.iter_mut().find(|(k, _)| k.is("Ofst")).unwrap().1 = D::Integer(offset);
        let action = roundtrip(d);
        let mut s = fixture(32);
        let input = photocraft_compose::flatten(&s.active().unwrap().doc).into_px();
        assert_eq!(play(&mut s, action)["ran"], 1);
        for (got, rgba) in alpha(&s, 0).iter().zip(input) {
            let expected = ((rgba[0] + rgba[1]) / scale as f32 + offset as f32 / 255.0).clamp(0.0, 1.0);
            assert!((got - expected).abs() < 1e-6, "scale={scale} offset={offset}: {got} vs {expected}");
        }
    }
}
