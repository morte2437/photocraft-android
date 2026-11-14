//! The Triangle tool and Create Triangle name their shape layers Triangle N, not Polygon N (#13353).

use serde_json::{Value, json};

use super::super::{LayerContent, LayerId, LiveShape, shape_path};
use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 160, "background": "white"})).unwrap();
    s
}

/// The params the Triangle tool's drag and Create Triangle's OK send.
fn triangle(rect: [f64; 4]) -> Value {
    json!({"kind": "triangle", "rect": rect})
}

fn name(info: &Value) -> &str {
    info["name"].as_str().unwrap()
}

#[test]
fn triangles_are_named_triangle_n() {
    let mut s = session();
    let first = s.execute("shape.create", triangle([20.0, 20.0, 60.0, 40.0])).unwrap();
    let second = s.execute("shape.create", triangle([90.0, 30.0, 50.0, 50.0])).unwrap();
    assert_eq!((name(&first), name(&second)), ("Triangle 1", "Triangle 2"));
}

#[test]
fn a_triangle_is_a_live_triangle_that_ignores_sides_and_star_ratio() {
    let mut s = session();
    // Any `sides` or `starRatio` sent along is ignored: a triangle has 3 sides and no star.
    let p = json!({"kind": "triangle", "rect": [20, 20, 60, 40], "sides": 7, "starRatio": 0.3, "cornerRadius": 4});
    let id = s.execute("shape.create", p.clone()).unwrap()["layer"].as_u64().unwrap();
    let LayerContent::Shape(sh) = &s.active().unwrap().doc.layer(LayerId(id)).unwrap().content else { panic!("not a shape layer") };
    assert_eq!(sh.live, Some(LiveShape::Triangle { rect: [20.0, 20.0, 60.0, 40.0], corner_radius: 4.0 }));
    // The drag's preview and Path mode draw the same outline, which is not the inscribed 3-sided polygon.
    assert_eq!(sh.path, shape_path(&p).unwrap());
    let polygon = json!({"kind": "polygon", "rect": [20, 20, 60, 40], "sides": 3, "cornerRadius": 4});
    assert_ne!(sh.path, shape_path(&polygon).unwrap());
}

#[test]
fn polygons_and_triangles_count_on_their_own() {
    let mut s = session();
    let tri = s.execute("shape.create", triangle([20.0, 20.0, 60.0, 40.0])).unwrap();
    let poly = s.execute("shape.create", json!({"kind": "polygon", "rect": [90, 30, 50, 50], "sides": 5})).unwrap();
    let three = s.execute("shape.create", json!({"kind": "polygon", "rect": [90, 90, 50, 50], "sides": 3})).unwrap();
    assert_eq!((name(&tri), name(&poly), name(&three)), ("Triangle 1", "Polygon 1", "Polygon 2"));
}

#[test]
fn an_explicit_name_wins() {
    let mut s = session();
    let info = s.execute("shape.create", json!({"kind": "triangle", "rect": [20, 20, 60, 40], "name": "Roof"})).unwrap();
    assert_eq!(name(&info), "Roof");
}

#[test]
fn a_triangle_without_a_rect_is_refused() {
    let mut s = session();
    let history = s.active().unwrap().history.past_len();
    for p in [json!({"kind": "triangle"}), json!({"kind": "triangle", "rect": "x"})] {
        assert!(s.execute("shape.create", p.clone()).is_err(), "{p}");
    }
    assert!(shape_path(&json!({"kind": "triangle"})).is_err());
    assert_eq!(s.active().unwrap().history.past_len(), history);
}
