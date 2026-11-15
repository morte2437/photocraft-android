//! Background Eraser graded alpha (#11901): one dab erases a colour less the further it is from the
//! sampled colour, over the whole Tolerance, not fully up to the Tolerance. Expected values are the
//! reference application's single-click measurements from the issue (8-bit RGB, 40 px hard round
//! tip, Sampling Background Swatch with background (200,40,40), alpha at the patch centre).

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba};

use crate::bg_erase::{BgEraseSettings, apply_background_eraser};
use crate::replace::{Limits, Sampling};
use crate::{BrushSettings, Stroke, StrokePoint};

/// The sampled colour of every measurement: (200,40,40).
const SAMPLE: [i32; 3] = [200, 40, 40];

/// Allowed deviation from the measured alpha ("within 8 levels").
const LEVELS: i32 = 8;

fn rgb(c: [i32; 3]) -> [f32; 4] {
    [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]
}

fn click(at: (f64, f64)) -> Stroke {
    Stroke {
        brush: BrushSettings { size: 40.0, hardness: 1.0, spacing: 0.25, pressure_size: false, ..Default::default() },
        points: vec![StrokePoint::new(at.0, at.1, 1.0)],
    }
}

fn swatch(tolerance_percent: f32, background: [f32; 4]) -> BgEraseSettings {
    BgEraseSettings {
        sampling: Sampling::BackgroundSwatch,
        limits: Limits::Discontiguous,
        tolerance: tolerance_percent / 100.0,
        background,
        ..Default::default()
    }
}

/// One click in the centre of a 100 px patch of `colour` (straight RGBA) in `fmt`; alpha 0-255 at
/// the centre.
fn one_click(fmt: PixelFormat, colour: [f32; 4], bs: &BgEraseSettings) -> i32 {
    let mut s = Surface::new(fmt);
    s.fill_rect(Rect::new(0, 0, 100, 100), &from_rgba(&fmt, colour));
    apply_background_eraser(&mut s, &click((50.0, 50.0)), bs, None).unwrap();
    (s.rgba(50, 50)[3] * 255.0).round() as i32
}

/// The measured single-click table: (Tolerance, [(green difference d, alpha)]).
fn single_click_table() -> Vec<(f32, Vec<(i32, i32)>)> {
    vec![
        (10.0, vec![(10, 92), (20, 194), (30, 255), (40, 255)]),
        (30.0, vec![(10, 21), (20, 56), (30, 91), (40, 126), (50, 161), (60, 196), (80, 255), (100, 255)]),
        (50.0, vec![(10, 8), (20, 29), (30, 50), (40, 71), (50, 92), (60, 113), (80, 155), (100, 196), (120, 238), (140, 255)]),
        (70.0, vec![(10, 3), (20, 18), (30, 33), (40, 48), (50, 63), (60, 78), (80, 107), (100, 137), (120, 167), (140, 197), (160, 227)]),
        (100.0, vec![(10, 2), (20, 12), (30, 23), (40, 33), (50, 43), (60, 54), (80, 74), (100, 95), (120, 116), (140, 136), (160, 157)]),
    ]
}

fn assert_near(got: i32, want: i32, what: &str) {
    assert!((got - want).abs() <= LEVELS, "{what}: got alpha {got}, measured {want}");
}

#[test]
fn one_click_falls_off_over_the_whole_tolerance() {
    let bs = |tol| swatch(tol, rgb(SAMPLE));
    for (tol, row) in single_click_table() {
        for (d, want) in row {
            let got = one_click(PixelFormat::RGBA8, rgb([SAMPLE[0], SAMPLE[1] + d, SAMPLE[2]]), &bs(tol));
            assert_near(got, want, &format!("Tolerance {tol}, green +{d}"));
        }
    }
}

#[test]
fn same_sign_differences_count_once_and_opposite_signs_add_up() {
    // Measured at Tolerance 50, single clicks, sample (200,40,40): (difference per channel, alpha).
    let rows: [([i32; 3], i32); 15] = [
        ([0, 20, 0], 29),
        ([-20, 0, 0], 29),
        ([0, 0, 20], 29),
        ([0, 20, 20], 29),
        ([0, 40, 20], 71),
        ([0, 20, 40], 71),
        ([0, 60, 20], 113),
        ([-20, 20, 0], 71),
        ([-40, 40, 0], 155),
        ([-60, 60, 0], 238),
        ([10, 10, 10], 8),
        ([20, 20, 20], 29),
        ([40, 40, 40], 71),
        ([-10, -10, -10], 8),
        ([-40, -40, -40], 71),
    ];
    let bs = swatch(50.0, rgb(SAMPLE));
    for (diff, want) in rows {
        let colour = [SAMPLE[0] + diff[0], SAMPLE[1] + diff[1], SAMPLE[2] + diff[2]];
        assert_near(one_click(PixelFormat::RGBA8, rgb(colour), &bs), want, &format!("difference {diff:?}"));
    }
}

#[test]
fn a_blurred_edge_keeps_its_transition_graded() {
    // Red (220,30,30) | green (30,180,60) at x 200, blurred with a 4 px Gaussian; the measured red
    // channel along y 70 from x 185. A Gaussian blur mixes every channel with the same weights, so
    // green and blue follow red's mix fraction.
    const RED: [i32; 30] =
        [220, 220, 220, 220, 219, 219, 217, 214, 210, 203, 194, 182, 168, 152, 134, 116, 98, 82, 68, 56, 47, 40, 36, 33, 31, 31, 30, 30, 30, 30];
    const ALPHA: [i32; 15] = [0, 0, 0, 0, 0, 0, 0, 4, 21, 46, 79, 123, 178, 238, 255];
    let fmt = PixelFormat::RGBA8;
    let mut s = Surface::new(fmt);
    s.fill_rect(Rect::new(0, 0, 185, 140), &from_rgba(&fmt, rgb([220, 30, 30])));
    s.fill_rect(Rect::new(215, 0, 400, 140), &from_rgba(&fmt, rgb([30, 180, 60])));
    for (i, r) in RED.iter().enumerate() {
        let t = (220 - r) as f32 / 190.0;
        let c = [*r as f32 / 255.0, (30.0 + 150.0 * t) / 255.0, (30.0 + 30.0 * t) / 255.0, 1.0];
        let x = 185 + i as i32;
        s.fill_rect(Rect::new(x, 0, x + 1, 140), &from_rgba(&fmt, c));
    }
    for limits in [Limits::Contiguous, Limits::Discontiguous] {
        let mut t = s.clone();
        let bs = BgEraseSettings { sampling: Sampling::Once, limits, tolerance: 0.5, ..Default::default() };
        apply_background_eraser(&mut t, &click((190.0, 70.0)), &bs, None).unwrap();
        for (i, want) in ALPHA.iter().enumerate() {
            let x = 185 + i as i32;
            let got = (t.rgba(x, 70)[3] * 255.0).round() as i32;
            assert_near(got, *want, &format!("{limits:?} x {x}"));
        }
        for x in 200..215 {
            assert_eq!(t.rgba(x, 70)[3], 1.0, "{limits:?} x {x} beyond the Tolerance kept");
        }
    }
}

#[test]
fn an_exact_match_is_erased_and_far_colours_are_kept_at_every_tolerance() {
    for tol in [0.0, 1.0, 10.0, 30.0, 50.0, 70.0, 100.0] {
        let bs = swatch(tol, rgb(SAMPLE));
        assert_eq!(one_click(PixelFormat::RGBA8, rgb(SAMPLE), &bs), 0, "Tolerance {tol}: the sample itself");
        if tol < 100.0 {
            // Opposite-sign differences of 255 + 215: beyond every Tolerance below 100 %.
            assert_eq!(one_click(PixelFormat::RGBA8, rgb([0, 255, 40]), &bs), 255, "Tolerance {tol}: a far colour");
        }
    }
    // Tolerance 0 erases nothing but the exact colour.
    assert_eq!(one_click(PixelFormat::RGBA8, rgb([200, 41, 40]), &swatch(0.0, rgb(SAMPLE))), 255);
}

#[test]
fn the_distance_is_measured_in_the_documents_own_channels() {
    // 16-bit RGB: the same single-click table as 8-bit.
    let rgb16 = PixelFormat::new(ColorMode::Rgb, SampleType::U16, true);
    for (tol, row) in single_click_table() {
        for (d, want) in row {
            let got = one_click(rgb16, rgb([SAMPLE[0], SAMPLE[1] + d, SAMPLE[2]]), &swatch(tol, rgb(SAMPLE)));
            assert_near(got, want, &format!("16-bit Tolerance {tol}, green +{d}"));
        }
    }
    // Gray (8 and 16 bit): the distance is the absolute difference of the gray levels, so a gray
    // patch d levels from the swatch erases like a one-channel difference of d in RGB, either way.
    for sample in [SampleType::U8, SampleType::U16] {
        let gray = PixelFormat::new(ColorMode::Grayscale, sample, true);
        let level = |v: i32| [v as f32 / 255.0, v as f32 / 255.0, v as f32 / 255.0, 1.0];
        for (tol, row) in single_click_table() {
            for (d, want) in row {
                let bs = swatch(tol, level(60));
                if 60 + d <= 255 {
                    assert_near(one_click(gray, level(60 + d), &bs), want, &format!("{sample:?} gray Tolerance {tol}, +{d}"));
                }
                if d <= 60 {
                    assert_near(one_click(gray, level(60 - d), &bs), want, &format!("{sample:?} gray Tolerance {tol}, -{d}"));
                }
            }
        }
    }
}
