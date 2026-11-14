//! Synthetic measured-layout recordings through public ATN load/save and playback.
use craft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Descriptor, Id, UnicodeString, Value as D},
};
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use serde_json::json;

fn enm(ty: &str, value: &str) -> D {
    D::Enumerated { type_id: Id::new(ty), value: Id::new(value) }
}
fn pct(value: f64) -> D {
    D::UnitFloat { unit: *b"#Prc", value }
}
fn point(x: f64, y: f64) -> D {
    D::Descriptor(Descriptor::new("Pnt ").with("Hrzn", D::UnitFloat { unit: *b"#Pxl", value: x }).with("Vrtc", D::UnitFloat { unit: *b"#Pxl", value: y }))
}
fn color(t: i32, rgb: [f64; 3]) -> D {
    let c = Descriptor::new("RGBC").with("Rd  ", D::Double(rgb[0])).with("Grn ", D::Double(rgb[1])).with("Bl  ", D::Double(rgb[2]));
    D::Descriptor(
        Descriptor::new("Clrt").with("Clr ", D::Descriptor(c)).with("Type", enm("Clry", "UsrS")).with("Lctn", D::Integer(t)).with("Mdpn", D::Integer(50)),
    )
}
fn opacity(t: i32, value: f64) -> D {
    D::Descriptor(Descriptor::new("TrnS").with("Opct", pct(value)).with("Lctn", D::Integer(t)).with("Mdpn", D::Integer(50)))
}
fn fixture(style: &str, method: &str, changed: bool) -> Descriptor {
    let gradient = Descriptor::new("Grdn")
        .with("Nm  ", D::Text(UnicodeString::new_nul("Recorded gradient")))
        .with("GrdF", enm("GrdF", "CstS"))
        .with("Intr", D::Double(4096.0))
        .with("Clrs", D::List(vec![color(0, [0.0, 32.0, 255.0]), color(4096, [255.0, 224.0, 0.0])]))
        .with("Trns", D::List(vec![opacity(0, 20.0), opacity(4096, 80.0)]));
    let mut d = Descriptor::new("null").with("From", point(1.0, 1.0)).with("T   ", point(6.0, 2.0));
    if changed {
        d = d.with("Opct", pct(54.5)).with("Md  ", enm("BlnM", "Mltp"));
    }
    d = d.with("Type", enm("GrdT", style));
    if changed {
        d = d.with("Rvrs", D::Boolean(true)).with("UsMs", D::Boolean(true));
    }
    d.with("gradientsInterpolationMethod", enm("gradientInterpolationMethodType", method)).with("Grad", D::Descriptor(gradient))
}
fn recording(event: AtnEvent, d: Descriptor) -> Vec<u8> {
    let file = AtnFile {
        name: UnicodeString::new_nul("Set"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Gradient"),
            items: vec![AtnItem { event, descriptor: Some(d), dictionary: b"Gradient".to_vec(), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    };
    atn::write(&file).unwrap()
}
fn imported(d: Descriptor) -> atn_cmds::Imported {
    atn_cmds::import(&recording(AtnEvent::Code(*b"Grdn"), d)).unwrap()
}
fn saved(action: &Action) -> AtnItem {
    let (bytes, skipped) = atn_cmds::export("Set", &[action], &[], false).unwrap();
    assert!(skipped.is_empty());
    atn::parse(&bytes).unwrap().actions[0].items[0].clone()
}
fn replace(d: &mut Descriptor, key: &str, v: D) {
    d.items.iter_mut().find(|(k, _)| k.is(key)).unwrap().1 = v;
}
fn obj<'a>(d: &'a mut Descriptor, key: &str) -> &'a mut Descriptor {
    let D::Descriptor(value) = &mut d.items.iter_mut().find(|(k, _)| k.is(key)).unwrap().1 else { panic!("not an object") };
    value
}

#[test]
fn both_event_ids_and_top_level_classes_keep_complete_unchanged_descriptors() {
    for class in ["null", "Grdn"] {
        for style in ["Lnr ", "Rdl ", "Angl", "Rflc", "Dmnd"] {
            for method in ["Gcls", "Perc", "Lnr ", "Smoo"] {
                for changed in [false, true] {
                    for event in [AtnEvent::Code(*b"Grdn"), AtnEvent::name("gradientClassEvent")] {
                        let mut d = fixture(style, method, changed);
                        d.class_id = Id::new(class);
                        let loaded = atn_cmds::import(&recording(event, d.clone())).unwrap();
                        assert!(loaded.unsupported.is_empty(), "{style} {method}");
                        let action = &loaded.actions[0];
                        assert_eq!(action.steps[0].0, "paint.gradient");
                        assert_eq!(saved(action).descriptor.unwrap().to_bytes(), d.to_bytes());
                        assert_eq!(action.steps[0].1["dither"], false, "omitted Dthr is off, unlike native default");
                        assert_eq!(action.steps[0].1["opacity"], if changed { json!(54.5) } else { json!(100) });
                        assert_eq!(action.steps[0].1["transparency"], if changed { json!([[0, 20], [1, 80]]) } else { json!([]) });
                    }
                }
            }
        }
    }
}

#[test]
fn explicit_defaults_key_order_id_encoding_and_flags_are_preserved() {
    let mut d = fixture("Lnr ", "Gcls", false);
    d.items.push((Id::Str(b"Opct".to_vec()), pct(100.0)));
    d.items.push((Id::new("Dthr"), D::Boolean(false)));
    d.items.push((Id::new("Rvrs"), D::Boolean(false)));
    d.items.reverse();
    let mut loaded = imported(d.clone());
    assert!(loaded.unsupported.is_empty());
    loaded.actions[0].disabled.insert(0);
    loaded.actions[0].dialogs.insert(0);
    let item = saved(&loaded.actions[0]);
    assert_eq!(item.descriptor.unwrap().to_bytes(), d.to_bytes());
    assert!(!item.enabled);
    assert!(item.with_dialog);
}

#[test]
fn supported_edits_replace_stale_source_and_export_recorded_default_omissions() {
    let original = fixture("Rdl ", "Perc", true);
    let mut loaded = imported(original.clone());
    let p = &mut loaded.actions[0].steps[0].1;
    p["opacity"] = json!(100);
    p["mode"] = json!("Normal");
    p["reverse"] = json!(false);
    p["dither"] = json!(true);
    p["transparency"] = json!([]);
    p["from"] = json!([2, 3]);
    let item = saved(&loaded.actions[0]);
    let d = item.descriptor.unwrap();
    assert_ne!(d.to_bytes(), original.to_bytes());
    let keys: Vec<&[u8]> = d.items.iter().map(|(k, _)| k.as_bytes()).collect();
    assert_eq!(keys, vec![b"From".as_slice(), b"T   ", b"Type", b"Dthr", b"gradientsInterpolationMethod", b"Grad"]);
    let back = imported(d);
    assert!(back.unsupported.is_empty());
    assert_eq!(back.actions[0].steps[0].1["from"], json!([2, 3]));
    assert_eq!(back.actions[0].steps[0].1["dither"], true);
    assert_eq!(back.actions[0].steps[0].1["transparency"], json!([]));
}

#[test]
fn malformed_unknown_and_unrepresentable_descriptors_stay_whole() {
    let base = fixture("Lnr ", "Gcls", true);
    let mut bad = Vec::new();
    let mut d = base.clone();
    d.class_id = Id::new("UnknownGradient");
    bad.push(d);
    let mut d = base.clone();
    d.items.retain(|(k, _)| !k.is("Grad"));
    bad.push(d);
    let mut d = base.clone();
    d.items.retain(|(k, _)| !k.is("gradientsInterpolationMethod"));
    bad.push(d);
    let mut d = base.clone();
    replace(&mut d, "Type", enm("GrdT", "Unknown"));
    bad.push(d);
    let mut d = base.clone();
    replace(&mut d, "Type", enm("wrong", "Lnr "));
    bad.push(d);
    let mut d = base.clone();
    replace(&mut d, "Opct", D::Double(54.5));
    bad.push(d);
    let mut d = base.clone();
    d.items.push((Id::new("Dthr"), D::Integer(1)));
    bad.push(d);
    let mut d = base.clone();
    d.items.push((Id::Str(b"Opct".to_vec()), pct(100.0)));
    bad.push(d);
    let mut d = base.clone();
    d.items.push((Id::new("unknownSetting"), D::Boolean(true)));
    bad.push(d);
    let mut d = base.clone();
    replace(&mut d, "gradientsInterpolationMethod", enm("gradientInterpolationMethodType", "unknown"));
    bad.push(d);
    let mut d = base.clone();
    replace(obj(&mut d, "From"), "Hrzn", D::UnitFloat { unit: *b"#Pxl", value: f64::NAN });
    bad.push(d);
    let mut d = base.clone();
    replace(obj(&mut d, "From"), "Hrzn", pct(1.0));
    bad.push(d);
    for (key, value) in [("Mdpn", D::Integer(25)), ("Type", enm("Clry", "FrgC")), ("Lctn", D::Integer(-1))] {
        let mut d = base.clone();
        let D::List(stops) = &mut obj(&mut d, "Grad").items.iter_mut().find(|(k, _)| k.is("Clrs")).unwrap().1 else { panic!() };
        let D::Descriptor(stop) = &mut stops[0] else { panic!() };
        replace(stop, key, value);
        bad.push(d);
    }
    let mut d = base.clone();
    let D::List(stops) = &mut obj(&mut d, "Grad").items.iter_mut().find(|(k, _)| k.is("Clrs")).unwrap().1 else { panic!() };
    let D::Descriptor(stop) = &mut stops[0] else { panic!() };
    replace(obj(stop, "Clr "), "Rd  ", D::Double(0.5));
    bad.push(d);
    for d in bad {
        let bytes = d.to_bytes();
        let loaded = imported(d);
        assert_eq!(loaded.unsupported.len(), 1);
        assert_eq!(loaded.actions[0].steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        assert_eq!(saved(&loaded.actions[0]).descriptor.unwrap().to_bytes(), bytes);
    }
}

#[test]
fn unsupported_native_targets_and_damaged_origins_are_refused() {
    let loaded = imported(fixture("Lnr ", "Gcls", false));
    for (key, value) in [
        ("target", json!("mask")),
        ("gradient", json!("current preset")),
        ("method", json!("unknown")),
        ("style", json!("unknown")),
        ("style", json!(42)),
        ("__atnClassicGradient", json!("not base64")),
        ("__atnClassicGradient", json!("A".repeat(128 * 1024 + 1))),
        ("stops", json!([[0, "foreground"], [1, "background"]])),
        ("stops", json!([[0, "#000000"]])),
        ("stops", json!([[0.1, "#000000"], [1, "#ffffff"]])),
    ] {
        let mut action = loaded.actions[0].clone();
        action.steps[0].1[key] = value;
        assert!(atn_cmds::export("S", &[&action], &[], false).is_err(), "{key}");
        let (_, skipped) = atn_cmds::export("S", &[&action], &[], true).unwrap();
        assert_eq!(skipped.len(), 1, "{key}");
    }
}

#[test]
fn colour_and_transparency_stop_counts_have_supported_and_refused_boundaries() {
    for key in ["Clrs", "Trns"] {
        for count in [1, 2, 64, 65] {
            let mut d = fixture("Lnr ", "Gcls", true);
            let stops = (0..count)
                .map(|i| {
                    let t = i * 4096 / (count - 1).max(1);
                    if key == "Clrs" { color(t, [32.0, 96.0, 224.0]) } else { opacity(t, 50.0) }
                })
                .collect();
            replace(obj(&mut d, "Grad"), key, D::List(stops));
            let bytes = d.to_bytes();
            let loaded = imported(d);
            assert_eq!(loaded.unsupported.is_empty(), (2..=64).contains(&count), "{key} {count}");
            assert_eq!(saved(&loaded.actions[0]).descriptor.unwrap().to_bytes(), bytes);
        }
    }
}

#[test]
fn omitted_transparency_and_dither_use_recorded_defaults_during_playback() {
    for depth in [8, 16, 32] {
        for dither in [false, true] {
            let mut d = fixture("Lnr ", "Gcls", false);
            d.class_id = Id::new("Grdn");
            if dither {
                d.items.push((Id::new("Dthr"), D::Boolean(true)));
            }
            let loaded = imported(d);
            assert!(loaded.unsupported.is_empty());
            let mut actual = Session::new();
            let mut control = Session::new();
            for s in [&mut actual, &mut control] {
                s.execute("file.new", json!({"width":8,"height":4,"depth":depth,"background":"#204060"})).unwrap();
            }
            actual.actions.list = loaded.actions;
            actual.execute("actions.play", json!({"action":0})).unwrap();
            // The descriptor contains non-opaque Trns, but omitted UsMs disables them.
            // Dthr=false is explicit, overriding the native command's true default.
            control
                .execute(
                    "paint.gradient",
                    json!({"from":[1,1],"to":[6,2],"style":"linear","method":"classic","smoothness":100,
                "stops":[[0,"#0020ff"],[1,"#ffe000"]],"transparency":[],"opacity":100,"mode":"Normal","reverse":false,"dither":dither}),
                )
                .unwrap();
            let a = actual.active().unwrap().doc.layers[0].surface().unwrap();
            let b = control.active().unwrap().doc.layers[0].surface().unwrap();
            for y in 0..4 {
                for x in 0..8 {
                    assert_eq!(a.pixel(x, y), b.pixel(x, y), "{depth} dither={dither}");
                }
            }
        }
    }
}

#[test]
fn recorded_gradients_match_native_pixels_and_undo_at_each_depth() {
    for class in ["null", "Grdn"] {
        for depth in [8, 16, 32] {
            for (style, native_style) in [("Lnr ", "linear"), ("Rdl ", "radial"), ("Angl", "angle"), ("Rflc", "reflected"), ("Dmnd", "diamond")] {
                for (method, native_method) in [("Gcls", "classic"), ("Perc", "perceptual"), ("Lnr ", "linear"), ("Smoo", "smooth")] {
                    let mut descriptor = fixture(style, method, true);
                    descriptor.class_id = Id::new(class);
                    let loaded = imported(descriptor);
                    assert!(loaded.unsupported.is_empty());
                    let mut actual = Session::new();
                    let mut control = Session::new();
                    for s in [&mut actual, &mut control] {
                        s.execute("file.new", json!({"width":8,"height":4,"depth":depth,"background":"#204060"})).unwrap();
                    }
                    let before = actual.active().unwrap().doc.clone();
                    actual.actions.list = loaded.actions;
                    actual.execute("actions.play", json!({"action":0})).unwrap();
                    control
                        .execute(
                            "paint.gradient",
                            json!({"from":[1,1],"to":[6,2],"style":native_style,"method":native_method,"smoothness":100,
                    "stops":[[0,"#0020ff"],[1,"#ffe000"]],"transparency":[[0,20],[1,80]],"opacity":54.5,"mode":"Multiply","reverse":true,"dither":false}),
                        )
                        .unwrap();
                    let pixels = |s: &Session| s.active().unwrap().doc.layers[0].surface().unwrap().clone();
                    let (a, b) = (pixels(&actual), pixels(&control));
                    for y in 0..4 {
                        for x in 0..8 {
                            assert_eq!(a.pixel(x, y), b.pixel(x, y), "{depth} {style} {method} ({x},{y})");
                        }
                    }
                    assert_ne!(a.pixel(3, 1), before.layers[0].surface().unwrap().pixel(3, 1), "non-vacuous paint");
                    assert!(actual.undo());
                    assert_eq!(actual.active().unwrap().doc, before);
                }
            }
        }
    }
}
