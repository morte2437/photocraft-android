//! Object Selection's Object Finder click on a transparent layer (#12852): a mid-grey object is
//! an object of its own, and transparency stops the growth instead of reading as 50 % grey.
//!
//! The documents are the ones the issue measured against the reference application.

use super::*;

const W: u32 = 800;
const H: u32 = 600;

/// The issue's document: white Background, then a transparent pixel layer holding a red
/// rectangle, a blue disc and a ten-block grey ramp (100, 105, …, 145 from x 100).
fn issue_document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    paint_new_layer(&mut s, W, H, |x, y| {
        let (x, y) = (x as i32, y as i32);
        if (100..300).contains(&x) && (100..250).contains(&y) {
            return [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0];
        }
        let (dx, dy) = (x as f32 + 0.5 - 500.0, y as f32 + 0.5 - 175.0);
        if dx * dx + dy * dy <= 75.0 * 75.0 {
            return [30.0 / 255.0, 30.0 / 255.0, 220.0 / 255.0, 1.0];
        }
        if (100..500).contains(&x) && (350..450).contains(&y) {
            let v = (100 + 5 * ((x - 100) / 40)) as f32 / 255.0;
            return [v, v, v, 1.0];
        }
        [0.0, 0.0, 0.0, 0.0]
    });
    s
}

/// A Grayscale 8-bit document with a grey-87 rectangle on a transparent layer.
fn gray_document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "mode": "gray", "depth": 8})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    paint_new_layer(&mut s, 400, 300, |x, y| {
        let (x, y) = (x as i32, y as i32);
        if (100..300).contains(&x) && (100..250).contains(&y) { [87.0 / 255.0, 0.0, 0.0, 1.0] } else { [0.0; 4] }
    });
    s
}

/// Writes `px` (straight RGBA) over the whole surface of the last layer, which `file.new` plus
/// `layer.new.layer` leaves transparent.
fn paint_new_layer(s: &mut Session, w: u32, h: u32, px: impl Fn(u32, u32) -> [f32; 4]) {
    s.edit("paint", |doc, _| {
        let Some(surf) = doc.layers.last_mut().and_then(Layer::surface_mut) else {
            return Err(EngineError::Other("the new layer has no surface".into()));
        };
        let n = surf.channels();
        let mut data = Vec::with_capacity((w * h) as usize * n);
        for y in 0..h {
            for x in 0..w {
                let c = px(x, y);
                if n >= 4 {
                    data.extend_from_slice(&c);
                } else if n == 2 {
                    data.extend_from_slice(&[c[0], c[3]]);
                } else {
                    data.extend_from_slice(&c[..n.min(3)]);
                }
            }
        }
        surf.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
        Ok(())
    })
    .unwrap();
}

fn cov(s: &Session, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
}

/// `[left, top, right, bottom]` of the selection, as the issue writes bounds.
fn bounds(s: &Session) -> [i32; 4] {
    let b = s.active().unwrap().doc.selection.as_ref().unwrap().content_bounds();
    [b.x0, b.y0, b.x1, b.y1]
}

#[test]
fn a_grey_ramp_on_a_transparent_layer_is_found_at_both_points() {
    for (x, y) in [(120, 400), (300, 400)] {
        let mut s = issue_document();
        let before = s.active().unwrap().history.past_len();
        let r = s.execute("select.object", json!({"point": [x, y]})).unwrap();
        assert_eq!(r["changed"], true, "click at {x},{y}");
        assert_eq!(bounds(&s), [99, 349, 501, 451], "click at {x},{y}");
        // The whole ramp, and nothing around it.
        assert!(cov(&s, 120, 400) >= 0.5 && cov(&s, 480, 440) >= 0.5);
        assert!(cov(&s, 20, 20) < 0.5 && cov(&s, 700, 500) < 0.5);
        // One "Object Selection" step, undone in one go.
        assert_eq!(s.active().unwrap().history.past_len(), before + 1);
        assert_eq!(s.active().unwrap().history.undo_label(), Some("Object Selection"));
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.selection.is_none());
    }
}

#[test]
fn a_grey_rectangle_on_a_transparent_gray_layer_is_found() {
    let mut s = gray_document();
    let r = s.execute("select.object", json!({"point": [150, 150]})).unwrap();
    assert_eq!(r["changed"], true);
    assert_eq!(bounds(&s), [99, 99, 301, 251]);
    assert!(cov(&s, 150, 150) >= 0.5 && cov(&s, 20, 20) < 0.5);
}

#[test]
fn clicking_transparency_selects_nothing() {
    let mut s = issue_document();
    // Transparent pixels inside the layer's own content: between the rectangle and the disc, and
    // between the rectangle and the ramp.
    for (x, y) in [(350, 150), (200, 300)] {
        let r = s.execute("select.object", json!({"point": [x, y]})).unwrap();
        assert_eq!(r["changed"], false, "click at {x},{y}");
        assert!(s.active().unwrap().doc.selection.is_none());
    }
}

#[test]
fn red_and_blue_still_select_their_own_objects() {
    let mut s = issue_document();
    for (x, y) in [(200, 150), (500, 175)] {
        let r = s.execute("select.object", json!({"point": [x, y]})).unwrap();
        assert_eq!(r["changed"], true, "click at {x},{y}");
        assert!(cov(&s, x, y) >= 0.5);
        // The click stays on its own object.
        let other = if x < 400 { (500, 175) } else { (200, 150) };
        assert!(cov(&s, other.0, other.1) < 0.5, "click at {x},{y} leaked into the other object");
    }
}

#[test]
fn sampling_all_layers_still_finds_the_ramp() {
    let mut s = issue_document();
    let r = s.execute("select.object", json!({"point": [120, 400], "sampleAllLayers": true})).unwrap();
    assert_eq!(r["changed"], true);
    assert!(cov(&s, 120, 400) >= 0.5 && cov(&s, 480, 440) >= 0.5);
}
