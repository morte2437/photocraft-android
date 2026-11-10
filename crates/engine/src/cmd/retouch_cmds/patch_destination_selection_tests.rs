//! Patch in Destination mode moves a plain rectangular selection with the patch (#12119).

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "depth": 8, "mode": "rgb"})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let v = if (x / 8 + y / 8) % 2 == 0 { 0.2 } else { 0.8 };
                let px = if x >= 400 { [0.1, 0.6, 0.2, 1.0] } else { [v, v, v, 1.0] };
                data.extend(photocraft_raster::from_rgba(&fmt, px));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

fn bounds(s: &mut Session) -> Value {
    s.execute("document.inspect", json!({})).unwrap()["selectionBounds"].clone()
}

fn px(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

fn history_len(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn select_rect(s: &mut Session) {
    s.execute("select.rect", json!({"x": 120, "y": 120, "width": 40, "height": 40})).unwrap();
}

const DESTINATION: fn() -> Value = || json!({"mode": "destination", "offset": [460, 280]});

#[test]
fn destination_moves_the_selection_with_the_patch_and_undo_restores_both() {
    let mut s = session();
    select_rect(&mut s);
    let before = px(&s, 600, 420);
    let source_before: Vec<[f32; 4]> = [(130, 130), (140, 140), (148, 140)].iter().map(|&(x, y)| px(&s, x, y)).collect();
    let r = s.execute("paint.patch", DESTINATION()).unwrap();
    assert_eq!(r["offset"], json!([460, 280]));
    // The damage covers the patched area (the 40 px square at the destination, plus the solve's ring).
    assert_eq!(r["damage"], json!([578, 398, 44, 44]));
    assert_eq!(bounds(&mut s), json!([580, 400, 40, 40]));
    assert_ne!(px(&s, 600, 420), before, "pixels changed too");
    // Source pixels untouched.
    let source_after: Vec<[f32; 4]> = [(130, 130), (140, 140), (148, 140)].iter().map(|&(x, y)| px(&s, x, y)).collect();
    assert_eq!(source_after, source_before);
    // The destination carries the source texture: the step between two neighbouring checker cells is kept
    // in the same direction and at a similar strength (the seamless clone adds a smooth colour correction, and the
    // red channel clamps at 0 on the dark green, so the step shrinks).
    let step_src = px(&s, 140, 140)[0] - px(&s, 148, 140)[0];
    let step_dst = px(&s, 600, 420)[0] - px(&s, 608, 420)[0];
    assert!(step_src * step_dst > 0.0 && step_dst.abs() > 0.2, "texture kept: source step {step_src}, destination step {step_dst}");
    // The moved selection is still a hard-edged rectangle exactly filling its bounds.
    let d = s.active().unwrap();
    let sel = d.doc.selection.as_ref().unwrap();
    assert!(is_hard_rect(sel, sel.content_bounds()));
    // One history state: a single undo brings back the pixels and the old selection.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(bounds(&mut s), json!([120, 120, 40, 40]));
    assert_eq!(px(&s, 600, 420), before);
}

#[test]
fn destination_selection_stays_when_it_is_not_a_plain_rectangle() {
    let cases: [(&str, Value, Value); 4] = [
        ("feathered", json!({"x": 120, "y": 120, "width": 40, "height": 40, "feather": 6.0}), json!(null)),
        ("ellipse", json!({"x": 120, "y": 120, "width": 40, "height": 40, "ellipse": true}), json!(null)),
        ("lasso", json!({"points": [[120, 120], [160, 130], [140, 165]]}), json!("lasso")),
        ("rectangle with a hole", json!({"x": 120, "y": 120, "width": 40, "height": 40}), json!("hole")),
    ];
    for (name, params, kind) in cases {
        let mut s = session();
        let cmd = if kind == json!("lasso") { "select.lasso" } else { "select.rect" };
        s.execute(cmd, params).unwrap();
        if kind == json!("hole") {
            // Same bounds as the rectangle, but not every pixel inside is fully selected.
            s.execute("select.rect", json!({"x": 135, "y": 135, "width": 10, "height": 10, "mode": "subtract"})).unwrap();
        }
        let sel_before = bounds(&mut s);
        let area = |s: &Session| -> Vec<[f32; 4]> { (400..440).flat_map(|y| (580..620).map(move |x| (x, y))).map(|(x, y)| px(s, x, y)).collect() };
        let before = area(&s);
        s.execute("paint.patch", DESTINATION()).unwrap();
        assert_eq!(bounds(&mut s), sel_before, "{name}: the selection stays in place");
        assert_ne!(area(&s), before, "{name}: the patch still lands");
    }
}

#[test]
fn partial_coverage_and_oversized_rectangles_are_not_constructible_apart_from_the_hole_case() {
    // `plan.area` is the selection's own content bounds, so a rectangle larger than the area cannot
    // exist, and the area is never wider than the selection. The only way the bounds equal the area
    // yet coverage falls short is a hole (covered above). An inverted selection makes the area the
    // whole canvas, which no non-zero drag can keep on the canvas, so Patch refuses it.
    let mut s = session();
    select_rect(&mut s);
    s.execute("select.inverse", json!({})).unwrap();
    assert!(s.execute("paint.patch", DESTINATION()).is_err());
}

#[test]
fn destination_without_a_selection_fails_and_leaves_history_alone() {
    let mut s = session();
    // A marker step after the setup, so a stray undo would be visible.
    s.edit("marker", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 10, 20, 20), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let h = history_len(&s);
    assert!(s.execute("paint.patch", DESTINATION()).is_err());
    assert_eq!(history_len(&s), h, "no history entry");
    // Undo reverts the marker (the last real step), not the setup.
    s.execute("edit.undo", json!({})).unwrap();
    assert_ne!(px(&s, 15, 15), [1.0, 0.0, 0.0, 1.0], "the marker step was undone");
    assert!((px(&s, 450, 300)[0] - 0.1).abs() < 0.01 && (px(&s, 450, 300)[1] - 0.6).abs() < 0.01, "the setup step is still there");
}

#[test]
fn redo_restores_the_patched_pixels_and_the_moved_selection() {
    let mut s = session();
    select_rect(&mut s);
    let before = px(&s, 600, 420);
    s.execute("paint.patch", DESTINATION()).unwrap();
    let patched = px(&s, 600, 420);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(bounds(&mut s), json!([120, 120, 40, 40]));
    assert_eq!(px(&s, 600, 420), before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(bounds(&mut s), json!([580, 400, 40, 40]));
    assert_eq!(px(&s, 600, 420), patched);
}

#[test]
fn source_mode_keeps_the_selection() {
    let mut s = session();
    select_rect(&mut s);
    s.execute("paint.patch", json!({"offset": [260, 0]})).unwrap();
    assert_eq!(bounds(&mut s), json!([120, 120, 40, 40]));
}

#[test]
fn content_aware_keeps_the_selection_and_changes_the_pixels() {
    let mut s = session();
    select_rect(&mut s);
    // Content-Aware ignores the mode and fills the selection itself from the dragged-to place (the
    // green half), so the pixels under the selection change.
    let before = px(&s, 140, 140);
    s.execute("paint.patch", json!({"offset": [400, 0], "contentAware": true})).unwrap();
    assert_eq!(bounds(&mut s), json!([120, 120, 40, 40]));
    assert_ne!(px(&s, 140, 140), before, "the selected area was filled");
}

#[test]
fn destination_moves_a_hard_rectangle_that_extends_past_the_canvas() {
    let mut s = session();
    // 60x40 rectangle with 20 columns off the left edge; the part on the canvas is x 0..40.
    s.execute("select.rect", json!({"x": 0, "y": 100, "width": 60, "height": 40})).unwrap();
    s.execute("select.transformSelection", json!({"dx": -20, "dy": 0})).unwrap();
    let sel_before = bounds(&mut s);
    let before = px(&s, 480, 400);
    s.execute("paint.patch", DESTINATION()).unwrap();
    assert_ne!(px(&s, 480, 400), before, "the patch lands on the canvas part");
    let d = s.active().unwrap();
    let sel = d.doc.selection.as_ref().unwrap();
    assert_eq!(sel.content_bounds(), Rect::new(440, 380, 500, 420), "the whole rectangle moved, off-canvas part included");
    assert!(sel.read_region(Rect::new(440, 380, 500, 420)).chunks_exact(sel.channels().max(1)).all(|p| p[0] >= 1.0));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(bounds(&mut s), sel_before);
    assert_eq!(px(&s, 480, 400), before);
}

#[test]
fn hard_rect_check_is_capped_and_scans_every_band() {
    let mut s = session();
    // 40x150 rectangle: three bands of 64 rows (64 + 64 + 22).
    s.execute("select.rect", json!({"x": 100, "y": 100, "width": 40, "height": 150})).unwrap();
    let d = s.active().unwrap();
    let sel = d.doc.selection.as_ref().unwrap();
    let b = sel.content_bounds();
    assert_eq!(b, Rect::new(100, 100, 140, 250));
    assert!(!is_hard_rect_within(sel, b, 5999), "over the limit is never a hard rectangle");
    assert!(is_hard_rect_within(sel, b, 6000), "at the limit it is checked as usual");
    assert!(is_hard_rect_within(sel, b, u64::MAX));
    // A hole in the third band must still be found.
    let mut holed = sel.clone();
    let fmt = holed.format();
    holed.write_region(Rect::new(120, 240, 121, 241), &photocraft_raster::from_rgba(&fmt, [0.0, 0.0, 0.0, 0.0]));
    assert!(!is_hard_rect_within(&holed, b, u64::MAX), "a hole past the first band fails the check");
}
