//! Color Sampler Sample Size (#11732): each readout is the N × N average of document pixels
//! centred on the point, as measured in the reference application, for the seven sizes the
//! Eyedropper shares with the Color Sampler.

use super::*;
use photocraft_geom::Rect;

/// The issue's test document: 800 × 600 RGB/8 white, with 2 px black and 2 px white vertical
/// stripes at 350,100 to 450,200 (black at x 350–351, 354–355, …).
fn stripes() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    s.edit("stripes", |doc, _| {
        let surf = doc.layers[0].surface_mut().unwrap();
        let black = photocraft_raster::from_rgba(&surf.format(), [0.0, 0.0, 0.0, 1.0]);
        for x in (350..450).step_by(4) {
            surf.fill_rect(Rect::new(x, 100, x + 2, 200), &black);
        }
        Ok(())
    })
    .unwrap();
    s
}

/// The measured readout (R = G = B) of a sampler on pixel 352,150, by Sample Size.
const MEASURED: [(u32, i64); 7] = [(1, 255), (3, 170), (5, 102), (11, 162), (31, 173), (51, 185), (101, 188)];

#[test]
fn the_list_reads_the_measured_average_for_every_size() {
    let mut s = stripes();
    s.execute("view.colorSamplers.add", json!({"x": 352.4, "y": 150.6})).unwrap();
    for (size, grey) in MEASURED {
        let l = s.execute("view.colorSamplers.list", json!({"size": size})).unwrap();
        assert_eq!(l["size"], size);
        assert_eq!(l["samplers"][0]["rgb"], json!([grey, grey, grey]), "{size} by {size}");
    }
}

#[test]
fn point_sample_is_the_default_and_point_is_accepted() {
    let mut s = stripes();
    s.execute("view.colorSamplers.add", json!({"x": 352, "y": 150})).unwrap();
    for p in [json!({}), json!({"size": "point"}), json!({"size": 1})] {
        let l = s.execute("view.colorSamplers.list", p.clone()).unwrap();
        assert_eq!(l["size"], 1, "{p}");
        assert_eq!(l["samplers"][0]["rgb"], json!([255, 255, 255]), "{p}");
    }
}

#[test]
fn add_and_move_report_the_point_with_the_size_they_are_given() {
    let mut s = stripes();
    let r = s.execute("view.colorSamplers.add", json!({"x": 352, "y": 150, "size": 3})).unwrap();
    assert_eq!(r["point"]["rgb"], json!([170, 170, 170]));
    s.execute("view.colorSamplers.add", json!({"x": 10, "y": 10})).unwrap();
    let r = s.execute("view.colorSamplers.move", json!({"index": 1, "x": 352, "y": 150, "size": 5})).unwrap();
    assert_eq!(r["point"]["rgb"], json!([102, 102, 102]));
}

#[test]
fn changing_the_size_rereads_every_point_without_moving_it() {
    let mut s = stripes();
    s.execute("view.colorSamplers.add", json!({"x": 352, "y": 150})).unwrap();
    s.execute("view.colorSamplers.add", json!({"x": 700, "y": 500})).unwrap();
    let before = s.active().unwrap().color_samplers.clone();
    let rev = s.active().unwrap().revision;
    let l = s.execute("view.colorSamplers.list", json!({"size": 11})).unwrap();
    assert_eq!(l["samplers"][0]["rgb"], json!([162, 162, 162]));
    assert_eq!(l["samplers"][1]["rgb"], json!([255, 255, 255]), "a flat white area reads white at any size");
    let st = s.active().unwrap();
    assert_eq!(st.color_samplers, before, "reading never moves a point");
    assert_eq!(st.revision, rev, "reading is not an edit");
}

#[test]
fn a_box_at_the_canvas_edge_averages_only_the_pixels_inside() {
    // The corner pixel's 3 × 3 box keeps its 2 × 2 part on the canvas: one black pixel of four.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 10, "background": "white"})).unwrap();
    s.edit("dot", |doc, _| {
        let surf = doc.layers[0].surface_mut().unwrap();
        let black = photocraft_raster::from_rgba(&surf.format(), [0.0, 0.0, 0.0, 1.0]);
        surf.fill_rect(Rect::new(0, 0, 1, 1), &black);
        Ok(())
    })
    .unwrap();
    s.execute("view.colorSamplers.add", json!({"x": 0, "y": 0})).unwrap();
    let l = s.execute("view.colorSamplers.list", json!({"size": 3})).unwrap();
    let rgb = l["samplers"][0]["rgb"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect::<Vec<_>>();
    assert!(rgb.iter().all(|&v| (190..=192).contains(&v)), "3/4 white: {rgb:?}");
}

#[test]
fn a_bad_size_is_an_error_not_a_panic() {
    let mut s = stripes();
    s.execute("view.colorSamplers.add", json!({"x": 352, "y": 150})).unwrap();
    for bad in [json!(2), json!(0), json!(-3), json!("big"), json!(1.5)] {
        assert!(s.execute("view.colorSamplers.list", json!({"size": bad})).is_err(), "list size {bad}");
        assert!(s.execute("view.colorSamplers.add", json!({"x": 1, "y": 1, "size": bad})).is_err(), "add size {bad}");
        assert!(s.execute("view.colorSamplers.move", json!({"index": 0, "x": 1, "y": 1, "size": bad})).is_err(), "move size {bad}");
    }
    assert_eq!(s.active().unwrap().color_samplers.len(), 1, "a refused add adds nothing");
}

#[test]
fn bitmap_and_multichannel_documents_keep_the_point_readout_at_every_size() {
    // #11732 did not measure these two modes, so they keep today's behaviour: a Point Sample
    // reading, whatever Sample Size the Color Sampler and the Eyedropper share.
    for mode in [photocraft_color::ColorMode::Bitmap, photocraft_color::ColorMode::Multichannel] {
        let mut s = stripes();
        s.edit("mode", |doc, _| {
            doc.mode = mode;
            Ok(())
        })
        .unwrap();
        s.execute("view.colorSamplers.add", json!({"x": 352, "y": 150})).unwrap();
        for (size, _) in MEASURED {
            let l = s.execute("view.colorSamplers.list", json!({"size": size})).unwrap();
            assert_eq!(l["samplers"][0]["rgb"], json!([255, 255, 255]), "{mode:?} at {size} by {size}");
        }
        let r = s.execute("view.colorSamplers.add", json!({"x": 352, "y": 150, "size": 3})).unwrap();
        assert_eq!(r["point"]["rgb"], json!([255, 255, 255]), "{mode:?}: add reports the point readout");
    }
}
