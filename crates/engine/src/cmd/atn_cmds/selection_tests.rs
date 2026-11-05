use super::*;

/// An independently constructed selection-property reference, as recorded in Group F.
fn selection() -> D {
    D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: code("fsel") }])
}

/// Bounds stay in their recorded Top/Left/Bottom/Right order and pixel-double units.
fn rectangle(bounds: [f64; 4], ellipse: bool) -> Descriptor {
    let mut shape = desc(if ellipse { "Elps" } else { "Rctn" });
    for (key, value) in ["Top ", "Left", "Btom", "Rght"].into_iter().zip(bounds) {
        shape = shape.with(key, px(value));
    }
    let mut d = desc("null").with("null", selection()).with("T   ", D::Descriptor(shape));
    if ellipse {
        d = d.with("AntA", D::Boolean(true));
    }
    d
}

/// One real action file, shared by descriptor and engine playback witnesses.
fn file(event: AtnEvent, descriptor: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Group F"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Step"),
            items: vec![AtnItem { event, dictionary: b"Step".to_vec(), descriptor: Some(descriptor), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

/// Recorded and changed descriptors shared by successful and kept-step round trips.
fn fixtures() -> Vec<(&'static str, Option<&'static str>, Descriptor)> {
    let mut cases = vec![
        ("AddT", Some("addTo"), rectangle([30.0, 30.0, 90.0, 90.0], false)),
        ("AddT", Some("addTo"), rectangle([30.0, 30.0, 90.0, 90.0], true)),
        ("SbtF", Some("subtractFrom"), rectangle([30.0, 30.0, 60.0, 60.0], false)),
        ("IntW", Some("intersectWith"), rectangle([30.0, 30.0, 120.0, 120.0], false)),
        ("PstI", Some("pasteInto"), desc("null").with("AntA", enm("Annt", "Anno")).with("As  ", D::Class(class("Pxel")))),
        ("PstO", Some("pasteOutside"), desc("null").with("AntA", enm("Annt", "Anno")).with("As  ", D::Class(class("Pxel")))),
        ("Fade", Some("fade"), desc("null").with("Opct", pct(50.0)).with("Md  ", enm("BlnM", "Mltp"))),
        ("", Some("linkSelectedLayers"), desc("null").with("null", target_ref("Lyr "))),
        ("Dstr", Some("distribute"), desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", "AdLf"))),
        ("RmvL", Some("removeLayerMask"), desc("null")),
        ("DfnP", Some("definePattern"), desc("null")),
        ("MrgL", Some("mergeLayers"), desc("null")),
    ];
    // Opposite-signed fractions can lose an endpoint through x+(right-x); preserve it explicitly.
    for mode in ["AddT", "SbtF", "IntW"] {
        cases.push((mode, None, rectangle([-0.1, -0.1, 0.3, 0.2], false)));
    }
    let mut ellipse = rectangle([1.5, 2.5, 8.5, 10.5], true);
    ellipse.items.last_mut().unwrap().1 = D::Boolean(false);
    cases.push(("AddT", Some("addTo"), ellipse));
    for mode in ["Scrn", "H", "linearDodge"] {
        cases.push(("Fade", Some("fade"), desc("null").with("Opct", pct(25.5)).with("Md  ", enm("BlnM", mode))));
    }
    for edge in ["AdCH", "AdRg", "AdTp", "AdCV", "AdBt"] {
        cases.push(("Dstr", Some("distribute"), desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", edge))));
    }
    cases
}

/// Both recorded event forms retain explicit example settings and descriptor key/type order.
#[test]
fn group_f_load_save_preserves_recorded_descriptors() {
    for (id, name, d) in fixtures() {
        let mut events = Vec::new();
        if !id.is_empty() {
            events.push(AtnEvent::Code(id.as_bytes().try_into().unwrap()));
        }
        if let Some(name) = name {
            events.push(AtnEvent::name(name));
        }
        for event in events {
            let mut s = Session::new();
            let loaded = s.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&file(event.clone(),d.clone()))})).unwrap();
            assert_eq!(loaded["unsupported"], json!([]), "{id}");
            let saved = s.execute("actions.save", json!({"action":"Step"})).unwrap();
            let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
            let back = atn::parse(&bytes).unwrap();
            assert_eq!(back.actions[0].items[0].descriptor, Some(d.clone()), "{id}");
            assert_eq!(back.actions[0].items[0].event.id(), if id.is_empty() { name.unwrap() } else { id });
        }
    }
}

/// A kept step retains its descriptor bytes, event identity and playback flags.
fn assert_kept(id: &str, name: Option<&str>, descriptor: Descriptor) {
    let mut original = atn::parse(&file(fixture_event(id, name), descriptor.clone())).unwrap();
    original.actions[0].items[0].enabled = false;
    original.actions[0].items[0].with_dialog = true;
    original.actions[0].items[0].dialog_options = 2;
    let mut s = Session::new();
    let loaded = s.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&atn::write(&original).unwrap())})).unwrap();
    assert_eq!(loaded["unsupported"].as_array().unwrap().len(), 1, "{id}/{name:?}/{descriptor:?}");
    assert_eq!(s.actions.list[0].steps[0].0, PHOTOSHOP_STEP);
    assert!(s.actions.list[0].disabled.contains(&0));
    assert!(s.actions.list[0].dialogs.contains(&0));
    let saved = s.execute("actions.save", json!({"action":"Step"})).unwrap();
    assert_eq!(saved["skipped"], json!([]));
    let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
    let back = atn::parse(&bytes).unwrap();
    let saved = &back.actions[0].items[0];
    let original = &original.actions[0].items[0];
    assert_eq!(saved.event, original.event);
    assert_eq!((saved.enabled, saved.with_dialog, saved.dialog_options), (original.enabled, original.with_dialog, original.dialog_options));
    assert_eq!(saved.descriptor.as_ref().unwrap().to_bytes(), descriptor.to_bytes());
}

/// The event of a fixture, in its code form where it has one.
fn fixture_event(id: &str, name: Option<&str>) -> AtnEvent {
    if id.is_empty() { AtnEvent::name(name.unwrap()) } else { AtnEvent::Code(id.as_bytes().try_into().unwrap()) }
}

/// The outer name and class and the nested shape's name and class encoding do not gate mapping:
/// such a step maps to the same command and params as the empty-name `null` header.
#[test]
fn noncanonical_group_f_headers_map_like_the_canonical_form() {
    for (id, name, d) in fixtures() {
        let event = fixture_event(id, name);
        let canonical = import(&file(event.clone(), d.clone())).unwrap();
        assert!(canonical.unsupported.is_empty(), "{id}");
        for variant in 0..5 {
            let mut descriptor = d.clone();
            match variant {
                0 => descriptor.name = UnicodeString::new_nul("Recorded descriptor name"),
                1 => descriptor.class_id = code("Lyr "),
                2 => descriptor.class_id = Id::Str(b"null".to_vec()),
                _ => {
                    let Some((_, D::Descriptor(shape))) = descriptor.items.iter_mut().find(|(key, _)| key.is("T   ")) else { continue };
                    if variant == 3 {
                        shape.name = UnicodeString::new_nul("Recorded shape name");
                    } else {
                        shape.class_id = Id::Str(shape.class_id.as_bytes().to_vec());
                    }
                }
            }
            let loaded = import(&file(event.clone(), descriptor.clone())).unwrap();
            assert!(loaded.unsupported.is_empty(), "{id}/{name:?}/{variant}");
            assert_eq!(loaded.actions[0].steps, canonical.actions[0].steps, "{id}/{name:?}/{variant}");
        }
    }
}

/// The reference application names each step descriptor after the localized step (with a NUL),
/// gives it the event code as its class and names nested shapes too (synthetic descriptors).
#[test]
fn group_f_maps_steps_recorded_with_the_application_header() {
    let mut shape = Descriptor { name: UnicodeString::new_nul(""), class_id: code("Rctn"), items: Vec::new() };
    for (key, value) in ["Top ", "Left", "Btom", "Rght"].into_iter().zip([30.0, 30.0, 90.0, 90.0]) {
        shape = shape.with(key, px(value));
    }
    let cases = [
        (
            AtnEvent::name("fade"),
            Descriptor {
                name: UnicodeString::new_nul("Fade"),
                class_id: code("Fade"),
                items: vec![(code("Opct"), pct(50.0)), (code("Md  "), enm("BlnM", "Nrml"))],
            },
            ("edit.fade", json!({"opacity":50,"mode":"Normal"})),
        ),
        (
            AtnEvent::name("addTo"),
            Descriptor {
                name: UnicodeString::new_nul("Add To"),
                class_id: code("AddT"),
                items: vec![(code("null"), selection()), (code("T   "), D::Descriptor(shape))],
            },
            ("select.rect", json!({"x":30,"y":30,"width":60,"height":60,"mode":"add"})),
        ),
        (
            AtnEvent::Code(*b"MrgL"),
            Descriptor { name: UnicodeString::new_nul("Merge Layers"), class_id: code("MrgL"), items: Vec::new() },
            ("layer.mergeLayers", json!({"_atnEvent":"MrgL"})),
        ),
    ];
    for (event, d, (command, params)) in cases {
        let loaded = import(&file(event, d)).unwrap();
        assert!(loaded.unsupported.is_empty(), "{command}");
        assert_eq!(loaded.actions[0].steps, vec![(command.to_string(), params)]);
    }
}

/// Steps recorded with the reference application's string IDs map like their four-character codes.
#[test]
fn group_f_string_ids_map_like_their_codes() {
    for (id, name, d) in [
        ("IntW", "intersectWith", rectangle([30.0, 30.0, 120.0, 120.0], false)),
        ("Dstr", "distribute", desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", "AdLf"))),
        ("MrgL", "mergeLayers", desc("null")),
        // The run-time names the reference application reports for IntW and Dstr.
        ("IntW", "interfaceWhite", rectangle([30.0, 30.0, 120.0, 120.0], false)),
        ("Dstr", "distort", desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", "AdLf"))),
    ] {
        let by_code = import(&file(AtnEvent::Code(id.as_bytes().try_into().unwrap()), d.clone())).unwrap();
        let by_name = import(&file(AtnEvent::name(name), d)).unwrap();
        assert!(by_code.unsupported.is_empty() && by_name.unsupported.is_empty(), "{name}");
        assert_eq!(by_name.actions[0].steps, by_code.actions[0].steps, "{name}");
    }
}

/// Every keyed converter rejects alternate key encodings and adjacent order changes.
#[test]
fn noncanonical_group_f_keys_keep_their_bytes_and_flags() {
    let variants = |d: &Descriptor| {
        let mut variants = Vec::new();
        for index in 0..d.items.len() {
            let mut changed = d.clone();
            changed.items[index].0 = Id::Str(changed.items[index].0.as_bytes().to_vec());
            variants.push(changed);
            if index > 0 {
                let mut changed = d.clone();
                changed.items.swap(index - 1, index);
                variants.push(changed);
            }
        }
        variants
    };
    for (id, name, d) in fixtures() {
        for changed in variants(&d) {
            assert_kept(id, name, changed);
        }
        for (index, (_, value)) in d.items.iter().enumerate() {
            let D::Descriptor(shape) = value else { continue };
            for changed in variants(shape) {
                let mut outer = d.clone();
                outer.items[index].1 = D::Descriptor(changed);
                assert_kept(id, name, outer);
            }
        }
    }
}

/// Enum type/value encodings and Fade aliases must not be normalized on save.
#[test]
fn noncanonical_group_f_enums_keep_their_bytes_and_flags() {
    for (id, name, d) in fixtures() {
        for (index, (_, value)) in d.items.iter().enumerate() {
            let D::Enumerated { type_id, value } = value else { continue };
            let mut padded = value.as_bytes().to_vec();
            padded.push(b' ');
            for (type_id, value) in [
                (Id::Str(type_id.as_bytes().to_vec()), value.clone()),
                (type_id.clone(), Id::Str(value.as_bytes().to_vec())),
                (type_id.clone(), Id::Str(padded)),
            ] {
                let mut changed = d.clone();
                changed.items[index].1 = D::Enumerated { type_id, value };
                if changed != d {
                    assert_kept(id, name, changed);
                }
            }
        }
    }
    for alias in ["normal", "multiply", "hue"] {
        assert_kept("Fade", Some("fade"), desc("null").with("Opct", pct(50.0)).with("Md  ", enm("BlnM", alias)));
    }
}

/// Fade writes native defaults and clamps only its documented opacity range.
#[test]
fn fade_defaults_and_opacity_limits_are_explicit() {
    let native = export_item("edit.fade", &json!({}), "S").unwrap().descriptor.unwrap();
    assert_eq!(native, desc("null").with("Opct", pct(100.0)).with("Md  ", enm("BlnM", "Nrml")));
    for (opacity, expected) in [(-5.0, 0.0), (0.0, 0.0), (100.0, 100.0), (105.0, 100.0)] {
        let bytes = file(AtnEvent::Code(*b"Fade"), desc("null").with("Opct", pct(opacity)).with("Md  ", enm("BlnM", "Nrml")));
        let loaded = import(&bytes).unwrap();
        assert!(loaded.unsupported.is_empty());
        assert_eq!(loaded.actions[0].steps[0].1["opacity"].as_f64(), Some(expected));
        let saved = export("Group F", &[&loaded.actions[0]], &[], false).unwrap().0;
        let d = atn::parse(&saved).unwrap().actions[0].items[0].descriptor.clone().unwrap();
        assert_eq!(d, desc("null").with("Opct", pct(expected)).with("Md  ", enm("BlnM", "Nrml")));
    }
}

/// Native exports keep their current event; only imported old mask/merge steps select old IDs.
#[test]
fn group_f_legacy_origin_is_allowlisted_and_does_not_replace_native_events() {
    for (id, old, current) in [("layer.layerMask.delete", "RmvL", "Dlt "), ("layer.mergeLayers", "MrgL", "Mrg2")] {
        assert_eq!(export_item(id, &json!({}), "S").unwrap().event.id(), current);
        let params = json!({"_atnEvent":old});
        assert_eq!(export_item(id, &params, "S").unwrap().event.id(), old);
        assert!(export_item(id, &json!({"_atnEvent":"arbitraryEvent"}), "S").is_none());
    }
    assert!(export_item("edit.definePattern", &json!({"_atnEvent":"MrgL"}), "S").is_none());
    assert!(export_item("select.rect", &json!({"x":0,"y":0,"width":1,"height":1,"mode":"replace","_atnBounds":[0,0,1,1]}), "S").is_none());
    assert!(export_item("select.rect", &json!({"x":0,"y":0,"width":2,"height":1,"mode":"add","_atnBounds":[0,0,1,1]}), "S").is_none());
}

/// Missing settings, unknown enums, repeated keys and wrong targets stay kept with their flags.
#[test]
fn group_f_unsupported_settings_are_not_silently_replaced() {
    let mut wrong_ref = rectangle([1.0, 2.0, 3.0, 4.0], false);
    wrong_ref.items[0].1 = target_ref("Lyr ");
    let mut cases = vec![
        (*b"AddT", desc("null")),
        (*b"AddT", wrong_ref),
        (*b"PstI", desc("null").with("AntA", enm("Annt", "Unknown")).with("As  ", D::Class(class("Pxel")))),
        (*b"PstO", desc("null").with("AntA", enm("Annt", "Anno")).with("As  ", D::Class(class("Other")))),
        (*b"Fade", desc("null").with("Opct", pct(50.0))),
        (*b"Fade", desc("null").with("Opct", px(50.0)).with("Md  ", enm("BlnM", "Nrml"))),
        (*b"Dstr", desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", "Unknown"))),
        (*b"RmvL", desc("null").with("apply", D::Boolean(true))),
    ];
    for (event, d) in [
        (*b"AddT", rectangle([1.0, 2.0, 3.0, 4.0], true)),
        (*b"PstI", desc("null").with("AntA", enm("Annt", "Anno")).with("As  ", D::Class(class("Pxel")))),
        (*b"Fade", desc("null").with("Opct", pct(50.0)).with("Md  ", enm("BlnM", "Nrml"))),
        (*b"Dstr", desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", "AdLf"))),
    ] {
        for item in &d.items {
            let mut repeated = d.clone();
            repeated.items.push(item.clone());
            cases.push((event, repeated));
        }
    }
    for key in ["Top ", "Left", "Btom", "Rght"] {
        let mut d = rectangle([1.0, 2.0, 3.0, 4.0], false);
        let D::Descriptor(shape) = &mut d.items[1].1 else { panic!("rectangle fixture must contain a shape") };
        shape.items.push((code(key), px(9.0)));
        cases.push((*b"AddT", d));
    }
    for (event, d) in cases {
        let mut original = atn::parse(&file(AtnEvent::Code(event), d)).unwrap();
        original.actions[0].items[0].enabled = false;
        original.actions[0].items[0].with_dialog = true;
        original.actions[0].items[0].dialog_options = 2;
        let loaded = import(&atn::write(&original).unwrap()).unwrap();
        assert_eq!(loaded.unsupported.len(), 1);
        assert_eq!(loaded.actions[0].steps[0].0, PHOTOSHOP_STEP);
        let saved = export("Group F", &[&loaded.actions[0]], &[], false).unwrap().0;
        assert_eq!(atn::parse(&saved).unwrap().actions[0].items[0], original.actions[0].items[0]);
    }
    for (id, p) in [
        ("edit.pasteSpecial.pasteInto", json!({"center":[1,2]})),
        ("edit.pasteSpecial.pasteOutside", json!({"target":"mask"})),
        ("layer.linkLayers", json!({"layer":"Other"})),
        ("layer.distribute.leftEdges", json!({"layer":7})),
        ("edit.definePattern", json!({"name":"Frozen name"})),
        ("edit.fade", json!({"mode":"unknown"})),
        ("select.rect", json!({"x":1,"y":2,"width":3,"height":4,"mode":"add","feather":2})),
    ] {
        assert!(export_item(id, &p, "S").is_none(), "{id}: {p}");
    }
}

/// Execute the action name assigned by the loader, including collision-safe renaming.
fn play(s: &mut Session, event: AtnEvent, d: Descriptor) -> Value {
    let loaded = s.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&file(event,d))})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]));
    s.execute("actions.play", json!({"action":loaded["actions"][0]})).unwrap()
}

/// A small image with real pixels; all witnesses operate through the public command boundary.
fn image(depth: u8) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":8,"depth":depth,"background":"transparent"})).unwrap();
    s.execute("edit.fill", json!({"contents":"color","color":"#4080c0"})).unwrap();
    s
}

/// Compare the actual stored selection, rather than only the translated command params.
#[test]
fn selection_combinations_play_on_an_existing_mask_and_undo() {
    for depth in [8, 16, 32] {
        for (event, mode, ellipse) in [(*b"AddT", "add", false), (*b"AddT", "add", true), (*b"SbtF", "subtract", false), (*b"IntW", "intersect", false)] {
            let mut actual = image(depth);
            let mut direct = image(depth);
            for s in [&mut actual, &mut direct] {
                s.execute("select.rect", json!({"x":1,"y":1,"width":4,"height":4})).unwrap();
            }
            let mask = |s: &Session| s.active().unwrap().doc.selection.as_ref().map(|m| m.read_region(s.active().unwrap().doc.bounds()));
            let before = mask(&actual);
            let result = play(&mut actual, AtnEvent::Code(event), rectangle([3.0, 3.0, 7.0, 7.0], ellipse));
            assert!(result.get("failed").is_none(), "{result}");
            let mut p = json!({"x":3,"y":3,"width":4,"height":4,"mode":mode});
            if ellipse {
                p["ellipse"] = json!(true);
                p["antiAlias"] = json!(true);
            }
            direct.execute("select.rect", p).unwrap();
            assert_eq!(mask(&actual), mask(&direct), "{depth}/{mode}/{ellipse}");
            assert_ne!(mask(&actual), before, "the action changes the existing mask");
            actual.execute("edit.undo", json!({})).unwrap();
            assert_eq!(mask(&actual), before);
        }
    }
}

/// Clipboard-driven paste creates the same layer and mask through imported and direct commands.
#[test]
fn paste_actions_use_the_clipboard_and_selection_and_undo() {
    for (event, id) in [(*b"PstI", "edit.pasteSpecial.pasteInto"), (*b"PstO", "edit.pasteSpecial.pasteOutside")] {
        let setup = || {
            let mut s = image(32);
            s.execute("select.all", json!({})).unwrap();
            s.execute("edit.copy", json!({})).unwrap();
            s.execute("select.rect", json!({"x":2,"y":2,"width":3,"height":3})).unwrap();
            s
        };
        let mut actual = setup();
        let mut direct = setup();
        let before = actual.active().unwrap().doc.layers.len();
        let d = desc("null").with("AntA", enm("Annt", "Anno")).with("As  ", D::Class(class("Pxel")));
        let result = play(&mut actual, AtnEvent::Code(event), d);
        assert!(result.get("failed").is_none(), "{result}");
        direct.execute(id, json!({})).unwrap();
        let state = |s: &Session| {
            let st = s.active().unwrap();
            let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
            (st.doc.layers.len(), layer.surface().unwrap().read_region(st.doc.bounds()), layer.mask.as_ref().unwrap().surface.read_region(st.doc.bounds()))
        };
        assert_eq!(actual.active().unwrap().doc.layers.len(), before + 1);
        assert_eq!(state(&actual), state(&direct));
        actual.execute("edit.undo", json!({})).unwrap();
        assert_eq!(actual.active().unwrap().doc.layers.len(), before);
    }
}

/// Zero-opacity Fade restores the preceding pixels; no previous edit fails without mutation.
#[test]
fn fade_action_consumes_the_previous_edit_and_fails_without_it() {
    for depth in [8, 16, 32] {
        let mut s = image(depth);
        let before = s.execute("document.pixel", json!({"x":3,"y":3})).unwrap();
        s.execute("image.adjustments.invert", json!({})).unwrap();
        assert_ne!(s.execute("document.pixel", json!({"x":3,"y":3})).unwrap(), before);
        let result = play(&mut s, AtnEvent::Code(*b"Fade"), desc("null").with("Opct", pct(0.0)).with("Md  ", enm("BlnM", "Nrml")));
        assert!(result.get("failed").is_none(), "{result}");
        assert_eq!(s.execute("document.pixel", json!({"x":3,"y":3})).unwrap(), before);
    }
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":8})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let result = play(&mut s, AtnEvent::Code(*b"Fade"), desc("null").with("Opct", pct(50.0)).with("Md  ", enm("BlnM", "Mltp")));
    assert_eq!(result["failed"]["id"], "edit.fade");
    assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc));
}

/// The Link event uses the specified existing toggle, including a second playback and Undo.
#[test]
fn link_action_targets_the_selected_layers() {
    let mut s = image(8);
    s.execute("layer.new.layer", json!({"name":"One"})).unwrap();
    s.execute("layer.new.layer", json!({"name":"Two"})).unwrap();
    s.execute("layer.select", json!({"layer":"One"})).unwrap();
    s.execute("layer.select", json!({"layer":"Two","mode":"add"})).unwrap();
    let linked = |s: &Session| s.active().unwrap().doc.layers.iter().filter(|l| l.link_group.is_some()).count();
    for expected in [2, 0] {
        let result = play(&mut s, AtnEvent::name("linkSelectedLayers"), desc("null").with("null", target_ref("Lyr ")));
        assert!(result.get("failed").is_none(), "{result}");
        assert_eq!(linked(&s), expected);
    }
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(linked(&s), 2);
}

/// Distinct sized/placed layers exercise each recorded distribution edge and center.
#[test]
fn distribute_actions_match_the_requested_native_direction_and_undo() {
    for (value, edge) in [
        ("AdLf", "leftEdges"),
        ("AdCH", "horizontalCenters"),
        ("AdRg", "rightEdges"),
        ("AdTp", "topEdges"),
        ("AdCV", "verticalCenters"),
        ("AdBt", "bottomEdges"),
    ] {
        let setup = || {
            let mut s = image(8);
            for (name, x, y, size) in [("One", 0, 0, 1), ("Two", 4, 3, 2), ("Three", 6, 6, 1)] {
                s.execute("layer.new.layer", json!({"name":name})).unwrap();
                s.execute("select.rect", json!({"x":x,"y":y,"width":size,"height":size})).unwrap();
                s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
                s.execute("select.deselect", json!({})).unwrap();
            }
            s.execute("layer.select", json!({"layer":"One"})).unwrap();
            for name in ["Two", "Three"] {
                s.execute("layer.select", json!({"layer":name,"mode":"add"})).unwrap();
            }
            s
        };
        let mut actual = setup();
        let mut direct = setup();
        let bounds = |s: &Session| s.active().unwrap().doc.layers.iter().map(|l| (l.name.clone(), l.surface().map(|v| v.content_bounds()))).collect::<Vec<_>>();
        let before = bounds(&actual);
        let result = play(&mut actual, AtnEvent::Code(*b"Dstr"), desc("null").with("null", target_ref("Lyr ")).with("Usng", enm("ADSt", value)));
        assert!(result.get("failed").is_none(), "{result}");
        direct.execute(&format!("layer.distribute.{edge}"), json!({})).unwrap();
        assert_eq!(bounds(&actual), bounds(&direct), "{edge}");
        actual.execute("edit.undo", json!({})).unwrap();
        assert_eq!(bounds(&actual), before);
    }
}

/// Legacy steps perform real edits and keep their native Undo behavior.
#[test]
fn old_mask_and_merge_actions_execute_their_mapped_commands() {
    let mut s = image(8);
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let has_mask = |s: &Session| {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().mask.is_some()
    };
    assert!(has_mask(&s));
    let result = play(&mut s, AtnEvent::Code(*b"RmvL"), desc("null"));
    assert!(result.get("failed").is_none(), "{result}");
    assert!(!has_mask(&s));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(has_mask(&s));
    s.execute("layer.new.layer", json!({"name":"Overlay"})).unwrap();
    s.execute("edit.fill", json!({"color":"#ff0000"})).unwrap();
    s.execute("select.allLayers", json!({})).unwrap();
    let before = s.active().unwrap().doc.layers.len();
    let result = play(&mut s, AtnEvent::Code(*b"MrgL"), desc("null"));
    assert!(result.get("failed").is_none(), "{result}");
    assert_eq!(s.active().unwrap().doc.layers.len(), 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), before);
}

/// The empty Define Pattern event creates the selected region without changing canvas pixels.
#[test]
fn define_pattern_action_uses_the_selected_region() {
    let mut s = image(8);
    s.execute("select.rect", json!({"x":1,"y":2,"width":3,"height":2})).unwrap();
    let before = s.execute("document.pixel", json!({"x":3,"y":3})).unwrap();
    let count = s.patterns.items.len();
    let result = play(&mut s, AtnEvent::Code(*b"DfnP"), desc("null"));
    assert!(result.get("failed").is_none(), "{result}");
    assert_eq!(s.patterns.items.len(), count + 1);
    let p = s.patterns.items.last().unwrap();
    assert_eq!((p.width, p.height), (3, 2));
    assert!(!p.surface.format().alpha);
    assert_eq!(s.execute("document.pixel", json!({"x":3,"y":3})).unwrap(), before);
}
