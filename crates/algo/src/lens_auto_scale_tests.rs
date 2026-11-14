//! #13992: with Auto Scale on, Lens Correction scales far enough that no output pixel mixes in
//! the Edge fill, as measured in the reference application on a 400 × 300 picture with a red
//! 20 × 20 square at 0,0.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

use super::{EdgeMode, LensCorrection, auto_scale, correct, generic_profile};

const FRAME: Rect = Rect { x0: 0, y0: 0, x1: 400, y1: 300 };
/// Half an 8-bit level: anything closer reads back as the same byte.
const TOL: f32 = 0.5 / 255.0;
const BLACK: EdgeMode = EdgeMode::Color([0.0, 0.0, 0.0, 1.0]);
const WHITE: EdgeMode = EdgeMode::Color([1.0, 1.0, 1.0, 1.0]);

/// The measured fixture: white RGB 8-bit without alpha, red 20 × 20 square at 0,0.
fn picture() -> Surface {
    let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, false));
    s.fill_rect(FRAME, &[1.0, 1.0, 1.0]);
    s.fill_rect(Rect::new(0, 0, 20, 20), &[1.0, 0.0, 0.0]);
    s
}

fn lens(focal: f64, edge: EdgeMode) -> LensCorrection {
    LensCorrection { profile: Some(generic_profile(focal)), edge, ..Default::default() }
}

/// Every channel within [`TOL`] of `want`.
fn close(px: &[f32], want: &[f32]) -> bool {
    px.len() == want.len() && px.iter().zip(want).all(|(v, w)| (v - w).abs() <= TOL)
}

fn auto_scaled(focal: f64, edge: EdgeMode) -> LensCorrection {
    let lc = lens(focal, edge);
    LensCorrection { scale: auto_scale(&lc, FRAME), ..lc }
}

/// Generic profiles in 12-200 mm whose pincushion shrinks the image, so Auto Scale enlarges.
const SHRINKING: [f64; 4] = [60.0, 85.0, 135.0, 200.0];

#[test]
fn auto_scale_leaves_no_edge_fill_for_shrinking_profiles() {
    let src = picture();
    for focal in SHRINKING {
        let clear = correct(&src, FRAME, &auto_scaled(focal, EdgeMode::Transparency));
        assert!(clear.format().alpha);
        let clear_px = clear.read_region(FRAME);
        let worst = clear_px.as_chunks::<4>().0.iter().map(|p| 1.0 - p[3]).fold(0.0f32, f32::max);
        assert!(worst <= TOL, "{focal} mm: alpha falls to {}", 1.0 - worst);
        for edge in [BLACK, WHITE] {
            let filled = correct(&src, FRAME, &auto_scaled(focal, edge)).read_region(FRAME);
            // Every pixel matches the transparent run's colour: the edge colour never mixes in.
            let pairs = filled.as_chunks::<3>().0.iter().zip(clear_px.as_chunks::<4>().0);
            let diff = pairs.flat_map(|(f, c)| f.iter().zip(c).map(|(a, b)| (a - b).abs())).fold(0.0f32, f32::max);
            assert!(diff <= TOL, "{focal} mm {edge:?}: edge colour mixes in by {diff}");
        }
    }
}

#[test]
fn auto_scale_corner_stays_source_red_at_200_mm() {
    let src = picture();
    for edge in [EdgeMode::Transparency, BLACK, WHITE, EdgeMode::Extension] {
        let px = correct(&src, FRAME, &auto_scaled(200.0, edge)).pixel(0, 0);
        // Only Transparency adds an alpha channel; without one the pixel is opaque.
        assert!(close(&px, &[1.0, 0.0, 0.0, 1.0][..px.len()]), "{edge:?}: {px:?}");
    }
}

#[test]
fn auto_scale_enlarges_only_shrinking_profiles() {
    for focal in SHRINKING {
        let s = auto_scaled(focal, EdgeMode::Transparency).scale;
        assert!(s > 100.0, "{focal} mm: {s}");
    }
    // Identity and magnifying profiles already cover the frame and stay at 100 % (unchanged).
    for focal in [12.0, 16.0, 24.0, 35.0, 50.0] {
        let s = auto_scaled(focal, EdgeMode::Transparency).scale;
        assert!(s == 100.0, "{focal} mm: {s}");
    }
}

#[test]
fn auto_scale_off_keeps_edge_fills() {
    let src = picture();
    let corner = |edge: EdgeMode| correct(&src, FRAME, &lens(200.0, edge)).pixel(0, 0);
    let clear = corner(EdgeMode::Transparency);
    assert!(clear[3] <= TOL, "transparency: {clear:?}");
    let black = corner(BLACK);
    assert!(close(&black, &[0.0, 0.0, 0.0]), "black: {black:?}");
    let white = corner(WHITE);
    assert!(close(&white, &[1.0, 1.0, 1.0]), "white: {white:?}");
    // Edge Extension repeats the border, here the red square.
    let ext = corner(EdgeMode::Extension);
    assert!(close(&ext, &[1.0, 0.0, 0.0]), "extension: {ext:?}");
}

#[test]
fn auto_scale_with_angle_and_distortion_leaves_every_pixel_opaque() {
    // An Angle as Straighten sets it plus Remove Distortion: the outermost row bends, so its
    // extreme falls between evenly spaced check points. 800 x 600 makes that gap wide enough
    // to show.
    let frame = Rect::new(0, 0, 800, 600);
    let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::U8, false));
    src.fill_rect(frame, &[1.0, 1.0, 1.0]);
    let lc = LensCorrection { angle: 8.0, distortion: 60.0, ..Default::default() };
    let out = correct(&src, frame, &LensCorrection { scale: auto_scale(&lc, frame), ..lc });
    assert!(out.format().alpha);
    let worst = out.read_region(frame).as_chunks::<4>().0.iter().map(|p| 1.0 - p[3]).fold(0.0f32, f32::max);
    assert!(worst <= TOL, "alpha falls to {}", 1.0 - worst);
}

#[test]
fn degenerate_frames_keep_the_users_scale() {
    // A 1 px side leaves no half-pixel margin, so a rotation could never be covered.
    let lc = LensCorrection { angle: 5.0, ..Default::default() };
    for frame in [Rect::new(0, 0, 1, 40), Rect::new(0, 0, 40, 1)] {
        let s = auto_scale(&lc, frame);
        assert!(s == 100.0, "{frame:?}: {s}");
    }
}
