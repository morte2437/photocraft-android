//! Pattern Stamp: paint with the current pattern through a brush stroke.
//!
//! Aligned strokes lock the tile origin in document space. Unaligned strokes restart the origin at
//! the first point of that stroke. Impressionist jitters each dab's phase with a seeded RNG so a
//! replay of the same points is identical.

use photocraft_color::{BlendMode, PixelFormat};
use photocraft_compose::pattern::{Placement, Tile, render};
use photocraft_doc::{LayerId, Pattern};
use photocraft_geom::Rect;
use photocraft_paint::retouch::{Footprint, Region, apply_coverage, apply_dab_stroke, over_native};
use photocraft_paint::{Stroke, StrokePoint};
use photocraft_raster::{Surface, from_rgba_into, to_rgba};
use serde_json::{Value, json};

use super::{bad, blend_param, damage_json, flag, has_pixel_layer, parse_brush, reject_huge_stroke};
use crate::{Result, Session};

const STREAM_PHASE_X: u64 = 101;
const STREAM_PHASE_Y: u64 = 102;

pub(super) fn enabled(s: &Session) -> std::result::Result<(), String> {
    has_pixel_layer(s)?;
    if crate::pattern_cmds::resolve(s, "").is_none() { Err("no pattern selected".into()) } else { Ok(()) }
}

/// A parsed `paint.patternStamp`: shared by the command and its live preview, so both paint the
/// same stroke (seeded the same way) with the same tile and placement.
pub(super) struct Setup {
    pub(super) stroke: Stroke,
    pub(super) id: Option<LayerId>,
    pub(super) mode: BlendMode,
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
        let mode = blend_param(p, CMD)?;
        let aligned = flag(p, "aligned", true);
        let impressionist = flag(p, "impressionist", false);
        let pat = crate::pattern_cmds::resolve_param(s, CMD, p)?;
        if pat.is_empty() {
            return Err(bad(CMD, "the pattern is empty"));
        }
        let scale = stamp_scale(p, CMD)?;
        let angle = stamp_angle(p, CMD)?;
        let Some(first) = stroke.points.first() else {
            return Err(bad(CMD, "`points` is empty"));
        };
        let phase = stamp_phase(p, CMD, aligned, first)?;
        let tile = std::sync::Arc::new(Tile::new(&pat).ok_or_else(|| bad(CMD, "the pattern is empty or too large to paint"))?);
        reject_huge_stroke(CMD, &stroke)?;
        let seed = p.get("seed").and_then(Value::as_u64).unwrap_or_else(|| {
            let bytes = p.get("points").map(std::string::ToString::to_string).unwrap_or_default();
            photocraft_paint::rng::seed_from_bytes(bytes.as_bytes())
        });
        stroke.brush.seed = seed;
        Ok(Self { stroke, id, mode, aligned, impressionist, pat, tile, phase, scale, angle, seed })
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
            Ok(apply_dab_stroke(surf, &st.stroke, sel, lock, 0, effect))
        } else {
            paint_aligned(surf, &st.stroke, sel, lock, &st.tile, st.placement(), st.mode, CMD)
        }
    })?;
    if let Some(state) = s.active_mut() {
        state.last_damage = Some(dmg);
    }
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

fn stamp_phase(p: &Value, cmd: &str, aligned: bool, first: &StrokePoint) -> Result<(f32, f32)> {
    match p.get("phase") {
        None => {
            if aligned {
                Ok((0.0, 0.0))
            } else {
                Ok((first.x as f32, first.y as f32))
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

#[allow(clippy::too_many_arguments)]
fn paint_aligned(
    surf: &mut Surface,
    stroke: &Stroke,
    sel: Option<&Surface>,
    lock: bool,
    tile: &Tile,
    place: Placement,
    mode: BlendMode,
    cmd: &str,
) -> Result<Rect> {
    let (bounds, cov) = super::coverage(cmd, stroke)?;
    if bounds.is_empty() {
        return Ok(Rect::EMPTY);
    }
    let paint = paint_region(tile, &place, &surf.format(), bounds);
    Ok(apply_coverage(surf, bounds, &cov, stroke.brush.opacity, sel, lock, &paint, mode))
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
