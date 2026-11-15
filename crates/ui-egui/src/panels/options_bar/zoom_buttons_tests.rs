//! #12068: the Zoom tool's bar shows 100%, Fit Screen and Fill Screen in that order, with the
//! reference tooltips, and no hint text.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn zoom_bar() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 3000, "height": 2000})).unwrap();
    // The bar's fonts are installed after the first frame, so it starts on a tool without "100%".
    app.ui.tool = Tool::Move;
    let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.state_mut().ui.tool = Tool::Zoom;
    h.run_steps(4);
    h
}

#[test]
fn the_buttons_are_100_fit_and_fill_in_that_order() {
    let h = zoom_bar();
    let x = |label: &str| h.get_by_label(label).rect().center().x;
    assert!(x("Scrubby Zoom") < x("100%"), "after the checkboxes");
    assert!(x("100%") < x("Fit Screen") && x("Fit Screen") < x("Fill Screen"));
}

#[test]
fn each_control_has_the_reference_tooltip() {
    for (label, tip) in [
        ("100%", "Zoom current window to 1:1"),
        ("Fit Screen", "Zoom current window to screen size"),
        ("Fill Screen", "Zoom current window to fill the screen"),
        ("Resize Windows to Fit", "Resize windows when zooming using the Zoom Tool"),
        ("Scrubby Zoom", "Click and move left or right to scale using the Zoom Tool (OpenGL only)"),
    ] {
        let mut h = zoom_bar();
        h.get_by_label(label).hover();
        h.run_steps(60);
        assert!(h.query_by_label(tip).is_some(), "{label}: no tooltip {tip:?}");
    }
}

#[test]
fn the_bar_shows_no_hint_text() {
    let h = zoom_bar();
    assert!(h.query_by_label_contains("to zoom out").is_none());
    assert!(h.query_by_label_contains("Click to zoom in").is_none());
}

#[test]
fn fill_screen_fills_as_on_the_hand_bar() {
    let mut h = zoom_bar();
    h.get_by_label("Fit Screen").click();
    h.run_steps(2);
    assert!(h.state().ui.views[0].fit_pending);
    h.get_by_label("Fill Screen").click();
    h.run_steps(2);
    let v = &h.state().ui.views[0];
    assert!(v.fill_pending && !v.fit_pending);
    h.get_by_label("100%").click();
    h.run_steps(2);
    let v = &h.state().ui.views[0];
    assert!(!v.fill_pending && !v.fit_pending && v.zoom == 1.0);
}
