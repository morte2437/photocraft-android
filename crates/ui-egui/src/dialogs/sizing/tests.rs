use super::*;

#[test]
fn unit_conversions_round_trip() {
    for ppi in [72.0, 72.27] {
        for unit in ["px", "percent", "in", "cm", "mm", "pt", "pica"] {
            let v = to_unit(1500.0, unit, 3000.0, 300.0, ppi);
            assert!((from_unit(v, unit, 3000.0, 300.0, ppi) - 1500.0).abs() < 1e-9, "{unit} at {ppi} pt/in");
        }
    }
    assert_eq!(to_unit(1500.0, "percent", 3000.0, 300.0, 72.0), 50.0);
    assert_eq!(to_unit(600.0, "in", 3000.0, 300.0, 72.0), 2.0);
    assert_eq!(human_bytes(6016.0 * 6016.0 * 4.0), "138.1M");
}

/// #5756: points and picas use the Point/Pica Size preference (Traditional is 72.27 pt/in),
/// and a dialog without the field (or with a bad one) falls back to PostScript's 72.
#[test]
fn points_and_picas_use_the_points_per_inch() {
    assert!((to_unit(72.0, "pt", 72.0, 72.0, 72.27) - 72.27).abs() < 1e-9);
    assert!((to_unit(72.0, "pica", 72.0, 72.0, 72.27) - 72.27 / 12.0).abs() < 1e-9);
    assert_eq!(to_unit(72.0, "pt", 72.0, 72.0, 72.0), 72.0);
    assert_eq!(to_unit(72.0, "pica", 72.0, 72.0, 72.0), 6.0);
    assert!((from_unit(72.27, "pt", 72.0, 72.0, 72.27) - 72.0).abs() < 1e-9);
    let mut f = Map::new();
    assert_eq!(pt_per_in(&f), 72.0);
    for bad in [json!(0.0), json!(-3.0), json!("x")] {
        f.insert("__ptPerIn".into(), bad);
        assert_eq!(pt_per_in(&f), 72.0);
    }
    f.insert("__ptPerIn".into(), json!(72.27));
    assert_eq!(pt_per_in(&f), 72.27);
}

#[test]
fn dialog_maps_to_engine_command() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    let id = open(&mut app, "image.imageSize").unwrap();
    let d = app.ui.dialog_mut(id).unwrap();
    d.fields.insert("width".into(), json!(100));
    d.fields.insert("height".into(), json!(50));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.active().unwrap().doc.size, photocraft_doc::Size::new(100, 50));
    let id = open(&mut app, "image.canvasSize").unwrap();
    let d = app.ui.dialog_mut(id).unwrap();
    d.fields.insert("width".into(), json!(120));
    d.fields.insert("anchor".into(), json!("topLeft"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.active().unwrap().doc.size, photocraft_doc::Size::new(120, 50));
}

#[test]
fn alt_reset_restores_open_dimensions() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 100, "resolution": 72})).unwrap();
    let id = open(&mut app, "image.imageSize").unwrap();
    let d = app.ui.dialog_mut(id).unwrap();
    d.fields.insert("width".into(), json!(50));
    d.fields.insert("height".into(), json!(25));
    d.fields.insert("resolution".into(), json!(300));
    d.fields.insert("__constrain".into(), json!(false));
    reset(&mut d.fields);
    assert_eq!((num(&d.fields, "width"), num(&d.fields, "height"), num(&d.fields, "resolution")), (200.0, 100.0, 72.0));
    assert_eq!(d.fields.get("__constrain"), Some(&json!(true)));
    assert!(app.ui.dialogs.iter().any(|x| x.id == id), "reset leaves the dialog open");
}

#[test]
fn canvas_extension_dropdown_is_named_for_its_setting() {
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 100, "height": 50})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    open(h.state_mut(), "image.canvasSize").unwrap();
    h.run_steps(3);
    let labels: Vec<String> = h.query_all_by_role(Role::ComboBox).filter_map(|n| n.accesskit_node().label()).collect();
    assert!(labels.iter().any(|label| label == "Canvas extension color:"), "{labels:?}");
    assert!(!labels.iter().any(|label| label == "Size"), "the dialog's Size label names a different field: {labels:?}");
}

/// #6019: on an Indexed Color document the dialog opens on, and offers only, Automatic and Nearest Neighbor.
#[test]
fn indexed_color_resamples_with_nearest_neighbor_only() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.session.execute("image.mode.indexedColor", json!({})).unwrap();
    for (pref, method) in [("bicubic", "nearest"), ("bicubicAutomatic", "automatic"), ("nearestNeighbor", "nearest")] {
        app.session.execute("prefs.set", json!({"path": "general.imageInterpolation", "value": pref})).unwrap();
        let id = open(&mut app, "image.imageSize").unwrap();
        let f = &app.ui.dialog_mut(id).unwrap().fields;
        assert_eq!(f.get("resample"), Some(&json!(method)), "{pref}");
        assert_eq!(f.get("__indexed"), Some(&json!(true)));
        app.ui.close_dialog(id);
    }
    // The open list offers only Automatic and Nearest Neighbor.
    use egui_kittest::{Harness, kittest::Queryable};
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    open(h.state_mut(), "image.imageSize").unwrap();
    h.run_steps(3);
    let combo = h.query_all_by_role(egui::accesskit::Role::ComboBox).find(|n| n.value().as_deref() == Some("Nearest Neighbor (hard edges)"));
    combo.expect("the Resample list shows Nearest Neighbor").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("Automatic").is_some(), "Automatic is offered");
    for gone in ["Bicubic", "Bilinear", "Lanczos", "Preserve Details"] {
        assert!(h.query_by_label_contains(gone).is_none(), "{gone} is not offered for Indexed Color");
    }
}

/// The dialog starts on Preferences › General › Image Interpolation: Bicubic Automatic opens
/// as Automatic and the others as themselves (Bicubic Sharper used to open as Lanczos).
#[test]
fn resample_follows_the_interpolation_preference() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    for (pref, method) in
        [("bicubicAutomatic", "automatic"), ("bicubicSharper", "bicubicSharper"), ("bicubicSmoother", "bicubicSmoother"), ("nearestNeighbor", "nearest")]
    {
        app.session.execute("prefs.set", json!({"path": "general.imageInterpolation", "value": pref})).unwrap();
        let id = open(&mut app, "image.imageSize").unwrap();
        let f = &app.ui.dialog_mut(id).unwrap().fields;
        assert_eq!((f.get("resample"), f.get("__resample")), (Some(&json!(method)), Some(&json!(method))), "{pref}");
        app.ui.close_dialog(id);
    }
    let id = open(&mut app, "image.imageSize").unwrap();
    let d = app.ui.dialog_mut(id).unwrap();
    d.fields.insert("width".into(), json!(50));
    d.fields.insert("height".into(), json!(25));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.active().unwrap().doc.size, photocraft_doc::Size::new(50, 25));
}

/// #441: a typed zero width showed "0 px × 1 px" while OK resized to 1×1.
#[test]
fn typed_zero_width_shows_the_applied_size() {
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = open(h.state_mut(), "image.imageSize").unwrap();
    h.run_steps(3);
    // The dialog opens on Width, selected (#2329): the typed 0 replaces the 4.
    assert!(h.query_all_by_role(Role::SpinButton).next().expect("the Width field").is_focused());
    h.event(egui::Event::Text("0".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    let f = h.state().ui.dialogs.first().map(|d| d.fields.clone()).expect("the dialog is open");
    assert_eq!((num(&f, "width"), num(&f, "height")), (1.0, 1.0), "{f:?}");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(h.state().session.active().unwrap().doc.size, photocraft_doc::Size::new(1, 1));
}

/// An Image Size dialog for a 100 x 50 px, 72 ppi document with Resample off, shown in `unit`.
fn resample_off_dialog(unit: &str) -> (egui_kittest::Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 100, "height": 50, "resolution": 72})).unwrap();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = open(h.state_mut(), "image.imageSize").unwrap();
    let d = h.state_mut().ui.dialog_mut(id).unwrap();
    d.fields.insert("resample".into(), json!("none"));
    d.fields.insert("__unit".into(), json!(unit));
    h.run_steps(3);
    (h, id)
}

/// #5756: points and picas follow Preferences › Units & Rulers › Point/Pica Size. With
/// Traditional (72.27 pt/in) and Resample off, 100 px typed as 72.27 pt wide is 100 ppi.
#[test]
fn traditional_point_size_drives_points_and_picas() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 100, "height": 50, "resolution": 72})).unwrap();
    app.session.execute("prefs.set", json!({"path": "unitsAndRulers.pointSize", "value": "traditional"})).unwrap();
    for command in ["image.imageSize", "image.canvasSize"] {
        let id = open(&mut app, command).unwrap();
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields.get("__ptPerIn"), Some(&json!(72.27)), "{command}");
        app.ui.close_dialog(id);
    }
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let id = open(h.state_mut(), "image.imageSize").unwrap();
    let d = h.state_mut().ui.dialog_mut(id).unwrap();
    d.fields.insert("resample".into(), json!("none"));
    d.fields.insert("__unit".into(), json!("pt"));
    h.run_steps(3);
    type_width(&mut h, "72.27");
    let f = fields(&h);
    assert_eq!((num(&f, "width"), num(&f, "height")), (100.0, 50.0), "{f:?}");
    assert!((num(&f, "resolution") - 100.0).abs() < 1e-2, "{f:?}");
}

/// Types `text` into the Width field and leaves it with Tab.
fn type_width(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, text: &str) {
    type_into(h, 0, text);
}

/// Types `text` into the `n`th number field (Width, Height, Resolution) and leaves it with Tab.
fn type_into(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, n: usize, text: &str) {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let width = h.query_all_by_role(Role::SpinButton).nth(n).map(|n| n.rect()).expect("the number field");
    h.hover_at(width.center());
    h.run_steps(1);
    h.drag_at(width.center());
    h.run_steps(1);
    h.drop_at(width.center());
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn fields(h: &egui_kittest::Harness<'static, PhotocraftApp>) -> Map<String, Value> {
    h.state().ui.dialogs.first().map(|d| d.fields.clone()).expect("the dialog is open")
}

/// #2393: with Resample off, a pixel Width can't be edited and the readouts keep showing the
/// pixels OK keeps (they showed "200 px × 50 px" and "39K (was 20K)").
#[test]
fn resample_off_keeps_pixel_dimensions_in_the_dialog() {
    use egui_kittest::kittest::Queryable;
    let (mut h, id) = resample_off_dialog("px");
    type_width(&mut h, "200");
    let f = fields(&h);
    assert_eq!((num(&f, "width"), num(&f, "height"), num(&f, "resolution")), (100.0, 50.0, 72.0), "{f:?}");
    assert!(h.query_by_label("100 px × 50 px").is_some(), "the Dimensions line shows the kept pixels");
    assert!(h.query_by_label_contains("20K (was 20K)").is_some(), "the Image Size estimate is unchanged");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!((doc.size, doc.resolution_dpi), (photocraft_doc::Size::new(100, 50), 72.0));
}

/// With Resample off, a physical Width changes the resolution and keeps the pixels (Photoshop):
/// 100 px at 72 ppi is 1.39 in; 2 in wide is 50 ppi.
#[test]
fn resample_off_physical_width_changes_the_resolution() {
    let (mut h, id) = resample_off_dialog("in");
    type_width(&mut h, "2");
    let f = fields(&h);
    assert_eq!((num(&f, "width"), num(&f, "height")), (100.0, 50.0), "{f:?}");
    assert!((num(&f, "resolution") - 50.0).abs() < 1e-3, "{f:?}");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(doc.size, photocraft_doc::Size::new(100, 50));
    assert!((doc.resolution_dpi - 50.0).abs() < 1e-3, "{}", doc.resolution_dpi);
}

/// #6979: with Resample on, a new Resolution keeps the print size in a physical unit and
/// resamples the pixels (Photoshop): 720 x 576 px at 72 ppi is 10 x 8 in, so 300 ppi is
/// 3000 x 2400 px. In pixels the pixel count stays.
#[test]
fn resample_on_resolution_keeps_the_physical_size() {
    for (unit, size) in [("in", (3000.0, 2400.0)), ("cm", (3000.0, 2400.0)), ("px", (720.0, 576.0))] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 720, "height": 576, "resolution": 72})).unwrap();
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1400.0, 900.0))
            .build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let id = open(h.state_mut(), "image.imageSize").unwrap();
        let d = h.state_mut().ui.dialog_mut(id).unwrap();
        d.fields.insert("resample".into(), json!("bicubic"));
        d.fields.insert("__unit".into(), json!(unit));
        h.run_steps(3);
        type_into(&mut h, 2, "300");
        let f = fields(&h);
        assert_eq!((num(&f, "width"), num(&f, "height"), num(&f, "resolution")), (size.0, size.1, 300.0), "{unit}: {f:?}");
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        let doc = &h.state().session.active().unwrap().doc;
        assert_eq!((doc.size, doc.resolution_dpi), (photocraft_doc::Size::new(size.0 as u32, size.1 as u32), 300.0), "{unit}");
    }
}

/// #4162: a document wider than 30000 units in a physical unit (40000 px at 72 ppi is 40000 pt)
/// kept its size in the fields and on OK; the fields clamped it to 30000 on open, so OK with
/// no edit resized (Image Size) or cropped (Canvas Size) it.
#[test]
fn large_documents_in_physical_units_open_unchanged() {
    use egui_kittest::Harness;
    let mut wrong = Vec::new();
    for command in ["image.imageSize", "image.canvasSize"] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 40000, "height": 4, "resolution": 72})).unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let id = open(h.state_mut(), command).unwrap();
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("__unit".into(), json!("pt"));
        h.run_steps(3);
        let f = fields(&h);
        let shown = (num(&f, "width"), num(&f, "height"));
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        let size = h.state().session.active().unwrap().doc.size;
        if shown != (40000.0, 4.0) || size != photocraft_doc::Size::new(40000, 4) {
            wrong.push(format!("{command}: fields {shown:?}, document {size:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// #2329: Image Size and Canvas Size open on Width with its value selected, as in Photoshop:
/// typing replaces it and Enter applies it.
#[test]
fn sizing_dialogs_open_on_width_selected() {
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};
    for command in ["image.imageSize", "image.canvasSize"] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        open(h.state_mut(), command).unwrap();
        h.run_steps(3);
        assert!(h.query_all_by_role(Role::SpinButton).next().expect("the Width field").is_focused(), "{command}: Width has focus");
        h.event(egui::Event::Text("50".into()));
        h.run_steps(1);
        let f = h.state().ui.dialogs.first().map(|d| d.fields.clone()).expect("the dialog is open");
        assert_eq!(num(&f, "width"), 50.0, "{command}: typing replaced the selected 200");
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty(), "{command}: Enter is OK");
        assert_eq!(h.state().session.active().unwrap().doc.size.width, 50, "{command}");
    }
}
