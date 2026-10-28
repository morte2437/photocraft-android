//! #790: paths can be reshaped after they're drawn, as in Photoshop: the Direct Selection tool
//! (A), and the Pen with ⌘/Ctrl (Direct Selection) or ⌥/Alt (Convert Point). #3204: Convert Point
//! is a tool of its own, in the Pen's flyout.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use photocraft_geom::Point;
use serde_json::json;

use super::{Drag, PathRef};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

/// A square work path; knot 1 at (120, 40) is smooth with horizontal handles.
fn with_path(app: &mut PhotocraftApp) {
    let path = json!({"subpaths": [{"closed": true, "knots": [
        [40, 40],
        {"anchor": [120, 40], "in": [100, 40], "out": [140, 40], "smooth": true},
        [120, 120],
        [40, 120],
    ]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
}

fn work(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}

fn anchor(app: &PhotocraftApp, k: usize) -> Point {
    work(app).subpaths[0].knots[k].anchor
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    tool_event(app, ToolEvent::Move { x: mid[0], y: mid[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

fn click(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    drag(app, at, at, mods);
}

#[test]
fn dragging_an_anchor_moves_it_in_one_undoable_step() {
    let mut app = app(Tool::DirectSelection);
    with_path(&mut app);
    let (before, n) = (work(&app), steps(&app));
    drag(&mut app, [40.5, 120.5], [50.5, 130.5], Modifiers::NONE);
    assert_eq!(anchor(&app, 3), Point::new(50.0, 130.0));
    assert_eq!(anchor(&app, 0), Point::new(40.0, 40.0), "only the dragged anchor moved");
    assert_eq!(steps(&app), n + 1, "one history step for the whole drag");
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Work), vec![[0, 3]]));
    assert!(app.session.undo());
    assert_eq!(work(&app), before);
}

#[test]
fn the_preview_is_what_the_drag_commits() {
    let mut app = app(Tool::DirectSelection);
    with_path(&mut app);
    tool_event(&mut app, ToolEvent::Down { x: 80.0, y: 120.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 80.0, y: 150.0, pressure: 1.0 }, Modifiers::NONE);
    let Some(Drag::Edit { preview: Some(shown), .. }) = app.ui.direct_selection.drag.clone() else { panic!("a segment drag with a preview") };
    assert_ne!(shown, work(&app), "the document changes only on release");
    tool_event(&mut app, ToolEvent::Up { x: 80.0, y: 150.0 }, Modifiers::NONE);
    assert_eq!(work(&app), shown);
    // The straight bottom segment moved with both its anchors.
    assert_eq!((anchor(&app, 2), anchor(&app, 3)), (Point::new(120.0, 150.0), Point::new(40.0, 150.0)));
}

#[test]
fn a_smooth_points_handles_turn_together_and_alt_breaks_them() {
    let mut app = app(Tool::DirectSelection);
    with_path(&mut app);
    click(&mut app, [120.0, 40.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]], "a click selects the anchor and shows its handles");
    drag(&mut app, [140.0, 40.0], [120.0, 60.0], Modifiers::NONE);
    let k = work(&app).subpaths[0].knots[1];
    assert_eq!(k.out_ctrl, Point::new(120.0, 60.0));
    assert!((k.in_ctrl.x - 120.0).abs() < 1e-9 && (k.in_ctrl.y - 20.0).abs() < 1e-9, "the opposite handle turned with it: {:?}", k.in_ctrl);
    drag(&mut app, [120.0, 20.0], [100.0, 20.0], Modifiers::ALT);
    let k = work(&app).subpaths[0].knots[1];
    assert_eq!((k.in_ctrl, k.out_ctrl, k.smooth), (Point::new(100.0, 20.0), Point::new(120.0, 60.0), false));
}

#[test]
fn marquee_shift_and_empty_clicks_select_like_photoshop() {
    let mut app = app(Tool::DirectSelection);
    with_path(&mut app);
    drag(&mut app, [100.0, 100.0], [180.0, 180.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 2]]);
    click(&mut app, [40.0, 120.0], Modifiers::SHIFT);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 2], [0, 3]], "⇧-click adds");
    // Dragging one selected anchor moves them all.
    drag(&mut app, [40.0, 120.0], [40.0, 110.0], Modifiers::NONE);
    assert_eq!((anchor(&app, 2), anchor(&app, 3)), (Point::new(120.0, 110.0), Point::new(40.0, 110.0)));
    click(&mut app, [40.0, 110.0], Modifiers::SHIFT);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 2]], "⇧-click on a selected anchor deselects it");
    click(&mut app, [80.0, 80.0], Modifiers::NONE);
    assert!(app.ui.direct_selection.anchors.is_empty(), "a click on empty canvas deselects");
    click(&mut app, [80.0, 40.0], Modifiers::ALT);
    assert_eq!(app.ui.direct_selection.anchors.len(), 4, "⌥-click selects the whole subpath");
}

#[test]
fn pen_with_command_ends_the_path_being_drawn_when_pressed_off_it() {
    let mut app = app(Tool::Pen);
    for p in [[30.0, 30.0], [90.0, 30.0]] {
        click(&mut app, p, Modifiers::NONE);
    }
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(2));
    // ⌘-click (Ctrl-click off the Mac) on empty canvas: the path is finished, open.
    click(&mut app, [170.0, 170.0], Modifiers::COMMAND);
    assert!(app.ui.pen.is_none());
    assert_eq!(app.ui.tool, Tool::Pen, "the Pen stays the tool");
    assert!(!work(&app).subpaths[0].closed);
    // ⌘-drag then edits the finished path.
    drag(&mut app, [90.0, 30.0], [90.0, 70.0], Modifiers::COMMAND);
    assert_eq!(anchor(&app, 1), Point::new(90.0, 70.0));
    // Without ⌘ the Pen draws again.
    click(&mut app, [150.0, 150.0], Modifiers::NONE);
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(1));
}

/// #4022: ⌘/Ctrl-dragging an anchor of the path being drawn moves it in place; the path stays
/// unfinished, outside the document and its history, and the Pen carries on from its last anchor.
#[test]
fn pen_with_command_edits_the_path_being_drawn_in_place() {
    let mut app = app(Tool::Pen);
    for p in [[30.0, 30.0], [90.0, 30.0]] {
        click(&mut app, p, Modifiers::NONE);
    }
    let before = steps(&app);
    drag(&mut app, [90.0, 30.0], [90.0, 70.0], Modifiers::COMMAND);
    let pen = app.ui.pen.clone().expect("still drawing");
    assert_eq!(pen.knots.len(), 2);
    assert_eq!(pen.knots[1][0], [90.0, 70.0], "the anchor moved");
    assert_eq!(pen.knots[0][0], [30.0, 30.0]);
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "not committed");
    assert_eq!(steps(&app), before, "no history step until the path is finished");
    // Let go of ⌘: the next click adds an anchor after the moved one.
    click(&mut app, [150.0, 150.0], Modifiers::NONE);
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(3));
    crate::vector_ui::pen_commit(&mut app, false);
    let wp = work(&app);
    assert_eq!(wp.subpaths[0].knots.len(), 3);
    assert_eq!(wp.subpaths[0].knots[1].anchor, Point::new(90.0, 70.0));
    assert_eq!(app.ui.direct_selection.target, None, "the Pen selection is gone with the Pen path");
}

/// ⌘/Ctrl-click selects an anchor of the path being drawn, showing its handles; ⌘-dragging one
/// turns it and the other follows (a smooth point), still without committing.
#[test]
fn pen_with_command_turns_a_handle_of_the_path_being_drawn() {
    let mut app = app(Tool::Pen);
    click(&mut app, [30.0, 30.0], Modifiers::NONE);
    drag(&mut app, [90.0, 30.0], [110.0, 30.0], Modifiers::NONE);
    assert_eq!(app.ui.pen.as_ref().unwrap().knots[1], [[90.0, 30.0], [70.0, 30.0], [110.0, 30.0]]);
    click(&mut app, [90.0, 30.0], Modifiers::COMMAND);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Pen));
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]]);
    drag(&mut app, [110.0, 30.0], [110.0, 50.0], Modifiers::COMMAND);
    let k = app.ui.pen.as_ref().expect("still drawing").knots[1];
    assert_eq!(k[2], [110.0, 50.0], "the outgoing handle turned");
    let (o, i) = ([k[2][0] - k[0][0], k[2][1] - k[0][1]], [k[1][0] - k[0][0], k[1][1] - k[0][1]]);
    assert!((o[0] * i[1] - o[1] * i[0]).abs() < 1e-6 && o[0] * i[0] + o[1] * i[1] < 0.0, "the incoming handle stays opposite: {k:?}");
    assert!(app.session.active().unwrap().doc.work_path.is_none());
}

/// The Curvature Pen too: ⌘/Ctrl-dragging an anchor of its path in progress moves it, and the
/// curve through the anchors follows.
#[test]
fn curvature_pen_with_command_moves_an_anchor_of_the_path_being_drawn() {
    let mut app = app(Tool::CurvaturePen);
    for p in [[30.0, 30.0], [90.0, 30.0], [150.0, 60.0]] {
        click(&mut app, p, Modifiers::NONE);
    }
    let before = app.ui.pen.clone().unwrap();
    drag(&mut app, [90.0, 30.0], [90.0, 10.0], Modifiers::COMMAND);
    let pen = app.ui.pen.clone().expect("still drawing");
    assert!(pen.curvature && pen.knots.len() == 3);
    assert_eq!(pen.knots[1][0], [90.0, 10.0]);
    assert_ne!(pen.knots[1][1], before.knots[1][1], "the handles follow the moved anchor");
    assert!(app.session.active().unwrap().doc.work_path.is_none());
}

#[test]
fn pen_with_alt_converts_points() {
    let mut app = app(Tool::Pen);
    with_path(&mut app);
    click(&mut app, [120.0, 40.0], Modifiers::ALT);
    assert_eq!(work(&app).subpaths[0].knots[1], photocraft_doc::Knot::corner(120.0, 40.0), "⌥-click makes a corner");
    drag(&mut app, [40.0, 120.0], [40.0, 140.0], Modifiers::ALT);
    let k = work(&app).subpaths[0].knots[3];
    assert_eq!((k.in_ctrl, k.out_ctrl, k.smooth), (Point::new(40.0, 100.0), Point::new(40.0, 140.0), true), "⌥-drag pulls out smooth handles");
    assert!(app.ui.pen.is_none(), "no pen path was started");
    // ⌥ off the path is a plain Pen click.
    click(&mut app, [170.0, 170.0], Modifiers::ALT);
    assert!(app.ui.pen.is_some());
}

#[test]
fn shape_layer_paths_are_edited_too() {
    let mut app = app(Tool::DirectSelection);
    let id = app.run("shape.create", json!({"kind": "rect", "rect": [20, 20, 60, 60], "fill": "#3070c0"})).unwrap()["layer"].as_u64().unwrap();
    drag(&mut app, [80.0, 80.0], [100.0, 90.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Layer(id)));
    let info = app.run("shape.info", json!({"layer": id})).unwrap();
    assert_eq!(info["path"]["subpaths"][0]["knots"][2]["anchor"], json!([100.0, 90.0]));
    let b: [i64; 4] = serde_json::from_value(info["bounds"].clone()).unwrap();
    assert!(b[0] + b[2] >= 100 && b[1] + b[3] >= 90, "the shape re-rendered: {b:?}");
}

#[test]
fn the_convert_point_tool_converts_points_without_a_modifier() {
    let mut app = app(Tool::ConvertPoint);
    with_path(&mut app);
    let n = steps(&app);
    click(&mut app, [120.0, 40.0], Modifiers::NONE);
    assert_eq!(work(&app).subpaths[0].knots[1], photocraft_doc::Knot::corner(120.0, 40.0), "a click makes a corner");
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]], "the converted anchor is selected");
    drag(&mut app, [40.0, 120.0], [40.0, 140.0], Modifiers::NONE);
    let k = work(&app).subpaths[0].knots[3];
    assert_eq!((k.in_ctrl, k.out_ctrl, k.smooth), (Point::new(40.0, 100.0), Point::new(40.0, 140.0), true), "a drag pulls out smooth handles");
    assert_eq!(steps(&app), n + 2, "one history step per press");
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 3]]);
    assert!(app.ui.pen.is_none(), "the tool never starts a pen path");
    assert_eq!(app.ui.tool, Tool::ConvertPoint);
    assert!(app.session.undo() && app.session.undo());
    assert!(work(&app).subpaths[0].knots[1].smooth, "undo restores the smooth point");
}

#[test]
fn the_convert_point_tool_moves_a_handle_alone_and_ignores_empty_canvas_and_segments() {
    let mut app = app(Tool::DirectSelection);
    with_path(&mut app);
    // Select the smooth anchor with Direct Selection so its handles show.
    click(&mut app, [120.0, 40.0], Modifiers::NONE);
    app.ui.tool = Tool::ConvertPoint;
    drag(&mut app, [140.0, 40.0], [140.0, 70.0], Modifiers::NONE);
    let k = work(&app).subpaths[0].knots[1];
    assert_eq!((k.out_ctrl, k.in_ctrl, k.smooth), (Point::new(140.0, 70.0), Point::new(100.0, 40.0), false), "the handle moved without its twin");
    // Nothing happens on empty canvas or on a segment, and the tool keeps the event.
    let (before, n) = (work(&app), steps(&app));
    click(&mut app, [80.0, 80.0], Modifiers::NONE);
    drag(&mut app, [120.0, 80.0], [120.0, 100.0], Modifiers::NONE);
    assert!(app.ui.direct_selection.drag.is_none(), "no marquee");
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]], "the selection stays");
    assert_eq!((work(&app), steps(&app)), (before, n));
    assert!(app.ui.pen.is_none() && app.drag.is_none(), "the press was not handed to another tool");
}

#[test]
fn the_convert_point_tool_with_command_is_direct_selection() {
    let mut app = app(Tool::ConvertPoint);
    with_path(&mut app);
    drag(&mut app, [40.5, 120.5], [50.5, 130.5], Modifiers::COMMAND);
    assert_eq!(anchor(&app, 3), Point::new(50.0, 130.0), "⌘-drag moves the anchor");
    assert!(work(&app).subpaths[0].knots[3].in_ctrl == work(&app).subpaths[0].knots[3].anchor, "and does not pull out handles");
    // A handle drag with ⌘ turns the smooth pair together, like Direct Selection.
    click(&mut app, [120.0, 40.0], Modifiers::COMMAND);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]]);
    drag(&mut app, [140.0, 40.0], [120.0, 60.0], Modifiers::COMMAND);
    let k = work(&app).subpaths[0].knots[1];
    assert!((k.in_ctrl.x - 120.0).abs() < 1e-9 && (k.in_ctrl.y - 20.0).abs() < 1e-9, "the twin handle turned with it: {:?}", k.in_ctrl);
}

#[test]
fn the_convert_point_tool_shows_the_path_overlay() {
    let mut app = app(Tool::ConvertPoint);
    assert!(super::shows(&app, Modifiers::NONE), "anchors are drawn before any press");
    app.ui.tool = Tool::Pen;
    assert!(!super::shows(&app, Modifiers::NONE));
    assert!(super::shows(&app, Modifiers::COMMAND));
}

/// One key press through the canvas arrow keys, as egui delivers it.
fn press(app: &mut PhotocraftApp, key: egui::Key, mods: Modifiers) -> bool {
    let ctx = egui::Context::default();
    ctx.input_mut(|i| {
        i.modifiers = mods;
        i.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: mods });
    });
    crate::move_mods::arrow_keys(app, &ctx)
}

/// #4486: the arrow keys nudge the selected anchors 1 px (⇧ 10 px), one history step per press,
/// as in Photoshop. With no anchor selected they leave the path alone.
#[test]
fn arrow_keys_nudge_the_selected_anchors() {
    let mut app = app(Tool::DirectSelection);
    with_path(&mut app);
    // The path stays targeted after a click on empty canvas, but no anchor is selected.
    click(&mut app, [40.0, 120.0], Modifiers::NONE);
    click(&mut app, [80.0, 80.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Work));
    let n = steps(&app);
    assert!(!press(&mut app, egui::Key::ArrowRight, Modifiers::NONE), "no anchor selected: not Direct Selection's key");
    assert_eq!(steps(&app), n);
    click(&mut app, [40.0, 120.0], Modifiers::NONE);
    click(&mut app, [120.0, 120.0], Modifiers::SHIFT);
    let (before, n) = (work(&app), steps(&app));
    assert!(press(&mut app, egui::Key::ArrowRight, Modifiers::NONE));
    assert_eq!((anchor(&app, 2), anchor(&app, 3)), (Point::new(121.0, 120.0), Point::new(41.0, 120.0)));
    assert!(press(&mut app, egui::Key::ArrowDown, Modifiers::SHIFT));
    assert_eq!((anchor(&app, 2), anchor(&app, 3)), (Point::new(121.0, 130.0), Point::new(41.0, 130.0)), "⇧ nudges 10 px");
    assert_eq!(anchor(&app, 0), Point::new(40.0, 40.0), "unselected anchors stay");
    assert_eq!(steps(&app), n + 2, "one history step per press");
    assert!(app.session.undo() && app.session.undo());
    assert_eq!(work(&app), before);
}

/// #5475: ⇧ during an anchor drag locks it to 45°, whether it's held from the press or added later.
#[test]
fn shift_locks_an_anchor_drag_to_45_degrees() {
    let mut app = app(Tool::DirectSelection);
    with_path(&mut app);
    tool_event(&mut app, ToolEvent::Down { x: 40.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 44.0, pressure: 1.0 }, Modifiers::SHIFT);
    let Some(Drag::Edit { preview: Some(p), .. }) = &app.ui.direct_selection.drag else { panic!("dragging") };
    assert_eq!(p.subpaths[0].knots[0].anchor, Point::new(70.0, 40.0), "the preview is locked too");
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 47.0 }, Modifiers::SHIFT);
    assert_eq!(anchor(&app, 0), Point::new(100.0, 40.0));

    // Diagonal, and without ⇧ the drag stays free.
    // (⇧ at the press would deselect the anchor, so it comes once the drag is under way.)
    tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 131.0, y: 69.0 }, Modifiers::SHIFT);
    let a = anchor(&app, 0);
    assert!((a.x - 130.0).abs() < 1e-9 && (a.y - 70.0).abs() < 1e-9, "{a:?}");
    drag(&mut app, [130.0, 70.0], [140.0, 73.0], Modifiers::NONE);
    assert_eq!(anchor(&app, 0), Point::new(140.0, 73.0));
}

#[test]
fn the_curvature_pen_drags_an_anchor_of_the_active_path_without_a_modifier() {
    // As in Photoshop: with no path being drawn, pressing an anchor and dragging moves it.
    let mut app = app(Tool::CurvaturePen);
    with_path(&mut app);
    assert!(super::shows(&app, Modifiers::NONE), "the Curvature Pen shows the active path's anchors");
    let (before, n) = (work(&app), steps(&app));
    drag(&mut app, [40.5, 120.5], [50.5, 130.5], Modifiers::NONE);
    assert_eq!(anchor(&app, 3), Point::new(50.0, 130.0));
    assert_eq!(anchor(&app, 0), Point::new(40.0, 40.0), "only the dragged anchor moved");
    assert_eq!(work(&app).subpaths.len(), 1, "no new path was started");
    assert!(app.ui.pen.is_none(), "nor is one being drawn");
    assert_eq!(steps(&app), n + 1, "one history step for the whole drag");
    assert!(app.session.undo());
    assert_eq!(work(&app), before);
}

#[test]
fn the_curvature_pen_off_an_anchor_still_draws_and_edits_its_own_path() {
    let mut app = app(Tool::CurvaturePen);
    with_path(&mut app);
    let before = work(&app);
    // A press on empty canvas starts a new path, as before (a segment would get an anchor).
    click(&mut app, [80.0, 170.0], Modifiers::NONE);
    click(&mut app, [170.0, 170.0], Modifiers::NONE);
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(2));
    assert!(!super::shows(&app, Modifiers::NONE), "while drawing, the Curvature Pen shows its own path");
    // While drawing, a press on an anchor of the path being drawn moves that one, not the work path's.
    drag(&mut app, [170.0, 170.0], [180.0, 160.0], Modifiers::NONE);
    assert_eq!(app.ui.pen.as_ref().and_then(|p| p.knots.get(1)).map(|k| k[0]), Some([180.0, 160.0]));
    assert_eq!(work(&app), before);
}

#[test]
fn a_closed_curvature_pen_path_stays_editable_with_the_curvature_pen() {
    let mut app = app(Tool::CurvaturePen);
    app.ui.tool_options.vector_mode = "path".into();
    for p in [[40.0, 40.0], [160.0, 40.0], [100.0, 160.0], [40.0, 40.0]] {
        click(&mut app, p, Modifiers::NONE);
    }
    assert!(app.ui.pen.is_none(), "clicking the first anchor closed the path");
    assert!(work(&app).subpaths[0].closed);
    let n = steps(&app);
    tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 160.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 100.0, y: 180.0, pressure: 1.0 }, Modifiers::NONE);
    let Some(Drag::Edit { preview: Some(shown), .. }) = app.ui.direct_selection.drag.clone() else { panic!("an anchor drag with a preview") };
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 180.0 }, Modifiers::NONE);
    assert_eq!(work(&app), shown, "the preview is what the drag commits");
    assert_eq!((work(&app).subpaths.len(), steps(&app)), (1, n + 1));
    // The whole loop reflows through the moved anchor, as the Curvature Pen drew it.
    let pts = [[40.0, 40.0], [160.0, 40.0], [100.0, 180.0]];
    let want = photocraft_vector::edit::curvature_knots(&pts, &[false; 3], true);
    for (k, w) in work(&app).subpaths[0].knots.iter().zip(want) {
        assert_eq!([[k.anchor.x, k.anchor.y], [k.in_ctrl.x, k.in_ctrl.y], [k.out_ctrl.x, k.out_ctrl.y]], w);
    }
}

/// A Pen-drawn work path: a closed square of corners (40, 40)–(160, 160).
fn with_corner_square(app: &mut PhotocraftApp) {
    let path = json!({"subpaths": [{"closed": true, "knots": [[40, 40], [160, 40], [160, 160], [40, 160]]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
}

#[test]
fn the_curvature_pen_adds_an_anchor_on_a_segment_and_drags_it_in_one_step() {
    let mut app = app(Tool::CurvaturePen);
    with_corner_square(&mut app);
    let n = steps(&app);
    // A press on the top edge adds an anchor there; the drag pulls it up.
    drag(&mut app, [100.0, 40.0], [100.0, 10.0], Modifiers::NONE);
    let sp = &work(&app).subpaths[0];
    assert_eq!(sp.knots.len(), 5, "one anchor added");
    assert_eq!(sp.knots[1].anchor, Point::new(100.0, 10.0));
    assert!(app.ui.pen.is_none(), "no new path was started");
    assert_eq!(steps(&app), n + 1, "adding and dragging the anchor is one history step");
    assert!(app.session.undo());
    assert_eq!(work(&app).subpaths[0].knots.len(), 4);
    // A click on a segment just adds the anchor.
    click(&mut app, [160.0, 100.0], Modifiers::NONE);
    assert_eq!(work(&app).subpaths[0].knots.len(), 5);
}

#[test]
fn a_curvature_pen_double_click_rounds_a_pen_corner_and_back() {
    let mut app = app(Tool::CurvaturePen);
    with_corner_square(&mut app);
    let before = work(&app);
    crate::vector_ui::curvature_toggle(&mut app, 160.0, 40.0);
    let k = work(&app).subpaths[0].knots[1];
    assert!(k.smooth && k.in_ctrl != k.anchor && k.out_ctrl != k.anchor, "the corner turned smooth: {k:?}");
    crate::vector_ui::curvature_toggle(&mut app, 160.0, 40.0);
    assert_eq!(work(&app), before);
    // Off the path it does nothing (and starts nothing).
    crate::vector_ui::curvature_toggle(&mut app, 100.0, 100.0);
    assert_eq!((work(&app), app.ui.pen.is_none()), (before, true));
}

#[test]
fn the_curvature_pen_cursor_shows_what_a_press_does() {
    use crate::tool_feedback::{Badge, badge};
    let mut app = app(Tool::CurvaturePen);
    with_corner_square(&mut app);
    for (at, want) in [([100.0, 40.0], Some(Badge::Add)), ([160.0, 40.0], Some(Badge::Move)), ([100.0, 100.0], None)] {
        app.hover_doc = Some(at);
        assert_eq!(badge(&app, Tool::CurvaturePen, Modifiers::NONE), want, "at {at:?}");
    }
    // ⌘/Ctrl is Direct Selection: no Curvature Pen badge.
    app.hover_doc = Some([160.0, 40.0]);
    assert_eq!(badge(&app, Tool::CurvaturePen, Modifiers::COMMAND), None);
    // While drawing, its own anchors take the move badge and the active path's segments none.
    click(&mut app, [20.0, 180.0], Modifiers::NONE);
    click(&mut app, [60.0, 190.0], Modifiers::NONE);
    app.hover_doc = Some([60.0, 190.0]);
    assert_eq!(badge(&app, Tool::CurvaturePen, Modifiers::NONE), Some(Badge::Move));
    app.hover_doc = Some([100.0, 40.0]);
    assert_eq!(badge(&app, Tool::CurvaturePen, Modifiers::NONE), None);
}

/// The real canvas: a Curvature Pen double-click on a Pen corner of the active path rounds it.
#[test]
fn a_curvature_pen_double_click_through_the_canvas_rounds_a_corner() {
    use egui::{PointerButton, vec2};
    use egui_kittest::Harness;
    let mut app = app(Tool::CurvaturePen);
    with_corner_square(&mut app);
    app.ui.extras.rulers = false;
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_step_dt(1.0 / 60.0).build_ui_state(
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
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [100.0, 100.0];
    v.fit_pending = false;
    h.run_steps(2);
    let p = {
        let app = h.state();
        let v = &app.ui.views[0];
        let xf = crate::canvas::ViewXform {
            rect: crate::rulers::content_rect(app, app.last_canvas_rect),
            zoom: v.zoom,
            center: v.center,
            flip: false,
            rotation: v.rotation,
            aspect: app.ui.view.display_aspect(),
        };
        xf.to_screen(160.0, 40.0)
    };
    for _ in 0..2 {
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(1);
        for down in [true, false] {
            h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
            h.run_steps(1);
        }
    }
    h.run_steps(1);
    let k = work(h.state()).subpaths[0].knots[1];
    assert!(k.smooth && k.in_ctrl != k.anchor, "the double-clicked corner is smooth: {k:?}");
    assert!(h.state().ui.pen.is_none(), "and no path was started");
}

/// #9114: with type on a path selected, Direct Selection reshapes the type's own path (in front of
/// the work path it was made from) and the type follows, in one undoable step.
#[test]
fn dragging_an_anchor_of_type_on_a_path_moves_the_type() {
    let mut app = app(Tool::DirectSelection);
    app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[100, 20], [100, 180]]}]}})).unwrap();
    let id = app.run("type.create", json!({"x": 20, "y": 70, "text": "On a path", "size": 24})).unwrap()["layer"].as_u64().unwrap();
    app.run("type.attachToPath", json!({})).unwrap();
    let type_path = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
        photocraft_doc::LayerContent::Text(t) => photocraft_engine::type_extra_cmds::type_path(t).unwrap(),
        _ => panic!("not a type layer"),
    };
    let n = steps(&app);
    drag(&mut app, [100.2, 180.2], [160.2, 180.2], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::TypePath(id)), "the type path, in front of the work path");
    let end = type_path(&app).subpaths[0].knots[1].anchor;
    assert!((end.x - 160.0).abs() < 1e-6 && (end.y - 180.0).abs() < 1e-6, "{end:?}");
    assert_eq!(work(&app).subpaths[0].knots[1].anchor, Point::new(100.0, 180.0), "the work path stays");
    assert_eq!(steps(&app), n + 1);
    assert!(app.session.undo());
    assert_eq!(type_path(&app).subpaths[0].knots[1].anchor.x.round(), 100.0);
}

/// #9114: with a type-on-a-path layer's vector mask targeted, Direct Selection edits the mask, not
/// the Type Path; untargeted, it edits the Type Path and the mask stays.
#[test]
fn a_targeted_vector_mask_on_type_on_a_path_is_edited_not_the_type_path() {
    let mut app = app(Tool::DirectSelection);
    app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[100, 20], [100, 180]]}]}})).unwrap();
    let id = app.run("type.create", json!({"x": 20, "y": 70, "text": "On a path", "size": 24})).unwrap()["layer"].as_u64().unwrap();
    app.run("type.attachToPath", json!({})).unwrap();
    let rect = json!({"subpaths": [{"closed": true, "knots": [[20, 20], [180, 20], [180, 160], [20, 160]]}]});
    app.run("layer.vectorMask.add", json!({"layer": id, "path": rect})).unwrap();
    app.ui.vector_mask_target = true;
    let layer = |app: &PhotocraftApp| app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().clone();
    let type_path = |app: &PhotocraftApp| match &layer(app).content {
        photocraft_doc::LayerContent::Text(t) => photocraft_engine::type_extra_cmds::type_path(t).unwrap(),
        _ => panic!("not a type layer"),
    };
    let before = type_path(&app);
    drag(&mut app, [20.2, 20.2], [40.2, 30.2], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::Layer(id)));
    assert_eq!(layer(&app).vector_mask.unwrap().path.subpaths[0].knots[0].anchor, Point::new(40.0, 30.0), "the mask's corner moved");
    assert_eq!(type_path(&app), before, "the Type Path stays");
    // With the mask no longer targeted, the Type Path is in front again and the mask stays.
    app.ui.vector_mask_target = false;
    drag(&mut app, [100.2, 180.2], [160.2, 180.2], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.target, Some(PathRef::TypePath(id)));
    assert!((type_path(&app).subpaths[0].knots[1].anchor.x - 160.0).abs() < 1e-6);
    assert_eq!(layer(&app).vector_mask.unwrap().path.subpaths[0].knots[0].anchor, Point::new(40.0, 30.0));
}
