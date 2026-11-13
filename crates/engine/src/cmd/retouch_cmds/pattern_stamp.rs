//! Pattern Stamp: paint with the current pattern through a brush stroke.
//!
//! Aligned strokes lock the tile origin in document space. Unaligned strokes derive a new origin
//! from the tile size and the first point of each stroke. Impressionist jitters each dab's phase
//! with a seeded RNG so a replay of the same points is identical.
//!
//! `"mode": "behind"` is the painting-only Behind mode, as the Brush's: the pattern goes beneath
//! the layer's pixels, so it shows only where the layer is transparent (#12879). Behind with
//! Impressionist was not measured: those strokes lay their dabs over the pixels in Normal, as
//! they did before Behind existed.

use photocraft_color::{BlendMode, ColorMode, PixelFormat};
use photocraft_compose::pattern::{Placement, Tile, render};
use photocraft_doc::{LayerId, Pattern};
use photocraft_geom::Rect;
use photocraft_paint::retouch::{Footprint, Region, alpha_index, apply_coverage, apply_dab_stroke, over_native};
use photocraft_paint::{Stroke, StrokePoint};
use photocraft_raster::{Surface, from_rgba_into, to_rgba};
use serde_json::{Value, json};

use super::{bad, blend_param, damage_json, flag, has_pixel_layer, parse_brush, reject_huge_stroke};
use crate::{EngineError, Result, Session};

const STREAM_PHASE_X: u64 = 101;
const STREAM_PHASE_Y: u64 = 102;

/// The reference's alert for a Pattern Stamp stroke in an Indexed Color document (#12887).
pub const INDEXED_REFUSAL: &str = "Could not fill because the pattern and image have different color modes.";

/// The reference refuses every pattern in an Indexed Color document before painting: RGB,
/// Grayscale, CMYK, Lab and Indexed ones from another image. It paints only a pattern with the
/// document's own colour table, which PhotoCraft can't tell apart, so that one is refused too.
/// Bitmap, Duotone and Multichannel documents weren't measured and paint as before.
fn mode_refusal(s: &Session) -> Result<()> {
    if s.active().is_some_and(|st| st.doc.mode == ColorMode::Indexed) { Err(EngineError::Other(INDEXED_REFUSAL.into())) } else { Ok(()) }
}

/// [`mode_refusal`] for a press with the stroke's `p`, so the canvas can alert before the stroke
/// starts: `None` when the pattern paints, or when `p` names no pattern (the stroke reports that).
pub fn press_refusal(s: &Session, p: &Value) -> Option<String> {
    mode_refusal(s).err()?;
    crate::pattern_cmds::resolve_param(s, "paint.patternStamp", p).ok()?;
    Some(INDEXED_REFUSAL.into())
}

pub(super) fn enabled(s: &Session) -> std::result::Result<(), String> {
    has_pixel_layer(s)?;
    if crate::pattern_cmds::resolve_id(s, "").is_none() { Err("no pattern selected".into()) } else { Ok(()) }
}

/// A parsed `paint.patternStamp`: shared by the command and its live preview, so both paint the
/// same stroke (seeded the same way) with the same tile and placement.
pub(super) struct Setup {
    pub(super) stroke: Stroke,
    pub(super) id: Option<LayerId>,
    pub(super) mode: BlendMode,
    /// Behind: paint beneath the layer's pixels (`mode` is then Normal). Aligned strokes only:
    /// Impressionist dabs are laid in `mode`.
    pub(super) behind: bool,
    aligned: bool,
    pub(super) impressionist: bool,
    pat: Pattern,
    pub(super) tile: std::sync::Arc<Tile>,
    phase: (f32, f32),
    scale: f32,
    angle: f32,
    seed: u64,
}

impl Setup {
    pub(super) fn parse(s: &Session, p: &Value) -> Result<Self> {
        const CMD: &str = "paint.patternStamp";
        let (mut stroke, id) = parse_brush(s, p, CMD)?;
        // Behind is a painting mode, not a blend mode: the pattern itself is laid in Normal.
        let behind = p.get("mode").and_then(Value::as_str).is_some_and(|m| m.eq_ignore_ascii_case("behind"));
        let mode = if behind { BlendMode::Normal } else { blend_param(p, CMD)? };
        let aligned = flag(p, "aligned", true);
        let impressionist = flag(p, "impressionist", false);
        let pat = crate::pattern_cmds::resolve_param(s, CMD, p)?;
        mode_refusal(s)?;
        if pat.is_empty() {
            return Err(bad(CMD, "the pattern is empty"));
        }
        let scale = stamp_scale(p, CMD)?;
        let angle = stamp_angle(p, CMD)?;
        let Some(first) = stroke.points.first() else {
            return Err(bad(CMD, "`points` is empty"));
        };
        let phase = stamp_phase(p, CMD, aligned, first, (pat.width, pat.height))?;
        let tile = std::sync::Arc::new(Tile::new(&pat).ok_or_else(|| bad(CMD, "the pattern is empty or too large to paint"))?);
        reject_huge_stroke(CMD, &stroke)?;
        let seed = p.get("seed").and_then(Value::as_u64).unwrap_or_else(|| {
            let bytes = p.get("points").map(std::string::ToString::to_string).unwrap_or_default();
            photocraft_paint::rng::seed_from_bytes(bytes.as_bytes())
        });
        stroke.brush.seed = seed;
        Ok(Self { stroke, id, mode, behind, aligned, impressionist, pat, tile, phase, scale, angle, seed })
    }

    /// Where the pattern sits (aligned strokes; Impressionist jitters it per dab).
    pub(super) fn placement(&self) -> Placement {
        placement(self.phase, self.scale, self.angle)
    }

    /// One Impressionist dab's edit of a working copy in `fmt`.
    fn impressionist_effect(&self, fmt: PixelFormat) -> super::DabEffect {
        let (tile, phase, scale, angle, seed, mode) = (self.tile.clone(), self.phase, self.scale, self.angle, self.seed, self.mode);
        let (tw, th) = (self.pat.width as f32 * scale, self.pat.height as f32 * scale);
        Box::new(move |work, fp| stamp_dab(work, &fmt, &tile, fp, phase, scale, angle, seed, tw, th, mode))
    }
}

/// `src` beneath `dst` at coverage `k`, in native channels with alpha at `a`: the Brush's Behind.
/// Opaque pixels are unchanged; partly transparent ones keep their colour's share.
fn under_native(a: usize, dst: &mut [f32], src: &[f32], k: f32) {
    let (Some(&da), Some(&sa)) = (dst.get(a), src.get(a)) else { return };
    if da >= 1.0 {
        return;
    }
    let under = sa * k * (1.0 - da);
    let alpha = da + under;
    if alpha <= 0.0 {
        return;
    }
    for (d, s) in dst.iter_mut().zip(src).take(a) {
        *d = (*d * da + *s * under) / alpha;
    }
    if let Some(d) = dst.get_mut(a) {
        *d = alpha;
    }
}

/// [`apply_coverage`], or with `behind` the same weights (`cov × opacity × selection`) laid
/// beneath the pixels of `target`: shared by the commit and the live preview. Behind paints
/// nothing where there is no transparency to paint into: a target without alpha (the Background,
/// a mask, a channel) or with transparency locked.
#[allow(clippy::too_many_arguments)]
pub(super) fn lay_coverage(
    target: &mut Surface,
    rect: Rect,
    cov: &[f32],
    opacity: f32,
    sel: Option<&Surface>,
    lock: bool,
    paint: &Region,
    mode: BlendMode,
    behind: bool,
) -> Rect {
    if !behind {
        return apply_coverage(target, rect, cov, opacity, sel, lock, paint, mode);
    }
    let fmt = target.format();
    let (Some(a), false, false) = (alpha_index(&fmt), lock, rect.is_empty()) else { return Rect::EMPTY };
    let n = fmt.channels();
    let w = rect.width() as usize;
    let mut region = target.read_region(rect);
    // The selection row under the current coverage row.
    let mut sel_row = vec![1.0f32; if sel.is_some() { w } else { 0 }];
    for (row, (pixels, cov)) in region.chunks_exact_mut(w.saturating_mul(n)).zip(cov.chunks_exact(w)).enumerate() {
        let y = rect.y0.saturating_add(row as i32);
        if let Some(s) = sel {
            s.sample_row_strided(y, rect.x0, 1, 0, &mut sel_row);
        }
        for (col, (px, c)) in pixels.chunks_exact_mut(n).zip(cov).enumerate() {
            let k = (c * opacity * sel_row.get(col).copied().unwrap_or(1.0)).min(1.0);
            if k > 0.0 {
                under_native(a, px, paint.px(rect.x0.saturating_add(col as i32), y), k);
            }
        }
    }
    target.write_region(rect, &region);
    rect
}

pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "paint.patternStamp";
    let st = Setup::parse(s, p)?;
    let dmg = s.edit("Pattern Stamp", |doc, _| {
        crate::pattern_cmds::ensure_in_doc(doc, &st.pat);
        let pre = doc.clone();
        let (surf, lock) = crate::channel_cmds::target_surface(doc, st.id, p)?;
        let sel = pre.selection.as_ref();
        if st.impressionist {
            // Halo is the dab footprint only: Impressionist jitters phase, it does not blur.
            let effect = st.impressionist_effect(surf.format());
            apply_dab_stroke(surf, &st.stroke, sel, lock, 0, effect).map_err(|e| bad(CMD, e.to_string()))
        } else {
            paint_aligned(surf, &st, sel, lock, CMD)
        }
    })?;
    s.hint_damage(dmg);
    Ok(json!({
        "damage": damage_json(dmg),
        "phase": [st.phase.0, st.phase.1],
        "aligned": st.aligned,
        "pattern": st.pat.id,
    }))
}

/// A Pattern Stamp stroke with Impressionist on, shown while it is drawn: the same jittered dabs,
/// laid one by one as the commit lays them (`apply_dab_stroke`), so the preview at the last point
/// is the committed result. Aligned strokes are live through [`super::LiveRetouch`].
pub fn live_impressionist(s: &Session, p: &Value) -> Result<super::LiveDab> {
    const CMD: &str = "paint.patternStamp";
    has_pixel_layer(s).map_err(crate::EngineError::Other)?;
    let st = Setup::parse(s, p)?;
    if !st.impressionist {
        return Err(bad(CMD, "only Impressionist strokes are laid dab by dab; aligned strokes preview through LiveRetouch"));
    }
    let (stroke, id) = (st.stroke.clone(), st.id);
    let doc = (*s.active().ok_or(crate::EngineError::NoDocument)?.doc).clone();
    super::LiveDab::with_effect_on(doc, CMD, p, stroke, id, 0, |pre| st.impressionist_effect(pre.format()))?.with_tail_preview()
}

/// The pattern under destination `rect` in `fmt` (native channels): what an aligned stroke paints
/// through its coverage.
pub(super) fn paint_region(tile: &Tile, place: &Placement, fmt: &PixelFormat, rect: Rect) -> Region {
    rgba_to_region(fmt, rect, &render(tile, place, rect))
}

fn stamp_scale(p: &Value, cmd: &str) -> Result<f32> {
    match p.get("scale") {
        None => Ok(1.0),
        Some(v) => {
            let s = v.as_f64().ok_or_else(|| bad(cmd, "`scale` must be a number (1..1000 %)"))?;
            if !s.is_finite() || !(1.0..=1000.0).contains(&s) {
                return Err(bad(cmd, "scale must be 1..1000 %"));
            }
            Ok((s as f32) / 100.0)
        }
    }
}

fn stamp_angle(p: &Value, cmd: &str) -> Result<f32> {
    match p.get("angle") {
        None => Ok(0.0),
        Some(v) => {
            let a = v.as_f64().ok_or_else(|| bad(cmd, "`angle` must be a number (degrees)"))?;
            if !a.is_finite() {
                return Err(bad(cmd, "`angle` must be finite"));
            }
            Ok(a as f32)
        }
    }
}

fn stamp_phase(p: &Value, cmd: &str, aligned: bool, first: &StrokePoint, size: (u32, u32)) -> Result<(f32, f32)> {
    match p.get("phase") {
        None => {
            if aligned {
                Ok((0.0, 0.0))
            } else {
                // Each axis starts from half its tile plus one pixel, mirrored about the
                // stroke's first point. Keep one phase for the complete stroke, independent
                // of its brush tip. Empty patterns have already been rejected by Setup.
                let origin = |first: f64, edge: u32| (f64::from(edge / 2 + 1) - first).rem_euclid(f64::from(edge)) as f32;
                Ok((origin(first.x, size.0), origin(first.y, size.1)))
            }
        }
        Some(v) => {
            let a = v.as_array().ok_or_else(|| bad(cmd, "`phase` must be [px, py]"))?;
            let x = a.first().and_then(Value::as_f64).ok_or_else(|| bad(cmd, "`phase` must be [px, py]"))?;
            let y = a.get(1).and_then(Value::as_f64).ok_or_else(|| bad(cmd, "`phase` must be [px, py]"))?;
            if !x.is_finite() || !y.is_finite() {
                return Err(bad(cmd, "`phase` coordinates must be finite"));
            }
            Ok((x as f32, y as f32))
        }
    }
}

fn placement(phase: (f32, f32), scale: f32, angle: f32) -> Placement {
    Placement::new(Rect::EMPTY, false, phase, scale, angle)
}

fn rgba_to_region(fmt: &PixelFormat, rect: Rect, rgba: &[[f32; 4]]) -> Region {
    let n = fmt.channels();
    let mut paint = Region::new(rect, n);
    let mut enc = [0.0f32; 8];
    for (i, px) in rgba.iter().enumerate() {
        from_rgba_into(fmt, *px, &mut enc);
        let Some(start) = i.checked_mul(n) else { continue };
        let Some(end) = start.checked_add(n) else { continue };
        if let (Some(dst), Some(src)) = (paint.data.get_mut(start..end), enc.get(..n)) {
            dst.copy_from_slice(src);
        }
    }
    paint
}

fn paint_aligned(surf: &mut Surface, st: &Setup, sel: Option<&Surface>, lock: bool, cmd: &str) -> Result<Rect> {
    let (bounds, cov) = super::coverage(cmd, &st.stroke)?;
    if bounds.is_empty() {
        return Ok(Rect::EMPTY);
    }
    let paint = paint_region(&st.tile, &st.placement(), &surf.format(), bounds);
    Ok(lay_coverage(surf, bounds, &cov, st.stroke.brush.opacity, sel, lock, &paint, st.mode, st.behind))
}

fn footprint_cov(fp: &Footprint, x: i32, y: i32) -> f32 {
    if x < fp.rect.x0 || y < fp.rect.y0 {
        return 0.0;
    }
    let dx = x.wrapping_sub(fp.rect.x0) as u32;
    let dy = y.wrapping_sub(fp.rect.y0) as u32;
    let w = fp.rect.width();
    let idx = (dy as usize).saturating_mul(w as usize).saturating_add(dx as usize);
    fp.cov.get(idx).copied().unwrap_or(0.0)
}

#[allow(clippy::too_many_arguments)]
fn stamp_dab(
    work: &mut Region,
    fmt: &PixelFormat,
    tile: &Tile,
    fp: &Footprint,
    phase: (f32, f32),
    scale: f32,
    angle: f32,
    seed: u64,
    tw: f32,
    th: f32,
    mode: BlendMode,
) {
    let r = fp.rect.intersect(&work.rect);
    if r.is_empty() {
        return;
    }
    let i = fp.index as u64;
    let jx = photocraft_paint::rng::rand_signed(seed, i, STREAM_PHASE_X) * tw;
    let jy = photocraft_paint::rng::rand_signed(seed, i, STREAM_PHASE_Y) * th;
    let place = placement((phase.0 + jx, phase.1 + jy), scale, angle);
    let rgba = render(tile, &place, r);
    let rw = r.width() as usize;
    let n = work.ch;
    let mut enc = [0.0f32; 8];
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let k = footprint_cov(fp, x, y);
            if k <= 0.0 {
                continue;
            }
            let ox = (x - r.x0) as usize;
            let oy = (y - r.y0) as usize;
            let Some(px) = rgba.get(oy.saturating_mul(rw).saturating_add(ox)) else { continue };
            from_rgba_into(fmt, *px, &mut enc);
            let dst = work.px_mut(x, y);
            if mode == BlendMode::Normal {
                if let Some(src) = enc.get(..n) {
                    over_native(fmt, dst, src, k, false);
                }
            } else {
                let d = to_rgba(fmt, dst);
                let o = photocraft_color::blend::composite(mode, d, *px, k);
                from_rgba_into(fmt, o, &mut enc);
                if let Some(src) = enc.get(..n) {
                    dst.copy_from_slice(src);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "pattern_stamp_origin_tests.rs"]
mod pattern_stamp_origin_tests;

#[cfg(test)]
#[path = "pattern_stamp_behind_tests.rs"]
mod pattern_stamp_behind_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(points: &[(f64, f64)], size: f32) -> Stroke {
        let brush = photocraft_paint::BrushSettings { size, ..Default::default() };
        Stroke { brush, points: points.iter().map(|&(x, y)| StrokePoint::new(x, y, 1.0)).collect() }
    }

    #[test]
    fn a_diagonal_across_a_24_mp_canvas_is_not_too_large() {
        let across = stroke(&[(0.0, 0.0), (6015.0, 3999.0)], 100.0);
        assert!(reject_huge_stroke("t", &across).is_ok());
        assert!(super::super::coverage("t", &across).is_ok(), "the rendered extent fits too");
        assert!(reject_huge_stroke("t", &stroke(&[(0.0, 0.0), (9000.0, 9000.0)], 100.0)).is_err());
    }
}

#[cfg(test)]
#[path = "pattern_stamp_indexed_tests.rs"]
mod pattern_stamp_indexed_tests;
