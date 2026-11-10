use super::*;
use photocraft_psd::atn::{AtnAction, AtnEvent, AtnFile, AtnItem};
use photocraft_psd::descriptor::{Descriptor, Id, ReferenceItem, UnicodeString, Value as D};
use serde_json::json;

/// A wire key has one mapper; distinct wire aliases may still name the same command.
#[test]
fn event_wire_keys_are_unique() {
    let mut codes = std::collections::BTreeSet::new();
    let mut names = std::collections::BTreeSet::new();
    for event in EVENTS {
        assert!(event.code.is_empty() || codes.insert(event.code), "duplicate ATN code {}", event.code);
        assert!(event.name.is_empty() || names.insert(event.name), "duplicate ATN name {}", event.name);
    }
}

#[test]
fn blend_modes_accept_photoshop_string_ids_and_export_canonical_ids() {
    for (string_id, char_id, label) in [
        ("normal", "Nrml", "Normal"),
        ("dissolve", "Dslv", "Dissolve"),
        ("darken", "Drkn", "Darken"),
        ("multiply", "Mltp", "Multiply"),
        ("colorBurn", "CBrn", "Color Burn"),
        ("lighten", "Lghn", "Lighten"),
        ("screen", "Scrn", "Screen"),
        ("colorDodge", "CDdg", "Color Dodge"),
        ("overlay", "Ovrl", "Overlay"),
        ("softLight", "SftL", "Soft Light"),
        ("hardLight", "HrdL", "Hard Light"),
        ("difference", "Dfrn", "Difference"),
        ("exclusion", "Xclu", "Exclusion"),
        ("hue", "H", "Hue"),
        ("saturation", "Strt", "Saturation"),
        ("color", "Clr", "Color"),
        ("luminosity", "Lmns", "Luminosity"),
        ("passThrough", "passThrough", "Pass Through"),
    ] {
        assert_eq!(blend_label(string_id), Some(label), "{string_id}");
        assert_eq!(blend_ps(label), Some(char_id), "{label} remains serialized canonically");
    }
}

/// #8454: a Set and a Make layer step whose blend mode is stored as a Photoshop stringID import
/// with that mode (a Set was unsupported, a Make silently fell back to Normal).
#[test]
fn layer_steps_with_string_id_blend_modes_import_with_their_mode() {
    let set = desc("null").with("null", target_ref("Lyr ")).with("T   ", D::Descriptor(desc("Lyr ").with("Md  ", enm("BlnM", "softLight"))));
    let make = desc("null").with("null", class_ref("Lyr ")).with("Usng", D::Descriptor(desc("Lyr ").with("Md  ", enm("BlnM", "luminosity"))));
    let file = AtnFile {
        name: UnicodeString::new_nul("Blend"),
        expanded: false,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("A"),
            items: vec![item("setd", "Set", Some(set)), item("Mk  ", "Make", Some(make))],
            ..Default::default()
        }],
        warnings: vec![],
        ..Default::default()
    };
    let imp = import(&atn::write(&file).unwrap()).unwrap();
    assert!(imp.unsupported.is_empty(), "{:?}", imp.unsupported.iter().map(|u| &u.event).collect::<Vec<_>>());
    let steps = &imp.actions[0].steps;
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].0, "layer.setProps");
    assert_eq!(steps[0].1["blend"], json!("Soft Light"));
    assert_eq!(steps[1].0, "layer.new.layer");
    assert_eq!(steps[1].1["mode"], json!("Luminosity"));
}

fn item(event: &str, label: &str, d: Option<Descriptor>) -> AtnItem {
    let event = match event.as_bytes() {
        [a, b, c, dd] => AtnEvent::Code([*a, *b, *c, *dd]),
        _ => AtnEvent::name(event),
    };
    AtnItem { event, dictionary: label.as_bytes().to_vec(), descriptor: d, ..Default::default() }
}

fn b64(bytes: &[u8]) -> String {
    photocraft_paint::tile::b64_encode(bytes)
}

fn color_lookup_case() -> Value {
    let bytes = b"LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
    let d = desc("null")
        .with("lookupType", enm("colorLookupType", "3DLUT"))
        .with("Nm  ", text("identity.cube"))
        .with("Dthr", D::Boolean(false))
        .with("profile", D::RawData(Vec::new()))
        .with("LUTFormat", enm("LUTFormatType", "LUTFormatCUBE"))
        .with("LUT3DFileData", D::RawData(Vec::new()))
        .with("LUT3DFileName", text("identity.cube"));
    json!({"data":b64(bytes),"dataEncoding":"base64","lutFormat":"cube","fileName":"identity.cube","dither":false,"__atnColorLookup":b64(&d.to_bytes())})
}

/// Keep the starting counter nested in naming rules, as recorded by the Batch dialog.
fn batch_descriptor(component: &str) -> Descriptor {
    let rules = desc("fileNamingRules")
        .with("fileNamingComponents", D::List(vec![D::Descriptor(desc("fileNamingComponent").with("component", enm("fileNamingComponent", component)))]))
        .with("Strt", D::Integer(7))
        .with("Mcnt", D::Boolean(true))
        .with("Win ", D::Boolean(false))
        .with("unix", D::Boolean(true));
    desc("Btch")
        .with("null", path_value("/input"))
        .with("Fldr", D::Boolean(false))
        .with(
            "Usng",
            D::Reference(vec![
                ReferenceItem::Name { class: class("Actn"), name: UnicodeString::new_nul("Child") },
                ReferenceItem::Name { class: class("ASet"), name: UnicodeString::new_nul("S") },
            ]),
        )
        .with("T   ", path_value("/output"))
        .with("Ovrd", D::Boolean(false))
        .with("fileNamingRules", D::Descriptor(rules))
}

/// Preserve the recorded template in the generic registry round-trip witness.
fn batch_case() -> Value {
    json!({"input":"/input","output":"/output","action":"Child","set":"S","flatOutput":true,"includeSubfolders":false,"overrideSaveAs":false,"stopOnError":true,"includeNonImages":true,
        "namingComponents":[{"component":"serial3"}],"startingSerial":7,"compatibility":{"macintosh":true,"windows":false,"unix":true},
        "__atnBatch":b64(&batch_descriptor("threeDigit").to_bytes())})
}

/// Save and Close replaces the folder path while retaining its recorded naming metadata.
fn batch_save_and_close_case() -> Value {
    let mut d = batch_descriptor("threeDigit");
    d.items.retain(|(key, _)| !key.is("T   "));
    let d = d.with("SvAn", D::Boolean(true));
    let mut p = batch_case();
    p.as_object_mut().unwrap().remove("output");
    p.as_object_mut().unwrap().remove("flatOutput");
    p["destination"] = json!("saveAndClose");
    p["__atnBatch"] = json!(b64(&d.to_bytes()));
    p
}

/// Destination changes retain exact descriptors and reject ambiguous output/override settings.
#[test]
fn batch_save_and_close_round_trips_without_folder_defaults() {
    let p = batch_save_and_close_case();
    let d = super::steps_batch::batch_to("file.automate.batch", &p).unwrap();
    let bytes = photocraft_paint::tile::b64_decode(&set_of("S", vec![("Driver", vec![item("Btch", "Batch", Some(d.clone()))])])).unwrap();
    let imported = import(&bytes).unwrap();
    assert!(imported.unsupported.is_empty());
    assert_eq!(imported.actions[0].steps[0].1, p);
    let (out, skipped) = export("S", &[&imported.actions[0]], &[], false).unwrap();
    assert!(skipped.is_empty());
    assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor, Some(d.clone()));
    for bad in [d.clone().with("T   ", path_value("/other")), {
        let mut bad = d.clone();
        bad.items.retain(|(key, _)| !key.is("Ovrd"));
        bad.with("Ovrd", D::Boolean(true))
    }] {
        let bytes = photocraft_paint::tile::b64_decode(&set_of("S", vec![("Driver", vec![item("Btch", "Batch", Some(bad.clone()))])])).unwrap();
        let imported = import(&bytes).unwrap();
        assert_eq!(imported.actions[0].steps[0].0, PHOTOSHOP_STEP);
        let (out, _) = export("S", &[&imported.actions[0]], &[], false).unwrap();
        assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor, Some(bad));
    }
}

/// Each documented component and portability flag must survive native conversion and save.
#[test]
fn batch_all_documented_components_and_compatibility_flags_round_trip() {
    for &(wire, native) in crate::file_cmds::BATCH_NAMING_COMPONENTS {
        let d = batch_descriptor(wire);
        let bytes = photocraft_paint::tile::b64_decode(&set_of("S", vec![("Driver", vec![item("Btch", "Batch", Some(d.clone()))])])).unwrap();
        let imported = import(&bytes).unwrap();
        assert!(imported.unsupported.is_empty(), "{wire}");
        let p = &imported.actions[0].steps[0].1;
        assert_eq!(p["namingComponents"], json!([{"component":native}]));
        assert_eq!(p["startingSerial"], 7);
        assert_eq!(p["stopOnError"], true);
        assert_eq!(p["includeNonImages"], true);
        assert_eq!(p["compatibility"], json!({"macintosh":true,"windows":false,"unix":true}));
        let (out, skipped) = export("S", &[&imported.actions[0]], &[], false).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor, Some(d));
    }
}

/// Batch's Open binding round-trips, but richer Open descriptors must remain whole.
#[test]
fn batch_override_open_and_unrepresented_open_settings_round_trip() {
    for enabled in [false, true] {
        let mut p = batch_case();
        p.as_object_mut().unwrap().remove("__atnBatch");
        p["overrideOpen"] = json!(enabled);
        p["suppressWarnings"] = json!(enabled);
        p["suppressOpenOptions"] = json!(enabled);
        let a = Action::new("Driver", vec![("file.automate.batch".into(), p)]);
        let (bytes, skipped) = export("S", &[&a], &[], false).unwrap();
        assert!(skipped.is_empty());
        let loaded = import(&bytes).unwrap();
        assert_eq!(loaded.actions[0].steps[0].1["overrideOpen"], enabled);
        assert_eq!(loaded.actions[0].steps[0].1["suppressWarnings"], enabled);
        assert_eq!(loaded.actions[0].steps[0].1["suppressOpenOptions"], enabled);
        let (again, skipped) = export("S", &[&loaded.actions[0]], &[], false).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(atn::parse(&again).unwrap().actions[0].items[0].descriptor, atn::parse(&bytes).unwrap().actions[0].items[0].descriptor);
    }
    let rich = desc("null").with("null", path_value("/recorded.png")).with("unrepresentedOpenSetting", D::Boolean(true));
    let bytes = photocraft_paint::tile::b64_decode(&set_of("S", vec![("Driver", vec![item("Opn ", "Open", Some(rich.clone()))])])).unwrap();
    let loaded = import(&bytes).unwrap();
    assert_eq!(loaded.actions[0].steps[0].0, PHOTOSHOP_STEP);
    let (again, _) = export("S", &[&loaded.actions[0]], &[], false).unwrap();
    assert_eq!(atn::parse(&again).unwrap().actions[0].items[0].descriptor, Some(rich));
    assert!(super::steps_doc::open_to("file.openAs", &json!({"path":"/recorded.png","as":"png"})).is_none());
}

/// Unsupported modes and misplaced fields must not lose data through a partial conversion.
#[test]
fn batch_unknown_components_wrong_start_placement_and_unsupported_modes_stay_whole() {
    let mut cases = vec![batch_descriptor("unknownComponent")];
    for (key, value) in [("Strt", D::Integer(3)), ("OvrO", D::Integer(1)), ("SvAn", D::Boolean(true)), ("Log ", path_value("/log"))] {
        cases.push(batch_descriptor("Nm  ").with(key, value));
    }
    let mut odd = batch_descriptor("Nm  ");
    odd.items.push((Id::new("Fldr"), D::Boolean(true))); // Duplicate keys cannot disappear on save.
    cases.push(odd);
    for d in cases {
        let bytes = photocraft_paint::tile::b64_decode(&set_of("S", vec![("Driver", vec![item("Btch", "Batch", Some(d.clone()))])])).unwrap();
        let imported = import(&bytes).unwrap();
        assert_eq!(imported.actions[0].steps[0].0, PHOTOSHOP_STEP);
        let (out, _) = export("S", &[&imported.actions[0]], &[], false).unwrap();
        assert_eq!(atn::parse(&out).unwrap().actions[0].items[0].descriptor, Some(d));
    }
    let mut p = batch_case();
    p["target"] = json!("mask");
    assert!(super::steps_batch::batch_to("file.automate.batch", &p).is_none());
    for key in ["stopOnError", "includeNonImages"] {
        for value in [Value::Null, json!(false), json!("yes")] {
            let mut p = batch_case();
            p[key] = value;
            assert!(super::steps_batch::batch_to("file.automate.batch", &p).is_none());
        }
        let mut p = batch_case();
        p.as_object_mut().unwrap().remove(key);
        assert!(super::steps_batch::batch_to("file.automate.batch", &p).is_none());
    }
}

/// A set with a mapped filter, an adjustment layer, selection steps, a layer by name, a Stop,
/// a Play, a tool selection (no document effect) and an event PhotoCraft has no command for.
fn sample() -> AtnFile {
    let levels = desc("Lvls").with(
        "Adjs",
        D::List(vec![D::Descriptor(
            desc("LvlA")
                .with("Chnl", channel_ref(None))
                .with("Inpt", D::List(vec![long(10.0), long(240.0)]))
                .with("Gmm ", D::Double(1.2))
                .with("Otpt", D::List(vec![long(0.0), long(250.0)])),
        )]),
    );
    let make_levels = desc("null").with("null", class_ref("AdjL")).with("Usng", D::Descriptor(desc("AdjL").with("Type", D::Descriptor(levels))));
    let select_all =
        desc("null").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }])).with("T   ", enm("Ordn", "Al  "));
    let select_layer = desc("null").with("null", name_ref("Lyr ", "Shadow")).with("MkVs", D::Boolean(false));
    let tool = desc("null").with("null", class_ref("MvTl"));
    let play = desc("null").with("null", D::Reference(vec![ReferenceItem::Name { class: class("Actn"), name: UnicodeString::new_nul("Finish") }]));
    let stop = desc("null").with("Msge", text("Check the result")).with("Cntn", D::Boolean(true));
    let fancy = desc("null").with("prompt", text("a cat")).with("DocI", long(5.0));
    let mut blur = item("GsnB", "Gaussian Blur", Some(desc("null").with("Rds ", px(2.5))));
    blur.with_dialog = true;
    let mut off = item("Fltn", "Flatten Image", None);
    off.enabled = false;
    AtnFile {
        name: UnicodeString::new_nul("Test Set"),
        expanded: true,
        actions: vec![
            AtnAction {
                name: UnicodeString::new_nul("Soften"),
                function_key: 3,
                items: vec![
                    blur,
                    item("Mk  ", "Make", Some(make_levels)),
                    item("setd", "Set", Some(select_all)),
                    item("select", "Select", Some(select_layer)),
                    item("slct", "Select", Some(tool)),
                    item("Stop", "Stop", Some(stop)),
                    item("syntheticFill", "Generative Fill", Some(fancy)),
                    item("Ply ", "Play", Some(play)),
                    off,
                ],
                ..Default::default()
            },
            AtnAction { name: UnicodeString::new_nul("Finish"), items: vec![item("Invs", "Inverse", None)], ..Default::default() },
        ],
        warnings: Vec::new(),
        ..Default::default()
    }
}

#[test]
fn layer_comp_apply_imports_plays_and_undoes() {
    let name = "合成 𠮷";
    let data = set_of("Comps", vec![("Apply", vec![item("applyComp", "Apply Layer Comp", Some(desc("null").with("null", name_ref("compsClass", name))))])]);
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        let loaded = s.execute("actions.load", json!({"data": data})).unwrap();
        assert_eq!(loaded["unsupported"], json!([]), "{loaded}");
        assert_eq!(s.actions.list[0].steps, [("layerComp.apply".into(), json!({"comp": name}))]);

        s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth})).unwrap();
        s.execute("layerComp.new", json!({"name": "Other"})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let played = s.execute("actions.play", json!({"action": "Apply"})).unwrap();
        assert_eq!(played["ran"], 0, "{played}");
        assert!(played["failed"]["error"].as_str().unwrap().contains(name), "{played}");
        assert_eq!(*s.active().unwrap().doc, *before, "a missing comp must not apply another comp");

        let comp = s.execute("layerComp.new", json!({"name": name})).unwrap()["comp"].clone();
        let layer = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.setProps", json!({"visible": false})).unwrap();
        s.execute("layerComp.new", json!({"name": "Hidden"})).unwrap();
        let played = s.execute("actions.play", json!({"action": "Apply"})).unwrap();
        assert_eq!(played["ran"], 1, "{played}");
        assert!(played.get("failed").is_none(), "{played}");
        assert!(s.active().unwrap().doc.layer(layer).unwrap().visible);
        assert_eq!(json!(s.active().unwrap().doc.last_applied_comp), comp);
        assert!(s.undo());
        assert!(!s.active().unwrap().doc.layer(layer).unwrap().visible);
        assert!(s.redo());
        assert!(s.active().unwrap().doc.layer(layer).unwrap().visible);
    }
}

#[test]
fn layer_comp_apply_round_trips_by_name() {
    for name in ["Comp 1", "合成 𠮷"] {
        let descriptor = desc("null").with("null", name_ref("compsClass", name));
        let data = set_of("Comps", vec![("Apply", vec![item("applyComp", "Apply Layer Comp", Some(descriptor.clone()))])]);
        let mut s = Session::new();
        let loaded = s.execute("actions.load", json!({"data": data})).unwrap();
        assert_eq!(loaded["unsupported"], json!([]), "{loaded}");
        let saved = s.execute("actions.save", json!({"set": "Comps"})).unwrap();
        assert_eq!(saved["skipped"], json!([]));
        let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
        let file = atn::parse(&bytes).unwrap();
        assert_eq!(file.actions[0].items[0].event, AtnEvent::Name("applyComp".into()));
        assert_eq!(file.actions[0].items[0].descriptor, Some(descriptor));
        let mut reloaded = Session::new();
        reloaded.execute("actions.load", json!({"data": saved["data"]})).unwrap();
        assert_eq!(reloaded.actions.list, s.actions.list);
    }
    // Local comp ids and the default comp cannot be resolved to a portable name at export.
    for params in [json!({}), json!({"comp": 1}), json!({"comp": null}), json!({"comp": true}), json!({"comp": []})] {
        assert!(export_item("layerComp.apply", &params, "Comps").is_none(), "{params}");
    }
}

#[test]
fn layer_comp_apply_unknown_references_are_kept_and_stop_playback() {
    let references = [
        None,
        Some(D::Integer(1)),
        Some(D::Reference(vec![])),
        Some(name_ref("Lyr ", "Comp 1")),
        Some(class_ref("compsClass")),
        Some(D::Reference(vec![ReferenceItem::Offset { class: class("compsClass"), offset: 1 }])),
        Some(D::Reference(vec![ReferenceItem::Enumerated { class: class("compsClass"), type_id: code("Ordn"), value: code("unsupported") }])),
        Some(D::Reference(vec![
            ReferenceItem::Name { class: class("compsClass"), name: UnicodeString::new_nul("Comp 1") },
            ReferenceItem::Name { class: class("Dcmn"), name: UnicodeString::new_nul("Other") },
        ])),
    ];
    let items: Vec<_> =
        references.into_iter().map(|reference| item("applyComp", "Apply Layer Comp", reference.map(|r| desc("null").with("null", r)))).collect();
    let data = set_of("Comps", vec![("Unknown", items.clone())]);
    let mut s = Session::new();
    let loaded = s.execute("actions.load", json!({"data": data})).unwrap();
    assert_eq!(loaded["unsupported"].as_array().unwrap().len(), items.len());
    assert!(s.actions.list[0].steps.iter().all(|(id, _)| id == PHOTOSHOP_STEP));
    assert!(s.actions.list[0].disabled.is_empty());
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layerComp.new", json!({"name": "Comp 1"})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let played = s.execute("actions.play", json!({"action": "Unknown"})).unwrap();
    assert_eq!(played["ran"], 0, "{played}");
    assert!(played["failed"]["error"].as_str().unwrap().contains("Apply Layer Comp"), "{played}");
    assert_eq!(*s.active().unwrap().doc, *before);
    let saved = s.execute("actions.save", json!({"set": "Comps"})).unwrap();
    let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
    let file = atn::parse(&bytes).unwrap();
    assert_eq!(file.actions[0].items.len(), items.len());
    for (saved, original) in file.actions[0].items.iter().zip(&items) {
        assert_eq!(saved.event, original.event);
        assert_eq!(saved.descriptor, original.descriptor);
    }
}

#[test]
fn load_maps_events_keeps_flags_and_plays() {
    let mut s = Session::new();
    let bytes = atn::write(&sample()).unwrap();
    let r = s.execute("actions.load", json!({"data": b64(&bytes)})).unwrap();
    assert_eq!(r["set"], "Test Set");
    assert_eq!(r["actions"], json!(["Soften", "Finish"]));
    assert_eq!(r["steps"], 10);
    assert_eq!(r["functionKeys"], json!([{"action": "Soften", "key": "F3"}]));
    assert_eq!(s.prefs().shortcuts.get("actions.play:Soften").map(String::as_str), Some("F3"));
    let unsupported: Vec<(String, String)> =
        r["unsupported"].as_array().unwrap().iter().map(|u| (u["event"].as_str().unwrap().into(), u["name"].as_str().unwrap().into())).collect();
    assert_eq!(unsupported, [("slct".to_string(), "Select".to_string()), ("syntheticFill".to_string(), "Generative Fill".to_string())]);

    let a = &s.actions.list[0];
    assert_eq!(a.set.as_deref(), Some("Test Set"));
    let ids: Vec<&str> = a.steps.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "filter.blur.gaussianBlur",
            "layer.newAdjustmentLayer.levels",
            "select.all",
            "layer.select",
            PHOTOSHOP_STEP,
            STOP_MESSAGE,
            PHOTOSHOP_STEP,
            "actions.play",
            "layer.flattenImage"
        ]
    );
    assert_eq!(a.steps[0].1, json!({"radius": 2.5}));
    assert_eq!(a.steps[1].1, json!({"inBlack": 10, "inWhite": 240, "gamma": 1.2, "outBlack": 0, "outWhite": 250}));
    assert_eq!(a.steps[3].1, json!({"layer": "Shadow"}));
    assert_eq!(a.steps[5].1, json!({"message": "Check the result", "allowContinue": true}));
    assert_eq!(a.steps[6].1["event"], "syntheticFill");
    assert_eq!(a.steps[7].1, json!({"action": "Finish"}));
    assert_eq!(a.dialogs.iter().copied().collect::<Vec<_>>(), [0], "the dialog toggle is kept");
    assert_eq!(a.disabled.iter().copied().collect::<Vec<_>>(), [4, 8], "tool selections are switched off, as is the step the file had off");
    assert_eq!(s.actions.list[1].steps, [("select.inverse".to_string(), json!({}))]);

    // Playback: the mapped steps run, the layer is found by name, and the kept Photoshop step
    // stops the action with its name.
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Shadow"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    let played = s.execute("actions.play", json!({"action": "Soften"})).unwrap();
    assert_eq!(played["ran"], 5, "{played}");
    assert_eq!(played["failed"]["step"], 6);
    assert!(played["failed"]["error"].as_str().unwrap().contains("Generative Fill"), "{played}");
    let d = s.active().unwrap();
    assert_eq!(d.doc.layer(d.active_layer.unwrap()).unwrap().name, "Shadow");
    assert!(d.doc.walk().iter().any(|(_, _, l)| l.name.starts_with("Levels")));

    // Loading the same set again replaces it; a different set with the same action names
    // gets unique names and its Play steps follow.
    s.execute("actions.load", json!({"data": b64(&bytes)})).unwrap();
    assert_eq!(s.actions.list.len(), 2);
    let r = s.execute("actions.load", json!({"data": b64(&bytes), "set": "Other"})).unwrap();
    assert_eq!(r["actions"], json!(["Soften 2", "Finish 2"]));
    assert_eq!(s.actions.list[2].steps[7].1, json!({"action": "Finish 2"}));
    assert_eq!(s.actions.list[2].set.as_deref(), Some("Other"));
}

/// Renames that chain ("A" becomes "A 2" while the file's own "A 2" becomes "A 2 2") rewrite
/// each Play and Conditional target once, to the renamed copy of the action it named.
#[test]
fn load_renames_each_play_target_once() {
    let mut s = Session::new();
    let mut mine = Action::new("A", vec![]);
    mine.set = Some("Mine".into());
    s.actions.list.push(mine);
    let a = Action::new("A", vec![("actions.conditional".into(), json!({"if": "always", "then": "A", "else": "A 2"}))]);
    let a2 = Action::new("A 2", vec![("actions.play".into(), json!({"action": "A"}))]);
    let keys = [("A".to_string(), "F4".to_string())];
    let (bytes, _) = export("File", &[&a, &a2], &keys, false).unwrap();
    let r = s.execute("actions.load", json!({"data": b64(&bytes)})).unwrap();
    assert_eq!(r["actions"], json!(["A 2", "A 2 2"]));
    let get = |n: &str| s.actions.list.iter().find(|a| a.name == n && a.set.as_deref() == Some("File")).unwrap();
    assert_eq!(get("A 2 2").steps[0].1["action"], "A 2", "plays the file's A, not its A 2");
    let cond = &get("A 2").steps[0].1;
    assert_eq!((&cond["then"], &cond["else"]), (&json!("A 2"), &json!("A 2 2")), "{cond}");
    assert_eq!(r["functionKeys"], json!([{"action": "A 2", "key": "F4"}]));

    // Adding to a set of the same name leaves the Play steps already in it alone.
    let mut s = Session::new();
    for (name, steps) in [("A", vec![]), ("B", vec![("actions.play".to_string(), json!({"action": "A"}))])] {
        let mut a = Action::new(name, steps);
        a.set = Some("File".into());
        s.actions.list.push(a);
    }
    let (bytes, _) = export("File", &[&Action::new("A", vec![])], &[], false).unwrap();
    let r = s.execute("actions.load", json!({"data": b64(&bytes), "replace": false})).unwrap();
    assert_eq!(r["actions"], json!(["A 2"]));
    assert_eq!(s.actions.list[1].steps[0].1, json!({"action": "A"}), "B still plays the A it had");
}

/// A name over the 1,024-character limit is clipped for the action and for the steps that call it,
/// so a rename still finds its Play target.
#[test]
fn load_renames_play_targets_longer_than_the_name_limit() {
    let mut s = Session::new();
    let long = "x".repeat(2000);
    let clipped = "x".repeat(1024);
    let mut mine = Action::new(clipped.clone(), vec![]);
    mine.set = Some("Mine".into());
    s.actions.list.push(mine);
    let a = Action::new(long.clone(), vec![]);
    let caller = Action::new("Caller", vec![("actions.play".into(), json!({"action": long}))]);
    let (bytes, _) = export("File", &[&a, &caller], &[], false).unwrap();
    s.execute("actions.load", json!({"data": b64(&bytes)})).unwrap();
    let renamed = format!("{clipped} 2");
    let caller = s.actions.list.iter().find(|a| a.name == "Caller").unwrap();
    assert_eq!(caller.steps[0].1["action"], renamed.as_str());
}

#[test]
fn load_many_duplicate_names_get_sequential_suffixes() {
    // Several actions share one base name. Deduplication must stay O(n) overall and produce
    // the sequence A, A 2, A 3, A 4 (the next-suffix cache cannot skip or repeat a number).
    let mut s = Session::new();
    let dupes: Vec<Action> = (0..4).map(|_| Action::new("A", vec![])).collect();
    let refs: Vec<&Action> = dupes.iter().collect();
    let (bytes, _) = export("File", &refs, &[], false).unwrap();
    let r = s.execute("actions.load", json!({"data": b64(&bytes)})).unwrap();
    assert_eq!(r["actions"], json!(["A", "A 2", "A 3", "A 4"]));
}

#[test]
fn save_round_trips_through_photoshop_events() {
    let mut s = Session::new();
    let bytes = atn::write(&sample()).unwrap();
    s.execute("actions.load", json!({"data": b64(&bytes)})).unwrap();
    let first = s.actions.list.clone();
    let r = s.execute("actions.save", json!({"set": "Test Set"})).unwrap();
    assert_eq!(r["actions"], json!(["Soften", "Finish"]));
    assert_eq!(r["skipped"], json!([]));
    let saved = photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap();
    let file = atn::parse(&saved).unwrap();
    assert_eq!(file.name.to_string_lossy(), "Test Set");
    assert_eq!(file.actions[0].function_key, 3);
    let events: Vec<String> = file.actions[0].items.iter().map(|i| i.event.id()).collect();
    assert_eq!(events, ["GsnB", "Mk  ", "setd", "slct", "slct", "Stop", "syntheticFill", "Ply ", "FltI"]);
    assert!(file.actions[0].items[0].with_dialog);
    assert!(!file.actions[0].items[8].enabled);
    // The kept Photoshop step is written back byte for byte.
    assert_eq!(file.actions[0].items[6].descriptor, sample().actions[0].items[6].descriptor);
    // Play names the set too, as Photoshop does.
    assert_eq!(get_ref(file.actions[0].items[7].descriptor.as_ref().unwrap(), "null").unwrap().len(), 2);

    // Loading what was saved gives the same actions.
    let mut t = Session::new();
    t.execute("actions.load", json!({"data": b64(&saved)})).unwrap();
    assert_eq!(t.actions.list, first);

    // A step with no Photoshop equivalent fails unless skipped.
    s.actions.list.push(Action::new("Ours", vec![("unsupported.forTest".into(), json!({}))]));
    let e = s.execute("actions.save", json!({"action": "Ours"})).unwrap_err().to_string();
    assert!(e.contains("unsupported.forTest"), "{e}");
    let r = s.execute("actions.save", json!({"action": "Ours", "skipUnsupported": true})).unwrap();
    assert_eq!(r["skipped"], json!([{"action": "Ours", "step": 0, "id": "unsupported.forTest"}]));
    assert_eq!(r["set"], "Ours");
}

/// `image.adjustments.selectiveColor` takes a range as an array (`"reds": [c, m, y, k]`) or as
/// the scalar form that names one range (`"colors": "reds", "cyan": 35`). Only the arrays were
/// written, so a step recorded the other way exported as an adjustment that changes nothing, with
/// `skipped` empty (#7672).
#[test]
fn a_selective_color_step_in_the_scalar_form_keeps_its_values() {
    let back = |params: Value| -> Value {
        let mut s = Session::new();
        s.actions.list.push(Action::new("A", vec![("image.adjustments.selectiveColor".into(), params)]));
        let r = s.execute("actions.save", json!({"action": "A"})).unwrap();
        assert_eq!(r["skipped"], json!([]));
        let imported = import(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap();
        imported.actions[0].steps[0].1.clone()
    };

    // The numbers are read back as numbers; a percentage may arrive as 35 or 35.0.
    let range =
        |step: &Value, name: &str| -> Vec<f64> { step[name].as_array().unwrap_or(&Vec::new()).iter().map(|v| v.as_f64().unwrap_or(f64::NAN)).collect() };

    // The reported step: one range, one channel.
    let scalar = back(json!({"method": "absolute", "colors": "reds", "cyan": 35}));
    assert_eq!(scalar["method"], "absolute");
    assert_eq!(range(&scalar, "reds"), [35.0, 0.0, 0.0, 0.0], "{scalar}");

    // The scalar form over an array keeps both, as playing the step would.
    let both = back(json!({"reds": [10, 20, 30, 40], "colors": "reds", "cyan": 35}));
    assert_eq!(range(&both, "reds"), [35.0, 20.0, 30.0, 40.0], "{both}");

    // The array form is unchanged.
    let array = back(json!({"method": "relative", "yellows": [0, 0, -15, 5]}));
    assert_eq!(range(&array, "yellows"), [0.0, 0.0, -15.0, 5.0], "{array}");

    // A channel the command would refuse is not quietly dropped: the step is refused too, since
    // writing it would export an adjustment that is not the one recorded. `null` is absent, as
    // the command reads it.
    let mut s = Session::new();
    s.actions.list.push(Action::new("A", vec![("image.adjustments.selectiveColor".into(), json!({"colors": "reds", "cyan": "35"}))]));
    let e = s.execute("actions.save", json!({"action": "A"})).unwrap_err().to_string();
    assert!(e.contains("image.adjustments.selectiveColor"), "{e}");

    let nulled = back(json!({"colors": "reds", "cyan": 35, "magenta": null}));
    assert_eq!(range(&nulled, "reds"), [35.0, 0.0, 0.0, 0.0], "{nulled}");
}

/// A setting the exported event has no field for is dropped by export, and the step then plays back
/// as a different edit: Crystallize's `seed` (#7668), Radial Blur's centre, Minimum's and
/// Maximum's `preserve`, Equalize's `selectedAreaOnly`. Such a step is refused, as one with no
/// equivalent event already is. So is an adjustment edit that names its layer: the event edits the
/// active one (#8532).
#[test]
fn a_setting_the_event_cannot_hold_is_refused() {
    for (id, params) in [
        ("filter.pixelate.crystallize", json!({"cellSize": 5, "seed": 37})),
        ("filter.blur.radialBlur", json!({"amount": 10, "centerX": 0.25, "centerY": 0.5})),
        ("filter.other.minimum", json!({"radius": 4, "preserve": "roundness"})),
        ("filter.other.maximum", json!({"radius": 4, "preserve": "roundness"})),
        ("image.adjustments.equalize", json!({"selectedAreaOnly": false})),
        // Equalize reads `selectedAreaOnly` as a boolean only and refuses the string and null.
        ("image.adjustments.equalize", json!({"selectedAreaOnly": "true"})),
        ("image.adjustments.equalize", json!({"selectedAreaOnly": null})),
        ("layer.setAdjustment", json!({"kind": "exposure", "layer": "Target_existing", "exposure": -0.75, "offset": 0.035, "gamma": 1.37})),
        ("layer.setAdjustment", json!({"kind": "exposure", "document": 2, "exposure": -0.75})),
    ] {
        let mut s = Session::new();
        s.actions.list.push(Action::new("A", vec![(id.into(), params)]));
        let before = s.actions.list[0].steps.clone();
        let e = s.execute("actions.save", json!({"action": "A"})).unwrap_err().to_string();
        assert!(e.contains(id), "{id}: {e}");
        // A refused save leaves the action as it was: the step is still there to be played.
        assert_eq!(s.actions.list[0].steps, before, "{id}");
        let r = s.execute("actions.save", json!({"action": "A", "skipUnsupported": true})).unwrap();
        assert_eq!(r["skipped"], json!([{"action": "A", "step": 0, "id": id}]), "{id}");
        assert_eq!(s.actions.list[0].steps, before, "{id}: skipping a step does not drop it");
    }
}

/// The same steps at the value the event plays as, and with the setting absent: written as before.
#[test]
fn settings_at_the_value_the_event_plays_still_export() {
    for (id, params) in [
        ("filter.pixelate.crystallize", json!({"cellSize": 5, "seed": 0})),
        ("filter.pixelate.crystallize", json!({"cellSize": 5})),
        ("filter.blur.radialBlur", json!({"amount": 10, "centerX": 0.5, "centerY": 0.5})),
        ("filter.blur.radialBlur", json!({"amount": 10})),
        ("filter.other.minimum", json!({"radius": 4, "preserve": "squareness"})),
        ("filter.other.maximum", json!({"radius": 4})),
        ("layer.setAdjustment", json!({"kind": "exposure", "exposure": -0.75, "offset": 0.035, "gamma": 1.37})),
        ("layer.setAdjustment", json!({"kind": "exposure", "layer": null, "exposure": -0.75})),
        ("layer.setAdjustment", json!({"kind": "exposure", "document": null, "exposure": -0.75})),
        // `preserve` reaches the filter through a comparison with the exact string "roundness",
        // so these play as squareness -- replay's own value -- and lose nothing by export (#7759
        // review).
        ("filter.other.minimum", json!({"radius": 4, "preserve": "Squareness"})),
        ("filter.other.maximum", json!({"radius": 4, "preserve": "Roundness"})),
        ("image.adjustments.equalize", json!({"selectedAreaOnly": true})),
        ("image.adjustments.equalize", json!({})),
    ] {
        let mut s = Session::new();
        s.actions.list.push(Action::new("A", vec![(id.into(), params.clone())]));
        let r = s.execute("actions.save", json!({"action": "A"})).unwrap_or_else(|e| panic!("{id} {params}: {e}"));
        assert_eq!(r["skipped"], json!([]), "{id} {params}");
    }
}

/// Every exported command has a canonical import, including recorded full-stack navigation.
#[test]
/// Each mapped command retains its parameters through its corresponding ATN descriptor.
fn every_mapped_command_exports_and_imports_back() {
    let cases: Vec<(&str, Value)> = vec![
        (
            "edit.perspectiveWarp",
            json!({"planes":[{"src":[[4,4],[16,4],[16,20],[4,20]],"dst":[[5,5],[17,4],[18,20],[4,19]]}],
                "target":"pixels","__atnPerspectiveWarpRect":[0,0,32,24]}),
        ),
        (
            "paint.gradient",
            json!({"from":[1,1],"to":[6,2],"style":"linear","method":"classic","smoothness":100,
            "stops":[[0,"#0020ff"],[1,"#ffe000"]],"transparency":[],"opacity":100,"mode":"Normal","reverse":false,"dither":false}),
        ),
        ("filter.render.clouds", json!({})),
        ("filter.render.differenceClouds", json!({})),
        ("filter.render.fibers", json!({"variance": 20, "strength": 7, "seed": -123})),
        ("filter.render.lensFlare", json!({"brightness": 180, "centerX": 5, "centerY": -5, "lens": "moviePrime"})),
        ("select.rect", json!({"x":1,"y":2,"width":3,"height":4,"mode":"add"})),
        ("select.rect", json!({"x":1,"y":2,"width":3,"height":4,"mode":"add","ellipse":true,"antiAlias":false})),
        ("select.rect", json!({"x":1,"y":2,"width":3,"height":4,"mode":"subtract"})),
        ("select.rect", json!({"x":1,"y":2,"width":3,"height":4,"mode":"intersect"})),
        ("edit.pasteSpecial.pasteInto", json!({})),
        ("edit.pasteSpecial.pasteOutside", json!({})),
        (
            "edit.contentAwareFill",
            json!({"sampling":"rectangular","sampleAllLayers":true,"colorAdaptation":"default",
                "rotationAdaptation":"none","scale":true,"mirror":false,"output":"new"}),
        ),
        ("edit.fade", json!({"opacity":50,"mode":"Multiply"})),
        ("layer.linkLayers", json!({})),
        ("layer.smartObjects.stackMode.range", json!({})),
        ("layer.smartObjects.stackMode.entropy", json!({})),
        ("layer.smartObjects.stackMode.kurtosis", json!({})),
        ("layer.smartObjects.stackMode.maximum", json!({})),
        ("layer.smartObjects.stackMode.median", json!({})),
        ("layer.smartObjects.stackMode.minimum", json!({})),
        ("layer.smartObjects.stackMode.mean", json!({})),
        ("layer.smartObjects.stackMode.skewness", json!({})),
        ("layer.smartObjects.stackMode.standardDeviation", json!({})),
        ("layer.smartObjects.stackMode.summation", json!({})),
        ("layer.smartObjects.stackMode.variance", json!({})),
        ("layer.smartObjects.stackMode.none", json!({})),
        ("layer.distribute.leftEdges", json!({})),
        ("layer.distribute.horizontalCenters", json!({})),
        ("layer.distribute.rightEdges", json!({})),
        ("layer.distribute.topEdges", json!({})),
        ("layer.distribute.verticalCenters", json!({})),
        ("layer.distribute.bottomEdges", json!({})),
        ("edit.definePattern", json!({})),
        ("layer.layerMask.delete", json!({"_atnEvent":"RmvL"})),
        ("layer.mergeLayers", json!({"_atnEvent":"MrgL"})),
        ("filter.stylize.findEdges", json!({})),
        ("filter.stylize.solarize", json!({})),
        ("filter.pixelate.facet", json!({})),
        ("filter.pixelate.fragment", json!({})),
        ("filter.stylize.extrude", json!({"size":42,"depth":91,"solidFrontFaces":true,"maskIncompleteBlocks":true,"type":"pyramids","depthMode":"levelBased"})),
        ("filter.stylize.traceContour", json!({"level":43,"edge":"lower"})),
        ("filter.stylize.wind", json!({"method":"stagger","direction":"fromLeft"})),
        ("filter.stylize.tiles", json!({"count":17,"maxOffset":23,"fill":"foreground"})),
        ("filter.pixelate.colorHalftone", json!({"maxRadius":12,"channel1":-10000,"channel2":10000,"channel3":-721,"channel4":1001})),
        ("filter.pixelate.mezzotint", json!({"type":"longStrokes"})),
        ("filter.pixelate.mosaic", json!({"cellSize":31.5})),
        ("filter.pixelate.pointillize", json!({"cellSize":19})),
        ("filter.blur.gaussianBlur", json!({"radius": 3.5})),
        ("filter.sharpen.unsharpMask", json!({"amount": 120, "radius": 1.5, "threshold": 3})),
        ("filter.noise.addNoise", json!({"amount": 12.5, "distribution": "gaussian", "monochromatic": true, "seed": 7})),
        ("filter.noise.median", json!({"radius": 2})),
        ("filter.stylize.emboss", json!({"angle": 135, "height": 3, "amount": 100})),
        ("image.calculations", json!({"source1":{"channel":{"color":"red"}},"source2":{"channel":{"color":"green"}},"blending":"add","scale":1,"offset":0})),
        ("image.adjustments.colorLookup", color_lookup_case()),
        ("file.automate.batch", batch_case()),
        ("file.automate.batch", batch_save_and_close_case()),
        ("filter.other.custom", json!({"kernel":[0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0],"scale":1,"offset":0})),
        ("filter.render.fibers", json!({"variance":64,"strength":1,"seed":-1})),
        ("filter.blur.shapeBlur", json!({"radius":5,"shapeName":"Recorded Square","shapeId":"original-fixture-square","shapeOriginType":9})),
        ("filter.blur.motionBlur", json!({"angle": 45, "distance": 20})),
        ("filter.blur.smartBlur", json!({"radius": 2.5, "threshold": 40, "quality": "medium", "mode": "overlayEdge"})),
        ("filter.other.offset", json!({"horizontal": 10, "vertical": -5, "undefinedAreas": "wrap"})),
        ("filter.blur.blur", json!({})),
        ("file.automate.createDroplet", json!({"path": "/d/Sharpen.pcdroplet", "action": "Sharpen", "set": "Tools", "output": "/d/out", "overrideOpen": true})),
        ("filter.distort.displace", json!({"horizontal":-999,"vertical":999,"fit":"stretch","undefinedAreas":"repeat","mapPath":LOCAL_MAP})),
        ("filter.distort.shear", json!({"points": [[0,0], [0.5,0.25], [1,0]], "undefinedAreas": "repeat"})),
        ("filter.distort.pinch", json!({"amount": -30})),
        ("filter.distort.polarCoordinates", json!({"mode": "polarToRectangular"})),
        ("filter.distort.spherize", json!({"amount": -50, "mode": "verticalOnly"})),
        ("filter.distort.twirl", json!({"angle": -200})),
        (
            "filter.distort.wave",
            json!({"type": "square", "generators": 3, "wavelengthMin": 20, "wavelengthMax": 200, "amplitudeMin": 4, "amplitudeMax": 40,
                "undefinedAreas": "wrap", "seed": 4_000_000_000_u32}),
        ),
        ("filter.distort.zigZag", json!({"amount": -20, "ridges": 12, "style": "aroundCenter"})),
        ("filter.video.deInterlace", json!({"eliminate": "evenFields", "createBy": "duplication"})),
        ("filter.video.ntscColors", json!({})),
        ("filter.other.hsbHsl", json!({"inputMode": "hsl", "rowOrder": "rgb"})),
        (
            "filter.other.custom",
            json!({"kernel": [0, 0, 0, 0, 0, 0, -1, -1, -1, 0, 0, -1, 9, -1, 0, 0, -1, -1, -1, 0, 0, 0, 0, 0, 0], "scale": 3, "offset": -10}),
        ),
        ("filter.blur.average", json!({})),
        ("layer.matting.removeBlackMatte", json!({})),
        ("layer.matting.removeWhiteMatte", json!({})),
        ("filter.stylize.oilPaint", json!({"lighting": false, "stylization": 6.5, "cleanliness": 3, "scale": 2, "bristleDetail": 7, "angle": 45, "shine": 2})),
        ("filter.sharpen.smartSharpen", json!({"amount": 150, "radius": 2.5, "reduceNoise": 0})),
        ("select.focusArea", json!({})),
        ("select.sky", json!({"sampleAllLayers": false})),
        ("layer.removeBackground", json!({})),
        ("channel.split", json!({})),
        ("channel.mergeSpot", json!({})),
        ("image.trap", json!({"width": 3})),
        ("image.adjustments.matchColor", json!({"luminance": 120, "intensity": 80, "fade": 25})),
        (
            "image.adjustments.levels",
            json!({"inBlack": 5, "inWhite": 250, "gamma": 0.9, "outBlack": 0, "outWhite": 255, "red": {"inBlack": 2, "inWhite": 255, "gamma": 1, "outBlack": 0, "outWhite": 255}}),
        ),
        ("image.adjustments.curves", json!({"points": [[0, 0], [128, 140], [255, 255]], "blue": [[0, 10], [255, 245]]})),
        (
            "image.adjustments.hueSaturation",
            json!({"colorize": false, "hue": 10, "saturation": -20, "lightness": 0, "reds": {"hue": 5, "saturation": 0, "lightness": 0, "range": [315, 345, 15, 45]}}),
        ),
        ("image.adjustments.brightnessContrast", json!({"brightness": 10, "contrast": -5, "legacy": false})),
        (
            "image.adjustments.shadowsHighlights",
            json!({"shadowAmount": 60, "shadowTone": 40, "shadowRadius": 25, "highlightAmount": 10, "highlightTone": 50, "highlightRadius": 30, "blackClip": 0.02, "whiteClip": 0.05, "midtone": 15, "color": 20}),
        ),
        ("image.adjustments.threshold", json!({"level": 100})),
        ("image.adjustments.posterize", json!({"levels": 4})),
        ("image.adjustments.invert", json!({})),
        ("image.adjustments.colorBalance", json!({"shadows": [1, 2, 3], "midtones": [0, 0, 0], "highlights": [-5, 0, 5], "preserveLuminosity": true})),
        ("image.adjustments.vibrance", json!({"vibrance": 30, "saturation": 5})),
        ("image.adjustments.exposure", json!({"exposure": 0.5, "offset": 0.01, "gamma": 1.1})),
        ("image.adjustments.photoFilter", json!({"color": "#ec8a00", "density": 25, "preserveLuminosity": true})),
        (
            "image.adjustments.blackWhite",
            json!({"reds": 40, "yellows": 60, "greens": 40, "cyans": 60, "blues": 20, "magentas": 80, "tint": true, "tintColor": "#e4d5b0"}),
        ),
        ("image.adjustments.channelMixer", json!({"red": [100, 0, 0, 0], "green": [0, 90, 10, 0], "blue": [0, 0, 100, 5], "monochrome": false})),
        ("image.adjustments.selectiveColor", json!({"method": "absolute", "reds": [10, 0, -5, 0], "yellows": [0, 5, 0, 0], "neutrals": [0, 0, 0, 3]})),
        ("select.subject", json!({"sampleAllLayers": false})),
        ("select.colorRange", json!({"select": "midtones", "fuzziness": 30, "tonalRange": [90, 160], "invert": true})),
        ("select.colorRange", json!({"select": "skinTones", "fuzziness": 40})),
        ("select.colorRange", json!({"select": "highlights", "tonalRange": 200})),
        ("select.colorRange", json!({"select": "reds"})),
        ("layer.createClippingMask", json!({})),
        ("layer.releaseClippingMask", json!({"layer": "Shadow"})),
        ("layer.layerMask.revealAll", json!({})),
        ("layer.layerMask.hideAll", json!({})),
        ("layer.layerMask.revealSelection", json!({})),
        ("layer.layerMask.hideSelection", json!({})),
        ("tools.setColors", json!({"foreground": "#336699"})),
        ("tools.setColors", json!({"background": "#ffffff"})),
        ("image.adjustments.desaturate", json!({})),
        ("image.autoTone", json!({})),
        ("image.imageSize", json!({"width": 800, "height": 600, "resolution": 72, "resample": "bicubicSharper", "scaleStyles": true})),
        ("image.canvasSize", json!({"width": 100, "height": 50, "relative": true, "anchor": "topLeft", "extensionColor": "white"})),
        ("image.imageRotation.90cw", json!({})),
        ("image.rotation.arbitrary", json!({"angle": 12.5, "direction": "ccw"})),
        ("edit.transform.flipHorizontal", json!({})),
        ("image.imageRotation.flipCanvasVertical", json!({})),
        ("image.crop", json!({"x": 10, "y": 20, "width": 100, "height": 50, "deleteCroppedPixels": true})),
        ("image.trim", json!({"basedOn": "transparent", "top": true, "bottom": true, "left": false, "right": true})),
        ("image.mode.grayscale", json!({})),
        ("image.mode.bits16", json!({})),
        ("layer.duplicate", json!({"name": "copy"})),
        // Duplicate into a new document is a `Mk`, not a `Dplc` (the three shapes of
        // a recorded Duplicate Layer with Destination: New, plus a named layer).
        ("layer.duplicate", json!({"document": "new"})),
        ("layer.duplicate", json!({"document": "new", "documentName": "test new name"})),
        ("layer.duplicate", json!({"document": "new", "name": "Laer name change"})),
        ("layer.duplicate", json!({"layer": "Shadow", "document": "new", "documentName": "Doc"})),
        ("image.duplicate", json!({"mergedOnly": true})),
        ("layer.new.layer", json!({"name": "Ink", "opacity": 0.5, "mode": "Multiply"})),
        ("type.convertToShape", json!({})),
        ("type.createWorkPath", json!({})),
        (
            "path.set",
            json!({"name":"work","path":{"fillRule":"evenodd","inverted":false,"subpaths":[{"closed":false,"op":"combine","knots":[
                {"anchor":[1,2],"in":[1,2],"out":[3,4],"smooth":false},
                {"anchor":[8,9],"in":[6,7],"out":[8,9],"smooth":false}
            ]}]}}),
        ),
        ("filter.blur.shapeBlur", json!({})),
        ("filter.blur.shapeBlur", json!({"radius": 10})),
        ("layer.new.group", json!({"name": "G"})),
        ("layer.groupLayers", json!({})),
        ("layer.linkLayers", json!({"linked": false})),
        ("layer.linkLayers", json!({"linked": false, "_atnNoTarget": true})),
        ("layer.arrange.reverse", json!({})),
        ("layer.arrange.reverse", json!({"_atnNoTarget": true})),
        ("layer.selectLinkedLayers", json!({})),
        ("layer.layerStyle.clear", json!({})),
        ("layer.rasterize.allLayers", json!({})),
        ("layer.videoLayers.newBlankVideoLayer", json!({})),
        ("select.isolateLayers", json!({})),
        ("type.loadDefaultTypeStyles", json!({})),
        ("type.saveDefaultTypeStyles", json!({})),
        ("image.analysis.recordMeasurements", json!({})),
        ("image.analysis.setMeasurementScale", json!({"preset": "custom", "pixelLength": 120, "logicalLength": 2.5, "units": "mm"})),
        ("layer.videoLayers.insertBlankFrame", json!({})),
        ("layer.videoLayers.duplicateFrame", json!({})),
        ("layer.videoLayers.deleteFrame", json!({})),
        ("file.automate.cropAndStraightenPhotos", json!({})),
        ("file.automate.cropAndStraightenPhotos", json!({"_atnNoTarget": true})),
        ("layer.new.artboardFromGroup", json!({})),
        ("layer.new.artboardFromGroup", json!({"background":"white","color":[1,1,1],"preset":"","guideIDs":[],"changeBackground":1})),
        (
            "layer.artboard.set",
            json!({"background":"custom","color":[0.25,0.5,0.75],"recordedPreset":"","guideIDs":[],"recordedRect":[2,2,6,6],"changeSizes":8,"changeBackground":1}),
        ),
        ("layer.new.layerViaCopy", json!({})),
        ("layer.newFillLayer.solidColor", json!({"color": "#336699"})),
        ("layer.newAdjustmentLayer.hueSaturation", json!({})),
        ("layer.newAdjustmentLayer.threshold", json!({"level": 77})),
        ("file.new", json!({"width": 640, "height": 480, "resolution": 72, "mode": "rgb", "depth": 8, "background": "transparent", "name": "Doc"})),
        ("select.all", json!({})),
        ("select.deselect", json!({})),
        ("select.loadSelection", json!({"channel": "transparency"})),
        ("select.loadSelection", json!({"channel": "Alpha 1"})),
        ("select.rect", json!({"x": 1, "y": 2, "width": 30, "height": 40})),
        ("select.rect", json!({"x": 1, "y": 2, "width": 30, "height": 40, "ellipse": true})),
        ("layer.setProps", json!({"name": "Renamed", "opacity": 0.4, "blend": "Linear Light"})),
        ("layer.setProps", json!({"layer": "Shadow", "visible": false})),
        ("layer.setLabelColor", json!({"color": "red"})),
        ("layer.showOnly", json!({"layer": "Solo"})),
        ("layer.select", json!({"layer": "Shadow", "mode": "add"})),
        ("layer.select", json!({"layer": "Shadow", "mode": "range"})),
        ("layer.select", json!({"layer": {"index": 2}})),
        ("layer.moveTo", json!({"target": {"index": 0}})),
        ("layer.moveTo", json!({"layer": {"index": 2}, "target": {"index": 4}})),
        ("layer.selectAbove", json!({})),
        ("layer.selectBottom", json!({})),
        ("layer.selectTop", json!({})),
        ("layer.selectBelow", json!({})),
        ("layer.addBelowToSelection", json!({})),
        ("layer.addAboveToSelection", json!({})),
        ("layer.delete", json!({})),
        ("edit.clear", json!({})),
        ("layer.hideLayers", json!({"layer": "Background"})),
        ("layer.showLayers", json!({})),
        ("layer.translate", json!({"dx": 3, "dy": -4})),
        ("layer.arrange.sendToBack", json!({})),
        ("layer.flattenImage", json!({})),
        ("layer.stampVisible", json!({})),
        ("layer.mergeVisible", json!({})),
        ("layer.matting.defringe", json!({"width": 2})),
        ("layer.align.leftEdges", json!({"to": "canvas"})),
        ("select.inverse", json!({})),
        ("select.modify.feather", json!({"radius": 4})),
        ("select.modify.contract", json!({"radius": 1})),
        ("edit.fill", json!({"contents": "color", "color": "#ff0080", "opacity": 50, "mode": "Multiply", "preserveTransparency": true})),
        ("edit.fill", json!({"contents": "foreground", "opacity": 100, "mode": "Normal"})),
        ("edit.stroke", json!({"width": 3, "location": "outside", "opacity": 100, "mode": "Normal", "color": "#000000", "preserveTransparency": false})),
        ("edit.copyMerged", json!({})),
        ("edit.pasteSpecial.pasteInPlace", json!({})),
        ("edit.paste", json!({})),
        ("edit.purge.histories", json!({})),
        ("edit.checkSpelling", json!({"allLayers": true})),
        ("edit.checkSpelling", json!({"allLayers": false})),
        ("tools.defaultColors", json!({})),
        (
            "edit.convertToProfile",
            json!({"profile": "display-p3", "intent": "perceptual", "bpc": false, "dither": false, "flatten": false, "rasterizePlaced": false}),
        ),
        ("edit.assignProfile", json!({"profile": "srgb"})),
        ("file.close", json!({})),
        ("file.save", json!({})),
        ("layer.layerMask.delete", json!({})),
        ("select.lasso", json!({"points": [[1, 2], [30, 2], [15, 40]]})),
        ("select.editInQuickMaskMode", json!({"on": true})),
        ("select.editInQuickMaskMode", json!({"on": false})),
        ("edit.transform", json!({"dx": 10, "dy": -5, "scaleX": 50, "scaleY": 75, "rotate": 12.5, "interpolation": "bicubic"})),
        ("edit.transform", json!({"layer": "Shadow", "dx": 0, "dy": 0, "scaleX": 200})),
        ("select.transformSelection", json!({"dx": 3, "dy": 4, "scaleX": 110, "scaleY": 110, "rotate": -30})),
        (
            "image.applyImage",
            json!({"source": {"channel": "composite", "layer": "High-Frequency"}, "blending": "subtract", "scale": 2, "offset": 128, "preserveTransparency": true}),
        ),
        ("image.applyImage", json!({"source": {"channel": "red", "layer": "merged", "invert": true}, "blending": "multiply", "opacity": 50})),
        (
            "file.export.saveForWebLegacy",
            json!({"path": "/tmp/out/banner.png", "format": "png24", "interlaced": false, "transparency": true, "embedIcc": false, "progressive": false, "optimized": true, "matte": "#ffffff"}),
        ),
        (
            "file.export.saveForWebLegacy",
            json!({"folder": "/tmp/out", "format": "png24", "interlaced": false, "transparency": true, "embedIcc": false, "progressive": false, "optimized": true, "matte": "#ffffff"}),
        ),
        (
            "file.export.saveForWebLegacy",
            json!({"path": "C:\\shots\\a.jpg", "format": "jpeg", "interlaced": false, "transparency": false, "embedIcc": true, "progressive": true, "optimized": true, "quality": 80, "matte": "none"}),
        ),
        ("image.adjustments.gradientMap", json!({"stops": [[0, "#000000"], [0.5, "#ff8000"], [1, "#ffffff"]], "reverse": true, "dither": false})),
        ("image.adjustments.gradientMap", json!({"stops": [[0,[0.5,0,0]],[1,[0,1,0]]], "__atnGrMp":{"name":"Recorded","colors":[[127.5,0,0],[0,255,0]]}})),
        ("image.adjustments.replaceColor", json!({"labMinimum":[40,30,40],"labMaximum":[60,35,45],"fuzziness":40,"hue":30,"saturation":0,"lightness":0})),
        ("layer.newAdjustmentLayer.gradientMap", json!({"stops": [[0, "#102030"], [1, "#f0e0d0"]], "reverse": false, "dither": true})),
        ("layer.smartObjects.convertToSmartObject", json!({})),
        ("layer.smartObjects.editContents", json!({})),
        ("layer.smartObjects.updateModifiedContent", json!({})),
        ("layer.smartObjects.newSmartObjectViaCopy", json!({})),
        ("layer.smartObjects.convertToLayers", json!({})),
        ("layer.smartObjects.convertToEmbedded", json!({})),
        ("layer.smartObjects.updateAllModifiedContent", json!({})),
        ("layer.smartObjects.exportContents", json!({"path": "/tmp/contents.png"})),
        ("layer.smartObjects.convertToLinked", json!({"path": "/tmp/linked.png"})),
        ("layer.smartObjects.relinkToFile", json!({"path": "/tmp/relinked.png"})),
        ("layer.smartObjects.replaceContents", json!({"path": "/tmp/replacement.png"})),
        ("paint.bucket", json!({"x": 10, "y": 20, "tolerance": 32, "antiAlias": true, "contiguous": false, "opacity": 100})),
        ("image.imageSize", json!({"percent": 200, "resample": "bicubicAutomatic", "scaleStyles": true})),
        ("file.placeEmbedded", json!({"path": "/tmp/logo.png", "scale": 50})),
        ("file.placeLinked", json!({"path": "/tmp/logo.png"})),
        ("file.openAs", json!({"path": "/tmp/a.psd"})),
        ("file.saveAs", json!({"path": "/tmp/out/a.png"})),
        ("file.saveACopy", json!({"path": "/tmp/out/a.tif"})),
        ("actions.conditional", json!({"if": "hasLayers", "then": "FS8-Layers", "else": "FS8-No-Layers", "set": "S"})),
        ("actions.conditional", json!({"if": "landscape", "then": "Wide", "else": null, "set": "S"})),
        ("tools.setColors", json!({"foreground": "#102030"})),
        ("tools.setColors", json!({"background": "#ffffff"})),
        ("view.fitOnScreen", json!({})),
        ("view.zoomIn", json!({})),
        ("layer.createClippingMask", json!({})),
        ("layer.ungroupLayers", json!({})),
        ("layer.layerStyle.copyLayerStyle", json!({})),
        ("layer.layerStyle.scaleEffects", json!({"scale": 200})),
        ("layer.layerStyle.createLayer", json!({})),
        ("layer.layerStyle.pasteLayerStyle", json!({"allowGroups": true})),
        ("layer.setExpanded", json!({"all": true, "expanded": false})),
        ("layerComp.apply", json!({"comp": "Original"})),
        ("actions.play", json!({"action": "Other", "set": "S"})),
        (STOP_MESSAGE, json!({"message": "Hi", "allowContinue": false})),
        ("document.activate", json!({"offset": -1})),
        ("document.activate", json!({"document": {"index": 2}})),
        // The TrueGrit Texture Supply sets (Debaser, Distress Press, Halftone Zine Machine).
        ("filter.distort.ripple", json!({"amount": 50, "size": "medium"})),
        ("filter.pixelate.crystallize", json!({"cellSize": 5})),
        ("filter.gallery.spatter", json!({"sprayRadius": 7, "smoothness": 15})),
        ("filter.gallery.accentedEdges", json!({"edgeWidth": 14, "edgeBrightness": 50, "smoothness": 15})),
        ("filter.gallery.angledStrokes", json!({"directionBalance": 100, "strokeLength": 50, "sharpness": 10})),
        ("filter.gallery.basRelief", json!({"detail": 15, "smoothness": 15, "light": "bottomLeft"})),
        ("filter.gallery.chalkCharcoal", json!({"charcoalArea": 20, "chalkArea": 20, "strokePressure": 5})),
        ("filter.gallery.charcoal", json!({"charcoalThickness": 7, "detail": 0, "lightDarkBalance": 100})),
        ("filter.gallery.chrome", json!({"detail": 10, "smoothness": 10})),
        ("filter.gallery.coloredPencil", json!({"pencilWidth": 24, "strokePressure": 15, "paperBrightness": 50})),
        (
            "filter.gallery.conteCrayon",
            json!({"foregroundLevel": 15, "backgroundLevel": 15, "texture": "brick", "scaling": 200, "relief": 50, "light": "bottomLeft", "invert": true}),
        ),
        ("filter.gallery.craquelure", json!({"crackSpacing": 100, "crackDepth": 10, "crackBrightness": 10})),
        ("filter.gallery.crosshatch", json!({"strokeLength": 50, "sharpness": 20, "strength": 3})),
        ("filter.gallery.cutout", json!({"numberOfLevels": 8, "edgeSimplicity": 10, "edgeFidelity": 3})),
        ("filter.gallery.darkStrokes", json!({"balance": 10, "blackIntensity": 10, "whiteIntensity": 10})),
        ("filter.gallery.diffuseGlow", json!({"graininess": 10, "glowAmount": 20, "clearAmount": 20})),
        ("filter.gallery.dryBrush", json!({"brushSize": 10, "brushDetail": 10, "texture": 3})),
        ("filter.gallery.filmGrain", json!({"grain": 20, "highlightArea": 20, "intensity": 0})),
        ("filter.gallery.fresco", json!({"brushSize": 10, "brushDetail": 10, "texture": 3})),
        ("filter.gallery.glass", json!({"distortion": 20, "smoothness": 15, "texture": "canvas", "scaling": 200, "invert": true})),
        ("filter.gallery.glowingEdges", json!({"edgeWidth": 14, "edgeBrightness": 20, "smoothness": 15})),
        ("filter.gallery.graphicPen", json!({"strokeLength": 1, "lightDarkBalance": 100, "strokeDirection": "horizontal"})),
        ("filter.gallery.halftonePattern", json!({"size": 12, "contrast": 50, "patternType": "circle"})),
        ("filter.gallery.inkOutlines", json!({"strokeLength": 50, "darkIntensity": 50, "lightIntensity": 50})),
        ("filter.gallery.mosaicTiles", json!({"tileSize": 100, "groutWidth": 15, "lightenGrout": 10})),
        ("filter.gallery.neonGlow", json!({"glowSize": 24, "glowBrightness": 50, "glowColor": "#336699"})),
        ("filter.gallery.notePaper", json!({"imageBalance": 50, "graininess": 20, "relief": 25})),
        ("filter.gallery.oceanRipple", json!({"rippleSize": 15, "rippleMagnitude": 20})),
        ("filter.gallery.paintDaubs", json!({"brushSize": 50, "sharpness": 40, "brushType": "darkRough"})),
        ("filter.gallery.paletteKnife", json!({"strokeSize": 50, "strokeDetail": 1, "softness": 10})),
        ("filter.gallery.patchwork", json!({"squareSize": 10, "relief": 25})),
        ("filter.gallery.photocopy", json!({"detail": 24, "darkness": 50})),
        ("filter.gallery.plaster", json!({"imageBalance": 50, "smoothness": 15, "light": "bottomLeft"})),
        ("filter.gallery.plasticWrap", json!({"highlightStrength": 20, "detail": 15, "smoothness": 15})),
        ("filter.gallery.posterEdges", json!({"edgeThickness": 10, "edgeIntensity": 10, "posterization": 6})),
        ("filter.gallery.reticulation", json!({"density": 50, "foregroundLevel": 50, "backgroundLevel": 50})),
        (
            "filter.gallery.roughPastels",
            json!({"strokeLength": 40, "strokeDetail": 20, "texture": "brick", "scaling": 200, "relief": 50, "light": "bottomLeft", "invert": true}),
        ),
        ("filter.gallery.smudgeStick", json!({"strokeLength": 10, "highlightArea": 20, "intensity": 0})),
        ("filter.gallery.sponge", json!({"brushSize": 10, "definition": 25, "smoothness": 15})),
        ("filter.gallery.sprayedStrokes", json!({"strokeLength": 20, "sprayRadius": 25, "strokeDirection": "horizontal"})),
        ("filter.gallery.stainedGlass", json!({"cellSize": 50, "borderThickness": 20, "lightIntensity": 10})),
        ("filter.gallery.stamp", json!({"lightDarkBalance": 50, "smoothness": 50})),
        ("filter.gallery.sumiE", json!({"strokeWidth": 15, "strokePressure": 15, "contrast": 40})),
        ("filter.gallery.texturizer", json!({"texture": "brick", "scaling": 200, "relief": 50, "light": "bottomLeft", "invert": true})),
        ("filter.gallery.tornEdges", json!({"imageBalance": 50, "smoothness": 15, "contrast": 25})),
        (
            "filter.gallery.underpainting",
            json!({"brushSize": 40, "textureCoverage": 40, "texture": "brick", "scaling": 200, "relief": 50, "light": "bottomLeft", "invert": true}),
        ),
        ("filter.gallery.waterPaper", json!({"fiberLength": 50, "brightness": 100, "contrast": 100})),
        ("filter.gallery.watercolor", json!({"brushDetail": 14, "shadowIntensity": 10, "texture": 3})),
        ("filter.gallery.grain", json!({"intensity": 100, "contrast": 100, "grainType": "contrasty"})),
        (
            "select.refineEdge",
            json!({"radius": 0, "smartRadius": false, "smooth": 2, "feather": 0.5, "contrast": 15, "shiftEdge": -100, "decontaminate": false, "output": "layerMask"}),
        ),
        ("channel.target", json!({"channel": "mask"})),
        ("channel.target", json!({"channel": "composite"})),
        ("edit.undo", json!({})),
        ("edit.redo", json!({})),
        ("layer.select", json!({"layer": "YELLOW", "mode": "remove"})),
        ("layer.select", json!({"layer": "Or paste photocopy texture here", "mode": "range"})),
        ("layer.translate", json!({"dx": 0.36, "dy": -1.08, "units": "pt"})),
        ("select.loadSelection", json!({"channel": "targeted"})),
        ("layer.newFillLayer.pattern", json!({"pattern": "Debaser-Distressed-Dark", "scale": 50, "angle": 0, "link": true})),
        ("layer.fill.pattern.set", json!({"pattern": "1 -R2.jpg"})),
        ("layer.newFillLayer.pattern", json!({"pattern": "b7a3c1d2-0000-4000-8000-000000000001", "patternName": "Twin", "scale": 50})),
        ("image.mode.bitmap", json!({"method": "threshold"})),
        ("image.imageSize", json!({"resolution": 1200, "resample": "preserveDetails", "keepPrintSize": true, "scaleStyles": true})),
        ("layer.new.layerFromBackground", json!({"ifPresent": true, "select": false})),
        // COPYCAT, AJ_Actions2023 and the Line and Color tutorial set.
        ("layer.smartFilter.deleteFilterMask", json!({"ifPresent": true})),
        ("layer.smartFilter.blendingOptions", json!({"index": 1, "opacity": 0.5, "blend": "Darken"})),
        ("layer.layerStyle.blendingOptions", json!({"blendClippedLayersAsGroup": false, "transparencyShapesLayer": false})),
        ("layer.layerStyle.blendingOptions", json!({"blendIf": [{"channel": "gray", "thisLayer": [0, 30, 200, 255], "underlying": [0, 0, 255, 255]}]})),
        (
            "layer.layerStyle.blendingOptions",
            json!({"opacity": 40, "fillOpacity": 70, "blend": "Multiply", "blendIf": [{"channel": "blue", "thisLayer": [5, 5, 250, 250]}]}),
        ),
        ("layer.createClippingMask", json!({})),
        ("layer.lockLayers", json!({"transparency": true})),
        ("filter.stylize.diffuse", json!({"mode": "normal", "seed": 321654})),
        ("history.clear", json!({})),
        ("history.newSnapshot", json!({"name": "CopyCat - action result", "from": "fullDocument", "replaceExisting": true})),
        ("layerComp.apply", json!({"comp": "Comp 1"})),
        ("layerComp.apply", json!({"comp": {"index": 2}})),
        ("layerComp.apply", json!({"comp": {"id": 7}})),
        ("layerComp.apply", json!({"comp": {"ordinal": "Nxt"}})),
    ];
    for (id, params) in &cases {
        let item = export_item(id, params, "S").unwrap_or_else(|| panic!("{id} does not export"));
        if *id == "file.close" {
            // Close exports "don't save" and imports as a plain close; the Save answer is tested below.
            assert_eq!(item.event.id(), "Cls ");
            continue;
        }
        let e = event_for(&item.event).unwrap_or_else(|| panic!("{id} exported as an unknown event {}", item.event.id()));
        assert_eq!(item.descriptor.is_none(), *id == super::steps_stack::NONE, "{id}: unexpected descriptor presence");
        let descriptor = item.descriptor.unwrap_or_default();
        match (e.from)(&descriptor) {
            Mapped::Steps(steps) => {
                assert_eq!(steps.len(), 1, "{id}: {steps:?}");
                let navigation = match *id {
                    "layer.selectAbove" => Some(("Frwr", false)),
                    "layer.selectBelow" => Some(("Bckw", false)),
                    "layer.selectTop" => Some(("Frnt", false)),
                    "layer.selectBottom" => Some(("Back", false)),
                    "layer.addAboveToSelection" => Some(("Frwr", true)),
                    "layer.addBelowToSelection" => Some(("Bckw", true)),
                    _ => None,
                };
                if let Some((ordinal, add)) = navigation {
                    let mut expected = json!({"layer": {"ordinal": ordinal}});
                    if add {
                        expected["mode"] = json!("add");
                    }
                    assert_eq!(steps, vec![("layer.select".into(), expected)]);
                    continue;
                }
                assert_eq!(steps[0].0, *id, "{id} came back as another command");
                let mut back = steps[0].1.clone();
                if *id == "paint.gradient" {
                    // Executable values match; independent public tests verify the source carrier.
                    back.as_object_mut().unwrap().remove("__atnClassicGradient");
                }
                if *id == "path.set" {
                    // The imported step retains the original descriptor alongside its playable path.
                    back.as_object_mut().unwrap().remove(super::steps_path::SOURCE).unwrap();
                }
                if *id == "edit.perspectiveWarp" {
                    back.as_object_mut().unwrap().remove("__atnPerspectiveWarp");
                }
                normalise(&mut back);
                if *id == "actions.play" || *id == "actions.conditional" {
                    back["set"] = json!("S");
                }
                assert_eq!(back, *params, "{id}");
            }
            Mapped::Unsupported { .. } => panic!("{id} does not import back"),
        }
    }
    // Partial native backgrounds need no recorded preset, guide IDs or last-used color.
    for (background, imported_background) in [("white", "white"), ("black", "black"), ("transparent", "transparent"), ("custom", "white")] {
        let item = export_item("layer.new.artboardFromGroup", &json!({"background":background}), "S").unwrap();
        let Mapped::Steps(steps) = (event_for(&item.event).unwrap().from)(item.descriptor.as_ref().unwrap()) else { panic!("Artboard did not import") };
        assert_eq!(steps, [("layer.new.artboardFromGroup".into(), json!({"background":imported_background,"color":[1.0,1.0,1.0],"preset":"","guideIDs":[]}))]);
        assert_eq!(export_item(&steps[0].0, &steps[0].1, "S").unwrap().descriptor, item.descriptor);
    }
    // Every command the table claims to export has a case above.
    let covered: std::collections::BTreeSet<&str> = cases.iter().map(|(id, _)| *id).collect();
    let mut missing: Vec<&str> = EVENTS.iter().flat_map(|e| e.ids.iter().copied()).filter(|id| !covered.contains(id)).collect();
    missing.sort_unstable();
    let allowed = [
        "edit.transform.flipVertical",
        "edit.transform.rotate180",
        "edit.transform.rotate90Ccw",
        "edit.transform.rotate90Cw",
        "image.autoColor",
        "image.autoContrast",
        "image.imageRotation.180",
        "image.imageRotation.90ccw",
        "image.imageRotation.flipCanvasHorizontal",
        "image.mode.bits32",
        "image.mode.bits8",
        "image.mode.cmyk",
        "image.mode.lab",
        "image.mode.multichannel",
        "image.mode.rgb",
        "layer.align.bottomEdges",
        "layer.align.horizontalCenters",
        "layer.align.rightEdges",
        "layer.align.topEdges",
        "layer.align.verticalCenters",
        "layer.arrange.bringForward",
        "layer.arrange.bringToFront",
        "layer.arrange.sendBackward",
        "layer.mergeLayers",
        "layer.new.layerViaCut",
        "layer.newAdjustmentLayer.blackWhite",
        "layer.newAdjustmentLayer.brightnessContrast",
        "layer.newAdjustmentLayer.channelMixer",
        "layer.newAdjustmentLayer.colorBalance",
        "layer.newAdjustmentLayer.curves",
        "layer.newAdjustmentLayer.exposure",
        "layer.newAdjustmentLayer.invert",
        "layer.newAdjustmentLayer.levels",
        "layer.newAdjustmentLayer.photoFilter",
        "layer.newAdjustmentLayer.posterize",
        "layer.newAdjustmentLayer.selectiveColor",
        "layer.newAdjustmentLayer.vibrance",
        "layer.rasterize.layer",
        "layer.renameLayer",
        "layer.setAdjustment",
        "select.allLayers",
        "select.deselectLayers",
        "select.grow",
        "select.modify.border",
        "select.modify.expand",
        "select.modify.smooth",
        "select.reselect",
        "select.similar",
        "view.actualPixels",
        "view.zoomOut",
        "tools.swapColors",
        "edit.cut",
        "edit.copy",
        "edit.purge.all",
        "edit.purge.clipboard",
        "edit.purge.undo",
        "file.newFromClipboard",
        "file.revert",
        "image.adjustments.equalize",
        "image.revealAll",
        "filter.blur.blurMore",
        "filter.blur.boxBlur",
        "filter.blur.radialBlur",
        "filter.blur.surfaceBlur",
        "filter.noise.despeckle",
        "filter.noise.dustAndScratches",
        "filter.other.highPass",
        "filter.other.maximum",
        "filter.other.minimum",
        "filter.sharpen.sharpen",
        "filter.sharpen.sharpenEdges",
        "filter.sharpen.sharpenMore",
        // Saves back only as the descriptor it was loaded from (loaded_camera_raw_filter_steps_save_back_unchanged).
        "filter.cameraRaw",
        // Load with the recorded descriptor riding along (gradient_fill_steps_export_and_import_back).
        "gradient.fill.set",
        "layer.newFillLayer.gradient",
    ];
    let unexpected: Vec<&str> = missing.iter().copied().filter(|id| !allowed.contains(id)).collect();
    assert!(unexpected.is_empty(), "add round-trip cases for {unexpected:?}");
    // Every command the table names exists.
    // (Save is the desktop shell's: recorded by it, replayed by it, written by Batch.)
    for id in EVENTS.iter().flat_map(|e| e.ids.iter()) {
        assert!(
            crate::commands::find(id).is_some() || crate::actions_cmds::shell_save_command(id) || crate::actions_cmds::shell_view_command(id),
            "the .atn table names an unknown command `{id}`"
        );
    }
}

/// `paint.bucket` exported to .atn and imported back → its params, or None when it doesn't export.
fn bucket_round_trip(params: Value) -> Option<Value> {
    let item = export_item("paint.bucket", &params, "S")?;
    let e = event_for(&item.event)?;
    match (e.from)(item.descriptor.as_ref()?) {
        Mapped::Steps(steps) => steps.first().map(|(id, p)| {
            assert_eq!(id, "paint.bucket");
            p.clone()
        }),
        _ => None,
    }
}

/// A Paint Bucket step with an explicit colour keeps it through .atn (#7199): replayed on a fresh
/// document, whose foreground is black, it still fills red. A colour .atn can't hold makes the
/// step not export rather than fill with the foreground.
#[test]
fn bucket_keeps_an_explicit_color_through_atn() {
    let fill = |params: &Value| {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "#ffffff"})).unwrap();
        s.execute("paint.bucket", params.clone()).unwrap();
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().pixel(2, 2)
    };
    let red = json!({"x": 2, "y": 2, "antiAlias": false, "color": "#ff0000"});
    let back = bucket_round_trip(red.clone()).unwrap();
    assert_eq!(back["color"], "#ff0000", "{back}");
    assert_eq!(fill(&back), fill(&red));
    assert_eq!(fill(&back), vec![1.0, 0.0, 0.0, 1.0]);
    // The colour as an array of channels.
    assert_eq!(bucket_round_trip(json!({"x": 1, "y": 1, "color": [0.0, 0.5, 1.0]})).unwrap()["color"], "#0080ff");
    // No colour, or one the command can't read: the foreground, as before.
    for p in [json!({"x": 1, "y": 1}), json!({"x": 1, "y": 1, "color": "red"})] {
        let back = bucket_round_trip(p.clone()).unwrap();
        assert!(back.get("color").is_none() && back.get("contents").is_none(), "{p} → {back}");
    }
    // A pattern fill ignores the colour, also when named with the older `source`.
    for key in ["contents", "source"] {
        let back = bucket_round_trip(json!({"x": 1, "y": 1, key: "pattern", "color": "#ff0000"})).unwrap();
        assert_eq!(back["contents"], "pattern", "{key}: {back}");
        assert!(back.get("color").is_none(), "{key}: {back}");
    }
    // Translucent or out-of-range colours have no .atn form.
    for c in [json!("#ff000080"), json!([1.0, 0.0, 0.0, 0.5]), json!([2.0, 0.0, 0.0])] {
        assert!(bucket_round_trip(json!({"x": 1, "y": 1, "color": c})).is_none(), "{c}");
    }
}

/// A Stroke step that leaves settings at their defaults still writes them (#8303): Photoshop
/// opens the Stroke dialog for a descriptor without its blend mode, even with dialogs off.
#[test]
fn stroke_writes_the_defaults_it_leaves_out() {
    let exported = |params: Value| export_item("edit.stroke", &params, "S").and_then(|item| item.descriptor).unwrap();
    let d = exported(json!({"width": 2, "location": "inside", "color": "#d03060", "opacity": 63}));
    assert_eq!(get_enum(&d, "Md  ").as_deref(), Some("Nrml"), "{d:?}");
    assert_eq!(get_enum(&d, "Lctn").as_deref(), Some("Insd"), "an explicit setting is kept");
    assert_eq!(get_num(&d, "Opct"), Some(63.0));
    let d = exported(json!({"width": 2}));
    assert_eq!(get_enum(&d, "Lctn").as_deref(), Some("Cntr"), "{d:?}");
    assert_eq!(get_num(&d, "Opct"), Some(100.0));
    assert_eq!(get_enum(&d, "Md  ").as_deref(), Some("Nrml"));
    assert!(get_bool(&d, "PrsT") == Some(false), "{d:?}");
    // An explicit mode is kept, and the step imports back to the same stroke.
    let p = json!({"width": 3, "location": "outside", "mode": "Multiply", "opacity": 40, "color": "#000000", "preserveTransparency": true});
    let d = exported(p.clone());
    let Mapped::Steps(steps) = (event_for(&AtnEvent::Code(*b"Strk")).unwrap().from)(&d) else { panic!("{d:?}") };
    assert_eq!(steps[0].1, json!({"width": 3.0, "location": "outside", "mode": "Multiply", "opacity": 40.0, "color": "#000000", "preserveTransparency": true}));
}

/// `image.adjustments.photoFilter` exported to .atn and imported back → its params, or None.
fn photo_filter_round_trip(params: Value) -> Option<Value> {
    let item = export_item("image.adjustments.photoFilter", &params, "S")?;
    let e = event_for(&item.event)?;
    match (e.from)(item.descriptor.as_ref()?) {
        Mapped::Steps(steps) => steps.first().map(|(id, p)| {
            assert_eq!(id, "image.adjustments.photoFilter");
            p.clone()
        }),
        _ => None,
    }
}

/// A `Mk` layerSection step exported to .atn and imported back → (id, params), or None.
fn group_round_trip(id: &str, params: &Value) -> Option<(String, Value)> {
    let item = export_item(id, params, "S")?;
    let e = event_for(&item.event)?;
    match (e.from)(item.descriptor.as_ref()?) {
        Mapped::Steps(steps) => steps.first().map(|(i, p)| (i.clone(), p.clone())),
        _ => None,
    }
}

#[test]
fn photo_filter_preset_only_step_keeps_its_resolved_color() {
    // #7235: the Photoshop descriptor keeps only a colour (`phfl`/`PhtF` are colour-only; the io
    // notes reach the same conclusion from the CS2 oracles). A step recorded with only `filter`
    // previously exported with no colour at all, so re-imported playback fell back to the default
    // warming filter; it must export the resolved colour of the chosen preset instead.
    let mut back = photo_filter_round_trip(json!({"filter": "cooling80", "density": 60, "preserveLuminosity": true})).unwrap();
    normalise(&mut back);
    assert_eq!(back, json!({"color": "#006dff", "density": 60, "preserveLuminosity": true}));
    // An omitted density stays omitted: running the imported step applies the same default.
    let mut back = photo_filter_round_trip(json!({"filter": "red"})).unwrap();
    normalise(&mut back);
    assert_eq!(back, json!({"color": "#ea1a1a"}));
    // With an explicit colour too, the colour wins, as when running the command.
    let mut back = photo_filter_round_trip(json!({"filter": "cooling80", "color": "#123456", "density": 25})).unwrap();
    normalise(&mut back);
    assert_eq!(back, json!({"color": "#123456", "density": 25}));
    let mut back = photo_filter_round_trip(json!({"color": "#123456", "density": 25})).unwrap();
    normalise(&mut back);
    assert_eq!(back, json!({"color": "#123456", "density": 25}));
    // An unknown preset id keeps the old export: no colour resolves, playback applies its default.
    let mut back = photo_filter_round_trip(json!({"filter": "nope", "density": 25})).unwrap();
    normalise(&mut back);
    assert_eq!(back, json!({"density": 25}));
}

#[test]
fn group_layers_keeps_its_explicit_target_through_atn() {
    // #7236: a recorded step with an explicit layer groups that layer, not the current
    // selection. The source goes into the `From` reference by name and comes back the same
    // way; a step with no source layer keeps the implicit form, imported as before.
    let (id, p) = group_round_trip("layer.groupLayers", &json!({"layer": "A", "name": "Group"})).unwrap();
    assert_eq!(id, "layer.groupLayers");
    assert_eq!(p, json!({"layer": "A", "name": "Group"}));
    // The implicit form survives unchanged: the reference stays the target layer.
    let (id, p) = group_round_trip("layer.groupLayers", &json!({"name": "Group"})).unwrap();
    assert_eq!(id, "layer.groupLayers");
    assert_eq!(p, json!({"name": "Group"}));
    let (id, p) = group_round_trip("layer.groupLayers", &json!({})).unwrap();
    assert_eq!(id, "layer.groupLayers");
    assert_eq!(p, json!({}));
    // Group Without Layers: no `From` at all, imported as a plain new group.
    let (id, p) = group_round_trip("layer.new.group", &json!({"name": "G"})).unwrap();
    assert_eq!(id, "layer.new.group");
    assert_eq!(p, json!({"name": "G"}));
}

#[test]
fn group_layers_from_index_reference_stays_implicit() {
    // A Photoshop-authored `From` that names nothing portable (a zero-based index) keeps the
    // old implicit import instead of inventing a wrong explicit target (#7236, edge).
    let d = desc("null")
        .with("null", class_ref("layerSection"))
        .with("From", D::Reference(vec![ReferenceItem::Index { class: class("Lyr "), index: 2 }]))
        .with("Usng", D::Descriptor(desc("layerSection").with("Nm  ", text("Group"))));
    match make_from(&d) {
        Mapped::Steps(steps) => {
            assert_eq!(steps[0].0, "layer.groupLayers");
            assert_eq!(steps[0].1, json!({"name": "Group"}));
        }
        _ => panic!("an index source imports as the implicit form"),
    }
}

#[test]
fn unsupported_shapes_become_photoshop_steps_not_errors() {
    // Image size in two different percentages, a Paint Bucket click in percent of the canvas, a
    // move in percent, an unknown adjustment class.
    let items = vec![
        item("ImgS", "Image Size", Some(desc("null").with("Wdth", pct(200.0)).with("Hght", pct(150.0)))),
        item(
            "Fl  ",
            "Fill",
            Some(desc("null").with("From", D::Descriptor(desc("Pnt ").with("Hrzn", pct(0.0)).with("Vrtc", pct(0.0)))).with("Usng", enm("FlCn", "FrgC"))),
        ),
        item(
            "move",
            "Move",
            Some(desc("null").with("null", target_ref("Lyr ")).with("T   ", D::Descriptor(desc("Ofst").with("Hrzn", pct(3.0)).with("Vrtc", pct(1.0))))),
        ),
        item(
            "make",
            "Make",
            Some(desc("null").with("null", class_ref("AdjL")).with("Usng", D::Descriptor(desc("AdjL").with("Type", D::Descriptor(desc("GdMp")))))),
        ),
        item("GsnB", "Gaussian Blur", None),
        item("GsnB", "Gaussian Blur", Some(desc("null"))),
        // Duplicate into another open document: the `T` reference means nothing at
        // replay, so the step is kept verbatim instead of misplayed as a same-doc copy.
        item(
            "Dplc",
            "Duplicate",
            Some(
                desc("null")
                    .with("null", target_ref("Lyr "))
                    .with("T   ", D::Reference(vec![ReferenceItem::Name { class: class("Dcmn"), name: UnicodeString::new_nul("Other") }])),
            ),
        ),
    ];
    let file = AtnFile {
        name: UnicodeString::new_nul("Odd"),
        expanded: false,
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        warnings: vec![],
        ..Default::default()
    };
    let imp = import(&atn::write(&file).unwrap()).unwrap();
    let ids: Vec<&str> = imp.actions[0].steps.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, [PHOTOSHOP_STEP, PHOTOSHOP_STEP, PHOTOSHOP_STEP, PHOTOSHOP_STEP, MENU_ITEM, "filter.blur.gaussianBlur", PHOTOSHOP_STEP]);
    assert_eq!(imp.actions[0].steps[4].1["id"], "filter.blur.gaussianBlur");
    assert_eq!(imp.actions[0].steps[4].1["eventKind"], "code");
    assert_eq!(imp.actions[0].steps[5].1, json!({}));
    assert_eq!(imp.unsupported.iter().map(|step| step.step).collect::<Vec<_>>(), [0, 1, 2, 3, 6]);
    assert!(imp.actions[0].disabled.is_empty(), "steps with a document effect stay on so playback reports them");
}

/// #8482: four-byte string events must not become `long` event codes when retained and saved.
#[test]
fn save_preserves_the_kind_of_unsupported_atn_events() {
    let source = AtnFile {
        name: UnicodeString::new_nul("S"),
        expanded: false,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("A"),
            items: vec![
                AtnItem { event: AtnEvent::Name(b"make".to_vec()), dictionary: b"Make".to_vec(), ..Default::default() },
                AtnItem { event: AtnEvent::Code(*b"Qwer"), dictionary: b"Unknown code".to_vec(), ..Default::default() },
            ],
            ..Default::default()
        }],
        warnings: vec![],
        ..Default::default()
    };
    let mut session = Session::new();
    session.execute("actions.load", json!({"data": b64(&atn::write(&source).unwrap())})).unwrap();
    let saved = session.execute("actions.save", json!({"set": "S"})).unwrap();
    let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
    let round_trip = atn::parse(&bytes).unwrap();

    assert_eq!(
        round_trip.actions[0].items.iter().map(|item| item.event.clone()).collect::<Vec<_>>(),
        [AtnEvent::Name(b"make".to_vec()), AtnEvent::Code(*b"Qwer")]
    );

    // Legacy steps may omit eventKind, but an explicitly malformed value must not be treated
    // as if it were absent.
    assert!(export_item(PHOTOSHOP_STEP, &json!({"event": "make", "eventKind": null}), "S").is_none());
    assert!(export_item(PHOTOSHOP_STEP, &json!({"event": "make", "eventKind": 4}), "S").is_none());
    assert_eq!(export_item(PHOTOSHOP_STEP, &json!({"event": "make"}), "S").unwrap().event, AtnEvent::Code(*b"make"));
}

/// #6745: the events of a Photoshop 2026 retouching set (ID-photo background swap, whitening,
/// highlight cleanup, high-pass sharpening), in the stringID form Photoshop wrote them.
#[test]
fn retouching_set_events_map_to_their_commands() {
    let smart_brush = desc("null")
        .with("smartBrushRadius", px(3.0))
        .with("smartBrushSmooth", long(2.0))
        .with("smartBrushFeather", px(0.5))
        .with("smartBrushContrast", pct(10.0))
        .with("smartBrushShiftEdge", pct(-20.0))
        .with("sampleAllLayers", D::Boolean(false))
        .with("smartBrushUseSmartRadius", D::Boolean(true))
        .with("smartBrushDecontaminate", D::Boolean(false))
        .with("smartBrushDeconAmount", pct(100.0))
        .with("refineEdgeOutput", enm("refineEdgeOutput", "refineEdgeOutputUserMask"));
    let selective = desc("null").with("Mthd", enm("CrcM", "Rltv")).with(
        "ClrC",
        D::List(vec![
            D::Descriptor(
                desc("ClrC").with("Clrs", enm("Clrs", "Ylws")).with("Cyn ", pct(-30.0)).with("Mgnt", pct(0.0)).with("Yllw", pct(-40.0)).with("Blck", pct(0.0)),
            ),
            D::Descriptor(
                desc("ClrC").with("Clrs", enm("Clrs", "Whts")).with("Cyn ", pct(0.0)).with("Mgnt", pct(0.0)).with("Yllw", pct(0.0)).with("Blck", pct(-10.0)),
            ),
        ]),
    );
    let lab = |l: f64| D::Descriptor(desc("LbCl").with("Lmnc", D::Double(l)).with("A   ", D::Double(0.0)).with("B   ", D::Double(0.0)));
    let sampled = desc("null").with("Fzns", long(40.0)).with("Mnm ", lab(80.0)).with("Mxm ", lab(95.0)).with("colorModel", long(0.0));
    let mask = desc("null")
        .with("Nw  ", D::Class(class("Chnl")))
        .with("At  ", D::Reference(vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Chnl"), value: code("Msk ") }]))
        .with("Usng", enm("UsrM", "RvlS"));
    let fg = desc("null")
        .with("null", D::Reference(vec![ReferenceItem::Property { class: class("Clr "), key: Id::new("FrgC") }]))
        .with("T   ", D::Descriptor(desc("RGBC").with("Rd  ", D::Double(0.0)).with("Grn ", D::Double(0.0)).with("Bl  ", D::Double(255.0))));
    let lab_fg = desc("null").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Clr "), key: Id::new("BckC") }])).with("T   ", lab(50.0));
    let items = vec![
        item("autoCutout", "Select Subject", Some(desc("null").with("sampleAllLayers", D::Boolean(false)))),
        item("smartBrushWorkspace", "Select and Mask", Some(smart_brush)),
        item("selectiveColor", "Selective Color", Some(selective)),
        item("colorRange", "Color Range", Some(desc("null").with("Clrs", enm("Clrs", "skinTone")).with("Fzns", long(40.0)))),
        item("colorRange", "Color Range", Some(sampled)),
        item("colorRange", "Color Range", Some(desc("null").with("Clrs", enm("Clrs", "Rds ")).with("UsFc", D::Boolean(true)))),
        item("Mk  ", "Make", Some(mask)),
        item("set", "Set", Some(fg)),
        item("set", "Set", Some(lab_fg)),
        item("groupEvent", "Create Clipping Mask", Some(desc("null").with("null", target_ref("Lyr ")))),
        item("GrpL", "Create Clipping Mask", Some(desc("null").with("null", target_ref("Lyr ")))),
        item("groupEvent", "Create Clipping Mask", Some(desc("null").with("null", name_ref("Lyr ", "Shadow")))),
        item("someFutureEvent", "Something New", Some(desc("null").with("Amnt", long(5.0)))),
    ];
    let file = AtnFile {
        name: UnicodeString::new_nul("Retouch"),
        expanded: false,
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        warnings: vec![],
        ..Default::default()
    };
    let imp = import(&atn::write(&file).unwrap()).unwrap();
    let steps = &imp.actions[0].steps;
    let expected = [
        ("select.subject", json!({"sampleAllLayers": false})),
        (
            "select.refineEdge",
            json!({"radius": 3, "smartRadius": true, "smooth": 2, "feather": 0.5, "contrast": 10, "shiftEdge": -20, "decontaminate": false, "amount": 100, "output": "layerMask", "sampleAllLayers": false}),
        ),
        ("image.adjustments.selectiveColor", json!({"method": "relative", "yellows": [-30, 0, -40, 0], "whites": [0, 0, 0, -10]})),
        ("select.colorRange", json!({"select": "skinTones", "fuzziness": 40})),
        (PHOTOSHOP_STEP, Value::Null),
        (PHOTOSHOP_STEP, Value::Null),
        ("layer.layerMask.revealSelection", json!({})),
        ("tools.setColors", json!({"foreground": "#0000ff"})),
        (PHOTOSHOP_STEP, Value::Null),
        ("layer.createClippingMask", json!({})),
        ("layer.createClippingMask", json!({})),
        ("layer.createClippingMask", json!({"layer": "Shadow"})),
        (PHOTOSHOP_STEP, Value::Null),
    ];
    assert_eq!(steps.len(), expected.len(), "{steps:?}");
    for ((id, params), (want_id, want)) in steps.iter().zip(&expected) {
        assert_eq!(id, want_id, "{steps:?}");
        if *want_id != PHOTOSHOP_STEP {
            assert_eq!(params, want, "{id}");
        }
    }
    // Sampled Colors (Lab range), an unknown Color Range key, a Lab colour and an unknown
    // event stay unsupported rather than play with other values.
    let unsupported: Vec<(&str, usize)> = imp.unsupported.iter().map(|u| (u.event.as_str(), u.step)).collect();
    assert_eq!(unsupported, [("colorRange", 4), ("colorRange", 5), ("set", 8), ("someFutureEvent", 12)]);
    // A legacy PhotoCraft export's `Yllw` yellows still read.
    let legacy = desc("null").with("ClrC", D::List(vec![D::Descriptor(desc("ClrC").with("Clrs", enm("Clrs", "Yllw")).with("Cyn ", pct(5.0)))]));
    assert_eq!(selective_from(&legacy), Some(json!({"yellows": [5.0, 0.0, 0.0, 0.0]})));
    // The mask of a named layer is not the target layer's mask: it stays unsupported.
    let other_layer = desc("null")
        .with("Nw  ", D::Class(class("Chnl")))
        .with(
            "At  ",
            D::Reference(vec![
                ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Chnl"), value: code("Msk ") },
                ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul("Other") },
            ]),
        )
        .with("Usng", enm("UsrM", "RvlA"));
    assert!(matches!(make_from(&other_layer), Mapped::Unsupported { harmless: false }));
}

/// Select and Mask exports as the older `refineSelectionEdge` event, which has no Sample All
/// Layers; a step that sets it exports as `smartBrushWorkspace` so the value survives.
#[test]
fn select_and_mask_keeps_sample_all_layers_on_export() {
    let (event, d) = event_and_descriptor("select.refineEdge", &json!({"radius": 3, "output": "layerMask"})).unwrap();
    assert_eq!(event.name, "refineSelectionEdge");
    assert!(d.get("refineEdgeBorderRadius").is_some());
    for sample in [true, false] {
        let params = json!({"radius": 3, "output": "layerMask", "sampleAllLayers": sample});
        let (event, d) = event_and_descriptor("select.refineEdge", &params).unwrap();
        assert_eq!(event.name, "smartBrushWorkspace");
        let Mapped::Steps(steps) = (event.from)(&d) else { panic!("{d:?}") };
        assert_eq!(steps[0].1["sampleAllLayers"], sample);
    }
}

#[test]
fn crop_sides_in_other_units_are_refused_not_played_as_pixels() {
    let rect = |unit: [u8; 4]| {
        let side = |v: f64| D::UnitFloat { unit, value: v };
        desc("null")
            .with("To  ", D::Descriptor(desc("Rctn").with("Top ", side(10.0)).with("Left", side(10.0)).with("Btom", side(90.0)).with("Rght", side(90.0))))
    };
    for unit in [PCT, *b"#Rlt"] {
        assert!(matches!(crop_from(&rect(unit)), Mapped::Unsupported { harmless: false }), "{}", String::from_utf8_lossy(&unit));
    }
    let Mapped::Steps(steps) = crop_from(&rect(PX)) else { panic!("a crop in pixels maps") };
    assert_eq!(steps, [("image.crop".to_string(), json!({"x": 10.0, "y": 10.0, "width": 80.0, "height": 80.0}))]);
}

#[test]
fn step_commands_and_bad_params_fail_gracefully() {
    let mut s = Session::new();
    for p in [
        json!({}),
        json!({"path": ""}),
        json!({"path": "/nonexistent/set.atn"}),
        json!({"data": 42}),
        json!({"data": "%%%"}),
        json!({"data": b64(b"")}),
        json!({"data": b64(&[0, 0, 0, 12, 0, 0, 0, 0])}),
        json!({"data": b64(&atn::write(&sample()).unwrap()[..40])}),
        json!({"data": b64(&atn::write(&sample()).unwrap()), "set": ""}),
        json!({"data": b64(&atn::write(&sample()).unwrap()), "set": 3}),
        json!({"data": b64(&atn::write(&sample()).unwrap()), "replace": "yes"}),
    ] {
        assert!(s.execute("actions.load", p.clone()).is_err(), "{p}");
    }
    assert!(s.actions.list.is_empty());
    for p in [json!({}), json!({"set": "Nope"}), json!({"action": 3}), json!({"path": 7}), json!({"skipUnsupported": "no"}), json!({"set": ""})] {
        assert!(s.execute("actions.save", p.clone()).is_err(), "{p}");
    }
    s.actions.list.push(Action::new("One", vec![("select.all".into(), json!({}))]));
    assert!(s.execute("actions.save", json!({"path": ""})).is_err());
    assert!(s.execute("actions.save", json!({})).unwrap()["data"].is_string());

    assert!(s.execute(STOP_MESSAGE, json!({"message": "halt"})).unwrap_err().to_string().contains("halt"));
    assert_eq!(s.execute(STOP_MESSAGE, json!({"message": "go on", "allowContinue": true})).unwrap()["continued"], true);
    assert!(s.execute(STOP_MESSAGE, json!({"message": 5, "allowContinue": "x"})).is_err());
    for p in [json!({}), json!({"event": 1}), json!({"event": "GsnB", "name": "Gaussian Blur", "descriptor": "not base64"})] {
        assert!(s.execute(PHOTOSHOP_STEP, p.clone()).is_err(), "{p}");
    }
    // A Photoshop step with a damaged descriptor is left out of a save rather than crashing it.
    s.actions.list.push(Action::new("Broken", vec![(PHOTOSHOP_STEP.into(), json!({"event": "GsnB", "name": "Blur", "descriptor": b64(b"\x00\x00\x00")}))]));
    assert!(s.execute("actions.save", json!({"action": "Broken"})).is_err());
    assert_eq!(s.execute("actions.save", json!({"action": "Broken", "skipUnsupported": true})).unwrap()["skipped"].as_array().unwrap().len(), 1);
}

#[test]
fn save_writes_the_file_and_the_set_of_the_selected_action() {
    let mut s = Session::new();
    s.execute("actions.load", json!({"data": b64(&atn::write(&sample()).unwrap())})).unwrap();
    s.actions.list.push(Action::new("Loose", vec![("select.all".into(), json!({}))]));
    let dir = std::env::temp_dir().join(format!("pc-atn-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Saved.atn");
    let r = s.execute("actions.save", json!({"action": "Finish", "path": path.to_string_lossy()})).unwrap();
    assert_eq!((r["set"].as_str(), r["actions"].as_array().map(Vec::len)), (Some("Test Set"), Some(1)));
    let file = atn::parse(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(file.actions.len(), 1);
    assert_eq!(file.actions[0].name.to_string_lossy(), "Finish");
    // Without a set, everything goes out under the default name.
    let r = s.execute("actions.save", json!({})).unwrap();
    assert_eq!((r["set"].as_str(), r["actions"].as_array().map(Vec::len)), (Some(DEFAULT_SET), Some(3)));
    std::fs::remove_dir_all(&dir).ok();
}

/// #7290: saving over an existing ATN must replace it atomically (a temporary file renamed over
/// it), never truncate and rewrite it in place, so a write that fails partway (a full disk)
/// leaves the previously valid file readable. A handle opened on the old file still sees the old
/// bytes after an atomic replacement; an in-place rewrite changes them under it.
#[cfg(unix)]
#[test]
fn save_over_an_existing_file_never_rewrites_it_in_place() {
    use std::io::Read;
    let mut s = Session::new();
    s.actions.list.push(Action::new("Owned", vec![("select.all".into(), json!({}))]));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Owned.atn");
    let p = path.to_string_lossy().to_string();
    s.execute("actions.save", json!({"path": p})).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut old = std::fs::File::open(&path).unwrap();
    s.actions.list.push(Action::new("Second", vec![("select.all".into(), json!({}))]));
    s.execute("actions.save", json!({"path": p})).unwrap();
    let mut seen = Vec::new();
    old.read_to_end(&mut seen).unwrap();
    assert_eq!(seen, before, "the existing ATN was rewritten in place");
    let after = atn::parse(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(after.actions.len(), 2);
    let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names, vec![std::ffi::OsString::from("Owned.atn")], "no temporary file is left behind");
}

#[test]
fn layer_param_accepts_a_name() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
    s.execute("layer.select", json!({"layer": "A"})).unwrap();
    let d = s.active().unwrap();
    assert_eq!(d.doc.layer(d.active_layer.unwrap()).unwrap().name, "A");
    assert!(s.execute("layer.select", json!({"layer": "Nope"})).is_err());
    assert!(s.execute("layer.select", json!({"layer": true})).is_err());
    assert!(s.execute("layer.select", json!({"layer": -1})).is_err());
}

#[test]
fn colours_convert_both_ways() {
    assert_eq!(color_hex(&desc("RGBC").with("Rd  ", D::Double(255.0)).with("Grn ", D::Double(127.6)).with("Bl  ", D::Double(0.0))).as_deref(), Some("#ff8000"));
    assert_eq!(color_hex(&desc("HSBC").with("H   ", ang(0.0)).with("Strt", D::Double(0.0)).with("Brgh", D::Double(100.0))).as_deref(), Some("#ffffff"));
    assert_eq!(color_hex(&desc("HSBC").with("H   ", ang(120.0)).with("Strt", D::Double(100.0)).with("Brgh", D::Double(100.0))).as_deref(), Some("#00ff00"));
    assert_eq!(color_hex(&desc("Grsc").with("Gry ", D::Double(100.0))).as_deref(), Some("#000000"));
    assert_eq!(color_hex(&desc("CMYC")), None);
    assert_eq!(color_desc("#12ab9F").map(|d| color_hex(&d)), Some(Some("#12ab9f".into())));
    assert_eq!(color_desc("12ab9f"), None);
    assert_eq!(color_desc("#12ab9"), None);
    assert_eq!(color_desc("#ééééé"), None);
}

#[test]
fn remaining_events_import_in_their_photoshop_shapes() {
    // Close with the Save answer becomes Save then Close.
    let close_yes = item("Cls ", "Close", Some(desc("null").with("Svng", enm("YsN ", "Ys  "))));
    let close_no = item("close", "Close", Some(desc("null").with("Svng", enm("YsN ", "N   "))));
    // A layer only moved is a translation; scaled, it is the relative Free Transform; the
    // selection's frame goes to Transform Selection; points (not pixels) stay unsupported.
    let moved = desc("null")
        .with("null", target_ref("Lyr "))
        .with("FTcs", enm("QCSt", "Qcsa"))
        .with("Ofst", D::Descriptor(desc("Ofst").with("Hrzn", px(-57.0)).with("Vrtc", px(12.0))))
        .with("Intr", enm("Intp", "Bcbc"));
    let scaled = desc("null")
        .with("null", target_ref("Lyr "))
        .with("FTcs", enm("QCSt", "Qcsa"))
        .with("Ofst", D::Descriptor(desc("Ofst").with("Hrzn", px(10.0)).with("Vrtc", px(0.0))))
        .with("Wdth", pct(50.0))
        .with("Hght", pct(50.0))
        .with("Lnkd", D::Boolean(true))
        .with("Intr", enm("Intp", "Bcbc"));
    let sel = desc("null")
        .with("null", D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }]))
        .with("FTcs", enm("QCSt", "Qcs0"))
        .with("Ofst", D::Descriptor(desc("Ofst").with("Hrzn", px(1014.0)).with("Vrtc", px(0.0))));
    let points = desc("null").with("null", target_ref("Lyr ")).with(
        "Ofst",
        D::Descriptor(desc("Ofst").with("Hrzn", D::UnitFloat { unit: *b"#Rlt", value: 3.0 }).with("Vrtc", D::UnitFloat { unit: *b"#Rlt", value: 1.0 })),
    );
    // Apply Image as recorded by the frequency-separation action.
    let apply = desc("null").with(
        "With",
        D::Descriptor(
            desc("null")
                .with(
                    "T   ",
                    D::Reference(vec![
                        ReferenceItem::Enumerated { class: class("Chnl"), type_id: Id::new("Chnl"), value: Id::new("RGB ") },
                        ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul("High-Frequency") },
                    ]),
                )
                .with("Clcl", enm("Clcn", "Sbtr"))
                .with("Scl ", D::Double(2.0))
                .with("Ofst", D::Double(128.0))
                .with("PrsT", D::Boolean(true)),
        ),
    );
    // Save for Web with and without a destination.
    let web = desc("null")
        .with(
            "Usng",
            D::Descriptor(
                desc("SaveForWeb")
                    .with("Fmt ", enm("IRFm", "PN24"))
                    .with("Intr", D::Boolean(false))
                    .with("Trns", D::Boolean(true))
                    .with("Mtt ", D::Boolean(true))
                    .with("MttR", long(255.0))
                    .with("MttG", long(255.0))
                    .with("MttB", long(255.0))
                    .with("EICC", D::Boolean(false)),
            ),
        )
        .with("In  ", path_value("/Users/me/Desktop/out.png"));
    let web_nowhere = desc("null").with("Usng", D::Descriptor(desc("SaveForWeb").with("Fmt ", enm("IRFm", "PN24"))));
    // Quick Mask on/off, a polygon selection, the mask of the target layer deleted.
    let qm_on = desc("null").with("null", quick_mask_ref());
    let polygon = desc("null").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }])).with(
        "T   ",
        D::Descriptor(desc("Plgn").with(
            "Pts ",
            D::List(vec![
                D::Descriptor(desc("Pnt ").with("Hrzn", px(1.0)).with("Vrtc", px(2.0))),
                D::Descriptor(desc("Pnt ").with("Hrzn", px(30.0)).with("Vrtc", px(2.0))),
                D::Descriptor(desc("Pnt ").with("Hrzn", px(15.0)).with("Vrtc", px(40.0))),
            ]),
        )),
    );
    let mask_delete = desc("null").with("null", target_ref("Chnl"));
    let items = vec![
        item("GsnB", "Gaussian Blur", Some(desc("null").with("Rds ", px(1.0)))),
        close_yes,
        close_no,
        item("transform", "Transform", Some(moved)),
        item("Trnf", "Transform", Some(scaled)),
        item("transform", "Transform", Some(sel)),
        item("transform", "Transform", Some(points)),
        item("applyImageEvent", "Apply Image", Some(apply)),
        item("export", "Export", Some(web)),
        item("export", "Export", Some(web_nowhere)),
        item("setd", "Set", Some(qm_on.clone())),
        item("clearEvent", "Clear", Some(qm_on.clone())),
        item("set", "Set", Some(polygon)),
        item("delete", "Delete", Some(mask_delete)),
        item("newPlacedLayer", "Convert to Smart Object", None),
        item("placedLayerEditContents", "Edit Contents", None),
        item("updatePlacedLayer", "Update Smart Objects", None),
    ];
    let file = AtnFile {
        name: UnicodeString::new_nul("More"),
        expanded: false,
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        warnings: vec![],
        ..Default::default()
    };
    let imp = import(&atn::write(&file).unwrap()).unwrap();
    let steps = &imp.actions[0].steps;
    let ids: Vec<&str> = steps.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "filter.blur.gaussianBlur",
            "file.save",
            "file.close",
            "file.close",
            "layer.translate",
            "edit.transform",
            "select.transformSelection",
            PHOTOSHOP_STEP,
            "image.applyImage",
            "file.export.saveForWebLegacy",
            PHOTOSHOP_STEP,
            "select.editInQuickMaskMode",
            "select.editInQuickMaskMode",
            "select.lasso",
            "layer.layerMask.delete",
            "layer.smartObjects.convertToSmartObject",
            "layer.smartObjects.editContents",
            "layer.smartObjects.updateModifiedContent",
        ]
    );
    assert_eq!(steps[4].1, json!({"dx": -57, "dy": 12}));
    assert_eq!(steps[5].1, json!({"dx": 10, "dy": 0, "scaleX": 50, "scaleY": 50, "interpolation": "bicubic"}));
    assert_eq!(steps[6].1, json!({"dx": 1014, "dy": 0}));
    assert_eq!(
        steps[8].1,
        json!({"source": {"channel": "composite", "layer": "High-Frequency"}, "blending": "subtract", "scale": 2, "offset": 128, "preserveTransparency": true})
    );
    assert_eq!(
        steps[9].1,
        json!({"path": "/Users/me/Desktop/out.png", "format": "png24", "interlaced": false, "transparency": true, "embedIcc": false, "matte": "#ffffff"})
    );
    assert_eq!(steps[11].1, json!({"on": true}));
    assert_eq!(steps[12].1, json!({"on": false}));
    assert_eq!(steps[13].1, json!({"points": [[1, 2], [30, 2], [15, 40]]}));
    assert_eq!(imp.unsupported.len(), 2);

    // The frequency-separation set's own conditional, a percent Image Size, a Paint Bucket
    // click in pixels (percent stays unsupported) and a Place.
    let cond = desc("null")
        .with("null", enm("Cndt", "Lyrs"))
        .with(
            "then",
            D::Reference(vec![
                ReferenceItem::Name { class: class("Actn"), name: UnicodeString::new_nul("FS8-Layers") },
                ReferenceItem::Name { class: class("ASet"), name: UnicodeString::new_nul("FS") },
            ]),
        )
        .with("else", D::Reference(vec![ReferenceItem::Name { class: class("Actn"), name: UnicodeString::new_nul("FS8-No-Layers") }]));
    let size = desc("null")
        .with("Wdth", pct(200.0))
        .with("scaleStyles", D::Boolean(true))
        .with("CnsP", D::Boolean(true))
        .with("Intr", enm("Intp", "bicubicAutomatic"));
    let bucket_px = desc("null")
        .with("From", D::Descriptor(desc("Pnt ").with("Hrzn", px(0.0)).with("Vrtc", px(0.0))))
        .with("Tlrn", long(32.0))
        .with("Usng", enm("FlCn", "FrgC"))
        .with("Cntg", D::Boolean(false));
    let bucket_pct = desc("null")
        .with("From", D::Descriptor(desc("Pnt ").with("Hrzn", pct(0.0)).with("Vrtc", pct(0.0))))
        .with("Tlrn", long(32.0))
        .with("Usng", enm("FlCn", "FrgC"));
    let place = desc("null")
        .with("null", path_value("/Users/me/logo.png"))
        .with("FTcs", enm("QCSt", "Qcsa"))
        .with("Wdth", pct(50.0))
        .with("Hght", pct(50.0))
        .with("Lnkd", D::Boolean(false));
    let items = vec![
        item("conditional", "If", Some(cond)),
        item("imageSize", "Image Size", Some(size)),
        item("Fl  ", "Fill", Some(bucket_px)),
        item("Fl  ", "Fill", Some(bucket_pct)),
        item("placeEvent", "Place", Some(place)),
        item("Opn ", "Open", Some(desc("null").with("null", path_value("/Users/me/in.psd")))),
    ];
    let file = AtnFile {
        name: UnicodeString::new_nul("Rest"),
        expanded: false,
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        warnings: vec![],
        ..Default::default()
    };
    let rest = import(&atn::write(&file).unwrap()).unwrap();
    let rest_steps = &rest.actions[0].steps;
    let rest_ids: Vec<&str> = rest_steps.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(rest_ids, ["actions.conditional", "image.imageSize", "paint.bucket", PHOTOSHOP_STEP, "file.placeEmbedded", "file.openAs"]);
    // A transform without a reference (a replaced smart object) is the target layer's; the
    // selection outline nudged; a layer moved to index 0 (the bottom); a View menu item; the
    // foreground colour.
    let replaced = desc("null")
        .with("FTcs", enm("QCSt", "Qcsa"))
        .with("Ofst", D::Descriptor(desc("Ofst").with("Hrzn", px(4.0)).with("Vrtc", px(6.0))))
        .with("Wdth", pct(50.0))
        .with("Hght", pct(50.0))
        .with("replaceLayer", D::Descriptor(desc("null")));
    let nudged = desc("null")
        .with("null", D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }]))
        .with("T   ", D::Descriptor(desc("Ofst").with("Hrzn", px(3.0)).with("Vrtc", px(-2.0))));
    let to_bottom = desc("null")
        .with("null", target_ref("Lyr "))
        .with("T   ", D::Reference(vec![ReferenceItem::Index { class: class("Lyr "), index: 0 }]))
        .with("Adjs", D::Boolean(false));
    let fit =
        desc("null").with("null", D::Reference(vec![ReferenceItem::Enumerated { class: class("Mn  "), type_id: Id::new("MnIt"), value: Id::new("FtOn") }]));
    let fg = desc("null")
        .with("null", D::Reference(vec![ReferenceItem::Property { class: class("Clr "), key: Id::new("FrgC") }]))
        .with("T   ", D::Descriptor(desc("RGBC").with("Rd  ", D::Double(0.0)).with("Grn ", D::Double(0.0)).with("Bl  ", D::Double(0.0))));
    let items = vec![
        item("transform", "Transform", Some(replaced)),
        item("move", "Move", Some(nudged)),
        item("move", "Move", Some(to_bottom)),
        item("select", "Select", Some(fit)),
        item("set", "Set", Some(fg)),
    ];
    let file = AtnFile {
        name: UnicodeString::new_nul("More2"),
        expanded: false,
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        warnings: vec![],
        ..Default::default()
    };
    let more = import(&atn::write(&file).unwrap()).unwrap();
    let more_steps = &more.actions[0].steps;
    assert_eq!(
        more_steps.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
        ["edit.transform", "select.transformSelection", "layer.moveTo", "view.fitOnScreen", "tools.setColors"]
    );
    assert_eq!(more_steps[0].1, json!({"dx": 4, "dy": 6, "scaleX": 50, "scaleY": 50}));
    assert_eq!(more_steps[1].1, json!({"dx": 3, "dy": -2}));
    assert_eq!(more_steps[2].1, json!({"target": {"index": 0}}));
    assert_eq!(more_steps[4].1, json!({"foreground": "#000000"}));
    assert!(more.unsupported.is_empty());
    assert_eq!(rest_steps[0].1, json!({"if": "hasLayers", "then": "FS8-Layers", "else": "FS8-No-Layers"}));
    assert_eq!(rest_steps[1].1, json!({"percent": 200, "resample": "bicubicAutomatic", "scaleStyles": true}));
    assert_eq!(rest_steps[2].1, json!({"x": 0, "y": 0, "tolerance": 32, "contiguous": false}));
    assert_eq!(rest_steps[4].1, json!({"path": "/Users/me/logo.png", "scale": 50}));
    assert_eq!(rest_steps[5].1, json!({"path": "/Users/me/in.psd"}));

    // The Photoshop path record round-trips and rejects other shapes.
    assert_eq!(path_text(&path_value("/a/b c.png")).as_deref(), Some("/a/b c.png"));
    assert_eq!(path_text(&D::Path(b"txtu".to_vec())), None);
    assert_eq!(path_text(&D::Path(vec![])), None);
    assert_eq!(path_text(&D::Text(UnicodeString::new("x"))), None);
    assert_eq!(path_text(&path_value("")), None);

    // Apply Image by layer name runs on a document; a missing name is a clear error.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "High-Frequency"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s.execute("image.applyImage", steps[8].1.clone()).unwrap();
    let e = s.execute("image.applyImage", json!({"source": {"layer": "Nope"}})).unwrap_err().to_string();
    assert!(e.contains("Nope"), "{e}");
}

fn channel_ord(value: &str) -> D {
    D::Reference(vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: Id::new("Chnl"), value: code(value) }])
}

fn fsel(t: D) -> Descriptor {
    desc("null").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }])).with("T   ", t)
}

/// #7103: Photoshop records switching to another open document as a `select` of `Dcmn` by
/// offset (previous -1, next +1). The template action of the report, which copies from the
/// source image and pastes into the template opened after it, imports with no Photoshop steps,
/// plays across the two documents and saves back the same shape. With no previous document
/// open, playback stops at the switch instead of pasting into the wrong document.
#[test]
fn select_document_by_offset_imports_plays_and_saves_back() {
    let doc = |offset: i32| desc("null").with("null", D::Reference(vec![ReferenceItem::Offset { class: class("Dcmn"), offset }]));
    // A recorded document id (`DocI`) means nothing in another session: ignored, not written back.
    let copy_in = vec![
        item("slct", "Select", Some(doc(-1).with("DocI", long(35.0)))),
        item("setd", "Set", Some(fsel(enm("Ordn", "Al  ")))),
        item("copy", "Copy", None),
        item("slct", "Select", Some(doc(1))),
        item("past", "Paste", None),
    ];
    let data = set_of("Template", vec![("Copy In", copy_in)]);

    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "name": "source"})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8, "name": "template", "background": "transparent"})).unwrap();
    // Reorder tabs before playback: the source remains the previous *opened* document.
    s.execute("document.move", json!({"document": 1, "to": 0})).unwrap();
    let r = s.execute("actions.load", json!({"data": data})).unwrap();
    assert_eq!(r["unsupported"], json!([]), "{r}");
    let steps = &s.actions.list[0].steps;
    assert_eq!(steps[0], ("document.activate".to_string(), json!({"offset": -1})));
    assert_eq!(steps[3], ("document.activate".to_string(), json!({"offset": 1})));

    let pixel = |s: &mut Session| -> Vec<f32> { serde_json::from_value(s.execute("document.pixel", json!({"x": 4, "y": 4})).unwrap()).unwrap() };
    assert_eq!(pixel(&mut s), [0.0; 4]);
    let r = s.execute("actions.play", json!({"action": "Copy In"})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    assert_eq!(s.active().unwrap().doc.name, "template");
    assert_eq!(s.active().unwrap().doc.layers.len(), 2, "the source's pixels are pasted on a new layer");
    assert_eq!(pixel(&mut s), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(s.documents()[1].doc.layers.len(), 1, "the source is only copied from");

    let first = s.actions.list.clone();
    let r = s.execute("actions.save", json!({"set": "Template"})).unwrap();
    assert_eq!(r["skipped"], json!([]));
    let saved = photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap();
    let file = atn::parse(&saved).unwrap();
    let offsets: Vec<i32> = file.actions[0]
        .items
        .iter()
        .filter_map(|i| match get_ref(i.descriptor.as_ref()?, "null")? {
            [ReferenceItem::Offset { class, offset }] if trimmed(&class.class_id) == "Dcmn" => Some(*offset),
            _ => None,
        })
        .collect();
    assert_eq!(offsets, [-1, 1]);
    assert!(file.actions[0].items[0].descriptor.as_ref().unwrap().get("DocI").is_none());
    let mut t = Session::new();
    t.execute("actions.load", json!({"data": b64(&saved)})).unwrap();
    assert_eq!(t.actions.list, first);
    // An absolute tab index has no Photoshop equivalent: it stays out of a strict save.
    assert!(export_item("document.activate", &json!({"document": 0}), "S").is_none());
    assert!(export_item("document.activate", &json!({"document": 0, "offset": 1}), "S").is_none());

    let mut one = Session::new();
    one.execute("file.new", json!({"width": 8, "height": 8, "name": "template"})).unwrap();
    one.execute("actions.load", json!({"data": data})).unwrap();
    let r = one.execute("actions.play", json!({"action": "Copy In"})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    assert_eq!(one.active().unwrap().doc.layers.len(), 2, "offsets wrap back to the only open document");
}

fn set_of(name: &str, actions: Vec<(&str, Vec<AtnItem>)>) -> String {
    let file = AtnFile {
        name: UnicodeString::new_nul(name),
        expanded: false,
        actions: actions.into_iter().map(|(n, items)| AtnAction { name: UnicodeString::new_nul(n), items, ..Default::default() }).collect(),
        warnings: vec![],
        ..Default::default()
    };
    b64(&atn::write(&file).unwrap())
}

/// The step shapes of the TrueGrit Texture Supply sets (Distress Press, Debaser, Transparency,
/// Halftone Zine Machine), rebuilt here as Photoshop records them, import and play: the
/// distressing filters land on the targeted layer mask, Remove White leaves the white
/// transparent, nudges in points follow the resolution, and Step Backward undoes a step.
#[test]
fn distressing_and_remove_white_shapes_import_and_play() {
    let ord = |cls: &str, ty: &str, v: &str| D::Reference(vec![ReferenceItem::Enumerated { class: class(cls), type_id: Id::new(ty), value: code(v) }]);
    let black = D::Descriptor(desc("RGBC").with("Rd  ", D::Double(0.0)).with("Grn ", D::Double(0.0)).with("Bl  ", D::Double(0.0)));
    let fill = desc("null")
        .with("null", class_ref("contentLayer"))
        .with("Usng", D::Descriptor(desc("contentLayer").with("Type", D::Descriptor(desc("solidColorLayer").with("Clr ", black)))));
    let refine = desc("null")
        .with("refineEdgeBorderRadius", px(0.0))
        .with("refineEdgeBorderContrast", pct(100.0))
        .with("refineEdgeSmooth", long(2.0))
        .with("refineEdgeFeatherRadius", px(0.0))
        .with("refineEdgeChoke", pct(0.0))
        .with("refineEdgeAutoRadius", D::Boolean(false))
        .with("refineEdgeDecontaminate", D::Boolean(false))
        .with("refineEdgeOutput", enm("refineEdgeOutput", "refineEdgeOutputUserMask"));
    let distress = vec![
        item("setd", "Set", Some(fsel(channel_ord("Trsp")))),
        item("Hd  ", "Hide", Some(desc("null").with("null", D::List(vec![target_ref("Lyr ")])))),
        item("Mk  ", "Make", Some(fill)),
        item("slct", "Select", Some(desc("null").with("null", channel_ord("Msk")).with("MkVs", D::Boolean(false)))),
        item("refineSelectionEdge", "Select and Mask", Some(refine)),
        item("Rple", "Ripple", Some(desc("null").with("Amnt", long(50.0)).with("RplS", enm("RplS", "Sml")))),
        item("Crst", "Crystallize", Some(desc("null").with("ClSz", long(3.0)))),
        item("GEfc", "Filter Gallery", Some(desc("null").with("GEfk", enm("GEft", "Spt")).with("SprR", long(7.0)).with("Smth", long(15.0)))),
        item("slct", "Select", Some(desc("null").with("null", ord("Chnl", "Ordn", "Trgt")).with("MkVs", D::Boolean(false)))),
        item("setd", "Set", Some(fsel(ord("Chnl", "Ordn", "Trgt")))),
    ];
    let bckg = D::Reference(vec![ReferenceItem::Property { class: class("Lyr "), key: Id::new("Bckg") }]);
    let remove_white = vec![
        item("CnvM", "Convert Mode", Some(desc("null").with("T   ", D::Class(class("Grys"))))),
        item(
            "setd",
            "Set",
            Some(desc("null").with("null", bckg).with("T   ", D::Descriptor(desc("Lyr ").with("Opct", pct(100.0)).with("Md  ", enm("BlnM", "Nrml"))))),
        ),
        item("setd", "Set", Some(fsel(ord("Chnl", "Ordn", "Trgt")))),
        item("Dlt ", "Delete", None),
        item("setd", "Set", Some(fsel(enm("Ordn", "None")))),
    ];
    let pt = |v: f64| D::UnitFloat { unit: PT, value: v };
    let nudge = vec![item(
        "move",
        "Move",
        Some(desc("null").with("null", target_ref("Lyr ")).with("T   ", D::Descriptor(desc("Ofst").with("Hrzn", pt(0.36)).with("Vrtc", pt(-0.36))))),
    )];
    let undo = vec![item("Invs", "Inverse", None), item("slct", "Select", Some(desc("null").with("null", ord("HstS", "Ordn", "Prvs"))))];
    let data = set_of("TrueGrit", vec![("Distress", distress), ("Remove White", remove_white), ("Nudge", nudge), ("Undo", undo)]);

    let mut s = Session::new();
    s.execute("file.new", json!({"width": 120, "height": 80, "resolution": 200})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 20, "width": 80, "height": 40})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let r = s.execute("actions.load", json!({"data": data})).unwrap();
    // Only the re-selection of the targeted channel stays a Photoshop step, switched off.
    let unsupported: Vec<&str> = r["unsupported"].as_array().unwrap().iter().map(|u| u["action"].as_str().unwrap()).collect();
    assert_eq!(unsupported, ["Distress"], "{r}");

    let r = s.execute("actions.play", json!({"action": "Distress"})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    let st = s.active().unwrap();
    let fill = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let mask = &fill.mask.as_ref().expect("the fill layer is masked by the ink").surface;
    // The rectangle's straight top edge is roughened: pixels along it differ.
    let edge: Vec<f32> = (20..100).map(|x| mask.sample_channel(x, 20, 0)).collect();
    assert!(edge.iter().any(|v| *v < 0.5) && edge.iter().any(|v| *v > 0.5), "{edge:?}");
    assert!(mask.sample_channel(60, 40, 0) > 0.9 && mask.sample_channel(5, 5, 0) < 0.1);
    assert!(st.doc.selection.is_some(), "the targeted mask loads as the selection");
    assert_eq!(st.channel_view.target, crate::channel_cmds::ChannelTarget::LayerMask);

    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#202020"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("actions.load", json!({"data": data})).unwrap();
    let r = s.execute("actions.play", json!({"action": "Remove White"})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    let d = &s.active().unwrap().doc;
    assert_eq!(d.mode, photocraft_color::ColorMode::Grayscale);
    let l = d.layers.last().unwrap();
    assert!(!d.is_background(l.id), "Layer From Background");
    let surf = l.surface().unwrap();
    assert!(surf.sample_channel(2, 2, 1) < 0.01, "white is cleared");
    // #202020 is about 22 in the default (Dot Gain 20 %-compatible) working gray.
    let ink = surf.sample_channel(20, 15, 1);
    assert!((ink - (1.0 - 22.0 / 255.0)).abs() < 0.02, "the ink keeps its darkness as opacity: {ink}");
    // An image that opened as a normal layer has no Background: the step leaves it as it is.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "transparent"})).unwrap();
    s.execute("edit.fill", json!({"contents": "white"})).unwrap();
    s.execute("actions.load", json!({"data": data})).unwrap();
    let r = s.execute("actions.play", json!({"action": "Remove White"})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    let surf = s.active().unwrap().doc.layers.last().unwrap().surface().unwrap().clone();
    assert!(surf.sample_channel(2, 2, 1) < 0.01, "white is cleared without a Background");

    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "resolution": 200})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 5, "height": 5})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("actions.load", json!({"data": data})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    assert!(s.execute("actions.play", json!({"action": "Nudge"})).unwrap().get("failed").is_none());
    let st = s.active().unwrap();
    let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert_eq!((b.x0, b.y0), (11, 9), "0.36 pt is 1 px at 200 ppi");
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    let before = s.active().unwrap().doc.selection.clone();
    assert!(s.execute("actions.play", json!({"action": "Undo"})).unwrap().get("failed").is_none());
    assert_eq!(s.active().unwrap().doc.selection, before, "Step Backward undid the Inverse");
}

/// Remove White on art in its own layer above a white Background: the Background becomes a
/// normal layer, but the art layer stays targeted, so the white is cleared from the art.
#[test]
fn remove_white_clears_the_targeted_layer_not_the_converted_background() {
    let ord = |cls: &str, ty: &str, v: &str| D::Reference(vec![ReferenceItem::Enumerated { class: class(cls), type_id: Id::new(ty), value: code(v) }]);
    let bckg = D::Reference(vec![ReferenceItem::Property { class: class("Lyr "), key: Id::new("Bckg") }]);
    let steps = vec![
        item("CnvM", "Convert Mode", Some(desc("null").with("T   ", D::Class(class("Grys"))))),
        item(
            "setd",
            "Set",
            Some(desc("null").with("null", bckg).with("T   ", D::Descriptor(desc("Lyr ").with("Opct", pct(100.0)).with("Md  ", enm("BlnM", "Nrml"))))),
        ),
        item("setd", "Set", Some(fsel(ord("Chnl", "Ordn", "Trgt")))),
        item("Dlt ", "Delete", None),
        item("setd", "Set", Some(fsel(enm("Ordn", "None")))),
    ];
    let data = set_of("TrueGrit", vec![("Remove White", steps)]);
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    let art = s.execute("layer.new.layer", json!({"name": "Art"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("edit.fill", json!({"contents": "white"})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("actions.load", json!({"data": data})).unwrap();
    let r = s.execute("actions.play", json!({"action": "Remove White"})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    let st = s.active().unwrap();
    assert_eq!(st.active_layer.map(|l| l.0), Some(art), "the art layer stays targeted");
    let art = st.doc.layer(photocraft_doc::LayerId(art)).unwrap().surface().unwrap();
    assert!(art.sample_channel(2, 2, 1) < 0.01, "the art's white is cleared");
    assert!(art.sample_channel(20, 15, 1) > 0.99, "the black stays");
    let bottom = st.doc.layers.first().unwrap();
    assert!(!st.doc.is_background(bottom.id), "the Background became a normal layer");
    assert!(bottom.surface().unwrap().sample_channel(2, 2, 1) > 0.99, "and kept its pixels");
}

/// Measured in Photoshop: `executeAction` fails for the codes `BoxB`, `SrfB`, `Fltn`, `BlnW`
/// and `PhtF` ("the command is not currently available"), while the names `boxblur`,
/// `surfaceBlur`, `flattenImage`, `blackAndWhite`, `photoFilter` and the code `FltI` run.
#[test]
fn events_photoshop_only_runs_by_name_export_by_name_and_old_codes_still_import() {
    let bw = json!({"reds": 40, "yellows": 60, "greens": 40, "cyans": 60, "blues": 20, "magentas": 80, "tint": false, "tintColor": "#e4d5b0"});
    let photo_filter = json!({"color": "#0022cd", "density": 67, "preserveLuminosity": false});
    let cases = [
        ("filter.blur.boxBlur", json!({"radius": 3}), "boxblur", "BoxB"),
        ("filter.blur.surfaceBlur", json!({"radius": 5, "threshold": 15}), "surfaceBlur", "SrfB"),
        ("layer.flattenImage", json!({}), "FltI", "Fltn"),
        ("image.adjustments.blackWhite", bw, "blackAndWhite", "BlnW"),
        ("image.adjustments.photoFilter", photo_filter, "photoFilter", "PhtF"),
    ];
    for (id, params, written, legacy) in &cases {
        let exported = export_item(id, params, "S").unwrap_or_else(|| panic!("{id} does not export"));
        assert_eq!(exported.event.id(), *written, "{id} exports under an id Photoshop runs");
        assert_eq!(matches!(exported.event, AtnEvent::Name(_)), written.len() != 4, "{id}: a name is written as a name");
        let d = exported.descriptor.clone();
        // The written id, the name for Flatten Image, and the old code all import to the command.
        let name = if *id == "layer.flattenImage" { "flattenImage" } else { written };
        for event in [*written, name, *legacy] {
            let e = event_for(&item(event, "", None).event).unwrap_or_else(|| panic!("{event} does not import"));
            assert!(e.ids.contains(id), "{event} imports as {:?}, not {id}", e.ids);
            match (e.from)(d.as_ref().unwrap()) {
                Mapped::Steps(steps) => {
                    assert_eq!(steps.len(), 1, "{event}: {steps:?}");
                    assert_eq!(steps[0].0, *id, "{event}");
                    let mut back = steps[0].1.clone();
                    normalise(&mut back);
                    assert_eq!(back, *params, "{event}");
                }
                Mapped::Unsupported { .. } => panic!("{event} does not import back"),
            }
        }
    }

    // A whole set of Photoshop-shaped steps (names, FltI) and old PhotoCraft-shaped ones
    // (legacy codes) loads to the same commands.
    let blur = |event: &str| item(event, "Box Blur", Some(desc("null").with("Rds ", px(3.0))));
    let surface = |event: &str| item(event, "Surface Blur", Some(desc("null").with("Rds ", px(5.0)).with("Thsh", long(15.0))));
    let photoshop = vec![blur("boxblur"), surface("surfaceBlur"), item("FltI", "Flatten Image", None), item("flattenImage", "Flatten Image", None)];
    let legacy = vec![blur("BoxB"), surface("SrfB"), item("Fltn", "Flatten Image", None)];
    let data = set_of("S", vec![("Photoshop", photoshop), ("Legacy", legacy)]);
    let mut s = Session::new();
    let r = s.execute("actions.load", json!({"data": data})).unwrap();
    assert_eq!(r["unsupported"], json!([]), "{r}");
    let ids = |i: usize| s.actions.list[i].steps.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(0), ["filter.blur.boxBlur", "filter.blur.surfaceBlur", "layer.flattenImage", "layer.flattenImage"]);
    assert_eq!(ids(1), ["filter.blur.boxBlur", "filter.blur.surfaceBlur", "layer.flattenImage"]);
    assert_eq!(s.actions.list[1].steps[1].1, json!({"radius": 5, "threshold": 15}));
    // Saving them writes the ids Photoshop runs, never the old codes.
    let saved = s.execute("actions.save", json!({"set": "S"})).unwrap();
    let file = atn::parse(&photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap()).unwrap();
    let events: Vec<String> = file.actions.iter().flat_map(|a| a.items.iter().map(|i| i.event.id())).collect();
    assert_eq!(events, ["boxblur", "surfaceBlur", "FltI", "FltI", "boxblur", "surfaceBlur", "FltI"]);

    // Unknown ids, near misses and the legacy codes written as names stay Photoshop steps,
    // never a panic or a wrong command.
    for event in ["BoxBlur", "boxBlur", "BOXB", "Fltn ", "FLTI", "", "\u{0}\u{0}\u{0}\u{0}", "BlnWx", "PHTF", "PhtF ", "photofilter"] {
        let e = event_for(&item(event, "", None).event);
        assert!(e.is_none(), "{event:?} maps to {:?}", e.map(|e| e.ids));
    }
    // A legacy code with a broken descriptor is unsupported, not a crash.
    let e = event_for(&item("SrfB", "", None).event).unwrap();
    assert!(matches!((e.from)(&desc("null").with("Rds ", D::Double(f64::NAN))), Mapped::Unsupported { .. } | Mapped::Steps(_)));
    // A wrong-typed key is skipped like any other unreadable field: the step keeps the defaults.
    match (e.from)(&desc("null").with("Rds ", text("x"))) {
        Mapped::Steps(steps) => assert!(steps.iter().all(|(_, p)| p.get("radius").is_none()), "{steps:?}"),
        Mapped::Unsupported { .. } => {}
    }
}

/// #8301: Photoshop stops on a Photo Filter step saved as `PhtF` ("not currently available")
/// and runs the same descriptor under the name `photoFilter`. A recorded step saves by name,
/// a Photoshop-authored step re-saves by name, and actions saved before with `PhtF` still load
/// and play the same.
#[test]
fn photo_filter_saves_under_the_name_photoshop_runs() {
    let params = json!({"color": "#0022cd", "density": 67, "preserveLuminosity": false});
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "#808080"})).unwrap();
    s.execute("actions.record", json!({"name": "Filter"})).unwrap();
    s.execute("image.adjustments.photoFilter", params.clone()).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let saved = s.execute("actions.save", json!({"action": "Filter"})).unwrap();
    assert_eq!(saved["skipped"], json!([]), "{saved}");
    let file = atn::parse(&photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap()).unwrap();
    let step = &file.actions[0].items[0];
    assert_eq!(step.event, AtnEvent::Name("photoFilter".into()), "never the code PhtF");
    assert_eq!(step.dictionary, "Photo Filter".as_bytes());
    let d = step.descriptor.clone().unwrap();
    for key in ["Clr ", "Dnst", "PrsL"] {
        assert!(d.get(key).is_some(), "the descriptor keeps {key}: {d:?}");
    }

    // A Photoshop-authored step (by name) and an old PhotoCraft one (`PhtF`) both load to the
    // command, and saving them again writes the name.
    let data =
        set_of("S", vec![("Photoshop", vec![item("photoFilter", "Photo Filter", Some(d.clone()))]), ("Legacy", vec![item("PhtF", "Photo Filter", Some(d))])]);
    let mut t = Session::new();
    let r = t.execute("actions.load", json!({"data": data})).unwrap();
    assert_eq!(r["unsupported"], json!([]), "{r}");
    for a in &t.actions.list {
        assert_eq!(a.steps.len(), 1, "{}", a.name);
        assert_eq!(a.steps[0].0, "image.adjustments.photoFilter", "{}", a.name);
        let mut p = a.steps[0].1.clone();
        normalise(&mut p);
        assert_eq!(p, params, "{}", a.name);
    }
    let resaved = t.execute("actions.save", json!({"set": "S"})).unwrap();
    assert_eq!(resaved["skipped"], json!([]), "{resaved}");
    let file = atn::parse(&photocraft_paint::tile::b64_decode(resaved["data"].as_str().unwrap()).unwrap()).unwrap();
    let events: Vec<String> = file.actions.iter().flat_map(|a| a.items.iter().map(|i| i.event.id())).collect();
    assert_eq!(events, ["photoFilter", "photoFilter"]);

    // Both play like the command run directly, at 8 and 16 bits.
    for depth in [8, 16] {
        let pixel = |s: &Session| {
            let st = s.active().unwrap();
            let surface = st.doc.layers.first().unwrap().surface().unwrap();
            [0, 1, 2].map(|c| surface.sample_channel(3, 3, c))
        };
        let new_doc = |t: &mut Session| {
            t.execute("file.new", json!({"width": 8, "height": 8, "background": "#808080", "depth": depth})).unwrap();
        };
        new_doc(&mut t);
        t.execute("image.adjustments.photoFilter", params.clone()).unwrap();
        let direct = pixel(&t);
        assert_ne!(direct, [128.0 / 255.0; 3], "the filter changes the grey");
        for action in ["Photoshop", "Legacy"] {
            new_doc(&mut t);
            let r = t.execute("actions.play", json!({"action": action})).unwrap();
            assert!(r.get("failed").is_none(), "{action}: {r}");
            assert_eq!(pixel(&t), direct, "{action} at {depth} bits");
        }
    }
}

/// A Black & White adjustment layer saves with Photoshop's `BanW` class (#8481): measured in
/// Photoshop, a Make whose type is `BlnW` is refused. Photoshop's own `BanW` Make loads, and so
/// does the `BlnW` one PhotoCraft wrote before.
#[test]
fn black_white_adjustment_layer_saves_as_photoshops_class_and_old_class_still_imports() {
    let id = "layer.newAdjustmentLayer.blackWhite";
    let params = json!({"reds": 17, "yellows": 83, "greens": -12, "cyans": 107, "blues": 51, "magentas": 133, "tint": true, "tintColor": "#c08040"});
    let mut s = Session::new();
    s.actions.list.push(Action::new("BW", vec![(id.into(), params.clone())]));
    let r = s.execute("actions.save", json!({"action": "BW"})).unwrap();
    assert_eq!(r["skipped"], json!([]));
    let saved = photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap();
    let file = atn::parse(&saved).unwrap();
    let make = file.actions[0].items[0].descriptor.as_ref().unwrap();
    let kind = get_obj(make, "Usng").and_then(|u| get_obj(u, "Type")).unwrap();
    assert_eq!(trimmed(&kind.class_id), "BanW");
    let mut t = Session::new();
    let r = t.execute("actions.load", json!({"data": b64(&saved)})).unwrap();
    assert_eq!(r["unsupported"], json!([]), "{r}");
    assert_eq!(t.actions.list[0].steps, vec![(id.to_string(), params)]);

    // Photoshop's minimal Make (no settings) and an old PhotoCraft Make with a setting both load,
    // the setting kept; near misses stay Photoshop steps.
    let make = |class: &str| {
        let kind = if class == "BlnW" { desc(class).with("Rd  ", long(17.0)) } else { desc(class) };
        let using = desc("AdjL").with("Type", D::Descriptor(kind));
        item("Mk  ", "Make", Some(desc("null").with("null", class_ref("AdjL")).with("Usng", D::Descriptor(using))))
    };
    let data = set_of("S", vec![("A", ["BanW", "BlnW", "BANW", "BlnWx"].into_iter().map(make).collect())]);
    let mut t = Session::new();
    let r = t.execute("actions.load", json!({"data": data})).unwrap();
    let steps: Vec<&str> = t.actions.list[0].steps.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(steps, [id, id, PHOTOSHOP_STEP, PHOTOSHOP_STEP], "{r}");
    assert_eq!(t.actions.list[0].steps[1].1["reds"], 17, "{r}");
}

/// Smart Blur's quality uses Photoshop's `SmBQ` codes (`SBQL`/`SBQM`/`SBQH`): measured in
/// Photoshop, the old `Hgh `/`Mdm `/`Low ` play back as its fallback quality. Old codes still read.
#[test]
fn smart_blur_quality_uses_photoshop_codes() {
    let smart_blur =
        |quality: D, mode: &str| desc("null").with("Rds ", D::Double(6.0)).with("Thsh", D::Double(60.0)).with("SmBQ", quality).with("SmBM", enm("SmBM", mode));
    // Every quality writes Photoshop's code, as a double radius/threshold and both enums,
    // and reads back as the same step.
    for (quality, code) in [("low", "SBQL"), ("medium", "SBQM"), ("high", "SBQH")] {
        let params = json!({"radius": 6, "threshold": 60, "quality": quality, "mode": "edgeOnly"});
        let a = Action::new("Q", vec![("filter.blur.smartBlur".into(), params.clone())]);
        let (bytes, skipped) = export("S", &[&a], &[], false).unwrap();
        assert!(skipped.is_empty());
        let file = atn::parse(&bytes).unwrap();
        let it = &file.actions[0].items[0];
        assert_eq!(it.event.id(), "SmrB");
        assert_eq!(it.descriptor.as_ref(), Some(&smart_blur(enm("SmBQ", code), "SBME")), "{quality}");
        let back = import(&bytes).unwrap();
        assert!(back.unsupported.is_empty(), "{quality}");
        assert_eq!(back.actions[0].steps, vec![("filter.blur.smartBlur".to_string(), params)]);
    }

    // Photoshop-shaped steps (each quality and mode, as Photoshop stores them) and the codes
    // PhotoCraft wrote before this fix read as the right quality.
    let cases = [
        ("SBQH", "SBMN", "high", "normal"),
        ("SBQM", "SBMO", "medium", "overlayEdge"),
        ("SBQL", "SBME", "low", "edgeOnly"),
        ("Hgh ", "SBMN", "high", "normal"),
        ("Mdm ", "SBMN", "medium", "normal"),
        ("Low ", "SBMN", "low", "normal"),
    ];
    let items = cases.iter().map(|(q, m, _, _)| item("SmrB", "Smart Blur", Some(smart_blur(enm("SmBQ", q), m)))).collect();
    let data = set_of("Blurs", vec![("Smart", items)]);
    let mut s = Session::new();
    s.execute("actions.load", json!({"data": data})).unwrap();
    let steps = &s.actions.list[0].steps;
    for ((q, _, quality, mode), (id, p)) in cases.iter().zip(steps) {
        assert_eq!(id, "filter.blur.smartBlur", "{q}");
        assert_eq!(p, &json!({"radius": 6, "threshold": 60, "quality": quality, "mode": mode}), "{q}");
    }

    // An unknown code, a quality of the wrong type, NaN and a missing quality never panic: the
    // unknown ones stay Photoshop steps, kept verbatim.
    let odd = [
        smart_blur(enm("SmBQ", "Xxxx"), "SBMN"),
        smart_blur(D::Integer(2), "SBMN"),
        smart_blur(enm("SmBQ", "SBQH"), "Xxxx"),
        desc("null").with("Rds ", D::Double(f64::NAN)).with("SmBQ", enm("SmBQ", "SBQM")),
        desc("null"),
    ];
    let items: Vec<AtnItem> = odd.iter().map(|d| item("SmrB", "Smart Blur", Some(d.clone()))).collect();
    let imported = import(&photocraft_paint::tile::b64_decode(&set_of("Odd", vec![("Odd", items)])).unwrap()).unwrap();
    let steps = &imported.actions[0].steps;
    assert_eq!(steps.len(), odd.len());
    assert_eq!(steps[0].0, PHOTOSHOP_STEP, "an unknown quality is kept verbatim");
    assert_eq!(steps[2].0, PHOTOSHOP_STEP, "an unknown mode is kept verbatim");
    assert_eq!(imported.unsupported.iter().map(|u| u.step).collect::<Vec<_>>(), [0, 2]);
    // A wrong-type quality is left out (the filter's default applies), as for every enum field.
    assert_eq!(steps[1], ("filter.blur.smartBlur".to_string(), json!({"radius": 6, "threshold": 60, "mode": "normal"})));
    // An unknown quality or a non-string one is not exported.
    for bad in [json!({"quality": "best"}), json!({"quality": 3}), json!({"radius": "x"})] {
        let a = Action::new("Bad", vec![("filter.blur.smartBlur".into(), bad.clone())]);
        assert!(export("S", &[&a], &[], false).is_err(), "{bad}");
    }
}

/// #5830: Shadows/Highlights (`adaptCorrect`) recorded by Photoshop, with string IDs, plays as
/// Image › Adjustments › Shadows/Highlights instead of stopping as a step with no equivalent.
#[test]
fn photoshop_shadows_highlights_steps_play() {
    let tones = |amount: f64, width: f64, radius: i32| {
        D::Descriptor(desc("adaptCorrectTones").with("amount", pct(amount)).with("width", pct(width)).with("radius", D::Integer(radius)))
    };
    let d = desc("null")
        .with("shadowMode", tones(35.0, 50.0, 30))
        .with("highlightMode", tones(20.0, 40.0, 24))
        .with("blackClip", D::Double(0.01))
        .with("whiteClip", D::Double(0.02))
        .with("center", D::Integer(5))
        .with("colorCorrection", D::Integer(20));
    let e = event_for(&item("adaptCorrect", "Sombra/Realce", None).event).expect("adaptCorrect is mapped");
    let Mapped::Steps(steps) = (e.from)(&d) else { panic!("Shadows/Highlights maps to a step") };
    assert_eq!(steps.len(), 1);
    let (id, p) = &steps[0];
    assert_eq!(id, "image.adjustments.shadowsHighlights");
    let mut p = p.clone();
    normalise(&mut p);
    assert_eq!(
        p,
        json!({"shadowAmount": 35, "shadowTone": 50, "shadowRadius": 30, "highlightAmount": 20, "highlightTone": 40, "highlightRadius": 24, "blackClip": 0.01, "whiteClip": 0.02, "midtone": 5, "color": 20})
    );
    // The same settings as char IDs read the same; absent keys are left to the defaults.
    let codes = desc("null")
        .with("sdwM", D::Descriptor(desc("adaptCorrectTones").with("Amnt", pct(35.0)).with("Wdth", pct(50.0)).with("Rds ", D::Integer(30))))
        .with("BlcC", D::Double(0.01));
    let Mapped::Steps(steps) = (e.from)(&codes) else { panic!("char IDs map too") };
    let mut q = steps[0].1.clone();
    normalise(&mut q);
    assert_eq!(q, json!({"shadowAmount": 35, "shadowTone": 50, "shadowRadius": 30, "blackClip": 0.01}));
    // And it runs on a document.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "#404040"})).unwrap();
    s.execute(id, p).unwrap();
}

#[test]
fn save_while_recording_includes_pending_steps() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("actions.record", json!({"name": "Pending"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Recorded"})).unwrap();

    // While still recording (without calling actions.stop), save the action.
    let saved = s.execute("actions.save", json!({"action": "Pending"})).unwrap();
    assert_eq!(saved["steps"], 1, "pending steps must be flushed and included in save");
    assert_eq!(saved["skipped"], json!([]));

    // After save, recording continues and additional steps are flushed on stop without duplication.
    s.execute("layer.new.layer", json!({"name": "Second"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();

    let action = s.execute("actions.get", json!({"action": "Pending"})).unwrap();
    assert_eq!(action["steps"].as_array().unwrap().len(), 2, "both steps must be present without duplication");
}

/// #7233: the Minimum / Maximum descriptor carries the radius only, so a step with Preserve:
/// Roundness is refused by strict export (and left out, reported, otherwise) instead of replaying
/// as the default Squareness. Squareness, explicit or by default, still round-trips.
#[test]
fn minimum_and_maximum_with_roundness_are_not_exported_as_squareness() {
    for id in ["filter.other.minimum", "filter.other.maximum"] {
        let round = Action { name: "Round".into(), steps: vec![(id.into(), json!({"radius": 2, "preserve": "roundness"}))], ..Default::default() };
        assert!(export("Audit", &[&round], &[], false).is_err(), "{id}: strict export refuses it");
        let (_, skipped) = export("Audit", &[&round], &[], true).unwrap();
        assert_eq!(skipped, vec![Skipped { action: "Round".into(), step: 0, id: id.into() }], "{id}");
        for p in [json!({"radius": 2}), json!({"radius": 2, "preserve": "squareness"})] {
            let square = Action { name: "Square".into(), steps: vec![(id.into(), p.clone())], ..Default::default() };
            let (bytes, skipped) = export("Audit", &[&square], &[], false).unwrap();
            assert!(skipped.is_empty(), "{id} {p}");
            let imported = import(&bytes).unwrap();
            let (got, params) = &imported.actions[0].steps[0];
            assert_eq!((got.as_str(), params["radius"].as_f64()), (id, Some(2.0)), "{id} {p}");
            assert!(params.get("preserve").is_none_or(|v| v == "squareness"), "{id} {p}: {params}");
        }
    }
}

/// #7515: a Transform Selection's interpolation survives the round trip as a layer transform's
/// does. An arbitrary coordinate reference, a skew or a warp can't be written,
/// so the step is refused instead of replaying as a plain scale.
#[test]
fn transform_selection_keeps_its_interpolation_and_refuses_what_it_cannot_carry() {
    let one = |id: &str, p: Value| Action { name: "Probe".into(), steps: vec![(id.into(), p)], ..Default::default() };
    let p = json!({"dx": 0.7, "dy": 0, "scaleX": 140, "scaleY": 80, "interpolation": "nearest"});
    let (bytes, skipped) = export("Audit7", &[&one("select.transformSelection", p.clone())], &[], false).unwrap();
    assert!(skipped.is_empty());
    let imported = import(&bytes).unwrap();
    assert_eq!(imported.actions[0].steps, vec![("select.transformSelection".to_string(), p)]);
    for id in ["select.transformSelection", "edit.transform"] {
        for p in [
            json!({"scaleX": 140, "reference": [3, 4]}),
            json!({"scaleX": 140, "skewX": 12}),
            json!({"scaleX": 140, "skewY": -5}),
            json!({"warp": "arc"}),
            json!({"corners": [[0, 0], [10, 0], [10, 10], [0, 10]]}),
        ] {
            let err = export("Audit7", &[&one(id, p.clone())], &[], false).unwrap_err();
            assert!(err.contains("no Photoshop equivalent") && err.contains(&format!("`{id}`")), "{id} {p}: {err}");
            let (_, skipped) = export("Audit7", &[&one(id, p.clone())], &[], true).unwrap();
            assert_eq!(skipped, vec![Skipped { action: "Probe".into(), step: 0, id: id.into() }], "{id} {p}");
        }
        let carried = json!({"scaleX": 140, "reference": "center", "skewX": 0, "skewY": 0});
        assert!(export("Audit7", &[&one(id, carried.clone())], &[], false).is_ok(), "{id} {carried}");
    }
}

/// #7540-#7542: Feather, Contract and Smooth (and Expand) with Apply Effect at Canvas Bounds, and
/// #7543-#7546: a pattern Paint Bucket with a chosen pattern, scale, angle or phase, can't be
/// written, so strict export refuses them. Their defaults still export and round-trip.
#[test]
fn canvas_bound_modify_and_placed_pattern_bucket_refuse_strict_export() {
    let one = |id: &str, p: Value| Action { name: "Probe".into(), steps: vec![(id.into(), p)], ..Default::default() };
    for id in ["select.modify.feather", "select.modify.expand", "select.modify.contract", "select.modify.smooth"] {
        assert!(export("Audit7", &[&one(id, json!({"radius": 2, "applyAtCanvasBounds": true}))], &[], false).is_err(), "{id}");
        let plain = json!({"radius": 2, "applyAtCanvasBounds": false});
        let (bytes, _) = export("Audit7", &[&one(id, plain)], &[], false).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(import(&bytes).unwrap().actions[0].steps, vec![(id.to_string(), json!({"radius": 2}))]);
    }
    let bucket = |extra: Value| {
        let mut p = json!({"x": 5, "y": 5, "contents": "pattern", "opacity": 70, "tolerance": 80});
        if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            p.extend(e.clone());
        }
        one("paint.bucket", p)
    };
    for extra in [json!({"pattern": "AuditPattern"}), json!({"scale": 50}), json!({"angle": 30}), json!({"phase": [3, 5]})] {
        assert!(export("Audit7", &[&bucket(extra.clone())], &[], false).is_err(), "{extra}");
    }
    for extra in [json!({}), json!({"scale": 100, "angle": 0, "phase": [0, 0]})] {
        assert!(export("Audit7", &[&bucket(extra.clone())], &[], false).is_ok(), "{extra}");
    }
}

/// #7512, #7513, #7514: a New Layer step with a colour label, neutral fill or a position below
/// the active layer isn't exported. Invalid clipping values are also refused. Defaults still
/// export, and the step round-trips.
#[test]
fn new_layer_options_the_descriptor_cannot_carry_refuse_strict_export() {
    let new_layer = |p: Value| Action { name: "Probe".into(), steps: vec![("layer.new.layer".into(), p)], ..Default::default() };
    for p in [
        json!({"name": "New", "color": "red"}),
        json!({"name": "New", "mode": "Overlay", "fillNeutral": true}),
        json!({"name": "New", "position": "below"}),
        json!({"name": "New", "clipToPrevious": 1}),
    ] {
        assert!(export("Audit7", &[&new_layer(p.clone())], &[], false).is_err(), "{p}");
        let (_, skipped) = export("Audit7", &[&new_layer(p.clone())], &[], true).unwrap();
        assert_eq!(skipped, vec![Skipped { action: "Probe".into(), step: 0, id: "layer.new.layer".into() }], "{p}");
    }
    let defaults =
        json!({"name": "New", "mode": "Multiply", "opacity": 0.6, "clipToPrevious": false, "color": "none", "fillNeutral": false, "position": "above"});
    let (bytes, skipped) = export("Audit7", &[&new_layer(defaults)], &[], false).unwrap();
    assert!(skipped.is_empty());
    let imported = import(&bytes).unwrap();
    assert_eq!(imported.actions[0].steps, vec![("layer.new.layer".to_string(), json!({"name": "New", "mode": "Multiply", "opacity": 0.6}))]);
}

/// Options the exported descriptors can't carry refuse strict export (and a partial export skips
/// the step) instead of exporting a step that replays differently; the forms they can carry still
/// export (#7196, #7246, #7287, #7503, #7506, #7507).
#[test]
fn options_the_descriptors_cannot_carry_refuse_strict_export() {
    let one = |id: &str, p: Value| Action { name: "Probe".into(), steps: vec![(id.into(), p)], ..Default::default() };
    let refused = [
        ("layer.newFillLayer.solidColor", json!({"color": "#c0603080"})),
        ("layer.newFillLayer.solidColor", json!({"color": "teal"})),
        ("file.new", json!({"width": 24, "height": 18, "background": "black"})),
        ("file.new", json!({"width": 24, "height": 18, "background": "#ab2233"})),
        ("file.new", json!({"width": 24, "height": 18, "background": "backgroundColor", "backgroundColor": [0.2, 0.4, 0.6]})),
        ("select.loadSelection", json!({"channel": "Alpha 1", "invert": true})),
        ("select.loadSelection", json!({"channel": "Alpha 1", "operation": "subtract"})),
        ("layer.setProps", json!({"opacity": 0.5, "locked": true})),
        ("layer.setProps", json!({"opacity": 0.5, "clipped": true})),
        ("layer.setProps", json!({"layers": [2, 3], "opacity": 0.5})),
        ("filter.noise.addNoise", json!({"amount": 30, "seed": 2147483648_u64})),
        ("filter.noise.addNoise", json!({"amount": 30, "seed": 4294967295_u64})),
    ];
    for (id, p) in refused {
        let err = export("Audit7", &[&one(id, p.clone())], &[], false).unwrap_err();
        assert!(err.contains(&format!("`{id}`")), "{id} {p}: the error names the step: {err}");
        let (_, skipped) = export("Audit7", &[&one(id, p.clone())], &[], true).unwrap();
        assert_eq!(skipped, vec![Skipped { action: "Probe".into(), step: 0, id: id.into() }], "{id} {p}");
    }
    let carried = [
        ("layer.groupLayers", json!({"name": "Group"})),
        ("layer.newFillLayer.solidColor", json!({"color": "#c06030"})),
        ("file.new", json!({"width": 24, "height": 18, "background": "white"})),
        ("file.new", json!({"width": 24, "height": 18, "background": "transparent"})),
        ("file.new", json!({"width": 24, "height": 18, "background": "backgroundColor"})),
        ("select.loadSelection", json!({"channel": "Alpha 1", "invert": false, "operation": "new"})),
        // A named target (a numeric one is #7248, #7262).
        ("layer.setProps", json!({"layer": "Pixels", "opacity": 0.5, "visible": true})),
        ("filter.noise.addNoise", json!({"amount": 30, "seed": 2147483647})),
    ];
    for (id, p) in carried {
        let (bytes, skipped) = export("Audit7", &[&one(id, p.clone())], &[], false).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
        assert!(skipped.is_empty(), "{id} {p}");
        let imported = import(&bytes).unwrap();
        assert_eq!(imported.actions[0].steps.first().map(|(i, _)| i.as_str()), Some(id), "{p}");
    }
}

/// #8697: Photoshop's other transform interpolation modes survive import: its bicubic variants
/// are kept (they render as Bicubic and export back unchanged), Automatic becomes Bicubic
/// Automatic and Preserve Details Bicubic; before, they fell back to the command's default.
#[test]
fn photoshop_transform_interpolation_modes_survive_import() {
    let one = |id: &str, p: Value| Action { name: "Probe".into(), steps: vec![(id.into(), p)], ..Default::default() };
    for id in ["select.transformSelection", "edit.transform"] {
        for (mode, imported) in [
            ("bicubicSharper", "bicubicSharper"),
            ("bicubicSmoother", "bicubicSmoother"),
            ("bicubicAutomatic", "bicubicAutomatic"),
            ("automatic", "bicubicAutomatic"),
            ("preserveDetails", "bicubic"),
        ] {
            let (bytes, _) = export("Audit8", &[&one(id, json!({"scaleX": 140, "interpolation": mode}))], &[], false).unwrap();
            let steps = import(&bytes).unwrap().actions[0].steps.clone();
            let got = steps.first().and_then(|(_, p)| p.get("interpolation")).and_then(Value::as_str).map(str::to_owned);
            assert_eq!(got.as_deref(), Some(imported), "{id} {mode}: {steps:?}");
        }
    }
}

/// #8531: a partial `layer.setAdjustment` of Color Balance or Hue/Saturation can't be written
/// (their descriptors carry every setting, filling absent ones with defaults), so strict export
/// refuses it instead of replaying an edit that also resets the rest. Complete ones still export.
#[test]
fn partial_color_balance_and_hue_saturation_edits_refuse_strict_export() {
    let one = |p: Value| Action { name: "Probe".into(), steps: vec![("layer.setAdjustment".into(), p)], ..Default::default() };
    for p in [
        json!({"kind": "colorBalance", "shadows": [32, -24, 16]}),
        json!({"kind": "colorBalance", "preserveLuminosity": true}),
        json!({"kind": "hueSaturation", "hue": 20}),
        json!({"kind": "hueSaturation", "reds": {"hue": 10}}),
    ] {
        let err = export("Audit8", &[&one(p.clone())], &[], false).unwrap_err();
        assert!(err.contains("`layer.setAdjustment`"), "{p}: {err}");
        let (_, skipped) = export("Audit8", &[&one(p.clone())], &[], true).unwrap();
        assert_eq!(skipped, vec![Skipped { action: "Probe".into(), step: 0, id: "layer.setAdjustment".into() }], "{p}");
    }
    let full = json!({"kind": "colorBalance", "shadows": [32, -24, 16], "midtones": [-17, 27, -31], "highlights": [11, -19, 23], "preserveLuminosity": false});
    let (bytes, _) = export("Audit8", &[&one(full)], &[], false).unwrap();
    let steps = import(&bytes).unwrap().actions[0].steps.clone();
    let (id, p) = steps.first().unwrap();
    assert_eq!(id, "layer.setAdjustment");
    assert_eq!(p.get("midtones"), Some(&json!([-17, 27, -31])), "{p}");
    assert_eq!(p.get("preserveLuminosity"), Some(&json!(false)), "{p}");
    let hue = json!({"kind": "hueSaturation", "colorize": false, "hue": 20, "saturation": -5, "lightness": 0});
    assert!(export("Audit8", &[&one(hue)], &[], false).is_ok());
    // A colour range written without all of its own settings would reset the rest too.
    let range = json!({"kind": "hueSaturation", "colorize": false, "hue": 0, "saturation": 0, "lightness": 0, "reds": {"hue": 10}});
    assert!(export("Audit8", &[&one(range)], &[], false).is_err());
    let range =
        json!({"kind": "hueSaturation", "colorize": false, "hue": 0, "saturation": 0, "lightness": 0, "reds": {"hue": 10, "saturation": 5, "lightness": -5}});
    assert!(export("Audit8", &[&one(range)], &[], false).is_ok());
    let bounded = json!({"kind": "hueSaturation", "colorize": false, "hue": 0, "saturation": 0, "lightness": 0, "reds": {"hue": 10, "saturation": 5, "lightness": -5, "range": [315, 345, 15, 45]}});
    assert!(export("Audit8", &[&one(bounded)], &[], false).is_ok());
    // Values of the wrong shape would be written as 0: refused too.
    for bad in [
        json!({"kind": "colorBalance", "shadows": "invalid", "midtones": [0, 0, 0], "highlights": [0, 0, 0], "preserveLuminosity": true}),
        json!({"kind": "colorBalance", "shadows": [1, 2], "midtones": [0, 0, 0], "highlights": [0, 0, 0], "preserveLuminosity": true}),
        json!({"kind": "colorBalance", "shadows": [0, 0, 0], "midtones": [0, 0, 0], "highlights": [0, 0, 0], "preserveLuminosity": "yes"}),
        json!({"kind": "hueSaturation", "colorize": false, "hue": "20", "saturation": 0, "lightness": 0}),
        json!({"kind": "hueSaturation", "colorize": false, "hue": 0, "saturation": 0, "lightness": 0, "reds": 5}),
        json!({"kind": "hueSaturation", "colorize": false, "hue": 0, "saturation": 0, "lightness": 0, "reds": {"hue": 1, "saturation": 0, "lightness": 0, "range": [0, 20, 40]}}),
        json!({"kind": "hueSaturation", "colorize": false, "hue": 0, "saturation": 0, "lightness": 0, "reds": {"hue": 1, "saturation": 0, "lightness": 0, "range": "invalid"}}),
        json!({"kind": "hueSaturation", "colorize": false, "hue": 0, "saturation": 0, "lightness": 0, "reds": {"hue": 1, "saturation": 0, "lightness": 0, "range": [315, 345.5, 15, 45]}}),
    ] {
        assert!(export("Audit8", &[&one(bad.clone())], &[], false).is_err(), "{bad}");
    }
}

/// Levels, like Color Balance and Hue/Saturation: the descriptor writes both ends of the Input and
/// of the Output range, filling the end an edit did not name with 0 or 255, so replaying
/// `inBlack: 31` alone would also reset a retained Input White (#8530). Strict export refuses it;
/// a pair given whole, or none of it, still exports and round-trips.
#[test]
fn partial_levels_edits_refuse_strict_export() {
    let one = |p: Value| Action { name: "Probe".into(), steps: vec![("layer.setAdjustment".into(), p)], ..Default::default() };
    for p in [
        json!({"kind": "levels", "inBlack": 31}),
        json!({"kind": "levels", "inWhite": 239}),
        json!({"kind": "levels", "outBlack": 10}),
        json!({"kind": "levels", "outWhite": 240}),
        json!({"kind": "levels", "inBlack": 31, "inWhite": 239, "outWhite": 240}),
        json!({"kind": "levels", "red": {"inBlack": 2}}),
        json!({"kind": "levels", "inBlack": 1, "inWhite": 250, "blue": {"outBlack": 3}}),
        // A value of the wrong shape is no value: the other end is then alone.
        json!({"kind": "levels", "inBlack": 31, "inWhite": "239"}),
        // CodeRabbit on batch 11: both ends of the wrong shape, or a channel that isn't an object,
        // would be written as nothing too.
        json!({"kind": "levels", "inBlack": "31", "inWhite": "239"}),
        json!({"kind": "levels", "gamma": "1.2"}),
        json!({"kind": "levels", "red": 5}),
        json!({"kind": "levels", "green": [0, 255]}),
        json!({"kind": "levels", "blue": {"outBlack": "3", "outWhite": "250"}}),
    ] {
        let err = export("Audit8", &[&one(p.clone())], &[], false).unwrap_err();
        assert!(err.contains("`layer.setAdjustment`"), "{p}: {err}");
        let (_, skipped) = export("Audit8", &[&one(p.clone())], &[], true).unwrap();
        assert_eq!(skipped, vec![Skipped { action: "Probe".into(), step: 0, id: "layer.setAdjustment".into() }], "{p}");
    }
    for p in [
        json!({"kind": "levels", "inBlack": 31, "inWhite": 239}),
        json!({"kind": "levels", "outBlack": 10, "outWhite": 240}),
        json!({"kind": "levels", "inBlack": 31, "inWhite": 239, "outBlack": 10, "outWhite": 240, "gamma": 1.2}),
        json!({"kind": "levels", "gamma": 0.8}),
        json!({"kind": "levels", "inBlack": 1, "inWhite": 250, "red": {"inBlack": 2, "inWhite": 200}}),
    ] {
        let (bytes, skipped) = export("Audit8", &[&one(p.clone())], &[], false).unwrap_or_else(|e| panic!("{p}: {e}"));
        assert!(skipped.is_empty(), "{p}");
        let steps = import(&bytes).unwrap().actions[0].steps.clone();
        let (id, back) = steps.first().unwrap();
        assert_eq!(id, "layer.setAdjustment");
        for k in ["inBlack", "inWhite", "outBlack", "outWhite"] {
            assert_eq!(back.get(k), p.get(k), "{p}: {k} in {back}");
        }
    }
}

/// Photoshop's Close has no document target: a Close of a specific document must not be
/// exported as a plain Close, which would close the active document on playback.
#[test]
fn close_with_document_target_is_not_exported_as_plain_close() {
    let targeted = Action { name: "Probe".into(), steps: vec![("file.close".into(), json!({"document": 0}))], ..Default::default() };
    assert!(export("Audit4", &[&targeted], &[], false).is_err());
    let (_, skipped) = export("Audit4", &[&targeted], &[], true).unwrap();
    assert_eq!(skipped, vec![Skipped { action: "Probe".into(), step: 0, id: "file.close".into() }]);
    let plain = Action { name: "Plain".into(), steps: vec![("file.close".into(), json!({}))], ..Default::default() };
    let (bytes, skipped) = export("Audit4", &[&plain], &[], false).unwrap();
    assert!(skipped.is_empty());
    let imported = import(&bytes).unwrap();
    assert_eq!(imported.actions[0].steps, vec![("file.close".to_string(), json!({}))]);
}

/// A translation-only Free Transform with a fractional offset must not collapse into an
/// integer `layer.translate`: the sub-pixel placement and interpolation have to survive.
#[test]
fn fractional_translation_only_transform_keeps_offset_and_interpolation() {
    let params = json!({"dx": 0.5, "dy": 0, "interpolation": "bilinear"});
    let d = transform_to("edit.transform", &params).unwrap();
    let file = AtnFile {
        name: UnicodeString::new_nul("Frac"),
        expanded: false,
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items: vec![item("transform", "Transform", Some(d))], ..Default::default() }],
        warnings: vec![],
        ..Default::default()
    };
    let imp = import(&atn::write(&file).unwrap()).unwrap();
    assert!(imp.unsupported.is_empty());
    let steps = &imp.actions[0].steps;
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].0, "edit.transform");
    assert_eq!(steps[0].1.get("dx").and_then(Value::as_f64), Some(0.5));
    assert_eq!(steps[0].1.get("interpolation"), Some(&json!("bilinear")));
}

#[test]
fn numeric_layer_references_refuse_silent_fallback_to_current_selection() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();

    let parse_first_desc = |v: &Value| -> Descriptor {
        let bytes = photocraft_paint::tile::b64_decode(v["data"].as_str().unwrap()).unwrap();
        let file = atn::parse(&bytes).unwrap();
        file.actions[0].items[0].descriptor.clone().unwrap()
    };

    // Named layer reference exports successfully with Name reference to "A"
    s.actions.list.push(Action::new("DeleteNamed", vec![("layer.delete".into(), json!({"layer": "A"}))]));
    let saved_named = s.execute("actions.save", json!({"action": "DeleteNamed"})).unwrap();
    assert_eq!(saved_named["steps"], 1);
    let named_desc = parse_first_desc(&saved_named);
    match get_ref(&named_desc, "null").unwrap() {
        [ReferenceItem::Name { name, .. }] => assert_eq!(name.to_string_lossy(), "A"),
        other => panic!("expected Name reference to A, got {other:?}"),
    }

    // Omitted layer reference (target selection) exports successfully with Trgt enumerated reference
    s.actions.list.push(Action::new("DeleteTarget", vec![("layer.delete".into(), json!({}))]));
    let saved_target = s.execute("actions.save", json!({"action": "DeleteTarget"})).unwrap();
    assert_eq!(saved_target["steps"], 1);
    let target_desc = parse_first_desc(&saved_target);
    match get_ref(&target_desc, "null").unwrap() {
        [ReferenceItem::Enumerated { value, .. }] => assert!(value.is("Trgt")),
        other => panic!("expected Trgt enumerated reference, got {other:?}"),
    }

    // Explicit null layer reference also exports successfully as target selection with Trgt enumerated reference
    s.actions.list.push(Action::new("DeleteNull", vec![("layer.delete".into(), json!({"layer": null}))]));
    let saved_null = s.execute("actions.save", json!({"action": "DeleteNull"})).unwrap();
    assert_eq!(saved_null["steps"], 1);
    let null_desc = parse_first_desc(&saved_null);
    match get_ref(&null_desc, "null").unwrap() {
        [ReferenceItem::Enumerated { value, .. }] => assert!(value.is("Trgt")),
        other => panic!("expected Trgt enumerated reference, got {other:?}"),
    }

    // Explicit numeric ID reference cannot be silently converted to current target in strict export
    s.actions.list.push(Action::new("DeleteNumeric", vec![("layer.delete".into(), json!({"layer": 42}))]));
    let before_actions = s.actions.list.clone();
    let before_rev = s.active().unwrap().revision;
    let err_delete = s.execute("actions.save", json!({"action": "DeleteNumeric"}));
    assert!(err_delete.is_err(), "strict export must reject numeric layer ID for layer.delete rather than falling back to current target");
    assert_eq!(s.actions.list, before_actions, "session actions must remain unchanged when save fails");
    assert_eq!(s.active().unwrap().revision, before_rev, "document revision must remain unchanged when save fails");
    let err_msg = err_delete.unwrap_err().to_string();
    assert!(err_msg.contains("(`layer.delete` with numeric layer 42)"), "error message should name the numeric layer: {err_msg}");

    // Same rejection and state invariance for remaining commands with numeric layer reference
    for (name, id, params) in [
        ("DuplicateNumeric", "layer.duplicate", json!({"layer": 42, "name": "Copy"})),
        ("DuplicateNewDocNumeric", "layer.duplicate", json!({"layer": 42, "document": "new"})),
        ("TranslateNumeric", "layer.translate", json!({"layer": 42, "dx": 5, "dy": 5})),
        ("SetPropsNumeric", "layer.setProps", json!({"layer": 42, "opacity": 0.5})),
        ("HideNumeric", "layer.hideLayers", json!({"layer": 42})),
        ("TransformNumeric", "edit.transform", json!({"layer": 42, "dx": 5, "dy": 5})),
        ("RenameNumeric", "layer.renameLayer", json!({"layer": 42, "name": "Renamed"})),
        ("LabelNumeric", "layer.setLabelColor", json!({"layer": 42, "color": "red"})),
        ("ShowNumeric", "layer.showLayers", json!({"layer": 42})),
        ("SelectNumeric", "layer.select", json!({"layer": 42})),
        ("BringToFrontNumeric", "layer.arrange.bringToFront", json!({"layer": 42})),
        ("SendToBackNumeric", "layer.arrange.sendToBack", json!({"layer": 42})),
        ("BringForwardNumeric", "layer.arrange.bringForward", json!({"layer": 42})),
        ("SendBackwardNumeric", "layer.arrange.sendBackward", json!({"layer": 42})),
    ] {
        s.actions.list.push(Action::new(name, vec![(id.into(), params)]));
        let before_actions = s.actions.list.clone();
        let before_rev = s.active().unwrap().revision;
        assert!(s.execute("actions.save", json!({"action": name})).is_err());
        assert_eq!(s.actions.list, before_actions, "session actions must remain unchanged when save fails");
        assert_eq!(s.active().unwrap().revision, before_rev, "document revision must remain unchanged when save fails");
    }

    // With skipUnsupported: true, numeric layer references are skipped
    let skipped = s.execute("actions.save", json!({"action": "DeleteNumeric", "skipUnsupported": true})).unwrap();
    assert_eq!(skipped["steps"], 0);
    assert_eq!(skipped["skipped"][0]["id"], "layer.delete");

    // Blend-mode shortcut records layer.setProps with active layer ID, which normalises to the target form
    let active_id = s.active().unwrap().active_layer.unwrap().0;
    s.execute("actions.record", json!({"name": "RecordBlend"})).unwrap();
    s.execute("layer.setProps", json!({"layer": active_id, "blend": "multiply"})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let saved_blend = s.execute("actions.save", json!({"action": "RecordBlend"})).unwrap();
    assert_eq!(saved_blend["steps"], 1, "recorded blend shortcut step with active layer id must export successfully as target layer");
    assert_eq!(saved_blend["skipped"].as_array().unwrap().len(), 0);

    // Record explicit layer.delete of the sole selected layer in a two-layer document
    s.execute("layer.new.layer", json!({"name": "Second"})).unwrap();
    let second_id = s.active().unwrap().active_layer.unwrap().0;
    s.execute("actions.record", json!({"name": "RecordDelete"})).unwrap();
    s.execute("layer.delete", json!({"layer": second_id})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let saved_delete = s.execute("actions.save", json!({"action": "RecordDelete"})).unwrap();
    assert_eq!(saved_delete["steps"], 1, "recorded explicit delete of sole selected layer must export successfully as target delete");
    assert_eq!(saved_delete["skipped"].as_array().unwrap().len(), 0);
}

/// Arbitrary canvas rotation must keep its interpolation through a strict export: dropping it
/// silently replays the rotation with the default bicubic resampling.
#[test]
fn arbitrary_rotation_keeps_interpolation_through_atn() {
    for params in [
        json!({"angle": 17, "direction": "ccw", "interpolation": "nearest"}),
        json!({"angle": 17, "direction": "ccw", "interpolation": "bilinear"}),
        json!({"angle": 17, "direction": "ccw", "interpolation": "bicubic"}),
        json!({"angle": 17, "direction": "ccw"}),
    ] {
        let a = Action { name: "Rot".into(), steps: vec![("image.rotation.arbitrary".into(), params.clone())], ..Default::default() };
        let (bytes, skipped) = export("Rot", &[&a], &[], false).unwrap();
        assert!(skipped.is_empty());
        let imported = import(&bytes).unwrap();
        assert!(imported.unsupported.is_empty());
        assert_eq!(imported.actions[0].steps, vec![("image.rotation.arbitrary".to_string(), params)]);
    }
}

/// Filter › Stylize › Emboss saves as Photoshop's `Embs` event with integer `Angl`, `Hght` and
/// `Amnt`, and loads back as the same step (#7946: it was skipped, so a portable action lost it).
#[test]
fn emboss_saves_as_photoshops_event_and_loads_back() {
    let mut s = Session::new();
    let params = json!({"angle": 130, "height": 1, "amount": 70});
    s.actions.list.push(Action::new("Pore", vec![("filter.stylize.emboss".into(), params.clone())]));
    let r = s.execute("actions.save", json!({"action": "Pore"})).unwrap();
    assert_eq!(r["skipped"], json!([]));
    let saved = photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap();
    let file = atn::parse(&saved).unwrap();
    let step = &file.actions[0].items[0];
    assert_eq!(step.event.id(), "Embs");
    let d = step.descriptor.as_ref().unwrap();
    for (key, want) in [("Angl", 130), ("Hght", 1), ("Amnt", 70)] {
        assert_eq!(d.get(key), Some(&D::Integer(want)), "{key}");
    }
    let mut t = Session::new();
    t.execute("actions.load", json!({"data": b64(&saved)})).unwrap();
    assert_eq!(t.actions.list[0].steps, vec![("filter.stylize.emboss".to_string(), params)]);
}

/// A filter recorded against a layer mask must not export as a plain filter that edits the
/// layer's pixels on replay: Photoshop's descriptor has no field for PhotoCraft's `target`.
#[test]
fn add_noise_with_mask_target_is_not_exported_as_pixel_filter() {
    let masked = Action {
        name: "Probe".into(),
        steps: vec![("filter.noise.addNoise".into(), json!({"amount": 75, "seed": 123, "target": "mask"}))],
        ..Default::default()
    };
    assert!(export("Audit", &[&masked], &[], false).is_err());
    let (_, skipped) = export("Audit", &[&masked], &[], true).unwrap();
    assert_eq!(skipped, vec![Skipped { action: "Probe".into(), step: 0, id: "filter.noise.addNoise".into() }]);
    for params in [json!({"amount": 75, "seed": 123}), json!({"amount": 75, "seed": 123, "target": "pixels"})] {
        let plain = Action { name: "Plain".into(), steps: vec![("filter.noise.addNoise".into(), params)], ..Default::default() };
        let (bytes, skipped) = export("Audit", &[&plain], &[], false).unwrap();
        assert!(skipped.is_empty());
        let imported = import(&bytes).unwrap();
        assert_eq!(imported.actions[0].steps, vec![("filter.noise.addNoise".to_string(), json!({"amount": 75, "seed": 123}))]);
    }
}

/// Photoshop's Duplicate Layer has no in-place flag: an in-place duplicate (the artboard copy
/// that stays on the original) must not be exported as a plain duplicate, which places the
/// copy beside the original on playback.
#[test]
fn duplicate_in_place_is_not_exported_as_plain_duplicate() {
    let in_place = Action { name: "Probe".into(), steps: vec![("layer.duplicate".into(), json!({"inPlace": true}))], ..Default::default() };
    assert!(export("Audit5", &[&in_place], &[], false).is_err());
    let (_, skipped) = export("Audit5", &[&in_place], &[], true).unwrap();
    assert_eq!(skipped, vec![Skipped { action: "Probe".into(), step: 0, id: "layer.duplicate".into() }]);
    for params in [json!({}), json!({"inPlace": false})] {
        let plain = Action { name: "Plain".into(), steps: vec![("layer.duplicate".into(), params)], ..Default::default() };
        let (bytes, skipped) = export("Audit5", &[&plain], &[], false).unwrap();
        assert!(skipped.is_empty());
        let imported = import(&bytes).unwrap();
        assert_eq!(imported.actions[0].steps, vec![("layer.duplicate".to_string(), json!({}))]);
    }
}

/// A Play Action that starts the called action at a later step must not export as a call of the
/// whole action: that would replay the steps the recording deliberately skipped.
#[test]
fn partial_play_action_is_not_exported_as_whole_action_call() {
    let partial = Action { name: "Parent".into(), steps: vec![("actions.play".into(), json!({"action": "Child", "from": 1}))], ..Default::default() };
    assert!(export("Set", &[&partial], &[], false).is_err());
    let (_, skipped) = export("Set", &[&partial], &[], true).unwrap();
    assert_eq!(skipped, vec![Skipped { action: "Parent".into(), step: 0, id: "actions.play".into() }]);
    for params in [json!({"action": "Child"}), json!({"action": "Child", "from": 0})] {
        let whole = Action { name: "Parent".into(), steps: vec![("actions.play".into(), params)], ..Default::default() };
        let (bytes, skipped) = export("Set", &[&whole], &[], false).unwrap();
        assert!(skipped.is_empty());
        let imported = import(&bytes).unwrap();
        assert_eq!(imported.actions[0].steps, vec![("actions.play".to_string(), json!({"action": "Child"}))]);
    }
}

/// #7646: an action's function key keeps Photoshop's Shift and Command options through
/// `.atn` load and save, as `Shift+F5`, `Cmd+F5` and `Cmd+Shift+F5` shortcuts.
#[test]
fn function_keys_keep_shift_and_command() {
    let mut file = sample();
    file.actions[1].function_key = 5;
    file.actions[1].shift = true;
    file.actions[1].command = true;
    file.actions[0].shift = true;
    let mut s = Session::new();
    let r = s.execute("actions.load", json!({"data": b64(&atn::write(&file).unwrap())})).unwrap();
    assert_eq!(r["functionKeys"], json!([{"action": "Soften", "key": "Shift+F3"}, {"action": "Finish", "key": "Cmd+Shift+F5"}]));
    let shortcuts = &s.prefs().shortcuts;
    assert_eq!(shortcuts.get("actions.play:Soften").map(String::as_str), Some("Shift+F3"));
    assert_eq!(shortcuts.get("actions.play:Finish").map(String::as_str), Some("Cmd+Shift+F5"));
    let r = s.execute("actions.save", json!({"set": "Test Set"})).unwrap();
    let saved = atn::parse(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap();
    let keys: Vec<_> = saved.actions.iter().map(|a| (a.function_key, a.shift, a.command)).collect();
    assert_eq!(keys, [(3, true, false), (5, true, true)]);

    assert_eq!(super::parse_function_key("Cmd+F12"), Some((12, false, true)));
    assert_eq!(super::parse_function_key("Shift+Cmd+F2"), Some((2, true, true)));
    for bad in ["F16", "F0", "Alt+F2", "Cmd+Cmd+F2", "Shift+", "Cmd+K", ""] {
        assert_eq!(super::parse_function_key(bad), None, "{bad}");
    }
    assert_eq!(super::function_key_name(7, false, false), "F7");
}

/// The Apply Image dialog's flat source fields keep their meaning through a strict `.atn` export
/// (#7197): they used to come back as the merged composite.
#[test]
fn apply_image_flat_source_fields_survive_a_strict_export() {
    let flat = json!({"sourceLayer": "Pixels", "sourceChannel": "green", "sourceInvert": true, "blending": "normal", "opacity": 75});
    let item = export_item("image.applyImage", &flat, "S").expect("the flat form exports");
    let event = event_for(&item.event).expect("a known event");
    let Mapped::Steps(steps) = (event.from)(item.descriptor.as_ref().unwrap()) else { panic!("does not import back") };
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].1["source"], json!({"layer": "Pixels", "channel": "green", "invert": true}));
    // The nested form is unchanged, and the "merged" layer of a flat form stays merged.
    let merged = json!({"sourceLayer": "merged", "sourceChannel": "red"});
    let item = export_item("image.applyImage", &merged, "S").expect("exports");
    let Mapped::Steps(steps) = (event.from)(item.descriptor.as_ref().unwrap()) else { panic!("does not import back") };
    assert_eq!(steps[0].1["source"], json!({"layer": "merged", "channel": "red"}));
    // A source in another document cannot be written: the step is skipped, not rewritten.
    assert!(export_item("image.applyImage", &json!({"source": {"document": "Other", "channel": "red"}}), "S").is_none());
    assert!(export_item("image.applyImage", &json!({"sourceDocument": "Other"}), "S").is_none());
}

/// What the Apply Image dialog records survives a strict `.atn` export and comes back: its source
/// channel is a Channels-panel name ("RGB", "Red", "Alpha 1"), not a lowercase key.
#[test]
fn apply_image_dialog_channel_names_survive_a_strict_export() {
    let dialog = |channel: &str| json!({"sourceLayer": "merged", "sourceChannel": channel, "sourceInvert": false, "blending": "multiply", "opacity": 100.0, "scale": 1.0, "offset": 0, "preserveTransparency": false, "maskChannel": "none"});
    for (channel, back) in [
        ("RGB", "composite"),
        ("CMYK", "composite"),
        ("Lab", "composite"),
        ("Red", "red"),
        ("Gray", "gray"),
        ("Alpha 1", "Alpha 1"),
        ("transparency", "transparency"),
    ] {
        let a = Action::new("A", vec![("image.applyImage".into(), dialog(channel))]);
        let (bytes, skipped) = export("S", &[&a], &[], false).unwrap_or_else(|e| panic!("{channel}: {e}"));
        assert!(skipped.is_empty(), "{channel}");
        let file = atn::parse(&bytes).unwrap();
        assert_eq!(file.actions[0].items[0].event.id(), "AppI");
        let back_file = import(&bytes).unwrap();
        assert!(back_file.unsupported.is_empty(), "{channel}");
        let p = &back_file.actions[0].steps[0].1;
        assert_eq!(p["source"], json!({"layer": "merged", "channel": back, "invert": false}), "{channel}");
    }
    // A named channel is written as Photoshop records it: a `Chnl` name reference.
    let item = export_item("image.applyImage", &dialog("Alpha 1"), "S").unwrap();
    let with = get_obj(item.descriptor.as_ref().unwrap(), "With").unwrap();
    let r = get_ref(with, "T   ").unwrap();
    assert!(matches!(&r[0], ReferenceItem::Name { class, name } if class.class_id.is("Chnl") && name.to_string_lossy() == "Alpha 1"));
}

/// A numeric layer or channel selector has no name to record: the step is refused, not exported as
/// the merged composite.
#[test]
fn apply_image_numeric_source_selectors_are_refused() {
    for p in [
        json!({"sourceLayer": 3}),
        json!({"sourceChannel": 2}),
        json!({"source": {"channel": 2}}),
        json!({"source": {"layer": 7, "channel": "red"}}),
        json!({"source": "red"}),
        json!({"sourceChannel": "quickMask"}),
    ] {
        assert!(export_item("image.applyImage", &p, "S").is_none(), "{p}");
    }
    let a = Action::new("A", vec![("image.applyImage".into(), json!({"sourceLayer": 3}))]);
    assert!(export("S", &[&a], &[], false).is_err());
}

/// A param a PhotoCraft step leaves out exports as PhotoCraft's default wherever Photoshop plays
/// the missing key differently (checked against Photoshop): Canvas Size anchors top left,
/// Apply Image blends Normal, Color Balance turns preserve luminosity off, and a Paint Bucket
/// step without a tolerance doesn't play at all.
#[test]
fn defaults_photoshop_plays_differently_are_written() {
    let exported = |id: &str, p: Value| export_item(id, &p, "S").and_then(|i| i.descriptor).unwrap_or_else(|| panic!("{id} does not export"));
    let bucket = exported("paint.bucket", json!({"x": 1, "y": 2}));
    assert_eq!((get_num(&bucket, "Tlrn"), get_bool(&bucket, "AntA")), (Some(32.0), Some(true)));
    let canvas = exported("image.canvasSize", json!({"width": 10, "height": 10}));
    assert_eq!((get_enum(&canvas, "Hrzn").as_deref(), get_enum(&canvas, "Vrtc").as_deref()), (Some("Cntr"), Some("Cntr")));
    let apply = exported("image.applyImage", json!({}));
    assert_eq!(get_obj(&apply, "With").and_then(|w| get_enum(w, "Clcl")).as_deref(), Some("Mltp"));
    let balance = exported("image.adjustments.colorBalance", json!({"midtones": [10, 0, 0]}));
    assert_eq!(get_bool(&balance, "PrsL"), Some(true));
    assert_eq!(get_list(&balance, "ShdL").map(<[D]>::len), Some(3));
}

/// Photoshop records the composite of a Levels or Curves entry as `Chnl.Cmps` and silently skips
/// a Set that names it `Ordn.Trgt` (#8543). A colour channel keeps its own code.
#[test]
fn levels_and_curves_name_the_composite_channel_as_photoshop_records_it() {
    let exported = |id: &str, p: Value| export_item(id, &p, "S").and_then(|i| i.descriptor).unwrap_or_else(|| panic!("{id} does not export"));
    let channels = |d: &Descriptor| -> Vec<(String, String)> {
        get_list(d, "Adjs")
            .unwrap_or_default()
            .iter()
            .filter_map(|e| match e {
                D::Descriptor(e) => match get_ref(e, "Chnl")?.first()? {
                    ReferenceItem::Enumerated { type_id, value, .. } => Some((trimmed(type_id), trimmed(value))),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    };
    let composite = ("Chnl".to_string(), "Cmps".to_string());
    let red = ("Chnl".to_string(), "Rd".to_string());
    let levels = exported("image.adjustments.levels", json!({"inBlack": 31, "red": {"inBlack": 2}}));
    assert_eq!(channels(&levels), vec![composite.clone(), red.clone()]);
    let curves = exported("image.adjustments.curves", json!({"points": [[0, 0], [128, 140], [255, 255]], "red": [[0, 10], [255, 245]]}));
    assert_eq!(channels(&curves), vec![composite, red]);
}

/// A Lab document's Lightness channel is `Chnl.Lght` (#9384): Photoshop plays a Levels or Apply
/// Image step naming it and rejects `Chnl.Lmnc` ("luminance"), which PhotoCraft wrote before.
/// Steps PhotoCraft saved with `Lmnc` still load.
#[test]
fn lab_lightness_channel_is_written_and_read_as_photoshop_names_it() {
    let exported = |id: &str, p: Value| export_item(id, &p, "S").and_then(|i| i.descriptor).unwrap_or_else(|| panic!("{id} does not export"));
    let entry_codes = |d: &Descriptor| -> Vec<String> {
        get_list(d, "Adjs")
            .unwrap_or_default()
            .iter()
            .filter_map(|e| match e {
                D::Descriptor(e) => match get_ref(e, "Chnl")?.first()? {
                    ReferenceItem::Enumerated { value, .. } => Some(trimmed(value)),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    };
    let levels = exported("image.adjustments.levels", json!({"lightness": {"gamma": 1.5}, "a": {"inBlack": 3}}));
    assert_eq!(entry_codes(&levels), ["Cmps", "Lght", "A"]);
    let curves = exported("image.adjustments.curves", json!({"lightness": [[0, 10], [255, 245]]}));
    assert_eq!(entry_codes(&curves), ["Lght"]);
    let apply = exported("image.applyImage", json!({"sourceLayer": "merged", "sourceChannel": "Lightness"}));
    let source = get_ref(get_obj(&apply, "With").unwrap(), "T   ").unwrap();
    assert!(matches!(&source[0], ReferenceItem::Enumerated { value, .. } if trimmed(value) == "Lght"), "{source:?}");

    // What Photoshop records, and what PhotoCraft wrote before, both read as Lightness.
    let chnl = |c: &str| D::Reference(vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Chnl"), value: code(c) }]);
    for c in ["Lght", "Lmnc"] {
        let lvls = desc("Lvls").with("Adjs", D::List(vec![D::Descriptor(desc("LvlA").with("Chnl", chnl(c)).with("Gmm ", D::Double(1.5)))]));
        assert_eq!(levels_from(&lvls), Some(json!({"lightness": {"gamma": 1.5}})), "{c}");
        let with = desc("Clcl").with("T   ", chnl(c));
        let Mapped::Steps(steps) = apply_image_from(&desc("null").with("With", D::Descriptor(with))) else { panic!("{c}: Apply Image does not import") };
        assert_eq!(steps[0].1["source"]["channel"], "lightness", "{c}");
    }
}

/// Photoshop sets one colour per step: a step that sets both doesn't export rather than drop one.
#[test]
fn set_colors_exports_only_a_single_colour() {
    assert!(export_item("tools.setColors", &json!({"foreground": "#ff0000", "background": "#0000ff"}), "S").is_none());
    assert!(export_item("tools.setColors", &json!({"foregroundCmyk": [0, 0, 0, 100], "background": "#0000ff"}), "S").is_none());
    assert!(export_item("tools.setColors", &json!({"background": "#0000ff"}), "S").is_some());
}

/// Photoshop's reset of an adjustment layer (`set` with an empty `T`) stays a reset: `kind`
/// only joins params that edit something, as any key makes `layer.setAdjustment` merge.
#[test]
fn adjustment_layer_reset_still_resets() {
    let set = |t: Descriptor| desc("null").with("null", target_ref("AdjL")).with("T   ", D::Descriptor(t));
    let reset = set(desc("HStr").with("presetKind", enm("presetKindType", "presetKindDefault")));
    let edit = set(desc("ClrB").with("MdtL", D::List(vec![long(10.0), long(0.0), long(0.0)])));
    let file = AtnFile {
        name: UnicodeString::new_nul("Reset"),
        expanded: false,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("A"),
            items: vec![item("setd", "Set", Some(reset)), item("setd", "Set", Some(edit))],
            ..Default::default()
        }],
        warnings: vec![],
        ..Default::default()
    };
    let steps = import(&atn::write(&file).unwrap()).unwrap().actions.remove(0).steps;
    assert_eq!(steps[0], ("layer.setAdjustment".to_string(), json!({})));
    assert_eq!(steps[1].1, json!({"midtones": [10, 0, 0], "kind": "colorBalance"}));
}

/// Photoshop's Properties panel records an edit of an existing Vibrance layer with the string
/// class `vibrance`, not the `Vbrn` code: both import as the same `layer.setAdjustment` (#8534).
#[test]
fn adjustment_layer_set_accepts_the_vibrance_string_class() {
    let set = |class: &str| {
        let t = desc(class).with("temperature", long(0.0)).with("tint", long(0.0)).with("vibrance", long(28.0)).with("Strt", long(-7.0));
        desc("null").with("null", target_ref("AdjL")).with("T   ", D::Descriptor(t))
    };
    let file = AtnFile {
        name: UnicodeString::new_nul("Vibrance"),
        expanded: false,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("A"),
            items: vec![item("set", "Set", Some(set("vibrance"))), item("set", "Set", Some(set("Vbrn")))],
            ..Default::default()
        }],
        warnings: vec![],
        ..Default::default()
    };
    let action = import(&atn::write(&file).unwrap()).unwrap().actions.remove(0);
    assert_eq!(action.steps[0].0, "layer.setAdjustment", "{:?}", action.steps);
    assert_eq!(action.steps[0].1, json!({"vibrance": 28, "saturation": -7, "kind": "vibrance"}));
    assert_eq!(action.steps[0], action.steps[1]);
}

/// A Vibrance Set and Make as Photoshop records them (class `vibrance`) save back with that
/// class, not the `Vbrn` code Photoshop refuses (#8383); old PhotoCraft files with `Vbrn` still
/// load and now save with `vibrance` too.
#[test]
fn vibrance_adjustment_layer_steps_save_with_the_string_class() {
    let set = |class: &str| {
        let t = desc(class).with("temperature", long(0.0)).with("tint", long(0.0)).with("vibrance", long(28.0)).with("Strt", long(-7.0));
        desc("null").with("null", target_ref("AdjL")).with("T   ", D::Descriptor(t))
    };
    let make = |class: &str| {
        let t = desc(class).with("vibrance", long(10.0)).with("Strt", long(5.0));
        desc("null").with("null", class_ref("AdjL")).with("Usng", D::Descriptor(desc("AdjL").with("Type", D::Descriptor(t))))
    };
    let data = set_of(
        "Vibrance",
        vec![(
            "A",
            vec![
                item("set", "Set", Some(set("vibrance"))),
                item("Mk  ", "Make", Some(make("vibrance"))),
                item("set", "Set", Some(set("Vbrn"))),
                item("Mk  ", "Make", Some(make("Vbrn"))),
            ],
        )],
    );
    let mut s = Session::new();
    let loaded = s.execute("actions.load", json!({"data": data})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]), "{loaded}");
    let steps = &s.actions.list[0].steps;
    assert_eq!(steps[1], ("layer.newAdjustmentLayer.vibrance".to_string(), json!({"vibrance": 10, "saturation": 5})), "{steps:?}");
    assert_eq!((&steps[0], &steps[1]), (&steps[2], &steps[3]), "the old `Vbrn` class imports the same");
    let saved = s.execute("actions.save", json!({"set": "Vibrance"})).unwrap();
    assert_eq!(saved["skipped"], json!([]));
    let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
    let file = atn::parse(&bytes).unwrap();
    let items = &file.actions[0].items;
    assert_eq!(items.len(), 4);
    for pair in items.chunks(2) {
        let t = get_obj(pair[0].descriptor.as_ref().unwrap(), "T   ").unwrap();
        assert!(t.class_id.is("vibrance"), "Set class {:?}", t.class_id);
        let ty = get_obj(pair[1].descriptor.as_ref().unwrap(), "Usng").and_then(|u| get_obj(u, "Type")).unwrap();
        assert!(ty.class_id.is("vibrance"), "Make class {:?}", ty.class_id);
    }
    let mut reloaded = Session::new();
    reloaded.execute("actions.load", json!({"data": saved["data"]})).unwrap();
    assert_eq!(reloaded.actions.list[0].steps, s.actions.list[0].steps);
}

/// Our Vibrance has no `temperature` or `tint`: a recorded Vibrance Set or Make with either one
/// non-zero stays a kept Photoshop step and saves back unchanged instead of losing them.
#[test]
fn vibrance_steps_with_temperature_or_tint_save_back_unchanged() {
    let t = || desc("vibrance").with("temperature", long(0.0)).with("tint", long(12.0)).with("vibrance", long(28.0)).with("Strt", long(-7.0));
    let set = desc("null").with("null", target_ref("AdjL")).with("T   ", D::Descriptor(t()));
    let warm = desc("vibrance").with("temperature", long(-20.0)).with("vibrance", long(5.0));
    let make = desc("null").with("null", class_ref("AdjL")).with("Usng", D::Descriptor(desc("AdjL").with("Type", D::Descriptor(warm))));
    let data = set_of("Vibrance", vec![("A", vec![item("set", "Set", Some(set.clone())), item("Mk  ", "Make", Some(make.clone()))])]);
    let mut s = Session::new();
    s.execute("actions.load", json!({"data": data})).unwrap();
    assert!(s.actions.list[0].steps.iter().all(|(id, _)| id == "actions.photoshopStep"), "{:?}", s.actions.list[0].steps);
    let saved = s.execute("actions.save", json!({"set": "Vibrance"})).unwrap();
    let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
    let file = atn::parse(&bytes).unwrap();
    assert_eq!(file.actions[0].items[0].descriptor, Some(set));
    assert_eq!(file.actions[0].items[1].descriptor, Some(make));
}

/// A Free Transform recorded against an unlinked mask or an alpha channel must not export as a
/// layer transform that moves the layer's pixels on replay (#7195).
#[test]
fn transform_with_mask_or_channel_target_is_not_exported_as_layer_transform() {
    for target in [json!("mask"), json!({"channel": 0}), json!("quickMask")] {
        let a = Action {
            name: "Probe".into(),
            steps: vec![("edit.transform".into(), json!({"target": target, "dx": 3, "dy": 2, "scaleX": 100, "interpolation": "nearest"}))],
            ..Default::default()
        };
        assert!(export("Targets", &[&a], &[], false).is_err(), "{target} exported");
        let (_, skipped) = export("Targets", &[&a], &[], true).unwrap();
        assert_eq!(skipped, vec![Skipped { action: "Probe".into(), step: 0, id: "edit.transform".into() }]);
    }
    for params in [
        json!({"dx": 3, "dy": 2, "scaleX": 50, "interpolation": "nearest"}),
        json!({"target": "pixels", "dx": 3, "dy": 2, "scaleX": 50, "interpolation": "nearest"}),
    ] {
        let plain = Action { name: "Plain".into(), steps: vec![("edit.transform".into(), params)], ..Default::default() };
        let (bytes, skipped) = export("Targets", &[&plain], &[], false).unwrap();
        assert!(skipped.is_empty());
        let imported = import(&bytes).unwrap();
        assert_eq!(imported.actions[0].steps, vec![("edit.transform".to_string(), json!({"dx": 3, "dy": 2, "scaleX": 50, "interpolation": "nearest"}))]);
    }
    for form in [json!({"quad": [[0, 0], [1, 0], [1, 1], [0, 1]]}), json!({"matrix": [1, 0, 0, 1, 0, 0]}), json!({"rect": [0, 0, 1, 1]})] {
        assert!(transform_to("edit.transform", &form).is_none());
    }
}

/// Apply Image's optional blending mask has no `.atn` mapping: a strict export must refuse the
/// step rather than drop the mask and widen the edit to the whole layer.
#[test]
fn apply_image_with_blending_mask_is_not_exported_without_its_mask() {
    let source = json!({"layer": "Pixels", "channel": "green", "invert": true});
    for params in [json!({"source": source, "blending": "normal", "mask": {"channel": 0}}), json!({"source": source, "blending": "normal", "maskChannel": 0})] {
        let masked = Action { name: "Masked".into(), steps: vec![("image.applyImage".into(), params.clone())], ..Default::default() };
        assert!(export("Audit5", &[&masked], &[], false).is_err(), "{params}");
        let (_, skipped) = export("Audit5", &[&masked], &[], true).unwrap();
        assert_eq!(skipped, vec![Skipped { action: "Masked".into(), step: 0, id: "image.applyImage".into() }]);
    }
    for params in [json!({"source": source, "blending": "normal"}), json!({"source": source, "blending": "normal", "mask": {"channel": "none"}})] {
        let plain = Action { name: "Plain".into(), steps: vec![("image.applyImage".into(), params.clone())], ..Default::default() };
        let (bytes, skipped) = export("Audit5", &[&plain], &[], false).unwrap();
        assert!(skipped.is_empty());
        let imported = import(&bytes).unwrap();
        assert_eq!(imported.actions[0].steps, vec![("image.applyImage".to_string(), json!({"source": source, "blending": "normal"}))]);
    }
}

fn image_size_round_trip(params: &Value) -> Option<Value> {
    let item = export_item("image.imageSize", params, "S")?;
    let e = event_for(&item.event).unwrap();
    match (e.from)(item.descriptor.as_ref().unwrap()) {
        Mapped::Steps(steps) => Some(steps[0].1.clone()),
        _ => panic!("image size descriptor did not import as steps"),
    }
}

/// A step with brush strokes and Sample All Layers is refused by `refineSelectionEdge`; it must
/// not fall through to `smartBrushWorkspace`, which has no field for the strokes either.
#[test]
fn refine_edge_with_strokes_and_sample_all_layers_is_not_exported_without_the_strokes() {
    let stroke = json!({"points": [[5, 4]], "size": 4, "mode": "add"});
    let brushed = Action {
        name: "Probe".into(),
        steps: vec![("select.refineEdge".into(), json!({"radius": 1, "sampleAllLayers": true, "refineStrokes": [stroke]}))],
        ..Default::default()
    };
    assert!(export("Audit", &[&brushed], &[], false).is_err());
    let (_, skipped) = export("Audit", &[&brushed], &[], true).unwrap();
    assert_eq!(skipped, vec![Skipped { action: "Probe".into(), step: 0, id: "select.refineEdge".into() }]);
    let plain = Action {
        name: "Plain".into(),
        steps: vec![("select.refineEdge".into(), json!({"radius": 1, "sampleAllLayers": true, "refineStrokes": []}))],
        ..Default::default()
    };
    assert!(export("Audit", &[&plain], &[], false).is_ok());
}

/// Photoshop's Select and Mask descriptor as mapped has no field for the Refine Edge Brush,
/// so a step with brush strokes must not export as a plain Select and Mask without them.
#[test]
fn refine_edge_with_brush_strokes_is_not_exported_without_them() {
    for mode in ["add", "subtract"] {
        let brushed = Action {
            name: "Probe".into(),
            steps: vec![("select.refineEdge".into(), json!({"radius": 1, "refineStrokes": [{"points": [[5, 4]], "size": 4, "mode": mode}]}))],
            ..Default::default()
        };
        assert!(export("Audit", &[&brushed], &[], false).is_err(), "{mode}");
        let (_, skipped) = export("Audit", &[&brushed], &[], true).unwrap();
        assert_eq!(skipped, vec![Skipped { action: "Probe".into(), step: 0, id: "select.refineEdge".into() }]);
    }
    for params in [json!({"radius": 1}), json!({"radius": 1, "refineStrokes": []}), json!({"radius": 1, "refineStrokes": null})] {
        let plain = Action { name: "Plain".into(), steps: vec![("select.refineEdge".into(), params)], ..Default::default() };
        let (bytes, skipped) = export("Audit", &[&plain], &[], false).unwrap();
        assert!(skipped.is_empty());
        let imported = import(&bytes).unwrap();
        assert_eq!(imported.actions[0].steps, vec![("select.refineEdge".to_string(), json!({"radius": 1}))]);
    }
}

/// Nor does it hold the workspace's tool edits (Quick Selection, Brush, Lasso…), with or without
/// Sample All Layers, so a step with them is reported instead of exported without them.
#[test]
fn refine_edge_with_workspace_edits_is_not_exported_without_them() {
    let edit = json!({"tool": "quick", "points": [[5, 4]], "size": 4});
    for params in [json!({"radius": 1, "edits": [edit]}), json!({"radius": 1, "sampleAllLayers": true, "edits": [{"tool": "invert"}]})] {
        let edited = Action { name: "Probe".into(), steps: vec![("select.refineEdge".into(), params.clone())], ..Default::default() };
        assert!(export("Audit", &[&edited], &[], false).is_err(), "{params}");
        let (_, skipped) = export("Audit", &[&edited], &[], true).unwrap();
        assert_eq!(skipped, vec![Skipped { action: "Probe".into(), step: 0, id: "select.refineEdge".into() }]);
    }
    for params in [json!({"radius": 1, "edits": []}), json!({"radius": 1, "edits": null})] {
        let plain = Action { name: "Plain".into(), steps: vec![("select.refineEdge".into(), params)], ..Default::default() };
        assert!(export("Audit", &[&plain], &[], false).is_ok());
    }
}

#[test]
fn image_size_resolution_only_does_not_gain_keep_print_size() {
    // The .atn cannot tell resolution-only from keepPrintSize:true, so the former must not
    // export as a descriptor that imports back as keep-print-size.
    let params = json!({"resolution": 144, "resample": "nearest"});
    assert_eq!(image_size_round_trip(&params), None);
}

#[test]
fn image_size_intentional_controls_still_round_trip() {
    for params in [
        json!({"resolution": 144.0, "resample": "nearest", "keepPrintSize": true, "scaleStyles": true}),
        json!({"width": 12.0, "height": 10.0, "resolution": 144.0, "resample": "nearest", "scaleStyles": true}),
    ] {
        assert_eq!(image_size_round_trip(&params), Some(params.clone()));
    }
}

#[test]
fn recorded_layer_select_exports_to_atn() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let l = s.execute("layer.new.layer", json!({"name": "Wanted"})).unwrap();
    let wanted_id = l["layer"].as_u64().unwrap();
    s.execute("actions.record", json!({"name": "SelectWanted"})).unwrap();
    s.execute("layer.select", json!({"layer": wanted_id})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();

    let action = s.execute("actions.get", json!({"action": "SelectWanted"})).unwrap();
    assert_eq!(action["steps"][0], json!(["layer.select", {"name": "Wanted"}]));

    // Strict export should succeed and produce bytes without skipUnsupported
    let saved = s.execute("actions.save", json!({"action": "SelectWanted"})).unwrap();
    assert_eq!(saved["steps"], 1);
    assert_eq!(saved["skipped"], json!([]));

    // When exported with skipUnsupported: true, it shouldn't skip the step either
    let saved_skip = s.execute("actions.save", json!({"action": "SelectWanted", "skipUnsupported": true})).unwrap();
    assert_eq!(saved_skip["steps"], 1);
    assert_eq!(saved_skip["skipped"], json!([]));

    // If an action has an occurrence target (duplicate names), it cannot be represented unambiguously in .atn
    let bad_action = Action::new("Dup", vec![("layer.select".into(), json!({"name": "Wanted", "occurrence": 1}))]);
    let err = export("Set", &[&bad_action], &[], false);
    assert!(err.is_err(), "occurrence without representation must fail strict export");
}

#[test]
fn select_rect_modes_export_without_becoming_replace() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();

    // Default or "replace" mode succeeds
    s.actions.list.push(Action::new("ReplaceAct", vec![("select.rect".into(), json!({"x": 4, "y": 4, "width": 2, "height": 2, "mode": "replace"}))]));
    let saved = s.execute("actions.save", json!({"action": "ReplaceAct"})).unwrap();
    assert_eq!(saved["steps"], 1);

    for (mode, event) in [("add", "AddT"), ("subtract", "SbtF"), ("intersect", "IntW")] {
        s.actions.list.push(Action::new(mode, vec![("select.rect".into(), json!({"x":4,"y":4,"width":2,"height":2,"mode":mode}))]));
        let before = s.actions.list.clone();
        let saved = s.execute("actions.save", json!({"action":mode})).unwrap();
        let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
        assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].event.id(), event);
        assert_eq!(import(&bytes).unwrap().actions[0].steps[0].1["mode"], mode);
        assert_eq!(s.actions.list, before);
    }

    s.actions.list.push(Action::new("Unsupported", vec![("select.rect".into(), json!({"x":4,"y":4,"width":2,"height":2,"mode":"xor"}))]));
    let before = s.actions.list.clone();
    assert!(s.execute("actions.save", json!({"action":"Unsupported"})).is_err());
    assert_eq!(s.actions.list, before, "rejected export must leave actions unchanged");

    // With skipUnsupported: true, unrepresentable modes are reported in skipped
    let skipped = s.execute("actions.save", json!({"action": "Unsupported", "skipUnsupported": true})).unwrap();
    assert_eq!(skipped["steps"], 0);
    assert_eq!(skipped["skipped"][0]["id"], "select.rect");
}

#[test]
fn radial_blur_off_center_strict_export_refuses_silent_recentering() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();

    // Default center (omitted) exports successfully
    s.actions.list.push(Action::new("DefaultCenter", vec![("filter.blur.radialBlur".into(), json!({"amount": 50, "method": "spin", "quality": "good"}))]));
    let saved = s.execute("actions.save", json!({"action": "DefaultCenter"})).unwrap();
    assert_eq!(saved["steps"], 1);

    // Explicit (0.5, 0.5) center also exports successfully
    s.actions.list.push(Action::new(
        "ExplicitDefaultCenter",
        vec![("filter.blur.radialBlur".into(), json!({"amount": 50, "method": "spin", "quality": "good", "centerX": 0.5, "centerY": 0.5}))],
    ));
    let saved_exp = s.execute("actions.save", json!({"action": "ExplicitDefaultCenter"})).unwrap();
    assert_eq!(saved_exp["steps"], 1);

    // Off-center radial blur cannot be silently recentered in strict export
    s.actions.list.push(Action::new(
        "OffCenter",
        vec![("filter.blur.radialBlur".into(), json!({"amount": 50, "method": "spin", "quality": "good", "centerX": 0.1, "centerY": 0.2}))],
    ));
    let before_actions = s.actions.list.clone();
    let err = s.execute("actions.save", json!({"action": "OffCenter"}));
    assert!(err.is_err(), "strict export must reject off-center radial blur rather than silently recentering it");
    assert_eq!(s.actions.list, before_actions, "actions list must remain unchanged after rejected save");

    // With skipUnsupported: true, off-center radial blur is skipped
    let skipped = s.execute("actions.save", json!({"action": "OffCenter", "skipUnsupported": true})).unwrap();
    assert_eq!(skipped["steps"], 0);
    assert_eq!(skipped["skipped"][0]["id"], "filter.blur.radialBlur");

    // Radial blur targeting a mask must not export as a plain filter that edits pixels
    s.actions
        .list
        .push(Action::new("MaskTarget", vec![("filter.blur.radialBlur".into(), json!({"amount": 50, "method": "spin", "quality": "good", "target": "mask"}))]));
    let before_actions = s.actions.list.clone();
    let err_mask = s.execute("actions.save", json!({"action": "MaskTarget"}));
    assert!(err_mask.is_err(), "strict export must reject radial blur with non-pixel target");
    assert_eq!(s.actions.list, before_actions, "actions list must remain unchanged after rejected save");

    let skipped_mask = s.execute("actions.save", json!({"action": "MaskTarget", "skipUnsupported": true})).unwrap();
    assert_eq!(skipped_mask["steps"], 0);
    assert_eq!(skipped_mask["skipped"][0]["id"], "filter.blur.radialBlur");

    // Explicit pixels target exports successfully
    s.actions.list.push(Action::new(
        "PixelsTarget",
        vec![("filter.blur.radialBlur".into(), json!({"amount": 50, "method": "spin", "quality": "good", "target": "pixels"}))],
    ));
    let saved_pixels = s.execute("actions.save", json!({"action": "PixelsTarget"})).unwrap();
    assert_eq!(saved_pixels["steps"], 1);
}

/// A crop with W x H x Resolution output options must not export as a plain rectangle crop
/// that keeps the old size and resolution on replay: the mapper has no field for them.
#[test]
fn crop_with_output_size_or_resolution_is_not_exported_as_plain_crop() {
    for params in [
        json!({"x": 1, "y": 1, "width": 4, "height": 3, "targetWidth": 8, "targetHeight": 6, "resolution": 144, "resample": "nearest"}),
        json!({"x": 1, "y": 1, "width": 4, "height": 3, "resolution": 144}),
        json!({"x": 1, "y": 1, "width": 4, "height": 3, "targetWidth": 8, "targetHeight": 6}),
    ] {
        let sized = Action { name: "Probe".into(), steps: vec![("image.crop".into(), params)], ..Default::default() };
        assert!(export("Audit", &[&sized], &[], false).is_err());
        let (_, skipped) = export("Audit", &[&sized], &[], true).unwrap();
        assert_eq!(skipped, vec![Skipped { action: "Probe".into(), step: 0, id: "image.crop".into() }]);
    }
    let plain = Action { name: "Plain".into(), steps: vec![("image.crop".into(), json!({"x": 1, "y": 1, "width": 4, "height": 3}))], ..Default::default() };
    let (bytes, skipped) = export("Audit", &[&plain], &[], false).unwrap();
    assert!(skipped.is_empty());
    let imported = import(&bytes).unwrap();
    assert_eq!(imported.actions[0].steps, vec![("image.crop".to_string(), json!({"x": 1, "y": 1, "width": 4, "height": 3}))]);
}

/// `image.imageSize` with `resample: "none"` ignores width, height and percent; the `.atn`
/// round trip must not turn them into a pixel resize (#7189).
#[test]
fn image_size_resample_none_keeps_ignoring_its_size_through_atn_export() {
    let event = event_for(&export_item("image.imageSize", &json!({"resolution": 72}), "S").expect("exports").event).expect("a known event");
    let back = |p: Value| {
        let item = export_item("image.imageSize", &p, "S")?;
        let Mapped::Steps(steps) = (event.from)(item.descriptor.as_ref().unwrap()) else { panic!("does not import back") };
        let mut back = steps[0].1.clone();
        normalise(&mut back);
        Some(back)
    };
    for sized in [json!({"width": 12, "height": 10}), json!({"width": 12}), json!({"percent": 200}), json!({"width": 12, "percent": 50, "height": 3})] {
        let mut p = sized.clone();
        p["resolution"] = json!(144);
        p["resample"] = json!("none");
        assert_eq!(back(p.clone()), Some(json!({"resolution": 144, "resample": "none", "scaleStyles": true})), "{p}");
    }
    // A step with nothing left to record is refused rather than exported as an empty (unsupported) event.
    assert_eq!(back(json!({"width": 12, "height": 10, "resample": "none"})), None);
    // Real resizes are untouched.
    assert_eq!(
        back(json!({"width": 12, "height": 10, "resample": "bicubic"})),
        Some(json!({"width": 12, "height": 10, "resample": "bicubic", "scaleStyles": true}))
    );
}

/// #8302, #8308: Hue/Saturation's hue is `H   ` in Photoshop's `Hst2`, both ways. It was written
/// and read as `Hue `: exported actions failed in Photoshop, and Photoshop-recorded ones lost
/// their hue on import with no warning. PhotoCraft's own older files (`Hue `) still read.
#[test]
fn hue_saturation_uses_photoshops_hue_key() {
    let params = json!({"colorize": false, "hue": 30, "saturation": 20, "lightness": -7, "greens": {"hue": -12, "saturation": 0, "lightness": 0, "range": [75, 105, 135, 165]}});
    let a = Action::new("H", vec![("image.adjustments.hueSaturation".into(), params.clone())]);
    let (bytes, skipped) = export("S", &[&a], &[], false).unwrap();
    assert!(skipped.is_empty());
    let file = atn::parse(&bytes).unwrap();
    let d = file.actions[0].items[0].descriptor.clone().expect("a descriptor");
    let entries: Vec<Descriptor> =
        get_list(&d, "Adjs").expect("Adjs").iter().filter_map(|e| if let D::Descriptor(e) = e { Some(e.clone()) } else { None }).collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(get_num(&entries[0], "H   "), Some(30.0), "the master hue under H");
    assert_eq!(get_num(&entries[1], "H   "), Some(-12.0), "a range's hue under H");
    assert!(entries.iter().all(|e| get_num(e, "Hue ").is_none()), "no Hue key");

    // A Photoshop-recorded step (H) and an older PhotoCraft one (Hue ) both import with the hue.
    let step = |key: &str| {
        desc("null")
            .with("presetKind", enm("presetKindType", "presetKindCustom"))
            .with("Clrz", D::Boolean(false))
            .with("Adjs", D::List(vec![D::Descriptor(desc("Hst2").with(key, long(30.0)).with("Strt", long(20.0)).with("Lght", long(-7.0)))]))
    };
    for key in ["H   ", "Hue "] {
        let e = event_for(&item("HStr", "", None).event).expect("HStr imports");
        match (e.from)(&step(key)) {
            Mapped::Steps(steps) => {
                assert_eq!(steps[0].0, "image.adjustments.hueSaturation");
                assert_eq!(steps[0].1["hue"], json!(30.0), "{key:?}: {}", steps[0].1);
                assert_eq!(steps[0].1["saturation"], json!(20.0));
            }
            Mapped::Unsupported { .. } => panic!("{key:?} is unsupported"),
        }
    }
    // Through a whole file, saved and loaded back: the hue survives.
    let back = import(&bytes).unwrap();
    let mut p = back.actions[0].steps[0].1.clone();
    normalise(&mut p);
    assert_eq!(p["hue"], json!(30), "{p}");
}

/// #6745: a recorded Camera Raw Filter step (`Adobe Camera Raw Filter`, the layout of 62 real
/// recordings) imports as `filter.cameraRaw` with its sliders, colour mixer, grading wheels,
/// vignette style and point curves, and plays. One that uses the Calibration panel (or noise
/// settings the command lacks) stays a Photoshop step rather than replaying a different look.
#[test]
fn camera_raw_filter_steps_import_as_filter_camera_raw() {
    let int = |v: i32| D::Integer(v);
    // Every step carries the version keys a recording has (a verified process version).
    let cr = || desc("null").with("CrVe", D::Text(UnicodeString::new_nul("16.1.1"))).with("PrVN", int(5)).with("PrVe", int(184_549_376));
    let mut d = cr()
        .with("WBal", enm("WBal", "Cst "))
        .with("Temp", int(12))
        .with("Tint", int(-4))
        .with("Ex12", D::Double(0.35))
        .with("Cr12", int(10))
        .with("Hi12", int(-30))
        .with("Sh12", int(25))
        .with("Cl12", int(8))
        .with("Vibr", int(15))
        .with("PC_1", int(20))
        .with("PC_2", int(50))
        .with("PC_3", int(80))
        .with("STSH", int(210))
        .with("STSS", int(12))
        .with("CgSL", int(0))
        .with("CgBl", int(60))
        .with("STB ", int(-10))
        .with("Shrp", int(40))
        .with("ShpR", D::Double(1.2))
        .with("LNR ", int(20))
        .with("LNRC", int(0))
        .with("CNR ", int(25))
        .with("CNRS", int(50))
        .with("PCVA", int(-20))
        .with("PCVS", int(2))
        .with("Crv ", D::List([0, 0, 64, 50, 255, 255].map(int).to_vec()))
        .with("CrvR", D::List([0, 0, 255, 255].map(int).to_vec()));
    for (k, v) in ["R", "O", "Y", "G", "A", "B", "P", "M"].iter().zip(1..) {
        d = d.with(&format!("HA_{k}"), int(v));
    }
    d = d.with("SA_O", int(-15));
    let calibrated = cr().with("Ex12", D::Double(0.2)).with("RHue", int(8));
    let smoothed = cr().with("CNR ", int(25)).with("CNRS", int(70));
    let auto_wb = cr().with("WBal", enm("WBal", "Atmt")).with("Temp", int(5));
    let partial_splits = cr().with("PC_1", int(20));
    let odd_curve = cr().with("Crv ", int(3));
    // Review of #8714: values that would replay differently, or fail to play, stay Photoshop steps.
    let reversed_splits = cr().with("PC_1", int(80)).with("PC_2", int(50)).with("PC_3", int(20));
    let split_past_100 = cr().with("PC_1", int(20)).with("PC_2", int(50)).with("PC_3", int(120));
    let one_point_curve = cr().with("Crv ", D::List([0, 0].map(int).to_vec()));
    let unordered_curve = cr().with("CrvG", D::List([0, 0, 128, 90, 64, 50, 255, 255].map(int).to_vec()));
    let text_contrast = cr().with("Cr12", D::Text(UnicodeString::new_nul("ten")));
    let text_hue = cr().with("HA_R", D::Text(UnicodeString::new_nul("red")));
    let text_grade = cr().with("STSH", D::Text(UnicodeString::new_nul("blue")));
    let exposure_past_5 = cr().with("Ex12", D::Double(6.0));
    let negative_sharpening = cr().with("Shrp", int(-10));
    // CodeRabbit on batch 11: a process version the PSD mapping hasn't verified renders differently.
    let unknown_version =
        desc("null").with("CrVe", D::Text(UnicodeString::new_nul("16.1.1"))).with("PrVN", int(4)).with("PrVe", int(184_549_376)).with("Ex12", D::Double(0.2));
    let no_version_number = desc("null").with("CrVe", D::Text(UnicodeString::new_nul("16.1.1"))).with("PrVe", int(184_549_376)).with("Ex12", D::Double(0.2));
    let mismatched_pair =
        desc("null").with("CrVe", D::Text(UnicodeString::new_nul("16.1.1"))).with("PrVN", int(6)).with("PrVe", int(184_549_376)).with("Ex12", D::Double(0.2));
    let no_camera_raw_version = desc("null").with("PrVN", int(5)).with("PrVe", int(184_549_376)).with("Ex12", D::Double(0.2));
    let items = vec![
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(d)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(calibrated)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(smoothed)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(auto_wb)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(partial_splits)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(odd_curve)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(reversed_splits)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(split_past_100)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(one_point_curve)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(unordered_curve)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(text_contrast)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(text_hue)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(text_grade)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(exposure_past_5)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(negative_sharpening)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(unknown_version)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(no_version_number)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(mismatched_pair)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(no_camera_raw_version)),
    ];
    let file = AtnFile {
        name: UnicodeString::new_nul("Looks"),
        expanded: false,
        actions: vec![AtnAction { name: UnicodeString::new_nul("Warm"), items, ..Default::default() }],
        warnings: vec![],
        ..Default::default()
    };
    let imp = import(&atn::write(&file).unwrap()).unwrap();
    let steps = &imp.actions[0].steps;
    let (id, p) = &steps[0];
    assert_eq!(id, "filter.cameraRaw");
    // The recorded descriptor rides along, so Save Actions can write the step back.
    let mut p = p.clone();
    assert!(p.as_object_mut().unwrap().remove("__atnDescriptor").is_some(), "{p}");
    assert_eq!(
        p,
        json!({
            "temperature": 12, "tint": -4, "exposure": 0.35, "contrast": 10, "highlights": -30, "shadows": 25,
            "clarity": 8, "vibrance": 15, "curveSplits": [20, 50, 80],
            "hslHue": [1, 2, 3, 4, 5, 6, 7, 8], "hslSat": [0, -15, 0, 0, 0, 0, 0, 0],
            "gradeShadows": {"hue": 210, "sat": 12, "lum": 0}, "gradeBlending": 60, "gradeBalance": -10,
            "sharpenAmount": 40, "sharpenRadius": 1.2, "noiseLuminance": 20, "noiseColor": 25,
            "vignetteAmount": -20, "vignetteStyle": "colorPriority", "pointCurve": [[0, 0], [64, 50], [255, 255]]
        })
    );
    for (k, (id, _)) in steps.iter().enumerate().skip(1) {
        assert_eq!(id, PHOTOSHOP_STEP, "step {k}");
    }
    assert_eq!(imp.unsupported.iter().map(|u| u.step).collect::<Vec<_>>(), (1..=18).collect::<Vec<_>>());
    // The imported step plays.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
    s.execute(id, p.clone()).unwrap();
}

/// Review of #8714: a Camera Raw Filter step that was loaded saves back as the recorded step,
/// byte for byte, as it did while it was a kept Photoshop step; once edited (or when PhotoCraft
/// recorded it) Save Actions refuses it as before. The loaded step plays.
#[test]
fn loaded_camera_raw_filter_steps_save_back_unchanged() {
    let recorded = desc("null")
        .with("CrVe", D::Text(UnicodeString::new_nul("16.1.1")))
        .with("PrVN", D::Integer(5))
        .with("PrVe", D::Integer(184_549_376))
        .with("WBal", enm("WBal", "Cst "))
        .with("Ex12", D::Double(0.35))
        .with("Cr12", D::Integer(10))
        .with("Crv ", D::List([0, 0, 64, 50, 255, 255].map(D::Integer).to_vec()));
    let file = one_action(vec![item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(recorded.clone()))]);
    let mut s = Session::new();
    let loaded = s.execute("actions.load", json!({"data": b64(&atn::write(&file).unwrap())})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]), "{loaded}");
    assert_eq!(s.actions.list[0].steps[0].0, "filter.cameraRaw");
    let saved_step = |r: &Value| {
        let saved = atn::parse(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap();
        saved.actions[0].items.first().map(|i| (i.event.clone(), i.dictionary.clone(), i.descriptor.clone()))
    };
    // Strict save, and the Actions panel's save (which skips what can't be written). The step's
    // dictionary is stored action data, so it keeps Photoshop's wording and not the menu label
    // ("RAW Develop Filter"): a loaded step reads the same in Photoshop's Actions panel.
    for skip in [false, true] {
        let r = s.execute("actions.save", json!({"action": "A", "skipUnsupported": skip})).unwrap();
        assert_eq!(r["skipped"], json!([]), "{r}");
        let expected = (AtnEvent::name("Adobe Camera Raw Filter"), b"Camera Raw Filter".to_vec(), Some(recorded.clone()));
        assert_eq!(saved_step(&r), Some(expected));
    }
    // It plays.
    s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
    let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert_eq!(played["ran"], 1, "{played}");
    assert!(played.get("failed").is_none(), "{played}");
    // Edited, it no longer matches what was recorded.
    let original = s.actions.list[0].steps[0].1.clone();
    s.actions.list[0].steps[0].1["exposure"] = json!(1.5);
    let e = s.execute("actions.save", json!({"action": "A"})).unwrap_err().to_string();
    assert!(e.contains("filter.cameraRaw"), "{e}");
    // Recorded by PhotoCraft: no descriptor to write back.
    s.actions.list[0].steps[0].1 = original;
    s.actions.list[0].steps[0].1.as_object_mut().unwrap().remove("__atnDescriptor");
    let r = s.execute("actions.save", json!({"action": "A", "skipUnsupported": true})).unwrap();
    assert_eq!(r["skipped"].as_array().unwrap().len(), 1, "{r}");
}

/// Review of #8714: a Camera Raw Filter step with a key the table doesn't map (an Upright
/// setting, a profile name, any other panel) or with no settings at all stays a Photoshop step
/// and is reported, rather than replaying without that processing.
#[test]
fn camera_raw_filter_steps_with_unknown_keys_stay_photoshop_steps() {
    let cr = || desc("null").with("CrVe", D::Text(UnicodeString::new_nul("16.1.1"))).with("PrVN", D::Integer(5)).with("PrVe", D::Integer(184_549_376));
    let upright = cr().with("Ex12", D::Double(0.2)).with("PerV", D::Integer(20));
    let profile = cr().with("Cr12", D::Integer(10)).with("LookName", D::Text(UnicodeString::new_nul("B&W 05")));
    let file = one_action(vec![
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(upright.clone())),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(profile)),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", None),
        item("Adobe Camera Raw Filter", "Camera Raw Filter", Some(cr().with("Ex12", D::Double(0.2)))),
    ]);
    let mut s = Session::new();
    let loaded = s.execute("actions.load", json!({"data": b64(&atn::write(&file).unwrap())})).unwrap();
    let unsupported: Vec<u64> = loaded["unsupported"].as_array().unwrap().iter().map(|u| u["step"].as_u64().unwrap()).collect();
    assert_eq!(unsupported, [0, 1, 2], "{loaded}");
    let ids: Vec<&str> = s.actions.list[0].steps.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, [PHOTOSHOP_STEP, PHOTOSHOP_STEP, PHOTOSHOP_STEP, "filter.cameraRaw"]);
    // Kept, it still saves back as recorded.
    let r = s.execute("actions.save", json!({"action": "A"})).unwrap();
    let saved = atn::parse(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(saved.actions[0].items[0].descriptor, Some(upright));
}

fn one_action(items: Vec<AtnItem>) -> AtnFile {
    AtnFile {
        name: UnicodeString::new_nul("Saves"),
        expanded: false,
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        warnings: vec![],
        ..Default::default()
    }
}

fn save_as(format: &str, path: &str) -> Descriptor {
    desc("null").with("As  ", D::Descriptor(desc(format))).with("In  ", path_value(path)).with("DocI", long(53.0))
}

#[test]
fn save_as_to_a_file_imports_with_its_destination() {
    let file = one_action(vec![
        item("save", "Save", Some(save_as("PNGF", "/out/result.png"))),
        item("save", "Save", Some(save_as("Google WebPShop", "C:\\out\\result.webp").with("Cpy ", D::Boolean(true)))),
        item("save", "Save", Some(save_as("JPEG", "/out/result.jpeg"))),
        item("save", "Save", Some(save_as("WebPFormat", "/out/result.webp"))),
        item("save", "Save", None),
        item("save", "Save", Some(desc("null").with("DocI", long(53.0)))),
    ]);
    let imp = import(&atn::write(&file).unwrap()).unwrap();
    assert_eq!(
        imp.actions[0].steps,
        [
            ("file.saveAs".to_string(), json!({"path": "/out/result.png"})),
            ("file.saveACopy".to_string(), json!({"path": "C:\\out\\result.webp"})),
            ("file.saveAs".to_string(), json!({"path": "/out/result.jpeg"})),
            ("file.saveAs".to_string(), json!({"path": "/out/result.webp"})),
            ("file.save".to_string(), json!({})),
            ("file.save".to_string(), json!({})),
        ]
    );
    assert!(imp.unsupported.is_empty());
}

#[test]
fn save_as_that_cannot_be_mapped_stops_playback_instead_of_saving_over_the_file() {
    // Before #7102 each of these imported as a plain File › Save, which overwrote the file the
    // action was played on.
    let file = one_action(vec![
        item("save", "Save", Some(save_as("PNGF", "/out/folder"))),
        // Save a Copy into a folder under the document's own name has no PhotoCraft step.
        item("save", "Save", Some(save_as("PNGF", "/out").with("DIDr", D::Boolean(true)).with("Cpy ", D::Boolean(true)))),
        item("save", "Save", Some(save_as("Not a format", "/out").with("DIDr", D::Boolean(true)))),
        item("save", "Save", Some(save_as("PNGF", "/out/result.webp"))),
        item("save", "Save", Some(save_as("GIFf", "/out/result.gif"))),
        item("save", "Save", Some(desc("null").with("As  ", D::Descriptor(desc("PNGF"))))),
        // A format that isn't a descriptor, with and without a destination.
        item("save", "Save", Some(desc("null").with("As  ", long(1.0)))),
        item("save", "Save", Some(desc("null").with("As  ", long(1.0)).with("In  ", path_value("/out/result.png")))),
        // A destination with no format at all.
        item("save", "Save", Some(desc("null").with("In  ", path_value("/out/result.png")))),
    ]);
    let imp = import(&atn::write(&file).unwrap()).unwrap();
    let ids: Vec<&str> = imp.actions[0].steps.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, [PHOTOSHOP_STEP; 9]);
    assert!(imp.actions[0].disabled.is_empty(), "kept saves stay on, so playback stops on them");
    // The kept steps are written back byte for byte.
    let a = &imp.actions[0];
    let (bytes, skipped) = export("Saves", &[a], &[], false).unwrap();
    assert!(skipped.is_empty());
    let back = atn::parse(&bytes).unwrap();
    for (out, orig) in back.actions[0].items.iter().zip(&file.actions[0].items) {
        assert_eq!(out.descriptor, orig.descriptor);
    }
}

#[test]
fn save_as_into_a_folder_imports_as_a_folder_step_and_saves_back() {
    // #5305: "save in directory" (`DIDr`), recorded when Photoshop's suggested name is kept:
    // `In` is the folder, and each played document saves under its own name there.
    let file = one_action(vec![
        item("save", "Save", Some(save_as("PNGF", "/out/web").with("DIDr", D::Boolean(true)))),
        item("save", "Save", Some(save_as("JPEG", "C:\\shots").with("DIDr", D::Boolean(true)))),
    ]);
    let imp = import(&atn::write(&file).unwrap()).unwrap();
    let steps = vec![
        ("file.saveAs".to_string(), json!({"folder": "/out/web", "extension": "png"})),
        ("file.saveAs".to_string(), json!({"folder": "C:\\shots", "extension": "jpg"})),
    ];
    assert_eq!(imp.actions[0].steps, steps);
    assert!(imp.unsupported.is_empty());
    // Saved back as the same "save in directory" steps.
    let (bytes, skipped) = export("Saves", &[&imp.actions[0]], &[], false).unwrap();
    assert!(skipped.is_empty());
    let back = atn::parse(&bytes).unwrap();
    for (out, orig) in back.actions[0].items.iter().zip(&file.actions[0].items) {
        let (out, orig) = (out.descriptor.as_ref().unwrap(), orig.descriptor.as_ref().unwrap());
        for key in ["As  ", "In  ", "DIDr"] {
            assert_eq!(out.get(key), orig.get(key), "{key}");
        }
    }
    assert_eq!(import(&bytes).unwrap().actions[0].steps, steps);
    // A folder step without its extension, or as a copy, has no Photoshop event.
    let odd = Action::new("B", vec![("file.saveAs".to_string(), json!({"folder": "/out"}))]);
    assert!(export("Saves", &[&odd], &[], false).is_err());
    let copy = Action::new("C", vec![("file.saveACopy".to_string(), json!({"folder": "/out", "extension": "png"}))]);
    assert!(export("Saves", &[&copy], &[], false).is_err());
}

#[test]
fn saves_round_trip_through_photoshop_events() {
    let steps = vec![
        ("file.saveAs".to_string(), json!({"path": "/out/result.psd"})),
        ("file.saveACopy".to_string(), json!({"path": "/out/copy.webp"})),
        ("file.save".to_string(), json!({})),
    ];
    let a = Action::new("A", steps.clone());
    let (bytes, skipped) = export("Saves", &[&a], &[], false).unwrap();
    assert!(skipped.is_empty());
    let file = atn::parse(&bytes).unwrap();
    let items = &file.actions[0].items;
    assert!(items.iter().all(|i| i.event.id() == "save"));
    let first = items[0].descriptor.as_ref().unwrap();
    assert_eq!(get_obj(first, "As  ").map(|f| trimmed(&f.class_id)).as_deref(), Some("Pht3"));
    assert_eq!(first.get("In  ").and_then(path_text).as_deref(), Some("/out/result.psd"));
    assert_eq!(get_bool(items[1].descriptor.as_ref().unwrap(), "Cpy "), Some(true));
    // Photoshop's own WebP class, not the plug-in's: Photoshop 2026 rejects `Google WebPShop`.
    assert_eq!(get_obj(items[1].descriptor.as_ref().unwrap(), "As  ").map(|f| trimmed(&f.class_id)).as_deref(), Some("WebPFormat"));
    assert_eq!(import(&bytes).unwrap().actions[0].steps, steps);

    // A Save As PhotoCraft can't express in Photoshop's terms is skipped, never exported as a
    // plain Save.
    let odd = Action::new("B", vec![("file.saveAs".to_string(), json!({"path": "/out/result.xcf"}))]);
    assert!(export("Saves", &[&odd], &[], false).is_err());
}

#[test]
fn long_names_are_clipped_instead_of_copied_into_every_diagnostic() {
    let long = "N".repeat(5000);
    let unknown = |n: usize| (0..n).map(|_| item("zzzz", "Mystery", None)).collect::<Vec<_>>();
    let file = AtnFile {
        name: UnicodeString::new_nul(&long),
        expanded: false,
        actions: vec![AtnAction { name: UnicodeString::new_nul(&long), items: unknown(50), ..Default::default() }],
        warnings: vec![],
        ..Default::default()
    };
    let imp = import(&atn::write(&file).unwrap()).unwrap();
    assert_eq!(imp.set.chars().count(), MAX_NAME_CHARS);
    assert_eq!(imp.actions[0].name.chars().count(), MAX_NAME_CHARS);
    assert_eq!(imp.actions[0].set.as_deref().map(|s| s.chars().count()), Some(MAX_NAME_CHARS));
    assert_eq!(imp.unsupported.len(), 50);
    assert!(imp.unsupported.iter().all(|u| u.action.chars().count() == MAX_NAME_CHARS));
}

/// A Filter Gallery step as Photoshop records it: `GEfc` with the effect in `GEfk`, or the
/// effect's own event carrying the same key.
fn gallery_step(event: &str, effect: &str, d: Descriptor) -> Mapped {
    let d = d.with("GEfk", enm("GEft", effect));
    let event = AtnEvent::Code(event.as_bytes().try_into().unwrap());
    (events::event_for(&event).unwrap_or_else(|| panic!("{effect} is unknown")).from)(&d)
}

fn gallery_params(m: Mapped) -> (String, Value) {
    match m {
        Mapped::Steps(mut s) if s.len() == 1 => s.remove(0),
        _ => panic!("kept"),
    }
}

/// The Filter Gallery effects load with Photoshop's defaults for missing keys and its clamping,
/// from `GEfc` or from the effect's own event, play, and save back as `GEfc` with every key in
/// recorded order (#10592).
#[test]
fn gallery_effect_steps_load_clamp_play_and_save_back() {
    let edges = desc("null").with("EdgW", D::Integer(3)).with("EdgB", D::Integer(20)).with("Smth", D::Integer(7));
    assert_eq!(
        gallery_params(gallery_step("GEfc", "AccE", edges.clone())),
        ("filter.gallery.accentedEdges".into(), json!({"edgeWidth": 3, "edgeBrightness": 20, "smoothness": 7}))
    );
    // Run from its own menu item, the effect's id is the event.
    assert_eq!(gallery_params(gallery_step("AccE", "AccE", edges.clone())).0, "filter.gallery.accentedEdges");
    // Missing keys take Photoshop's defaults; enumerations, booleans and colours map.
    assert_eq!(gallery_params(gallery_step("GEfc", "BsRl", desc("null"))).1, json!({"detail": 13, "smoothness": 3, "light": "bottom"}));
    let pastels = desc("null").with("TxtT", enm("TxtT", "TxBr")).with("LghD", enm("LghD", "LDTR")).with("InvT", D::Boolean(true));
    let p = gallery_params(gallery_step("GEfc", "RghP", pastels)).1;
    assert_eq!((p["texture"].as_str(), p["light"].as_str(), p["invert"].as_bool()), (Some("brick"), Some("topRight"), Some(true)));
    let glow = desc("null").with("Clr ", D::Descriptor(color_desc("#33cc66").unwrap()));
    assert_eq!(gallery_params(gallery_step("GEfc", "NGlw", glow)).1["glowColor"], "#33cc66");
    // Out-of-range values are clamped as Photoshop clamps them.
    let wide = desc("null").with("EdgW", D::Integer(99)).with("EdgB", D::Integer(-5));
    let p = gallery_params(gallery_step("GEfc", "AccE", wide)).1;
    assert_eq!((p["edgeWidth"].as_i64(), p["edgeBrightness"].as_i64()), (Some(14), Some(0)));

    // Saved back as `GEfc`, with `GEfk` first and every key in recorded order.
    let item = export_item("filter.gallery.accentedEdges", &json!({"edgeWidth": 3, "edgeBrightness": 20, "smoothness": 7}), "S").unwrap();
    assert_eq!(item.event.id(), "GEfc");
    let mut expected = vec![(Id::new("GEfk"), enm("GEft", "AccE"))];
    expected.extend(edges.items);
    assert_eq!(item.descriptor.unwrap().items, expected);

    // They play.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 24, "height": 24})).unwrap();
    s.execute("filter.render.clouds", json!({})).unwrap();
    for (code, effect) in [("AccE", "accentedEdges"), ("Ct  ", "cutout"), ("Plst", "plaster")] {
        let (id, p) = gallery_params(gallery_step("GEfc", code, desc("null")));
        assert_eq!(id, format!("filter.gallery.{effect}"));
        s.actions.list.push(Action::new(code, vec![(id, p)]));
        let played = s.execute("actions.play", json!({"action": code})).unwrap();
        assert_eq!(played["ran"], 1, "{code}: {played}");
    }
}

/// What the effects cannot play keeps the Photoshop step: an unknown effect or value, a Glass
/// texture the reference gives no code for, a key the effect does not have (#10592).
#[test]
fn gallery_effect_steps_the_commands_cannot_play_are_kept() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    assert!(kept(gallery_step("GEfc", "Zzzz", desc("null"))));
    assert!(kept(gallery_step("GEfc", "BsRl", desc("null").with("LghD", enm("LghD", "LDCn")))));
    assert!(kept(gallery_step("GEfc", "Gls ", desc("null").with("TxtT", enm("TxtT", "TxBr")))));
    assert!(kept(gallery_step("GEfc", "AccE", desc("null").with("Rds ", D::Integer(3)))));
}

/// What the event cannot hold refuses the step: a fraction in an integer key, the colours a
/// Sketch effect pinned, a value with no Photoshop code, a mask target (#10592).
#[test]
fn gallery_effect_steps_that_cannot_be_written_are_refused() {
    for (id, params) in [
        ("filter.gallery.accentedEdges", json!({"edgeWidth": 2.5})),
        ("filter.gallery.charcoal", json!({"foreground": "#000000"})),
        ("filter.gallery.glass", json!({"texture": "blocks"})),
        ("filter.gallery.cutout", json!({"target": "mask"})),
    ] {
        assert!(export_item(id, &params, "S").is_none(), "{id} {params} exported");
    }
    assert!(export_item("filter.gallery.glass", &json!({"texture": "frosted"}), "S").is_some());
}

/// An integer outside the setting's range refuses the step rather than saving a value Photoshop
/// clamps on playback and that loads back as another (`edgeWidth` 99 as 14); each end of a range
/// still saves (#10592).
#[test]
fn gallery_effect_integers_outside_the_settings_range_are_refused() {
    for (id, params) in [
        ("filter.gallery.accentedEdges", json!({"edgeWidth": 99})),
        ("filter.gallery.accentedEdges", json!({"edgeWidth": 0})),
        ("filter.gallery.neonGlow", json!({"glowSize": -25, "glowColor": "#336699"})),
        ("filter.gallery.glass", json!({"scaling": 201})),
    ] {
        assert!(export_item(id, &params, "S").is_none(), "{id} {params} exported");
    }
    for (id, params) in [
        ("filter.gallery.accentedEdges", json!({"edgeWidth": 1})),
        ("filter.gallery.accentedEdges", json!({"edgeWidth": 14})),
        ("filter.gallery.neonGlow", json!({"glowSize": -24, "glowColor": "#336699"})),
        ("filter.gallery.glass", json!({"scaling": 200})),
    ] {
        assert!(export_item(id, &params, "S").is_some(), "{id} {params} refused");
    }
}

/// Neon Glow without a colour plays with the effect's default glow colour, so it saves with that
/// colour, component for component (#10592).
#[test]
fn gallery_neon_glow_without_a_colour_saves_the_colour_it_plays_with() {
    let item = export_item("filter.gallery.neonGlow", &json!({"glowSize": 5}), "S").expect("refused");
    let d = item.descriptor.unwrap();
    let c = get_obj(&d, "Clr ").expect("no colour");
    assert_eq!(trimmed(&c.class_id), "RGBC");
    let neon = photocraft_algo::GalleryEffect::new(photocraft_algo::GalleryFilter::NeonGlow).color;
    for (key, v) in [("Rd  ", neon[0]), ("Grn ", neon[1]), ("Bl  ", neon[2])] {
        assert_eq!(get_num(c, key), Some(f64::from(v) * 255.0), "{key}");
    }
}

/// Each enumeration saves with the type Photoshop's terminology gives it: stroke direction's is
/// `StrD`, not its key, and texture's is `TxtT`, as Photoshop records Glass (#10592).
#[test]
fn gallery_effect_enumerations_save_with_photoshops_type_ids() {
    let enum_type = |id: &str, params: Value, key: &str| match export_item(id, &params, "S").unwrap().descriptor.unwrap().get(key) {
        Some(D::Enumerated { type_id, .. }) => trimmed(type_id),
        other => panic!("{id} {key}: {other:?}"),
    };
    assert_eq!(enum_type("filter.gallery.graphicPen", json!({}), "SDir"), "StrD");
    assert_eq!(enum_type("filter.gallery.sprayedStrokes", json!({}), "SDir"), "StrD");
    for id in ["filter.gallery.conteCrayon", "filter.gallery.glass", "filter.gallery.roughPastels", "filter.gallery.texturizer", "filter.gallery.underpainting"]
    {
        assert_eq!(enum_type(id, json!({}), "TxtT"), "TxtT", "{id}");
    }
    assert_eq!(enum_type("filter.gallery.basRelief", json!({}), "LghD"), "LghD");
    assert_eq!(enum_type("filter.gallery.plaster", json!({}), "LghP"), "LghP");
    assert_eq!(enum_type("filter.gallery.halftonePattern", json!({}), "ScrT"), "ScrT");
    assert_eq!(enum_type("filter.gallery.paintDaubs", json!({}), "BrsT"), "BrsT");
    assert_eq!(enum_type("filter.gallery.grain", json!({}), "Grnt"), "Grnt");
}

/// Neon Glow recorded without `Clr ` glows in Photoshop's default colour, blue (0, 0, 255) per
/// the event reference, not PhotoCraft's own default: the step loads with that colour and saves
/// it back, loading again as the same step (#10592).
#[test]
fn gallery_neon_glow_without_a_colour_loads_photoshops_blue() {
    let (id, p) = gallery_params(gallery_step("GEfc", "NGlw", desc("null")));
    assert_eq!(id, "filter.gallery.neonGlow");
    assert_eq!(p, json!({"glowSize": 5, "glowBrightness": 15, "glowColor": "#0000ff"}));
    let d = export_item(&id, &p, "S").expect("refused").descriptor.unwrap();
    let c = get_obj(&d, "Clr ").expect("no colour");
    assert_eq!(trimmed(&c.class_id), "RGBC");
    assert_eq!([get_num(c, "Rd  "), get_num(c, "Grn "), get_num(c, "Bl  ")], [Some(0.0), Some(0.0), Some(255.0)]);
    let event = AtnEvent::Code(*b"GEfc");
    let back = (events::event_for(&event).unwrap().from)(&d);
    assert_eq!(gallery_params(back), (id, p));
}

/// An effect recorded under its own event by string ID (`cutout`, as newer versions record it)
/// maps like its 4-char code; only the effect's string IDs count as names (#10592).
#[test]
fn gallery_effects_recorded_by_string_id_map_like_their_codes() {
    for (code, name, effect) in [("Ct  ", "cutout", "Ct"), ("HlfS", "halftoneScreen", "HlfS"), ("Grn ", "grain", "Grn")] {
        let d = desc("null").with("GEfk", enm("GEft", effect));
        let by_name = (events::event_for(&AtnEvent::name(name)).unwrap_or_else(|| panic!("{name} by name")).from)(&d);
        let by_code = (events::event_for(&AtnEvent::Code(code.as_bytes().try_into().unwrap())).unwrap().from)(&d);
        assert_eq!(gallery_params(by_name), gallery_params(by_code), "{name}");
    }
    assert!(events::event_for(&AtnEvent::name("Ct  ")).is_none());
    assert!(events::event_for(&AtnEvent::Code(*b"MscT")).is_some());
}

/// A step recorded as the effect's own event (by code or string ID) loads, plays and saves back
/// as that event with its panel name and descriptor unchanged; a `GEfc` step stays `GEfc`
/// (#10592).
#[test]
fn gallery_effect_steps_save_back_as_the_event_they_were_recorded_as() {
    let edges = desc("null").with("GEfk", enm("GEft", "AccE")).with("EdgW", D::Integer(2)).with("EdgB", D::Integer(38)).with("Smth", D::Integer(5));
    let cutout = desc("null").with("GEfk", enm("GEft", "Ct")).with("NmbL", D::Integer(6)).with("EdgS", D::Integer(4)).with("EdgF", D::Integer(2));
    let gallery = desc("null").with("GEfk", enm("GEft", "AccE")).with("EdgW", D::Integer(3)).with("EdgB", D::Integer(20)).with("Smth", D::Integer(7));
    let source = vec![item("AccE", "Accented Edges", Some(edges)), item("cutout", "Cutout", Some(cutout)), item("GEfc", "Filter Gallery", Some(gallery))];
    let data = set_of("S", vec![("A", source.clone())]);
    let mut s = Session::new();
    let loaded = s.execute("actions.load", json!({"data": data})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]), "{loaded}");
    let ids: Vec<&str> = s.actions.list[0].steps.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["filter.gallery.accentedEdges", "filter.gallery.cutout", "filter.gallery.accentedEdges"]);

    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("filter.render.clouds", json!({})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert_eq!(played["ran"], 3, "{played}");
    // The remembered event is for Save Actions only: playback strips it, so the journal (Last
    // Filter) and the smart filters (`.pcraft` saves) never carry it.
    let gallery: Vec<&Value> = s.journal.iter().filter(|(id, _)| id.starts_with("filter.gallery.")).map(|(_, p)| p).collect();
    assert_eq!(gallery.len(), 3, "{:?}", s.journal);
    assert!(gallery.iter().all(|p| p.get("_atnEvent").is_none()), "{gallery:?}");
    let d = s.active().unwrap();
    let photocraft_doc::LayerContent::Smart(sm) = &d.doc.layer(d.active_layer.unwrap()).unwrap().content else { panic!("not smart") };
    assert_eq!(sm.smart_filters.len(), 3);
    assert!(sm.smart_filters.iter().all(|f| f.params.get("_atnEvent").is_none()), "{:?}", sm.smart_filters);

    let saved = s.execute("actions.save", json!({"set": "S"})).unwrap();
    assert_eq!(saved["skipped"], json!([]));
    let file = atn::parse(&photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap()).unwrap();
    let got: Vec<_> = file.actions[0].items.iter().map(|i| (i.event.clone(), i.dictionary.clone(), i.descriptor.clone())).collect();
    let want: Vec<_> = source.iter().map(|i| (i.event.clone(), i.dictionary.clone(), i.descriptor.clone())).collect();
    assert_eq!(got, want);

    // A step made in PhotoCraft keeps saving as `GEfc`, and so does a forged event param.
    assert_eq!(export_item("filter.gallery.cutout", &json!({}), "S").unwrap().event, AtnEvent::Code(*b"GEfc"));
    let forged = json!({"_atnEvent": {"event": "GsnB", "eventKind": "code", "name": "Gaussian Blur"}});
    assert_eq!(export_item("filter.gallery.cutout", &forged, "S").unwrap().event, AtnEvent::Code(*b"GEfc"));
    // Another effect's event (stale or forged) does not relabel the step either.
    for (event, kind) in [("AccE", "code"), ("accentedEdges", "name")] {
        let other = json!({"_atnEvent": {"event": event, "eventKind": kind, "name": "Accented Edges"}});
        let item = export_item("filter.gallery.cutout", &other, "S").unwrap();
        assert_eq!((item.event, item.dictionary), (AtnEvent::Code(*b"GEfc"), b"Filter Gallery".to_vec()), "{event}");
    }
    let own = json!({"_atnEvent": {"event": "cutout", "eventKind": "name", "name": "Cutout"}});
    assert_eq!(export_item("filter.gallery.cutout", &own, "S").unwrap().event, AtnEvent::name("cutout"));
}

/// Only a missing key takes Photoshop's default: a key there with the wrong type or a non-finite
/// number keeps the Photoshop step instead of playing a default (#10592).
#[test]
fn gallery_effect_keys_with_the_wrong_type_keep_the_step() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    assert!(kept(gallery_step("GEfc", "Txtz", desc("null").with("InvT", D::Integer(1)))));
    assert!(kept(gallery_step("GEfc", "AccE", desc("null").with("EdgW", D::Double(f64::NAN)))));
    assert!(kept(gallery_step("GEfc", "AccE", desc("null").with("EdgW", D::Double(f64::INFINITY)))));
    assert!(kept(gallery_step("GEfc", "AccE", desc("null").with("EdgW", text("9")))));
    assert!(kept(gallery_step("GEfc", "BsRl", desc("null").with("LghD", D::Integer(3)))));
    assert!(kept(gallery_step("GEfc", "NGlw", desc("null").with("Clr ", D::Integer(3)))));
    // Missing keys still take the defaults.
    assert_eq!(gallery_params(gallery_step("GEfc", "Txtz", desc("null"))).1["invert"], false);
    assert_eq!(gallery_params(gallery_step("GEfc", "AccE", desc("null").with("EdgB", D::Integer(9)))).1["edgeWidth"], 2);
}

/// Photoshop's Split Channels, Merge Spot Channel, Trap and Match Color steps load as their
/// commands and save back with the keys Photoshop records (#10609).
#[test]
fn channel_and_color_steps_load_and_save_back() {
    let ids = D::List(vec![D::Integer(182), D::Integer(185), D::Integer(188)]);
    let matched = desc("null").with("Lght", D::Integer(120)).with("ClrR", D::Integer(80)).with("Fade", D::Integer(25)).with("noReference", D::Boolean(true));
    let items = vec![
        item("SplC", "Split Channels", Some(desc("null").with("documentIDs", ids))),
        item("MSpt", "Merge Spot Channel", Some(desc("null"))),
        item("Trap", "Trap", Some(desc("null").with("Wdth", px(2.0)))),
        item("matchColor", "Match Color", Some(matched.clone())),
    ];
    let file = AtnFile {
        name: UnicodeString::new_nul("Prepress"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("Inks"), items: items.clone(), ..Default::default() }],
        ..Default::default()
    };
    let mut s = Session::new();
    let r = s.execute("actions.load", json!({"data": b64(&atn::write(&file).unwrap())})).unwrap();
    assert_eq!(r["unsupported"], json!([]));
    assert_eq!(
        s.actions.list[0].steps,
        vec![
            ("channel.split".to_string(), json!({})),
            ("channel.mergeSpot".to_string(), json!({})),
            ("image.trap".to_string(), json!({"width": 2})),
            ("image.adjustments.matchColor".to_string(), json!({"luminance": 120, "intensity": 80, "fade": 25})),
        ]
    );
    let saved = s.execute("actions.save", json!({"action": "Inks"})).unwrap();
    let back = atn::parse(&photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap()).unwrap();
    let back = &back.actions[0].items;
    let events: Vec<String> = back.iter().map(|i| i.event.id()).collect();
    assert_eq!(events, ["SplC", "MSpt", "Trap", "matchColor"]);
    // The split's document IDs belong to the run that recorded it, so none are written.
    assert!(back[0].descriptor.as_ref().unwrap().items.is_empty());
    for (b, original) in back.iter().zip(&items).skip(1) {
        assert_eq!(b.descriptor.as_ref().unwrap().items, original.descriptor.as_ref().unwrap().items, "{}", original.event.id());
    }
    // Match Color with no reference plays on the image itself.
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("filter.render.clouds", json!({})).unwrap();
    s.actions.list.push(Action::new("Match", vec![s.actions.list[0].steps[3].clone()]));
    let played = s.execute("actions.play", json!({"action": "Match"})).unwrap();
    assert_eq!(played["ran"], 1, "{played}");
}

/// Missing Match Color keys take Photoshop's defaults and Trap's width is clamped to its
/// 32767 px; a step matching another image, or with a key the event lacks, stays a Photoshop
/// step (#10609).
#[test]
fn channel_and_color_steps_default_clamp_and_keep() {
    let from = |name: &str, d: Descriptor| {
        let event = if name.len() == 4 { AtnEvent::Code(name.as_bytes().try_into().unwrap()) } else { AtnEvent::name(name) };
        (events::event_for(&event).unwrap_or_else(|| panic!("{name} is unknown")).from)(&d)
    };
    let params = |m: Mapped| match m {
        Mapped::Steps(s) => s[0].1.clone(),
        Mapped::Unsupported { .. } => panic!("kept"),
    };
    assert_eq!(params(from("matchColor", desc("null"))), json!({"luminance": 100, "intensity": 100, "fade": 0}));
    assert_eq!(params(from("Trap", desc("null").with("Wdth", px(99_999.0)))), json!({"width": 32767.0}));
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    assert!(kept(from("matchColor", desc("null").with("noReference", D::Boolean(false)))));
    assert!(kept(from("SplC", desc("null").with("Chnl", D::Integer(1)))));
    assert!(kept(from("MSpt", desc("null").with("Chnl", D::Integer(1)))));
    assert!(kept(from("Trap", desc("null").with("Hght", px(2.0)))));
}

/// Settings the events have no key for refuse the step instead of replaying differently: a
/// split that keeps the original, a named spot channel, a Match Color source or option, a
/// fractional luminance (#10609).
#[test]
fn channel_and_color_steps_that_cannot_be_written_are_refused() {
    for (id, params) in [
        ("channel.split", json!({"closeOriginal": false})),
        ("channel.mergeSpot", json!({"channel": 1})),
        ("image.trap", json!({"width": 2, "layer": 3})),
        ("image.adjustments.matchColor", json!({"source": 0})),
        ("image.adjustments.matchColor", json!({"neutralize": true})),
        ("image.adjustments.matchColor", json!({"luminance": 99.5})),
    ] {
        assert!(export_item(id, &params, "S").is_none(), "{id} {params} exported");
    }
    assert!(export_item("image.adjustments.matchColor", &json!({"neutralize": false}), "S").is_some());
}

/// A Trap width above the 32767 px the import clamps to refuses the step instead of writing a
/// `Wdth` Photoshop doesn't take (#10609).
#[test]
fn trap_wider_than_photoshop_takes_is_refused() {
    assert!(export_item("image.trap", &json!({"width": 32768}), "S").is_none());
    let d = export_item("image.trap", &json!({"width": 32767}), "S").unwrap().descriptor.unwrap();
    assert_eq!(d.items, desc("null").with("Wdth", px(32767.0)).items);
}

/// The channel or colour step `d` of the Photoshop event `name`, as PhotoCraft loads it.
fn channel_step(name: &str, d: Descriptor) -> Mapped {
    let event = if name.len() == 4 { AtnEvent::Code(name.as_bytes().try_into().unwrap()) } else { AtnEvent::name(name) };
    (events::event_for(&event).unwrap_or_else(|| panic!("{name} is unknown")).from)(&d)
}

/// A `noReference` that isn't a Boolean doesn't say the step has no source, so the step stays
/// a Photoshop step instead of playing as a source-less Match Color (#10609).
#[test]
fn match_color_with_a_non_boolean_no_reference_is_kept() {
    for v in [D::Integer(1), text("true"), enm("Ordn", "Trgt")] {
        let m = channel_step("matchColor", desc("null").with("noReference", v.clone()));
        assert!(matches!(m, Mapped::Unsupported { harmless: false }), "noReference {v:?} was mapped");
    }
}

/// A recorded Match Color or Trap setting that isn't a number is the step's own setting, not a
/// missing one: the step stays a Photoshop step instead of playing with the default (#10609).
#[test]
fn channel_and_color_steps_with_a_non_numeric_setting_are_kept() {
    for (name, key, v) in [
        ("matchColor", "Lght", text("120")),
        ("matchColor", "ClrR", D::Boolean(true)),
        ("matchColor", "Fade", enm("Ordn", "Trgt")),
        ("matchColor", "Lght", D::Double(f64::NAN)),
        ("Trap", "Wdth", text("2")),
        ("Trap", "Wdth", D::Boolean(true)),
    ] {
        let m = channel_step(name, desc("null").with(key, v.clone()));
        assert!(matches!(m, Mapped::Unsupported { harmless: false }), "{name} with {key} = {v:?} was mapped");
    }
}

/// Trap saves its width as `#Pxl`, so a width in another unit, or without one, stays a
/// Photoshop step instead of loading as that many pixels and saving back as pixels (#10609).
#[test]
fn trap_width_not_in_pixels_is_kept() {
    for v in [
        D::UnitFloat { unit: *b"#Pnt", value: 2.0 },
        D::UnitFloat { unit: *b"#Mlm", value: 2.0 },
        D::UnitFloat { unit: *b"#Prc", value: 2.0 },
        D::Double(2.0),
        D::Integer(2),
    ] {
        let m = channel_step("Trap", desc("null").with("Wdth", v.clone()));
        assert!(matches!(m, Mapped::Unsupported { harmless: false }), "Wdth {v:?} was mapped");
    }
    assert!(matches!(channel_step("Trap", desc("null").with("Wdth", px(2.5))), Mapped::Steps(_)));
}

/// Match Color saves its settings as integers, so a recorded value it couldn't save back as
/// the same integer stays a Photoshop step instead of loading and then failing to save (#10609).
#[test]
fn match_color_setting_that_cannot_be_saved_back_is_kept() {
    for (key, v) in [
        ("Lght", D::Double(99.5)),
        ("Lght", D::Double(100.0)),
        ("ClrR", D::LargeInteger(1 << 40)),
        ("ClrR", D::LargeInteger(80)),
        ("Fade", D::UnitFloat { unit: *b"#Prc", value: 25.0 }),
    ] {
        let m = channel_step("matchColor", desc("null").with(key, v.clone()));
        assert!(matches!(m, Mapped::Unsupported { harmless: false }), "{key} = {v:?} was mapped");
    }
}

/// Photoshop's narrowest Trap width wasn't measured, so a recorded width below the command's
/// 1 px isn't clamped up to it: the step stays a Photoshop step, and a PhotoCraft step that
/// narrow is refused on export instead of written as a width the command doesn't trap (#10609).
#[test]
fn trap_width_below_one_pixel_is_kept() {
    for w in [0.25, 0.0, -2.0] {
        let m = channel_step("Trap", desc("null").with("Wdth", px(w)));
        assert!(matches!(m, Mapped::Unsupported { harmless: false }), "Wdth {w} was mapped");
        assert!(export_item("image.trap", &json!({"width": w}), "S").is_none(), "width {w} exported");
    }
    let d = export_item("image.trap", &json!({"width": 1}), "S").unwrap().descriptor.unwrap();
    assert_eq!(d.items, desc("null").with("Wdth", px(1.0)).items);
    assert!(matches!(channel_step("Trap", desc("null").with("Wdth", px(1.0))), Mapped::Steps(_)));
}

/// Match Color's Luminance and Color Intensity take 1..200 and Fade 0..100, and Photoshop's
/// handling of a recorded value outside them wasn't measured: such a step stays a Photoshop
/// step rather than being clamped, and a PhotoCraft step outside them is refused on export
/// instead of written as a value the command plays clamped (#10609).
#[test]
fn match_color_setting_outside_its_range_is_kept() {
    for (key, n) in [("Lght", 0), ("Lght", 201), ("ClrR", 0), ("ClrR", 201), ("Fade", -1), ("Fade", 101)] {
        let m = channel_step("matchColor", desc("null").with(key, D::Integer(n)));
        assert!(matches!(m, Mapped::Unsupported { harmless: false }), "{key} = {n} was mapped");
    }
    let edges = desc("null").with("Lght", D::Integer(1)).with("ClrR", D::Integer(200)).with("Fade", D::Integer(100));
    assert!(matches!(channel_step("matchColor", edges), Mapped::Steps(_)));
    for p in [json!({"luminance": 0}), json!({"luminance": 201}), json!({"intensity": 0}), json!({"fade": -1}), json!({"fade": 101})] {
        assert!(export_item("image.adjustments.matchColor", &p, "S").is_none(), "{p} exported");
    }
    assert!(export_item("image.adjustments.matchColor", &json!({"luminance": 1, "intensity": 200, "fade": 100}), "S").is_some());
}

/// A Match Color step that names its source (`Srce`, a document by name, or the name as text)
/// matches another image the command isn't given, so it stays a Photoshop step even when it
/// also says `noReference` (#10609).
#[test]
fn match_color_with_a_source_is_kept() {
    let from = |d: Descriptor| (events::event_for(&AtnEvent::name("matchColor")).unwrap().from)(&d);
    let document = D::Reference(vec![ReferenceItem::Name { class: class("Dcmn"), name: UnicodeString::new_nul("Reference.psd") }]);
    for source in [document, text("Reference.psd")] {
        for d in [desc("null").with("Srce", source.clone()), desc("null").with("Srce", source.clone()).with("noReference", D::Boolean(true))] {
            assert!(matches!(from(d.clone()), Mapped::Unsupported { harmless: false }), "{:?} was mapped", d.items);
        }
    }
}

/// A Trap step recorded without `Wdth` plays the command's 1 px and saves back as `Wdth` 1 px,
/// so it must load as that width too: load, save and load give the same step.
#[test]
fn trap_step_without_a_width_saves_back_the_same() {
    let file = AtnFile {
        name: UnicodeString::new_nul("S"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("T"), items: vec![item("Trap", "Trap", Some(desc("null")))], ..Default::default() }],
        ..Default::default()
    };
    let mut s = Session::new();
    s.execute("actions.load", json!({"data": b64(&atn::write(&file).unwrap())})).unwrap();
    let loaded = s.actions.list.clone();
    assert_eq!(loaded[0].steps, vec![("image.trap".to_string(), json!({"width": 1}))]);
    let saved = s.execute("actions.save", json!({"set": "S"})).unwrap();
    let mut t = Session::new();
    t.execute("actions.load", json!({"data": saved["data"].as_str().unwrap()})).unwrap();
    assert_eq!(t.actions.list, loaded);
}

/// The Match Color dialog records Source "None" as `"source": -1`, which the command plays
/// without a source: such a step saves, while a real source document is still refused.
#[test]
fn match_color_recorded_with_source_none_saves() {
    let none = json!({"source": -1, "luminance": 100.0, "intensity": 100.0, "fade": 0.0, "neutralize": false, "useSelectionInSource": false, "useSelectionInTarget": false});
    let d = export_item("image.adjustments.matchColor", &none, "S").unwrap().descriptor.unwrap();
    let expected = desc("null").with("Lght", D::Integer(100)).with("ClrR", D::Integer(100)).with("Fade", D::Integer(0)).with("noReference", D::Boolean(true));
    assert_eq!(d.items, expected.items);
    assert!(export_item("image.adjustments.matchColor", &json!({"source": -1, "sourceLayer": 4}), "S").is_some());
    for source in [json!({"source": 0}), json!({"source": 0, "sourceLayer": 4}), json!({"source": "x"}), json!({"source": -1, "neutralize": true})] {
        assert!(export_item("image.adjustments.matchColor", &source, "S").is_none(), "{source} exported");
    }
}

/// Native component references import, select the same target as the command and export again.
#[test]
fn select_color_components_import_play_and_round_trip() {
    for (mode, channels) in [
        ("rgb", &["red", "green", "blue"][..]),
        ("cmyk", &["cyan", "magenta", "yellow", "black"][..]),
        ("lab", &["lightness", "a", "b"][..]),
        ("gray", &["gray"][..]),
    ] {
        for depth in [8, 16, 32] {
            for channel in channels {
                let mut s = Session::new();
                s.execute("file.new", json!({"width": 2, "height": 1, "mode": mode, "depth": depth})).unwrap();
                let expected = s.execute("channel.target", json!({"channel": channel})).unwrap();
                s.execute("channel.target", json!({"channel": "composite"})).unwrap();
                let selection = desc("null").with("null", channel_ref(Some(channel)));
                let data = set_of("Components", vec![("Pick", vec![item("select", "Select", Some(selection))])]);
                let loaded = s.execute("actions.load", json!({"data": data})).unwrap();
                assert_eq!(loaded["unsupported"], json!([]), "{mode}/{depth}/{channel}");
                let played = s.execute("actions.play", json!({"action": "Pick"})).unwrap();
                assert!(played.get("failed").is_none(), "{played}");
                assert_eq!(s.execute("channel.list", json!({})).unwrap(), expected, "{mode}/{depth}/{channel}");
                let saved = s.execute("actions.save", json!({"set": "Components"})).unwrap();
                let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
                assert_eq!(import(&bytes).unwrap().actions[0].steps, s.actions.list[0].steps);
            }
        }
    }
}

/// Unknown targets and selection modes retain their failure instead of selecting another channel.
#[test]
fn select_color_components_preserve_unsupported_selection_shapes() {
    let chained = vec![
        ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Chnl"), value: code("Cyn") },
        ReferenceItem::Identifier { class: class("Dcmn"), id: 7 },
    ];
    for (mode, selection) in [
        ("cmyk", desc("null").with("null", channel_ord("Xxxx"))),
        ("cmyk", desc("null").with("null", channel_ref(Some("cyan"))).with("selectionModifier", enm("selectionModifierType", "addToSelection"))),
        ("cmyk", desc("null").with("null", D::Reference(chained))),
        ("cmyk", desc("null").with("null", D::Reference(vec![ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Ordn"), value: code("Cyn") }]))),
        // Shift-click adds the channel to the selection; playing it as a plain select would lose that.
        ("cmyk", desc("null").with("null", channel_ref(Some("cyan"))).with("Extd", D::Boolean(true))),
        // The legacy `Lmnc` code would otherwise target Lightness and save back as `Lght`.
        ("lab", desc("null").with("null", channel_ord("Lmnc"))),
    ] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 2, "height": 1, "mode": mode})).unwrap();
        let data = set_of("Unknown", vec![("Pick", vec![item("select", "Select", Some(selection.clone()))])]);
        let loaded = s.execute("actions.load", json!({"data": data})).unwrap();
        assert_eq!(loaded["unsupported"].as_array().unwrap().len(), 1);
        assert_eq!(s.actions.list[0].steps[0].0, PHOTOSHOP_STEP);
        let target = s.execute("channel.list", json!({})).unwrap();
        let played = s.execute("actions.play", json!({"action": "Pick"})).unwrap();
        assert_eq!(played["failed"]["id"], PHOTOSHOP_STEP);
        assert_eq!(s.execute("channel.list", json!({})).unwrap(), target);
        let saved = s.execute("actions.save", json!({"set": "Unknown"})).unwrap();
        let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
        assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor.as_ref(), Some(&selection));
    }
    for params in [
        json!({"channel": {"color": "cyan"}, "extend": true}),
        json!({"channel": {"color": "cyan"}, "colors": [0, 1]}),
        json!({"channel": 0}),
        json!({"channel": null}),
        json!({"channel": {"alpha": 0}}),
        json!({"channel": "cyan"}),
    ] {
        assert!(super::steps_layer::select_to("channel.target", &params).is_none(), "{params}");
    }
    assert!(super::steps_layer::select_to("channel.target", &json!({"channel": {"color": "CYAN"}, "extend": false})).is_some());
    let gray = set_of("Gray", vec![("Pick", vec![item("select", "Select", Some(desc("null").with("null", channel_ord("Gry"))))])]);
    let bytes = photocraft_paint::tile::b64_decode(&gray).unwrap();
    assert_eq!(import(&bytes).unwrap().actions[0].steps[0], ("channel.target".into(), json!({"channel": "composite"})));
    // A plain click (`Extd` false) is still an ordinary single-channel select.
    let plain = set_of(
        "Plain",
        vec![("Pick", vec![item("select", "Select", Some(desc("null").with("null", channel_ref(Some("cyan"))).with("Extd", D::Boolean(false))))])],
    );
    let bytes = photocraft_paint::tile::b64_decode(&plain).unwrap();
    assert_eq!(import(&bytes).unwrap().actions[0].steps[0], ("channel.target".into(), json!({"channel": {"color": "cyan"}})));
}

/// A CMYK component cannot retarget a same-named alpha channel when replayed on RGB.
#[test]
fn select_color_components_do_not_fall_back_to_alpha_names() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 2, "height": 1, "mode": "rgb"})).unwrap();
    s.execute("channel.new", json!({"name": "cyan"})).unwrap();
    for channel in [json!("cyan"), json!(0), json!({"alpha": 0})] {
        assert_eq!(s.execute("channel.target", json!({"channel": channel})).unwrap()["target"]["kind"], "alpha");
    }
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    let data = set_of("Components", vec![("Pick", vec![item("select", "Select", Some(desc("null").with("null", channel_ref(Some("cyan")))))])]);
    assert_eq!(s.execute("actions.load", json!({"data": data})).unwrap()["unsupported"], json!([]));
    let played = s.execute("actions.play", json!({"action": "Pick"})).unwrap();
    assert_eq!(played["failed"]["id"], "channel.target");
    assert_eq!(s.execute("channel.list", json!({})).unwrap()["target"]["kind"], "composite");
}

/// Nested color indices take precedence over the object's single-color fallback at runtime.
#[test]
fn select_channel_export_rejects_nested_color_selections() {
    for colors in [vec![0, 1], vec![1]] {
        let params = json!({"channel":{"color":"cyan","colors":colors}});
        let mut s = Session::new();
        s.execute("file.new", json!({"width":2,"height":1,"mode":"cmyk"})).unwrap();
        s.execute("channel.target", params.clone()).unwrap();
        assert_eq!(s.active().unwrap().channel_view.selected_colors, colors);
        s.actions.list.push(Action::new("Nested", vec![("channel.target".into(), params.clone())]));
        assert!(s.execute("actions.save", json!({"action":"Nested"})).is_err(), "{params} must not become a single Cyan selection");
        let skipped = s.execute("actions.save", json!({"action":"Nested","skipUnsupported":true})).unwrap();
        assert_eq!(skipped["skipped"], json!([{"action":"Nested","step":0,"id":"channel.target"}]));
        let bytes = photocraft_paint::tile::b64_decode(skipped["data"].as_str().unwrap()).unwrap();
        assert!(atn::parse(&bytes).unwrap().actions[0].items.is_empty());
        assert_eq!(s.actions.list[0].steps[0].1, params);
    }
    let mut s = Session::new();
    s.actions.list.push(Action::new("Cyan", vec![("channel.target".into(), json!({"channel":{"color":"CYAN"},"extend":false}))]));
    let saved = s.execute("actions.save", json!({"action":"Cyan"})).unwrap();
    assert_eq!(saved["skipped"], json!([]));
    let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
    assert_eq!(import(&bytes).unwrap().actions[0].steps, vec![("channel.target".into(), json!({"channel":{"color":"cyan"}}))]);
}

/// A typed Gray target must retain its failure on RGB instead of becoming portable composite.
#[test]
fn select_channel_export_rejects_mode_specific_gray_objects() {
    for color in ["gray", "GRAY", "GrAy"] {
        for mode in ["gray", "rgb"] {
            let params = json!({"channel":{"color":color}});
            let mut s = Session::new();
            s.execute("file.new", json!({"width":2,"height":1,"mode":mode})).unwrap();
            assert_eq!(s.execute("channel.target", params.clone()).is_ok(), mode == "gray");
            s.actions.list.push(Action::new("Gray object", vec![("channel.target".into(), params.clone())]));
            assert!(s.execute("actions.save", json!({"action":"Gray object"})).is_err(), "{mode}/{color} must not become composite");
            let skipped = s.execute("actions.save", json!({"action":"Gray object","skipUnsupported":true})).unwrap();
            assert_eq!(skipped["skipped"], json!([{"action":"Gray object","step":0,"id":"channel.target"}]));
            let bytes = photocraft_paint::tile::b64_decode(skipped["data"].as_str().unwrap()).unwrap();
            assert!(atn::parse(&bytes).unwrap().actions[0].items.is_empty());
            assert_eq!(s.actions.list[0].steps[0].1, params);
        }
    }
    let mut s = Session::new();
    s.actions.list.push(Action::new("Portable", vec![("channel.target".into(), json!({"channel":"composite"}))]));
    let saved = s.execute("actions.save", json!({"action":"Portable"})).unwrap();
    assert_eq!(saved["skipped"], json!([]));
    let bytes = photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap();
    assert_eq!(import(&bytes).unwrap().actions[0].steps, s.actions.list[0].steps);
    let gray = set_of("Gray", vec![("Pick", vec![item("select", "Select", Some(desc("null").with("null", channel_ord("Gry"))))])]);
    let mut rgb = Session::new();
    rgb.execute("file.new", json!({"width":2,"height":1,"mode":"rgb"})).unwrap();
    rgb.execute("actions.load", json!({"data":gray})).unwrap();
    assert_eq!(rgb.actions.list[0].steps[0], ("channel.target".into(), json!({"channel":"composite"})));
    assert!(rgb.execute("actions.play", json!({"action":"Pick"})).unwrap().get("failed").is_none());
}

/// Newer Group from Layers recordings put the name outside Usng; existing nested names win.
#[test]
fn mapped_make_group_name_survives_load_and_save() {
    for (top, nested, expected) in [(Some("Selected"), None, "Selected"), (None, Some("Nested"), "Nested"), (Some("Top"), Some("Nested"), "Nested")] {
        let mut d = desc("null").with("null", class_ref("layerSection")).with("From", target_ref("Lyr "));
        if let Some(name) = top {
            d = d.with("Nm  ", text(name));
        }
        if let Some(name) = nested {
            d = d.with("Usng", D::Descriptor(desc("layerSection").with("Nm  ", text(name))));
        }
        let source = one_action(vec![item("Mk  ", "Group from Layers", Some(d))]);
        let loaded = import(&atn::write(&source).unwrap()).unwrap();
        assert_eq!(loaded.actions[0].steps, vec![("layer.groupLayers".into(), json!({"name": expected}))]);
        let (saved, skipped) = export("Groups", &[&loaded.actions[0]], &[], false).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(import(&saved).unwrap().actions[0].steps, loaded.actions[0].steps);
    }
}

/// A recorded New Layer clipping flag must reach the command and survive strict export.
#[test]
fn mapped_make_clipping_flag_survives_load_and_save() {
    for clip in [None, Some(false), Some(true)] {
        let mut using = desc("Lyr ").with("Nm  ", text("Clipped"));
        if let Some(clip) = clip {
            using = using.with("Grup", D::Boolean(clip));
        }
        let d = desc("null").with("null", class_ref("Lyr ")).with("Usng", D::Descriptor(using));
        let loaded = import(&atn::write(&one_action(vec![item("Mk  ", "New Layer", Some(d))])).unwrap()).unwrap();
        let mut expected = json!({"name": "Clipped"});
        if clip == Some(true) {
            expected["clipToPrevious"] = json!(true);
        }
        assert_eq!(loaded.actions[0].steps, vec![("layer.new.layer".into(), expected)]);
        let (saved, skipped) = export("Layers", &[&loaded.actions[0]], &[], false).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(import(&saved).unwrap().actions[0].steps, loaded.actions[0].steps);
    }
}

/// An unreadable clipping flag keeps the original step instead of creating an unclipped layer.
#[test]
fn mapped_make_invalid_clipping_flag_is_kept() {
    let using = desc("Lyr ").with("Grup", long(1.0));
    let d = desc("null").with("null", class_ref("Lyr ")).with("Usng", D::Descriptor(using));
    let loaded = import(&atn::write(&one_action(vec![item("Mk  ", "New Layer", Some(d))])).unwrap()).unwrap();
    assert_eq!(loaded.actions[0].steps[0].0, PHOTOSHOP_STEP);
}

/// Load a recorded descriptor through the same parser as Load Actions.
fn mapped_recording(event: &str, descriptor: Descriptor) -> Imported {
    import(&atn::write(&one_action(vec![item(event, event, Some(descriptor))])).unwrap()).unwrap()
}

/// Every frame anchor survives import/export; unknown anchors stay available verbatim.
#[test]
fn mapped_transform_reference_points_round_trip() {
    for (code, reference) in [
        ("Qcsa", "center"),
        ("Qcs0", "topLeft"),
        ("Qcs4", "top"),
        ("Qcs1", "topRight"),
        ("Qcs7", "left"),
        ("Qcs5", "right"),
        ("Qcs3", "bottomLeft"),
        ("Qcs6", "bottom"),
        ("Qcs2", "bottomRight"),
    ] {
        for target in [target_ref("Lyr "), D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: Id::new("fsel") }])] {
            let d = desc("null")
                .with("null", target)
                .with("FTcs", enm("QCSt", code))
                .with("Wdth", pct(50.0))
                .with("Hght", pct(50.0))
                .with("Ofst", D::Descriptor(desc("Ofst").with("Hrzn", px(0.0)).with("Vrtc", px(0.0))));
            let loaded = mapped_recording("Trnf", d);
            let params = &loaded.actions[0].steps[0].1;
            assert_eq!(params.get("reference").and_then(Value::as_str).unwrap_or("center"), reference);
            let (bytes, _) = export("Anchors", &[&loaded.actions[0]], &[], false).unwrap();
            assert_eq!(import(&bytes).unwrap().actions[0].steps, loaded.actions[0].steps);
            if reference == "topLeft" && loaded.actions[0].steps[0].0 == "select.transformSelection" {
                let mut s = Session::new();
                s.execute("file.new", json!({"width":256,"height":256})).unwrap();
                s.execute("select.rect", json!({"x":60,"y":60,"width":100,"height":100})).unwrap();
                let (id, p) = &loaded.actions[0].steps[0];
                s.execute(id, p.clone()).unwrap();
                assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().content_bounds(), photocraft_geom::Rect::new(60, 60, 110, 110));
            }
        }
    }
    // The reference application never records `Qcs8` or `Qcsi` (independent): kept steps.
    for code in ["unknown", "Qcs8", "Qcsi"] {
        let d = desc("null").with("null", target_ref("Lyr ")).with("FTcs", enm("QCSt", code)).with("Wdth", pct(50.0));
        assert_eq!(mapped_recording("Trnf", d).actions[0].steps[0].0, PHOTOSHOP_STEP, "{code}");
    }
    // A native anchor saves as the reference application's code; nothing writes `Qcs8`.
    for (reference, code) in [("top", "Qcs4"), ("topRight", "Qcs1"), ("bottom", "Qcs6"), ("bottomRight", "Qcs2")] {
        let native = Action::new("Native", vec![("edit.transform".into(), json!({"scaleX":50,"scaleY":50,"reference":reference}))]);
        let (bytes, _) = export("Anchors", &[&native], &[], false).unwrap();
        let saved = atn::parse(&bytes).unwrap();
        let d = saved.actions[0].items[0].descriptor.as_ref().unwrap();
        assert!(matches!(d.get("FTcs"), Some(D::Enumerated { value, .. }) if value.is(code)), "{reference}: {:?}", d.get("FTcs"));
    }
}

/// Alt-clicking an eye twice must isolate and then restore visibility, not just show a layer.
#[test]
fn mapped_show_only_round_trip_and_toggle() {
    let d = desc("null").with("null", D::List(vec![name_ref("Lyr ", "Solo")])).with("TglO", D::Boolean(true));
    let loaded = mapped_recording("Shw ", d);
    assert_eq!(loaded.actions[0].steps, vec![("layer.showOnly".into(), json!({"layer": "Solo"}))]);
    let (bytes, _) = export("Eyes", &[&loaded.actions[0]], &[], false).unwrap();
    let saved = atn::parse(&bytes).unwrap();
    assert_eq!(get_bool(saved.actions[0].items[0].descriptor.as_ref().unwrap(), "TglO"), Some(true));
    assert_eq!(import(&bytes).unwrap().actions[0].steps, loaded.actions[0].steps);
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4})).unwrap();
    s.execute("layer.new.layer", json!({"name":"Solo"})).unwrap();
    let visible = |s: &Session| s.active().unwrap().doc.layers.iter().map(|l| l.visible).collect::<Vec<_>>();
    let original = visible(&s);
    let (id, params) = &loaded.actions[0].steps[0];
    assert_eq!(s.execute(id, params.clone()).unwrap()["shownAlone"], true);
    assert_ne!(visible(&s), original);
    assert_eq!(s.execute(id, params.clone()).unwrap()["shownAlone"], false);
    assert_eq!(visible(&s), original);
    let malformed = desc("null").with("null", D::List(vec![name_ref("Lyr ", "Solo"), long(1.0)])).with("TglO", D::Boolean(true));
    assert_eq!(mapped_recording("Shw ", malformed).actions[0].steps[0].0, PHOTOSHOP_STEP);
}

/// A layer label is separate from an adjustment's own colour parameter.
#[test]
fn mapped_adjustment_label_survives_export() {
    let d = desc("null")
        .with("null", class_ref("AdjL"))
        .with("Usng", D::Descriptor(desc("AdjL").with("Type", D::Descriptor(desc("Invr"))).with("Clr ", enm("Clr ", "Rd  "))));
    let loaded = mapped_recording("Mk  ", d);
    assert_eq!(loaded.actions[0].steps, vec![("layer.newAdjustmentLayer.invert".into(), json!({})), ("layer.setLabelColor".into(), json!({"color":"red"}))]);
    let (bytes, _) = export("Labels", &[&loaded.actions[0]], &[], false).unwrap();
    assert_eq!(import(&bytes).unwrap().actions[0].steps, loaded.actions[0].steps);
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4})).unwrap();
    for (id, params) in &loaded.actions[0].steps {
        s.execute(id, params.clone()).unwrap();
    }
    let st = s.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().label.id(), "red");
}

/// Keeping an unmapped shape is safer than creating a solid fill over the whole canvas.
#[test]
fn mapped_shape_creation_is_kept_with_its_geometry_and_stroke() {
    let d = desc("null").with("null", class_ref("contentLayer")).with("Shp ", D::Descriptor(desc("Rctn").with("Wdth", px(20.0)))).with(
        "Usng",
        D::Descriptor(
            desc("contentLayer")
                .with("Type", D::Descriptor(desc("solidColorLayer")))
                .with("strokeStyle", D::Descriptor(desc("strokeStyle").with("strokeStyleLineWidth", px(3.0)))),
        ),
    );
    let loaded = mapped_recording("Mk  ", d.clone());
    assert_eq!(loaded.actions[0].steps[0].0, PHOTOSHOP_STEP);
    let (bytes, _) = export("Shapes", &[&loaded.actions[0]], &[], false).unwrap();
    assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor.as_ref(), Some(&d));
}

/// Copy and Cut retain all recorded layer options, including labels and fractional opacity.
#[test]
fn mapped_copy_cut_layer_options_survive_export() {
    for (copy, id) in [(true, "layer.new.layerViaCopy"), (false, "layer.new.layerViaCut")] {
        let options = desc("Lyr ").with("Nm  ", text("Extracted")).with("Md  ", enm("BlnM", "Mltp")).with("Opct", pct(37.5)).with("Clr ", enm("Clr ", "Bl  "));
        let d = desc("null").with("Nw  ", D::Descriptor(options)).with("Usng", enm("ArSl", "Slct")).with("Cpy ", D::Boolean(copy));
        let loaded = mapped_recording("Mk  ", d);
        assert_eq!(
            loaded.actions[0].steps,
            vec![
                (id.into(), json!({})),
                ("layer.setLabelColor".into(), json!({"color":"blue"})),
                ("layer.setProps".into(), json!({"name":"Extracted","blend":"Multiply","opacity":0.375}))
            ]
        );
        let (bytes, _) = export("Extract", &[&loaded.actions[0]], &[], false).unwrap();
        let saved = import(&bytes).unwrap();
        assert_eq!(saved.actions[0].steps[..2], loaded.actions[0].steps[..2]);
        let mut props = serde_json::Map::new();
        for (id, p) in &saved.actions[0].steps[2..] {
            assert_eq!(id, "layer.setProps");
            props.extend(p.as_object().unwrap().clone());
        }
        assert_eq!(Value::Object(props), loaded.actions[0].steps[2].1);
        let mut s = Session::new();
        s.execute("file.new", json!({"width":4,"height":4,"background":"white"})).unwrap();
        s.execute("select.rect", json!({"x":0,"y":0,"width":2,"height":2})).unwrap();
        for (id, params) in &saved.actions[0].steps {
            s.execute(id, params.clone()).unwrap();
        }
        let st = s.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        assert_eq!(layer.name, "Extracted");
        assert_eq!(layer.opacity, 0.375);
        assert_eq!(layer.blend, photocraft_color::BlendMode::Multiply);
        assert_eq!(layer.label.id(), "blue");
    }
}

/// Loading transparency must keep the source layer instead of using the active layer.
#[test]
fn mapped_named_layer_transparency_survives_export() {
    let source = D::Reference(vec![
        ReferenceItem::Enumerated { class: class("Chnl"), type_id: code("Chnl"), value: code("Trsp") },
        ReferenceItem::Name { class: class("Lyr "), name: UnicodeString::new_nul("Source") },
    ]);
    let d = desc("null").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Chnl"), key: code("fsel") }])).with("T   ", source);
    let loaded = mapped_recording("setd", d);
    assert_eq!(loaded.actions[0].steps, vec![("select.loadSelection".into(), json!({"channel":"transparency","layer":"Source"}))]);
    let (bytes, _) = export("Transparency", &[&loaded.actions[0]], &[], false).unwrap();
    assert_eq!(import(&bytes).unwrap().actions[0].steps, loaded.actions[0].steps);
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4,"background":"white"})).unwrap();
    s.execute("layer.setProps", json!({"name":"Source"})).unwrap();
    s.execute("layer.new.layer", json!({"name":"Empty"})).unwrap();
    let (id, p) = &loaded.actions[0].steps[0];
    s.execute(id, p.clone()).unwrap();
    assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().content_bounds(), photocraft_geom::Rect::new(0, 0, 4, 4));
}

/// A localized Background and an ordinary layer named Background are distinct targets.
#[test]
fn mapped_background_property_survives_export_and_playback() {
    let r = D::Reference(vec![ReferenceItem::Property { class: class("Lyr "), key: code("Bckg") }]);
    let loaded = mapped_recording("Hd  ", desc("null").with("null", D::List(vec![r.clone()])));
    assert_eq!(loaded.actions[0].steps, vec![("layer.hideLayers".into(), json!({"layer":{"background":true}}))]);
    let (bytes, _) = export("Background", &[&loaded.actions[0]], &[], false).unwrap();
    assert_eq!(get_list(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor.as_ref().unwrap(), "null"), Some(&[r][..]));
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4,"background":"white"})).unwrap();
    std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc).layers[0].name = "Hintergrund".into();
    s.execute("layer.new.layer", json!({"name":"Background"})).unwrap();
    let (id, p) = &loaded.actions[0].steps[0];
    s.execute(id, p.clone()).unwrap();
    let layers = &s.active().unwrap().doc.layers;
    assert!(!layers[0].visible);
    assert!(layers[1].visible);
    s.execute("layer.hideLayers", json!({"layer":"Background"})).unwrap();
    assert!(!s.active().unwrap().doc.layers[1].visible);
    assert!(s.execute("layer.showLayers", json!({"layer":{"background":false}})).is_err());
    s.execute("layer.setLabelColor", json!({"layer":{"background":true},"color":"red"})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers[0].label.id(), "red");
    s.execute("select.loadSelection", json!({"channel":"transparency","layer":{"background":true}})).unwrap();
    assert_eq!(s.active().unwrap().doc.selection.as_ref().unwrap().content_bounds(), photocraft_geom::Rect::new(0, 0, 4, 4));
    let selected = mapped_recording("slct", desc("null").with("null", D::Reference(vec![ReferenceItem::Property { class: class("Lyr "), key: code("Bckg") }])));
    let (bytes, _) = export("Select Background", &[&selected.actions[0]], &[], false).unwrap();
    assert_eq!(import(&bytes).unwrap().actions[0].steps, selected.actions[0].steps);
    let (id, p) = &selected.actions[0].steps[0];
    s.execute(id, p.clone()).unwrap();
    assert_eq!(s.active().unwrap().active_layer, Some(s.active().unwrap().doc.layers[0].id));
    let revision = s.active().unwrap().revision;
    for selector in [json!({"background":false}), json!({"background":"true"}), json!({"background":true,"extra":0})] {
        assert!(s.execute("select.loadSelection", json!({"channel":"transparency","layer":selector})).is_err());
        assert_eq!(s.active().unwrap().revision, revision);
    }
}

/// An unanswered Close must not turn into an explicit discard or close a changed document.
#[test]
fn mapped_close_keeps_ask_and_discard_distinct() {
    let loaded = mapped_recording("Cls ", desc("null"));
    assert_eq!(loaded.actions[0].steps, vec![("file.close".into(), json!({"save":"ask"}))]);
    let (bytes, _) = export("Close", &[&loaded.actions[0]], &[], false).unwrap();
    assert!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor.as_ref().unwrap().get("Svng").is_none());
    assert_eq!(import(&bytes).unwrap().actions[0].steps, loaded.actions[0].steps);
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let (id, p) = &loaded.actions[0].steps[0];
    assert!(s.execute(id, p.clone()).is_err());
    assert_eq!(s.documents().len(), 1);
    s.execute("file.close", json!({"save":"discard"})).unwrap();
    assert!(s.documents().is_empty());
    s.execute("file.new", json!({"width":4,"height":4})).unwrap();
    s.execute(id, p.clone()).unwrap();
    assert!(s.documents().is_empty());
}

/// `"save": false` is the older spelling of discard: it closes a changed document and saves as
/// "don't save" (`Svng N`); `true` and other values stay refused.
#[test]
fn close_save_false_discards_and_saves_as_dont_save() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":4,"height":4})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    for refused in [json!(true), json!("yes"), json!(0)] {
        assert!(s.execute("file.close", json!({"save": refused})).is_err(), "{refused}");
        assert_eq!(s.documents().len(), 1);
    }
    s.execute("file.close", json!({"save": false})).unwrap();
    assert!(s.documents().is_empty());
    let native = Action::new("Close", vec![("file.close".into(), json!({"save": false}))]);
    let (bytes, _) = export("Close", &[&native], &[], false).unwrap();
    let d = atn::parse(&bytes).unwrap().actions[0].items[0].descriptor.clone().unwrap();
    assert!(matches!(d.get("Svng"), Some(D::Enumerated { type_id, value }) if type_id.is("YsN ") && value.is("N   ")), "{:?}", d.get("Svng"));
}

/// Lab Photo Filter colours use the CMS conversion and remain mapped after saving as RGB.
#[test]
fn mapped_lab_photo_filter_imports_and_round_trips() {
    for (lab, expected) in
        [([0.0, 0.0, 0.0], "#000000"), ([100.0, 0.0, 0.0], "#ffffff"), ([50.0, 0.0, 0.0], "#777777"), ([54.2917, 80.8125, 69.8851], "#ff0000")]
    {
        let color = desc("LbCl").with("Lmnc", D::Double(lab[0])).with("A   ", D::Double(lab[1])).with("B   ", D::Double(lab[2]));
        let loaded = mapped_recording("photoFilter", desc("null").with("Clr ", D::Descriptor(color)).with("Dnst", long(25.0)));
        let (id, params) = &loaded.actions[0].steps[0];
        assert_eq!(id, "image.adjustments.photoFilter");
        assert_eq!(params["color"], expected);
        assert_eq!(params["density"].as_f64(), Some(25.0));
        let (bytes, _) = export("Lab", &[&loaded.actions[0]], &[], false).unwrap();
        let saved = import(&bytes).unwrap();
        assert_eq!(saved.actions[0].steps[0].0, *id);
        assert_eq!(saved.actions[0].steps[0].1["color"], expected);
        assert_eq!(saved.actions[0].steps[0].1["density"].as_f64(), Some(25.0));
    }
}

/// A Photoshop step of one of the newer features, as its event reference records it (#10610).
fn newer(name: &str, d: Descriptor) -> Mapped {
    (events::event_for(&AtnEvent::name(name)).unwrap_or_else(|| panic!("{name} is unknown")).from)(&d)
}

fn mapped(m: Mapped) -> (String, Value) {
    match m {
        Mapped::Steps(mut s) if s.len() == 1 => s.remove(0),
        _ => panic!("not one step"),
    }
}

/// Oil Paint, Smart Sharpen, Focus Area, Select Sky and Remove Background load with the keys and
/// defaults Photoshop records; Oil Paint's and Select Sky's defaults differ from the commands'
/// and are written out (#10610).
#[test]
fn newer_feature_steps_load_with_photoshops_defaults() {
    let oil = desc("null")
        .with("lightingOn", D::Boolean(true))
        .with("stylization", D::Double(4.0))
        .with("cleanliness", D::Double(2.299_999_952_316_28))
        .with("brushScale", D::Double(0.800_000_011_920_93))
        .with("microBrush", D::Double(10.0))
        .with("LghD", D::Integer(-60))
        .with("specularity", D::Double(1.299_999_952_316_28));
    let expected_oil = json!({"lighting": true, "stylization": 4.0, "cleanliness": 2.299_999_952_316_28, "scale": 0.800_000_011_920_93,
        "bristleDetail": 10.0, "angle": -60.0, "shine": 1.299_999_952_316_28});
    assert_eq!(mapped(newer("oilPaint", oil.clone())), ("filter.stylize.oilPaint".into(), expected_oil.clone()));
    assert_eq!(mapped(newer("oilPaint", desc("null"))).1, expected_oil, "a missing key takes Photoshop's default");
    // Smart Sharpen in the form the command plays (the newer filter, removing a Gaussian blur).
    let sharpen = desc("null")
        .with("presetKind", enm("presetKindType", "presetKindCustom"))
        .with("useLegacy", D::Boolean(false))
        .with("Amnt", pct(150.0))
        .with("Rds ", px(2.5))
        .with("noiseReduction", pct(20.0))
        .with("blur", enm("blurType", "GsnB"));
    assert_eq!(
        mapped(newer("smartSharpen", sharpen.clone())),
        ("filter.sharpen.smartSharpen".into(), json!({"amount": 150.0, "radius": 2.5, "reduceNoise": 20.0}))
    );
    assert_eq!(
        mapped(newer("smartSharpen", gaussian_sharpen(&[]))).1,
        json!({"amount": 200.0, "radius": 1.0, "reduceNoise": 10.0}),
        "a missing amount, radius or noise reduction takes Photoshop's default"
    );
    assert_eq!(mapped(newer("selectSky", desc("null"))), ("select.sky".into(), json!({"sampleAllLayers": false})));
    assert_eq!(mapped(newer("focusMask", desc("null"))), ("select.focusArea".into(), json!({})));
    assert_eq!(mapped(newer("removeBackground", desc("null"))), ("layer.removeBackground".into(), json!({})));

    // Written back, they carry the same keys in the same order.
    let back = |id: &str, p: Value| export_item(id, &p, "S").unwrap_or_else(|| panic!("{id} does not export")).descriptor.unwrap();
    assert_eq!(back("filter.stylize.oilPaint", expected_oil).items, oil.items);
    assert_eq!(back("filter.sharpen.smartSharpen", json!({"amount": 150, "radius": 2.5, "reduceNoise": 20})).items, sharpen.items);
    let (id, params) = mapped(newer("smartSharpen", sharpen.clone()));
    assert_eq!(back(&id, params).items, sharpen.items, "load -> save changed the step");

    // Oil Paint and Smart Sharpen play.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 24, "height": 24})).unwrap();
    s.execute("filter.render.clouds", json!({})).unwrap();
    let action = Action::new(
        "Paint",
        vec![
            ("filter.stylize.oilPaint".into(), mapped(newer("oilPaint", desc("null"))).1),
            ("filter.sharpen.smartSharpen".into(), mapped(newer("smartSharpen", gaussian_sharpen(&[]))).1),
        ],
    );
    s.actions.list.push(action);
    let played = s.execute("actions.play", json!({"action": "Paint"})).unwrap();
    assert_eq!(played["ran"], 2, "{played}");
}

/// What the commands cannot do keeps the Photoshop step instead of playing something else
/// (#10610).
#[test]
fn newer_feature_steps_the_commands_cannot_play_are_kept() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    // Photoshop's own default step: the legacy filter removing Lens Blur, which the command's
    // Gaussian sharpen does not play.
    let legacy = desc("null")
        .with("presetKind", enm("presetKindType", "presetKindCustom"))
        .with("useLegacy", D::Boolean(true))
        .with("Amnt", pct(200.0))
        .with("Rds ", px(1.0))
        .with("moreAccurate", D::Boolean(false))
        .with("blur", enm("blurType", "lensBlur"));
    assert!(kept(newer("smartSharpen", legacy)));
    assert!(kept(newer("smartSharpen", desc("null"))), "an empty step is the legacy filter");
    assert!(kept(newer("smartSharpen", desc("null").with("useLegacy", D::Boolean(false)))), "no blur recorded is Lens Blur");
    assert!(kept(newer("smartSharpen", desc("null").with("blur", enm("blurType", "GsnB")))), "no useLegacy recorded is the legacy filter");
    for blur in ["lensBlur", "MtnB", "motionBlur"] {
        assert!(kept(newer("smartSharpen", gaussian_sharpen(&[("blur", enm("blurType", blur))]))), "{blur}");
    }
    assert!(kept(newer("smartSharpen", gaussian_sharpen(&[("useLegacy", D::Boolean(true))]))));
    assert!(kept(newer("smartSharpen", gaussian_sharpen(&[("blur", enm("Ordn", "GsnB"))]))), "a blur of another type");
    assert!(kept(newer("smartSharpen", gaussian_sharpen(&[("moreAccurate", D::Boolean(false))]))));
    assert!(kept(newer("smartSharpen", gaussian_sharpen(&[("presetKind", enm("presetKindType", "presetKindDefault"))]))));
    assert!(kept(newer("focusMask", desc("null").with("Rng ", D::Double(3.0)))));
    assert!(kept(newer("removeBackground", desc("null").with("Lyr ", D::Integer(1)))));
    assert!(kept(newer("selectSky", desc("null").with("smartRadius", D::Boolean(true)))));
    assert!(kept(newer("oilPaint", desc("null").with("Angl", D::Integer(3)))));
}

/// Settings the events have no key for refuse the step rather than replay differently (#10610).
#[test]
fn newer_feature_steps_that_cannot_be_written_are_refused() {
    for (id, params) in [
        ("filter.sharpen.smartSharpen", json!({"amount": 150, "blur": "lens"})),
        ("filter.sharpen.smartSharpen", json!({"target": "mask"})),
        ("filter.stylize.oilPaint", json!({"angle": 10.5})),
        ("filter.stylize.oilPaint", json!({"target": "mask"})),
        ("select.focusArea", json!({"range": 0.3})),
        ("select.sky", json!({"mode": "add"})),
        ("select.sky", json!({"threshold": 70})),
        ("layer.removeBackground", json!({"layer": 3})),
    ] {
        assert!(export_item(id, &params, "S").is_none(), "{id} {params} exported");
    }
    assert!(export_item("select.sky", &json!({"mode": "replace", "threshold": 50, "softness": 50}), "S").is_some());
}

/// A Smart Sharpen step in the form the command plays (`useLegacy` false, `blur` `GsnB`) plus
/// `settings`, which replace those two when they name them.
fn gaussian_sharpen(settings: &[(&str, D)]) -> Descriptor {
    let named = |k: &str| settings.iter().any(|(key, _)| key.trim_end() == k);
    let mut d = desc("null");
    if !named("useLegacy") {
        d = d.with("useLegacy", D::Boolean(false));
    }
    if !named("blur") {
        d = d.with("blur", enm("blurType", "GsnB"));
    }
    settings.iter().fold(d, |d, (k, v)| d.with(k, v.clone()))
}

/// `kept_when_wrong_typed` for Smart Sharpen, on the form it would otherwise map.
fn sharpen_kept_when(settings: &[(&str, D)]) {
    for (key, v) in settings {
        let m = newer("smartSharpen", gaussian_sharpen(&[(key, v.clone())]));
        assert!(matches!(m, Mapped::Unsupported { harmless: false }), "smartSharpen with {key:?} = {v:?} was mapped");
    }
}

/// A recorded setting of the wrong type is the step's own setting, not a missing one: the
/// mapper must not play it with a default but keep the Photoshop step (#10610).
fn kept_when_wrong_typed(name: &str, settings: &[(&str, D)]) {
    for (key, v) in settings {
        let m = newer(name, desc("null").with(key, v.clone()));
        assert!(matches!(m, Mapped::Unsupported { harmless: false }), "{name} with {key:?} = {v:?} was mapped");
    }
}

#[test]
fn select_sky_with_a_wrong_typed_setting_is_kept() {
    kept_when_wrong_typed("selectSky", &[("sampleAllLayers", D::Integer(1)), ("sampleAllLayers", text("true"))]);
}

#[test]
fn oil_paint_with_a_wrong_typed_setting_is_kept() {
    kept_when_wrong_typed(
        "oilPaint",
        &[
            ("lightingOn", D::Integer(0)),
            ("stylization", D::Boolean(true)),
            ("cleanliness", text("5")),
            ("brushScale", enm("Ordn", "Trgt")),
            ("microBrush", D::Double(f64::NAN)),
            ("LghD", text("45")),
            ("specularity", D::Boolean(false)),
        ],
    );
}

#[test]
fn smart_sharpen_with_a_wrong_typed_setting_is_kept() {
    sharpen_kept_when(&[
        ("presetKind", text("presetKindDefault")),
        ("useLegacy", D::Integer(0)),
        ("blur", text("GsnB")),
        ("Amnt", D::Boolean(true)),
        ("Rds ", text("2")),
        ("Rds", text("2")),
        ("noiseReduction", text("10")),
    ]);
}

/// `only_keys` admits a code without its space padding, so the mapper reads it too: an
/// unpadded `Rds` radius is the step's radius, not the default 1 (#10610).
#[test]
fn smart_sharpen_reads_an_unpadded_radius() {
    let d = gaussian_sharpen(&[("Amnt", pct(150.0)), ("Rds", px(2.5))]);
    assert_eq!(mapped(newer("smartSharpen", d)), ("filter.sharpen.smartSharpen".into(), json!({"amount": 150.0, "radius": 2.5, "reduceNoise": 10.0})));
}

/// A step recording one of its keys twice, padded or not, can't be saved back with both values,
/// so it stays a Photoshop step instead of playing the first and dropping the second (#10610).
#[test]
fn newer_feature_steps_with_a_repeated_key_are_kept() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    let mapped = [
        ("smartSharpen", "Amnt", "Amnt", pct(150.0), pct(250.0)),
        ("smartSharpen", "Rds ", "Rds ", px(2.0), px(3.0)),
        ("smartSharpen", "Rds ", "Rds", px(2.0), px(3.0)),
        ("smartSharpen", "Rds", "Rds", px(2.0), px(2.0)),
        ("smartSharpen", "presetKind", "presetKind", enm("presetKindType", "presetKindCustom"), enm("presetKindType", "presetKindCustom")),
        ("smartSharpen", "useLegacy", "useLegacy", D::Boolean(false), D::Boolean(false)),
        ("smartSharpen", "noiseReduction", "noiseReduction", pct(10.0), pct(20.0)),
        ("smartSharpen", "blur", "blur", enm("blurType", "GsnB"), enm("blurType", "GsnB")),
        ("oilPaint", "lightingOn", "lightingOn", D::Boolean(true), D::Boolean(false)),
        ("oilPaint", "stylization", "stylization", D::Double(4.0), D::Double(6.0)),
        ("oilPaint", "cleanliness", "cleanliness", D::Double(2.0), D::Double(3.0)),
        ("oilPaint", "brushScale", "brushScale", D::Double(1.0), D::Double(2.0)),
        ("oilPaint", "microBrush", "microBrush", D::Double(5.0), D::Double(7.0)),
        ("oilPaint", "LghD", "LghD", D::Integer(-60), D::Integer(45)),
        ("oilPaint", "specularity", "specularity", D::Double(1.0), D::Double(2.0)),
        ("selectSky", "sampleAllLayers", "sampleAllLayers", D::Boolean(false), D::Boolean(true)),
    ]
    .into_iter()
    .filter(|(name, key, again, a, b)| {
        let d = if *name == "smartSharpen" {
            gaussian_sharpen(&[(key, a.clone()), (again, b.clone())])
        } else {
            desc("null").with(key, a.clone()).with(again, b.clone())
        };
        !kept(newer(name, d))
    })
    .map(|(name, key, again, ..)| format!("{name} {key:?} {again:?}"))
    .collect::<Vec<_>>();
    assert!(mapped.is_empty(), "mapped with a repeated key: {mapped:?}");
}

/// Oil Paint records its settings as plain doubles and the light direction as an integer, and
/// saves them back so: a value with a unit, or of the other number type, would load as a bare
/// number and save back changed, so the step stays a Photoshop step (#10610).
#[test]
fn oil_paint_setting_with_a_unit_or_another_number_type_is_kept() {
    kept_when_wrong_typed(
        "oilPaint",
        &[
            ("stylization", pct(4.0)),
            ("cleanliness", px(2.0)),
            ("brushScale", D::UnitFloat { unit: *b"#Rlt", value: 0.8 }),
            ("microBrush", D::Integer(10)),
            ("LghD", ang(-60.0)),
            ("LghD", D::Double(-60.5)),
            ("specularity", D::UnitFloat { unit: *b"#Prc", value: 1.3 }),
        ],
    );
}

/// Smart Sharpen records `Amnt` in `#Prc` and its radius in `#Pxl`, and saves them back so:
/// another unit, or none, would load as that number and save back in the other unit, so the
/// step stays a Photoshop step, whether the radius key is padded or not (#10610).
#[test]
fn smart_sharpen_amount_or_radius_in_another_unit_is_kept() {
    sharpen_kept_when(&[
        ("Amnt", px(150.0)),
        ("Amnt", D::Double(150.0)),
        ("Amnt", D::Integer(150)),
        ("Rds ", pct(2.0)),
        ("Rds ", D::Double(2.0)),
        ("Rds", D::UnitFloat { unit: *b"#Mlm", value: 2.0 }),
        ("Rds", D::Double(2.0)),
        ("noiseReduction", px(10.0)),
        ("noiseReduction", D::Double(10.0)),
    ]);
    for rds in ["Rds ", "Rds"] {
        let d = gaussian_sharpen(&[("Amnt", pct(150.0)), (rds, px(2.5))]);
        assert_eq!(
            mapped(newer("smartSharpen", d)),
            ("filter.sharpen.smartSharpen".into(), json!({"amount": 150.0, "radius": 2.5, "reduceNoise": 10.0})),
            "{rds:?}"
        );
    }
}

/// `filter.sharpen.smartSharpen` takes an amount of 1..500 %, a radius of 0.1..64 px and a noise
/// reduction of 0..100 %. Photoshop clamps a recorded value outside them when the step plays,
/// so the import clamps it too; a step outside them is not written (#10610).
#[test]
fn smart_sharpen_outside_the_commands_limits_is_clamped_on_import_and_refused_on_export() {
    for (key, v, param, want) in [
        ("Amnt", pct(0.5), "amount", 1.0),
        ("Amnt", pct(500.5), "amount", 500.0),
        ("Rds ", px(0.05), "radius", 0.1),
        ("Rds ", px(64.5), "radius", 64.0),
        ("Rds", px(1000.0), "radius", 64.0),
        ("noiseReduction", pct(-5.0), "reduceNoise", 0.0),
        ("noiseReduction", pct(150.0), "reduceNoise", 100.0),
    ] {
        let (id, p) = mapped(newer("smartSharpen", gaussian_sharpen(&[(key, v.clone())])));
        assert_eq!((id.as_str(), &p[param]), ("filter.sharpen.smartSharpen", &json!(want)), "{key:?} = {v:?}");
    }
    for (amount, radius, noise) in [(1.0, 0.1, 0.0), (500.0, 64.0, 100.0)] {
        let d = gaussian_sharpen(&[("Amnt", pct(amount)), ("Rds ", px(radius)), ("noiseReduction", pct(noise))]);
        assert_eq!(
            mapped(newer("smartSharpen", d)),
            ("filter.sharpen.smartSharpen".into(), json!({"amount": amount, "radius": radius, "reduceNoise": noise})),
            "{amount} {radius}"
        );
    }
    for params in [
        json!({"amount": 0.5}),
        json!({"amount": 501}),
        json!({"radius": 0.05}),
        json!({"radius": 65}),
        json!({"reduceNoise": -1}),
        json!({"reduceNoise": 101}),
    ] {
        assert!(export_item("filter.sharpen.smartSharpen", &params, "S").is_none(), "{params} exported");
    }
}

/// Focus Area, Select Sky and Remove Background, mapped from the empty steps Photoshop records,
/// play and do their work: on a textured red disc against a smooth blue sky over textured ground,
/// Focus Area selects the textured ground, Select Sky the sky, and Remove Background masks the
/// layer to the disc. Each step plays on its own, with nothing selected, so a step that runs
/// without finding anything, or that works from the selection left by the step before, fails
/// (#10610).
#[test]
fn newer_feature_selection_steps_play() {
    let (w, h) = (160u32, 100u32);
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h})).unwrap();
    s.edit("paint", |doc, _| {
        let bg = doc.layers[0].surface_mut().unwrap();
        let n = bg.channels();
        let mut data = Vec::with_capacity((w * h) as usize * n);
        for y in 0..h {
            for x in 0..w {
                let nz = (((x * 7919 + y * 104_729) % 97) as f32 / 97.0 - 0.5) * 0.25;
                let (dx, dy) = (x as f32 - 80.0, y as f32 - 40.0);
                let c = if dx * dx + dy * dy <= 22.0 * 22.0 {
                    [0.85 + nz * 0.5, 0.2 + nz, 0.15]
                } else if y < 60 {
                    let t = y as f32 / 60.0;
                    [0.25 + 0.4 * t, 0.5 + 0.3 * t, 0.9 + 0.05 * t]
                } else {
                    [0.35 + nz, 0.3 + nz, 0.15]
                };
                data.extend_from_slice(&c.map(|v| v.clamp(0.0, 1.0))[..n.min(3)]);
                if n == 4 {
                    data.push(1.0);
                }
            }
        }
        bg.write_region(photocraft_geom::Rect::new(0, 0, w as i32, h as i32), &data);
        Ok(())
    })
    .unwrap();
    let play = |s: &mut Session, name: &str| {
        s.actions.list.push(Action::new(name, vec![mapped(newer(name, desc("null")))]));
        let played = s.execute("actions.play", json!({"action": name})).unwrap();
        assert!(played.get("failed").is_none(), "{name}: {played}");
        assert_eq!(played["ran"], 1, "{name}: {played}");
    };
    let selected = |s: &Session, x: i32, y: i32| s.active().unwrap().doc.selection.as_ref().map(|m| m.sample_channel(x, y, 0));

    play(&mut s, "focusMask");
    assert!(selected(&s, 20, 85).is_some_and(|v| v > 0.5), "Focus Area left the textured ground out");
    assert!(selected(&s, 20, 15).is_some_and(|v| v < 0.5), "Focus Area took the smooth sky");
    s.execute("select.deselect", json!({})).unwrap();

    play(&mut s, "selectSky");
    assert!(selected(&s, 20, 15).is_some_and(|v| v > 0.9), "Select Sky left the sky out");
    assert!(selected(&s, 20, 85).is_some_and(|v| v < 0.1), "Select Sky took the ground");
    s.execute("select.deselect", json!({})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_none(), "Remove Background would play with the sky selected");

    play(&mut s, "removeBackground");
    let mask = s.active().unwrap().doc.layers[0].mask.as_ref().expect("Remove Background set no mask");
    assert!(mask.value(80, 40) > 0.8, "Remove Background hid the red disc");
    assert!(mask.value(20, 15) < 0.2, "Remove Background revealed the sky");
}

/// `filter.stylize.oilPaint` takes a stylization and scale of 0.1..10, a cleanliness, bristle
/// detail and shine of 0..10 and a light direction of -180..180. Photoshop clamps a recorded
/// value outside them when the step plays, so the import clamps it too; a step outside them is
/// not written (#10610).
#[test]
fn oil_paint_outside_the_commands_ranges_is_clamped_on_import_and_refused_on_export() {
    for (key, v, param, want) in [
        ("stylization", D::Double(0.05), "stylization", 0.1),
        ("stylization", D::Double(10.5), "stylization", 10.0),
        ("cleanliness", D::Double(-0.5), "cleanliness", 0.0),
        ("cleanliness", D::Double(10.5), "cleanliness", 10.0),
        ("brushScale", D::Double(0.05), "scale", 0.1),
        ("brushScale", D::Double(10.5), "scale", 10.0),
        ("microBrush", D::Double(-1.0), "bristleDetail", 0.0),
        ("microBrush", D::Double(10.5), "bristleDetail", 10.0),
        ("specularity", D::Double(-0.5), "shine", 0.0),
        ("specularity", D::Double(10.5), "shine", 10.0),
        ("LghD", D::Integer(500), "angle", 180.0),
        ("LghD", D::Integer(-500), "angle", -180.0),
        ("LghD", D::Integer(181), "angle", 180.0),
    ] {
        let (id, p) = mapped(newer("oilPaint", desc("null").with(key, v.clone())));
        assert_eq!((id.as_str(), &p[param]), ("filter.stylize.oilPaint", &json!(want)), "{key:?} = {v:?}");
    }
    for (low, high) in [(0.1, 0.0), (10.0, 10.0)] {
        let d = desc("null")
            .with("stylization", D::Double(low))
            .with("cleanliness", D::Double(high))
            .with("brushScale", D::Double(low))
            .with("microBrush", D::Double(high))
            .with("specularity", D::Double(high));
        assert_eq!(
            mapped(newer("oilPaint", d)),
            (
                "filter.stylize.oilPaint".into(),
                json!({"lighting": true, "stylization": low, "cleanliness": high, "scale": low, "bristleDetail": high, "angle": -60.0, "shine": high})
            ),
            "{low} {high}"
        );
    }
    for params in [
        json!({"stylization": 0.05}),
        json!({"stylization": 10.5}),
        json!({"cleanliness": -0.5}),
        json!({"cleanliness": 10.5}),
        json!({"scale": 0.05}),
        json!({"scale": 10.5}),
        json!({"bristleDetail": -1}),
        json!({"bristleDetail": 11}),
        json!({"shine": -0.5}),
        json!({"shine": 10.5}),
        json!({"angle": 181}),
        json!({"angle": -181}),
        json!({"angle": 1000}),
    ] {
        assert!(export_item("filter.stylize.oilPaint", &params, "S").is_none(), "{params} exported");
    }
}

/// The step `d` of the Photoshop event `code` (a four-character code), as PhotoCraft loads it.
fn other_step(code: &str, d: Descriptor) -> Mapped {
    let event = AtnEvent::Code(code.as_bytes().try_into().unwrap());
    (events::event_for(&event).unwrap_or_else(|| panic!("{code} is unknown")).from)(&d)
}

/// Photoshop's De-Interlace, NTSC Colors, HSB/HSL, Custom, Average and Remove Black / White
/// Matte steps load as their commands, play, and save back with the same keys (#10607).
#[test]
fn video_other_and_matting_steps_load_play_and_save_back() {
    let cells: Vec<D> = (0..25)
        .map(|i| {
            D::Integer(if i == 12 {
                5
            } else if [7, 11, 13, 17].contains(&i) {
                -1
            } else {
                0
            })
        })
        .collect();
    let kernel: Vec<i32> = (0..25)
        .map(|i| {
            if i == 12 {
                5
            } else if [7, 11, 13, 17].contains(&i) {
                -1
            } else {
                0
            }
        })
        .collect();
    let cases = [
        (
            "Dntr",
            desc("null").with("IntE", enm("IntE", "ElmE")).with("IntC", enm("IntC", "CrtD")),
            "filter.video.deInterlace",
            json!({"eliminate": "evenFields", "createBy": "duplication"}),
        ),
        ("NTSC", desc("null"), "filter.video.ntscColors", json!({})),
        (
            "HsbP",
            desc("null").with("Inpt", enm("ClrS", "RGBC")).with("Otpt", enm("ClrS", "HSLC")),
            "filter.other.hsbHsl",
            json!({"inputMode": "rgb", "rowOrder": "hsl"}),
        ),
        (
            "Cstm",
            desc("null").with("Scl ", D::Integer(1)).with("Ofst", D::Integer(0)).with("Mtrx", D::List(cells)),
            "filter.other.custom",
            json!({"kernel": kernel, "scale": 1, "offset": 0}),
        ),
        ("Avrg", desc("null"), "filter.blur.average", json!({})),
        ("RmvB", desc("null"), "layer.matting.removeBlackMatte", json!({})),
        ("RmvW", desc("null"), "layer.matting.removeWhiteMatte", json!({})),
    ];
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 24, "height": 24, "background": "transparent"})).unwrap();
    s.execute("filter.render.clouds", json!({})).unwrap();
    for (code, d, id, params) in cases {
        let Mapped::Steps(steps) = other_step(code, d.clone()) else { panic!("{code} kept") };
        let mut p = steps[0].1.clone();
        // Numbers come back as the descriptor's integers.
        if let Some(k) = p.get_mut("kernel") {
            *k = json!(k.as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as i32).collect::<Vec<_>>());
        }
        for k in ["scale", "offset"] {
            if let Some(v) = p.get_mut(k) {
                *v = json!(v.as_f64().unwrap() as i32);
            }
        }
        assert_eq!((steps[0].0.as_str(), &p), (id, &params), "{code}");
        assert_eq!(export_item(id, &steps[0].1, "S").unwrap().descriptor.unwrap().items, d.items, "{code}");
        s.actions.list.push(Action::new(code, vec![(id.to_string(), steps[0].1.clone())]));
        let played = s.execute("actions.play", json!({"action": code})).unwrap();
        assert_eq!(played["ran"], 1, "{code}: {played}");
    }
    // Missing keys are Photoshop's defaults.
    let Mapped::Steps(steps) = other_step("Dntr", desc("null")) else { panic!("kept") };
    assert_eq!(steps[0].1, json!({"eliminate": "oddFields", "createBy": "interpolation"}));
    let Mapped::Steps(steps) = other_step("HsbP", desc("null")) else { panic!("kept") };
    assert_eq!(steps[0].1, json!({"inputMode": "rgb", "rowOrder": "hsb"}));
}

/// A step the commands cannot play keeps the Photoshop step: a Custom matrix that is not 5 × 5,
/// an unknown enumeration value, or a key the event does not have (#10607).
#[test]
fn video_other_and_matting_steps_the_commands_cannot_play_are_kept() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    let nine = D::List((0..9).map(D::Integer).collect());
    assert!(kept(other_step("Cstm", desc("null").with("Mtrx", nine))));
    assert!(kept(other_step("Cstm", desc("null"))));
    assert!(kept(other_step("HsbP", desc("null").with("Inpt", enm("ClrS", "LbCl")))));
    assert!(kept(other_step("Dntr", desc("null").with("IntE", enm("IntE", "ElmA")))));
    assert!(kept(other_step("NTSC", desc("null").with("Lvl ", D::Integer(1)))));
    assert!(kept(other_step("RmvW", desc("null").with("Lyr ", D::Integer(1)))));
}

/// What the events cannot hold refuses the step: a fractional kernel cell, a kernel that is not
/// 25 cells, a mask target, an unknown mode (#10607).
#[test]
fn video_other_and_matting_steps_that_cannot_be_written_are_refused() {
    let mut cells = vec![json!(0); 25];
    cells[12] = json!(1.5);
    for (id, params) in [
        ("filter.other.custom", json!({"kernel": cells})),
        ("filter.other.custom", json!({"kernel": [0, 0, 0, 0, 1, 0, 0, 0, 0]})),
        ("filter.other.custom", json!({"scale": 2.5})),
        ("filter.blur.average", json!({"target": "mask"})),
        ("filter.other.hsbHsl", json!({"inputMode": "lab"})),
        ("filter.video.deInterlace", json!({"eliminate": "allFields"})),
        ("filter.other.custom", json!([1])),
        ("filter.blur.average", json!(5)),
        ("filter.other.hsbHsl", json!("hsl")),
    ] {
        assert!(export_item(id, &params, "S").is_none(), "{id} {params} exported");
    }
    // `null` params set nothing.
    assert!(export_item("filter.blur.average", &Value::Null, "S").is_some());
    // A Custom step without a kernel is the command's identity kernel.
    let d = export_item("filter.other.custom", &json!({}), "S").unwrap().descriptor.unwrap();
    let cells = get_list(&d, "Mtrx").unwrap();
    assert_eq!((cells.len(), cells.get(12)), (25, Some(&D::Integer(1))));
}

/// A recorded setting of the wrong type is the step's own setting, not a missing one: the
/// mapper keeps the Photoshop step instead of playing it with Photoshop's default (#10607).
#[test]
fn video_other_steps_with_a_wrong_typed_setting_are_kept() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    let identity = D::List((0..25).map(|i| D::Integer(i32::from(i == 12))).collect());
    for (code, d) in [
        ("Dntr", desc("null").with("IntE", D::Integer(1))),
        ("Dntr", desc("null").with("IntC", text("CrtD"))),
        ("HsbP", desc("null").with("Inpt", D::Boolean(true))),
        ("HsbP", desc("null").with("Otpt", text("HSLC"))),
        ("Cstm", desc("null").with("Mtrx", identity.clone()).with("Scl ", text("2"))),
        ("Cstm", desc("null").with("Mtrx", identity.clone()).with("Ofst", D::Boolean(true))),
        ("Cstm", desc("null").with("Mtrx", identity.clone()).with("Scl", enm("Ordn", "Trgt"))),
    ] {
        assert!(kept(other_step(code, d.clone())), "{code} {d:?} was mapped");
    }
}

/// The key gate admits `Scl` without its padding, so the mapper reads it too: an unpadded
/// scale is the step's scale, not the default 1.
#[test]
fn custom_reads_an_unpadded_scale() {
    let identity = D::List((0..25).map(|i| D::Integer(i32::from(i == 12))).collect());
    let Mapped::Steps(steps) = other_step("Cstm", desc("null").with("Scl", D::Integer(3)).with("Mtrx", identity)) else { panic!("kept") };
    assert_eq!(steps[0].1["scale"], json!(3.0));
}

/// `filter.other.custom` plays a scale of 1..9999, an offset of -9999..9999 and cells of
/// -999..999, and clamps anything else: `Scl 0` would play as 1 and save back as 0. Such a step
/// stays a Photoshop step, as does a matrix of more than 25 cells, and export refuses those
/// values instead of writing what the command doesn't play (#10607).
#[test]
fn custom_outside_the_commands_range_is_kept_and_refused() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    let kernel = |cell: i32| D::List((0..25).map(|i| D::Integer(if i == 12 { cell } else { 0 })).collect());
    let identity = kernel(1);
    for d in [
        desc("null").with("Mtrx", identity.clone()).with("Scl ", D::Integer(0)),
        desc("null").with("Mtrx", identity.clone()).with("Scl ", D::Integer(10_000)),
        desc("null").with("Mtrx", identity.clone()).with("Ofst", D::Integer(-10_000)),
        desc("null").with("Mtrx", identity.clone()).with("Ofst", D::Integer(10_000)),
        desc("null").with("Mtrx", kernel(1000)),
        desc("null").with("Mtrx", kernel(-1000)),
        desc("null").with("Mtrx", D::List((0..26).map(|_| D::Integer(0)).collect())),
        desc("null").with("Mtrx", D::List((0..1000).map(|_| D::Integer(0)).collect())),
        // Fractions: export writes integers, so Save Actions would refuse the mapped step.
        desc("null").with("Mtrx", identity.clone()).with("Scl ", D::Double(1.5)),
        desc("null").with("Mtrx", identity.clone()).with("Ofst", D::Double(2.5)),
        desc("null").with("Mtrx", D::List((0..25).map(|i| if i == 12 { D::Double(0.5) } else { D::Integer(0) }).collect())),
    ] {
        assert!(kept(other_step("Cstm", d.clone())), "{:?} was mapped", d.items);
    }
    for (scale, offset, cell) in [(1, -9999, -999), (9999, 9999, 999)] {
        let d = desc("null").with("Scl ", D::Integer(scale)).with("Ofst", D::Integer(offset)).with("Mtrx", kernel(cell));
        let Mapped::Steps(steps) = other_step("Cstm", d) else { panic!("{scale} {offset} {cell} kept") };
        assert_eq!(
            (&steps[0].1["scale"], &steps[0].1["offset"], &steps[0].1["kernel"][12]),
            (&json!(f64::from(scale)), &json!(f64::from(offset)), &json!(f64::from(cell)))
        );
    }
    // An integral Double is a whole number and still maps.
    let Mapped::Steps(steps) = other_step("Cstm", desc("null").with("Mtrx", identity.clone()).with("Scl ", D::Double(2.0))) else { panic!("Scl 2.0 kept") };
    assert_eq!(steps[0].1["scale"], json!(2.0));
    let mut cells = vec![json!(0); 25];
    cells[12] = json!(1000);
    for params in [json!({"scale": 0}), json!({"scale": 10_000}), json!({"offset": -10_000}), json!({"offset": 10_000}), json!({"kernel": cells})] {
        assert!(export_item("filter.other.custom", &params, "S").is_none(), "{params} exported");
    }
}

/// A local absolute map path on this platform: playback may read only such a path.
#[cfg(windows)]
const LOCAL_MAP: &str = r"C:\máp.psd";
#[cfg(not(windows))]
const LOCAL_MAP: &str = "/máp.psd";
/// [`LOCAL_MAP`] as a `txtu` path, encoded independently of the production path writer.
#[cfg(windows)]
const LOCAL_MAP_WIRE: &[u8] = b"txtu\x22\0\0\0\x0b\0\0\0C\0:\0\\\0m\0\xe1\0p\0.\0p\0s\0d\0\0\0";
#[cfg(not(windows))]
const LOCAL_MAP_WIRE: &[u8] = b"txtu\x1e\0\0\0\x09\0\0\0/\0m\0\xe1\0p\0.\0p\0s\0d\0\0\0";

/// The selected-map descriptor measured in a Windows Displace recording.
fn displace_descriptor(path: D) -> Descriptor {
    desc("null")
        .with("HrzS", D::Integer(10))
        .with("VrtS", D::Integer(10))
        .with("DspM", enm("DspM", "StrF"))
        .with("UndA", enm("UndA", "RptE"))
        .with("DspF", path)
}

/// Preserve the typed path and materialize native defaults when saving sparse steps.
#[test]
fn recorded_displace_round_trips_text_event_and_typed_path() {
    let descriptor = displace_descriptor(D::Path(LOCAL_MAP_WIRE.to_vec()));
    let data = set_of("Maps", vec![("A", vec![item("displace", "Displace", Some(descriptor.clone()))])]);
    let imported = import(&photocraft_paint::tile::b64_decode(&data).unwrap()).unwrap();
    assert!(imported.unsupported.is_empty());
    let (id, params) = &imported.actions[0].steps[0];
    assert_eq!(id, "filter.distort.displace");
    assert_eq!(*params, json!({"horizontal":10,"vertical":10,"fit":"stretch","undefinedAreas":"repeat","mapPath":LOCAL_MAP}));
    let exported = export_item(id, params, "Maps").unwrap();
    assert_eq!(exported.event, AtnEvent::name("displace"));
    assert_eq!(exported.descriptor, Some(descriptor.clone()));
    let minimal = json!({"mapPath":LOCAL_MAP});
    assert_eq!(export_item(id, &minimal, "Maps").unwrap().descriptor, Some(descriptor.clone()));
    for key in ["horizontal", "vertical", "fit", "undefinedAreas"] {
        let mut sparse = params.clone();
        sparse.as_object_mut().unwrap().remove(key);
        assert_eq!(export_item(id, &sparse, "Maps").unwrap().descriptor, Some(descriptor.clone()), "{key}");
    }
    let custom = json!({"horizontal":-7,"mapPath":LOCAL_MAP});
    let mut expected = descriptor;
    expected.items[0].1 = D::Integer(-7);
    assert_eq!(export_item(id, &custom, "Maps").unwrap().descriptor, Some(expected));
}

/// A shared action file must not make playback open a network share, a device or a path taken
/// from the working directory: such a map path stays a kept step with its bytes.
#[test]
fn displace_map_paths_a_document_may_not_name_stay_kept_steps() {
    let refused = [r"\\server\share\map.psd", "//server/share/map.psd", r"\\?\UNC\server\share\map.psd", r"\\.\COM1", "file://server/share/map.psd", "map.psd"];
    let cases: Vec<_> = refused.iter().map(|path| item("displace", "Displace", Some(displace_descriptor(path_value(path))))).collect();
    let data = set_of("Maps", vec![("A", cases.clone())]);
    let imported = import(&photocraft_paint::tile::b64_decode(&data).unwrap()).unwrap();
    assert_eq!(imported.unsupported.len(), cases.len());
    for ((id, params), expected) in imported.actions[0].steps.iter().zip(cases) {
        assert_eq!(id, PHOTOSHOP_STEP, "{params}");
        assert_eq!(export_item(id, params, "Maps").unwrap().descriptor, expected.descriptor);
    }
}

/// Keep unmeasured inputs intact and refuse settings that would be changed by export.
#[test]
fn unmeasured_displace_stays_opaque_and_lossy_export_is_refused() {
    assert!(!super::menu_item_commands().contains(&"filter.distort.displace"));
    let path = r"C:\map.psd";
    let wire = path_value(path);
    let descriptor = displace_descriptor(wire.clone());
    let replace = |key: &str, value: D| {
        let mut d = descriptor.clone();
        d.items.iter_mut().find(|(id, _)| id.is(key)).unwrap().1 = value;
        d
    };
    let alter_path = |i: usize, pair: [u8; 2]| {
        let D::Path(mut bytes) = wire.clone() else { panic!("expected a path") };
        bytes[i..i + 2].copy_from_slice(&pair);
        D::Path(bytes)
    };
    let mut without_path = descriptor.clone();
    without_path.items.retain(|(id, _)| !id.is("DspF"));
    let mut string_key = descriptor.clone();
    string_key.items[0].0 = Id::Str(b"HrzS".to_vec());
    let cases = [
        item("displace", "Displace", None),
        item("displace", "Displace", Some(without_path)),
        item("displace", "Displace", Some(string_key)),
        item("displace", "Displace", Some(descriptor.clone().with("HrzS", D::Integer(10)))),
        item("displace", "Displace", Some(replace("HrzS", D::Double(10.0)))),
        item("displace", "Displace", Some(replace("VrtS", D::Integer(1000)))),
        item("displace", "Displace", Some(replace("DspM", enm("DspM", "Tile")))),
        item("displace", "Displace", Some(replace("UndA", enm("OtherEnum", "RptE")))),
        item("displace", "Displace", Some(replace("DspF", D::Alias(vec![1, 2, 3])))),
        item("displace", "Displace", Some(replace("DspF", alter_path(4, [0, 0])))),
        item("displace", "Displace", Some(replace("DspF", alter_path(8, [9, 0])))),
        item("displace", "Displace", Some(replace("DspF", alter_path(12, [0, 0])))),
        item("displace", "Displace", Some(replace("DspF", alter_path(12, [0, 0xd8])))),
        item("Dspl", "Displace", Some(descriptor)),
    ];
    let data = set_of("Maps", vec![("A", cases.to_vec())]);
    let imported = import(&photocraft_paint::tile::b64_decode(&data).unwrap()).unwrap();
    assert_eq!(imported.unsupported.len(), cases.len());
    assert_eq!(imported.actions[0].steps.len(), cases.len());
    for ((id, params), expected) in imported.actions[0].steps.iter().zip(cases) {
        assert_eq!(id, PHOTOSHOP_STEP);
        let exported = export_item(id, params, "Maps").unwrap();
        assert_eq!(exported.event, expected.event);
        assert_eq!(exported.descriptor, expected.descriptor);
    }

    let good = json!({"horizontal":10,"vertical":10,"fit":"stretch","undefinedAreas":"repeat","mapPath":path});
    for (key, value) in [
        ("horizontal", json!(10.5)),
        ("horizontal", json!(1000)),
        ("vertical", json!(-1000)),
        ("vertical", json!("10")),
        ("fit", json!("tile")),
        ("fit", Value::Null),
        ("undefinedAreas", json!("wrap")),
        ("undefinedAreas", json!(false)),
        ("mapPath", json!("x\0y.psd")),
        ("mapPath", json!("")),
        ("mapLayer", json!(1)),
        ("mapDocument", json!(0)),
        ("target", json!("mask")),
    ] {
        let mut params = good.clone();
        params[key] = value;
        assert!(export_item("filter.distort.displace", &params, "Maps").is_none(), "{params}");
    }
}

/// Loading needs no map; playback reads it only after authorization and stretches it.
#[test]
fn recorded_displace_loads_without_map_and_plays_through_the_step_gate() {
    use photocraft_geom::Rect;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic-map.psd");
    let wire = displace_descriptor(path_value(path.to_str().unwrap()));
    let data = set_of("Maps", vec![("A", vec![item("displace", "Displace", Some(wire))])]);
    let mut source = Session::new();
    let loaded = source.execute("actions.load", json!({"data":data})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]));
    assert!(!path.exists(), "loading the ATN needs no map");

    let paint_split = |s: &mut Session, side: i32, split: i32, right: [f32; 4]| {
        s.execute("file.new", json!({"width":side,"height":side})).unwrap();
        s.edit("Two-tone fixture", |doc, _| {
            let id = doc.layers[0].id;
            let surface = doc.layer_mut(id).unwrap().surface_mut().unwrap();
            surface.fill_rect(Rect::new(0, 0, split, side), &[0.0, 0.0, 0.0, 1.0]);
            surface.fill_rect(Rect::new(split, 0, side, side), &right);
            Ok(())
        })
        .unwrap();
    };
    // Offset the source edge so the positive shift crosses it within the white part of the map.
    paint_split(&mut source, 64, 48, [1.0, 0.0, 0.0, 1.0]);
    source
        .edit("Bottom edge fixture", |doc, _| {
            let id = doc.layers[0].id;
            let surface = doc.layer_mut(id).unwrap().surface_mut().unwrap();
            surface.fill_rect(Rect::new(0, 48, 64, 64), &[0.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let snapshot = |s: &Session| {
        let active = s.active().unwrap();
        (active.revision, active.history.past_len(), photocraft_compose::render(&active.doc, active.doc.bounds()).into_px())
    };
    let before = snapshot(&source);
    let missing = source.execute("actions.play", json!({"action":"A"})).unwrap();
    assert_eq!(missing["ran"], 0);
    assert_eq!(missing["failed"]["id"], "filter.distort.displace");
    assert!(missing["failed"]["error"].as_str().unwrap().contains("cannot read displacement map"));
    assert_eq!(snapshot(&source), before);

    // Leave the map absent so a read before authorization would report the same I/O failure.
    source.authorize = Some(|id, p| {
        if id == "filter.distort.displace" && p["mapPath"].is_string() {
            return Err(EngineError::Other("test denied the map path".into()));
        }
        Ok(())
    });
    let denied = source.execute("actions.play", json!({"action":"A"})).unwrap();
    assert_eq!(denied["ran"], 0);
    assert_eq!(denied["failed"]["id"], "filter.distort.displace");
    assert!(denied["failed"]["error"].as_str().unwrap().contains("test denied the map path"));
    assert_eq!(snapshot(&source), before);

    source.authorize = None;
    let mut map = Session::new();
    paint_split(&mut map, 16, 8, [1.0, 1.0, 1.0, 1.0]);
    let encoded = photocraft_io::export(&map.active().unwrap().doc, "psd", &Default::default()).unwrap();
    std::fs::write(&path, encoded.bytes).unwrap();
    let played = source.execute("actions.play", json!({"action":"A"})).unwrap();
    assert_eq!(played["ran"], 1, "{played}");
    assert!(played.get("failed").is_none(), "{played}");
    let pixels = snapshot(&source).2;
    // With stretch, x=38 samples the white half and shifts into red; tiling samples black.
    assert_eq!(before.2[32 * 64 + 38], [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(pixels[32 * 64 + 38], [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(pixels[32 * 64 + 20], [0.0, 0.0, 0.0, 1.0]);
    // The vertical shift crosses into green, then repeats the bottom edge instead of wrapping to red.
    assert_eq!(before.2[40 * 64 + 38], [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(pixels[40 * 64 + 38], [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(pixels[60 * 64 + 38], [0.0, 1.0, 0.0, 1.0]);
}

/// The step `d` of Photoshop event `code`, as PhotoCraft loads it.
fn distort(code: &str, d: Descriptor) -> Mapped {
    let event = AtnEvent::Code(code.as_bytes().try_into().unwrap());
    (events::event_for(&event).unwrap_or_else(|| panic!("{code} is unknown")).from)(&d)
}

fn one_step(m: Mapped) -> (String, Value) {
    match m {
        Mapped::Steps(mut s) if s.len() == 1 => s.remove(0),
        _ => panic!("not one step"),
    }
}

/// Photoshop's Distort filter steps load with its defaults for missing keys and its clamping for
/// out-of-range values, play, and save back with every key in recorded order (#10605).
#[test]
fn distort_filter_steps_load_clamp_play_and_save_back() {
    // Photoshop's defaults, as its steps record them.
    let wave = desc("null")
        .with("Wvtp", enm("Wvtp", "WvSn"))
        .with("NmbG", D::Integer(5))
        .with("WLMn", D::Integer(10))
        .with("WLMx", D::Integer(120))
        .with("AmMn", D::Integer(5))
        .with("AmMx", D::Integer(35))
        .with("SclH", D::Integer(100))
        .with("SclV", D::Integer(100))
        .with("UndA", enm("UndA", "RptE"))
        .with("RndS", D::Integer(-7));
    let zigzag = desc("null").with("Amnt", D::Integer(10)).with("NmbR", D::Integer(5)).with("ZZTy", enm("ZZTy", "PndR"));
    let cases = [
        ("Pnch", desc("null").with("Amnt", D::Integer(50)), "filter.distort.pinch", json!({"amount": 50})),
        ("Plr ", desc("null").with("Cnvr", enm("Cnvr", "RctP")), "filter.distort.polarCoordinates", json!({"mode": "rectangularToPolar"})),
        (
            "Sphr",
            desc("null").with("Amnt", D::Integer(100)).with("SphM", enm("SphM", "Nrml")),
            "filter.distort.spherize",
            json!({"amount": 100, "mode": "normal"}),
        ),
        ("Twrl", desc("null").with("Angl", D::Integer(50)), "filter.distort.twirl", json!({"angle": 50})),
        (
            "Wave",
            wave,
            "filter.distort.wave",
            json!({"type": "sine", "generators": 5, "wavelengthMin": 10, "wavelengthMax": 120, "amplitudeMin": 5, "amplitudeMax": 35,
                "undefinedAreas": "repeat", "seed": 4_294_967_289_u32}),
        ),
        ("ZgZg", zigzag, "filter.distort.zigZag", json!({"amount": 10, "ridges": 5, "style": "pondRipples"})),
    ];
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 24, "height": 24})).unwrap();
    s.execute("filter.render.clouds", json!({})).unwrap();
    for (code, d, id, params) in cases {
        let (got, p) = one_step(distort(code, d.clone()));
        assert_eq!((got.as_str(), &p), (id, &params), "{code}");
        // Every key, in the order Photoshop records them.
        assert_eq!(export_item(id, &p, "S").unwrap().descriptor.unwrap().items, d.items, "{code}");
        s.actions.list.push(Action::new(code, vec![(id.to_string(), p)]));
        let played = s.execute("actions.play", json!({"action": code})).unwrap();
        assert_eq!(played["ran"], 1, "{code}: {played}");
    }
    // A step that leaves its keys out takes Photoshop's defaults: Wave's undefined areas are
    // Repeat Edge Pixels, where the command's own default is wrap.
    assert_eq!(one_step(distort("Wave", desc("null"))).1["undefinedAreas"], "repeat");
    assert_eq!(one_step(distort("Sphr", desc("null"))).1, json!({"amount": 100, "mode": "normal"}));
    // Out-of-range values are clamped, as Photoshop clamps them.
    assert_eq!(one_step(distort("Pnch", desc("null").with("Amnt", D::Integer(500)))).1, json!({"amount": 100}));
    assert_eq!(one_step(distort("Twrl", desc("null").with("Angl", D::Integer(-5000)))).1, json!({"angle": -999}));
    assert_eq!(one_step(distort("ZgZg", desc("null").with("NmbR", D::Integer(40)))).1["ridges"], 20);
    assert_eq!(one_step(distort("Wave", desc("null").with("NmbG", D::Integer(0)))).1["generators"], 1);
}

/// PhotoCraft's renderer draws at most `MAX_WAVE_GENERATORS` (64) generators, so a Wave step
/// recorded with more keeps the Photoshop step on import and is refused on export, as the PSD
/// smart-filter mapper does, rather than mapping a step that plays differently (#10605).
#[test]
fn wave_generators_past_what_photocraft_draws_keep_the_step() {
    assert_eq!(photocraft_algo::MAX_WAVE_GENERATORS, 64);
    let p = one_step(distort("Wave", desc("null").with("NmbG", D::Integer(64)))).1;
    assert_eq!(p["generators"], 64, "{p}");
    for n in [65, 999, 1_000_000] {
        let m = distort("Wave", desc("null").with("NmbG", D::Integer(n)));
        assert!(matches!(m, Mapped::Unsupported { harmless: false }), "NmbG {n}");
    }
    assert!(export_item("filter.distort.wave", &json!({"generators": 65}), "S").is_none());
    assert!(export_item("filter.distort.wave", &json!({"generators": 64}), "S").is_some());
}

/// What the commands cannot play keeps the Photoshop step: an unknown enumeration value, a Wave
/// scaled below 100 % (the command has no scale), or a key the event does not have (#10605).
#[test]
fn distort_filter_steps_the_commands_cannot_play_are_kept() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    assert!(kept(distort("Sphr", desc("null").with("SphM", enm("SphM", "Dgnl")))));
    assert!(kept(distort("Plr ", desc("null").with("Cnvr", enm("Cnvr", "Sprl")))));
    assert!(kept(distort("Wave", desc("null").with("SclH", D::Integer(50)))));
    assert!(kept(distort("Wave", desc("null").with("UndA", enm("UndA", "Trns")))));
    assert!(kept(distort("Pnch", desc("null").with("Rds ", D::Integer(3)))));
    // A scale of 100 % (or above, which Photoshop clamps to 100) plays.
    assert!(!kept(distort("Wave", desc("null").with("SclH", D::Integer(100)).with("SclV", D::Integer(250)))));
}

/// A setting the events cannot hold refuses the step instead of replaying differently: a
/// fractional amount (the keys are integers), a mask target, a seed past 32 bits (#10605).
#[test]
fn distort_filter_steps_that_cannot_be_written_are_refused() {
    for (id, params) in [
        ("filter.distort.pinch", json!({"amount": 12.5})),
        ("filter.distort.twirl", json!({"angle": 10, "target": "mask"})),
        ("filter.distort.zigZag", json!({"ridges": 2.5})),
        ("filter.distort.wave", json!({"seed": 4_294_967_296_u64})),
        ("filter.distort.wave", json!({"undefinedAreas": "transparent"})),
        ("filter.distort.spherize", json!({"mode": "diagonal"})),
    ] {
        assert!(export_item(id, &params, "S").is_none(), "{id} {params} exported");
    }
    // Left out, a setting is written as the command replays it: Wave wraps.
    let wave = export_item("filter.distort.wave", &json!({}), "S").unwrap().descriptor.unwrap();
    assert_eq!(get_enum(&wave, "UndA").as_deref(), Some("WrpA"));
}

/// A recorded setting of the wrong type is the step's own setting, not a missing one: the
/// mapper keeps the Photoshop step instead of playing it with Photoshop's default (#10605).
#[test]
fn distort_filter_steps_with_a_wrong_typed_setting_are_kept() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    for (code, key, v) in [
        ("Pnch", "Amnt", D::Boolean(true)),
        ("Twrl", "Angl", text("90")),
        ("Plr ", "Cnvr", D::Integer(1)),
        ("Sphr", "SphM", text("HrzO")),
        ("ZgZg", "NmbR", enm("Ordn", "Trgt")),
        ("Wave", "NmbG", D::Boolean(false)),
        ("Wave", "Wvtp", D::Integer(2)),
        ("Wave", "SclH", text("50")),
        ("Wave", "SclV", D::Boolean(true)),
        ("Wave", "UndA", D::Integer(1)),
        ("Wave", "RndS", text("7")),
    ] {
        assert!(kept(distort(code, desc("null").with(key, v.clone()))), "{code} with {key} = {v:?} was mapped");
    }
}

/// An integer outside the range Photoshop clamps the key to refuses the step: written as is, it
/// would load back clamped, as another value (#10605).
#[test]
fn distort_filter_steps_outside_photoshops_range_are_refused() {
    for (id, params) in [
        ("filter.distort.pinch", json!({"amount": 500})),
        ("filter.distort.spherize", json!({"amount": -101})),
        ("filter.distort.twirl", json!({"angle": 1000})),
        ("filter.distort.zigZag", json!({"ridges": 21})),
        ("filter.distort.zigZag", json!({"ridges": -1})),
        ("filter.distort.wave", json!({"generators": 0})),
        ("filter.distort.wave", json!({"amplitudeMax": 1000})),
    ] {
        assert!(export_item(id, &params, "S").is_none(), "{id} {params} exported");
    }
    // The limits themselves are written.
    for (id, params) in [
        ("filter.distort.pinch", json!({"amount": -100})),
        ("filter.distort.twirl", json!({"angle": 999})),
        ("filter.distort.zigZag", json!({"ridges": 0})),
        ("filter.distort.wave", json!({"generators": 64, "wavelengthMin": 1})),
    ] {
        assert!(export_item(id, &params, "S").is_some(), "{id} {params} refused");
    }
}

/// Wave's `RndS` is the seed's 32 bits, signed or not: it loads exactly or keeps the step,
/// never saturated or truncated to another seed (#10605).
#[test]
fn wave_seed_loads_its_exact_32_bits_or_keeps_the_step() {
    let seed = |v: D| match distort("Wave", desc("null").with("RndS", v)) {
        Mapped::Steps(s) => Some(s[0].1["seed"].clone()),
        Mapped::Unsupported { .. } => None,
    };
    assert_eq!(seed(D::LargeInteger(4_000_000_000)), Some(json!(4_000_000_000_u32)));
    assert_eq!(seed(D::LargeInteger(-1)), Some(json!(u32::MAX)));
    assert_eq!(seed(D::Integer(-7)), Some(json!(4_294_967_289_u32)));
    assert_eq!(seed(D::Double(12.0)), Some(json!(12)));
    for v in [D::Double(7.5), D::LargeInteger(4_294_967_296), D::LargeInteger(i64::from(i32::MIN) - 1)] {
        assert_eq!(seed(v.clone()), None, "RndS {v:?} was mapped");
    }
}

/// A step recording one of its keys twice can't be saved back with both values, so it stays a
/// Photoshop step instead of playing the first and dropping the second (#10605).
#[test]
fn distort_filter_steps_with_a_repeated_key_are_kept() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    for (code, key, a, b) in [
        ("Pnch", "Amnt", D::Integer(10), D::Integer(20)),
        ("Sphr", "SphM", enm("SphM", "Nrml"), enm("SphM", "HrzO")),
        ("Plr ", "Cnvr", enm("Cnvr", "RctP"), enm("Cnvr", "RctP")),
        ("Wave", "NmbG", D::Integer(5), D::Integer(7)),
        ("Wave", "SclH", D::Integer(100), D::Integer(50)),
        ("Wave", "UndA", enm("UndA", "WrpA"), enm("UndA", "RptE")),
        ("Wave", "RndS", D::Integer(1), D::Integer(2)),
        ("ZgZg", "NmbR", D::Integer(5), D::Integer(6)),
    ] {
        assert!(kept(distort(code, desc("null").with(key, a.clone()).with(key, b.clone()))), "{code} with {key} twice was mapped");
    }
}

/// An enumeration of another type isn't the setting Photoshop records, so the step stays a
/// Photoshop step instead of loading its value and saving back the expected type (#10605).
#[test]
fn distort_filter_steps_with_an_enumeration_of_another_type_are_kept() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    for (code, key, v) in [
        ("Plr ", "Cnvr", enm("Ordn", "RctP")),
        ("Sphr", "SphM", enm("Cnvr", "Nrml")),
        ("ZgZg", "ZZTy", enm("Ordn", "PndR")),
        ("Wave", "Wvtp", enm("WvSn", "WvSn")),
        ("Wave", "UndA", enm("Ordn", "WrpA")),
    ] {
        assert!(kept(distort(code, desc("null").with(key, v.clone()))), "{code} with {key} = {v:?} was mapped");
    }
    assert!(!kept(distort("Plr ", desc("null").with("Cnvr", enm("Cnvr", "PlrR")))));
    assert!(!kept(distort("Wave", desc("null").with("UndA", enm("UndA", "WrpA")))));
}

/// Photoshop clamps Wave's wavelengths and amplitudes to 1..999, but `filter.distort.wave`
/// takes a minimum wavelength of 1..998, a maximum of 2..999 and a minimum amplitude of 1..998.
/// A value the command rejects keeps the Photoshop step on import and refuses it on export,
/// rather than mapping a step that fails when it plays; a minimum above its maximum, which the
/// command takes, maps and plays (#10605).
#[test]
fn wave_values_the_command_rejects_are_kept_or_refused() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    for (key, n) in [("WLMn", 999), ("WLMn", 5000), ("WLMx", 1), ("WLMx", 0), ("AmMn", 999), ("AmMn", 1200)] {
        assert!(kept(distort("Wave", desc("null").with(key, D::Integer(n)))), "{key} {n} was mapped");
    }
    for (param, n) in [("wavelengthMin", 999), ("wavelengthMax", 1), ("amplitudeMin", 999)] {
        assert!(export_item("filter.distort.wave", &json!({ param: n }), "S").is_none(), "{param} {n} exported");
    }
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 24, "height": 24})).unwrap();
    s.execute("filter.render.clouds", json!({})).unwrap();
    let plays = [
        desc("null").with("WLMn", D::Integer(998)),
        desc("null").with("WLMx", D::Integer(2)),
        desc("null").with("AmMn", D::Integer(998)),
        desc("null").with("WLMn", D::Integer(200)).with("WLMx", D::Integer(100)),
        desc("null").with("AmMn", D::Integer(40)).with("AmMx", D::Integer(20)),
    ];
    for (k, d) in plays.into_iter().enumerate() {
        let (id, p) = one_step(distort("Wave", d));
        assert!(export_item(&id, &p, "S").is_some(), "{p} refused");
        let name = format!("W{k}");
        s.actions.list.push(Action::new(&name, vec![(id, p.clone())]));
        let played = s.execute("actions.play", json!({"action": name})).unwrap();
        assert_eq!(played["ran"], 1, "{p}: {played}");
    }
}

/// Wave saves its scales as the integer 100, so a fractional scale, even one that rounds or
/// clamps to 100, stays a Photoshop step instead of saving back as 100 (#10605).
#[test]
fn wave_with_a_fractional_scale_is_kept() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    for (key, n) in [("SclH", 99.6), ("SclV", 99.5), ("SclH", 100.4)] {
        assert!(kept(distort("Wave", desc("null").with(key, D::Double(n)))), "{key} {n} was mapped");
    }
    assert!(!kept(distort("Wave", desc("null").with("SclH", D::Integer(100)).with("SclV", D::Integer(100)))));
}

/// A Pattern Fill step whose `Ptrn` holds `id` and `name`, as the reference application records it.
fn pattern_fill_step(id: &str, name: &str) -> AtnItem {
    let t = desc("patternLayer").with("Ptrn", D::Descriptor(desc("Ptrn").with("Idnt", text(id)).with("Nm  ", text(name))));
    item(
        "Mk  ",
        "Make",
        Some(desc("null").with("null", class_ref("contentLayer")).with("Usng", D::Descriptor(desc("contentLayer").with("Type", D::Descriptor(t))))),
    )
}

/// Defines a 4×4 pattern of `color` named `name` in `s`'s library from its active document.
fn define_pattern(s: &mut Session, color: &str, name: &str) -> String {
    s.execute("edit.fill", json!({"contents": "color", "color": color})).unwrap();
    s.execute("edit.definePattern", json!({"name": name})).unwrap()["pattern"].as_str().unwrap().to_string()
}

fn pixel(s: &mut Session) -> Vec<f64> {
    let v = s.execute("document.pixel", json!({"x": 1, "y": 1})).unwrap();
    v.as_array().unwrap().iter().map(|c| c.as_f64().unwrap()).collect()
}

/// A Pattern Fill step whose `Ptrn` names both an installed id and a display name shared by
/// another pattern plays the pattern with that id; a foreign id falls back to the name.
#[test]
fn pattern_fill_prefers_an_installed_id_over_a_colliding_name() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
    let mut define = |color: &str| {
        s.execute("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        s.execute("edit.definePattern", json!({"name": "Twin"})).unwrap()["pattern"].as_str().unwrap().to_string()
    };
    let red = define("#ff0000");
    let blue = define("#0000ff");
    assert_ne!(red, blue);
    s.patterns.items.retain(|p| p.id == red || p.id == blue);
    s.patterns.items.sort_by_key(|p| p.id != blue);
    let fill_step = |id: &str| {
        let t = desc("patternLayer").with("Ptrn", D::Descriptor(desc("Ptrn").with("Idnt", text(id)).with("Nm  ", text("Twin"))));
        item(
            "Mk  ",
            "Make",
            Some(desc("null").with("null", class_ref("contentLayer")).with("Usng", D::Descriptor(desc("contentLayer").with("Type", D::Descriptor(t))))),
        )
    };
    let data = set_of("Pat", vec![("Installed", vec![fill_step(&red)]), ("Foreign", vec![fill_step("not-installed-id")])]);
    let r = s.execute("actions.load", json!({"data": data})).unwrap();
    assert!(r["unsupported"].as_array().is_none_or(|u| u.is_empty()), "{r}");
    for (action, want) in [("Installed", [1.0, 0.0, 0.0, 1.0]), ("Foreign", [0.0, 0.0, 1.0, 1.0])] {
        s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
        let r = s.execute("actions.play", json!({"action": action})).unwrap();
        assert!(r.get("failed").is_none(), "{r}");
        let v = s.execute("document.pixel", json!({"x": 1, "y": 1})).unwrap();
        let got: Vec<f64> = v.as_array().unwrap().iter().map(|c| c.as_f64().unwrap()).collect();
        assert_eq!(got, want, "{action}");
    }
}

/// The recorded id is looked up by id alone, in the document and then the library, before any
/// name: a document pattern *named* like the id does not beat the library pattern with that id,
/// and a foreign id that happens to be another pattern's name falls back to the recorded name.
/// An id and a name of the same pattern play it.
#[test]
fn pattern_fill_ids_beat_names_across_document_and_library() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
    let red = define_pattern(&mut s, "#ff0000", "Red");
    define_pattern(&mut s, "#0000ff", "Blue");
    let green = define_pattern(&mut s, "#00ff00", &red);
    let data = set_of(
        "Pat",
        vec![
            ("IdWins", vec![pattern_fill_step(&red, "Blue")]),
            ("NameFallback", vec![pattern_fill_step("Red", "Blue")]),
            ("Same", vec![pattern_fill_step(&red, "Red")]),
        ],
    );
    s.execute("actions.load", json!({"data": data})).unwrap();
    // Only the playback document holds the green pattern named like the red one's id.
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
    s.execute("layer.newFillLayer.pattern", json!({"pattern": green})).unwrap();
    s.execute("pattern.delete", json!({"pattern": green})).unwrap();
    assert!(s.active().unwrap().doc.patterns.iter().any(|p| p.id == green && p.name == red));
    assert_eq!(pixel(&mut s), [0.0, 1.0, 0.0, 1.0]);
    for (action, want) in [("IdWins", [1.0, 0.0, 0.0, 1.0]), ("NameFallback", [0.0, 0.0, 1.0, 1.0]), ("Same", [1.0, 0.0, 0.0, 1.0])] {
        let r = s.execute("actions.play", json!({"action": action})).unwrap();
        assert!(r.get("failed").is_none(), "{action}: {r}");
        assert_eq!(pixel(&mut s), want, "{action}");
    }
}

/// CodeRabbit on batch 11: a pattern step that names its pattern only by `patternName` exports
/// that name, rather than a step with no pattern (which plays the current one).
#[test]
fn pattern_steps_named_only_by_pattern_name_export_the_name() {
    let one = Action { name: "P".into(), steps: vec![("layer.newFillLayer.pattern".into(), json!({"patternName": "Checkerboard"}))], ..Default::default() };
    let (bytes, skipped) = export("Pat", &[&one], &[], false).unwrap();
    assert!(skipped.is_empty());
    let steps = import(&bytes).unwrap().actions[0].steps.clone();
    assert_eq!(steps[0].0, "layer.newFillLayer.pattern");
    assert_eq!(steps[0].1["pattern"], json!("Checkerboard"), "{}", steps[0].1);
}

/// CodeRabbit on batch 11: when the recorded id is installed nowhere, the recorded name is looked
/// up as a name only. A pattern whose id equals that name text doesn't beat the one named so.
#[test]
fn pattern_fill_name_fallback_matches_names_not_ids() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
    let red = define_pattern(&mut s, "#ff0000", "Red");
    // Green is named like the red pattern's id.
    define_pattern(&mut s, "#00ff00", &red);
    let data = set_of("Pat", vec![("NameIsAnId", vec![pattern_fill_step("5e1c0000-aaaa-4bbb-8ccc-000000000000", &red)])]);
    s.execute("actions.load", json!({"data": data})).unwrap();
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
    let r = s.execute("actions.play", json!({"action": "NameIsAnId"})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    assert_eq!(pixel(&mut s), [0.0, 1.0, 0.0, 1.0]);
}

/// A recorded pattern installed neither by id nor by name fails with an error that names the
/// pattern, from the imported step and from the Pattern Fill dialog's command alike.
#[test]
fn pattern_fill_missing_everywhere_names_the_pattern() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "mode": "rgb", "depth": 8})).unwrap();
    define_pattern(&mut s, "#ff0000", "Red");
    let data = set_of("Pat", vec![("Missing", vec![pattern_fill_step("5e1c0000-aaaa-4bbb-8ccc-000000000000", "My Wood")])]);
    s.execute("actions.load", json!({"data": data})).unwrap();
    let r = s.execute("actions.play", json!({"action": "Missing"})).unwrap();
    let err = r["failed"]["error"].as_str().unwrap_or_default();
    assert!(err.contains("\"My Wood\""), "{r}");
    s.execute("layer.newFillLayer.pattern", json!({"pattern": "Red"})).unwrap();
    let e = s.execute("layer.fill.pattern.set", json!({"pattern": "5e1c0000-aaaa-4bbb-8ccc-000000000000", "patternName": "My Wood"})).unwrap_err();
    assert!(e.to_string().contains("\"My Wood\""), "{e}");
}

/// FredApps review on #10459: a set whose name has surrounding spaces (a Photoshop `.atn` or an
/// older actions.json can have one, and New Action records into it under that exact name) saves
/// by that name. `actions.save` used to trim it and then find no set.
#[test]
fn save_finds_a_set_by_its_exact_name_with_spaces() {
    let mut s = Session::new();
    s.actions.list.push(Action { set: Some(" Web ".into()), ..Action::new("Old", Vec::new()) });
    s.execute("actions.record", json!({"name": "New", "set": " Web "})).unwrap();
    s.execute("actions.stop", json!({})).unwrap();
    let saved = s.execute("actions.save", json!({"set": " Web "})).unwrap();
    assert_eq!((&saved["set"], &saved["actions"]), (&json!(" Web "), &json!(["Old", "New"])));
    // The file keeps the name, as Photoshop's does.
    let mut reloaded = Session::new();
    let r = reloaded.execute("actions.load", json!({"data": saved["data"]})).unwrap();
    assert_eq!(r["set"], " Web ");
    // Other names are still trimmed.
    s.execute("actions.assignSet", json!({"action": "Old", "set": "Print"})).unwrap();
    assert_eq!(s.execute("actions.save", json!({"set": "  Print "})).unwrap()["set"], "Print");
}

/// FredApps review on #10459: Save Actions… on an empty set made with New Set writes it, as
/// Photoshop saves an empty set to the `.atn`, and loading that file brings the empty set back.
#[test]
fn an_empty_new_set_saves_and_loads_back() {
    let mut s = Session::new();
    s.execute("actions.newSet", json!({"name": "Later"})).unwrap();
    let saved = s.execute("actions.save", json!({"set": "Later"})).unwrap();
    assert_eq!((&saved["set"], &saved["actions"], &saved["steps"]), (&json!("Later"), &json!([]), &json!(0)));
    let mut other = Session::new();
    let r = other.execute("actions.load", json!({"data": saved["data"]})).unwrap();
    assert_eq!((&r["set"], &r["actions"]), (&json!("Later"), &json!([])));
    assert_eq!(crate::actions_edit_cmds::set_names(&other), ["Later"], "an empty folder");
    other.execute("actions.load", json!({"data": saved["data"]})).unwrap();
    assert_eq!(crate::actions_edit_cmds::set_names(&other), ["Later"], "loaded twice, still one folder");
    // New Action records into it as into any set, and it saves with that action.
    other.execute("actions.record", json!({"name": "Now", "set": "Later"})).unwrap();
    other.execute("actions.stop", json!({})).unwrap();
    assert_eq!(other.execute("actions.save", json!({"set": "Later"})).unwrap()["actions"], json!(["Now"]));
    // A set that doesn't exist is still refused, and so is saving every action when there is none.
    let err = s.execute("actions.save", json!({"set": "Nope"})).unwrap_err().to_string();
    assert!(err.contains("no actions in a set named `Nope`"), "{err}");
    let err = Session::new().execute("actions.save", json!({})).unwrap_err().to_string();
    assert!(err.contains("there are no actions to save"), "{err}");
}

/// A set from Chinese Photoshop: UTF-16 set and action names, a step name in GBK (可选颜色,
/// the bytes from #6743) on an event PhotoCraft has no command for, with no descriptor.
fn gbk_set() -> Vec<u8> {
    let step = AtnItem { dictionary: b"\xbf\xc9\xd1\xa1\xd1\xd5\xc9\xab".to_vec(), ..item("syntheticFill", "", None) };
    let file = AtnFile {
        name: UnicodeString::new_nul("相馆"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("调白"), items: vec![step], ..Default::default() }],
        ..Default::default()
    };
    atn::write(&file).unwrap()
}

#[test]
fn load_decodes_legacy_step_names() {
    let mut s = Session::new();
    let r = s.execute("actions.load", json!({"data": b64(&gbk_set())})).unwrap();
    assert_eq!(r["encoding"], "GBK", "{r}");
    assert_eq!(r["unsupported"][0]["name"], "可选颜色", "{r}");
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let played = s.execute("actions.play", json!({"action": "调白"})).unwrap();
    assert!(played["failed"]["error"].as_str().unwrap().contains("可选颜色"), "{played}");
}

#[test]
fn load_keeps_windows_1252_symbols() {
    // Windows Photoshop step names with no Unicode copy: Mac Roman reads ° and » as ∞ and ª,
    // which must not win ("50 ∞", "Grˆﬂe ª"). Save Actions writes the names back as that text.
    let steps = vec![
        AtnItem { dictionary: b"50 \xb0".to_vec(), ..item("syntheticFill", "", None) },
        AtnItem { dictionary: b"Gr\xf6\xdfe \xbb".to_vec(), ..item("syntheticFill", "", None) },
    ];
    let file = AtnFile {
        name: UnicodeString::new_nul("Größe ändern"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("Logo"), items: steps, ..Default::default() }],
        ..Default::default()
    };
    let mut s = Session::new();
    let r = s.execute("actions.load", json!({"data": b64(&atn::write(&file).unwrap())})).unwrap();
    assert_eq!(r["encoding"], "windows-1252", "{r}");
    assert_eq!(r["unsupported"][0]["name"], "50 °", "{r}");
    assert_eq!(r["unsupported"][1]["name"], "Größe »", "{r}");
    let saved = s.execute("actions.save", json!({"set": "Größe ändern"})).unwrap();
    let saved = atn::parse(&photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap()).unwrap();
    let names: Vec<String> = saved.actions[0].items.iter().map(|i| saved.item_label(i)).collect();
    assert_eq!(names, ["50 °", "Größe »"]);
}

#[test]
fn load_encoding_override() {
    let mut s = Session::new();
    let r = s.execute("actions.load", json!({"data": b64(&gbk_set()), "encoding": "shift_jis"})).unwrap();
    assert_eq!(r["encoding"], "Shift_JIS", "{r}");
    let r = s.execute("actions.load", json!({"data": b64(&gbk_set()), "encoding": "auto"})).unwrap();
    assert_eq!(r["encoding"], "GBK", "{r}");
}

#[test]
fn load_reports_utf8() {
    let mut s = Session::new();
    let r = s.execute("actions.load", json!({"data": b64(&atn::write(&sample()).unwrap())})).unwrap();
    assert_eq!(r["encoding"], "UTF-8", "{r}");
}

#[test]
fn load_rejects_bad_encodings() {
    let mut s = Session::new();
    for bad in [json!(3), json!(""), json!("nope"), json!("replacement"), json!(["gbk"])] {
        assert!(s.execute("actions.load", json!({"data": b64(&gbk_set()), "encoding": bad})).is_err(), "{bad}");
    }
    assert!(s.actions.list.is_empty(), "nothing is loaded when the encoding is bad");
}

/// Edit › Check Spelling in its recorded form: `spellCheck`, `null` = the `spellCheck` property of
/// all text layers, `Usng` = `checkAll`, with the step's dialog on.
fn spell_check(all: bool) -> Descriptor {
    desc("null")
        .with(
            "null",
            D::Reference(vec![
                ReferenceItem::Property { class: class("Prpr"), key: Id::new("spellCheck") },
                ReferenceItem::Enumerated { class: class("TxLr"), type_id: code("Ordn"), value: code("Al") },
            ]),
        )
        .with("Usng", D::Descriptor(desc("spellCheck").with("checkAll", D::Boolean(all))))
}

#[test]
fn check_spelling_loads_and_saves_as_the_spell_check_event() {
    for all in [true, false] {
        let mut step = item("spellCheck", "Check Spelling", Some(spell_check(all)));
        step.with_dialog = true;
        let file = AtnFile {
            name: UnicodeString::new_nul("S"),
            actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items: vec![step], ..Default::default() }],
            ..Default::default()
        };
        let imported = import(&atn::write(&file).unwrap()).unwrap();
        assert!(imported.unsupported.is_empty(), "{:?}", imported.unsupported);
        let action = &imported.actions[0];
        assert_eq!(action.steps, [("edit.checkSpelling".to_string(), json!({"allLayers": all}))]);
        assert_eq!(action.dialogs.iter().copied().collect::<Vec<_>>(), [0]);
        let (bytes, skipped) = export("S", &[action], &[], false).unwrap();
        assert!(skipped.is_empty());
        let saved = atn::parse(&bytes).unwrap();
        let saved = &saved.actions[0].items[0];
        assert_eq!(saved.event.id(), "spellCheck");
        assert!(saved.with_dialog);
        assert_eq!(saved.descriptor.as_ref().unwrap().to_bytes(), spell_check(all).to_bytes(), "checkAll {all}");
    }
}

#[test]
fn other_check_spelling_forms_stay_kept_steps_and_corrections_are_not_saved() {
    let event = event_for(&AtnEvent::name("spellCheck")).unwrap();
    let mut extra = spell_check(true);
    extra.items.push((Id::new("Fwd "), D::Boolean(true)));
    let mut other_target = spell_check(true);
    other_target.items[0].1 = D::Reference(vec![ReferenceItem::Property { class: class("Prpr"), key: Id::new("spellCheck") }]);
    let mut other_class = spell_check(true);
    other_class.items[1].1 = D::Descriptor(desc("findReplace").with("checkAll", D::Boolean(true)));
    let mut more_using = spell_check(true);
    more_using.items[1].1 = D::Descriptor(desc("spellCheck").with("checkAll", D::Boolean(true)).with("Fwd ", D::Boolean(true)));
    let mut twice = spell_check(true);
    twice.items[1] = twice.items[0].clone();
    for d in [extra, other_target, other_class, more_using, twice, desc("null")] {
        assert!(matches!((event.from)(&d), Mapped::Unsupported { harmless: false }), "{d:?}");
    }
    for params in [
        json!({"action": "change", "start": 0, "end": 3, "replace": "The"}),
        json!({"action": "changeAll", "word": "teh", "replace": "the"}),
        json!({"allLayers": true, "layer": "Title"}),
        json!({"allLayers": "yes"}),
    ] {
        assert!(export_item("edit.checkSpelling", &params, "S").is_none(), "{params}");
    }
}

/// Check Spelling's `null` and `Usng` are read by name: a step that holds them in the other
/// order loads as the same step.
#[test]
fn check_spelling_reads_its_keys_in_either_order() {
    let event = event_for(&AtnEvent::name("spellCheck")).unwrap();
    for all in [true, false] {
        let mut reversed = spell_check(all);
        reversed.items.reverse();
        match (event.from)(&reversed) {
            Mapped::Steps(steps) => assert_eq!(steps, [("edit.checkSpelling".to_string(), json!({"allLayers": all}))]),
            Mapped::Unsupported { .. } => panic!("Usng before null was kept: {reversed:?}"),
        }
    }
}

/// A setting recorded twice (`Wdth`, `Fade`, `Scl `, a gallery key) would play one value and
/// save one, dropping the other: such a step stays a Photoshop step (CodeRabbit on #10776).
#[test]
fn steps_that_record_a_setting_twice_are_kept() {
    let kept = |m: Mapped| matches!(m, Mapped::Unsupported { harmless: false });
    let identity = D::List((0..25).map(|i| D::Integer(i32::from(i == 12))).collect());
    let once = [
        ("Trap", desc("null").with("Wdth", px(2.0))),
        ("matchColor", desc("null").with("Fade", D::Integer(10))),
        ("Cstm", desc("null").with("Mtrx", identity.clone()).with("Scl ", D::Integer(3))),
    ];
    for (event, d) in once {
        assert!(!kept(channel_step(event, d.clone())), "{event} once was kept");
        let (key, value) = d.items.last().cloned().unwrap();
        let twice = Descriptor { items: [d.items.clone(), vec![(key, value)]].concat(), ..d };
        assert!(kept(channel_step(event, twice)), "{event} twice was mapped");
    }
    let edges = || desc("null").with("EdgW", D::Integer(2)).with("EdgB", D::Integer(38)).with("Smth", D::Integer(5));
    assert!(!kept(gallery_step("GEfc", "AccE", edges())));
    assert!(kept(gallery_step("GEfc", "AccE", edges().with("EdgW", D::Integer(9)))));
}

/// A gallery effect's own step whose panel name is in the file's legacy code page (GBK
/// "强化的边缘", not UTF-8) saves back with the same name bytes (CodeRabbit on #10776).
#[test]
fn gallery_effect_steps_keep_a_legacy_panel_name_byte_for_byte() {
    let edges = desc("null").with("GEfk", enm("GEft", "AccE")).with("EdgW", D::Integer(2)).with("EdgB", D::Integer(38)).with("Smth", D::Integer(5));
    let gbk = vec![0xC7, 0xBF, 0xBB, 0xAF, 0xB5, 0xC4, 0xB1, 0xDF, 0xD4, 0xB5];
    assert!(std::str::from_utf8(&gbk).is_err());
    let source = AtnItem { dictionary: gbk.clone(), ..item("AccE", "", Some(edges)) };
    let mut s = Session::new();
    let loaded = s.execute("actions.load", json!({"data": set_of("S", vec![("A", vec![source])])})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]), "{loaded}");
    let saved = s.execute("actions.save", json!({"set": "S"})).unwrap();
    let file = atn::parse(&photocraft_paint::tile::b64_decode(saved["data"].as_str().unwrap()).unwrap()).unwrap();
    let back = &file.actions[0].items[0];
    assert_eq!((back.event.clone(), back.dictionary.clone()), (AtnEvent::Code(*b"AccE"), gbk));
}

/// A Cmd-click that adds the Background (a property reference) to the selection keeps its
/// modifier, as a click on a named layer does, and saves back the same (CodeRabbit on #10776).
#[test]
fn a_background_selection_keeps_its_modifier() {
    let bckg = || D::Reference(vec![ReferenceItem::Property { class: class("Lyr "), key: code("Bckg") }]);
    for (modifier, mode) in [("addToSelection", "add"), ("removeFromSelection", "remove"), ("addToSelectionContinuous", "range")] {
        let d = desc("null").with("null", bckg()).with("selectionModifier", enm("selectionModifierType", modifier)).with("MkVs", D::Boolean(false));
        let loaded = mapped_recording("slct", d.clone());
        assert_eq!(loaded.actions[0].steps, vec![("layer.select".into(), json!({"layer":{"background":true},"mode":mode}))], "{modifier}");
        let (bytes, _) = export("S", &[&loaded.actions[0]], &[], false).unwrap();
        assert_eq!(atn::parse(&bytes).unwrap().actions[0].items[0].descriptor, Some(d));
    }
}

/// A Move whose destination is an ordinal of something other than a layer is no stack move:
/// it stays a Photoshop step (CodeRabbit on #10776).
#[test]
fn a_move_to_an_ordinal_that_is_not_a_layer_is_kept() {
    let ord = |cls: &str, o: &str| D::Reference(vec![ReferenceItem::Enumerated { class: class(cls), type_id: code("Ordn"), value: code(o) }]);
    let target = || D::Reference(vec![ReferenceItem::Enumerated { class: class("Lyr "), type_id: code("Ordn"), value: code("Trgt") }]);
    for o in ["Frnt", "Back", "Nxt ", "Prvs"] {
        let layer = mapped_recording("move", desc("null").with("null", target()).with("T   ", ord("Lyr ", o)));
        assert!(layer.unsupported.is_empty(), "{o} on a layer");
        let channel = mapped_recording("move", desc("null").with("null", target()).with("T   ", ord("Chnl", o)));
        assert_eq!(channel.unsupported.len(), 1, "{o} on a channel was mapped: {:?}", channel.actions[0].steps);
    }
}

/// Nonfinite recorded doubles remain unsupported before native parameters can be constructed.
#[test]
fn shear_nonfinite_coordinates_stay_kept() {
    let point = |x, y| D::Descriptor(desc("Pnt ").with("Hrzn", D::Double(x)).with("Vrtc", D::Double(y)));
    let finite = desc("null").with("ShrP", D::List(vec![point(-10000.0, -1000.0), point(10000.0, 1000.0)]));
    assert!(matches!(steps_distort::shear_from(&finite), Mapped::Steps(_)), "finite coordinates still clamp and map");
    for n in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for d in [
            desc("null").with("ShrP", D::List(vec![point(n, 1.0), point(0.0, 128.0)])),
            desc("null").with("ShrP", D::List(vec![point(0.0, 1.0), point(0.0, n)])),
        ] {
            assert!(matches!(steps_distort::shear_from(&d), Mapped::Unsupported { .. }), "{d:?}");
        }
    }
}

/// #10771: a Gradient Fill as Photoshop records it: three stops, a transparency stop at 40 % and
/// one midpoint per stop (`Mdpn` of stop k is the segment that ends at it).
fn recorded_gradient(mids: [i32; 3]) -> Descriptor {
    let stop = |hex: &str, at: i32, mid: i32| {
        D::Descriptor(
            desc("Clrt")
                .with("Clr ", D::Descriptor(color_desc(hex).unwrap()))
                .with("Type", enm("Clry", "UsrS"))
                .with("Lctn", D::Integer(at))
                .with("Mdpn", D::Integer(mid)),
        )
    };
    let trn = |at: i32, o: f64| D::Descriptor(desc("TrnS").with("Opct", pct(o)).with("Lctn", D::Integer(at)).with("Mdpn", D::Integer(50)));
    let grad = desc("Grdn")
        .with("Nm  ", text("Custom"))
        .with("GrdF", enm("GrdF", "CstS"))
        .with("Intr", D::Double(4096.0))
        .with("Clrs", D::List(vec![stop("#ff0000", 0, mids[0]), stop("#00ff00", 2048, mids[1]), stop("#0000ff", 4096, mids[2])]))
        .with("Trns", D::List(vec![trn(0, 100.0), trn(4096, 40.0)]));
    desc("gradientLayer").with("Angl", ang(30.0)).with("Type", enm("GrdT", "Rdl ")).with("Grad", D::Descriptor(grad))
}

fn gradient_set(t: Descriptor) -> AtnItem {
    item("setd", "Set", Some(desc("null").with("null", target_ref("contentLayer")).with("T   ", D::Descriptor(t))))
}

fn gradient_make(t: Descriptor) -> AtnItem {
    let d = desc("null").with("null", class_ref("contentLayer")).with("Usng", D::Descriptor(desc("contentLayer").with("Type", D::Descriptor(t))));
    item("Mk  ", "Make", Some(d))
}

/// The first saved step of `action` (strict save).
fn saved_first(s: &mut Session) -> (AtnEvent, Option<Descriptor>) {
    let r = s.execute("actions.save", json!({"action": "A"})).unwrap();
    assert_eq!(r["skipped"], json!([]), "{r}");
    let saved = atn::parse(&photocraft_paint::tile::b64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap();
    let i = &saved.actions[0].items[0];
    (i.event.clone(), i.descriptor.clone())
}

fn without_hidden(p: &Value) -> Value {
    let mut p = p.clone();
    p.as_object_mut().unwrap().remove("__atnDescriptor");
    normalise(&mut p);
    p
}

fn active_fill(s: &Session) -> photocraft_doc::Fill {
    let st = s.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        photocraft_doc::LayerContent::Fill(f) => f.clone(),
        c => panic!("not a fill layer: {}", c.kind_name()),
    }
}

/// #10771: a recorded Set of a Gradient Fill loads as `gradient.fill.set` with its midpoints on
/// the segment they belong to, plays them, and saves back as the recorded descriptor.
#[test]
fn gradient_fill_set_imports_midpoints_plays_and_saves_back_unchanged() {
    let recorded = recorded_gradient([50, 25, 70]);
    let mut s = Session::new();
    let loaded = s.execute("actions.load", json!({"data": b64(&atn::write(&one_action(vec![gradient_set(recorded.clone())])).unwrap())})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]), "{loaded}");
    let (id, p) = s.actions.list[0].steps[0].clone();
    assert_eq!(id, "gradient.fill.set");
    let mut expected = json!({
        "style": "radial", "angle": 30,
        "stops": [[0, "#ff0000"], [0.5, "#00ff00"], [1, "#0000ff"]],
        "transparency": [[0, 100], [1, 40]],
        "midpoints": [0.25, 0.7]
    });
    normalise(&mut expected);
    assert_eq!(without_hidden(&p), expected);
    assert_eq!(
        saved_first(&mut s),
        (AtnEvent::Code(*b"setd"), Some(desc("null").with("null", target_ref("contentLayer")).with("T   ", D::Descriptor(recorded))))
    );
    // It plays on a Gradient Fill layer.
    s.execute("file.new", json!({"width": 16, "height": 12})).unwrap();
    s.execute("layer.newFillLayer.gradient", json!({"from": "#000000", "to": "#ffffff"})).unwrap();
    let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    let photocraft_doc::Fill::Gradient { stops, midpoints, opacity_stops, .. } = active_fill(&s) else { panic!("not a gradient") };
    assert_eq!(stops.len(), 3);
    assert_eq!(midpoints, vec![0.25, 0.7]);
    assert_eq!(opacity_stops.len(), 2);
    // Default midpoints load as `[]`, which clears the custom ones when played.
    let mut s2 = Session::new();
    s2.execute("actions.load", json!({"data": b64(&atn::write(&one_action(vec![gradient_set(recorded_gradient([50, 50, 50]))])).unwrap())})).unwrap();
    assert_eq!(s2.actions.list[0].steps[0].1["midpoints"], json!([]));
    s.actions = s2.actions;
    s.execute("actions.play", json!({"action": "A"})).unwrap();
    let photocraft_doc::Fill::Gradient { midpoints, .. } = active_fill(&s) else { panic!("not a gradient") };
    assert!(midpoints.is_empty() || midpoints.iter().all(|m| *m == 0.5), "{midpoints:?}");
}

/// #10771: an edited loaded step writes a fresh descriptor from its params, midpoints included.
#[test]
fn edited_gradient_fill_set_writes_a_fresh_descriptor() {
    let mut s = Session::new();
    s.execute("actions.load", json!({"data": b64(&atn::write(&one_action(vec![gradient_set(recorded_gradient([50, 25, 70]))])).unwrap())})).unwrap();
    s.actions.list[0].steps[0].1["midpoints"] = json!([0.4, 0.6]);
    let (_, d) = saved_first(&mut s);
    let t = get_obj(d.as_ref().unwrap(), "T   ").unwrap();
    let clrs = get_list(get_obj(t, "Grad").unwrap(), "Clrs").unwrap();
    let mids: Vec<f64> = clrs.iter().map(|c| if let D::Descriptor(c) = c { get_num(c, "Mdpn").unwrap() } else { panic!() }).collect();
    assert_eq!(mids, vec![50.0, 40.0, 60.0]);
    // Its own import is the edited params.
    let Mapped::Steps(steps) = (event_for(&AtnEvent::Code(*b"setd")).unwrap().from)(d.as_ref().unwrap()) else { panic!("did not import") };
    let mut edited = without_hidden(&s.actions.list[0].steps[0].1);
    edited["midpoints"] = json!([0.4, 0.6]);
    normalise(&mut edited);
    assert_eq!(without_hidden(&steps[0].1), edited);
}

/// #10771: a recorded Make of a Gradient Fill layer plays its stops, transparency and midpoints
/// and saves back unchanged.
#[test]
fn gradient_fill_make_imports_plays_and_saves_back_unchanged() {
    let recorded = recorded_gradient([50, 25, 70]);
    let mut s = Session::new();
    let loaded = s.execute("actions.load", json!({"data": b64(&atn::write(&one_action(vec![gradient_make(recorded.clone())])).unwrap())})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]), "{loaded}");
    assert_eq!(s.actions.list[0].steps[0].0, "layer.newFillLayer.gradient");
    let (event, d) = saved_first(&mut s);
    assert_eq!(event, AtnEvent::Code(*b"Mk  "));
    assert_eq!(get_obj(get_obj(d.as_ref().unwrap(), "Usng").unwrap(), "Type"), Some(&recorded));
    for depth in [8, 16] {
        s.execute("file.new", json!({"width": 16, "height": 12, "depth": depth})).unwrap();
        let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
        assert!(played.get("failed").is_none(), "{played}");
        let photocraft_doc::Fill::Gradient { stops, midpoints, opacity_stops, .. } = active_fill(&s) else { panic!("not a gradient") };
        assert_eq!((stops.len(), midpoints, opacity_stops.len()), (3, vec![0.25, 0.7], 2));
        // One undo step.
        s.execute("edit.undo", json!({})).unwrap();
        assert!(!matches!(s.active().unwrap().doc.layers.last().unwrap().content, photocraft_doc::LayerContent::Fill(_)));
    }
}

/// #10771: PhotoCraft's own Gradient Fill steps export and load back as the same fill.
#[test]
fn gradient_fill_steps_export_and_import_back() {
    let set = json!({"style": "linear", "angle": 45, "stops": [[0, "#ff0000"], [0.25, "#00ff00"], [1, "#0000ff"]], "transparency": [[0, 100], [1, 50]], "midpoints": [0.3, 0.6]});
    let item = export_item("gradient.fill.set", &set, "S").unwrap();
    let Mapped::Steps(steps) = (event_for(&item.event).unwrap().from)(item.descriptor.as_ref().unwrap()) else { panic!("did not import") };
    let mut expected = set.clone();
    normalise(&mut expected);
    assert_eq!((steps[0].0.as_str(), without_hidden(&steps[0].1)), ("gradient.fill.set", expected));
    // The two-colour Make plays as the same two stops at full opacity.
    let item = export_item("layer.newFillLayer.gradient", &json!({"from": "#ff0000", "to": "#0000ff", "angle": 45}), "S").unwrap();
    let Mapped::Steps(steps) = (event_for(&item.event).unwrap().from)(item.descriptor.as_ref().unwrap()) else { panic!("did not import") };
    let mut expected = json!({"angle": 45, "stops": [[0, "#ff0000"], [1, "#0000ff"]], "transparency": [[0, 100], [1, 100]], "midpoints": []});
    normalise(&mut expected);
    assert_eq!((steps[0].0.as_str(), without_hidden(&steps[0].1)), ("layer.newFillLayer.gradient", expected));
    // Stops replace `from` and `to` when it plays, so a Make with all three exports its stops.
    let p = json!({"from": "#ff0000", "to": "#0000ff", "stops": [[0, "#00ff00"], [1, "#ffffff"]], "transparency": [[0, 100], [1, 100]]});
    let item = export_item("layer.newFillLayer.gradient", &p, "S").unwrap();
    let Mapped::Steps(steps) = (event_for(&item.event).unwrap().from)(item.descriptor.as_ref().unwrap()) else { panic!("did not import") };
    assert_eq!(steps[0].1["stops"], json!([[0.0, "#00ff00"], [1.0, "#ffffff"]]));
    // What the event cannot hold is refused: a Set without its transparency, toolbox colours,
    // dragged handles, a path, a midpoint off a whole percent.
    for (id, p) in [
        ("gradient.fill.set", json!({"stops": [[0, "#ff0000"], [1, "#0000ff"]]})),
        ("gradient.fill.set", json!({"stops": [[0, "foreground"], [1, "#0000ff"]], "transparency": [[0, 100], [1, 100]]})),
        ("gradient.fill.set", json!({"from": [0, 0], "stops": [[0, "#ff0000"], [1, "#0000ff"]], "transparency": [[0, 100], [1, 100]]})),
        ("gradient.fill.set", json!({"stops": [[0, "#ff0000"], [1, "#0000ff"]], "transparency": [[0, 100], [1, 100]], "midpoints": [0.333]})),
        ("layer.newFillLayer.gradient", json!({"from": "#ff0000", "to": "#0000ff", "path": "work"})),
        ("layer.newFillLayer.gradient", json!({"from": "#ff0000"})),
    ] {
        assert!(export_item(id, &p, "S").is_none(), "{id}: {p}");
    }
}

/// #10771: a Noise gradient, smoothness below 100 % or a transparency midpoint stay Photoshop
/// steps rather than playing as a different fill.
#[test]
fn gradient_fills_the_command_cannot_play_stay_photoshop_steps() {
    // `with` appends; these replace the recorded value.
    fn replaced(mut d: Descriptor, key: &str, v: D) -> Descriptor {
        d.items.retain(|(k, _)| !k.is(key));
        d.with(key, v)
    }
    let base = recorded_gradient([50, 50, 50]);
    let with_grad = |f: &dyn Fn(Descriptor) -> Descriptor| {
        let g = get_obj(&base, "Grad").unwrap().clone();
        replaced(base.clone(), "Grad", D::Descriptor(f(g)))
    };
    let trns_mid = desc("TrnS").with("Opct", pct(100.0)).with("Lctn", D::Integer(0)).with("Mdpn", D::Integer(30));
    let trns_end = desc("TrnS").with("Opct", pct(100.0)).with("Lctn", D::Integer(4096)).with("Mdpn", D::Integer(50));
    let trns = |at: i32, o: f64| D::Descriptor(desc("TrnS").with("Opct", pct(o)).with("Lctn", D::Integer(at)).with("Mdpn", D::Integer(50)));
    for t in [
        // A midpoint outside the dialog's 5..95 % would play clamped.
        recorded_gradient([50, 2, 50]),
        recorded_gradient([50, 50, 97]),
        // Transparency stops out of range or out of order would play clamped and sorted.
        with_grad(&|g| replaced(g, "Trns", D::List(vec![trns(0, 100.0), trns(5000, 40.0)]))),
        with_grad(&|g| replaced(g, "Trns", D::List(vec![trns(0, 150.0), trns(4096, 40.0)]))),
        with_grad(&|g| replaced(g, "Trns", D::List(vec![trns(4096, 100.0), trns(0, 40.0)]))),
        // A Set without transparency stops would keep the layer's own.
        with_grad(&|mut g| {
            g.items.retain(|(k, _)| !k.is("Trns"));
            g
        }),
        with_grad(&|g| replaced(g, "GrdF", enm("GrdF", "ClNs"))),
        with_grad(&|g| replaced(g, "Intr", D::Double(2048.0))),
        with_grad(&|g| replaced(g, "Trns", D::List(vec![D::Descriptor(trns_mid.clone()), D::Descriptor(trns_end.clone())]))),
        base.clone().with("gradientsInterpolationMethod", enm("gradientInterpolationMethodType", "Perc")),
    ] {
        let imp = import(&atn::write(&one_action(vec![gradient_set(t)])).unwrap()).unwrap();
        assert_eq!(imp.actions[0].steps[0].0, PHOTOSHOP_STEP);
    }
    // A Make whose new layer also gets a name, opacity, mode or clipping: the command plays none
    // of them, so the step stays as recorded.
    for (key, v) in [("Nm  ", text("Sky")), ("Opct", pct(50.0)), ("Md  ", enm("BlnM", "Mltp")), ("Grup", D::Boolean(true))] {
        let using = desc("contentLayer").with("Type", D::Descriptor(base.clone())).with(key, v);
        let make = item("Mk  ", "Make", Some(desc("null").with("null", class_ref("contentLayer")).with("Usng", D::Descriptor(using))));
        let imp = import(&atn::write(&one_action(vec![make])).unwrap()).unwrap();
        assert_eq!(imp.actions[0].steps[0].0, PHOTOSHOP_STEP, "{key}");
    }
}

fn blend_if_entry(channel: &str, this: [i32; 4], under: Option<[i32; 4]>) -> D {
    let mut d = desc("Blnd").with("Chnl", channel_ref(Some(channel)));
    for (k, v) in ["SrcB", "Srcl", "SrcW", "Srcm"].into_iter().zip(this) {
        d = d.with(k, D::Integer(v));
    }
    if let Some(u) = under {
        for (k, v) in ["DstB", "Dstl", "DstW", "Dstt"].into_iter().zip(u) {
            d = d.with(k, D::Integer(v));
        }
    }
    D::Descriptor(d)
}

fn layer_set(t: Descriptor) -> AtnItem {
    item("setd", "Set", Some(desc("null").with("null", target_ref("Lyr ")).with("T   ", D::Descriptor(t))))
}

fn blend_layers(depth: u8) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"color": "#808080"})).unwrap();
    s
}

fn active_info(s: &Session) -> Value {
    let st = s.active().unwrap();
    crate::inspect::layer(st.doc.layer(st.active_layer.unwrap()).unwrap())
}

/// #10771: Blend If loads as `layer.layerStyle.blendingOptions` and plays (it used to be a
/// silent no-op), on 8- and 16-bit documents, Gray and a colour channel, split sliders.
#[test]
fn blend_if_imports_and_plays() {
    let blnd = D::List(vec![blend_if_entry("gray", [0, 30, 200, 255], Some([10, 10, 240, 250])), blend_if_entry("red", [20, 40, 255, 255], None)]);
    for depth in [8, 16] {
        let mut s = blend_layers(depth);
        let loaded = s
            .execute("actions.load", json!({"data": b64(&atn::write(&one_action(vec![layer_set(desc("Lyr ").with("Blnd", blnd.clone()))])).unwrap())}))
            .unwrap();
        assert_eq!(loaded["unsupported"], json!([]), "{loaded}");
        let mut expected = json!({"blendIf": [{"channel": "gray", "thisLayer": [0, 30, 200, 255], "underlying": [10, 10, 240, 250]}, {"channel": "red", "thisLayer": [20, 40, 255, 255]}]});
        normalise(&mut expected);
        let (id, p) = &s.actions.list[0].steps[0];
        assert_eq!((id.as_str(), p), ("layer.layerStyle.blendingOptions", &expected));
        let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
        assert!(played.get("failed").is_none(), "{played}");
        let b = active_info(&s)["blendIf"].clone();
        assert_eq!(b[0], json!({"channel": 0, "thisLayer": [0, 30, 200, 255], "underlying": [10, 10, 240, 250]}), "{b}");
        assert_eq!(b[1], json!({"channel": 1, "thisLayer": [20, 40, 255, 255], "underlying": [0, 0, 255, 255]}), "{b}");
        // It saves back as the same `Blnd`.
        let (_, d) = saved_first(&mut s);
        assert_eq!(get_obj(d.as_ref().unwrap(), "T   ").unwrap().get("Blnd"), Some(&blnd));
    }
}

/// #10771: the Blending Options dialog's OK (opacity, fill, mode and Blend If in one Set) is one
/// step that applies them all; a Set that also renames splits into setProps and Blend If.
#[test]
fn blend_if_with_the_dialog_options_is_one_step_and_a_rename_splits() {
    let blnd = D::List(vec![blend_if_entry("gray", [0, 0, 128, 255], None)]);
    let dialog = desc("Lyr ").with("Opct", pct(40.0)).with("fillOpacity", pct(70.0)).with("Md  ", enm("BlnM", "Mltp")).with("Blnd", blnd.clone());
    let mut s = blend_layers(8);
    s.execute("actions.load", json!({"data": b64(&atn::write(&one_action(vec![layer_set(dialog.clone())])).unwrap())})).unwrap();
    let steps = &s.actions.list[0].steps;
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert_eq!(steps[0].0, "layer.layerStyle.blendingOptions");
    let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    let info = active_info(&s);
    assert!((info["opacity"].as_f64().unwrap() - 0.4).abs() < 1e-3, "{info}");
    assert_eq!(info["blend"], json!("Multiply"), "{info}");
    assert_eq!(info["blendIf"][0]["thisLayer"], json!([0, 0, 128, 255]), "{info}");
    let (_, d) = saved_first(&mut s);
    assert_eq!(get_obj(d.as_ref().unwrap(), "T   "), Some(&dialog));
    // With a rename the Set is not the dialog's: the name and opacity go through setProps.
    let mut s = blend_layers(8);
    s.execute("actions.load", json!({"data": b64(&atn::write(&one_action(vec![layer_set(desc("Lyr ").with("Nm  ", text("Renamed")).with("Opct", pct(40.0)).with("Blnd", blnd))])).unwrap())}))
        .unwrap();
    let ids: Vec<&str> = s.actions.list[0].steps.iter().map(|(id, _)| id.as_str()).collect();
    assert!(ids.contains(&"layer.setProps") && ids.contains(&"layer.layerStyle.blendingOptions"), "{ids:?}");
    let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    let info = active_info(&s);
    assert_eq!(info["name"], json!("Renamed"));
    assert_eq!(info["blendIf"][0]["thisLayer"], json!([0, 0, 128, 255]), "{info}");
}

/// #10771: the Layer Style dialog's OK on a layer without effects also records the effects'
/// scale (`Lefx` = {Scl}). With Blend If it is still the dialog's one step and saves back with it.
#[test]
fn blend_if_with_the_recorded_effects_scale_saves_back_unchanged() {
    let blnd = D::List(vec![blend_if_entry("gray", [10, 20, 200, 230], Some([0, 0, 255, 255]))]);
    let dialog = desc("Lyr ").with("Opct", pct(80.0)).with("Blnd", blnd.clone()).with("Lefx", D::Descriptor(desc("Lefx").with("Scl ", pct(100.0))));
    let mut s = blend_layers(8);
    s.execute("actions.load", json!({"data": b64(&atn::write(&one_action(vec![layer_set(dialog.clone())])).unwrap())})).unwrap();
    let steps = &s.actions.list[0].steps;
    assert_eq!(steps.len(), 1, "{steps:?}");
    assert_eq!(steps[0].0, "layer.layerStyle.blendingOptions");
    let played = s.execute("actions.play", json!({"action": "A"})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    let info = active_info(&s);
    assert!((info["opacity"].as_f64().unwrap() - 0.8).abs() < 1e-3, "{info}");
    assert_eq!(info["blendIf"][0]["thisLayer"], json!([10, 20, 200, 230]), "{info}");
    let (_, d) = saved_first(&mut s);
    assert_eq!(get_obj(d.as_ref().unwrap(), "T   "), Some(&dialog));
    // A `Lefx` with an effect in it is not the scale alone: the step is not the dialog's.
    let with_fx = desc("Lyr ").with("Blnd", blnd).with("Lefx", D::Descriptor(desc("Lefx").with("Scl ", pct(100.0)).with("DrSh", D::Descriptor(desc("DrSh")))));
    let imp = import(&atn::write(&one_action(vec![layer_set(with_fx)])).unwrap()).unwrap();
    assert!(imp.actions[0].steps.iter().all(|(_, p)| p.get("__atnEffectsScale").is_none()), "{:?}", imp.actions[0].steps);
    // Nor can it play without the shadow: it stays the recorded step.
    assert_eq!(imp.actions[0].steps.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(), [PHOTOSHOP_STEP], "{:?}", imp.actions[0].steps);
}

/// #10771: Blend If entries that are not whole levels, out of order, with half the Underlying
/// sliders or an unknown key stay Photoshop steps.
#[test]
fn blend_if_the_command_cannot_play_stays_a_photoshop_step() {
    let with = |key: &str| match blend_if_entry("gray", [0, 0, 255, 255], None) {
        D::Descriptor(d) => D::Descriptor(d.with(key, D::Integer(3))),
        e => e,
    };
    let (half, unknown) = (with("DstB"), with("Zzzz"));
    let inverted = blend_if_entry("gray", [0, 200, 100, 255], None);
    for e in [blend_if_entry("gray", [0, 0, 300, 255], None), inverted, half, unknown] {
        let imp = import(&atn::write(&one_action(vec![layer_set(desc("Lyr ").with("Blnd", D::List(vec![e])))])).unwrap()).unwrap();
        assert_eq!(imp.actions[0].steps[0].0, PHOTOSHOP_STEP);
    }
    for p in [
        json!({"blendIf": null}),
        json!({"blendIf": {"channel": 2, "thisLayer": [0, 255]}}),
        json!({"blendIf": {"channel": "gray", "thisLayer": [200, 100]}}),
        json!({"blendIf": {"channel": "gray", "underlying": [0, 255]}}),
    ] {
        assert!(export_item("layer.layerStyle.blendingOptions", &p, "S").is_none(), "{p}");
    }
}

/// Save one action of steps through `actions.save`, returning the skipped list and the steps
/// `actions.load` reads back.
fn save_and_reload(steps: Vec<(String, Value)>) -> (Value, Vec<(String, Value)>) {
    let mut s = Session::new();
    let mut action = Action::new("A", steps);
    action.set = Some("Review".into());
    s.actions.list.push(action);
    let saved = s.execute("actions.save", json!({"set": "Review", "skipUnsupported": true})).unwrap();
    let mut fresh = Session::new();
    fresh.execute("actions.load", json!({"data": saved["data"]})).unwrap();
    (saved["skipped"].clone(), fresh.actions.list[0].steps.clone())
}

/// Calculations plays an omitted blending as Multiply, so Save Actions writes it as Multiply.
#[test]
fn calculations_without_blending_saves_as_multiply() {
    let sources = json!({"source1": {"channel": {"color": "red"}}, "source2": {"channel": {"color": "green"}}});
    for blending in [None, Some(Value::Null)] {
        let mut params = sources.clone();
        if let Some(b) = blending.clone() {
            params["blending"] = b;
        }
        let (skipped, steps) = save_and_reload(vec![("image.calculations".into(), params)]);
        assert_eq!(skipped, json!([]), "{blending:?}");
        let mut expected = sources.clone();
        expected["blending"] = json!("multiply");
        assert_eq!(steps, vec![("image.calculations".to_string(), expected)], "{blending:?}");
    }
    let mut unknown = sources.clone();
    unknown["blending"] = json!("bogus");
    assert_eq!(save_and_reload(vec![("image.calculations".into(), unknown)]).0.as_array().map(Vec::len), Some(1));
}

/// A transparency source with no layer reads the active layer's transparency; Save Actions
/// refuses it rather than write the merged transparency. Merged and named layers still save.
#[test]
fn apply_image_active_layer_transparency_is_not_saved_as_merged() {
    for source in [json!({"channel": "transparency"}), json!({"channel": "Transparency", "layer": null})] {
        let (skipped, _) = save_and_reload(vec![("image.applyImage".into(), json!({"source": source, "blending": "normal"}))]);
        assert_eq!(skipped.as_array().map(Vec::len), Some(1), "{source}: {skipped}");
    }
    for layer in ["merged", "Pixels"] {
        let params = json!({"source": {"channel": "transparency", "layer": layer}, "blending": "normal"});
        let (skipped, steps) = save_and_reload(vec![("image.applyImage".into(), params)]);
        assert_eq!(skipped, json!([]), "{layer}");
        assert_eq!(steps[0].1["source"]["layer"], layer, "{layer}: {steps:?}");
    }
    // Other channels keep the merged default for an omitted layer.
    let (skipped, steps) = save_and_reload(vec![("image.applyImage".into(), json!({"source": {"channel": "red"}, "blending": "normal"}))]);
    assert_eq!(skipped, json!([]));
    assert_eq!(steps[0].1["source"]["layer"], "merged");
}

fn make_levels_layer(using: Descriptor) -> Imported {
    let levels = desc("Lvls").with("Adjs", D::List(vec![D::Descriptor(desc("LvlA").with("Chnl", channel_ref(None)).with("Gmm ", D::Double(1.2)))]));
    let make = desc("null").with("null", class_ref("AdjL")).with("Usng", D::Descriptor(using.with("Type", D::Descriptor(levels))));
    let file = AtnFile {
        name: UnicodeString::new_nul("S"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items: vec![item("Mk  ", "Make", Some(make))], ..Default::default() }],
        ..Default::default()
    };
    import(&atn::write(&file).unwrap()).unwrap()
}

/// A Make adjustment layer takes `Md  ` as its blend mode only when it is a `BlnM` enumeration.
#[test]
fn adjustment_layer_blend_mode_needs_a_blend_mode_enumeration() {
    let imported = make_levels_layer(desc("AdjL").with("Md  ", enm("BlnM", "Mltp")));
    assert!(imported.unsupported.is_empty());
    assert_eq!(imported.actions[0].steps[0].1["mode"], "Multiply");
    let imported = make_levels_layer(desc("AdjL").with("Md  ", enm("Ordn", "Mltp")));
    assert_eq!(imported.unsupported.len(), 1);
    assert_eq!(imported.actions[0].steps[0].0, PHOTOSHOP_STEP);
    // A plain Make layer drops it rather than read another enumeration as a mode.
    let make = desc("null").with("null", class_ref("Lyr ")).with("Usng", D::Descriptor(desc("Lyr ").with("Md  ", enm("Ordn", "Mltp"))));
    let file = AtnFile {
        name: UnicodeString::new_nul("S"),
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items: vec![item("Mk  ", "Make", Some(make))], ..Default::default() }],
        ..Default::default()
    };
    let imported = import(&atn::write(&file).unwrap()).unwrap();
    assert_eq!(imported.actions[0].steps[0].0, "layer.new.layer");
    assert!(imported.actions[0].steps[0].1.get("mode").is_none(), "{:?}", imported.actions[0].steps);
}

/// An adjustment layer name that would not save back unchanged keeps the whole step.
#[test]
fn adjustment_layer_name_must_round_trip() {
    for (name, kept) in [
        (UnicodeString::new_nul("Grade"), true),
        (UnicodeString::new("Grade"), true),
        (UnicodeString(vec![u16::from(b'G'), 0xD800, 0]), false),
        (UnicodeString(vec![u16::from(b'G'), 0, 0]), false),
    ] {
        let imported = make_levels_layer(desc("AdjL").with("Nm  ", D::Text(name.clone())));
        let step = &imported.actions[0].steps[0];
        if kept {
            assert!(imported.unsupported.is_empty(), "{name:?}");
            assert_eq!(step.1["name"], "Grade", "{name:?}");
        } else {
            assert_eq!(step.0, PHOTOSHOP_STEP, "{name:?}");
        }
    }
}
