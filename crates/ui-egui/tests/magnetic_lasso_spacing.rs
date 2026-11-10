//! #11687: the Magnetic Lasso drops its automatic fastening points as far apart as the reference
//! application does: with the button up about 4 x the stored spacing + 4 screen px, with the
//! button held the stored spacing itself, which is `floor(40 - 0.35 x Frequency)`.
//!
//! Driven through the public `tool_event` entry (the one the mouse and `ui.pointer` share) on a
//! straight edge at 100 % zoom; the spacing is the distance between consecutive fastening points.

use egui::Modifiers;
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_ui_egui::canvas::{ToolEvent, tool_event};
use photocraft_ui_egui::state::Tool;
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::json;

const W: i32 = 900;
const H: i32 = 120;
/// The straight edge: white above, black below.
const EDGE_Y: f64 = 60.0;
const START_X: f64 = 20.0;
const END_X: f64 = 880.0;

/// A white document with a black lower half, the Magnetic Lasso at `frequency`, 100 % zoom.
fn app(frequency: f32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": W, "height": H})).unwrap();
    app.sync_views();
    app.session
        .edit("paint", |doc, _| {
            let bg = doc.layers[0].surface_mut().unwrap();
            bg.fill_rect(Rect::new(0, 0, W, H), &[1.0, 1.0, 1.0, 1.0]);
            bg.fill_rect(Rect::new(0, EDGE_Y as i32, W, H), &[0.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::MagneticLasso;
    app.ui.tool_options.magnetic_frequency = frequency;
    assert!((app.point_zoom() - 1.0).abs() < 1e-6, "the test measures at 100 % zoom");
    app
}

/// Click the first point on the edge, then move along it to the far end in 2 px steps; with
/// `held` the button stays down for the whole walk, otherwise it is released after the click.
fn walk(app: &mut PhotocraftApp, held: bool) {
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: START_X, y: EDGE_Y, pressure: 1.0 }, m);
    if !held {
        tool_event(app, ToolEvent::Up { x: START_X, y: EDGE_Y }, m);
    }
    let mut x = START_X;
    while x < END_X {
        x = (x + 2.0).min(END_X);
        tool_event(app, ToolEvent::Move { x, y: EDGE_Y, pressure: 1.0 }, m);
    }
}

/// The distances between consecutive fastening points of the border so far.
fn gaps(app: &PhotocraftApp) -> Vec<f64> {
    let m = &app.ui.magnetic;
    let pts: Vec<[f64; 2]> = m.anchors.iter().map(|&i| m.path[i]).collect();
    pts.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).collect()
}

/// Every gap is within 10 % of `want`, and there are at least `min` of them.
fn check(frequency: f32, held: bool, want: f64, min: usize) {
    let mut a = app(frequency);
    walk(&mut a, held);
    let g = gaps(&a);
    let state = if held { "held" } else { "up" };
    assert!(g.len() >= min, "Frequency {frequency}, button {state}: only {} gaps {g:?}, wanted at least {min} of ~{want}", g.len());
    for (i, d) in g.iter().enumerate() {
        assert!((d - want).abs() <= want * 0.10, "Frequency {frequency}, button {state}: gap {i} is {d:.1} px, the reference measures ~{want} ({g:?})");
    }
}

#[test]
fn button_up_spacing_at_frequency_0() {
    check(0.0, false, 166.0, 4);
}

#[test]
fn button_held_spacing_at_frequency_0() {
    check(0.0, true, 39.0, 20);
}

#[test]
fn button_up_spacing_at_the_default_frequency_57() {
    check(57.0, false, 85.0, 9);
}

#[test]
fn button_held_spacing_at_the_default_frequency_57() {
    check(57.0, true, 20.0, 40);
}

#[test]
fn button_up_spacing_at_frequency_100() {
    check(100.0, false, 24.0, 30);
}

#[test]
fn button_held_spacing_at_frequency_100() {
    check(100.0, true, 5.0, 150);
}

#[test]
fn releasing_the_button_widens_the_spacing_again() {
    let mut a = app(57.0);
    let m = Modifiers::NONE;
    tool_event(&mut a, ToolEvent::Down { x: START_X, y: EDGE_Y, pressure: 1.0 }, m);
    for x in (22..=300).step_by(2) {
        tool_event(&mut a, ToolEvent::Move { x: f64::from(x), y: EDGE_Y, pressure: 1.0 }, m);
    }
    let held_points = a.ui.magnetic.anchors.len();
    tool_event(&mut a, ToolEvent::Up { x: 300.0, y: EDGE_Y }, m);
    for x in (302..=600).step_by(2) {
        tool_event(&mut a, ToolEvent::Move { x: f64::from(x), y: EDGE_Y, pressure: 1.0 }, m);
    }
    let g = gaps(&a);
    let after = &g[held_points - 1..];
    assert!(after.len() >= 2, "points keep fastening after the release: {g:?}");
    assert!(after.iter().skip(1).all(|d| (d - 85.0).abs() <= 8.5), "after the release the gaps are ~85 px: {g:?}");
}

#[test]
fn out_of_range_and_non_finite_frequencies_are_clamped() {
    // Above 100 behaves as 100, below 0 as 0, and NaN as the default 57 (button up).
    check(250.0, false, 24.0, 30);
    check(-30.0, false, 166.0, 4);
    check(f32::NAN, false, 85.0, 9);
}

/// Walks the edge like `walk`, at `zoom` (1.0 = 100 %), with the pointer moving 2 screen px a step
/// below 100 % and 4 screen px from 100 % up (coarser steps give alternating gaps at 50 %).
fn walk_at(app: &mut PhotocraftApp, held: bool, zoom: f32) {
    app.ui.views[0].zoom = zoom;
    assert!((app.point_zoom() - zoom).abs() < 1e-6, "the view is at the zoom under test");
    let m = Modifiers::NONE;
    let step = (if zoom < 1.0 { 2.0 } else { 4.0 }) / f64::from(zoom);
    tool_event(app, ToolEvent::Down { x: START_X, y: EDGE_Y, pressure: 1.0 }, m);
    if !held {
        tool_event(app, ToolEvent::Up { x: START_X, y: EDGE_Y }, m);
    }
    let mut x = START_X;
    while x < END_X {
        x = (x + step).min(END_X);
        tool_event(app, ToolEvent::Move { x, y: EDGE_Y, pressure: 1.0 }, m);
    }
}

/// Frequency 57 at `zoom`: every gap (document px) is within 10 % of `want_doc`.
fn check_at(zoom: f32, held: bool, want_doc: f64, min: usize) {
    let mut a = app(57.0);
    walk_at(&mut a, held, zoom);
    let g = gaps(&a);
    let state = if held { "held" } else { "up" };
    assert!(g.len() >= min, "zoom {zoom}, button {state}: only {} gaps {g:?}, wanted at least {min} of ~{want_doc}", g.len());
    for (i, d) in g.iter().enumerate() {
        assert!(
            (d - want_doc).abs() <= want_doc * 0.10,
            "zoom {zoom}, button {state}: gap {i} is {d:.1} document px, the reference measures ~{want_doc} ({g:?})"
        );
    }
}

#[test]
fn button_up_spacing_halves_below_100_percent_in_screen_px() {
    check_at(0.5, false, 84.0, 9); // 42 screen px
    check_at(2.0 / 3.0, false, 63.0, 12); // 42 screen px
}

#[test]
fn button_up_spacing_stays_84_screen_px_above_100_percent() {
    check_at(2.0, false, 42.0, 18);
    check_at(3.0, false, 28.0, 28);
}

#[test]
fn button_held_spacing_is_the_stored_value_in_document_px_above_100_percent() {
    check_at(2.0, true, 20.0, 40); // 40 screen px
    check_at(3.0, true, 20.0, 40); // 60 screen px
}
