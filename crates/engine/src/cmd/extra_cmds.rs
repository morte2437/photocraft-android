//! Everyday Photoshop commands that build on the core ones: Edit › Stroke, the fixed Transform
//! presets (Rotate 180°/90°, Flip), Paste Into, Reselect, Equalize, Reveal All, Layer from
//! Background, Copy/Paste Layer Style, Hide All Effects, layer-mask toggles, Rasterize variants,
//! Delete Hidden / Empty Layers, Ungroup, Hide/Show Layers, Show Only This Layer (⌥-click an eye),
//! Average and Clouds.

use photocraft_algo::selection as sel;
use photocraft_doc::{Document, Layer, LayerContent, LayerId, LayerMask};
use photocraft_geom::Rect;
use photocraft_raster::{from_rgba_into, to_rgba};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, color_param, layer_param};
use crate::{EngineError, Result, Session};

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn active_layer(s: &Session) -> std::result::Result<&Layer, String> {
    let d = s.active().ok_or("no document open")?;
    d.active_layer.and_then(|id| d.doc.layer(id)).ok_or_else(|| "no active layer".into())
}

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    active_layer(s).map(|_| ())
}

fn has_pixels(s: &Session) -> std::result::Result<(), String> {
    match active_layer(s)?.content {
        LayerContent::Raster(_) => Ok(()),
        ref c => Err(format!("active layer is {} {} layer, not a pixel layer", c.article(), c.kind_name())),
    }
}

fn has_mask(s: &Session) -> std::result::Result<(), String> {
    active_layer(s)?.mask.as_ref().map(|_| ()).ok_or_else(|| "the active layer has no layer mask".into())
}

fn has_clip_and_selection(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    s.clipboard.as_ref().ok_or("the clipboard is empty")?;
    d.doc.selection.as_ref().map(|_| ()).ok_or_else(|| "Paste Into needs a selection".into())
}

fn has_background(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    d.doc.background().map(|_| ()).ok_or_else(|| "the document has no Background layer".into())
}

/// The layers a layer-style command acts on: as [`crate::layer_multi_cmds::targets`], but with
/// several layers selected only those `fits` (the Background carries no style, Clear needs one).
pub(crate) fn style_targets(s: &Session, p: &Value, fits: impl Fn(&Layer) -> bool) -> Result<Vec<LayerId>> {
    let ids = crate::layer_multi_cmds::targets(s, p)?;
    if ids.len() < 2 {
        return Ok(ids);
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let ids: Vec<LayerId> = ids.into_iter().filter(|id| d.doc.layer(*id).is_some_and(|l| !d.doc.is_background(l.id) && fits(l))).collect();
    if ids.is_empty() {
        return Err(EngineError::Other("none of the selected layers can take this layer style change".into()));
    }
    Ok(ids)
}

/// A key that makes the sub-commands of one command share a single history step.
pub(crate) fn step_key(s: &Session, what: &str) -> String {
    format!("{what}#{}", s.active().map_or(0, |d| d.revision))
}

// ---------- pixel helpers ----------

/// Rewrite the active pixel layer's RGBA over the selection bounds (or the canvas). `f` gets the
/// area, the pixels, and the selection coverage per pixel; results are blended back by coverage,
/// and transparency stays locked when the layer locks it. When `p` targets an alpha channel, the
/// Quick Mask or the layer mask (`Session::execute` fills that in from the Channels panel), that
/// grayscale surface is rewritten instead, as the core filters do (#935).
fn map_pixels(s: &mut Session, label: &str, p: &Value, f: impl FnOnce(Rect, &mut [[f32; 4]], &[f32])) -> Result<Value> {
    let id = layer_param(s, &Value::Null)?;
    let channel = crate::channel_cmds::target_of(p) != crate::channel_cmds::Target::Pixels;
    s.edit(label, |doc, _| {
        let canvas = doc.bounds();
        let selection = doc.selection.clone();
        let area = selection.as_ref().map_or(canvas, |m| m.content_bounds().intersect(&canvas));
        let (surf, lock_alpha) = if channel {
            let Some(surf) = crate::channel_cmds::channel_surface_for_filter(doc, Some(id), p)? else { return Ok(()) };
            (surf, false)
        } else {
            let locks = doc.effective_locks(id);
            let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            if locks.pixels || locks.all {
                return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
            }
            (l.surface_mut().ok_or_else(|| EngineError::Other("not a pixel layer".into()))?, locks.transparency)
        };
        if area.is_empty() {
            return Ok(());
        }
        let fmt = surf.format();
        let n = fmt.channels();
        let mut raw = surf.read_region(area);
        let orig: Vec<[f32; 4]> = raw.chunks_exact(n).map(|p| to_rgba(&fmt, p)).collect();
        let w = area.width() as usize;
        let cover: Vec<f32> = match &selection {
            Some(m) => (0..orig.len()).map(|i| m.sample_channel(area.x0 + (i % w) as i32, area.y0 + (i / w) as i32, 0)).collect(),
            None => vec![1.0; orig.len()],
        };
        let mut px = orig.clone();
        f(area, &mut px, &cover);
        let mut enc = [0.0f32; 8];
        for (i, out) in raw.chunks_exact_mut(n).enumerate() {
            let k = cover[i];
            if k <= 0.0 {
                continue;
            }
            let (o, p) = (orig[i], px[i]);
            let mut v = [0.0; 4];
            for c in 0..4 {
                v[c] = o[c] + (p[c] - o[c]) * k;
            }
            if lock_alpha {
                v[3] = o[3];
            }
            from_rgba_into(&fmt, v, &mut enc);
            out.copy_from_slice(&enc[..n]);
        }
        surf.write_region(area, &raw);
        surf.prune();
        Ok(())
    })?;
    Ok(Value::Null)
}

/// The effective whole-pixel Stroke width, shared by execution and ATN export.
pub(crate) fn stroke_width(p: &Value) -> usize {
    // Whole pixels, rounded up, as in Photoshop (0.5 px strokes 1 px).
    p.get("width").and_then(Value::as_f64).filter(|w| w.is_finite()).unwrap_or(1.0).clamp(1.0, 250.0).ceil() as usize
}

/// Edit › Stroke: a band along the selection edge (or the layer's opaque edge without a selection).
fn stroke(s: &mut Session, p: &Value) -> Result<Value> {
    let width = stroke_width(p);
    let color = color_param("edit.stroke", p, "color", s.tools.foreground)?;
    // A colour picked as CMYK strokes a CMYK document with the picked inks (#4196).
    let picked = s.tools.picked_cmyk([color[0], color[1], color[2]]);
    let mode = match p.get("mode") {
        None | Some(Value::Null) => photocraft_color::BlendMode::Normal,
        Some(Value::String(m)) => crate::commands::blend_from_str(m)
            .filter(|m| *m != photocraft_color::BlendMode::PassThrough)
            .ok_or_else(|| EngineError::BadParams { cmd: "edit.stroke".into(), msg: format!("unknown blend mode `{m}`") })?,
        Some(v) => return Err(EngineError::BadParams { cmd: "edit.stroke".into(), msg: format!("mode must be a blend mode name, not {v}") }),
    };
    let opacity = match p.get("opacity") {
        None | Some(Value::Null) => 1.0,
        Some(v) => {
            (v.as_f64()
                .filter(|x| x.is_finite())
                .ok_or_else(|| EngineError::BadParams { cmd: "edit.stroke".into(), msg: "opacity must be a finite number from 0 to 100".into() })?
                .clamp(0.0, 100.0)
                / 100.0) as f32
        }
    };
    let preserve = match p.get("preserveTransparency") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(v)) => *v,
        Some(_) => return Err(EngineError::BadParams { cmd: "edit.stroke".into(), msg: "preserveTransparency must be true or false".into() }),
    };
    let location = p.get("location").and_then(Value::as_str).unwrap_or("center").to_string();
    let id = layer_param(s, p)?;
    s.edit("Stroke", |doc, _| {
        let canvas = doc.bounds();
        let (w, h) = (canvas.width() as usize, canvas.height() as usize);
        let edge = match &doc.selection {
            Some(m) => sel::mask_from_surface(Some(m), canvas),
            None => {
                let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
                let surf = l.surface().ok_or_else(|| EngineError::Other("not a pixel layer".into()))?;
                let mut px = vec![[0.0f32; 4]; w * h];
                surf.read_rgba_into(canvas, &mut px);
                px.iter().map(|p| p[3]).collect()
            }
        };
        // Past the canvas nothing is selected, so the canvas edge is a selection edge like any
        // other (Select All, then Stroke › Inside, frames the canvas): stroke on a padded copy.
        let pad = width + 1;
        let (pw, ph) = (w + 2 * pad, h + 2 * pad);
        let mut padded = vec![0.0f32; pw * ph];
        for (row, src) in padded.chunks_exact_mut(pw).skip(pad).zip(edge.chunks_exact(w.max(1))) {
            if let Some(dst) = row.get_mut(pad..pad + w) {
                dst.copy_from_slice(src);
            }
        }
        // Whole-pixel bands, fully opaque: Center puts the odd pixel inside (1 px is all inside).
        let (out_px, in_px) = match location.as_str() {
            "inside" => (0, width),
            "outside" => (width, 0),
            _ => (width / 2, width.div_ceil(2)),
        };
        // A hard-edged selection strokes hard, matching Photoshop pixel for pixel on curves:
        // outside, a pixel is stroked when its centre is less than width + 1 from a selected
        // pixel's; inside, less than width + ½ from an unselected pixel's (fitted to Photoshop's
        // strokes of a 14 px circle: 6 px Outside; 3, 5 and 6 px Inside). Both give whole
        // pixels on straight edges. Soft selections keep soft edges.
        let hard = padded.iter().all(|v| *v <= 0.0 || *v >= 1.0);
        let in_r = if hard && in_px > 0 { in_px as f32 - 0.5 } else { in_px as f32 };
        let mut outer = sel::expand(&padded, pw, ph, out_px as f32);
        let mut inner = sel::contract(&padded, pw, ph, in_r);
        if hard {
            outer.iter_mut().for_each(|v| *v = if *v > 0.0 { 1.0 } else { 0.0 });
            inner.iter_mut().for_each(|v| *v = if *v >= 1.0 { 1.0 } else { 0.0 });
        }
        let band: Vec<f32> = outer
            .chunks_exact(pw)
            .zip(inner.chunks_exact(pw))
            .skip(pad)
            .take(h)
            .flat_map(|(o, i)| o.iter().zip(i).skip(pad).take(w).map(|(o, i)| (o - i).clamp(0.0, 1.0)))
            .collect();
        let band = sel::mask_to_surface(&band, canvas);
        let lock = doc.effective_locks(id).transparency;
        let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
        let area = band.content_bounds().intersect(&canvas);
        if !area.is_empty() {
            crate::fill_cmds::blend_color_mask(surf, area, color, picked, &band, mode, opacity, preserve || lock)?;
            surf.prune();
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Histogram equalization, like Photoshop's: one CDF over the R, G and B values of
/// the selected pixels, applied to every channel.
fn equalize(s: &mut Session, p: &Value) -> Result<Value> {
    map_pixels(s, "Equalize", p, |_, px, cover| {
        let mut hist = [0f64; 256];
        for (p, &k) in px.iter().zip(cover) {
            if k > 0.0 && p[3] > 0.0 {
                for &v in &p[..3] {
                    hist[(v.clamp(0.0, 1.0) * 255.0).round() as usize] += 1.0;
                }
            }
        }
        let total: f64 = hist.iter().sum();
        if total == 0.0 {
            return;
        }
        let first = hist.iter().copied().find(|v| *v > 0.0).unwrap_or(0.0);
        let mut lut = [0f32; 256];
        let mut acc = 0.0;
        for (i, h) in hist.iter().enumerate() {
            acc += h;
            lut[i] = if total > first { ((acc - first) / (total - first)).clamp(0.0, 1.0) as f32 } else { i as f32 / 255.0 };
        }
        // Same 256 bins as the histogram, at every depth (like Photoshop).
        let map = |v: f32| lut[(v.clamp(0.0, 1.0) * 255.0).round() as usize];
        for p in px.iter_mut() {
            for v in &mut p[..3] {
                *v = map(*v);
            }
        }
    })
}

/// Filter › Blur › Average: the selection (or layer) filled with its mean colour.
fn average(s: &mut Session, p: &Value) -> Result<Value> {
    map_pixels(s, "Average", p, |_, px, cover| {
        let (mut sum, mut wsum, mut asum, mut n) = ([0f64; 3], 0f64, 0f64, 0f64);
        for (p, &k) in px.iter().zip(cover) {
            let w = (p[3] * k) as f64;
            for c in 0..3 {
                sum[c] += p[c] as f64 * w;
            }
            wsum += w;
            asum += (p[3] * k) as f64;
            n += k as f64;
        }
        if wsum <= 0.0 {
            return;
        }
        let avg = [(sum[0] / wsum) as f32, (sum[1] / wsum) as f32, (sum[2] / wsum) as f32, (asum / n.max(1e-9)) as f32];
        px.fill(avg);
    })
}

// ---------- transform presets ----------

/// The presets' params: like Free Transform, they follow the Channels and Layers panel target.
const TRANSFORM_TARGET: &str =
    r#"{"target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target (an unlinked mask, an alpha channel or the Quick Mask turns alone)}"#;

/// Flips and turns about the centre of the box Free Transform would open: with a selection
/// limiting the transform it is the selection's own bounds, as in Photoshop, so Select All
/// mirrors across the canvas (#6928); otherwise the content's bounds.
fn transform_preset(s: &mut Session, p: &Value, kind: &str) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    // A targeted alpha channel, the Quick Mask or an unlinked layer mask turns by itself (#2835).
    let (b, mut params) = match crate::transform_cmds::lone_target(&d.doc, d.active_layer, p)? {
        Some(surf) => (crate::transform_cmds::target_frame(&d.doc, surf), json!({"target": p.get("target")})),
        None => {
            let id = d.active_layer.ok_or(EngineError::Other("no active layer".into()))?;
            let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            (crate::transform_cmds::transform_frame(&d.doc, l), json!({"layer": id.0}))
        }
    };
    if b.is_empty() {
        return Err(EngineError::Other("nothing to transform".into()));
    }
    // Integer offsets keep these exact pixel permutations on the grid (nearest sampling).
    let (sx, sy) = ((b.x0 + b.x1) as f64, (b.y0 + b.y1) as f64);
    let (cx, cy) = (sx / 2.0, sy / 2.0);
    let m: [f64; 6] = match kind {
        "rotate180" => [-1.0, 0.0, 0.0, -1.0, sx, sy],
        "rotate90Cw" => [0.0, 1.0, -1.0, 0.0, (cx + cy).round(), (cy - cx).round()],
        "rotate90Ccw" => [0.0, -1.0, 1.0, 0.0, (cx - cy).round(), (cx + cy).round()],
        "flipHorizontal" => [-1.0, 0.0, 0.0, 1.0, sx, 0.0],
        _ => [1.0, 0.0, 0.0, -1.0, 0.0, sy],
    };
    params["matrix"] = json!(m);
    params["interpolation"] = json!("nearest");
    s.execute("edit.transform", params)
}

// ---------- selection / clipboard ----------

fn reselect_target(s: &Session) -> Option<photocraft_raster::Surface> {
    let d = s.active()?;
    if d.doc.selection.is_some() {
        return None;
    }
    (0..d.history.past_len()).rev().find_map(|i| d.history.state(i).and_then(|doc| doc.selection.clone()))
}

fn can_reselect(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    reselect_target(s).map(|_| ()).ok_or_else(|| "there is no selection to restore".into())
}

/// Edit › Paste Special › Paste Into (or Outside): the clipboard centred on the selection, not
/// clamped to the canvas, as a new layer masked to the selection (or its inverse), or into the
/// targeted mask or channel. One history step.
fn paste_into(s: &mut Session, p: &Value, outside: bool) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let selection = d.doc.selection.clone().ok_or(EngineError::Other("Paste Into needs a selection".into()))?;
    let canvas = d.doc.bounds();
    let b = selection.content_bounds();
    // Where the paste shows: the selection, or everything but it (default reveal, the
    // selection's area inverted).
    let limit = if outside {
        let area = b.intersect(&canvas);
        let inv: Vec<f32> = sel::mask_from_surface(Some(&selection), area).iter().map(|v| 1.0 - v).collect();
        let mut m = photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[1.0]);
        m.write_region(area, &inv);
        m
    } else {
        selection
    };
    let label = if outside { "Paste Outside" } else { "Paste Into" };
    // Centred on the selection, even where that runs off the canvas (not clamped like a paste).
    let mut params = json!({"center": [(b.x0 + b.x1) as f64 / 2.0, (b.y0 + b.y1) as f64 / 2.0], "clampToCanvas": false});
    if let Some(c) = p.get("center") {
        params["center"] = c.clone();
    }
    // A targeted colour channel (#2307): the paste goes into it, only where `limit` allows.
    if let Some(k) = crate::channel_clip::paste_color_target(s, p) {
        return crate::channel_clip::paste(s, &params, k, false, Some(&limit), label);
    }
    if crate::channel_cmds::target_of(p) != crate::channel_cmds::Target::Pixels {
        // A targeted mask or channel (#1035): the paste goes into it, only where `limit` allows.
        params["target"] = p.get("target").cloned().unwrap_or_default();
        return crate::edit_cmds::paste_to_target(s, &params, false, Some(&limit), label);
    }
    // The pasted layer lands beside the active one, inside its group, and a Lock-All group refuses
    // its layer mask: refuse before pasting rather than leave an unmasked paste behind.
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let group = d.active_layer.and_then(|a| d.doc.path_of(a)).and_then(|p| d.doc.layer_at(p.split_last()?.1).map(|g| g.id));
    if let Some(g) = group {
        crate::commands::check_all_unlocked(&d.doc, g)?;
    }
    let key = step_key(s, "pasteInto");
    params["coalesce"] = json!(key);
    params["target"] = json!("pixels");
    let before = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let r = s.execute("edit.paste", params)?;
    let id = layer_param(s, &Value::Null)?;
    s.execute("layer.layerMask.revealAll", json!({"layer": id.0, "coalesce": key}))?;
    s.coalesce_request = Some(key);
    let r2 = s.edit(label, |doc, _| {
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        l.mask = Some(LayerMask { surface: limit, linked: false, ..LayerMask::reveal_all() });
        Ok(())
    });
    s.coalesce_request = None;
    r2?;
    // Only where the masked paste draws needs recompositing (#2901).
    crate::layer_multi_cmds::note_insert(s, &before);
    Ok(r)
}

/// The canvas grown to every layer's pixels: Image › Reveal All's new canvas, and what the Crop
/// tool shows while a frame is edited.
pub fn reveal_all_bounds(doc: &Document) -> Rect {
    let canvas = doc.bounds();
    doc.walk().into_iter().filter_map(|(_, _, l)| l.surface().map(|s| visible_bounds(s, canvas))).fold(canvas, |a, b| a.union(&b))
}

/// Bounds of `s`'s pixels outside `canvas` with alpha > 0, as Photoshop's layer bounds (#9379):
/// the Eraser only clears alpha, so an erased pixel keeps its colour bytes and `content_bounds`
/// counts it. Pixels inside `canvas` can't grow the result and aren't read.
fn visible_bounds(s: &photocraft_raster::Surface, canvas: Rect) -> Rect {
    use rayon::prelude::*;
    let fmt = s.format();
    if !fmt.alpha || s.default_pixel().last().is_none_or(|a| *a > 0.0) {
        return s.content_bounds();
    }
    let (n, bpp, side) = (fmt.channels(), fmt.bytes_per_pixel(), photocraft_geom::TILE_SIZE);
    let tiles: Vec<_> = s.tiles().filter(|(c, _)| !canvas.contains_rect(&c.rect())).collect();
    let tile_bounds = |o: Rect, bytes: &[u8]| {
        let mut out = Rect::EMPTY;
        for (ly, row) in bytes.chunks_exact(bpp * side as usize).enumerate() {
            let y = o.y0.saturating_add(ly as i32);
            let on_canvas = (canvas.y0..canvas.y1).contains(&y);
            let visible = |lx: &i32| {
                let x = o.x0.saturating_add(*lx);
                !(on_canvas && (canvas.x0..canvas.x1).contains(&x))
                    && row.get(*lx as usize * bpp..).is_some_and(|px| photocraft_color::read_sample(px, fmt.sample, n - 1) > 0.0)
            };
            let Some(first) = (0..side).find(visible) else { continue };
            let last = (first..side).rfind(visible).unwrap_or(first);
            out = out.union(&Rect::new(o.x0.saturating_add(first), y, o.x0.saturating_add(last).saturating_add(1), y.saturating_add(1)));
        }
        out
    };
    tiles.par_iter().map(|(c, t)| tile_bounds(c.rect(), t.bytes())).reduce(|| Rect::EMPTY, |a, b| a.union(&b))
}

// ---------- layers ----------

fn layer_from_background(s: &mut Session, p: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    if d.doc.background().is_none() && p.get("ifPresent").and_then(Value::as_bool) == Some(true) {
        return Ok(json!({"layer": null}));
    }
    // `select: false`: the targeted layer stays targeted, as when a Photoshop action's `set`
    // names the Background while another layer is selected.
    let select = p.get("select").and_then(Value::as_bool).unwrap_or(true);
    let id = s.edit("Layer From Background", |doc, active| {
        let l = doc.background_mut().ok_or(EngineError::Other("the document has no Background layer".into()))?;
        unlock_background(l);
        if select || active.is_none() {
            *active = Some(l.id);
        }
        Ok(l.id)
    })?;
    Ok(json!({"layer": id.0}))
}

pub(crate) fn unlock_background(l: &mut Layer) {
    l.name = "Layer 0".into();
    l.locks.transparency = false;
    l.locks.position = false;
}

/// Before `id` gets a layer mask: the Background can't have one, so it becomes a normal layer
/// first ("Layer 0"), as when adding a mask to it in Photoshop. Other layers are left alone.
pub(crate) fn background_to_layer_for_mask(doc: &mut photocraft_doc::Document, id: photocraft_doc::LayerId) {
    background_to_layer(doc, id);
}

/// If `id` is the Background ([`photocraft_doc::Document::background`], whatever its name), turns
/// it into a normal layer ("Layer 0"), as Photoshop does before an edit the Background can't
/// take (moving, transforming, exposing transparency, a style). Other layers are left alone.
pub(crate) fn background_to_layer(doc: &mut photocraft_doc::Document, id: photocraft_doc::LayerId) {
    if let Some(l) = doc.background_mut().filter(|l| l.id == id) {
        unlock_background(l);
    }
}

fn toggle_mask(s: &mut Session, p: &Value, key: &str) -> Result<Value> {
    let id = layer_param(s, p)?;
    let label = if key == "enabled" { "Enable/Disable Layer Mask" } else { "Link/Unlink Layer Mask" };
    let v = s.edit(label, |doc, _| {
        let m = doc.layer_mut(id).and_then(|l| l.mask.as_mut()).ok_or(EngineError::Other("the layer has no layer mask".into()))?;
        let slot = if key == "enabled" { &mut m.enabled } else { &mut m.linked };
        *slot = p.get(key).and_then(Value::as_bool).unwrap_or(!*slot);
        Ok(*slot)
    })?;
    Ok(json!({ key: v }))
}

fn set_all_effects(s: &mut Session, enabled: bool) -> Result<Value> {
    s.edit(if enabled { "Show All Effects" } else { "Hide All Effects" }, |doc, _| {
        fn rec(ls: &mut [Layer], on: bool) {
            for l in ls {
                if !l.effects.items.is_empty() {
                    l.effects.enabled = on;
                }
                if let Some(ch) = l.children_mut() {
                    rec(ch, on);
                }
            }
        }
        rec(&mut doc.layers, enabled);
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Remove every layer matching `pred` (whole subtrees), keeping at least one layer.
fn delete_where(s: &mut Session, label: &str, pred: fn(&Layer) -> bool) -> Result<Value> {
    let n = s.edit(label, |doc, active| {
        fn rec(ls: &mut Vec<Layer>, pred: fn(&Layer) -> bool) -> usize {
            let before = ls.len();
            ls.retain(|l| !pred(l));
            let mut n = before - ls.len();
            for l in ls.iter_mut() {
                if let Some(ch) = l.children_mut() {
                    n += rec(ch, pred);
                }
            }
            n
        }
        let backup = doc.layers.clone();
        // The clip groups of the doomed layers, read before they go (clip_group rule 3: layers
        // clipped to a deleted base are released).
        let doomed: Vec<LayerId> = doc.walk().into_iter().filter(|(_, _, l)| pred(l)).map(|(_, _, l)| l.id).collect();
        let before = crate::clip_group::Before::read(doc, &doomed);
        let n = rec(&mut doc.layers, pred);
        if doc.layers.is_empty() {
            doc.layers = backup;
            return Err(EngineError::Other("a document must keep at least one layer".into()));
        }
        crate::clip_group::settle(doc, &[], &before, false);
        if active.is_none_or(|id| doc.layer(id).is_none()) {
            *active = doc.top_layer();
        }
        Ok(n)
    })?;
    Ok(json!({"deleted": n}))
}

fn is_empty_layer(l: &Layer) -> bool {
    match &l.content {
        LayerContent::Raster(s) => s.content_bounds().is_empty() && l.mask.is_none() && l.effects.items.is_empty(),
        LayerContent::Group(g) => g.children.iter().all(is_empty_layer),
        _ => false,
    }
}

fn ungroup(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    s.edit("Ungroup Layers", |doc, active| {
        let path = doc.path_of(id).ok_or(EngineError::NoLayer(id))?;
        let g = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        let LayerContent::Group(group) = &g.content else {
            return Err(EngineError::Other("the active layer is not a group".into()));
        };
        let children = group.children.clone();
        let (&idx, parent) = path.split_last().ok_or(EngineError::NoLayer(id))?;
        let sib = if parent.is_empty() { &mut doc.layers } else { doc.layer_at_mut(parent).and_then(Layer::children_mut).ok_or(EngineError::NoLayer(id))? };
        sib.remove(idx);
        let top = children.last().map(|l| l.id);
        for (k, l) in children.into_iter().enumerate() {
            sib.insert(idx + k, l);
        }
        *active = top.or_else(|| doc.top_layer());
        Ok(())
    })?;
    Ok(Value::Null)
}

fn set_visible(s: &mut Session, p: &Value, visible: bool) -> Result<Value> {
    if crate::layer_multi_cmds::multi(s, p) {
        return crate::layer_multi_cmds::set_visible_selected(s, visible);
    }
    let id = layer_param(s, p)?;
    s.edit_visibility(if visible { "Show Layer" } else { "Hide Layer" }, |doc, _| {
        doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.visible = visible;
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Layer › Hide Layers (⌘,): hide the targeted layers, or show them when every one is already
/// hidden, as Photoshop's item does (it reads Show Layers then), so ⌘, toggles.
fn hide_or_show(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let targets = if crate::layer_multi_cmds::multi(s, p) { crate::layer_multi_cmds::selected(s) } else { vec![layer_param(s, p)?] };
    let all_hidden = targets.iter().all(|id| st.doc.layer(*id).is_some_and(|l| !l.visible));
    set_visible(s, p, all_hidden)
}

/// `/`: Lock Transparent Pixels on or off for the targeted layers, as the Layers panel's first lock
/// button does. With several layers targeted, any unlocked one means lock them all. The
/// Background, which can't take this lock, is left out.
fn toggle_lock_transparency(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let targets: Vec<LayerId> = if crate::layer_multi_cmds::multi(s, p) { crate::layer_multi_cmds::selected(s) } else { vec![layer_param(s, p)?] };
    let targets: Vec<LayerId> = targets.into_iter().filter(|id| st.doc.layer(*id).is_some_and(|l| !st.doc.is_background(l.id))).collect();
    if targets.is_empty() {
        return Err(EngineError::Other("the Background layer can't lock transparent pixels".into()));
    }
    let lock = !targets.iter().all(|id| st.doc.layer(*id).is_some_and(|l| l.locks.transparency));
    s.edit(if lock { "Lock Transparent Pixels" } else { "Unlock Transparent Pixels" }, |doc, _| {
        for id in &targets {
            doc.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?.locks.transparency = lock;
        }
        Ok(())
    })?;
    Ok(json!({"locked": lock}))
}

/// Is the layer at `path` shown alone: it and its enclosing groups visible, every layer outside
/// it hidden (the layers inside a group keep their own visibility)?
fn shown_alone(doc: &Document, path: &[usize]) -> bool {
    doc.walk().iter().all(|(p, _, l)| if path.starts_with(p) { l.visible } else { p.starts_with(path) || !l.visible })
}

/// ⌥-click on a layer's eye: show only that layer (and its enclosing groups), or, when it already
/// is shown alone, restore every layer's visibility from before (every layer shown when there is
/// no snapshot). ⌥-clicking another eye while one layer is shown alone moves the solo and keeps
/// the snapshot. One history step either way; the snapshot is view state.
fn show_only(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = &st.doc;
    let path = doc.path_of(id).ok_or(EngineError::NoLayer(id))?;
    // The snapshot only counts while its solo is still in effect (an undo or a plain eye click ends it).
    let saved = st.show_only.as_ref().filter(|(prev, _)| doc.path_of(*prev).is_some_and(|pp| shown_alone(doc, &pp)));
    let walk = doc.walk();
    let solo = !shown_alone(doc, &path);
    let (label, rows, next): (_, Vec<(Vec<usize>, bool)>, _) = if solo {
        let snapshot = saved.map_or_else(|| walk.iter().map(|(_, _, l)| (l.id, l.visible)).collect(), |(_, v)| v.clone());
        // Layers inside the shown layer keep their visibility.
        let rows = walk.iter().filter(|(p, _, _)| p.len() <= path.len() || !p.starts_with(&path)).map(|(p, _, _)| (p.clone(), path.starts_with(p))).collect();
        ("Show Only This Layer", rows, Some((id, snapshot)))
    } else {
        let before: Option<std::collections::HashMap<LayerId, bool>> = saved.map(|(_, v)| v.iter().copied().collect());
        // Layers added since the snapshot keep their current visibility.
        let rows = walk.iter().map(|(p, _, l)| (p.clone(), before.as_ref().is_none_or(|b| b.get(&l.id).copied().unwrap_or(l.visible)))).collect();
        ("Show Layers", rows, None)
    };
    s.edit_visibility(label, |doc, _| {
        for (p, visible) in &rows {
            if let Some(l) = doc.layer_at_mut(p) {
                l.visible = *visible;
            }
        }
        Ok(())
    })?;
    if let Some(st) = s.active_mut() {
        st.show_only = next;
    }
    Ok(json!({ "shownAlone": solo }))
}

/// Pixels of a fill or smart-object layer's content alone (no mask, effects or opacity).
fn content_pixels(doc: &Document, l: &Layer) -> photocraft_raster::Surface {
    let mut tmp = l.clone();
    tmp.mask = None;
    tmp.vector_mask = None;
    tmp.effects.items.clear();
    tmp.opacity = 1.0;
    tmp.fill_opacity = 1.0;
    tmp.blend = photocraft_color::BlendMode::Normal;
    tmp.clipped = false;
    tmp.visible = true;
    let canvas = doc.bounds();
    let buf = photocraft_compose::render_layer(&tmp, canvas);
    let fmt = doc.pixel_format();
    let n = fmt.channels();
    let mut data = vec![0.0f32; buf.px.len() * n];
    for (p, out) in buf.px.iter().zip(data.chunks_exact_mut(n)) {
        from_rgba_into(&fmt, *p, out);
    }
    let mut s = photocraft_raster::Surface::new(fmt);
    s.write_region(canvas, &data);
    s.prune();
    s
}

pub(crate) fn kind_of(s: &Session, id: LayerId) -> Option<&'static str> {
    let d = s.active()?;
    Some(match &d.doc.layer(id)?.content {
        LayerContent::Text(_) => "type",
        LayerContent::Shape(_) => "shape",
        LayerContent::Fill(_) => "fill",
        LayerContent::Smart(_) => "smart",
        _ => return None,
    })
}

/// Rasterize one layer; `only` restricts it to one kind. Without `only` (Rasterize › Layer / All
/// Layers: "all vector data") a vector mask becomes a pixel mask too; layer effects stay live, as
/// in Photoshop (Rasterize › Layer Style bakes them). Returns false when there was nothing to do.
pub(crate) fn rasterize_one(s: &mut Session, id: LayerId, only: Option<&str>, key: &str) -> Result<bool> {
    let vector_mask = only.is_none() && s.active().and_then(|d| d.doc.layer(id)).is_some_and(|l| l.vector_mask.is_some());
    let content = rasterize_content(s, id, only, key)?;
    if vector_mask {
        s.execute("layer.rasterize.vectorMask", json!({"layer": id.0, "coalesce": key}))?;
    }
    Ok(content || vector_mask)
}

fn rasterize_content(s: &mut Session, id: LayerId, only: Option<&str>, key: &str) -> Result<bool> {
    let Some(kind) = kind_of(s, id).filter(|k| only.is_none_or(|o| o == *k)) else { return Ok(false) };
    match kind {
        "type" => {
            s.execute("type.rasterize", json!({"layer": id.0, "coalesce": key}))?;
        }
        "shape" => {
            s.execute("layer.rasterize.shape", json!({"layer": id.0, "coalesce": key}))?;
        }
        "smart" => {
            s.coalesce_request = Some(key.to_string());
            let r = s.edit("Rasterize Smart Object", |doc, _| {
                let snapshot = doc.clone();
                let mut snap = doc.layer(id).cloned().ok_or(EngineError::NoLayer(id))?;
                if matches!(snap.content, LayerContent::Smart(ref sm) if sm.cache.is_none()) {
                    let _ = crate::smart_cmds::refresh_layer(&snapshot, &mut snap);
                }
                let fmt = snapshot.pixel_format();
                let surface = match &mut snap.content {
                    LayerContent::Smart(sm) => match sm.cache.take() {
                        Some(c) => c,
                        None => content_pixels(&snapshot, &snap),
                    },
                    _ => return Err(EngineError::Other(format!("layer {} is not a smart object", id.0))),
                };
                let surface = if surface.format() == fmt { surface } else { surface.convert(fmt) };
                let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
                l.content = LayerContent::Raster(surface);
                l.psd_blocks.retain(|(k, _)| k != b"SoLd" && k != b"SoLE" && k != b"PlLd");
                Ok(())
            });
            s.coalesce_request = None;
            r?;
        }
        _ => {
            s.coalesce_request = Some(key.to_string());
            let r = s.edit("Rasterize Fill Content", |doc, _| {
                let snap = doc.layer(id).cloned().ok_or(EngineError::NoLayer(id))?;
                let px = content_pixels(doc, &snap);
                let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
                l.content = LayerContent::Raster(px);
                l.fill_cache = None;
                // The PSD fill or placed-layer blocks no longer describe the layer: kept, they
                // would reopen it as a fill layer or smart object and drop these pixels.
                l.psd_blocks.retain(|(k, _)| !matches!(k, b"SoCo" | b"GdFl" | b"PtFl" | b"SoLd" | b"PlLd" | b"SoLE"));
                Ok(())
            });
            s.coalesce_request = None;
            r?;
        }
    }
    Ok(true)
}

/// Layer › Rasterize › Layer / Type / Shape / Fill Content / Smart Object. With a `layer` param,
/// just that layer. Otherwise, as in Photoshop ("select the layer or layers you'd like to
/// rasterize"), every selected layer the command applies to, in one history step.
pub(crate) fn rasterize(s: &mut Session, p: &Value, only: Option<&'static str>) -> Result<Value> {
    let ids = crate::layer_multi_cmds::targets(s, p)?;
    let many = ids.len() > 1;
    let key = step_key(s, "rasterize");
    let mut done = Vec::new();
    for id in ids {
        if rasterize_one(s, id, only, &key)? {
            done.push(id.0);
        }
    }
    let Some(&first) = done.first() else {
        return Err(EngineError::Other(match (only, many) {
            (Some(k), false) => format!("the layer is not a {k} layer"),
            (Some(k), true) => format!("none of the selected layers is a {k} layer"),
            (None, false) => "the layer has nothing to rasterize".into(),
            (None, true) => "none of the selected layers has anything to rasterize".into(),
        }));
    };
    let active = s.active().and_then(|d| d.active_layer).map(|id| id.0).filter(|id| done.contains(id));
    Ok(json!({"layer": active.unwrap_or(first), "layers": done}))
}

pub(crate) fn rasterize_all(s: &mut Session) -> Result<Value> {
    let ids: Vec<LayerId> = s.active().map(|d| d.doc.walk().into_iter().map(|(_, _, l)| l.id).collect()).unwrap_or_default();
    let key = step_key(s, "rasterizeAll");
    let mut n = 0;
    for id in ids {
        // Every sub-step shares one history entry via the coalescing key.
        if rasterize_one(s, id, None, &key)? {
            n += 1;
        }
    }
    Ok(json!({"rasterized": n}))
}

// ---------- registry ----------

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $sc:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: $sc, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    vec![
        spec!(
            "edit.stroke",
            "Stroke…",
            &["Edit"],
            None,
            r##"{"width":1..250=1,"color":"#rrggbb|[r,g,b,a]"=foreground,"location":"inside|center|outside"="center","mode":"any layer blend mode"="normal","opacity":0..100=100,"preserveTransparency":bool=false}"##,
            has_pixels,
            stroke
        ),
        spec!("edit.transform.rotate180", "Rotate 180°", &["Edit", "Transform"], None, TRANSFORM_TARGET, has_layer, |s, p| transform_preset(
            s,
            p,
            "rotate180"
        )),
        spec!("edit.transform.rotate90Cw", "Rotate 90° Clockwise", &["Edit", "Transform"], None, TRANSFORM_TARGET, has_layer, |s, p| transform_preset(
            s,
            p,
            "rotate90Cw"
        )),
        spec!("edit.transform.rotate90Ccw", "Rotate 90° Counter Clockwise", &["Edit", "Transform"], None, TRANSFORM_TARGET, has_layer, |s, p| {
            transform_preset(s, p, "rotate90Ccw")
        }),
        spec!("edit.transform.flipHorizontal", "Flip Horizontal", &["Edit", "Transform"], None, TRANSFORM_TARGET, has_layer, |s, p| transform_preset(
            s,
            p,
            "flipHorizontal"
        )),
        spec!("edit.transform.flipVertical", "Flip Vertical", &["Edit", "Transform"], None, TRANSFORM_TARGET, has_layer, |s, p| transform_preset(
            s,
            p,
            "flipVertical"
        )),
        spec!(
            "edit.pasteSpecial.pasteInto",
            "Paste Into",
            &["Edit", "Paste Special"],
            Some("Cmd+Alt+Shift+V"),
            concat!(r#"{"center":[x,y]?,"#, crate::edit_cmds::paste_target!(), "}"),
            has_clip_and_selection,
            |s, p| paste_into(s, p, false)
        ),
        spec!(
            "edit.pasteSpecial.pasteOutside",
            "Paste Outside",
            &["Edit", "Paste Special"],
            None,
            concat!(r#"{"center":[x,y]?,"#, crate::edit_cmds::paste_target!(), "}"),
            has_clip_and_selection,
            |s, p| paste_into(s, p, true)
        ),
        spec!("select.reselect", "Reselect", &["Select"], Some("Cmd+Shift+D"), "{}", can_reselect, |s, _| {
            let m = reselect_target(s).ok_or(EngineError::Other("there is no selection to restore".into()))?;
            s.edit("Reselect", |doc, _| {
                doc.selection = Some(m);
                Ok(())
            })?;
            Ok(json!({"selected": true}))
        }),
        spec!("image.adjustments.equalize", "Equalize", &["Image", "Adjustments"], None, "{}", has_pixels, equalize),
        spec!("image.revealAll", "Reveal All", &["Image"], None, "{}", has_doc, |s, _| {
            let d = s.active().ok_or(EngineError::NoDocument)?;
            let all = reveal_all_bounds(&d.doc);
            if all == d.doc.bounds() {
                return Ok(json!({"changed": false}));
            }
            // One History step named as in Photoshop, not the crop's "Crop".
            let p = json!({"x": all.x0, "y": all.y0, "width": all.width(), "height": all.height(), "deleteCroppedPixels": false});
            crate::analysis_cmds::compound(s, "Reveal All", |s| s.execute("image.crop", p))?;
            Ok(json!({"changed": true}))
        }),
        spec!(
            "layer.new.layerFromBackground",
            "Layer From Background…",
            &["Layer", "New"],
            None,
            r##"{"ifPresent":bool=false (true: a document without a Background is left as it is, as a Photoshop action's step needs on an image that opened as a normal layer),"select":bool=true (false: the targeted layer stays targeted)}"##,
            has_background,
            layer_from_background
        ),
        spec!("layer.layerStyle.copyLayerStyle", "Copy Layer Style", &["Layer", "Layer Style"], None, r##"{"layer":id?}"##, has_layer, |s, p| {
            let id = layer_param(s, p)?;
            let d = s.active().ok_or(EngineError::NoDocument)?;
            let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            let style = crate::CopiedStyle {
                effects: l.effects.clone(),
                blend: l.blend,
                fill: l.fill_opacity,
                advanced: l.advanced,
                blend_if: l.blend_if.clone(),
                excluded_channels: l.excluded_channels,
                mode: d.doc.mode,
            };
            s.style_clipboard = Some(style);
            Ok(Value::Null)
        }),
        spec!(
            "layer.layerStyle.pasteLayerStyle",
            "Paste Layer Style",
            &["Layer", "Layer Style"],
            None,
            r##"{"layer":id?} (no layer: every selected layer but the Background)"##,
            |s| {
                has_layer(s)?;
                s.style_clipboard.as_ref().map(|_| ()).ok_or_else(|| "no layer style has been copied".into())
            },
            |s, p| {
                let ids = style_targets(s, p, |_| true)?;
                let style = s.style_clipboard.clone().ok_or(EngineError::Other("no layer style has been copied".into()))?;
                let mut fx = style.effects;
                s.edit("Paste Layer Style", |doc, _| {
                    // A style copied from a document in another mode takes this one's colours.
                    let mode = doc.mode;
                    fx.items = std::mem::take(&mut fx.items).into_iter().map(|e| e.in_mode(mode)).collect();
                    // Blend If's Gray range carries over to any mode; its channel ranges and the
                    // channel restrictions only to the mode they were set in.
                    let same_mode = style.mode == mode;
                    let blend_if = if same_mode {
                        style.blend_if.clone()
                    } else {
                        // Through `set`, which keeps the normalised form (no trailing full
                        // entries), so a full Gray range pastes as the default.
                        let mut gray = photocraft_doc::BlendIf::default();
                        gray.set(0, style.blend_if.get(0));
                        gray
                    };
                    for id in ids {
                        background_to_layer(doc, id);
                        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
                        l.effects = fx.clone();
                        // Pass Through is a group's mode: a group's style pastes onto other layers as
                        // Normal, as a merged group's Pass Through becomes Normal.
                        l.blend = if style.blend == photocraft_color::BlendMode::PassThrough && !l.is_group() {
                            photocraft_color::BlendMode::Normal
                        } else {
                            style.blend
                        };
                        l.fill_opacity = style.fill;
                        l.advanced = style.advanced;
                        l.blend_if = blend_if.clone();
                        l.excluded_channels = if same_mode { style.excluded_channels } else { 0 };
                    }
                    Ok(())
                })?;
                Ok(Value::Null)
            }
        ),
        spec!("layer.layerStyle.hideAllEffects", "Hide All Effects", &["Layer", "Layer Style"], None, "{}", has_doc, |s, _| set_all_effects(s, false)),
        spec!("layer.layerStyle.showAllEffects", "Show All Effects", &[], None, "{}", has_doc, |s, _| set_all_effects(s, true)),
        spec!(
            "layer.layerMask.enabled",
            "Disable Layer Mask",
            &["Layer", "Layer Mask"],
            None,
            r##"{"layer":id?,"enabled":bool? (default: toggle)}"##,
            has_mask,
            |s, p| toggle_mask(s, p, "enabled")
        ),
        spec!(
            "layer.layerMask.linked",
            "Unlink Layer Mask",
            &["Layer", "Layer Mask"],
            None,
            r##"{"layer":id?,"linked":bool? (default: toggle)}"##,
            has_mask,
            |s, p| toggle_mask(s, p, "linked")
        ),
        spec!(
            "layer.rasterize.layer",
            "Layer",
            &["Layer", "Rasterize"],
            None,
            r##"{"layer":id?} (no layer: every selected layer it applies to)"##,
            has_layer,
            |s, p| rasterize(s, p, None)
        ),
        spec!("layer.rasterize.allLayers", "All Layers", &["Layer", "Rasterize"], None, "{}", has_doc, |s, _| rasterize_all(s)),
        spec!(
            "layer.rasterize.type",
            "Type",
            &["Layer", "Rasterize"],
            None,
            r##"{"layer":id?} (no layer: every selected layer it applies to)"##,
            has_layer,
            |s, p| rasterize(s, p, Some("type"))
        ),
        spec!(
            "layer.rasterize.fillContent",
            "Fill Content",
            &["Layer", "Rasterize"],
            None,
            r##"{"layer":id?} (no layer: every selected layer it applies to)"##,
            has_layer,
            |s, p| rasterize(s, p, Some("fill"))
        ),
        spec!(
            "layer.rasterize.smartObject",
            "Smart Object",
            &["Layer", "Rasterize"],
            None,
            r##"{"layer":id?} (no layer: every selected layer it applies to)"##,
            has_layer,
            |s, p| rasterize(s, p, Some("smart"))
        ),
        spec!("layer.delete.hiddenLayers", "Hidden Layers", &["Layer", "Delete"], None, "{}", has_doc, |s, _| delete_where(s, "Delete Hidden Layers", |l| !l
            .visible)),
        spec!("file.scripts.deleteAllEmptyLayers", "Delete All Empty Layers", &["File", "Scripts"], None, "{}", has_doc, |s, _| delete_where(
            s,
            "Delete All Empty Layers",
            is_empty_layer
        )),
        spec!(
            "layer.ungroupLayers",
            "Ungroup Layers",
            &["Layer"],
            Some("Cmd+Shift+G"),
            r##"{"layer":id?}"##,
            |s| match active_layer(s)?.is_group() {
                true => Ok(()),
                false => Err("the active layer is not a group".into()),
            },
            ungroup
        ),
        spec!(
            "layer.hideLayers",
            "Hide Layers",
            &["Layer"],
            Some("Cmd+,"),
            r##"{"layer":id?} (shows them instead when every targeted layer is already hidden, so ⌘, toggles)"##,
            has_layer,
            hide_or_show
        ),
        spec!("layer.showLayers", "Show Layers", &[], None, r##"{"layer":id?}"##, has_layer, |s, p| set_visible(s, p, true)),
        spec!(
            "layer.toggleLockTransparency",
            "Lock Transparent Pixels",
            &[],
            Some("/"),
            r##"{"layer":id?} (toggles; several layers selected: locks them all unless every one is locked)"##,
            has_layer,
            toggle_lock_transparency
        ),
        spec!(
            "layer.showOnly",
            "Show Only This Layer",
            &[],
            None,
            r##"{"layer":id?} → {shownAlone} (⌥-click a layer's eye; again restores the other layers' visibility)"##,
            has_layer,
            show_only
        ),
        spec!("filter.blur.average", "Average", &["Filter", "Blur"], None, "{}", has_pixels, average),
    ]
}

#[cfg(test)]
mod tests;
