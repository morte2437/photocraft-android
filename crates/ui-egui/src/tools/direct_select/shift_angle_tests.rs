//! #13207: ⇧ during a Direct Selection anchor drag, as measured in the reference application on
//! the corner anchor (300, 300): a (+60, -25) drag stays on the horizontal at (360, 300), and a
//! (+50, -50) drag ends on the diagonal at (350, 250). Other angles weren't measured and keep the
//! 45° snap of #5475.

use egui::Modifiers;
use photocraft_geom::Point;
use serde_json::json;

use super::Drag;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::DirectSelection;
    let path = json!({"subpaths": [{"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app
}

/// Drags the corner (300, 300) by `by` with ⇧ held once the drag is under way (⇧ at the press
/// would toggle the anchor's selection) and returns where the preview and the result put it.
fn shift_drag(by: [f64; 2]) -> (Point, Point) {
    let mut app = app();
    let to = [300.0 + by[0], 300.0 + by[1]];
    tool_event(&mut app, ToolEvent::Down { x: 300.0, y: 300.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, Modifiers::SHIFT);
    let Some(Drag::Edit { preview: Some(p), .. }) = &app.ui.direct_selection.drag else { panic!("dragging") };
    let preview = p.subpaths[0].knots[1].anchor;
    tool_event(&mut app, ToolEvent::Up { x: to[0], y: to[1] }, Modifiers::SHIFT);
    (preview, app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].knots[1].anchor)
}

fn near(a: Point, want: [f64; 2]) -> bool {
    (a.x - want[0]).abs() < 1e-9 && (a.y - want[1]).abs() < 1e-9
}

#[test]
fn a_shallow_shift_drag_stays_horizontal() {
    let (preview, end) = shift_drag([60.0, -25.0]);
    assert_eq!(preview, Point::new(360.0, 300.0), "the preview is locked too");
    assert_eq!(end, Point::new(360.0, 300.0));
}

#[test]
fn the_measured_drag_stays_horizontal_with_f32_pointer_noise() {
    // The screen-to-document mapping runs in f32, so the measured pointer lands a hair off (-25).
    let (preview, end) = shift_drag([60.0, -25.0 - 5e-5]);
    assert_eq!((preview, end), (Point::new(360.0, 300.0), Point::new(360.0, 300.0)));
}

#[test]
fn the_mirrored_vertical_drag_stays_vertical() {
    let (preview, end) = shift_drag([25.0, -60.0]);
    assert_eq!((preview, end), (Point::new(300.0, 240.0), Point::new(300.0, 240.0)));
}

#[test]
fn a_diagonal_shift_drag_still_snaps_to_45_degrees() {
    let (preview, end) = shift_drag([50.0, -50.0]);
    for a in [preview, end] {
        assert!(near(a, [350.0, 250.0]), "{a:?}");
    }
}

#[test]
fn unmeasured_angles_keep_the_45_degree_snap() {
    // (+60, -26) is 23.4°, just steeper than the measured drag; (+60, -30) is 26.6°.
    for by in [[60.0, -26.0], [60.0, -30.0]] {
        let want = crate::stroke_constraint::snap45([300.0, 300.0], [300.0 + by[0], 300.0 + by[1]]);
        assert!(want[0] > 340.0 && (want[0] - 300.0 + want[1] - 300.0).abs() < 1e-9, "on the diagonal: {want:?}");
        let (preview, end) = shift_drag(by);
        for a in [preview, end] {
            assert!(near(a, want), "{by:?}: {a:?} vs {want:?}");
        }
    }
}
