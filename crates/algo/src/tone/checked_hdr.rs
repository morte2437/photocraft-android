//! Checked replay of PhotoCraft's native local HDR adaptation. No format-specific controls.
use super::{HdrToning, curve_eval, luma};
use crate::fxutil::gauss_box_radii;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface, from_rgba_into, to_rgba};

pub const MAX_PIXELS: usize = 64 * 1024 * 1024;
pub const MAX_WORKSPACE_BYTES: usize = 1_500_000_000;
const MAX_EDGE: usize = 65_536;
const MAX_CURVE: usize = 4096;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    InvalidGeometry,
    InvalidParameter,
    NonFiniteSample,
    WorkspaceLimit,
    Allocation,
}
impl std::fmt::Display for Error {
    /// Render checked HDR geometry, parameter, sample, allocation and cancellation failures as actionable messages.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "native HDR cancelled",
            Self::InvalidGeometry => "invalid native HDR geometry",
            Self::InvalidParameter => "invalid native HDR parameter",
            Self::NonFiniteSample => "native HDR requires finite samples",
            Self::WorkspaceLimit => "native HDR exceeds bounded workspace",
            Self::Allocation => "native HDR workspace allocation failed",
        })
    }
}
impl std::error::Error for Error {}
/// Convert the shared interrupt check into this operation's explicit cancellation error.
fn check(ctl: &Interrupt<'_>) -> Result<(), Error> {
    ctl.check().map_err(|_| Error::Cancelled)
}
// Independent pixel phases use PhotoCraft's normal native rayon scheduling. No parallel
// reduction: the visibility-weighted mean retains its original deterministic serial order.
/// Process disjoint chunks in parallel only on sufficiently large native workloads; preserve chunk offsets and the
/// serial wasm path.
fn independent_chunks<T: Send>(values: &mut [T], f: impl Fn(usize, &mut [T]) -> Result<(), Error> + Sync + Send) -> Result<(), Error> {
    #[cfg(not(target_arch = "wasm32"))]
    if values.len() >= 65_536 {
        use rayon::prelude::*;
        return values.par_chunks_mut(2048).enumerate().try_for_each(|(i, q)| f(i * 2048, q));
    }
    values.chunks_mut(2048).enumerate().try_for_each(|(i, q)| f(i * 2048, q))
}
/// Allocate scratch fallibly and initialize it in cancellable bounded chunks.
fn zeroes<T: Clone>(count: usize, value: T, ctl: &Interrupt<'_>) -> Result<Vec<T>, Error> {
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(|_| Error::Allocation)?;
    while out.len() < count {
        check(ctl)?;
        out.resize(out.len().saturating_add(2048).min(count), value.clone());
    }
    Ok(out)
}
/// Refuse nonfinite or out-of-range native HDR controls and invalid bounded tone-curve points before processing.
pub fn validate(p: &HdrToning) -> Result<(), Error> {
    for (v, lo, hi) in [
        (p.radius, 1., 500.),
        (p.strength, 0.1, 4.),
        (p.gamma, 0.1, 2.),
        (p.exposure, -5., 5.),
        (p.detail, -100., 300.),
        (p.shadow, -100., 100.),
        (p.highlight, -100., 100.),
        (p.vibrance, -100., 100.),
        (p.saturation, -100., 100.),
    ] {
        if !v.is_finite() || v < lo || v > hi {
            return Err(Error::InvalidParameter);
        }
    }
    if p.curve.len() > MAX_CURVE || p.curve.iter().any(|(x, y)| !x.is_finite() || !y.is_finite() || !(0. ..=1.).contains(x) || !(0. ..=1.).contains(y)) {
        return Err(Error::InvalidParameter);
    }
    Ok(())
}
/// Admit only positive bounded edges and a checked pixel count within the native HDR workspace domain.
fn geometry(w: usize, h: usize) -> Result<usize, Error> {
    if w == 0 || h == 0 || w > MAX_EDGE || h > MAX_EDGE {
        return Err(Error::InvalidGeometry);
    }
    w.checked_mul(h).filter(|n| *n <= MAX_PIXELS).ok_or(Error::WorkspaceLimit)
}
// The same six running-sum box passes as fxutil::gauss_blur_n, with preallocated scratch
// and cooperative checks. f64 sums preserve the existing native arithmetic order.
/// Apply the existing native box-approximation blur with bounded row/column passes and interrupt checks.
fn blur(base: &mut [f32], w: usize, h: usize, scratch: &mut [f32], sigma: f32, ctl: &Interrupt<'_>) -> Result<(), Error> {
    for radius in gauss_box_radii(sigma) {
        let r = isize::try_from(radius).map_err(|_| Error::InvalidParameter)?;
        let norm = 1. / (2 * radius + 1) as f64;
        for row in base.chunks_exact_mut(w) {
            check(ctl)?;
            let tmp = scratch.get_mut(..w).ok_or(Error::InvalidGeometry)?;
            tmp.copy_from_slice(row);
            let at = |x: isize| -> f64 { f64::from(tmp.get(x.clamp(0, w as isize - 1) as usize).copied().unwrap_or(0.)) };
            let mut sum: f64 = (-r..=r).map(at).sum();
            for (x, v) in row.iter_mut().enumerate() {
                *v = (sum * norm) as f32;
                sum += at(x as isize + r + 1) - at(x as isize - r);
            }
        }
    }
    for radius in gauss_box_radii(sigma) {
        let r = isize::try_from(radius).map_err(|_| Error::InvalidParameter)?;
        let norm = 1. / (2 * radius + 1) as f64;
        for x in 0..w {
            check(ctl)?;
            let tmp = scratch.get_mut(..h).ok_or(Error::InvalidGeometry)?;
            for (y, v) in tmp.iter_mut().enumerate() {
                *v = *base.get(y * w + x).ok_or(Error::InvalidGeometry)?;
            }
            let at = |y: isize| -> f64 { f64::from(tmp.get(y.clamp(0, h as isize - 1) as usize).copied().unwrap_or(0.)) };
            let mut sum: f64 = (-r..=r).map(at).sum();
            for y in 0..h {
                *base.get_mut(y * w + x).ok_or(Error::InvalidGeometry)? = (sum * norm) as f32;
                sum += at(y as isize + r + 1) - at(y as isize - r);
            }
        }
    }
    Ok(())
}
/// The same native luminance coefficients in a domain that cannot overflow on finite f32 channels.
fn wide_luma(c: [f64; 3]) -> f64 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

/// The caller owns an unpublished work buffer; cancellation may leave that buffer partly processed.
pub fn process_with(px: &mut [[f32; 4]], w: usize, h: usize, p: &HdrToning, ctl: &Interrupt<'_>) -> Result<(), Error> {
    check(ctl)?;
    validate(p)?;
    let count = geometry(w, h)?;
    if count != px.len() {
        return Err(Error::InvalidGeometry);
    }
    if count.checked_mul(24).and_then(|v| v.checked_add(w.max(h) * 4)).and_then(|v| v.checked_add(p.curve.len() * 8)).is_none_or(|v| v > MAX_WORKSPACE_BYTES) {
        return Err(Error::WorkspaceLimit);
    }
    let mut logl = zeroes(count, 0f32, ctl)?;
    independent_chunks(&mut logl, |start, out| {
        check(ctl)?;
        let input = px.get(start..start + out.len()).ok_or(Error::InvalidGeometry)?;
        for (q, v) in input.iter().zip(out) {
            if q.iter().any(|v| !v.is_finite()) {
                return Err(Error::NonFiniteSample);
            }
            let luminance = luma([q[0], q[1], q[2]]);
            *v = if luminance.is_finite() {
                (luminance.max(0.) + 1e-4).ln()
            } else {
                (wide_luma([f64::from(q[0]), f64::from(q[1]), f64::from(q[2])]).max(0.) + 1e-4).ln() as f32
            };
        }
        Ok(())
    })?;
    let mut base = zeroes(count, 0f32, ctl)?;
    independent_chunks(&mut base, |start, dst| {
        check(ctl)?;
        let src = logl.get(start..start + dst.len()).ok_or(Error::InvalidGeometry)?;
        dst.copy_from_slice(src);
        Ok(())
    })?;
    let mut scratch = zeroes(w.max(h), 0f32, ctl)?;
    blur(&mut base, w, h, &mut scratch, p.radius / 2., ctl)?;
    // F32 loses successive log-luminance additions on ordinary megapixel images.
    let mut total = 0f64;
    let mut visible = 0usize;
    for (qs, bs) in px.chunks(2048).zip(base.chunks(2048)) {
        check(ctl)?;
        for (q, b) in qs.iter().zip(bs) {
            if q[3] > 0. {
                total += f64::from(*b);
                visible += 1;
            }
        }
    }
    let mean = (total / visible.max(1) as f64) as f32;
    let mut curve = zeroes(p.curve.len(), (0f32, 0f32), ctl)?;
    curve.copy_from_slice(&p.curve);
    // Stable bounded in-place insertion avoids slice::sort's infallible temporary allocation.
    for i in 1..curve.len() {
        check(ctl)?;
        let mut j = i;
        while j > 0 && curve.get(j - 1).zip(curve.get(j)).is_some_and(|(a, b)| a.0.total_cmp(&b.0).is_gt()) {
            curve.swap(j - 1, j);
            j -= 1;
        }
    }
    let compress = 1. / (1. + p.strength);
    let detail = 1. + p.detail / 100.;
    let exposure = 2f32.powf(p.exposure);
    independent_chunks(px, |start, qs| {
        check(ctl)?;
        let bs = base.get(start..start + qs.len()).ok_or(Error::InvalidGeometry)?;
        let ls = logl.get(start..start + qs.len()).ok_or(Error::InvalidGeometry)?;
        for ((q, b), l) in qs.iter_mut().zip(bs).zip(ls) {
            if q[3] <= 0. {
                continue;
            }
            let c = [q[0], q[1], q[2]];
            let l0 = luma(c).max(0.) + 1e-4;
            let new_log = mean + (*b - mean) * compress + (*l - *b) * detail;
            let mut l1 = (new_log.exp() * exposure).max(0.).powf(1. / p.gamma);
            l1 += p.shadow / 100. * (1. - l1).powi(3) * 0.5 - p.highlight / 100. * l1.powi(3) * 0.5;
            if !l1.is_finite() {
                // Preserve ordinary native arithmetic, but evaluate extreme valid HDR in a
                // wider domain. A cap beyond every f32 control's polynomial crossover keeps
                // cubing finite; those values already saturate at the final display clamp.
                let log_value = (f64::from(new_log) + f64::from(p.exposure) * std::f64::consts::LN_2) / f64::from(p.gamma);
                let value = log_value.exp().min(1e100);
                let shadow = f64::from(p.shadow) / 200.;
                let highlight = f64::from(p.highlight) / 200.;
                let mapped = ((-(shadow + highlight) * value + 3. * shadow) * value + (1. - 3. * shadow)) * value + shadow;
                l1 = mapped.clamp(0., 1.) as f32;
                if !l1.is_finite() {
                    return Err(Error::NonFiniteSample);
                }
            }
            l1 = curve_eval(&curve, l1.clamp(0., 1.));
            let mut out = c.map(|v| v * (l1 / l0));
            let ll = luma(out);
            let (mx, mn) = (out[0].max(out[1]).max(out[2]), out[0].min(out[1]).min(out[2]));
            let sat = if mx > 0. { (mx - mn) / mx } else { 0. };
            let s = 1. + p.saturation / 100. + p.vibrance / 100. * (1. - sat);
            out = out.map(|v| (ll + (v - ll) * s.max(0.)).clamp(0., 1.));
            if out.iter().any(|v| !v.is_finite()) {
                // Signed wide-gamut values can cancel to black luminance while individual
                // channels overflow the f32 rescale. Clamp only after the wide color correction.
                let source = c.map(f64::from);
                let scale = f64::from(l1) / (wide_luma(source).max(0.) + 1e-4);
                let wide = source.map(|value| value * scale);
                let luminance = wide_luma(wide);
                let (max, min) = (wide[0].max(wide[1]).max(wide[2]), wide[0].min(wide[1]).min(wide[2]));
                let saturation = if max > 0. { (max - min) / max } else { 0. };
                let strength = (1. + f64::from(p.saturation) / 100. + f64::from(p.vibrance) / 100. * (1. - saturation)).max(0.);
                out = wide.map(|value| (luminance + (value - luminance) * strength).clamp(0., 1.) as f32);
                if out.iter().any(|value| !value.is_finite()) {
                    return Err(Error::NonFiniteSample);
                }
            }
            q[..3].copy_from_slice(&out);
        }
        Ok(())
    })?;
    check(ctl)
}
/// Visit clipped 64-by-64 publication regions and propagate the first callback error.
fn tiles(r: Rect, mut f: impl FnMut(Rect) -> Result<(), Error>) -> Result<(), Error> {
    let mut y = r.y0;
    while y < r.y1 {
        let y1 = y.saturating_add(64).min(r.y1);
        let mut x = r.x0;
        while x < r.x1 {
            let x1 = x.saturating_add(64).min(r.x1);
            f(Rect::new(x, y, x1, y1))?;
            x = x1;
        }
        y = y1;
    }
    Ok(())
}
/// Whole-owner native replay; selection/area restrict publication, not the global adaptation domain.
pub fn apply_with(surface: &Surface, p: &HdrToning, area: Rect, domain: Rect, selection: Option<&Surface>, ctl: &Interrupt<'_>) -> Result<Surface, Error> {
    check(ctl)?;
    validate(p)?;
    if area.x1 < area.x0 || area.y1 < area.y0 {
        return Err(Error::InvalidGeometry);
    }
    let w = usize::try_from(i64::from(domain.x1) - i64::from(domain.x0)).map_err(|_| Error::InvalidGeometry)?;
    let h = usize::try_from(i64::from(domain.y1) - i64::from(domain.y0)).map_err(|_| Error::InvalidGeometry)?;
    let count = geometry(w, h)?;
    let area = area.intersect(&domain);
    let ts = i64::from(photocraft_geom::TILE_SIZE);
    let tile_count = |lo: i32, hi: i32| -> Option<usize> {
        if hi <= lo { Some(0) } else { usize::try_from((i64::from(hi) - 1).div_euclid(ts) - i64::from(lo).div_euclid(ts) + 1).ok() }
    };
    let output = tile_count(area.x0, area.x1)
        .and_then(|x| tile_count(area.y0, area.y1).and_then(|y| x.checked_mul(y)))
        .and_then(|n| n.checked_mul(ts as usize * ts as usize * surface.format().bytes_per_pixel() + 128))
        .ok_or(Error::WorkspaceLimit)?;
    let bytes = count
        .checked_mul(24)
        .and_then(|n| n.checked_add(w.max(h) * 4))
        .and_then(|n| n.checked_add(output))
        .and_then(|n| n.checked_add(surface.tile_count().checked_mul(128)?))
        .and_then(|n| n.checked_add(64 * 64 * 4 * (surface.channels() * 3 + selection.map_or(0, Surface::channels))))
        .and_then(|n| n.checked_add(MAX_CURVE * 8))
        .ok_or(Error::WorkspaceLimit)?;
    if bytes > MAX_WORKSPACE_BYTES {
        return Err(Error::WorkspaceLimit);
    }
    let mut px = zeroes(count, [0f32; 4], ctl)?;
    let fmt = surface.format();
    let channels = surface.channels();
    tiles(domain, |tile| {
        check(ctl)?;
        let raw = surface.try_read_region(tile).map_err(|_| Error::Allocation)?;
        let tw = tile.width() as usize;
        for (i, q) in raw.chunks_exact(channels).enumerate() {
            let x = (i64::from(tile.x0) - i64::from(domain.x0)) as usize + i % tw;
            let y = (i64::from(tile.y0) - i64::from(domain.y0)) as usize + i / tw;
            *px.get_mut(y * w + x).ok_or(Error::InvalidGeometry)? = to_rgba(&fmt, q);
        }
        Ok(())
    })?;
    process_with(&mut px, w, h, p, ctl)?;
    let mut output = surface.clone();
    tiles(area, |tile| {
        check(ctl)?;
        let original = crate::image::Image { rect: tile, ch: channels, data: surface.try_read_region(tile).map_err(|_| Error::Allocation)? };
        let mut raw = zeroes(original.data.len(), 0f32, ctl)?;
        let tw = tile.width() as usize;
        for (i, q) in raw.chunks_exact_mut(channels).enumerate() {
            let x = (i64::from(tile.x0) - i64::from(domain.x0)) as usize + i % tw;
            let y = (i64::from(tile.y0) - i64::from(domain.y0)) as usize + i / tw;
            from_rgba_into(&fmt, *px.get(y * w + x).ok_or(Error::InvalidGeometry)?, q);
        }
        if let Some(mask) = selection {
            let selected = mask.try_read_region(tile).map_err(|_| Error::Allocation)?;
            if selected.iter().any(|v| !v.is_finite()) {
                return Err(Error::NonFiniteSample);
            }
            crate::mix_selection(&mut raw, tile, mask, &original, fmt.alpha);
        }
        if fmt.alpha {
            // Toning skips these pixels. RGB conversion must not regenerate their
            // hidden native separations, including after selection publication.
            for (q, before) in raw.chunks_exact_mut(channels).zip(original.data.chunks_exact(channels)) {
                if before.last().is_some_and(|alpha| *alpha <= 0.) {
                    q.copy_from_slice(before);
                }
            }
        }
        output.try_write_region(tile, &raw).map_err(|_| Error::Allocation)
    })?;
    check(ctl)?;
    ctl.progress(1.);
    Ok(output)
}

/// Compatibility kernel. Checked Surface callers propagate errors instead of using this fallback.
pub(crate) fn run_image(src: &crate::Image, out: Rect, ctx: &crate::Ctx, p: &HdrToning) -> Vec<f32> {
    let run = || -> Result<Vec<f32>, Error> {
        let w = usize::try_from(i64::from(src.rect.x1) - i64::from(src.rect.x0)).map_err(|_| Error::InvalidGeometry)?;
        let h = usize::try_from(i64::from(src.rect.y1) - i64::from(src.rect.y0)).map_err(|_| Error::InvalidGeometry)?;
        let n = geometry(w, h)?;
        let fmt = photocraft_color::PixelFormat::new(ctx.mode, photocraft_color::SampleType::F32, ctx.alpha);
        if src.ch != fmt.channels() || n.checked_mul(src.ch) != Some(src.data.len()) || out.intersect(&src.rect) != out {
            return Err(Error::InvalidGeometry);
        }
        if n.checked_mul(24 + fmt.bytes_per_pixel() * 2).is_none_or(|v| v > MAX_WORKSPACE_BYTES) {
            return Err(Error::WorkspaceLimit);
        }
        let mut surface = Surface::new(fmt);
        surface.try_write_region(src.rect, &src.data).map_err(|_| Error::Allocation)?;
        let result = apply_with(&surface, p, out, src.rect, None, &Interrupt::NONE).unwrap_or(surface);
        result.try_read_region(out).map_err(|_| Error::Allocation)
    };
    run().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uniform ten-megapixel scenes retain their exposure regardless of the reduction length.
    #[test]
    fn megapixel_scene_exposure_matches_single_pixel_reference() {
        let p = HdrToning { radius: 1., detail: 0., saturation: 0., ..Default::default() };
        let sample = [0.18, 0.18, 0.18, 1.];
        let mut reference = [sample];
        process_with(&mut reference, 1, 1, &p, &Interrupt::NONE).unwrap();
        let mut pixels = vec![sample; 4000 * 2500];
        process_with(&mut pixels, 4000, 2500, &p, &Interrupt::NONE).unwrap();
        for pixel in pixels {
            for (value, expected) in pixel.into_iter().zip(reference[0]) {
                assert!((value - expected).abs() < 2e-6, "{value} versus {expected}");
            }
        }
    }
    #[test]
    /// Preserve skipped transparent CMYK and Lab samples bit for bit, with and without selection mixing.
    fn skipped_transparent_native_samples_survive_toning_and_selection_bit_for_bit() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let area = Rect::new(-1, 0, 1, 1);
        for mode in [ColorMode::Cmyk, ColorMode::Lab] {
            for depth in SampleType::ALL {
                let fmt = PixelFormat::new(mode, depth, true);
                let mut source = Surface::new(fmt);
                let mut pixel = vec![0.75; fmt.channels()];
                *pixel.last_mut().unwrap() = 0.;
                source.write_pixel(-1, 0, &pixel);
                *pixel.last_mut().unwrap() = if depth == SampleType::F32 { -0.5 } else { 0. };
                source.write_pixel(0, 0, &pixel);
                let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                mask.fill_rect(area, &[0.5]);
                for selection in [None, Some(&mask)] {
                    let result = apply_with(&source, &HdrToning::default(), area, area, selection, &Interrupt::NONE).unwrap();
                    assert_eq!(
                        result.read_region(area).iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                        source.read_region(area).iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                        "{fmt:?}"
                    );
                }
            }
        }
    }
    #[test]
    /// Compare checked HDR output to the existing native tone arithmetic, including unchanged alpha samples.
    fn matches_existing_native_arithmetic_and_alpha() {
        let mut input: Vec<_> = (0..72)
            .map(|i| {
                let t = i as f32 / 71.;
                [t * 3. - 0.1, t * 1.4, t * 0.6, if i % 4 == 0 { 0. } else { 0.4 }]
            })
            .collect();
        let mut expected = input.clone();
        let p = HdrToning { radius: 7., curve: vec![(0., 0.02), (0.6, 0.8), (1., 1.)], ..Default::default() };
        super::super::hdr_toning(&mut expected, 9, 8, &p);
        process_with(&mut input, 9, 8, &p, &Interrupt::NONE).unwrap();
        for (a, b) in input.iter().zip(&expected) {
            for (x, y) in a.iter().zip(b) {
                assert!((x - y).abs() < 1e-6, "{a:?} {b:?}");
            }
        }
    }
    #[test]
    /// Pin errors for mismatched geometry, cancellation, nonfinite pixels and controls before accepting native
    /// processing.
    fn hostile_and_cancelled_work_is_rejected() {
        let mut px = vec![[0.5; 4]; 4];
        assert_eq!(process_with(&mut px, 3, 2, &HdrToning::default(), &Interrupt::NONE), Err(Error::InvalidGeometry));
        let cancel = || true;
        assert_eq!(process_with(&mut px, 2, 2, &HdrToning::default(), &Interrupt::cancel_only(&cancel)), Err(Error::Cancelled));
        px[0][0] = f32::NAN;
        assert_eq!(process_with(&mut px, 2, 2, &HdrToning::default(), &Interrupt::NONE), Err(Error::NonFiniteSample));
        assert_eq!(validate(&HdrToning { radius: f32::INFINITY, ..Default::default() }), Err(Error::InvalidParameter));
        assert!(geometry(65_536, 65_536).is_err());
    }
    #[test]
    /// Compare a parallel-sized workload exactly with the existing native counterpart to guard arithmetic and
    /// reduction-order drift.
    fn native_parallel_chunks_keep_exact_counterpart_and_reduction_order() {
        let mut actual: Vec<_> = (0..256 * 256)
            .map(|i| {
                let t = (i % 511) as f32 / 127.;
                [t, t * 0.6, t * 0.3, if i % 7 == 0 { 0. } else { 0.75 }]
            })
            .collect();
        let mut expected = actual.clone();
        let params = HdrToning { radius: 11., saturation: -17., exposure: 0.3, ..Default::default() };
        super::super::hdr_toning(&mut expected, 256, 256, &params);
        process_with(&mut actual, 256, 256, &params, &Interrupt::NONE).unwrap();
        assert_eq!(actual, expected);
    }
    #[test]
    /// Verify offset native surfaces at every supported depth against separately processed pixels, including selection
    /// publication.
    fn native_depth_models_offsets_and_selection_match_native_counterpart() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        let bounds = Rect::new(-3, 5, 2, 8);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let format = PixelFormat::new(mode, depth, true);
                let mut source = Surface::new(format);
                let mut samples = Vec::new();
                for i in 0..15 {
                    let c = [i as f32 / 9., 0.3, 0.6, if i % 3 == 0 { 0. } else { 0.7 }];
                    samples.extend(photocraft_raster::from_rgba(&format, c));
                }
                source.write_region(bounds, &samples);
                let before = source.read_region(bounds);
                let mut expected: Vec<_> = before.chunks_exact(format.channels()).map(|q| to_rgba(&format, q)).collect();
                let p = HdrToning { radius: 3., detail: 12., saturation: 0., ..Default::default() };
                super::super::hdr_toning(&mut expected, 5, 3, &p);
                let expected: Vec<_> = expected
                    .iter()
                    .zip(before.chunks_exact(format.channels()))
                    .flat_map(|(q, original)| if q[3] <= 0. { original.to_vec() } else { photocraft_raster::from_rgba(&format, *q) })
                    .collect();
                let mut oracle = Surface::new(format);
                oracle.write_region(bounds, &expected);
                let got = apply_with(&source, &p, bounds, bounds, None, &Interrupt::NONE).unwrap();
                for (a, b) in got.read_region(bounds).iter().zip(oracle.read_region(bounds)) {
                    assert!((a - b).abs() < 2e-6, "{mode:?} {depth:?}: {a} {b}");
                }
                let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                selection.write_region(bounds, &[0.; 15]);
                let masked = apply_with(&source, &p, bounds, bounds, Some(&selection), &Interrupt::NONE).unwrap();
                assert_eq!(masked.read_region(bounds), before);
            }
        }
    }
    #[test]
    /// Require cancelled or oversized surface work to preserve the source and the raw adapter's valid identity
    /// fallback.
    fn cancelled_surface_never_publishes_and_workspace_is_bounded() {
        use photocraft_color::{PixelFormat, SampleType};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let bounds = Rect::new(0, 0, 64, 64);
        let mut source = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Rgb, SampleType::F32, true));
        source.write_region(bounds, &vec![0.5; 64 * 64 * 4]);
        let before = source.read_region(bounds);
        let calls = AtomicUsize::new(0);
        let cancelled = || calls.fetch_add(1, Ordering::Relaxed) > 80;
        assert!(matches!(apply_with(&source, &HdrToning::default(), bounds, bounds, None, &Interrupt::cancel_only(&cancelled)), Err(Error::Cancelled)));
        assert_eq!(source.read_region(bounds), before);
        assert!(matches!(
            apply_with(&source, &HdrToning::default(), Rect::new(0, 0, 65_536, 65_536), Rect::new(0, 0, 65_536, 65_536), None, &Interrupt::NONE),
            Err(Error::WorkspaceLimit)
        ));
        let ctx = crate::Ctx { bounds, mode: photocraft_color::ColorMode::Rgb, alpha: true };
        let raw = crate::Image { rect: bounds, ch: 4, data: before.clone() };
        let p = HdrToning { radius: 0., ..Default::default() };
        assert_eq!(run_image(&raw, bounds, &ctx, &p), before);
    }
}
