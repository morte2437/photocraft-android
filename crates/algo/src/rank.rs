//! Bounded native order-statistic filtering. Existing Median remains independent.
use crate::{Ctx, image::Image};
use photocraft_color::{PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Neighborhood {
    Disc,
    Square,
    Diamond,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Params {
    pub radius: u16,
    pub color_percentile: f32,
    pub alpha_percentile: f32,
    pub shape: Neighborhood,
    /// Absent selects all native channels; empty selects none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<Vec<bool>>,
}
pub use crate::coverage::Error;
const MAX_WORK: u64 = 128_000_000_000;
const TILE: i32 = 256;
/// Reject unsupported radius, nonfinite/out-of-range percentile values and oversized channel selections.
pub fn validate(p: &Params) -> Result<(), Error> {
    if p.radius == 0 || p.radius > 500 || [p.color_percentile, p.alpha_percentile].into_iter().any(|v| !v.is_finite() || !(0.0..=100.0).contains(&v)) {
        return Err(Error::InvalidParameter);
    }
    if p.channels.as_ref().is_some_and(|v| v.len() > 5) {
        return Err(Error::InvalidParameter);
    }
    Ok(())
}
/// Validate a positive bounded frame using widened arithmetic before sizing percentile workspaces.
fn dimensions(r: Rect) -> Result<(usize, usize), Error> {
    let w = i64::from(r.x1) - i64::from(r.x0);
    let h = i64::from(r.y1) - i64::from(r.y0);
    if w <= 0 || h <= 0 || w > 65536 || h > 65536 || w.checked_mul(h).is_none_or(|n| n > 64 * 1024 * 1024) {
        return Err(Error::InvalidGeometry);
    }
    Ok((w as usize, h as usize))
}
/// Allocate initialized scratch fallibly so a failed reservation becomes an allocation error.
fn zeroes<T: Clone>(n: usize, v: T) -> Result<Vec<T>, Error> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| Error::Allocation)?;
    out.resize(n, v);
    Ok(out)
}
/// Build each neighborhood row's support for square, diamond or the existing native Median disc footprint.
fn rows(p: &Params) -> Result<Vec<usize>, Error> {
    let r = usize::from(p.radius);
    let mut out = Vec::new();
    out.try_reserve_exact(2 * r + 1).map_err(|_| Error::Allocation)?;
    for y in 0..=2 * r {
        let d = y.abs_diff(r);
        out.push(match p.shape {
            Neighborhood::Square => r,
            Neighborhood::Diamond => r - d,
            // The existing native Median uses this discrete circular footprint.
            Neighborhood::Disc => ((r * r + r - d * d) as f64).sqrt().floor() as usize,
        });
    }
    Ok(out)
}
/// Price padded output tiles, halo scratch and order-statistic work before allocating or publishing output.
fn preflight(s: &Surface, p: &Params, area: Rect, selection: Option<&Surface>) -> Result<(), Error> {
    let (w, h) = dimensions(area)?;
    let channels = s.channels();
    if channels == 0 || channels > 5 {
        return Err(Error::InvalidGeometry);
    }
    let r = usize::from(p.radius);
    let hw = w.min(TILE as usize) + 2 * r;
    let hh = h.min(TILE as usize) + 2 * r;
    let halo = hw.checked_mul(hh).ok_or(Error::WorkspaceLimit)?;
    let scratch = halo
        .checked_mul(channels.checked_add(4).ok_or(Error::WorkspaceLimit)?)
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| n.checked_add(65536))
        .ok_or(Error::WorkspaceLimit)?;
    let tile_axis = |a: i32, b: i32| -> Result<usize, Error> {
        usize::try_from((i64::from(b) - 1).div_euclid(256) - i64::from(a).div_euclid(256) + 1).map_err(|_| Error::WorkspaceLimit)
    };
    let output = tile_axis(area.x0, area.x1)?
        .checked_mul(tile_axis(area.y0, area.y1)?)
        .and_then(|n| n.checked_mul(256 * 256 * s.format().bytes_per_pixel() + 128))
        .ok_or(Error::WorkspaceLimit)?;
    let tile_scratch = 256usize * 256 * (channels * 3 + selection.map_or(0, Surface::channels)) * 4;
    if output
        .checked_add(scratch)
        .and_then(|n| n.checked_add(tile_scratch))
        .and_then(|n| n.checked_add(s.tile_count().checked_mul(128)?))
        .is_none_or(|n| n > crate::coverage::MAX_WORKSPACE_BYTES)
    {
        return Err(Error::WorkspaceLimit);
    }
    let tiles = w.div_ceil(256).checked_mul(h.div_ceil(256)).ok_or(Error::WorkspaceLimit)? as u64;
    let log = (usize::BITS - halo.leading_zeros()) as u64;
    // Charge sorting, rank lookup, initial histogram and both Fenwick updates
    // plus order selection, rather than counting output samples alone.
    let work = tiles
        .checked_mul(channels as u64)
        .and_then(|n| {
            n.checked_mul((halo as u64).checked_mul(log)?.checked_mul(4)?.checked_add((w.min(256) * h.min(256)) as u64 * (4 * (2 * r + 1) + 2) as u64 * log)?)
        })
        .ok_or(Error::WorkspaceLimit)?;
    if work > MAX_WORK {
        return Err(Error::WorkspaceLimit);
    }
    Ok(())
}
// Fenwick counts permit independent percentile selection without scanning the value domain.
/// Update Fenwick counts with checked arithmetic, rejecting invalid support indices or underflow.
fn change(tree: &mut [u32], index: usize, add: bool) -> Result<(), Error> {
    let mut i = index.checked_add(1).ok_or(Error::InvalidGeometry)?;
    while i < tree.len() {
        let v = tree.get_mut(i).ok_or(Error::InvalidGeometry)?;
        *v = if add { v.checked_add(1) } else { v.checked_sub(1) }.ok_or(Error::InvalidGeometry)?;
        i = i.checked_add(i & i.wrapping_neg()).ok_or(Error::InvalidGeometry)?;
    }
    Ok(())
}
/// Locate an order statistic in Fenwick counts without scanning the sample-value domain.
fn select(tree: &[u32], rank: u32) -> Result<usize, Error> {
    let mut index = 0usize;
    let mut remaining = rank;
    let mut bit = 1usize;
    while bit < tree.len() {
        bit = bit.checked_mul(2).ok_or(Error::InvalidGeometry)?;
    }
    while bit > 0 {
        let next = index.checked_add(bit).ok_or(Error::InvalidGeometry)?;
        if let Some(&count) = tree.get(next)
            && count <= remaining
        {
            index = next;
            remaining -= count;
        }
        bit /= 2;
    }
    if index + 1 >= tree.len() {
        return Err(Error::InvalidGeometry);
    }
    Ok(index)
}
/// Compute per-channel neighborhood ranks with separate alpha policy and cancellable bounded scratch.
fn ranked(data: &[f32], width: usize, height: usize, channels: usize, p: &Params, alpha: bool, ctl: &Interrupt<'_>) -> Result<Vec<f32>, Error> {
    let r = usize::from(p.radius);
    let ww = width + 2 * r;
    let hh = height + 2 * r;
    let expected = ww.checked_mul(hh).and_then(|n| n.checked_mul(channels)).ok_or(Error::WorkspaceLimit)?;
    if data.len() != expected {
        return Err(Error::InvalidGeometry);
    }
    let row_widths = rows(p)?;
    let count = row_widths.iter().try_fold(0usize, |n, h| n.checked_add(2 * h + 1)).ok_or(Error::WorkspaceLimit)?;
    let mut output = zeroes(width * height * channels, 0f32)?;
    for y in 0..height {
        for x in 0..width {
            let original = (y + r) * ww * channels + (x + r) * channels;
            let target = (y * width + x) * channels;
            output
                .get_mut(target..target + channels)
                .ok_or(Error::InvalidGeometry)?
                .copy_from_slice(data.get(original..original + channels).ok_or(Error::InvalidGeometry)?);
        }
    }
    for channel in 0..channels {
        if p.channels.as_ref().is_some_and(|v| !v.get(channel).copied().unwrap_or(false)) {
            continue;
        }
        if ctl.cancelled() {
            return Err(Error::Cancelled);
        }
        let mut values = Vec::new();
        values.try_reserve_exact(ww * hh).map_err(|_| Error::Allocation)?;
        for (i, pixel) in data.chunks_exact(channels).enumerate() {
            if i.is_multiple_of(2048) && ctl.cancelled() {
                return Err(Error::Cancelled);
            }
            let v = *pixel.get(channel).ok_or(Error::InvalidGeometry)?;
            if !v.is_finite() {
                return Err(Error::NonFiniteSample);
            }
            values.push(v);
        }
        let mut sorted = Vec::new();
        sorted.try_reserve_exact(values.len()).map_err(|_| Error::Allocation)?;
        sorted.extend_from_slice(&values);
        sorted.sort_unstable_by(f32::total_cmp);
        sorted.dedup_by(|a, b| a.to_bits() == b.to_bits());
        if ctl.cancelled() {
            return Err(Error::Cancelled);
        }
        let mut ranks = Vec::new();
        ranks.try_reserve_exact(values.len()).map_err(|_| Error::Allocation)?;
        for (i, v) in values.iter().enumerate() {
            if i.is_multiple_of(2048) && ctl.cancelled() {
                return Err(Error::Cancelled);
            }
            ranks.push(sorted.binary_search_by(|a| a.total_cmp(v)).map_err(|_| Error::InvalidGeometry)?);
        }
        drop(values);
        let mut tree = zeroes(sorted.len() + 1, 0u32)?;
        let rank_at = |x: usize, y: usize| -> Result<usize, Error> {
            ranks.get(y.checked_mul(ww).and_then(|n| n.checked_add(x)).ok_or(Error::InvalidGeometry)?).copied().ok_or(Error::InvalidGeometry)
        };
        for (j, &half) in row_widths.iter().enumerate() {
            if ctl.cancelled() {
                return Err(Error::Cancelled);
            }
            for x in r - half..=r + half {
                change(&mut tree, rank_at(x, j)?, true)?;
            }
        }
        let percentile = if alpha && channel + 1 == channels { p.alpha_percentile } else { p.color_percentile };
        let target = (((count - 1) as f64 * f64::from(percentile) / 100.).floor() as usize).min(count - 1) as u32;
        for y in 0..height {
            if ctl.cancelled() {
                return Err(Error::Cancelled);
            }
            for step in 0..width {
                if step.is_multiple_of(8) && ctl.cancelled() {
                    return Err(Error::Cancelled);
                }
                let x = if y.is_multiple_of(2) { step } else { width - 1 - step };
                let cx = x + r;
                let cy = y + r;
                if y > 0 || step > 0 {
                    for (j, &half) in row_widths.iter().enumerate() {
                        let (gone, new) = if step == 0 {
                            let xx = cx + j - r;
                            ((xx, cy - 1 - half), (xx, cy + half))
                        } else if y.is_multiple_of(2) {
                            ((cx - 1 - half, cy + j - r), (cx + half, cy + j - r))
                        } else {
                            ((cx + 1 + half, cy + j - r), (cx - half, cy + j - r))
                        };
                        change(&mut tree, rank_at(gone.0, gone.1)?, false)?;
                        change(&mut tree, rank_at(new.0, new.1)?, true)?;
                    }
                }
                let value = *sorted.get(select(&tree, target)?).ok_or(Error::InvalidGeometry)?;
                *output.get_mut((y * width + x) * channels + channel).ok_or(Error::InvalidGeometry)? = value;
            }
        }
    }
    Ok(output)
}
/// Selection limits publication; the complete owner domain supplies repeated edge samples.
pub fn apply_with(s: &Surface, p: &Params, area: Rect, domain: Rect, selection: Option<&Surface>, ctl: &Interrupt<'_>) -> Result<Surface, Error> {
    validate(p)?;
    if p.channels.as_ref().is_some_and(|v| !v.is_empty() && v.len() != s.channels()) {
        return Err(Error::InvalidParameter);
    }
    // The owner only bounds edge sampling; preflight admits the clipped output and halo.
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
    if p.channels.as_ref().is_some_and(|v| v.iter().all(|selected| !*selected)) {
        if s.tile_count().checked_mul(128).is_none_or(|n| n > crate::coverage::MAX_WORKSPACE_BYTES) {
            return Err(Error::WorkspaceLimit);
        }
        return Ok(s.clone());
    }
    if area.is_empty() {
        return Ok(s.clone());
    }
    preflight(s, p, area, selection)?;
    let channels = s.channels();
    let r = i64::from(p.radius);
    let mut result = s.clone();
    let mut y = area.y0;
    while y < area.y1 {
        let y1 = (i64::from(y) + i64::from(TILE)).min(i64::from(area.y1)) as i32;
        let mut x = area.x0;
        while x < area.x1 {
            if ctl.cancelled() {
                return Err(Error::Cancelled);
            }
            let x1 = (i64::from(x) + i64::from(TILE)).min(i64::from(area.x1)) as i32;
            let tile = Rect::new(x, y, x1, y1);
            let (w, h) = dimensions(tile)?;
            let mut data = Vec::new();
            data.try_reserve_exact((w + 2 * r as usize) * (h + 2 * r as usize) * channels).map_err(|_| Error::Allocation)?;
            for yy in i64::from(y) - r..i64::from(y1) + r {
                if ctl.cancelled() {
                    return Err(Error::Cancelled);
                }
                let sy = yy.clamp(i64::from(domain.y0), i64::from(domain.y1) - 1) as i32;
                for xx in i64::from(x) - r..i64::from(x1) + r {
                    let sx = xx.clamp(i64::from(domain.x0), i64::from(domain.x1) - 1) as i32;
                    let mut pixel = [0.; 5];
                    s.read_pixel(sx, sy, &mut pixel);
                    data.extend_from_slice(pixel.get(..channels).ok_or(Error::InvalidGeometry)?);
                }
            }
            let mut filtered = ranked(&data, w, h, channels, p, s.format().alpha, ctl)?;
            drop(data);
            if let Some(selection) = selection {
                let mask = selection.try_read_region(tile).map_err(|_| Error::Allocation)?;
                if mask.iter().any(|v| !v.is_finite()) {
                    return Err(Error::NonFiniteSample);
                }
                let original = Image { rect: tile, ch: channels, data: s.try_read_region(tile).map_err(|_| Error::Allocation)? };
                crate::mix_selection(&mut filtered, tile, selection, &original, s.format().alpha);
                if let Some(mask) = &p.channels {
                    for (pixel, before) in filtered.chunks_exact_mut(channels).zip(original.data.chunks_exact(channels)) {
                        for (c, value) in pixel.iter_mut().enumerate() {
                            if !mask.get(c).copied().unwrap_or(false) {
                                *value = *before.get(c).ok_or(Error::InvalidGeometry)?;
                            }
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
/// Adapt raw float images to checked native percentile filtering, returning bounded identity data on rejected work.
pub(crate) fn run_image(src: &Image, out: Rect, ctx: &Ctx, p: &Params) -> Vec<f32> {
    let run = || -> Result<Vec<f32>, Error> {
        validate(p)?;
        let (w, h) = dimensions(src.rect)?;
        if src.data.len() > 64 * 1024 * 1024 {
            return Err(Error::WorkspaceLimit);
        }
        dimensions(out)?;
        let fmt = PixelFormat::new(ctx.mode, SampleType::F32, ctx.alpha);
        if src.ch != fmt.channels() || w.checked_mul(h).and_then(|n| n.checked_mul(src.ch)) != Some(src.data.len()) || out.intersect(&src.rect) != out {
            return Err(Error::InvalidGeometry);
        }
        let mut s = Surface::new(fmt);
        preflight(&s, p, src.rect, None)?;
        s.try_write_region(src.rect, &src.data).map_err(|_| Error::Allocation)?;
        apply_with(&s, p, out, src.rect, None, &Interrupt::NONE)?.try_read_region(out).map_err(|_| Error::Allocation)
    };
    run().unwrap_or_else(|_| {
        let mut v = Vec::new();
        if let (Ok((w, h)), Ok((sw, sh))) = (dimensions(out), dimensions(src.rect))
            && src.ch > 0
            && src.ch <= 5
            && sw.checked_mul(sh).and_then(|n| n.checked_mul(src.ch)) == Some(src.data.len())
            && out.intersect(&src.rect) == out
            && v.try_reserve_exact(w * h * src.ch).is_ok()
        {
            for y in out.y0..out.y1 {
                let start = ((i64::from(y) - i64::from(src.rect.y0)) as usize * sw + (i64::from(out.x0) - i64::from(src.rect.x0)) as usize) * src.ch;
                if let Some(row) = src.data.get(start..start + w * src.ch) {
                    v.extend_from_slice(row);
                }
            }
        }
        v
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::ColorMode;

    /// Sparse owner size must not change small native edits, repeated-edge samples or selection publication.
    #[test]
    fn small_percentile_edits_on_large_owners_match_clamped_oracle() {
        let interior = Rect::new(11, 13, 13, 15);
        let large = Rect::new(0, 0, 9000, 9000);
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let mut s = source(mode, depth, Rect::new(10, 12, 14, 16));
                s.fill_rect(Rect::new(0, 0, 3, 3), &vec![0.6; s.channels()]);
                s.fill_rect(Rect::new(-2, -2, 0, 0), &vec![1.; s.channels()]);
                let before = s.clone();
                for shape in [Neighborhood::Disc, Neighborhood::Square, Neighborhood::Diamond] {
                    let p = params(1, shape, 17., 83.);
                    for domain in [Rect::new(0, 0, 100, 100), large, Rect::new(0, 0, 90_000, 9000)] {
                        for area in [interior, Rect::new(-1, -1, 2, 2)] {
                            let clipped = area.intersect(&domain);
                            let mut expected = s.clone();
                            expected.try_write_region(clipped, &oracle(&s, &p, clipped, domain)).unwrap();
                            assert_eq!(apply_with(&s, &p, area, domain, None, &Interrupt::NONE).unwrap(), expected, "{mode:?}/{depth:?}/{shape:?}");
                        }
                    }
                    let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                    selection.fill_rect(interior, &[0.37]);
                    let selected = apply_with(&s, &p, interior, large, Some(&selection), &Interrupt::NONE).unwrap();
                    let small = apply_with(&s, &p, interior, Rect::new(0, 0, 100, 100), Some(&selection), &Interrupt::NONE).unwrap();
                    assert_eq!(selected, small);
                }
                assert_eq!(s, before);
            }
        }
    }

    /// Sampling bounds need no dense admission, while clipped output, padded workspace and cancellation retain their limits.
    #[test]
    fn large_percentile_owners_keep_geometry_work_and_cancellation_guards() {
        let s = Surface::with_default(PixelFormat::RGBA32F, &[0.25; 4]);
        let area = Rect::new(11, 13, 13, 15);
        let domain = Rect::new(0, 0, 9000, 9000);
        let p = params(1, Neighborhood::Square, 50., 50.);
        assert_eq!(s.tile_count(), 0);
        let shifted = Rect::new(11, 13, 9011, 9013);
        let out = apply_with(&s, &p, Rect::new(-100_000, -100_000, 13, 15), shifted, None, &Interrupt::NONE).unwrap();
        assert_eq!(out.read_region(area), vec![0.25; 16]);
        for invalid in [Rect::new(0, 0, 0, 9000), Rect::new(0, 1, 9000, 1), Rect::new(2, 0, 1, 9000), Rect::new(0, 2, 9000, 1)] {
            assert_eq!(apply_with(&s, &p, area, invalid, None, &Interrupt::NONE), Err(Error::InvalidGeometry));
        }
        assert_eq!(apply_with(&s, &p, domain, domain, None, &Interrupt::NONE), Err(Error::InvalidGeometry));
        let wide = Rect::new(0, 0, 90_000, 9000);
        assert_eq!(apply_with(&s, &p, Rect::new(0, 0, 65_537, 1), wide, None, &Interrupt::NONE), Err(Error::InvalidGeometry));
        let costly = params(500, Neighborhood::Square, 50., 50.);
        assert_eq!(apply_with(&s, &costly, Rect::new(0, 0, 8192, 8192), domain, None, &Interrupt::NONE), Err(Error::WorkspaceLimit));
        let cancelled = || true;
        assert_eq!(apply_with(&s, &p, area, domain, None, &Interrupt::cancel_only(&cancelled)), Err(Error::Cancelled));
        assert_eq!(apply_with(&s, &p, Rect::new(9001, 9001, 9003, 9003), domain, None, &Interrupt::NONE).unwrap(), s);
        assert_eq!(s.tile_count(), 0);
    }
    /// Build an all-channel percentile request with independently controlled color and alpha ranks.
    fn params(radius: u16, shape: Neighborhood, q: f32, a: f32) -> Params {
        Params { radius, shape, color_percentile: q, alpha_percentile: a, channels: None }
    }
    /// Create reproducible native-depth samples, including signed HDR values, for order-statistic tests.
    fn source(mode: ColorMode, depth: SampleType, r: Rect) -> Surface {
        let fmt = PixelFormat::new(mode, depth, true);
        let mut s = Surface::new(fmt);
        let mut state = 19u32;
        let data: Vec<f32> = (0..r.width() as usize * r.height() as usize * fmt.channels())
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                if depth == SampleType::F32 { (state % 113) as f32 / 17. - 2. } else { (state % 251) as f32 / 250. }
            })
            .collect();
        s.try_write_region(r, &data).unwrap();
        s
    }
    /// Sort an independently enumerated neighborhood to define expected ranks and clamped edge samples.
    fn oracle(s: &Surface, p: &Params, area: Rect, domain: Rect) -> Vec<f32> {
        let r = i32::from(p.radius);
        let mut out = Vec::new();
        for y in area.y0..area.y1 {
            for x in area.x0..area.x1 {
                for c in 0..s.channels() {
                    let mut v = Vec::new();
                    for dy in -r..=r {
                        for dx in -r..=r {
                            let include = match p.shape {
                                Neighborhood::Square => true,
                                Neighborhood::Diamond => dx.abs() + dy.abs() <= r,
                                Neighborhood::Disc => dx * dx + dy * dy <= r * r + r,
                            };
                            if include {
                                v.push(s.sample_channel((x + dx).clamp(domain.x0, domain.x1 - 1), (y + dy).clamp(domain.y0, domain.y1 - 1), c));
                            }
                        }
                    }
                    v.sort_unstable_by(f32::total_cmp);
                    let q = if c + 1 == s.channels() { p.alpha_percentile } else { p.color_percentile };
                    out.push(v[((v.len() - 1) as f64 * f64::from(q) / 100.).floor() as usize]);
                }
            }
        }
        out
    }
    #[test]
    /// Compare every neighborhood/rank combination with direct sorting on signed HDR samples and shifted bounds.
    fn independent_oracle_all_shapes_ranks_edges_and_hdr() {
        let r = Rect::new(-3, 5, 6, 12);
        let s = source(ColorMode::Rgb, SampleType::F32, r);
        for shape in [Neighborhood::Disc, Neighborhood::Square, Neighborhood::Diamond] {
            for radius in [1, 2, 4] {
                for q in [0., 17., 50., 83., 100.] {
                    let p = params(radius, shape, q, 100. - q);
                    assert_eq!(apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap().read_region(r), oracle(&s, &p, r, r), "{p:?}");
                }
            }
        }
    }
    #[test]
    /// Require the midpoint disc rank to match the established native Median across modes and depths.
    fn median50_matches_existing_native_median_all_models_depths() {
        let r = Rect::new(-3, -4, 12, 11);
        let out = Rect::new(0, 0, 9, 8);
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
                let s = source(mode, depth, r);
                let src = Image::read(&s, r);
                for radius in [1, 2, 3] {
                    let p = params(radius, Neighborhood::Disc, 50., 50.);
                    let expected = crate::noise::median(&src, out, f32::from(radius), None);
                    let got = apply_with(&s, &p, out, r, None, &Interrupt::NONE).unwrap().read_region(out);
                    assert_eq!(got, expected, "{mode:?} {depth:?} radius{radius}");
                }
            }
        }
    }
    #[test]
    /// Check endpoint percentiles against existing native extrema for an identical support footprint.
    fn extrema_match_native_minimum_maximum_same_footprint() {
        let r = Rect::new(-4, -4, 10, 10);
        let out = Rect::new(0, 0, 6, 6);
        let s = source(ColorMode::Rgb, SampleType::F32, r);
        let src = Image::read(&s, r);
        for shape in [Neighborhood::Disc, Neighborhood::Square] {
            for radius in [1u16, 2, 3] {
                for q in [0., 100.] {
                    let p = params(radius, shape, q, q);
                    let native_radius = if shape == Neighborhood::Disc { (f32::from(radius).powi(2) + f32::from(radius)).sqrt() } else { f32::from(radius) };
                    let preserve = if shape == Neighborhood::Disc { crate::Preserve::Roundness } else { crate::Preserve::Squareness };
                    let native = crate::other::min_max_preserve(&src, out, native_radius, q == 100., preserve);
                    assert_eq!(apply_with(&s, &p, out, r, None, &Interrupt::NONE).unwrap().read_region(out), native);
                }
            }
        }
    }
    #[test]
    /// Verify crop invariance and selection behavior while cancellation leaves the original surface unchanged.
    fn tiled_crop_invariance_selection_and_cancellation_are_atomic() {
        let r = Rect::new(-1, 3, 260, 8);
        let s = source(ColorMode::Rgb, SampleType::F32, r);
        let before = s.clone();
        let p = params(2, Neighborhood::Diamond, 29., 78.);
        let full = apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap();
        let crop = Rect::new(250, 4, 259, 7);
        assert_eq!(apply_with(&s, &p, crop, r, None, &Interrupt::NONE).unwrap().read_region(crop), full.read_region(crop));
        let zero = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        assert_eq!(apply_with(&s, &p, r, r, Some(&zero), &Interrupt::NONE).unwrap(), s);
        let cancel = || true;
        let ctl = Interrupt::cancel_only(&cancel);
        assert_eq!(apply_with(&s, &p, r, r, None, &ctl).unwrap_err(), Error::Cancelled);
        assert_eq!(s, before);
    }
    #[test]
    /// Ensure fractional coverage cannot modify unselected channels or an explicitly empty channel selection.
    fn selected_channels_preserve_unselected_bits_through_fractional_selection_and_empty_masks() {
        let r = Rect::new(-2, 3, 5, 8);
        let mut s = source(ColorMode::Rgb, SampleType::F32, r);
        s.write_pixel(0, 5, &[-0., std::f32::consts::PI, -4.125, 0.25]);
        let original = s.read_region(r);
        let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        selection.fill_rect(r, &[0.37]);
        let mut p = params(2, Neighborhood::Square, 100., 100.);
        p.channels = Some(vec![false, false, false, true]);
        let got = apply_with(&s, &p, r, r, Some(&selection), &Interrupt::NONE).unwrap().read_region(r);
        for (a, b) in got.as_chunks::<4>().0.iter().zip(original.as_chunks::<4>().0.iter()) {
            for c in 0..3 {
                assert_eq!(a[c].to_bits(), b[c].to_bits());
            }
        }
        assert!(got.as_chunks::<4>().0.iter().zip(original.as_chunks::<4>().0.iter()).any(|(a, b)| a[3] != b[3]));
        p.channels = Some(vec![true, true, true, false]);
        let got = apply_with(&s, &p, r, r, Some(&selection), &Interrupt::NONE).unwrap().read_region(r);
        for (a, b) in got.as_chunks::<4>().0.iter().zip(original.as_chunks::<4>().0.iter()) {
            assert_eq!(a[3].to_bits(), b[3].to_bits());
        }
        for mask in [vec![], vec![false; 4]] {
            p.channels = Some(mask);
            assert_eq!(apply_with(&s, &p, r, r, Some(&selection), &Interrupt::NONE).unwrap(), s);
        }
        p.channels = Some(vec![true; 3]);
        assert_eq!(apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap_err(), Error::InvalidParameter);
        p.channels = None;
        let json = serde_json::to_value(&p).unwrap();
        assert!(json.get("channels").is_none());
        let restored: Params = serde_json::from_value(json).unwrap();
        assert_eq!(restored, p);
    }
    #[test]
    /// Reject hostile parameters and oversized padded/CPU work before returning a modified surface.
    fn invalid_inputs_and_padded_tile_or_cpu_budget_fail_before_publication() {
        let r = Rect::new(0, 0, 2, 2);
        let s = source(ColorMode::Rgb, SampleType::F32, r);
        for p in [
            params(0, Neighborhood::Disc, 50., 50.),
            params(501, Neighborhood::Disc, 50., 50.),
            params(1, Neighborhood::Disc, f32::NAN, 50.),
            params(1, Neighborhood::Disc, 50., 101.),
        ] {
            assert_eq!(apply_with(&s, &p, r, r, None, &Interrupt::NONE).unwrap_err(), Error::InvalidParameter);
        }
        let huge = Rect::new(0, 0, 8192, 8192);
        assert_eq!(apply_with(&s, &params(500, Neighborhood::Square, 50., 50.), huge, huge, None, &Interrupt::NONE).unwrap_err(), Error::WorkspaceLimit);
        let thin = Rect::new(0, 0, 1, 65536);
        // Thin logical geometry still pays for256 full native output tiles. This
        // bounded case fits; interrupt while gathering its first halo, before publication.
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 8;
        assert_eq!(
            apply_with(&s, &params(400, Neighborhood::Square, 50., 50.), thin, thin, None, &Interrupt::cancel_only(&cancel)).unwrap_err(),
            Error::Cancelled
        );
        let mut bad = s.clone();
        bad.try_write_region(Rect::new(0, 0, 1, 1), &[f32::NAN, 0., 0., 1.]).unwrap();
        assert_eq!(apply_with(&bad, &params(1, Neighborhood::Disc, 50., 50.), r, r, None, &Interrupt::NONE).unwrap_err(), Error::NonFiniteSample);
    }
}
