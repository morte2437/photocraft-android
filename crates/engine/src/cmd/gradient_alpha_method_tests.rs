//! #12010: the transparency of a gradient drawn with the Gradient tool follows the interpolation
//! method (Smooth by default) instead of a straight line. Reference: Foreground to Transparent,
//! Smooth, dragged (100, 300) to (700, 300): alpha 255 / 179 / 100 / 35 / 0 at x 50 / 250 / 400 /
//! 550 / 750.

use serde_json::{Value, json};

use crate::Session;

fn drawn(extra: Value) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "transparent"})).unwrap();
    let mut p = json!({"gradient": "Foreground to Transparent", "from": [100, 300], "to": [700, 300], "dither": false});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    s.execute("paint.gradient", p).unwrap();
    s
}

fn alpha_at(s: &Session, x: i32) -> f32 {
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    let photocraft_doc::LayerContent::Raster(surface) = &l.content else { panic!("a pixel layer") };
    surface.pixel(x, 300)[3] * 255.0
}

#[test]
fn smooth_transparency_ramp_matches_the_reference() {
    let s = drawn(json!({"method": "smooth"}));
    for (x, want) in [(50, 255.0), (250, 179.0), (400, 100.0), (550, 35.0), (750, 0.0)] {
        let got = alpha_at(&s, x);
        assert!((got - want).abs() <= 2.0, "x={x}: alpha {got:.1}, want {want}");
    }
}

#[test]
fn without_a_method_the_ramp_stays_straight() {
    let s = drawn(json!({}));
    for (x, want) in [(250, 191.0), (400, 128.0), (550, 64.0)] {
        let got = alpha_at(&s, x);
        assert!((got - want).abs() <= 2.0, "x={x}: alpha {got:.1}, want {want}");
    }
}

#[test]
fn opaque_gradients_paint_the_same_with_a_method() {
    let paint = |extra: Value| {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 8, "background": "white"})).unwrap();
        let mut p = json!({"colors": ["#000000", "#ffffff"], "from": [0, 4], "to": [64, 4], "dither": false});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        s.execute("paint.gradient", p).unwrap();
        (0..64).map(|x| alpha_and_red(&s, x)).collect::<Vec<_>>()
    };
    fn alpha_and_red(s: &Session, x: i32) -> Vec<f32> {
        let d = s.active().unwrap();
        let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
        let photocraft_doc::LayerContent::Raster(surface) = &l.content else { panic!() };
        surface.pixel(x, 4)
    }
    // The colours still follow the method; the alpha stays opaque.
    let smooth = paint(json!({"method": "smooth"}));
    assert!(smooth.iter().all(|p| (p[3] - 1.0).abs() < 1e-3));
    assert_ne!(smooth, paint(json!({})));
}

// Not measured in the Gradient tool: the other methods, and Smooth below 100 % Smoothness, keep
// today's merged stops (each dense stop takes the nearest stop's alpha).
#[test]
fn unmeasured_methods_and_smoothness_keep_todays_ramp() {
    for extra in [json!({"method": "classic"}), json!({"method": "perceptual"}), json!({"method": "linear"}), json!({"method": "smooth", "smoothness": 50})] {
        let s = drawn(extra.clone());
        for (x, want) in [(250, 191.0), (400, 128.0), (550, 64.0)] {
            let got = alpha_at(&s, x);
            assert!((got - want).abs() <= 1.0, "{extra} x={x}: alpha {got:.1}, want {want}");
        }
    }
}

fn drawn_colors(colors: Value) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "transparent"})).unwrap();
    let p = json!({"colors": colors, "transparency": [[0, 100], [1, 0]], "method": "smooth", "from": [100, 300], "to": [700, 300], "dither": false});
    s.execute("paint.gradient", p).unwrap();
    s
}

#[test]
fn one_colour_with_a_transparency_ramp_follows_smooth() {
    let s = drawn_colors(json!(["#ff0000"]));
    for (x, want) in [(50, 255.0), (250, 179.0), (400, 100.0), (550, 35.0), (750, 0.0)] {
        let got = alpha_at(&s, x);
        assert!((got - want).abs() <= 2.0, "x={x}: alpha {got:.1}, want {want}");
    }
}

#[test]
fn legacy_colours_keep_their_own_alpha() {
    let s = drawn_colors(json!(["#ff000080", "#ff000080"]));
    for (x, want) in [(50, 128.0), (400, 64.0), (750, 0.0)] {
        let got = alpha_at(&s, x);
        assert!((got - want).abs() <= 1.0, "x={x}: alpha {got:.1}, want {want}");
    }
}

// paint_gradient scans the stops for every pixel: Smooth's colour and opacity bakes share their
// 97 positions and must not be cut into four per interval (385 stops).
#[test]
fn smooth_transparency_keeps_one_stop_per_sampled_position() {
    use photocraft_color::Color;
    let black = Color::rgb(0.0, 0.0, 0.0);
    let stops = super::paint_cmds::smooth_split_stops((vec![(0.0, black), (1.0, black)], vec![(0.0, 1.0), (1.0, 0.0)]));
    assert_eq!(stops.len(), 97);
    assert!((stops[0].1[3] - 1.0).abs() < 1e-6 && stops[96].1[3].abs() < 1e-6);
}
