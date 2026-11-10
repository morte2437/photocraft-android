//! #12026: the Hand tool's 100%, Fit Screen and Fill Screen buttons have the reference tooltips,
//! and the bar shows no hint text.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn hand_bar() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 3000, "height": 2000})).unwrap();
    // The bar's fonts are installed after the first frame, so it starts on a tool without "100%".
    app.ui.tool = Tool::Move;
    let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app| {
            // The labels and tooltips are checked in English, whatever this thread drew in before.
            let _language = crate::i18n::language_scope(crate::i18n::Lang::EN);
            crate::panels::options_bar(app, ui)
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.state_mut().ui.tool = Tool::Hand;
    h.run_steps(4);
    h
}

#[test]
fn each_button_has_the_reference_tooltip() {
    for (label, tip) in [
        ("100%", "Zoom current window to 1:1"),
        ("Fit Screen", "Zoom current window to screen size"),
        ("Fill Screen", "Zoom current window to fill the screen"),
    ] {
        let mut h = hand_bar();
        assert!(h.query_by_label(tip).is_none(), "{label}: no tooltip before hovering");
        h.get_by_label(label).hover();
        h.run_steps(60);
        assert!(h.query_by_label(tip).is_some(), "{label}: no tooltip {tip:?}");
    }
}

#[test]
fn the_bar_shows_no_hint_text() {
    let h = hand_bar();
    assert!(h.query_by_label("100%").is_some(), "the Hand bar is drawn");
    assert!(h.query_by_label_contains("Drag to pan").is_none());
    assert!(h.query_by_label_contains("hold Space").is_none());
}

#[test]
fn the_buttons_still_zoom_fit_and_fill() {
    let mut h = hand_bar();
    h.state_mut().ui.views[0].zoom = 0.25;
    h.get_by_label("Fit Screen").click();
    h.run_steps(2);
    assert!(h.state().ui.views[0].fit_pending);
    h.get_by_label("Fill Screen").click();
    h.run_steps(2);
    let v = &h.state().ui.views[0];
    assert!(v.fill_pending && !v.fit_pending);
    h.get_by_label("Fit Screen").click();
    h.run_steps(2);
    let v = &h.state().ui.views[0];
    assert!(v.fit_pending && !v.fill_pending, "Fit after Fill");
    h.get_by_label("Fill Screen").click();
    h.run_steps(2);
    assert!(h.state().ui.views[0].fill_pending);
    h.get_by_label("100%").click();
    h.run_steps(2);
    let v = &h.state().ui.views[0];
    assert!(!v.fill_pending && !v.fit_pending && v.zoom == 1.0);
}
