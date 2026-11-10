//! Background Eraser: along a stroke, sample the colour under the brush hotspot and erase pixels
//! of similar colour inside the brush to transparency.
//!
//! Colours are compared in the surface's own colour channels (RGB, CMYK, Gray, Lab… at any depth),
//! so nothing is converted through RGB; only alpha changes.

use photocraft_color::{ColorMode, SampleType, read_sample, write_sample};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba};
use serde::{Deserialize, Serialize};

use crate::Stroke;
use crate::replace::{Limits, Sampling};
use crate::retouch::{Footprint, Region, alpha_index, apply_dab_stroke};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BgEraseSettings {
    pub sampling: Sampling,
    /// Which matching pixels are erased. [`Limits::FindEdges`] grades the erase over the Tolerance
    /// like [`Limits::Contiguous`] in this tool (#11904), unlike Color Replacement, where it turns
    /// the soft falloff off.
    pub limits: Limits,
    /// 0..1: how far from the sampled colour a colour is still partly erased. The distance is the
    /// largest increase plus the largest decrease over the colour channels (`dist`); an exact
    /// match is erased fully, then less the further a colour is, and colours at or beyond the
    /// Tolerance are kept (`graded_weight`).
    pub tolerance: f32,
    /// Keep the foreground colour: the erase narrows to the colours between the sampled colour and
    /// the foreground colour (see [`protected_tolerance`]).
    pub protect_foreground: bool,
    /// Straight RGBA foreground colour (for Protect Foreground Color).
    pub foreground: [f32; 4],
    /// Straight RGBA background swatch (for [`Sampling::BackgroundSwatch`]).
    pub background: [f32; 4],
    /// The inks `foreground` was picked as (its sRGB colour, the CMYK values), for a CMYK target:
    /// Protect Foreground Color compares against exactly these, since an sRGB round trip does not
    /// give them back (#12352). Only valid while `foreground` is that sRGB colour.
    #[serde(skip)]
    pub foreground_cmyk: Option<([f32; 3], [f32; 4])>,
    /// The inks `background` was picked as, like [`Self::foreground_cmyk`] (for
    /// [`Sampling::BackgroundSwatch`]).
    #[serde(skip)]
    pub background_cmyk: Option<([f32; 3], [f32; 4])>,
}

impl Default for BgEraseSettings {
    fn default() -> Self {
        Self {
            sampling: Sampling::Continuous,
            limits: Limits::Contiguous,
            tolerance: 0.5,
            protect_foreground: false,
            foreground: [0.0, 0.0, 0.0, 1.0],
            background: [1.0; 4],
            foreground_cmyk: None,
            background_cmyk: None,
        }
    }
}

/// Colour distance of `a` from `b` over the colour channels: the largest increase plus the largest
/// decrease (#11901). Changes in the same direction count like the largest of them; changes in
/// opposite directions add up, as the reference application measures them (a red-to-green edge is
/// about twice as far as one channel alone). One channel (Gray) gives the absolute difference.
fn dist(a: &[f32], b: &[f32]) -> f32 {
    let (up, down) = a.iter().zip(b).fold((0.0f32, 0.0f32), |(up, down), (x, y)| {
        let diff = x - y;
        (up.max(diff), down.max(-diff))
    });
    up + down
}

/// The Tolerance Protect Foreground Color leaves (#11903): it narrows to `limit`, the distance from
/// the sampled colour to the foreground colour, so the sampled colour itself is still erased, the
/// erase fades out towards the foreground, and colours at least as far from the sample as the
/// foreground (the foreground itself included) are kept. `None` when the foreground is the sampled
/// colour: nothing is erased.
fn protected_tolerance(tol: f32, limit: f32) -> Option<f32> {
    if limit.is_nan() || limit <= 0.0 {
        return None;
    }
    Some(tol.min(limit))
}

/// `v` as the document stores it: rounded to the 8- or 16-bit levels, unchanged in 32-bit float.
/// The sampled colour is read from stored pixels, so the foreground is compared at the same
/// precision; otherwise a foreground between two levels misses its own stored colour (#11903).
fn at_sample_precision(sample: SampleType, v: f32) -> f32 {
    let mut bytes = [0u8; 4];
    write_sample(&mut bytes, sample, 0, v);
    read_sample(&bytes, sample, 0)
}

/// The match weight of one dab for a colour `d` away from the sampled colour (#11901): an exact
/// match is erased fully, then the weight falls off linearly over the whole Tolerance `tol` after
/// a short plateau `d0 = min(tol / 20, 8 levels)`, and colours at or beyond the Tolerance are kept.
/// Measured in the reference application with single clicks (one dab) at Tolerance 10 to 100.
fn graded_weight(d: f32, tol: f32) -> f32 {
    if !d.is_finite() || d >= tol {
        return if d <= 0.0 { 1.0 } else { 0.0 };
    }
    let d0 = (tol / 20.0).min(8.0 / 255.0);
    if d <= d0 {
        return 1.0;
    }
    (1.0 - (d - d0) / (tol - d0)).clamp(0.0, 1.0)
}

/// Erase along `stroke`. The brush opacity is ignored (Photoshop's Background Eraser has none);
/// the selection limits the effect. Returns the damaged rectangle; empty when the surface has no
/// alpha channel.
pub fn apply_background_eraser(target: &mut Surface, stroke: &Stroke, bs: &BgEraseSettings, selection: Option<&Surface>) -> Rect {
    let Some(effect) = background_eraser_effect(target, bs) else { return Rect::EMPTY };
    apply_dab_stroke(target, &background_eraser_stroke(stroke), selection, false, 0, effect)
}

/// The stroke [`apply_background_eraser`] runs its dabs along: full opacity (the tool has none),
/// never the brush's erase flag. Live previews run the same dabs.
pub fn background_eraser_stroke(stroke: &Stroke) -> Stroke {
    let mut stroke = stroke.clone();
    stroke.brush.opacity = 1.0;
    stroke.brush.erase = false;
    stroke
}

/// One Background Eraser dab's edit of a working copy of `pre` (the target before the stroke):
/// shared by [`apply_background_eraser`] and live previews that run the dabs as they come, so both
/// erase the same pixels. `None` when `pre` has no alpha channel (nothing can be erased).
pub fn background_eraser_effect(pre: &Surface, bs: &BgEraseSettings) -> Option<impl FnMut(&mut Region, &Footprint) + use<>> {
    let fmt = pre.format();
    let a = alpha_index(&fmt)?;
    let nc = a;
    let tol = if bs.tolerance.is_finite() { bs.tolerance.clamp(0.0, 1.0) } else { 0.0 };
    // A colour picked as CMYK is compared by its inks in a CMYK target (#12352), at the precision
    // the document stores them.
    let native = |c: [f32; 4], picked: Option<([f32; 3], [f32; 4])>| -> Vec<f32> {
        match picked {
            Some((rgb, inks)) if fmt.mode == ColorMode::Cmyk && rgb == [c[0], c[1], c[2]] => {
                inks.iter().take(nc).map(|&v| at_sample_precision(fmt.sample, v)).collect()
            }
            _ => from_rgba(&fmt, [c[0], c[1], c[2], 1.0])[..nc].to_vec(),
        }
    };
    let swatch = native(bs.background, bs.background_cmyk);
    let fg: Vec<f32> = native(bs.foreground, bs.foreground_cmyk).into_iter().map(|v| at_sample_precision(fmt.sample, v)).collect();
    let (sampling, limits, protect_foreground) = (bs.sampling, bs.limits, bs.protect_foreground);
    let float = fmt.sample == SampleType::F32;
    let mut once: Option<Option<Vec<f32>>> = None;
    let pre = pre.clone();
    // The pixels before the stroke under the working copy, read once per working area. Sampling
    // and matching read these, so a dab landing on pixels earlier dabs already erased still
    // samples and matches (#1280).
    let mut orig: Option<Region> = None;
    Some(move |work: &mut Region, fp: &Footprint| {
        let r = fp.rect.intersect(&work.rect);
        if r.is_empty() {
            return;
        }
        if orig.as_ref().is_none_or(|o| o.rect != work.rect) {
            orig = Some(Region::read(&pre, work.rect));
        }
        let Some(orig) = orig.as_ref() else { return };
        let (cx, cy) = (fp.dab.center.x.floor() as i32, fp.dab.center.y.floor() as i32);
        // The hotspot's colour (None where it was transparent before the stroke or is off the
        // working area).
        let hotspot = || -> Option<Vec<f32>> {
            if !orig.rect.contains(cx, cy) {
                return None;
            }
            let p = orig.px(cx, cy);
            (p[a] > 0.0).then(|| p[..nc].to_vec())
        };
        let sampled = match sampling {
            Sampling::Continuous => hotspot(),
            Sampling::Once => once.get_or_insert_with(hotspot).clone(),
            Sampling::BackgroundSwatch => Some(swatch.clone()),
        };
        let Some(sampled) = sampled else { return };
        // Protect Foreground Color narrows the Tolerance to the foreground colour (#11903), by the
        // same distance as the sample match: the reference application measures a foreground that
        // is off in two channels that way too.
        let tol = if protect_foreground {
            let Some(t) = protected_tolerance(tol, dist(&sampled, &fg)) else { return };
            t
        } else {
            tol
        };
        // Find Edges grades soft edges like Contiguous does instead of cutting them hard (#11904):
        // the reference application keeps at least as much of a blurred transition with it.
        let match_w = |d: f32| -> f32 { graded_weight(d, tol) };
        let (w, h) = (r.width() as usize, r.height() as usize);
        // Which pixels the contiguous fill may pass, from the pre-stroke colours as before: it
        // passes through pixels this stroke already erased.
        let mut reach = vec![0.0f32; w * h];
        // Per-pixel match weight from the working copy (#11902): a pixel an earlier dab erased
        // partly is matched by its current, decontaminated colour, which is pushed away from the
        // sampled colour, so later dabs erase it again only while it is still within the Tolerance.
        let mut weight = vec![0.0f32; w * h];
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                if fp.at(x, y) <= 0.0 {
                    continue;
                }
                let o = orig.px(x, y);
                if o[a] <= 0.0 {
                    continue;
                }
                let i = (y - r.y0) as usize * w + (x - r.x0) as usize;
                reach[i] = match_w(dist(&o[..nc], &sampled));
                let c = work.px(x, y);
                if c[a] > 0.0 {
                    weight[i] = match_w(dist(&c[..nc], &sampled));
                }
            }
        }
        if limits != Limits::Discontiguous {
            keep_connected(&mut reach, w, h, (cx.clamp(r.x0, r.x1 - 1) - r.x0) as usize, (cy.clamp(r.y0, r.y1 - 1) - r.y0) as usize);
            for (wv, rv) in weight.iter_mut().zip(&reach) {
                if *rv <= 0.0 {
                    *wv = 0.0;
                }
            }
        }
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let k = fp.at(x, y) * weight[(y - r.y0) as usize * w + (x - r.x0) as usize];
                if k <= 0.0 {
                    continue;
                }
                let keep = 1.0 - k.min(1.0);
                let p = work.px_mut(x, y);
                p[a] *= keep;
                if p[a] <= 0.0 || keep <= 0.0 {
                    p.fill(0.0);
                    continue;
                }
                decontaminate(&mut p[..nc], &sampled, keep, float);
            }
        }
    })
}

/// Colour decontamination of a partly erased pixel (#11902): of the colour `c`, the share `1 - keep`
/// the dab removed was the sampled colour, so what stays is pushed away from it until compositing
/// it at `keep` over the sampled colour gives back `c`: `(c - sampled × (1 - keep)) / keep` per
/// channel, in the document's own channels. Clipped to the channel range (0..1; 32-bit float keeps
/// values above 1 it already had).
fn decontaminate(c: &mut [f32], sampled: &[f32], keep: f32, float: bool) {
    for (v, s) in c.iter_mut().zip(sampled) {
        let hi = if float { v.max(1.0) } else { 1.0 };
        let d = (*v - s * (1.0 - keep)) / keep;
        *v = if d.is_finite() { d.clamp(0.0, hi) } else { v.clamp(0.0, hi) };
    }
}

/// Zeroes the weights not 4-connected to `(sx, sy)` through non-zero weights.
fn keep_connected(weight: &mut [f32], w: usize, h: usize, sx: usize, sy: usize) {
    let mut keep = vec![false; w * h];
    let si = sy * w + sx;
    if weight.get(si).is_some_and(|v| *v > 0.0) {
        let mut stack = vec![si];
        keep[si] = true;
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let mut nb = |j: usize| {
                if !keep[j] && weight[j] > 0.0 {
                    keep[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                nb(i - 1);
            }
            if x + 1 < w {
                nb(i + 1);
            }
            if y > 0 {
                nb(i - w);
            }
            if y + 1 < h {
                nb(i + w);
            }
        }
    }
    for (wv, k) in weight.iter_mut().zip(&keep) {
        if !k {
            *wv = 0.0;
        }
    }
}

#[cfg(test)]
#[path = "bg_erase_protect_foreground_tests.rs"]
mod protect_foreground_tests;

#[cfg(test)]
#[path = "bg_erase_graded_alpha_tests.rs"]
mod graded_alpha_tests;

#[cfg(test)]
#[path = "bg_erase_find_edges_tests.rs"]
mod find_edges_tests;

#[cfg(test)]
#[path = "bg_erase_decontamination_tests.rs"]
mod decontamination_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrushSettings, StrokePoint};
    use photocraft_color::{ColorMode, PixelFormat, SampleType};

    const GREEN: [f32; 4] = [0.2, 0.6, 0.2, 1.0];
    const BLUE: [f32; 4] = [0.1, 0.1, 0.5, 1.0];

    fn setup(fmt: PixelFormat) -> Surface {
        let mut s = Surface::new(fmt);
        s.fill_rect(Rect::new(0, 0, 40, 40), &from_rgba(&fmt, GREEN));
        s.fill_rect(Rect::new(40, 0, 80, 40), &from_rgba(&fmt, BLUE));
        s
    }

    fn stroke(points: &[(f64, f64)]) -> Stroke {
        Stroke {
            brush: BrushSettings { size: 30.0, hardness: 1.0, spacing: 0.25, pressure_size: false, ..Default::default() },
            points: points.iter().map(|&(x, y)| StrokePoint::new(x, y, 1.0)).collect(),
        }
    }

    fn alpha(s: &Surface, x: i32, y: i32) -> f32 {
        s.rgba(x, y)[3]
    }

    #[test]
    fn once_erases_only_the_sampled_colour_in_every_format() {
        for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Grayscale] {
            for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
                let fmt = PixelFormat::new(mode, sample, true);
                let mut s = setup(fmt);
                let bs = BgEraseSettings { sampling: Sampling::Once, limits: Limits::Discontiguous, tolerance: 0.05, ..Default::default() };
                let d = apply_background_eraser(&mut s, &stroke(&[(20.0, 20.0), (60.0, 20.0)]), &bs, None);
                assert!(!d.is_empty());
                assert_eq!(alpha(&s, 20, 20), 0.0, "{fmt:?} green under the hotspot erased");
                assert_eq!(alpha(&s, 45, 20), 1.0, "{fmt:?} blue kept");
                assert_eq!(alpha(&s, 20, 39), 1.0, "{fmt:?} outside the brush kept");
            }
        }
    }

    #[test]
    fn continuous_follows_the_hotspot_and_swatch_matches_background() {
        let fmt = PixelFormat::RGBA8;
        let mut s = setup(fmt);
        let bs = BgEraseSettings { sampling: Sampling::Continuous, limits: Limits::Discontiguous, tolerance: 0.05, ..Default::default() };
        apply_background_eraser(&mut s, &stroke(&[(20.0, 20.0), (60.0, 20.0)]), &bs, None);
        assert_eq!(alpha(&s, 20, 20), 0.0);
        assert_eq!(alpha(&s, 60, 20), 0.0, "blue erased once the hotspot reached it");
        // Background swatch = blue: only blue goes, wherever the hotspot is.
        let mut t = setup(fmt);
        let bs = BgEraseSettings { sampling: Sampling::BackgroundSwatch, background: BLUE, ..bs };
        apply_background_eraser(&mut t, &stroke(&[(30.0, 20.0), (50.0, 20.0)]), &bs, None);
        assert_eq!(alpha(&t, 30, 20), 1.0);
        assert_eq!(alpha(&t, 50, 20), 0.0);
    }

    #[test]
    fn contiguous_and_protect_foreground() {
        let fmt = PixelFormat::RGBA32F;
        let mut s = Surface::new(fmt);
        s.fill_rect(Rect::new(0, 0, 60, 40), &GREEN);
        // A blue wall separates a green island at x 30..40.
        s.fill_rect(Rect::new(26, 0, 30, 40), &BLUE);
        for (limits, island_erased) in [(Limits::Contiguous, false), (Limits::FindEdges, false), (Limits::Discontiguous, true)] {
            let mut t = s.clone();
            let bs = BgEraseSettings { sampling: Sampling::Once, limits, tolerance: 0.05, ..Default::default() };
            apply_background_eraser(&mut t, &stroke(&[(20.0, 20.0)]), &bs, None);
            assert_eq!(alpha(&t, 31, 20) == 0.0, island_erased, "{limits:?}");
            assert_eq!(alpha(&t, 20, 20), 0.0);
        }
        // Protecting a foreground equal to the sampled colour erases nothing.
        let mut t = s.clone();
        let bs = BgEraseSettings { sampling: Sampling::Once, tolerance: 0.05, protect_foreground: true, foreground: GREEN, ..Default::default() };
        apply_background_eraser(&mut t, &stroke(&[(20.0, 20.0)]), &bs, None);
        assert_eq!(t, s);
    }

    #[test]
    fn cmyk_targets_compare_the_picked_inks() {
        // #12352: cyan inks whose sRGB stand-in converts back to other inks.
        const STAND_IN: [f32; 4] = [0.0, 0.68, 0.94, 1.0];
        let picked = Some(([STAND_IN[0], STAND_IN[1], STAND_IN[2]], [1.0, 0.0, 0.0, 0.0]));
        let fmt = PixelFormat::new(ColorMode::Cmyk, SampleType::U8, true);
        let mut s = Surface::new(fmt);
        s.fill_rect(Rect::new(0, 0, 80, 40), &from_rgba(&fmt, [1.0; 4]));
        s.fill_rect(Rect::new(30, 0, 50, 40), &[1.0, 0.0, 0.0, 0.0, 1.0]);
        let st = stroke(&[(10.0, 20.0), (70.0, 20.0)]);
        let base = BgEraseSettings { limits: Limits::Discontiguous, tolerance: 0.01, ..Default::default() };
        let protect = BgEraseSettings { protect_foreground: true, foreground: STAND_IN, foreground_cmyk: picked, ..base.clone() };
        let mut t = s.clone();
        apply_background_eraser(&mut t, &st, &protect, None);
        assert_eq!(alpha(&t, 40, 20), 1.0, "the picked foreground inks are protected");
        assert_eq!(alpha(&t, 10, 20), 0.0, "white erased");
        let swatch = BgEraseSettings { sampling: Sampling::BackgroundSwatch, background: STAND_IN, background_cmyk: picked, ..base.clone() };
        let mut t = s.clone();
        apply_background_eraser(&mut t, &st, &swatch, None);
        assert_eq!(alpha(&t, 40, 20), 0.0, "the picked background inks are erased fully");
        assert_eq!(alpha(&t, 10, 20), 1.0, "white kept");
        // Inks picked for another colour, and non-CMYK targets, use the stand-in as before.
        let stale = BgEraseSettings { background_cmyk: Some(([0.5; 3], [1.0, 0.0, 0.0, 0.0])), ..swatch.clone() };
        let mut t = s.clone();
        apply_background_eraser(&mut t, &st, &stale, None);
        assert_eq!(alpha(&t, 40, 20), 1.0, "stale inks are ignored");
        let rgb = PixelFormat::RGBA8;
        let mut r = Surface::new(rgb);
        r.fill_rect(Rect::new(0, 0, 80, 40), &from_rgba(&rgb, STAND_IN));
        let (mut with, mut without) = (r.clone(), r.clone());
        apply_background_eraser(&mut with, &st, &swatch, None);
        apply_background_eraser(&mut without, &st, &BgEraseSettings { background_cmyk: None, ..swatch }, None);
        assert_eq!(with, without, "an RGB target ignores the inks");
    }

    #[test]
    fn picked_swatch_inks_match_their_stored_levels() {
        // #12352: typed inks between two 8-bit levels (30% is stored as 77/255) still match the
        // pixels filled with them, as the foreground already does.
        const INKS: [f32; 4] = [0.3, 0.0, 0.0, 0.0];
        const STAND_IN: [f32; 4] = [0.7, 1.0, 1.0, 1.0];
        let fmt = PixelFormat::new(ColorMode::Cmyk, SampleType::U8, true);
        let mut s = Surface::new(fmt);
        s.fill_rect(Rect::new(0, 0, 80, 40), &[INKS[0], INKS[1], INKS[2], INKS[3], 1.0]);
        let bs = BgEraseSettings {
            sampling: Sampling::BackgroundSwatch,
            limits: Limits::Discontiguous,
            tolerance: 0.01,
            background: STAND_IN,
            background_cmyk: Some(([STAND_IN[0], STAND_IN[1], STAND_IN[2]], INKS)),
            ..Default::default()
        };
        apply_background_eraser(&mut s, &stroke(&[(10.0, 20.0), (70.0, 20.0)]), &bs, None);
        assert_eq!(alpha(&s, 40, 20), 0.0, "the stored inks are erased fully");
    }

    #[test]
    fn one_pass_clears_the_whole_stroke_when_dabs_overlap_erased_pixels() {
        // #1280: later dabs land their hotspot on pixels earlier dabs already erased.
        let fmt = PixelFormat::RGBA8;
        let mut s = Surface::new(fmt);
        s.fill_rect(Rect::new(0, 0, 60, 30), &from_rgba(&fmt, GREEN));
        let mut st = stroke(&[(10.0, 15.0), (50.0, 15.0)]);
        st.brush.size = 20.0;
        for sampling in [Sampling::Continuous, Sampling::Once] {
            let mut t = s.clone();
            let bs = BgEraseSettings { sampling, limits: Limits::Contiguous, ..Default::default() };
            apply_background_eraser(&mut t, &st, &bs, None);
            // Everything within radius − 1 of the line.
            for y in 6..24 {
                for x in 10..51 {
                    assert_eq!(alpha(&t, x, y), 0.0, "{sampling:?} ({x}, {y}) left behind");
                }
            }
        }
    }

    #[test]
    fn selection_limits_and_degenerate_inputs() {
        let fmt = PixelFormat::RGBA8;
        let mut s = setup(fmt);
        let mut sel = Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::new(0, 0, 20, 40), &[1.0]);
        let bs = BgEraseSettings { sampling: Sampling::Once, limits: Limits::Discontiguous, tolerance: 0.05, ..Default::default() };
        apply_background_eraser(&mut s, &stroke(&[(20.0, 20.0)]), &bs, Some(&sel));
        assert_eq!(alpha(&s, 15, 20), 0.0);
        assert_eq!(alpha(&s, 25, 20), 1.0, "outside the selection kept");
        // Sampling a transparent hotspot erases nothing; NaN tolerance acts as 0; no alpha → no-op.
        let mut t = Surface::new(fmt);
        assert!(apply_background_eraser(&mut t, &stroke(&[(5.0, 5.0)]), &bs, None).width() > 0);
        assert_eq!(t.content_bounds(), Rect::EMPTY);
        let mut u = setup(fmt);
        let nan = BgEraseSettings { tolerance: f32::NAN, ..bs.clone() };
        apply_background_eraser(&mut u, &stroke(&[(20.0, 20.0)]), &nan, None);
        assert_eq!(alpha(&u, 20, 20), 0.0, "exact match still erased at tolerance 0");
        let mut g = Surface::new(PixelFormat::GRAY8);
        assert!(apply_background_eraser(&mut g, &stroke(&[(5.0, 5.0)]), &bs, None).is_empty());
    }
}
