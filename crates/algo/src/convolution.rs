//! Native5×5 channel-selective convolution with explicit edge and alpha support rules.
pub use crate::coverage::Error;
use crate::{Ctx, image::Image};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Edge {
    Transparent,
    Repeat,
    Wrap,
    Black,
    White,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Params {
    pub kernel: Vec<f64>,
    pub scale: f64,
    pub offset: f64,
    pub channels: Vec<bool>,
    pub alpha_weighted: bool,
    pub edge: Edge,
}
/// Require a finite 5-by-5 kernel, nonzero divisor and channel mask matching the actual native format.
pub fn validate(p: &Params, channels: usize) -> Result<(), Error> {
    if channels == 0
        || channels > 5
        || p.channels.len() != channels
        || p.kernel.len() != 25
        || p.kernel.iter().any(|v| !v.is_finite())
        || !p.scale.is_finite()
        || p.scale == 0.
        || !p.offset.is_finite()
        || !p.kernel.iter().map(|v| v.abs()).sum::<f64>().is_finite()
    {
        return Err(Error::InvalidParameter);
    }
    Ok(())
}
/// Bound positive convolution geometry using widened arithmetic before deriving buffer sizes.
fn dimensions(r: Rect) -> Result<(usize, usize), Error> {
    let w = i64::from(r.x1) - i64::from(r.x0);
    let h = i64::from(r.y1) - i64::from(r.y0);
    if w <= 0 || h <= 0 || w > 65536 || h > 65536 || w.checked_mul(h).is_none_or(|n| n > 64 * 1024 * 1024) {
        return Err(Error::InvalidGeometry);
    }
    Ok((w as usize, h as usize))
}
/// Admit padded output tiles, map clones and fixed halo scratch under the existing native workspace ceiling.
fn preflight(s: &Surface, area: Rect, selection: Option<&Surface>) -> Result<(), Error> {
    dimensions(area)?;
    let tiles = |a: i32, b: i32| -> Result<usize, Error> {
        usize::try_from((i64::from(b) - 1).div_euclid(256) - i64::from(a).div_euclid(256) + 1).map_err(|_| Error::WorkspaceLimit)
    };
    let bytes = tiles(area.x0, area.x1)?
        .checked_mul(tiles(area.y0, area.y1)?)
        .and_then(|n| n.checked_mul(256 * 256 * s.format().bytes_per_pixel() + 128))
        .and_then(|n| n.checked_add(s.tile_count().checked_mul(128)?))
        .and_then(|n| n.checked_add(68 * 68 * s.channels() * 4 + 64 * 64 * (s.channels() * 3 + selection.map_or(0, Surface::channels)) * 4))
        .ok_or(Error::WorkspaceLimit)?;
    if bytes > crate::coverage::MAX_WORKSPACE_BYTES {
        return Err(Error::WorkspaceLimit);
    }
    Ok(())
}
/// Fill black or white edge samples in the native color model, with opaque alpha when present.
fn neutral(mode: ColorMode, white: bool, out: &mut [f32], alpha: bool) {
    let color_count = out.len().saturating_sub(usize::from(alpha));
    let value = if white { 1. } else { 0. };
    for (c, v) in out.iter_mut().take(color_count).enumerate() {
        *v = match mode {
            ColorMode::Cmyk => {
                if c == 3 {
                    1. - value
                } else {
                    0.
                }
            }
            ColorMode::Lab => {
                if c == 0 {
                    value
                } else {
                    128. / 255.
                }
            }
            _ => value,
        };
    }
    if alpha && let Some(v) = out.last_mut() {
        *v = 1.;
    }
}
/// Sample the requested native edge policy, distinguishing repeated/wrapped coordinates from transparent or neutral
/// borders.
fn sample(s: &Surface, x: i64, y: i64, domain: Rect, edge: Edge, out: &mut [f32]) -> Result<(), Error> {
    let inside = x >= i64::from(domain.x0) && x < i64::from(domain.x1) && y >= i64::from(domain.y0) && y < i64::from(domain.y1);
    if inside {
        s.read_pixel(x as i32, y as i32, out);
        return Ok(());
    }
    let axis = |v: i64, lo: i32, hi: i32| match edge {
        Edge::Repeat => v.clamp(i64::from(lo), i64::from(hi) - 1),
        Edge::Wrap => i64::from(lo) + (v - i64::from(lo)).rem_euclid(i64::from(hi) - i64::from(lo)),
        _ => v,
    };
    match edge {
        Edge::Repeat | Edge::Wrap => s.read_pixel(axis(x, domain.x0, domain.x1) as i32, axis(y, domain.y0, domain.y1) as i32, out),
        Edge::Transparent => out.fill(0.),
        Edge::Black => neutral(s.format().mode, false, out, s.format().alpha),
        Edge::White => neutral(s.format().mode, true, out, s.format().alpha),
    };
    Ok(())
}
/// Apply selected kernel channels with explicit alpha support, finite-value checks and row cancellation.
fn convolve(data: &[f32], w: usize, h: usize, format: PixelFormat, p: &Params, ctl: &Interrupt<'_>) -> Result<Vec<f32>, Error> {
    let ch = format.channels();
    let stride = (w + 4) * ch;
    if data.len() != (w + 4) * (h + 4) * ch {
        return Err(Error::InvalidGeometry);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(w * h * ch).map_err(|_| Error::Allocation)?;
    let weight_sum = p.kernel.iter().map(|v| v.abs()).sum::<f64>();
    for y in 0..h {
        if ctl.cancelled() {
            return Err(Error::Cancelled);
        }
        for x in 0..w {
            let base = (y + 2) * stride + (x + 2) * ch;
            let original = data.get(base..base + ch).ok_or(Error::InvalidGeometry)?;
            for (c, &before) in original.iter().enumerate() {
                if !before.is_finite() {
                    return Err(Error::NonFiniteSample);
                }
                if !p.channels.get(c).copied().ok_or(Error::InvalidParameter)? {
                    out.push(before);
                    continue;
                }
                let alpha = format.alpha && c + 1 == ch;
                let subtractive = format.mode == ColorMode::Cmyk && !alpha;
                let weighted = p.alpha_weighted && format.alpha && !alpha;
                let mut sum = 0f64;
                let mut support = 0f64;
                for (k, &coefficient) in p.kernel.iter().enumerate() {
                    if coefficient == 0. {
                        continue;
                    }
                    let i = (y + k / 5) * stride + (x + k % 5) * ch;
                    let pixel = data.get(i..i + ch).ok_or(Error::InvalidGeometry)?;
                    let mut value = f64::from(*pixel.get(c).ok_or(Error::InvalidGeometry)?);
                    if !value.is_finite() {
                        return Err(Error::NonFiniteSample);
                    }
                    if subtractive {
                        value = 1. - value;
                    }
                    let a = if weighted {
                        let a = f64::from(*pixel.last().ok_or(Error::InvalidGeometry)?);
                        if !a.is_finite() {
                            return Err(Error::NonFiniteSample);
                        }
                        a.clamp(0., 1.)
                    } else {
                        1.
                    };
                    sum += coefficient * value * a;
                    support += coefficient.abs() * a;
                }
                if !sum.is_finite() || !support.is_finite() {
                    return Err(Error::NonFiniteSample);
                }
                // Absolute support avoids division by signed/zero convolved alpha. Empty
                // support retains hidden color; a zero kernel deliberately produces offset.
                let result = if weighted && weight_sum > 0. && support <= weight_sum * 1e-12 {
                    f64::from(before)
                } else {
                    if weighted && weight_sum > 0. {
                        sum *= weight_sum / support;
                    }
                    let result = sum / p.scale + p.offset;
                    if subtractive { 1. - result } else { result }
                };
                if !result.is_finite() || result.abs() > f64::from(f32::MAX) {
                    return Err(Error::NonFiniteSample);
                }
                out.push(if alpha { result.clamp(0., 1.) as f32 } else { result as f32 });
            }
        }
    }
    Ok(out)
}
/// Selection changes publication only; the complete owner supplies the stencil.
pub fn apply_with(s: &Surface, p: &Params, area: Rect, domain: Rect, selection: Option<&Surface>, ctl: &Interrupt<'_>) -> Result<Surface, Error> {
    validate(p, s.channels())?;
    // Edge sampling does not allocate the domain; preflight bounds the output area.
    if domain.is_empty() {
        return Err(Error::InvalidGeometry);
    }
    if ctl.cancelled() {
        return Err(Error::Cancelled);
    }
    if area.x1 < area.x0 || area.y1 < area.y0 {
        return Err(Error::InvalidGeometry);
    }
    let area = area.intersect(&domain);
    if area.is_empty() {
        return Ok(s.clone());
    }
    preflight(s, area, selection)?;
    let ch = s.channels();
    let mut result = s.clone();
    let mut y = area.y0;
    while y < area.y1 {
        let y1 = (i64::from(y) + 64).min(i64::from(area.y1)) as i32;
        let mut x = area.x0;
        while x < area.x1 {
            if ctl.cancelled() {
                return Err(Error::Cancelled);
            }
            let x1 = (i64::from(x) + 64).min(i64::from(area.x1)) as i32;
            let tile = Rect::new(x, y, x1, y1);
            let (w, h) = dimensions(tile)?;
            let mut data = Vec::new();
            data.try_reserve_exact((w + 4) * (h + 4) * ch).map_err(|_| Error::Allocation)?;
            for yy in i64::from(y) - 2..i64::from(y1) + 2 {
                if ctl.cancelled() {
                    return Err(Error::Cancelled);
                }
                for xx in i64::from(x) - 2..i64::from(x1) + 2 {
                    let mut pixel = [0.; 5];
                    let target = pixel.get_mut(..ch).ok_or(Error::InvalidGeometry)?;
                    sample(s, xx, yy, domain, p.edge, target)?;
                    data.extend_from_slice(target);
                }
            }
            let mut filtered = convolve(&data, w, h, s.format(), p, ctl)?;
            drop(data);
            if let Some(selection) = selection {
                let mask = selection.try_read_region(tile).map_err(|_| Error::Allocation)?;
                if mask.iter().any(|v| !v.is_finite()) {
                    return Err(Error::NonFiniteSample);
                }
                let original = Image { rect: tile, ch, data: s.try_read_region(tile).map_err(|_| Error::Allocation)? };
                crate::mix_selection(&mut filtered, tile, selection, &original, s.format().alpha);
                for (pixel, before) in filtered.chunks_exact_mut(ch).zip(original.data.chunks_exact(ch)) {
                    for ((value, original), selected) in pixel.iter_mut().zip(before).zip(&p.channels) {
                        if !selected {
                            *value = *original;
                        }
                    }
                }
            }
            if filtered.iter().any(|v| !v.is_finite()) {
                return Err(Error::NonFiniteSample);
            }
            result.try_write_region(tile, &filtered).map_err(|_| Error::Allocation)?;
            x = x1;
        }
        y = y1;
        ctl.progress((i64::from(y) - i64::from(area.y0)) as f32 / (i64::from(area.y1) - i64::from(area.y0)) as f32);
    }
    if ctl.cancelled() {
        return Err(Error::Cancelled);
    }
    Ok(result)
}
/// Bridge raw float images to checked convolution and use bounded identity output when admission fails.
pub(crate) fn run_image(src: &Image, out: Rect, ctx: &Ctx, p: &Params) -> Vec<f32> {
    let identity = || -> Result<Vec<f32>, Error> {
        let (w, h) = dimensions(src.rect)?;
        let (ow, oh) = dimensions(out)?;
        if src.ch == 0 || src.ch > 5 || out.intersect(&src.rect) != out || w.checked_mul(h).and_then(|n| n.checked_mul(src.ch)) != Some(src.data.len()) {
            return Err(Error::InvalidGeometry);
        }
        let mut result = Vec::new();
        result.try_reserve_exact(ow * oh * src.ch).map_err(|_| Error::Allocation)?;
        for y in out.y0..out.y1 {
            let i = ((i64::from(y) - i64::from(src.rect.y0)) as usize * w + (i64::from(out.x0) - i64::from(src.rect.x0)) as usize) * src.ch;
            result.extend_from_slice(src.data.get(i..i + ow * src.ch).ok_or(Error::InvalidGeometry)?);
        }
        Ok(result)
    };
    let run = || -> Result<Vec<f32>, Error> {
        validate(p, src.ch)?;
        let (w, h) = dimensions(src.rect)?;
        dimensions(out)?;
        if out.intersect(&src.rect) != out {
            return Err(Error::InvalidGeometry);
        }
        if src.data.len() > 64 * 1024 * 1024 || w.checked_mul(h).and_then(|n| n.checked_mul(src.ch)) != Some(src.data.len()) {
            return Err(Error::WorkspaceLimit);
        }
        let format = PixelFormat::new(ctx.mode, SampleType::F32, ctx.alpha);
        if format.channels() != src.ch {
            return Err(Error::InvalidGeometry);
        }
        let mut s = Surface::new(format);
        s.try_write_region(src.rect, &src.data).map_err(|_| Error::Allocation)?;
        apply_with(&s, p, out, src.rect, None, &Interrupt::NONE)?.try_read_region(out).map_err(|_| Error::Allocation)
    };
    run().unwrap_or_else(|_| identity().unwrap_or_default())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    /// Retain every convolution edge policy on a large owner without admitting oversized output work.
    fn small_convolution_area_in_large_domain_keeps_edge_sampling_and_work_bounds() {
        let area = Rect::new(0, 0, 1, 1);
        let mut s = Surface::new(PixelFormat::RGBA32F);
        s.fill_rect(area, &[0.2, 0.4, 0.6, 1.]);
        let domain = Rect::new(0, 0, 9_000, 9_000);
        let mut p = params(4);
        p.kernel[12] = 0.;
        p.kernel[11] = 1.;
        for edge in [Edge::Repeat, Edge::Wrap, Edge::Transparent, Edge::Black, Edge::White] {
            p.edge = edge;
            let got = apply_with(&s, &p, area, domain, None, &Interrupt::NONE).unwrap();
            let mut expected = [0.; 4];
            sample(&s, -1, 0, domain, edge, &mut expected).unwrap();
            assert_eq!(got.pixel(0, 0), expected, "{edge:?}");
        }
        assert_eq!(apply_with(&s, &p, domain, domain, None, &Interrupt::NONE), Err(Error::InvalidGeometry));
        assert_eq!(apply_with(&s, &p, area, Rect::new(0, 0, 0, 1), None, &Interrupt::NONE), Err(Error::InvalidGeometry));
    }
    /// Define the native custom-filter fixture kernel independently of imported source configuration.
    fn params(ch: usize) -> Params {
        let mut k = vec![0.; 25];
        k[12] = 1.;
        Params { kernel: k, scale: 1., offset: 0., channels: vec![true; ch], alpha_weighted: false, edge: Edge::Repeat }
    }
    /// Populate a native-format test surface with original samples for cross-depth counterpart comparisons.
    fn surface(mode: ColorMode, depth: SampleType, r: Rect) -> Surface {
        let fmt = PixelFormat::new(mode, depth, true);
        let mut s = Surface::new(fmt);
        let data: Vec<f32> = (0..r.width() as usize * r.height() as usize * fmt.channels())
            .map(|i| if depth == SampleType::F32 { (i % 31) as f32 / 9. - 1. } else { (i % 31) as f32 / 30. })
            .collect();
        s.try_write_region(r, &data).unwrap();
        s
    }
    #[test]
    /// Compare the checked kernel with the existing native Custom filter's row-major brightness policy.
    fn native_custom_counterpart_matches_row_major_brightness_at_every_depth_and_model() {
        let r = Rect::new(-3, -3, 10, 9);
        let out = Rect::new(0, 0, 7, 6);
        for mode in [
            ColorMode::Bitmap,
            ColorMode::Grayscale,
            ColorMode::Indexed,
            ColorMode::Rgb,
            ColorMode::Cmyk,
            ColorMode::Lab,
            ColorMode::Multichannel,
            ColorMode::Duotone,
        ] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let s = surface(mode, depth, r);
                let mut p = params(s.channels());
                p.kernel[12] = 2.;
                p.kernel[6] = -0.25;
                p.kernel[19] = 0.5;
                p.scale = 2.25;
                p.offset = 0.125;
                p.channels[s.channels() - 1] = false;
                let input = Image::read(&s, r);
                let kernel: Vec<f32> = p.kernel.iter().map(|v| *v as f32).collect();
                let expected = crate::other2::custom(&input, out, &Ctx { bounds: r, mode, alpha: true }, &kernel, p.scale as f32, p.offset as f32 * 255.);
                let mut native = Surface::new(s.format());
                native.try_write_region(out, &expected).unwrap();
                let got = apply_with(&s, &p, out, r, None, &Interrupt::NONE).unwrap().read_region(out);
                for (a, b) in got.iter().zip(native.read_region(out)) {
                    assert!((a - b).abs() < 2. / 65535., "{mode:?} {depth:?} {a}/{b}");
                }
            }
        }
    }
    #[test]
    /// Exercise signed kernels and missing alpha support without inventing nonfinite hidden HDR samples.
    fn signed_alpha_weighting_and_empty_support_preserve_finite_hidden_hdr() {
        let r = Rect::new(0, 0, 3, 1);
        let mut s = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true));
        s.try_write_region(r, &[1000., -1000., 2., 0., 4., -2., 0.5, 0.5, 8., 3., 0.25, 1.]).unwrap();
        let mut p = params(4);
        p.kernel[12] = 2.;
        p.kernel[11] = -1.;
        p.alpha_weighted = true;
        p.channels = vec![true, false, true, true];
        let got = apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap();
        assert_eq!(got.pixel(1, 0), vec![12., -2., 1.5, 1.]);
        s.fill_rect(r, &[-2., 3., 0.4, 0.]);
        let before = s.clone();
        let got = apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap();
        assert_eq!(got.read_region(r), before.read_region(r));
        p.kernel.fill(0.);
        p.offset = 0.25;
        let got = apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap();
        assert_eq!(got.pixel(1, 0), vec![0.25, 3., 0.25, 0.25]);
    }
    #[test]
    /// Check border policies, native neutral colors and actual scalar/alpha channel selections.
    fn explicit_edges_neutral_models_and_alpha_scalar_channel_masks() {
        let r = Rect::new(0, 0, 1, 1);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            let mut s = Surface::new(PixelFormat::new(mode, SampleType::F32, true));
            s.fill_rect(r, &vec![0.4; s.channels()]);
            let mut p = params(s.channels());
            p.kernel.fill(0.);
            p.kernel[11] = 1.;
            for edge in [Edge::Repeat, Edge::Wrap, Edge::Transparent, Edge::Black, Edge::White] {
                p.edge = edge;
                let got = apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap().pixel(0, 0);
                let mut expected = vec![0.; s.channels()];
                match edge {
                    Edge::Repeat | Edge::Wrap => expected.fill(0.4),
                    Edge::Transparent => {}
                    Edge::Black => neutral(mode, false, &mut expected, true),
                    Edge::White => neutral(mode, true, &mut expected, true),
                }
                assert_eq!(got, expected, "{mode:?} {edge:?}");
            }
        }
    }
    #[test]
    /// Verify tiled selection output while cancellation and numeric failure cannot modify the original input.
    fn tiling_selection_mid_cancel_and_numeric_failures_are_atomic() {
        let r = Rect::new(-2, 4, 132, 12);
        let s = surface(ColorMode::Rgb, SampleType::F32, r);
        let before = s.clone();
        let mut p = params(4);
        p.kernel[12] = 0.5;
        p.kernel[11] = 0.5;
        p.channels[3] = false;
        let got = apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap();
        let crop = Rect::new(60, 5, 68, 11);
        assert_eq!(apply_with(&s, &p, crop, r, None, &Interrupt::NONE).unwrap().read_region(crop), got.read_region(crop));
        let mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        assert_eq!(apply_with(&s, &p, r, r, Some(&mask), &Interrupt::NONE).unwrap(), s);
        let count = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 8;
        assert_eq!(apply_with(&s, &p, r, r, None, &Interrupt::cancel_only(&cancel)).unwrap_err(), Error::Cancelled);
        assert_eq!(s, before);
        p.scale = f64::MIN_POSITIVE;
        assert_eq!(apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap_err(), Error::NonFiniteSample);
    }
    #[test]
    /// Ensure opaque formats retain their real channel count rather than acquiring an invented alpha plane.
    fn opaque_native_models_use_actual_channel_counts_without_synthetic_alpha() {
        let r = Rect::new(0, 0, 3, 2);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let mut s = Surface::new(PixelFormat::new(mode, depth, false));
                let ch = s.channels();
                s.fill_rect(r, &vec![0.4; ch]);
                let mut p = params(ch);
                p.alpha_weighted = true;
                assert_eq!(apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap(), s);
                let rank =
                    crate::rank::Params { radius: 1, color_percentile: 17., alpha_percentile: 83., shape: crate::rank::Neighborhood::Disc, channels: None };
                assert_eq!(crate::rank::apply_with(&s, &rank, r, r, None, &Interrupt::NONE).unwrap(), s);
                let noise = crate::channel_noise::Params {
                    amounts: vec![0.; ch],
                    distribution: crate::Distribution::Uniform,
                    monochromatic: false,
                    relative: true,
                    seed: 71,
                };
                assert_eq!(crate::channel_noise::apply_with(&s, &noise, r, r, None, &Interrupt::NONE).unwrap(), s);
            }
        }
    }
    #[test]
    /// Reject malformed kernel shapes, masks and geometry before native convolution can allocate or publish work.
    fn strict_kernel_channel_and_geometry_admission() {
        let r = Rect::new(0, 0, 2, 2);
        let s = surface(ColorMode::Rgb, SampleType::F32, r);
        let mut p = params(4);
        for bad in [0., f64::NAN, f64::INFINITY] {
            p.scale = bad;
            assert!(validate(&p, 4).is_err());
        }
        p = params(4);
        p.channels.pop();
        assert!(validate(&p, 4).is_err());
        p = params(4);
        p.kernel[0] = f64::INFINITY;
        assert!(validate(&p, 4).is_err());
        p = params(4);
        p.kernel[0] = f64::MAX;
        p.kernel[1] = f64::MAX;
        assert!(validate(&p, 4).is_err());
        let huge = Rect::new(0, 0, 65536, 65536);
        assert_eq!(apply_with(&s, &params(4), huge, huge, None, &Interrupt::NONE).unwrap_err(), Error::InvalidGeometry);
    }
}
