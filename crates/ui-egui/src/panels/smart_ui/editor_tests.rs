use egui::{Modifiers, PointerButton, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::{
    PhotocraftApp,
    smart_filter_dialog::tests::{app, smart},
};

/// The app with a recorded Smart Object, in a `width`-point dock and the given theme, ready to click.
fn harness(width: f32, scale: f32, theme: crate::theme::ThemeKind) -> Harness<'static, PhotocraftApp> {
    let (app, _) = app(8, "rgb");
    let mut h =
        Harness::builder().with_size(vec2(1200.0, 850.0)).with_pixels_per_point(scale).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, theme);
            app
        });
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new(
        "ui.set",
        json!({"theme":theme.id(),"dockWidth":width,"dock":{"collapsed":["color","properties","history","navigator"]}}),
    );
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(5);
    h
}

/// Click `at` with the primary button, hovering first so the row reacts.
fn click(h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// A filter row's name and blend icon are separate double-click targets, side by side without overlap, in every theme and dock width.
#[test]
fn names_and_blend_icons_have_separate_double_click_targets_and_no_overlap() {
    for theme in crate::theme::ThemeKind::ALL {
        for width in [250.0, 420.0] {
            let mut h = harness(width, 1.0, theme);
            let layer = h.state().session.active().unwrap().active_layer.unwrap();
            let rows = crate::layer_row_ui::recorded(&h.ctx);
            let row = rows.iter().find(|r| r.layer == layer.0).unwrap();
            assert!(row.indicators.iter().any(|(k, _)| *k == crate::layer_row_ui::Indicator::SmartFilters));
            let blend = h.get_by_label("Blending Options (Box Blur)").rect();
            let edit = h.get_by_label("Box Blur").rect();
            assert!(edit.right() <= blend.left(), "{width} {theme:?}: {edit:?} {blend:?}");
            click(&mut h, blend.center());
            assert!(h.state().ui.dialogs.is_empty(), "single click does not open blending");
            click(&mut h, blend.center());
            assert!(crate::smart_filter_dialog::owns(&h.state().ui.dialogs.last().unwrap().fields));
            assert_eq!(h.state().ui.dialogs.last().unwrap().fields["__smartFilter"]["index"], json!(0));
            let id = h.state().ui.dialogs.last().unwrap().id;
            crate::dialogs::cancel(h.state_mut(), id).unwrap();
            // Start a new gesture after egui's triple-click interval, as native users do.
            for _ in 0..40 {
                h.step();
            }
            click(&mut h, edit.center());
            click(&mut h, edit.center());
            assert_eq!(h.state().ui.dialogs.last().unwrap().fields["__command"], json!("filter.blur.boxBlur"));
            assert_eq!(h.state().ui.dialogs.last().unwrap().fields["radius"].as_f64(), Some(4.0));
        }
    }
}

/// The effects disclosure hides the effects and Smart Filters stacks together, as a view change only.
#[test]
fn disclosure_hides_both_stacks_without_changing_pixels_history_or_selection() {
    let mut h = harness(290.0, 1.5, Default::default());
    let layer = h.state().session.active().unwrap().active_layer.unwrap();
    h.state_mut().run("layer.layerStyle.dropShadow", json!({"layer":layer.0})).unwrap();
    h.state_mut().run("layer.new.layer", json!({"name":"Selected elsewhere"})).unwrap();
    h.run_steps(5);
    let before = h.state().session.active().unwrap().doc.clone();
    let revision = h.state().session.active().unwrap().revision;
    let steps = h.state().session.active().unwrap().history.past_len();
    let active = h.state().session.active().unwrap().active_layer;
    let at = h.get_by_label("Collapse effects Smart Object").rect().center();
    click(&mut h, at);
    assert!(h.state().session.active().unwrap().fx_collapsed.contains(&layer));
    assert!(h.query_by_label("Blending Options (Box Blur)").is_none());
    assert!(h.query_by_label("Drop Shadow").is_none());
    assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.active().unwrap().doc));
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps);
    assert_eq!(h.state().session.active().unwrap().active_layer, active);
    let at = h.get_by_label("Expand effects Smart Object").rect().center();
    click(&mut h, at);
    assert!(h.query_by_label("Blending Options (Box Blur)").is_some());
}

/// An adjustment on a Smart Object reopens its editor with the stored values and OK updates it in one step; Invert has no editor.
#[test]
fn adjustment_editor_reopens_stored_parameters_and_updates_instead_of_appending() {
    let (mut app, layer) = app(8, "rgb");
    app.run("image.adjustments.hueSaturation", json!({"hue":35,"saturation":12,"lightness":-7})).unwrap();
    let ctx = egui::Context::default();
    let sm = smart(&app, layer).clone();
    super::open_editor(&mut app, &ctx, layer.0, &sm, 2);
    let id = app.ui.dialogs.last().unwrap().id;
    let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!(fields["hue"].as_f64(), Some(35.0));
    assert!(crate::adjust_dialog::owns(fields));
    fields.insert("hue".into(), json!(75));
    let idx = app.session.active_index().unwrap();
    assert!(!crate::adjust_preview::on_layer(&mut app, idx));
    assert!(crate::canvas::ensure_filter_preview(&mut app, idx, &ctx).is_some());
    let steps = app.session.active().unwrap().history.past_len();
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(smart(&app, layer).smart_filters.len(), 3);
    assert_eq!(smart(&app, layer).smart_filters[2].params["hue"], json!(75));
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(smart(&app, layer).smart_filters[2].params["hue"], json!(35));
    app.run("image.adjustments.invert", json!({})).unwrap();
    let sm = smart(&app, layer).clone();
    super::open_editor(&mut app, &ctx, layer.0, &sm, 3);
    assert!(app.ui.dialogs.is_empty(), "Invert has no parameter editor");
}

/// Records `command` on a Smart Object in a `mode` document, opens the filter's editor, lets it
/// draw and confirms with no edit. The filter's stored params afterwards.
fn stored_after_confirming_unedited(mode: &str, command: &str, recorded: &serde_json::Value) -> serde_json::Value {
    let (mut app, layer) = app(8, mode);
    app.run(command, recorded.clone()).unwrap();
    let sm = smart(&app, layer).clone();
    let index = sm.smart_filters.len() - 1;
    assert_eq!(sm.smart_filters[index].command, command);
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let ctx = h.ctx.clone();
    super::open_editor(h.state_mut(), &ctx, layer.0, &sm, index);
    let dialog = h.state().ui.dialogs.last().unwrap().id;
    h.run_steps(3);
    crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
    let after = smart(h.state(), layer);
    assert_eq!(after.smart_filters.len(), sm.smart_filters.len(), "{command}: no filter was stacked");
    after.smart_filters[index].params.clone()
}

/// Opening an adjustment smart filter and pressing OK changes nothing it renders, whichever of a
/// setting's equivalent keys it was recorded with: the keys the dialog starts with must not be
/// read over the recorded ones.
#[test]
fn confirming_an_unedited_adjustment_smart_filter_keeps_what_it_renders() {
    use photocraft_doc::ColorMode::{Cmyk, Lab, Rgb};
    use photocraft_engine::adjust_params::{default_for, from_params};
    let dim = json!([[0, 0], [128, 64], [255, 255]]);
    let lift = json!([[0, 40], [255, 255]]);
    let drawn: Vec<f64> = (0..photocraft_doc::adjust::CURVE_MAP_LEN).map(|level| level as f64 * 0.5).collect();
    let jagged: Vec<f64> = (0..photocraft_doc::adjust::CURVE_MAP_LEN).map(|level| if level % 2 == 0 { 40.0 } else { 200.0 }).collect();
    let cases = [
        ("lab", Lab, "curves", json!({"pencil": true, "smooth": 2, "points": jagged})),
        ("rgb", Rgb, "curves", json!({"pencil": true, "smooth": 2, "points": jagged})),
        ("lab", Lab, "curves", json!({"points": dim})),
        ("lab", Lab, "curves", json!({"points": dim, "a": lift})),
        ("lab", Lab, "curves", json!({"lightness": dim})),
        ("lab", Lab, "curves", json!({"pencil": true, "points": drawn})),
        ("lab", Lab, "levels", json!({"inBlack": 30, "gamma": 1.5})),
        ("lab", Lab, "levels", json!({"inWhite": 200, "a": {"inBlack": 12}})),
        ("lab", Lab, "levels", json!({"lightness": {"inBlack": 30}})),
        ("rgb", Rgb, "curves", json!({"points": dim, "red": lift})),
        ("rgb", Rgb, "curves", json!({"gray": dim})),
        ("rgb", Rgb, "curves", json!({"pencil": true, "points": drawn})),
        ("rgb", Rgb, "levels", json!({"inWhite": 200, "blue": {"gamma": 0.8}})),
        ("cmyk", Cmyk, "curves", json!({"points": dim})),
        ("cmyk", Cmyk, "curves", json!({"black": dim})),
        ("cmyk", Cmyk, "levels", json!({"inBlack": 30})),
        ("rgb", Rgb, "photoFilter", json!({"filter": "red"})),
        ("rgb", Rgb, "channelMixer", json!({"monochrome": true})),
        ("rgb", Rgb, "hueSaturation", json!({"colorize": true})),
        ("rgb", Rgb, "brightnessContrast", json!({"brightness": 40})),
    ];
    let mut changed = Vec::new();
    for (mode, color_mode, kind, recorded) in cases {
        let read = |params: &serde_json::Value| from_params(kind, params, None, color_mode).unwrap();
        assert_ne!(read(&recorded), default_for(kind, color_mode).unwrap(), "{mode} {kind} {recorded}: the case must record something");
        let stored = stored_after_confirming_unedited(mode, &format!("image.adjustments.{kind}"), &recorded);
        if read(&stored) != read(&recorded) {
            changed.push(format!("{mode} {kind} {recorded} became {stored}"));
        }
    }
    assert!(changed.is_empty(), "{}", changed.join("\n"));
}

/// The editor's controls show what the filter renders: in Lab the top-level `points` and Levels
/// keys are the Lightness channel, which the dialog edits under `lightness`.
#[test]
fn a_lab_tone_smart_filter_opens_with_its_settings_on_the_lightness_channel() {
    let (mut app, layer) = app(8, "lab");
    let dim = json!([[0.0, 0.0], [128.0, 64.0], [255.0, 255.0]]);
    app.run("image.adjustments.curves", json!({"points": dim})).unwrap();
    app.run("image.adjustments.levels", json!({"inBlack": 30})).unwrap();
    let sm = smart(&app, layer).clone();
    let ctx = egui::Context::default();

    super::open_editor(&mut app, &ctx, layer.0, &sm, 2);
    let dialog = app.ui.dialogs.last().unwrap();
    assert_eq!(dialog.fields["lightness"], dim);
    assert_eq!(dialog.fields["points"], dim, "both keys say the same");
    let id = dialog.id;
    crate::dialogs::cancel(&mut app, id).unwrap();

    super::open_editor(&mut app, &ctx, layer.0, &sm, 3);
    let fields = &app.ui.dialogs.last().unwrap().fields;
    assert_eq!(fields["lightness"]["inBlack"].as_f64(), Some(30.0));
    assert_eq!(fields["inBlack"].as_f64(), Some(30.0), "both keys say the same");
}

/// Settings the engine can't read still open, as recorded, for OK to report on; the dialog and the
/// Smart Filter are left as they are, without a panic.
#[test]
fn an_unreadable_adjustment_smart_filter_opens_with_its_recorded_values() {
    let (mut app, layer) = app(8, "lab");
    app.run("image.adjustments.curves", json!({"points": [[0, 0], [255, 255]]})).unwrap();
    let mut sm = smart(&app, layer).clone();
    sm.smart_filters[2].params = json!({"points": "not a curve"});
    super::open_editor(&mut app, &egui::Context::default(), layer.0, &sm, 2);
    let fields = &app.ui.dialogs.last().unwrap().fields;
    assert_eq!(fields["points"], json!("not a curve"));
    assert!(fields.contains_key("__smartFilter"));
}

/// The Smart Filters mask and the layer mask of one layer are cached as separate thumbnails.
#[test]
fn smart_and_layer_masks_have_separate_thumbnail_cache_entries() {
    let (mut app, layer) = app(8, "rgb");
    app.run("layer.layerMask.revealAll", json!({"layer":layer.0})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let mask = doc.layer(layer).unwrap().mask.as_ref().unwrap();
    let ctx = egui::Context::default();
    let first = app.mask_thumb(&ctx, &doc, layer, mask);
    let mut other = mask.clone();
    other.surface.fill_rect(photocraft_geom::Rect::new(0, 0, 64, 48), &[0.0]);
    let second = app.mask_thumb_kind(&ctx, &doc, layer, &other, crate::mask_thumbs_ui::THUMB_SMART_MASK);
    assert_ne!(first, second);
    assert_eq!(first, app.mask_thumb(&ctx, &doc, layer, mask));
}

/// ⌘I on a Smart Object adds the ordinary Invert as a smart filter, and undo/redo leave the source contents alone.
#[test]
fn ctrl_i_uses_existing_invert_command_and_undo_redo_keep_the_source() {
    let mut h = harness(290.0, 1.0, Default::default());
    let layer = h.state().session.active().unwrap().active_layer.unwrap();
    let before = smart(h.state(), layer).clone();
    let source = photocraft_engine::smart_cmds::contents_of(&h.state().session.active().unwrap().doc, layer).unwrap();
    let steps = h.state().session.active().unwrap().history.past_len();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::I);
    h.run_steps(4);
    let sm = smart(h.state(), layer);
    assert_eq!(sm.smart_filters.len(), 3);
    assert_eq!(sm.smart_filters[2].command, "image.adjustments.invert");
    assert_eq!(photocraft_engine::smart_cmds::contents_of(&h.state().session.active().unwrap().doc, layer).unwrap(), source);
    assert_ne!(sm.cache.as_ref().unwrap().pixel(8, 8), before.cache.as_ref().unwrap().pixel(8, 8));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(smart(h.state(), layer).smart_filters.len(), 2);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(smart(h.state(), layer).smart_filters.len(), 3);
}

/// A recorded filter that lacks a parameter reopens with the schema default for it, not with the
/// value the user last used in that filter's dialog; the parameters it does hold still win.
#[test]
fn filter_editor_starts_missing_parameters_from_defaults_not_remembered_values() {
    let (mut app, layer) = app(8, "rgb");
    let command = "filter.sharpen.smartSharpen";
    app.session.prefs.edit(|p| {
        p.dialogs.insert(command.into(), json!({"reduceNoise": 60, "radius": 9}));
    });
    let ctx = egui::Context::default();
    let mut sm = smart(&app, layer).clone();
    let index = sm.smart_filters.iter().position(|f| f.command == command).unwrap();
    sm.smart_filters[index].params = json!({"amount": 125, "radius": 2});

    super::open_editor(&mut app, &ctx, layer.0, &sm, index);
    let fields = &app.ui.dialogs.last().unwrap().fields;
    assert!(fields.contains_key("__smartFilter"), "it edits the recorded filter");
    assert_eq!(fields["reduceNoise"].as_f64(), Some(10.0), "the spec default, not the remembered 60");
    assert_eq!(fields["amount"].as_f64(), Some(125.0));
    assert_eq!(fields["radius"].as_f64(), Some(2.0), "recorded values beat the remembered 9");

    // A plain Filter menu dialog still starts from the remembered values.
    let id = crate::filter_dialog::open(&mut app, command).unwrap();
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["reduceNoise"].as_f64(), Some(60.0));
}

/// A smart object in a `mode` document with `command` recorded as its one smart filter, and its layer id.
fn adjustment_filter(mode: &str, command: &str, params: serde_json::Value) -> (PhotocraftApp, u64) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "mode": mode})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "black"})).unwrap();
    let id = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
    s.execute(command, params).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.background_jobs = false;
    (app, id)
}

/// The smart object on layer `id`.
fn recorded(app: &PhotocraftApp, id: u64) -> photocraft_doc::SmartObject {
    match &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
        photocraft_doc::LayerContent::Smart(sm) => sm.clone(),
        other => panic!("expected a smart object, got {}", other.kind_name()),
    }
}

/// The app in a harness with the editor of the layer's first smart filter open, and the dialog's id.
fn edit_first_filter(app: PhotocraftApp, id: u64) -> (Harness<'static, PhotocraftApp>, u64) {
    let sm = recorded(&app, id);
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let ctx = h.ctx.clone();
    super::open_editor(h.state_mut(), &ctx, id, &sm, 0);
    let dialog = h.state().ui.dialogs.last().unwrap().id;
    h.run_steps(3);
    (h, dialog)
}

/// Monochrome off removes `gray` from the dialog's values; a stored one read over the Red row
/// would keep the old mix, whatever the Red sliders say, on OK and on every later edit.
#[test]
fn a_channel_mixer_smart_filter_turned_off_monochrome_keeps_its_red_row() {
    let (app, id) = adjustment_filter("rgb", "image.adjustments.channelMixer", json!({"monochrome": true, "gray": [30, 30, 40, 0]}));
    let (mut h, dialog) = edit_first_filter(app, id);
    h.get_by_label("Monochrome").click();
    h.run_steps(3);
    let fields = &mut h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    assert_eq!(fields.get("monochrome"), Some(&json!(false)));
    assert!(!fields.contains_key("gray"), "the editor removed the gray row: {fields:?}");
    fields.insert("red".into(), json!([70.0, 30.0, 0.0, 0.0]));
    h.run_steps(2);
    crate::dialogs::confirm(h.state_mut(), dialog).unwrap();

    let sm = recorded(h.state(), id);
    assert_eq!(sm.smart_filters.len(), 1, "no filter was stacked");
    let adjustment = photocraft_engine::adjust_params::from_params("channelMixer", &sm.smart_filters[0].params, None, photocraft_doc::ColorMode::Rgb).unwrap();
    let photocraft_doc::Adjustment::ChannelMixer { matrix, monochrome } = adjustment else { panic!("Channel Mixer params") };
    assert!(!monochrome);
    assert!(matrix[0].iter().zip([0.7, 0.3, 0.0, 0.0]).all(|(a, b)| (a - b).abs() < 1e-5), "the edited Red row renders: {:?}", matrix[0]);
}

/// In Lab, top-level `points` aliases Lightness and the editor removes it on an edit. A stored
/// one must not override the edit: the Lightness the editor sends is what the filter renders.
#[test]
fn a_lab_curves_smart_filter_keeps_a_lightness_edit() {
    let old = json!([[0, 0], [128, 64], [255, 255]]);
    let (app, id) = adjustment_filter("lab", "image.adjustments.curves", json!({"points": old, "lightness": old}));
    let (mut h, dialog) = edit_first_filter(app, id);
    // Add a point on the Lightness curve, the channel the dialog shows first.
    let graph = h.ctx.data(|d| d.get_temp::<egui::Rect>(crate::adjust_dialog::dialog_mem("curves").with("curves-graph"))).unwrap();
    click(&mut h, egui::pos2(graph.left() + graph.width() * 0.75, graph.top() + graph.height() * 0.1));
    let fields = &h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    assert!(!fields.contains_key("points"), "the editor removed the alias: {fields:?}");
    let edited = fields["lightness"].clone();
    assert_ne!(edited, old, "the click edited Lightness");
    crate::dialogs::confirm(h.state_mut(), dialog).unwrap();

    let sm = recorded(h.state(), id);
    assert_eq!(sm.smart_filters.len(), 1, "no filter was stacked");
    let read = |params: &serde_json::Value| photocraft_engine::adjust_params::from_params("curves", params, None, photocraft_doc::ColorMode::Lab).unwrap();
    let photocraft_doc::Adjustment::Curves { per_channel, .. } = read(&sm.smart_filters[0].params) else { panic!("Curves params") };
    let photocraft_doc::Adjustment::Curves { per_channel: wanted, .. } = read(&json!({"lightness": edited})) else { panic!("Curves params") };
    assert_eq!(per_channel[0], wanted[0], "the edited Lightness renders");
    assert!(sm.smart_filters[0].params.get("points").is_none(), "the removed alias is not stored");
}

/// Emptying a text field removes its key from the Color Lookup dialog, which draws from the
/// command's schema. A stored `data` would be read over the `lut` chosen instead.
#[test]
fn a_color_lookup_smart_filter_with_its_data_cleared_uses_the_chosen_lut() {
    let mut cube = String::from("LUT_3D_SIZE 2\n");
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                cube.push_str(&format!("{} {} {}\n", 1 - r, 1 - g, 1 - b));
            }
        }
    }
    let (app, id) = adjustment_filter("rgb", "image.adjustments.colorLookup", json!({"data": cube, "fileName": "invert.cube"}));
    let (mut h, dialog) = edit_first_filter(app, id);
    let fields = &mut h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    assert_eq!(fields.get("data"), Some(&json!(cube)), "opens on the recorded settings");
    fields.insert("data".into(), json!(""));
    fields.insert("lut".into(), json!("warm"));
    h.run_steps(2);
    let fields = &h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    assert!(!fields.contains_key("data"), "the dialog removed the emptied field");
    crate::dialogs::confirm(h.state_mut(), dialog).unwrap();

    let sm = recorded(h.state(), id);
    assert_eq!(sm.smart_filters.len(), 1, "no filter was stacked");
    let params = &sm.smart_filters[0].params;
    assert!(params.get("data").is_none(), "the removed data is not stored: {params}");
    assert_eq!(params.get("lut"), Some(&json!("warm")));
}
