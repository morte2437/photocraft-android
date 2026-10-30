use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use photocraft_color::{Color, ColorMode};
use photocraft_doc::{Fill, LayerContent, LayerId};
use serde_json::json;

use super::*;

fn harness(ppp: f32) -> (Harness<'static, PhotocraftApp>, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": 16})).unwrap();
    let id = LayerId(s.execute("layer.newFillLayer.solidColor", json!({"color": "#33669980"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Other"})).unwrap();
    let mut h =
        Harness::builder().with_size(vec2(1440.0, 900.0)).with_pixels_per_point(ppp).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            PhotocraftApp::new(s, crate::Services::default())
        });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    (h, id)
}

fn double_click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.run_steps(40);
    for _ in 0..2 {
        h.hover_at(at);
        h.step();
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.step();
        }
        h.run_steps(3);
    }
}

fn row(h: &Harness<'_, PhotocraftApp>, id: LayerId) -> crate::layer_row_ui::RowRects {
    crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == id.0).unwrap()
}

#[test]
fn new_solid_color_from_menu_cancel_keeps_document_and_history_unchanged() {
    let (mut h, _) = harness(1.0);
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    let before = app.session.active().unwrap().doc.clone();
    let active = app.session.active().unwrap().active_layer;
    let steps = app.session.active().unwrap().history.entries().len();

    let opened = crate::menus::invoke(app, &ctx, "layer.newFillLayer.solidColor", json!({})).unwrap();
    assert!(opened.get("layer").is_none(), "no layer exists until OK");
    let dialog = opened["dialog"].as_u64().unwrap();
    let picker = app.ui.dialogs.last().unwrap();
    assert_eq!(picker.id, dialog);
    assert!(owns_new(&picker.fields), "new Solid Color fill opens the shared Color Picker");
    assert_eq!(picker.fields["__params"]["document"], before.id.0);
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps);

    app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#ff8800"));
    crate::dialogs::cancel(app, dialog).unwrap();
    assert!(app.ui.dialogs.iter().all(|d| d.id != dialog));
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "Cancel must not create or delete a layer");
    assert_eq!(app.session.active().unwrap().active_layer, active);
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps, "Cancel must not record a history entry");
}

#[test]
fn new_solid_color_from_menu_ok_creates_one_layer_with_chosen_or_default_color() {
    let (mut h, _) = harness(1.0);
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    let steps = app.session.active().unwrap().history.entries().len();

    // OK with the initial foreground colour still creates the fill layer.
    let first = crate::menus::invoke(app, &ctx, "layer.newFillLayer.solidColor", json!({})).unwrap();
    let created = crate::dialogs::confirm(app, first["dialog"].as_u64().unwrap()).unwrap();
    let id = LayerId(created["layer"].as_u64().unwrap());
    assert!(matches!(app.session.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Fill(Fill::Solid(_))));
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 1);

    let second = crate::menus::invoke(app, &ctx, "layer.newFillLayer.solidColor", json!({})).unwrap();
    let dialog = second["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#ff8800"));
    let created = crate::dialogs::confirm(app, dialog).unwrap();
    let id = LayerId(created["layer"].as_u64().unwrap());
    let LayerContent::Fill(Fill::Solid(color)) = app.session.active().unwrap().doc.layer(id).unwrap().content else {
        panic!("expected the newly created Solid Color fill");
    };
    assert_eq!(color.to_rgba8()[..3], [255, 136, 0]);
    assert_eq!(app.session.active().unwrap().active_layer, Some(id));
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 2);

    let dialogs = app.ui.dialogs.len();
    let explicit = crate::menus::invoke(app, &ctx, "layer.newFillLayer.solidColor", json!({"color": "#123456"})).unwrap();
    assert!(explicit["layer"].as_u64().is_some());
    assert!(explicit.get("dialog").is_none());
    assert_eq!(app.ui.dialogs.len(), dialogs, "parameterised creation stays non-interactive");
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 3);
}

#[test]
fn new_solid_color_picker_rejects_a_different_active_document() {
    let (mut h, _) = harness(1.0);
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    let dialog = crate::menus::invoke(app, &ctx, "layer.newFillLayer.solidColor", json!({})).unwrap()["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#ff8800"));
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.entries().len();
    assert!(crate::dialogs::confirm(app, dialog).is_err());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps);
}

#[test]
fn only_the_colour_thumbnail_opens_the_solid_fill_picker() {
    for ppp in [1.0, 2.0] {
        let (mut h, id) = harness(ppp);
        let r = row(&h, id).row;
        let count = h.state().session.active().unwrap().history.entries().len();
        double_click(&mut h, pos2(r.left() + 46.0, r.center().y));
        assert_eq!(h.state().session.active().unwrap().active_layer, Some(id));
        assert!(!h.state().ui.mask_target && !h.state().ui.vector_mask_target, "the colour thumbnail targets the fill, not its mask");
        let d = h.state().ui.dialogs.last().expect("Color Picker opened");
        assert!(owns(&d.fields));
        assert_eq!(d.fields["__label"], "Color Picker (Solid Color)");
        assert_eq!(d.fields["color"], "#336699");
        assert_eq!(d.fields["__params"]["layer"], id.0);
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), count);

        let (mut h, id) = harness(ppp);
        let r = row(&h, id).row;
        double_click(&mut h, pos2(r.right() - 35.0, r.center().y));
        assert_eq!(h.state().ui.dialogs.last().unwrap().kind, crate::state::DialogKind::LayerStyle, "row body keeps Layer Style");

        let (mut h, id) = harness(ppp);
        let r = crate::mask_thumbs_ui::recorded(&h.ctx, id.0).unwrap().0[0].1;
        double_click(&mut h, r.center());
        assert!(!h.state().ui.dialogs.iter().any(|d| owns(&d.fields)), "mask thumbnail is not the colour thumbnail");
    }
}

#[test]
fn gradient_and_adjustment_thumbnails_keep_their_properties_controls() {
    for command in ["layer.newFillLayer.gradient", "layer.newAdjustmentLayer.invert"] {
        let (mut h, _) = harness(1.0);
        let id = LayerId(h.state_mut().run(command, json!({})).unwrap()["layer"].as_u64().unwrap());
        h.run_steps(4);
        let r = row(&h, id).row;
        double_click(&mut h, pos2(r.left() + 46.0, r.center().y));
        assert!(h.state().ui.panels.properties);
        assert!(!h.state().ui.dialogs.iter().any(|d| owns(&d.fields)));
    }
}

#[test]
fn picker_previews_without_editing_cancel_discards_and_ok_is_one_step() {
    let (mut h, id) = harness(1.0);
    let app = h.state_mut();
    app.run("layer.select", json!({"layer": id.0})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let LayerContent::Fill(Fill::Solid(original)) = before.layer(id).unwrap().content else { panic!() };
    let steps = app.session.active().unwrap().history.entries().len();
    let tools = app.session.tools.foreground;
    let dialog = open(app).unwrap();
    app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#ff8800"));
    let (preview, key) = display_doc(app, 0).unwrap();
    let (again, again_key) = display_doc(app, 0).unwrap();
    assert_eq!(key, again_key);
    assert!(Arc::ptr_eq(&preview, &again), "preview is cached");
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "preview does not dirty the real document");
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps);
    let LayerContent::Fill(Fill::Solid(color)) = preview.layer(id).unwrap().content else { panic!() };
    assert_eq!(color.to_rgba8(), [255, 136, 0, 128]);
    assert_eq!(thumbnail_color(app, before.id, id, original), color);
    app.ui.close_dialog(dialog);
    assert!(display_doc(app, 0).is_none());
    assert!(app.solid_fill_preview.is_none());
    assert_eq!(thumbnail_color(app, before.id, id, original), original);
    let dialog = open(app).unwrap();
    app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#ff8800"));
    let (_, reopened_key) = display_doc(app, 0).unwrap();
    assert_ne!(key, reopened_key, "reopening a picker never reuses a cancelled preview key");
    crate::dialogs::confirm(app, dialog).unwrap();
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 1);
    assert_eq!(app.session.tools.foreground, tools, "editing a fill doesn't change the tool colour");
    assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Fill(Fill::Solid(color)));
    let bytes = photocraft_format::save_to_bytes(&app.session.active().unwrap().doc, &Default::default()).unwrap();
    let loaded = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(loaded.layer(id).unwrap().content, LayerContent::Fill(Fill::Solid(color)));
    assert_eq!(loaded.layer(id).unwrap().mask, before.layer(id).unwrap().mask);
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "fill.psd", &Default::default()).unwrap().bytes;
    let loaded = photocraft_io::import("fill.psd", &bytes).unwrap().document;
    let fill = loaded.walk().into_iter().find(|(_, _, l)| matches!(l.content, LayerContent::Fill(Fill::Solid(_)))).unwrap().2;
    let LayerContent::Fill(Fill::Solid(saved)) = fill.content else { panic!() };
    // PSD's existing SoCo descriptor stores the fill's colour components, not Color.alpha.
    assert_eq!(&saved.to_rgba8()[..3], &color.to_rgba8()[..3]);
    assert!(fill.mask.is_some());
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(*app.session.active().unwrap().doc, *before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Fill(Fill::Solid(color)));
}

#[test]
fn unchanged_ok_keeps_imported_model_precision_and_alpha() {
    let (mut h, id) = harness(1.0);
    let app = h.state_mut();
    app.run("layer.select", json!({"layer": id.0})).unwrap();
    let color = Color { mode: ColorMode::Lab, c: [0.512345, 0.498765, 0.56789, 0.0], alpha: 0.345678 };
    app.run(cmds::SET, json!({"color": color})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let dialog = open(app).unwrap();
    crate::dialogs::confirm(app, dialog).unwrap();
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
}

#[test]
fn picker_never_applies_to_a_different_document_with_the_same_layer_id() {
    let (mut h, id) = harness(1.0);
    let app = h.state_mut();
    app.run("layer.select", json!({"layer": id.0})).unwrap();
    let dialog = open(app).unwrap();
    app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#ff0000"));
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    app.run("layer.newFillLayer.solidColor", json!({})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    assert!(display_doc(app, app.session.active_index().unwrap()).is_none());
    assert!(crate::dialogs::confirm(app, dialog).is_err());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
}

#[test]
fn cmyk_lab_preview_thumbnail_cancel_and_ok_are_consistent() {
    for (command, mode) in [("image.mode.cmyk", ColorMode::Cmyk), ("image.mode.lab", ColorMode::Lab)] {
        let (mut h, id) = harness(1.0);
        let app = h.state_mut();
        app.run(command, json!({})).unwrap();
        app.run("layer.select", json!({"layer": id.0})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let LayerContent::Fill(Fill::Solid(original)) = before.layer(id).unwrap().content else { panic!() };
        assert_eq!(before.mode, mode);
        let count = app.session.active().unwrap().history.entries().len();
        let dialog = open(app).unwrap();
        app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#ff8800"));
        let (shown, _) = display_doc(app, app.session.active_index().unwrap()).unwrap();
        let LayerContent::Fill(Fill::Solid(preview)) = shown.layer(id).unwrap().content else { panic!() };
        assert_eq!(preview, Color::rgba(1.0, 136.0 / 255.0, 0.0, original.alpha).in_mode(mode));
        assert_eq!(thumbnail_color(app, before.id, id, original), preview);
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        assert_eq!(app.session.active().unwrap().history.entries().len(), count);
        app.ui.close_dialog(dialog);
        assert!(display_doc(app, app.session.active_index().unwrap()).is_none());
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        assert_eq!(app.session.active().unwrap().history.entries().len(), count);
        let dialog = open(app).unwrap();
        app.ui.dialog_mut(dialog).unwrap().fields.insert("color".into(), json!("#ff8800"));
        crate::dialogs::confirm(app, dialog).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Fill(Fill::Solid(preview)));
        assert_eq!(app.session.active().unwrap().history.entries().len(), count + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(*app.session.active().unwrap().doc, *before);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Fill(Fill::Solid(preview)));
    }
}

/// #8568: CMYK 50/50/50/100 typed into the Solid Color picker of a CMYK document is the fill's
/// colour after OK, for a new fill and for an edited one, not what its sRGB stand-in converts back
/// to through the document profile.
#[test]
fn cmyk_typed_into_the_picker_is_the_fills_colour_in_a_cmyk_document() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("image.mode.cmyk", json!({})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    let typed = |f: &mut serde_json::Map<String, serde_json::Value>, inks: [f32; 4]| {
        let [r, g, b] = photocraft_color::convert::cmyk_to_rgb(inks).map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        f.insert("color".into(), json!(format!("#{r:02x}{g:02x}{b:02x}")));
        f.insert("__cmyk".into(), json!(inks));
    };
    let fill = |app: &PhotocraftApp| {
        let st = app.session.active().unwrap();
        match st.doc.layer(st.active_layer.unwrap()).unwrap().content {
            LayerContent::Fill(Fill::Solid(c)) => c,
            _ => panic!("not a solid fill"),
        }
    };
    let inks = [0.5, 0.5, 0.5, 1.0];
    let id = open_new(&mut app).unwrap();
    let mut f = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    typed(&mut f, inks);
    confirm_new(&mut app, &f).unwrap();
    assert_eq!(fill(&app), Color { mode: ColorMode::Cmyk, c: inks, alpha: 1.0 });

    let edit = [0.25, 0.75, 0.0, 0.1];
    let id = open(&mut app).unwrap();
    let mut f = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    typed(&mut f, edit);
    confirm(&mut app, &f).unwrap();
    assert_eq!(fill(&app).c, edit);
    // Reopening the picker shows the stored inks.
    let id = open(&mut app).unwrap();
    let shown = color_picker_ui::current(&app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields);
    assert_eq!(shown.cmyk, edit);

    // Other inks with the same display colour are still an edit.
    let near = [0.5, 0.5, 0.499, 1.0];
    let id = open(&mut app).unwrap();
    let mut f = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    typed(&mut f, near);
    confirm(&mut app, &f).unwrap();
    let rich = [0.5, 0.5, 0.5, 1.0];
    let id = open(&mut app).unwrap();
    let mut f = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    typed(&mut f, rich);
    assert_eq!(f.get("color"), f.get("__orig"), "both inks show as the same colour");
    confirm(&mut app, &f).unwrap();
    assert_eq!(fill(&app).c, rich);
}

fn with_inks(f: &mut serde_json::Map<String, serde_json::Value>, inks: [f32; 4]) {
    let [r, g, b] = photocraft_color::convert::cmyk_to_rgb(inks).map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    f.insert("color".into(), json!(format!("#{r:02x}{g:02x}{b:02x}")));
    f.insert("__cmyk".into(), json!(inks));
}

/// Inks that share the picker's hex with the fill's own are still a preview and a thumbnail, as
/// they are an edit on OK (#8568).
#[test]
fn same_hex_inks_preview_and_thumbnail_as_ok_commits_them() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("image.mode.cmyk", json!({})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    let near = [0.5, 0.5, 0.499, 1.0];
    let id = open_new(&mut app).unwrap();
    let mut f = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    with_inks(&mut f, near);
    confirm_new(&mut app, &f).unwrap();

    let rich = [0.5, 0.5, 0.5, 1.0];
    let id = open(&mut app).unwrap();
    with_inks(&mut app.ui.dialog_mut(id).unwrap().fields, rich);
    let fields = &app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields;
    assert_eq!(fields.get("color"), fields.get("__orig"), "both inks show as the same colour");

    let st = app.session.active().unwrap();
    let (document, layer) = (st.doc.id, st.active_layer.unwrap());
    let LayerContent::Fill(Fill::Solid(original)) = st.doc.layer(layer).unwrap().content else { panic!("not a solid fill") };
    assert_eq!(thumbnail_color(&app, document, layer, original).c, rich);
    let (shown, _) = display_doc(&mut app, 0).expect("a preview of the new inks");
    let LayerContent::Fill(Fill::Solid(previewed)) = shown.layer(layer).unwrap().content else { panic!("not a solid fill") };
    assert_eq!(previewed.c, rich);
}

/// An RGB fill inside a CMYK document that the picker gives inks at the unchanged hex is an edit;
/// in an RGB document, which ignores the inks, it is not.
#[test]
fn inks_given_to_a_non_cmyk_fill_in_a_cmyk_document_are_an_edit() {
    let original = Color::rgba(0.2, 0.3, 0.4, 1.0);
    let mut f = serde_json::Map::new();
    with_inks(&mut f, [0.3, 0.2, 0.1, 0.4]);
    f.insert("__orig".into(), f["color"].clone());
    assert!(is_edit(&f, original, ColorMode::Cmyk));
    assert!(!is_edit(&f, original, ColorMode::Rgb));
    assert!(picked_color(&f, original, ColorMode::Cmyk).is_some_and(|c| c.mode == ColorMode::Cmyk && c.c == [0.3, 0.2, 0.1, 0.4]));
}
