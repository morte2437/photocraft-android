use super::clone::{blend_param, clamp_samples, clone_mapping, clone_result, clone_sample};
use super::sampling::{SampleLayers, composite_region, sample_all_layers, sample_layers};
use super::{
    BlendMode, CompleteParams, Document, EngineError, LayerId, PixelFormat, Rect, Region, Result, Session, Stroke, Surface, Value, apply_coverage, bad,
    coverage, damage_json, flag, inpaint, json, parse_brush, poisson, reject_huge_stroke, run_stroke, string,
};

/// Grow a stroke's coverage map by `m` pixels of zero coverage (a Dirichlet boundary for the solve).
fn pad_coverage(bounds: Rect, cov: &[f32], m: i32) -> (Rect, Vec<f32>) {
    let g = bounds.inflate(m);
    let (w, gw) = (bounds.width() as usize, g.width() as usize);
    let mut out = vec![0.0f32; gw * g.height() as usize];
    for (i, c) in cov.iter().enumerate() {
        let (x, y) = (i % w + m as usize, i / w + m as usize);
        out[y * gw + x] = *c;
    }
    (g, out)
}

/// Seamless-clone `src` into `dst` over the mask (both regions cover the same rect). The solve runs
/// only on the mask's bounding box plus a one-pixel Dirichlet ring; the rest is `dst`.
pub(super) fn heal_region(fmt: &PixelFormat, src: &Region, dst: &Region, mask: &[bool]) -> Region {
    let w = dst.width();
    let bbox = mask.iter().enumerate().filter(|(_, m)| **m).fold(Rect::EMPTY, |r, (i, _)| {
        let (x, y) = (dst.rect.x0 + (i % w) as i32, dst.rect.y0 + (i / w) as i32);
        r.union(&Rect::new(x, y, x + 1, y + 1))
    });
    let mut out = dst.clone();
    if bbox.is_empty() {
        return out;
    }
    let r = bbox.inflate(1).intersect(&dst.rect);
    let (s, d) = (src.crop(r), dst.crop(r));
    let m: Vec<bool> = (r.y0..r.y1).flat_map(|y| (r.x0..r.x1).map(move |x| (x, y))).map(|(x, y)| mask[dst.index(x, y)]).collect();
    let mut data = poisson::seamless_clone(d.width(), d.height(), d.ch, &s.data, &d.data, &m);
    clamp_samples(fmt, &mut data);
    let (n, rw) = (dst.ch, r.width() as usize);
    for y in r.y0..r.y1 {
        let o = out.index(r.x0, y) * n;
        let i = (y - r.y0) as usize * rw * n;
        out.data[o..o + rw * n].copy_from_slice(&data[i..i + rw * n]);
    }
    out
}

pub(super) fn healing_brush(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "paint.healingBrush";
    let (stroke, id) = parse_brush(s, p, CMD)?;
    reject_huge_stroke(CMD, &stroke)?;
    let which = sample_layers(p, CMD)?;
    let mode = blend_param(p, CMD)?;
    let aligned = flag(p, "aligned", true);
    let map = clone_mapping(s, p, &stroke, CMD)?;
    let off = map.offset();
    let dmg = run_stroke(s, "Healing Brush", id, p, |pre, surf, sel, lock| {
        let (bounds, cov) = coverage(CMD, &stroke)?;
        let (g, cov) = pad_coverage(bounds, &cov, 2);
        let fmt = surf.format();
        let src = clone_sample(pre, id, surf, which, g, &map, false, CMD)?;
        // The destination the texture is fitted to: the layer itself, or what the user sees when
        // sampling several layers (so healing onto an empty layer matches the composite).
        let dst = if which == SampleLayers::Current { Region::read(surf, g) } else { composite_region(pre, id, which, g, fmt) };
        let mask: Vec<bool> = cov.iter().map(|c| *c > 0.0).collect();
        let healed = heal_region(&fmt, &src, &dst, &mask);
        Ok(apply_coverage(surf, g, &cov, stroke.brush.opacity, sel, lock, &healed, mode))
    })?;
    Ok(clone_result(dmg, off, aligned, p, &stroke))
}

/// The History Brush's source: the targeted surface (pixels, mask, channel) as it was in History
/// panel row `state` or snapshot `snapshot` (default: the source set in the History panel,
/// `history.setSource`, else the oldest state still held, the "Open" state unless trimmed).
pub(super) fn history_source(s: &Session, id: Option<LayerId>, p: &Value, cmd: &str) -> Result<Surface> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let source_doc = crate::history_source_cmds::paint_source(st, p, cmd)?;
    crate::channel_cmds::read_target_surface(source_doc.as_ref(), id, p)
        .cloned()
        .map_err(|_| EngineError::Other("the target did not exist (as pixels, a mask or a channel) in that history state".into()))
}

pub(super) fn history_brush(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "paint.historyBrush";
    let (stroke, id) = parse_brush(s, p, CMD)?;
    let mode = blend_param(p, CMD)?;
    // The coverage and the source region below are both dense over the stroke's bounds, so a
    // thin stroke across a large canvas would still ask for the whole box. Refuse it from the
    // points and brush size, before the source surface is cloned or anything is rendered (#4961).
    reject_huge_stroke(CMD, &stroke)?;
    let live = s.live_commit.take().filter(|l| l.matches(s, CMD, p, &stroke.points));
    let from_live = live.is_some();
    let src_surface = history_source(s, id, p, CMD)?;
    let dmg = run_stroke(s, "History Brush", id, p, |_, surf, sel, lock| {
        if let Some(l) = live {
            *surf = l.surface;
            return Ok(l.bounds);
        }
        let fmt = surf.format();
        let src = if src_surface.format() == fmt { src_surface } else { src_surface.convert(fmt) };
        let (bounds, cov) = coverage(CMD, &stroke)?;
        let paint = Region::read(&src, bounds);
        Ok(apply_coverage(surf, bounds, &cov, stroke.brush.opacity, sel, lock, &paint, mode))
    })?;
    let mut r = json!({ "damage": damage_json(dmg) });
    if from_live {
        r["fromLive"] = json!(true);
    }
    Ok(r)
}

// ---------------------------------------------------------------------------------------------
// Spot Healing
// ---------------------------------------------------------------------------------------------

/// Dilate a mask by `r` pixels (square structuring element).
fn dilate(w: usize, h: usize, m: &[bool], r: usize) -> Vec<bool> {
    let mut tmp = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            if m[y * w + x] {
                for xx in x.saturating_sub(r)..(x + r + 1).min(w) {
                    tmp[y * w + xx] = true;
                }
            }
        }
    }
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            if tmp[y * w + x] {
                for yy in y.saturating_sub(r)..(y + r + 1).min(h) {
                    out[yy * w + x] = true;
                }
            }
        }
    }
    out
}

/// Copy a translated Proximity Match patch from the *pre-healing* pixels. Reading
/// `out` here would make overlapping offsets repeatedly copy the pixels just written,
/// leaving streaks instead of the selected source texture.
pub(super) fn proximity_source_patch(img: &[f32], w: usize, h: usize, ch: usize, mask: &[bool], dx: i32, dy: i32) -> Option<Vec<f32>> {
    if w == 0 || h == 0 || ch == 0 || mask.len() != w.checked_mul(h)? || img.len() != mask.len().checked_mul(ch)? {
        return None;
    }
    let mut out = img.to_vec();
    for (p, masked) in mask.iter().enumerate() {
        if !masked {
            continue;
        }
        let x = i32::try_from(p % w).ok()?.checked_add(dx)?;
        let y = i32::try_from(p / w).ok()?.checked_add(dy)?;
        if x < 0 || y < 0 || x >= i32::try_from(w).ok()? || y >= i32::try_from(h).ok()? {
            return None;
        }
        let src = (usize::try_from(y).ok()?.checked_mul(w)? + usize::try_from(x).ok()?).checked_mul(ch)?;
        let dst = p.checked_mul(ch)?;
        out.get_mut(dst..dst + ch)?.copy_from_slice(img.get(src..src + ch)?);
    }
    Some(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpotType {
    ContentAware,
    CreateTexture,
    ProximityMatch,
}

/// Fill the stroke area with no source, then gradient-domain blend it into its surroundings. With
/// `all` (Sample All Layers) it heals what is visible, so it works on an empty layer above the image.
///
/// Photoshop 25.4 (measured): a dab changes exactly the brush's footprint, and Content-Aware takes
/// its texture from around it, as Edit › Fill does from its window
/// ([`crate::fill_cmds::sampling_window`] of the stroke's bounds), which is what this samples.
fn spot_heal_surface(surf: &mut Surface, pre: &Document, stroke: &Stroke, kind: SpotType, all: bool, sel: Option<&Surface>, lock: bool) -> Result<Rect> {
    let (bounds, cov) = coverage("paint.spotHealing", stroke)?;
    let size = stroke.brush.size;
    let region_rect = crate::fill_cmds::sampling_window(bounds, pre.bounds());
    if region_rect.is_empty() {
        return Ok(Rect::EMPTY);
    }
    // Proximity Match looks for its source anywhere in that window.
    let margin = (region_rect.width().max(region_rect.height()) / 2) as i32;
    let fmt = surf.format();
    let img = if all { composite_region(pre, None, SampleLayers::All, region_rect, fmt) } else { Region::read(surf, region_rect) };
    let (w, h, ch) = (img.width(), img.height(), img.ch);
    let bw = bounds.width() as usize;
    let mut stroke_cov = vec![0.0f32; w * h];
    for (i, c) in cov.iter().enumerate() {
        let (x, y) = (bounds.x0 + (i % bw) as i32, bounds.y0 + (i / bw) as i32);
        if region_rect.contains(x, y) {
            stroke_cov[img.index(x, y)] = *c;
        }
    }
    let hole: Vec<bool> = stroke_cov.iter().map(|c| *c > 0.0).collect();
    if !hole.iter().any(|h| *h) {
        return Ok(Rect::EMPTY);
    }
    // Synthesise over a slightly larger domain than the Poisson region, so the solve's boundary pixels
    // carry synthesised values that differ from the untouched surroundings: that mismatch is what the
    // membrane corrects (otherwise the Dirichlet data would be all zero and the blend a no-op).
    let domain = dilate(w, h, &hole, 2);
    let heal_mask = dilate(w, h, &hole, 1);
    let content_aware =
        |img: &[f32]| inpaint::complete(w, h, ch, img, &domain, &CompleteParams::default()).unwrap_or_else(|| poisson::membrane_fill(w, h, ch, img, &domain));
    let filled = match kind {
        SpotType::ContentAware => content_aware(&img.data),
        SpotType::CreateTexture => inpaint::synthesize(w, h, ch, &img.data, &domain, (size / 4.0).clamp(6.0, 32.0) as usize, 0x5eed),
        SpotType::ProximityMatch => {
            let ring = (size / 8.0).clamp(3.0, 16.0) as usize;
            match inpaint::best_offset(w, h, ch, &img.data, &domain, ring, margin) {
                Some((dx, dy)) => proximity_source_patch(&img.data, w, h, ch, &domain, dx, dy).unwrap_or_else(|| content_aware(&img.data)),
                None => content_aware(&img.data),
            }
        }
    };
    let src = Region { rect: region_rect, ch, data: filled };
    let healed = heal_region(&fmt, &src, &img, &heal_mask);
    let dmg = bounds.intersect(&region_rect);
    let dmg_cov: Vec<f32> = (dmg.y0..dmg.y1).flat_map(|y| (dmg.x0..dmg.x1).map(move |x| (x, y))).map(|(x, y)| stroke_cov[img.index(x, y)]).collect();
    Ok(apply_coverage(surf, dmg, &dmg_cov, stroke.brush.opacity, sel, lock, &healed, BlendMode::Normal))
}

pub(super) fn spot_healing(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "paint.spotHealing";
    let (stroke, id) = parse_brush(s, p, CMD)?;
    reject_huge_stroke(CMD, &stroke)?;
    let kind = match string(p, "type", "contentAware") {
        "contentAware" => SpotType::ContentAware,
        "createTexture" => SpotType::CreateTexture,
        "proximityMatch" => SpotType::ProximityMatch,
        o => return Err(bad(CMD, format!("unknown type `{o}` (contentAware|createTexture|proximityMatch)"))),
    };
    let all = sample_all_layers(p);
    let dmg = run_stroke(s, "Spot Healing Brush", id, p, |pre, surf, sel, lock| spot_heal_surface(surf, pre, &stroke, kind, all, sel, lock))?;
    Ok(json!({ "damage": damage_json(dmg) }))
}
