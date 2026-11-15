//! #13239: with the Ellipse tool, the box round the active ellipse shows a centre mark (a
//! crosshair in a small circle) as well as its eight handles, measured against the reference
//! application: a drag from 400,300 to 600,400 makes Ellipse 1, box 399,299 to 600,400.

use egui::epaint::ColorMode;
use egui::{Color32, Modifiers, Pos2, Shape};
use photocraft_doc::LayerContent;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform, tool_event};
use crate::state::Tool;

const NONE: Modifiers = Modifiers::NONE;

/// An 800 × 600 document with Ellipse 1 drawn by the measured drag, the Ellipse tool current.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::EllipseShape;
    tool_event(&mut app, ToolEvent::Down { x: 400.0, y: 300.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Move { x: 500.0, y: 350.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Move { x: 600.0, y: 400.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Up { x: 600.0, y: 400.0 }, NONE);
    app
}

/// The active shape's box `[x0, y0, x1, y1]`.
fn bounds(app: &PhotocraftApp) -> [f64; 4] {
    let st = app.session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("a shape layer") };
    let (x0, y0, x1, y1) = sh.path.control_bounds().unwrap();
    [x0, y0, x1, y1]
}

/// A view at `zoom` screen px per document px, the document centred.
fn view(zoom: f32) -> ViewXform {
    let rect = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0));
    ViewXform { rect, zoom, center: [500.0, 350.0], flip: false, rotation: 0.0, aspect: 1.0 }
}

/// What the canvas's vector overlay paints in a frame: the hollow circles, the line segments and
/// the colours of the box's outline and handles.
struct Painted {
    circles: Vec<(Pos2, f32, Color32)>,
    lines: Vec<([Pos2; 2], Color32)>,
    box_colours: Vec<Color32>,
}

fn painted(app: &PhotocraftApp, xf: &ViewXform) -> Painted {
    let ctx = egui::Context::default();
    let doc = &app.session.active().unwrap().doc;
    let mut o = ctx.run_ui(Default::default(), |ui| crate::vector_ui::draw_overlay(app, ui.painter(), xf, doc));
    o.textures_delta.clear();
    let mut p = Painted { circles: Vec::new(), lines: Vec::new(), box_colours: Vec::new() };
    for c in &o.shapes {
        match &c.shape {
            Shape::Circle(c) if c.fill == Color32::TRANSPARENT => p.circles.push((c.center, c.radius, c.stroke.color)),
            Shape::LineSegment { points, stroke } => p.lines.push((*points, stroke.color)),
            // The box outline is the only four-point closed path; each 7 × 7 handle is a white
            // square under a hollow one that carries its stroke.
            Shape::Path(path) if path.closed && path.points.len() == 4 => {
                if let ColorMode::Solid(c) = path.stroke.color {
                    p.box_colours.push(c);
                }
            }
            Shape::Rect(r) if r.fill == Color32::TRANSPARENT && r.rect.size() == egui::vec2(7.0, 7.0) => p.box_colours.push(r.stroke.color),
            _ => {}
        }
    }
    p
}

/// The centre mark's radius on screen and colour if one is painted at screen point `c`: a hollow
/// circle there with a horizontal and a vertical line through it, all in one colour.
fn mark_at(app: &PhotocraftApp, xf: &ViewXform, c: Pos2) -> Option<(f32, Color32)> {
    let p = painted(app, xf);
    let near = |q: Pos2| q.distance(c) < 0.5;
    let (_, radius, colour) = p.circles.into_iter().find(|(at, ..)| near(*at))?;
    let through = |[a, b]: [Pos2; 2]| near(Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0));
    let horizontal = p.lines.iter().any(|&(l, lc)| through(l) && lc == colour && (l[0].y - l[1].y).abs() < 1e-3);
    let vertical = p.lines.iter().any(|&(l, lc)| through(l) && lc == colour && (l[0].x - l[1].x).abs() < 1e-3);
    (horizontal && vertical).then_some((radius, colour))
}

fn centre(r: [f64; 4]) -> [f32; 2] {
    [((r[0] + r[2]) / 2.0) as f32, ((r[1] + r[3]) / 2.0) as f32]
}

#[test]
fn the_ellipse_tool_marks_the_box_centre() {
    let app = app();
    let r = bounds(&app);
    assert!(r.iter().zip([399.0, 299.0, 600.0, 400.0]).all(|(g, w)| (g - w).abs() <= 1.0), "Ellipse 1's box: {r:?}");
    let [cx, cy] = centre(r);
    assert!((cx - 500.0).abs() <= 1.0 && (cy - 350.0).abs() <= 1.0, "about 500,350: {cx},{cy}");
    let xf = view(1.0);
    let (_, colour) = mark_at(&app, &xf, xf.to_screen(cx, cy)).expect("a crosshair in a circle at the box centre");
    let box_colours = painted(&app, &xf).box_colours;
    assert_eq!(box_colours.len(), 9, "the outline and its eight handles");
    assert!(box_colours.iter().all(|c| *c == colour), "the mark has the box's colour: {colour:?} vs {box_colours:?}");
}

#[test]
fn the_mark_keeps_its_screen_size_at_every_zoom() {
    let app = app();
    let [cx, cy] = centre(bounds(&app));
    let sizes: Vec<Option<f32>> = [0.25, 1.0, 4.0].into_iter().map(|z| mark_at(&app, &view(z), view(z).to_screen(cx, cy)).map(|(r, _)| r)).collect();
    assert!(sizes[0].is_some(), "{sizes:?}");
    assert!(sizes.iter().all(|s| *s == sizes[0]), "the same radius on screen: {sizes:?}");
}

#[test]
fn the_mark_follows_the_box_while_a_handle_is_dragged() {
    let mut app = app();
    let r = bounds(&app);
    let right = [r[2], (r[1] + r[3]) / 2.0];
    tool_event(&mut app, ToolEvent::Down { x: right[0], y: right[1], pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Move { x: right[0] + 100.0, y: right[1], pressure: 1.0 }, NONE);
    assert!(app.ui.shape_box.is_some(), "the right handle is dragged");
    let xf = view(1.0);
    let [cx, cy] = centre([r[0], r[1], r[2] + 100.0, r[3]]);
    assert!(mark_at(&app, &xf, xf.to_screen(cx, cy)).is_some(), "at the centre of the scaled box");
    let [ox, oy] = centre(r);
    assert!(mark_at(&app, &xf, xf.to_screen(ox, oy)).is_none(), "not where the box was");
}

/// A drag by (+40, +30) from document point `from` with the Ellipse tool over Ellipse 1: whether
/// the box took the press, the layer count, the History length and the new active shape's size.
fn drag_from(from: [f64; 2]) -> (bool, usize, usize, [f64; 2]) {
    let mut app = app();
    tool_event(&mut app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, NONE);
    let boxed = app.ui.shape_box.is_some();
    tool_event(&mut app, ToolEvent::Move { x: from[0] + 20.0, y: from[1] + 15.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Move { x: from[0] + 40.0, y: from[1] + 30.0, pressure: 1.0 }, NONE);
    tool_event(&mut app, ToolEvent::Up { x: from[0] + 40.0, y: from[1] + 30.0 }, NONE);
    let st = app.session.active().unwrap();
    let r = bounds(&app);
    (boxed, st.doc.walk().len(), st.history.past_len(), [r[2] - r[0], r[3] - r[1]])
}

#[test]
fn a_press_on_the_mark_does_what_a_press_elsewhere_on_the_shape_does() {
    let r = bounds(&app());
    let [cx, cy] = centre(r);
    let on_mark = drag_from([f64::from(cx), f64::from(cy)]);
    assert!(!on_mark.0, "the box takes no press at its centre");
    assert_eq!(on_mark, drag_from([r[0] + 50.0, f64::from(cy)]), "the same as a press elsewhere on Ellipse 1");
}
