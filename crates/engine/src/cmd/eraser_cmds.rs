//! The Eraser group's colour-aware tools: Magic Eraser (`paint.magicEraser`, a wand-style region
//! erased to transparency) and Background Eraser (`paint.backgroundEraser`, a brush that erases
//! colours similar to the one under its hotspot).
//!
//! Like Photoshop, both turn a Background layer into a normal layer ("Layer 0") first, since the
//! Background can't hold transparency. The Magic Eraser paints the background colour on other
//! layers with locked transparency; the Background Eraser refuses them. Both respect the selection,
//! work in the layer's own colour model and depth, and are one undo step per click / stroke.

use photocraft_algo::selection as sel;
use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_paint::Stroke;
use photocraft_paint::bg_erase::{BgEraseSettings, apply_background_eraser, background_eraser_effect, background_eraser_stroke};
use photocraft_paint::replace::{Limits, Sampling};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn flag(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}

/// A finite number param clamped to `range`, `default` when absent; non-numbers are an error.
/// A numeric param clamped to `range`; missing or null gives `default`, anything else not a
/// finite number is BadParams. Shared with the Paint Bucket and Gradient opacity.
pub(crate) fn number(p: &Value, cmd: &str, k: &str, default: f64, range: std::ops::RangeInclusive<f64>) -> Result<f64> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => {
            v.as_f64().filter(|f| f.is_finite()).map(|f| f.clamp(*range.start(), *range.end())).ok_or_else(|| bad(cmd, format!("`{k}` must be a number")))
        }
    }
}

fn enum_param<T: serde::de::DeserializeOwned>(p: &Value, cmd: &str, k: &str, default: T, allowed: &str) -> Result<T> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => serde_json::from_value(v.clone()).map_err(|_| bad(cmd, format!("`{k}` must be one of {allowed}"))),
    }
}

/// The active layer must hold pixels (the erasers don't work on type, shapes, smart objects…).
fn has_pixel_layer(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    let l = d.active_layer.and_then(|id| d.doc.layer(id)).ok_or("no active layer")?;
    if matches!(l.content, LayerContent::Raster(_)) {
        Ok(())
    } else {
        Err(format!("active layer is {} {} layer, not a pixel layer", l.content.article(), l.content.kind_name()))
    }
}

fn active_pixel_layer(s: &Session, p: &Value, cmd: &str) -> Result<LayerId> {
    let id = crate::commands::layer_param(s, p)?;
    let layer = s.active().and_then(|d| d.doc.layer(id)).ok_or(EngineError::NoLayer(id))?;
    if !matches!(layer.content, LayerContent::Raster(_)) {
        return Err(bad(cmd, "the targeted layer is not a pixel layer"));
    }
    Ok(id)
}

fn pixels_locked(doc: &Document, l: &Layer) -> Option<EngineError> {
    let locks = doc.effective_locks(l.id);
    (locks.pixels || locks.all).then(|| EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)))
}

fn damage(s: &mut Session, dmg: Rect, erased: bool) -> Value {
    if !dmg.is_empty() {
        s.hint_damage(dmg);
    }
    json!({ "erased": erased, "damage": [dmg.x0, dmg.y0, dmg.width(), dmg.height()] })
}

/// `paint.magicEraser`: erase the wand region around `(x, y)`.
fn magic_eraser(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "paint.magicEraser";
    let coord = |k: &str| -> Result<f64> {
        p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite() && v.abs() < 1e9).ok_or_else(|| bad(CMD, format!("missing or invalid `{k}`")))
    };
    let (x, y) = (coord("x")?.floor() as i32, coord("y")?.floor() as i32);
    let tolerance = number(p, CMD, "tolerance", 32.0, 0.0..=255.0)? as f32;
    let opacity = number(p, CMD, "opacity", 100.0, 1.0..=100.0)? as f32 / 100.0;
    let (anti_alias, contiguous, sample_all) = (flag(p, "antiAlias", true), flag(p, "contiguous", true), flag(p, "sampleAllLayers", false));
    let id = active_pixel_layer(s, p, CMD)?;
    {
        let d = s.active().ok_or(EngineError::NoDocument)?;
        if let Some(e) = d.doc.layer(id).and_then(|l| pixels_locked(&d.doc, l)) {
            return Err(e);
        }
        // Clicking off the canvas erases nothing (and records no history step).
        if !d.doc.bounds().contains(x, y) {
            return Ok(damage(s, Rect::EMPTY, false));
        }
    }
    let (area, img) = if sample_all {
        crate::selection_cmds::sample_rgba8(s, true)?
    } else {
        let d = s.active().ok_or(EngineError::NoDocument)?;
        let surf = d.doc.layer(id).and_then(Layer::surface).ok_or(EngineError::NoLayer(id))?;
        (d.doc.bounds(), sel::rgba8_image(surf, d.doc.bounds()))
    };
    let region = sel::wand_region(&img, area, (x, y), tolerance, contiguous, anti_alias);
    drop(img);
    let Some(region) = region else { return Ok(damage(s, Rect::EMPTY, false)) };
    let bg = s.tools.background;
    let channels = crate::color_channels::target(s, p)?.is_some();
    let dmg = s.edit("Magic Eraser", |doc, _| {
        let selection = doc.selection.clone();
        // Photoshop converts the Background to a normal layer before erasing to transparency.
        doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        if !channels {
            crate::extra_cmds::background_to_layer(doc, id);
        }
        let lock = doc.effective_locks(id).transparency || channels;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let surf = l.surface_mut().ok_or_else(|| bad(CMD, "not a pixel layer"))?;
        let fmt = surf.format();
        // Locked transparency: paint the background colour instead (Photoshop).
        let lock_color = lock.then(|| photocraft_raster::from_rgba(&fmt, [bg[0], bg[1], bg[2], 1.0]));
        Ok(photocraft_algo::erase::magic_erase(surf, &region, opacity, selection.as_ref(), lock_color.as_deref()))
    })?;
    Ok(damage(s, dmg, true))
}

/// The stroke, settings and layer of a `paint.backgroundEraser` (shared by the command and its
/// live preview), after the same checks.
fn background_eraser_setup(s: &Session, p: &Value) -> Result<(Stroke, BgEraseSettings, LayerId)> {
    const CMD: &str = "paint.backgroundEraser";
    let pts = crate::brush_cmds::parse_points(p, CMD)?;
    if pts.iter().any(|q| !q.x.is_finite() || !q.y.is_finite() || q.x.abs() > 1e7 || q.y.abs() > 1e7) {
        return Err(bad(CMD, "`points` must be finite canvas coordinates"));
    }
    let brush = crate::brush_cmds::resolve_brush(s, p, CMD)?;
    let bs = BgEraseSettings {
        sampling: enum_param(p, CMD, "sampling", Sampling::Continuous, "continuous|once|backgroundSwatch")?,
        limits: enum_param(p, CMD, "limits", Limits::Contiguous, "discontiguous|contiguous|findEdges")?,
        tolerance: number(p, CMD, "tolerance", 50.0, 0.0..=100.0)? as f32 / 100.0,
        protect_foreground: flag(p, "protectForegroundColor", false),
        foreground: s.tools.foreground,
        background: s.tools.background,
    };
    let id = active_pixel_layer(s, p, CMD)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    if let Some(e) = pixels_locked(&d.doc, l) {
        return Err(e);
    }
    if d.doc.effective_locks(id).transparency && !d.doc.is_background(id) && crate::color_channels::target(s, p)?.is_none() {
        return Err(EngineError::Other(format!("Could not use the Background Eraser because the transparency of layer \"{}\" is locked", l.name)));
    }
    Ok((Stroke { brush, points: pts }, bs, id))
}

/// `paint.backgroundEraser`: a stroke erasing colours similar to the sampled one.
fn background_eraser(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "paint.backgroundEraser";
    let (stroke, bs, id) = background_eraser_setup(s, p)?;
    let colors = crate::color_channels::target(s, p)?;
    let bg = s.tools.background;
    let dmg = s.edit("Background Eraser", |doc, _| {
        let selection = doc.selection.clone();
        if colors.is_none() {
            crate::extra_cmds::background_to_layer(doc, id);
        }
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let surf = l.surface_mut().ok_or_else(|| bad(CMD, "not a pixel layer"))?;
        if let Some(colors) = colors {
            let effect = selected_background_effect(surf, &bs, colors, bg);
            Ok(photocraft_paint::retouch::apply_dab_stroke(surf, &background_eraser_stroke(&stroke), selection.as_ref(), false, 0, effect))
        } else {
            Ok(apply_background_eraser(surf, &stroke, &bs, selection.as_ref()))
        }
    })?;
    Ok(damage(s, dmg, true))
}

fn selected_background_effect(
    pre: &photocraft_raster::Surface,
    bs: &BgEraseSettings,
    colors: crate::color_channels::ColorChannels,
    bg: [f32; 4],
) -> crate::retouch_cmds::DabEffect {
    let fmt = pre.format();
    let alpha = fmt.mode.color_channels();
    let background = photocraft_raster::from_rgba(&fmt, [bg[0], bg[1], bg[2], 1.0]);
    // Opaque Backgrounds have no alpha sample. The matching algorithm uses a temporary opaque
    // alpha to calculate erased coverage; the real layer keeps its original format and identity.
    let opaque = pre.convert(photocraft_color::PixelFormat::new(fmt.mode, fmt.sample, true));
    let Some(mut effect) = background_eraser_effect(&opaque, bs) else {
        return Box::new(|_, _| {});
    };
    Box::new(move |region, footprint| {
        use photocraft_paint::retouch::Region;
        let count = region.ch.max(1);
        let before = region.data.clone();
        let mut work = Region::new(region.rect, alpha + 1);
        for (src, dst) in before.chunks_exact(count).zip(work.data.chunks_exact_mut(alpha + 1)) {
            for (v, o) in dst.iter_mut().zip(src) {
                *v = *o;
            }
            if !fmt.alpha
                && let Some(a) = dst.get_mut(alpha)
            {
                *a = 1.0;
            }
        }
        effect(&mut work, footprint);
        for ((out, old), erased) in region.data.chunks_exact_mut(count).zip(before.chunks_exact(count)).zip(work.data.chunks_exact(alpha + 1)) {
            let a = if fmt.alpha { old.get(alpha).copied().unwrap_or(0.0) } else { 1.0 };
            let coverage = if a > 0.0 { (1.0 - erased.get(alpha).copied().unwrap_or(a) / a).clamp(0.0, 1.0) } else { 0.0 };
            for (k, (v, original)) in out.iter_mut().zip(old).enumerate() {
                *v = if colors.contains(k) { original + (background.get(k).copied().unwrap_or(*original) - original) * coverage } else { *original };
            }
        }
    })
}

/// A `paint.backgroundEraser` stroke shown while it is drawn: the same params, checks and
/// erasing per dab as the commit (on a Background already turned into a normal layer), so the
/// preview at the last point is the committed result.
pub fn live_background_eraser(s: &Session, p: &Value) -> Result<crate::retouch_cmds::LiveDab> {
    const CMD: &str = "paint.backgroundEraser";
    let (stroke, bs, id) = background_eraser_setup(s, p)?;
    let mut doc = (*s.active().ok_or(EngineError::NoDocument)?.doc).clone();
    doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let colors = crate::color_channels::target(s, p)?;
    let bg = s.tools.background;
    if colors.is_none() {
        crate::extra_cmds::background_to_layer(&mut doc, id);
    }
    let stroke = background_eraser_stroke(&stroke);
    if let Some(colors) = colors {
        return crate::retouch_cmds::LiveDab::with_effect_on(doc, CMD, &json!({"target":colors.params()}), stroke, Some(id), 0, |pre| {
            selected_background_effect(pre, &bs, colors, bg)
        })?
        .with_tail_preview();
    }
    crate::retouch_cmds::LiveDab::with_effect_on(doc, CMD, &json!({}), stroke, Some(id), 0, |pre| match background_eraser_effect(pre, &bs) {
        Some(effect) => Box::new(effect),
        // No alpha: the commit erases nothing either.
        None => Box::new(|_: &mut photocraft_paint::retouch::Region, _: &photocraft_paint::retouch::Footprint| {}),
    })?
    .with_tail_preview()
}

/// Eraser-group command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "paint.magicEraser",
            label: "Magic Eraser",
            menu: &[],
            shortcut: None,
            params: r##"{"x":px,"y":px,"tolerance":0..255=32,"antiAlias":bool=true,"contiguous":bool=true,"sampleAllLayers":bool=false,"opacity":1..100=100}"##,
            enabled: has_pixel_layer,
            run: magic_eraser,
            journal: true,
        },
        CommandSpec {
            id: "paint.backgroundEraser",
            label: "Background Eraser",
            menu: &[],
            shortcut: None,
            params: r##"{"points":[[x,y,pressure?],…],"size":px?,"hardness":0..1?,"brush":{…}?,"preset":name?,"sampling":"continuous|once|backgroundSwatch"="continuous","limits":"discontiguous|contiguous|findEdges"="contiguous","tolerance":0..100=50,"protectForegroundColor":bool=false,"seed":u64?}"##,
            enabled: has_pixel_layer,
            run: background_eraser,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests;
