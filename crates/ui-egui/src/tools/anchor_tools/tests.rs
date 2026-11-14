//! #1044: the Add Anchor Point and Delete Anchor Point tools insert and remove anchors on the
//! paths Direct Selection edits, one history step per click.

use egui::Modifiers;
use photocraft_doc::vector::Path;
use photocraft_geom::Point;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::direct_select::PathRef;
use crate::state::Tool;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

/// A square work path; knot 1 at (120, 40) is smooth, so the top edge (0 → 1) is a curve.
fn with_path(app: &mut PhotocraftApp) {
    let path = json!({"subpaths": [{"closed": true, "knots": [
        [40, 40],
        {"anchor": [120, 40], "in": [100, 20], "out": [140, 60], "smooth": true},
        [120, 120],
        [40, 120],
    ]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
}

fn work(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().unwrap()
}

pub(super) fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

pub(super) fn click(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, mods);
}

#[test]
fn the_anchor_tools_have_no_letter() {
    let group: Vec<Tool> = Tool::ALL.iter().copied().filter(|t| t.key() == 'P').collect();
    assert_eq!(group, vec![Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen], "P cycles Pen → Freeform → Curvature and never reaches the anchor point tools");
    let mut app = app(Tool::Brush);
    app.ui.tool = Tool::from_name("addAnchorPoint").unwrap();
    assert_eq!(app.ui.tool, Tool::AddAnchorPoint);
}

#[test]
fn add_on_a_straight_segment_inserts_a_smooth_anchor_in_one_step() {
    let mut app = app(Tool::AddAnchorPoint);
    with_path(&mut app);
    let (before, n) = (work(&app), steps(&app));
    click(&mut app, [80.0, 121.0], Modifiers::NONE);
    let p = work(&app);
    assert_eq!(p.subpaths[0].knots.len(), 5);
    let k = p.subpaths[0].knots[3];
    assert!(k.smooth && (k.anchor.y - 120.0).abs() < 1e-9 && (k.anchor.x - 80.0).abs() < 1.0, "a smooth anchor on the bottom edge: {k:?}");
    assert!(
        (k.in_ctrl.y - 120.0).abs() < 1e-9 && (k.out_ctrl.y - 120.0).abs() < 1e-9 && k.in_ctrl.x != k.anchor.x && k.out_ctrl.x != k.anchor.x,
        "with handles along the edge: {k:?}"
    );
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
    assert!(app.session.undo());
    assert_eq!(work(&app), before);
}

#[test]
fn add_on_a_curve_splits_it_where_clicked() {
    let mut app = app(Tool::AddAnchorPoint);
    with_path(&mut app);
    let seg = work(&app).subpaths[0].segments()[0];
    let on = photocraft_vector::edit::eval(&seg, 0.5);
    click(&mut app, [on.x, on.y], Modifiers::NONE);
    let p = work(&app);
    let k = p.subpaths[0].knots[1];
    assert!(k.smooth && (k.anchor.x - on.x).hypot(k.anchor.y - on.y) < 0.2, "a smooth anchor where clicked: {k:?}");
    // The two halves still pass through the old curve's quarter points.
    let halves = &p.subpaths[0].segments()[..2];
    for t in [0.25, 0.75] {
        let old = photocraft_vector::edit::eval(&seg, t);
        let near = halves
            .iter()
            .flat_map(|s| (0..=400).map(move |i| photocraft_vector::edit::eval(s, f64::from(i) / 400.0)))
            .fold(f64::MAX, |d, q| d.min((q.x - old.x).hypot(q.y - old.y)));
        assert!(near < 0.1, "t={t}: {near}");
    }
}

#[test]
fn delete_removes_the_clicked_anchor_and_alt_swaps_the_tools() {
    let mut app = app(Tool::DeleteAnchorPoint);
    with_path(&mut app);
    let n = steps(&app);
    click(&mut app, [120.0, 120.0], Modifiers::NONE);
    let p = work(&app);
    assert_eq!((p.subpaths[0].knots.len(), p.subpaths[0].closed), (3, true));
    assert_eq!(p.subpaths[0].knots[2].anchor, Point::new(40.0, 120.0));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
    // ⌥ with Delete adds; ⌥ with Add deletes.
    click(&mut app, [80.0, 120.0], Modifiers::ALT);
    assert_eq!(work(&app).subpaths[0].knots.len(), 3, "the bottom edge is gone; ⌥ on empty canvas adds nothing");
    click(&mut app, [40.0, 80.0], Modifiers::ALT);
    assert_eq!(work(&app).subpaths[0].knots.len(), 4);
    app.ui.tool = Tool::AddAnchorPoint;
    click(&mut app, [40.0, 80.0], Modifiers::ALT);
    assert_eq!(work(&app).subpaths[0].knots.len(), 3);
    assert_eq!(steps(&app), n + 3);
}

#[test]
fn clicks_on_nothing_editable_change_nothing() {
    let mut app = app(Tool::AddAnchorPoint);
    with_path(&mut app);
    let (before, n) = (work(&app), steps(&app));
    click(&mut app, [80.0, 80.0], Modifiers::NONE);
    click(&mut app, [40.0, 40.0], Modifiers::NONE);
    app.ui.tool = Tool::DeleteAnchorPoint;
    click(&mut app, [80.0, 80.0], Modifiers::NONE);
    click(&mut app, [40.0, 80.0], Modifiers::NONE);
    assert_eq!((work(&app), steps(&app)), (before, n), "empty canvas, an anchor with Add, a segment with Delete");
}

#[test]
fn shape_layer_paths_are_edited_and_the_added_anchor_replaces_the_selection() {
    let mut app = app(Tool::DirectSelection);
    let id = app.run("shape.create", json!({"kind": "rect", "rect": [20, 20, 60, 60], "fill": "#3070c0"})).unwrap()["layer"].as_u64().unwrap();
    click(&mut app, [80.0, 80.0], Modifiers::NONE);
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Layer(id)), vec![[0, 2]]));
    app.ui.tool = Tool::AddAnchorPoint;
    click(&mut app, [50.0, 20.0], Modifiers::NONE);
    let info = app.run("shape.info", json!({"layer": id})).unwrap();
    assert_eq!(info["path"]["subpaths"][0]["knots"].as_array().unwrap().len(), 5);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]], "the new anchor is selected, not a shifted old one");
    app.ui.tool = Tool::DeleteAnchorPoint;
    click(&mut app, [20.0, 80.0], Modifiers::NONE);
    let info = app.run("shape.info", json!({"layer": id})).unwrap();
    assert_eq!(info["path"]["subpaths"][0]["knots"].as_array().unwrap().len(), 4);
}

#[test]
fn the_cursor_shows_plus_or_minus() {
    use crate::tool_feedback::{Badge, badge};
    let mut app = app(Tool::AddAnchorPoint);
    assert_eq!(badge(&app, Tool::AddAnchorPoint, Modifiers::NONE), None, "no path targeted, no segment under the pointer (#12729)");
    with_path(&mut app);
    app.ui.selected_path = Some("work".into());
    app.hover_doc = Some([80.0, 80.0]);
    assert_eq!(badge(&app, Tool::AddAnchorPoint, Modifiers::NONE), None, "off the path's segments");
    app.hover_doc = Some([80.0, 120.0]);
    assert_eq!(badge(&app, Tool::AddAnchorPoint, Modifiers::NONE), Some(Badge::Add));
    assert_eq!(badge(&app, Tool::AddAnchorPoint, Modifiers::ALT), Some(Badge::Subtract));
    assert_eq!(badge(&app, Tool::DeleteAnchorPoint, Modifiers::NONE), Some(Badge::Subtract));
    assert_eq!(badge(&app, Tool::DeleteAnchorPoint, Modifiers::COMMAND), None, "⌘ is Direct Selection");
}

/// #4144: with Auto Add/Delete (on by default) a Pen click on a segment of the active path adds
/// an anchor and a click on one of its anchors deletes it, one step each, and no Pen path starts.
#[test]
fn the_pen_adds_and_deletes_anchors_on_the_active_path() {
    let mut app = app(Tool::Pen);
    assert!(app.ui.tool_options.pen_auto_add_delete, "on by default, as in Photoshop");
    with_path(&mut app);
    let n = steps(&app);
    click(&mut app, [80.0, 120.0], Modifiers::NONE);
    assert_eq!(work(&app).subpaths[0].knots.len(), 5);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
    assert!(app.ui.pen.is_none(), "no Pen path was started");
    click(&mut app, [40.0, 120.0], Modifiers::NONE);
    let p = work(&app);
    assert_eq!(p.subpaths[0].knots.len(), 4);
    assert!(!p.subpaths[0].knots.iter().any(|k| k.anchor == Point::new(40.0, 120.0)));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
    assert_eq!(steps(&app), n + 2);
    assert!(app.ui.pen.is_none());
    // Off the path the Pen draws.
    click(&mut app, [170.0, 170.0], Modifiers::NONE);
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(1));
}

/// Auto Add/Delete off, or ⇧ held, or a path being drawn: the Pen click places an anchor.
#[test]
fn the_pen_draws_over_the_path_with_auto_add_delete_off_shift_or_while_drawing() {
    let mut app = app(Tool::Pen);
    with_path(&mut app);
    let before = work(&app);
    app.ui.tool_options.pen_auto_add_delete = false;
    click(&mut app, [80.0, 120.0], Modifiers::NONE);
    assert_eq!((work(&app), app.ui.pen.as_ref().map(|p| p.knots.len())), (before.clone(), Some(1)));
    app.ui.pen = None;

    app.ui.tool_options.pen_auto_add_delete = true;
    click(&mut app, [80.0, 120.0], Modifiers::SHIFT);
    assert_eq!((work(&app), app.ui.pen.as_ref().map(|p| p.knots.len())), (before.clone(), Some(1)), "⇧ turns it off");
    app.ui.pen = None;

    click(&mut app, [170.0, 170.0], Modifiers::NONE);
    click(&mut app, [80.0, 120.0], Modifiers::NONE);
    assert_eq!((work(&app), app.ui.pen.as_ref().map(|p| p.knots.len())), (before, Some(2)), "a path being drawn takes the click");
}

/// The end points of an open subpath stay the Pen's; its middle anchors are deleted.
#[test]
fn the_pen_leaves_open_end_points_alone() {
    let mut app = app(Tool::Pen);
    let path = json!({"subpaths": [{"closed": false, "knots": [[40, 40], [120, 40], [120, 120]]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    let before = work(&app);
    for end in [[40.0, 40.0], [120.0, 120.0]] {
        click(&mut app, end, Modifiers::NONE);
        assert_eq!(work(&app), before, "{end:?}");
        app.ui.pen = None;
    }
    click(&mut app, [120.0, 40.0], Modifiers::NONE);
    assert_eq!(work(&app).subpaths[0].knots.len(), 2);
}

#[test]
fn the_pen_cursor_shows_what_auto_add_delete_would_do() {
    use crate::tool_feedback::{Badge, badge};
    let mut app = app(Tool::Pen);
    with_path(&mut app);
    for (at, want) in [([80.0, 120.0], Some(Badge::Add)), ([40.0, 120.0], Some(Badge::Subtract)), ([80.0, 80.0], None)] {
        app.hover_doc = Some(at);
        assert_eq!(badge(&app, Tool::Pen, Modifiers::NONE), want, "{at:?}");
    }
    app.hover_doc = Some([80.0, 120.0]);
    assert_eq!(badge(&app, Tool::Pen, Modifiers::SHIFT), None);
    app.ui.tool_options.pen_auto_add_delete = false;
    assert_eq!(badge(&app, Tool::Pen, Modifiers::NONE), None);
}

#[test]
fn add_and_delete_anchor_tools_edit_the_selected_saved_path() {
    let mut app = app(Tool::AddAnchorPoint);
    with_path(&mut app);
    let before_work = work(&app);
    let path = photocraft_engine::vector_cmds::path_json(&before_work);
    app.run("path.set", json!({"name":"Saved", "path":path})).unwrap();
    app.ui.selected_path = Some("Saved".into());
    let n = steps(&app);
    click(&mut app, [80.0, 120.0], Modifiers::NONE);
    let saved = || app.session.active().unwrap().doc.paths.iter().find(|path| path.name == "Saved").unwrap().path.clone();
    let added = saved();
    assert_eq!(added.subpaths[0].knots.len(), 5);
    assert_eq!(work(&app), before_work);
    assert_eq!(steps(&app), n + 1);
    app.ui.tool = Tool::DeleteAnchorPoint;
    let anchor = added.subpaths[0].knots[3].anchor;
    click(&mut app, [anchor.x, anchor.y], Modifiers::NONE);
    let deleted = app.session.active().unwrap().doc.paths.iter().find(|path| path.name == "Saved").unwrap().path.clone();
    assert_eq!(deleted.subpaths[0].knots.len(), 4);
    assert_eq!(work(&app), before_work);
    assert_eq!(steps(&app), n + 2);
    assert!(app.session.undo());
    assert_eq!(app.session.active().unwrap().doc.paths.iter().find(|path| path.name == "Saved").unwrap().path, added);
}

fn pen_anchors(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    app.ui.pen.as_ref().map(|p| p.knots.iter().map(|k| k[0]).collect()).unwrap_or_default()
}

/// #6140: while drawing, a Pen click on the path being drawn adds an anchor on a segment or
/// deletes a middle anchor, and drawing carries on from the last anchor. Nothing reaches History.
#[test]
fn the_pen_adds_and_deletes_anchors_on_the_path_being_drawn() {
    let mut app = app(Tool::Pen);
    let before = steps(&app);
    for at in [[40.0, 40.0], [120.0, 40.0], [120.0, 120.0]] {
        click(&mut app, at, Modifiers::NONE);
    }
    click(&mut app, [80.0, 40.0], Modifiers::NONE);
    assert_eq!(pen_anchors(&app), vec![[40.0, 40.0], [80.0, 40.0], [120.0, 40.0], [120.0, 120.0]], "added on the first segment");
    click(&mut app, [40.0, 120.0], Modifiers::NONE);
    assert_eq!(pen_anchors(&app).last(), Some(&[40.0, 120.0]), "the next click carries on from the last anchor");
    assert_eq!(pen_anchors(&app).len(), 5);

    click(&mut app, [120.0, 40.0], Modifiers::NONE);
    assert_eq!(pen_anchors(&app), vec![[40.0, 40.0], [80.0, 40.0], [120.0, 120.0], [40.0, 120.0]], "a middle anchor is deleted");
    assert_eq!(steps(&app), before, "the path being drawn is not in the document yet");
}

/// The path being drawn keeps its end points (the first closes it, the last breaks its handle), ⇧
/// or the option turned off places an anchor on its segment, and another path takes no edit.
#[test]
fn the_pen_draws_on_the_path_being_drawn_where_auto_add_delete_does_not_apply() {
    let mut app = app(Tool::Pen);
    with_path(&mut app);
    let work_before = work(&app);
    for at in [[20.0, 160.0], [180.0, 160.0], [180.0, 180.0]] {
        click(&mut app, at, Modifiers::NONE);
    }
    click(&mut app, [180.0, 180.0], Modifiers::NONE);
    assert_eq!(pen_anchors(&app).len(), 3, "a click on the last anchor breaks its handle");
    click(&mut app, [100.0, 160.0], Modifiers::SHIFT);
    assert_eq!(pen_anchors(&app).len(), 4, "⇧ places an anchor");
    app.ui.tool_options.pen_auto_add_delete = false;
    click(&mut app, [100.0, 160.0], Modifiers::NONE);
    assert_eq!(pen_anchors(&app).len(), 5, "with the option off the Pen places an anchor");
    app.ui.tool_options.pen_auto_add_delete = true;
    click(&mut app, [80.0, 120.0], Modifiers::NONE);
    assert_eq!((pen_anchors(&app).len(), work(&app)), (6, work_before), "another path takes no edit while drawing");
    click(&mut app, [20.0, 160.0], Modifiers::NONE);
    assert!(app.ui.pen.is_none(), "the first anchor closes the path");
}

#[test]
fn the_pen_cursor_shows_auto_add_delete_on_the_path_being_drawn() {
    use crate::tool_feedback::{Badge, badge};
    let mut app = app(Tool::Pen);
    for at in [[40.0, 40.0], [120.0, 40.0], [120.0, 120.0]] {
        click(&mut app, at, Modifiers::NONE);
    }
    let cases = [([80.0, 40.0], Some(Badge::Add)), ([120.0, 40.0], Some(Badge::Subtract)), ([40.0, 40.0], None), ([120.0, 120.0], None), ([60.0, 100.0], None)];
    for (at, want) in cases {
        app.hover_doc = Some(at);
        assert_eq!(badge(&app, Tool::Pen, Modifiers::NONE), want, "{at:?}");
    }
    app.hover_doc = Some([80.0, 40.0]);
    assert_eq!(badge(&app, Tool::Pen, Modifiers::SHIFT), None);
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Modifiers::NONE);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    for at in [mid, to] {
        tool_event(app, ToolEvent::Move { x: at[0], y: at[1], pressure: 1.0 }, Modifiers::NONE);
    }
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, Modifiers::NONE);
}

/// A smooth knot: both handles pulled out, on one line through the anchor, on opposite sides.
fn assert_smooth(k: &photocraft_doc::Knot) {
    let (vi, vo) = ((k.in_ctrl.x - k.anchor.x, k.in_ctrl.y - k.anchor.y), (k.out_ctrl.x - k.anchor.x, k.out_ctrl.y - k.anchor.y));
    assert!(k.smooth, "{k:?}");
    assert!(vi.0.hypot(vi.1) > 1.0 && vo.0.hypot(vo.1) > 1.0, "both handles pulled out: {k:?}");
    let cross = (vi.0 * vo.1 - vi.1 * vo.0) / (vi.0.hypot(vi.1) * vo.0.hypot(vo.1));
    assert!(cross.abs() < 1e-9 && vi.0 * vo.0 + vi.1 * vo.1 < 0.0, "collinear, opposite handles: {k:?}");
}

/// The same knots within a pixel: the Pen's presses and drags snap (View › Snap), so its anchor
/// lands up to a pixel from the raw pointer.
fn assert_close(got: &Path, want: &Path, what: &str) {
    let knots = |p: &Path| -> Vec<(Point, Point, Point, bool)> {
        p.subpaths.iter().flat_map(|s| s.knots.iter().map(|k| (k.anchor, k.in_ctrl, k.out_ctrl, k.smooth))).collect()
    };
    let (g, w) = (knots(got), knots(want));
    assert_eq!(g.len(), w.len(), "{what}: {got:?}");
    for (a, b) in g.iter().zip(&w) {
        let near = |p: Point, q: Point| (p.x - q.x).hypot(p.y - q.y) <= 1.5;
        assert!(near(a.0, b.0) && near(a.1, b.1) && near(a.2, b.2) && a.3 == b.3, "{what}: {a:?} vs {b:?}");
    }
}

/// `path` with the anchor a click at `at` adds, as the vector edit makes it, and that anchor.
fn added_at(path: &Path, at: Point) -> (Path, [usize; 2]) {
    let mut p = path.clone();
    let Some(photocraft_vector::edit::Hit::Segment(k, t)) = photocraft_vector::edit::hit(&p, at, 6.0, &[]) else { panic!("no segment at {at:?}") };
    let added = photocraft_vector::edit::add_anchor(&mut p, k, t).unwrap();
    (p, added)
}

/// #8653: a click on a curve with Add Anchor Point leaves the new smooth anchor selected, its
/// direction handles showing, and the path's shape unchanged.
#[test]
fn an_added_anchor_on_a_curve_is_selected_smooth_and_shows_its_handles() {
    let mut app = app(Tool::AddAnchorPoint);
    with_path(&mut app);
    let before = work(&app);
    let on = photocraft_vector::edit::eval(&before.subpaths[0].segments()[0], 0.4);
    click(&mut app, [on.x, on.y], Modifiers::NONE);
    let p = work(&app);
    assert_eq!(p, added_at(&before, on).0, "the click is the vector split, nothing more");
    assert_smooth(&p.subpaths[0].knots[1]);
    let ds = &app.ui.direct_selection;
    assert_eq!((ds.target, ds.anchors.clone(), ds.drag.is_none()), (Some(PathRef::Work), vec![[0, 1]], true));
    assert!(crate::direct_select::shows(&app, Modifiers::NONE));
    assert!(app.session.undo());
    assert_eq!(work(&app), before);
}

/// #8653: a drag from the click moves the anchor it adds, handles and all; the
/// add and the move are one history step, for the Add Anchor Point tool and the Pen's Auto Add.
#[test]
fn dragging_from_an_add_moves_the_new_anchor_in_one_step() {
    for tool in [Tool::AddAnchorPoint, Tool::Pen] {
        let mut app = app(tool);
        with_path(&mut app);
        let (before, n) = (work(&app), steps(&app));
        let on = photocraft_vector::edit::eval(&before.subpaths[0].segments()[0], 0.5);
        drag(&mut app, [on.x, on.y], [on.x + 5.0, on.y + 15.0]);
        let (mut want, k) = added_at(&before, on);
        photocraft_vector::edit::move_anchors(&mut want, &[k], [5.0, 15.0]).unwrap();
        assert_close(&work(&app), &want, &format!("{tool:?}: the new anchor moved with its handles"));
        assert_smooth(&work(&app).subpaths[0].knots[1]);
        assert_eq!(steps(&app), n + 1, "{tool:?}: one step");
        assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Add Anchor Point"));
        assert!(app.ui.pen.is_none(), "{tool:?}: no Pen path was started");
        assert_eq!(app.ui.direct_selection.anchors, vec![k], "{tool:?}: it stays selected");
        assert!(crate::direct_select::shows(&app, Modifiers::NONE), "{tool:?}: its handles show");
        assert!(app.session.undo());
        assert_eq!(work(&app), before, "{tool:?}: one undo takes back the add and the move");
    }
}

/// A Pen path being drawn: a corner at (40, 40), then a smooth anchor at (120, 40) dragged down,
/// so the segment between them is a curve.
fn app_with_curve_draft() -> PhotocraftApp {
    let mut app = app(Tool::Pen);
    click(&mut app, [40.0, 40.0], Modifiers::NONE);
    drag(&mut app, [120.0, 40.0], [120.0, 90.0]);
    app
}

fn pen_path(app: &PhotocraftApp) -> Path {
    crate::vector_ui::pen_path(app.ui.pen.as_ref().unwrap())
}

/// #8653 on the path being drawn: the Pen's Auto Add splits a curve into a smooth anchor without
/// changing its shape and selects it; a drag moves it; drawing on deselects it.
#[test]
fn the_pen_adds_a_smooth_anchor_to_a_curve_being_drawn_and_drags_it() {
    let mut app = app_with_curve_draft();
    let before = pen_path(&app);
    let seg = before.subpaths[0].segments()[0];
    assert!(seg[2] != seg[3], "the segment is a curve");
    let on = photocraft_vector::edit::eval(&seg, 0.5);

    click(&mut app, [on.x, on.y], Modifiers::NONE);
    let (want, k) = added_at(&before, on);
    assert_eq!(pen_path(&app), want, "split without changing the shape");
    assert_smooth(&pen_path(&app).subpaths[0].knots[1]);
    assert_eq!((app.ui.direct_selection.target, app.ui.direct_selection.anchors.clone()), (Some(PathRef::Pen), vec![k]));
    click(&mut app, [180.0, 150.0], Modifiers::NONE);
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(4), "drawing carries on from the last anchor");
    assert!(app.ui.direct_selection.anchors.is_empty(), "and deselects the added anchor");

    let mut app = app_with_curve_draft();
    drag(&mut app, [on.x, on.y], [on.x, on.y - 20.0]);
    let (mut want, k) = added_at(&before, on);
    photocraft_vector::edit::move_anchors(&mut want, &[k], [0.0, -20.0]).unwrap();
    assert_close(&pen_path(&app), &want, "the drag moved the new anchor");
    assert_eq!(app.ui.pen.as_ref().map(|p| p.knots.len()), Some(3), "and placed no anchor of its own");
}

/// A press a couple of pixels off an existing anchor hits the anchor (anchors win the hit test), so
/// Add Anchor Point adds nothing there, and a drag from it moves nothing and leaves no selection.
#[test]
fn a_drag_near_an_existing_anchor_adds_and_moves_nothing() {
    let mut app = app(Tool::AddAnchorPoint);
    with_path(&mut app);
    let (before, n) = (work(&app), steps(&app));
    assert!(matches!(photocraft_vector::edit::hit(&before, Point::new(42.0, 41.0), 6.0, &[]), Some(photocraft_vector::edit::Hit::Anchor([0, 0]))));
    drag(&mut app, [42.0, 41.0], [60.0, 70.0]);
    assert_eq!((work(&app), steps(&app)), (before, n));
    assert!(app.ui.direct_selection.anchors.is_empty() && app.ui.direct_selection.drag.is_none());
}
