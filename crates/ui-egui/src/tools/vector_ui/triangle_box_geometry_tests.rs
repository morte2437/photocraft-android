//! #13350: the Triangle tool's drag fills its box, apex at the top centre, in either direction and
//! from the centre with Alt, and Create Triangle sends the same triangle. Shift keeps the
//! equilateral triangle of the constrained square's circle.

use egui::Modifiers;
use serde_json::{Value, json};

use super::{finish_shape, shape_geometry};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app(w: u32, h: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": w, "height": h})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Triangle;
    app
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

/// `shape.info` of the active layer.
fn info(app: &mut PhotocraftApp) -> Value {
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    app.session.execute("shape.info", json!({"layer": id})).unwrap()
}

fn anchors(v: &Value) -> Vec<(f64, f64)> {
    let knots = v["path"]["subpaths"][0]["knots"].as_array().unwrap();
    knots.iter().map(|k| (k["anchor"][0].as_f64().unwrap(), k["anchor"][1].as_f64().unwrap())).collect()
}

fn close(got: &[(f64, f64)], want: &[(f64, f64)], tol: f64) {
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g.0 - w.0).abs() <= tol && (g.1 - w.1).abs() <= tol, "{got:?} vs {want:?}");
    }
}

#[test]
fn a_drag_fills_its_box_in_either_direction() {
    for (from, to) in [([400.0, 300.0], [600.0, 400.0]), ([600.0, 400.0], [400.0, 300.0])] {
        let mut app = app(800, 600);
        let n = steps(&app);
        finish_shape(&mut app, Tool::Triangle, from, to, Modifiers::NONE);
        let v = info(&mut app);
        assert_eq!(anchors(&v), [(500.0, 300.0), (600.0, 400.0), (400.0, 400.0)]);
        assert_eq!(v["live"]["kind"], "triangle");
        assert_eq!(steps(&app), n + 1);
    }
}

#[test]
fn alt_after_the_start_draws_from_the_centre() {
    let mut app = app(800, 600);
    finish_shape(&mut app, Tool::Triangle, [400.0, 300.0], [500.0, 350.0], Modifiers::ALT);
    assert_eq!(anchors(&info(&mut app)), [(400.0, 250.0), (500.0, 350.0), (300.0, 350.0)]);
}

#[test]
fn the_dialog_and_the_drag_send_the_same_triangle() {
    let app = app(800, 600);
    let fields = json!({"__createShape": "triangle", "__at": [400.0, 300.0], "width": 100.0, "height": 100.0, "fromCenter": false});
    let Value::Object(fields) = fields else { panic!("an object") };
    let dialog = crate::shape_dialog::geometry(&fields).unwrap();
    let drag = shape_geometry(&app.ui.tool_options, Tool::Triangle, [400.0, 300.0], [500.0, 400.0], Modifiers::NONE).unwrap();
    assert_eq!(dialog, drag);
    let path = photocraft_engine::vector_cmds::shape_path(&dialog).unwrap();
    let got: Vec<(f64, f64)> = path.subpaths[0].knots.iter().map(|k| (k.anchor.x, k.anchor.y)).collect();
    assert_eq!(got, [(450.0, 300.0), (500.0, 400.0), (400.0, 400.0)]);
}

#[test]
fn shift_still_draws_the_equilateral_triangle_of_the_squares_circle() {
    for mods in [Modifiers::SHIFT, Modifiers::SHIFT | Modifiers::ALT] {
        let mut app = app(800, 600);
        let (start, end) = ([400.0, 300.0], [600.0, 400.0]);
        // The square the Rectangle tool gets for the same drag, whichever side sizes it.
        let sq = shape_geometry(&app.ui.tool_options, Tool::Rectangle, start, end, mods).unwrap();
        let [x, y, m, _]: [f64; 4] = serde_json::from_value(sq["rect"].clone()).unwrap();
        finish_shape(&mut app, Tool::Triangle, start, end, mods);
        let v = info(&mut app);
        assert_eq!(v["live"]["kind"], "triangle");
        let s60 = 60f64.to_radians().sin();
        let want = [(x + m / 2.0, y), (x + m / 2.0 + m / 2.0 * s60, y + 0.75 * m), (x + m / 2.0 - m / 2.0 * s60, y + 0.75 * m)];
        let got = anchors(&v);
        close(&got, &want, 1e-9);
        let d = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).hypot(a.1 - b.1);
        let sides = [d(got[0], got[1]), d(got[1], got[2]), d(got[2], got[0])];
        assert!((sides[0] - sides[1]).abs() < 1e-9 && (sides[1] - sides[2]).abs() < 1e-9, "{sides:?}");
    }
}

#[test]
fn the_corner_widget_rounds_a_live_triangle() {
    let mut app = app(200, 200);
    app.run("shape.create", json!({"kind": "triangle", "rect": [40, 40, 120, 120], "cornerRadius": 20})).unwrap();
    app.ui.tool = Tool::PathSelection;
    let n = steps(&app);
    let cy = 40.0 + 20.0 * 5f64.sqrt();
    let to = cy + 4.0 * 5f64.sqrt();
    tool_event(&mut app, ToolEvent::Down { x: 100.0, y: cy, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 100.0, y: to, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: to }, Modifiers::NONE);
    let v = info(&mut app);
    assert_eq!(v["live"]["kind"], "triangle");
    assert_eq!(v["live"]["rect"], json!([40.0, 40.0, 120.0, 120.0]));
    assert!((v["live"]["corner_radius"].as_f64().unwrap() - 24.0).abs() < 1e-9, "{}", v["live"]);
    assert_eq!(anchors(&v).len(), 6);
    assert_eq!(steps(&app), n + 1);
    assert!(app.session.undo());
    assert_eq!(info(&mut app)["live"]["corner_radius"].as_f64(), Some(20.0));
}
