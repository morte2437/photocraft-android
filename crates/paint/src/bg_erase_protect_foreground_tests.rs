//! Protect Foreground Color (#11903): the protection fades with the distance from the foreground
//! colour, so the sampled colour is still erased while colours at and beyond the foreground are kept.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba};

use crate::bg_erase::{BgEraseSettings, apply_background_eraser};
use crate::replace::{Limits, Sampling};
use crate::{BrushSettings, Stroke, StrokePoint};

/// Block `i` of the reference test layer: (200, 40 + 20 i, 40), so block i differs from block 0
/// by 20 i in green only.
fn block(i: usize) -> [f32; 4] {
    [200.0 / 255.0, (40.0 + 20.0 * i as f32) / 255.0, 40.0 / 255.0, 1.0]
}

/// Eight 100 px blocks side by side, 900 x 300 (the reference's layout, shifted up).
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

fn settings(protect: Option<usize>) -> BgEraseSettings {
    BgEraseSettings {
        sampling: Sampling::Once,
        limits: Limits::Discontiguous,
        tolerance: 0.5,
        protect_foreground: protect.is_some(),
        foreground: block(protect.unwrap_or(0)),
        ..Default::default()
    }
}

/// The reference's drag from block 0's centre through all eight, with `bs`; alpha (0-255) at each
/// centre.
fn drag_with(fmt: PixelFormat, bs: &BgEraseSettings) -> [i32; 8] {
    let mut s = blocks(fmt);
    apply_background_eraser(&mut s, &stroke(&[(50.0, 150.0), (750.0, 150.0)]), bs, None).unwrap();
    std::array::from_fn(|i| (s.rgba(100 * i as i32 + 50, 150)[3] * 255.0).round() as i32)
}

/// The reference's drag with the foreground at block `protect` (protection off for `None`).
fn drag(fmt: PixelFormat, protect: Option<usize>) -> [i32; 8] {
    drag_with(fmt, &settings(protect))
}

/// Allowed deviation from the measured alpha ("within 8 levels").
const LEVELS: i32 = 8;

fn assert_near(got: [i32; 8], want: [i32; 8], what: &str) {
    for i in 0..8 {
        assert!((got[i] - want[i]).abs() <= LEVELS, "{what}: block {i}, got {got:?}, reference {want:?}");
    }
}

/// The measured rows: Sampling Once, Discontiguous, Tolerance 50, 40 px hard tip.
const REFERENCE: [(usize, [i32; 8]); 3] =
    [(1, [0, 255, 255, 255, 255, 255, 255, 255]), (3, [0, 76, 166, 255, 255, 255, 255, 255]), (5, [0, 40, 94, 148, 201, 255, 255, 255])];

/// The same drag without protection in the reference (the graded erase of #11901).
const UNPROTECTED: [i32; 8] = [0, 30, 75, 114, 156, 198, 238, 255];

#[test]
fn the_sampled_colour_is_erased_and_colours_from_the_foreground_on_are_kept() {
    for (fg, want) in REFERENCE {
        let got = drag(PixelFormat::RGBA8, Some(fg));
        assert_near(got, want, &format!("fg block {fg}"));
        for i in 0..8 {
            // Fully kept and fully erased blocks match exactly.
            if want[i] == 255 || want[i] == 0 {
                assert_eq!(got[i], want[i], "fg block {fg}: block {i}, got {got:?}");
            }
        }
    }
}

#[test]
fn a_foreground_off_in_two_channels_narrows_by_the_sample_distance() {
    // Measured with the same drag: the Tolerance narrows to the foreground's distance from the
    // sample (200,40,40) as the sample match measures it, the largest increase plus the largest
    // decrease over the channels, not the largest per-channel difference.
    let rows: [([i32; 3], [i32; 8]); 4] = [
        // Red -100, green +100: 200 levels away, beyond Tolerance 50, so nothing is protected
        // (the largest per-channel difference, 100, would keep blocks 5 to 7).
        ([100, 140, 40], [0, 32, 75, 112, 156, 196, 238, 255]),
        // Red -50, green +50: 100 levels, like the foreground at block 5.
        ([150, 90, 40], [0, 40, 94, 153, 201, 255, 255, 255]),
        // Two increases, the larger 50: 50 levels.
        ([230, 90, 40], [0, 101, 203, 255, 255, 255, 255, 255]),
        ([200, 90, 70], [0, 98, 202, 255, 255, 255, 255, 255]),
    ];
    for (fg, want) in rows {
        let foreground = [fg[0] as f32 / 255.0, fg[1] as f32 / 255.0, fg[2] as f32 / 255.0, 1.0];
        let got = drag_with(PixelFormat::RGBA8, &BgEraseSettings { foreground, ..settings(Some(0)) });
        assert_near(got, want, &format!("fg {fg:?}"));
        assert_eq!(got[0], 0, "fg {fg:?}: the sample erased, got {got:?}");
    }
}

#[test]
fn protection_narrows_the_tolerance_to_the_foreground_in_one_dab() {
    // One click (Background Swatch = block 0, so the click's own pixel does not matter) on each
    // block: with the foreground at block `fg`, the protected click erases what an unprotected
    // click at Tolerance = the sample-to-foreground distance erases, and keeps block fg and beyond.
    let fmt = PixelFormat::RGBA8;
    let click = |protect: Option<usize>, tolerance: f32| -> Vec<f32> {
        (0..8)
            .map(|i| {
                let mut s = blocks(fmt);
                let bs = BgEraseSettings { sampling: Sampling::BackgroundSwatch, background: block(0), tolerance, ..settings(protect) };
                let x = 100.0 * i as f64 + 50.0;
                apply_background_eraser(&mut s, &stroke(&[(x, 150.0)]), &bs, None).unwrap();
                s.rgba(x as i32, 150)[3]
            })
            .collect()
    };
    for fg in [1, 3, 5] {
        let protected = click(Some(fg), 0.5);
        let narrowed = click(None, 20.0 * fg as f32 / 255.0);
        for i in 0..8 {
            if i < fg {
                assert!((protected[i] - narrowed[i]).abs() <= 1.0 / 255.0, "fg block {fg}: block {i} {protected:?} vs {narrowed:?}");
            } else {
                assert_eq!(protected[i], 1.0, "fg block {fg}: block {i} kept, {protected:?}");
            }
        }
    }
}

#[test]
fn a_foreground_equal_to_the_sample_erases_nothing() {
    for limits in [Limits::Contiguous, Limits::Discontiguous, Limits::FindEdges] {
        for tolerance in [0.0, 0.1, 0.5, 1.0] {
            let s = blocks(PixelFormat::RGBA8);
            let mut t = s.clone();
            let bs = BgEraseSettings { limits, tolerance, ..settings(Some(0)) };
            apply_background_eraser(&mut t, &stroke(&[(50.0, 150.0), (750.0, 150.0)]), &bs, None).unwrap();
            assert_eq!(t, s, "{limits:?} tolerance {tolerance}");
        }
    }
}

#[test]
fn find_edges_keeps_the_foreground_colour_itself() {
    // Find Edges grades like Contiguous (#11904); a pixel exactly at the foreground stays.
    let fmt = PixelFormat::RGBA8;
    let mut s = blocks(fmt);
    let bs = BgEraseSettings { limits: Limits::FindEdges, ..settings(Some(1)) };
    apply_background_eraser(&mut s, &stroke(&[(80.0, 150.0), (120.0, 150.0)]), &bs, None).unwrap();
    assert_eq!(s.rgba(75, 150)[3], 0.0, "the sample erased");
    assert_eq!(s.rgba(110, 150)[3], 1.0, "the foreground colour kept");
}

#[test]
fn protect_off_is_unchanged() {
    // The drag without protection: the reference's row, which the protection change leaves alone.
    assert_near(drag(PixelFormat::RGBA8, None), UNPROTECTED, "protection off");
    let mut s = blocks(PixelFormat::RGBA8);
    let mut t = s.clone();
    let on_far = BgEraseSettings { foreground: [0.0, 0.0, 1.0, 1.0], protect_foreground: true, ..settings(None) };
    apply_background_eraser(&mut s, &stroke(&[(50.0, 150.0), (750.0, 150.0)]), &settings(None), None).unwrap();
    apply_background_eraser(&mut t, &stroke(&[(50.0, 150.0), (750.0, 150.0)]), &on_far, None).unwrap();
    assert_eq!(s, t, "a foreground beyond the Tolerance protects nothing");
}

#[test]
fn protection_works_in_the_documents_own_channels() {
    // 16-bit RGB: the same rows as 8-bit.
    let rgb16 = PixelFormat::new(ColorMode::Rgb, SampleType::U16, true);
    for (fg, want) in REFERENCE {
        assert_near(drag(rgb16, Some(fg)), want, &format!("16-bit fg block {fg}"));
    }
    // Gray: blocks stepping by 20 in gray; foreground at block 3 keeps blocks 3 and beyond and
    // still erases block 0.
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, true);
    let mut s = Surface::new(gray);
    let level = |i: usize| (40.0 + 20.0 * i as f32) / 255.0;
    for i in 0..8 {
        let x = 100 * i as i32;
        s.fill_rect(Rect::new(x, 0, x + 100, 300), &[level(i), 1.0]);
    }
    let fg = [level(3), level(3), level(3), 1.0];
    let bs = BgEraseSettings { foreground: fg, ..settings(Some(3)) };
    apply_background_eraser(&mut s, &stroke(&[(50.0, 150.0), (750.0, 150.0)]), &bs, None).unwrap();
    let a: Vec<f32> = (0..8).map(|i| s.rgba(100 * i + 50, 150)[3]).collect();
    assert_eq!(a[0], 0.0, "gray: the sample erased, {a:?}");
    for (i, v) in a.iter().enumerate().skip(3) {
        assert_eq!(*v, 1.0, "gray: block {i} kept, {a:?}");
    }
}

#[test]
fn the_foreground_is_matched_at_the_documents_precision() {
    // Mid grey 0.5 lies between two 8-bit (and 16-bit) levels and is stored one level up, nearer
    // white. The stored foreground colour must still count as the foreground, and a foreground
    // that is stored as the sampled colour must still protect everything.
    let grey = [0.5, 0.5, 0.5, 1.0];
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let fmt = PixelFormat::new(mode, sample, true);
            let bs = BgEraseSettings {
                sampling: Sampling::Once,
                limits: Limits::FindEdges,
                tolerance: 1.0,
                protect_foreground: true,
                foreground: grey,
                ..Default::default()
            };
            // White (the sample) next to the foreground grey.
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(0, 0, 100, 100), &from_rgba(&fmt, [1.0; 4]));
            s.fill_rect(Rect::new(100, 0, 200, 100), &from_rgba(&fmt, grey));
            apply_background_eraser(&mut s, &stroke(&[(50.0, 50.0), (150.0, 50.0)]), &bs, None).unwrap();
            assert_eq!(s.rgba(50, 50)[3], 0.0, "{fmt:?}: the sample erased");
            assert_eq!(s.rgba(150, 50)[3], 1.0, "{fmt:?}: the stored foreground colour kept");
            // The foreground grey is also the sample.
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(0, 0, 200, 100), &from_rgba(&fmt, grey));
            let t = s.clone();
            apply_background_eraser(&mut s, &stroke(&[(50.0, 50.0), (150.0, 50.0)]), &bs, None).unwrap();
            assert_eq!(s, t, "{fmt:?}: a foreground stored as the sample erases nothing");
        }
    }
}
