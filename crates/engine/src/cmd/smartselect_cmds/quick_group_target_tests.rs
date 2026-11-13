//! Quick Selection with a group active (#12933): with Sample All Layers off it reads a flat area,
//! so a click selects only the brush disc; with it on, or with an adjustment layer active, it
//! reads the composite as before.
//!
//! The document is the one the issue measured against the reference application.

use super::*;

const W: i32 = 800;
const H: i32 = 600;
/// The centre of "red 2", the click the issue measured.
const RED2_CLICK: [f32; 2] = [550.0, 150.0];

/// The issue's document: white Background; pixel layer Shapes with red (220,30,30) rectangles
/// 100,100-300,250 ("red 1") and 500,100-600,200 ("red 2"); an empty pixel layer Blue on top; then
/// a new, empty group, which is left active.
fn issue_document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Shapes"})).unwrap();
    for [x, y, w, h] in [[100, 100, 200, 150], [500, 100, 100, 100]] {
        s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
        s.execute("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Blue"})).unwrap();
    s.execute("layer.new.group", json!({})).unwrap();
    s
}

/// A Quick Selection click at `at` with the default 30 px brush.
fn click(s: &mut Session, at: [f32; 2], sample_all: bool) -> Value {
    s.execute("select.quick", json!({"points": [at], "size": 30, "mode": "new", "sampleAllLayers": sample_all})).unwrap()
}

/// The kind of the active layer ("Group", "Adjustment", "Pixel", ...).
fn active_kind(s: &Session) -> &'static str {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().content.kind_name()
}

/// The selection's coverage at every document pixel, row-major.
fn coverage(s: &Session) -> Vec<f32> {
    let sel = s.active().unwrap().doc.selection.clone();
    (0..H).flat_map(|y| (0..W).map(move |x| (x, y))).map(|(x, y)| sel.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))).collect()
}

/// `[left, top, right, bottom]` of the selection, as the issue writes bounds.
fn bounds(s: &Session) -> [i32; 4] {
    let b = s.active().unwrap().doc.selection.as_ref().unwrap().content_bounds();
    [b.x0, b.y0, b.x1, b.y1]
}

/// Asserts the selection covers the whole rectangle `[x0, y0, x1, y1)` and reaches at most
/// `slack` pixels beyond it: the composite's shape, not a brush disc.
fn assert_selects_rect(s: &Session, [x0, y0, x1, y1]: [i32; 4], slack: i32) {
    let cov = coverage(s);
    for y in y0..y1 {
        for x in x0..x1 {
            assert!(cov[(y * W + x) as usize] >= 0.5, "({x},{y}) inside the shape is not selected");
        }
    }
    let [l, t, r, b] = bounds(s);
    assert!(l >= x0 - slack && t >= y0 - slack && r <= x1 + slack && b <= y1 + slack, "bounds {:?} reach past the shape", [l, t, r, b]);
}

#[test]
fn a_group_without_sample_all_layers_selects_only_the_brush_disc() {
    // The disc a click on a flat area gives: Background active, which is white everywhere.
    let mut flat = issue_document();
    let d = flat.active_mut().unwrap();
    d.active_layer = Some(d.doc.layers[0].id);
    assert_eq!(active_kind(&flat), "Pixel");
    click(&mut flat, RED2_CLICK, false);
    let disc = coverage(&flat);

    let mut s = issue_document();
    let before = s.active().unwrap().history.past_len();
    let out = click(&mut s, RED2_CLICK, false);
    assert_eq!(out["changed"], json!(true));
    assert_eq!(s.active().unwrap().history.past_len(), before + 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Quick Selection"));
    assert_eq!(active_kind(&s), "Group");
    // Pixel for pixel the flat-area disc, not red 2 (100 x 100) from the composite.
    assert!(coverage(&s) == disc, "a group click differs from the brush disc on a flat area");
    let [l, t, r, b] = bounds(&s);
    assert!((30..=40).contains(&(r - l)) && (30..=40).contains(&(b - t)), "bounds {:?} are not a 30 px brush disc", [l, t, r, b]);
    assert!(l < 550 && r > 550 && t < 150 && b > 150, "bounds {:?} miss the click", [l, t, r, b]);
}

#[test]
fn a_group_with_sample_all_layers_reads_the_composite() {
    let mut s = issue_document();
    assert_eq!(active_kind(&s), "Group");
    click(&mut s, [200.0, 175.0], true);
    // Measured: 99,99,301,251, all of red 1.
    assert_selects_rect(&s, [100, 100, 300, 250], 3);
}

#[test]
fn an_adjustment_layer_reads_the_composite() {
    let mut s = issue_document();
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    assert_eq!(active_kind(&s), "Adjustment");
    click(&mut s, RED2_CLICK, false);
    assert_selects_rect(&s, [500, 100, 600, 200], 3);
}

#[test]
fn an_artboard_or_a_frame_still_reads_the_composite() {
    // Not measured for these groups, so they keep reading the composite.
    for (cmd, params) in
        [("layer.new.artboard", json!({"rect": [700, 500, 50, 50]})), ("layer.new.frame", json!({"x": 700, "y": 500, "width": 50, "height": 50}))]
    {
        let mut s = issue_document();
        s.execute(cmd, params).unwrap();
        assert_eq!(active_kind(&s), "Group", "{cmd}");
        click(&mut s, RED2_CLICK, false);
        assert_selects_rect(&s, [500, 100, 600, 200], 3);
    }
}
