//! #12018: the Gradient tool samples each pixel where the reference application does, so an Angle
//! gradient's start row and Radial values match its measurements.
//!
//! The ramp is the reference application's measured Classic black → white Linear curve (Smooth,
//! every tenth of the way), so every shape is checked against the same colour curve.

use photocraft_color::PixelFormat;
use photocraft_raster::Surface;

use super::{BlendMode, GradientShape, Rect, paint_gradient};

const LINEAR: [f32; 11] = [0.0, 2.0, 17.0, 42.0, 70.0, 99.0, 129.0, 162.0, 196.0, 228.0, 255.0];

fn ramp() -> Vec<(f32, [f32; 4])> {
    LINEAR.iter().enumerate().map(|(i, v)| (i as f32 / 10.0, [v / 255.0, v / 255.0, v / 255.0, 1.0])).collect()
}

/// An 800 × 600 RGB8 gradient from `from` to `to`, dither off.
fn painted(shape: GradientShape, from: (f32, f32), to: (f32, f32)) -> Surface {
    let mut s = Surface::new(PixelFormat::RGBA8);
    paint_gradient(&mut s, Rect::new(0, 0, 800, 600), from, to, shape, &ramp(), false, 1.0, BlendMode::Normal, false, None, false);
    s
}

fn check(s: &Surface, shape: GradientShape, expected: &[((i32, i32), f32, f32)]) {
    for &((x, y), want, within) in expected {
        let got = s.pixel(x, y)[0] * 255.0;
        assert!((got - want).abs() <= within, "{shape:?} ({x},{y}): {got} expected {want} ± {within}");
    }
}

#[test]
fn an_angle_gradient_starts_with_the_end_colour_east_of_the_start() {
    let s = painted(GradientShape::Angle, (400.0, 300.0), (600.0, 300.0));
    check(&s, GradientShape::Angle, &[((450, 300), 254.5, 0.5), ((700, 300), 254.5, 0.5), ((500, 400), 4.0, 2.0), ((400, 200), 179.0, 2.0)]);
    // Further round, clockwise on screen.
    check(&s, GradientShape::Angle, &[((400, 400), 30.0, 1.0), ((200, 300), 99.5, 1.0), ((500, 200), 220.0, 2.0)]);
}

#[test]
fn radial_values_match_the_reference() {
    let s = painted(GradientShape::Radial, (400.0, 300.0), (600.0, 300.0));
    check(&s, GradientShape::Radial, &[((400, 300), 0.0, 1.0), ((450, 300), 28.0, 1.0), ((300, 300), 100.0, 1.0), ((400, 200), 100.0, 1.0)]);
    check(&s, GradientShape::Radial, &[((500, 300), 98.0, 1.0), ((400, 400), 98.0, 1.0), ((700, 300), 255.0, 0.0)]);
}

#[test]
fn linear_values_stay_within_one_level() {
    let s = painted(GradientShape::Linear, (100.0, 300.0), (700.0, 300.0));
    let expected: Vec<_> = LINEAR.iter().enumerate().map(|(i, &v)| ((100 + 60 * i as i32, 300), v, 1.0)).collect();
    check(&s, GradientShape::Linear, &expected);
}

#[test]
fn reflected_and_diamond_values_match_the_reference() {
    let s = painted(GradientShape::Reflected, (400.0, 300.0), (600.0, 300.0));
    check(&s, GradientShape::Reflected, &[((300, 300), 100.0, 1.0), ((500, 300), 98.0, 1.0), ((400, 100), 0.0, 1.0), ((400, 500), 0.0, 1.0)]);
    let s = painted(GradientShape::Diamond, (400.0, 300.0), (600.0, 300.0));
    // The ramp is straight between its tenths, so (300,300) and (400,200) land 1.5 below the
    // reference's 102, where today's sample point lands 3 below.
    check(
        &s,
        GradientShape::Diamond,
        &[((300, 300), 102.0, 2.0), ((500, 300), 99.0, 1.0), ((400, 200), 102.0, 2.0), ((450, 250), 99.0, 1.0), ((500, 200), 255.0, 0.0)],
    );
}
