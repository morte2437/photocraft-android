//! Bounded alpha-weighted averaging and Photoshop's coverage-preserving Average.
pub use crate::coverage::Error;
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface, from_rgba_into, to_rgba};

/// Accumulation order and F32 products preserve the original Raster Average policy.
#[derive(Default)]
pub struct Mean {
    sum: [f64; 3],
    weight: f64,
    alpha: f64,
    count: f64,
}
impl Mean {
    /// Accumulate the native selection/alpha-weighted color mean while preserving the established multiplication
    /// order.
    pub fn push(&mut self, p: [f32; 4], coverage: f32) {
        let weight = f64::from(p[3] * coverage);
        for (sum, value) in self.sum.iter_mut().zip(p) {
            *sum += f64::from(value) * weight;
        }
        self.weight += weight;
        self.alpha += f64::from(p[3] * coverage);
        self.count += f64::from(coverage);
    }
    /// Photoshop Average weights straight colors by selection, including hidden RGB, without changing coverage.
    fn push_colour(&mut self, p: [f32; 4], coverage: f32) {
        let weight = f64::from(coverage);
        for (sum, value) in self.sum.iter_mut().zip(p) {
            *sum += f64::from(value) * weight;
        }
        self.weight += weight;
        self.count += weight;
        self.alpha += f64::from(p[3]) * weight;
    }
    /// Return the normalized mean only when positive alpha weight exists, keeping the native average-alpha policy.
    pub fn rgba(&self) -> Option<[f32; 4]> {
        (self.weight > 0.).then(|| {
            [
                (self.sum[0] / self.weight) as f32,
                (self.sum[1] / self.weight) as f32,
                (self.sum[2] / self.weight) as f32,
                (self.alpha / self.count.max(1e-9)) as f32,
            ]
        })
    }
}

/// Translate interruption into the shared checked native-filter cancellation error.
fn check(ctl: &Interrupt) -> Result<(), Error> {
    ctl.check().map_err(|_| Error::Cancelled)
}

/// Selection coverage saturates like the ordinary filter path; nonfinite masks remain invalid.
fn selection_coverage(selection: Option<&Surface>, x: i32, y: i32) -> Result<f32, Error> {
    let coverage = selection.map_or(1., |surface| surface.sample_channel(x, y, 0));
    if !coverage.is_finite() {
        return Err(Error::NonFiniteSample);
    }
    Ok(coverage.clamp(0., 1.))
}
/// Average a bounded native area using fallible row storage and publish only finite, uncancelled selection-mixed
/// output.
pub fn apply_with(surface: &Surface, area: Rect, bounds: Rect, selection: Option<&Surface>, ctl: &Interrupt) -> Result<Surface, Error> {
    apply_policy(surface, area, bounds, selection, ctl, false)
}

/// Photoshop Average edits straight colors with selection weights while preserving every native alpha sample.
pub fn apply_preserving_alpha_with(surface: &Surface, area: Rect, bounds: Rect, selection: Option<&Surface>, ctl: &Interrupt) -> Result<Surface, Error> {
    apply_policy(surface, area, bounds, selection, ctl, true)
}

/// Preserve the upstream optional dispatcher contract while using bounded checked publication.
pub(crate) fn apply(surface: &Surface, area: Rect, selection: Option<&Surface>, ctl: &Interrupt) -> Option<Surface> {
    apply_preserving_alpha_with(surface, area, area, selection, ctl).ok()
}

/// Both averaging policies share finite-sample checks, output admission and cancellable row scratch.
fn apply_policy(surface: &Surface, area: Rect, bounds: Rect, selection: Option<&Surface>, ctl: &Interrupt, preserve_alpha: bool) -> Result<Surface, Error> {
    check(ctl)?;
    let area = area.intersect(&bounds);
    if area.is_empty() {
        return Ok(surface.clone());
    }
    let w = area.width() as usize;
    let h = area.height() as usize;
    if w.checked_mul(h).is_none_or(|n| n > crate::coverage::MAX_PIXELS) {
        return Err(Error::InvalidGeometry);
    }
    let tile = i64::from(photocraft_geom::TILE_SIZE);
    let axes = |a: i32, b: i32| -> Result<usize, Error> {
        usize::try_from((i64::from(b) - 1).div_euclid(tile) - i64::from(a).div_euclid(tile) + 1).map_err(|_| Error::WorkspaceLimit)
    };
    let bytes = axes(area.x0, area.x1)?
        .checked_mul(axes(area.y0, area.y1)?)
        .and_then(|n| n.checked_mul((tile * tile) as usize * surface.format().bytes_per_pixel() + 128))
        .and_then(|n| n.checked_add(w * (surface.channels() + 4) * 4))
        .and_then(|n| n.checked_add(surface.tile_count().checked_mul(128)?))
        .ok_or(Error::WorkspaceLimit)?;
    if bytes > crate::coverage::MAX_WORKSPACE_BYTES {
        return Err(Error::WorkspaceLimit);
    }
    let n = surface.channels();
    if n == 0 || n > 5 {
        return Err(Error::InvalidGeometry);
    }
    let mut row = Vec::new();
    row.try_reserve_exact(w * n).map_err(|_| Error::Allocation)?;
    let mut mean = Mean::default();
    for y in area.y0..area.y1 {
        check(ctl)?;
        surface.read_region_into(Rect::new(area.x0, y, area.x1, y + 1), &mut row);
        for (x, pixel) in (area.x0..area.x1).zip(row.chunks_exact(n)) {
            let p = to_rgba(&surface.format(), pixel);
            let k = selection_coverage(selection, x, y)?;
            if p.into_iter().any(|v| !v.is_finite()) {
                return Err(Error::NonFiniteSample);
            }
            if preserve_alpha {
                mean.push_colour(p, k);
            } else {
                mean.push(p, k);
            }
        }
    }
    let Some(avg) = mean.rgba() else { return Ok(surface.clone()) };
    if avg.into_iter().any(|v| !v.is_finite()) {
        return Err(Error::NonFiniteSample);
    }
    let mut out = surface.clone();
    for y in area.y0..area.y1 {
        check(ctl)?;
        surface.read_region_into(Rect::new(area.x0, y, area.x1, y + 1), &mut row);
        for (x, dst) in (area.x0..area.x1).zip(row.chunks_exact_mut(n)) {
            let k = selection_coverage(selection, x, y)?;
            if k == 0. {
                continue;
            }
            let original_alpha = dst.last().copied();
            let p = to_rgba(&surface.format(), dst);
            let mut target = avg;
            if preserve_alpha {
                target[3] = p[3];
            }
            let mut mixed = p;
            if !preserve_alpha && surface.format().alpha && p[3] != avg[3] {
                let before = f64::from(p[3]) * (1. - f64::from(k));
                let after = f64::from(avg[3]) * f64::from(k);
                let alpha = before + after;
                for c in 0..3 {
                    mixed[c] = if alpha > 0. { ((f64::from(p[c]) * before + f64::from(avg[c]) * after) / alpha) as f32 } else { 0. };
                }
                mixed[3] = alpha as f32;
                if mixed.iter().any(|value| !value.is_finite()) {
                    return Err(Error::NonFiniteSample);
                }
                from_rgba_into(&surface.format(), mixed, dst);
                if dst.iter().any(|v| !v.is_finite()) {
                    return Err(Error::NonFiniteSample);
                }
                continue;
            }
            for (v, (a, b)) in mixed.iter_mut().zip(p.into_iter().zip(target)) {
                *v = a + (b - a) * k;
                if !v.is_finite() {
                    let precise = f64::from(a) + (f64::from(b) - f64::from(a)) * f64::from(k);
                    if !precise.is_finite() || precise.abs() > f64::from(f32::MAX) {
                        return Err(Error::NonFiniteSample);
                    }
                    *v = precise as f32;
                }
            }
            from_rgba_into(&surface.format(), mixed, dst);
            if preserve_alpha
                && surface.format().alpha
                && let Some(alpha) = dst.last_mut()
            {
                *alpha = original_alpha.unwrap_or(p[3]);
            }
            if dst.iter().any(|v| !v.is_finite()) {
                return Err(Error::NonFiniteSample);
            }
        }
        out.try_write_region(Rect::new(area.x0, y, area.x1, y + 1), &row).map_err(|_| Error::Allocation)?;
    }
    check(ctl)?;
    out.prune();
    Ok(out)
}

/// Adapt raw native float images to checked Average, preserving bounded identity fallback on rejected work.
#[cfg(test)]
pub(crate) fn run_image(src: &crate::image::Image, out: Rect, ctx: &crate::Ctx) -> Vec<f32> {
    run_image_policy(src, out, ctx, false)
}

/// The ordinary command's direct kernel shares the canvas-clipped mean and alpha-preserving policy.
pub(crate) fn kernel(src: &crate::image::Image, out: Rect, ctx: &crate::Ctx) -> Vec<f32> {
    run_image_policy(src, out, ctx, true)
}

/// Raw native images remain bounded and retain identity fallback on rejected work.
fn run_image_policy(src: &crate::image::Image, out: Rect, ctx: &crate::Ctx, preserve_alpha: bool) -> Vec<f32> {
    let format = photocraft_color::PixelFormat::new(ctx.mode, photocraft_color::SampleType::F32, ctx.alpha);
    let valid = [src.rect, out].into_iter().all(|r| (r.width() as usize).checked_mul(r.height() as usize).is_some_and(|n| n <= crate::coverage::MAX_PIXELS));
    if !valid
        || src.ch != format.channels()
        || (src.rect.width() as usize).checked_mul(src.rect.height() as usize).and_then(|n| n.checked_mul(src.ch)) != Some(src.data.len())
    {
        return Vec::new();
    }
    let tile = photocraft_geom::TILE_SIZE as u64;
    let tiles = |r: Rect| (u64::from(r.width()).div_ceil(tile) + 1) * (u64::from(r.height()).div_ceil(tile) + 1);
    if (tiles(src.rect) + tiles(out)).saturating_mul(tile * tile * format.bytes_per_pixel() as u64) > crate::coverage::MAX_WORKSPACE_BYTES as u64 {
        return Vec::new();
    }
    let mut surface = Surface::new(format);
    let identity = || -> Result<Vec<f32>, Error> {
        let count = (out.width() as usize).checked_mul(out.height() as usize).and_then(|n| n.checked_mul(src.ch)).ok_or(Error::WorkspaceLimit)?;
        let mut values = Vec::new();
        values.try_reserve_exact(count).map_err(|_| Error::Allocation)?;
        for y in out.y0..out.y1 {
            for x in out.x0..out.x1 {
                for c in 0..src.ch {
                    values.push(src.get(x, y, c));
                }
            }
        }
        Ok(values)
    };
    let mut run = || -> Result<Vec<f32>, Error> {
        let domain = src.rect.intersect(&ctx.bounds);
        surface.try_write_region(src.rect, &src.data).map_err(|_| Error::Allocation)?;
        // Average the complete supplied image, even when only one output tile is
        // requested. Unpopulated canvas pixels are not part of this image.
        apply_policy(&surface, domain, domain, None, &Interrupt::NONE, preserve_alpha)?.try_read_region(out).map_err(|_| Error::Allocation)
    };
    run().unwrap_or_else(|_| identity().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};

    /// Canvas size and output tiling must not dilute a small offset image's color or alpha mean.
    #[test]
    fn raw_average_uses_image_domain_and_keeps_global_mean_across_output_tiles() {
        let rect = Rect::new(12, 15, 14, 16);
        for alpha in [false, true] {
            let format = PixelFormat::new(ColorMode::Rgb, SampleType::F32, alpha);
            let mut data = vec![0.2, 0.4, 0.6];
            if alpha {
                data.push(0.5);
            }
            data.extend([0.8, 0.6, 0.4]);
            if alpha {
                data.push(1.);
            }
            let image = crate::Image { rect, ch: format.channels(), data };
            for bounds in [Rect::new(0, 0, 20, 20), Rect::new(0, 0, 10_000, 10_000)] {
                let ctx = crate::Ctx { bounds, mode: ColorMode::Rgb, alpha };
                let expected = if alpha { vec![0.6, 8. / 15., 7. / 15., 0.75] } else { vec![0.5, 0.5, 0.5] };
                for out in [rect, Rect::new(12, 15, 13, 16), Rect::new(13, 15, 14, 16)] {
                    let result = run_image(&image, out, &ctx);
                    assert_eq!(result.len(), out.size().area() as usize * format.channels());
                    for pixel in result.chunks_exact(format.channels()) {
                        for (value, want) in pixel.iter().zip(&expected) {
                            assert!((value - want).abs() < 1e-6);
                        }
                    }
                }
            }
        }
    }
    #[test]
    /// Keep unselected CMYK separations and out-of-gamut Lab samples unchanged at every native depth.
    fn zero_coverage_preserves_native_cmyk_and_out_of_gamut_lab_samples() {
        for mode in [ColorMode::Cmyk, ColorMode::Lab] {
            for depth in SampleType::ALL {
                let format = PixelFormat::new(mode, depth, true);
                let mut source = Surface::new(format);
                let untouched: &[f32] = if mode == ColorMode::Cmyk { &[0.2, 0.4, 0.6, 0.5, 1.] } else { &[0.5, 1., 1., 1.] };
                source.write_pixel(1, 0, untouched);
                source.write_pixel(0, 0, &vec![0.75; format.channels()]);
                let before = source.pixel(1, 0);
                let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                mask.write_pixel(0, 0, &[1.]);
                let bounds = Rect::new(0, 0, 2, 1);
                let output = apply_with(&source, bounds, bounds, Some(&mask), &Interrupt::NONE).unwrap();
                assert_eq!(output.pixel(1, 0), before, "{mode:?} {depth:?}");
            }
        }
    }
    #[test]
    #[ignore = "24MP native Average performance probe"]
    /// Measure release Average on a full-size original surface with bounded rows rather than a whole-frame sample
    /// copy.
    fn average_24mp_bounded_rows_benchmark() {
        let b = Rect::new(0, 0, 6000, 4000);
        let mut source = Surface::new(PixelFormat::RGBA32F);
        source.fill_rect(Rect::new(0, 0, 3000, 4000), &[1., 0., 0., 0.5]);
        source.fill_rect(Rect::new(3000, 0, 6000, 4000), &[0., 0., 1., 1.]);
        let started = std::time::Instant::now();
        let output = apply_with(&source, b, b, None, &Interrupt::NONE).unwrap();
        println!("native Average24MP RGBA32F elapsed_ms={}", started.elapsed().as_millis());
        let p = output.pixel(3000, 2000);
        assert!((p[0] - 1. / 3.).abs() < 1e-6 && (p[2] - 2. / 3.).abs() < 1e-6 && p[3] == 0.75);
    }
    #[test]
    /// Pin native mean weighting, premultiplied feathering and quantization across color models and depths.
    fn rgba_weighted_mean_and_premultiplied_selection_at_every_depth() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let format = PixelFormat { mode, sample: depth, alpha: true };
                let mut s = Surface::new(format);
                let mut p = vec![0.; format.channels()];
                from_rgba_into(&format, [0.2, 0.4, 0.6, 0.5], &mut p);
                s.write_pixel(0, 0, &p);
                from_rgba_into(&format, [0.8, 0.3, 0.1, 1.], &mut p);
                s.write_pixel(1, 0, &p);
                let a = to_rgba(&format, &s.pixel(0, 0));
                let b = to_rgba(&format, &s.pixel(1, 0));
                let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                mask.write_pixel(0, 0, &[1.]);
                mask.write_pixel(1, 0, &[0.25]);
                let mut m = Mean::default();
                m.push(a, 1.);
                m.push(b, 0.25);
                let expected = m.rgba().unwrap();
                let out = apply_with(&s, Rect::new(0, 0, 2, 1), Rect::new(0, 0, 2, 1), Some(&mask), &Interrupt::NONE).unwrap();
                let mut encoded = Surface::new(format);
                from_rgba_into(&format, expected, &mut p);
                encoded.write_pixel(0, 0, &p);
                assert_eq!(out.pixel(0, 0), encoded.pixel(0, 0));
                let alpha = f64::from(b[3]) * 0.75 + f64::from(expected[3]) * 0.25;
                let mixed = std::array::from_fn(|c| {
                    if c == 3 {
                        alpha as f32
                    } else {
                        ((f64::from(b[c]) * f64::from(b[3]) * 0.75 + f64::from(expected[c]) * f64::from(expected[3]) * 0.25) / alpha) as f32
                    }
                });
                from_rgba_into(&format, mixed, &mut p);
                encoded.write_pixel(1, 0, &p);
                assert_eq!(out.pixel(1, 0), encoded.pixel(1, 0));
            }
        }
    }
    #[test]
    /// Compare with an independently calculated weighted mean and require cancellation to preserve source pixels.
    fn independent_mean_and_cancel_are_atomic() {
        let mut s = Surface::new(PixelFormat::RGBA32F);
        s.write_pixel(0, 0, &[1., 0., 0., 0.5]);
        s.write_pixel(1, 0, &[0., 0., 1., 1.]);
        let b = Rect::new(0, 0, 2, 1);
        let out = apply_with(&s, b, b, None, &Interrupt::NONE).unwrap();
        let p = out.pixel(0, 0);
        assert!((p[0] - 1. / 3.).abs() < 1e-6);
        assert!((p[2] - 2. / 3.).abs() < 1e-6);
        assert_eq!(p[3], 0.75);
        let cancel = || true;
        assert_eq!(apply_with(&s, b, b, None, &Interrupt::cancel_only(&cancel)).unwrap_err(), Error::Cancelled);
        assert_eq!(s.pixel(0, 0), vec![1., 0., 0., 0.5]);
        assert!(apply_with(&s, Rect::new(0, 0, 100000, 100000), Rect::new(0, 0, 100000, 100000), None, &Interrupt::NONE).is_err());
    }
    #[test]
    /// Cover opaque HDR, empty alpha support and delayed cancellation under the existing native Average policy.
    fn opaque_hdr_empty_alpha_and_mid_pass_cancel_keep_native_rules() {
        let b = Rect::new(0, 0, 2, 2);
        let mut opaque = Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, false));
        opaque.fill_rect(b, &[4., -2., 0.5]);
        let out = apply_with(&opaque, b, b, None, &Interrupt::NONE).unwrap();
        assert_eq!(out.read_region(b), opaque.read_region(b));
        let mut transparent = Surface::new(PixelFormat::RGBA32F);
        transparent.fill_rect(b, &[1., 0., 0., 0.]);
        assert_eq!(apply_with(&transparent, b, b, None, &Interrupt::NONE).unwrap().read_region(b), transparent.read_region(b));
        use std::sync::atomic::{AtomicUsize, Ordering};
        let checks = AtomicUsize::new(0);
        let cancel = || checks.fetch_add(1, Ordering::Relaxed) >= 3;
        let before = opaque.clone();
        assert_eq!(apply_with(&opaque, b, b, None, &Interrupt::cancel_only(&cancel)).unwrap_err(), Error::Cancelled);
        assert_eq!(opaque, before);
        opaque.write_pixel(0, 0, &[f32::NAN, 0., 0.]);
        assert_eq!(apply_with(&opaque, b, b, None, &Interrupt::NONE).unwrap_err(), Error::NonFiniteSample);
    }
    #[test]
    /// Exercise extreme finite HDR selection mixtures so intermediate overflow cannot publish nonfinite pixels.
    fn finite_hdr_selection_intermediates_never_publish_infinity() {
        let b = Rect::new(0, 0, 2, 1);
        let mut source = Surface::new(PixelFormat::RGBA32F);
        source.write_pixel(0, 0, &[f32::MAX, 0., 0., 1.]);
        source.write_pixel(1, 0, &[-f32::MAX, 0., 0., 1.]);
        let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        selection.write_pixel(0, 0, &[1.]);
        selection.write_pixel(1, 0, &[0.5]);
        let before = source.clone();
        let output = apply_with(&source, b, b, Some(&selection), &Interrupt::NONE).unwrap();
        assert!(output.read_region(b).iter().all(|v| v.is_finite()));
        assert!((f64::from(output.pixel(0, 0)[0]) / f64::from(f32::MAX) - 1. / 3.).abs() < 1e-6);
        assert!((f64::from(output.pixel(1, 0)[0]) / f64::from(f32::MAX) + 1. / 3.).abs() < 1e-6);
        assert_eq!(source, before);
        // Hostile finite signed coverage can make the mean itself unrepresentable.
        source.write_pixel(1, 0, &[-f32::MAX, 0., 0., -0.5]);
        let before = source.clone();
        assert_eq!(apply_with(&source, b, b, None, &Interrupt::NONE).unwrap_err(), Error::NonFiniteSample);
        assert_eq!(source, before);
    }

    #[test]
    /// Reject selected Average results that overflow native Lab conversion without modifying the input surface.
    fn alpha_changing_lab_selection_rejects_nonfinite_native_conversion_atomically() {
        let format = PixelFormat::new(ColorMode::Lab, SampleType::F32, true);
        let bounds = Rect::new(0, 0, 3, 1);
        let neutral = 128. / 255.;
        // Finite signed alpha is hostile but accepted input. Its weights nearly cancel while the
        // white pixel's color contribution remains, creating finite HDR RGB that overflows Lab.
        let pixels: [[f32; 4]; 3] = [[1., neutral, neutral, 1.], [0., neutral, neutral, -1.], [0., neutral, neutral, 1e-20]];
        let mut source = Surface::new(format);
        let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        let mut mean = Mean::default();
        for (x, (pixel, coverage)) in pixels.into_iter().zip([1., 1., 0.5]).enumerate() {
            assert!(pixel.iter().all(|v| v.is_finite()));
            source.write_pixel(x as i32, 0, &pixel);
            selection.write_pixel(x as i32, 0, &[coverage]);
            let rgba = to_rgba(&format, &pixel);
            assert!(rgba.iter().all(|v| v.is_finite()));
            mean.push(rgba, coverage);
        }
        let avg = mean.rgba().unwrap();
        assert!(avg.iter().all(|v| v.is_finite()));
        assert!(avg[0] > 1e19);
        assert_ne!(avg[3], pixels[0][3]);
        // Full selection coverage at the first pixel makes its premultiplied mix exactly avg.
        let mut native = [0.; 4];
        from_rgba_into(&format, avg, &mut native);
        assert!(native.iter().any(|v| !v.is_finite()));
        let before = source.clone();
        assert_eq!(apply_with(&source, bounds, bounds, Some(&selection), &Interrupt::NONE).unwrap_err(), Error::NonFiniteSample);
        assert_eq!(source, before);
    }
}

#[cfg(test)]
#[path = "average/tests.rs"]
mod upstream_tests;
