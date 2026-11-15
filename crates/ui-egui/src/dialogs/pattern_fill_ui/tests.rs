use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{Fill, LayerContent, LayerId};
use serde_json::json;

use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "depth": 16})).unwrap();
    app
}

fn pattern_id(app: &PhotocraftApp, name: &str) -> String {
    app.session.patterns.items.iter().find(|p| p.name == name).unwrap().id.clone()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn set(app: &mut PhotocraftApp, dialog: u64, values: serde_json::Value) {
    let d = app.ui.dialog_mut(dialog).unwrap();
    for (k, v) in values.as_object().unwrap() {
        d.fields.insert(k.clone(), v.clone());
    }
}

/// A drag frame at an arbitrary pointer position; the canvas tests cover the pointer's release.
fn drag(f: &mut serde_json::Map<String, serde_json::Value>, start: bool, total: [f64; 2], target: &DragTarget) {
    drag_phase(f, start, total, [0.0, 0.0], target);
}

/// The private drag state never holds a non-finite coordinate, even after a huge travel.
fn assert_finite_state(f: &serde_json::Map<String, serde_json::Value>) {
    let state = f.get(DRAG_KEY).expect("drag state");
    for key in ["base", "total", "shown", "at", "travel"] {
        let a = state[key].as_array().unwrap_or_else(|| panic!("{key}: {state}"));
        assert!(a.iter().all(|v| v.as_f64().is_some_and(f64::is_finite)), "{key} is finite: {state}");
    }
}

#[test]
fn new_pattern_fill_opens_the_dialog_previews_and_ok_is_one_step() {
    let mut app = app();
    let ctx = egui::Context::default();
    let before = app.session.active().unwrap().doc.clone();
    let count = steps(&app);
    let dialog = crate::menus::invoke(&mut app, &ctx, cmds::CREATE, json!({})).unwrap()["dialog"].as_u64().unwrap();
    let d = app.ui.dialogs.last().unwrap();
    assert!(owns(&d.fields));
    assert_eq!(crate::dialogs::title(d), "Pattern Fill");
    assert_eq!((d.fields["scale"].clone(), d.fields["angle"].clone(), d.fields["link"].clone()), (json!(100.0), json!(0.0), json!(true)));
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "no layer until OK");
    let bricks = pattern_id(&app, "Bricks");
    set(&mut app, dialog, json!({"pattern": bricks, "scale": 200.0, "angle": 45.0, "link": false}));
    let (shown, key) = display_doc(&mut app, 0).unwrap();
    let (again, again_key) = display_doc(&mut app, 0).unwrap();
    assert!(Arc::ptr_eq(&shown, &again) && key == again_key, "the preview is cached");
    assert_eq!(shown.layers.len(), before.layers.len() + 1, "the preview shows the new layer");
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "previewing does not edit");
    assert_eq!(steps(&app), count);
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert_eq!(steps(&app), count + 1);
    let st = app.session.active().unwrap();
    let LayerContent::Fill(Fill::Pattern { id, scale, angle, link, .. }) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else {
        panic!("no pattern fill")
    };
    assert_eq!((id.as_str(), *scale, *angle, *link), (bricks.as_str(), 2.0, 45.0, false));
    assert!(display_doc(&mut app, 0).is_none() && app.pattern_fill_preview.is_none());
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(*app.session.active().unwrap().doc, *before);
}

#[test]
fn cancelling_a_new_pattern_fill_adds_nothing() {
    let mut app = app();
    let ctx = egui::Context::default();
    let before = app.session.active().unwrap().doc.clone();
    let count = steps(&app);
    let dialog = crate::menus::invoke(&mut app, &ctx, cmds::CREATE, json!({})).unwrap()["dialog"].as_u64().unwrap();
    set(&mut app, dialog, json!({"scale": 300.0}));
    assert!(display_doc(&mut app, 0).is_some());
    crate::dialogs::cancel(&mut app, dialog).unwrap();
    assert!(display_doc(&mut app, 0).is_none());
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(steps(&app), count);
    // Explicit params (Actions, CLI, MCP) still create the layer directly.
    let r = crate::menus::invoke(&mut app, &ctx, cmds::CREATE, json!({"pattern": "Grid"})).unwrap();
    assert!(r["layer"].as_u64().is_some());
    // No document: an error, not a dialog or a panic.
    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert!(crate::menus::invoke(&mut empty, &ctx, cmds::CREATE, json!({})).is_err());
    assert!(empty.ui.dialogs.is_empty());
}

#[test]
fn content_options_edit_the_layer_and_cancel_restores() {
    let mut app = app();
    let ctx = egui::Context::default();
    let id = app.run(cmds::CREATE, json!({"pattern": "Checkerboard", "scale": 150, "angle": 10})).unwrap()["layer"].as_u64().unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let count = steps(&app);
    let dialog = crate::menus::invoke(&mut app, &ctx, "layer.layerContentOptions", serde_json::Value::Null).unwrap()["dialog"].as_u64().unwrap();
    let checkerboard = pattern_id(&app, "Checkerboard");
    let f = &app.ui.dialog_mut(dialog).unwrap().fields;
    assert!(owns(f));
    assert_eq!((f["pattern"].clone(), f["scale"].clone(), f["angle"].clone()), (json!(checkerboard), json!(150.0), json!(10.0)));
    assert_eq!(f["__params"]["layer"], id);
    // Unchanged: no preview and OK is no History step.
    assert!(display_doc(&mut app, 0).is_none());
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert_eq!(steps(&app), count);
    // Changed then cancelled: previewed, but the layer keeps its fill.
    let dialog = open(&mut app).unwrap();
    let dots = pattern_id(&app, "Dots");
    set(&mut app, dialog, json!({"pattern": dots, "phase": [5.0, 5.0]}));
    let (shown, _) = display_doc(&mut app, 0).unwrap();
    assert!(matches!(&shown.layer(LayerId(id)).unwrap().content, LayerContent::Fill(Fill::Pattern { phase, .. }) if *phase == (5.0, 5.0)));
    crate::dialogs::cancel(&mut app, dialog).unwrap();
    assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert!(display_doc(&mut app, 0).is_none());
    // A stale dialog never edits another document.
    let dialog = open(&mut app).unwrap();
    set(&mut app, dialog, json!({"scale": 50.0}));
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    assert!(display_doc(&mut app, 1).is_none());
    assert!(crate::dialogs::confirm(&mut app, dialog).is_err());
    // Gradient and Solid Color fills keep their own editors.
    app.run("layer.newFillLayer.gradient", json!({})).unwrap();
    assert!(open(&mut app).is_none());
}

fn harness() -> (Harness<'static, PhotocraftApp>, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let id = LayerId(s.execute(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.new.layer", json!({"name": "Other"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
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

#[test]
fn double_clicking_the_thumbnail_opens_the_dialog_and_a_picked_pattern_is_applied() {
    let (mut h, id) = harness();
    let r = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == id.0).unwrap().row;
    let count = steps(h.state());
    double_click(&mut h, pos2(r.left() + 46.0, r.center().y));
    let d = h.state().ui.dialogs.last().expect("the Pattern Fill dialog opened").clone();
    assert!(owns(&d.fields));
    assert_eq!(d.fields["__params"]["layer"], id.0);
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Patterns").click();
    h.run_steps(3);
    h.get_by_label("Bricks").click();
    h.run_steps(3);
    let bricks = pattern_id(h.state(), "Bricks");
    assert_eq!(h.state().ui.dialogs.iter().find(|x| x.id == d.id).unwrap().fields["pattern"], json!(bricks));
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(!h.state().ui.dialogs.iter().any(|x| owns(&x.fields)));
    assert_eq!(steps(h.state()), count + 1);
    let content = h.state().session.active().unwrap().doc.layer(id).unwrap().content.clone();
    assert!(matches!(content, LayerContent::Fill(Fill::Pattern { id: ref p, .. }) if *p == bricks));
}

#[test]
fn an_imported_out_of_range_fill_keeps_its_fields_when_the_phase_changes() {
    let mut app = app();
    let id = LayerId(app.run(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap()["layer"].as_u64().unwrap());
    // A PSD may carry values outside the dialog's ranges; opening never rewrites them.
    {
        let st = app.session.active_mut().unwrap();
        let doc = Arc::make_mut(&mut st.doc);
        let LayerContent::Fill(Fill::Pattern { name, id: pattern, .. }) = &doc.layer(id).unwrap().content else { panic!() };
        let (name, pattern) = (name.clone(), pattern.clone());
        doc.layer_mut(id).unwrap().content =
            LayerContent::Fill(Fill::Pattern { name, id: pattern, scale: 20.0, angle: 300.0, link: true, phase: (2.5, -3.25) });
    }
    let before = app.session.active().unwrap().doc.clone();
    let count = steps(&app);
    let dialog = open(&mut app).unwrap();
    assert_eq!(app.ui.dialog_mut(dialog).unwrap().fields["scale"], json!(2000.0));
    assert_eq!(app.ui.dialog_mut(dialog).unwrap().fields["angle"], json!(300.0));
    // Unchanged OK is no History step and rewrites nothing.
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert_eq!(steps(&app), count);
    assert_eq!(*app.session.active().unwrap().doc, *before);
    // Editing the phase sends only the phase: the imported values survive, and undo restores them.
    let dialog = open(&mut app).unwrap();
    set(&mut app, dialog, json!({"phase": [5.0, 5.0]}));
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    let st = app.session.active().unwrap();
    let LayerContent::Fill(Fill::Pattern { scale, angle, phase, .. }) = &st.doc.layer(id).unwrap().content else { panic!() };
    assert_eq!((*scale, *angle, *phase), (20.0, 300.0, (5.0, 5.0)));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(*app.session.active().unwrap().doc, *before);
}

#[test]
fn a_canvas_drag_needs_a_linked_editable_pattern_target() {
    let mut app = app();
    let id = app.run(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap()["layer"].as_u64().unwrap();
    let dialog = open(&mut app).unwrap();
    let fields = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    assert!(drag_target(&app, &fields).is_some());
    // Unlinked: the pattern does not follow the pointer.
    let mut f = fields.clone();
    f.insert("link".into(), json!(false));
    assert!(drag_target(&app, &f).is_none());
    // A wrong kind (the Background) is not a Pattern Fill.
    let background = app.session.active().unwrap().doc.layers[0].id.0;
    let mut f = fields.clone();
    f.insert("__params".into(), json!({"layer": background, "document": app.session.active().unwrap().doc.id.0}));
    assert!(drag_target(&app, &f).is_none());
    // A pixel-locked fill cannot move.
    app.run("layer.lockLayers", json!({"pixels": true})).unwrap();
    assert!(drag_target(&app, &fields).is_none());
    app.run("layer.lockLayers", json!({"pixels": false})).unwrap();
    // A deleted layer...
    app.run("layer.delete", json!({"layer": id})).unwrap();
    assert!(drag_target(&app, &fields).is_none());
    // ...and a document switched away from are stale.
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    assert!(drag_target(&app, &fields).is_none());
    // A new fill may be inserted above a locked base: creation is not the edit's lock check.
    let dialog = open_new(&mut app).unwrap();
    let fields = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    app.run("layer.lockLayers", json!({"all": true})).unwrap();
    assert!(drag_target(&app, &fields).is_some());
    // Hostile phase fields are ignored, never a panic.
    for phase in [json!("nope"), json!([1.0]), json!([1.0e300, 0.0])] {
        let mut f = fields.clone();
        f.insert("phase".into(), phase);
        assert!(drag_target(&app, &f).is_none());
    }
}

#[test]
fn a_pattern_drag_uses_the_total_travel_and_keeps_external_edits() {
    let mut app = app();
    let id = app.run(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap()["layer"].as_u64().unwrap();
    let dialog = open(&mut app).unwrap();
    let mut f = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    let target = drag_target(&app, &f).unwrap();
    f.insert("phase".into(), json!([2.5, -3.25]));
    drag(&mut f, true, [0.0, 0.0], &target);
    assert_eq!(f["phase"], json!([2.5, -3.25]), "a press leaves the phase");
    drag(&mut f, false, [3.0, 4.0], &target);
    drag(&mut f, false, [7.0, 4.0], &target);
    assert_eq!(f["phase"], json!([9.5, 0.75]), "the total since the press, not each frame");
    drag(&mut f, false, [0.0, 0.0], &target);
    assert_eq!(f["phase"], json!([2.5, -3.25]), "a drag back restores the baseline exactly");
    // An external phase update under the press is kept, and the drag continues from it.
    drag(&mut f, false, [10.0, 0.0], &target);
    f.insert("phase".into(), json!([0.0, 0.0]));
    drag(&mut f, false, [10.0, 0.0], &target);
    assert_eq!(f["phase"], json!([0.0, 0.0]));
    drag(&mut f, false, [12.0, 0.0], &target);
    assert_eq!(f["phase"], json!([2.0, 0.0]));
    // Snap to Origin zeroes the phase and clears the origin; the held press does not restart.
    f.insert("phase".into(), json!([0.0, 0.0]));
    end_phase_drag(&mut f);
    drag(&mut f, false, [5.0, 5.0], &target);
    assert_eq!(f["phase"], json!([0.0, 0.0]));
    assert!(!f.contains_key(DRAG_KEY));
    // A finite huge travel publishes only what the engine accepts.
    drag(&mut f, true, [0.0, 0.0], &target);
    drag(&mut f, false, [1.0e12, -1.0e12], &target);
    assert_eq!(f["phase"], json!([MAX_PHASE, -MAX_PHASE]));
    assert_finite_state(&f);
    // A changed target ends the gesture instead of moving another layer.
    let phase = f["phase"].clone();
    let mut other = target.clone();
    other.layer = Some(id + 1);
    drag(&mut f, true, [0.0, 0.0], &target);
    drag(&mut f, false, [5.0, 5.0], &other);
    assert_eq!(f["phase"], phase);
    assert!(!f.contains_key(DRAG_KEY));
    // The drag state is not a command param and does not make the dialog changed.
    let mut clean = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    clean.insert(DRAG_KEY.into(), json!({"base": [1.0, 2.0], "total": [0.0, 0.0], "shown": [1.0, 2.0], "at": [0.0, 0.0], "travel": [0.0, 0.0]}));
    assert!(unchanged(&clean));
    let (_, params) = command(&app, &clean).unwrap();
    assert!(params.get(DRAG_KEY).is_none());
}

#[test]
fn a_drag_continues_after_an_external_edit_near_the_phase_bound() {
    let mut app = app();
    app.run(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap();
    let dialog = open(&mut app).unwrap();
    let mut f = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    let target = drag_target(&app, &f).unwrap();
    // At the negative bound: the rebase stores the current phase and the travel it changed at,
    // so the next frame continues instead of rejecting a baseline outside the published range.
    f.insert("phase".into(), json!([-MAX_PHASE, 0.0]));
    drag(&mut f, true, [0.0, 0.0], &target);
    drag(&mut f, false, [100.0, 0.0], &target);
    assert_eq!(f["phase"], json!([-MAX_PHASE + 100.0, 0.0]));
    f.insert("phase".into(), json!([-MAX_PHASE, 0.0]));
    drag(&mut f, false, [200.0, 0.0], &target);
    assert_eq!(f["phase"], json!([-MAX_PHASE, 0.0]), "the external edit is kept");
    assert_finite_state(&f);
    drag(&mut f, false, [250.0, 0.0], &target);
    assert_eq!(f["phase"], json!([-MAX_PHASE + 50.0, 0.0]), "and the drag continues from it");
    assert_finite_state(&f);
    // The same at the positive bound.
    end_phase_drag(&mut f);
    f.insert("phase".into(), json!([MAX_PHASE, 0.0]));
    drag(&mut f, true, [0.0, 0.0], &target);
    drag(&mut f, false, [-100.0, 0.0], &target);
    assert_eq!(f["phase"], json!([MAX_PHASE - 100.0, 0.0]));
    f.insert("phase".into(), json!([MAX_PHASE, 0.0]));
    drag(&mut f, false, [-200.0, 0.0], &target);
    assert_eq!(f["phase"], json!([MAX_PHASE, 0.0]), "the external edit is kept");
    drag(&mut f, false, [-250.0, 0.0], &target);
    assert_eq!(f["phase"], json!([MAX_PHASE - 50.0, 0.0]), "and the drag continues from it");
    assert_finite_state(&f);
    // A huge finite travel clamps instead of storing an infinity.
    end_phase_drag(&mut f);
    f.insert("phase".into(), json!([0.0, 0.0]));
    drag(&mut f, true, [0.0, 0.0], &target);
    drag(&mut f, false, [1.0e300, -1.0e300], &target);
    assert_eq!(f["phase"], json!([MAX_PHASE, -MAX_PHASE]));
    assert_finite_state(&f);
}

#[test]
fn a_release_that_stays_past_the_bound_keeps_the_bound() {
    let mut app = app();
    app.run(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap();
    let dialog = open(&mut app).unwrap();
    let mut f = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    let target = drag_target(&app, &f).unwrap();
    // The travel passed the bound; a release slightly back still sits past it, so the phase stays
    // on the bound instead of moving off it from the last shown value.
    f.insert("phase".into(), json!([0.0, 0.0]));
    drag(&mut f, true, [0.0, 0.0], &target);
    drag(&mut f, false, [MAX_PHASE + 1000.0, 0.0], &target);
    assert_eq!(f["phase"], json!([MAX_PHASE, 0.0]));
    release_phase_drag(&mut f, [-500.0, 0.0], &target);
    assert_eq!(f["phase"], json!([MAX_PHASE, 0.0]), "the total is still past the bound");
    assert!(!f.contains_key(DRAG_KEY));
    // An overflowing release delta writes nothing and still ends the gesture.
    f.insert("phase".into(), json!([0.0, 0.0]));
    drag(&mut f, true, [0.0, 0.0], &target);
    drag(&mut f, false, [f64::MAX, 0.0], &target);
    assert_eq!(f["phase"], json!([MAX_PHASE, 0.0]));
    release_phase_drag(&mut f, [f64::MAX, 0.0], &target);
    assert_eq!(f["phase"], json!([MAX_PHASE, 0.0]), "an infinite total is not published");
    assert!(!f.contains_key(DRAG_KEY));
}

#[test]
fn non_finite_motion_ends_an_active_drag_and_never_resumes_it() {
    let mut app = app();
    app.run(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap();
    let dialog = open(&mut app).unwrap();
    let mut f = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    let target = drag_target(&app, &f).unwrap();
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        end_phase_drag(&mut f);
        f.insert("phase".into(), json!([1.0, 2.0]));
        drag(&mut f, true, [0.0, 0.0], &target);
        drag(&mut f, false, [2.0, 0.0], &target);
        assert_eq!(f["phase"], json!([3.0, 2.0]), "the gesture is active");
        let phase = f["phase"].clone();
        drag(&mut f, false, [bad, 0.0], &target);
        assert_eq!(f["phase"], phase, "invalid travel writes nothing");
        assert!(!f.contains_key(DRAG_KEY), "and ends the gesture");
        drag(&mut f, false, [5.0, 0.0], &target);
        assert_eq!(f["phase"], phase, "the same held press cannot restart");
        assert!(!f.contains_key(DRAG_KEY));
    }
}

#[test]
fn a_new_fill_dialog_rejects_a_changed_insertion_layer() {
    let mut app = app();
    // A selected empty pixel layer is what the new fill would replace.
    app.run("layer.new.layer", json!({})).unwrap();
    let insert = app.session.active().unwrap().active_layer.unwrap();
    let background = app.session.active().unwrap().doc.layers[0].id;
    let dialog = open_new(&mut app).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let count = steps(&app);
    let fields = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    let target = drag_target(&app, &fields).unwrap();
    let mut dragged = fields.clone();
    drag_phase(&mut dragged, true, [0.0, 0.0], [0.0, 0.0], &target);
    drag_phase(&mut dragged, false, [2.0, -1.0], [2.0, -1.0], &target);
    assert_eq!(dragged["phase"], json!([2.0, -1.0]));
    app.ui.dialog_mut(dialog).unwrap().fields = dragged;
    assert!(display_doc(&mut app, 0).is_some(), "the preview is live");
    // Selecting another layer under the dialog changes what the command would insert above.
    app.run("layer.select", json!({"layer": background.0})).unwrap();
    let fields = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    assert!(drag_target(&app, &fields).is_none(), "a changed insertion layer is stale");
    assert!(display_doc(&mut app, 0).is_none(), "no preview on the wrong insertion layer");
    let err = crate::dialogs::confirm(&mut app, dialog).unwrap_err();
    assert!(err.contains("above changed"), "{err}");
    let st = app.session.active().unwrap();
    assert_eq!(*st.doc, *before, "a stale OK converts or creates nothing");
    assert_eq!(st.history.entries().len(), count, "and records no History step");
    assert_eq!(st.active_layer, Some(background), "the selected layer stays selected");
    // The empty layer selected when the dialog opened is untouched.
    assert!(matches!(&st.doc.layer(insert).unwrap().content, LayerContent::Raster(_)));
    // Reopening at the new active layer is a valid insertion context.
    let dialog = open_new(&mut app).unwrap();
    let fields = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    assert!(drag_target(&app, &fields).is_some());
    assert!(display_doc(&mut app, 0).is_some());
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.history.entries().len(), count + 1);
    assert_eq!(st.doc.layers.len(), before.layers.len() + 1, "a new fill is created above the background");
    assert!(matches!(&st.doc.layer(insert).unwrap().content, LayerContent::Raster(_)), "the old insertion layer is untouched");
    // No active layer is a valid insertion context too.
    let count = app.session.active().unwrap().doc.layers.len();
    app.session.active_mut().unwrap().active_layer = None;
    let dialog = open_new(&mut app).unwrap();
    let fields = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    assert!(drag_target(&app, &fields).is_some());
    assert!(display_doc(&mut app, 0).is_some());
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), count + 1);
}

#[test]
fn a_replaced_or_deleted_edit_target_is_not_previewed_or_committed() {
    let mut app = app();
    let id = LayerId(app.run(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap()["layer"].as_u64().unwrap());
    let dialog = open(&mut app).unwrap();
    set(&mut app, dialog, json!({"phase": [3.0, 4.0]}));
    assert!(display_doc(&mut app, 0).is_some());
    let count = steps(&app);
    // Rasterizing replaces the Pattern Fill with pixels: the dialog's target no longer exists.
    app.run("layer.rasterize.layer", json!({"layer": id})).unwrap();
    let replaced = app.session.active().unwrap().doc.clone();
    let rasterized = count + 1;
    let fields = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    assert!(drag_target(&app, &fields).is_none());
    assert!(display_doc(&mut app, 0).is_none());
    assert!(crate::dialogs::confirm(&mut app, dialog).is_err());
    assert_eq!(*app.session.active().unwrap().doc, *replaced);
    assert_eq!(steps(&app), rasterized, "a stale OK adds no History step");
    // Deleting the fill is stale too.
    let id = LayerId(app.run(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap()["layer"].as_u64().unwrap());
    let dialog = open(&mut app).unwrap();
    set(&mut app, dialog, json!({"phase": [5.0, 6.0]}));
    app.run("layer.delete", json!({"layer": id})).unwrap();
    let deleted = app.session.active().unwrap().doc.clone();
    let count = steps(&app);
    let fields = app.ui.dialog_mut(dialog).unwrap().fields.clone();
    assert!(drag_target(&app, &fields).is_none());
    assert!(display_doc(&mut app, 0).is_none());
    assert!(crate::dialogs::confirm(&mut app, dialog).is_err());
    assert_eq!(*app.session.active().unwrap().doc, *deleted);
    assert_eq!(steps(&app), count, "a stale OK adds no History step");
}

fn drag_screen(h: &mut Harness<'_, PhotocraftApp>, from: Pos2, delta: egui::Vec2) {
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerMoved(from + delta));
    h.step();
    h.event(Event::PointerButton { pos: from + delta, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// A window-sized document with a linked Pattern Fill dialog open over its canvas.
fn canvas_harness() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    s.execute(cmds::CREATE, json!({"pattern": "Checkerboard"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    let dialog = open(h.state_mut()).unwrap();
    h.run_steps(4);
    (h, dialog)
}

#[test]
fn a_canvas_drag_moves_the_linked_pattern_but_not_from_the_dialog() {
    let (mut h, dialog) = canvas_harness();
    let phase = |h: &Harness<'_, PhotocraftApp>| h.state().ui.dialogs.iter().find(|d| d.id == dialog).unwrap().fields["phase"].clone();
    let before = phase(&h);
    let canvas = h.state().last_canvas_rect;
    let xf = crate::canvas::ViewXform::active(h.state()).unwrap();
    // A drag begun on the dialog is the dialog's, not the canvas's.
    let snap = h.get_by_role_and_label(egui::accesskit::Role::Button, "Snap to Origin").rect().center();
    assert!(canvas.contains(snap), "the dialog is over the canvas");
    h.hover_at(snap);
    h.run_steps(1);
    assert!(crate::dialogs::free_pointer_over(&h.ctx, canvas).is_none());
    drag_screen(&mut h, snap, vec2(30.0, 20.0));
    assert_eq!(phase(&h), before, "a press on the dialog does not move the pattern");
    // A drag on the free canvas moves it by the same document distance.
    let start = xf.to_screen(20.0, 20.0);
    h.hover_at(start);
    h.run_steps(1);
    assert!(crate::dialogs::free_pointer_over(&h.ctx, canvas).is_some(), "a free canvas point");
    drag_screen(&mut h, start, xf.map_vec(vec2(12.0, -8.0)));
    let got = phase(&h);
    let a = got.as_array().unwrap();
    assert!((a[0].as_f64().unwrap() - 12.0).abs() < 1e-4 && (a[1].as_f64().unwrap() + 8.0).abs() < 1e-4, "{got}");
}

#[test]
fn an_occluded_release_or_a_dialog_press_does_not_move_the_pattern() {
    let (mut h, dialog) = canvas_harness();
    let phase = |h: &Harness<'_, PhotocraftApp>| h.state().ui.dialogs.iter().find(|d| d.id == dialog).unwrap().fields["phase"].clone();
    let canvas = h.state().last_canvas_rect;
    let xf = crate::canvas::ViewXform::active(h.state()).unwrap();
    let start = xf.to_screen(20.0, 20.0);
    let mid = start + xf.map_vec(vec2(10.0, 0.0));
    let elsewhere = start + xf.map_vec(vec2(24.0, 0.0));
    let snap = h.get_by_role_and_label(egui::accesskit::Role::Button, "Snap to Origin").rect().center();
    h.hover_at(elsewhere);
    h.run_steps(1);
    assert!(crate::dialogs::free_pointer_over(&h.ctx, canvas).is_some(), "the hover point is free canvas");
    h.hover_at(start);
    h.run_steps(1);
    assert!(crate::dialogs::free_pointer_over(&h.ctx, canvas).is_some());
    h.event(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerMoved(mid));
    h.step();
    let held = phase(&h);
    let a = held.as_array().unwrap();
    assert!((a[0].as_f64().unwrap() - 10.0).abs() < 1e-3 && a[1].as_f64().unwrap().abs() < 1e-3, "the drag previewed its own travel: {held}");
    // Release under the dialog while the pointer moves back to the free canvas in the same frame:
    // the release position, not the hover, decides.
    h.input_mut().events.extend([
        Event::PointerButton { pos: snap, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
        Event::PointerMoved(elsewhere),
    ]);
    h.step();
    h.run_steps(1);
    assert_eq!(phase(&h), held, "an occluded release applies nothing");
    // A press that begins on the dialog and is released on the free canvas starts no gesture.
    h.hover_at(snap);
    h.step();
    h.event(Event::PointerButton { pos: snap, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerMoved(elsewhere));
    h.step();
    h.event(Event::PointerButton { pos: elsewhere, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(phase(&h), held, "a dialog press cannot become a canvas drag");
}
