use super::*;

/// #321: New Document offers the working space and the other profiles of the mode's space.
#[test]
fn color_profile_menu_lists_the_profiles_of_the_modes_space() {
    use photocraft_cms::{Builtin, ColorSpace, ProfileClass};
    use photocraft_engine::installed_profiles::InstalledProfile;
    let installed = [
        InstalledProfile { path: "press.icc".into(), description: "Press".into(), class: ProfileClass::Output, color_space: ColorSpace::Cmyk },
        InstalledProfile { path: "panel.icc".into(), description: "Panel".into(), class: ProfileClass::Display, color_space: ColorSpace::Rgb },
    ];
    let values = |mode: &str| {
        profile_options(color_mode_of(mode).map(photocraft_engine::color_cmds::mode_space), "sRGB", &installed).into_iter().map(|(v, _)| v).collect::<Vec<_>>()
    };
    let rgb = values("rgb");
    assert_eq!(rgb[0], "working");
    assert!(rgb.contains(&Builtin::AdobeRgbCompat.id().to_string()) && rgb.contains(&"panel.icc".to_string()));
    assert!(!rgb.contains(&Builtin::CoatedCmyk.id().to_string()) && !rgb.contains(&"press.icc".to_string()));
    let cmyk = values("cmyk");
    assert!(cmyk.contains(&"press.icc".to_string()) && !cmyk.contains(&Builtin::Srgb.id().to_string()));
    assert!(profile_options(None, "sRGB", &installed).len() == 1, "an unknown mode offers only the working space");
    // Every entry is one `file.new` takes for that mode.
    for mode in ["rgb", "gray", "cmyk", "lab"] {
        for v in values(mode).into_iter().filter(|v| !v.ends_with(".icc")) {
            let mut s = photocraft_engine::Session::new();
            s.execute("file.new", json!({"width": 2, "height": 2, "mode": mode, "profile": v})).unwrap_or_else(|e| panic!("{mode} {v}: {e}"));
        }
    }
}

/// A saved preset replaces the Color Profile picked before: its own, or the working space.
#[test]
fn applying_a_preset_replaces_the_picked_profile() {
    let mut s = photocraft_engine::Session::new();
    let base = json!({"width": 8, "height": 8, "mode": "cmyk", "unit": "px"});
    s.execute("document.presets.save", json!({"name": "Plain", "settings": base})).unwrap();
    let mut press = base.clone();
    press["profile"] = json!("coated-cmyk");
    s.execute("document.presets.save", json!({"name": "Press", "settings": press})).unwrap();
    let mut f = Map::new();
    f.insert("profile".into(), json!(photocraft_cms::Builtin::AdobeRgbCompat.id()));
    apply_saved_preset(&mut f, &s.presets.documents[0]);
    assert!(!f.contains_key("profile"), "{f:?}");
    assert!(s.execute("file.new", command_params(&f)).is_ok());
    apply_saved_preset(&mut f, &s.presets.documents[1]);
    assert_eq!(f["profile"], "coated-cmyk");
}

/// #9086: the Artboards checkbox's field reaches `file.new`, which starts the document as one
/// artboard; the dialog's own `__` keys don't.
#[test]
fn the_artboards_box_creates_an_artboard_document() {
    let mut s = photocraft_engine::Session::new();
    let mut f = Map::new();
    for (k, v) in [("width", json!(64)), ("height", json!(48)), ("artboard", json!(true)), ("__orientation", json!("landscape"))] {
        f.insert(k.into(), v);
    }
    s.execute("file.new", command_params(&f)).unwrap();
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.artboards().len(), 1);
    assert_eq!(doc.artboards()[0].2.rect, doc.bounds());
}

#[test]
fn units_round_trip_through_pixels() {
    assert_eq!(to_unit(2100.0, "in", 300.0), 7.0);
    assert_eq!(from_unit(7.0, "in", 300.0), 2100.0);
    assert_eq!(from_unit(2.54, "cm", 300.0), 300.0);
    assert_eq!(to_unit(640.0, "px", 72.0), 640.0);
}

/// #1147: 5 mm at 72 ppi is 14 px, which is 4.94 mm. The field must keep showing the typed 5.
#[test]
fn typed_size_in_a_physical_unit_shows_as_typed() {
    let mut f = crate::state::UiState::new_document_fields();
    set_size(&mut f, "height", 5.0, "mm", 72.0);
    assert_eq!(command_params(&f)["height"], json!(14));
    assert_eq!(shown_size(&f, "height", 1080.0, "mm", 72.0), 5.0);
    // Another unit, or pixels changed elsewhere (a preset, the orientation swap): converted.
    assert_eq!(shown_size(&f, "height", 1080.0, "cm", 72.0), to_unit(14.0, "cm", 72.0));
    f.insert("height".into(), json!(1080));
    assert_eq!(shown_size(&f, "height", 1080.0, "mm", 72.0), to_unit(1080.0, "mm", 72.0));
    // Pixels show whole pixels.
    set_size(&mut f, "width", 512.4, "px", 72.0);
    assert_eq!(shown_size(&f, "width", 1920.0, "px", 72.0), 512.0);
    // Over the 300000 px limit the field shows the limit.
    set_size(&mut f, "width", 300_000.0, "mm", 72.0);
    assert_eq!(command_params(&f)["width"], json!(300_000));
    assert_eq!(shown_size(&f, "width", 1920.0, "mm", 72.0), to_unit(300_000.0, "mm", 72.0));
}

/// #5661: 1 px at 300 ppi is 0.0033 in. Showing it in inches clamped the field to 0.01 in and
/// turned the width into 3 px without an edit.
#[test]
fn tiny_size_in_inches_is_not_changed_by_showing_it() {
    use egui_kittest::Harness;
    let app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut f = crate::state::UiState::new_document_fields();
    for (k, v) in [("width", json!(1)), ("height", json!(1)), ("resolution", json!(300.0)), ("__unit", json!("in"))] {
        f.insert(k.into(), v);
    }
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).build_ui_state(
        |ui, (app, f): &mut (crate::PhotocraftApp, Map<String, Value>)| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                body(app, ui, f);
            }
        },
        (app, f),
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    let p = command_params(&h.state().1);
    assert_eq!((p["width"].as_u64(), p["height"].as_u64()), (Some(1), Some(1)));
}

/// #1147: the Orientation buttons swap the typed values too: 841 x 1189 mm becomes 1189 x 841.
#[test]
fn orientation_swap_keeps_the_typed_values() {
    let mut f = crate::state::UiState::new_document_fields();
    set_size(&mut f, "width", 841.0, "mm", 72.0);
    set_size(&mut f, "height", 1189.0, "mm", 72.0);
    swap_size(&mut f);
    let p = command_params(&f);
    assert_eq!((p["width"].clone(), p["height"].clone()), (json!(3370), json!(2384)));
    assert_eq!((shown_size(&f, "width", 1920.0, "mm", 72.0), shown_size(&f, "height", 1080.0, "mm", 72.0)), (1189.0, 841.0));
}

#[test]
fn resolution_keeps_physical_size_in_physical_units_and_pixels_in_px() {
    let a4 = CATEGORIES.iter().find(|c| c.0 == "Print").unwrap().1.iter().find(|p| p.0 == "A4").unwrap();
    let mut f = crate::state::UiState::new_document_fields();
    apply_preset(&mut f, a4, &Default::default());
    f.insert("__unit".into(), json!("in"));
    set_resolution(&mut f, 150.0);
    let p = command_params(&f);
    assert_eq!((p["width"].clone(), p["height"].clone(), p["resolution"].clone()), (json!(1240), json!(1754), json!(150.0)));
    assert!(!f.contains_key("__preset"));

    let mut f = crate::state::UiState::new_document_fields();
    apply_preset(&mut f, a4, &Default::default());
    f.insert("__unit".into(), json!("px"));
    set_resolution(&mut f, 150.0);
    let p = command_params(&f);
    assert_eq!((p["width"].clone(), p["height"].clone(), p["resolution"].clone()), (json!(2480), json!(3508), json!(150.0)));
}

/// Units & Rulers › New Document Preset Resolutions: print presets keep their inches at the
/// Print Resolution, screen presets keep their pixels at the Screen Resolution.
#[test]
fn iso_b_paper_presets_use_correct_dimensions_and_selected_units() {
    let presets = CATEGORIES.iter().find(|c| c.0 == "Print").unwrap().1;
    let mut f = crate::state::UiState::new_document_fields();
    f.insert("__unit".into(), json!("mm"));
    for (name, w, h) in [("B3", 4169, 5906), ("B4", 2953, 4169)] {
        let preset = presets.iter().find(|p| p.0 == name).unwrap();
        assert_eq!((preset.1, preset.2, preset.3), (w, h, 300.0));
        assert_eq!(preset_unit(preset, &f), "mm");
        apply_preset(&mut f, preset, &Default::default());
        let params = command_params(&f);
        assert_eq!((params["width"].as_u64(), params["height"].as_u64()), (Some(u64::from(w)), Some(u64::from(h))));
        assert_eq!(params["resolution"], 300.0);
    }
}

#[test]
fn presets_follow_the_preset_resolution_preferences() {
    let find = |cat: &str, name: &str| *CATEGORIES.iter().find(|c| c.0 == cat).unwrap().1.iter().find(|p| p.0 == name).unwrap();
    let (letter, web) = (find("Print", "Letter"), find("Web", "Web Minimum"));
    let stock = UnitsAndRulers::default();
    assert_eq!(at_pref_resolutions(&letter, &stock), letter, "the defaults are the listed sizes");
    assert_eq!(at_pref_resolutions(&web, &stock), web);

    let units = UnitsAndRulers { print_resolution: 200.0, screen_resolution: 96.0, ..stock.clone() };
    assert_eq!(at_pref_resolutions(&letter, &units), ("Letter", 1700, 2200, 200.0), "8.5 x 11 in at 200 ppi");
    assert_eq!(at_pref_resolutions(&find("Print", "A4"), &units), ("A4", 1653, 2339, 200.0));
    assert_eq!(at_pref_resolutions(&web, &units), ("Web Minimum", 1024, 768, 96.0), "pixels kept");
    let clipboard = (CLIPBOARD, 123, 45, 72.0);
    assert_eq!(at_pref_resolutions(&clipboard, &units), clipboard, "the copied pixels keep their resolution");

    let mut f = crate::state::UiState::new_document_fields();
    apply_preset(&mut f, &letter, &units);
    assert_eq!((f["width"].as_u64(), f["height"].as_u64(), f["resolution"].as_f64()), (Some(1700), Some(2200), Some(200.0)));
    assert_eq!((f["__preset"].as_str(), f["__unit"].as_str()), (Some("Letter"), Some("in")), "still a print preset below 300 ppi");
    apply_preset(&mut f, &web, &units);
    assert_eq!((f["width"].as_u64(), f["resolution"].as_f64(), f["__unit"].as_str()), (Some(1024), Some(96.0), Some("px")));

    // Out-of-range values (a hand-edited preferences file) never make an unusable size.
    for bad in [0.0, -5.0, f64::NAN, f64::INFINITY] {
        let units = UnitsAndRulers { print_resolution: bad, screen_resolution: bad, ..stock.clone() };
        assert_eq!(at_pref_resolutions(&letter, &units), letter, "{bad}");
        assert_eq!(at_pref_resolutions(&web, &units), web, "{bad}");
    }
    let huge = UnitsAndRulers { print_resolution: 1e9, ..stock };
    let (_, w, h, _) = at_pref_resolutions(&find("Art & Illustration", "Poster"), &huge);
    assert_eq!((w, h), (300_000, 300_000), "clamped to the file.new limit");
}

#[test]
fn preset_card_titles_fit_the_card_in_every_language() {
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        for lang in crate::i18n::Lang::all() {
            for p in CATEGORIES.iter().flat_map(|c| c.1.iter()) {
                let title = crate::i18n::tr(lang, p.0);
                let g = card_title(ui.painter(), title, 152.0, egui::Color32::WHITE);
                assert!(g.size().x <= 152.0 && g.rows.len() <= 2 && !g.elided, "{}: {title}", lang.code());
            }
        }
    });
    out.textures_delta.clear();
}

#[test]
fn recent_category_lists_clipboard_then_created_documents_then_uncovered_defaults() {
    let settings = |w, h, ppi| DocumentSettings {
        width: w,
        height: h,
        resolution: ppi,
        mode: "rgb".into(),
        depth: 8,
        background: "white".into(),
        background_color: None,
        profile: None,
        unit: "px".into(),
        resolution_unit: "in".into(),
    };
    let mut f = Map::new();
    set_clipboard(&mut f, 20, 30);
    let cards = recent_cards(&f, &[settings(800, 600, 72.0), settings(1920, 1080, 72.0)], &UnitsAndRulers::default());
    let titles: Vec<(&str, String)> = cards.iter().map(|c| (c.title(), c.size_line(&UnitsAndRulers::default(), &f))).collect();
    assert_eq!(
        titles,
        [
            (CLIPBOARD, "20 x 30 px @ 72 ppi".to_string()),
            ("Custom", "800 x 600 px @ 72 ppi".to_string()),
            ("HDTV 1080p", "1920 x 1080 px @ 72 ppi".to_string()),
            ("Default PhotoCraft Size", "7 x 5 in @ 300 ppi".to_string()),
        ]
    );
    assert!(holds(&f, &settings(20, 30, 72.0)) && !holds(&f, &settings(800, 600, 72.0)));
}

#[test]
fn clipboard_preset_comes_first_and_is_selected() {
    // Nothing on the clipboard: the dialog opens as before.
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let f = app.new_document_fields();
    assert_eq!(f, crate::state::UiState::new_document_fields());
    assert!(clipboard_preset(&f).is_none());
    // Pixels copied in the app: the Clipboard preset takes their size, at 72 ppi, selected.
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("select.rect", json!({"x": 10, "y": 20, "width": 123, "height": 45})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    let f = app.new_document_fields();
    assert_eq!(clipboard_preset(&f), Some((CLIPBOARD, 123, 45, 72.0)));
    assert_eq!((f["width"].as_u64(), f["height"].as_u64(), f["__preset"].as_str()), (Some(123), Some(45), Some(CLIPBOARD)));
    let p = command_params(&f);
    assert!(p.get("__clipboard").is_none(), "file.new never sees the dialog's keys");
    assert_eq!((p["width"].as_u64(), p["height"].as_u64(), p["resolution"].as_f64()), (Some(123), Some(45), Some(72.0)));
}

#[test]
fn preset_sets_size_resolution_and_create_params_drop_ui_keys() {
    let mut f = crate::state::UiState::new_document_fields();
    let a4 = CATEGORIES.iter().find(|c| c.0 == "Print").unwrap().1.iter().find(|p| p.0 == "A4").unwrap();
    apply_preset(&mut f, a4, &Default::default());
    let p = command_params(&f);
    assert_eq!(p["width"], 2480);
    assert_eq!(p["height"], 3508);
    assert_eq!(p["resolution"], 300.0);
    assert!(p.get("__preset").is_none() && p.get("__unit").is_none());
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", p).unwrap();
    let d = &s.active().unwrap().doc;
    assert_eq!((d.size.width, d.size.height, d.resolution_dpi), (2480, 3508, 300.0));
}

#[test]
fn saved_selection_restores_all_settings_but_keeps_the_document_name() {
    let mut session = photocraft_engine::Session::new();
    session
        .execute(
            "document.presets.save",
            json!({"name":"Print proof","settings":{
                "width":1600,"height":800,"resolution":254.0,"mode":"cmyk","depth":16,
                "background":"backgroundColor","backgroundColor":[0.2,0.3,0.4],"unit":"mm","resolutionUnit":"cm"
            }}),
        )
        .unwrap();
    let preset = session.presets.documents[0].clone();
    let mut f = crate::state::UiState::new_document_fields();
    set_clipboard(&mut f, 20, 30);
    f.insert("name".into(), json!("Catalog cover"));
    apply_saved_preset(&mut f, &preset);
    assert_eq!(f["name"], "Catalog cover");
    assert_eq!(f["__unit"], "mm");
    assert_eq!(f["__resUnit"], "cm");
    assert_eq!(f["__savedPreset"], "Print proof");
    assert!(f.get("__preset").is_none());
    assert_eq!(clipboard_preset(&f), Some((CLIPBOARD, 20, 30, 72.0)));
    let mut expected = preset.settings.command_params();
    expected["name"] = json!("Catalog cover");
    assert_eq!(command_params(&f), expected);
    f.insert("width".into(), json!(42));
    assert_eq!(session.presets.documents[0], preset, "editing the form does not edit the snapshot");
    apply_preset(&mut f, &(CLIPBOARD, 20, 30, 72.0), &Default::default());
    assert!(f.get("__savedPreset").is_none());
    assert_eq!((f["width"].as_u64(), f["height"].as_u64()), (Some(20), Some(30)));
}
/// The real dialog (#254): a typed size must reach `file.new`, however it is confirmed.
mod dialog {
    use super::super::{CATEGORIES, apply_preset};
    use crate::PhotocraftApp;
    use crate::state::{DialogKind, UiState};
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::sync::{Arc, Mutex};

    /// The real preference services, backed by an in-memory store shared across app restarts.
    fn harness_with_store(store: &Arc<Mutex<Option<String>>>) -> Harness<'static, PhotocraftApp> {
        let (read, write) = (store.clone(), store.clone());
        let services = crate::Services {
            load_prefs: Some(Box::new(move || read.lock().unwrap().clone())),
            save_prefs: Some(Box::new(move |text| {
                *write.lock().unwrap() = Some(text.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
            |ui, app| {
                crate::prefs_ui::tick(app, ui.ctx());
                crate::dialogs::show(app, ui.ctx());
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
        h.run_steps(3);
        h
    }

    /// #4789: the swatch beside Background Contents (and Custom…) opens the Color Picker; its
    /// colour becomes the new document's background (kept as its "#rrggbb").
    #[test]
    fn a_custom_background_color_is_picked_and_used() {
        let mut h = harness_with_store(&Arc::new(Mutex::new(None)));
        let nd = h.state().ui.dialogs[0].id;
        assert!(h.query_by_label("Custom background color").is_some());
        h.get_by_label("Custom background color").click();
        h.run_steps(3);
        let picker = h
            .state()
            .ui
            .dialogs
            .iter()
            .find(|d| d.fields.get("__dialog").and_then(serde_json::Value::as_u64) == Some(nd))
            .map(|d| d.id)
            .expect("the Color Picker opens over the dialog");
        h.state_mut().ui.dialog_mut(picker).unwrap().fields.insert("color".into(), serde_json::json!("#336699"));
        crate::dialogs::confirm(h.state_mut(), picker).unwrap();
        h.run_steps(3);
        let bg = h.state().ui.dialogs.iter().find(|d| d.id == nd).unwrap().fields["background"].clone();
        assert_eq!(bg, "#336699");
        crate::dialogs::confirm(h.state_mut(), nd).unwrap();
        let doc = &h.state().session.active().unwrap().doc;
        let px = photocraft_compose::flatten(doc).px[0];
        assert!((px[0] * 255.0 - 51.0).abs() < 1.0 && (px[1] * 255.0 - 102.0).abs() < 1.0 && (px[2] * 255.0 - 153.0).abs() < 1.0, "{px:?}");
    }

    /// #4789: the swatch shows what each Background Contents choice paints, Background Color included.
    #[test]
    fn the_swatch_shows_each_background_choice() {
        let mut h = harness_with_store(&Arc::new(Mutex::new(None)));
        h.state_mut().run("tools.setColors", serde_json::json!({"foreground": "#000000", "background": "#336699"})).unwrap();
        let rgb = |bg: &str| super::super::background_rgb(h.state(), serde_json::json!({ "background": bg }).as_object().unwrap());
        assert_eq!(rgb("white"), Some([1.0; 3]));
        assert_eq!(rgb("black"), Some([0.0; 3]));
        assert_eq!(rgb("transparent"), None);
        assert_eq!(rgb("#ff0000"), Some([1.0, 0.0, 0.0]));
        let c = rgb("backgroundColor").expect("Background Color shows the toolbox colour");
        assert!((c[0] * 255.0 - 51.0).abs() < 1.0 && (c[1] * 255.0 - 102.0).abs() < 1.0 && (c[2] * 255.0 - 153.0).abs() < 1.0, "{c:?}");
    }

    fn name_preset(h: &mut Harness<'static, PhotocraftApp>, name: &str) {
        h.get_by_label("Save Preset…").click();
        h.run_steps(2);
        h.get_by_role_and_label(Role::TextInput, "Preset Name").click();
        h.run_steps(1);
        h.event(egui::Event::Text(name.into()));
        h.run_steps(1);
    }

    #[test]
    fn save_restart_select_create_and_delete_through_the_dialog() {
        let store = Arc::new(Mutex::new(None));
        let mut h = harness_with_store(&store);
        type_into(&mut h, 0, "1600");
        type_into(&mut h, 1, "1600");
        let mut f = fields(&h);
        f.insert("background".into(), serde_json::json!("transparent"));
        set_fields(&mut h, f);
        name_preset(&mut h, " Product square ");
        h.get_by_label("Save").click();
        h.run_steps(3);
        assert!(h.state().session.documents().is_empty(), "saving does not create a document");
        assert_eq!(h.state().session.presets.documents[0].name, "Product square");
        type_into(&mut h, 0, "32");
        assert_eq!(h.state().session.presets.documents[0].settings.width, 1600);
        drop(h);

        let mut h = harness_with_store(&store);
        h.get_by_label("Saved").click();
        h.run_steps(2);
        h.get_by_label("Product square").click();
        h.run_steps(2);
        assert!(h.state().session.documents().is_empty(), "selection only populates the form");
        assert_eq!(fields(&h)["name"], "Untitled-1");
        assert_eq!(fields(&h)["background"], "transparent");
        h.get_by_label("Create").click();
        h.run_steps(3);
        assert_eq!(created(&h), (1600, 1600, 72.0));
        let pixel = h.state_mut().session.execute("document.pixel", serde_json::json!({"x":0,"y":0})).unwrap();
        assert_eq!(pixel[3], 0.0);

        h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
        h.run_steps(2);
        h.get_by_label("Saved").click();
        h.run_steps(2);
        h.get_by_label("Delete").click();
        h.run_steps(3);
        drop(h);
        let mut h = harness_with_store(&store);
        h.get_by_label("Saved").click();
        h.run_steps(2);
        assert!(h.state().session.presets.documents.is_empty());
        assert!(h.query_by_label("No saved presets yet.").is_some());
    }

    #[test]
    fn enter_saves_the_preset_and_bad_names_leave_the_form_open() {
        let mut h = harness();
        name_preset(&mut h, "Enter test");
        enter(&mut h);
        assert_eq!(h.state().session.presets.documents.len(), 1);
        assert!(h.state().session.documents().is_empty(), "Enter must not also press Create");
        assert_eq!(h.state().ui.dialogs.len(), 1);
        name_preset(&mut h, " ENTER TEST ");
        h.get_by_label("Save").click();
        h.run_steps(2);
        assert!(fields(&h).get("__presetError").is_some());
        assert_eq!(h.state().session.presets.documents.len(), 1);
        assert!(h.state().session.documents().is_empty());
        h.get_by_label("Cancel").click();
        h.run_steps(2);
        name_preset(&mut h, "");
        // Cancel keeps the entered name; clear it as an automation edit, then use the button.
        h.state_mut().ui.dialogs[0].fields.insert("__presetName".into(), serde_json::json!("   "));
        h.run_steps(2);
        h.get_by_label("Save").click();
        h.run_steps(2);
        assert!(fields(&h).get("__presetError").is_some());
        assert_eq!(h.state().session.presets.documents.len(), 1);
    }

    fn harness() -> Harness<'static, PhotocraftApp> {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h =
            Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
        h.run_steps(3);
        h
    }

    fn click_at(h: &mut Harness<'static, PhotocraftApp>, at: egui::Pos2) {
        h.hover_at(at);
        h.run_steps(1);
        h.drag_at(at);
        h.run_steps(1);
        h.drop_at(at);
        h.run_steps(2);
    }

    /// A finger tap at `at`. Events for one phase share a frame (`input_mut`), because `Harness::step` would otherwise run one frame per queued event and drop Touch before the click release (so `second_click` would not see touch).
    fn touch_tap_at(h: &mut Harness<'static, PhotocraftApp>, at: egui::Pos2) {
        let touch = |phase| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase, pos: at, force: None };
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        h.input_mut().events.extend([egui::Event::PointerMoved(at), touch(egui::TouchPhase::Start), button(true)]);
        h.step();
        h.input_mut().events.extend([touch(egui::TouchPhase::End), button(false)]);
        h.step();
        h.run_steps(1);
    }

    /// The Width (0), Height (1) and Resolution (2) fields.
    fn field(h: &Harness<'static, PhotocraftApp>, i: usize) -> egui::Rect {
        h.query_all_by_role(Role::SpinButton).nth(i).map(|n| n.rect()).expect("a size field")
    }

    /// Click into field `i` (which selects its text) and type `text`, as a user does.
    fn type_into(h: &mut Harness<'static, PhotocraftApp>, i: usize, text: &str) {
        let r = field(h, i);
        click_at(h, r.center());
        for c in text.chars() {
            h.event(egui::Event::Text(c.to_string()));
            h.run_steps(1);
        }
    }

    fn fields(h: &Harness<'static, PhotocraftApp>) -> serde_json::Map<String, serde_json::Value> {
        h.state().ui.dialogs.first().map(|d| d.fields.clone()).expect("the dialog is open")
    }

    fn set_fields(h: &mut Harness<'static, PhotocraftApp>, f: serde_json::Map<String, serde_json::Value>) {
        h.state_mut().ui.dialogs[0].fields = f;
        h.run_steps(2);
    }

    fn enter(h: &mut Harness<'static, PhotocraftApp>) {
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
    }

    fn created(h: &Harness<'static, PhotocraftApp>) -> (u32, u32, f32) {
        assert!(h.state().ui.dialogs.is_empty(), "the dialog closed");
        let d = &h.state().session.active().expect("a new document").doc;
        (d.size.width, d.size.height, d.resolution_dpi)
    }

    /// Preset cards paint their titles directly, without an accessibility label.
    fn preset_title_at(h: &Harness<'static, PhotocraftApp>, title: &str) -> egui::Pos2 {
        fn find(shape: &egui::Shape, title: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text == title => Some(text.visual_bounding_rect().center()),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, title)),
                _ => None,
            }
        }
        h.output().shapes.iter().find_map(|shape| find(&shape.shape, title)).expect("visible preset title")
    }

    fn second_click_creates_once(h: &mut Harness<'static, PhotocraftApp>, at: egui::Pos2) {
        assert!(h.state().session.documents().is_empty(), "the first click only selects the preset");
        assert_eq!(h.state().ui.dialogs.len(), 1);
        // Two real pointer press/release pairs within egui's double-click interval.
        click_at(h, at);
        h.run_steps(3);
        assert_eq!(h.state().session.documents().len(), 1, "the second click creates exactly one document");
        assert!(h.state().ui.dialogs.is_empty(), "double-click closes the New Document dialog");
        assert!(h.state().ui.views[0].fit_pending, "the normal Create path fits the new document");
    }

    fn assert_pixel(h: &mut Harness<'static, PhotocraftApp>, expected: [f64; 4]) {
        let pixel = h.state_mut().session.execute("document.pixel", serde_json::json!({"x":0,"y":0})).unwrap();
        for (actual, expected) in pixel.as_array().unwrap().iter().zip(expected) {
            assert!((actual.as_f64().unwrap() - expected).abs() < 0.001, "background pixel: {pixel}");
        }
    }

    fn cursor_over(h: &mut Harness<'static, PhotocraftApp>, at: egui::Pos2) -> egui::CursorIcon {
        h.hover_at(at);
        h.run_steps(2);
        h.output().platform_output.cursor_icon
    }

    /// #8331: headings and field labels keep the arrow; only the text fields show the I-beam.
    #[test]
    fn labels_show_the_arrow_and_only_text_fields_the_text_cursor() {
        use egui_kittest::kittest::NodeT;
        let mut h = harness();
        let label_at = |h: &Harness<'static, PhotocraftApp>, text: &str| {
            h.query_all_by_label(text)
                .find(|n| n.accesskit_node().role() == Role::Label)
                .map(|n| n.rect().center())
                .unwrap_or_else(|| panic!("a {text:?} label"))
        };
        for text in ["PRESET DETAILS", "Width", "Height", "Resolution", "Color Mode", "Background Contents"] {
            let at = label_at(&h, text);
            assert_eq!(cursor_over(&mut h, at), egui::CursorIcon::Default, "over {text:?}");
        }
        // The name field still shows the text cursor, so the check above can tell them apart.
        let name = h.query_all_by_role(Role::TextInput).next().map(|n| n.rect().center()).expect("the name field");
        assert_eq!(cursor_over(&mut h, name), egui::CursorIcon::Text, "over the name field");
        // The Saved tab's labels too.
        h.get_by_label("Saved").click();
        h.run_steps(2);
        let at = label_at(&h, "No saved presets yet.");
        assert_eq!(cursor_over(&mut h, at), egui::CursorIcon::Default, "over the empty Saved list");
    }

    #[test]
    fn double_click_builtin_preset_creates_its_size_and_current_details() {
        let mut h = harness();
        let mut f = fields(&h);
        f.extend(serde_json::json!({"name":"Portrait proof","mode":"cmyk","depth":16,"background":"transparent"}).as_object().unwrap().clone());
        set_fields(&mut h, f);
        h.get_by_label("Photo").click();
        h.run_steps(2);
        let at = preset_title_at(&h, "Portrait, 4 x 6");
        click_at(&mut h, at);
        assert_eq!(fields(&h)["__preset"], "Portrait, 4 x 6");
        assert_eq!(
            super::super::command_params(&fields(&h)),
            serde_json::json!({"name":"Portrait proof","width":1200,"height":1800,"resolution":300.0,
                "mode":"cmyk","depth":16,"background":"transparent"})
        );
        second_click_creates_once(&mut h, at);
        assert_eq!(created(&h), (1200, 1800, 300.0));
        let doc = &h.state().session.active().unwrap().doc;
        assert_eq!(doc.name, "Portrait proof");
        assert_eq!((doc.mode, doc.depth), (photocraft_doc::ColorMode::Cmyk, photocraft_doc::SampleType::U16));
        assert_pixel(&mut h, [0.0; 4]);
    }

    /// Framework-style touch: the second tap lands ~48 px off (past egui's 6 px and the old 32 px touch slop) and still creates.
    #[test]
    fn touch_double_tap_with_jitter_creates_from_builtin_preset() {
        let mut h = harness();
        h.get_by_label("Photo").click();
        h.run_steps(2);
        let at = preset_title_at(&h, "Portrait, 4 x 6");
        touch_tap_at(&mut h, at);
        assert_eq!(fields(&h)["__preset"], "Portrait, 4 x 6");
        assert!(h.state().session.documents().is_empty(), "first tap only selects");
        touch_tap_at(&mut h, egui::pos2(at.x + 48.0, at.y));
        assert_eq!(created(&h), (1200, 1800, 300.0));
    }

    /// Mouse keeps egui's tight distance: 20 px between clicks is two singles, not a create.
    #[test]
    fn mouse_double_click_with_large_jitter_does_not_create() {
        let mut h = harness();
        h.get_by_label("Photo").click();
        h.run_steps(2);
        let at = preset_title_at(&h, "Portrait, 4 x 6");
        click_at(&mut h, at);
        click_at(&mut h, at + egui::vec2(20.0, 0.0));
        assert!(h.state().session.documents().is_empty());
        assert_eq!(h.state().ui.dialogs.len(), 1, "dialog stays open");
    }

    #[test]
    fn double_click_clipboard_preset_creates_its_size_and_current_details() {
        let mut h = harness();
        let mut f = fields(&h);
        super::super::set_clipboard(&mut f, 37, 23);
        apply_preset(&mut f, &("Other size", 100, 200, 300.0), &Default::default());
        f.extend(serde_json::json!({"mode":"gray","depth":8,"background":"white"}).as_object().unwrap().clone());
        set_fields(&mut h, f);
        let at = preset_title_at(&h, super::super::CLIPBOARD);
        click_at(&mut h, at);
        assert_eq!(fields(&h)["__preset"], super::super::CLIPBOARD);
        assert_eq!(
            super::super::command_params(&fields(&h)),
            serde_json::json!({"name":"Untitled-1","width":37,"height":23,"resolution":72.0,
                "mode":"gray","depth":8,"background":"white"})
        );
        second_click_creates_once(&mut h, at);
        assert_eq!(created(&h), (37, 23, 72.0));
        let doc = &h.state().session.active().unwrap().doc;
        assert_eq!((doc.mode, doc.depth), (photocraft_doc::ColorMode::Grayscale, photocraft_doc::SampleType::U8));
        assert_pixel(&mut h, [1.0; 4]);
    }

    #[test]
    fn double_click_saved_preset_restores_all_settings_and_captured_background() {
        let mut h = harness();
        h.state_mut()
            .run(
                "document.presets.save",
                serde_json::json!({"name":"Saved proof","settings":{
                    "width":48,"height":32,"resolution":254.0,"mode":"rgb","depth":32,
                    "background":"backgroundColor","backgroundColor":[0.125,0.375,0.625],"unit":"mm","resolutionUnit":"cm"
                }}),
            )
            .unwrap();
        h.state_mut().session.tools.background = [1.0, 0.0, 1.0, 1.0];
        let mut f = fields(&h);
        f.insert("name".into(), serde_json::json!("Current document name"));
        set_fields(&mut h, f);
        h.get_by_label("Saved").click();
        h.run_steps(2);
        let at = h.get_by_label("Saved proof").rect().center();
        click_at(&mut h, at);
        assert_eq!(fields(&h)["__savedPreset"], "Saved proof");
        assert_eq!(fields(&h)["__unit"], "mm");
        assert_eq!(fields(&h)["__resUnit"], "cm");
        second_click_creates_once(&mut h, at);
        assert_eq!(created(&h), (48, 32, 254.0));
        let doc = &h.state().session.active().unwrap().doc;
        assert_eq!(doc.name, "Current document name");
        assert_eq!((doc.mode, doc.depth), (photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::F32));
        assert_pixel(&mut h, [0.125, 0.375, 0.625, 1.0]);
    }

    #[test]
    fn preset_cards_use_the_preset_resolution_preferences() {
        let mut h = harness();
        h.state_mut().run("prefs.set", serde_json::json!({"path": "unitsAndRulers.printResolution", "value": 150.0})).unwrap();
        h.state_mut().run("prefs.set", serde_json::json!({"path": "unitsAndRulers.screenResolution", "value": 96.0})).unwrap();
        h.get_by_label("Photo").click();
        h.run_steps(2);
        let at = preset_title_at(&h, "Landscape, 6 x 4");
        click_at(&mut h, at);
        assert_eq!(fields(&h)["__unit"], "in");
        second_click_creates_once(&mut h, at);
        assert_eq!(created(&h), (900, 600, 150.0), "6 x 4 in at 150 ppi");

        h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
        h.run_steps(2);
        h.get_by_label("Web").click();
        h.run_steps(2);
        let at = preset_title_at(&h, "Web Minimum");
        click_at(&mut h, at);
        enter(&mut h);
        assert_eq!(created(&h), (1024, 768, 96.0));
    }

    #[test]
    fn arithmetic_dimensions_create_the_evaluated_size() {
        let mut h = harness();
        type_into(&mut h, 0, "1920/2");
        type_into(&mut h, 1, "(100+50)*2");
        enter(&mut h);
        assert_eq!(created(&h), (960, 300, 72.0));
    }

    #[test]
    fn typed_size_then_enter_creates_that_size() {
        let mut h = harness();
        type_into(&mut h, 0, "512");
        type_into(&mut h, 1, "512");
        let f = fields(&h);
        assert_eq!((f["width"].as_u64(), f["height"].as_u64()), (Some(512), Some(512)), "whole pixels: {f:?}");
        enter(&mut h);
        assert_eq!(created(&h), (512, 512, 72.0));
    }

    #[test]
    fn typed_size_then_create_without_leaving_the_field_creates_that_size() {
        let mut h = harness();
        type_into(&mut h, 0, "512");
        type_into(&mut h, 1, "300");
        // Still editing Height: click Create straight away.
        let create = h.get_by_label("Create").rect();
        click_at(&mut h, create.center());
        assert_eq!(created(&h), (512, 300, 72.0));
    }

    #[test]
    fn typing_over_a_preset_wins() {
        let mut h = harness();
        let mut f = fields(&h);
        let web = CATEGORIES.iter().find(|c| c.0 == "Web").unwrap().1.iter().find(|p| p.0 == "Web Minimum").unwrap();
        apply_preset(&mut f, web, &Default::default());
        set_fields(&mut h, f);
        type_into(&mut h, 0, "512");
        type_into(&mut h, 1, "512");
        assert!(fields(&h).get("__preset").is_none(), "typing deselects the preset");
        enter(&mut h);
        assert_eq!(created(&h), (512, 512, 72.0));
    }

    #[test]
    fn the_clipboard_card_is_first_and_creates_the_clipboard_size() {
        let mut h = harness();
        let mut f = fields(&h);
        super::super::set_clipboard(&mut f, 640, 360);
        set_fields(&mut h, f);
        // Recent lists the Clipboard card first: three presets instead of two.
        assert!(h.query_by_label_contains("BLANK DOCUMENT PRESETS (3)").is_some());
        let heading = h.get_by_label_contains("BLANK DOCUMENT PRESETS").rect();
        // Pick the second card, then the first (Clipboard) again.
        click_at(&mut h, heading.left_bottom() + egui::vec2(80.0 + 172.0, 60.0));
        assert_eq!(fields(&h).get("__preset").and_then(|v| v.as_str()), Some("Default PhotoCraft Size"));
        click_at(&mut h, heading.left_bottom() + egui::vec2(80.0, 60.0));
        assert_eq!(fields(&h).get("__preset").and_then(|v| v.as_str()), Some(super::super::CLIPBOARD));
        enter(&mut h);
        assert_eq!(created(&h), (640, 360, 72.0));
    }

    #[test]
    fn clicking_a_preset_card_after_typing_sets_its_size() {
        let mut h = harness();
        type_into(&mut h, 0, "512");
        h.get_by_label("Photo").click();
        h.run_steps(2);
        // The first card ("Landscape, 6 x 4") sits under the presets heading.
        let heading = h.get_by_label_contains("BLANK DOCUMENT PRESETS").rect();
        click_at(&mut h, heading.left_bottom() + egui::vec2(80.0, 60.0));
        assert_eq!(fields(&h).get("__preset").and_then(|v| v.as_str()), Some("Landscape, 6 x 4"));
        enter(&mut h);
        assert_eq!(created(&h), (1800, 1200, 300.0));
    }

    #[test]
    fn typed_size_in_inches_converts_at_the_resolution() {
        let mut h = harness();
        type_into(&mut h, 2, "300");
        let mut f = fields(&h);
        f.insert("__unit".into(), serde_json::json!("in"));
        set_fields(&mut h, f);
        type_into(&mut h, 0, "2");
        type_into(&mut h, 1, "1.5");
        enter(&mut h);
        assert_eq!(created(&h), (600, 450, 300.0));
    }

    /// #1147: typed digit by digit, millimetres used to be rewritten after each keystroke
    /// (`8` became `8.1`), so 841 never arrived.
    #[test]
    fn typed_size_in_millimetres_keeps_every_digit() {
        let mut h = harness();
        let mut f = fields(&h);
        f.insert("__unit".into(), serde_json::json!("mm"));
        set_fields(&mut h, f);
        type_into(&mut h, 0, "841");
        type_into(&mut h, 1, "1189");
        enter(&mut h);
        assert_eq!(created(&h), (2384, 3370, 72.0));
    }

    #[test]
    fn changing_units_keeps_the_typed_pixel_size() {
        let mut h = harness();
        type_into(&mut h, 0, "512");
        type_into(&mut h, 1, "512");
        for unit in ["in", "cm", "mm", "pt", "pica", "px"] {
            let mut f = fields(&h);
            f.insert("__unit".into(), serde_json::json!(unit));
            set_fields(&mut h, f);
        }
        enter(&mut h);
        assert_eq!(created(&h), (512, 512, 72.0));
    }

    #[test]
    fn orientation_swap_keeps_whole_pixels() {
        let mut h = harness();
        type_into(&mut h, 0, "512");
        type_into(&mut h, 1, "256");
        // The Portrait icon button follows the "Orientation" label (icons have tooltips only).
        let label = h.get_by_label("Orientation").rect();
        let gap = h.ctx.global_style().spacing.item_spacing.x;
        click_at(&mut h, egui::pos2(label.right() + gap + 12.0, label.center().y));
        enter(&mut h);
        assert_eq!(created(&h), (256, 512, 72.0));
    }

    /// #4668: once an orientation is picked, the presets turn to it, on the cards and when applied.
    #[test]
    fn picked_orientation_turns_the_presets() {
        let mut h = harness();
        h.get_by_label("Print").click();
        h.run_steps(2);
        // The default 1920 x 1080 is already landscape: Landscape swaps nothing, but is picked.
        let label = h.get_by_label("Orientation").rect();
        let gap = h.ctx.global_style().spacing.item_spacing.x;
        click_at(&mut h, egui::pos2(label.right() + gap + 24.0 + gap + 12.0, label.center().y));
        assert_eq!((fields(&h)["width"].as_u64(), fields(&h)["height"].as_u64()), (Some(1920), Some(1080)));
        let _ = preset_title_at(&h, "11.7 x 8.3 in @ 300 ppi");
        let at = preset_title_at(&h, "A4");
        click_at(&mut h, at);
        enter(&mut h);
        assert_eq!(created(&h), (3508, 2480, 300.0));
    }

    /// #4668: a physical unit picked for Width/Height is the one print presets show and keep.
    #[test]
    fn picked_physical_unit_is_the_print_presets_unit() {
        let mut h = harness();
        let mut f = fields(&h);
        f.insert("__unit".into(), serde_json::json!("mm"));
        set_fields(&mut h, f);
        h.get_by_label("Print").click();
        h.run_steps(2);
        let _ = preset_title_at(&h, "210 x 297 mm @ 300 ppi");
        let at = preset_title_at(&h, "A4");
        click_at(&mut h, at);
        assert_eq!(fields(&h)["__unit"], "mm");
        // Screen presets stay in pixels.
        h.get_by_label("Web").click();
        h.run_steps(2);
        let _ = preset_title_at(&h, "1024 x 768 px @ 72 ppi");
        enter(&mut h);
        assert_eq!(created(&h), (2480, 3508, 300.0));
    }

    #[test]
    fn hovering_preset_tabs_shows_pointing_hand_not_text_cursor() {
        let mut h = harness();
        for tab in ["Recent", "Saved", "Photo", "Print", "Art & Illustration", "Web", "Mobile", "Film & Video"] {
            let rect = h.get_by_label(tab).rect();
            h.hover_at(rect.center());
            h.run_steps(2);
            assert_eq!(
                h.output().platform_output.cursor_icon,
                egui::CursorIcon::PointingHand,
                "hovering preset tab {tab:?} must show PointingHand instead of text caret"
            );
        }
    }
}
