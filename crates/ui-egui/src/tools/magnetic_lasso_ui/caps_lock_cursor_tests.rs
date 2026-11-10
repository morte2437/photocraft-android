//! Caps Lock and the Magnetic Lasso's cursor (#11690): it swaps Preferences › Cursors › Other
//! Cursors, so with Standard and Caps Lock on the pointer shows a circle as big as the detection
//! Width (scaled with zoom), and with Precise and Caps Lock on it is the tool's icon, as with
//! Standard alone.

use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn app(other: &str, caps_lock: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.ui.tool = Tool::MagneticLasso;
    app.ui.tool_options.magnetic_width = 10.0;
    app.run("prefs.set", json!({"path": "cursors.other", "value": other})).unwrap();
    app.caps_lock = caps_lock;
    app
}

/// The radii of the circles `cursor` draws at `zoom`, and the OS cursor it asks for.
fn drawn(app: &PhotocraftApp, zoom: f32) -> (Vec<f32>, egui::CursorIcon) {
    let (radii, icon, _) = drawn_with_others(app, zoom);
    (radii, icon)
}

/// As [`drawn`], plus how many shapes that are not circles it draws (the icon).
fn drawn_with_others(app: &PhotocraftApp, zoom: f32) -> (Vec<f32>, egui::CursorIcon, usize) {
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput::default());
    let painter = ctx.layer_painter(egui::LayerId::background());
    let icon = super::cursor(app, &painter, egui::pos2(100.0, 100.0), zoom);
    let mut out = ctx.end_pass();
    out.textures_delta.clear();
    let radii: Vec<f32> = out
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Circle(c) => Some(c.radius),
            _ => None,
        })
        .collect();
    let others = out.shapes.len() - radii.len();
    (radii, icon, others)
}

#[test]
fn caps_lock_on_standard_draws_the_width_circle_scaled_with_zoom() {
    let app = app("standard", true);
    let (radii, icon) = drawn(&app, 1.0);
    assert!(radii.iter().any(|r| (r - 10.0).abs() < 1e-3), "a 10 px circle at Width 10, 100 %: {radii:?}");
    assert_eq!(icon, egui::CursorIcon::Crosshair, "with a crosshair in the middle");
    let (radii, _) = drawn(&app, 2.0);
    assert!(radii.iter().any(|r| (r - 20.0).abs() < 1e-3), "twice as big at 200 %: {radii:?}");
}

#[test]
fn caps_lock_on_precise_swaps_back_to_the_tool_icon() {
    let (radii, icon, others) = drawn_with_others(&app("precise", true), 1.0);
    assert!(radii.is_empty(), "no width circle: {radii:?}");
    assert_eq!(icon, egui::CursorIcon::None, "the OS cursor hides behind the icon");
    assert!(others > 0, "the tool icon is drawn");
}

#[test]
fn without_caps_lock_the_preference_decides() {
    let (radii, icon, others) = drawn_with_others(&app("standard", false), 1.0);
    assert!(radii.is_empty(), "Standard: no circle");
    assert_eq!(icon, egui::CursorIcon::None, "Standard: the tool icon, not a crosshair");
    assert!(others > 0, "Standard: the tool icon is drawn");
    let (radii, icon) = drawn(&app("precise", false), 1.0);
    assert!(!radii.is_empty(), "Precise: the width circle");
    assert_eq!(icon, egui::CursorIcon::Crosshair, "Precise: with the crosshair");
}
