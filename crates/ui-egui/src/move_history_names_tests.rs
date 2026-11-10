//! #11383: the Move tool's History steps have the reference application's names: a drag and an
//! Alt+drag copy of a layer are "Move", arrow-key nudges are "Nudge" (one per press, or one per
//! run with Bundle Arrow-Key Nudges on), an Alt+drag of selected pixels is "Duplicate". An
//! Alt+arrow nudge keeps "Duplicate + Move" (not measured).

use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// A 64x64 document with one painted layer (8..24)^2, selected, and the Move tool active.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(8, 8, 24, 24), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = false;
    // Snapping (to the document centre with a 1:1 test view) would shift the expected offsets.
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app
}

fn bounds(app: &PhotocraftApp, id: photocraft_doc::LayerId) -> Rect {
    app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds()
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], by: [f64; 2], mods: egui::Modifiers) {
    crate::canvas::tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    let mid = [from[0] + by[0] / 2.0, from[1] + by[1] / 2.0];
    crate::canvas::tool_event(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, mods);
    let end = [from[0] + by[0], from[1] + by[1]];
    crate::canvas::tool_event(app, ToolEvent::Move { x: end[0], y: end[1], pressure: 1.0 }, mods);
    crate::canvas::tool_event(app, ToolEvent::Up { x: end[0], y: end[1] }, mods);
}

fn press(app: &mut PhotocraftApp, key: egui::Key, mods: egui::Modifiers, time: f64) {
    let ctx = egui::Context::default();
    ctx.input_mut(|i| {
        i.time = time;
        i.modifiers = mods;
        i.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: mods });
    });
    assert!(crate::move_mods::arrow_keys(app, &ctx));
}

fn history(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn newest(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.entries().pop()
}

/// The five presses of the reference measurement: ->, Shift+->, down, Shift+down, <-.
fn five_presses(app: &mut PhotocraftApp) {
    let keys = [
        (egui::Key::ArrowRight, egui::Modifiers::NONE),
        (egui::Key::ArrowRight, egui::Modifiers::SHIFT),
        (egui::Key::ArrowDown, egui::Modifiers::NONE),
        (egui::Key::ArrowDown, egui::Modifiers::SHIFT),
        (egui::Key::ArrowLeft, egui::Modifiers::NONE),
    ];
    for (i, (key, mods)) in keys.into_iter().enumerate() {
        press(app, key, mods, 1.0 + 0.1 * i as f64);
    }
}

#[test]
fn a_drag_of_a_layer_is_one_move_step() {
    let mut app = app();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let h0 = history(&app);
    drag(&mut app, [16.0, 16.0], [10.0, 4.0], egui::Modifiers::NONE);
    assert_eq!(bounds(&app, id), Rect::new(18, 12, 34, 28));
    assert_eq!(history(&app), h0 + 1);
    assert_eq!(newest(&app).as_deref(), Some("Move"));
}

#[test]
fn an_alt_drag_copy_of_a_layer_is_one_move_step() {
    let mut app = app();
    let orig = app.session.active().unwrap().active_layer.unwrap();
    let h0 = history(&app);
    drag(&mut app, [16.0, 16.0], [20.0, 10.0], egui::Modifiers::ALT);
    let copy = app.session.active().unwrap().active_layer.unwrap();
    assert_ne!(copy, orig, "the copy is selected");
    assert_eq!(bounds(&app, copy), Rect::new(28, 18, 44, 34), "at the offset");
    assert_eq!(bounds(&app, orig), Rect::new(8, 8, 24, 24), "the original stays");
    assert_eq!(history(&app), h0 + 1, "the copy and the move are one step");
    assert_eq!(newest(&app).as_deref(), Some("Move"));
}

#[test]
fn five_arrow_presses_with_bundling_off_are_five_nudge_steps() {
    let mut app = app();
    app.session.edit_prefs(|p| p.tools.bundle_nudges = false);
    let id = app.session.active().unwrap().active_layer.unwrap();
    let h0 = history(&app);
    five_presses(&mut app);
    assert_eq!(bounds(&app, id), Rect::new(8 + 1 + 10 - 1, 8 + 1 + 10, 24 + 10, 24 + 11));
    assert_eq!(history(&app), h0 + 5);
    let entries = app.session.active().unwrap().history.entries();
    assert_eq!(&entries[entries.len() - 5..], ["Nudge"; 5]);
}

#[test]
fn five_arrow_presses_with_bundling_on_are_one_nudge_step() {
    let mut app = app();
    app.session.edit_prefs(|p| p.tools.bundle_nudges = true);
    let h0 = history(&app);
    five_presses(&mut app);
    assert_eq!(history(&app), h0 + 1);
    assert_eq!(newest(&app).as_deref(), Some("Nudge"));
}

#[test]
fn an_alt_drag_of_selected_pixels_is_one_duplicate_step() {
    let mut app = app();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let h0 = history(&app);
    drag(&mut app, [16.0, 16.0], [20.0, 10.0], egui::Modifiers::ALT);
    // The copied piece floats until something drops it.
    app.session.execute("select.drop", json!({})).unwrap();
    assert_eq!(history(&app), h0 + 1, "the copy and its move are one step");
    assert_eq!(newest(&app).as_deref(), Some("Duplicate"));
}

#[test]
fn a_blank_or_overlong_step_name_drops_as_move() {
    let mut app = app();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    for name in ["  ", &"x".repeat(65)] {
        app.session.execute("select.float", json!({"dx": 2, "dy": 0, "copy": true, "stepName": name})).unwrap();
        app.session.execute("select.drop", json!({})).unwrap();
        assert_eq!(newest(&app).as_deref(), Some("Move"), "{name:?}");
    }
    app.session.execute("select.float", json!({"dx": 2, "dy": 0, "copy": true, "stepName": "Duplicate"})).unwrap();
    app.session.execute("select.drop", json!({})).unwrap();
    assert_eq!(newest(&app).as_deref(), Some("Duplicate"));
}

#[test]
fn an_alt_arrow_nudge_keeps_duplicate_plus_move() {
    let mut app = app();
    let h0 = history(&app);
    press(&mut app, egui::Key::ArrowRight, egui::Modifiers::ALT, 1.0);
    assert_eq!(history(&app), h0 + 1);
    assert_eq!(newest(&app).as_deref(), Some("Duplicate + Move"));
}

/// A zero offset moves nothing: no step is added, and the step on top (here the paint) keeps its
/// name.
#[test]
fn a_zero_nudge_renames_no_earlier_step() {
    let mut app = app();
    for bundle in [false, true] {
        app.session.edit_prefs(|p| p.tools.bundle_nudges = bundle);
        let before: Vec<String> = app.session.active().unwrap().history.entries().to_vec();
        crate::move_mods::nudge(&mut app, 0.0, 0.0, false);
        let after: Vec<String> = app.session.active().unwrap().history.entries().to_vec();
        assert_eq!(after, before, "bundle {bundle}");
    }
}
