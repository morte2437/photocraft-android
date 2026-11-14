use super::*;
use photocraft_raster::{Interrupt, Surface};

#[test]
/// Without an owner extent, the requested area supplies Average's domain even beyond reference bounds.
fn average_without_extent_uses_requested_area_beyond_reference_bounds() {
    let bounds = Rect::new(0, 0, 1, 1);
    let area = Rect::new(-1, 0, 2, 1);
    for sample in SampleType::ALL {
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        let colors = if sample == SampleType::F32 { [[-2., 0., 4.], [0., 2., 2.], [2., 4., 0.]] } else { [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] };
        for (x, (color, alpha)) in (-1..2).zip(colors.into_iter().zip([0., 0.5, 1.])) {
            source.write_pixel(x, 0, &[color[0], color[1], color[2], alpha]);
        }
        source.write_pixel(-2, 0, &[0.25, 0.5, 0.75, 1.]);
        source.write_pixel(2, 0, &[0.75, 0.5, 0.25, 1.]);
        let before = source.clone();
        let params = FilterParams::Average;
        for output in [
            apply(&source, &params, area, bounds, None),
            apply_tiled(&source, &params, area, bounds, None, 1, None),
            apply_tiled_with(&source, &params, area, bounds, None, 2, None, &Interrupt::NONE).unwrap(),
        ] {
            let expected = if sample == SampleType::F32 { [0., 2., 2.] } else { [1. / 3.; 3] };
            for x in -1..2 {
                let pixel = output.pixel(x, 0);
                for (value, expected) in pixel.iter().zip(expected) {
                    assert!((value - expected).abs() < 1e-6, "{sample:?}, x={x}: {pixel:?}");
                }
                assert_eq!(pixel[3], source.pixel(x, 0)[3], "native alpha is preserved");
            }
            for x in [-2, 2] {
                assert_eq!(output.pixel(x, 0), source.pixel(x, 0), "the requested area still limits publication");
            }
        }
        assert_eq!(source, before);
    }
}

#[test]
/// Checked and optional Average replay publish the requested owner area beyond the document canvas.
fn average_dispatch_uses_owner_extent_for_off_canvas_mean_and_writes() {
    let canvas = Rect::new(0, 0, 2, 1);
    let extent = Rect::new(-2, 0, 4, 1);
    let area = Rect::new(-2, 0, 3, 1);
    let mut source = Surface::new(PixelFormat::RGBA32F);
    for x in -2..4 {
        source.write_pixel(x, 0, &[(x + 2) as f32, 0.25, 0.5, 0.5]);
    }
    let expected = average::apply_preserving_alpha_with(&source, area, extent, None, &Interrupt::NONE).unwrap();
    assert_eq!(expected.pixel(-2, 0), vec![2., 0.25, 0.5, 0.5]);
    assert_eq!(expected.pixel(3, 0), source.pixel(3, 0), "outside the requested publication area");
    let params = FilterParams::Average;
    let actual = try_apply_in_with(&source, &params, area, canvas, None, extent, &Interrupt::NONE).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(apply_in_with(&source, &params, area, canvas, None, extent, &Interrupt::NONE).unwrap(), expected);
}

#[test]
/// HDR adaptation reads the complete owner while area restricts publication, including off-canvas pixels.
fn hdr_dispatch_uses_owner_extent_for_adaptation_and_off_canvas_writes() {
    let canvas = Rect::new(0, 0, 2, 1);
    let extent = Rect::new(-2, -1, 4, 2);
    let area = Rect::new(-1, 0, 3, 1);
    let mut source = Surface::new(PixelFormat::RGBA32F);
    for y in -1..2 {
        for x in -2..4 {
            source.write_pixel(x, y, &[if x == -2 { 16. } else { 0.2 + 0.3 * (x + 2) as f32 }, 0.1 + 0.12 * (y + 1) as f32, 0.4, 1.]);
        }
    }
    let options = tone::HdrToning { radius: 1., ..Default::default() };
    let expected = tone::checked_hdr::apply_with(&source, &options, area, extent, None, &Interrupt::NONE).unwrap();
    let canvas_only = tone::checked_hdr::apply_with(&source, &options, area, canvas, None, &Interrupt::NONE).unwrap();
    assert_ne!(expected.pixel(-1, 0), source.pixel(-1, 0), "off-canvas publication must actually change pixels");
    assert_ne!(expected.pixel(0, 0), canvas_only.pixel(0, 0), "off-canvas owner samples must affect adaptation inside the canvas");
    assert_eq!(expected.pixel(-2, -1), source.pixel(-2, -1), "area still limits publication");
    let params = FilterParams::HdrToning(options);
    let actual = try_apply_in_with(&source, &params, area, canvas, None, extent, &Interrupt::NONE).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(apply_in_with(&source, &params, area, canvas, None, extent, &Interrupt::NONE).unwrap(), expected);
}

/// The fallible dispatcher retains native failures instead of converting them into missing output.
#[test]
fn checked_dispatch_preserves_parameters_samples_limits_and_cancellation() {
    let area = Rect::new(0, 0, 1, 1);
    let mut source = Surface::new(PixelFormat::RGBA32F);
    source.write_pixel(0, 0, &[f32::NAN, 0., 0., 1.]);
    for params in [FilterParams::Average, FilterParams::SetAlpha { value: 0.5 }] {
        assert_eq!(try_apply_in_with(&source, &params, area, area, None, area, &Interrupt::NONE), Err(ApplyError::Native(coverage::Error::NonFiniteSample)));
    }
    let bad_hdr = FilterParams::HdrToning(tone::HdrToning { radius: f32::NAN, ..Default::default() });
    assert_eq!(
        try_apply_in_with(&source, &bad_hdr, area, area, None, area, &Interrupt::NONE),
        Err(ApplyError::Hdr(tone::checked_hdr::Error::InvalidParameter))
    );
    let huge = Rect::new(0, 0, 10_000, 10_000);
    assert_eq!(
        try_apply_in_with(&source, &FilterParams::Average, huge, huge, None, huge, &Interrupt::NONE),
        Err(ApplyError::Native(coverage::Error::InvalidGeometry))
    );
    let cancel = || true;
    assert_eq!(try_apply_in_with(&source, &FilterParams::Average, area, area, None, area, &Interrupt::cancel_only(&cancel)), Err(ApplyError::Cancelled));
    assert!(apply_in_with(&source, &FilterParams::Average, area, area, None, area, &Interrupt::NONE).is_none());
}

#[test]
/// Pin CMYK convolution offsets to brightness rather than increasing the stored ink amounts.
fn cmyk_convolution_offset_is_a_brightness_offset() {
    let rect = Rect::new(0, 0, 1, 1);
    let source = Surface::with_default(PixelFormat::new(ColorMode::Cmyk, SampleType::F32, false), &[0.3; 4]);
    for (offset, ink) in [(0., 1.), (0.25, 0.75)] {
        let params =
            convolution::Params { kernel: vec![0.; 25], scale: 1., offset, channels: vec![true; 4], alpha_weighted: false, edge: convolution::Edge::Repeat };
        assert_eq!(convolution::apply_with(&source, &params, rect, rect, None, &Interrupt::NONE).unwrap().pixel(0, 0), vec![ink; 4]);
    }
}

#[test]
/// Require flat RGB and grayscale histograms to produce identity tone, contrast and color corrections.
fn flat_auto_levels_are_identity_at_every_native_depth() {
    for depth in SampleType::ALL {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for value in [0., 128. / 255., 1.] {
                let format = PixelFormat::new(mode, depth, true);
                let source = Surface::with_default(format, &[vec![value; format.channels() - 1], vec![1.]].concat());
                for kind in [tone::auto_levels::Kind::Tone, tone::auto_levels::Kind::Contrast, tone::auto_levels::Kind::Color] {
                    assert_eq!(tone::auto_levels::analyze(&source, Rect::new(0, 0, 2, 2), kind, &Interrupt::NONE).unwrap(), [[0., 1., 1.]; 3]);
                }
            }
        }
    }
}

#[test]
/// Compare matte removal with native-channel unassociation across models, depths and alpha endpoints.
fn native_matte_unassociation_keeps_separations_and_hdr() {
    let rect = Rect::new(0, 0, 1, 1);
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Multichannel] {
        for depth in SampleType::ALL {
            let format = PixelFormat::new(mode, depth, true);
            for alpha in [0., 0.5, 1.] {
                let mut values = (0..format.channels() - 1).map(|c| 0.1 * (c + 1) as f32).collect::<Vec<_>>();
                if depth == SampleType::F32 {
                    values[0] = -0.75;
                    if values.len() > 1 {
                        values[1] = 2.;
                    }
                }
                values.push(alpha);
                let mut source = Surface::new(format);
                source.write_pixel(0, 0, &values);
                let before = source.pixel(0, 0);
                let mut expected = before.clone();
                let a = *before.last().unwrap();
                if a > 0. && a < 1. {
                    let cmyk_black = photocraft_color::convert::rgb_to_cmyk([0.; 3]);
                    for (channel, v) in expected.iter_mut().take(format.channels() - 1).enumerate() {
                        let black: f32 = match (mode, channel) {
                            (ColorMode::Cmyk, c) => cmyk_black[c],
                            (ColorMode::Lab, 1 | 2) => 128. / 255.,
                            _ => 0.,
                        };
                        *v = ((f64::from(*v) - f64::from(black) * (1. - f64::from(a))) / f64::from(a)) as f32;
                    }
                }
                let mut oracle = Surface::new(format);
                oracle.write_pixel(0, 0, &expected);
                assert_eq!(
                    matting::remove_black_matte_with(&source, rect, &Interrupt::NONE).unwrap().pixel(0, 0),
                    oracle.pixel(0, 0),
                    "{mode:?}/{depth:?}/{alpha}"
                );
                assert_eq!(source.pixel(0, 0), before);
            }
        }
    }
}

#[test]
/// Recover CMYK foreground inks and preserve neutral Lab chroma when removing a black matte.
fn black_matte_keeps_lab_neutral_and_cmyk_native_separations() {
    let rect = Rect::new(0, 0, 1, 1);
    let mut cmyk = Surface::new(PixelFormat::new(ColorMode::Cmyk, SampleType::F32, true));
    let foreground = [0., 0.1, 0.2, 0.3];
    let matte = photocraft_color::convert::rgb_to_cmyk([0.; 3]);
    let mut observed: Vec<_> = foreground.into_iter().zip(matte).map(|(f, b)| 0.5 * f + 0.5 * b).collect();
    observed.push(0.5);
    cmyk.write_pixel(0, 0, &observed);
    let unmatte = matting::remove_black_matte_with(&cmyk, rect, &Interrupt::NONE).unwrap();
    for (got, expected) in unmatte.pixel(0, 0).into_iter().zip(foreground) {
        assert!((got - expected).abs() < 1e-6);
    }
    let mut lab = Surface::new(PixelFormat::new(ColorMode::Lab, SampleType::F32, true));
    lab.write_pixel(0, 0, &[0.2, 128. / 255., 128. / 255., 0.5]);
    let unmatte = matting::remove_black_matte_with(&lab, rect, &Interrupt::NONE).unwrap();
    assert_eq!(unmatte.pixel(0, 0), vec![0.4, 128. / 255., 128. / 255., 0.5]);
}

#[test]
/// Keep unselected color-channel bits intact when convolution or seeded noise changes selected alpha.
fn selected_alpha_filters_preserve_every_unselected_channel_bit() {
    let rect = Rect::new(0, 0, 1, 1);
    let mask = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.3]);
    let mut source = Surface::new(PixelFormat::RGBA32F);
    source.write_pixel(0, 0, &[0.3, -0., 1e30, 0.25]);
    let before = source.pixel(0, 0);
    let convolution = convolution::Params {
        kernel: vec![0.; 25],
        scale: 1.,
        offset: 0.7,
        channels: vec![false, false, false, true],
        alpha_weighted: false,
        edge: convolution::Edge::Repeat,
    };
    let got = convolution::apply_with(&source, &convolution, rect, rect, Some(&mask), &Interrupt::NONE).unwrap();
    for (v, b) in got.pixel(0, 0).iter().zip(&before).take(3) {
        assert_eq!(v.to_bits(), b.to_bits());
    }
    for seed in 0..32 {
        let noise = channel_noise::Params { amounts: vec![0., 0., 0., 100.], distribution: Distribution::Uniform, monochromatic: false, relative: false, seed };
        let got = channel_noise::apply_with(&source, &noise, rect, rect, Some(&mask), &Interrupt::NONE).unwrap();
        for (v, b) in got.pixel(0, 0).iter().zip(&before).take(3) {
            assert_eq!(v.to_bits(), b.to_bits());
        }
    }
}

#[test]
/// Exclude hidden source colors from the premultiplied Average result when a selection creates coverage.
fn average_selection_does_not_mix_hidden_colors_into_new_coverage() {
    let rect = Rect::new(0, 0, 2, 1);
    let mask = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.5]);
    for depth in SampleType::ALL {
        let mut source = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
        source.write_pixel(0, 0, &[1., 0., 0., 0.]);
        source.write_pixel(1, 0, &[0., 0., 1., 1.]);
        let out = average::apply_with(&source, rect, rect, Some(&mask), &Interrupt::NONE).unwrap();
        for (v, expected) in out.pixel(0, 0).into_iter().zip([0., 0., 1., 0.25]) {
            assert!((v - expected).abs() < 0.005);
        }
    }
}

#[test]
/// Exercise extreme finite HDR colors and controls without allowing nonfinite or out-of-range tone results.
fn checked_hdr_extreme_finite_inputs_and_controls_stay_finite() {
    for (shadow, highlight) in [(0., 0.), (50., 50.), (50., -50.), (-50., 50.)] {
        for mut pixels in [
            [[0.001, 0.001, 0.001, 1.], [1e30, 1e30, 1e30, 1.]],
            [[1e30, 1e30, 1e30, 1.], [f32::MAX, -f32::MAX, 0., 1.]],
            [[f32::MAX, f32::MAX, f32::MAX, 1.], [0.001, 0.001, 0.001, 1.]],
        ] {
            let params = tone::HdrToning { radius: 2., detail: 300., exposure: 5., gamma: 0.1, shadow, highlight, ..Default::default() };
            tone::checked_hdr::process_with(&mut pixels, 2, 1, &params, &Interrupt::NONE).unwrap();
            assert!(pixels.iter().flatten().all(|v| v.is_finite() && (0. ..=1.).contains(v)));
        }
    }
}
