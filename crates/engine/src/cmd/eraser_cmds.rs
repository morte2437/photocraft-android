//! The Eraser group's colour-aware tools: Magic Eraser (`paint.magicEraser`, a wand-style region
//! erased to transparency) and Background Eraser (`paint.backgroundEraser`, a brush that erases
//! colours similar to the one under its hotspot).
//!
//! Like Photoshop, both turn a Background layer into a normal layer ("Layer 0") first, since the
//! Background can't hold transparency. The Magic Eraser paints the background colour on other
//! layers with locked transparency; the Background Eraser erases them to transparency and leaves
//! the lock on. Both respect the selection, work in the layer's own colour model and depth, and are
//! one undo step per click / stroke.
//!
//! The Magic Eraser refuses Indexed Color and Bitmap documents with the reference's alerts
//! ([`MAGIC_ERASER_INDEXED_ERROR`] and [`MAGIC_ERASER_BITMAP_ERROR`], #12032), before it samples
//! anything, so a refused click leaves the document exactly as it was.

use photocraft_algo::selection as sel;
use photocraft_color::{ColorMode, SampleType};
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

/// The reference's alert for a Magic Eraser click in an Indexed Color document (#12032), measured:
/// one alert with an OK button. The UI shows this text through `tl!` (English is the catalog
/// source); the command returns it as its error.
pub const MAGIC_ERASER_INDEXED_ERROR: &str =
    "Could not use the magic eraser because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";
/// The same alert for a Bitmap document (#12032), which points at Grayscale instead.
pub const MAGIC_ERASER_BITMAP_ERROR: &str =
    "Could not use the magic eraser because it does not work with Bitmap images (convert image to Grayscale mode to edit).";

/// The alert `doc` gets instead of a Magic Eraser click, or `None` when the click may erase
/// (#12032). Indexed Color and Bitmap are the two measured refusals, at every bit depth; Duotone
/// and Multichannel are not measured and keep erasing, as do RGB, Grayscale, CMYK and Lab.
/// The text is exactly what `paint.magicEraser` answers with, so the tool can refuse the press with
/// what the command would refuse.
pub fn magic_eraser_refusal(doc: &Document) -> Option<&'static str> {
    match doc.mode {
        ColorMode::Indexed => Some(MAGIC_ERASER_INDEXED_ERROR),
        ColorMode::Bitmap => Some(MAGIC_ERASER_BITMAP_ERROR),
        _ => None,
    }
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
    // #12032: an Indexed Color or a Bitmap document gets the reference's alert before anything is
    // sampled or edited, so the click changes nothing: no pixels, no selection, no History step,
    // and the Background stays a Background. The tool refuses the press with this same text
    // (`eraser_ui::click`), which is why the check belongs to the command.
    if let Some(msg) = s.active().and_then(|st| magic_eraser_refusal(&st.doc)) {
        return Err(EngineError::Other(msg.into()));
    }
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
    // The reference's colour distance is measured for 8-bit RGB only (#11995).
    let rgb8 = s.active().is_some_and(|d| d.doc.mode == ColorMode::Rgb && d.doc.depth == SampleType::U8);
    let distance = if rgb8 { sel::ColorDistance::Spread } else { sel::ColorDistance::LargestChannel };
    let region = sel::wand_region_by(&img, area, (x, y), tolerance, contiguous, anti_alias, None, distance);
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

/// The reference application's alert for a colour-channel target that isn't the composite (#12488).
const CHANNEL_TARGET_REFUSED: &str = "Could not use the background eraser because the target channels do not cover the composite.";

/// The stroke, settings, layer and colour-channel target of a `paint.backgroundEraser` (shared by
/// the command and its live preview), after the same checks.
fn background_eraser_setup(s: &Session, p: &Value) -> Result<(Stroke, BgEraseSettings, LayerId, Option<crate::color_channels::ColorChannels>)> {
    const CMD: &str = "paint.backgroundEraser";
    let pts = crate::brush_cmds::parse_points(p, CMD)?;
    if pts.iter().any(|q| !q.x.is_finite() || !q.y.is_finite() || q.x.abs() > 1e7 || q.y.abs() > 1e7) {
        return Err(bad(CMD, "`points` must be finite canvas coordinates"));
    }
    let brush = crate::brush_cmds::resolve_brush(s, p, CMD)?;
    // The inks a colour was picked as, for a CMYK document (#12352).
    let picked = |c: [f32; 4]| s.tools.picked_cmyk([c[0], c[1], c[2]]).map(|k| ([c[0], c[1], c[2]], k));
    let bs = BgEraseSettings {
        sampling: enum_param(p, CMD, "sampling", Sampling::Continuous, "continuous|once|backgroundSwatch")?,
        limits: enum_param(p, CMD, "limits", Limits::Contiguous, "discontiguous|contiguous|findEdges")?,
        tolerance: number(p, CMD, "tolerance", 50.0, 1.0..=100.0)? as f32 / 100.0,
        protect_foreground: flag(p, "protectForegroundColor", false),
        foreground: s.tools.foreground,
        background: s.tools.background,
        foreground_cmyk: picked(s.tools.foreground),
        background_cmyk: picked(s.tools.background),
    };
    let id = active_pixel_layer(s, p, CMD)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    // Lock transparent pixels is no refusal: the reference application erases to transparency
    // anyway and leaves the lock on.
    if let Some(e) = pixels_locked(&d.doc, l) {
        return Err(e);
    }
    let colors = crate::color_channels::target(s, p)?;
    // Measured: one colour channel of an RGB document. Other partial sets keep erasing those channels.
    if d.doc.mode == ColorMode::Rgb && colors.is_some_and(|c| c.len() == 1) {
        return Err(EngineError::Other(CHANNEL_TARGET_REFUSED.into()));
    }
    Ok((Stroke { brush, points: pts }, bs, id, colors))
}

/// `paint.backgroundEraser`: a stroke erasing colours similar to the sampled one.
fn background_eraser(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "paint.backgroundEraser";
    let (stroke, bs, id, colors) = background_eraser_setup(s, p)?;
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
            photocraft_paint::retouch::apply_dab_stroke(surf, &background_eraser_stroke(&stroke), selection.as_ref(), false, 0, effect)
                .map_err(|e| bad(CMD, e.to_string()))
        } else {
            apply_background_eraser(surf, &stroke, &bs, selection.as_ref()).map_err(|e| bad(CMD, e.to_string()))
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
    let (stroke, bs, id, colors) = background_eraser_setup(s, p)?;
    let mut doc = (*s.active().ok_or(EngineError::NoDocument)?.doc).clone();
    doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let bg = s.tools.background;
    if colors.is_none() {
        crate::extra_cmds::background_to_layer(&mut doc, id);
    }
    let mut stroke = background_eraser_stroke(&stroke);
    // The commit mixes back without the transparency lock (a locked layer is erased too, and a
    // channel target leaves alpha alone), so the preview does as well; the points go in once the
    // lock is off.
    let points = std::mem::take(&mut stroke.points);
    if let Some(colors) = colors {
        let mut live = crate::retouch_cmds::LiveDab::with_effect_on(doc, CMD, &json!({"target":colors.params()}), stroke, Some(id), 0, |pre| {
            selected_background_effect(pre, &bs, colors, bg)
        })?
        .unlocked();
        live.push(&points)?;
        return live.with_tail_preview();
    }
    let mut live = crate::retouch_cmds::LiveDab::with_effect_on(doc, CMD, &json!({}), stroke, Some(id), 0, |pre| match background_eraser_effect(pre, &bs) {
        Some(effect) => Box::new(effect),
        // No alpha: the commit erases nothing either.
        None => Box::new(|_: &mut photocraft_paint::retouch::Region, _: &photocraft_paint::retouch::Footprint| {}),
    })?
    .unlocked();
    live.push(&points)?;
    live.with_tail_preview()
}

/// Eraser-group command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "paint.magicEraser",
            label: "Magic Eraser",
            menu: &[],
            shortcut: None,
            params: r##"{"x":px,"y":px,"tolerance":0..255=32,"antiAlias":bool=true,"contiguous":bool=true,"sampleAllLayers":bool=false,"opacity":1..100=100} (one undo step; a click off the canvas erases nothing and records no step; refused with the tool's alert and nothing changes: an Indexed Color document answers "Could not use the magic eraser because it does not work with Indexed Color images (convert image to RGB Color mode to edit).", a Bitmap document "Could not use the magic eraser because it does not work with Bitmap images (convert image to Grayscale mode to edit)."; Duotone and Multichannel erase as before)"##,
            enabled: has_pixel_layer,
            run: magic_eraser,
            journal: true,
        },
        CommandSpec {
            id: "paint.backgroundEraser",
            label: "Background Eraser",
            menu: &[],
            shortcut: None,
            params: r##"{"points":[[x,y,pressure?],…],"size":px?,"hardness":0..1?,"brush":{…}?,"preset":name?,"sampling":"continuous|once|backgroundSwatch"="continuous","limits":"discontiguous|contiguous|findEdges"="contiguous","tolerance":1..100=50,"protectForegroundColor":bool=false,"seed":u64?}"##,
            enabled: has_pixel_layer,
            run: background_eraser,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod magic_eraser_mode_refusal_tests;
#[cfg(test)]
mod magic_eraser_tolerance_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
#[path = "background_eraser_transparency_lock_tests.rs"]
mod transparency_lock_tests;

#[cfg(test)]
#[path = "background_eraser_channel_target_tests.rs"]
mod channel_target_tests;
