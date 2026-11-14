//! #11751: the Eyedropper's N by N Average box depends on the view zoom, as in Photoshop. Above
//! 100 % it is the centred N × N square of document pixels; at 100 % and below it covers
//! (N+1) × (N+1) screen pixels (at 100 % the document pixels x−N/2 … x+N/2+1). Without a zoom
//! `document.sampleColor` keeps the centred square.

use serde_json::{Value, json};

use crate::Session;

/// The issue's test document: 800 × 600 RGB/8 white; on a pixel layer a red (220,30,30) block at
/// 100,100–300,250, 2 px black / 2 px white stripes at 350,100–450,200 (black at x 350–351,
/// 354–355, …) and a grey ramp of ten 40 px blocks from x 100, values 100, 105, …, 145, at
/// y 350–450.
fn test_document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Pixels"})).unwrap();
    let fill = |s: &mut Session, x: i32, y: i32, w: i32, h: i32, grey_or_hex: String| {
        s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": grey_or_hex})).unwrap();
    };
    fill(&mut s, 100, 100, 200, 150, "#dc1e1e".into());
    fill(&mut s, 350, 100, 100, 100, "#ffffff".into());
    for x in (350..450).step_by(4) {
        fill(&mut s, x, 100, 2, 100, "#000000".into());
    }
    for i in 0..10 {
        let v = 100 + 5 * i;
        fill(&mut s, 100 + 40 * i, 350, 40, 100, format!("#{v:02x}{v:02x}{v:02x}"));
    }
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// A black 400 × 400 document with one white pixel at 200,200 (the issue's probe).
fn probe() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 400, "background": "white"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
    s.execute("select.rect", json!({"x": 200, "y": 200, "width": 1, "height": 1})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ffffff"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// The 8-bit RGB read at document pixel (x, y).
fn read(s: &mut Session, x: i32, y: i32, size: u32, zoom: Option<f64>) -> [i64; 3] {
    let mut p = json!({"x": f64::from(x) + 0.5, "y": f64::from(y) + 0.5, "size": size});
    if let Some(z) = zoom {
        p["zoom"] = json!(z);
    }
    let v = s.execute("document.sampleColor", p).unwrap();
    let c = |i: usize| (v[i].as_f64().unwrap() * 255.0).round() as i64;
    [c(0), c(1), c(2)]
}

fn grey(v: i64) -> [i64; 3] {
    [v, v, v]
}

#[test]
fn at_100_percent_the_test_document_reads_the_measured_table() {
    // Sample Size: stripes x 351, stripes x 352 (y 150), grey x 139 (y 400), red left edge x 100 (y 150).
    // The reference application's table, except grey x 139 up to 51 by 51: that box is exactly
    // half 100 and half 105, a mean of 102.5, which the reference reads 102 and PhotoCraft's shared
    // `average` rounds to 103 (half up, as every Sample Size average does today). The box is right
    // (101 by 101, whose mean is no tie, reads 122 in both).
    const TIE: i64 = 103;
    let measured: [(u32, [[i64; 3]; 4]); 6] = [
        (3, [grey(128), grey(128), grey(TIE), [229, 86, 86]]),
        (5, [grey(128), grey(85), grey(TIE), [232, 105, 105]]),
        (11, [grey(170), grey(149), grey(TIE), [235, 124, 124]]),
        (31, [grey(175), grey(175), grey(TIE), [236, 135, 135]]),
        (51, [grey(186), grey(181), grey(TIE), [237, 138, 138]]),
        (101, [grey(189), grey(186), grey(122), [237, 140, 140]]),
    ];
    let mut s = test_document();
    for (size, want) in measured {
        let got = [
            read(&mut s, 351, 150, size, Some(1.0)),
            read(&mut s, 352, 150, size, Some(1.0)),
            read(&mut s, 139, 400, size, Some(1.0)),
            read(&mut s, 100, 150, size, Some(1.0)),
        ];
        assert_eq!(got, want, "{size} by {size} at 100 %");
    }
}

#[test]
fn without_a_zoom_the_box_stays_the_centred_square() {
    // Today's values from the issue: 3 by 3 at stripes x 351 reads 85, red edge 232,105,105.
    let mut s = test_document();
    assert_eq!(read(&mut s, 351, 150, 3, None), grey(85));
    assert_eq!(read(&mut s, 352, 150, 3, None), grey(170));
    assert_eq!(read(&mut s, 100, 150, 3, None), [232, 105, 105]);
    assert_eq!(read(&mut s, 351, 150, 11, None), grey(162));
}

#[test]
fn the_probe_at_100_percent_reads_16_for_3_by_3_and_7_for_5_by_5() {
    let mut s = probe();
    // 3 by 3 is 4 × 4 from x−1 to x+2: clicks at 198–201 (both axes) catch the white pixel.
    for x in 197..=202 {
        let want = if (198..=201).contains(&x) { 16 } else { 0 };
        assert_eq!(read(&mut s, x, 200, 3, Some(1.0)), grey(want), "3 by 3, x {x}");
        assert_eq!(read(&mut s, 200, x, 3, Some(1.0)), grey(want), "3 by 3, y {x}");
    }
    // 5 by 5 is 6 × 6 from x−2 to x+3.
    for x in 196..=203 {
        let want = if (197..=202).contains(&x) { 7 } else { 0 };
        assert_eq!(read(&mut s, x, 200, 5, Some(1.0)), grey(want), "5 by 5, x {x}");
    }
}

#[test]
fn at_50_percent_3_by_3_covers_8_by_8_document_pixels() {
    let mut s = probe();
    assert_eq!(read(&mut s, 200, 200, 3, Some(0.5)), grey(4), "255 / 64");
}

#[test]
fn above_100_percent_it_is_the_centred_square() {
    let mut s = probe();
    for zoom in [2.0, 3.125, 128.0] {
        for x in 198..=202 {
            let want = if (199..=201).contains(&x) { 28 } else { 0 };
            assert_eq!(read(&mut s, x, 200, 3, Some(zoom)), grey(want), "{zoom}: x {x}");
        }
    }
}

#[test]
fn point_sample_is_one_pixel_at_every_zoom() {
    let mut s = probe();
    for zoom in [None, Some(0.05), Some(0.5), Some(1.0), Some(4.0)] {
        assert_eq!(read(&mut s, 200, 200, 1, zoom), grey(255), "{zoom:?}");
        assert_eq!(read(&mut s, 201, 200, 1, zoom), grey(0), "{zoom:?}");
    }
}

#[test]
fn a_far_zoomed_out_box_is_capped_and_clipped_not_a_panic() {
    let mut s = probe();
    // At 1 % a 101 by 101 box would be 10 200 px wide: capped, and clipped to the canvas.
    let v = read(&mut s, 0, 0, 101, Some(0.01));
    assert!(v.iter().all(|c| (0..=1).contains(c)), "one white pixel in a canvas-wide average: {v:?}");
    let rect = super::sample_cmds::sample_rect_at(200.5, 200.5, 101, Some(0.01), photocraft_geom::Rect::new(0, 0, 100_000, 100_000)).unwrap();
    assert!(rect.width() <= super::sample_cmds::MAX_ZOOM_BOX && rect.height() <= super::sample_cmds::MAX_ZOOM_BOX, "{rect:?}");
}

#[test]
fn a_bad_zoom_is_an_error_not_a_panic() {
    let mut s = probe();
    for bad in [json!(0), json!(-1.0), json!("fast"), json!([1]), json!(-1e300)] {
        let r = s.execute("document.sampleColor", json!({"x": 10, "y": 10, "size": 3, "zoom": bad}));
        assert!(r.is_err(), "zoom {bad}: {r:?}");
    }
    // A huge zoom is above 100 %: the centred square.
    let v: Value = s.execute("document.sampleColor", json!({"x": 200.5, "y": 200.5, "size": 3, "zoom": 1e300})).unwrap();
    assert!((v[0].as_f64().unwrap() * 255.0 - 28.3).abs() < 1.0, "{v}");
}
