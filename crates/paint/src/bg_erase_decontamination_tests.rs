//! Background Eraser colour decontamination (#11902): a partly erased pixel loses the sampled
//! colour's share, so composited over the sampled colour it gives back its original colour, and
//! later dabs match its current, decontaminated colour. Expected values are the reference
//! application's measurements from the issue (8-bit RGB, 40 px hard round tip, sample (200,40,40)).

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba};

use crate::bg_erase::{BgEraseSettings, apply_background_eraser};
use crate::replace::{Limits, Sampling};
use crate::{BrushSettings, Stroke, StrokePoint};

/// Block `i` of the reference test layer: (200, 40 + 20 i, 40).
fn block(i: usize) -> [f32; 4] {
    [200.0 / 255.0, (40.0 + 20.0 * i as f32) / 255.0, 40.0 / 255.0, 1.0]
}

/// Eight 100 px blocks side by side, 900 x 300; the reference's drag row y 450 is y 150 here.
fn blocks(fmt: PixelFormat) -> Surface {
    let mut s = Surface::new(fmt);
    for i in 0..8 {
        let x = 100 * i as i32;
        s.fill_rect(Rect::new(x, 0, x + 100, 300), &from_rgba(&fmt, block(i)));
    }
    s
}

fn stroke(points: &[(f64, f64)]) -> Stroke {
    Stroke {
        brush: BrushSettings { size: 40.0, hardness: 1.0, spacing: 0.25, pressure_size: false, ..Default::default() },
        points: points.iter().map(|&(x, y)| StrokePoint::new(x, y, 1.0)).collect(),
    }
}

fn once(tolerance_percent: f32) -> BgEraseSettings {
    BgEraseSettings { sampling: Sampling::Once, limits: Limits::Discontiguous, tolerance: tolerance_percent / 100.0, ..Default::default() }
}

fn swatch(tolerance_percent: f32) -> BgEraseSettings {
    BgEraseSettings { sampling: Sampling::BackgroundSwatch, background: block(0), ..once(tolerance_percent) }
}

/// RGBA 0-255 at block `i`'s centre.
fn rgba(s: &Surface, i: usize) -> [i32; 4] {
    let p = s.rgba(100 * i as i32 + 50, 150);
    p.map(|v| (v * 255.0).round() as i32)
}

/// The reference's drag from block 0's centre to `to_x`, Sampling Once.
fn drag(fmt: PixelFormat, tolerance_percent: f32, to_x: f64) -> Surface {
    let mut s = blocks(fmt);
    apply_background_eraser(&mut s, &stroke(&[(50.0, 150.0), (to_x, 150.0)]), &once(tolerance_percent), None);
    s
}

/// Separate single-click strokes on every block centre, `clicks` times, Background Swatch.
fn clicks(fmt: PixelFormat, tolerance_percent: f32, clicks: usize) -> Surface {
    let mut s = blocks(fmt);
    for _ in 0..clicks {
        for i in 0..8 {
            apply_background_eraser(&mut s, &stroke(&[(100.0 * i as f64 + 50.0, 150.0)]), &swatch(tolerance_percent), None);
        }
    }
    s
}

fn assert_alpha_row(s: &Surface, want: &[i32], what: &str) {
    let got: Vec<i32> = (0..want.len()).map(|i| rgba(s, i)[3]).collect();
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() <= 8, "{what}: block {i} alpha {g}, measured {w} (row {got:?})");
    }
}

#[test]
fn the_eight_block_drag_gives_the_measured_rgba() {
    const WANT: [[i32; 4]; 8] = [
        [200, 40, 40, 0],
        [200, 216, 40, 29],
        [200, 184, 40, 71],
        [200, 175, 40, 113],
        [200, 172, 40, 155],
        [200, 170, 40, 196],
        [200, 169, 40, 238],
        [200, 180, 40, 255],
    ];
    let s = drag(PixelFormat::RGBA8, 50.0, 750.0);
    for (i, want) in WANT.iter().enumerate() {
        let got = rgba(&s, i);
        // Block 0 is fully erased: only its alpha is defined.
        let channels = if want[3] == 0 { 3..4 } else { 0..4 };
        for c in channels {
            assert!((got[c] - want[c]).abs() <= 8, "block {i}: got {got:?}, measured {want:?}");
        }
    }
}

#[test]
fn the_drag_table_of_11901_holds_with_decontamination() {
    let rows: [(f32, [i32; 8]); 5] = [
        (10.0, [0, 194, 255, 255, 255, 255, 255, 255]),
        (20.0, [0, 90, 197, 255, 255, 255, 255, 255]),
        (30.0, [0, 56, 126, 196, 255, 255, 255, 255]),
        (50.0, [0, 30, 75, 114, 156, 198, 238, 255]),
        (70.0, [0, 18, 48, 78, 107, 137, 167, 197]),
    ];
    for (tol, want) in rows {
        assert_alpha_row(&drag(PixelFormat::RGBA8, tol, 750.0), &want, &format!("drag at Tolerance {tol}"));
    }
    // At Tolerance 100 the decontaminated colours stay inside the Tolerance, so every dab erases
    // a bit more; the drag on to 790 is the measured row (block 7 is no reference at 750).
    assert_alpha_row(&drag(PixelFormat::RGBA8, 100.0, 790.0), &[0, 7, 19, 32, 44, 56, 68, 81], "drag at Tolerance 100");
}

#[test]
fn later_strokes_match_the_decontaminated_colour() {
    let fmt = PixelFormat::RGBA8;
    assert_alpha_row(&clicks(fmt, 100.0, 1), &[0, 12, 33, 54, 74, 95, 116, 136], "Tolerance 100, 1 click");
    assert_alpha_row(&clicks(fmt, 100.0, 2), &[0, 10, 28, 45, 62, 80, 97, 114], "Tolerance 100, 2 clicks");
    assert_alpha_row(&clicks(fmt, 100.0, 3), &[0, 8, 23, 38, 52, 67, 81, 96], "Tolerance 100, 3 clicks");
    // At Tolerance 50 the decontaminated colours are outside the Tolerance: a second click leaves
    // the block centres as they were.
    let one = clicks(fmt, 50.0, 1);
    assert_alpha_row(&one, &[0, 29, 71, 113, 155, 196, 238, 255], "Tolerance 50, 1 click");
    let two = clicks(fmt, 50.0, 2);
    for i in 1..8 {
        assert_eq!(rgba(&two, i), rgba(&one, i), "Tolerance 50, 2 clicks: block {i}");
    }
}

#[test]
fn partly_erased_pixels_composite_back_to_their_original_colour() {
    // Composited over the sampled colour, each partly erased block centre gives back its original
    // colour within 2 levels, after one click and after the drag.
    for s in [clicks(PixelFormat::RGBA8, 50.0, 1), drag(PixelFormat::RGBA8, 50.0, 750.0)] {
        for i in 1..7 {
            let [r, g, b, a] = rgba(&s, i);
            assert!(a > 0 && a < 255, "block {i} partly erased, alpha {a}");
            let over = |c: i32, s: i32| (c * a + s * (255 - a)) as f32 / 255.0;
            let orig = block(i).map(|v| v * 255.0);
            for (k, (got, want)) in [over(r, 200), over(g, 40), over(b, 40)].iter().zip(orig).enumerate() {
                assert!((got - want).abs() <= 2.0, "block {i} channel {k}: composite {got}, original {want}");
            }
        }
    }
}

#[test]
fn kept_and_erased_pixels_and_the_channel_range() {
    let s = drag(PixelFormat::RGBA8, 50.0, 750.0);
    // Fully kept: the colour exactly; outside the brush: untouched.
    assert_eq!(rgba(&s, 7), [200, 180, 40, 255]);
    assert_eq!(s.rgba(350, 10), blocks(PixelFormat::RGBA8).rgba(350, 10));
    // Fully erased: alpha 0.
    assert_eq!(rgba(&s, 0)[3], 0);
    // At Tolerance 100 the decontaminated green clips at 255 rather than wrapping, and 32-bit
    // float results stay finite and within 0..1 for in-range colours.
    let t = clicks(PixelFormat::RGBA8, 100.0, 1);
    assert_eq!(rgba(&t, 7)[1], 255, "clipped, not wrapped");
    let f = clicks(PixelFormat::RGBA32F, 100.0, 3);
    for x in (0..900).step_by(7) {
        for v in f.rgba(x, 150) {
            assert!(v.is_finite() && (0.0..=1.0).contains(&v), "x {x}: {v}");
        }
    }
}

#[test]
fn it_works_in_the_documents_own_channels() {
    // 16-bit RGB: the same RGBA as 8-bit.
    let rgb16 = PixelFormat::new(ColorMode::Rgb, SampleType::U16, true);
    let a = drag(PixelFormat::RGBA8, 50.0, 750.0);
    let b = drag(rgb16, 50.0, 750.0);
    for i in 0..8 {
        let (x, y) = (rgba(&a, i), rgba(&b, i));
        for c in 0..4 {
            assert!((x[c] - y[c]).abs() <= 2 || x[3] == 0, "16-bit block {i}: {y:?} against 8-bit {x:?}");
        }
    }
    // Gray (8 and 16 bit): the gray level is decontaminated the same way, so the composite over
    // the sampled gray gives back the original level, and a second click at Tolerance 50 changes
    // nothing at the block centres.
    for sample in [SampleType::U8, SampleType::U16] {
        let gray = PixelFormat::new(ColorMode::Grayscale, sample, true);
        let level = |i: usize| (40.0 + 20.0 * i as f32) / 255.0;
        let mut s = Surface::new(gray);
        for i in 0..8 {
            let x = 100 * i as i32;
            s.fill_rect(Rect::new(x, 0, x + 100, 300), &[level(i), 1.0]);
        }
        let bs = BgEraseSettings { background: [level(0), level(0), level(0), 1.0], ..swatch(50.0) };
        let click_all = |s: &mut Surface| {
            for i in 0..8 {
                apply_background_eraser(s, &stroke(&[(100.0 * i as f64 + 50.0, 150.0)]), &bs, None);
            }
        };
        click_all(&mut s);
        let first = s.clone();
        for i in 1..7 {
            let p = s.pixel(100 * i as i32 + 50, 150);
            let (v, a) = (p[0], p[1]);
            assert!(a > 0.0 && a < 1.0, "{sample:?} block {i} partly erased, alpha {a}");
            let over = v * a + level(0) * (1.0 - a);
            assert!((over - level(i)).abs() <= 2.0 / 255.0, "{sample:?} block {i}: composite {over}, original {}", level(i));
        }
        click_all(&mut s);
        for i in 1..8 {
            let at = (100 * i + 50, 150);
            assert_eq!(s.pixel(at.0, at.1), first.pixel(at.0, at.1), "{sample:?} block {i}: a second click at Tolerance 50 changes nothing");
        }
    }
}
