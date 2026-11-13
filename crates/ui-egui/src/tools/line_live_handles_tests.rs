//! #13263: with the Line tool, the active live line shows a handle at each end and a round widget
//! at its midpoint instead of the shape tools' box, measured against the reference application: a
//! line drawn 400,300 -> 600,400 has handles there and the widget at 500,350; with an arrowhead it
//! also shows its box aligned with the segment, with side handles. Dragging the ends is not
//! measured, so they keep scaling the line as the box's corner handles there did.

use egui::{Color32, Pos2, Shape};
use photocraft_doc::LayerContent;
use photocraft_doc::vector::LiveShape;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::shape_bounds_box_tests::{drag, view};
use crate::state::Tool;

/// An 800 × 600 document with the measured line drawn by the Line tool, arrowheads as given.
fn app(arrows: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Line;
    app.ui.tool_options.arrow_start = arrows;
    app.ui.tool_options.arrow_end = arrows;
    drag(&mut app, [400.0, 300.0], [600.0, 400.0]);
    app
}

/// The active layer's live line ends.
fn ends(app: &PhotocraftApp) -> ([f64; 2], [f64; 2]) {
    let st = app.session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("a shape layer") };
    match sh.live.as_ref() {
        Some(LiveShape::Line { from, to, .. } | LiveShape::ArrowLine { from, to, .. }) => (*from, *to),
        other => panic!("a live line: {other:?}"),
    }
}

/// What the canvas's vector overlay paints in a frame: the centres of the white square handles,
/// the centres of the white round widgets and the closed outlines' points.
struct Painted {
    squares: Vec<Pos2>,
    widgets: Vec<Pos2>,
    closed: Vec<Vec<Pos2>>,
}

fn painted(app: &PhotocraftApp, xf: &ViewXform) -> Painted {
    let ctx = egui::Context::default();
    let doc = &app.session.active().unwrap().doc;
    let mut o = ctx.run_ui(Default::default(), |ui| crate::vector_ui::draw_overlay(app, ui.painter(), xf, doc));
    o.textures_delta.clear();
    let mut p = Painted { squares: Vec::new(), widgets: Vec::new(), closed: Vec::new() };
    for c in &o.shapes {
        match &c.shape {
            // 7 px squares: the anchors Path Selection adds are 6 px.
            Shape::Rect(r) if r.fill == Color32::WHITE && (r.rect.width() - 7.0).abs() < 0.01 => p.squares.push(r.rect.center()),
            Shape::Circle(c) if c.fill == Color32::WHITE => p.widgets.push(c.center),
            Shape::Path(path) if path.closed => p.closed.push(path.points.clone()),
            _ => {}
        }
    }
    p
}

fn scr(xf: &ViewXform, p: [f64; 2]) -> Pos2 {
    xf.to_screen(p[0] as f32, p[1] as f32)
}

fn has(points: &[Pos2], p: Pos2) -> bool {
    points.iter().any(|q| q.distance(p) < 0.5)
}

/// The active layer's id, the document's layer count and its History: the past steps and the last
/// one's name.
fn state(app: &PhotocraftApp) -> (u64, usize, usize, Option<String>) {
    let st = app.session.active().unwrap();
    (st.active_layer.unwrap().0, st.doc.walk().len(), st.history.past_len(), st.history.entries().last().cloned())
}

fn bounds(app: &PhotocraftApp) -> [f64; 4] {
    let st = app.session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("a shape layer") };
    let (x0, y0, x1, y1) = sh.path.control_bounds().unwrap();
    [x0, y0, x1, y1]
}

#[test]
fn a_live_line_shows_its_end_handles_and_a_midpoint_widget_and_no_box() {
    let app = app(false);
    let (from, to) = ends(&app);
    assert!(from.iter().chain(&to).zip([400.0, 300.0, 600.0, 400.0]).all(|(g, w)| (g - w).abs() <= 1.0), "the measured line: {from:?} {to:?}");
    let xf = view(0.0);
    let p = painted(&app, &xf);
    assert_eq!(p.squares.len(), 2, "a handle at each end, not the box's eight: {:?}", p.squares);
    assert!(has(&p.squares, scr(&xf, from)) && has(&p.squares, scr(&xf, to)), "at the ends: {:?}", p.squares);
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    assert_eq!(p.widgets.len(), 1, "{:?}", p.widgets);
    assert!(has(&p.widgets, scr(&xf, mid)), "the round widget at about 500,350: {:?}", p.widgets);
    let axis_box = [[400.0, 300.0], [600.0, 300.0], [600.0, 400.0], [400.0, 400.0]].map(|c| scr(&xf, c));
    let near_box = |pts: &Vec<Pos2>| pts.len() == 4 && axis_box.iter().all(|c| pts.iter().any(|q| q.distance(*c) <= 1.0));
    assert!(!p.closed.iter().any(near_box), "no axis-aligned box: {:?}", p.closed);
    // The hidden box's other corners take no press and show no cursor.
    for corner in [[600.0, 300.0], [400.0, 400.0]] {
        assert!(crate::shape_bounds_box::cursor(&app, &xf, corner).is_none(), "no box handle at {corner:?}");
    }
}

#[test]
fn dragging_an_end_scales_the_line_as_the_box_corner_did() {
    for (handle, from, to) in [([1, 1], [600.0, 400.0], [700.0, 450.0]), ([-1, -1], [400.0, 300.0], [300.0, 250.0])] {
        let mut app = app(false);
        let (id, layers, steps, _) = state(&app);
        let r = bounds(&app);
        assert!(crate::shape_bounds_box::cursor(&app, &view(0.0), from).is_some(), "{from:?}: the corner's cursor");
        drag(&mut app, from, to);
        assert_eq!(state(&app), (id, layers, steps + 1, Some("Transform Path".to_string())), "{from:?}: one Transform Path step, no new layer");
        // About the opposite corner of the box, as that corner handle scaled it.
        let s = crate::shape_bounds_box::scale_for(r, handle, [to[0] - from[0], to[1] - from[1]]);
        let (fx, fy) = if handle[0] > 0 { (r[0], r[1]) } else { (r[2], r[3]) };
        let at = |v: f64, f: f64, s: f64| f + (v - f) * s;
        let (xs, ys) = ([at(r[0], fx, s[0]), at(r[2], fx, s[0])], [at(r[1], fy, s[1]), at(r[3], fy, s[1])]);
        let want = [xs[0].min(xs[1]), ys[0].min(ys[1]), xs[0].max(xs[1]), ys[0].max(ys[1])];
        let got = bounds(&app);
        assert!(s[0] > 1.0 && got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1e-6), "{from:?}: {got:?} vs {want:?}");
    }
}

#[test]
fn a_press_where_the_box_corner_was_draws_a_new_line() {
    let mut app = app(false);
    let (id, layers, ..) = state(&app);
    drag(&mut app, [600.0, 300.0], [650.0, 320.0]);
    let (now, count, _, last) = state(&app);
    assert!(now != id && count == layers + 1, "the Line tool draws a new line there");
    assert_ne!(last.as_deref(), Some("Transform Path"));
}

#[test]
fn an_arrowhead_adds_the_box_aligned_with_the_segment_and_its_side_handles() {
    let app = app(true);
    let (from, to) = ends(&app);
    let xf = view(0.0);
    let p = painted(&app, &xf);
    // Heads at both ends, so the box runs from end to end and is as wide as a head.
    let half = f64::from(app.ui.tool_options.arrow_width) / 2.0;
    let len = (to[0] - from[0]).hypot(to[1] - from[1]);
    let n = [(from[1] - to[1]) / len * half, (to[0] - from[0]) / len * half];
    let corners = [[from[0] + n[0], from[1] + n[1]], [to[0] + n[0], to[1] + n[1]], [to[0] - n[0], to[1] - n[1]], [from[0] - n[0], from[1] - n[1]]];
    let boxed = p.closed.iter().find(|pts| pts.len() == 4 && corners.iter().all(|c| has(pts, scr(&xf, *c))));
    assert!(boxed.is_some(), "a box along the segment, corners {corners:?}: {:?}", p.closed);
    let side = |a: [f64; 2], b: [f64; 2]| scr(&xf, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]);
    for (i, a) in corners.iter().enumerate() {
        let b = corners[(i + 1) % 4];
        assert!(has(&p.squares, side(*a, b)), "a handle in the middle of side {i}: {:?}", p.squares);
    }
    assert!(has(&p.squares, scr(&xf, from)) && has(&p.squares, scr(&xf, to)), "the end handles stay: {:?}", p.squares);
    assert_eq!(p.widgets.len(), 1, "the midpoint widget stays: {:?}", p.widgets);
}

#[test]
fn the_other_shape_tools_still_show_the_box_round_the_line() {
    for tool in [Tool::PathSelection, Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Star] {
        let mut app = app(false);
        app.ui.tool = tool;
        let xf = view(0.0);
        let p = painted(&app, &xf);
        assert_eq!(p.squares.len(), 8, "{tool:?}: the box's eight handles");
        assert!(p.widgets.is_empty(), "{tool:?}: no midpoint widget");
        assert!(crate::shape_bounds_box::cursor(&app, &xf, [600.5, 300.0]).is_some(), "{tool:?}: a box corner");
    }
}

/// The Line tool keeps today's box, and no line controls, round its selection.
fn assert_box(app: &PhotocraftApp, what: &str) {
    let p = painted(app, &view(0.0));
    assert_eq!(p.squares.len(), 8, "{what}: the box's eight handles: {:?}", p.squares);
    assert!(p.widgets.is_empty(), "{what}: no midpoint widget");
}

#[test]
fn the_line_tool_keeps_the_box_round_other_shapes() {
    let mut app = app(false);
    app.run("shape.create", json!({"kind": "rect", "rect": [100, 100, 200, 100], "fill": "#e02020"})).unwrap();
    app.ui.tool = Tool::Line;
    assert_box(&app, "a rectangle");
}

#[test]
fn the_line_tool_keeps_the_box_round_several_selected_layers() {
    let mut app = app(false);
    let (line, ..) = state(&app);
    app.run("shape.create", json!({"kind": "rect", "rect": [100, 100, 200, 100], "fill": "#e02020"})).unwrap();
    app.run("layer.select", json!({"layer": line, "mode": "toggle"})).unwrap();
    app.ui.tool = Tool::Line;
    assert_eq!(app.session.active().unwrap().selected_layers().len(), 2);
    assert_box(&app, "a live line and a rectangle");
}

#[test]
fn the_line_tool_keeps_the_box_round_a_line_that_is_no_longer_live() {
    let mut app = app(false);
    let (line, ..) = state(&app);
    // A skew, unlike a rotation, leaves no line the live parameters could describe.
    app.run("edit.transform", json!({"layer": line, "matrix": [1.0, 0.0, 0.5, 1.0, 0.0, 0.0], "stepName": "Transform Path"})).unwrap();
    let st = app.session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("a shape layer") };
    assert!(sh.live.is_none(), "a skew leaves a plain path: {:?}", sh.live);
    assert_box(&app, "a skewed line");
}
