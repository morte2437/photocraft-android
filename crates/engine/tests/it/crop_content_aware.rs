//! Crop tool › Fill: Content-Aware Fill (#3823). A rotated crop, or one that extends past the
//! canvas, fills the area the image doesn't cover from its surroundings instead of the background
//! colour, in the crop's one undo step; an unrotated crop inside the canvas is unaffected.

use photocraft_engine::Session;
use serde_json::{Value, json};

const MODES: [(&str, u32); 5] = [("rgb", 8), ("rgb", 16), ("rgb", 32), ("grayscale", 16), ("cmyk", 8)];

/// A 64 × 48 document whose Background is a light green, with black as the background colour (what
/// a plain crop puts in the uncovered area).
fn doc(mode: &str, depth: u32, background: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "mode": mode, "depth": depth, "background": background})).unwrap();
    s.execute("tools.setColors", json!({"foreground": "#c0e0a0", "background": "#000000"})).unwrap();
    s.execute("edit.fill", json!({"contents": "foreground"})).unwrap();
    s
}

fn layer(s: &Session) -> &photocraft_raster::Surface {
    let d = &s.active().unwrap().doc;
    d.layers[0].surface().unwrap()
}

fn size(s: &Session) -> (u32, u32) {
    let d = &s.active().unwrap().doc;
    (d.size.width, d.size.height)
}

fn close(a: [f32; 4], b: [f32; 4], tol: f32) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < tol)
}

/// Every pixel of the cropped layer is the image's colour: no black (or transparent) area left.
fn assert_all_image(s: &Session, want: [f32; 4], what: &str) {
    let (w, h) = size(s);
    let surf = layer(s);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let px = surf.rgba(x, y);
            assert!(close(px, want, 0.06), "{what}: ({x}, {y}) is {px:?}, want {want:?}");
        }
    }
}

fn crop(s: &mut Session, mut p: Value, content_aware: bool) -> photocraft_engine::Result<Value> {
    p["contentAware"] = json!(content_aware);
    s.execute("image.crop", p)
}

#[test]
fn rotated_crop_fills_the_corners_in_every_mode() {
    for (mode, depth) in MODES {
        let frame = json!({"x": 0, "y": 0, "width": 64, "height": 48, "angle": 10});
        let mut plain = doc(mode, depth, "white");
        let want = layer(&plain).rgba(32, 24);
        crop(&mut plain, frame.clone(), false).unwrap();
        assert!(close(layer(&plain).rgba(0, 0), [0.0, 0.0, 0.0, 1.0], 0.06), "{mode} {depth}: a plain rotated crop leaves black corners");

        let mut s = doc(mode, depth, "white");
        let past = s.active().unwrap().history.past_len();
        crop(&mut s, frame, true).unwrap();
        assert_eq!(size(&s), (64, 48), "{mode} {depth}");
        assert_eq!(s.active().unwrap().history.past_len(), past + 1, "{mode} {depth}: one undo step");
        assert_all_image(&s, want, &format!("{mode} {depth} rotated"));
    }
}

#[test]
fn extending_crop_fills_the_added_canvas_in_every_mode() {
    for (mode, depth) in MODES {
        let frame = json!({"x": -8, "y": -6, "width": 80, "height": 60});
        let mut plain = doc(mode, depth, "white");
        let want = layer(&plain).rgba(32, 24);
        crop(&mut plain, frame.clone(), false).unwrap();
        assert!(close(layer(&plain).rgba(0, 0), [0.0, 0.0, 0.0, 1.0], 0.06), "{mode} {depth}");

        let mut s = doc(mode, depth, "white");
        crop(&mut s, frame, true).unwrap();
        assert_eq!(size(&s), (80, 60), "{mode} {depth}");
        assert_all_image(&s, want, &format!("{mode} {depth} extended"));
    }
}

/// Without a Background the active pixel layer is filled: no transparent strip is left.
#[test]
fn extending_crop_fills_the_active_layer_without_a_background() {
    let mut s = doc("rgb", 16, "transparent");
    let want = layer(&s).rgba(32, 24);
    crop(&mut s, json!({"x": 0, "y": -10, "width": 64, "height": 58}), true).unwrap();
    assert_all_image(&s, want, "transparent document");
}

/// Undo takes the crop and the fill back in one step.
#[test]
fn undo_restores_crop_and_fill_in_one_step() {
    let mut s = doc("rgb", 8, "white");
    s.execute("tools.setColors", json!({"foreground": "#ff0000", "background": "#000000"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
    s.execute("edit.fill", json!({"contents": "foreground"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let before = s.active().unwrap().doc.clone();
    crop(&mut s, json!({"x": -4, "y": -4, "width": 40, "height": 40, "angle": -6}), true).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(size(&s), (64, 48));
    let after = layer(&s);
    let orig = before.layers[0].surface().unwrap();
    for y in 0..48 {
        for x in 0..64 {
            assert_eq!(after.rgba(x, y), orig.rgba(x, y), "({x}, {y})");
        }
    }
}

/// `contentAware: false` (or absent), and a crop that uncovers nothing, are the plain crop.
#[test]
fn nothing_uncovered_or_off_is_the_plain_crop() {
    for (frame, ca) in [
        (json!({"x": 5, "y": 4, "width": 30, "height": 20}), true),
        (json!({"x": -5, "y": -4, "width": 80, "height": 60}), false),
        (json!({"x": 20, "y": 15, "width": 20, "height": 14, "angle": 4}), true),
    ] {
        let mut a = doc("rgb", 16, "white");
        let mut b = doc("rgb", 16, "white");
        a.execute("image.crop", frame.clone()).unwrap();
        crop(&mut b, frame.clone(), ca).unwrap();
        assert_eq!(size(&a), size(&b), "{frame}");
        let (w, h) = size(&a);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                assert_eq!(layer(&a).rgba(x, y), layer(&b).rgba(x, y), "{frame} ({x}, {y})");
            }
        }
    }
}

/// Bad params and an unfillable crop fail without changing the document.
#[test]
fn bad_params_and_unfillable_crops_fail_cleanly() {
    let frame = json!({"x": -4, "y": 0, "width": 70, "height": 48});
    for v in [json!("yes"), json!(1), json!([true]), json!({"on": true})] {
        let mut s = doc("rgb", 8, "white");
        let past = s.active().unwrap().history.past_len();
        let mut p = frame.clone();
        p["contentAware"] = v.clone();
        assert!(s.execute("image.crop", p).is_err(), "{v}");
        assert_eq!(size(&s), (64, 48), "{v}");
        assert_eq!(s.active().unwrap().history.past_len(), past, "{v}");
    }
    // Entirely off the canvas: nothing of the image to sample from (both paths).
    for p in [json!({"x": 500, "y": 500, "width": 20, "height": 20}), json!({"x": 500, "y": 500, "width": 20, "height": 20, "angle": 30})] {
        let mut s = doc("rgb", 8, "white");
        let past = s.active().unwrap().history.past_len();
        let err = crop(&mut s, p.clone(), true).unwrap_err();
        assert!(err.to_string().contains("sample"), "{p}: {err}");
        assert_eq!(size(&s), (64, 48), "{p}");
        assert_eq!(s.active().unwrap().history.past_len(), past, "{p}");
    }
}
