//! #12830: a Freeform Pen stroke closes only when it is released within 3 screen px of its start
//! on each axis, as in Photoshop. Other zoom levels keep the screen-based scaling.

use egui::Modifiers;
use photocraft_doc::vector::Subpath;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The measured document: 800 × 600 px with the Freeform Pen in Path mode at `zoom`.
fn app(zoom: f32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = zoom;
    app.ui.tool = Tool::FreeformPen;
    app.ui.tool_options.vector_mode = "path".into();
    app
}

/// The measured stroke: a circle around (600, 300) of radius 100 from (700, 300) in 60 moves,
/// released `off` document px from its start. Returns the stroke's subpath.
fn circle(app: &mut PhotocraftApp, off: [f64; 2]) -> Subpath {
    let mods = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: 700.0, y: 300.0, pressure: 1.0 }, mods);
    for i in 1..60 {
        let a = std::f64::consts::TAU * f64::from(i) / 60.0;
        tool_event(app, ToolEvent::Move { x: 600.0 + 100.0 * a.cos(), y: 300.0 + 100.0 * a.sin(), pressure: 1.0 }, mods);
    }
    let [x, y] = [700.0 + off[0], 300.0 + off[1]];
    tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x, y }, mods);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.undo_label(), Some("Freeform Pen"), "one History step");
    st.doc.work_path.as_ref().unwrap().subpaths.last().unwrap().clone()
}

/// The measured cases at 100 %: within ±3 px on each axis (a square) closes, 4 px off does not.
#[test]
fn at_100_percent_a_release_within_3_px_on_each_axis_closes_the_circle() {
    for off in [[0.0, 0.0], [3.0, 0.0], [0.0, -3.0], [3.0, 3.0], [-3.0, 3.0]] {
        let mut app = app(1.0);
        let n = app.session.active().unwrap().history.past_len();
        assert!(circle(&mut app, off).closed, "released {off:?} off the start closes");
        assert_eq!(app.session.active().unwrap().history.past_len(), n + 1);
    }
    for off in [[4.0, 0.0], [0.0, 4.0], [5.0, 0.0], [4.0, 4.0], [5.0, 3.0], [-4.0, 0.0]] {
        assert!(!circle(&mut app(1.0), off).closed, "released {off:?} off the start stays open");
    }
}

/// Not measured, kept as today: the box scales with zoom like the old reach, 3 screen px.
#[test]
fn at_other_zooms_the_3_px_box_is_in_screen_pixels() {
    for (zoom, inside, outside) in [(2.0, [1.5, 1.5], [2.0, 0.0]), (0.5, [6.0, 6.0], [0.0, 8.0])] {
        assert!(circle(&mut app(zoom), inside).closed, "{inside:?} off at {zoom}× closes");
        assert!(!circle(&mut app(zoom), outside).closed, "{outside:?} off at {zoom}× stays open");
    }
}

/// Not measured: a trace with ⌥-clicked straight segments closes like a freehand one. It ends in
/// a freehand run released near the start: an ⌥-click there would be the outline's own grab.
#[test]
fn a_trace_with_straight_segments_closes_within_the_same_box() {
    for (off, closed) in [([3.0, 3.0], true), ([4.0, 0.0], false)] {
        let mut app = app(1.0);
        for [x, y] in [[100.0, 100.0], [300.0, 100.0], [300.0, 300.0]] {
            tool_event(&mut app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::ALT);
            tool_event(&mut app, ToolEvent::Up { x, y }, Modifiers::ALT);
        }
        tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 300.0, pressure: 1.0 }, Modifiers::ALT);
        let end = [100.0 + off[0], 100.0 + off[1]];
        for i in 1..=10 {
            let y = 300.0 + (end[1] - 300.0) * f64::from(i) / 10.0;
            tool_event(&mut app, ToolEvent::Move { x: 100.0 + off[0] * f64::from(i) / 10.0, y, pressure: 1.0 }, Modifiers::ALT);
        }
        tool_event(&mut app, ToolEvent::Up { x: end[0], y: end[1] }, Modifiers::NONE);
        let st = app.session.active().unwrap();
        assert_eq!(st.history.undo_label(), Some("Freeform Pen"), "one History step");
        let sub = &st.doc.work_path.as_ref().unwrap().subpaths[0];
        assert_eq!(sub.knots[0].out_ctrl, sub.knots[0].anchor, "the first segment is straight");
        assert_eq!(sub.closed, closed, "released {off:?} off the start");
    }
}

/// The release decides: a short last run that fitting shortens past the box still closes.
#[test]
fn a_release_in_the_box_closes_even_when_fitting_moves_the_last_anchor() {
    let trace = [[100.0, 100.0], [200.0, 100.0], [200.0, 200.0], [103.01, 100.0], [103.0, 100.0]];
    let (pen, closed) = super::fit_with_straight_segments(&trace, &[1, 2, 3], 2.0, true);
    assert!(closed);
    assert_eq!(pen.knots.len(), 3, "the last anchor became the first one");
}
