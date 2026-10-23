use super::*;
use photocraft_doc::{SmartFilter, SmartObject};

/// A document reopened with the same id is a different allocation, so a dialog opened on the old one cannot edit it.
#[test]
fn a_same_id_reopened_document_rejects_the_previous_dialog() {
    let (mut app, layer) = app(8, "rgb");
    let saved = app.session.active().unwrap().doc.as_ref().clone();
    app.session.close(0).unwrap();
    app.session.add_document(saved.clone(), None);
    let id = open(&mut app, layer, 0).unwrap();
    let old = app.session.active().unwrap().doc.clone();
    app.session.close(0).unwrap();
    app.session.add_document(saved, None);
    assert_eq!(app.session.active().unwrap().revision, 1);
    assert_eq!(old.id, app.session.active().unwrap().doc.id);
    let before = app.session.active().unwrap().doc.clone();
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
}

/// An 8×8 image still gets the full-height preview area instead of collapsing it.
#[test]
fn tiny_images_keep_a_usable_preview_viewport() {
    let (mut app, layer) = app(8, "rgb");
    let id = open(&mut app, layer, 0).unwrap();
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    fields.insert("__imageSize".into(), json!([8, 8]));
    fields.insert("__zoom".into(), json!(100));
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 480.0))), ..Default::default() };
    ctx.run_ui(input, |ui| {
        let start = ui.cursor().top();
        super::viewport(&mut app, ui, &mut fields, false);
        assert!(ui.cursor().top() - start >= 220.0, "tiny image must not collapse the preview");
    })
    .textures_delta
    .clear();
}

/// Run `body` for one frame and return every string it painted.
fn painted_text(app: &mut PhotocraftApp, fields: &mut Map<String, Value>) -> Vec<String> {
    /// Gather the text of `shape`, descending into nested shape groups.
    fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 640.0))), ..Default::default() };
    let mut output = ctx.run_ui(input, |ui| super::body(app, ui, fields));
    output.textures_delta.clear();
    let mut painted = Vec::new();
    output.shapes.iter().for_each(|s| collect(&s.shape, &mut painted));
    painted
}

/// Of all the blend modes, only Dissolve shows the approximation note beside the Mode control.
#[test]
fn only_dissolve_shows_the_approximation_note_near_the_mode_control() {
    let note = "Dissolve is approximated: it blends without Photoshop's dither.";
    let (mut app, layer) = app(8, "rgb");
    let id = open(&mut app, layer, 0).unwrap();
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    for blend in BlendMode::LAYER_MODES {
        fields.insert("blend".into(), json!(blend.label()));
        let painted = painted_text(&mut app, &mut fields);
        assert!(painted.iter().any(|s| s == "Mode"), "the Mode control paints for {}", blend.label());
        assert_eq!(painted.iter().any(|s| s == note), blend == BlendMode::Dissolve, "note for {}", blend.label());
    }
    assert_eq!(mode_note(None), None);
    assert_eq!(mode_note(Some("Normal")), None);
    assert_eq!(mode_note(Some("Dissolve")), Some(note));
}

/// A document with one Smart Object holding Box Blur and Smart Sharpen, wrapped in an app that renders on the calling thread.
pub(crate) fn app(depth: u32, mode: &str) -> (PhotocraftApp, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth, "mode": mode})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Smart Object"})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#305080"})).unwrap();
    s.execute("select.rect", json!({"x":0,"y":0,"width":32,"height":48})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#e09040"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap();
    s.execute("filter.blur.boxBlur", json!({"radius": 4})).unwrap();
    s.execute("filter.sharpen.smartSharpen", json!({"amount": 125, "radius": 2})).unwrap();
    let mut app = PhotocraftApp::new(s, Default::default());
    app.background_jobs = false;
    (app, layer)
}

/// The Smart Object content of `layer` in the active document.
pub(crate) fn smart(app: &PhotocraftApp, layer: LayerId) -> &SmartObject {
    let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(layer).unwrap().content else { panic!("smart") };
    sm
}

/// A copy of the Smart Object's filters, for comparing before and after an edit.
fn filters(app: &PhotocraftApp, layer: LayerId) -> Vec<SmartFilter> {
    smart(app, layer).smart_filters.clone()
}

/// Blending edits one filter in a single history step, opacity is entered as a percentage, and undo/redo restore it.
#[test]
fn blending_edits_one_filter_with_percent_opacity_and_undo_redo() {
    for depth in [8, 16, 32] {
        for mode in ["rgb", "gray"] {
            let (mut app, layer) = app(depth, mode);
            let original = filters(&app, layer);
            let source = photocraft_engine::smart_cmds::contents_of(&app.session.active().unwrap().doc, layer).unwrap();
            for opacity in [0.0, 1.0, 50.0, 100.0] {
                for blend in ["Normal", "Multiply", "Screen", "Luminosity"] {
                    let steps = app.session.active().unwrap().history.past_len();
                    let id = open(&mut app, layer, 0).unwrap();
                    let f = &mut app.ui.dialog_mut(id).unwrap().fields;
                    f.insert("opacity".into(), json!(opacity));
                    f.insert("blend".into(), json!(blend));
                    let fields = f.clone();
                    let (cmd, params) = edit_command(&app, &fields).unwrap();
                    let before = app.session.active().unwrap().doc.clone();
                    let expected = crate::filter_dialog::preview_document(&before, Some(layer), &cmd, &params, 1).unwrap();
                    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
                    crate::dialogs::confirm(&mut app, id).unwrap();
                    let changed = filters(&app, layer);
                    assert!((changed[0].opacity - opacity as f32 / 100.0).abs() < 1e-6);
                    assert_eq!(changed[0].blend.label(), blend);
                    assert_eq!(serde_json::to_value(&changed[1]).unwrap(), serde_json::to_value(&original[1]).unwrap());
                    assert_eq!(photocraft_engine::smart_cmds::contents_of(&app.session.active().unwrap().doc, layer).unwrap(), source);
                    assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().blend, BlendMode::Normal);
                    assert_eq!(
                        smart(&app, layer).cache.as_ref().unwrap().read_region(before.bounds()),
                        expected.layer(layer).unwrap().surface().unwrap().read_region(expected.bounds())
                    );
                    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
                    app.run("edit.undo", json!({})).unwrap();
                    assert_eq!(serde_json::to_value(filters(&app, layer)).unwrap(), serde_json::to_value(&original).unwrap());
                    app.run("edit.redo", json!({})).unwrap();
                    let id = open(&mut app, layer, 0).unwrap();
                    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["opacity"].as_f64().unwrap(), opacity);
                    crate::dialogs::cancel(&mut app, id).unwrap();
                    app.run("edit.undo", json!({})).unwrap();
                }
            }
        }
    }
}

/// The preview, switching it off and on, and Cancel never change the document or its history.
#[test]
fn preview_toggle_and_cancel_leave_document_and_history_untouched() {
    let (mut app, layer) = app(8, "rgb");
    let st = app.session.active().unwrap();
    let before = st.doc.clone();
    let steps = st.history.past_len();
    let id = open(&mut app, layer, 0).unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("opacity".into(), json!(1));
    let ctx = egui::Context::default();
    let idx = app.session.active_index().unwrap();
    assert!(crate::canvas::ensure_filter_preview(&mut app, idx, &ctx).is_some());
    assert_eq!(app.filter_preview.as_ref().unwrap().key.params["opacity"], json!(0.01));
    app.ui.dialog_mut(id).unwrap().fields.insert("__preview".into(), json!(false));
    assert!(crate::canvas::ensure_filter_preview(&mut app, idx, &ctx).is_none());
    app.ui.dialog_mut(id).unwrap().fields.insert("__preview".into(), json!(true));
    assert!(crate::canvas::ensure_filter_preview(&mut app, idx, &ctx).is_some());
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert!(app.filter_preview.is_none());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

/// A dialog whose document or filter has since changed, or been switched away from, refuses to confirm.
#[test]
fn stale_filter_dialogs_cannot_edit_a_switched_or_changed_target() {
    for command in ["file.new", "edit.undo", "layer.smartFilter.delete", "layer.smartFilter.move"] {
        let (mut app, layer) = app(8, "rgb");
        let id = open(&mut app, layer, 0).unwrap();
        let params = match command {
            "file.new" => json!({"width": 16,"height":16}),
            "layer.smartFilter.move" => json!({"layer":layer.0,"index":0,"to":1}),
            _ => json!({"layer":layer.0,"index":0}),
        };
        app.run(command, params).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        assert!(crate::dialogs::confirm(&mut app, id).is_err(), "{command}");
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    }
}

/// A reduced-size preview scales the filters' pixel parameters and the shared mask, and still contains the same number of filters.
#[test]
fn reduced_preview_scales_placement_stack_and_mask_without_adding_filters() {
    let (mut app, layer) = app(16, "rgb");
    // A half-visible shared mask, different from the layer mask.
    let st = app.session.active_mut().unwrap();
    let doc = Arc::make_mut(&mut st.doc);
    let LayerContent::Smart(sm) = &mut doc.layer_mut(layer).unwrap().content else { panic!() };
    let mut mask = photocraft_doc::LayerMask::reveal_all();
    mask.surface.fill_rect(photocraft_geom::Rect::new(32, 0, 64, 48), &[0.0]);
    sm.filter_mask = Some(mask);
    let id = open(&mut app, layer, 0).unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("opacity".into(), json!(50));
    let (command, params) = edit_command(&app, &app.ui.dialogs[0].fields).unwrap();
    let original = app.session.active().unwrap().doc.clone();
    let preview = crate::filter_dialog::preview_document(&original, Some(layer), &command, &params, 2).unwrap();
    assert_eq!(preview.size.width, 32);
    let LayerContent::Smart(sm) = &preview.layer(layer).unwrap().content else { panic!() };
    assert_eq!(sm.smart_filters.len(), 2);
    assert_eq!(sm.smart_filters[0].params["radius"], json!(2.0));
    assert_eq!(sm.smart_filters[1].params["radius"], json!(1.0));
    assert_eq!(sm.filter_mask.as_ref().unwrap().surface.pixel(17, 0)[0], 0.0);
    assert!(sm.cache.as_ref().unwrap().content_bounds().x1 <= 32);
    assert!(Arc::ptr_eq(&original, &app.session.active().unwrap().doc));
}

/// An edited blend and an Invert recorded earlier survive a save and reload of the native format.
#[test]
fn native_save_preserves_edited_blending_and_existing_invert() {
    let (mut app, layer) = app(8, "rgb");
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "image.adjustments.invert", json!({})).unwrap();
    assert_eq!(smart(&app, layer).smart_filters.last().unwrap().command, "image.adjustments.invert");
    let id = open(&mut app, layer, 2).unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("opacity".into(), json!(1));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let loaded = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(doc, &Default::default()).unwrap()).unwrap();
    let LayerContent::Smart(sm) = &loaded.layers.last().unwrap().content else { panic!() };
    assert_eq!(serde_json::to_value(&sm.smart_filters).unwrap(), serde_json::to_value(filters(&app, layer)).unwrap());
}

/// An opacity outside 0–100% or not a number fails the dialog and leaves the document alone.
#[test]
fn invalid_opacity_fails_without_an_edit() {
    let (mut app, layer) = app(8, "rgb");
    for opacity in [json!(-1), json!(101), json!("x"), Value::Null] {
        let id = open(&mut app, layer, 0).unwrap();
        app.ui.dialog_mut(id).unwrap().fields.insert("opacity".into(), opacity);
        let before = app.session.active().unwrap().doc.clone();
        assert!(crate::dialogs::confirm(&mut app, id).is_err());
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    }
}

/// Blending edits the filter the dialog opened on, whatever layer is selected and whether or not a layer mask is targeted.
#[test]
fn blending_does_not_follow_layer_selection_or_the_layer_mask_target() {
    let (mut app, layer) = app(8, "rgb");
    app.run("layer.new.layer", json!({"name":"Selected pixel layer"})).unwrap();
    let selected = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.layerMask.revealAll", json!({"layer":selected.0})).unwrap();
    app.ui.mask_target = true;
    let doc = app.session.active().unwrap().doc.clone();
    let mask = doc.layer(selected).unwrap().mask.as_ref().unwrap().surface.clone();
    let id = open(&mut app, layer, 0).unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("opacity".into(), json!(25));
    let idx = app.session.active_index().unwrap();
    assert!(crate::canvas::ensure_filter_preview(&mut app, idx, &Default::default()).is_some());
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(smart(&app, layer).smart_filters[0].opacity, 0.25);
    assert_eq!(app.session.active().unwrap().active_layer, Some(selected));
    assert_eq!(
        crate::surface_fingerprint(&app.session.active().unwrap().doc.layer(selected).unwrap().mask.as_ref().unwrap().surface),
        crate::surface_fingerprint(&mask)
    );
}

/// Blending an Invert smart filter keeps the translucent pixels' alpha at 8, 16 and 32 bits.
#[test]
fn invert_blending_keeps_transparency_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width":8,"height":8,"depth":depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.edit("synthetic translucent pixels", |doc, active| {
            let bounds = doc.bounds();
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(bounds, &[0.2, 0.4, 0.6, 0.5]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let layer = s.active().unwrap().active_layer.unwrap();
        let before = s.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().pixel(3, 3);
        s.execute("image.adjustments.invert", json!({})).unwrap();
        let mut app = PhotocraftApp::new(s, Default::default());
        app.background_jobs = false;
        let id = open(&mut app, layer, 0).unwrap();
        app.ui.dialog_mut(id).unwrap().fields.insert("opacity".into(), json!(1));
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!(smart(&app, layer).cache.as_ref().unwrap().pixel(3, 3)[3], before[3]);
    }
}
