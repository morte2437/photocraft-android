//! Native deterministic channel-wise additive or signal-relative noise.
pub use crate::coverage::Error;
use crate::{Ctx, Distribution, image::Image};
use photocraft_color::{PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Params {
    pub amounts: Vec<f32>,
    pub distribution: Distribution,
    pub monochromatic: bool,
    pub relative: bool,
    pub seed: u32,
}
/// Require one finite bounded amplitude per native channel and reject unsupported channel counts.
pub fn validate(p: &Params, channels: usize) -> Result<(), Error> {
    if channels == 0 || channels > 5 || p.amounts.len() != channels || p.amounts.iter().any(|v| !v.is_finite() || !(0.0..=400.).contains(v)) {
        return Err(Error::InvalidParameter);
    }
    Ok(())
}
/// Validate positive bounded extents with widened arithmetic before calculating a dense noise region.
fn dimensions(r: Rect) -> Result<(usize, usize), Error> {
    let w = i64::from(r.x1) - i64::from(r.x0);
    let h = i64::from(r.y1) - i64::from(r.y0);
    if w <= 0 || h <= 0 || w > 65536 || h > 65536 || w.checked_mul(h).is_none_or(|n| n > 64 * 1024 * 1024) {
        return Err(Error::InvalidGeometry);
    }
    Ok((w as usize, h as usize))
}
/// Admit padded output tiles, cloned sparse headers and selection scratch before noise allocation.
fn preflight(s: &Surface, area: Rect, selection: Option<&Surface>) -> Result<(), Error> {
    dimensions(area)?;
    let count = |lo: i32, hi: i32| -> Result<usize, Error> {
        usize::try_from((i64::from(hi) - 1).div_euclid(256) - i64::from(lo).div_euclid(256) + 1).map_err(|_| Error::WorkspaceLimit)
    };
    let bytes = count(area.x0, area.x1)?
        .checked_mul(count(area.y0, area.y1)?)
        .and_then(|n| n.checked_mul(256 * 256 * s.format().bytes_per_pixel() + 128))
        .and_then(|n| n.checked_add(s.tile_count().checked_mul(128)?))
        .and_then(|n| n.checked_add(64 * 64 * 4 * (s.channels() * 3 + selection.map_or(0, Surface::channels))))
        .ok_or(Error::WorkspaceLimit)?;
    if bytes > crate::coverage::MAX_WORKSPACE_BYTES {
        return Err(Error::WorkspaceLimit);
    }
    Ok(())
}
/// Apply coordinate-seeded native noise per channel, preserving zero-amplitude channels and rejecting nonfinite results
/// or cancellation.
fn pixels(data: &mut [f32], rect: Rect, channels: usize, alpha: bool, p: &Params, ctl: &Interrupt<'_>) -> Result<(), Error> {
    let (w, h) = dimensions(rect)?;
    if data.len() != w * h * channels {
        return Err(Error::InvalidGeometry);
    }
    for (i, pixel) in data.chunks_exact_mut(channels).enumerate() {
        if i.is_multiple_of(1024) && ctl.cancelled() {
            return Err(Error::Cancelled);
        }
        let x = (i64::from(rect.x0) + (i % w) as i64) as i32;
        let y = (i64::from(rect.y0) + (i / w) as i64) as i32;
        for (c, value) in pixel.iter_mut().enumerate() {
            if !value.is_finite() {
                return Err(Error::NonFiniteSample);
            }
            let amount = *p.amounts.get(c).ok_or(Error::InvalidParameter)?;
            if amount == 0. {
                continue;
            }
            let channel = if p.monochromatic { 0 } else { c as u32 };
            let amplitude = amount / 200.;
            let noise = match p.distribution {
                Distribution::Uniform => (crate::noise::hash01(x, y, channel, p.seed) - 0.5) * 2. * amplitude,
                Distribution::Gaussian => {
                    let u1 = crate::noise::hash01(x, y, channel * 2 + 101, p.seed).max(1e-7);
                    let u2 = crate::noise::hash01(x, y, channel * 2 + 102, p.seed);
                    (-2. * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos() * amplitude * 0.5
                }
            };
            let delta = if p.relative { f64::from(*value) * f64::from(noise) } else { f64::from(noise) };
            let result = f64::from(*value) + delta;
            if !result.is_finite() || result.abs() > f64::from(f32::MAX) {
                return Err(Error::NonFiniteSample);
            }
            *value = if alpha && c + 1 == channels { result.clamp(0., 1.) as f32 } else { result as f32 };
        }
    }
    Ok(())
}
/// Publish deterministic noise over the clipped domain with selection mixing, checked workspace and no source mutation
/// on failure.
pub fn apply_with(s: &Surface, p: &Params, area: Rect, domain: Rect, selection: Option<&Surface>, ctl: &Interrupt<'_>) -> Result<Surface, Error> {
    validate(p, s.channels())?;
    // The domain clips work; only the clipped publication area needs dense admission.
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
            let mut data = s.try_read_region(tile).map_err(|_| Error::Allocation)?;
            pixels(&mut data, tile, s.channels(), s.format().alpha, p, ctl)?;
            if let Some(selection) = selection {
                let mask = selection.try_read_region(tile).map_err(|_| Error::Allocation)?;
                if mask.iter().any(|v| !v.is_finite()) {
                    return Err(Error::NonFiniteSample);
                }
                let original = Image { rect: tile, ch: s.channels(), data: s.try_read_region(tile).map_err(|_| Error::Allocation)? };
                crate::mix_selection(&mut data, tile, selection, &original, s.format().alpha);
                for (pixel, before) in data.chunks_exact_mut(s.channels()).zip(original.data.chunks_exact(s.channels())) {
                    for ((value, original), amount) in pixel.iter_mut().zip(before).zip(&p.amounts) {
                        if *amount == 0. {
                            *value = *original;
                        }
                    }
                }
            }
            if data.iter().any(|v| !v.is_finite()) {
                return Err(Error::NonFiniteSample);
            }
            result.try_write_region(tile, &data).map_err(|_| Error::Allocation)?;
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
/// Copy a contained, structurally valid raw-image region for the infallible adapter's fallback.
fn identity(src: &Image, out: Rect) -> Result<Vec<f32>, Error> {
    let (w, h) = dimensions(src.rect)?;
    let (ow, oh) = dimensions(out)?;
    if src.ch == 0 || src.ch > 5 || out.intersect(&src.rect) != out || w.checked_mul(h).and_then(|n| n.checked_mul(src.ch)) != Some(src.data.len()) {
        return Err(Error::InvalidGeometry);
    }
    let count = ow
        .checked_mul(oh)
        .and_then(|n| n.checked_mul(src.ch))
        .filter(|n| n.checked_mul(4).is_some_and(|b| b <= crate::coverage::MAX_WORKSPACE_BYTES))
        .ok_or(Error::WorkspaceLimit)?;
    let mut output = Vec::new();
    output.try_reserve_exact(count).map_err(|_| Error::Allocation)?;
    for y in out.y0..out.y1 {
        let start = ((i64::from(y) - i64::from(src.rect.y0)) as usize * w + (i64::from(out.x0) - i64::from(src.rect.x0)) as usize) * src.ch;
        output.extend_from_slice(src.data.get(start..start + ow * src.ch).ok_or(Error::InvalidGeometry)?);
    }
    Ok(output)
}
/// Raw compatibility retains exact-shaped identity on a rejected operation. Checked
/// Surface callers propagate the error and never publish an unsuccessful edit.
pub(crate) fn run_image(src: &Image, out: Rect, ctx: &Ctx, p: &Params) -> Vec<f32> {
    let run = || -> Result<Vec<f32>, Error> {
        validate(p, src.ch)?;
        if PixelFormat::new(ctx.mode, SampleType::F32, ctx.alpha).channels() != src.ch {
            return Err(Error::InvalidGeometry);
        }
        let mut output = identity(src, out)?;
        pixels(&mut output, out, src.ch, ctx.alpha, p, &Interrupt::NONE)?;
        Ok(output)
    };
    run().unwrap_or_else(|_| identity(src, out).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::ColorMode;
    #[test]
    /// Admit a small noise edit on a large owner while retaining output-size and invalid-domain checks.
    fn small_noise_area_in_large_domain_keeps_work_bounds() {
        let area = Rect::new(11, 13, 13, 15);
        let s = surface(ColorMode::Rgb, SampleType::F32, area);
        let params = p(vec![40., 80., 0., 0.]);
        let domain = Rect::new(0, 0, 12_000, 8_000);
        let got = apply_with(&s, &params, area, domain, None, &Interrupt::NONE).unwrap();
        let expected = apply_with(&s, &params, area, area, None, &Interrupt::NONE).unwrap();
        assert_eq!(got, expected);
        assert_eq!(apply_with(&s, &params, domain, domain, None, &Interrupt::NONE), Err(Error::InvalidGeometry));
        assert_eq!(apply_with(&s, &params, area, Rect::new(1, 1, 0, 0), None, &Interrupt::NONE), Err(Error::InvalidGeometry));
    }
    /// Supply a reproducible uniform-noise configuration with independent channels and a fixed seed.
    fn p(amounts: Vec<f32>) -> Params {
        Params { amounts, distribution: Distribution::Uniform, monochromatic: false, relative: false, seed: 71 }
    }
    /// Build a half-valued native surface so noise amplitudes and unchanged channels have explicit expected baselines.
    fn surface(mode: ColorMode, depth: SampleType, r: Rect) -> Surface {
        let fmt = PixelFormat::new(mode, depth, true);
        let mut s = Surface::new(fmt);
        s.fill_rect(r, &vec![0.5; fmt.channels()]);
        s
    }
    #[test]
    /// Compare uniform/Gaussian output to the existing native noise function and separately bound asymmetric channel
    /// amplitudes.
    fn native_noise_counterpart_uniform_gaussian_and_independent_channel_amplitudes() {
        let r = Rect::new(-2, 5, 11, 12);
        let s = surface(ColorMode::Rgb, SampleType::F32, r);
        let src = Image::read(&s, r);
        let ctx = Ctx { bounds: r, mode: ColorMode::Rgb, alpha: true };
        for distribution in [Distribution::Uniform, Distribution::Gaussian] {
            for mono in [false, true] {
                let mut p = p(vec![40., 40., 40., 0.]);
                p.distribution = distribution;
                p.monochromatic = mono;
                let native = crate::noise::add(&src, r, &ctx, 40., distribution, mono, 71);
                let ours = apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap().read_region(r);
                for (a, b) in ours.iter().zip(native) {
                    assert!((a - b).abs() < 1e-7);
                }
            }
        }
        let p = p(vec![20., 40., 0., 80.]);
        let got = apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap().read_region(r);
        assert!(
            got.as_chunks::<4>().0.iter().all(|v| v[2] == 0.5 && (0.4..=0.6).contains(&v[0]) && (0.3..=0.7).contains(&v[1]) && (0.1..=0.9).contains(&v[3]))
        );
    }
    #[test]
    /// Check alpha-only color preservation across models/depths and signal-relative noise on signed HDR channels.
    fn all_native_models_depths_alpha_only_and_relative_hdr_are_explicit() {
        let r = Rect::new(0, 0, 8, 4);
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
                let ch = s.channels();
                let mut amounts = vec![0.; ch];
                amounts[ch - 1] = 400.;
                let got = apply_with(&s, &p(amounts), r, r, None, &Interrupt::NONE).unwrap().read_region(r);
                let original = s.read_region(r);
                for (a, b) in got.chunks_exact(ch).zip(original.chunks_exact(ch)) {
                    assert_eq!(&a[..ch - 1], &b[..ch - 1]);
                    assert!((0.0..=1.0).contains(&a[ch - 1]));
                }
            }
        }
        let mut s = surface(ColorMode::Rgb, SampleType::F32, r);
        s.fill_rect(r, &[-2., 3., 0., 0.5]);
        let mut p = p(vec![100., 100., 100., 0.]);
        p.relative = true;
        let got = apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap();
        for v in got.read_region(r).as_chunks::<4>().0 {
            assert_eq!(v[2], 0.);
            assert_eq!(v[3], 0.5);
            assert!((-3.0..=-1.0).contains(&v[0]));
            assert!((1.5..=4.5).contains(&v[1]));
        }
    }
    #[test]
    /// Require crop-independent seeded output, zero-selection identity and unchanged sources after cancellation or
    /// nonfinite overflow.
    fn coordinates_tiling_selection_cancellation_and_overflow_are_atomic() {
        let r = Rect::new(-3, 5, 130, 12);
        let s = surface(ColorMode::Rgb, SampleType::F32, r);
        let before = s.clone();
        let p = p(vec![40., 80., 10., 20.]);
        let all = apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap();
        let crop = Rect::new(61, 6, 68, 11);
        assert_eq!(apply_with(&s, &p, crop, r, None, &Interrupt::NONE).unwrap().read_region(crop), all.read_region(crop));
        let mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        assert_eq!(apply_with(&s, &p, r, r, Some(&mask), &Interrupt::NONE).unwrap(), s);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 4;
        assert_eq!(apply_with(&s, &p, r, r, None, &Interrupt::cancel_only(&cancel)).unwrap_err(), Error::Cancelled);
        assert_eq!(s, before);
        let mut huge = s.clone();
        huge.fill_rect(r, &[f32::MAX, f32::MAX, f32::MAX, 0.5]);
        let mut p = p.clone();
        p.relative = true;
        p.amounts = vec![400.; 4];
        assert_eq!(apply_with(&huge, &p, r, r, None, &Interrupt::NONE).unwrap_err(), Error::NonFiniteSample);
    }
    #[test]
    /// Reject empty, mismatched, excessive and nonfinite amplitude arrays before native noise processing.
    fn strict_shape_and_numeric_validation() {
        for amounts in [vec![], vec![0.; 6], vec![f32::NAN; 4], vec![401.; 4], vec![-1.; 4]] {
            assert!(validate(&p(amounts), 4).is_err());
        }
        let s = surface(ColorMode::Rgb, SampleType::F32, Rect::new(0, 0, 1, 1));
        assert_eq!(apply_with(&s, &p(vec![0.; 3]), Rect::new(0, 0, 1, 1), Rect::new(0, 0, 1, 1), None, &Interrupt::NONE).unwrap_err(), Error::InvalidParameter);
    }
}
