//! Color Sampler points sit on the centre of the clicked pixel (#11725), as in Photoshop: a click
//! anywhere in pixel 199,149 stores 199.5,149.5, the readout still reads that pixel, and document
//! operations move the centre with its pixel.

use super::*;
use photocraft_geom::Rect;

/// An 800 × 600 white document, as in the issue's measurement.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    s
}

fn positions(s: &mut Session) -> Vec<[f64; 2]> {
    let l = s.execute("view.colorSamplers.list", json!({})).unwrap();
    l["samplers"].as_array().unwrap().iter().map(|p| [p["position"][0].as_f64().unwrap(), p["position"][1].as_f64().unwrap()]).collect()
}

fn assert_near(got: &[[f64; 2]], want: &[[f64; 2]], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g[0] - w[0]).abs() < 1e-9 && (g[1] - w[1]).abs() < 1e-9, "{what}: got {got:?}, want {want:?}");
    }
}

#[test]
fn a_point_inside_a_pixel_is_stored_and_listed_at_its_centre() {
    let mut s = session();
    let r = s.execute("view.colorSamplers.add", json!({"x": 199.3, "y": 149.6})).unwrap();
    assert_eq!(r["point"]["position"], json!([199.5, 149.5]));
    for (x, y) in [(199.0, 149.0), (199.99, 149.01)] {
        s.execute("view.colorSamplers.add", json!({"x": x, "y": y})).unwrap();
    }
    assert_near(&positions(&mut s), &[[199.5, 149.5]; 3], "anywhere in pixel 199,149");
    // The last pixel of the canvas.
    s.execute("view.colorSamplers.clear", json!({})).unwrap();
    s.execute("view.colorSamplers.add", json!({"x": 799.7, "y": 598.2})).unwrap();
    assert_near(&positions(&mut s), &[[799.5, 598.5]], "the last pixel");
}

#[test]
fn moving_a_point_snaps_it_to_the_pixel_centre() {
    let mut s = session();
    s.execute("view.colorSamplers.add", json!({"x": 10, "y": 10})).unwrap();
    let r = s.execute("view.colorSamplers.move", json!({"index": 0, "x": 352.9, "y": 150.2})).unwrap();
    assert_near(&positions(&mut s), &[[352.5, 150.5]], &format!("moved: {r}"));
    // Off the canvas is still refused and the point stays put.
    assert!(s.execute("view.colorSamplers.move", json!({"index": 0, "x": 800.0, "y": 10.0})).is_err());
    assert_near(&positions(&mut s), &[[352.5, 150.5]], "after a refused move");
}

#[test]
fn the_readout_reads_the_pixel_the_centre_is_on() {
    let mut s = session();
    // Pixel 199,149 red, its lower-right neighbour 200,150 blue.
    s.edit("paint", |doc, _| {
        let surf = doc.layers[0].surface_mut().unwrap();
        let red = photocraft_raster::from_rgba(&surf.format(), [1.0, 0.0, 0.0, 1.0]);
        let blue = photocraft_raster::from_rgba(&surf.format(), [0.0, 0.0, 1.0, 1.0]);
        surf.fill_rect(Rect::new(199, 149, 200, 150), &red);
        surf.fill_rect(Rect::new(200, 150, 201, 151), &blue);
        Ok(())
    })
    .unwrap();
    let r = s.execute("view.colorSamplers.add", json!({"x": 199.3, "y": 149.6})).unwrap();
    assert_eq!(r["point"]["rgb"], json!([255, 0, 0]), "199.5,149.5 reads pixel 199,149");
    let l = s.execute("view.colorSamplers.list", json!({})).unwrap();
    assert_eq!(l["samplers"][0]["rgb"], json!([255, 0, 0]));
}

/// The issue's two measured points (#1 on pixel 239,169 and #2 on pixel 352,150) after `id`.
fn after(id: &str, p: Value) -> Vec<[f64; 2]> {
    let mut s = session();
    s.execute("view.colorSamplers.add", json!({"x": 239.4, "y": 169.8})).unwrap();
    s.execute("view.colorSamplers.add", json!({"x": 352.1, "y": 150.0})).unwrap();
    assert_near(&positions(&mut s), &[[239.5, 169.5], [352.5, 150.5]], "placed");
    s.execute(id, p).unwrap();
    positions(&mut s)
}

#[test]
fn document_operations_move_the_centre_as_measured() {
    let cases = [
        ("image.canvasSize", json!({"width": 1000, "height": 800, "anchor": "center"}), [[339.5, 269.5], [452.5, 250.5]]),
        ("image.imageRotation.90cw", json!({}), [[430.5, 239.5], [449.5, 352.5]]),
        ("image.rotation.arbitrary", json!({"angle": 90, "direction": "cw"}), [[430.5, 239.5], [449.5, 352.5]]),
        ("image.imageRotation.flipCanvasHorizontal", json!({}), [[560.5, 169.5], [447.5, 150.5]]),
        // Scaled with the pixels, not snapped back to a pixel centre.
        ("image.imageSize", json!({"width": 400, "height": 300}), [[119.75, 84.75], [176.25, 75.25]]),
        ("image.crop", json!({"x": 100, "y": 100, "width": 400, "height": 300}), [[139.5, 69.5], [252.5, 50.5]]),
    ];
    for (id, p, want) in cases {
        assert_near(&after(id, p), &want, id);
    }
}

#[test]
fn points_outside_the_new_canvas_are_still_dropped() {
    let mut s = session();
    s.execute("view.colorSamplers.add", json!({"x": 239, "y": 169})).unwrap();
    s.execute("view.colorSamplers.add", json!({"x": 650, "y": 500})).unwrap();
    // The last pixel inside the crop stays; the one just outside goes.
    s.execute("view.colorSamplers.add", json!({"x": 499, "y": 399})).unwrap();
    s.execute("view.colorSamplers.add", json!({"x": 500, "y": 399})).unwrap();
    s.execute("image.crop", json!({"x": 100, "y": 100, "width": 400, "height": 300})).unwrap();
    assert_near(&positions(&mut s), &[[139.5, 69.5], [399.5, 299.5]], "after the crop");
    assert_eq!(s.take_samplers_removed(), 2);
}
