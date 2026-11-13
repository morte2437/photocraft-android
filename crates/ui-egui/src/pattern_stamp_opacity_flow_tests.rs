//! #12616: the Pattern Stamp's options bar has Opacity and Flow after Mode (whole percents 1..100,
//! default 100, the reference's tooltips, the Brush's silent clamp for a typed value past the
//! range), its strokes paint with them, live and committed alike, and Reset Tool puts both back
//! to 100.

use std::collections::BTreeSet;

use egui::accesskit::Role;
use egui::{Modifiers, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::eraser_range_alert_tests::{alerts, bar, type_into};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, display_doc, tool_event};
use crate::state::Tool;

/// The field named `label` ("Opacity", "Flow"), by its caption's name.
fn field<'a>(h: &'a Harness<'static, PhotocraftApp>, label: &str) -> egui_kittest::Node<'a> {
    let label = format!("{label}:");
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label.as_str())).unwrap_or_else(|| panic!("the {label} field"))
}

/// The live brush's Opacity and Flow in percent.
fn opacity_flow(app: &PhotocraftApp) -> (f32, f32) {
    let b = &app.session.tools.brush;
    ((b.opacity * 100.0).round(), (b.flow * 100.0).round())
}

#[test]
fn the_bar_shows_opacity_and_flow_after_mode_at_100() {
    let h = bar(Tool::PatternStamp);
    let mode = h.query_all_by_role(Role::ComboBox).find(|n| n.value().as_deref() == Some("Normal")).expect("the Mode dropdown").rect();
    let (opacity, flow) = (field(&h, "Opacity"), field(&h, "Flow"));
    assert_eq!((opacity.accesskit_node().numeric_value(), flow.accesskit_node().numeric_value()), (Some(100.0), Some(100.0)));
    assert_eq!(opacity.accesskit_node().description().as_deref(), Some("Set opacity for stroke"));
    assert_eq!(flow.accesskit_node().description().as_deref(), Some("Set flow rate for stroke"));
    let (opacity, flow) = (opacity.rect(), flow.rect());
    assert!(mode.right() <= opacity.left() && opacity.right() <= flow.left(), "Mode, Opacity, Flow: {mode:?} {opacity:?} {flow:?}");
}

#[test]
fn typed_values_are_whole_percents_clamped_to_1_100_without_an_alert() {
    let mut h = bar(Tool::PatternStamp);
    for (opacity, flow, want) in [("50", "10", (50.0, 10.0)), ("37.6", "12.4", (38.0, 12.0)), ("150", "0", (100.0, 1.0))] {
        type_into(&mut h, "Opacity", opacity);
        type_into(&mut h, "Flow", flow);
        assert_eq!(opacity_flow(h.state()), want, "typed {opacity} / {flow}");
        assert!(alerts(&h).is_empty(), "the Brush's fields clamp silently");
    }
    let b = &h.state().session.tools.brush;
    assert_eq!((b.opacity, b.flow), (1.0, 0.01), "stored as whole percents");
}

#[test]
fn reset_tool_puts_opacity_and_flow_back_to_100() {
    let mut h = bar(Tool::PatternStamp);
    type_into(&mut h, "Opacity", "40");
    type_into(&mut h, "Flow", "25");
    assert_eq!(opacity_flow(h.state()), (40.0, 25.0));
    crate::burn_default_tip_tests::invoke(h.state_mut(), "tool.reset", json!({}));
    h.run_steps(3);
    assert_eq!(opacity_flow(h.state()), (100.0, 100.0));
    assert_eq!(field(&h, "Opacity").accesskit_node().numeric_value(), Some(100.0));
    assert_eq!(field(&h, "Flow").accesskit_node().numeric_value(), Some(100.0));
}

/// The issue's test document: 800 × 600 RGB 8 bit, white with its right half green (40,160,60),
/// a 20 × 20 pattern of 10 px quadrants (red, green / blue, white) and the Pattern Stamp with a
/// 40 px hard tip, Aligned on, at `opacity` and `flow` (fractions).
fn document(opacity: f32, flow: f32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 20})).unwrap();
    for (x, y, color) in [(0, 0, "#ff0000"), (10, 0, "#00ff00"), (0, 10, "#0000ff"), (10, 10, "#ffffff")] {
        app.run("select.rect", json!({"x": x, "y": y, "width": 10, "height": 10})).unwrap();
        app.run("edit.fill", json!({"color": color})).unwrap();
    }
    app.run("select.deselect", json!({})).unwrap();
    let pattern = app.run("edit.definePattern", json!({"name": "Quadrants", "rect": [0, 0, 20, 20]})).unwrap()["pattern"].clone();
    app.run("pattern.presets.select", json!({"pattern": pattern})).unwrap();
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("select.rect", json!({"x": 400, "y": 0, "width": 400, "height": 600})).unwrap();
    app.run("edit.fill", json!({"color": "#28a03c"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::PatternStamp;
    app.ui.tool_options.pattern_stamp_aligned = true;
    crate::paint_mouse::sync_tool_brush(&mut app);
    let brush = json!({"size": 40, "hardness": 1.0, "opacity": opacity, "flow": flow, "pressureSize": false, "smoothing": {"amount": 0.0}});
    app.run("tools.setBrush", json!({"brush": brush})).unwrap();
    app
}

/// The distinct 8-bit colours inside the tip of a click at (600, 300), in `doc`.
fn colours(doc: &photocraft_doc::Document, app: &PhotocraftApp) -> BTreeSet<[u8; 3]> {
    let surf = doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap();
    let inside = Rect::from_center_size(egui::pos2(600.0, 300.0), egui::vec2(24.0, 24.0));
    let (x0, y0, x1, y1) = (inside.left() as i32, inside.top() as i32, inside.right() as i32, inside.bottom() as i32);
    (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).map(|(x, y)| surf.rgba(x, y)).map(|c| [c[0], c[1], c[2]].map(|v| (v * 255.0).round() as u8)).collect()
}

/// One click at (600, 300) on the green half: what the canvas showed before the release, and what
/// the release committed.
fn click(app: &mut PhotocraftApp) -> (BTreeSet<[u8; 3]>, BTreeSet<[u8; 3]>) {
    tool_event(app, ToolEvent::Down { x: 600.0, y: 300.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(app.live_stroke.is_some(), "the stroke draws live");
    // The canvas shows the second document; the first holds the pattern.
    let idx = app.session.active_index().unwrap();
    let shown = colours(&display_doc(app, idx).0, app);
    tool_event(app, ToolEvent::Up { x: 600.0, y: 300.0 }, Modifiers::NONE);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let committed = colours(&app.session.active().unwrap().doc, app);
    (shown, committed)
}

/// The reference's values for Opacity 50, and for Flow 50 (the same): half the pattern over green.
const HALF: [[u8; 3]; 4] = [[148, 80, 30], [20, 208, 30], [20, 80, 158], [148, 208, 158]];

#[test]
fn opacity_50_paints_the_reference_values_live_and_committed() {
    let mut app = document(0.5, 1.0);
    let (shown, committed) = click(&mut app);
    assert_eq!(committed, BTreeSet::from(HALF));
    assert_eq!(shown, committed, "the live preview and the commit agree");
    let (_, params) = app.session.journal.iter().rev().find(|(name, _)| name == "paint.patternStamp").unwrap();
    assert_eq!((params["opacity"].as_f64(), params["flow"].as_f64()), (Some(50.0), Some(100.0)), "{params}");
}

#[test]
fn flow_50_paints_the_same_values_as_opacity_50() {
    let mut app = document(1.0, 0.5);
    let (shown, committed) = click(&mut app);
    assert_eq!(committed, BTreeSet::from(HALF));
    assert_eq!(shown, committed);
}

#[test]
fn flow_10_paints_about_10_percent() {
    let mut app = document(1.0, 0.1);
    let (shown, committed) = click(&mut app);
    assert_eq!(shown, committed);
    // Red over green: the reference's 62,144,54.
    let red = committed.iter().find(|c| c[0] > c[2] && c[1] < 150).copied().expect("the red cells");
    assert!(red.iter().zip([62u8, 144, 54]).all(|(a, b)| a.abs_diff(b) <= 1), "{red:?} in {committed:?}");
}

/// A first pick doesn't carry the Brush's Opacity and Flow over: the Pattern Stamp starts at 100.
#[test]
fn a_first_pick_after_a_30_percent_brush_starts_at_100() {
    let mut h = bar(Tool::Brush);
    h.state_mut().run("tools.setBrush", json!({"brush": {"opacity": 0.3, "flow": 0.4}})).unwrap();
    h.run_steps(3);
    h.state_mut().ui.tool = Tool::PatternStamp;
    h.run_steps(4);
    assert_eq!(field(&h, "Opacity").accesskit_node().numeric_value(), Some(100.0));
    assert_eq!(field(&h, "Flow").accesskit_node().numeric_value(), Some(100.0));
    let app = h.state_mut();
    assert!(crate::retouch_ui::finish_stroke(app, Tool::PatternStamp, &[[40.0, 40.0, 1.0]], Modifiers::NONE, false));
    let (_, params) = app.session.journal.iter().rev().find(|(name, _)| name == "paint.patternStamp").unwrap();
    assert_eq!((params["opacity"].as_f64(), params["flow"].as_f64()), (Some(100.0), Some(100.0)), "{params}");
}

/// Impressionist strokes take Opacity too (not measured: only that they paint with it, live and
/// committed alike).
#[test]
fn an_impressionist_stroke_paints_with_opacity_live_and_committed() {
    let painted = |opacity: f32| {
        let mut app = document(opacity, 1.0);
        app.ui.tool_options.pattern_stamp_impressionist = true;
        let (shown, committed) = click(&mut app);
        assert_eq!(shown, committed, "opacity {opacity}: the live preview and the commit agree");
        let (_, params) = app.session.journal.iter().rev().find(|(name, _)| name == "paint.patternStamp").unwrap();
        assert_eq!(params["opacity"].as_f64(), Some(f64::from(opacity) * 100.0), "{params}");
        committed
    };
    assert_ne!(painted(0.5), painted(1.0), "Opacity 50 paints differently from 100");
}
