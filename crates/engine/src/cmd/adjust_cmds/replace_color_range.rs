//! Recorded Replace Color samples are a Lab box, not a single RGB colour.
use craft_cms::{Builtin, TransformOptions};
use photocraft_algo::{selection, tone};
use photocraft_doc::{PixelFormat, SampleType};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::Surface;
use serde_json::Value;

use super::{bad, num};
use crate::{EngineError, Result, Session};

const CMD: &str = "image.adjustments.replaceColor";

/// Unstored pixels can still be selected when the mask has a nonzero default.
fn edit_bounds(content: Rect, selected: Option<&Surface>) -> Rect {
    match selected {
        Some(mask) if mask.default_pixel().first() == Some(&0.0) => content.intersect(&mask.content_bounds()),
        _ => content,
    }
}

/// Validate original precision before checking order and converting to native f32.
fn bound(p: &Value, key: &str) -> Result<[f64; 3]> {
    let values = p.get(key).and_then(Value::as_array).filter(|a| a.len() == 3).ok_or_else(|| bad(CMD, format!("`{key}` must be three finite Lab numbers")))?;
    let mut result = [0.0; 3];
    for (out, value) in result.iter_mut().zip(values) {
        let n = value.as_f64().filter(|n| n.is_finite() && (*n as f32).is_finite()).ok_or_else(|| bad(CMD, format!("invalid `{key}` Lab number")))?;
        *out = n;
    }
    Ok(result)
}

/// Apply the same Lab match, tone shift and selection mix to stored and implicit pixels.
fn replace_sample(format: &PixelFormat, pixel: &mut [f32], sample: &[f32; 4], range: &selection::LabRange, fuzz: f32, shift: [f32; 3], selected: f32) -> bool {
    if sample[3].is_nan() || sample[3] <= 0.0 || selected <= 0.0 {
        return false;
    }
    let value = [sample[0] * 100.0, sample[1] * 255.0 - 128.0, sample[2] * 255.0 - 128.0];
    let coverage = if format.sample == SampleType::U8 {
        selection::lab_range_coverage_8bit(selection::quantize_lab8(value), range, fuzz)
    } else {
        selection::lab_range_coverage(value, range, fuzz)
    };
    if coverage <= 0.0 {
        return false;
    }
    let original = photocraft_raster::to_rgba(format, pixel);
    let mut edited = [original];
    tone::replace_color_masked(&mut edited, &[coverage], shift[0], shift[1], shift[2]);
    // Unchanged native samples must not take an incidental RGB roundtrip.
    if edited[0] == original {
        return false;
    }
    let mut mixed = std::array::from_fn(|c| original[c] * (1.0 - selected) + edited[0][c] * selected);
    mixed[3] = original[3];
    photocraft_raster::from_rgba_into(format, mixed, pixel);
    true
}

/// A zero-default selection or an unchanged source default needs only stored tiles.
/// Otherwise retain the full edit rectangle, subject to the existing adjustment storage budget.
fn work_tiles(surface: &Surface, selected: Option<&Surface>, bounds: Rect, default_changes: bool) -> Result<Option<Vec<TileCoord>>> {
    let sparse = selected.filter(|mask| mask.default_pixel().first() == Some(&0.0)).or_else(|| (!default_changes).then_some(surface));
    if let Some(sparse) = sparse {
        let coords = || sparse.tiles().map(|(tile, _)| *tile).filter(|tile| !tile.rect().intersect(&bounds).is_empty());
        let mut tiles = crate::allocation::capacity(coords().count(), "Replace Color tile coordinates")?;
        tiles.extend(coords());
        return Ok(Some(tiles));
    }
    let stored = surface.tiles().filter(|(tile, _)| !tile.rect().intersect(&bounds).is_empty()).count();
    crate::pixels::check_adjustment_tile_budget(bounds, stored, &surface.format()).map_err(EngineError::Other)?;
    Ok(None)
}

/// Apply explicit recorded bounds; `None` leaves the legacy RGB/sample path alone.
/// Matching converts original surface channels through their document profile,
/// before the existing RGBA tone shift can clip or change their colour space.
pub(super) fn apply(s: &mut Session, p: &Value) -> Result<Option<Value>> {
    if p.get("labMinimum").is_none() && p.get("labMaximum").is_none() {
        return Ok(None);
    }
    if ["color", "colors", "points", "subtractPoints", "order", "localized", "range", "channel", "targetChannel"].iter().any(|key| p.get(*key).is_some()) {
        return Err(bad(CMD, "Lab bounds cannot be combined with RGB samples, localized settings or channel targets"));
    }
    if p.get("target").is_some_and(|t| t.as_str() != Some("pixels")) || crate::channel_cmds::edits_channel(s) || crate::color_channels::target(s, p)?.is_some()
    {
        return Err(bad(CMD, "Lab bounds require the composite pixel-layer target"));
    }
    let (lo, hi) = (bound(p, "labMinimum")?, bound(p, "labMaximum")?);
    if lo.iter().zip(hi).any(|(lo, hi)| *lo > hi) {
        return Err(bad(CMD, "Lab minimum must not exceed maximum"));
    }
    let mut range = selection::LabRange { lo: lo.map(|v| v as f32), hi: hi.map(|v| v as f32) };
    // Validate supplied shifts before the edit rather than silently defaulting.
    for key in ["fuzziness", "hue", "saturation", "lightness"] {
        if p.get(key).is_some_and(|v| !v.as_f64().is_some_and(f64::is_finite)) {
            return Err(bad(CMD, format!("`{key}` must be a finite number")));
        }
    }
    let fuzz = num(p, "fuzziness", 40.0).clamp(0.0, 200.0);
    let (h, sat, l) = (num(p, "hue", 0.0).clamp(-180.0, 180.0), num(p, "saturation", 0.0).clamp(-100.0, 100.0), num(p, "lightness", 0.0).clamp(-100.0, 100.0));
    let id = crate::commands::layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    super::super::enabled::check_pixels_unlocked(&st.doc, id)?;
    let surface = st.doc.layer(id).and_then(|l| l.surface()).ok_or_else(|| bad(CMD, "Lab bounds require a pixel layer"))?;
    let format = surface.format();
    if format.mode != st.doc.mode {
        return Err(bad(CMD, "pixel layout does not match the document profile"));
    }
    let transform = craft_cms::cached(&crate::color_cmds::document_profile(&st.doc), Builtin::LabD50.profile(), TransformOptions::default())
        .map_err(|e| bad(CMD, format!("Lab colour transform: {e}")))?;
    if transform.inputs() != format.mode.color_channels() || transform.outputs() != 3 || format.channels() != transform.inputs() + usize::from(format.alpha) {
        return Err(bad(CMD, "unsupported Lab colour-transform layout"));
    }
    let grid = format.sample == SampleType::U8;
    if grid {
        range.lo = selection::quantize_lab8(range.lo);
        range.hi = selection::quantize_lab8(range.hi);
    }
    if h == 0.0 && sat == 0.0 && l == 0.0 {
        // Validation still runs, but a zero shift must not re-separate native inks.
        return Ok(Some(Value::Null));
    }
    s.edit("Replace Color", |doc, _| {
        let canvas = doc.bounds();
        let selected = doc.selection.clone();
        let surface = crate::commands::paint_surface(doc, id, &Value::Null)?;
        let transparent_default = format.alpha && surface.default_pixel().last().is_some_and(|a| a.is_nan() || *a <= 0.0);
        let content = surface.content_bounds();
        // A nontransparent default is visible across the canvas even without stored tiles.
        let extent = if transparent_default { content } else { content.union(&canvas) };
        let bounds = edit_bounds(extent, selected.as_ref());
        let mut default = surface.default_pixel();
        let mut default_lab = [[0.0, 0.0, 0.0, 1.0]];
        transform.convert_f32(&default, format.channels(), default_lab.as_flattened_mut(), 4, true).map_err(crate::color_cmds::cms_err)?;
        let default_selected = selected.as_ref().map_or(1.0, |mask| mask.default_pixel()[0]);
        let default_changes = replace_sample(&format, &mut default, &default_lab[0], &range, fuzz, [h, sat, l], default_selected);
        let tiles = work_tiles(surface, selected.as_ref(), bounds, default_changes)?;
        // Matching and tone changes are pointwise, so pixel work stays tile-sized.
        let mut process_tile = |tile: TileCoord| -> Result<()> {
            let rect = tile.rect().intersect(&bounds);
            if rect.is_empty() {
                return Ok(());
            }
            let mut raw = crate::allocation::read_region(surface, rect, "Replace Color")?;
            let mut lab = crate::allocation::filled(raw.len() / format.channels(), [0.0, 0.0, 0.0, 1.0], "Replace Color")?;
            transform.convert_f32(&raw, format.channels(), lab.as_flattened_mut(), 4, true).map_err(crate::color_cmds::cms_err)?;
            let width = rect.width() as usize;
            let mut selection_row = crate::allocation::filled(width, 1.0, "Replace Color")?;
            let mut changed = false;
            for (y, (row, lab_row)) in raw.chunks_exact_mut(width * format.channels()).zip(lab.chunks_exact(width)).enumerate() {
                if let Some(selected) = selected.as_ref() {
                    selected.sample_row_strided(rect.y0.saturating_add(y as i32), rect.x0, 1, 0, &mut selection_row);
                }
                for ((pixel, sample), selected) in row.chunks_exact_mut(format.channels()).zip(lab_row).zip(&selection_row) {
                    changed |= replace_sample(&format, pixel, sample, &range, fuzz, [h, sat, l], *selected);
                }
            }
            if changed {
                surface.try_write_region(rect, &raw).map_err(|e| crate::allocation::error("Replace Color", &e.to_string()))?;
            }
            Ok(())
        };
        if let Some(tiles) = tiles {
            for tile in tiles {
                process_tile(tile)?;
            }
        } else {
            for tile in bounds.tiles() {
                process_tile(tile)?;
            }
        }
        Ok(Value::Null)
    })
    .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;

    /// Work buffers follow explicit selected pixels without cropping implicitly selected coverage.
    #[test]
    fn selected_work_bounds_preserve_implicit_selection_and_clip_explicit_pixels() {
        let content = Rect::new(-100, -100, 100, 100);
        let mut mask = Surface::new(PixelFormat::GRAY8);
        assert_eq!(edit_bounds(content, Some(&mask)), Rect::EMPTY);
        mask.fill_rect(Rect::new(98, 96, 110, 105), &[0.5]);
        assert_eq!(edit_bounds(content, Some(&mask)), Rect::new(98, 96, 100, 100));
        assert_eq!(edit_bounds(content, None), content);
        for coverage in [0.5, 1.0] {
            let mut mask = Surface::with_default(PixelFormat::GRAY8, &[coverage]);
            mask.fill_rect(Rect::new(2, 2, 3, 3), &[0.0]);
            assert_eq!(edit_bounds(content, Some(&mask)), content, "stored holes do not bound selected pixels");
        }
    }

    /// The work list follows stored source or selection tiles rather than their distant bounds.
    #[test]
    fn sparse_work_tiles_follow_pixels_that_can_change() {
        let near = Rect::new(0, 0, 1, 1);
        let far = Rect::new(100_000, 100_000, 100_001, 100_001);
        let bounds = near.union(&far);
        for alpha in [false, true] {
            let format = PixelFormat { alpha, ..PixelFormat::RGBA8 };
            let mut surface = Surface::with_default(format, &[1.0; 4][..format.channels()]);
            surface.fill_rect(far, &[0.0; 4][..format.channels()]);
            assert_eq!(work_tiles(&surface, None, bounds, false).unwrap(), Some(vec![TileCoord::containing(far.x0, far.y0)]));
            let mut mask = Surface::new(PixelFormat::GRAY8);
            mask.fill_rect(near, &[1.0]);
            mask.fill_rect(far, &[1.0]);
            assert_eq!(
                work_tiles(&surface, Some(&mask), bounds, true).unwrap(),
                Some(vec![TileCoord::containing(0, 0), TileCoord::containing(far.x0, far.y0)])
            );
            assert!(work_tiles(&surface, None, near, true).unwrap().is_none(), "a small default-only area remains editable");
        }
    }

    /// A matching opaque default cannot allocate gap tiles beyond the existing adjustment budget.
    #[test]
    fn opaque_gap_budget_refuses_before_pixel_allocation_or_edit() {
        let mut s = Session::new();
        s.execute("file.new", serde_json::json!({"width":4,"height":2,"mode":"lab","depth":32,"background":"transparent"})).unwrap();
        s.edit("Distant opaque exception", |doc, active| {
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            *surface = Surface::with_default(surface.format(), &[0.5, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
            surface.fill_rect(Rect::new(100_000, 100_000, 100_001, 100_001), &[0.25, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        crate::allocation::fail_next_for_test();
        let result = s.execute(CMD, serde_json::json!({"labMinimum":[49,0,0],"labMaximum":[51,0,0],"fuzziness":0,"lightness":100}));
        // Drain the probe even on failure so another test cannot inherit it. A skipped probe
        // proves that the resource check ran before any fallible tile-buffer allocation.
        let probe_unused = crate::allocation::capacity::<u8>(0, "allocation probe").is_err();
        assert!(result.as_ref().is_err_and(|e| e.to_string().contains("too large an area")), "{result:?}");
        assert!(probe_unused);
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }

    /// Add to Sample / Subtract from Sample (#6280) are RGB samples: with Lab bounds they are
    /// refused, as `points` are, and the document is left as it was.
    #[test]
    fn lab_bounds_refuse_subtracted_samples_and_click_order() {
        for extra in [serde_json::json!({"subtractPoints": [[1, 1]]}), serde_json::json!({"order": ["+", "-"]}), serde_json::json!({"subtractPoints": []})] {
            let mut s = Session::new();
            s.execute("file.new", serde_json::json!({"width": 4, "height": 2})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let mut p = serde_json::json!({"labMinimum": [0, -10, -10], "labMaximum": [100, 10, 10], "lightness": 50});
            p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            let err = s.execute(CMD, p).unwrap_err().to_string();
            assert!(err.contains("Lab bounds cannot be combined"), "{extra}: {err}");
            assert_eq!(s.active().unwrap().doc, before, "{extra}");
        }
    }
}
