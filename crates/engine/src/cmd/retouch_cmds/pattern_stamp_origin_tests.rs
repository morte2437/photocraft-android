use super::*;

fn fixture(width: i32, height: i32, depth: u64) -> (Session, String) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 700, "height": 600, "depth": depth})).unwrap();
    let fmt = s.active().unwrap().doc.pixel_format();
    let mut surface = Surface::new(fmt);
    for (rect, color) in [
        (Rect::new(0, 0, width / 2, height / 2), [1.0, 0.0, 0.0, 1.0]),
        (Rect::new(width / 2, 0, width, height / 2), [0.0, 1.0, 0.0, 1.0]),
        (Rect::new(0, height / 2, width / 2, height), [0.0, 0.0, 1.0, 1.0]),
        (Rect::new(width / 2, height / 2, width, height), [1.0; 4]),
    ] {
        surface.fill_rect(rect, &color);
    }
    let pat = Pattern::new("Origin quadrants", surface, width as u32, height as u32);
    let id = pat.id.clone();
    s.edit("Test pattern", |doc, _| {
        doc.patterns.push(pat);
        Ok(())
    })
    .unwrap();
    (s, id)
}

fn stamp(s: &mut Session, id: &str, points: &[(i32, i32)], size: u32, extra: Value) -> Value {
    let mut p = json!({"pattern": id, "points": points, "size": size, "hardness": 100, "spacing": 10, "aligned": false});
    for (key, value) in extra.as_object().unwrap() {
        p[key] = value.clone();
    }
    s.execute("paint.patternStamp", p).unwrap()
}

/// Read a patch spanning tile seams and all four quadrants, rather than only its origin.
fn assert_patch(s: &Session, center: (i32, i32), size: (i32, i32), origin: (i32, i32)) {
    let d = s.active().unwrap();
    let surf = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap();
    for dy in (-15..=15).step_by(5) {
        for dx in (-15..=15).step_by(5) {
            let (x, y) = (center.0 + dx, center.1 + dy);
            let (u, v) = ((x - origin.0).rem_euclid(size.0), (y - origin.1).rem_euclid(size.1));
            let want = match (u < size.0 / 2, v < size.1 / 2) {
                (true, true) => [1.0, 0.0, 0.0, 1.0],
                (false, true) => [0.0, 1.0, 0.0, 1.0],
                (true, false) => [0.0, 0.0, 1.0, 1.0],
                (false, false) => [1.0; 4],
            };
            assert_eq!(surf.rgba(x, y), want, "pixel ({x},{y}), tile {size:?}, origin {origin:?}");
        }
    }
}

#[test]
fn measured_origins_paint_reference_pixels_independent_of_tip_size() {
    // Origins measured in the reference application, per axis modulo the pattern size.
    for (edge, first, origin) in
        [(20, (605, 455), (6, 16)), (20, (500, 500), (11, 11)), (20, (561, 503), (10, 8)), (20, (450, 100), (1, 11)), (30, (150, 500), (16, 26))]
    {
        for depth in [8, 16, 32] {
            for tip in [64, 128] {
                let (mut s, id) = fixture(edge, edge, depth);
                let r = stamp(&mut s, &id, &[first], tip, json!({}));
                assert_eq!(r["phase"], json!([origin.0 as f32, origin.1 as f32]), "{edge}px, {first:?}, tip {tip}, depth {depth}");
                assert_patch(&s, first, (edge, edge), origin);
            }
        }
    }
}

#[test]
fn rectangular_pattern_uses_each_axis_size() {
    let (mut s, id) = fixture(20, 30, 16);
    let first = (150, 500);
    let r = stamp(&mut s, &id, &[first], 64, json!({}));
    assert_eq!(r["phase"], json!([1.0, 26.0]));
    assert_patch(&s, first, (20, 30), (1, 26));
}

#[test]
fn aligned_strokes_keep_document_origin() {
    let (mut s, id) = fixture(20, 20, 8);
    for first in [(605, 455), (500, 500)] {
        let r = stamp(&mut s, &id, &[first], 64, json!({"aligned": true}));
        assert_eq!(r["phase"], json!([0.0, 0.0]));
        assert_patch(&s, first, (20, 20), (0, 0));
    }
}

#[test]
fn explicit_phase_overrides_both_alignment_modes() {
    for aligned in [false, true] {
        let (mut s, id) = fixture(20, 30, 32);
        let r = stamp(&mut s, &id, &[(561, 503)], 64, json!({"aligned": aligned, "phase": [-3, 7]}));
        assert_eq!(r["phase"], json!([-3.0, 7.0]));
        assert_patch(&s, (561, 503), (20, 30), (-3, 7));
    }
}

#[test]
fn one_stroke_keeps_its_origin_and_new_stroke_resets_it() {
    // Two points model a Shift-click line; intermediate points model a dragged stroke.
    for points in [vec![(500, 500), (561, 503)], vec![(500, 500), (530, 490), (561, 503)]] {
        let (mut s, id) = fixture(20, 20, 8);
        let history = s.active().unwrap().history.past_len();
        let r = stamp(&mut s, &id, &points, 64, json!({}));
        assert_eq!(r["phase"], json!([11.0, 11.0]));
        for &point in &points {
            assert_patch(&s, point, (20, 20), (11, 11));
        }
        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
        assert!(s.undo());
        assert!(s.redo());
        assert_patch(&s, (561, 503), (20, 20), (11, 11));

        let r = stamp(&mut s, &id, &[(561, 503)], 64, json!({}));
        assert_eq!(r["phase"], json!([10.0, 8.0]));
        assert_patch(&s, (561, 503), (20, 20), (10, 8));
    }
}
