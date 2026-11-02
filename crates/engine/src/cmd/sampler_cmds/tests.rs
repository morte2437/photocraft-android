use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Layer, Size};
use photocraft_geom::Rect;

/// A 100×80 white document.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 80, "background": "white"})).unwrap();
    s
}

fn list(s: &mut Session) -> Value {
    s.execute("view.colorSamplers.list", json!({})).unwrap()
}

fn points(s: &mut Session) -> Vec<Value> {
    list(s)["samplers"].as_array().cloned().unwrap_or_default()
}

#[test]
fn add_reads_the_composite_pixel_and_lists_it() {
    let mut s = session();
    let r = s.execute("view.colorSamplers.add", json!({"x": 5, "y": 6})).unwrap();
    assert_eq!(r["index"], 0);
    assert_eq!(r["count"], 1);
    assert_eq!(r["point"]["number"], 1);
    assert_eq!(r["point"]["position"], json!([5.0, 6.0]));
    assert_eq!(r["point"]["rgb"], json!([255, 255, 255]));
    assert_eq!(r["point"]["cmyk"], json!([0, 0, 0, 0]));
    let l = list(&mut s);
    assert_eq!(l["max"], 10);
    assert_eq!(l["samplers"].as_array().unwrap().len(), 1);
    assert_eq!(l["samplers"][0]["position"], json!([5.0, 6.0]));
}

#[test]
fn fractional_clicks_floor_to_the_pixel() {
    let mut s = session();
    let r = s.execute("view.colorSamplers.add", json!({"x": 5.9, "y": 6.1})).unwrap();
    assert_eq!(r["point"]["position"], json!([5.0, 6.0]));
}

#[test]
fn add_reads_an_actual_painted_pixel() {
    // A black pixel at (3, 3) of an otherwise white document.
    let mut s = session();
    s.edit("paint", |doc, _| {
        let surf = doc.layers[0].surface_mut().unwrap();
        let black = photocraft_raster::from_rgba(&surf.format(), [0.0, 0.0, 0.0, 1.0]);
        surf.fill_rect(Rect::new(3, 3, 4, 4), &black);
        Ok(())
    })
    .unwrap();
    let r = s.execute("view.colorSamplers.add", json!({"x": 3, "y": 3})).unwrap();
    assert_eq!(r["point"]["rgb"], json!([0, 0, 0]));
    assert_eq!(r["point"]["cmyk"], json!(device_cmyk(0.0, 0.0, 0.0)), "the profile's black, not the naive 0/0/0/100");
    // Painting over the point changes its readout (the list is computed live).
    s.edit("paint", |doc, _| {
        let surf = doc.layers[0].surface_mut().unwrap();
        let red = photocraft_raster::from_rgba(&surf.format(), [1.0, 0.0, 0.0, 1.0]);
        surf.fill_rect(Rect::new(3, 3, 4, 4), &red);
        Ok(())
    })
    .unwrap();
    assert_eq!(points(&mut s)[0]["rgb"], json!([255, 0, 0]));
}

#[test]
fn ten_points_then_the_limit() {
    let mut s = session();
    for i in 0..10 {
        s.execute("view.colorSamplers.add", json!({"x": i, "y": 0})).unwrap();
    }
    assert_eq!(points(&mut s).len(), 10);
    let err = s.execute("view.colorSamplers.add", json!({"x": 20, "y": 0})).unwrap_err().to_string();
    assert!(err.contains("limit"), "{err}");
    assert_eq!(points(&mut s).len(), 10);
}

#[test]
fn move_delete_and_clear() {
    let mut s = session();
    s.execute("view.colorSamplers.add", json!({"x": 5, "y": 5})).unwrap();
    s.execute("view.colorSamplers.add", json!({"x": 50, "y": 50})).unwrap();
    let r = s.execute("view.colorSamplers.move", json!({"index": 0, "x": 10, "y": 20})).unwrap();
    assert_eq!(r["point"]["position"], json!([10.0, 20.0]));
    assert_eq!(points(&mut s)[0]["position"], json!([10.0, 20.0]));
    s.execute("view.colorSamplers.delete", json!({"index": 1})).unwrap();
    assert_eq!(points(&mut s).len(), 1);
    // The remaining point is the moved #1.
    assert_eq!(points(&mut s)[0]["position"], json!([10.0, 20.0]));
    let r = s.execute("view.colorSamplers.clear", json!({})).unwrap();
    assert_eq!(r["deleted"], 1);
    assert!(points(&mut s).is_empty());
}

#[test]
fn placing_and_moving_add_no_history_and_change_no_pixels() {
    let mut s = session();
    let before = s.active().unwrap().doc.layers[0].surface().unwrap().pixel(5, 5);
    let past = s.active().unwrap().history.past_len();
    s.execute("view.colorSamplers.add", json!({"x": 5, "y": 5})).unwrap();
    s.execute("view.colorSamplers.move", json!({"index": 0, "x": 9, "y": 9})).unwrap();
    s.execute("view.colorSamplers.delete", json!({"index": 0})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), past, "no history steps");
    assert_eq!(s.active().unwrap().doc.layers[0].surface().unwrap().pixel(5, 5), before, "no pixels changed");
    assert!(!s.active().unwrap().is_dirty(), "a clean document stays clean");
}

#[test]
fn points_belong_to_their_document() {
    let mut s = session();
    s.execute("view.colorSamplers.add", json!({"x": 5, "y": 5})).unwrap();
    s.execute("file.new", json!({"width": 40, "height": 40, "background": "white"})).unwrap();
    assert!(points(&mut s).is_empty(), "the new document has no points");
    s.execute("document.activate", json!({"document": 0})).unwrap();
    assert_eq!(points(&mut s).len(), 1, "the first document kept its point");
    // Closing the second document leaves the first's points alone.
    s.execute("file.close", json!({"document": 1})).unwrap();
    assert_eq!(points(&mut s).len(), 1);
    // Closing the first discards its points with it.
    s.execute("file.close", json!({"document": 0})).unwrap();
    assert!(s.execute("view.colorSamplers.list", json!({})).is_err(), "no document open");
}

#[test]
fn a_crop_drops_points_outside_the_new_canvas() {
    let mut s = session();
    s.execute("view.colorSamplers.add", json!({"x": 5, "y": 5})).unwrap();
    s.execute("view.colorSamplers.add", json!({"x": 50, "y": 50})).unwrap();
    s.execute("image.crop", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let p = points(&mut s);
    assert_eq!(p.len(), 1, "the outside point was removed");
    assert_eq!(p[0]["position"], json!([5.0, 5.0]));
    assert_eq!(s.take_samplers_removed(), 1, "one removed point reported");
    assert_eq!(s.take_samplers_removed(), 0, "reported once");
}

#[test]
fn a_crop_moves_points_with_their_pixels() {
    // A crop from a non-zero origin moves the canvas: the point follows its pixel.
    let mut s = session();
    s.execute("view.colorSamplers.add", json!({"x": 50, "y": 40})).unwrap();
    s.execute("image.crop", json!({"x": 40, "y": 30, "width": 20, "height": 20})).unwrap();
    let p = points(&mut s);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0]["position"], json!([10.0, 10.0]), "moved by (-40, -30)");
    assert_eq!(s.take_samplers_removed(), 0, "nothing removed");
}

#[test]
fn a_canvas_size_shift_moves_points() {
    // Canvas Size anchored bottom-right shifts the canvas origin by (+20, +20).
    let mut s = session();
    s.execute("view.colorSamplers.add", json!({"x": 30, "y": 30})).unwrap();
    s.execute("image.canvasSize", json!({"width": 120, "height": 100, "anchor": "bottomRight"})).unwrap();
    let p = points(&mut s);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0]["position"], json!([50.0, 50.0]), "moved by the canvas shift");
}

#[test]
fn a_right_angle_rotation_moves_points() {
    // 90° clockwise about the centre: pixel (x, y) of the 100×80 canvas goes to (79 - y, x).
    let mut s = session();
    s.execute("view.colorSamplers.add", json!({"x": 50, "y": 40})).unwrap();
    s.execute("image.rotation.arbitrary", json!({"angle": 90, "direction": "cw"})).unwrap();
    let p = points(&mut s);
    assert_eq!(p.len(), 1);
    let pos = p[0]["position"].as_array().unwrap();
    let (x, y) = (pos[0].as_f64().unwrap(), pos[1].as_f64().unwrap());
    assert!((x - 39.0).abs() < 1e-6 && (y - 50.0).abs() < 1e-6, "{pos:?}");
}

#[test]
fn flips_and_right_angle_turns_keep_points_on_their_pixel() {
    use crate::canvas_geom::Turn;
    for (id, turn) in [
        ("image.imageRotation.flipCanvasHorizontal", Turn::FlipHorizontal),
        ("image.imageRotation.flipCanvasVertical", Turn::FlipVertical),
        ("image.imageRotation.180", Turn::Rotate180),
        ("image.imageRotation.90cw", Turn::Cw90),
        ("image.imageRotation.90ccw", Turn::Ccw90),
    ] {
        // A black pixel at (3, 3) of a white 100×80 document, with a point on it.
        let mut s = session();
        s.edit("paint", |doc, _| {
            let surf = doc.layers[0].surface_mut().unwrap();
            let black = photocraft_raster::from_rgba(&surf.format(), [0.0, 0.0, 0.0, 1.0]);
            surf.fill_rect(Rect::new(3, 3, 4, 4), &black);
            Ok(())
        })
        .unwrap();
        s.execute("view.colorSamplers.add", json!({"x": 3, "y": 3})).unwrap();
        s.execute(id, json!({})).unwrap();
        let (x, y) = turn.pixel_map(100, 80)(3, 3);
        let p = points(&mut s);
        assert_eq!(p.len(), 1, "{id}");
        assert_eq!(p[0]["position"], json!([f64::from(x), f64::from(y)]), "{id}");
        assert_eq!(p[0]["rgb"], json!([0, 0, 0]), "{id}");
    }
}

#[test]
fn an_arbitrary_rotation_keeps_points_on_their_pixel() {
    // The pixel at (3, 3) of the 100×80 document, and where each turn moves it.
    for (angle, direction, at) in [(90, "cw", [76.0, 3.0]), (90, "ccw", [3.0, 96.0]), (180, "cw", [96.0, 76.0])] {
        let mut s = session();
        s.edit("paint", |doc, _| {
            let surf = doc.layers[0].surface_mut().unwrap();
            let black = photocraft_raster::from_rgba(&surf.format(), [0.0, 0.0, 0.0, 1.0]);
            surf.fill_rect(Rect::new(3, 3, 4, 4), &black);
            Ok(())
        })
        .unwrap();
        s.execute("view.colorSamplers.add", json!({"x": 3, "y": 3})).unwrap();
        s.execute("view.colorSamplers.add", json!({"x": 0, "y": 0})).unwrap();
        s.execute("image.rotation.arbitrary", json!({"angle": angle, "direction": direction})).unwrap();
        assert_eq!(s.take_samplers_removed(), 0, "{angle} {direction}: the corner point stays on the canvas");
        let p = points(&mut s);
        assert_eq!(p.len(), 2, "{angle} {direction}: the corner point stays on the canvas");
        assert_eq!(p[0]["position"], json!(at), "{angle} {direction}");
        assert_eq!(p[0]["rgb"], json!([0, 0, 0]), "{angle} {direction}");
    }
}

#[test]
fn a_rotated_crop_moves_points() {
    // The crop frame is centred on the point's pixel; turned 90° that pixel lands at (10, 9).
    let mut s = session();
    s.edit("paint", |doc, _| {
        let surf = doc.layers[0].surface_mut().unwrap();
        let black = photocraft_raster::from_rgba(&surf.format(), [0.0, 0.0, 0.0, 1.0]);
        surf.fill_rect(Rect::new(50, 40, 51, 41), &black);
        Ok(())
    })
    .unwrap();
    s.execute("view.colorSamplers.add", json!({"x": 50, "y": 40})).unwrap();
    s.execute("image.crop", json!({"x": 40, "y": 30, "width": 20, "height": 20, "angle": 90})).unwrap();
    let p = points(&mut s);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0]["position"], json!([10.0, 9.0]));
    assert_eq!(p[0]["rgb"], json!([0, 0, 0]));
}

#[test]
fn a_crop_to_a_target_size_scales_points() {
    // The crop resamples the 20 x 20 area to 40 x 40: the point (10, 10 after the shift; 10, 9
    // turned 90°, as in `a_rotated_crop_moves_points`) doubles.
    for (angle, at) in [(None, [20.0, 20.0]), (Some(90), [20.0, 18.0])] {
        let mut s = session();
        s.execute("view.colorSamplers.add", json!({"x": 50, "y": 40})).unwrap();
        let mut p = json!({"x": 40, "y": 30, "width": 20, "height": 20, "targetWidth": 40, "targetHeight": 40});
        if let Some(a) = angle {
            p["angle"] = json!(a);
        }
        s.execute("image.crop", p).unwrap();
        let p = points(&mut s);
        assert_eq!(p.len(), 1, "angle {angle:?}");
        assert_eq!(p[0]["position"], json!(at), "angle {angle:?}");
        assert_eq!(s.take_samplers_removed(), 0, "angle {angle:?}");
    }
}

#[test]
fn bad_params_and_no_document_are_errors_not_panics() {
    let mut s = Session::new();
    for id in ["view.colorSamplers.add", "view.colorSamplers.move", "view.colorSamplers.delete", "view.colorSamplers.clear", "view.colorSamplers.list"] {
        assert!(s.execute(id, json!({})).is_err(), "{id} without a document");
    }
    let mut s = session();
    for (id, p) in [
        ("view.colorSamplers.add", json!({})),
        ("view.colorSamplers.add", json!({"x": "a", "y": 1})),
        ("view.colorSamplers.add", json!({"x": 1e300, "y": 1})),
        ("view.colorSamplers.add", json!({"x": 5, "y": f64::NAN})),
        ("view.colorSamplers.add", json!({"x": -1, "y": 5})),
        ("view.colorSamplers.add", json!({"x": 5, "y": 1000})),
        ("view.colorSamplers.move", json!({"index": 0, "x": 1, "y": 1})),
        ("view.colorSamplers.delete", json!({"index": 0})),
        ("view.colorSamplers.delete", json!({})),
        ("view.colorSamplers.move", json!({"index": 4_294_967_296u64, "x": 1, "y": 1})),
    ] {
        assert!(matches!(s.execute(id, p.clone()), Err(EngineError::BadParams { .. }) | Err(EngineError::Other(_))), "{id} {p}");
    }
    // An off-canvas point is refused and places nothing.
    assert!(points(&mut s).is_empty());
}

/// The Info panel and the Color Picker read CMYK through one conversion (#8101): the
/// colour-managed `rgb_to_cmyk`, not a naive formula that disagrees with it by tens of percent.
#[test]
fn device_cmyk_agrees_with_the_color_picker_conversion() {
    for rgb in [[1.0, 1.0, 1.0], [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 0.5, 0.5], [0.2, 0.6, 0.9], [1.0, 0.93, 0.0]] {
        let picker = photocraft_color::convert::rgb_to_cmyk(rgb).map(|v| (v * 100.0).round() as i32);
        assert_eq!(device_cmyk(rgb[0], rgb[1], rgb[2]), picker, "{rgb:?}");
    }
    assert_eq!(device_cmyk(1.0, 1.0, 1.0), [0, 0, 0, 0]);
    assert_ne!(device_cmyk(1.0, 0.0, 0.0), [0, 100, 100, 0], "pure red is not the naive 0/100/100/0");
}

/// 32-bit samples can lie outside 0..1 (or be NaN); the CMYK readout clamps them like the RGB
/// readout does instead of reporting K:-50% or C:120% (#4191).
#[test]
fn device_cmyk_clamps_out_of_range_and_nan_samples() {
    assert_eq!(device_cmyk(1.5, 1.5, 1.5), [0, 0, 0, 0]);
    assert_eq!(device_cmyk(2.0, 0.0, 0.0), device_cmyk(1.0, 0.0, 0.0));
    let in_range = |c: [i32; 4]| c.iter().all(|v| (0..=100).contains(v));
    for (r, g, b) in [(-0.2, 0.5, 0.5), (-1.0, -1.0, -1.0), (f32::NAN, 0.5, 0.5), (f32::NAN, f32::NAN, f32::NAN), (f32::INFINITY, f32::NEG_INFINITY, 0.5)] {
        let c = device_cmyk(r, g, b);
        assert!(in_range(c), "{r} {g} {b} -> {c:?}");
    }
    assert_eq!(device_cmyk(f32::NAN, f32::NAN, f32::NAN), device_cmyk(0.0, 0.0, 0.0));
}

#[test]
fn hdr_pixel_on_a_32_bit_document_reads_in_range() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": 32, "background": "white"})).unwrap();
    s.edit("paint", |doc, _| {
        let surf = doc.layers[0].surface_mut().unwrap();
        let hdr = photocraft_raster::from_rgba(&surf.format(), [2.0, 2.0, 2.0, 1.0]);
        surf.fill_rect(Rect::new(3, 3, 4, 4), &hdr);
        Ok(())
    })
    .unwrap();
    let r = s.execute("view.colorSamplers.add", json!({"x": 3, "y": 3})).unwrap();
    assert_eq!(r["point"]["rgb"], json!([255, 255, 255]));
    assert_eq!(r["point"]["cmyk"], json!([0, 0, 0, 0]));
}

#[test]
fn points_work_at_every_depth_and_mode() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("grayscale", 8), ("cmyk", 8), ("lab", 8)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 20, "height": 20, "mode": mode, "depth": depth, "background": "white"})).unwrap();
        let r = s.execute("view.colorSamplers.add", json!({"x": 10, "y": 10})).unwrap();
        let rgb = r["point"]["rgb"].as_array().unwrap();
        assert_eq!(rgb.len(), 3);
        assert!(rgb.iter().all(|v| v.as_i64().is_some_and(|n| (0..=255).contains(&n))), "{mode} {depth}: {rgb:?}");
    }
}

/// The mode and depth only affect the composite the readout samples, not the point model.
#[test]
fn document_helpers_are_not_disturbed() {
    let mut d = Document::new("t", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    d.layers.push(Layer::raster("Background", d.pixel_format()));
    assert_eq!(pixel(&d, 0.0, 0.0), Some([0.0, 0.0]));
    assert_eq!(pixel(&d, 3.9, 3.9), Some([3.0, 3.0]));
    assert_eq!(pixel(&d, 4.0, 0.0), None);
}
