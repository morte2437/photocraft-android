//! Filter › Video: De-Interlace and NTSC Colors.

use photocraft_color::ColorMode;
use photocraft_geom::Rect;

use crate::Ctx;
use crate::fxutil::{MAXC, rgba, set_rgba, xy};
use crate::image::Image;

/// De-Interlace: removes one field (odd = lines 1, 3, 5… counted from the
/// bounds' top, i.e. even offsets) and rebuilds it from the neighbouring
/// kept lines by duplication or (premultiplied) interpolation.
pub(crate) fn deinterlace(src: &Image, out: Rect, ctx: &Ctx, eliminate_even: bool, interpolate: bool) -> Vec<f32> {
    let n = src.ch;
    let b = ctx.bounds;
    let mut res = src.crop(out);
    let mut acc = [0.0f32; MAXC];
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        if !b.contains(x, y) {
            continue;
        }
        // 1-based line number parity: offset 0 is line 1 (odd).
        let odd_line = (y - b.y0) % 2 == 0;
        if odd_line == eliminate_even {
            continue;
        }
        let above = (y > b.y0).then_some(y - 1);
        let below = (y + 1 < b.y1).then_some(y + 1);
        let (rows, cnt) = match (interpolate, above, below) {
            (true, Some(a), Some(c)) => ([a, c], 2),
            (_, Some(a), _) => ([a, a], 1),
            (_, None, Some(c)) => ([c, c], 1),
            _ => continue,
        };
        acc[..n].fill(0.0);
        let k = 1.0 / cnt as f32;
        for &yy in &rows[..cnt] {
            let a = if ctx.alpha { src.get(x, yy, n - 1) } else { 1.0 };
            for (c, v) in acc.iter_mut().enumerate().take(n) {
                let s = src.get(x, yy, c);
                *v += if ctx.alpha && c < n - 1 { s * a } else { s } * k;
            }
        }
        crate::fxutil::unpremul_px(&mut acc[..n], ctx.alpha);
        px.copy_from_slice(&acc[..n]);
    }
    res
}

/// Composite-signal limits (fractions of the 7.5–100 IRE luma range): broadcast-safe
/// is −20…120 IRE, i.e. Y ± C within these bounds.
const NTSC_MAX: f32 = (120.0 - 7.5) / 92.5;
const NTSC_MIN: f32 = (-20.0 - 7.5) / 92.5;

/// NTSC Colors: scales chroma (in YIQ) so the composite signal Y ± C stays
/// within broadcast limits; hue and luma are kept.
pub(crate) fn ntsc(src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    let n = src.ch;
    let native = ctx.mode == ColorMode::Rgb;
    let mut res = src.crop(out);
    for px in res.chunks_exact_mut(n) {
        let c = if native { [px[0], px[1], px[2], 1.0] } else { rgba(ctx, px) };
        let yy = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
        let ii = 0.595_716 * c[0] - 0.274_453 * c[1] - 0.321_263 * c[2];
        let qq = 0.211_456 * c[0] - 0.522_591 * c[1] + 0.311_135 * c[2];
        let chroma = (ii * ii + qq * qq).sqrt();
        if chroma <= 1e-7 {
            continue;
        }
        let mut k: f32 = 1.0;
        if yy + chroma > NTSC_MAX {
            k = k.min(((NTSC_MAX - yy) / chroma).max(0.0));
        }
        if yy - chroma < NTSC_MIN {
            k = k.min(((yy - NTSC_MIN) / chroma).max(0.0));
        }
        if k >= 1.0 {
            continue;
        }
        let (i2, q2) = (ii * k, qq * k);
        let rgb = [yy + 0.956_3 * i2 + 0.621_0 * q2, yy - 0.272_1 * i2 - 0.647_4 * q2, yy - 1.107_0 * i2 + 1.704_6 * q2];
        if native {
            px[..3].copy_from_slice(&rgb);
        } else {
            set_rgba(ctx, px, [rgb[0], rgb[1], rgb[2], c[3]]);
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FilterParams, apply};
    use photocraft_color::{PixelFormat, SampleType};
    use photocraft_raster::Surface;

    const DEPTHS: [SampleType; 3] = [SampleType::U8, SampleType::U16, SampleType::F32];

    fn tolerance(depth: SampleType) -> f32 {
        match depth {
            SampleType::U8 => 1.0 / 255.0,
            SampleType::U16 => 1.0 / 65535.0,
            SampleType::F32 => 1e-6,
        }
    }

    fn rows(depth: SampleType, bounds: Rect, pixels: &[[f32; 4]]) -> Surface {
        let mut src = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
        for (row, px) in pixels.iter().enumerate() {
            let y = bounds.y0 + row as i32;
            src.fill_rect(Rect::new(bounds.x0, y, bounds.x1, y + 1), px);
        }
        src
    }

    #[test]
    fn deinterlace_all_field_and_interpolation_choices_use_bounds_relative_rows() {
        // Odd document origin: parity is relative to the bounds, not absolute y.
        let b = Rect::new(-2, 11, 0, 16);
        let pixels = [0.0, 0.2, 0.6, 0.8, 1.0].map(|v| [v, 1.0 - v, v / 2.0, 1.0]);
        for depth in DEPTHS {
            let src = rows(depth, b, &pixels);
            let before = src.read_region(b);
            for (eliminate_even, interpolate, neighbours) in [
                (false, false, [(1, 1), (1, 1), (1, 1), (3, 3), (3, 3)]),
                (false, true, [(1, 1), (1, 1), (1, 3), (3, 3), (3, 3)]),
                (true, false, [(0, 0), (0, 0), (2, 2), (2, 2), (4, 4)]),
                (true, true, [(0, 0), (0, 2), (2, 2), (2, 4), (4, 4)]),
            ] {
                let out = apply(&src, &FilterParams::DeInterlace { eliminate_even, interpolate }, b, b, None);
                for (row, (a, c)) in neighbours.into_iter().enumerate() {
                    for x in b.x0..b.x1 {
                        let (pa, pc) = (src.pixel(x, b.y0 + a), src.pixel(x, b.y0 + c));
                        let actual = out.pixel(x, b.y0 + row as i32);
                        for channel in 0..4 {
                            let expected = (pa[channel] + pc[channel]) / 2.0;
                            assert!(
                                (actual[channel] - expected).abs() <= tolerance(depth),
                                "{depth:?}, even={eliminate_even}, interp={interpolate}, row={row}: {actual:?}"
                            );
                        }
                    }
                }
            }
            assert_eq!(src.read_region(b), before);
        }
    }

    #[test]
    fn deinterlace_interpolation_weights_colour_by_alpha() {
        let b = Rect::new(0, 0, 1, 3);
        for depth in DEPTHS {
            for [a, c] in [[0.25, 0.75], [1.0, 0.0], [0.0, 0.0]] {
                let src = rows(depth, b, &[[1.0, 0.0, 0.0, a], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, c]]);
                let out = apply(&src, &FilterParams::DeInterlace { eliminate_even: true, interpolate: true }, b, b, None);
                let (a, c) = (src.pixel(0, 0)[3], src.pixel(0, 2)[3]);
                let total = a + c;
                let expected = if total > 0.0 { [a / total, 0.0, c / total, total / 2.0] } else { [0.0; 4] };
                let actual = out.pixel(0, 1);
                for (v, want) in actual.iter().zip(expected) {
                    assert!((v - want).abs() <= tolerance(depth), "{depth:?}: {actual:?} != {expected:?}");
                }
                assert_eq!(out.pixel(0, 0), src.pixel(0, 0));
                assert_eq!(out.pixel(0, 2), src.pixel(0, 2));
            }
        }
    }

    #[test]
    fn deinterlace_one_and_two_row_images_without_alpha_handle_missing_neighbours() {
        for depth in DEPTHS {
            for height in [1, 2] {
                let b = Rect::new(0, -3, 1, -3 + height);
                let mut src = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
                src.fill_rect(b, &[0.2]);
                if height == 2 {
                    src.fill_rect(Rect::new(0, -2, 1, -1), &[0.8]);
                }
                for eliminate_even in [false, true] {
                    for interpolate in [false, true] {
                        let out = apply(&src, &FilterParams::DeInterlace { eliminate_even, interpolate }, b, b, None);
                        let kept_y = if height == 2 && !eliminate_even { -2 } else { -3 };
                        for y in b.y0..b.y1 {
                            assert_eq!(out.pixel(0, y), src.pixel(0, kept_y), "{depth:?}, height={height}, even={eliminate_even}, interp={interpolate}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn deinterlace_leaves_pixels_outside_reference_bounds_unchanged() {
        let area = Rect::new(0, 0, 1, 5);
        let b = Rect::new(0, 1, 1, 4);
        let src = rows(SampleType::F32, area, &[[0.1; 4], [0.2; 4], [0.4; 4], [0.8; 4], [1.0; 4]]);
        for eliminate_even in [false, true] {
            for interpolate in [false, true] {
                let out = apply(&src, &FilterParams::DeInterlace { eliminate_even, interpolate }, area, b, None);
                assert_eq!(out.pixel(0, 0), src.pixel(0, 0));
                assert_eq!(out.pixel(0, 4), src.pixel(0, 4));
            }
        }
    }

    #[test]
    fn ntsc_safe_colours_and_alpha_are_bit_exact_at_all_depths() {
        let b = Rect::new(0, 0, 1, 5);
        for depth in DEPTHS {
            let src = rows(depth, b, &[[0.0, 0.0, 0.0, 0.0], [0.5, 0.5, 0.5, 0.25], [1.0, 1.0, 1.0, 0.75], [0.2, 0.3, 0.4, 1.0], [1.0, 0.0, 1.0, 1.0]]);
            let out = apply(&src, &FilterParams::NtscColors, b, b, None);
            assert_eq!(out.read_region(b), src.read_region(b), "{depth:?}");
        }
    }

    fn luma_chroma(p: &[f32]) -> (f32, f32) {
        let y = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
        let i = 0.595716 * p[0] - 0.274453 * p[1] - 0.321263 * p[2];
        let q = 0.211456 * p[0] - 0.522591 * p[1] + 0.311135 * p[2];
        (y, i.hypot(q))
    }

    #[test]
    fn ntsc_saturated_colours_keep_luma_alpha_and_meet_composite_limits() {
        // Independent signal checks: -20..120 IRE relative to the 7.5..100 IRE luma range.
        let (lo, hi) = ((-20.0 - 7.5) / 92.5, (120.0 - 7.5) / 92.5);
        let colours = [[1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.25], [1.0, 1.0, 0.0, 0.5], [0.0, 1.0, 1.0, 0.75]];
        let b = Rect::new(0, 0, 1, colours.len() as i32);
        for depth in DEPTHS {
            let src = rows(depth, b, &colours);
            let out = apply(&src, &FilterParams::NtscColors, b, b, None);
            // The rounded YIQ inverse contributes < 0.0003; integer writes add one sample step.
            let eps = 0.0003 + 2.0 * tolerance(depth);
            for y in b.y0..b.y1 {
                let (before, after) = (src.pixel(0, y), out.pixel(0, y));
                let (y0, c0) = luma_chroma(&before);
                let (y1, c1) = luma_chroma(&after);
                assert!(y0 - c0 < lo || y0 + c0 > hi, "fixture must require correction");
                assert!(c1 < c0, "chroma not reduced: {depth:?} {after:?}");
                assert!((y1 - y0).abs() <= eps, "luma changed: {depth:?} {before:?} -> {after:?}");
                assert!(y1 - c1 >= lo - eps && y1 + c1 <= hi + eps, "signal out of gamut: {depth:?} {after:?}");
                assert!(after[..3].iter().all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0));
                assert_eq!(after[3], before[3], "alpha changed: {depth:?}");
            }
        }
    }
}
