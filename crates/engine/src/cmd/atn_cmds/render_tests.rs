use super::*;

/// A real action file with one step, used by both descriptor and playback checks.
fn file(event: AtnEvent, descriptor: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Render set"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Render"),
            items: vec![AtnItem { event, dictionary: b"Render".to_vec(), descriptor: Some(descriptor), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

/// Measured descriptor order and types, independent of the converter's field construction.
fn fibers(variance: i32, strength: i32, seed: i32) -> Descriptor {
    desc("null").with("Vrnc", D::Integer(variance)).with("Strg", D::Integer(strength)).with("RndS", D::Integer(seed))
}

/// The measured center is a Point object with two doubles, not pixel or percent units.
fn flare(brightness: i32, center: [f64; 2], lens: &str) -> Descriptor {
    desc("null")
        .with("Brgh", D::Integer(brightness))
        .with("FlrC", D::Descriptor(desc("Pnt ").with("Hrzn", D::Double(center[0])).with("Vrtc", D::Double(center[1]))))
        .with("Lns ", enm("Lns ", lens))
}

/// Codes and TEXT names have separate identities, including the old Code-only aliases.
#[test]
fn event_lookup_keeps_codes_separate_from_names() {
    for (code, id) in [
        (*b"Clds", "filter.render.clouds"),
        (*b"DfrC", "filter.render.differenceClouds"),
        (*b"Fbrs", "filter.render.fibers"),
        (*b"LnsF", "filter.render.lensFlare"),
        (*b"BoxB", "filter.blur.boxBlur"),
        (*b"SrfB", "filter.blur.surfaceBlur"),
        (*b"Fltn", "layer.flattenImage"),
        (*b"BlnW", "image.adjustments.blackWhite"),
        (*b"PhtF", "image.adjustments.photoFilter"),
    ] {
        assert!(event_for(&AtnEvent::Code(code)).unwrap().ids.contains(&id));
        assert!(event_for(&AtnEvent::Name(code.to_vec())).is_none(), "{code:?} is not a TEXT name");
    }
    assert!(event_for(&AtnEvent::name("play")).unwrap().ids.contains(&"actions.play"));
    assert!(event_for(&AtnEvent::Code(*b"Ply ")).unwrap().ids.contains(&"actions.play"));
    assert!(event_for(&AtnEvent::Code(*b"play")).is_none(), "a TEXT name is not a code");
    for event in [AtnEvent::Code(*b"save"), AtnEvent::name("save")] {
        assert!(event_for(&event).unwrap().ids.contains(&"file.save"), "save is registered in both forms");
    }
    assert!(event_for(&AtnEvent::name("")).is_none(), "empty registry names are not event identifiers");
}

/// Unknown event forms stay kept rather than becoming commands, and save verbatim.
#[test]
fn code_spellings_in_text_steps_keep_their_bytes_and_flags() {
    let mut events: Vec<_> = ["Clds", "DfrC", "Fbrs", "LnsF", "BoxB", "SrfB", "Fltn", "BlnW", "PhtF"].into_iter().map(AtnEvent::name).collect();
    events.push(AtnEvent::Code(*b"play"));
    for event in events {
        let mut original = atn::parse(&file(event, desc("null"))).unwrap();
        original.actions[0].items[0].enabled = false;
        original.actions[0].items[0].with_dialog = true;
        original.actions[0].items[0].dialog_options = 2;
        let bytes = atn::write(&original).unwrap();
        let mut s = Session::new();
        let loaded = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&bytes)})).unwrap();
        assert_eq!(loaded["unsupported"].as_array().unwrap().len(), 1, "{:?}", original.actions[0].items[0].event);
        assert_eq!(s.actions.list[0].steps[0].0, PHOTOSHOP_STEP);
        let saved = s.execute("actions.save", json!({"action": "Render"})).unwrap();
        assert_eq!(saved["skipped"], json!([]));
        let back = atn::parse(&photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(back.actions[0].items, original.actions[0].items);
        let mut compared = original;
        compared.actions[0].items = back.actions[0].items.clone();
        assert_eq!(atn::write(&compared).unwrap(), bytes);
    }
}

/// Both recorded event IDs map empty Clouds descriptors without inventing frozen colors.
#[test]
fn clouds_events_map_and_refuse_unrepresentable_native_settings() {
    for (code, name, id) in [(*b"Clds", "clouds", "filter.render.clouds"), (*b"DfrC", "differenceClouds", "filter.render.differenceClouds")] {
        for event in [AtnEvent::Code(code), AtnEvent::name(name)] {
            let file = AtnFile {
                actions: vec![AtnAction {
                    name: UnicodeString::new_nul("Render"),
                    items: vec![AtnItem { event, descriptor: Some(desc("null")), ..Default::default() }],
                    ..Default::default()
                }],
                ..Default::default()
            };
            let loaded = import(&atn::write(&file).unwrap()).unwrap();
            assert!(loaded.unsupported.is_empty());
            assert_eq!(loaded.actions[0].steps, vec![(id.into(), json!({}))]);
            assert_eq!(export_item(id, &json!({}), "Render").unwrap().descriptor, Some(desc("null")));
        }
        for p in [json!({"seed": 7}), json!({"foreground": "#ff0000"}), json!({"background": "#0000ff"}), json!({"layer": 7}), json!({"target": "mask"})] {
            assert!(export_item(id, &p, "Render").is_none(), "{id}: {p}");
        }
        // The Clouds dialog records `seed: 0`, which is what the empty imported step plays as.
        for p in [json!({"seed": 0}), json!({"seed": 0.0}), json!({"seed": 0, "target": "pixels"})] {
            assert_eq!(export_item(id, &p, "Render").map(|item| item.descriptor), Some(Some(desc("null"))), "{id}: {p}");
        }
        for p in [json!({"seed": -1}), json!({"seed": 0.5}), json!({"seed": "0"}), json!({"seed": 0, "foreground": "#ff0000"})] {
            assert!(export_item(id, &p, "Render").is_none(), "{id}: {p}");
        }
    }
}

/// A recorded dialog step saves through the real Save Actions command.
#[test]
fn clouds_recorded_through_the_dialog_save() {
    for id in ["filter.render.clouds", "filter.render.differenceClouds"] {
        let mut s = Session::new();
        s.actions.list.push(Action::new("Render", vec![(id.into(), json!({"seed": 0}))]));
        let saved = s.execute("actions.save", json!({"action": "Render"})).unwrap();
        assert_eq!(saved["skipped"], json!([]), "{id}");
        let back = import(&photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(back.actions[0].steps, vec![(id.into(), json!({}))], "{id}");
    }
}

/// Defaults and changed descriptors survive the actual load/save commands, including key order.
#[test]
fn render_descriptors_load_and_save_without_losing_values() {
    let center = [0.40000000596046; 2];
    let mut cases = vec![("Fbrs", None, fibers(16, 4, 123)), ("LnsF", Some("lensFlare"), flare(100, center, "Zm"))];
    for seed in [i32::MIN, -1, 0, i32::MAX] {
        cases.push(("Fbrs", None, fibers(-23, i32::MAX, seed)));
    }
    for lens in ["Zm", "Nkn", "Nkn1", "PnVs"] {
        cases.push(("LnsF", Some("lensFlare"), flare(280, [5.0, -5.0], lens)));
    }
    for (code_id, name, descriptor) in cases {
        let mut events = vec![AtnEvent::Code(code_id.as_bytes().try_into().unwrap())];
        if let Some(name) = name {
            events.push(AtnEvent::name(name));
        }
        for event in events {
            let mut s = Session::new();
            let loaded = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&file(event, descriptor.clone()))})).unwrap();
            assert_eq!(loaded["unsupported"], json!([]));
            let saved = s.execute("actions.save", json!({"action": "Render"})).unwrap();
            let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            let back = atn::parse(&bytes).unwrap();
            assert_eq!(back.actions[0].items[0].event.id(), code_id);
            assert_eq!(back.actions[0].items[0].descriptor, Some(descriptor.clone()));
        }
    }
}

/// Missing import fields use measured defaults; native export keeps the command's own defaults.
#[test]
fn render_defaults_seeds_and_brightness_bounds_are_explicit() {
    let bytes = file(AtnEvent::Code(*b"Fbrs"), desc("null"));
    let action = import(&bytes).unwrap().actions.remove(0);
    let p = &action.steps[0].1;
    assert_eq!(p["variance"], 16);
    assert_eq!(p["strength"], 4);
    assert!(i32::try_from(p["seed"].as_i64().unwrap()).is_ok());
    let saved = export("S", &[&action], &[], false).unwrap().0;
    assert_eq!(saved, export("S", &[&action], &[], false).unwrap().0, "the captured seed is stable");
    assert_eq!(import(&saved).unwrap().actions[0].steps, action.steps);
    for (brightness, expected) in [(-500, 10), (100, 100), (500, 300)] {
        let action = import(&file(AtnEvent::Code(*b"LnsF"), desc("null").with("Brgh", D::Integer(brightness)))).unwrap().actions.remove(0);
        assert_eq!(action.steps[0].1, json!({"brightness": expected, "centerX": 0.40000000596046, "centerY": 0.40000000596046, "lens": "zoom"}));
    }
    let native = export_item("filter.render.lensFlare", &json!({}), "S").unwrap();
    let back = import(&file(native.event, native.descriptor.unwrap())).unwrap().actions.remove(0);
    assert_eq!(back.steps[0].1["centerX"], 0.5);
    assert_eq!(
        crate::filters_ext::params_for("filter.render.lensFlare", &back.steps[0].1),
        crate::filters_ext::params_for("filter.render.lensFlare", &json!({}))
    );
    let unsigned = export_item("filter.render.fibers", &json!({"seed": u32::MAX}), "S").unwrap();
    assert_eq!(unsigned.descriptor.unwrap().get("RndS"), Some(&D::Integer(-1)));
}

/// Unknown enums, malformed types and unmeasured extra fields stay visibly unsupported.
#[test]
fn unsupported_render_descriptors_keep_their_bytes_and_flags() {
    for descriptor in [
        desc("null").with("Lns ", enm("Lns ", "Bad!")),
        desc("null").with("Lns ", enm("Other", "Zm")),
        desc("null").with("Brgh", D::Double(100.0)),
        desc("null").with("FlrC", D::Descriptor(desc("Other"))),
        desc("null").with("FlrC", D::Descriptor(desc("Pnt ").with("Hrzn", px(0.4)))),
        desc("null").with("futureSetting", D::Boolean(true)),
    ] {
        let mut original = atn::parse(&file(AtnEvent::Code(*b"LnsF"), descriptor.clone())).unwrap();
        original.actions[0].items[0].enabled = false;
        original.actions[0].items[0].with_dialog = true;
        original.actions[0].items[0].dialog_options = 2;
        let loaded = import(&atn::write(&original).unwrap()).unwrap();
        assert_eq!(loaded.unsupported.len(), 1);
        assert_eq!(loaded.actions[0].steps[0].0, PHOTOSHOP_STEP);
        let bytes = export("Render set", &[&loaded.actions[0]], &[], false).unwrap().0;
        assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0], original.actions[0].items[0]);
    }
    for (event, descriptor) in [(*b"Fbrs", desc("null").with("RndS", D::Double(1.0))), (*b"Clds", desc("null").with("FlRs", D::Integer(1)))] {
        assert_eq!(import(&file(AtnEvent::Code(event), descriptor)).unwrap().unsupported.len(), 1);
    }
    for id in ["filter.render.fibers", "filter.render.lensFlare"] {
        for p in [json!({"target": "mask"}), json!({"foreground": "#ff0000"}), json!({"layer": "Other"}), json!({"unknown": true})] {
            assert!(export_item(id, &p, "S").is_none(), "{id}: {p}");
        }
    }
    for p in [json!({"seed": 4294967296u64}), json!({"seed": -2147483649i64}), json!({"variance": 1.5})] {
        assert!(export_item("filter.render.fibers", &p, "S").is_none(), "{p}");
    }
    assert!(export_item("filter.render.lensFlare", &json!({"lens": "unknown"}), "S").is_none());
}

/// Duplicate keys must remain a kept step, rather than losing later values on save.
#[test]
fn duplicate_render_keys_keep_their_bytes_and_flags() {
    let mut cases = Vec::new();
    for key in ["Vrnc", "Strg", "RndS"] {
        cases.push(("Fbrs", key, fibers(16, 4, 123).with(key, D::Integer(99))));
    }
    for key in ["Brgh", "FlrC", "Lns "] {
        let descriptor = flare(100, [0.4; 2], "Zm");
        let duplicate = descriptor.get(key).unwrap().clone();
        cases.push(("LnsF", key, descriptor.with(key, duplicate)));
    }
    for key in ["Hrzn", "Vrtc"] {
        let point = desc("Pnt ").with("Hrzn", D::Double(0.4)).with("Vrtc", D::Double(0.4)).with(key, D::Double(0.8));
        cases.push(("LnsF", key, desc("null").with("FlrC", D::Descriptor(point))));
    }
    for (event, key, descriptor) in cases {
        let mut original = atn::parse(&file(AtnEvent::Code(event.as_bytes().try_into().unwrap()), descriptor)).unwrap();
        original.actions[0].items[0].enabled = false;
        original.actions[0].items[0].with_dialog = true;
        original.actions[0].items[0].dialog_options = 2;
        let bytes = atn::write(&original).unwrap();
        let mut s = Session::new();
        let loaded = s.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&bytes)})).unwrap();
        assert_eq!(loaded["unsupported"].as_array().unwrap().len(), 1, "{event}/{key}");
        assert_eq!(s.actions.list[0].steps[0].0, PHOTOSHOP_STEP);
        let saved = s.execute("actions.save", json!({"action": "Render"})).unwrap();
        assert_eq!(saved["skipped"], json!([]));
        let back = atn::parse(&photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(back.actions[0].items, original.actions[0].items, "{event}/{key}");
        // Keep the original set/action metadata to compare the encoded step bytes.
        let mut compared = original.clone();
        compared.actions[0].items = back.actions[0].items.clone();
        assert_eq!(atn::write(&compared).unwrap(), bytes, "{event}/{key}");
    }
}

/// Imported playback uses current tool colors and the same filter path as direct commands.
#[test]
fn render_actions_match_direct_filters_and_undo() {
    let cases = [
        (*b"Clds", desc("null"), "filter.render.clouds", json!({})),
        (*b"DfrC", desc("null"), "filter.render.differenceClouds", json!({})),
        (*b"Fbrs", fibers(23, 6, -7), "filter.render.fibers", json!({"variance": 23, "strength": 6, "seed": -7})),
        (*b"LnsF", flare(240, [5.0, -5.0], "Nkn1"), "filter.render.lensFlare", json!({"brightness": 240, "centerX": 5.0, "centerY": -5.0, "lens": "prime105"})),
        // Just right of the canvas: the dialog's 0..1 range does not clamp a recorded centre.
        (*b"LnsF", flare(100, [1.4, 0.25], "Zm"), "filter.render.lensFlare", json!({"brightness": 100, "centerX": 1.4, "centerY": 0.25, "lens": "zoom"})),
    ];
    for mode in ["rgb", "gray"] {
        for depth in [8, 16, 32] {
            for (event, descriptor, id, params) in &cases {
                let setup = || {
                    let mut s = Session::new();
                    s.execute("file.new", json!({"width": 8, "height": 6, "mode": mode, "depth": depth, "background": "transparent"})).unwrap();
                    s.execute("edit.fill", json!({"contents": "color", "color": "#304050"})).unwrap();
                    s.execute("tools.setColors", json!({"foreground": "#fe1020", "background": "#1050e0"})).unwrap();
                    s
                };
                let mut action = setup();
                let mut direct = setup();
                let before = action.execute("document.pixel", json!({"x": 3, "y": 2})).unwrap();
                action.execute("actions.load", json!({"data": photocraft_paint::tile::b64_encode(&file(AtnEvent::Code(*event), descriptor.clone()))})).unwrap();
                let played = action.execute("actions.play", json!({"action": "Render"})).unwrap();
                assert!(played.get("failed").is_none(), "{played}");
                direct.execute(id, params.clone()).unwrap();
                let surface = |s: &Session| {
                    let st = s.active().unwrap();
                    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(st.doc.bounds())
                };
                assert_eq!(surface(&action), surface(&direct), "{mode}/{depth}/{id}");
                action.execute("edit.undo", json!({})).unwrap();
                assert_eq!(action.execute("document.pixel", json!({"x": 3, "y": 2})).unwrap(), before);
            }
        }
    }
}
