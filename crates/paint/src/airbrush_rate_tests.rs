//! #11886: with Airbrush on, a brush held still builds up at about ten dabs a second and a plain
//! click lays two dabs, as in Photoshop. Measured there with the default 13 px soft brush at Flow
//! 10 %, black on white: the dab centre reads 205 after a click and 26 after a 2 s hold, and 229
//! with Airbrush off however long the button is held.

use photocraft_color::PixelFormat;
use photocraft_raster::Surface;

use crate::{BrushSettings, StrokePoint, render_stroke};

fn brush(build_up: bool) -> BrushSettings {
    BrushSettings { size: 13.0, hardness: 0.0, flow: 0.1, pressure_size: false, build_up, color: [0.0, 0.0, 0.0, 1.0], ..BrushSettings::default() }
}

/// The 0–255 value at the dab centre over white after holding the button still for `ms`.
fn centre_after_hold(b: &BrushSettings, ms: f64) -> f32 {
    let a = StrokePoint { time: 0.0, ..StrokePoint::new(30.0, 30.0, 1.0) };
    let z = StrokePoint { time: ms, ..a };
    let mut s = Surface::new(PixelFormat::RGBA32F);
    render_stroke(&mut s, b, &[a, z], None, false, 1.0);
    255.0 * (1.0 - s.rgba(30, 30)[3])
}

#[test]
fn the_default_rate_is_ten_dabs_a_second() {
    assert_eq!(BrushSettings::default().build_up_rate, 10.0);
}

#[test]
fn a_click_lays_two_dabs() {
    let v = centre_after_hold(&brush(true), 0.0);
    assert!((v - 205.0).abs() <= 5.0, "{v}");
}

#[test]
fn a_two_second_hold_builds_up_to_the_reference_value() {
    let v = centre_after_hold(&brush(true), 2000.0);
    assert!((v - 26.0).abs() <= 8.0, "{v}");
}

#[test]
fn airbrush_off_keeps_one_dab_however_long_it_is_held() {
    for ms in [0.0, 500.0, 2000.0] {
        let v = centre_after_hold(&brush(false), ms);
        assert!((v - 229.0).abs() <= 3.0, "{ms} ms: {v}");
    }
}
