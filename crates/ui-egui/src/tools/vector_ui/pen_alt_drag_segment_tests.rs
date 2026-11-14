//! #12899: ⌥/Alt-dragging a straight segment of the active path with the Pen adds an anchor where
//! it was pressed and pulls out its handles, as one History step (Add Anchor Point), as in
//! Photoshop. Everywhere else ⌥ does what it did.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use photocraft_geom::Point;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The document of the issue: 800 × 600 px with a Work Path whose first segment is straight and
/// runs through (150, 300). The segment 1 → 2 is a curve.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Pen;
    let path = json!({"subpaths": [{"closed": false, "knots": [
        [100, 300],
        {"anchor": [300, 300], "in": [300, 300], "out": [360, 340], "smooth": false},
        [400, 500],
    ]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app
}

/// The Work Path as it is now.
fn work(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}

/// How many History steps the document has.
fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// A press at `from`, a drag to `to` in a few moves and the release there, with `mods` held.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    if from != to {
        for i in 1..=4 {
            let t = f64::from(i) / 4.0;
            tool_event(app, ToolEvent::Move { x: from[0] + (to[0] - from[0]) * t, y: from[1] + (to[1] - from[1]) * t, pressure: 1.0 }, mods);
        }
    }
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

/// Is `a` within 0.5 px of `b` on both axes?
fn near(a: Point, b: [f64; 2]) -> bool {
    (a.x - b[0]).abs() <= 0.5 && (a.y - b[1]).abs() <= 0.5
}

/// The measured case: press (150, 300), release (150, 260).
#[test]
fn alt_drag_on_a_segment_adds_an_anchor_and_pulls_out_its_handles() {
    let mut app = app();
    let (before, n) = (work(&app), steps(&app));
    drag(&mut app, [150.0, 300.0], [150.0, 260.0], Modifiers::ALT);
    let path = work(&app);
    let knots = &path.subpaths[0].knots;
    assert_eq!(knots.len(), 4, "one anchor was added");
    let k = knots[1];
    assert!(near(k.anchor, [150.0, 300.0]), "at the press point: {k:?}");
    assert!(near(k.out_ctrl, [150.0, 260.0]), "the outgoing handle is under the pointer: {k:?}");
    assert!(near(k.in_ctrl, [150.0, 340.0]) && k.smooth, "the incoming handle points the other way: {k:?}");
    let was = &before.subpaths[0].knots;
    assert_eq!((knots[0], knots[2], knots[3]), (was[0], was[1], was[2]), "the other anchors stay");
    assert_eq!(steps(&app), n + 1, "the anchor and its drag are one History step");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
    assert!(app.ui.pen.is_none() && app.ui.direct_selection.drag.is_none(), "no Pen path was started and the drag is over");
    assert!(app.session.undo());
    assert_eq!(work(&app), before, "one Undo takes both back");
}

/// The drag previews the handles, and the release commits where the pointer is.
#[test]
fn the_handle_follows_the_pointer_during_the_drag() {
    let mut app = app();
    let n = steps(&app);
    tool_event(&mut app, ToolEvent::Down { x: 150.0, y: 300.0, pressure: 1.0 }, Modifiers::ALT);
    assert_eq!((work(&app).subpaths[0].knots.len(), steps(&app)), (4, n + 1), "the press adds the anchor");
    tool_event(&mut app, ToolEvent::Move { x: 170.0, y: 280.0, pressure: 1.0 }, Modifiers::ALT);
    let Some(crate::direct_select::Drag::Edit { preview: Some(preview), .. }) = app.ui.direct_selection.drag.clone() else {
        panic!("the drag previews the handles")
    };
    let k = preview.subpaths[0].knots[1];
    assert!(near(k.anchor, [150.0, 300.0]) && near(k.out_ctrl, [170.0, 280.0]) && near(k.in_ctrl, [130.0, 320.0]), "{k:?}");
    assert!(app.ui.pen.is_none(), "the Pen draws nothing meanwhile");
    // Releasing ⌥ before the button changes nothing: the press decided.
    tool_event(&mut app, ToolEvent::Up { x: 180.0, y: 270.0 }, Modifiers::NONE);
    let k = work(&app).subpaths[0].knots[1];
    assert!(near(k.out_ctrl, [180.0, 270.0]) && near(k.in_ctrl, [120.0, 330.0]), "{k:?}");
    assert_eq!(steps(&app), n + 1);
}

/// Without a drag the new anchor has no handles.
#[test]
fn alt_click_on_a_segment_adds_a_corner_in_one_step() {
    let mut app = app();
    let n = steps(&app);
    drag(&mut app, [150.0, 300.0], [150.0, 300.0], Modifiers::ALT);
    let k = work(&app).subpaths[0].knots[1];
    assert_eq!(work(&app).subpaths[0].knots.len(), 4);
    assert!(near(k.anchor, [150.0, 300.0]) && k.in_ctrl == k.anchor && k.out_ctrl == k.anchor, "no handles without a drag: {k:?}");
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
}

/// The + badge shows only where the press would add an anchor.
#[test]
fn the_cursor_shows_the_add_badge_over_a_straight_segment_with_alt() {
    use crate::tool_feedback::{Badge, badge};
    let mut app = app();
    app.hover_doc = Some([150.0, 300.0]);
    assert_eq!(badge(&app, Tool::Pen, Modifiers::ALT), Some(Badge::Add));
    app.hover_doc = Some([100.0, 300.0]);
    assert_eq!(badge(&app, Tool::Pen, Modifiers::ALT), None, "an anchor: Convert Point");
    app.hover_doc = Some([500.0, 100.0]);
    assert_eq!(badge(&app, Tool::Pen, Modifiers::ALT), None, "empty canvas");
}

/// What the issue left unmeasured keeps what it did: a curved segment, Auto Add/Delete off, an
/// anchor, empty canvas.
#[test]
fn alt_elsewhere_keeps_what_it_did() {
    let mut app = app();
    let (before, n) = (work(&app), steps(&app));
    // On the curve 1 → 2 (its point at t = 0.5).
    let seg = before.subpaths[0].segments()[1];
    let on_curve = photocraft_vector::edit::eval(&seg, 0.5);
    drag(&mut app, [on_curve.x, on_curve.y], [on_curve.x, on_curve.y - 40.0], Modifiers::ALT);
    assert_eq!((work(&app), steps(&app)), (before.clone(), n), "a curved segment: nothing, as before");
    assert!(app.ui.pen.is_none());

    app.ui.tool_options.pen_auto_add_delete = false;
    drag(&mut app, [150.0, 300.0], [150.0, 260.0], Modifiers::ALT);
    assert_eq!((work(&app), steps(&app)), (before.clone(), n), "Auto Add/Delete off: nothing, as before");
    assert!(app.ui.pen.is_none());
    app.ui.tool_options.pen_auto_add_delete = true;

    // On an anchor ⌥ is Convert Point: a drag pulls out that anchor's handles, no anchor is added.
    drag(&mut app, [100.0, 300.0], [100.0, 260.0], Modifiers::ALT);
    let path = work(&app);
    assert_eq!(path.subpaths[0].knots.len(), 3);
    assert!(near(path.subpaths[0].knots[0].out_ctrl, [100.0, 260.0]));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Convert Point"));

    // On empty canvas the Pen starts a path.
    let n = steps(&app);
    drag(&mut app, [600.0, 100.0], [600.0, 100.0], Modifiers::ALT);
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(1));
    assert_eq!((work(&app).subpaths[0].knots.len(), steps(&app)), (3, n));
}

/// While a path is being drawn ⌥ stays the last anchor's: a press on a segment of the draft adds
/// no anchor mid-way.
#[test]
fn alt_on_a_segment_of_the_path_being_drawn_keeps_what_it_did() {
    let mut app = app();
    let (before, n) = (work(&app), steps(&app));
    drag(&mut app, [500.0, 100.0], [500.0, 100.0], Modifiers::NONE);
    drag(&mut app, [700.0, 100.0], [700.0, 100.0], Modifiers::NONE);
    let draft = app.ui.pen.as_ref().unwrap().knots.clone();
    assert_eq!(draft.len(), 2);
    drag(&mut app, [600.0, 100.0], [600.0, 60.0], Modifiers::ALT);
    let knots = app.ui.pen.as_ref().unwrap().knots.clone();
    assert_eq!(knots[..2], draft[..], "no anchor went into the segment pressed");
    assert_eq!((work(&app), steps(&app)), (before, n), "the active path is untouched");
}
