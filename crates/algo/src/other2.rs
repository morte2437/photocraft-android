//! Filter › Other: Custom (5×5 convolution) and HSB/HSL.

use photocraft_color::ColorMode;
use photocraft_geom::Rect;

use crate::fxutil::{ncol, rgba, set_rgba, xy};
use crate::image::Image;
use crate::{Ctx, HsbModel};

/// Custom: `Σ kernel·pixel / scale + offset/255` per colour channel (alpha
/// kept). Kernels shorter than 25 entries are zero-padded. CMYK samples are
/// ink, so there it runs on `1 - ink` and a positive offset brightens.
pub(crate) fn custom(src: &Image, out: Rect, ctx: &Ctx, kernel: &[f32], scale: f32, offset: f32) -> Vec<f32> {
    let n = src.ch;
    let cc = ncol(ctx, n);
    let k: Vec<(i32, i32, f32)> =
        (0..25).filter_map(|i| kernel.get(i).copied().filter(|v| *v != 0.0).map(|v| (i as i32 % 5 - 2, i as i32 / 5 - 2, v))).collect();
    let s = if scale == 0.0 { 1.0 } else { scale };
    let off = offset / 255.0;
    let sub = crate::fxutil::subtractive(ctx);
    let mut res = src.crop(out);
    if k.is_empty() {
        return res;
    }
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        for (c, v) in px.iter_mut().enumerate().take(cc) {
            // CMYK: convolve brightness (1 - ink), then back to ink.
            let lum = |v: f32| if sub { 1.0 - v } else { v };
            let sum: f32 = k.iter().map(|&(dx, dy, w)| lum(src.get(x + dx, y + dy, c)) * w).sum();
            *v = lum(sum / s + off);
        }
    }
    res
}

/// RGB → HSB (hue, saturation, brightness), all 0–1.
pub(crate) fn rgb_to_hsb(c: [f32; 3]) -> [f32; 3] {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let d = mx - mn;
    let h = hue(c, mx, d);
    [h, if mx > 0.0 { d / mx } else { 0.0 }, mx]
}

/// RGB → HSL, all 0–1.
pub(crate) fn rgb_to_hsl(c: [f32; 3]) -> [f32; 3] {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let d = mx - mn;
    let l = (mx + mn) / 2.0;
    let s = if d <= 0.0 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-7) };
    [hue(c, mx, d), s.clamp(0.0, 1.0), l]
}

fn hue(c: [f32; 3], mx: f32, d: f32) -> f32 {
    if d <= 0.0 {
        return 0.0;
    }
    let h = if mx == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    h / 6.0
}

/// Chroma/min helper: RGB from hue, chroma and the value to add.
fn from_hcm(h: f32, chroma: f32, m: f32) -> [f32; 3] {
    let hp = h.rem_euclid(1.0) * 6.0;
    let x = chroma * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as i32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    [r + m, g + m, b + m]
}

pub(crate) fn hsb_to_rgb(c: [f32; 3]) -> [f32; 3] {
    let chroma = c[2] * c[1];
    from_hcm(c[0], chroma, c[2] - chroma)
}

pub(crate) fn hsl_to_rgb(c: [f32; 3]) -> [f32; 3] {
    let chroma = (1.0 - (2.0 * c[2] - 1.0).abs()) * c[1];
    from_hcm(c[0], chroma, c[2] - chroma / 2.0)
}

/// HSB/HSL: reinterprets the three colour channels — encodes RGB as
/// H, S, B/L in the R, G, B channels, or decodes such channels back to RGB.
pub(crate) fn hsb_hsl(src: &Image, out: Rect, ctx: &Ctx, input: HsbModel, output: HsbModel) -> Vec<f32> {
    let n = src.ch;
    let mut res = src.crop(out);
    if input == output {
        return res;
    }
    let native = ctx.mode == ColorMode::Rgb;
    for px in res.chunks_exact_mut(n) {
        let c4 = if native { [px[0], px[1], px[2], 1.0] } else { rgba(ctx, px) };
        let c = [c4[0].clamp(0.0, 1.0), c4[1].clamp(0.0, 1.0), c4[2].clamp(0.0, 1.0)];
        let rgb = match input {
            HsbModel::Rgb => c,
            HsbModel::Hsb => hsb_to_rgb(c),
            HsbModel::Hsl => hsl_to_rgb(c),
        };
        let o = match output {
            HsbModel::Rgb => rgb,
            HsbModel::Hsb => rgb_to_hsb(rgb),
            HsbModel::Hsl => rgb_to_hsl(rgb),
        };
        if native {
            px[..3].copy_from_slice(&o);
        } else {
            set_rgba(ctx, px, [o[0], o[1], o[2], c4[3]]);
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{PixelFormat, SampleType};
    use photocraft_raster::Surface;

    const DEPTHS: [SampleType; 3] = [SampleType::U8, SampleType::U16, SampleType::F32];

    fn custom_fixture(depth: SampleType, mode: ColorMode, alpha: bool) -> (Surface, Rect) {
        let b = Rect::new(-2, -1, 3, 4);
        let format = PixelFormat::new(mode, depth, alpha);
        let mut src = Surface::new(format);
        let mut pixels = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                for c in 0..format.channels() {
                    pixels.push(((x - b.x0) + 5 * (y - b.y0) + 3 * c as i32) as f32 / 40.0);
                }
            }
        }
        src.write_region(b, &pixels);
        (src, b)
    }

    fn assert_sample(actual: f32, expected: f32, depth: SampleType) {
        let (expected, tolerance) = match depth {
            SampleType::U8 => (expected.clamp(0.0, 1.0), 1.0 / 255.0),
            SampleType::U16 => (expected.clamp(0.0, 1.0), 1.0 / 65535.0),
            SampleType::F32 => (expected, 1e-6),
        };
        assert!((actual - expected).abs() <= tolerance, "{depth:?}: {actual} != {expected}");
    }

    fn run_custom(src: &Surface, b: Rect, kernel: Vec<f32>, scale: f32, offset: f32) -> Surface {
        crate::apply_in(src, &crate::FilterParams::Custom { kernel, scale, offset }, b, b, None, b)
    }

    #[test]
    fn custom_all_25_taps_use_row_major_offsets_and_repeat_canvas_edges() {
        for depth in DEPTHS {
            let (src, b) = custom_fixture(depth, ColorMode::Rgb, true);
            for tap in 0..25 {
                let mut kernel = vec![0.0; 25];
                kernel[tap] = 1.0;
                let out = run_custom(&src, b, kernel, 1.0, 0.0);
                let (dx, dy) = (tap as i32 % 5 - 2, tap as i32 / 5 - 2);
                for y in b.y0..b.y1 {
                    for x in b.x0..b.x1 {
                        let expected = src.pixel((x + dx).clamp(b.x0, b.x1 - 1), (y + dy).clamp(b.y0, b.y1 - 1));
                        let actual = out.pixel(x, y);
                        for c in 0..3 {
                            assert_sample(actual[c], expected[c], depth);
                        }
                        assert_eq!(actual[3], src.pixel(x, y)[3], "tap={tap}, alpha must not shift");
                    }
                }
            }
        }
    }

    #[test]
    fn custom_short_kernel_zero_pads_and_ignores_entries_after_25() {
        for depth in DEPTHS {
            let (src, b) = custom_fixture(depth, ColorMode::Rgb, true);
            let mut short = vec![0.0; 13];
            short[0] = 0.25;
            short[12] = 0.75;
            let mut padded = short.clone();
            padded.resize(25, 0.0);
            let mut long = padded.clone();
            long.extend([1e6, -1e6, f32::NAN]);
            let out = run_custom(&src, b, short, 1.0, 0.0);
            assert_eq!(out.read_region(b), run_custom(&src, b, padded, 1.0, 0.0).read_region(b));
            assert_eq!(out.read_region(b), run_custom(&src, b, long, 1.0, 0.0).read_region(b));
            for y in b.y0..b.y1 {
                for x in b.x0..b.x1 {
                    let corner = src.pixel((x - 2).max(b.x0), (y - 2).max(b.y0));
                    let centre = src.pixel(x, y);
                    let actual = out.pixel(x, y);
                    for c in 0..3 {
                        assert_sample(actual[c], 0.25 * corner[c] + 0.75 * centre[c], depth);
                    }
                }
            }
        }
    }

    #[test]
    fn custom_scale_zero_is_one_and_offset_is_in_255ths() {
        for depth in DEPTHS {
            let (src, b) = custom_fixture(depth, ColorMode::Rgb, true);
            let before = src.read_region(b);
            let mut kernel = vec![0.0; 25];
            kernel[12] = 2.0;
            let out = run_custom(&src, b, kernel.clone(), 0.0, -51.0);
            assert_eq!(out.read_region(b), run_custom(&src, b, kernel, 1.0, -51.0).read_region(b));
            for y in b.y0..b.y1 {
                for x in b.x0..b.x1 {
                    let original = src.pixel(x, y);
                    let actual = out.pixel(x, y);
                    for c in 0..3 {
                        assert_sample(actual[c], 2.0 * original[c] - 0.2, depth);
                    }
                    assert_eq!(actual[3], original[3]);
                }
            }
            assert_eq!(src.read_region(b), before);
        }
    }

    #[test]
    fn custom_neighbourhood_convolution_preserves_each_pixels_alpha() {
        for depth in DEPTHS {
            for mode in [ColorMode::Rgb, ColorMode::Cmyk] {
                let (src, b) = custom_fixture(depth, mode, true);
                let out = run_custom(&src, b, vec![1.0; 25], 25.0, 0.0);
                let alpha = src.channels() - 1;
                assert_ne!(out.pixel(b.x0, b.y0)[0], src.pixel(b.x0, b.y0)[0], "fixture must change colour");
                for y in b.y0..b.y1 {
                    for x in b.x0..b.x1 {
                        assert_eq!(out.pixel(x, y)[alpha], src.pixel(x, y)[alpha], "{depth:?}, {mode:?}, {x},{y}");
                    }
                }
            }
        }
    }

    #[test]
    fn custom_cmyk_convolves_brightness_including_black_with_or_without_alpha() {
        for depth in DEPTHS {
            for alpha in [false, true] {
                let (src, b) = custom_fixture(depth, ColorMode::Cmyk, alpha);
                for offset in [-51.0, 51.0] {
                    let mut kernel = vec![0.0; 25];
                    kernel[12] = 2.0;
                    let out = run_custom(&src, b, kernel, 1.0, offset);
                    for y in b.y0..b.y1 {
                        for x in b.x0..b.x1 {
                            let original = src.pixel(x, y);
                            let actual = out.pixel(x, y);
                            for c in 0..4 {
                                // Twice the reflected brightness, plus offset, then back to ink.
                                assert_sample(actual[c], 2.0 * original[c] - 1.0 - offset / 255.0, depth);
                            }
                            if alpha {
                                assert_eq!(actual[4], original[4]);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn custom_float_retains_hdr_while_integer_storage_clamps() {
        let b = Rect::new(0, 0, 1, 1);
        for depth in DEPTHS {
            let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            src.fill_rect(b, &[0.0, 0.5, 1.0, 0.4]);
            let mut kernel = vec![0.0; 25];
            kernel[12] = 2.0;
            let out = run_custom(&src, b, kernel, 1.0, -51.0);
            let actual = out.pixel(0, 0);
            let original = src.pixel(0, 0);
            for c in 0..3 {
                assert_sample(actual[c], 2.0 * original[c] - 0.2, depth);
            }
            assert_eq!(actual[3], original[3]);
            if depth == SampleType::F32 {
                assert!(actual[0] < 0.0 && actual[2] > 1.0);
            } else {
                assert_eq!((actual[0], actual[2]), (0.0, 1.0));
            }
        }
    }

    #[test]
    fn hsb_hsl_round_trip() {
        for c in [[0.2, 0.7, 0.4], [1.0, 0.0, 0.0], [0.5, 0.5, 0.5], [0.1, 0.2, 0.9], [0.9, 0.8, 0.05]] {
            let a = hsb_to_rgb(rgb_to_hsb(c));
            let b = hsl_to_rgb(rgb_to_hsl(c));
            for k in 0..3 {
                assert!((a[k] - c[k]).abs() < 1e-5 && (b[k] - c[k]).abs() < 1e-5, "{c:?} {a:?} {b:?}");
            }
        }
        assert_eq!(rgb_to_hsb([1.0, 0.0, 0.0]), [0.0, 1.0, 1.0]);
        let l = rgb_to_hsl([0.0, 0.0, 1.0]);
        assert!((l[0] - 2.0 / 3.0).abs() < 1e-6 && (l[2] - 0.5).abs() < 1e-6);
    }
}
