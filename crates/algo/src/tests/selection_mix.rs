use super::*;
use photocraft_color::{PixelFormat, SampleType};

fn selection(coverage: f32) -> Surface {
    Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[coverage])
}

#[test]
fn feathered_color_to_alpha_preserves_appearance_over_the_key() {
    let r = Rect::new(0, 0, 1, 1);
    let filter = FilterParams::ColorToAlpha { color: [1.0; 4], transparency_threshold: 0.0, opacity_threshold: 1.0 };
    for sample in SampleType::ALL {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for alpha in [0.5, 1.0] {
                let format = PixelFormat::new(mode, sample, true);
                let mut s = Surface::new(format);
                let mut p = vec![0.5; format.channels()];
                *p.last_mut().unwrap() = alpha;
                s.write_pixel(0, 0, &p);
                let original = s.pixel(0, 0);
                let a = format.channels() - 1;
                for coverage in [0.0, 0.25, 0.5, 0.75, 1.0] {
                    let out = apply(&s, &filter, r, r, Some(&selection(coverage)));
                    let actual = out.pixel(0, 0);
                    let tolerance = if sample == SampleType::U8 { 2.0 / 255.0 } else { 1e-4 };
                    for c in 0..a {
                        let before = original[c] * original[a] + 1.0 - original[a];
                        let after = actual[c] * actual[a] + 1.0 - actual[a];
                        assert!(
                            (before - after).abs() < tolerance,
                            "{sample:?}/{mode:?}, alpha={alpha}, coverage={coverage}: {original:?} -> {actual:?}, over white {before} -> {after}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn feathered_offset_ignores_hidden_colour_of_the_original_pixel() {
    let r = Rect::new(0, 0, 2, 1);
    let filter = FilterParams::Offset { horizontal: 1, vertical: 0, undefined: UndefinedAreas::Wrap };
    for sample in SampleType::ALL {
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, sample, true));
        s.write_pixel(0, 0, &[1.0, 0.0, 0.0, 0.0]);
        s.write_pixel(1, 0, &[0.0, 0.0, 1.0, 1.0]);
        let out = apply(&s, &filter, r, r, Some(&selection(0.5)));
        let p = out.pixel(0, 0);
        assert_eq!(&p[..3], &[0.0, 0.0, 1.0], "{sample:?}: hidden red contaminated the selected blue: {p:?}");
        assert!((p[3] - 0.5).abs() < 1.0 / 255.0);
    }
}

#[test]
fn feathered_blurs_do_not_reintroduce_hidden_colour() {
    let r = Rect::new(0, 0, 3, 3);
    let mut s = Surface::new(PixelFormat::RGBA32F);
    s.fill_rect(r, &[1.0, 0.0, 0.0, 0.0]);
    s.write_pixel(1, 1, &[0.0, 0.0, 1.0, 1.0]);
    for filter in [FilterParams::BoxBlur { radius: 1.0 }, FilterParams::MotionBlur { angle: 0.0, distance: 3.0 }] {
        let full = apply(&s, &filter, r, r, None);
        let out = apply(&s, &filter, r, r, Some(&selection(0.5)));
        let p = out.pixel(0, 1);
        assert!(p[3] > 0.0, "{filter:?}: expected blur to spread coverage");
        assert!((p[3] - full.pixel(0, 1)[3] * 0.5).abs() < 1e-6, "{filter:?}: selection should halve the new coverage");
        assert!(p[0].abs() < 1e-6 && (p[2] - 1.0).abs() < 1e-6, "{filter:?}: hidden red reappeared: {p:?}");
    }
}

#[test]
fn selection_mixing_keeps_hdr_range_and_exact_endpoints() {
    let r = Rect::new(0, 0, 1, 1);
    let original = Image { rect: r, ch: 4, data: vec![-2.0, 4.0, 0.5, 0.25] };
    let filtered = vec![8.0, -4.0, 0.75, 1.0];
    let mut data = filtered.clone();
    mix_selection(&mut data, r, &selection(0.5), &original, true);
    for (&actual, expected) in data.iter().zip([6.0, -2.4, 0.7, 0.625]) {
        assert!((actual - expected).abs() < 1e-6, "{data:?}");
    }
    for coverage in [0.0, 1.0] {
        let mut data = filtered.clone();
        mix_selection(&mut data, r, &selection(coverage), &original, true);
        assert_eq!(data, if coverage == 0.0 { &original.data } else { &filtered }.clone());
    }
}

#[test]
fn selection_mixing_without_alpha_keeps_every_channel_independent() {
    let r = Rect::new(0, 0, 1, 1);
    // One channel is a mask; three/four channels can be RGB or CMYK without alpha.
    for ch in [1, 3, 4] {
        let original = Image { rect: r, ch, data: vec![0.25; ch] };
        let mut data = vec![0.75; ch];
        mix_selection(&mut data, r, &selection(0.25), &original, false);
        assert_eq!(data, vec![0.375; ch]);
    }
}

#[test]
/// Exercise shared selection mixing with extreme finite HDR through five independently checked native filter entry
/// points.
fn selected_finite_hdr_is_safe_through_all_five_checked_native_apis() {
    let ctl = photocraft_raster::Interrupt::NONE;
    let b = Rect::new(0, 0, 1, 1);
    let mut source = Surface::new(PixelFormat::RGBA32F);
    source.write_pixel(0, 0, &[f32::MAX, 0., 0., 1.]);
    let mut kernel = vec![0.; 25];
    kernel[12] = -1.;
    let p = convolution::Params {
        kernel,
        scale: 1.,
        offset: 0.,
        channels: vec![true, false, false, false],
        alpha_weighted: false,
        edge: convolution::Edge::Repeat,
    };
    let output = convolution::apply_with(&source, &p, b, b, Some(&selection(0.5)), &ctl).unwrap();
    assert_eq!(output.pixel(0, 0), vec![0., 0., 0., 1.]);
    let p = channel_noise::Params { amounts: vec![400., 0., 0., 0.], distribution: Distribution::Uniform, monochromatic: false, relative: true, seed: 0 };
    let baseline = channel_noise::apply_with(&source, &p, b, b, None, &ctl).unwrap().pixel(0, 0)[0];
    assert!(baseline < 0.);
    let output = channel_noise::apply_with(&source, &p, b, b, Some(&selection(0.5)), &ctl).unwrap();
    assert_eq!(output.pixel(0, 0)[0], (f64::from(f32::MAX) + (f64::from(baseline) - f64::from(f32::MAX)) * 0.5) as f32);
    let b = Rect::new(0, 0, 3, 3);
    source.fill_rect(b, &[-f32::MAX, 0., 0., 1.]);
    source.write_pixel(1, 1, &[f32::MAX, 0., 0., 1.]);
    let p = rank::Params { radius: 1, color_percentile: 50., alpha_percentile: 50., shape: rank::Neighborhood::Square, channels: None };
    let output = rank::apply_with(&source, &p, b, b, Some(&selection(0.5)), &ctl).unwrap();
    assert_eq!(output.pixel(1, 1), vec![0., 0., 0., 1.]);
    source.fill_rect(b, &[f32::MAX, 0., 0., 1.]);
    source.write_pixel(1, 1, &[-f32::MAX, 0., 0., 1.]);
    let output =
        coverage::apply_with(&source, coverage::Operation::FillHoles { connectivity: 4, alpha_only: false }, b, b, Some(&selection(0.5)), &ctl).unwrap();
    assert_eq!(output.pixel(1, 1), vec![0., 0., 0., 1.]);
    let b = Rect::new(0, 0, 2, 1);
    source.write_pixel(0, 0, &[f32::MAX, 0., 0., 1.]);
    source.write_pixel(1, 0, &[-f32::MAX, 0., 0., 1.]);
    let mut mask = selection(1.);
    mask.write_pixel(1, 0, &[0.5]);
    let output = average::apply_with(&source, b, b, Some(&mask), &ctl).unwrap();
    assert!((f64::from(output.pixel(1, 0)[0]) / f64::from(f32::MAX) + 1. / 3.).abs() < 1e-6);
}

#[test]
/// Verify changed-alpha mixing handles finite HDR roundoff and rejects unrepresentable results without source
/// mutation.
fn changed_alpha_hdr_roundoff_and_unrepresentable_mix_do_not_corrupt_source() {
    let b = Rect::new(0, 0, 1, 1);
    let mut source = Surface::new(PixelFormat::RGBA32F);
    source.write_pixel(0, 0, &[f32::MAX, 0., 0., 0.001]);
    let original = source.clone();
    let out = coverage::apply_with(&source, coverage::Operation::SetAlpha(1.), b, b, Some(&selection(0.003)), &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(out.pixel(0, 0)[0], f32::MAX);
    assert_eq!(source, original);
    assert_eq!(out.pixel(0, 0)[3], 0.001 + (1. - 0.001) * 0.003);
    let b = Rect::new(0, 0, 3, 3);
    source.fill_rect(b, &[f32::MAX, 0., 0., 1.]);
    source.write_pixel(1, 1, &[-f32::MAX, 0., 0., -0.5]);
    let original = source.clone();
    assert_eq!(
        coverage::apply_with(
            &source,
            coverage::Operation::FillHoles { connectivity: 4, alpha_only: false },
            b,
            b,
            Some(&selection(0.5)),
            &photocraft_raster::Interrupt::NONE
        )
        .unwrap_err(),
        coverage::Error::NonFiniteSample
    );
    assert_eq!(source, original);
}

#[test]
/// Pin ordinary finite selection arithmetic bit-for-bit so overflow safeguards do not alter established native
/// output.
fn finite_ordinary_selection_results_keep_legacy_float_bits() {
    let b = Rect::new(0, 0, 1, 1);
    for a in [0., 0.25, 0.75, 1.] {
        for f in [0., 0.25, 0.75, 1.] {
            for k in [0., 0.1, 0.5, 0.9, 1.] {
                let before = vec![-2., 0.25, 4., a];
                let filtered = vec![4., 0.75, -2., f];
                let mut expected = filtered.clone();
                if k == 0. {
                    expected = before.clone();
                } else if k < 1. {
                    if a != f {
                        let ma = a + (f - a) * k;
                        for c in 0..3 {
                            expected[c] = if ma > 0. { (before[c] * (a * (1. - k)) + filtered[c] * (f * k)) / ma } else { 0. };
                        }
                        expected[3] = ma;
                    } else {
                        for c in 0..4 {
                            expected[c] = before[c] + (filtered[c] - before[c]) * k;
                        }
                    }
                }
                let mut actual = filtered;
                mix_selection(&mut actual, b, &selection(k), &Image { rect: b, ch: 4, data: before }, true);
                assert_eq!(actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>());
            }
        }
    }
}
