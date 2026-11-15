//! A smart object's filter mask (#215): how the filtered pixels and the unfiltered (placed) ones
//! combine.
//!
//! Photoshop stores a smart object's layer pixels with its filters applied but not its filter
//! mask, and mixes the unfiltered pixels back in through the mask when it composites (photoshop
//! corpus `sf-filter-mask-hard.psd` and `sf-filter-mask-gradient.psd`: the stored pixels are
//! filtered everywhere, the merged image only where the mask shows the filter). Our smart objects
//! keep the mixed result as their pixels, so PSD import mixes ([`mix`]) and export unmixes
//! ([`unmix`]).

use photocraft_color::SampleType;
use photocraft_doc::LayerMask;
use photocraft_raster::Surface;

/// `filtered` mixed over `base` (the unfiltered pixels) through `mask`: premultiplied, any
/// channel layout. Pixels the mask hides entirely are `base`'s.
pub fn mix(base: &Surface, filtered: &Surface, mask: &LayerMask) -> Surface {
    let fmt = base.format();
    let area = base.content_bounds().union(&filtered.content_bounds());
    let mut out = filtered.clone();
    if area.is_empty() || filtered.format() != fmt {
        return out;
    }
    let n = fmt.channels();
    // Tile by tile: the union of two sparse surfaces far apart is a rectangle of mostly nothing,
    // and transparent pixels in tiles neither surface has stay as they are.
    let whole = area;
    let same_default = base.default_pixel() == filtered.default_pixel();
    // Keep tile scratch for the operation. Reallocating these float buffers per tile
    // dominates large masks; every output sample is overwritten before publication.
    let (mut b, mut f, mut k, mut o) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for t in whole.tiles() {
        let area = t.rect().intersect(&whole);
        let bare = !base.has_tiles_in(area) && !filtered.has_tiles_in(area);
        if area.is_empty() || (bare && (fmt.alpha || same_default)) {
            continue;
        }
        base.read_region_into(area, &mut b);
        filtered.read_region_into(area, &mut f);
        mask.values_into(area, &mut k);
        o.resize(b.len(), 0.0f32);
        for (i, ((bp, fp), op)) in b.chunks_exact(n).zip(f.chunks_exact(n)).zip(o.chunks_exact_mut(n)).enumerate() {
            let m = k.get(i).copied().unwrap_or(1.0).clamp(0.0, 1.0);
            if fmt.sample == SampleType::F32 {
                // Native float colors can be signed or HDR. Weighted F64 sums also avoid overflowing
                // the difference between opposite finite F32 extremes before applying coverage.
                let m = f64::from(m);
                if fmt.alpha {
                    let a = n - 1;
                    let before = f64::from(bp[a]) * (1.0 - m);
                    let after = f64::from(fp[a]) * m;
                    let alpha = before + after;
                    op[a] = alpha as f32;
                    for c in 0..a {
                        op[c] = if alpha > 0.0 { ((f64::from(bp[c]) * before + f64::from(fp[c]) * after) / alpha) as f32 } else { 0.0 };
                    }
                } else {
                    for c in 0..n {
                        op[c] = (f64::from(bp[c]) * (1.0 - m) + f64::from(fp[c]) * m) as f32;
                    }
                }
                continue;
            }
            if !fmt.alpha {
                for c in 0..n {
                    op[c] = bp[c] + (fp[c] - bp[c]) * m;
                }
                continue;
            }
            let a = n - 1;
            let oa = bp[a] + (fp[a] - bp[a]) * m;
            op[a] = oa;
            for c in 0..a {
                let pm = bp[c] * bp[a] + (fp[c] * fp[a] - bp[c] * bp[a]) * m;
                op[c] = if oa > 0.0 { (pm / oa).clamp(0.0, 1.0) } else { 0.0 };
            }
        }
        out.write_region(area, &o);
    }
    out.prune();
    out
}

/// The mask value below which a pixel's filtered value is taken to be unknown: [`unmix`] gives the
/// unfiltered value there, which the mask shows (almost) none of anyway.
const HIDDEN: f32 = 1.0 / 1024.0;

/// The inverse of [`mix`]: the filtered pixels that mix with `base` through `mask` to `mixed`
/// (clamped to 0..1 except F32 colors, which retain their signed/HDR range).
/// Where the mask hides the filter entirely they are `base`'s, which Photoshop doesn't show there.
pub fn unmix(base: &Surface, mixed: &Surface, mask: &LayerMask) -> Surface {
    let fmt = base.format();
    let area = base.content_bounds().union(&mixed.content_bounds());
    let mut out = mixed.clone();
    if area.is_empty() || mixed.format() != fmt {
        return out;
    }
    // Tile by tile: the union of two sparse surfaces far apart is a rectangle of mostly nothing,
    // and transparent pixels in tiles neither surface has stay as they are.
    let whole = area;
    // Non-alpha still needs every tile whose defaults differ (an unmix through a non-unit mask
    // would change that default, and the clone already holds the old one).
    let same_default = base.default_pixel() == mixed.default_pixel();
    let mut tiles = whole.tiles().filter(|t| {
        let area = t.rect().intersect(&whole);
        let bare = !base.has_tiles_in(area) && !mixed.has_tiles_in(area);
        !area.is_empty() && !(bare && (fmt.alpha || same_default))
    });
    #[cfg(not(target_arch = "wasm32"))]
    if whole.tile_count() >= 8 && rayon::current_num_threads() > 1 {
        use rayon::prelude::*;
        // Eight reusable slots bound both scratch and in-flight results, even for a
        // huge sparse union. Surface publication stays ordered and single-threaded.
        let mut scratch: [UnmixScratch; 8] = std::array::from_fn(|_| UnmixScratch::default());
        let mut batch = Vec::with_capacity(scratch.len());
        loop {
            batch.clear();
            batch.extend(tiles.by_ref().take(scratch.len()));
            if batch.is_empty() {
                break;
            }
            scratch.par_iter_mut().zip(batch.par_iter()).for_each(|(scratch, t)| {
                unmix_tile(base, mixed, mask, t.rect().intersect(&whole), scratch);
            });
            for (t, scratch) in batch.iter().zip(&scratch) {
                out.write_region(t.rect().intersect(&whole), &scratch.output);
            }
        }
        out.prune();
        return out;
    }
    let mut scratch = UnmixScratch::default();
    for t in tiles.by_ref() {
        let area = t.rect().intersect(&whole);
        unmix_tile(base, mixed, mask, area, &mut scratch);
        out.write_region(area, &scratch.output);
    }
    out.prune();
    out
}

#[derive(Default)]
struct UnmixScratch {
    base: Vec<f32>,
    mixed: Vec<f32>,
    mask: Vec<f32>,
    output: Vec<f32>,
}

/// Recover a tile with the same pixel arithmetic on the serial and bounded parallel paths.
fn unmix_tile(base: &Surface, mixed: &Surface, mask: &LayerMask, area: photocraft_geom::Rect, scratch: &mut UnmixScratch) {
    let fmt = base.format();
    let n = fmt.channels();
    let UnmixScratch { base: b, mixed: x, mask: k, output: o } = scratch;
    base.read_region_into(area, b);
    mixed.read_region_into(area, x);
    mask.values_into(area, k);
    o.resize(b.len(), 0.0);
    // Early-continue branches intentionally leave zero samples. Reset reused slots too.
    o.fill(0.0);
    for (i, ((bp, xp), op)) in b.chunks_exact(n).zip(x.chunks_exact(n)).zip(o.chunks_exact_mut(n)).enumerate() {
        let m = k.get(i).copied().unwrap_or(1.0).clamp(0.0, 1.0);
        if m < HIDDEN {
            // A transparent base pixel (Photoshop's cache keeps white there) stays empty, so the
            // stored pixels don't grow past the content.
            if !fmt.alpha || bp[n - 1] > 0.0 {
                op.copy_from_slice(bp);
            }
            continue;
        }
        if !fmt.alpha {
            if fmt.sample == SampleType::F32 {
                if m == 1.0 {
                    op.copy_from_slice(xp);
                } else {
                    let m = f64::from(m);
                    for c in 0..n {
                        // Invert the weighted sum directly. Native differences can overflow;
                        // adding the base back also loses small results at full coverage.
                        let color = (f64::from(xp[c]) - f64::from(bp[c]) * (1.0 - m)) / m;
                        op[c] = color.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
                    }
                }
                continue;
            }
            for c in 0..n {
                op[c] = (bp[c] + (xp[c] - bp[c]) / m).clamp(0.0, 1.0);
            }
            continue;
        }
        let a = n - 1;
        if fmt.sample == SampleType::F32 {
            let m = f64::from(m);
            let before = f64::from(bp[a]) * (1.0 - m);
            let fa = ((f64::from(xp[a]) - before) / m).clamp(0.0, 1.0);
            if fa <= 0.0 {
                continue;
            }
            op[a] = fa as f32;
            for c in 0..a {
                // Invert the premultiplied weighted sum directly: F32 differences can
                // overflow, and adding the base back can erase the result at full coverage.
                let color = (f64::from(xp[c]) * f64::from(xp[a]) - f64::from(bp[c]) * before) / (m * fa);
                // Arbitrary stored mixed pixels need not admit a representable inverse.
                op[c] = color.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
            }
            continue;
        }
        let fa = (bp[a] + (xp[a] - bp[a]) / m).clamp(0.0, 1.0);
        if fa <= 0.0 {
            continue;
        }
        op[a] = fa;
        for c in 0..a {
            let pm = bp[c] * bp[a] + (xp[c] * xp[a] - bp[c] * bp[a]) / m;
            op[c] = (pm / fa).clamp(0.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    use photocraft_doc::LayerMask;
    use photocraft_raster::Surface;

    use super::*;

    fn surface(fmt: PixelFormat, f: impl Fn(i32, i32) -> Vec<f32>) -> Surface {
        let mut s = Surface::new(fmt);
        for y in 0..4 {
            for x in 0..8 {
                s.write_pixel(x, y, &f(x, y));
            }
        }
        s
    }

    /// A mask ramping from hidden (left) to shown (right).
    fn ramp() -> LayerMask {
        let mut m = LayerMask::reveal_all();
        m.surface = surface(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), |x, _| vec![x as f32 / 7.0]);
        m
    }

    #[test]
    /// Compare reused mask-mixing scratch with isolated pixel oracles across formats and sparse tile boundaries.
    fn reused_scratch_matches_isolated_pixels_across_partial_and_missing_tiles() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Multichannel] {
            for sample in SampleType::ALL {
                for alpha in [false, true] {
                    let fmt = PixelFormat::new(mode, sample, alpha);
                    let n = fmt.channels();
                    let colors = n - usize::from(alpha);
                    let base_default = vec![if alpha { 0. } else { 0.125 }; n];
                    let filtered_default = vec![if alpha { 0. } else { 0.75 }; n];
                    let mut base = Surface::with_default(fmt, &base_default);
                    let mut filtered = Surface::with_default(fmt, &filtered_default);
                    let mut bp = vec![0.25; n];
                    let mut fp = vec![0.625; n];
                    if sample == SampleType::F32 {
                        for c in 0..colors {
                            bp[c] = if c % 2 == 0 { -2. } else { 3. };
                            fp[c] = if c % 2 == 0 { 4. } else { -1. };
                        }
                    }
                    if alpha {
                        bp[colors] = 0.25;
                        fp[colors] = 0.75;
                    }
                    base.fill_rect(photocraft_geom::Rect::new(0, 0, 256, 256), &bp);
                    base.write_pixel(-2, -1, &bp);
                    filtered.fill_rect(photocraft_geom::Rect::new(256, 0, 261, 1), &fp);
                    filtered.write_pixel(260, 257, &fp);
                    let mut mask = LayerMask::reveal_all();
                    mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.5]);
                    mask.surface.fill_rect(photocraft_geom::Rect::new(0, 256, 256, 258), &[0.]);
                    mask.surface.write_pixel(256, 0, &[1.]);
                    if sample == SampleType::F32 {
                        let tiny = f32::from_bits(1);
                        for (x, y, b, f, ba, fa, coverage) in [
                            (-2, -1, f32::MAX, -f32::MAX, tiny, 2. * tiny, 0.5),
                            (1, 1, -0., 0., 0., 1., 0.),
                            (255, 255, f32::MAX, f32::MAX, 0.01, 0.01, 0.03),
                            (256, 257, 1., -f32::MAX, 1., f32::MIN_POSITIVE, 1.),
                            (260, 257, tiny, 2. * tiny, tiny, 2. * tiny, 0.5),
                        ] {
                            let (mut bp, mut fp) = (vec![b; n], vec![f; n]);
                            if alpha {
                                bp[colors] = ba;
                                fp[colors] = fa;
                            }
                            base.write_pixel(x, y, &bp);
                            filtered.write_pixel(x, y, &fp);
                            mask.surface.write_pixel(x, y, &[coverage]);
                        }
                    }
                    let before = (base.clone(), filtered.clone(), mask.clone());
                    let result = mix(&base, &filtered, &mask);
                    for y in [-1, 0, 1, 255, 256, 257] {
                        for x in [-2, -1, 0, 1, 254, 255, 256, 260] {
                            // An isolated one-pixel operation cannot retain another tile's scratch.
                            let mut isolated_base = Surface::new(fmt);
                            let mut isolated_filtered = Surface::new(fmt);
                            isolated_base.write_pixel(0, 0, &base.pixel(x, y));
                            isolated_filtered.write_pixel(0, 0, &filtered.pixel(x, y));
                            let mut isolated_mask = LayerMask::reveal_all();
                            isolated_mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[mask.value(x, y)]);
                            let expected = mix(&isolated_base, &isolated_filtered, &isolated_mask).pixel(0, 0);
                            assert_eq!(
                                result.pixel(x, y).iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                                expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                                "{fmt:?} ({x}, {y})"
                            );
                        }
                    }
                    assert_eq!((base, filtered, mask), before);
                }
            }
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    /// Require parallel native mask recovery to match one worker exactly, including hidden and empty tiles.
    fn native_parallel_unmix_matches_one_worker_bit_for_bit() {
        let serial = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let parallel = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
        let fmt = PixelFormat::RGBA32F;
        let bounds = photocraft_geom::Rect::new(-1, -1, 770, 258);
        let mut base = Surface::new(fmt);
        let mut mixed = Surface::new(fmt);
        base.fill_rect(bounds, &[-2., 3., 4., 0.75]);
        mixed.fill_rect(bounds, &[3., -0.5, 2.5, 0.75]);
        let mut mask = LayerMask::reveal_all();
        mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.5]);
        let hidden = photocraft_geom::Rect::new(0, 0, 256, 256);
        base.fill_rect(hidden, &[1., 2., 3., 0.]);
        mask.surface.fill_rect(hidden, &[0.]);
        let empty = photocraft_geom::Rect::new(256, 0, 512, 256);
        base.fill_rect(empty, &[1., 2., 3., 1.]);
        mixed.fill_rect(empty, &[4., 5., 6., 0.]);
        let expected = serial.install(|| unmix(&base, &mixed, &mask));
        let actual = parallel.install(|| unmix(&base, &mixed, &mask));
        assert_eq!(actual, expected);
    }

    #[test]
    /// Verify inverse scratch reuse clears skipped outputs and matches fresh pixels across sparse native formats.
    fn reused_unmix_scratch_matches_fresh_pixels_and_clears_early_continue_outputs() {
        use photocraft_geom::Rect;

        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Multichannel] {
            for sample in SampleType::ALL {
                for alpha in [false, true] {
                    let fmt = PixelFormat::new(mode, sample, alpha);
                    let n = fmt.channels();
                    let colors = n - usize::from(alpha);
                    let mut bd = vec![0.125; n];
                    let mut xd = vec![0.75; n];
                    let mut bp = vec![0.25; n];
                    let mut xp = vec![0.625; n];
                    if sample == SampleType::F32 {
                        for c in 0..colors {
                            bp[c] = if c % 2 == 0 { -2. } else { 4. };
                            xp[c] = if c % 2 == 0 { 3. } else { -1. };
                        }
                    }
                    if alpha {
                        bd[colors] = 0.;
                        xd[colors] = 0.;
                        bp[colors] = 0.25;
                        xp[colors] = 0.75;
                    }
                    let mut base = Surface::with_default(fmt, &bd);
                    let mut mixed = Surface::with_default(fmt, &xd);
                    let mut mask = LayerMask::reveal_all();
                    mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.5]);
                    // Three consecutive full tiles: nonzero output, hidden transparent base,
                    // then a nonpositive recovered alpha. Both latter tiles must clear scratch.
                    base.fill_rect(Rect::new(0, 0, 256, 256), &bp);
                    mixed.fill_rect(Rect::new(0, 0, 256, 256), &xp);
                    let (mut hidden_b, mut zero_x, mut opaque_b) = (bp.clone(), xp.clone(), bp.clone());
                    if alpha {
                        hidden_b[colors] = 0.;
                        zero_x[colors] = 0.;
                        opaque_b[colors] = 1.;
                    }
                    base.fill_rect(Rect::new(256, 0, 512, 256), &hidden_b);
                    mixed.fill_rect(Rect::new(256, 0, 512, 256), &xp);
                    mask.surface.fill_rect(Rect::new(256, 0, 512, 256), &[HIDDEN / 2.]);
                    base.fill_rect(Rect::new(512, 0, 768, 256), &opaque_b);
                    mixed.fill_rect(Rect::new(512, 0, 768, 256), &zero_x);
                    // Negative and partial boundary tiles vary every scratch buffer's length.
                    base.write_pixel(-2, -1, &bp);
                    mixed.write_pixel(-2, -1, &xp);
                    base.write_pixel(770, 258, &bp);
                    mixed.write_pixel(770, 258, &xp);
                    base.write_pixel(768, 256, &hidden_b);
                    mixed.write_pixel(768, 256, &xp);
                    mask.surface.write_pixel(768, 256, &[0.]);
                    base.write_pixel(769, 256, &opaque_b);
                    mixed.write_pixel(769, 256, &zero_x);
                    mask.surface.write_pixel(1, 1, &[HIDDEN]);
                    if sample == SampleType::F32 {
                        let tiny = f32::from_bits(1);
                        for (x, y, b, v, ba, xa, coverage) in [
                            (-2, -1, f32::MAX, -f32::MAX, 0.25, 0.75, 0.5),
                            (255, 255, -f32::MAX, f32::MAX, 0.5, 0.75, 0.5),
                            (768, 257, tiny, 2. * tiny, tiny, 2. * tiny, 0.5),
                            (769, 257, f32::MAX, -tiny, 1., tiny, 1.),
                            (770, 258, -4., 3., 0.25, 0.75, 1.),
                        ] {
                            let (mut b, mut v) = (vec![b; n], vec![v; n]);
                            if alpha {
                                b[colors] = ba;
                                v[colors] = xa;
                            }
                            base.write_pixel(x, y, &b);
                            mixed.write_pixel(x, y, &v);
                            mask.surface.write_pixel(x, y, &[coverage]);
                        }
                    }
                    let before = (base.clone(), mixed.clone(), mask.clone());
                    let result = unmix(&base, &mixed, &mask);
                    for y in [-1, 0, 1, 128, 255, 256, 257, 258] {
                        for x in [-2, -1, 0, 1, 128, 255, 256, 511, 512, 767, 768, 769, 770] {
                            let point = Rect::new(x, y, x + 1, y + 1);
                            if alpha && !base.has_tiles_in(point) && !mixed.has_tiles_in(point) {
                                // Bare alpha tiles retain the mixed default by design.
                                assert_eq!(result.pixel(x, y), mixed.default_pixel(), "{fmt:?} bare ({x}, {y})");
                                continue;
                            }
                            let mut isolated_base = Surface::new(fmt);
                            let mut isolated_mixed = Surface::new(fmt);
                            isolated_base.write_pixel(0, 0, &base.pixel(x, y));
                            isolated_mixed.write_pixel(0, 0, &mixed.pixel(x, y));
                            let mut isolated_mask = LayerMask::reveal_all();
                            isolated_mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[mask.value(x, y)]);
                            let expected = unmix(&isolated_base, &isolated_mixed, &isolated_mask).pixel(0, 0);
                            let actual = result.pixel(x, y);
                            assert!(actual.iter().all(|v| v.is_finite()), "{fmt:?} ({x}, {y}): {actual:?}");
                            assert_eq!(
                                actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                                expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                                "{fmt:?} ({x}, {y})"
                            );
                        }
                    }
                    if alpha {
                        for (x, y) in [(256, 0), (511, 255), (512, 0), (767, 255), (768, 256), (769, 256)] {
                            assert_eq!(result.pixel(x, y).iter().map(|v| v.to_bits()).collect::<Vec<_>>(), vec![0u32; n], "{fmt:?} stale output ({x}, {y})");
                        }
                    }
                    assert_eq!((base, mixed, mask), before);
                }
            }
        }
    }

    #[test]
    /// Compare selected native-channel mixing with wide premultiplied weights at float and integer depths.
    fn selection_mix_preserves_native_float_hdr_and_integer_ranges() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for depth in SampleType::ALL {
                for alpha in [false, true] {
                    let fmt = PixelFormat::new(mode, depth, alpha);
                    let n = fmt.channels();
                    let colors = n - usize::from(alpha);
                    let mut b: Vec<f32> = (0..n).map(|c| if c % 2 == 0 { -2. } else { 4. }).collect();
                    let mut f: Vec<f32> = (0..n).map(|c| if c % 2 == 0 { 4. } else { -2. }).collect();
                    if alpha {
                        b[colors] = 0.25;
                        f[colors] = 0.75;
                    }
                    let mut base = Surface::new(fmt);
                    let mut filtered = Surface::new(fmt);
                    base.write_pixel(0, 0, &b);
                    filtered.write_pixel(0, 0, &f);
                    let (b, f) = (base.pixel(0, 0), filtered.pixel(0, 0));
                    for coverage in [0., 0.5, 1.] {
                        let mut mask = LayerMask::reveal_all();
                        mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[coverage]);
                        let out = mix(&base, &filtered, &mask).pixel(0, 0);
                        let before = if alpha { f64::from(b[colors]) } else { 1. } * (1. - f64::from(coverage));
                        let after = if alpha { f64::from(f[colors]) } else { 1. } * f64::from(coverage);
                        let weight = before + after;
                        let mut expected = vec![0.; n];
                        for c in 0..colors {
                            expected[c] = ((f64::from(b[c]) * before + f64::from(f[c]) * after) / weight) as f32;
                            if depth != SampleType::F32 {
                                expected[c] = expected[c].clamp(0., 1.);
                            }
                        }
                        if alpha {
                            expected[colors] = weight as f32;
                        }
                        let epsilon = match depth {
                            SampleType::U8 => 1. / 255. + 1e-6,
                            SampleType::U16 => 1. / 65535. + 1e-6,
                            SampleType::F32 => 1e-6,
                        };
                        assert!(
                            out.iter().zip(&expected).all(|(a, b)| a.is_finite() && (a - b).abs() <= epsilon),
                            "{fmt:?} {coverage}: {out:?} vs {expected:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    /// Keep opposite finite F32 extremes representable during selected mixing with and without alpha.
    fn float_selection_mix_of_opposite_finite_extremes_stays_finite() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for alpha in [false, true] {
                let fmt = PixelFormat::new(mode, SampleType::F32, alpha);
                let colors = fmt.channels() - usize::from(alpha);
                let mut b = vec![f32::MAX; fmt.channels()];
                let mut f = vec![-f32::MAX; fmt.channels()];
                if alpha {
                    b[colors] = 0.25;
                    f[colors] = 0.75;
                }
                let mut base = Surface::new(fmt);
                let mut filtered = Surface::new(fmt);
                base.write_pixel(0, 0, &b);
                filtered.write_pixel(0, 0, &f);
                let mut mask = LayerMask::reveal_all();
                mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.5]);
                let out = mix(&base, &filtered, &mask).pixel(0, 0);
                assert!(out.iter().all(|v| v.is_finite()), "{fmt:?}: {out:?}");
                assert_eq!(&out[..colors], vec![if alpha { -f32::MAX / 2. } else { 0. }; colors]);
                if alpha {
                    assert_eq!(out[colors], 0.5);
                }
            }
        }
    }

    #[test]
    fn mixing_sparse_surfaces_far_apart_does_not_build_their_union() {
        // 10¹⁰ px between two single pixels would be 160 GB of floats read densely.
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
        let (mut base, mut filtered) = (Surface::new(fmt), Surface::new(fmt));
        base.write_pixel(0, 0, &[0.2, 0.4, 0.6, 1.0]);
        filtered.write_pixel(100_000, 100_000, &[1.0, 0.0, 0.0, 1.0]);
        let mixed = mix(&base, &filtered, &LayerMask::reveal_all());
        assert_eq!(mixed.pixel(100_000, 100_000), vec![1.0, 0.0, 0.0, 1.0]);
        let back = unmix(&base, &mixed, &LayerMask::reveal_all());
        assert_eq!(back.pixel(100_000, 100_000), vec![1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn unmix_inverts_mix_wherever_the_mask_shows_the_filter() {
        for fmt in [PixelFormat::new(ColorMode::Rgb, SampleType::F32, true), PixelFormat::new(ColorMode::Rgb, SampleType::F32, false)] {
            let base = surface(fmt, |x, y| {
                let mut p = vec![0.2 + 0.05 * x as f32, 0.6, 0.1 * y as f32];
                if fmt.alpha {
                    p.push(0.5 + 0.06 * x as f32);
                }
                p
            });
            let filtered = surface(fmt, |_, y| {
                let mut p = vec![0.9 - 0.1 * y as f32, 0.3, 0.5];
                if fmt.alpha {
                    p.push(0.8);
                }
                p
            });
            let m = ramp();
            let mixed = mix(&base, &filtered, &m);
            // Hidden column: the unfiltered pixels; shown column: the filtered ones.
            assert!(mixed.pixel(0, 2).iter().zip(base.pixel(0, 2)).all(|(a, b)| (a - b).abs() < 1e-6));
            assert!(mixed.pixel(7, 2).iter().zip(filtered.pixel(7, 2)).all(|(a, b)| (a - b).abs() < 1e-6));
            let back = unmix(&base, &mixed, &m);
            for y in 0..4 {
                for x in 1..8 {
                    let (g, w) = (back.pixel(x, y), filtered.pixel(x, y));
                    assert!(g.iter().zip(&w).all(|(a, b)| (a - b).abs() < 1e-4), "{fmt:?} ({x}, {y}): {g:?} vs {w:?}");
                }
                // Where the mask hides the filter entirely its value is unknown: the base stands in.
                assert_eq!(back.pixel(0, y), base.pixel(0, y));
            }
            // So mixing the unmixed pixels gives the mixed ones back everywhere.
            let again = mix(&base, &back, &m);
            for y in 0..4 {
                for x in 0..8 {
                    assert!(again.pixel(x, y).iter().zip(mixed.pixel(x, y)).all(|(a, b)| (a - b).abs() < 1e-4));
                }
            }
        }
    }

    #[test]
    /// Recover signed HDR filter colors on opaque native surfaces while respecting hidden-mask endpoints.
    fn opaque_float_unmix_recovers_native_signed_hdr_colors_and_hidden_endpoints() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Multichannel] {
            let fmt = PixelFormat::new(mode, SampleType::F32, false);
            let b: Vec<f32> = (0..fmt.channels()).map(|c| if c % 2 == 0 { -2. } else { 4. }).collect();
            let f: Vec<f32> = (0..fmt.channels()).map(|c| if c % 2 == 0 { 6. } else { -3. }).collect();
            let (mut base, mut filtered) = (Surface::new(fmt), Surface::new(fmt));
            base.write_pixel(0, 0, &b);
            filtered.write_pixel(0, 0, &f);
            let before = (base.clone(), filtered.clone());
            for coverage in [0., HIDDEN / 2., HIDDEN, 0.25, 0.5, 1.] {
                let mut mask = LayerMask::reveal_all();
                mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[coverage]);
                let mixed = mix(&base, &filtered, &mask);
                let back = unmix(&base, &mixed, &mask);
                assert_eq!(back.format(), fmt);
                assert_eq!(back.pixel(0, 0), if coverage < HIDDEN { b.clone() } else { f.clone() }, "{mode:?} {coverage}");
                if coverage >= HIDDEN {
                    assert_eq!(mix(&base, &back, &mask).pixel(0, 0), mixed.pixel(0, 0), "{mode:?} {coverage}");
                } else {
                    // Hidden inverse values are unknown even when a stored mixed pixel is unrelated.
                    assert_eq!(unmix(&base, &filtered, &mask).pixel(0, 0), b);
                }
            }
            assert_eq!((base, filtered), before);
        }
    }

    #[test]
    /// Recover opaque F32 extremes and full-coverage bits, saturating inverse values outside the native range.
    fn opaque_float_unmix_handles_extremes_full_coverage_and_unrepresentable_inverse() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Multichannel] {
            let fmt = PixelFormat::new(mode, SampleType::F32, false);
            for (b, f, coverage) in [
                (f32::MAX, -f32::MAX, 0.5),
                (-f32::MAX, f32::MAX, 0.5),
                (f32::MAX, 2., 1.),
                (-f32::MAX, -3., 1.),
                (f32::MAX, f32::from_bits(1), 1.),
                (-f32::MAX, -f32::from_bits(1), 1.),
            ] {
                let (mut base, mut filtered) = (Surface::new(fmt), Surface::new(fmt));
                base.write_pixel(0, 0, &vec![b; fmt.channels()]);
                filtered.write_pixel(0, 0, &vec![f; fmt.channels()]);
                let mut mask = LayerMask::reveal_all();
                mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[coverage]);
                let mixed = mix(&base, &filtered, &mask);
                let recovered = unmix(&base, &mixed, &mask).pixel(0, 0);
                // The forward mix defines the observed data; the inverse must recover the
                // original filter, even when the base is far larger or has the opposite sign.
                for value in recovered {
                    assert!(value.is_finite(), "{mode:?} {b} {f} {coverage}");
                    assert_eq!(value.to_bits(), filtered.pixel(0, 0)[0].to_bits(), "{mode:?} {b} {f} {coverage}");
                }
            }
            let (mut base, mut mixed) = (Surface::new(fmt), Surface::new(fmt));
            base.write_pixel(0, 0, &vec![-f32::MAX; fmt.channels()]);
            mixed.write_pixel(0, 0, &vec![f32::MAX; fmt.channels()]);
            let mut mask = LayerMask::reveal_all();
            mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.5]);
            assert_eq!(unmix(&base, &mixed, &mask).pixel(0, 0), vec![f32::MAX; fmt.channels()]);
            base.write_pixel(0, 0, &vec![f32::MAX; fmt.channels()]);
            mixed.write_pixel(0, 0, &vec![-f32::MAX; fmt.channels()]);
            assert_eq!(unmix(&base, &mixed, &mask).pixel(0, 0), vec![-f32::MAX; fmt.channels()]);
        }
    }

    #[test]
    /// Round-trip alpha-bearing signed HDR colors through mask mixing and recovery at coverage endpoints.
    fn float_alpha_unmix_recovers_signed_hdr_colors_and_mask_endpoints() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab, ColorMode::Multichannel] {
            let fmt = PixelFormat::new(mode, SampleType::F32, true);
            let colors = fmt.channels() - 1;
            let mut b: Vec<f32> = (0..colors).map(|c| if c % 2 == 0 { -2. } else { 4. }).collect();
            let mut f: Vec<f32> = (0..colors).map(|c| if c % 2 == 0 { 6. } else { -3. }).collect();
            b.push(0.5);
            f.push(0.75);
            let mut base = Surface::new(fmt);
            let mut filtered = Surface::new(fmt);
            base.write_pixel(0, 0, &b);
            filtered.write_pixel(0, 0, &f);
            for coverage in [0., HIDDEN / 2., HIDDEN, 0.25, 0.5, 1.] {
                let mut mask = LayerMask::reveal_all();
                mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[coverage]);
                let mixed = mix(&base, &filtered, &mask);
                let back = unmix(&base, &mixed, &mask);
                let expected = if coverage < HIDDEN { &b } else { &f };
                let got = back.pixel(0, 0);
                assert!(got.iter().zip(expected).all(|(a, b)| a.is_finite() && (a - b).abs() < 0.001), "{mode:?} {coverage}: {got:?} vs {expected:?}");
                if coverage >= HIDDEN {
                    let again = mix(&base, &back, &mask).pixel(0, 0);
                    assert!(again.iter().zip(mixed.pixel(0, 0)).all(|(a, b)| (a - b).abs() < 1e-5), "{mode:?} {coverage}: {again:?}");
                }
            }
        }
    }

    #[test]
    /// Recover extreme and subnormal alpha-bearing F32 samples using wide arithmetic and exact full coverage.
    fn float_alpha_unmix_uses_wide_arithmetic_for_extremes_and_full_coverage() {
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
        for (b, f, coverage) in [
            ([f32::MAX, -f32::MAX, f32::MAX, 0.25], [-f32::MAX, f32::MAX, -f32::MAX, 0.75], 0.5),
            ([f32::MAX, -f32::MAX, f32::MAX, 1.], [2., -3., 4., f32::MIN_POSITIVE], 1.),
            ([f32::MAX, -f32::MAX, f32::MAX, 1.], [-4., 3., -2., f32::from_bits(1)], 1.),
        ] {
            let mut base = Surface::new(fmt);
            let mut filtered = Surface::new(fmt);
            base.write_pixel(0, 0, &b);
            filtered.write_pixel(0, 0, &f);
            let mut mask = LayerMask::reveal_all();
            mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[coverage]);
            let mixed = mix(&base, &filtered, &mask);
            let back = unmix(&base, &mixed, &mask).pixel(0, 0);
            assert!(back.iter().all(|v| v.is_finite()), "{b:?} {f:?}: {back:?}");
            assert_eq!(back, f);
        }
    }

    #[test]
    /// Bound impossible inverse colors, normalize recovered alpha and clear nonpositive coverage results.
    fn float_alpha_unmix_bounds_impossible_inverse_and_keeps_unit_alpha() {
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
        let mut base = Surface::new(fmt);
        let mut mixed = Surface::new(fmt);
        base.write_pixel(0, 0, &[-f32::MAX, f32::MAX, 0., 1.]);
        mixed.write_pixel(0, 0, &[f32::MAX, -f32::MAX, 2., 1.]);
        let mut mask = LayerMask::reveal_all();
        mask.surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[0.5]);
        assert_eq!(unmix(&base, &mixed, &mask).pixel(0, 0), vec![f32::MAX, -f32::MAX, 4., 1.]);
        mixed.write_pixel(0, 0, &[2., -3., 4., 2.]);
        assert_eq!(unmix(&base, &mixed, &LayerMask::reveal_all()).pixel(0, 0), vec![4., -6., 8., 1.]);
        mixed.write_pixel(0, 0, &[2., -3., 4., 0.]);
        assert_eq!(unmix(&base, &mixed, &mask).pixel(0, 0), vec![0.; 4]);
    }

    #[test]
    fn transparent_base_pixels_stay_empty() {
        // Photoshop's unfiltered cache carries white in transparent pixels; where the mask hides
        // the filter those must not become content of the stored pixels.
        let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
        let base = surface(fmt, |x, _| if x < 4 { vec![1.0, 1.0, 1.0, 0.0] } else { vec![0.3, 0.4, 0.5, 1.0] });
        let mut hidden = LayerMask::reveal_all();
        hidden.surface = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        let mixed = mix(&base, &base, &hidden);
        let back = unmix(&base, &mixed, &hidden);
        assert_eq!(back.pixel(1, 1), vec![0.0; 4]);
        assert_eq!(back.content_bounds().x0, 4);
    }

    #[test]
    fn mismatched_or_empty_surfaces_are_left_alone() {
        let rgba = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
        let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, true);
        let a = surface(rgba, |_, _| vec![0.5, 0.5, 0.5, 1.0]);
        let b = surface(gray, |_, _| vec![0.1, 1.0]);
        assert_eq!(mix(&a, &b, &ramp()), b);
        assert_eq!(unmix(&a, &b, &ramp()), b);
        let empty = Surface::new(rgba);
        assert_eq!(unmix(&empty, &empty, &ramp()), empty);
    }
}
