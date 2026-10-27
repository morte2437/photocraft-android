use super::*;
use photocraft_psd::atn::{AtnAction, AtnEvent, AtnFile, AtnItem};
use photocraft_psd::descriptor::{Descriptor, Id, ReferenceItem, UnicodeString, Value as D};
use serde_json::json;

#[test]
fn blend_modes_accept_photoshop_string_ids_and_export_canonical_char_ids() {
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
        ("passThrough", "PsTh", "Pass Through"),
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
                    item("make", "Make", Some(make_levels)),
                    item("setd", "Set", Some(select_all)),
                    item("select", "Select", Some(select_layer)),
                    item("slct", "Select", Some(tool)),
                    item("stop", "Stop", Some(stop)),
                    item("syntheticFill", "Generative Fill", Some(fancy)),
                    item("play", "Play", Some(play)),
                    off,
                ],
                ..Default::default()
            },
            AtnAction { name: UnicodeString::new_nul("Finish"), items: vec![item("Invs", "Inverse", None)], ..Default::default() },
        ],
        warnings: Vec::new(),
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
        Some(target_ref("compsClass")),
        Some(class_ref("compsClass")),
        Some(D::Reference(vec![ReferenceItem::Index { class: class("compsClass"), index: 1 }])),
        Some(D::Reference(vec![ReferenceItem::Identifier { class: class("compsClass"), id: 1 }])),
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
    s.actions.list.push(Action::new("Ours", vec![("filter.render.clouds".into(), json!({}))]));
    let e = s.execute("actions.save", json!({"action": "Ours"})).unwrap_err().to_string();
    assert!(e.contains("filter.render.clouds"), "{e}");
    let r = s.execute("actions.save", json!({"action": "Ours", "skipUnsupported": true})).unwrap();
    assert_eq!(r["skipped"], json!([{"action": "Ours", "step": 0, "id": "filter.render.clouds"}]));
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

/// A setting Photoshop's event has no field for is dropped by export, and the step then plays back
/// as a different edit: Crystallize's `seed` (#7668), Radial Blur's centre, Minimum's and
/// Maximum's `preserve`. Such a step is refused, as one with no equivalent event already is.
#[test]
fn a_setting_the_event_cannot_hold_is_refused() {
    for (id, params) in [
        ("filter.pixelate.crystallize", json!({"cellSize": 5, "seed": 37})),
        ("filter.blur.radialBlur", json!({"amount": 10, "centerX": 0.25, "centerY": 0.5})),
        ("filter.other.minimum", json!({"radius": 4, "preserve": "roundness"})),
        ("filter.other.maximum", json!({"radius": 4, "preserve": "roundness"})),
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
        // `preserve` reaches the filter through a comparison with the exact string "roundness",
        // so these play as squareness -- replay's own value -- and lose nothing by export (#7759
        // review).
        ("filter.other.minimum", json!({"radius": 4, "preserve": "Squareness"})),
        ("filter.other.maximum", json!({"radius": 4, "preserve": "Roundness"})),
    ] {
        let mut s = Session::new();
        s.actions.list.push(Action::new("A", vec![(id.into(), params.clone())]));
        let r = s.execute("actions.save", json!({"action": "A"})).unwrap_or_else(|e| panic!("{id} {params}: {e}"));
        assert_eq!(r["skipped"], json!([]), "{id} {params}");
    }
}

#[test]
fn every_mapped_command_exports_and_imports_back() {
    let cases: Vec<(&str, Value)> = vec![
        ("filter.blur.gaussianBlur", json!({"radius": 3.5})),
        ("filter.sharpen.unsharpMask", json!({"amount": 120, "radius": 1.5, "threshold": 3})),
        ("filter.noise.addNoise", json!({"amount": 12.5, "distribution": "gaussian", "monochromatic": true, "seed": 7})),
        ("filter.noise.median", json!({"radius": 2})),
        ("filter.stylize.emboss", json!({"angle": 135, "height": 3, "amount": 100})),
        ("filter.blur.motionBlur", json!({"angle": 45, "distance": 20})),
        ("filter.blur.smartBlur", json!({"radius": 2.5, "threshold": 40, "quality": "medium", "mode": "overlayEdge"})),
        ("filter.other.offset", json!({"horizontal": 10, "vertical": -5, "undefinedAreas": "wrap"})),
        ("filter.blur.blur", json!({})),
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
        ("image.imageSize", json!({"width": 800, "height": 600, "resolution": 72, "resample": "bicubicSharper"})),
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
        ("layer.new.group", json!({"name": "G"})),
        ("layer.groupLayers", json!({})),
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
        ("layer.select", json!({"layer": "Shadow", "mode": "add"})),
        ("layer.select", json!({"layer": "Shadow", "mode": "range"})),
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
        ("tools.defaultColors", json!({})),
        ("edit.convertToProfile", json!({"profile": "display-p3", "intent": "perceptual", "bpc": false})),
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
            json!({"path": "C:\\shots\\a.jpg", "format": "jpeg", "interlaced": false, "transparency": false, "embedIcc": true, "progressive": true, "optimized": true, "quality": 80, "matte": "none"}),
        ),
        ("image.adjustments.gradientMap", json!({"stops": [[0, "#000000"], [0.5, "#ff8000"], [1, "#ffffff"]], "reverse": true, "dither": false})),
        ("layer.newAdjustmentLayer.gradientMap", json!({"stops": [[0, "#102030"], [1, "#f0e0d0"]], "reverse": false, "dither": true})),
        ("layer.smartObjects.convertToSmartObject", json!({})),
        ("layer.smartObjects.editContents", json!({})),
        ("layer.smartObjects.updateModifiedContent", json!({})),
        ("paint.bucket", json!({"x": 10, "y": 20, "tolerance": 32, "antiAlias": true, "contiguous": false, "opacity": 100})),
        ("image.imageSize", json!({"percent": 200, "resample": "bicubicAutomatic"})),
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
        ("layer.layerStyle.pasteLayerStyle", json!({})),
        ("layer.setExpanded", json!({"all": true, "expanded": false})),
        ("layerComp.apply", json!({"comp": "Original"})),
        ("actions.play", json!({"action": "Other", "set": "S"})),
        (STOP_MESSAGE, json!({"message": "Hi", "allowContinue": false})),
        ("document.activate", json!({"offset": -1})),
        // The TrueGrit Texture Supply sets (Debaser, Distress Press, Halftone Zine Machine).
        ("filter.distort.ripple", json!({"amount": 50, "size": "medium"})),
        ("filter.pixelate.crystallize", json!({"cellSize": 5})),
        ("filter.gallery.spatter", json!({"sprayRadius": 7, "smoothness": 15})),
        (
            "select.refineEdge",
            json!({"radius": 0, "smartRadius": false, "smooth": 2, "feather": 0.5, "contrast": 15, "shiftEdge": -100, "decontaminate": false, "output": "layerMask"}),
        ),
        ("channel.target", json!({"channel": "mask"})),
        ("channel.target", json!({"channel": "composite"})),
        ("edit.undo", json!({})),
        ("edit.redo", json!({})),
        ("layer.select", json!({"layer": "YELLOW", "mode": "remove"})),
        ("layer.translate", json!({"dx": 0.36, "dy": -1.08, "units": "pt"})),
        ("select.loadSelection", json!({"channel": "targeted"})),
        ("layer.newFillLayer.pattern", json!({"pattern": "Debaser-Distressed-Dark", "scale": 50, "angle": 0, "link": true})),
        ("layer.fill.pattern.set", json!({"pattern": "1 -R2.jpg"})),
        ("image.mode.bitmap", json!({"method": "threshold"})),
        ("image.imageSize", json!({"resolution": 1200, "resample": "preserveDetails", "keepPrintSize": true})),
        ("layer.new.layerFromBackground", json!({"ifPresent": true, "select": false})),
        ("layerComp.apply", json!({"comp": "Comp 1"})),
    ];
    for (id, params) in &cases {
        let item = export_item(id, params, "S").unwrap_or_else(|| panic!("{id} does not export"));
        if *id == "file.close" {
            // Close exports "don't save" and imports as a plain close; the Save answer is tested below.
            assert_eq!(item.event.id(), "Cls ");
            continue;
        }
        let e = event_for(&item.event).unwrap_or_else(|| panic!("{id} exported as an unknown event {}", item.event.id()));
        match (e.from)(item.descriptor.as_ref().unwrap()) {
            Mapped::Steps(steps) => {
                assert_eq!(steps.len(), 1, "{id}: {steps:?}");
                assert_eq!(steps[0].0, *id, "{id} came back as another command");
                let mut back = steps[0].1.clone();
                normalise(&mut back);
                if *id == "actions.play" || *id == "actions.conditional" {
                    back["set"] = json!("S");
                }
                assert_eq!(back, *params, "{id}");
            }
            Mapped::Unsupported { .. } => panic!("{id} does not import back"),
        }
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
        "layer.selectAbove",
        "layer.selectBottom",
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
    };
    let imp = import(&atn::write(&file).unwrap()).unwrap();
    let ids: Vec<&str> = imp.actions[0].steps.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, [PHOTOSHOP_STEP, PHOTOSHOP_STEP, PHOTOSHOP_STEP, PHOTOSHOP_STEP, "filter.blur.gaussianBlur", PHOTOSHOP_STEP]);
    assert_eq!(imp.unsupported.len(), 5);
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
        item("make", "Make", Some(mask)),
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
        item("fill", "Fill", Some(bucket_px)),
        item("fill", "Fill", Some(bucket_pct)),
        item("placeEvent", "Place", Some(place)),
        item("Opn ", "Open", Some(desc("null").with("null", path_value("/Users/me/in.psd")))),
    ];
    let file = AtnFile {
        name: UnicodeString::new_nul("Rest"),
        expanded: false,
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        warnings: vec![],
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
    };
    let more = import(&atn::write(&file).unwrap()).unwrap();
    let more_steps = &more.actions[0].steps;
    assert_eq!(
        more_steps.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
        ["edit.transform", "select.transformSelection", "layer.arrange.sendToBack", "view.fitOnScreen", "tools.setColors"]
    );
    assert_eq!(more_steps[0].1, json!({"dx": 4, "dy": 6, "scaleX": 50, "scaleY": 50}));
    assert_eq!(more_steps[1].1, json!({"dx": 3, "dy": -2}));
    assert_eq!(more_steps[4].1, json!({"foreground": "#000000"}));
    assert!(more.unsupported.is_empty());
    assert_eq!(rest_steps[0].1, json!({"if": "hasLayers", "then": "FS8-Layers", "else": "FS8-No-Layers"}));
    assert_eq!(rest_steps[1].1, json!({"percent": 200, "resample": "bicubicAutomatic"}));
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
    assert_eq!(s.documents()[0].doc.layers.len(), 1, "the source is only copied from");

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
    assert_eq!(r["failed"]["step"], 0, "{r}");
    assert_eq!(r["failed"]["error"], "no document 1 before the active one, which is 1 of 1", "{r}");
    assert_eq!(one.active().unwrap().doc.layers.len(), 1);
}

fn set_of(name: &str, actions: Vec<(&str, Vec<AtnItem>)>) -> String {
    let file = AtnFile {
        name: UnicodeString::new_nul(name),
        expanded: false,
        actions: actions.into_iter().map(|(n, items)| AtnAction { name: UnicodeString::new_nul(n), items, ..Default::default() }).collect(),
        warnings: vec![],
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
/// does. #7516-#7519: a reference point other than the centre, a skew or a warp can't be written,
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
            json!({"scaleX": 140, "reference": "topLeft"}),
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

/// #7502, #7512, #7513, #7514: a New Layer step that clips to the layer below, has a colour
/// label, fills with the neutral colour or goes below the active layer isn't exported, since the
/// descriptor carries only its name, opacity and mode. Those options at their defaults still
/// export, and the step round-trips.
#[test]
fn new_layer_options_the_descriptor_cannot_carry_refuse_strict_export() {
    let new_layer = |p: Value| Action { name: "Probe".into(), steps: vec![("layer.new.layer".into(), p)], ..Default::default() };
    for p in [
        json!({"name": "New", "clipToPrevious": true}),
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
    };
    let steps = import(&atn::write(&file).unwrap()).unwrap().actions.remove(0).steps;
    assert_eq!(steps[0], ("layer.setAdjustment".to_string(), json!({})));
    assert_eq!(steps[1].1, json!({"midtones": [10, 0, 0], "kind": "colorBalance"}));
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
        json!({"resolution": 144.0, "resample": "nearest", "keepPrintSize": true}),
        json!({"width": 12.0, "height": 10.0, "resolution": 144.0, "resample": "nearest"}),
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
fn select_rect_mode_strict_export_rejects_unrepresentable_modes() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();

    // Default or "replace" mode succeeds
    s.actions.list.push(Action::new("ReplaceAct", vec![("select.rect".into(), json!({"x": 4, "y": 4, "width": 2, "height": 2, "mode": "replace"}))]));
    let saved = s.execute("actions.save", json!({"action": "ReplaceAct"})).unwrap();
    assert_eq!(saved["steps"], 1);

    // "add" mode cannot be silently converted to replace (set) in strict export
    s.actions.list.push(Action::new("AddAct", vec![("select.rect".into(), json!({"x": 4, "y": 4, "width": 2, "height": 2, "mode": "add"}))]));
    let before_actions = s.actions.list.clone();
    let err = s.execute("actions.save", json!({"action": "AddAct"}));
    assert!(err.is_err(), "strict export must reject select.rect with add mode rather than converting to replace");
    assert_eq!(s.actions.list, before_actions, "actions list must remain unchanged after rejected save");

    // "subtract" mode cannot be silently converted to replace either
    s.actions.list.push(Action::new("SubtractAct", vec![("select.rect".into(), json!({"x": 4, "y": 4, "width": 2, "height": 2, "mode": "subtract"}))]));
    let before_actions_sub = s.actions.list.clone();
    let err_subtract = s.execute("actions.save", json!({"action": "SubtractAct"}));
    assert!(err_subtract.is_err(), "strict export must reject select.rect with subtract mode rather than converting to replace");
    assert_eq!(s.actions.list, before_actions_sub, "actions list must remain unchanged after rejected save");

    // With skipUnsupported: true, unrepresentable modes are reported in skipped
    let skipped = s.execute("actions.save", json!({"action": "AddAct", "skipUnsupported": true})).unwrap();
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
        assert_eq!(back(p.clone()), Some(json!({"resolution": 144, "resample": "none"})), "{p}");
    }
    // A step with nothing left to record is refused rather than exported as an empty (unsupported) event.
    assert_eq!(back(json!({"width": 12, "height": 10, "resample": "none"})), None);
    // Real resizes are untouched.
    assert_eq!(back(json!({"width": 12, "height": 10, "resample": "bicubic"})), Some(json!({"width": 12, "height": 10, "resample": "bicubic"})));
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

fn one_action(items: Vec<AtnItem>) -> AtnFile {
    AtnFile {
        name: UnicodeString::new_nul("Saves"),
        expanded: false,
        actions: vec![AtnAction { name: UnicodeString::new_nul("A"), items, ..Default::default() }],
        warnings: vec![],
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
        item("save", "Save", Some(save_as("PNGF", "/out/v1.png").with("DIDr", D::Boolean(true)))),
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
    assert_eq!(ids, [PHOTOSHOP_STEP; 8]);
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
    };
    let imp = import(&atn::write(&file).unwrap()).unwrap();
    assert_eq!(imp.set.chars().count(), MAX_NAME_CHARS);
    assert_eq!(imp.actions[0].name.chars().count(), MAX_NAME_CHARS);
    assert_eq!(imp.actions[0].set.as_deref().map(|s| s.chars().count()), Some(MAX_NAME_CHARS));
    assert_eq!(imp.unsupported.len(), 50);
    assert!(imp.unsupported.iter().all(|u| u.action.chars().count() == MAX_NAME_CHARS));
}
