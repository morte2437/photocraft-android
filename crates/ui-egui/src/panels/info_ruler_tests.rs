//! #11991: with the Ruler tool active and a ruler drawn, the Info panel shows A and L (A, L1 and
//! L2 for a protractor), with X/Y the ruler's start point (a protractor's second arm end) and W/H
//! its size, in ruler units and never in the measurement scale.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::{RulerRows, ruler_rows};
use crate::PhotocraftApp;
use crate::state::Tool;

/// An 800 × 600 document at 72 ppi with the Ruler tool active and rulers in pixels.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "resolution": 72})).unwrap();
    app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "pixels"})).unwrap();
    app.ui.tool = Tool::Ruler;
    app
}

fn measure(rows: &RulerRows) -> Vec<(&str, &str)> {
    rows.measure.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

#[test]
fn a_plain_ruler_shows_a_l_and_its_start_and_size() {
    let mut app = app();
    app.run("image.analysis.rulerTool", json!({"start": [100, 100], "end": [300, 250]})).unwrap();
    let r = ruler_rows(&app).expect("the ruler cells");
    assert_eq!(measure(&r), [("A:", "-36.9°"), ("L:", "250.00")]);
    assert_eq!((r.point, r.size), ([100.0, 100.0], Some([200.0, 150.0])));
}

#[test]
fn a_protractor_shows_a_l1_l2_and_its_second_arm_end() {
    let mut app = app();
    app.run("image.analysis.rulerTool", json!({"start": [400, 300], "end": [600, 300], "protractor": [500, 200]})).unwrap();
    let r = ruler_rows(&app).expect("the ruler cells");
    assert_eq!(measure(&r), [("A:", "45.0°"), ("L1:", "200.00"), ("L2:", "141.42")]);
    assert_eq!((r.point, r.size), ([500.0, 200.0], None));
}

#[test]
fn the_measurement_scale_never_changes_them() {
    let mut app = app();
    app.run("image.analysis.rulerTool", json!({"start": [100, 100], "end": [300, 250]})).unwrap();
    app.run("image.analysis.setMeasurementScale", json!({"pixelLength": 100, "logicalLength": 2, "units": "cm"})).unwrap();
    assert!((app.session.active().unwrap().doc.measurement.scale.factor() - 0.02).abs() < 1e-9, "the custom scale is set");
    app.ui.analysis.use_measurement_scale = true;
    let r = ruler_rows(&app).expect("the ruler cells");
    assert_eq!(measure(&r), [("A:", "-36.9°"), ("L:", "250.00")]);
    assert_eq!(r.size, Some([200.0, 150.0]));
}

#[test]
fn another_tool_or_no_ruler_reads_as_today() {
    let mut app = app();
    assert_eq!(ruler_rows(&app), None, "no ruler");
    app.run("image.analysis.rulerTool", json!({"start": [100, 100], "end": [300, 250]})).unwrap();
    app.ui.tool = Tool::Brush;
    assert_eq!(ruler_rows(&app), None, "another tool");
}

#[test]
fn the_panel_draws_the_cells() {
    let mut a = app();
    a.run("image.analysis.rulerTool", json!({"start": [100, 100], "end": [300, 250]})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(320.0, 400.0)).build_ui_state(|ui, app| super::info_panel(app, ui), a);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    for text in ["A:", "-36.9°", "L:", "250.00", "W:", "200", "H:", "150"] {
        assert!(h.query_all_by_label(text).next().is_some(), "{text}");
    }
    // X and Y are the start point, 100 each.
    assert!(h.query_all_by_label("100").count() >= 2, "X 100 and Y 100");
}
