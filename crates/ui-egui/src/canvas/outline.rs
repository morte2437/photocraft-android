//! Selection outlines: boundary segments of a coverage mask (threshold 0.5), merged into long runs,
//! drawn as animated marching ants.
//!
//! The outline is traced at display resolution: `outline_scaled` samples every `step`-th pixel, so
//! a 36 MP selection at 12% zoom costs a ~0.5 MP scan instead of reading the whole mask.

use photocraft_geom::Rect;
use photocraft_raster::Surface;

/// A boundary segment in document pixel coordinates (axis-aligned).
pub type Segment = ([i32; 2], [i32; 2]);

/// Bound a whole-selection trace, including offscreen pixels carried into view by a transform.
/// Include grid alignment and the two padding cells in the budget; ordinary traces keep their step.
pub(crate) fn bounded_step(bounds: Rect, mut step: u32) -> u32 {
    step = step.max(1);
    let cells = |step: u32| {
        let s = i64::from(step);
        let axis = |lo: i32, hi: i32| ((i64::from(hi) + s - 1).div_euclid(s) - i64::from(lo).div_euclid(s) + 2).max(0) as u64;
        axis(bounds.x0, bounds.x1).saturating_mul(axis(bounds.y0, bounds.y1))
    };
    while cells(step) > 1_048_576 {
        step = step.saturating_mul(2);
    }
    step
}

/// The trace step for a view of `device_zoom` device px per document px, of a mask reduced by `k`:
/// about one sample per device pixel, a power of two in 1..=64.
pub(crate) fn view_step(device_zoom: f32, k: u32) -> u32 {
    (1.0 / (device_zoom * k.max(1) as f32).max(1e-3)).log2().floor().exp2().clamp(1.0, 64.0) as u32
}

/// The part of a mask reduced by `k` to trace for the document rectangle `[x0, y0, x1, y1]` in
/// view: quantised to 256 cells of `step` so small pans keep it, and padded by that so the cut
/// edges of a trace fall outside the view.
pub(crate) fn view_region(view: [f64; 4], k: u32, step: u32) -> Rect {
    let k = f64::from(k.max(1));
    let q = 256 * step.clamp(1, 64) as i32;
    // The view centre can be any finite number. Past ±2³⁰ there is no document to outline, and
    // the padding (at most 2 · 256 · 64) can't overflow i32 from there.
    let px = |v: f64| v.clamp(-f64::from(1 << 30), f64::from(1 << 30)) as i32;
    let (x0, x1) = (view[0].min(view[2]) / k, view[0].max(view[2]) / k);
    let (y0, y1) = (view[1].min(view[3]) / k, view[1].max(view[3]) / k);
    Rect::new(
        px(x0.floor()).div_euclid(q) * q - q,
        px(y0.floor()).div_euclid(q) * q - q,
        (px(x1.ceil()).div_euclid(q) + 2) * q,
        (px(y1.ceil()).div_euclid(q) + 2) * q,
    )
}

/// Extract the boundary of `mask` (> 0.5 = selected) as merged horizontal and vertical segments.
pub fn outline(mask: &Surface, bounds: Rect) -> Vec<Segment> {
    outline_scaled(mask, bounds, 1)
}

/// Like [`outline`], sampling one pixel per `step`×`step` block (segments stay in document space).
pub fn outline_scaled(mask: &Surface, bounds: Rect, step: u32) -> Vec<Segment> {
    outline_scaled_impl(mask, bounds, step, true)
}

/// Trace without waiting for Rayon's shared worker pool. The UI uses this while background jobs
/// are active, because a long-running filter can occupy all the workers (#1554).
pub fn outline_scaled_serial(mask: &Surface, bounds: Rect, step: u32) -> Vec<Segment> {
    outline_scaled_impl(mask, bounds, step, false)
}

fn outline_scaled_impl(mask: &Surface, bounds: Rect, step: u32, parallel: bool) -> Vec<Segment> {
    if bounds.is_empty() {
        return Vec::new();
    }
    let step = step.max(1) as i32;
    // Grid of block samples, padded by one cell so edges at the bounds are detected.
    let gx0 = bounds.x0.div_euclid(step) - 1;
    let gy0 = bounds.y0.div_euclid(step) - 1;
    let gx1 = (bounds.x1 + step - 1).div_euclid(step) + 1;
    let gy1 = (bounds.y1 + step - 1).div_euclid(step) + 1;
    let (w, h) = ((gx1 - gx0) as usize, (gy1 - gy0) as usize);
    let mut grid = vec![false; w * h];
    let half = step / 2;
    let sample_row = |gy: usize, row: &mut [bool]| {
        let y0 = (gy0 + gy as i32) * step;
        if y0 >= bounds.y1 || y0 + step <= bounds.y0 {
            return;
        }
        // A coarse cell can overlap a one-pixel-wide selection even when its centre misses it.
        // Sample the overlapping edge; padding cells still stay outside the mask.
        let y = (y0 + half).clamp(bounds.y0, bounds.y1 - 1);
        let mut vals = vec![0.0f32; row.len()];
        mask.sample_row_strided(y, gx0 * step + half, step, 0, &mut vals);
        for (gx, (cell, v)) in row.iter_mut().zip(vals).enumerate() {
            let x = (gx0 + gx as i32) * step + half;
            *cell = if x >= bounds.x0 && x < bounds.x1 {
                v > 0.5
            } else {
                x - half < bounds.x1 && x - half + step > bounds.x0 && mask.sample_channel(x.clamp(bounds.x0, bounds.x1 - 1), y, 0) > 0.5
            };
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    if parallel {
        use rayon::prelude::*;
        grid.par_chunks_mut(w).enumerate().for_each(|(gy, row)| sample_row(gy, row));
    } else {
        grid.chunks_mut(w).enumerate().for_each(|(gy, row)| sample_row(gy, row));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = parallel;
        grid.chunks_mut(w).enumerate().for_each(|(gy, row)| sample_row(gy, row));
    }
    let inside = |x: usize, y: usize| grid[y * w + x];
    // Grid → document coordinates, clamped so coarse cells never draw outside the selection bounds.
    let r = Rect::new(gx0 * step, gy0 * step, gx1 * step, gy1 * step);
    let (cx, cy) = (|v: i32| v.clamp(bounds.x0, bounds.x1), |v: i32| v.clamp(bounds.y0, bounds.y1));
    let mut segs = Vec::new();
    // Horizontal edges between rows y-1 and y, merged along x.
    for y in 1..h {
        let mut run: Option<usize> = None;
        for x in 0..=w {
            let edge = x < w && inside(x, y) != inside(x, y - 1);
            match (edge, run) {
                (true, None) => run = Some(x),
                (false, Some(x0)) => {
                    let yy = cy(r.y0 + y as i32 * step);
                    segs.push(([cx(r.x0 + x0 as i32 * step), yy], [cx(r.x0 + x as i32 * step), yy]));
                    run = None;
                }
                _ => {}
            }
        }
    }
    // Vertical edges between columns x-1 and x, merged along y.
    for x in 1..w {
        let mut run: Option<usize> = None;
        for y in 0..=h {
            let edge = y < h && inside(x, y) != inside(x - 1, y);
            match (edge, run) {
                (true, None) => run = Some(y),
                (false, Some(y0)) => {
                    let xx = cx(r.x0 + x as i32 * step);
                    segs.push(([xx, cy(r.y0 + y0 as i32 * step)], [xx, cy(r.y0 + y as i32 * step)]));
                    run = None;
                }
                _ => {}
            }
        }
    }
    segs
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;

    #[test]
    fn whole_transform_trace_has_a_bounded_grid() {
        assert_eq!(bounded_step(Rect::new(0, 0, 200, 150), 1), 1);
        assert_eq!(bounded_step(Rect::new(0, 0, 6000, 6000), 1), 8);
        for bounds in [Rect::new(-50_001, -50_003, 50_000, 50_000), Rect::new(-1_000_000, 0, 1_000_000, 1)] {
            let step = i64::from(bounded_step(bounds, 1));
            let w = (i64::from(bounds.x1) + step - 1).div_euclid(step) - i64::from(bounds.x0).div_euclid(step) + 2;
            let h = (i64::from(bounds.y1) + step - 1).div_euclid(step) - i64::from(bounds.y0).div_euclid(step) + 2;
            assert!(w * h <= 1_048_576, "{bounds:?}: {w} × {h}");
        }
        // Both distant pieces survive the coarse trace: it covers the whole selection, not the viewport.
        let mut mask = Surface::new(PixelFormat::GRAY8);
        mask.fill_rect(Rect::new(-50_000, -50_000, -49_488, -49_488), &[1.0]);
        mask.fill_rect(Rect::new(49_488, 49_488, 50_000, 50_000), &[1.0]);
        let bounds = mask.content_bounds();
        let lines = outline_scaled_serial(&mask, bounds, bounded_step(bounds, 1));
        assert!(lines.iter().any(|(a, _)| a[0] < -49_000));
        assert!(lines.iter().any(|(a, _)| a[0] > 49_000));
        let mut thin = Surface::new(PixelFormat::GRAY8);
        thin.fill_rect(Rect::new(-1_000_000, 0, -999_488, 1), &[1.0]);
        thin.fill_rect(Rect::new(999_488, 0, 1_000_000, 1), &[1.0]);
        let bounds = thin.content_bounds();
        let lines = outline_scaled_serial(&thin, bounds, bounded_step(bounds, 1));
        assert!(lines.iter().any(|(a, _)| a[0] < -999_000), "thin left piece remains visible");
        assert!(lines.iter().any(|(a, _)| a[0] > 999_000), "thin right piece remains visible");
    }

    #[test]
    fn rectangle_has_four_edges() {
        let mut m = Surface::new(PixelFormat::GRAY8);
        m.fill_rect(Rect::new(2, 3, 10, 7), &[1.0]);
        let s = outline(&m, m.content_bounds());
        assert_eq!(s.len(), 4, "{s:?}");
        assert!(s.contains(&([2, 3], [10, 3])));
        assert!(s.contains(&([2, 7], [10, 7])));
        assert!(s.contains(&([2, 3], [2, 7])));
        assert!(s.contains(&([10, 3], [10, 7])));
    }

    #[test]
    fn hole_adds_inner_edges() {
        let mut m = Surface::new(PixelFormat::GRAY8);
        m.fill_rect(Rect::new(0, 0, 10, 10), &[1.0]);
        m.fill_rect(Rect::new(4, 4, 6, 6), &[0.0]);
        assert_eq!(outline(&m, m.content_bounds()).len(), 8);
    }

    #[test]
    fn scaled_outline_approximates_full_resolution() {
        let mut m = Surface::new(PixelFormat::GRAY8);
        m.fill_rect(Rect::new(8, 8, 72, 40), &[1.0]);
        let s = outline_scaled(&m, m.content_bounds(), 4);
        assert_eq!(s.len(), 4, "{s:?}");
        assert!(s.contains(&([8, 8], [72, 8])));
        assert!(s.contains(&([72, 8], [72, 40])));
        // Unaligned bounds are clamped, never drawn outside the selection.
        let mut m = Surface::new(PixelFormat::GRAY8);
        m.fill_rect(Rect::new(3, 5, 30, 21), &[1.0]);
        for seg in outline_scaled(&m, m.content_bounds(), 8) {
            for p in [seg.0, seg.1] {
                assert!(p[0] >= 3 && p[0] <= 30 && p[1] >= 5 && p[1] <= 21, "{seg:?}");
            }
        }
    }

    #[test]
    fn serial_outline_matches_parallel_for_scales_and_clipped_bounds() {
        let mut mask = Surface::new(PixelFormat::GRAY8);
        mask.fill_rect(Rect::new(3, 5, 110, 90), &[1.0]);
        mask.fill_rect(Rect::new(17, 21, 45, 65), &[0.0]);
        mask.fill_rect(Rect::new(73, 9, 88, 42), &[0.0]);
        for step in [1, 2, 4, 8, 16] {
            for bounds in [mask.content_bounds(), Rect::new(13, 19, 91, 77), Rect::new(200, 200, 240, 240)] {
                let parallel = outline_scaled(&mask, bounds, step);
                let serial = outline_scaled_serial(&mask, bounds, step);
                assert_eq!(serial, parallel, "outline differs at step {step}, bounds {bounds:?}");
            }
        }
    }

    #[test]
    fn view_step_and_region_quantise_and_survive_hostile_views() {
        assert_eq!(view_step(1.0, 1), 1);
        assert_eq!(view_step(0.25, 1), 4);
        assert_eq!(view_step(0.5, 2), 1, "a copy reduced by 2 seen at 50 % is at its own size");
        assert_eq!(view_region([10.0, 20.0, 300.0, 200.0], 1, 1), Rect::new(-256, -256, 768, 512));
        assert_eq!(view_region([0.0, 0.0, 1024.0, 512.0], 2, 1), Rect::new(-256, -256, 1024, 768));
        for zoom in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e9, 1e-30] {
            for k in [0, 1, 3, u32::MAX] {
                assert!((1..=64).contains(&view_step(zoom, k)), "zoom {zoom}, k {k}");
            }
        }
        for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e30, -1e30] {
            for step in [0, 1, 64, u32::MAX] {
                let r = view_region([v, -v, v, 1e30], 1, step);
                assert!(r.x0 <= r.x1 && r.y0 <= r.y1, "{v}, {step}: {r:?}");
            }
        }
    }

    #[test]
    fn empty_mask_has_no_outline() {
        let m = Surface::new(PixelFormat::GRAY8);
        assert!(outline(&m, m.content_bounds()).is_empty());
    }
}
