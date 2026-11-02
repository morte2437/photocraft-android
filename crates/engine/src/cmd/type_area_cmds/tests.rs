use super::*;
use photocraft_doc::text::TextShape;
use photocraft_doc::vector::{Path, PathOp, Subpath};
use photocraft_doc::{LayerContent, LayerId, TextLayer};
use serde_json::json;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth, "background": "white"})).unwrap();
    s
}

fn text_layer(s: &mut Session, text: &str) -> LayerId {
    let r = s.execute("type.create", json!({"x": 100, "y": 100, "text": text, "size": 14, "color": "#000000"})).unwrap();
    LayerId(r["layer"].as_u64().unwrap())
}

fn text(s: &Session, id: LayerId) -> &TextLayer {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t,
        c => panic!("not text: {}", c.kind_name()),
    }
}

fn set_work_path(s: &mut Session, p: Path) {
    s.edit("path", |doc, _| {
        doc.work_path = Some(p);
        Ok(())
    })
    .unwrap();
}

const WORDS: &str = "Well, that went better than expected. Now where did the cat hide the keys?";

#[test]
fn fill_path_wraps_text_inside_the_work_path_in_one_step() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let id = text_layer(&mut s, WORDS);
        set_work_path(&mut s, Path::new(vec![Subpath::polygon(&[(50.0, 50.0), (250.0, 50.0), (250.0, 200.0), (50.0, 200.0)])]));
        let steps = s.active().unwrap().history.entries().len();
        let r = s.execute("type.fillPath", json!({})).unwrap();
        assert_eq!(r["layer"], json!(id.0));
        assert_eq!(s.active().unwrap().history.entries().len(), steps + 1, "one history step");
        assert!(matches!(text(&s, id).shape, TextShape::Area { .. }));
        // The ink stays inside the path, in document space.
        let ink = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds();
        assert!(ink.x0 >= 49 && ink.x1 <= 251 && ink.y0 >= 49 && ink.y1 <= 201, "depth {depth}: {ink:?}");
        assert!(ink.height() > 30, "several lines");
        assert!(s.undo());
        assert_eq!(text(&s, id).shape, TextShape::Point);
        assert!(s.redo());
        assert!(matches!(text(&s, id).shape, TextShape::Area { .. }));
    }
}

#[test]
fn fill_path_uses_a_shape_layer_with_its_hole() {
    let mut s = session(8);
    let shape = s.execute("shape.create", json!({"kind": "rect", "rect": [20, 20, 300, 200], "fill": "#cccccc"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("shape.create", json!({"kind": "rect", "rect": [140, 80, 60, 80], "addTo": shape, "op": "subtract"})).unwrap();
    let id = text_layer(&mut s, &WORDS.repeat(3));
    s.execute("type.fillPath", json!({"layer": id.0, "shape": shape})).unwrap();
    let TextShape::Area { path } = &text(&s, id).shape else { panic!("area") };
    assert_eq!(path.subpaths.len(), 2);
    assert_eq!(path.subpaths[1].op, PathOp::Subtract);
}

#[test]
fn fill_path_maps_the_path_into_text_space() {
    let mut s = session(8);
    let id = text_layer(&mut s, WORDS); // transform: translate(100, 100)
    set_work_path(&mut s, Path::new(vec![Subpath::polygon(&[(100.0, 100.0), (300.0, 100.0), (300.0, 250.0), (100.0, 250.0)])]));
    s.execute("type.fillPath", json!({})).unwrap();
    let TextShape::Area { path } = &text(&s, id).shape else { panic!("area") };
    let a = path.subpaths[0].knots[0].anchor;
    assert!((a.x, a.y) == (0.0, 0.0), "{a:?}");
}

#[test]
fn fill_path_rejects_bad_paths_without_history() {
    let mut s = session(8);
    assert!(!s.is_enabled("type.fillPath"), "no type layer");
    let id = text_layer(&mut s, WORDS);
    assert!(s.is_enabled("type.fillPath"));
    let history = |s: &Session| s.active().unwrap().history.entries().len();
    // No work path.
    assert!(s.execute("type.fillPath", json!({})).is_err());
    for bad in [
        Path::new(vec![Subpath::polyline(&[(0.0, 0.0), (90.0, 90.0), (0.0, 90.0)])]), // open only
        Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (90.0, 0.0), (180.0, 0.0)])]),  // no area
        Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (f64::NAN, 0.0), (9.0, 9.0)])]),
        {
            let mut p = Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (90.0, 0.0), (90.0, 90.0)])]);
            p.inverted = true;
            p
        },
        Path::new(vec![Subpath::polygon(&(0..100_001).map(|i| ((i as f64).cos() * 90.0, (i as f64).sin() * 90.0)).collect::<Vec<_>>())]),
    ] {
        set_work_path(&mut s, bad);
        let h = history(&s);
        assert!(s.execute("type.fillPath", json!({})).is_err());
        assert_eq!(history(&s), h, "no history step on error");
        assert_eq!(text(&s, id).shape, TextShape::Point);
    }
    // Bad params, a shape param that is not a shape layer, and a locked layer: each refused
    // without a history step or a change to the layer.
    let refused = |s: &mut Session, p: serde_json::Value| {
        let h = history(s);
        assert!(s.execute("type.fillPath", p.clone()).is_err(), "{p}");
        assert_eq!(history(s), h, "no history step on error: {p}");
        assert_eq!(text(s, id).shape, TextShape::Point, "{p}");
    };
    for p in [json!({"shape": "x"}), json!({"shape": u64::MAX}), json!({"layer": u64::MAX}), json!({"shape": id.0})] {
        refused(&mut s, p);
    }
    set_work_path(&mut s, Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (200.0, 0.0), (200.0, 200.0)])]));
    s.execute("layer.setProps", json!({"layer": id.0, "locked": true})).unwrap();
    refused(&mut s, json!({}));
}

#[test]
fn release_turns_area_type_into_box_text_over_the_path() {
    let mut s = session(8);
    let id = text_layer(&mut s, WORDS);
    set_work_path(&mut s, Path::new(vec![Subpath::polygon(&[(100.0, 100.0), (300.0, 100.0), (300.0, 250.0)])]));
    s.execute("type.fillPath", json!({})).unwrap();
    assert!(s.is_enabled("type.detachFromPath"));
    assert!(!s.is_enabled("type.convertToPointText") && !s.is_enabled("type.convertToParagraphText"));
    s.execute("type.detachFromPath", json!({})).unwrap();
    assert_eq!(text(&s, id).shape, TextShape::Box { x: 0.0, y: 0.0, width: 200.0, height: 150.0 });
    assert!(s.undo());
    assert!(matches!(text(&s, id).shape, TextShape::Area { .. }));
}

#[test]
fn release_with_a_path_that_is_not_finite_is_an_error() {
    let mut s = session(8);
    let id = text_layer(&mut s, WORDS);
    // A file can carry any coordinates: one NaN or infinite point must not become a NaN text box.
    for bad in [f64::NAN, f64::INFINITY] {
        let path = Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (100.0, bad), (100.0, 100.0)])]);
        s.edit("area", |doc, _| {
            if let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content) {
                t.shape = TextShape::Area { path };
            }
            Ok(())
        })
        .unwrap();
        assert!(s.execute("type.detachFromPath", json!({})).is_err(), "{bad}");
        assert!(matches!(text(&s, id).shape, TextShape::Area { .. }), "unchanged");
    }
}

#[test]
fn area_type_saves_to_psd_as_paragraph_text_with_a_warning() {
    let mut s = session(8);
    text_layer(&mut s, WORDS);
    set_work_path(&mut s, Path::new(vec![Subpath::polygon(&[(100.0, 100.0), (300.0, 100.0), (300.0, 250.0), (100.0, 250.0)])]));
    s.execute("type.fillPath", json!({})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    let r = photocraft_io::export(&doc, "x.psd", &photocraft_io::ExportOptions::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("area type")), "{:?}", r.warnings);
    let back = photocraft_io::import("x.psd", &r.bytes).unwrap().document;
    let t = back.walk().into_iter().find_map(|(_, _, l)| match &l.content {
        LayerContent::Text(t) => Some(t.clone()),
        _ => None,
    });
    let t = t.expect("still a type layer");
    assert_eq!(t.text, WORDS);
    assert!(matches!(t.shape, TextShape::Box { width, height, .. } if (width - 200.0).abs() < 0.5 && (height - 150.0).abs() < 0.5), "{:?}", t.shape);
}

#[test]
fn too_many_anchors_is_its_own_error() {
    // A ring of `n` smooth (curved) anchors around (200, 150).
    let ring = |n: usize| {
        let p = |a: f64, r: f64| photocraft_geom::Point::new(200.0 + r * a.cos(), 150.0 + r * a.sin());
        let step = std::f64::consts::TAU / n as f64;
        let knots = (0..n)
            .map(|i| {
                let a = i as f64 * step;
                let (anchor, d) = (p(a, 100.0), 100.0 * step / 3.0);
                let (tx, ty) = (-a.sin() * d, a.cos() * d);
                let at = |k: f64| photocraft_geom::Point::new(anchor.x + k * tx, anchor.y + k * ty);
                photocraft_doc::vector::Knot::smooth(anchor, at(-1.0), at(1.0))
            })
            .collect();
        Path::new(vec![Subpath { closed: true, knots, op: PathOp::Combine }])
    };
    let mut s = session(8);
    let id = text_layer(&mut s, WORDS);
    // Every anchor up to the limit fits, even with every segment curved.
    set_work_path(&mut s, ring(MAX_AREA_KNOTS));
    s.execute("type.fillPath", json!({})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Area { .. }));
    // One more is refused for what it is, not as a path that encloses nothing.
    let id = text_layer(&mut s, WORDS);
    set_work_path(&mut s, ring(MAX_AREA_KNOTS + 1));
    let e = s.execute("type.fillPath", json!({})).unwrap_err().to_string();
    assert!(e.contains("too many anchor points"), "{e}");
    assert_eq!(text(&s, id).shape, TextShape::Point);
}

/// Image Size with a new resolution keeps area type's pixel size (#7124), so its path grows in
/// text space with the font: the same words wrap onto the same lines, resampled or not.
#[test]
fn a_resolution_change_keeps_area_type_wrapping_where_it_was() {
    for (params, k) in [(json!({"resolution": 144, "resample": "none"}), 1), (json!({"width": 800, "height": 600, "resolution": 144}), 2)] {
        let mut s = session(8);
        let id = text_layer(&mut s, WORDS);
        set_work_path(&mut s, Path::new(vec![Subpath::polygon(&[(50.0, 50.0), (250.0, 50.0), (250.0, 200.0), (50.0, 200.0)])]));
        s.execute("type.fillPath", json!({})).unwrap();
        let lines = |s: &Session| {
            let d = &s.active().unwrap().doc;
            let mut eng = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
            eng.layout(text(s, id), d.resolution_dpi).lines.iter().map(|l| l.range.clone()).collect::<Vec<_>>()
        };
        let size = |s: &Session| {
            let mut t = text(s, id).clone();
            crate::type_cmds::refresh(&s.active().unwrap().doc, &mut t);
            let b = t.cache.unwrap().content_bounds();
            (b.width(), b.height())
        };
        let (before, (w0, h0)) = (lines(&s), size(&s));
        assert!(before.len() >= 3, "{before:?}");
        s.execute("image.imageSize", params.clone()).unwrap();
        assert_eq!(lines(&s), before, "{params}");
        let (w, h) = size(&s);
        assert!(w.abs_diff(w0 * k) <= 3 && h.abs_diff(h0 * k) <= 3, "{params}: {w0}x{h0} -> {w}x{h}");
    }
}
