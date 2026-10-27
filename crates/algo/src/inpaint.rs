//! Hole filling for Content-Aware Fill and Spot Healing: patch-based completion, proximity match
//! and texture synthesis.
//!
//! * [`complete`], content-aware fill: the multi-scale "space-time completion" EM scheme of
//!   Y. Wexler, E. Shechtman, M. Irani, *Space-Time Completion of Video*, IEEE TPAMI 2007, with the
//!   nearest-neighbour field computed by C. Barnes, E. Shechtman, A. Finkelstein, D. B. Goldman,
//!   *PatchMatch: A Randomized Correspondence Algorithm for Structural Image Editing*, SIGGRAPH 2009
//!   (random init, propagation, exponentially shrinking random search). Each EM iteration finds, for
//!   every patch overlapping the hole, its most similar source patch, then re-estimates hole pixels
//!   by a similarity-weighted vote of the overlapping matches. Coarse to fine over an image pyramid
//!   (down to where the hole is about four patches across): the coarsest level starts by filling
//!   the hole from its edge inwards, ring by ring, each pixel from the patch that best matches what
//!   is known around it (rather than from a smooth membrane, which draws the fill towards flat
//!   areas); every finer level starts from the coarser level's matches scaled up and voted with the
//!   finer level's own pixels, so the texture stays sharp. Where neighbouring patches agree on their
//!   source (the usual case), the vote copies the source pixels exactly; only along the seams
//!   between copied pieces does it blend.
//!
//!   A thin hole (a selected wire, string or hair: long, but only a few patches thick) is
//!   measured by its thickness, not its bounds (#1366): the pyramid stops where it is still about
//!   two patches thick. That level is big, so the onion peel starts it only where its exhaustive
//!   search stays affordable; otherwise a membrane across the hole does, which over so short a
//!   distance is the surroundings themselves. Shrunk to where its bounds are four patches across,
//!   a long thin hole swallows its surroundings, the matches found there copy whatever happens to
//!   look like the blur (a dark blotch, or a piece of another string), and the finer levels,
//!   which only refine those matches, keep them: the removed object's outline stays visible.
//!
//!   This is what Photoshop's Content-Aware Fill does, measured as a black box on Photoshop 25.4
//!   (see `content_aware`): with colour adaptation off its fill is pieces of the image copied pixel
//!   for pixel, typically 15–40 px across, joined by seams a few pixels wide; with colour
//!   adaptation the copies come with a gain and bias. That is the patch transform of S. Darabi,
//!   E. Shechtman, C. Barnes, D. B. Goldman, P. Sen, *Image Melding: Combining Inconsistent Images
//!   using Patch-based Synthesis*, SIGGRAPH 2012: each source patch may be scaled and offset per
//!   channel to match its target, within limits ([`Adapt`]), and is voted in adjusted.
//! * [`best_offset`], proximity match: the single displacement whose surrounding ring best matches the
//!   ring around the hole (sum of squared differences), i.e. an automatic Healing Brush source.
//! * [`synthesize`], create texture: patch-based texture synthesis in the style of A. Efros,
//!   W. Freeman, *Image Quilting for Texture Synthesis and Transfer*, SIGGRAPH 2001 (blocks picked from
//!   random candidates by overlap error, joined along minimum-error boundary cuts, so the texture is
//!   copied, never averaged).
//!
//! All functions take interleaved `w × h × ch` normalised floats (any colour model/depth) and a
//! `w × h` hole mask, and return a full buffer equal to the input outside the hole. They are
//! deterministic (fixed-seed hash RNG, results independent of thread count).

use std::borrow::Cow;

use crate::par;
use crate::poisson::membrane_fill;

mod proximity_integral;
use proximity_integral::HoleIntegral;

mod common;
mod solver;
mod sources;
mod synthesis;

#[cfg(test)]
use common::Bounds;
pub(crate) use common::Rng;
pub use common::{Adapt, Px};
use common::{Level, SOURCE_MOMENTS_MAX};
use solver::Solver;
pub(crate) use solver::fit;
#[cfg(test)]
use solver::least_sum;
use sources::{Coarser, Sources, THIN_RATIO, downsample, hole_bounds, hole_thickness, peel_affordable, targets_around, valid_sources, window_count};
#[cfg(test)]
use sources::{hole_bbox, integral};
use synthesis::onion_peel;
pub use synthesis::{best_offset, synthesize};

/// Parameters for [`complete`].
#[derive(Clone, Debug, PartialEq)]
pub struct CompleteParams {
    /// Patch radius (patches are `2r+1` square).
    pub patch_radius: usize,
    /// EM iterations at the coarsest level (finer levels use fewer, down to `min_em_iters`).
    pub em_iters: usize,
    pub min_em_iters: usize,
    /// PatchMatch passes per EM iteration.
    pub pm_iters: usize,
    /// Levels where the hole is wider than this (in that level's pixels) only *refine* the
    /// scaled-up match field (one local PatchMatch pass and one vote), which keeps big holes
    /// tractable: the structure is decided at coarse scale, fine levels just add detail.
    pub refine_extent: usize,
    /// Colour adaptation of the copies.
    pub adapt: Adapt,
    pub seed: u64,
}

impl Default for CompleteParams {
    fn default() -> Self {
        Self { patch_radius: 3, em_iters: 10, min_em_iters: 3, pm_iters: 2, refine_extent: 192, adapt: Adapt::NONE, seed: 1 }
    }
}

/// Content-aware completion of `hole` (see module docs), copying from any pixel outside it. Returns
/// `None` when there is no fully-known patch to copy from (the caller can fall back to
/// [`membrane_fill`]).
pub fn complete(w: usize, h: usize, ch: usize, img: &[f32], hole: &[bool], p: &CompleteParams) -> Option<Vec<f32>> {
    // Never cancelled, so never `Err`.
    complete_with(w, h, ch, img, hole, p, &photocraft_raster::Interrupt::NONE).unwrap_or(None)
}

/// [`complete`] that can be cancelled (checked per EM iteration, PatchMatch pass and row band)
/// and reports progress (by pyramid level). `Err(Cancelled)` when `ctl` was cancelled.
pub fn complete_with(
    w: usize,
    h: usize,
    ch: usize,
    img: &[f32],
    hole: &[bool],
    p: &CompleteParams,
    ctl: &photocraft_raster::Interrupt,
) -> Result<Option<Vec<f32>>, photocraft_raster::Cancelled> {
    assert_eq!(hole.len(), w * h);
    let px: Vec<Px> = hole.iter().map(|h| if *h { Px::Hole } else { Px::Source }).collect();
    complete_masked(w, h, ch, img, &px, p, ctl)
}

/// [`complete_with`] with every pixel's role given ([`Px`]): the hole, the context, what may be
/// copied, and what isn't image at all.
pub fn complete_masked(
    w: usize,
    h: usize,
    ch: usize,
    img: &[f32],
    px: &[Px],
    p: &CompleteParams,
    ctl: &photocraft_raster::Interrupt,
) -> Result<Option<Vec<f32>>, photocraft_raster::Cancelled> {
    Ok(complete_cow(w, h, ch, Cow::Borrowed(img), px, p, ctl)?.ok())
}

/// [`complete_masked`] that fills `img` itself instead of a copy of it (only the hole changes):
/// `Err(img)`, untouched, when there is nothing to copy from.
pub fn complete_masked_owned(
    w: usize,
    h: usize,
    ch: usize,
    img: Vec<f32>,
    px: &[Px],
    p: &CompleteParams,
    ctl: &photocraft_raster::Interrupt,
) -> Result<Result<Vec<f32>, Vec<f32>>, photocraft_raster::Cancelled> {
    Ok(complete_cow(w, h, ch, Cow::Owned(img), px, p, ctl)?.map_err(Cow::into_owned))
}

/// The completion, on the caller's pixels or a copy of them: `Err` gives them back when there is
/// nothing to copy from.
fn complete_cow<'a>(
    w: usize,
    h: usize,
    ch: usize,
    img: Cow<'a, [f32]>,
    px: &'a [Px],
    p: &CompleteParams,
    ctl: &photocraft_raster::Interrupt,
) -> Result<Result<Vec<f32>, Cow<'a, [f32]>>, photocraft_raster::Cancelled> {
    assert_eq!(img.len(), w * h * ch);
    assert_eq!(px.len(), w * h);
    let Some(hb0) = hole_bounds(w, h, px) else { return Ok(Ok(img.into_owned())) };
    if ch == 0 {
        return Ok(Ok(Vec::new()));
    }
    let r = p.patch_radius.max(1);
    let psz = 2 * r + 1;
    // Pyramid: shrink until the hole is about four patches across (a deeper pyramid decides the
    // structure where the texture has averaged out, and fills flat). A thin hole stops while it
    // is still about two patches thick: through it, not along it, is what counts (#1366).
    let mut levels = vec![Level { w, h, img, px: Cow::Borrowed(px) }];
    let mut boxes = vec![hb0];
    let mut ext = hb0.width().max(hb0.height());
    let thickness = hole_thickness(w, hb0, px);
    let thin = thickness.saturating_mul(THIN_RATIO) <= ext;
    if thin {
        ext = thickness;
    }
    let mut exts = vec![ext];
    while let Some(l) = levels.last() {
        ctl.check()?;
        if ext <= if thin { 2 * psz } else { 4 * psz } || l.w / 2 < 3 * psz || l.h / 2 < 3 * psz {
            break;
        }
        let d = downsample(l, ch);
        levels.push(d);
        boxes.push(boxes[boxes.len() - 1].half());
        ext = ext.div_ceil(2);
        exts.push(ext);
    }
    let nlev = levels.len();
    // The coarser level's result.
    let mut prev: Option<Coarser> = None;
    let total_px: usize = levels.iter().map(|l| l.w * l.h).sum();
    let mut done_px = 0usize;
    let ri = r as i32;
    for li in (0..nlev).rev() {
        ctl.check()?;
        let Level { w: lw, h: lh, img: limg, px: lpx } = std::mem::replace(&mut levels[li], Level::EMPTY);
        let hb = boxes[li];
        let tb = hb.grow(r, lw, lh);
        let valid = valid_sources(lw, lh, r, &lpx, ctl).ok_or(photocraft_raster::Cancelled)?;
        let sources = Sources::new(&valid, lw);
        // Nothing to copy from at the finest level: the caller falls back (and gets its pixels
        // back as they were).
        if li == 0 && sources.len() == 0 {
            return Ok(Err(limg));
        }
        // The level's own pixels to fill (at level 0 the caller's, or a copy made in parallel).
        let mut limg = match limg {
            Cow::Owned(v) => v,
            Cow::Borrowed(b) => {
                const CHUNK: usize = 1 << 16;
                let mut v = vec![0.0f32; b.len()];
                par::chunks_mut(&mut v, CHUNK, true, |i, d| d.copy_from_slice(&b[i * CHUNK..i * CHUNK + d.len()]));
                v
            }
        };
        let targets = targets_around(lw, r, &lpx, hb, tb);
        ctl.check()?;
        // Initialise the hole: at the coarsest level peeled from its edge inwards (or a membrane
        // where nothing can be copied), else the coarser level's pixels, refined below by a vote of
        // its scaled-up matches.
        let coarser = prev.take();
        match &coarser {
            Some(c) => {
                let (pw, ph) = (c.w, c.h);
                for y in hb.y0..hb.y1 {
                    for x in hb.x0..hb.x1 {
                        if lpx[y * lw + x] == Px::Hole {
                            let (sx, sy) = ((x / 2).min(pw - 1), (y / 2).min(ph - 1));
                            let (d, s) = ((y * lw + x) * ch, (sy * pw + sx) * ch);
                            limg[d..d + ch].copy_from_slice(&c.img[s..s + ch]);
                        }
                    }
                }
            }
            // A thin hole, when peeling it would take too long, takes the membrane below: a few
            // patches thick here, a membrane across it is its surroundings.
            None if sources.len() > 0 && (!thin || peel_affordable(lw, &lpx, hb, sources.len())) => {
                onion_peel(lw, lh, ch, ri, &mut limg, &lpx, hb, &sources.all(&valid, lw), ctl)?
            }
            None => {
                let mut unknown: Vec<bool> = lpx.iter().map(|k| matches!(k, Px::Hole | Px::Void)).collect();
                // Across a thin hole the membrane starts a patch radius out, past the
                // anti-aliased fringe or halo that a selection drawn close to the object leaves.
                if thin {
                    let hole: Vec<bool> = lpx.iter().map(|k| *k == Px::Hole).collect();
                    for (u, d) in unknown.iter_mut().zip(crate::selection::edt(&hole, lw, lh)) {
                        *u |= d <= r as f32;
                    }
                }
                let m = membrane_fill(lw, lh, ch, &limg, &unknown);
                for (i, k) in lpx.iter().enumerate() {
                    if *k == Px::Hole {
                        limg[i * ch..(i + 1) * ch].copy_from_slice(&m[i * ch..(i + 1) * ch]);
                    }
                }
            }
        }
        if sources.len() == 0 {
            prev = Some(Coarser { w: lw, h: lh, img: limg, field: Vec::new(), tbox: tb });
            continue;
        }
        let refine_only = li + 1 < nlev && exts[li] > p.refine_extent;
        let n_targets = targets.iter().filter(|t| **t).count();
        // Source moments are worth keeping when there are few sources for the matches tried.
        let keep_source_moments = lw * lh <= SOURCE_MOMENTS_MAX && sources.len() <= n_targets.saturating_mul(if refine_only { 4 } else { 64 });
        let mut solver = Solver {
            w: lw,
            h: lh,
            ch,
            r: ri,
            img: &mut limg,
            px: &lpx,
            valid,
            targets,
            hole_box: hb,
            tbox: tb,
            full: Vec::new(),
            tmom: Vec::new(),
            smom: Vec::new(),
            adapt: p.adapt,
            ctl: *ctl,
        };
        solver.prepare(keep_source_moments);
        ctl.check()?;
        // NNF init: the coarser level's matches scaled up where they land on a valid source,
        // random otherwise.
        let mut rng = Rng::new(p.seed ^ (li as u64) << 20);
        let tw = tb.width();
        let mut nnf = vec![(0i32, 0i32); tb.area()];
        let mut upsampled = false;
        for y in tb.y0..tb.y1 {
            if y % 32 == 0 {
                ctl.check()?;
            }
            for x in tb.x0..tb.x1 {
                let k = (y - tb.y0) * tw + (x - tb.x0);
                if !solver.targets[k] {
                    continue;
                }
                let mut cand = None;
                if let Some(c) = &coarser
                    && !c.field.is_empty()
                {
                    let (cx, cy) = ((x / 2).min(c.w - 1), (y / 2).min(c.h - 1));
                    let (sx, sy) = c.field_at(cx, cy);
                    if sx != i32::MIN {
                        // The same displacement, twice as long.
                        let c = (x as i32 + 2 * (sx - cx as i32), y as i32 + 2 * (sy - cy as i32));
                        if solver.is_valid(c.0, c.1) {
                            cand = Some(c);
                        }
                    }
                }
                upsampled |= cand.is_some();
                nnf[k] = cand.unwrap_or_else(|| sources.nth((rng.next() % sources.len() as u64) as usize, &solver.valid, lw));
            }
        }
        // The coarser level has given all it had.
        drop(coarser);
        let mut cost = vec![f32::INFINITY; tb.area()];
        // A finer level starts from the coarse matches voted with its own (sharp) pixels.
        if upsampled {
            solver.costs(&nnf, &mut cost)?;
            solver.vote(&nnf, &cost);
        }
        let em = if li + 1 == nlev {
            p.em_iters
        } else if refine_only {
            1
        } else {
            (p.em_iters >> (nlev - 1 - li)).max(p.min_em_iters)
        };
        let pm_iters = if refine_only { 1 } else { p.pm_iters };
        let full_search = (lw.max(lh)) as i32;
        for it in 0..em {
            // Costs change as hole pixels are re-estimated: refresh before each PatchMatch.
            solver.costs(&nnf, &mut cost)?;
            // Wide random search on the coarsest level and on each level's first iteration,
            // local refinement afterwards.
            let radius = if refine_only {
                2
            } else if li + 1 == nlev || it == 0 {
                full_search
            } else {
                (4 * psz as i32).min(full_search)
            };
            for pass in 0..pm_iters {
                ctl.check()?;
                solver.patchmatch(&mut nnf, &mut cost, pass + it * pm_iters, p.seed.wrapping_add((li * 1000 + it) as u64), radius);
            }
            ctl.check()?;
            solver.vote(&nnf, &cost);
            ctl.check()?;
        }
        // A vote that saw the cancellation stopped part-way: never hand its pixels on.
        ctl.check()?;
        // Keep the match field (unmatched cells marked) for the next level (none after the last).
        let field: Vec<(i32, i32)> =
            if li == 0 { Vec::new() } else { nnf.iter().zip(&solver.targets).map(|(n, t)| if *t { *n } else { (i32::MIN, i32::MIN) }).collect() };
        // Finer levels cost ~4× the previous one: weight progress by pixel count.
        done_px += lw * lh;
        ctl.progress(done_px as f32 / total_px.max(1) as f32);
        prev = Some(Coarser { w: lw, h: lh, img: limg, field, tbox: tb });
    }
    Ok(prev.map(|c| c.img).ok_or(Cow::Owned(Vec::new())))
}

#[cfg(test)]
mod tests;
