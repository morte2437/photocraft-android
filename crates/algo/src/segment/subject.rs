//! Select Subject (classical, heuristic).
//!
//! Photoshop uses a learned model; we approximate it with salient-object detection feeding a
//! GrabCut:
//!
//! 1. SLIC superpixels on a working-resolution copy.
//! 2. Per-superpixel saliency from "Saliency Optimization from Robust Background Detection"
//!    (W. Zhu, S. Liang, Y. Wei and J. Sun, CVPR 2014): boundary connectivity
//!    `BndCon(p) = Len_bnd(p) / sqrt(Area(p))` over geodesic colour distances gives a background
//!    probability; background-weighted contrast gives foreground cues; a least-squares
//!    optimisation with smoothness between neighbours combines them.
//! 3. Blended with frequency-tuned saliency (R. Achanta, S. Hemami, F. Estrada and
//!    S. Süsstrunk, "Frequency-tuned Salient Region Detection", CVPR 2009): distance of the
//!    lightly blurred CIELAB image from its mean colour.
//! 4. Otsu's threshold on the saliency map seeds a GrabCut trimap (low-saliency border pixels
//!    are definite background); the result is cleaned: small islands are dropped unless they lie
//!    next to the main part (a head cut off by a neck too thin for the working resolution).
//! 5. A second GrabCut whose background model only knows the backdrop: a colour model fitted
//!    to the image's frame marks every pixel it explains as probable background, and the rest
//!    (the blob, plus whatever the backdrop cannot account for) starts as probable foreground.
//!    Parts of the subject whose colour is close to the backdrop (a cream shirt on a near-white
//!    wall) have low saliency and in step 4 land in the background model, which then explains
//!    them away; here they start as foreground and the cut keeps them (#2788). Only components
//!    touching the first pass's result survive, but attached clutter can survive with them.
//!    Where the subject runs out of the picture (a torso cut off by the bottom edge), the frame
//!    pixels the rest of the frame cannot explain are left out of the backdrop model and left to
//!    the cut; otherwise they would teach it the shirt's colour and be fixed as background.
//! 6. Holes are filled only when their colour fits the foreground model better than the
//!    background's, so a patch of backdrop enclosed by the subject (between an arm and the
//!    torso) stays open; specks still fill. Then the mask is re-cut at full resolution.
//!
//! Works well for a distinct object on a comparatively plain background; it is not a semantic
//! segmenter. Nearby clutter and detached subject parts with the same geometry cannot be
//! distinguished by the component filter.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use photocraft_geom::Rect;
use photocraft_raster::{Cancelled, Interrupt};

use super::gmm::Gmm;
use super::grabcut::{self, BG, PR_BG, PR_FG};
use super::slic::{rgb_to_lab, slic_lab};
use super::{Region, Sampler};

/// Working-resolution budget (pixels).
pub const WORK_PX: usize = 110_000;
const SIGMA_CLR: f32 = 10.0;
const SIGMA_SPA: f32 = 0.25;

/// Selects the most salient object on the canvas.
pub fn select_subject(sampler: &dyn Sampler, canvas: Rect) -> Option<Region> {
    select_subject_with(sampler, canvas, &Interrupt::NONE).unwrap_or_default()
}

/// [`select_subject`] that stops with `Err(Cancelled)` once `ctl` is cancelled, asked between
/// stages and inside every min cut (the GrabCut iterations and the full-resolution re-cut).
pub fn select_subject_with(sampler: &dyn Sampler, canvas: Rect, ctl: &Interrupt) -> Result<Option<Region>, Cancelled> {
    if canvas.width() < 4 || canvas.height() < 4 {
        return Ok(None);
    }
    let step = super::scale_for(canvas, WORK_PX);
    let img = sampler.rgb_scaled(canvas, step);
    let (w, h) = (img.w, img.h);
    let sal = saliency(&img);
    ctl.check()?;
    let t = otsu(&sal).clamp(0.15, 0.85);
    let mut trimap = vec![PR_BG; w * h];
    let mut any_fg = false;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let border = x == 0 || y == 0 || x + 1 == w || y + 1 == h;
            trimap[i] = if sal[i] >= t {
                any_fg = true;
                PR_FG
            } else if border || sal[i] < 0.4 * t {
                BG
            } else {
                PR_BG
            };
        }
    }
    if !any_fg {
        return Ok(None);
    }
    let Some((fg, bg)) = grabcut::grabcut_with(&img, &mut trimap, 6, ctl)? else { return Ok(None) };
    let low: Vec<bool> = trimap.iter().map(|v| *v == PR_FG || *v == grabcut::FG).collect();
    let low = keep_main_and_nearby(&low, w, h);
    if !low.iter().any(|v| *v) {
        return Ok(None);
    }
    let (low, fg, bg) = backdrop_pass(&img, &low, ctl)?.unwrap_or((low, fg, bg));
    ctl.check()?;
    let low = super::fill_holes_by_model(&low, &img, &fg, &bg, 0.002, 0.05);
    super::finish_region_with(sampler, canvas, step, &low, w, h, Some((&fg, &bg)), ctl)
}

/// Step 5: GrabCut again with a background model that only knows the backdrop. A colour model
/// is fitted to the image's frame (the outer pixels, minus a margin around `blob` in case the
/// subject runs out of the picture, and minus what [`subject_at_edge`] finds of the subject
/// there); the rest of the frame is definite background, every other pixel it
/// explains well is probable background, and the rest (the blob, and anything the backdrop
/// cannot account for, such as a cream shirt on a near-white wall) starts as probable
/// foreground. `Ok(None)` when the frame cannot be modelled or the cut keeps nothing that
/// touches the blob; `Err(Cancelled)` once `ctl` is cancelled, as in the first pass.
fn backdrop_pass(img: &super::RgbImage, blob: &[bool], ctl: &Interrupt) -> Result<Option<(Vec<bool>, Gmm, Gmm)>, Cancelled> {
    let (w, h) = (img.w, img.h);
    let Some(near) = margin_box(blob, w, h) else { return Ok(None) };
    let (bx0, by0, bx1, by1) = near;
    let frame = ((w.min(h) as f32 * 0.06).round() as usize).max(2);
    let in_frame = |x: usize, y: usize| x < frame || y < frame || x + frame >= w || y + frame >= h;
    let near_blob = |x: usize, y: usize| (bx0..=bx1).contains(&x) && (by0..=by1).contains(&y);
    let edge = |i: usize| in_frame(i % w, i / w) && !near_blob(i % w, i / w);
    // The subject running out of the picture is neither backdrop to learn nor fixed background.
    let released = subject_at_edge(img, blob, frame, near, &edge);
    let samples: Vec<[f32; 3]> = (0..w * h).filter(|i| edge(*i) && !released[*i]).map(|i| img.px[i]).collect();
    let Some((backdrop, limit)) = fit_backdrop(&samples) else { return Ok(None) };
    let mut trimap: Vec<u8> = (0..w * h)
        .map(|i| {
            if edge(i) && !released[i] {
                BG
            } else if blob[i] || backdrop.neg_log(img.px[i]) > limit {
                PR_FG
            } else {
                PR_BG
            }
        })
        .collect();
    let Some((fg, bg)) = grabcut::grabcut_with(img, &mut trimap, 4, ctl)? else { return Ok(None) };
    let cut: Vec<bool> = trimap.iter().map(|v| *v == PR_FG || *v == grabcut::FG).collect();
    let low = super::keep_seeded(&cut, blob, w, h);
    Ok(low.iter().any(|v| *v).then_some((low, fg, bg)))
}

/// The box around the pixels `blob` marks in a `w` × `h` image, grown on every side by 15% of
/// its longer side (at least four pixels) and clipped to the image, as (x0, y0, x1, y1)
/// inclusive. `None` when `blob` marks nothing.
fn margin_box(blob: &[bool], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    for (i, on) in blob.iter().enumerate() {
        if *on {
            let (x, y) = (i % w, i / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    if x1 < x0 || y1 < y0 {
        return None;
    }
    let margin = (((x1 - x0 + 1).max(y1 - y0 + 1) as f32 * 0.15).ceil() as usize).max(4);
    Some((x0.saturating_sub(margin), y0.saturating_sub(margin), (x1 + margin).min(w - 1), (y1 + margin).min(h - 1)))
}

/// The connected parts of `mask` that make up the subject: those at least a fifth the size of
/// the largest, and smaller ones (not specks) with pixels close to it. A part of the subject the
/// cut separated from the rest, such as a head whose thin neck the working resolution lost, a
/// tail or something held, would otherwise go missing; clutter further away is still dropped.
fn keep_main_and_nearby(mask: &[bool], w: usize, h: usize) -> Vec<bool> {
    let (lab, sizes) = super::components(mask, w, h);
    let Some((main, largest)) = sizes.iter().copied().enumerate().skip(1).max_by_key(|(_, n)| *n) else { return vec![false; w * h] };
    // Bounding boxes per component: (x0, y0, x1, y1).
    let mut bbox = vec![(usize::MAX, usize::MAX, 0usize, 0usize); sizes.len()];
    for (i, l) in lab.iter().enumerate() {
        if let Some(b) = bbox.get_mut(*l as usize) {
            let (x, y) = (i % w, i / w);
            *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
        }
    }
    let Some(&(mx0, my0, mx1, my1)) = bbox.get(main) else { return vec![false; w * h] };
    let margin = ((mx1 - mx0 + 1).max(my1 - my0 + 1) as f32 * 0.15).ceil() as usize;
    let (nx0, ny0, nx1, ny1) = (mx0.saturating_sub(margin), my0.saturating_sub(margin), mx1 + margin, my1 + margin);
    // A part is near when some of its pixels lie within the margin, not merely its box: a thin
    // diagonal streak from a corner (a cable, a rail) can have a box that reaches the margin.
    let mut near = vec![false; sizes.len()];
    for y in ny0..=ny1.min(h - 1) {
        for x in nx0..=nx1.min(w - 1) {
            let label = lab.get(y * w + x).copied().unwrap_or(0) as usize;
            if let Some(n) = near.get_mut(label) {
                *n = true;
            }
        }
    }
    let (big, speck) = ((largest as f32 * 0.2).ceil() as usize, ((largest as f32 * 0.005).ceil() as usize).max(4));
    // Only small, nearby components can be vetoed as edge clutter. A strip must be
    // narrow relative to the canvas and at least twice as long as it is wide.
    let keep: Vec<bool> = sizes
        .iter()
        .zip(&bbox)
        .enumerate()
        .map(|(label, (&size, &(x0, y0, x1, y1)))| {
            if label == 0 {
                return false;
            }
            if label == main || size >= big {
                return true;
            }
            let (width, height) = (x1 - x0 + 1, y1 - y0 + 1);
            let nearby = near.get(label).copied().unwrap_or(false);
            let horizontal_strip = (x0 == 0 || x1 + 1 == w) && height < h * 12 / 100 && width >= height * 2;
            let vertical_strip = (y0 == 0 || y1 + 1 == h) && width < w * 12 / 100 && height >= width * 2;
            let thin_edge = horizontal_strip || vertical_strip;
            size >= speck && nearby && !thin_edge
        })
        .collect();
    lab.iter().map(|l| keep.get(*l as usize).copied().unwrap_or(false)).collect()
}

#[cfg(test)]
mod component_tests {
    use super::{keep_main_and_nearby, margin_box, subject_at_edge};
    use crate::segment::RgbImage;

    #[test]
    fn nearby_component_margin_and_size_boundaries() {
        // The main 20x20 component has margin ceil(20 * .15) = 3 and a four-pixel floor.
        // A 30x30 component raises the floor to ceil(900 * .005) = 5.
        for (side, x, count, retained) in [(20, 42, 3, false), (20, 42, 4, true), (20, 43, 4, false), (30, 54, 4, false), (30, 54, 5, true), (30, 55, 5, false)]
        {
            let (w, h) = (80, 80);
            let mut mask = vec![false; w * h];
            for y in 20..20 + side {
                for x in 20..20 + side {
                    mask[y * w + x] = true;
                }
            }
            for y in 25..25 + count {
                mask[y * w + x] = true;
            }
            let result = keep_main_and_nearby(&mask, w, h);
            assert!(result[20 * w + 20]);
            assert_eq!(result[25 * w + x], retained, "side {side}, x {x}, size {count}");
        }
        // A qualifying nearby island is kept whether it is a head or unrelated clutter.
    }

    fn rotate(x: usize, y: usize, side: usize, turns: usize) -> (usize, usize) {
        let (mut x, mut y) = (x, y);
        for _ in 0..turns {
            (x, y) = (side - 1 - y, x);
        }
        (x, y)
    }

    #[test]
    fn cropped_heads_and_large_limbs_survive_on_every_edge() {
        let side = 240;
        for turns in 0..4 {
            for (main, part) in [((100, 20, 140, 70), (111, 0, 129, 18)), ((80, 70, 120, 120), (122, 90, 240, 100))] {
                let mut mask = vec![false; side * side];
                for (x0, y0, x1, y1) in [main, part] {
                    for y in y0..y1 {
                        for x in x0..x1 {
                            let (x, y) = rotate(x, y, side, turns);
                            mask[y * side + x] = true;
                        }
                    }
                }
                let result = keep_main_and_nearby(&mask, side, side);
                let (x, y) = rotate(part.0, part.1, side, turns);
                assert!(result[y * side + x], "edge {turns}, part {part:?}");
            }
        }
    }

    #[test]
    fn small_nearby_elongated_edge_strips_are_excluded() {
        let side = 240;
        for turns in 0..4 {
            let mut mask = vec![false; side * side];
            for (x0, y0, x1, y1) in [(70, 80, 170, 180), (112, 0, 120, 70)] {
                for y in y0..y1 {
                    for x in x0..x1 {
                        let (x, y) = rotate(x, y, side, turns);
                        mask[y * side + x] = true;
                    }
                }
            }
            let result = keep_main_and_nearby(&mask, side, side);
            let (x, y) = rotate(112, 0, side, turns);
            assert!(!result[y * side + x], "edge {turns}");
        }
    }

    #[test]
    fn narrow_edge_parts_matching_subject_are_released_on_every_edge() {
        let side = 240;
        let frame = 12;
        for turns in 0..4 {
            for (colour, expected) in [([0.8, 0.2, 0.1], true), ([0.1, 0.2, 0.8], false)] {
                let mut img = RgbImage::new(side, side);
                img.px.fill([0.5, 0.5, 0.5]);
                let mut blob = vec![false; side * side];
                for y in 60..100 {
                    for x in 100..140 {
                        let (x, y) = rotate(x, y, side, turns);
                        blob[y * side + x] = true;
                        img.px[y * side + x] = [0.8, 0.2, 0.1];
                    }
                }
                for y in 0..frame {
                    for x in 110..120 {
                        let (x, y) = rotate(x, y, side, turns);
                        img.px[y * side + x] = colour;
                    }
                }
                let edge = |i: usize| {
                    let (x, y) = (i % side, i / side);
                    x < frame || y < frame || x + frame >= side || y + frame >= side
                };
                let near = margin_box(&blob, side, side).unwrap();
                let released = subject_at_edge(&img, &blob, frame, near, &edge);
                let (x, y) = rotate(110, 0, side, turns);
                assert_eq!(released[y * side + x], expected, "edge {turns}, colour {colour:?}");
            }
        }
    }

    #[test]
    fn wide_edge_parts_are_released_only_alongside_the_subject() {
        // A dark pole along the left edge, next to the shoulder of a figure that runs off the
        // bottom edge, stays backdrop although it runs a long way along that edge. A cream torso
        // cut off by the bottom edge below a head (the first pass lost it, so the subject's
        // colours do not explain it either) is still released.
        let side = 240;
        let frame = 12;
        // (subject, part, part colour, released), boxes as (x0, y0, x1, y1) exclusive.
        let cases = [((10, 100, 110, 240), (0, 0, 8, 240), [0.1, 0.1, 0.1], false), ((100, 40, 140, 80), (60, 228, 180, 240), [0.9, 0.85, 0.7], true)];
        for turns in 0..4 {
            for (subject, part, colour, expected) in cases {
                let mut img = RgbImage::new(side, side);
                img.px.fill([0.5, 0.5, 0.5]);
                let mut blob = vec![false; side * side];
                for (x0, y0, x1, y1) in [subject, part] {
                    for y in y0..y1 {
                        for x in x0..x1 {
                            let (x, y) = rotate(x, y, side, turns);
                            let on = (x0, y0) == (subject.0, subject.1);
                            blob[y * side + x] = on;
                            img.px[y * side + x] = if on { [0.8, 0.2, 0.1] } else { colour };
                        }
                    }
                }
                // As in `backdrop_pass`: the frame, minus the blob's box.
                let (bx0, by0, bx1, by1) = margin_box(&blob, side, side).unwrap();
                let edge = |i: usize| {
                    let (x, y) = (i % side, i / side);
                    let in_frame = x < frame || y < frame || x + frame >= side || y + frame >= side;
                    in_frame && !((bx0..=bx1).contains(&x) && (by0..=by1).contains(&y))
                };
                let released = subject_at_edge(&img, &blob, frame, (bx0, by0, bx1, by1), &edge);
                let (x, y) = rotate(part.0, part.1, side, turns);
                assert_eq!(released[y * side + x], expected, "edge {turns}, part {part:?}");
            }
        }
    }

    #[test]
    fn nearby_means_pixels_within_the_margin_not_just_the_box() {
        // The main part covers 100..140 × 20..60, so its margin box is 94..=145 × 14..=65. A
        // one-pixel diagonal streak from the corner (a cable, a rail) to (100, 100) has a box
        // that overlaps the margin box, but none of its pixels lie in it: it is not kept. A
        // compact part whose pixels lie inside the margin is.
        let side = 240;
        let mut mask = vec![false; side * side];
        for y in 20..60 {
            for x in 100..140 {
                mask[y * side + x] = true;
            }
        }
        for k in 0..=100 {
            mask[k * side + k] = true;
        }
        for y in 30..35 {
            for x in 142..146 {
                mask[y * side + x] = true;
            }
        }
        let result = keep_main_and_nearby(&mask, side, side);
        assert!(result[20 * side + 100]);
        assert!(!result[0], "the corner of the streak is kept");
        assert!(!result[100 * side + 100], "the end of the streak is kept");
        assert!(result[30 * side + 142], "the compact part inside the margin is dropped");
    }

    #[test]
    fn clutter_away_from_the_subject_or_too_small_is_dropped() {
        // The main 40x40 part has margin ceil(40 * .15) = 6 and a speck floor of
        // ceil(1600 * .005) = 8 pixels. A 5x5 object just past the margin and a 2x3 one inside
        // it are clutter; a 3x3 one inside it is kept.
        let side = 240;
        let mut mask = vec![false; side * side];
        for (x0, y0, x1, y1) in [(100, 100, 140, 140), (147, 110, 152, 115), (110, 143, 112, 146), (120, 143, 123, 146)] {
            for y in y0..y1 {
                for x in x0..x1 {
                    mask[y * side + x] = true;
                }
            }
        }
        let result = keep_main_and_nearby(&mask, side, side);
        assert!(result[100 * side + 100]);
        assert!(!result[110 * side + 147], "clutter past the margin is kept");
        assert!(!result[143 * side + 110], "a speck inside the margin is kept");
        assert!(result[143 * side + 120], "a part inside the margin is dropped");
    }
}

/// A colour model of `samples` and the cost above which a pixel is not explained by it: what
/// the samples themselves cost under it (99th percentile), plus a margin. `None` without samples.
fn fit_backdrop(samples: &[[f32; 3]]) -> Option<(Gmm, f32)> {
    let model = Gmm::fit(&super::subsample(samples, 40_000), grabcut::K, super::gmm::DEFAULT_REG)?;
    let mut costs: Vec<f32> = samples.iter().map(|p| model.neg_log(*p)).collect();
    costs.sort_by(|a, b| a.total_cmp(b));
    let limit = costs.get(costs.len() * 99 / 100)? + 4.0;
    Some((model, limit))
}

/// The pixels of the frame (those `edge` accepts, `frame` wide) that belong to the subject
/// running out of the picture, such as a torso cut off by the bottom edge: pixels of a side
/// (its corners included) that the colours of the frame off that side cannot explain. Without
/// this, such a part trains the backdrop model and is fixed as background, so a cream shirt on a
/// near-white wall is lost. A floor or a table runs up the sides too, so the frame off the
/// bottom knows its colour and it stays backdrop; only a band confined to one side's strip is
/// released (and is then left to the cut). Small patches stay backdrop. Narrow patches are
/// released only if their colour is also explained by the retained subject; wide ones whatever
/// their colour, but only where they run alongside `near` (the blob's box with its margin, as
/// [`margin_box`] gives it): a pole or a door frame along a side next to the subject's shoulder
/// stays backdrop, while a torso the first pass lost still lies below the head.
fn subject_at_edge(img: &super::RgbImage, blob: &[bool], frame: usize, near: (usize, usize, usize, usize), edge: &dyn Fn(usize) -> bool) -> Vec<bool> {
    let (w, h) = (img.w, img.h);
    // Top, bottom, left, right.
    let on = |i: usize, s: usize| {
        let (x, y) = (i % w, i / w);
        [y < frame, y + frame >= h, x < frame, x + frame >= w].get(s).copied().unwrap_or(false)
    };
    let subject_samples: Vec<[f32; 3]> = img.px.iter().zip(blob).filter_map(|(pixel, on)| on.then_some(*pixel)).collect();
    let subject_model = fit_backdrop(&subject_samples);
    let mut odd = vec![false; w * h];
    for s in 0..4 {
        let others: Vec<[f32; 3]> = (0..w * h).filter(|i| edge(*i) && !on(*i, s)).map(|i| img.px[i]).collect();
        let Some((model, limit)) = fit_backdrop(&others) else { continue };
        for (i, o) in odd.iter_mut().enumerate() {
            if !*o && edge(i) && on(i, s) && model.neg_log(img.px[i]) > limit {
                *o = true;
            }
        }
    }
    // A stray pixel (noise, a dead pixel) is not a subject: only patches of some size count.
    let (lab, sizes) = super::components(&odd, w, h);
    let min = (frame * frame / 2).max(4);
    let mut bounds = vec![(w, h, 0usize, 0usize); sizes.len()];
    for (i, &label) in lab.iter().enumerate() {
        if label != 0
            && let Some(b) = bounds.get_mut(label as usize)
        {
            let (x, y) = (i % w, i / w);
            *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
        }
    }
    // Count matching pixels per component so a single coincidental pixel cannot release it.
    let mut matching = vec![0usize; sizes.len()];
    if let Some((model, limit)) = subject_model {
        for (i, &label) in lab.iter().enumerate() {
            if label != 0
                && model.neg_log(img.px[i]) <= limit
                && let Some(count) = matching.get_mut(label as usize)
            {
                *count += 1;
            }
        }
    }
    lab.iter()
        .map(|l| {
            let i = *l as usize;
            let Some((&size, &(x0, y0, x1, y1))) = sizes.get(i).zip(bounds.get(i)) else { return false };
            if *l == 0 || size < min {
                return false;
            }
            // Along the edge, a wide patch must overlap the blob's box: beside the subject, `edge`
            // leaves out that box, so a pole along a side shows up only above or below it.
            let (nx0, ny0, nx1, ny1) = near;
            let wide_on_horizontal_edge = (y0 == 0 || y1 + 1 == h) && x1 - x0 + 1 >= 2 * frame && x0 <= nx1 && x1 >= nx0;
            let wide_on_vertical_edge = (x0 == 0 || x1 + 1 == w) && y1 - y0 + 1 >= 2 * frame && y0 <= ny1 && y1 >= ny0;
            let matches_subject = matching.get(i).is_some_and(|count| *count >= size.div_ceil(2));
            wide_on_horizontal_edge || wide_on_vertical_edge || matches_subject
        })
        .collect()
}

/// Per-pixel saliency in `0..=1` (see the module docs).
pub fn saliency(img: &super::RgbImage) -> Vec<f32> {
    let (w, h) = (img.w, img.h);
    let lab: Vec<[f32; 3]> = img.px.iter().map(|p| rgb_to_lab(*p)).collect();
    let target = ((w * h) / 150).clamp(16, 400);
    let sp = slic_lab(&lab, w, h, target, 10.0, 8);
    let n = sp.count;
    // Region statistics.
    let mut mean = vec![[0.0f64; 3]; n];
    let mut pos = vec![[0.0f64; 2]; n];
    let mut cnt = vec![0.0f64; n];
    let mut on_border = vec![false; n];
    for y in 0..h {
        for x in 0..w {
            let l = sp.labels[y * w + x] as usize;
            let c = lab[y * w + x];
            for k in 0..3 {
                mean[l][k] += c[k] as f64;
            }
            pos[l][0] += x as f64 / w as f64;
            pos[l][1] += y as f64 / h as f64;
            cnt[l] += 1.0;
            if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
                on_border[l] = true;
            }
        }
    }
    let mean: Vec<[f32; 3]> = mean.iter().zip(&cnt).map(|(m, c)| m.map(|v| (v / c.max(1.0)) as f32)).collect();
    let pos: Vec<[f32; 2]> = pos.iter().zip(&cnt).map(|(p, c)| p.map(|v| (v / c.max(1.0)) as f32)).collect();
    let dapp = |a: usize, b: usize| super::d2(mean[a], mean[b]).sqrt();
    // Adjacency.
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut link = |a: usize, b: usize| {
        if a != b && !adj[a].contains(&b) {
            adj[a].push(b);
            adj[b].push(a);
        }
    };
    for y in 0..h {
        for x in 0..w {
            let l = sp.labels[y * w + x] as usize;
            if x + 1 < w {
                link(l, sp.labels[y * w + x + 1] as usize);
            }
            if y + 1 < h {
                link(l, sp.labels[(y + 1) * w + x] as usize);
            }
        }
    }
    // Boundary connectivity from all-pairs geodesic distances (Dijkstra per region).
    let mut wbg = vec![0.0f32; n];
    let mut geo = vec![f32::INFINITY; n];
    for (p, wb) in wbg.iter_mut().enumerate() {
        dijkstra(p, &adj, &dapp, &mut geo);
        let (mut area, mut len) = (0.0f32, 0.0f32);
        for q in 0..n {
            let s = (-geo[q] * geo[q] / (2.0 * SIGMA_CLR * SIGMA_CLR)).exp();
            area += s;
            if on_border[q] {
                len += s;
            }
        }
        let bnd = len / area.max(1e-6).sqrt();
        *wb = 1.0 - (-bnd * bnd / 2.0).exp();
    }
    // Background-weighted contrast.
    let mut wctr = vec![0.0f32; n];
    for p in 0..n {
        let mut s = 0.0;
        for q in 0..n {
            let (dx, dy) = (pos[p][0] - pos[q][0], pos[p][1] - pos[q][1]);
            let wspa = (-(dx * dx + dy * dy) / (2.0 * SIGMA_SPA * SIGMA_SPA)).exp();
            s += dapp(p, q) * wspa * wbg[q];
        }
        wctr[p] = s;
    }
    let mx = wctr.iter().copied().fold(0.0f32, f32::max).max(1e-6);
    let wfg: Vec<f32> = wctr.iter().map(|v| v / mx).collect();
    // Saliency optimisation: min Σ wbg s² + Σ wfg (s−1)² + Σ_{i~j} w_ij (s_i − s_j)² (Gauss–Seidel).
    let wij: Vec<Vec<f32>> = (0..n).map(|p| adj[p].iter().map(|&q| (-dapp(p, q).powi(2) / (2.0 * SIGMA_CLR * SIGMA_CLR)).exp() + 0.1).collect()).collect();
    let mut s = wfg.clone();
    for _ in 0..200 {
        for p in 0..n {
            let (mut num, mut den) = (wfg[p], wbg[p] + wfg[p]);
            for (q, wq) in adj[p].iter().zip(&wij[p]) {
                num += wq * s[*q];
                den += wq;
            }
            s[p] = num / den.max(1e-6);
        }
    }
    // Frequency-tuned saliency.
    let mut mean_all = [0.0f64; 3];
    for c in &lab {
        for k in 0..3 {
            mean_all[k] += c[k] as f64;
        }
    }
    let mean_all = mean_all.map(|v| (v / lab.len() as f64) as f32);
    let blurred = super::RgbImage { w, h, px: lab.clone() };
    let blurred = blur_lab(&blurred);
    let ft: Vec<f32> = blurred.px.iter().map(|c| super::d2(*c, mean_all).sqrt()).collect();
    let ftmax = ft.iter().copied().fold(0.0f32, f32::max).max(1e-6);
    let smin = s.iter().copied().fold(f32::MAX, f32::min);
    let smax = s.iter().copied().fold(f32::MIN, f32::max);
    let srange = (smax - smin).max(1e-6);
    let mut out: Vec<f32> = sp.labels.iter().zip(&ft).map(|(l, f)| 0.75 * (s[*l as usize] - smin) / srange + 0.25 * f / ftmax).collect();
    let (lo, hi) = out.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    let r = (hi - lo).max(1e-6);
    for v in &mut out {
        *v = (*v - lo) / r;
    }
    out
}

fn blur_lab(img: &super::RgbImage) -> super::RgbImage {
    let (w, h) = (img.w, img.h);
    let mut cur = img.clone();
    for _ in 0..2 {
        let mut tmp = cur.clone();
        for y in 0..h {
            for x in 0..w {
                let (l, c, r) = (cur.at(x.saturating_sub(1), y), cur.at(x, y), cur.at((x + 1).min(w - 1), y));
                tmp.px[y * w + x] = [0, 1, 2].map(|k| (l[k] + 2.0 * c[k] + r[k]) / 4.0);
            }
        }
        for y in 0..h {
            for x in 0..w {
                let (u, c, d) = (tmp.at(x, y.saturating_sub(1)), tmp.at(x, y), tmp.at(x, (y + 1).min(h - 1)));
                cur.px[y * w + x] = [0, 1, 2].map(|k| (u[k] + 2.0 * c[k] + d[k]) / 4.0);
            }
        }
    }
    cur
}

#[derive(PartialEq)]
struct Item(f32, usize);
impl Eq for Item {}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0)
    }
}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

fn dijkstra(src: usize, adj: &[Vec<usize>], wt: &dyn Fn(usize, usize) -> f32, dist: &mut [f32]) {
    dist.fill(f32::INFINITY);
    dist[src] = 0.0;
    let mut heap = BinaryHeap::new();
    heap.push(Item(0.0, src));
    while let Some(Item(d, u)) = heap.pop() {
        if d > dist[u] {
            continue;
        }
        for &v in &adj[u] {
            let nd = d + wt(u, v);
            if nd < dist[v] {
                dist[v] = nd;
                heap.push(Item(nd, v));
            }
        }
    }
}

/// Otsu's threshold of values in `0..=1` (256 bins).
pub fn otsu(v: &[f32]) -> f32 {
    let mut hist = [0u64; 256];
    for x in v {
        hist[(x.clamp(0.0, 1.0) * 255.0).round() as usize] += 1;
    }
    let total = v.len() as f64;
    let sum: f64 = hist.iter().enumerate().map(|(i, c)| i as f64 * *c as f64).sum();
    let (mut wb, mut sb, mut best, mut bt) = (0.0f64, 0.0f64, -1.0f64, 0usize);
    for (t, c) in hist.iter().enumerate() {
        wb += *c as f64;
        if wb == 0.0 {
            continue;
        }
        let wf = total - wb;
        if wf == 0.0 {
            break;
        }
        sb += t as f64 * *c as f64;
        let (mb, mf) = (sb / wb, (sum - sb) / wf);
        let between = wb * wf * (mb - mf) * (mb - mf);
        if between > best {
            best = between;
            bt = t;
        }
    }
    (bt as f32 + 0.5) / 255.0
}
