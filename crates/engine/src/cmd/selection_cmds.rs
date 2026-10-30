//! Select menu and selection tools: Magic Wand, Color Range, Modify
//! (expand/contract/border/smooth/feather), Grow, Similar, Lasso.
//! Selections are grayscale coverage surfaces clipped to the canvas.

use photocraft_algo::selection::{self as sel, SelectionMode};
use photocraft_doc::Document;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

mod modify_window;

#[cfg(test)]
mod color_range_performance;
#[cfg(test)]
mod color_range_window_tests;

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn has_selection(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.doc.selection.is_some()).map(|_| ()).ok_or_else(|| "no selection".into())
}

fn f(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}
fn b(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn mode(p: &Value) -> SelectionMode {
    SelectionMode::parse(p.get("mode").and_then(Value::as_str).unwrap_or("replace"))
}

/// A Color Range replacement is scoped to the selection that was active when it opened.
/// Explicit add/subtract/intersect operations retain their normal selection-combine semantics.
fn scope_color_range(mask: &mut [f32], doc: &Document, area: Rect, m: SelectionMode) {
    if m != SelectionMode::Replace {
        return;
    }
    if let Some(selection) = doc.selection.as_ref() {
        let previous = sel::mask_from_surface(Some(selection), area);
        for (value, bound) in mask.iter_mut().zip(previous) {
            *value = value.min(bound);
        }
    }
}

/// RGBA pixels the tools sample: the composite, or the active layer.
fn sample_pixels(s: &Session, all_layers: bool, window: Option<Rect>) -> Result<(Rect, Vec<[f32; 4]>)> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let area = window.map_or(d.doc.bounds(), |r| r.intersect(&d.doc.bounds()));
    // Match the decoders' raster budget before either the layer or composite path allocates.
    let len = area.size().area();
    let max_bytes: u64 = if cfg!(target_pointer_width = "64") { 8 << 30 } else { 2 << 30 };
    if len > max_bytes / std::mem::size_of::<[f32; 4]>() as u64 {
        return Err(EngineError::Other("the canvas is too large to sample for a selection".into()));
    }
    if area.is_empty() {
        return Ok((area, Vec::new()));
    }
    let layer = d.active_layer.and_then(|id| d.doc.layer(id)).and_then(|l| l.surface());
    match (all_layers, layer) {
        (false, Some(surf)) => {
            let len = usize::try_from(len).map_err(|_| EngineError::Other("the canvas is too large to sample for a selection".into()))?;
            let mut px = crate::allocation::filled(len, [0.0f32; 4], "sampling pixels for a selection")?;
            surf.read_rgba_into(area, &mut px);
            Ok((area, px))
        }
        _ => Ok((area, photocraft_compose::render(&d.doc, area).px)),
    }
}

fn set_selection(s: &mut Session, label: &str, area: Rect, mask: Vec<f32>, m: SelectionMode) -> Result<Value> {
    let selected = s.edit(label, |doc, _| {
        doc.selection = sel::combine(doc.selection.as_ref(), &mask, area, m);
        Ok(doc.selection.is_some())
    })?;
    Ok(json!({ "selected": selected }))
}

fn current_mask(doc: &Document) -> (Rect, Vec<f32>) {
    let area = doc.bounds();
    (area, sel::mask_from_surface(doc.selection.as_ref(), area))
}

/// Grayscale the canvas is actually showing, as opaque RGBA, when that is not the colour
/// composite: a gray layer-mask view, or — with every colour channel hidden — the first
/// visible alpha, Quick Mask, or the active layer's mask. The Magic Wand selects from this
/// so a white alpha stays one region instead of breaking up along the RGB underneath.
fn shown_gray(s: &Session) -> Option<(Rect, Vec<[u8; 4]>)> {
    let st = s.active()?;
    let doc = &st.doc;
    let area = doc.bounds();
    let v = &st.channel_view;
    let colors_off = v.visible_colors(crate::channel_cmds::color_count(doc)) == 0;
    let surf = if let Some((mask, crate::mask_view_cmds::MaskViewMode::Gray)) = v.shown_layer_mask(doc) {
        &mask.surface
    } else if colors_off {
        if let Some(i) = (0..doc.channels.len()).find(|i| v.alpha_shown(*i)) {
            &doc.channels[i].surface
        } else if let Some(q) = doc.quick_mask.as_ref().filter(|_| !v.quick_mask_hidden) {
            &q.surface
        } else if let Some((mask, _)) = v.shown_layer_mask(doc) {
            &mask.surface
        } else {
            &st.active_layer.and_then(|id| doc.layer(id))?.mask.as_ref()?.surface
        }
    } else {
        return None;
    };
    Some((area, sel::rgba8_image(surf, area)))
}

/// 8-bit RGBA of the active layer (or the composite with `all_layers`) over the canvas.
pub(crate) fn sample_rgba8(s: &Session, all_layers: bool) -> Result<(Rect, Vec<[u8; 4]>)> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let area = d.doc.bounds();
    let layer = d.active_layer.and_then(|id| d.doc.layer(id)).and_then(|l| l.surface());
    match (all_layers, layer) {
        (false, Some(surf)) => Ok((area, sel::rgba8_image(surf, area))),
        _ => {
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            Ok((area, photocraft_compose::render(&d.doc, area).px.iter().map(|p| p.map(q)).collect()))
        }
    }
}

fn magic_wand(s: &mut Session, p: &Value) -> Result<Value> {
    let (x, y) = (f(p, "x", 0.0).floor() as i32, f(p, "y", 0.0).floor() as i32);
    let (area, img) = match shown_gray(s) {
        Some(plane) => plane,
        None => sample_rgba8(s, b(p, "sampleAllLayers", false))?,
    };
    let region = sel::wand_region(&img, area, (x, y), f(p, "tolerance", 32.0), b(p, "contiguous", true), b(p, "antiAlias", true));
    drop(img);
    let m = mode(p);
    let selected = s.edit("Magic Wand", |doc, _| {
        doc.selection = sel::combine_region(doc.selection.as_ref(), region.as_ref(), m);
        Ok(doc.selection.is_some())
    })?;
    Ok(json!({ "selected": selected }))
}

fn parse_color(p: &Value) -> [f32; 3] {
    match p.get("color") {
        Some(Value::Array(a)) if a.len() >= 3 => [0, 1, 2].map(|i| a[i].as_f64().unwrap_or(0.0) as f32),
        Some(Value::String(h)) => {
            let h = h.trim_start_matches('#');
            let c = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map_or(0.0, |v| f32::from(v) / 255.0);
            [c(0), c(2), c(4)]
        }
        _ => [0.0; 3],
    }
}

/// Composite RGB (the compositor's output profile for the document) → CIE Lab D50.
fn lab_transform(s: &Session) -> Result<std::sync::Arc<photocraft_cms::Transform>> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let src = crate::color_cmds::composite_profile(&d.doc);
    photocraft_cms::cached(&src, photocraft_cms::Builtin::LabD50.profile(), photocraft_cms::TransformOptions::default())
        .map_err(|e| EngineError::Other(format!("colour management: {e}")))
}

/// The Lab profile's normalised encoding (ICC v4) → L 0–100, a and b −128–127.
fn decode_lab(v: [f32; 3]) -> [f32; 3] {
    [v[0] * 100.0, v[1] * 255.0 - 128.0, v[2] * 255.0 - 128.0]
}

fn colour_to_lab(t: &photocraft_cms::Transform, rgb: [f32; 3]) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    t.eval(&rgb.map(|v| v.clamp(0.0, 1.0)), &mut out);
    decode_lab(out)
}

/// RGBA pixels → Lab + alpha (L, a, b, alpha).
fn pixels_to_lab(t: &photocraft_cms::Transform, px: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let src: Vec<[f32; 4]> = px.iter().map(|p| [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0), p[2].clamp(0.0, 1.0), p[3]]).collect();
    let mut out = vec![[0.0f32; 4]; px.len()];
    t.convert_f32(src.as_flattened(), 4, out.as_flattened_mut(), 4, true);
    for p in &mut out {
        let [l, a, b] = decode_lab([p[0], p[1], p[2]]);
        *p = [l, a, b, p[3]];
    }
    out
}

/// Prepared Sampled Colors query shared by the command and its interactive preview.
/// Samples always come from the original document; reducing an image must not change them.
pub struct ColorRangeSamples {
    to_lab: std::sync::Arc<photocraft_cms::Transform>,
    range: sel::LabRange,
    grid: bool,
    fuzziness: f32,
    localized: Option<f32>,
    plus_at: Vec<(f32, f32)>,
}

impl ColorRangeSamples {
    /// Prepare the query without allocating a full-resolution composite. Point samples use
    /// exactly the same compositor/profile as the final selection.
    pub fn new(s: &Session, p: &Value) -> Result<Self> {
        let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
        let area = doc.bounds();
        let bad = |msg: String| EngineError::BadParams { cmd: "select.colorRange".into(), msg };
        let sample = |key: &str| -> Result<Vec<sel::RangeSample>> {
            let Some(pts) = p.get(key) else { return Ok(Vec::new()) };
            let pts = pts.as_array().ok_or_else(|| bad(format!("`{key}` must be [[x, y], …]")))?;
            let mut samples = Vec::new();
            for point in pts {
                let q = point.as_array().and_then(|v| match v.as_slice() {
                    [x, y] => Some((x.as_f64()?, y.as_f64()?)),
                    _ => None,
                });
                let Some((x, y)) = q.filter(|(x, y)| x.is_finite() && y.is_finite()) else {
                    return Err(bad(format!("bad point {point} in `{key}` (want [x, y])")));
                };
                let (x, y) = (x.floor() as i32, y.floor() as i32);
                if !area.contains(x, y) {
                    return Err(bad(format!("point [{x}, {y}] in `{key}` is outside the canvas")));
                }
                let r = Rect::from_xywh(x, y, 1, 1);
                let pixel = if !b(p, "sampleAllLayers", true) {
                    s.active().and_then(|d| d.active_layer.and_then(|id| d.doc.layer(id))).and_then(|l| l.surface()).map(|surf| {
                        let mut px = [[0.0; 4]];
                        surf.read_rgba_into(r, &mut px);
                        px[0]
                    })
                } else {
                    None
                };
                let px = match pixel {
                    Some(px) => px,
                    None => photocraft_compose::render(doc, r).px.first().copied().unwrap_or([0.0; 4]),
                };
                samples.push(sel::RangeSample { color: [px[0], px[1], px[2]], at: Some(((x - area.x0) as f32, (y - area.y0) as f32)) });
            }
            Ok(samples)
        };
        Self::with_samples(s, p, area, &sample("points")?, &sample("subtractPoints")?)
    }

    fn with_samples(s: &Session, p: &Value, area: Rect, plus: &[sel::RangeSample], minus: &[sel::RangeSample]) -> Result<Self> {
        let bad = |msg: String| EngineError::BadParams { cmd: "select.colorRange".into(), msg };
        let fuzz = |d: f32| f(p, "fuzziness", d).max(0.0);
        // The order the eyedropper clicks came in ("+" a point, "-" a subtracted point):
        // Photoshop applies them one after another. Without it, all additions come first.
        let order: Vec<bool> = match p.get("order") {
            None => std::iter::repeat_n(true, plus.len()).chain(std::iter::repeat_n(false, minus.len())).collect(),
            Some(o) => {
                let o = o.as_array().ok_or_else(|| bad("`order` must be a list of \"+\" and \"-\"".into()))?;
                let order = o
                    .iter()
                    .map(|v| match v.as_str() {
                        Some("+") => Ok(true),
                        Some("-") => Ok(false),
                        _ => Err(bad(format!("bad entry {v} in `order` (want \"+\" or \"-\")"))),
                    })
                    .collect::<Result<Vec<bool>>>()?;
                if order.iter().filter(|x| **x).count() != plus.len() || order.iter().filter(|x| !**x).count() != minus.len() {
                    return Err(bad("`order` must list each of `points` (\"+\") and `subtractPoints` (\"-\") once".into()));
                }
                order
            }
        };
        let mut fixed: Vec<[f32; 3]> = Vec::new();
        if let Some(cs) = p.get("colors") {
            let cs = cs.as_array().ok_or_else(|| bad("`colors` must be a list of colours".into()))?;
            fixed.extend(cs.iter().map(|c| parse_color(&json!({ "color": c }))));
        }
        if p.get("color").is_some() {
            fixed.push(parse_color(p));
        }
        // Localized Color Clusters: Range is a percentage of the canvas's longer side.
        let plus_at: Vec<(f32, f32)> = plus.iter().filter_map(|x| x.at).collect();
        let localized = if b(p, "localized", false) {
            if plus_at.is_empty() {
                return Err(bad("localized clusters need eyedropper `points`".into()));
            }
            Some(f(p, "range", 100.0).clamp(0.0, 100.0) / 100.0 * area.width().max(area.height()) as f32)
        } else {
            None
        };
        // Photoshop compares colours in Lab, through the document's profile
        // (sel::lab_range_coverage). The samples span one Lab box; "Add to Sample" grows it,
        // "Subtract from Sample" reshapes it (sel::LabRange::subtract).
        let to_lab = lab_transform(s)?;
        // 8-bit documents are compared on the 8-bit Lab grid, samples and pixels alike.
        let grid = s.active().is_some_and(|d| d.doc.depth == photocraft_doc::SampleType::U8);
        let q = |c: [f32; 3]| if grid { sel::quantize_lab8(c) } else { c };
        let colour_to_lab = |t: &photocraft_cms::Transform, c: [f32; 3]| q(colour_to_lab(t, c));
        let foreground = || {
            let [r, g, b, _] = s.tools.foreground;
            sel::LabRange::point(colour_to_lab(&to_lab, [r, g, b]))
        };
        let fixed_lab: Vec<[f32; 3]> = fixed.iter().map(|c| colour_to_lab(&to_lab, *c)).collect();
        let mut range = sel::LabRange::spanning(&fixed_lab);
        let (mut pi, mut mi) = (plus.iter(), minus.iter());
        for add in order {
            if add {
                if let Some(x) = pi.next() {
                    let c = colour_to_lab(&to_lab, x.color);
                    range = Some(range.map_or(sel::LabRange::point(c), |r| r.grow(&c)));
                }
            } else if let Some(x) = mi.next() {
                range = Some(range.unwrap_or_else(foreground).subtract(&colour_to_lab(&to_lab, x.color), fuzz(40.0)));
            }
        }
        let range = range.unwrap_or_else(foreground);
        Ok(Self { to_lab, range, grid, fuzziness: fuzz(40.0), localized, plus_at })
    }

    /// Coverage of a composite rectangle. `scale` is the original document pixels represented
    /// by one input pixel; `origin` is in original document pixels (for a visible canvas crop).
    pub fn coverage(&self, px: &[[f32; 4]], width: usize, scale: f32, origin: [f32; 2]) -> Vec<f32> {
        let mut lab = pixels_to_lab(&self.to_lab, px);
        if self.grid {
            for p in &mut lab {
                let [l, a, b] = sel::quantize_lab8([p[0], p[1], p[2]]);
                *p = [l, a, b, p[3]];
            }
        }
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let points: Vec<_> = self.plus_at.iter().map(|(x, y)| ((x - origin[0]) / scale, (y - origin[1]) / scale)).collect();
        sel::color_range_lab(&lab, width, &self.range, self.fuzziness, self.grid, self.localized.map(|r| (r / scale, points.as_slice())))
    }
}

/// Replace refines the existing selection; no output outside its nonzero bounds survives.
fn color_range_area(doc: &Document, m: SelectionMode) -> Rect {
    let canvas = doc.bounds();
    let Some(selection) = doc.selection.as_ref().filter(|_| m == SelectionMode::Replace) else { return canvas };
    // content_bounds describes differences from the default, not implicit selected pixels.
    if selection.default_pixel().first().is_some_and(|v| *v != 0.0) {
        return canvas;
    }
    // content_bounds orders raw tile coordinates by their differences. Reject coordinates
    // outside the pixel-addressable range before those differences can overflow.
    let valid = (i32::MIN / photocraft_geom::TILE_SIZE)..=(i32::MAX / photocraft_geom::TILE_SIZE);
    if selection.tiles().any(|(tile, _)| !valid.contains(&tile.tx) || !valid.contains(&tile.ty)) {
        return canvas;
    }
    selection.content_bounds().intersect(&canvas)
}

fn color_range(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "select.colorRange";
    let bad = |msg: String| EngineError::BadParams { cmd: CMD.into(), msg };
    let preset = p.get("select").map_or(Some("sampledColors"), Value::as_str).ok_or_else(|| bad("`select` must be a string".into()))?;
    let invert = b(p, "invert", false);
    let selection_mode = mode(p);
    let fuzz = |d: f32| f(p, "fuzziness", d).max(0.0);
    // Photoshop's Detect Faces needs a face detector, which there is none of yet (#4455).
    if b(p, "detectFaces", false) {
        return Err(bad("detectFaces: face detection isn't available yet; select without it".into()));
    }
    let area = color_range_area(&s.active().ok_or(EngineError::NoDocument)?.doc, selection_mode);
    let (area, mut mask) = match preset {
        "sampledColors" => {
            let (area, px) = sample_pixels(s, b(p, "sampleAllLayers", true), Some(area))?;
            let w = area.width() as usize;
            // The dialog's eyedroppers: colours picked on the image, at their positions.
            // Like Photoshop's, they may sample anywhere on the canvas: an active selection only
            // limits the result (scope_color_range), not where colours are picked (#5244).
            let query = ColorRangeSamples::new(s, p)?;
            let mask = query.coverage(&px, w, 1.0, [area.x0 as f32, area.y0 as f32]);
            (area, mask)
        }
        "reds" | "yellows" | "greens" | "cyans" | "blues" | "magentas" => {
            let center = match preset {
                "reds" => 0.0,
                "yellows" => 60.0,
                "greens" => 120.0,
                "cyans" => 180.0,
                "blues" => 240.0,
                _ => 300.0,
            };
            let (area, px) = sample_pixels(s, b(p, "sampleAllLayers", true), Some(area))?;
            (area, sel::hue_range(&px, center))
        }
        "skinTones" => {
            let (area, px) = sample_pixels(s, b(p, "sampleAllLayers", true), Some(area))?;
            (area, sel::skin_tones(&px, fuzz(40.0).min(200.0)))
        }
        "highlights" | "midtones" | "shadows" => {
            // Range in levels (Highlights / Shadows: one value, Midtones: [low, high]); Fuzziness
            // is a percentage of the tonal scale.
            let level = |v: &Value| v.as_f64().filter(|x| (0.0..=255.0).contains(x)).map(|x| x as f32);
            let r = p.get("tonalRange");
            let (lo, hi) = match (preset, r) {
                ("shadows", None) => (0.0, 65.0),
                ("highlights", None) => (190.0, 255.0),
                (_, None) => (105.0, 150.0),
                ("shadows", Some(v)) => (0.0, level(v).ok_or_else(|| bad(format!("tonalRange: want a level 0..255 (got {v})")))?),
                ("highlights", Some(v)) => (level(v).ok_or_else(|| bad(format!("tonalRange: want a level 0..255 (got {v})")))?, 255.0),
                (_, Some(v)) => v
                    .as_array()
                    .and_then(|a| match a.as_slice() {
                        [l, h] => Some((level(l)?, level(h)?)),
                        _ => None,
                    })
                    .filter(|(l, h)| l <= h)
                    .ok_or_else(|| bad(format!("tonalRange: want [low, high] levels 0..255 (got {v})")))?,
            };
            let (area, px) = sample_pixels(s, b(p, "sampleAllLayers", true), Some(area))?;
            (area, sel::tone_range(&px, lo, hi, fuzz(20.0).min(100.0) / 100.0 * 255.0))
        }
        "outOfGamut" => {
            // Pixels the current proof setup (View › Proof Setup) can't reproduce, as the gamut
            // warning shows them: judged on the composite.
            let d = s.active().ok_or(EngineError::NoDocument)?;
            let pv = s.color.proof(d.doc.id);
            let (m, _) = crate::color_cmds::gamut_mask(&d.doc, &pv.setup, pv.gamut_threshold)?;
            (d.doc.bounds(), m.iter().map(|v| f32::from(*v) / 255.0).collect())
        }
        other => {
            return Err(bad(format!(
                "unknown select `{other}` (sampledColors, reds, yellows, greens, cyans, blues, magentas, highlights, midtones, shadows, skinTones, outOfGamut)"
            )));
        }
    };
    if invert {
        for v in &mut mask {
            *v = 1.0 - *v;
        }
    }
    // Invert the colour result first, then constrain it to the original selection.
    // Add mode must still be able to extend an existing selection intentionally.
    scope_color_range(&mut mask, &s.active().ok_or(EngineError::NoDocument)?.doc, area, selection_mode);
    set_selection(s, "Color Range", area, mask, selection_mode)
}

fn modify(s: &mut Session, p: &Value, op: &str) -> Result<Value> {
    // The dialogs' ranges (Photoshop's limits); anything else is refused, never clamped silently.
    let (min, max): (f32, f32) = match op {
        "border" => (1.0, 200.0),
        "feather" => (0.1, 1000.0),
        _ => (1.0, 500.0),
    };
    let r = f(p, "radius", 1.0);
    // Below the dialog's minimum (a negative or zero radius, #993) or above its maximum is
    // refused: a clamp to 0 left the selection alone (Expand, Contract) or cleared it (Border)
    // while reporting success.
    if !(min..=max).contains(&r) {
        return Err(EngineError::BadParams { cmd: format!("select.modify.{op}"), msg: format!("radius must be a number in {min}..{max}") });
    }
    // Photoshop's "Apply effect at canvas bounds": when on, the canvas edge is a selection edge
    // (Select All then Contract shrinks from the edges); when off, the selection is taken to
    // continue past the canvas. Border has no such option: its band always follows the canvas edge,
    // or a Select All would border to nothing.
    let at_bounds = op == "border" || b(p, "applyAtCanvasBounds", false);
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let canvas = doc.bounds();
    if canvas.size().area() > MAX_MODIFY_PIXELS as u64 {
        return Err(EngineError::BadParams { cmd: format!("select.modify.{op}"), msg: "the selection is too large to modify".into() });
    }
    // Feather reaches ⌈3σ⌉ (σ = r, as in Photoshop); the others reach at most ceil(r).
    let reach = if op == "feather" { sel::feather_reach(r) } else { r.ceil() as usize };
    // Keep both mask conversion and tiled replacement within the affected region. The
    // zero halo makes internal window edges inert; nonzero edges can only be canvas edges,
    // so the existing Apply effect at canvas bounds padding still has the same meaning.
    let area = modify_window::area(doc, reach as f32);
    let m = sel::mask_from_surface(doc.selection.as_ref(), area);
    let (w, h) = (area.width() as usize, area.height() as usize);
    // A margin past the reach keeps the padded edge out of reach. A selection clear of the
    // canvas edge needs none: both readings agree there.
    let pad = if touches_edge(&m, w, h) { reach.saturating_add(2) } else { 0 };
    let grown = |n: usize| pad.checked_mul(2).and_then(|p| n.checked_add(p));
    let (Some(pw), Some(ph)) = (grown(w), grown(h)) else {
        return Err(EngineError::BadParams { cmd: format!("select.modify.{op}"), msg: "the selection is too large to modify".into() });
    };
    if pw.checked_mul(ph).is_none_or(|n| n > MAX_MODIFY_PIXELS) {
        return Err(EngineError::BadParams { cmd: format!("select.modify.{op}"), msg: "the selection is too large to modify".into() });
    }
    // Unpadded (the usual case) the mask is used as it is: two serial full-canvas copies were
    // most of a 24 MP Feather or Smooth (#211).
    let padded = if pad == 0 { m } else { pad_mask(&m, w, h, pad, at_bounds) };
    let out = match op {
        "expand" => sel::expand(&padded, pw, ph, r),
        "contract" => sel::contract(&padded, pw, ph, r),
        "border" => sel::border(&padded, pw, ph, r),
        "smooth" => sel::smooth(&padded, pw, ph, r),
        _ => sel::feather(&padded, pw, ph, r),
    };
    let out = if pad == 0 { out } else { crop_mask(&out, pw, pad, w, h) };
    let label = match op {
        "expand" => "Expand Selection",
        "contract" => "Contract Selection",
        "border" => "Border Selection",
        "smooth" => "Smooth Selection",
        _ => "Feather Selection",
    };
    set_selection(s, label, area, out, SelectionMode::Replace)
}

/// The largest padded mask Modify works on (a 300000 × 300000 canvas is 9e10 pixels; this caps a
/// padded copy at ~4 GB of f32, beyond which the edit is refused rather than aborting on allocation).
const MAX_MODIFY_PIXELS: usize = 1 << 30;

/// Whether any pixel on the outermost rows or columns of `m` (`w`×`h`) is selected.
fn touches_edge(m: &[f32], w: usize, h: usize) -> bool {
    let sel = |x: usize, y: usize| m.get(y * w + x).is_some_and(|v| *v > 0.0);
    (0..w).any(|x| sel(x, 0) || sel(x, h.saturating_sub(1))) || (0..h).any(|y| sel(0, y) || sel(w.saturating_sub(1), y))
}

/// `m` (`w`×`h`) with a `pad`-pixel margin: unselected when `outside_empty`, else the nearest
/// edge pixel repeated (the selection continues past the canvas).
fn pad_mask(m: &[f32], w: usize, h: usize, pad: usize, outside_empty: bool) -> Vec<f32> {
    let pw = w + 2 * pad;
    let mut out = vec![0.0f32; pw * (h + 2 * pad)];
    if w == 0 || h == 0 {
        return out;
    }
    // Row by row (a copy and two fills), not sample by sample (#211).
    for (py, row) in out.chunks_exact_mut(pw).enumerate() {
        if outside_empty && !(pad..pad + h).contains(&py) {
            continue;
        }
        let y = py.saturating_sub(pad).min(h - 1);
        let (Some(src), Some((left, rest))) = (m.get(y * w..(y + 1) * w), row.split_at_mut_checked(pad)) else { continue };
        let Some((mid, right)) = rest.split_at_mut_checked(w) else { continue };
        mid.copy_from_slice(src);
        if !outside_empty {
            left.fill(src.first().copied().unwrap_or(0.0));
            right.fill(src.last().copied().unwrap_or(0.0));
        }
    }
    out
}

/// The `w`×`h` centre of a mask padded by `pad_mask`.
fn crop_mask(m: &[f32], pw: usize, pad: usize, w: usize, h: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(w.saturating_mul(h));
    for row in m.chunks_exact(pw.max(1)).skip(pad).take(h) {
        out.extend_from_slice(row.get(pad..pad + w).unwrap_or_default());
    }
    out
}

/// Grow (contiguous) / Similar (anywhere): pixels whose colour lies within
/// the selected pixels' colour range widened by the tolerance.
fn grow_similar(s: &mut Session, p: &Value, contiguous: bool) -> Result<Value> {
    let tol = f(p, "tolerance", 32.0) / 255.0;
    let (area, px) = sample_pixels(s, b(p, "sampleAllLayers", false), None)?;
    let (_, m) = current_mask(&s.active().ok_or(EngineError::NoDocument)?.doc);
    let (mut lo, mut hi) = ([f32::MAX; 4], [f32::MIN; 4]);
    for (q, v) in px.iter().zip(&m) {
        if *v >= 0.5 {
            for c in 0..4 {
                lo[c] = lo[c].min(q[c]);
                hi[c] = hi[c].max(q[c]);
            }
        }
    }
    if lo[0] > hi[0] {
        return Err(EngineError::Other("selection is empty".into()));
    }
    let inside = |q: &[f32; 4]| (0..4).all(|c| q[c] >= lo[c] - tol - 1e-6 && q[c] <= hi[c] + tol + 1e-6);
    let (w, h) = (area.width() as usize, area.height() as usize);
    let mut out: Vec<f32> = m.iter().map(|v| if *v >= 0.5 { 1.0 } else { 0.0 }).collect();
    if contiguous {
        let mut stack: Vec<usize> = (0..out.len()).filter(|i| out[*i] > 0.0).collect();
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let n = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)];
            for j in n.into_iter().flatten() {
                if out[j] == 0.0 && inside(&px[j]) {
                    out[j] = 1.0;
                    stack.push(j);
                }
            }
        }
    } else {
        for (o, q) in out.iter_mut().zip(&px) {
            if inside(q) {
                *o = 1.0;
            }
        }
    }
    set_selection(s, if contiguous { "Grow" } else { "Similar" }, area, out, SelectionMode::Replace)
}

fn lasso(s: &mut Session, p: &Value) -> Result<Value> {
    let pts: Vec<(f32, f32)> = p
        .get("points")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|q| Some((q.get(0)?.as_f64()? as f32, q.get(1)?.as_f64()? as f32))).collect())
        .unwrap_or_default();
    if pts.len() < 3 {
        return Err(EngineError::BadParams { cmd: "select.lasso".into(), msg: "needs at least 3 points".into() });
    }
    polygon_selection(s, "select.lasso", "Lasso", &pts, p)
}

/// Selects the polygon `pts` with the lasso tools' `mode`, `antiAlias` and `feather` params, as
/// one history step called `label`.
pub(crate) fn polygon_selection(s: &mut Session, cmd: &str, label: &str, pts: &[(f32, f32)], p: &Value) -> Result<Value> {
    let feather = f(p, "feather", 0.0);
    // Select › Modify › Feather's range.
    if !(0.0..=1000.0).contains(&feather) {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: "feather must be a number in 0..1000".into() });
    }
    let area = s.active().ok_or(EngineError::NoDocument)?.doc.bounds();
    if feather > 0.0 {
        let (mut lo, mut hi) = ((f32::INFINITY, f32::INFINITY), (f32::NEG_INFINITY, f32::NEG_INFINITY));
        for &(x, y) in pts {
            if x.is_finite() && y.is_finite() {
                lo = (lo.0.min(x), lo.1.min(y));
                hi = (hi.0.max(x), hi.1.max(y));
            }
        }
        let bounds = if lo.0 <= hi.0 && lo.1 <= hi.1 {
            Rect::new(lo.0.floor() as i32, lo.1.floor() as i32, hi.0.ceil() as i32, hi.1.ceil() as i32)
        } else {
            Rect::EMPTY
        };
        let window = feather_window(bounds, area, feather);
        let mask = sel::polygon_window(pts, area, window, b(p, "antiAlias", true));
        let mask = sel::feather(&mask, window.width() as usize, window.height() as usize, feather);
        let selected = s.edit(label, |doc, _| {
            doc.selection = combine_band(doc.selection.as_ref(), &mask, window, mode(p));
            Ok(doc.selection.is_some())
        })?;
        return Ok(json!({ "selected": selected }));
    }
    // Only the rows the polygon spans (#4532): rows outside it are empty, and full-width rows
    // keep `polygon`'s coverage bit-identical (its x arithmetic is relative to the area's left
    // edge; its y samples are whole rows plus the same sub-row fractions).
    let band = polygon_rows(pts, area);
    let mask = sel::polygon(pts, band, b(p, "antiAlias", true));
    let m = mode(p);
    let selected = s.edit(label, |doc, _| {
        doc.selection = combine_band(doc.selection.as_ref(), &mask, band, m);
        Ok(doc.selection.is_some())
    })?;
    Ok(json!({ "selected": selected }))
}

/// The full-width rows of `area` that points `pts` span (empty when none are finite).
fn polygon_rows(pts: &[(f32, f32)], area: Rect) -> Rect {
    let ys = pts.iter().map(|p| p.1).filter(|y| y.is_finite());
    let (lo, hi) = ys.fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), y| (lo.min(y), hi.max(y)));
    if lo > hi {
        return Rect::EMPTY;
    }
    let clamp = |v: f32| v.clamp(area.y0 as f32, area.y1 as f32) as i32;
    let (y0, y1) = (clamp(lo.floor()), clamp(hi.ceil() + 1.0));
    if y0 >= y1 { Rect::EMPTY } else { Rect::new(area.x0, y0, area.x1, y1) }
}

/// [`sel::combine`] of a mask covering only `band` (zero elsewhere), giving exactly what the
/// whole-canvas combine gives: Replace and Intersect are empty outside the band, and Add and
/// Subtract keep the old selection there.
pub(crate) fn combine_band(old: Option<&Surface>, mask: &[f32], band: Rect, m: SelectionMode) -> Option<Surface> {
    sel::combine_band(old, mask, band, m)
}

/// Keep dense feather masks local to the drawn shape. Clip before inflating: pixels outside
/// the canvas never contribute to the original full-canvas blur, even near a canvas edge.
pub(crate) fn feather_window(bounds: Rect, canvas: Rect, feather: f32) -> Rect {
    let bounds = bounds.intersect(&canvas);
    if bounds.is_empty() {
        return Rect::EMPTY;
    }
    let reach = i32::try_from(sel::feather_reach(feather)).unwrap_or(i32::MAX);
    bounds.inflate(reach.saturating_add(1)).intersect(&canvas)
}

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: None, params: $params, enabled: $en, run: $run, journal: true }
    };
}

/// Selection command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "select.magicWand",
            "Magic Wand",
            [],
            r##"{"x":px,"y":px,"tolerance":0..255=32,"contiguous":bool=true,"antiAlias":bool=true,"sampleAllLayers":bool=false,"mode":"replace|add|subtract|intersect"="replace"} (samples the visible alpha, Quick Mask, or layer mask when that is the picture)"##,
            has_doc,
            magic_wand
        ),
        spec!(
            "select.colorRange",
            "Color Range…",
            ["Select"],
            r##"{"select":"sampledColors|reds|yellows|greens|cyans|blues|magentas|highlights|midtones|shadows|skinTones|outOfGamut"="sampledColors","color":"#rrggbb"=foreground,"colors":["#rrggbb",…]?,"points":[[x,y],…]? (eyedropper samples),"subtractPoints":[[x,y],…]? (minus eyedropper),"order":["+"|"-",…]? (click order of points and subtractPoints; default all points first),"fuzziness":0..200=40 (tones: 0..100 %=20),"detectFaces":bool=false (not available yet: refused),"localized":bool=false,"range":0..100=100 (% of the longer side),"tonalRange":level|[lo,hi] (shadows 65, highlights 190, midtones [105,150]),"invert":bool=false,"sampleAllLayers":bool=true,"mode":"replace|add|subtract|intersect"="replace"}"##,
            has_doc,
            color_range
        ),
        spec!("select.modify.border", "Border…", ["Select", "Modify"], r##"{"radius":1..200=1}"##, has_selection, |s, p| modify(s, p, "border")),
        spec!("select.modify.smooth", "Smooth…", ["Select", "Modify"], r##"{"radius":1..500=1,"applyAtCanvasBounds":bool=false}"##, has_selection, |s, p| {
            modify(s, p, "smooth")
        }),
        spec!("select.modify.expand", "Expand…", ["Select", "Modify"], r##"{"radius":1..500=1,"applyAtCanvasBounds":bool=false}"##, has_selection, |s, p| {
            modify(s, p, "expand")
        }),
        spec!(
            "select.modify.contract",
            "Contract…",
            ["Select", "Modify"],
            r##"{"radius":1..500=1,"applyAtCanvasBounds":bool=false}"##,
            has_selection,
            |s, p| modify(s, p, "contract")
        ),
        spec!(
            "select.modify.feather",
            "Feather…",
            ["Select", "Modify"],
            r##"{"radius":0.1..1000=1,"applyAtCanvasBounds":bool=false}"##,
            has_selection,
            |s, p| modify(s, p, "feather")
        ),
        spec!("select.grow", "Grow", ["Select"], r##"{"tolerance":0..255=32,"sampleAllLayers":bool=false}"##, has_selection, |s, p| grow_similar(s, p, true)),
        spec!("select.similar", "Similar", ["Select"], r##"{"tolerance":0..255=32,"sampleAllLayers":bool=false}"##, has_selection, |s, p| grow_similar(
            s, p, false
        )),
        spec!(
            "select.lasso",
            "Lasso",
            [],
            r##"{"points":[[x,y],…],"mode":"replace|add|subtract|intersect"="replace","antiAlias":bool=true,"feather":px=0}"##,
            has_doc,
            lasso
        ),
    ]
}

#[cfg(test)]
mod tests;
