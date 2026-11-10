use super::*;
use photocraft_color::ColorMode;

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
