use super::*;
use crate::{Ctx, Image};
use photocraft_color::{ColorMode, PixelFormat, SampleType};

#[test]
/// Long, thin images are admitted by their pixel/workspace budgets rather than an unrelated edge limit.
fn average_accepts_a_seventy_thousand_pixel_edge_within_workspace() {
    let bounds = Rect::new(0, 0, 70_000, 100);
    let mut source = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false));
    source.fill_rect(Rect::new(35_000, 0, 70_000, 100), &[1.]);
    let output = apply_preserving_alpha_with(&source, bounds, bounds, None, &Interrupt::NONE).unwrap();
    for (x, y) in [(0, 0), (35_000, 50), (69_999, 99)] {
        assert!((output.pixel(x, y)[0] - 0.5).abs() <= 1. / 255.);
    }
    assert_eq!(source.pixel(0, 0), vec![0.]);
    assert_eq!(source.pixel(69_999, 99), vec![1.]);
}

#[test]
/// The raw adapter retains one global mean when a long image produces only a small output tile.
fn raw_average_accepts_long_edges_and_keeps_the_global_mean() {
    for rect in [Rect::new(0, 0, 70_000, 1), Rect::new(0, 0, 1, 70_000)] {
        let mut data = vec![0.; 70_000];
        data[35_000..].fill(1.);
        let source = Image { rect, ch: 1, data };
        let ctx = Ctx { bounds: rect, mode: ColorMode::Grayscale, alpha: false };
        assert_eq!(kernel(&source, Rect::new(0, 0, 1, 1), &ctx), vec![0.5]);
    }
}

#[test]
/// Finite selection values saturate for both mean weighting and publication under both Average policies.
fn finite_selection_coverage_clamps_in_both_average_passes() {
    let bounds = Rect::new(0, 0, 3, 1);
    let mut source = Surface::new(PixelFormat::RGBA32F);
    source.write_region(bounds, &[1., 0., 0., 0.25, 0., 1., 0., 0.5, 0., 0., 1., 0.75]);
    let before = source.clone();
    let format = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
    let mut unbounded = Surface::new(format);
    unbounded.write_region(bounds, &[-0.75, 0.25, 2.]);
    let mut clamped = Surface::new(format);
    clamped.write_region(bounds, &[0., 0.25, 1.]);
    for preserve_alpha in [false, true] {
        let run = |mask: &Surface| {
            if preserve_alpha {
                apply_preserving_alpha_with(&source, bounds, bounds, Some(mask), &Interrupt::NONE)
            } else {
                apply_with(&source, bounds, bounds, Some(mask), &Interrupt::NONE)
            }
        };
        let expected = run(&clamped).unwrap();
        let actual = run(&unbounded).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual.pixel(0, 0), source.pixel(0, 0), "negative coverage hides the edit");
        assert_ne!(actual.pixel(2, 0), source.pixel(2, 0), "coverage above one fully applies the mean");
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            unbounded.write_pixel(1, 0, &[value]);
            assert_eq!(run(&unbounded).unwrap_err(), Error::NonFiniteSample);
        }
        unbounded.write_pixel(1, 0, &[0.25]);
    }
    assert_eq!(source, before);
}

#[test]
fn preserves_alpha_and_averages_hidden_rgb_at_each_depth() {
    let bounds = Rect::new(0, 0, 2, 1);
    for sample in SampleType::ALL {
        let fmt = PixelFormat::new(ColorMode::Rgb, sample, true);
        let mut surface = Surface::new(fmt);
        surface.write_region(bounds, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0]);
        let out = apply(&surface, bounds, None, &Interrupt::NONE).unwrap();
        for (i, p) in out.read_region(bounds).as_chunks::<4>().0.iter().enumerate() {
            assert!((p[0] - 0.5).abs() <= 1.0 / 255.0);
            assert_eq!(p[1], 0.0);
            assert!((p[2] - 0.5).abs() <= 1.0 / 255.0);
            assert_eq!(p[3], if i == 0 { 1.0 } else { 0.0 });
        }
    }
}

#[test]
fn direct_kernel_uses_reference_bounds_for_each_output_tile() {
    let bounds = Rect::new(-1, 0, 1, 1);
    let src = Image { rect: bounds, ch: 4, data: vec![1.0, 0.0, 0.0, 0.25, 0.0, 0.0, 1.0, 0.75] };
    let ctx = Ctx { bounds, mode: ColorMode::Rgb, alpha: true };
    assert_eq!(kernel(&src, Rect::new(-1, 0, 0, 1), &ctx), vec![0.5, 0.0, 0.5, 0.25]);
    assert_eq!(kernel(&src, Rect::new(0, 0, 1, 1), &ctx), vec![0.5, 0.0, 0.5, 0.75]);
}

#[test]
fn rejects_extreme_geometry_and_honours_pre_cancel() {
    let surface = Surface::new(PixelFormat::RGBA8);
    let huge = Rect::new(i32::MIN, 0, i32::MAX, 1);
    assert!(apply(&surface, huge, None, &Interrupt::NONE).is_none());
    assert!(apply(&surface, Rect::new(0, 0, 5, 5), None, &Interrupt::cancel_only(&|| true)).is_none());
    assert_eq!(surface.tile_count(), 0);
}

#[test]
fn soft_selection_weights_the_mean_and_output_mix() {
    let bounds = Rect::new(0, 0, 2, 1);
    for sample in SampleType::ALL {
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        surface.write_region(bounds, &[1.0, 0.0, 0.0, 0.25, 0.0, 0.0, 1.0, 0.75]);
        let before = surface.read_region(bounds);
        let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        selection.write_region(bounds, &[0.25, 1.0]);
        let out = apply(&surface, bounds, Some(&selection), &Interrupt::NONE).unwrap();
        // Unequal coverage makes the mean [0.2, 0, 0.8]. Each pixel then mixes
        // toward it using its own coverage, independently of its original alpha.
        for (i, expected) in [[0.8, 0.0, 0.2], [0.2, 0.0, 0.8]].iter().enumerate() {
            let pixel = out.pixel(i as i32, 0);
            for (actual, expected) in pixel.iter().zip(expected) {
                assert!((actual - expected).abs() <= 1.0 / 255.0, "{sample:?}: {pixel:?}");
            }
            assert_eq!(pixel[3], before[i * 4 + 3]);
        }
    }
}
