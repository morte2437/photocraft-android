//! #11816: Escape in the Brush Preset picker closes it and reverts a size or hardness typed in
//! it; Enter keeps them, a preset picked before Escape stays picked, a change made outside the
//! picker stays, and each opening reverts to its own start.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 20, "hardness": 0.0}})).unwrap();
    app.ui.tool = Tool::Brush;
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h
}

fn open(h: &mut Harness<'static, PhotocraftApp>) {
    let c = h.state().last_canvas_rect.center();
    h.state_mut().ui.brush_picker = Some([c.x, c.y]);
    h.run_steps(2);
}

/// Type `text` into the picker's `n`th spin field (0 Size, 1 Hardness), then Tab.
fn type_into(h: &mut Harness<'static, PhotocraftApp>, n: usize, text: &str) {
    h.get_all_by_role(egui::accesskit::Role::SpinButton).nth(n).expect("the field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
}

fn brush(h: &Harness<'static, PhotocraftApp>) -> (f32, f32) {
    let b = &h.state().session.tools.brush;
    (b.size, b.hardness)
}

#[test]
fn escape_reverts_typed_size_and_hardness() {
    let mut h = harness();
    let steps = h.state().session.active().unwrap().history.past_len();
    open(&mut h);
    type_into(&mut h, 0, "60");
    type_into(&mut h, 1, "100");
    assert!(h.state().ui.brush_picker.is_some(), "Tab keeps the picker open");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().ui.brush_picker, None);
    assert_eq!(brush(&h), (20.0, 0.0), "back to the brush the picker opened with");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "no history step");
}

#[test]
fn enter_keeps_the_typed_values() {
    let mut h = harness();
    open(&mut h);
    type_into(&mut h, 0, "60");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().ui.brush_picker, None);
    assert_eq!(brush(&h).0, 60.0);
}

#[test]
fn a_preset_picked_before_escape_stays_picked() {
    let mut h = harness();
    open(&mut h);
    let name = h.state().session.tools.presets.get(1).map(|p| p.name.clone()).expect("a preset");
    h.get_by_label(&name).click();
    h.run_steps(3);
    let picked = brush(&h);
    assert_ne!(picked, (20.0, 0.0), "the preset changed the brush");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().ui.brush_picker, None);
    assert_eq!(brush(&h), picked);
}

#[test]
fn escape_keeps_a_size_changed_outside_the_picker() {
    let mut h = harness();
    open(&mut h);
    type_into(&mut h, 1, "100");
    // As `]` or the options bar would, while the picker stays open.
    h.state_mut().run("tools.setBrush", json!({"brush": {"size": 45}})).unwrap();
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(brush(&h), (45.0, 0.0), "the typed hardness reverts, the size from elsewhere stays");
}

#[test]
fn a_reopened_picker_reverts_to_its_own_opening() {
    let mut h = harness();
    open(&mut h);
    type_into(&mut h, 0, "60");
    // Closed, changed and opened again (a control request) before the next frame.
    let ctx = h.ctx.clone();
    let app = h.state_mut();
    crate::brush_picker::close(&mut app.ui);
    app.run("tools.setBrush", json!({"brush": {"size": 33}})).unwrap();
    let at = app.last_canvas_rect.center();
    crate::brush_picker::open(&mut app.ui, &ctx, [at.x, at.y]);
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(brush(&h).0, 33.0, "the size this opening started with");
}
