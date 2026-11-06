//! The pre-optimization prefix implementation remains an exact scalar oracle.
//! A separate point-by-point convolution also checks normalization and sampling.
use super::*;
use crate::{FilterParams, apply_tiled, kernel};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_raster::Surface;

const SHAPES: [BlurShape; 9] = [
    BlurShape::Circle,
    BlurShape::Ring,
    BlurShape::Square,
    BlurShape::Diamond,
    BlurShape::Triangle,
    BlurShape::Hexagon,
    BlurShape::Star,
    BlurShape::Heart,
    BlurShape::Cross,
];

#[test]
fn binary_kernel_sampling_including_fractional_square_is_preserved() {
    for (shape, radius, count) in [
        (BlurShape::Circle, 3.0, 29.0),
        (BlurShape::Ring, 3.0, 20.0),
        (BlurShape::Square, 3.0, 49.0),
        (BlurShape::Square, 2.5, 49.0),
        (BlurShape::Diamond, 3.0, 25.0),
        (BlurShape::Triangle, 3.0, 18.0),
        (BlurShape::Cross, 3.0, 13.0),
    ] {
        assert_eq!(Spans::new(radius, shape_inside(shape)).count, count);
    }
}

fn image(rect: Rect, n: usize, alpha: bool) -> Image {
    let data = (0..rect.width() as usize * rect.height() as usize)
        .flat_map(|i| (0..n).map(move |c| if alpha && c + 1 == n { (i % 7) as f32 / 6.0 } else { ((i * 31 + c * 13) % 97) as f32 / 96.0 }))
        .collect();
    Image { rect, ch: n, data }
}

fn exact(a: &[f32], b: &[f32]) {
    assert_eq!(a.len(), b.len());
    for (i, (a, b)) in a.iter().zip(b).enumerate() {
        assert!(a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()), "sample {i}: {a} != {b}");
    }
}

#[test]
fn scalar_prefix_exact_all_shapes_radii_channels_subrects() {
    for shape in SHAPES {
        for radius in [0.0, 0.49, 0.5, 0.75, 1.0, 1.1, 2.5, 5.0, 25.0, 100.0] {
            let spans = Spans::new(radius, shape_inside(shape));
            for n in 1..=MAXC {
                for alpha in [false, true] {
                    let src = image(Rect::new(-5, -3, 9, 7), n, alpha);
                    let ctx = Ctx { bounds: src.rect, mode: ColorMode::Multichannel, alpha };
                    for out in [src.rect, Rect::new(-2, 1, 3, 2), Rect::new(-8, -7, 12, 10), Rect::EMPTY] {
                        exact(&shape_blur(&src, out, &ctx, radius, shape), &conv_spans(&src, out, &ctx, &spans));
                    }
                }
            }
        }
    }
}

#[test]
fn point_convolution_independent_of_prefix_and_span_accumulation() {
    for shape in SHAPES {
        for radius in [0.0f32, 0.5, 1.0, 1.25, 2.5, 5.0, 7.75] {
            let src = image(Rect::new(-3, -2, 4, 3), 4, true);
            let ctx = Ctx { bounds: src.rect, mode: ColorMode::Rgb, alpha: true };
            let out = Rect::new(-5, -4, 6, 5);
            let actual = shape_blur(&src, out, &ctx, radius, shape);
            let inside = shape_inside(shape);
            let ri = radius.ceil() as i32;
            let mut points: Vec<(i32, i32)> = (-ri..=ri)
                .flat_map(|y| (-ri..=ri).map(move |x| (x, y)))
                .filter(|&(x, y)| radius >= 0.5 && inside(x as f32 / radius, y as f32 / radius))
                .collect();
            if points.is_empty() {
                points.push((0, 0));
            }
            let mut reference = Vec::new();
            for y in out.y0..out.y1 {
                for x in out.x0..out.x1 {
                    let mut sums = [0.0f64; 4];
                    for &(dx, dy) in &points {
                        let a = src.get(x + dx, y + dy, 3);
                        for (c, sum) in sums.iter_mut().enumerate() {
                            let v = src.get(x + dx, y + dy, c);
                            *sum += f64::from(if c < 3 { v * a } else { v });
                        }
                    }
                    let mut px = sums.map(|s| (s / points.len() as f64) as f32);
                    unpremul_px(&mut px, true);
                    reference.extend(px);
                }
            }
            // Finite f32 inputs are exactly representable in these short f64 sums.
            exact(&actual, &reference);
        }
    }
}

#[test]
fn surfaces_exact_depth_models_alpha_selection_and_tile_boundaries() {
    let bounds = Rect::new(-2, -1, 263, 5);
    let area = Rect::new(0, 0, 260, 4);
    let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    let coverage: Vec<f32> = (0..area.width() as usize * area.height() as usize).map(|i| (i % 5) as f32 / 4.0).collect();
    selection.write_region(area, &coverage);
    for shape in SHAPES {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Multichannel] {
                for alpha in [false, true] {
                    let fmt = PixelFormat::new(mode, depth, alpha);
                    let n = fmt.channels();
                    let mut surface = Surface::new(fmt);
                    surface.write_region(bounds, &image(bounds, n, alpha).data);
                    let ctx = Ctx { bounds, mode, alpha };
                    for radius in [2.5, 5.0, 25.0] {
                        let p = FilterParams::ShapeBlur { radius, shape };
                        let reach = radius.ceil() as i32 + 1;
                        for extent in [None, Some(bounds)] {
                            let src = match extent {
                                None => Image::read(&surface, area.inflate(reach)),
                                Some(e) => Image::read_clamped(&surface, area.inflate(reach), e),
                            };
                            let spans = Spans::new(radius, shape_inside(shape));
                            let mut expected = conv_spans(&src, area, &ctx, &spans);
                            crate::mix_selection(&mut expected, area, &selection, &src, alpha);
                            let mut reference = surface.clone();
                            reference.write_region(area, &expected);
                            for tile in [17, 256] {
                                let actual = apply_tiled(&surface, &p, area, bounds, Some(&selection), tile, extent);
                                exact(&actual.read_region(bounds), &reference.read_region(bounds));
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn largest_supported_radius_and_fraction_keep_scalar_kernel() {
    let src = image(Rect::new(-1, -1, 2, 2), 4, true);
    let ctx = Ctx { bounds: src.rect, mode: ColorMode::Rgb, alpha: true };
    for shape in SHAPES {
        for radius in [999.25, 1000.0] {
            let spans = Spans::new(radius, shape_inside(shape));
            exact(&shape_blur(&src, src.rect, &ctx, radius, shape), &conv_spans(&src, src.rect, &ctx, &spans));
        }
    }
}

#[test]
fn degenerate_hdr_and_nonfinite_samples_keep_scalar_behavior() {
    for rect in [Rect::EMPTY, Rect::new(0, 0, 1, 1), Rect::new(0, 0, 1, 7), Rect::new(0, 0, 9, 1)] {
        for alpha in [false, true] {
            let mut src = image(rect, 4, alpha);
            for (i, v) in src.data.iter_mut().enumerate() {
                if !alpha || i % 4 != 3 {
                    *v = [f32::MAX, -f32::MAX, 1e-30, -17.0, f32::INFINITY, f32::NEG_INFINITY, f32::NAN][i % 7];
                } else {
                    *v = [0.0, 1e-8, 1e-7, 1.1e-7, 1.0][i % 5];
                }
            }
            let ctx = Ctx { bounds: rect, mode: ColorMode::Lab, alpha };
            for shape in SHAPES {
                let spans = Spans::new(2.5, shape_inside(shape));
                exact(&shape_blur(&src, rect, &ctx, 2.5, shape), &conv_spans(&src, rect, &ctx, &spans));
            }
        }
    }
    for alpha in [f32::NEG_INFINITY, -1.0, f32::NAN, 1e-30, f32::INFINITY, f32::MAX] {
        let rect = Rect::new(0, 0, 3, 2);
        let src = Image { rect, ch: 4, data: [1e20, -17.0, 0.0, alpha].repeat(6) };
        let ctx = Ctx { bounds: rect, mode: ColorMode::Rgb, alpha: true };
        for shape in SHAPES {
            let spans = Spans::new(2.5, shape_inside(shape));
            exact(&shape_blur(&src, rect, &ctx, 2.5, shape), &conv_spans(&src, rect, &ctx, &spans));
        }
    }
}

#[test]
fn hostile_shape_inputs_are_bounded_and_do_not_panic() {
    let rect = Rect::new(0, 0, 1, 1);
    let ctx = Ctx { bounds: rect, mode: ColorMode::Rgb, alpha: true };
    for src in [
        Image { rect, ch: 0, data: vec![] },
        Image { rect, ch: 9, data: vec![0.0; 9] },
        Image { rect, ch: 4, data: vec![] },
        Image { rect: Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX), ch: 4, data: vec![] },
    ] {
        assert!(kernel(&FilterParams::ShapeBlur { radius: 5.0, shape: BlurShape::Circle }, &src, rect, &ctx).is_empty());
    }
    let src = image(rect, 4, true);
    for radius in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, f32::MAX] {
        let p = FilterParams::ShapeBlur { radius, shape: BlurShape::Circle };
        assert!(matches!(p.halo(), crate::Halo::Radius(1..=1001)));
        assert_eq!(kernel(&p, &src, rect, &ctx).len(), 4);
    }
    assert!(shape_blur(&src, Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX), &ctx, 5.0, BlurShape::Circle).is_empty());
}

#[test]
fn shape_tile_overrides_and_cancel_preserve_input() {
    for r in [Rect::new(0, 0, 4, 4), Rect::new(i32::MAX - 8, i32::MAX - 8, i32::MAX - 4, i32::MAX - 4)] {
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(r, &[0.25, 0.5, 0.75, 1.0]);
        let before = s.read_region(r);
        let p = FilterParams::ShapeBlur { radius: 0.0, shape: BlurShape::Circle };
        for tile in [i32::MIN, -1, 0, 256, i32::MAX] {
            exact(&apply_tiled(&s, &p, r, r, None, tile, Some(r)).read_region(r), &before);
        }
        let ctl = photocraft_raster::Interrupt::cancel_only(&|| true);
        assert!(crate::apply_tiled_with(&s, &p, r, r, None, 256, Some(r), &ctl).is_none());
        exact(&s.read_region(r), &before);
    }
}

#[test]
#[ignore = "36 MP direct-kernel memory regression (about 1.7 GiB); run explicitly"]
fn direct_36mp_image_with_small_output_is_not_rejected() {
    let rect = Rect::new(0, 0, 6000, 6000);
    let src = Image { rect, ch: 4, data: vec![0.25; 36_000_000 * 4] };
    let ctx = Ctx { bounds: rect, mode: ColorMode::Rgb, alpha: true };
    let result = shape_blur(&src, Rect::new(3000, 3000, 3001, 3001), &ctx, 5.0, BlurShape::Circle);
    assert_eq!(result, vec![0.25; 4]);
}

/// An asymmetric weighted kernel detects mirroring independently of span accumulation.
#[test]
fn custom_shape_coverage_matches_point_convolution_without_mirroring() {
    let mask = Image { rect: Rect::new(0, 0, 3, 3), ch: 1, data: vec![1.0, 0.25, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0] };
    let prepared = crate::shape_blur::Prepared::custom(1.0, &mask, &photocraft_raster::Interrupt::NONE).unwrap();
    for (channels, alpha) in [(1, false), (4, true)] {
        let src = image(Rect::new(-2, -2, 5, 5), channels, alpha);
        let ctx = Ctx { bounds: src.rect, mode: ColorMode::Rgb, alpha };
        let out = Rect::new(-3, -3, 6, 6);
        let actual = prepared.run(&src, out, &ctx);
        let mut expected = Vec::new();
        for y in out.y0..out.y1 {
            for x in out.x0..out.x1 {
                let mut sums = vec![0.0f64; channels];
                for ky in 0..3 {
                    for kx in 0..3 {
                        let weight = f64::from(mask.data[ky * 3 + kx]);
                        let (sx, sy) = (x + 1 - kx as i32, y + 1 - ky as i32);
                        let a = if alpha { src.get(sx, sy, channels - 1) } else { 1.0 };
                        for (c, sum) in sums.iter_mut().enumerate() {
                            let v = src.get(sx, sy, c);
                            *sum += f64::from(if alpha && c + 1 < channels { v * a } else { v }) * weight;
                        }
                    }
                }
                let mut px: Vec<f32> = sums.iter().map(|sum| (sum / 1.75) as f32).collect();
                unpremul_px(&mut px, alpha);
                expected.extend(px);
            }
        }
        assert_eq!(actual.len(), expected.len());
        for (a, e) in actual.iter().zip(expected) {
            assert!((a - e).abs() < 1e-6, "{a} != {e}");
        }
    }
    // The top-left kernel corner, rather than its mirror, receives the impulse.
    let mut src = Image { rect: Rect::new(0, 0, 7, 7), ch: 1, data: vec![0.0; 49] };
    src.data[3 * 7 + 3] = 1.0;
    let ctx = Ctx { bounds: src.rect, mode: ColorMode::Grayscale, alpha: false };
    let out = prepared.run(&src, src.rect, &ctx);
    assert!((out[2 * 7 + 2] - 1.0 / 1.75).abs() < 1e-6);
    assert_eq!(out[4 * 7 + 4], 0.0);
}

/// The custom path must agree with the existing square blur, including tiled boundaries.
#[test]
fn custom_square_preserves_builtin_result_and_tile_boundaries() {
    let rect = Rect::new(-3, -2, 17, 14);
    let src = image(rect, 4, true);
    let ctx = Ctx { bounds: rect, mode: ColorMode::Rgb, alpha: true };
    let mask = Image { rect: Rect::new(0, 0, 7, 7), ch: 1, data: vec![1.0; 49] };
    let p = FilterParams::CustomShapeBlur { radius: 2.5, kernel: Some(std::sync::Arc::new(mask)) };
    let actual = kernel(&p, &src, rect, &ctx);
    let expected = shape_blur(&src, rect, &ctx, 2.5, BlurShape::Square);
    exact(&actual, &expected);
    let mut surface = Surface::new(PixelFormat { mode: ColorMode::Rgb, sample: SampleType::F32, alpha: true });
    surface.write_region(rect, &src.data);
    let tiled = apply_tiled(&surface, &p, rect, rect, None, 5, Some(rect));
    let ordinary = apply_tiled(&surface, &FilterParams::ShapeBlur { radius: 2.5, shape: BlurShape::Square }, rect, rect, None, 5, Some(rect));
    exact(&tiled.read_region(rect), &ordinary.read_region(rect));
}

/// Oversized and invalid kernel metadata must be rejected before proportional allocation.
#[test]
fn invalid_custom_kernels_are_refused_without_allocating_their_declared_size() {
    for mask in [
        Image { rect: Rect::new(0, 0, 3, 3), ch: 1, data: vec![0.0; 9] },
        Image { rect: Rect::new(0, 0, 3, 3), ch: 1, data: vec![f32::NAN; 9] },
        Image { rect: Rect::new(0, 0, 3, 3), ch: 1, data: vec![1.1; 9] },
        Image { rect: Rect::new(0, 0, 3, 3), ch: 4, data: vec![1.0; 36] },
        Image { rect: Rect::new(0, 0, 3, 3), ch: 1, data: vec![1.0] },
        Image { rect: Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX), ch: 1, data: vec![] },
    ] {
        assert!(crate::shape_blur::Prepared::custom(1.0, &mask, &photocraft_raster::Interrupt::NONE).is_none());
    }
}

/// Point-by-point weighted convolution of `src` (zero outside it) by a square `mask` of side
/// `2 ri + 1`, normalized by the mask's total coverage: the reference for custom kernels.
fn weighted_reference(mask: &Image, ri: i32, src: &Image, out: Rect, alpha: bool) -> Vec<f32> {
    let size = (2 * ri + 1) as usize;
    let channels = src.ch;
    let total: f64 = mask.data.iter().map(|&v| f64::from(v)).sum();
    let mut expected = Vec::new();
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let mut sums = vec![0.0f64; channels];
            for ky in 0..size {
                for kx in 0..size {
                    let weight = f64::from(mask.data[ky * size + kx]);
                    let (sx, sy) = (x + ri - kx as i32, y + ri - ky as i32);
                    let a = if alpha { src.get(sx, sy, channels - 1) } else { 1.0 };
                    for (c, sum) in sums.iter_mut().enumerate() {
                        let v = src.get(sx, sy, c);
                        *sum += f64::from(if alpha && c + 1 < channels { v * a } else { v }) * weight;
                    }
                }
            }
            let mut px: Vec<f32> = sums.iter().map(|sum| (sum / total) as f32).collect();
            unpremul_px(&mut px, alpha);
            expected.extend(px);
        }
    }
    expected
}

/// Custom weights reach both dispatch paths: the direct kernel and the tiled filter. An
/// all-ones mask could not tell weights that were ignored or mirrored.
#[test]
fn nonuniform_custom_kernel_matches_weighted_reference_through_kernel_and_tiles() {
    let rect = Rect::new(-3, -2, 13, 11);
    let src = image(rect, 4, true);
    let ctx = Ctx { bounds: rect, mode: ColorMode::Rgb, alpha: true };
    // Asymmetric, with distinct weights in every row (5 x 5 for radius 2).
    let data: Vec<f32> = (0..25).map(|i| [0.0, 0.125, 0.5, 1.0, 0.25, 0.75][(i * 7 + i / 5) % 6]).collect();
    let mask = Image { rect: Rect::new(0, 0, 5, 5), ch: 1, data };
    let expected = weighted_reference(&mask, 2, &src, rect, true);
    let p = FilterParams::CustomShapeBlur { radius: 2.0, kernel: Some(std::sync::Arc::new(mask)) };
    let close = |actual: &[f32], what: &str| {
        assert_eq!(actual.len(), expected.len(), "{what}");
        for (i, (a, e)) in actual.iter().zip(&expected).enumerate() {
            assert!((a - e).abs() < 1e-5, "{what} sample {i}: {a} != {e}");
        }
    };
    close(&kernel(&p, &src, rect, &ctx), "kernel");
    let mut surface = Surface::new(PixelFormat { mode: ColorMode::Rgb, sample: SampleType::F32, alpha: true });
    surface.write_region(rect, &src.data);
    // No extent: tiles read transparency beyond the layer, as the reference's zero extension.
    close(&apply_tiled(&surface, &p, rect, rect, None, 4, None).read_region(rect), "apply_tiled");
}

/// A fragmented custom mask (alternating coverage, about size² / 2 runs) would make every
/// output row scan millions of windows; it is refused, while a smooth disc of the same radius
/// and a fully fragmented small kernel are accepted.
#[test]
fn fragmented_large_custom_kernels_are_refused_before_convolution() {
    use crate::shape_blur::{Prepared, max_spans};
    let none = photocraft_raster::Interrupt::NONE;
    let size = 2001usize;
    let alternating = Image { rect: Rect::new(0, 0, size as i32, size as i32), ch: 1, data: (0..size * size).map(|i| (i % 2) as f32).collect() };
    assert!(size * size / 2 > max_spans(size));
    assert!(Prepared::custom(1000.0, &alternating, &none).is_none());
    // Through the filter: refused before any tile is convolved.
    let mut surface = Surface::new(PixelFormat { mode: ColorMode::Rgb, sample: SampleType::F32, alpha: true });
    let r = Rect::new(0, 0, 4, 4);
    surface.fill_rect(r, &[0.25, 0.5, 0.75, 1.0]);
    let p = FilterParams::CustomShapeBlur { radius: 1000.0, kernel: Some(std::sync::Arc::new(alternating)) };
    assert!(crate::apply_tiled_with(&surface, &p, r, r, None, 256, Some(r), &none).is_none());
    // A disc of radius 1000 has one run per row.
    let c = 1000.0f32;
    let disc: Vec<f32> = (0..size * size).map(|i| f32::from(((i % size) as f32 - c).hypot((i / size) as f32 - c) <= c)).collect();
    assert!(Prepared::custom(1000.0, &Image { rect: Rect::new(0, 0, size as i32, size as i32), ch: 1, data: disc }, &none).is_some());
    // Every kernel of up to 256 x 256 is accepted, however fragmented.
    let small = 255usize;
    let fragmented =
        Image { rect: Rect::new(0, 0, small as i32, small as i32), ch: 1, data: (0..small * small).map(|i| ((i % 4) as f32 + 1.0) / 4.0).collect() };
    assert!(Prepared::custom(127.0, &fragmented, &none).is_some());
}

/// Kernel preparation and each tile's convolution poll cancellation, so a large custom kernel
/// does not run a whole tile after Cancel.
#[test]
fn custom_kernel_preparation_and_convolution_stop_when_cancelled() {
    use crate::shape_blur::Prepared;
    let none = photocraft_raster::Interrupt::NONE;
    let cancelled = photocraft_raster::Interrupt::cancel_only(&|| true);
    let mask = Image { rect: Rect::new(0, 0, 3, 3), ch: 1, data: vec![1.0, 0.25, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.5] };
    assert!(Prepared::custom(1.0, &mask, &cancelled).is_none());
    let prepared = Prepared::custom(1.0, &mask, &none).unwrap();
    let src = image(Rect::new(0, 0, 6, 5), 4, true);
    let ctx = Ctx { bounds: src.rect, mode: ColorMode::Rgb, alpha: true };
    assert_eq!(prepared.checked_run(&src, src.rect, &ctx, &none).map(|v| v.len()), Some(6 * 5 * 4));
    assert!(prepared.checked_run(&src, src.rect, &ctx, &cancelled).is_none());
}
