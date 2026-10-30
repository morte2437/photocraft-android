//! Radius-limited selection morphology. Only the nonzero bounds plus a halo need distances;
//! the flat public API still requires a canvas-sized output. Canvas edges are not implicit
//! seeds: the engine supplies padding when Apply effect at canvas bounds is requested.

use super::distance::{EMPTY_DISTANCE, edt, edt_bounded};

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Expand,
    Contract,
    Border,
}

#[derive(Debug, PartialEq)]
struct Window {
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
}

fn window(m: &[f32], w: usize, h: usize, r: f32) -> Option<Window> {
    let mut b = Window { x0: w, x1: 0, y0: h, y1: 0 };
    for (y, row) in m.chunks_exact(w).enumerate() {
        if let Some(x) = row.iter().position(|v| *v != 0.0) {
            b.x0 = b.x0.min(x);
            b.y0 = b.y0.min(y);
            b.y1 = y + 1;
            b.x1 = b.x1.max(row.iter().rposition(|v| *v != 0.0).unwrap_or(x) + 1);
        }
    }
    if b.x1 == 0 {
        // Preserve the general EDT's finite no-seed sentinel even for library callers
        // passing radii far beyond the command's 500 px limit.
        return (r >= EMPTY_DISTANCE).then_some(Window { x0: 0, x1: w, y0: 0, y1: h });
    }
    // Coverage reaches r + 1 centres; the extra pixel also supplies real outside seeds
    // for erosion. Saturating arithmetic handles radii larger than the canvas.
    let halo = (r.ceil() as usize).saturating_add(1);
    Some(Window { x0: b.x0.saturating_sub(halo), x1: b.x1.saturating_add(halo).min(w), y0: b.y0.saturating_sub(halo), y1: b.y1.saturating_add(halo).min(h) })
}

pub(super) fn modify(m: &[f32], w: usize, h: usize, r: f32, kind: Kind) -> Vec<f32> {
    if w == 0 || h == 0 || w.checked_mul(h) != Some(m.len()) || m.len() > isize::MAX as usize / 4 || !r.is_finite() || r < 0.0 {
        return Vec::new();
    }
    // The window includes every nonzero sample, including fractional coverage, so all
    // pixels beyond it are zero for every operation. No canvas-sized copy is needed.
    let mut out = vec![0.0; m.len()];
    if matches!(kind, Kind::Border) && r == 0.0 {
        return out;
    }
    let Some(b) = window(m, w, h, r) else { return out };
    let (cw, ch) = (b.x1 - b.x0, b.y1 - b.y0);
    let mut seeds = Vec::with_capacity(cw * ch);
    for row in m.chunks_exact(w).skip(b.y0).take(ch) {
        if let Some(span) = row.get(b.x0..b.x1) {
            seeds.extend(span.iter().map(|v| match kind {
                // Do not replace this with v < 0.5: the old complement includes ties,
                // including values that round to 0.5 after subtraction.
                Kind::Contract => 1.0 - v >= 0.5,
                _ => *v >= 0.5,
            }));
        }
    }
    // A uniform binary mask has no edge inside the supplied canvas. Preserve the
    // finite no-seed sentinel behavior for enormous library radii, but skip both
    // distance planes and coverage passes at normal radii (including Select All).
    if matches!(kind, Kind::Border) && r < EMPTY_DISTANCE && seeds.first().is_some_and(|first| seeds.iter().all(|seed| seed == first)) {
        return out;
    }
    let distance = |seeds: &[bool]| {
        if r <= 5.0 { edt_bounded(seeds, cw, ch, r + 1.0) } else { edt(seeds, cw, ch) }
    };
    let passes = if matches!(kind, Kind::Border) { 2 } else { 1 };
    for pass in 0..passes {
        // Drop each distance plane before the next pass. Border never keeps two float
        // distance planes or two seed masks alive together.
        let d = distance(&seeds);
        for (y, (src, dst)) in m.chunks_exact(w).zip(out.chunks_exact_mut(w)).skip(b.y0).take(ch).enumerate() {
            let (Some(src), Some(dst), Some(dist)) = (src.get(b.x0..b.x1), dst.get_mut(b.x0..b.x1), d.get(y * cw..(y + 1) * cw)) else { continue };
            for ((v, out), d) in src.iter().zip(dst).zip(dist) {
                *out = match kind {
                    Kind::Expand => v.max((r + 1.0 - d).clamp(0.0, 1.0)),
                    Kind::Contract => 1.0 - (1.0 - v).max((r + 1.0 - d).clamp(0.0, 1.0)),
                    Kind::Border if (*v >= 0.5) == (pass == 1) => (1.0 - (d - 1.0) / r).clamp(0.0, 1.0),
                    Kind::Border => *out,
                };
            }
        }
        if pass + 1 < passes {
            for seed in &mut seeds {
                *seed = !*seed;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
