use super::*;

#[test]
fn a_unit_after_a_range_bound_does_not_reset_the_range_to_its_fallback() {
    // #7598: `0.5..5000 px?` fell back to a maximum of 100, so the Mixer Brush's Size slider
    // (and every range documented with a px or % suffix) topped out at 100.
    let range = |spec: &str| match parse_spec(spec).into_iter().next().map(|p| p.kind) {
        Some(Kind::Range { min, max, default }) => (min, max, default),
        other => panic!("{spec}: {other:?}"),
    };
    assert_eq!(range(r#"{"size":0.5..5000 px?}"#), (0.5, 5000.0, 0.5));
    assert_eq!(range(r#"{"scale":1..1000 %=100}"#), (1.0, 1000.0, 100.0));
    assert_eq!(range(r#"{"tolerance":0.5..10 px=2}"#), (0.5, 10.0, 2.0));
    assert_eq!(range(r#"{"angle":-180..180}"#), (-180.0, 180.0, -180.0));
    assert_eq!(range(r#"{"radius":0.1..1000=1}"#), (0.1, 1000.0, 1.0));
    assert_eq!(range(r#"{"gain":1e-3..2e2}"#), (0.001, 200.0, 0.001));
    // A bound that is no number at all still takes the old fallback rather than failing.
    assert_eq!(range(r#"{"x":0..many}"#), (0.0, 100.0, 0.0));
    // The documented specs of the real commands read their full ranges.
    let spec = photocraft_engine::commands::find("paint.pencil").expect("paint.pencil is registered");
    let size = parse_spec(spec.params).into_iter().find(|p| p.key == "size");
    assert!(matches!(size.map(|p| p.kind), Some(Kind::Range { max, .. }) if max == 5000.0));
}

#[test]
fn color_settings_working_spaces_list_installed_profiles_of_their_space() {
    // #6323: a press condition installed in the system can be the CMYK working space, as in
    // Photoshop's Color Settings; each working-space menu lists only profiles of its space.
    use photocraft_cms::{ColorSpace, ProfileClass};
    let p = |path: &str, class, color_space| InstalledProfile { path: path.into(), description: path.to_uppercase(), class, color_space };
    let installed = [
        p("gracol.icc", ProfileClass::Output, ColorSpace::Cmyk),
        p("monitor.icc", ProfileClass::Display, ColorSpace::Rgb),
        p("printer-rgb.icc", ProfileClass::Output, ColorSpace::Rgb),
        p("scanner.icc", ProfileClass::Input, ColorSpace::Rgb),
        p("gray.icc", ProfileClass::Output, ColorSpace::Gray),
        p("link.icc", ProfileClass::DeviceLink, ColorSpace::Cmyk),
    ];
    let paths = |key: &str| installed_choices("edit.colorSettings", key, &installed).into_iter().map(|(path, _)| path).collect::<Vec<_>>();
    assert_eq!(paths("workingCmyk"), ["gracol.icc"]);
    assert_eq!(paths("workingRgb"), ["monitor.icc"]);
    assert_eq!(paths("workingGray"), ["gray.icc"]);
    assert!(paths("policyCmyk").is_empty() && paths("intent").is_empty());
    assert_eq!(installed_choices("edit.colorSettings", "workingCmyk", &installed)[0].1, "GRACOL.ICC", "listed by description");
    // The profile dialogs keep listing every destination profile.
    assert_eq!(installed_choices("edit.convertToProfile", "profile", &installed).len(), 5);
    assert!(installed_choices("filter.blur.gaussianBlur", "profile", &installed).is_empty());
}

/// #5710: the seed field's Random button always gives a different seed the field can hold.
#[test]
fn random_seeds_stay_in_range_and_change() {
    let mut seen = std::collections::HashSet::new();
    for i in 0..200u64 {
        let s = random_seed(i, 7);
        assert!((0..=30_000).contains(&s), "{s}");
        assert_ne!(s, 7, "a click always changes the seed");
        seen.insert(s);
    }
    assert!(seen.len() > 100, "seeds spread out: {}", seen.len());
    assert_ne!(random_seed(1, 30_000), 30_000);
    // Every seed dialog shows the button: Clouds, Add Noise and the like carry a `seed` param.
    let specs = photocraft_engine::command_specs();
    assert!(
        specs
            .iter()
            .filter(|c| c.id.starts_with("filter."))
            .any(|c| parse_spec(c.params).iter().any(|p| p.key == "seed" && matches!(p.kind, Kind::Int { .. })))
    );
}

/// Preset names such as "iPhone 14 Pro Max" (the artboard dialog's choices) are text, not
/// camelCase keys: they used to read "I Phone 14  Pro  Max".
#[test]
fn display_names_are_not_split_like_camel_case_keys() {
    assert_eq!(source_label("iPhone 14 Pro Max"), "iPhone 14 Pro Max");
    assert_eq!(source_label("Web 1920"), "Web 1920");
    assert_eq!(source_label("centerX"), "Center X");
}

/// Convert to Profile's and Proof Setup's `bpc` read "Bpc": spelled out.
#[test]
fn black_point_compensation_is_spelled_out() {
    assert_eq!(source_label("bpc"), "Black Point Compensation");
}

fn key() -> FilterPreviewKey {
    FilterPreviewKey {
        doc: photocraft_doc::DocId(7),
        revision: 11,
        dialog: 3,
        active: Some(photocraft_doc::LayerId(2)),
        target: Default::default(),
        command: "filter.blur.gaussianBlur".into(),
        params: json!({"radius": 4.0}),
        k: 1,
    }
}

#[test]
fn same_target_ignores_parameters_but_nothing_else() {
    // A drag only changes the parameters, and that must not hide the finished frame.
    let base = key();
    let mut moved = base.clone();
    moved.params = json!({"radius": 40.0});
    assert!(base.same_target(&moved));
    assert!(moved.same_target(&base));

    // Anything else names a different preview and must not be shown.
    for break_it in [
        |k: &mut FilterPreviewKey| k.doc = photocraft_doc::DocId(8),
        |k: &mut FilterPreviewKey| k.revision = 12,
        |k: &mut FilterPreviewKey| k.dialog = 4,
        |k: &mut FilterPreviewKey| k.active = Some(photocraft_doc::LayerId(5)),
        |k: &mut FilterPreviewKey| k.active = None,
        |k: &mut FilterPreviewKey| k.target = photocraft_engine::channel_cmds::ChannelTarget::LayerMask,
        |k: &mut FilterPreviewKey| k.command = "filter.blur.motionBlur".into(),
        |k: &mut FilterPreviewKey| k.k = 2,
    ] {
        let mut other = base.clone();
        break_it(&mut other);
        assert!(!other.same_target(&base), "{other:?} must not pass as {base:?}");
        assert!(!base.same_target(&other));
    }
}

#[test]
fn only_direction_angles_get_the_dial() {
    // Every listed command still has a degree `angle` range, so the list cannot go stale.
    for cmd in ANGLE_DIAL {
        let spec = photocraft_engine::commands::find(cmd).unwrap_or_else(|| panic!("{cmd} is not a command")).params;
        assert!(
            parse_spec(spec).iter().any(|p| p.key == "angle" && matches!(p.kind, Kind::Range { min, max, .. } if min >= -360.0 && max <= 360.0)),
            "{cmd} has no angle range"
        );
        assert!(uses_angle_dial(cmd, "angle"));
        assert!(!uses_angle_dial(cmd, "distance"));
    }
    // Rotation amounts and angles with another zero keep their slider.
    for cmd in
        ["filter.distort.twirl", "image.rotation.arbitrary", "image.crop", "filter.blurGallery.tiltShift", "filter.lensCorrection", "filter.render.flame"]
    {
        assert!(!uses_angle_dial(cmd, "angle"), "{cmd}");
    }
}

#[test]
fn restores_only_valid_previous_filter_choices() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.prefs.edit(|prefs| {
        prefs.dialogs.insert("filter.blur.gaussianBlur".into(), json!({"radius": 11.0, "bogus": 42}));
    });
    open(&mut app, "filter.blur.gaussianBlur").unwrap();
    let fields = &app.ui.dialogs.last().unwrap().fields;
    assert_eq!(fields["radius"], json!(11.0));
    assert!(!fields.contains_key("bogus"));

    // A changed registry range, corrupt preference or stale path is never restored.
    let mut fields = Map::new();
    fields.insert("radius".into(), json!(2.0));
    restore_remembered(&app, "filter.blur.gaussianBlur", r#"{"radius":0.1..5=1}"#, &mut fields);
    assert_eq!(fields["radius"], json!(2.0));
}

#[test]
fn alt_reset_restores_schema_defaults_not_remembered() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.prefs.edit(|prefs| {
        prefs.dialogs.insert("filter.blur.gaussianBlur".into(), json!({"radius": 11.0}));
    });
    let id = open(&mut app, "filter.blur.gaussianBlur").unwrap();
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["radius"], json!(11.0));
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    reset_fields(&app, &mut fields);
    app.ui.dialog_mut(id).unwrap().fields = fields;
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    // Registry default for gaussian blur radius is 1.
    assert_eq!(f["radius"], json!(1.0));
    assert!(f.contains_key("__filter"));
    assert!(app.ui.dialogs.iter().any(|d| d.id == id));
}

/// #5806: OK in Displace with no map asks for the map file, as Photoshop does, and displaces
/// with the chosen file; Cancel in the picker applies nothing; a map already given runs at once.
#[test]
fn displace_asks_for_its_map_after_ok() {
    let dir = std::env::temp_dir().join(format!("photocraft-displace-map-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let map = dir.join("map.png");
    let mut m = photocraft_engine::Session::new();
    m.execute("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    std::fs::write(&map, photocraft_io::export(&m.active().unwrap().doc, "map.png", &Default::default()).unwrap().bytes).unwrap();
    let answers = vec![Some(crate::file_dialog::FileDialogAnswer::Paths(vec![map.to_string_lossy().into_owned()])), None];
    let (show, asked) = crate::file_dialog::fake(answers);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(show), ..Default::default() });
    app.run("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 16})).unwrap();
    app.run("edit.fill", json!({"color": "#000000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    let ctx = egui::Context::default();
    let id = open(&mut app, "filter.distort.displace").unwrap();
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.file_dialog_open(), "OK leaves the map picker pending");
    // Another document activated while the picker is open is left alone.
    app.run("file.new", json!({"width": 16, "height": 16, "background": "white"})).unwrap();
    let other_steps = app.session.documents()[1].history.past_len();
    // The frame shows the picker (the service is asked then) and acts on its answer.
    app.poll_file_dialog(&ctx, None);
    assert_eq!(asked.borrow().len(), 1, "OK asks for the map");
    assert_eq!(app.session.active_index(), Some(0), "the document that asked is shown");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "Displace ran with the chosen map");
    assert_eq!(app.session.documents()[1].history.past_len(), other_steps, "not on the other document");
    // Cancelled: nothing is applied.
    let id = open(&mut app, "filter.distort.displace").unwrap();
    crate::dialogs::confirm(&mut app, id).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(asked.borrow().len(), 2);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    // A map given in the dialog runs without asking.
    assert!(!needs_displace_map("filter.distort.displace", &json!({"mapPath": "/maps/x.psd"})));
    assert!(!needs_displace_map("filter.distort.displace", &json!({"mapDocument": 0})));
    assert!(needs_displace_map("filter.distort.displace", &json!({"mapDocument": -1, "mapPath": " "})));
    assert!(!needs_displace_map("filter.blur.gaussianBlur", &json!({})));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn remembering_dialogs_never_persists_paths_or_document_indices() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let mut fields = Map::new();
    fields.insert("__filter".into(), json!(true));
    fields.insert("radius".into(), json!(7.0));
    fields.insert("mapPath".into(), json!("/private/file"));
    fields.insert("document".into(), json!(5));
    remember(&mut app, "filter.blur.gaussianBlur", &fields);
    let saved = &app.session.prefs().dialogs["filter.blur.gaussianBlur"];
    assert_eq!(saved["radius"], json!(7.0));
    assert!(saved.get("mapPath").is_none());
    assert!(saved.get("document").is_none());
}

#[test]
fn parses_registry_notation() {
    let p = parse_spec(r#"{"radius":0.1..1000=1,"method":"spin|zoom","monochromatic":bool,"seed":u32=0,"horizontal":px=0}"#);
    assert_eq!(p[0], Param { key: "radius".into(), kind: Kind::Range { min: 0.1, max: 1000.0, default: 1.0 } });
    assert_eq!(p[1].kind, Kind::Choice { options: vec!["spin".into(), "zoom".into()], default: None });
    assert_eq!(p[2].kind, Kind::Bool(false));
    assert_eq!(p[3].kind, Kind::Int { default: 0 });
    assert_eq!(p[4].kind, Kind::Int { default: 0 });
    assert!(parse_spec("{}").is_empty());
    let p_choice = parse_spec(r#"{"quality":"draft|good|best"="good"}"#);
    assert_eq!(p_choice[0].kind, Kind::Choice { options: vec!["draft".into(), "good".into(), "best".into()], default: Some("good".into()) });
    let p = parse_spec(r#"{"lighting":bool=true,"kernel":int[25],"pins":json,"points":[[0,0],[1,0]],"mapPath":text,"mapDocument":doc,"scale":1..9999=1}"#);
    let kinds: Vec<&Kind> = p.iter().map(|p| &p.kind).collect();
    assert_eq!(
        kinds,
        [&Kind::Bool(true), &Kind::Grid(25), &Kind::Json, &Kind::Json, &Kind::Text, &Kind::Document, &Kind::Range { min: 1.0, max: 9999.0, default: 1.0 }]
    );
}

/// A layer-id param has no number field: drawing the Auto-Align dialog used to write
/// `reference: 0` back, which the engine rejects as not a selected layer (#674).
#[test]
fn layer_id_params_stay_out_of_the_dialog() {
    let mut f = Map::new();
    f.insert("__command".into(), json!("edit.autoAlignLayers"));
    egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut f)).textures_delta.clear();
    assert!(!params_of(&f).as_object().unwrap().contains_key("reference"), "{f:?}");
}

/// Layer › Layer Style › Global Light (#3054): Global Light dialog opens with the active
/// document's current shared lighting, preserving it on unchanged OK, ignoring stale
/// remembered dialog values, tracking document switching, applying edits, and leaving the
/// document unchanged on Cancel.
#[test]
fn global_light_dialog_reflects_active_document_light() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let doc1_id = app.session.active().unwrap().doc.id;

    // 0. Default-document control: fresh document starts at angle 120 and altitude 30.
    let d_default = crate::menus::invoke(&mut app, &ctx, "layer.layerStyle.globalLight", Value::Null).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(app.ui.dialog_mut(d_default).unwrap().fields["angle"], json!(120.0));
    assert_eq!(app.ui.dialog_mut(d_default).unwrap().fields["altitude"], json!(30.0));
    crate::dialogs::confirm(&mut app, d_default).unwrap();
    assert_eq!(app.session.active().unwrap().doc.global_light.angle, 120.0);
    assert_eq!(app.session.active().unwrap().doc.global_light.altitude, 30.0);

    // 1. Change document's global light to 30 degrees angle and 45 degrees altitude.
    app.run("layer.layerStyle.globalLight", json!({"angle": 30.0, "altitude": 45.0})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.global_light.angle, 30.0);
    assert_eq!(app.session.active().unwrap().doc.global_light.altitude, 45.0);

    // 2. Open Global Light dialog: opens with the active document's light (30 / 45).
    let d_id = crate::menus::invoke(&mut app, &ctx, "layer.layerStyle.globalLight", Value::Null).unwrap()["dialog"].as_u64().unwrap();
    let fields = &app.ui.dialog_mut(d_id).unwrap().fields;
    assert_eq!(fields["angle"], json!(30.0));
    assert_eq!(fields["altitude"], json!(45.0));

    // 3. Unchanged OK preserves the document's light.
    crate::dialogs::confirm(&mut app, d_id).unwrap();
    let gl = app.session.active().unwrap().doc.global_light;
    assert_eq!(gl.angle, 30.0);
    assert_eq!(gl.altitude, 45.0);

    // 4. Stale remembered values do not override the active document.
    app.session.prefs.edit(|prefs| {
        prefs.dialogs.insert(
            "layer.layerStyle.globalLight".into(),
            json!({
                "angle": 75.0,
                "altitude": 60.0,
            }),
        );
    });
    let d_id = open(&mut app, "layer.layerStyle.globalLight").unwrap();
    let fields = &app.ui.dialog_mut(d_id).unwrap().fields;
    assert_eq!(fields["angle"], json!(30.0), "reads active document, not remembered");
    assert_eq!(fields["altitude"], json!(45.0));

    // 5. Cancel preserves the document's light.
    app.ui.close_dialog(d_id);
    let gl = app.session.active().unwrap().doc.global_light;
    assert_eq!(gl.angle, 30.0);
    assert_eq!(gl.altitude, 45.0);

    // 6. Multiple documents: each document initializes the dialog to its own light.
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let doc2_id = app.session.active().unwrap().doc.id;
    assert_ne!(doc1_id, doc2_id);
    app.run("layer.layerStyle.globalLight", json!({"angle": -45.0, "altitude": 15.0})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.global_light.angle, -45.0);
    assert_eq!(app.session.active().unwrap().doc.global_light.altitude, 15.0);

    let d_id2 = open(&mut app, "layer.layerStyle.globalLight").unwrap();
    let fields2 = &app.ui.dialog_mut(d_id2).unwrap().fields;
    assert_eq!(fields2["angle"], json!(-45.0));
    assert_eq!(fields2["altitude"], json!(15.0));
    app.ui.close_dialog(d_id2);

    // Switch back to doc1: dialog still reflects doc1's light (30 / 45).
    assert!(app.session.set_active(0));
    let d_id1 = open(&mut app, "layer.layerStyle.globalLight").unwrap();
    assert_eq!(app.ui.dialog_mut(d_id1).unwrap().fields["angle"], json!(30.0));
    assert_eq!(app.ui.dialog_mut(d_id1).unwrap().fields["altitude"], json!(45.0));

    // 7. Modifying the dialog updates the active document.
    app.ui.dialog_mut(d_id1).unwrap().fields.insert("angle".into(), json!(-60.0));
    app.ui.dialog_mut(d_id1).unwrap().fields.insert("altitude".into(), json!(20.0));
    crate::dialogs::confirm(&mut app, d_id1).unwrap();
    assert_eq!(app.session.active().unwrap().doc.global_light.angle, -60.0);
    assert_eq!(app.session.active().unwrap().doc.global_light.altitude, 20.0);

    // Switch to doc2: doc2 is untouched (-45 / 15).
    assert!(app.session.set_active(1));
    assert_eq!(app.session.active().unwrap().doc.global_light.angle, -45.0);
    assert_eq!(app.session.active().unwrap().doc.global_light.altitude, 15.0);
}

/// View › Proof Setup (#3055): Custom Proof Setup opens with the active document's current
/// proof condition, preserving it on unchanged OK, ignoring stale remembered dialog values,
/// tracking document switching, applying edits, and leaving the document unchanged on Cancel.
#[test]
fn proof_setup_dialog_reflects_active_document_proof_condition() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let doc1_id = app.session.active().unwrap().doc.id;

    // 0. A fresh document defaults to working-cmyk.
    let d_fresh = crate::menus::invoke(&mut app, &ctx, "view.proofSetup.custom", Value::Null).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(app.ui.dialog_mut(d_fresh).unwrap().fields["profile"], json!("working-cmyk"));
    crate::dialogs::confirm(&mut app, d_fresh).unwrap();
    assert_eq!(app.session.color.proof(doc1_id).setup.name, "working-cmyk");

    // 1. Preset sets sRGB and turns Proof Colors on.
    crate::menus::invoke(&mut app, &ctx, "view.proofSetup.internetStandardRgb", Value::Null).unwrap();
    assert_eq!(app.session.color.proof(doc1_id).setup.name, "srgb");
    assert!(app.session.color.proof(doc1_id).enabled);

    // 2. Open Custom Proof Setup: starts with the active document's condition (sRGB).
    let d_id = crate::menus::invoke(&mut app, &ctx, "view.proofSetup.custom", Value::Null).unwrap()["dialog"].as_u64().unwrap();
    let fields = &app.ui.dialog_mut(d_id).unwrap().fields;
    assert_eq!(fields["profile"], json!("srgb"));
    assert_eq!(fields["intent"], json!("relative"));
    assert_eq!(fields["bpc"], json!(true));
    assert_eq!(fields["simulatePaper"], json!(false));

    // 3. Unchanged OK preserves the document's condition and leaves Proof Colors enabled.
    crate::dialogs::confirm(&mut app, d_id).unwrap();
    let pv = app.session.color.proof(doc1_id);
    assert_eq!(pv.setup.name, "srgb");
    assert!(pv.enabled);

    // 4. Stale remembered values do not override the active document.
    app.session.prefs.edit(|prefs| {
        prefs.dialogs.insert(
            "view.proofSetup".into(),
            json!({
                "profile": "display-p3",
                "intent": "perceptual",
                "bpc": false,
                "simulatePaper": true,
            }),
        );
    });
    let d_id = open(&mut app, "view.proofSetup").unwrap();
    let fields = &app.ui.dialog_mut(d_id).unwrap().fields;
    assert_eq!(fields["profile"], json!("srgb"), "reads active document, not remembered");
    assert_eq!(fields["intent"], json!("relative"));
    assert_eq!(fields["bpc"], json!(true));
    assert_eq!(fields["simulatePaper"], json!(false));

    // 5. Cancel preserves the document.
    app.ui.close_dialog(d_id);
    assert_eq!(app.session.color.proof(doc1_id).setup.name, "srgb");

    // 6. Multiple documents: each document initializes the dialog to its own condition.
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let doc2_id = app.session.active().unwrap().doc.id;
    assert_ne!(doc1_id, doc2_id);
    app.run("view.proofSetup", json!({"profile": "display-p3", "intent": "perceptual", "bpc": false, "simulatePaper": true})).unwrap();
    assert_eq!(app.session.color.proof(doc2_id).setup.name, "display-p3");

    let d_id2 = open(&mut app, "view.proofSetup").unwrap();
    let fields2 = &app.ui.dialog_mut(d_id2).unwrap().fields;
    assert_eq!(fields2["profile"], json!("display-p3"));
    assert_eq!(fields2["intent"], json!("perceptual"));
    assert_eq!(fields2["bpc"], json!(false));
    assert_eq!(fields2["simulatePaper"], json!(true));
    app.ui.close_dialog(d_id2);

    // Switch back to doc1: dialog still reflects doc1's sRGB.
    assert!(app.session.set_active(0));
    let d_id1 = open(&mut app, "view.proofSetup").unwrap();
    assert_eq!(app.ui.dialog_mut(d_id1).unwrap().fields["profile"], json!("srgb"));

    // 7. Modifying the dialog updates the active document.
    app.ui.dialog_mut(d_id1).unwrap().fields.insert("profile".into(), json!("adobe-rgb-compat"));
    crate::dialogs::confirm(&mut app, d_id1).unwrap();
    assert_eq!(app.session.color.proof(doc1_id).setup.name, "adobe-rgb-compat");
    // doc2 is untouched
    assert_eq!(app.session.color.proof(doc2_id).setup.name, "display-p3");
}

#[test]
fn indexed_counts_are_integer_json_while_dither_amounts_keep_their_decimals() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    open(&mut app, "image.mode.indexedColor").unwrap();
    let mut defaults = app.ui.dialogs.first().unwrap().fields.clone();
    let before = params_of(&defaults)["colors"].clone();
    egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut defaults)).textures_delta.clear();
    assert_eq!(before.as_u64(), Some(256));
    assert_eq!(params_of(&defaults)["colors"], before, "opening a preview must not schedule a second job just to normalize its count");
    let mut fields = Map::new();
    fields.insert("__command".into(), json!("image.mode.indexedColor"));
    fields.insert("colors".into(), json!(17));
    fields.insert("amount".into(), json!(37.5));
    egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut fields)).textures_delta.clear();
    let params = params_of(&fields);
    assert_eq!(params["colors"].as_u64(), Some(17));
    assert_eq!(params["amount"].as_f64(), Some(37.5));
}

/// #7400: Lens Blur's Source lists the document's alpha channels, by name, each choosable even
/// when two share a name; `depthChannel` is set through that menu, not shown as a field, and
/// like the other references it isn't remembered, so a remembered `channel` source opens as None.
#[test]
fn lens_blur_source_lists_the_alpha_channels() {
    let spec = photocraft_engine::commands::find(LENS_BLUR).unwrap().params;
    let params = parse_spec(spec);
    assert!(params.iter().any(|p| p.key == "depthChannel" && p.kind == Kind::Json), "depthChannel is no dialog field");
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    app.run("channel.new", json!({"name": "Depth Map", "fill": "white"})).unwrap();
    let id = open(&mut app, LENS_BLUR).unwrap();
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["__channels"], json!(["Depth Map"]));
    // The menu's values are channel indices, so two channels with one name are both choosable.
    app.run("channel.new", json!({"name": "Depth Map", "fill": "black"})).unwrap();
    let id = open(&mut app, LENS_BLUR).unwrap();
    let names: Vec<String> = serde_json::from_value(app.ui.dialog_mut(id).unwrap().fields["__channels"].clone()).unwrap();
    assert_eq!(names, ["Depth Map", "Depth Map"]);
    assert_eq!((channel_choice(0), channel_choice(1)), ("channel:0".to_string(), "channel:1".to_string()));
    assert_eq!(channel_index(&json!(1), &names), Some(1), "the second one by its index");
    assert_eq!(channel_index(&json!("Depth Map"), &names), Some(0), "a name is the first so named, as the engine reads it");
    assert_eq!(channel_index(&json!(2), &names), None);

    app.session.prefs.edit(|p| {
        p.dialogs.insert(LENS_BLUR.into(), json!({"depthMap": "channel", "depthChannel": "Depth Map"}));
    });
    let id = open(&mut app, LENS_BLUR).unwrap();
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!(f["depthMap"], "none", "no channel was remembered with it");
    assert!(f.get("depthChannel").is_none());
}

#[test]
fn wide_positive_ranges_use_the_logarithmic_slider_path() {
    assert!(uses_logarithmic_slider(0.1, 1000.0), "Gaussian Blur radius");
    assert!(uses_logarithmic_slider(1.0, 9999.0));
    assert!(!uses_logarithmic_slider(1.0, 500.0));
    assert!(!uses_logarithmic_slider(0.0, 1000.0));
}

#[test]
fn new_filters_have_dialogs_or_run_directly() {
    for id in [
        "filter.stylize.oilPaint",
        "filter.blur.lensBlur",
        "filter.blurGallery.irisBlur",
        "filter.other.custom",
        "filter.distort.displace",
        "filter.pixelate.mezzotint",
        "filter.render.lightingEffects",
        "filter.render.relight",
        "filter.other.colorToAlpha",
    ] {
        assert!(has_dialog(id), "{id}");
    }
    for id in ["filter.pixelate.facet", "filter.pixelate.fragment", "filter.video.ntscColors"] {
        assert!(!has_dialog(id), "{id}");
    }
}

#[test]
fn every_filter_command_spec_parses() {
    for c in photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with("filter.")) {
        let _ = parse_spec(c.params);
    }
    assert!(has_dialog("filter.blur.gaussianBlur"));
    assert!(!has_dialog("filter.stylize.findEdges"));
}

/// Select and Mask's Output To lists all six output targets.
#[test]
fn select_and_mask_output_lists_all_six_targets() {
    let spec = photocraft_engine::commands::find("select.refineEdge").unwrap().params;
    let p = parse_spec(spec).into_iter().find(|p| p.key == "output").unwrap();
    let Kind::Choice { options: choices, .. } = p.kind else { panic!("output is not a choice") };
    let labels: Vec<String> = choices.iter().map(|c| source_label(c)).collect();
    assert_eq!(labels, ["Selection", "Layer Mask", "New Layer", "New Layer With Mask", "New Document", "New Document With Mask"]);
}

#[test]
fn preview_runs_engine_command_on_proxy() {
    let mut doc = Document::with_background(
        "p",
        photocraft_doc::Size::new(64, 64),
        photocraft_doc::ColorMode::Rgb,
        photocraft_doc::SampleType::U8,
        photocraft_doc::Color::WHITE,
    );
    let bg = doc.layers[0].id;
    doc.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 32, 64), &[0.0, 0.0, 0.0, 1.0]);
    let out = preview_document(&doc, Some(bg), "filter.blur.gaussianBlur", &json!({"radius": 4.0}), 1).unwrap();
    let p = out.layers[0].surface().unwrap().pixel(32, 32);
    assert!(p[0] > 0.2 && p[0] < 0.8, "edge blurred: {p:?}");
    assert_eq!(label("wavelengthMin"), "Wavelength Min");
}

/// #7423 (review on #7799): re-editing a Displace smart filter and picking another open
/// document as its map previews with that document, as OK applies it. The preview session held
/// only the edited document, and the map's `mapDocument` sits inside the filter's `params`.
#[test]
fn smart_displace_preview_uses_the_map_document() {
    let mut s = photocraft_engine::Session::new();
    // Documents 0 and 1: a white and a black map, opened before the edited one.
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    s.execute("file.new", json!({"width": 8, "height": 8, "background": "black"})).unwrap();
    s.execute("file.new", json!({"width": 48, "height": 32, "background": "transparent"})).unwrap();
    s.edit("square", |doc, active| {
        let fmt = doc.pixel_format();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surf.fill_rect(photocraft_geom::Rect::new(10, 10, 20, 12), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        Ok(())
    })
    .unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("filter.distort.displace", json!({"horizontal": 10, "vertical": 0, "mapDocument": 0})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    let layer = s.active().unwrap().active_layer;
    let documents: Vec<_> = s.documents().iter().map(|d| d.doc.clone()).collect();
    for map in [1, 0] {
        let p = json!({"index": 0, "params": {"mapDocument": map}});
        let mut applied = photocraft_engine::Session::new();
        for d in &documents {
            applied.add_document((**d).clone(), None);
        }
        applied.execute(SMART_FILTER_SET_PARAMS, p.clone()).unwrap();
        let want = photocraft_compose::render(&applied.active().unwrap().doc, doc.bounds()).px;
        let preview = preview_document_for_target(
            &doc,
            layer,
            SMART_FILTER_SET_PARAMS,
            &p,
            1,
            None,
            photocraft_engine::channel_cmds::ChannelTarget::Composite,
            &documents,
            &ColorSettings::default(),
        )
        .unwrap_or_else(|| panic!("map {map}: no preview"));
        assert!(photocraft_compose::render(&preview, doc.bounds()).px == want, "map {map}: the preview differs from what OK applies");
    }
}

/// Match Color's preview reads the Source document from the open documents, as OK does: the
/// preview session held only the edited document, so a Source showed no change (#4851).
#[test]
fn match_color_preview_uses_the_source_document() {
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    let solid = |name: &str, c: [f32; 4]| {
        let mut d = Document::with_background(name, Size::new(32, 32), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        d.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 32, 32), &c);
        d.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 16, 32), &[c[0] * 0.5, c[1] * 0.5, c[2] * 0.5, 1.0]);
        d
    };
    // The source document is open before the edited one, so indices differ between sessions.
    let mut s = photocraft_engine::Session::new();
    s.add_document(solid("reference", [0.1, 0.3, 0.9, 1.0]), None);
    s.add_document(solid("texture", [0.9, 0.5, 0.2, 1.0]), None);
    let doc = s.active().unwrap().doc.clone();
    let layer = doc.layers[0].id;
    let documents: Vec<_> = s.documents().iter().map(|d| d.doc.clone()).collect();
    for (source, label) in [(0, "another document"), (1, "the edited document itself")] {
        let p = json!({"source": source});
        let mut applied = photocraft_engine::Session::new();
        for d in &documents {
            applied.add_document((**d).clone(), None);
        }
        applied.execute("image.adjustments.matchColor", p.clone()).unwrap();
        let want = applied.active().unwrap().doc.layers[0].surface().unwrap().pixel(24, 16);
        let preview = preview_document_for_target(
            &doc,
            Some(layer),
            "image.adjustments.matchColor",
            &p,
            1,
            None,
            photocraft_engine::channel_cmds::ChannelTarget::Composite,
            &documents,
            &ColorSettings::default(),
        )
        .unwrap_or_else(|| panic!("{label}: no preview"));
        let got = preview.layers[0].surface().unwrap().pixel(24, 16);
        assert!((0..3).all(|c| (got[c] - want[c]).abs() < 1.0 / 255.0 + 1e-4), "{label}: preview {got:?}, OK gives {want:?}");
    }
    // The preview is of the matched image: the orange texture took the reference's blue.
    let preview = preview_document_for_target(
        &doc,
        Some(layer),
        "image.adjustments.matchColor",
        &json!({"source": 0}),
        2,
        None,
        photocraft_engine::channel_cmds::ChannelTarget::Composite,
        &documents,
        &ColorSettings::default(),
    )
    .unwrap();
    let px = preview.layers[0].surface().unwrap().pixel(12, 8);
    assert!(px[2] > px[0], "proxy preview matched to the reference: {px:?}");
}

#[test]
fn core_filter_previews_keep_minimum_settings_after_downsampling() {
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    use photocraft_geom::Rect;

    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc = Document::with_background("preview", Size::new(32, 32), ColorMode::Rgb, depth, Color::WHITE);
        let layer = doc.layers[0].id;
        doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 16, 32), &[0.0, 0.0, 0.0, 1.0]);
        for (suffix, params) in [
            ("blur.boxBlur", json!({"radius":1})),
            ("blur.motionBlur", json!({"distance":1})),
            ("blur.surfaceBlur", json!({"radius":1})),
            ("noise.median", json!({"radius":1})),
            ("noise.dustAndScratches", json!({"radius":1})),
            ("pixelate.mosaic", json!({"cellSize":2})),
            ("stylize.emboss", json!({"height":1})),
            ("other.minimum", json!({"radius":0.2})),
            ("other.maximum", json!({"radius":0.2})),
            ("distort.wave", json!({"wavelengthMin":1,"wavelengthMax":2,"amplitudeMin":1,"amplitudeMax":1})),
        ] {
            let command = format!("filter.{suffix}");
            for k in [2, 4] {
                let proxy = crate::proxy::proxy_document(&doc, k);
                let expected = preview_document(&proxy, Some(layer), &command, &params, 1).unwrap();
                let out = preview_document(&doc, Some(layer), &command, &params, k).unwrap_or_else(|| panic!("{command}: {depth:?}, proxy factor {k}"));
                assert_eq!(
                    out.layers[0].surface().unwrap().read_region(proxy.bounds()),
                    expected.layers[0].surface().unwrap().read_region(proxy.bounds()),
                    "{command}: {depth:?}, proxy factor {k}"
                );
            }
        }
    }
}

/// A serde type error from numbers the dialog sends as JSON floats (`1.0`) for an integer
/// field ("invalid type: floating point `1.0`, expected u32"), as opposed to a legitimate
/// refusal such as "no selection" or "needs a map".
fn float_type_error(e: &str) -> bool {
    e.contains("invalid type: floating point")
}

/// #2501: every schema dialog, opened with its default fields and drawn once, runs its command
/// (OK) and its live preview without a type error from integer fields sent as floats.
#[test]
fn every_dialog_default_runs_without_float_type_errors() {
    let ids: Vec<&str> = photocraft_engine::command_specs().iter().map(|c| c.id).filter(|id| has_dialog(id)).collect();
    assert!(ids.len() > 100, "{} dialogs", ids.len());
    let mut failures = Vec::new();
    for id in ids {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 24, "height": 24})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let Some(dialog) = open(&mut app, id) else { continue };
        let mut f = app.ui.dialogs.iter().find(|d| d.id == dialog).unwrap().fields.clone();
        egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut f)).textures_delta.clear();
        let params = params_of(&f);
        let st = app.session.active().unwrap();
        let (doc, active) = ((*st.doc).clone(), st.active_layer);
        if let Err(e) = app.session.execute(id, params.clone()) {
            let e = e.to_string();
            if float_type_error(&e) {
                failures.push(format!("{id} OK {params}: {e}"));
            }
        }
        // The preview swallows the error; rerun its params at full size to see it.
        if preview_document(&doc, active, id, &params, 1).is_none() {
            let mut s = photocraft_engine::Session::new();
            s.add_document(doc, None);
            if let Some(a) = active {
                s.select_layer(a).unwrap();
            }
            if let Err(e) = s.execute(id, params.clone()).map_err(|e| e.to_string())
                && float_type_error(&e)
            {
                failures.push(format!("{id} preview {params}: {e}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn colour_params_parse_with_and_without_a_default() {
    let p = parse_spec(r##"{"color":color=#ffffff,"vineColor":color,"bad":color=#zz}"##);
    assert_eq!(p[0].kind, Kind::Color(Some([1.0, 1.0, 1.0])));
    assert_eq!(p[1].kind, Kind::Color(None));
    assert_eq!(p[2].kind, Kind::Color(None));
}

/// Color to Alpha (#1576): the dialog starts at white and the GIMP-style thresholds, drawing it
/// keeps those params, and its live preview makes the white of a Background transparent.
#[test]
fn color_to_alpha_dialog_defaults_draw_and_preview() {
    const ID: &str = "filter.other.colorToAlpha";
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    let id = open(&mut app, ID).unwrap();
    let mut f = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    assert_eq!(f.get("__preview"), Some(&json!(true)));
    let want = json!({"color": "#ffffff", "transparencyThreshold": 0.0, "opacityThreshold": 1.0});
    assert_eq!(params_of(&f), want);
    egui::Context::default().run_ui(Default::default(), |ui| body(ui, &mut f)).textures_delta.clear();
    assert_eq!(params_of(&f), want, "drawing the dialog keeps its params");

    let mut doc = Document::with_background(
        "p",
        photocraft_doc::Size::new(32, 32),
        photocraft_doc::ColorMode::Rgb,
        photocraft_doc::SampleType::U16,
        photocraft_doc::Color::WHITE,
    );
    let bg = doc.layers[0].id;
    doc.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 8, 32), &[0.0, 0.0, 0.0, 1.0]);
    for k in [1, 2] {
        let out = preview_document(&doc, Some(bg), ID, &want, k).unwrap();
        let s = out.layers[0].surface().unwrap();
        assert_eq!(out.layers[0].name, "Layer 0", "k={k}");
        assert!(s.pixel(24 / k as i32, 4)[3] < 1e-3, "k={k}: white removed");
        assert_eq!(s.pixel(1, 4)[3], 1.0, "k={k}: black kept");
    }
}

#[test]
fn preview_stays_inside_the_selection_at_every_proxy_factor() {
    use photocraft_doc::SampleType;
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc =
            Document::with_background("p", photocraft_doc::Size::new(64, 64), photocraft_doc::ColorMode::Rgb, depth, photocraft_doc::Color::gray(0.5));
        let bg = doc.layers[0].id;
        let mut sel = photocraft_raster::Surface::new(photocraft_doc::PixelFormat::GRAY8);
        sel.fill_rect(photocraft_geom::Rect::new(0, 0, 32, 64), &[1.0]);
        doc.selection = Some(sel);
        for k in [1, 2, 4] {
            let out = preview_document(&doc, Some(bg), "filter.noise.addNoise", &json!({"amount": 100.0}), k).unwrap();
            let s = out.layers[0].surface().unwrap();
            let half = 32 / k as i32;
            let changed = |x0: i32, x1: i32| (0..64 / k as i32).any(|y| (x0..x1).any(|x| s.pixel(x, y) != doc.layers[0].surface().unwrap().pixel(0, 0)));
            assert!(changed(0, half), "{depth:?} k={k}: noise inside the selection");
            assert!(!changed(half, 64 / k as i32), "{depth:?} k={k}: nothing outside the selection");
        }
    }
}

#[test]
fn define_pattern_asks_for_a_name() {
    // Edit › Define Pattern… named the pattern after the document without asking ("ask", then
    // "ask 2"). Photoshop's Pattern Name dialog starts from that name and lets you change it.
    let spec = photocraft_engine::commands::find("edit.definePattern").unwrap();
    assert_eq!(parse_spec(spec.params), vec![Param { key: "name".into(), kind: Kind::Text }, Param { key: "rect".into(), kind: Kind::Json }]);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8, "name": "ask.jpg"})).unwrap();
    let ctx = egui::Context::default();
    let define = |app: &mut PhotocraftApp, name: Option<&str>| {
        crate::menus::invoke(app, &ctx, "edit.definePattern", Value::Null).unwrap();
        let d = app.ui.dialogs.last().expect("Define Pattern opens a dialog").clone();
        let shown = d.fields["name"].as_str().unwrap().to_string();
        if let Some(n) = name {
            app.ui.dialog_mut(d.id).unwrap().fields.insert("name".into(), json!(n));
        }
        crate::dialogs::confirm(app, d.id).unwrap();
        shown
    };
    assert_eq!(define(&mut app, Some("Bricks")), "ask", "prefilled with the document's name");
    assert_eq!(define(&mut app, None), "ask", "Bricks didn't take it");
    assert_eq!(define(&mut app, None), "ask 2", "numbered past the library");
    let names: Vec<&str> = app.session.patterns.items.iter().map(|p| p.name.as_str()).collect();
    for n in ["Bricks", "ask", "ask 2"] {
        assert!(names.contains(&n), "{n} in {names:?}");
    }
}

#[test]
fn notes_after_a_spec_are_not_parameters() {
    // A note after the object, like Selective Color's "(per-range [c,m,y,k] arrays: …)", was
    // parsed as parameters: its commas split off junk fields and it glued itself onto the last
    // one, so "reds":json became a number field the dialog showed and sent.
    let p = parse_spec(r#"{"a":0..10=1,"b":json} (notes [x,y,z] here, and more) → {"c":id}"#);
    assert_eq!(p, vec![Param { key: "a".into(), kind: Kind::Range { min: 0.0, max: 10.0, default: 1.0 } }, Param { key: "b".into(), kind: Kind::Json }]);
    // Every command with a schema dialog has plain parameter keys.
    for c in photocraft_engine::command_specs().iter().filter(|c| has_dialog(c.id)) {
        for p in parse_spec(c.params) {
            assert!(p.key.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_'), "{}: junk parameter {:?}", c.id, p.key);
        }
    }
}

#[test]
fn selective_color_dialog_ok_applies() {
    // With the junk `reds: 0` the dialog's OK failed (`reds` must be an array of 4 numbers).
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let id = open(&mut app, "image.adjustments.selectiveColor").unwrap();
    let fields = app.ui.dialogs.last().unwrap().fields.clone();
    assert!(fields.keys().all(|k| k.starts_with("__") || k.chars().all(|ch| ch.is_ascii_alphanumeric())), "{fields:?}");
    assert!(!fields.contains_key("reds"), "the per-range arrays are not dialog fields");
    app.ui.dialog_mut(id).unwrap().fields.insert("black".into(), json!(40.0));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.journal.last().map(|j| j.0.as_str()), Some("image.adjustments.selectiveColor"));
}

/// A filtered test image with structure at every scale (low-frequency waves plus a ramp).
fn waves(n: u32) -> Document {
    let mut doc = Document::with_background(
        "w",
        photocraft_doc::Size::new(n, n),
        photocraft_doc::ColorMode::Rgb,
        photocraft_doc::SampleType::U8,
        photocraft_doc::Color::WHITE,
    );
    let s = doc.layers[0].surface_mut().unwrap();
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            let v = 0.5 + 0.4 * (x as f32 / 23.0).sin() * (y as f32 / 31.0).cos();
            s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &[v, 1.0 - v, x as f32 / n as f32, 1.0]);
        }
    }
    doc
}

/// Mean absolute RGB difference of two documents, compared at 1/4 scale.
fn diff_at_quarter(a: &Document, b: &Document) -> f32 {
    let quarter = |d: &Document| photocraft_compose::flatten(&crate::proxy::proxy_document(d, (d.size.width / 64).max(1)));
    let (a, b) = (quarter(a), quarter(b));
    assert_eq!(a.px.len(), b.px.len());
    a.px.iter().zip(&b.px).map(|(p, q)| (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>() / 3.0).sum::<f32>() / a.px.len() as f32
}

/// #2063: these filters drew their features in proxy pixels (noise, dots, dither, strokes,
/// kernels, scan lines, contour lines, streaks, depths, a tree's segments), so a 1/4 preview
/// showed them 4x larger than OK applied them. Their preview now runs at full resolution and
/// matches the applied result. Dust & Scratches (#3265) is the mirror image of those: a proxy
/// averages away the very specks the filter exists to remove, so its preview showed an
/// untouched image at a blocky scale.
#[test]
fn fixed_size_filters_preview_at_the_scale_they_apply() {
    let doc = sharp(256);
    let gray = grayscale(&doc);
    let edge = json!([0, 0, 0, 0, 0, 0, 0, -1, 0, 0, 0, -1, 4, -1, 0, 0, 0, -1, 0, 0, 0, 0, 0, 0, 0]);
    for (cmd, p) in [
        ("filter.distort.ripple", json!({"amount": 300.0, "size": "large"})),
        ("filter.stylize.diffuse", json!({"mode": "normal"})),
        ("filter.stylize.oilPaint", json!({})),
        ("filter.render.flame", json!({})),
        ("filter.render.fibers", json!({})),
        ("filter.pixelate.mezzotint", json!({"type": "mediumDots"})),
        ("filter.noise.addNoise", json!({"amount": 12.5})),
        ("filter.noise.dustAndScratches", json!({"radius": 4.0, "threshold": 0.0})),
        ("filter.noise.reduceNoise", json!({})),
        ("filter.other.custom", json!({"kernel": edge})),
        ("filter.video.deInterlace", json!({})),
        ("filter.stylize.traceContour", json!({})),
        ("filter.stylize.wind", json!({"method": "blast"})),
        ("filter.stylize.extrude", json!({})),
        ("filter.render.tree", json!({"baseTreeType": 1})),
        ("filter.sharpen.sharpen", json!({})),
        ("filter.sharpen.sharpenMore", json!({})),
        ("filter.sharpen.sharpenEdges", json!({})),
        ("filter.stylize.findEdges", json!({})),
        ("filter.stylize.emboss", json!({})),
        ("filter.noise.despeckle", json!({})),
        ("filter.gallery.graphicPen", json!({})),
        ("filter.gallery.stainedGlass", json!({"cellSize": 10})),
        ("image.mode.indexedColor", json!({"colors": 16})),
        ("image.mode.bitmap", json!({"method": "diffusion"})),
    ] {
        let doc = if cmd == "image.mode.bitmap" { &gray } else { &doc };
        let bg = doc.layers[0].id;
        let applied = preview_document(doc, Some(bg), cmd, &p, 1).unwrap();
        let old = preview_document(doc, Some(bg), cmd, &p, 4).unwrap();
        assert!(diff_at_quarter(&applied, &old) > 0.005, "{cmd}: a 1/4 proxy differs from the result");
        let k = preview_factor(cmd, &p, 4);
        assert_eq!(k, 1, "{cmd}");
        let shown = preview_document(doc, Some(bg), cmd, &p, k).unwrap();
        assert!(diff_at_quarter(&applied, &shown) < 1e-4, "{cmd}: the preview is what OK applies");
    }
    // Without dithering, the colour-mode conversions keep the proxy.
    assert_eq!(preview_factor("image.mode.indexedColor", &json!({"dither": "none"}), 4), 4);
    assert_eq!(preview_factor("image.mode.bitmap", &json!({"method": "threshold"}), 4), 4);
}

/// #6530: the Content-Aware Fill preview runs at full resolution, for every Sampling choice,
/// because its patch search finds different matches on a reduced copy.
#[test]
fn content_aware_fill_previews_at_full_resolution() {
    for params in [json!({}), json!({"sampling": "auto"}), json!({"sampling": "rectangular"})] {
        assert_eq!(preview_factor("edit.contentAwareFill", &params, 4), 1, "{params}");
    }
}

/// #2063: proxy previews scaled what isn't a length (Tree's canvas fraction, Lens Correction's
/// perspective), clamped negative lengths to +0.1 (Offset left/up), left Shadows/Highlights'
/// radii in document pixels and rounded Mosaic's 10 px cells to 3 proxy px (12 px).
#[test]
fn proxy_previews_scale_only_lengths() {
    let doc = waves(256);
    let bg = doc.layers[0].id;
    let error = |cmd: &str, p: Value| {
        let k = preview_factor(cmd, &p, 4);
        let applied = preview_document(&doc, Some(bg), cmd, &p, 1).unwrap();
        let shown = preview_document(&doc, Some(bg), cmd, &p, k).unwrap();
        (k, mean_abs(&every_kth(&applied, k), &every_kth(&shown, 1)))
    };
    for (cmd, p) in [
        ("filter.lensCorrection", json!({"vertical": -100.0, "horizontal": 60.0})),
        ("filter.other.offset", json!({"horizontal": -40, "vertical": -24, "undefinedAreas": "wrap"})),
        (
            "image.adjustments.shadowsHighlights",
            json!({"shadowAmount": 50.0, "shadowRadius": 30.0, "highlightAmount": 50.0, "highlightRadius": 30.0, "blackClip": 0.0, "whiteClip": 0.0}),
        ),
    ] {
        let (k, diff) = error(cmd, p);
        assert_eq!(k, 4, "{cmd} keeps the proxy");
        assert!(diff < 0.01, "{cmd}: the preview shows what OK applies ({diff})");
    }
    // Mosaic's cells divide evenly: 10 px at k = 2 is 5 proxy px; 40 px keeps k = 4.
    assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": 10.0}), 4), 2);
    assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": 40.0}), 4), 4);
    assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": 7.0}), 4), 1);
    assert_eq!(preview_factor("filter.pixelate.mosaic", &json!({"cellSize": "x"}), 4), 4);
    let (k, diff) = error("filter.pixelate.mosaic", json!({"cellSize": 10.0}));
    assert!(k == 2 && diff < 0.01, "Mosaic: {diff}");
    // Tree's and Picture Frame's sizes are relative, not pixels (and not shown as "px").
    assert!(!is_pixel_param_of("filter.render.tree", "size") && !is_pixel_param_of("filter.render.pictureFrame", "size"));
    assert!(is_pixel_param_of("filter.stylize.extrude", "size"));
}

/// #218: Warp Text renders the type again from the layer model, which a reduced proxy doesn't
/// scale: the preview showed the text k times too large. It is the full-size result, reduced.
#[test]
fn warp_text_preview_matches_the_full_size_result_on_a_reduced_proxy() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 4000, "height": 3000})).unwrap();
    s.execute("type.create", json!({"x": 300, "y": 1600, "text": "Warp", "size": 400})).unwrap();
    let st = s.active().unwrap();
    let (doc, layer) = ((*st.doc).clone(), st.active_layer.unwrap());
    let p = json!({"style": "arc", "bend": 40.0});
    let k = preview_factor("type.warpText", &p, crate::proxy::preview_factor(&doc, true));
    assert!(k > 1, "the preview runs on a reduced proxy");
    let cache = |d: &Document| match &d.layer(layer).unwrap().content {
        photocraft_doc::LayerContent::Text(t) => t.cache.clone().unwrap(),
        _ => panic!("a type layer"),
    };
    let full = crate::proxy::proxy_document(&preview_document(&doc, Some(layer), "type.warpText", &p, 1).unwrap(), k);
    let shown = preview_document(&doc, Some(layer), "type.warpText", &p, k).unwrap();
    assert_eq!(shown.size, full.size);
    assert_eq!(cache(&shown).content_bounds(), cache(&full).content_bounds());
    assert_eq!(photocraft_compose::flatten(&shown).px, photocraft_compose::flatten(&full).px);
}

/// #5766: Curves on a smart object adds a smart filter, which re-renders the source through
/// the object's placement. On a reduced proxy that placement stayed in full-size document
/// coordinates, so the preview showed the object `k` times too large until OK.
#[test]
fn smart_object_curves_preview_keeps_its_size_on_a_reduced_proxy() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 16, "y": 16, "width": 16, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"contents": "black"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let st = s.active().unwrap();
    let (doc, layer) = ((*st.doc).clone(), st.active_layer.unwrap());
    let alpha = |d: &Document, x: i32, y: i32| match &d.layer(layer).unwrap().content {
        photocraft_doc::LayerContent::Smart(sm) => sm.cache.as_ref().unwrap().pixel(x, y).last().copied().unwrap(),
        _ => panic!("a smart object"),
    };
    let k = 4;
    let params = json!({"points": [[0, 20], [90, 150], [255, 230]]});
    let shown = preview_document(&doc, Some(layer), "image.adjustments.curves", &params, k).unwrap();
    let full = preview_document(&doc, Some(layer), "image.adjustments.curves", &params, 1).unwrap();
    let photocraft_doc::LayerContent::Smart(full_sm) = &full.layer(layer).unwrap().content else { panic!("a smart object") };
    let reference = photocraft_algo::resample::resize_surface(full_sm.cache.as_ref().unwrap(), 0.25, 0.25, photocraft_algo::resample::Resample::Bicubic);
    let photocraft_doc::LayerContent::Smart(sm) = &shown.layer(layer).unwrap().content else { panic!("a smart object") };
    assert_eq!(sm.smart_filters.len(), 1, "the preview adds the Curves smart filter");
    assert_eq!(shown.size, crate::proxy::proxy_document(&doc, k).size);
    // The 16 px square at (16, 16) is a 4 px square at (4, 4) on the proxy, not 16 px at (16, 16).
    // Bicubic reduction blends the square's boundary with transparency; compare all alpha
    // values with a reduced full-size result, and require opacity only in the interior.
    for y in 0..shown.size.height as i32 {
        for x in 0..shown.size.width as i32 {
            let expected = reference.pixel(x, y).last().copied().unwrap();
            let actual = alpha(&shown, x, y);
            assert!((actual - expected).abs() <= 1.0 / 255.0, "({x}, {y}): alpha {actual}, expected {expected}");
        }
    }
    for (x, y) in [(5, 5), (5, 6), (6, 6)] {
        assert!(alpha(&shown, x, y) > 0.99, "({x}, {y}) is inside the reduced square");
    }
    for (x, y) in [(4, 4), (7, 7)] {
        assert!(alpha(&shown, x, y) > 0.5, "({x}, {y}) is a corner of the reduced square");
    }
    for (x, y) in [(2, 2), (9, 9), (12, 12), (15, 15)] {
        assert!(alpha(&shown, x, y) < 0.01, "({x}, {y}) is outside the reduced square");
    }
}

/// #2063: a pixel-sized parameter divided by the proxy factor was clamped to the command's
/// minimum (Pointillize's 5 px cells became 3 px proxy cells = 12 px), so the preview factor
/// drops until every pixel-sized value fits its range.
#[test]
fn proxy_previews_never_clamp_pixel_sized_params() {
    let pointillize = |c: f64| preview_factor("filter.pixelate.pointillize", &json!({"cellSize": c}), 4);
    assert_eq!((pointillize(5.0), pointillize(7.0), pointillize(12.0), pointillize(300.0)), (1, 2, 4, 4));
    assert_eq!(preview_factor("filter.pixelate.crystallize", &json!({"cellSize": 10.0}), 4), 3);
    assert_eq!(preview_factor("filter.pixelate.colorHalftone", &json!({"maxRadius": 8.0}), 4), 2);
    assert_eq!(preview_factor("filter.blur.gaussianBlur", &json!({"radius": 8.0}), 4), 4, "unclamped filters keep the proxy");
    // Hostile params never panic and leave the factor alone; a full-size document stays at 1.
    assert_eq!(preview_factor("filter.pixelate.pointillize", &json!({"cellSize": "x"}), 4), 4);
    assert_eq!(preview_factor("filter.pixelate.pointillize", &json!(null), 4), 4);
    assert_eq!(preview_factor("no.such.command", &json!({"cellSize": 1.0}), 4), 4);
    assert_eq!(preview_factor("filter.pixelate.pointillize", &json!({"cellSize": 5.0}), 0), 1);
    // Every previewed command, at each pixel-sized param's minimum, default and in between.
    let previewed = photocraft_engine::command_specs().iter().filter(|c| has_dialog(c.id) && (c.id.starts_with("filter.") || PREVIEWED.contains(&c.id)));
    for c in previewed {
        for p in parse_spec(c.params) {
            let Kind::Range { min, default, .. } = p.kind else { continue };
            if min <= 0.0 || !is_pixel_param_of(c.id, &p.key) {
                continue;
            }
            for v in [f64::from(min), f64::from(default), f64::from(min) * 2.5] {
                let k = preview_factor(c.id, &json!({ p.key.clone(): v }), 4);
                assert!(v / f64::from(k) >= f64::from(min) - 1e-9, "{} {}={v}: k={k} clamps to {min}", c.id, p.key);
            }
        }
    }
}

/// Sharp structure at every scale: [`waves`] with hard squares from 2 to 32 px, a fine
/// per-pixel texture and an interlaced band (odd rows inverted) at the bottom.
fn sharp(n: u32) -> Document {
    let mut doc = waves(n);
    let s = doc.layers[0].surface_mut().unwrap();
    let mut x0 = 4;
    for (i, size) in [2, 3, 4, 6, 8, 12, 16, 24, 32].into_iter().enumerate() {
        for y0 in (4..n as i32 * 3 / 4 - size).step_by((size * 3) as usize) {
            let v = if (i + y0 as usize / 7).is_multiple_of(2) { 0.05 } else { 0.95 };
            s.fill_rect(photocraft_geom::Rect::new(x0, y0, x0 + size, y0 + size), &[v, v * 0.8, 1.0 - v, 1.0]);
        }
        x0 += size + 6;
    }
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            let mut px = s.pixel(x, y);
            if (x * 7 + y * 13) % 5 == 0 {
                px.iter_mut().take(3).for_each(|c| *c = (*c + 0.08).min(1.0));
            }
            if y >= n as i32 * 3 / 4 && y % 2 == 1 {
                px[0] = 1.0 - px[0];
            }
            s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &px);
        }
    }
    doc
}

/// [`waves`] with strong per-pixel noise, the input of noise reduction.
fn noisy(n: u32) -> Document {
    let mut doc = waves(n);
    let s = doc.layers[0].surface_mut().unwrap();
    let hash = |x: i32, y: i32| ((x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663)).wrapping_mul(2_654_435_761) >> 8;
    for y in 0..n as i32 {
        for x in 0..n as i32 {
            let mut px = s.pixel(x, y);
            for (c, v) in px.iter_mut().take(3).enumerate() {
                *v = (*v + (hash(x * 3 + c as i32, y) as f32 / (1u32 << 24) as f32 - 0.5) * 0.3).clamp(0.0, 1.0);
            }
            s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &px);
        }
    }
    doc
}

/// How a preview at factor `k` compares with what OK applies (factor 1) on a document.
#[derive(Debug)]
struct PreviewError {
    /// Mean absolute RGB difference at the pixels the proxy samples.
    diff: f32,
    /// Feature scale of the preview shown at document size, relative to the result: 1 when
    /// the features match, 1/k when the preview draws them k times too large.
    scale: f32,
    /// How much the preview changes the image relative to how much OK does (sampled); NaN
    /// when OK barely changes it.
    change: f32,
}

/// Mean absolute difference of neighbouring pixels at the document's own resolution.
fn detail(d: &Document) -> f32 {
    let b = photocraft_compose::flatten(d);
    let w = b.rect.width() as usize;
    let mut sum = 0.0;
    for (i, p) in b.px.iter().enumerate() {
        for q in [b.px.get(i + 1).filter(|_| (i + 1) % w != 0), b.px.get(i + w)].into_iter().flatten() {
            sum += (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>() / 3.0;
        }
    }
    sum / b.px.len().max(1) as f32
}

/// Pixels of `d` at every `k`-th row and column: the document pixels a factor-`k` proxy
/// samples, so a preview at `k` compares with them one to one.
fn every_kth(d: &Document, k: u32) -> Vec<[f32; 4]> {
    let b = photocraft_compose::flatten(d);
    let w = b.rect.width() as usize;
    let k = k as usize;
    let h = b.px.len() / w.max(1);
    (0..h.div_ceil(k))
        .flat_map(|y| (0..w.div_ceil(k)).map(move |x| (x * k, y * k)))
        .map(|(x, y)| b.px.get(y * w + x).copied().unwrap_or([f32::NAN; 4]))
        .collect()
}

fn mean_abs(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
    a.iter().zip(b).map(|(p, q)| (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>() / 3.0).sum::<f32>() / a.len().max(1) as f32
}

/// `None` when the command can't run on `doc` at all (e.g. Displace without a map).
fn preview_error(doc: &Document, command: &str, p: &Value, k: u32) -> Option<PreviewError> {
    let bg = doc.layers[0].id;
    let applied = preview_document(doc, Some(bg), command, p, 1)?;
    let shown = preview_document(doc, Some(bg), command, p, k).unwrap_or_else(|| panic!("{command} {p}: the k = {k} preview failed"));
    let (a, s) = (every_kth(&applied, k), every_kth(&shown, 1));
    let (src, src_k) = (every_kth(doc, k), every_kth(&crate::proxy::proxy_document(doc, k), 1));
    let applied_change = mean_abs(&a, &src);
    Some(PreviewError {
        diff: mean_abs(&a, &s),
        scale: detail(&shown) / k as f32 / detail(&applied).max(1e-5),
        change: if applied_change > 0.01 { mean_abs(&s, &src_k) / applied_change } else { f32::NAN },
    })
}

/// The parameter sets to preview `command` with: the dialog's defaults, and each pixel-sized
/// value at its minimum and at a large value (whole numbers as integers: Tree and Picture
/// Frame read some ranges as `u32`).
fn sweep_params(app: &mut PhotocraftApp, command: &str) -> Vec<(String, Value)> {
    let Some(id) = open(app, command) else { return Vec::new() };
    let base = params_of(&app.ui.dialog_mut(id).unwrap().fields);
    let mut out = vec![("default".to_string(), base.clone())];
    for p in parse_spec(photocraft_engine::commands::find(command).unwrap().params) {
        if !is_pixel_param_of(command, &p.key) {
            continue;
        }
        let values = match p.kind {
            Kind::Int { .. } => vec![-40.0, 40.0],
            Kind::Range { min, max, default } => [min, (default * 4.0).clamp(min, max.min(64.0))].into_iter().filter(|v| *v != default).collect(),
            _ => continue,
        };
        for v in values {
            let mut q = base.clone();
            q[p.key.as_str()] = json!(v);
            out.push((format!("{}={v}", p.key), q));
        }
    }
    for (_, p) in &mut out {
        for v in p.as_object_mut().into_iter().flat_map(|o| o.values_mut()) {
            if let Some(x) = v.as_f64().filter(|x| x.fract() == 0.0 && x.abs() < 1e9) {
                *v = json!(x as i64);
            }
        }
    }
    out
}

/// Every previewed command with its parameter sets.
fn previewed_cases() -> Vec<(&'static str, String, Value)> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ids: Vec<&'static str> =
        photocraft_engine::command_specs().iter().map(|c| c.id).filter(|id| has_dialog(id) && (id.starts_with("filter.") || PREVIEWED.contains(id))).collect();
    ids.into_iter().flat_map(|id| sweep_params(&mut app, id).into_iter().map(move |(label, p)| (id, label, p))).collect()
}

fn grayscale(d: &Document) -> Document {
    let mut s = photocraft_engine::Session::new();
    s.add_document(d.clone(), None);
    s.execute("image.mode.grayscale", json!({})).unwrap();
    s.active().map(|d| (*d.doc).clone()).unwrap()
}

/// Smooth, sharp and noisy test images, each in RGB and grayscale (for the modes that need it).
fn test_images(n: u32) -> [(&'static str, Document, Document); 3] {
    let with_gray = |d: Document| (grayscale(&d), d);
    let ((ws, w), (ss, s), (ns, no)) = (with_gray(waves(n)), with_gray(sharp(n)), with_gray(noisy(n)));
    [("smooth", w, ws), ("sharp", s, ss), ("noisy", no, ns)]
}

fn image_for<'a>(command: &str, rgb: &'a Document, gray: &'a Document) -> &'a Document {
    if command == "image.mode.bitmap" || command == "image.mode.duotone" { gray } else { rgb }
}

/// Previews that only approximate the result at a proxy although their features have the
/// right size: dots and cells placed on the proxy's pixel grid (Color Halftone, Pointillize)
/// and tile edges resampled (Tiles). Two more only differ on a proxy as small as this test's
/// (64 x 64 px): Picture Frame's minimum line widths, and Shadows/Highlights' 0.01 % black and
/// white clips, which pick single pixels out of 4096 (its radii are checked in
/// `proxy_previews_scale_only_lengths`).
const APPROXIMATE_AT_PROXY: &[&str] = &[
    "filter.pixelate.colorHalftone",
    "filter.pixelate.pointillize",
    "filter.stylize.tiles",
    "filter.render.pictureFrame",
    "image.adjustments.shadowsHighlights",
];

/// Why a preview doesn't show what OK applies, if it doesn't: a clearly different result or
/// features at another size on the smooth image, or (on the sharp and noisy images, where a
/// sub-pixel radius can't show on a proxy) a filter that acts more strongly in the preview,
/// i.e. over a k times larger area.
fn mismatch(image: &str, e: &PreviewError) -> Option<String> {
    let off = if image == "smooth" { e.diff > 0.03 || (!(0.6..=1.6).contains(&e.scale) && e.diff > 0.002) } else { e.change > 1.25 };
    off.then(|| format!("{e:?}"))
}

/// #2063, every previewed command: the live preview at the factor [`preview_factor`] picks
/// for a 1/4 proxy shows what OK applies: the same features at the same size, on smooth,
/// sharp and noisy images, at the defaults and at small and large pixel sizes.
#[test]
fn every_preview_shows_what_ok_applies() {
    let cases: Vec<_> = previewed_cases().into_iter().filter(|(id, _, p)| preview_factor(id, p, 4) > 1 && !APPROXIMATE_AT_PROXY.contains(id)).collect();
    // One thread per test image: about 270 filter runs at 256 x 256.
    let bad: Vec<String> = std::thread::scope(|scope| {
        let runs: Vec<_> = test_images(256)
            .into_iter()
            .map(|(image, rgb, gray)| {
                let cases = &cases;
                scope.spawn(move || {
                    let mut bad = Vec::new();
                    for (id, label, p) in cases {
                        let k = preview_factor(id, p, 4);
                        if let Some(why) = preview_error(image_for(id, &rgb, &gray), id, p, k).and_then(|e| mismatch(image, &e)) {
                            bad.push(format!("{id} ({label}) at k = {k} on the {image} image: {why}"));
                        }
                    }
                    bad
                })
            })
            .collect();
        runs.into_iter().flat_map(|r| r.join().unwrap()).collect()
    });
    assert!(bad.is_empty(), "previews that differ from the result:\n{}", bad.join("\n"));
}

/// The table behind [`every_preview_shows_what_ok_applies`]: every previewed command at a
/// plain 1/4 proxy and at its chosen factor, on each test image.
/// `cargo test -p photocraft-ui-egui --lib filter_dialog::tests::sweep_all_previews -- --ignored --nocapture`
#[test]
#[ignore]
fn sweep_all_previews() {
    let images = test_images(256);
    println!("command | params | image | k | diff k=4 | scale k=4 | change k=4 | diff k | scale k | change k");
    for (id, label, p) in previewed_cases() {
        let k = preview_factor(id, &p, 4);
        for (image, rgb, gray) in &images {
            let doc = image_for(id, rgb, gray);
            match (preview_error(doc, id, &p, 4), preview_error(doc, id, &p, k)) {
                (Some(a), Some(b)) => println!(
                    "{id} | {label} | {image} | {k} | {:.4} | {:.2} | {:.2} | {:.4} | {:.2} | {:.2}",
                    a.diff, a.scale, a.change, b.diff, b.scale, b.change
                ),
                _ => println!("{id} | {label} | {image} | {k} | fails"),
            }
        }
    }
}

/// The profiles Assign Profile offers for a new document converted with `mode` (if any),
/// and which of the command's built-in profiles the command itself accepts for it.
fn assign_profile_offers(mode: Option<&str>) -> (Vec<String>, Vec<String>) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    if let Some(mode) = mode {
        app.run(mode, json!({})).unwrap();
    }
    open(&mut app, "edit.assignProfile").unwrap();
    let offered: Vec<String> =
        app.ui.dialogs.last().unwrap().fields["__profiles"].as_array().unwrap().iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
    let spec = photocraft_engine::commands::find("edit.assignProfile").unwrap();
    let builtin = parse_spec(spec.params).into_iter().find(|p| p.key == "profile").unwrap();
    let Kind::Choice { options, .. } = builtin.kind else { panic!("profile is a menu") };
    let accepted = options.into_iter().filter(|name| app.session.execute("edit.assignProfile", json!({"profile": name})).is_ok()).collect();
    (offered, accepted)
}

/// #4951: Assign Profile lists the profiles the command accepts for the document's colour
/// mode, and none of the others.
#[test]
fn assign_profile_offers_only_profiles_of_the_documents_colour_mode() {
    for mode in [None, Some("image.mode.cmyk"), Some("image.mode.grayscale")] {
        let (offered, accepted) = assign_profile_offers(mode);
        for name in &accepted {
            assert!(offered.contains(name), "{mode:?}: {name} can be assigned, so it is offered");
        }
        for name in ["srgb", "coated-cmyk", "sgray"] {
            assert_eq!(offered.iter().any(|o| o == name), accepted.iter().any(|a| a == name), "{mode:?}: {name}");
        }
        assert!(offered.iter().any(|o| o == "working") && offered.iter().any(|o| o == "none"), "{mode:?}: working space and none always fit");
    }
    let (rgb, _) = assign_profile_offers(None);
    assert!(rgb.iter().any(|o| o == "srgb") && !rgb.iter().any(|o| o == "coated-cmyk"), "an RGB document gets RGB profiles");
    let (cmyk, _) = assign_profile_offers(Some("image.mode.cmyk"));
    assert!(cmyk.iter().any(|o| o == "coated-cmyk") && !cmyk.iter().any(|o| o == "srgb"), "a CMYK document gets CMYK profiles");
}

/// #4951: a profile remembered from a document of another colour mode isn't preselected.
#[test]
fn assign_profile_drops_a_remembered_profile_that_does_not_fit() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.session.prefs.edit(|prefs| {
        prefs.dialogs.insert("edit.assignProfile".into(), json!({"profile": "coated-cmyk"}));
    });
    open(&mut app, "edit.assignProfile").unwrap();
    assert_eq!(app.ui.dialogs.last().unwrap().fields["profile"], json!("working"));
    app.session.prefs.edit(|prefs| {
        prefs.dialogs.insert("edit.assignProfile".into(), json!({"profile": "display-p3"}));
    });
    open(&mut app, "edit.assignProfile").unwrap();
    assert_eq!(app.ui.dialogs.last().unwrap().fields["profile"], json!("display-p3"), "one that fits is kept");
}

#[test]
fn schema_dialogs_respect_explicit_dropdown_defaults() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());

    // 1. Radial Blur has explicit default "good", not the first option ("draft").
    open(&mut app, "filter.blur.radialBlur").unwrap();
    let fields = &app.ui.dialogs.last().unwrap().fields;
    assert_eq!(fields["quality"], json!("good"), "Radial Blur must select explicit default 'good'");
    assert_eq!(fields["method"], json!("spin"), "Radial Blur method has no explicit default, retains first option");
    let params = params_of(fields);
    assert_eq!(params["quality"], json!("good"));

    // 2. Convert to Profile has explicit default "relative", not first option ("perceptual").
    open(&mut app, "edit.convertToProfile").unwrap();
    let fields = &app.ui.dialogs.last().unwrap().fields;
    assert_eq!(fields["intent"], json!("relative"), "Convert to Profile must select explicit default 'relative'");
    let params = params_of(fields);
    assert_eq!(params["intent"], json!("relative"));

    // 3. Proof Setup has explicit default "relative".
    open(&mut app, "view.proofSetup").unwrap();
    let fields = &app.ui.dialogs.last().unwrap().fields;
    assert_eq!(fields["intent"], json!("relative"), "Proof Setup must select explicit default 'relative'");
    let params = params_of(fields);
    assert_eq!(params["intent"], json!("relative"));

    // 4. Remembered values still override initial defaults.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.prefs.edit(|prefs| {
        prefs.dialogs.insert("filter.blur.radialBlur".into(), json!({"quality": "best"}));
    });
    open(&mut app, "filter.blur.radialBlur").unwrap();
    let fields = &app.ui.dialogs.last().unwrap().fields;
    assert_eq!(fields["quality"], json!("best"), "Remembered choice must override explicit default");

    // 5. Explicit default that is the first option (e.g. "cw" in arbitrary rotation).
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    open(&mut app, "image.rotation.arbitrary").unwrap();
    let fields = &app.ui.dialogs.last().unwrap().fields;
    assert_eq!(fields["direction"], json!("cw"), "Explicit default matching first option is preserved");
}

/// #4143: Replace Color shows the From colour (defaulting to the foreground) and its canvas
/// eyedropper samples the merged image into the dialog.
#[test]
fn replace_color_eyedropper_samples_the_image() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "background": "#00ff00"})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    let id = open(&mut app, REPLACE_COLOR).unwrap();
    assert_eq!(app.ui.dialogs.last().unwrap().fields["color"], json!("#ff0000"), "the dialog starts from the foreground colour");
    // #6279: armed as it opens, so the first click on the image samples.
    assert!(color_eyedropper_armed(&app));
    assert!(color_eyedropper_sample(&mut app, 4.0, 4.0));
    assert_eq!(app.ui.dialogs.last().unwrap().fields["color"], json!("#00ff00"));
    // #4457: where it was picked, for Localized Color Clusters.
    assert_eq!(app.ui.dialogs.last().unwrap().fields["points"], json!([[4.0, 4.0]]));
    // Off-image: the click is consumed, nothing changes and the eyedropper stays armed.
    assert!(color_eyedropper_sample(&mut app, -1.0, 4.0));
    assert_eq!(app.ui.dialogs.last().unwrap().fields["color"], json!("#00ff00"));
    assert!(color_eyedropper_armed(&app));
    // Disarmed, the dialog never claims the click.
    app.ui.dialog_mut(id).unwrap().fields.insert(COLOR_EYEDROPPER.into(), json!(false));
    assert!(!color_eyedropper_armed(&app));
    assert!(!color_eyedropper_sample(&mut app, 4.0, 4.0));
}

/// #4143: the eyedropper button disarms and re-arms the canvas picker (#6279: it opens armed).
#[test]
fn replace_color_dialog_arms_the_eyedropper_button() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), &mut app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    open(h.state_mut(), REPLACE_COLOR).unwrap();
    h.run_steps(3);
    assert!(color_eyedropper_armed(h.state()), "armed as the dialog opens");
    h.get_by_label("Pick Color From Image").click();
    h.run_steps(2);
    assert!(!color_eyedropper_armed(h.state()));
    h.get_by_label("Pick Color From Image").click();
    h.run_steps(2);
    assert!(color_eyedropper_armed(h.state()));
}

/// #6278: the Result swatch shows the sampled colour after the shift, and a colour picked for
/// it sets Hue, Saturation and Lightness so the sampled colour becomes it.
#[test]
fn replace_color_result_swatch_sets_the_sliders() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.run("tools.setColors", json!({"foreground": "#cc3333"})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), &mut app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = open(h.state_mut(), REPLACE_COLOR).unwrap();
    h.run_steps(3);
    // No shift yet: the Result is the sampled colour.
    assert_eq!(h.get_all_by_label("#cc3333").count(), 2, "the sampled colour and the Result both read #cc3333");
    h.get_by_role_and_label(egui::accesskit::Role::ColorWell, "Result").click();
    h.run_steps(3);
    let picker = h.state().ui.dialogs.last().unwrap();
    assert_ne!(picker.id, id, "the Color Picker opened over the dialog");
    assert_eq!(picker.fields["color"], json!("#cc3333"), "on the current Result");
    let picker = picker.id;
    h.state_mut().ui.dialog_mut(picker).unwrap().fields.insert("color".into(), json!("#3355cc"));
    crate::dialogs::confirm(h.state_mut(), picker).unwrap();
    h.state_mut().ui.dialogs.retain(|d| d.id != picker);
    h.run_steps(3);
    let f = h.state_mut().ui.dialog_mut(id).unwrap().fields.clone();
    let n = |k: &str| f[k].as_f64().unwrap() as f32;
    let got = photocraft_algo::tone::replace_color_result([0.8, 0.2, 0.2], n("hue"), n("saturation"), n("lightness"));
    let want = crate::color_picker_ui::parse_hex("#3355cc").unwrap();
    assert!(got.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.02), "the sliders turn the sampled colour into the pick: {got:?}");
    assert!(n("hue") != 0.0, "the hue moved");
    // Moving a slider afterwards is not mistaken for another pick.
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("lightness".into(), json!(10.0));
    h.run_steps(2);
    assert_eq!(h.state_mut().ui.dialog_mut(id).unwrap().fields["lightness"], json!(10.0));
}

/// #4457: Localized Color Clusters and its Range, as in Photoshop's dialog; the sampled point
/// and both settings reach the command.
#[test]
fn replace_color_localized_color_clusters() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    // Left half red, right half red too: localized on the left point, the right half stays.
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 40, "height": 10, "background": "#ff0000"})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 800.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), &mut app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = open(h.state_mut(), REPLACE_COLOR).unwrap();
    h.run_steps(3);
    let _ = h.get_by_label("Localized Color Clusters");
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert(COLOR_EYEDROPPER.into(), json!(true));
    assert!(color_eyedropper_sample(h.state_mut(), 2.0, 5.0));
    h.get_by_label("Localized Color Clusters").click();
    h.run_steps(2);
    let f = h.state().ui.dialogs.last().unwrap().fields.clone();
    assert_eq!((f["localized"].clone(), f["points"].clone()), (json!(true), json!([[2.0, 5.0]])));
    // The command with the dialog's fields, small range and a hue shift: only near (2, 5).
    let mut p: Map<String, Value> = f.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect();
    p.insert("range".into(), json!(10));
    p.insert("hue".into(), json!(120));
    h.state_mut().run(REPLACE_COLOR, Value::Object(p)).unwrap();
    let doc = &h.state().session.active().unwrap().doc;
    let px = |x: i32| doc.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(x, 5, x + 1, 6));
    assert!(px(2)[1] > 0.5, "near the point: shifted to green {:?}", px(2));
    assert!(px(38)[0] > 0.9 && px(38)[1] < 0.1, "far away: still red {:?}", px(38));
}
