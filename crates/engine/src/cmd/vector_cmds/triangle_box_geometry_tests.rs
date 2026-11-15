//! The Triangle tool and Create Triangle draw a triangle that fills the dragged box, apex at the
//! top centre, as its own live kind (#13350). A 3-sided polygon stays inscribed in its ellipse.

use serde_json::{Value, json};

use super::super::{LayerContent, LayerId};
use super::shape_path;
use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    s
}

fn create(s: &mut Session, p: Value) -> u64 {
    s.execute("shape.create", p).unwrap()["layer"].as_u64().unwrap()
}

fn info(s: &mut Session, id: u64) -> Value {
    s.execute("shape.info", json!({"layer": id})).unwrap()
}

fn anchors(v: &Value) -> Vec<(f64, f64)> {
    let knots = v["path"]["subpaths"][0]["knots"].as_array().unwrap();
    knots.iter().map(|k| (k["anchor"][0].as_f64().unwrap(), k["anchor"][1].as_f64().unwrap())).collect()
}

fn path_anchors(p: &Value) -> Vec<(f64, f64)> {
    shape_path(p).unwrap().subpaths[0].knots.iter().map(|k| (k.anchor.x, k.anchor.y)).collect()
}

fn close(got: &[(f64, f64)], want: &[(f64, f64)], tol: f64) {
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g.0 - w.0).abs() <= tol && (g.1 - w.1).abs() <= tol, "{got:?} vs {want:?}");
    }
}

type Tri = [(f64, f64); 3];

const BIG: [(f64, f64); 3] = [(500.0, 300.0), (600.0, 400.0), (400.0, 400.0)];
const ROUND: [(f64, f64); 6] = [(485.858, 314.142), (514.142, 314.142), (565.858, 365.858), (551.716, 400.0), (448.284, 400.0), (434.142, 365.858)];

#[test]
fn a_triangle_fills_its_box() {
    let mut s = session();
    let id = create(&mut s, json!({"kind": "triangle", "rect": [400, 300, 200, 100], "fill": "#000000"}));
    let v = info(&mut s, id);
    assert_eq!(anchors(&v), BIG);
    assert_eq!(v["kind"], "triangle");
    assert_eq!(v["live"], json!({"kind": "triangle", "rect": [400.0, 300.0, 200.0, 100.0], "corner_radius": 0.0}));
    let LayerContent::Shape(sh) = &s.active().unwrap().doc.layer(LayerId(id)).unwrap().content else { panic!("not a shape layer") };
    let c = sh.cache.as_ref().unwrap();
    assert!(c.rgba(598, 399)[3] > 0.9 && c.rgba(402, 399)[3] > 0.9, "the base corners are filled");
    assert!(c.rgba(410, 310)[3] < 0.01, "above the left edge is empty");
}

#[test]
fn the_preview_and_the_measured_boxes_agree() {
    let mut s = session();
    let cases: [([f64; 4], Tri); 3] = [
        ([400.0, 300.0, 200.0, 100.0], BIG),
        ([400.0, 300.0, 100.0, 100.0], [(450.0, 300.0), (500.0, 400.0), (400.0, 400.0)]),
        ([300.0, 250.0, 200.0, 100.0], [(400.0, 250.0), (500.0, 350.0), (300.0, 350.0)]),
    ];
    for (rect, want) in cases {
        let p = json!({"kind": "triangle", "rect": rect});
        assert_eq!(path_anchors(&p), want);
        let id = create(&mut s, p.clone());
        assert_eq!(
            shape_path(&p).unwrap(),
            match &s.active().unwrap().doc.layer(LayerId(id)).unwrap().content {
                LayerContent::Shape(sh) => sh.path.clone(),
                _ => panic!("not a shape layer"),
            }
        );
    }
}

#[test]
fn a_negative_rect_gives_the_same_triangle_apex_up() {
    let mut s = session();
    let id = create(&mut s, json!({"kind": "triangle", "rect": [600, 400, -200, -100]}));
    assert_eq!(anchors(&info(&mut s, id)), BIG);
}

#[test]
fn editing_the_radius_or_the_box_keeps_the_triangle() {
    let mut s = session();
    let id = create(&mut s, json!({"kind": "triangle", "rect": [400, 300, 200, 100]}));
    s.execute("shape.edit", json!({"layer": id, "cornerRadius": 20})).unwrap();
    let v = info(&mut s, id);
    close(&anchors(&v), &ROUND, 1e-3);
    assert_eq!(v["live"]["rect"], json!([400.0, 300.0, 200.0, 100.0]));
    assert_eq!(v["live"]["corner_radius"], json!(20.0));
    s.execute("shape.edit", json!({"layer": id, "rect": [10, 10, 50, 80]})).unwrap();
    let v = info(&mut s, id);
    assert_eq!((v["live"]["kind"].as_str(), v["live"]["corner_radius"].as_f64()), (Some("triangle"), Some(20.0)));
    let before = v["live"].clone();
    s.execute("shape.edit", json!({"layer": id, "sides": 7, "starRatio": 0.3})).unwrap();
    assert_eq!(info(&mut s, id)["live"], before);
}

/// `Session::execute` turns a command's panic into an `Err`, so a hostile-input test has to tell
/// that error from a graceful one.
fn graceful(r: &crate::Result<Value>) -> bool {
    !matches!(r, Err(e) if e.to_string().contains("internal error"))
}

#[test]
fn hostile_radii_never_panic() {
    let mut s = session();
    let id = create(&mut s, json!({"kind": "triangle", "rect": [400, 300, 200, 100]}));
    for r in [json!(-5), json!(1e300), json!("x")] {
        let result = s.execute("shape.edit", json!({"layer": id, "cornerRadius": r}));
        assert!(graceful(&result), "{result:?}");
        let r = info(&mut s, id)["live"]["corner_radius"].as_f64().unwrap();
        assert!(r.is_finite() && r >= 0.0, "{r}");
    }
}

#[test]
fn hostile_boxes_never_panic() {
    let mut s = session();
    let layers = s.active().unwrap().doc.layer_count();
    // Out of range, or not a box: refused, and no layer is added.
    for rect in [json!([0, 0, 1e300, 1e300]), json!([1e300, 1e300, 10, 10]), json!([0, 0, 10]), json!([0, 0, "w", 10]), json!(null)] {
        let result = s.execute("shape.create", json!({"kind": "triangle", "rect": rect, "cornerRadius": 20}));
        assert!(result.is_err() && graceful(&result), "{result:?}");
    }
    assert_eq!(s.active().unwrap().doc.layer_count(), layers);
    // A box with no width or no height has no corner to round.
    for rect in [[5, 5, 0, 0], [5, 5, 0, 50], [5, 5, 50, 0]] {
        let result = s.execute("shape.create", json!({"kind": "triangle", "rect": rect, "cornerRadius": 20}));
        assert!(graceful(&result), "{rect:?}: {result:?}");
    }
}

#[test]
fn a_scale_keeps_the_triangle_live_and_a_flip_makes_it_a_path() {
    let mut s = session();
    let id = create(&mut s, json!({"kind": "triangle", "rect": [400, 300, 200, 100], "cornerRadius": 10}));
    s.execute("shape.edit", json!({"layer": id, "transform": [0.5, 0, 0, 0.5, 0, 0]})).unwrap();
    let v = info(&mut s, id);
    assert_eq!(v["live"]["rect"], json!([200.0, 150.0, 100.0, 50.0]));
    assert_eq!(v["live"]["corner_radius"], json!(5.0));
    let id = create(&mut s, json!({"kind": "triangle", "rect": [400, 300, 200, 100]}));
    s.execute("shape.edit", json!({"layer": id, "transform": [1, 0, 0, -1, 0, 700]})).unwrap();
    let v = info(&mut s, id);
    assert!(v["live"].is_null());
    assert_eq!(v["kind"], "path");
    assert_eq!(anchors(&v), [(500.0, 400.0), (600.0, 300.0), (400.0, 300.0)]);
}

#[test]
fn a_three_sided_polygon_and_undo_are_unchanged() {
    let mut s = session();
    let id = create(&mut s, json!({"kind": "polygon", "rect": [400, 300, 200, 100], "sides": 3}));
    let v = info(&mut s, id);
    close(&anchors(&v), &[(500.0, 300.0), (586.603, 375.0), (413.397, 375.0)], 1e-3);
    assert_eq!(v["kind"], "polygon");

    let id = create(&mut s, json!({"kind": "triangle", "rect": [400, 300, 200, 100]}));
    s.execute("shape.edit", json!({"layer": id, "cornerRadius": 20})).unwrap();
    let before = info(&mut s, id);
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.redo", json!({})).unwrap();
    let after = info(&mut s, id);
    assert_eq!((&after["live"], &after["path"]), (&before["live"], &before["path"]));
}
