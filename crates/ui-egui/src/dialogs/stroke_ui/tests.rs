use serde_json::json;

use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "transparent"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
    app
}

fn pixel(app: &PhotocraftApp, x: i32, y: i32) -> [f32; 4] {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

/// Dialog entrypoints preserve target refusals, while parameterized Stroke calls execute directly.
#[test]
fn menu_and_canvas_invocations_open_options_but_explicit_params_run_directly() {
    let mut app = app();
    let ctx = egui::Context::default();
    let opened = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    assert!(opened["dialog"].is_u64());
    assert_eq!(app.ui.dialogs.len(), 1);
    assert!(owns(&app.ui.dialogs[0].fields));
    assert_eq!(app.session.active().unwrap().history.past_len(), 0);
    app.ui.close_dialog(opened["dialog"].as_u64().unwrap());
    let id = crate::dialogs::open_command_dialog(&mut app, COMMAND, "Stroke…");
    assert!(owns(&app.ui.dialogs.last().unwrap().fields));
    app.ui.close_dialog(id);

    app.run("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    crate::menus::invoke(&mut app, &ctx, COMMAND, json!({"width": 3, "location": "inside", "color": "#00ff00"})).unwrap();
    assert!(app.ui.dialogs.is_empty(), "explicit params must not show a modal dialog");
    assert!(pixel(&app, 10, 15)[1] > 0.98);

    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    assert!(crate::menus::invoke(&mut empty, &ctx, COMMAND, json!({})).is_err());
    assert!(empty.ui.dialogs.is_empty());
    app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    assert!(!crate::menus::is_enabled(&app, COMMAND), "ordinary adjustment masks retain their existing refusal");
    assert!(crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).is_err());
    assert!(app.ui.dialogs.is_empty());
}

/// A real filter mask supports direct Stroke and must also admit its dialog, selection and Undo.
#[test]
fn menu_stroke_edits_the_targeted_filter_mask_and_undoes() {
    let mut app = app();
    app.run("edit.fill", json!({"contents": "white"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap();
    app.run("layer.setProps", json!({"locks": {"pixels": true, "transparency": true}})).unwrap();
    app.ui.filter_mask_target = app.session.active().unwrap().active_layer.map(|id| id.0);
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    assert!(app.filter_mask_targeted());
    assert!(crate::menus::is_enabled(&app, COMMAND));
    let sample = |app: &PhotocraftApp, x, y| {
        let st = app.session.active().unwrap();
        photocraft_engine::channel_cmds::read_target_surface(&st.doc, st.active_layer, &json!({"target": "filterMask"})).unwrap().sample_channel(x, y, 0)
    };
    let history = app.session.active().unwrap().history.past_len();
    assert_eq!(sample(&app, 10, 15), 1.0, "the real filter mask starts white");
    // Direct execution already permits this target; opening its dialog must agree.
    app.run(COMMAND, json!({"width": 2, "location": "inside", "color": "#000000"})).unwrap();
    assert_eq!(sample(&app, 10, 15), 0.0);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(sample(&app, 10, 15), 1.0);

    let opened = crate::menus::invoke(&mut app, &egui::Context::default(), COMMAND, json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    let dialog = app.ui.dialog_mut(opened["dialog"].as_u64().unwrap()).unwrap();
    dialog.fields.insert("width".into(), json!(2));
    dialog.fields.insert("location".into(), json!("inside"));
    dialog.fields.insert("color".into(), json!("#000000"));
    crate::dialogs::confirm(&mut app, opened["dialog"].as_u64().unwrap()).unwrap();
    assert_eq!(sample(&app, 10, 15), 0.0, "the mask's selected edge is stroked");
    assert_eq!(sample(&app, 15, 15), 1.0, "the selection's interior is unchanged");
    assert_eq!(sample(&app, 9, 15), 1.0, "outside is unchanged");
    assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(sample(&app, 10, 15), 1.0);

    // With no selection the grayscale mask is opaque, so Stroke follows its canvas edge.
    app.run("select.deselect", json!({})).unwrap();
    app.run(COMMAND, json!({"width": 2, "location": "inside", "color": "#000000"})).unwrap();
    assert_eq!(sample(&app, 0, 15), 0.0);
    assert_eq!(sample(&app, 15, 15), 1.0);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(sample(&app, 0, 15), 1.0);
}

/// An implicit white smart-filter mask is a dialog target, and ui.set cannot redirect its Stroke.
#[test]
fn implicit_filter_mask_dialog_retains_its_target_when_ui_set_changes_it() {
    use crate::control::{ControlRequest, Outcome, handle};
    use photocraft_doc::LayerContent;
    let mut app = app();
    app.run("edit.fill", json!({"contents": "white"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    assert!(matches!(&app.session.active().unwrap().doc.layer(layer).unwrap().content, LayerContent::Smart(sm) if sm.filter_mask.is_none()));
    app.ui.filter_mask_target = Some(layer.0);
    let ctx = egui::Context::default();
    let id = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    let (request, _) = ControlRequest::new("ui.set", json!({"filterMaskTarget": false}));
    let Outcome::Done(reply) = handle(&mut app, &ctx, &request) else { panic!("ui.set should finish immediately") };
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(!app.filter_mask_targeted());
    let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
    fields.insert("color".into(), json!("#000000"));
    fields.insert("location".into(), json!("inside"));
    let history = app.session.active().unwrap().history.past_len();
    crate::dialogs::confirm(&mut app, id).unwrap();
    let st = app.session.active().unwrap();
    let surface = photocraft_engine::channel_cmds::read_target_surface(&st.doc, st.active_layer, &json!({"target": "filterMask"})).unwrap();
    assert_eq!(surface.sample_channel(0, 15, 0), 0.0);
    assert_eq!(surface.sample_channel(15, 15, 0), 1.0);
    assert_eq!(st.history.past_len(), history + 1);
    let saved = &app.session.prefs().dialogs[COMMAND];
    assert!(saved.get("target").is_none() && saved.get("__target").is_none());
    app.run("edit.undo", json!({})).unwrap();
    assert!(
        matches!(&app.session.active().unwrap().doc.layer(layer).unwrap().content, LayerContent::Smart(sm) if sm.filter_mask.is_none()),
        "Undo restores the implicit representation"
    );
}

/// Stroke retains its existing layer edit in Alpha and Quick Mask views, leaving their samples alone.
#[test]
fn stroke_dialog_keeps_layer_editing_in_channel_views() {
    for quick in [false, true] {
        let mut app = app();
        app.run("edit.fill", json!({"contents": "white"})).unwrap();
        let target = if quick {
            app.run("select.editInQuickMaskMode", json!({"on": true})).unwrap();
            json!("quickMask")
        } else {
            app.run("channel.new", json!({"fill": "white"})).unwrap();
            app.run("channel.target", json!({"channel": 0})).unwrap();
            json!({"channel": 0})
        };
        let samples = |app: &PhotocraftApp| {
            let st = app.session.active().unwrap();
            photocraft_engine::channel_cmds::read_target_surface(&st.doc, st.active_layer, &json!({"target": target})).unwrap().clone()
        };
        let before = samples(&app);
        let history = app.session.active().unwrap().history.past_len();
        let id = open(&mut app);
        let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
        fields.insert("color".into(), json!("#00ff00"));
        fields.insert("location".into(), json!("inside"));
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!(pixel(&app, 0, 15), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(pixel(&app, 15, 15), [1.0; 4]);
        assert_eq!(samples(&app), before, "the viewed channel is not stroked");
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(pixel(&app, 0, 15), [1.0; 4]);
    }
}

#[test]
fn stroke_choices_apply_on_confirm_and_cancel_does_not_modify_document() {
    let mut app = app();
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    let initial = app.session.active().unwrap().history.past_len();
    let cancelled = open(&mut app);
    let dialog = app.ui.dialog_mut(cancelled).unwrap();
    dialog.fields.insert("color".into(), json!("#00ff00"));
    dialog.fields.insert("width".into(), json!(4.0));
    app.ui.close_dialog(cancelled);
    assert_eq!(app.session.active().unwrap().history.past_len(), initial);
    assert_eq!(pixel(&app, 10, 15)[3], 0.0);

    let applied = open(&mut app);
    let d = app.ui.dialog_mut(applied).unwrap();
    d.fields.insert("color".into(), json!("#00ff00"));
    d.fields.insert("width".into(), json!(4.0));
    d.fields.insert("location".into(), json!("inside"));
    d.fields.insert("mode".into(), json!("normal"));
    crate::dialogs::confirm(&mut app, applied).unwrap();
    assert!(pixel(&app, 10, 15)[1] > 0.98, "inside edge receives selected green stroke");
    assert_eq!(pixel(&app, 15, 15)[3], 0.0, "unselected interior unchanged");
    assert_eq!(pixel(&app, 9, 15)[3], 0.0, "outside unchanged for Inside location");
    assert_eq!(app.session.active().unwrap().history.past_len(), initial + 1);
}

#[test]
fn preferences_remember_all_choices_and_reject_corrupt_values() {
    let mut app = app();
    let defaults = fields(&app);
    assert_eq!(defaults["width"], 1.0);
    assert_eq!(defaults["location"], "center");
    assert_eq!(defaults["mode"], "normal");
    assert_eq!(defaults["opacity"], 100.0);
    assert_eq!(defaults["preserveTransparency"], false);

    app.run("select.rect", json!({"x": 5, "y": 5, "width": 15, "height": 12})).unwrap();
    let id = open(&mut app);
    let d = app.ui.dialog_mut(id).unwrap();
    for (key, value) in json!({
        "width": 5,
        "color": "#112233",
        "location": "outside",
        "mode": "multiply",
        "opacity": 42,
        "preserveTransparency": true
    })
    .as_object()
    .unwrap()
    {
        d.fields.insert(key.clone(), value.clone());
    }
    crate::dialogs::confirm(&mut app, id).unwrap();
    let saved = app.session.prefs_to_json();
    let mut restored = photocraft_engine::Session::new();
    restored.load_prefs_json(&saved).unwrap();
    let mut again = PhotocraftApp::new(restored, Default::default());
    again.run("file.new", json!({"width": 40, "height": 30, "background": "transparent"})).unwrap();
    again.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    let remembered = fields(&again);
    assert_eq!(remembered["width"], 5);
    assert_eq!(remembered["color"], "#112233");
    assert_eq!(remembered["location"], "outside");
    assert_eq!(remembered["mode"], "multiply");
    assert_eq!(remembered["opacity"], 42);
    assert_eq!(remembered["preserveTransparency"], true);

    again.session.prefs.edit(|p| {
        p.dialogs.insert(
            COMMAND.into(),
            json!({
                "width": -5, "color": "#nan", "location": "sideways",
                "mode": "unrecognized", "opacity": "a lot", "preserveTransparency": 7
            }),
        )
    });
    let fallback = fields(&again);
    for (key, value) in [
        ("width", json!(1.0)),
        ("color", json!("#ff0000")),
        ("location", json!("center")),
        ("mode", json!("normal")),
        ("opacity", json!(100.0)),
        ("preserveTransparency", json!(false)),
    ] {
        assert_eq!(fallback[key], value, "invalid persisted value {key} must be discarded");
    }
}

#[test]
fn command_params_do_not_leak_private_dialog_fields() {
    let app = app();
    let fields = fields(&app);
    let out = params(&fields);
    assert_eq!(out.as_object().unwrap().len(), 6);
    assert!(out.get("__stroke").is_none());
    assert!(out.get("__label").is_none());
    assert_eq!(out["location"], "center");
    assert_eq!(out["color"], "#ff0000");
}

/// The Stroke dialog's colour swatch opens PhotoCraft's Color Picker on the stroke colour (as in
/// Photoshop), not egui's compact popup; OK sets the stroke colour, not the foreground, and the
/// stroke is drawn in it.
#[test]
fn the_color_swatch_opens_the_color_picker() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app()
    });
    h.run_steps(6);
    h.state_mut().run("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!("#20a040"));
    h.run_steps(3);
    assert_eq!(crate::color_picker_ui::top(h.state()), None);
    h.get_by_label("Stroke color").click();
    h.run_steps(4);
    let picker = crate::color_picker_ui::top(h.state()).expect("the swatch opens the Color Picker");
    let field = |h: &Harness<'_, PhotocraftApp>, d: u64, k: &str| h.state().ui.dialogs.iter().find(|x| x.id == d).and_then(|x| x.fields.get(k).cloned());
    assert_eq!(field(&h, picker, "color"), Some(json!("#20a040")));
    assert_eq!(field(&h, picker, "__label"), Some(json!("Color Picker (Stroke Color)")));
    assert!(!egui::Popup::is_any_open(&h.ctx), "no egui colour popup");
    h.state_mut().ui.dialog_mut(picker).unwrap().fields.insert("color".into(), json!("#0000ff"));
    crate::dialogs::confirm(h.state_mut(), picker).unwrap();
    h.run_steps(3);
    assert_eq!(crate::color_picker_ui::top(h.state()), None);
    assert_eq!(field(&h, id, "color"), Some(json!("#0000ff")));
    assert_eq!(h.state().session.tools.foreground, [1.0, 0.0, 0.0, 1.0], "the foreground is untouched");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let p = pixel(h.state(), 10, 15);
    assert!(p[2] > 0.98 && p[0] < 0.02, "stroked in the picked colour: {p:?}");
}
