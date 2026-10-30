//! Retouching tools as commands: Clone Stamp, Pattern Stamp, Healing Brush, Spot Healing Brush,
//! Patch, Content-Aware Move, Remove, Dodge, Burn, Sponge, Blur, Sharpen, Smudge, History Brush and
//! Art History Brush.
//!
//! Every command takes a Photoshop-style brush (`points`, `size`, `hardness`, `opacity`, `flow`,
//! `spacing`, `layer`), respects the active selection as a mask and the layer's transparency lock, and
//! works at any depth (8/16/32f) and colour model. Like the Brush, each paints the targeted surface:
//! the layer's pixels, its mask (`"target":"mask"`), an alpha channel or the Quick Mask (`"target"`,
//! filled in from the Channels panel when absent). The pixel algorithms live in `photocraft-algo`
//! (`poisson`, `inpaint`, `retouch`) and the dab machinery in `photocraft-paint::retouch`; this module
//! only parses parameters and wires them together.

use photocraft_algo::inpaint::{self, CompleteParams};
use photocraft_algo::poisson;
use photocraft_algo::retouch::{ToneRange, dodge_burn, local_blur, local_sharpen, sponge};
use photocraft_color::{BlendMode, PixelFormat, SampleType};
use photocraft_doc::{Document, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_paint::retouch::{Footprint, Region, Smudge, alpha_index, apply_coverage, apply_dab_stroke};
use photocraft_paint::{BrushSettings, Stroke, StrokePoint};
use photocraft_raster::{Surface, from_rgba_into, to_rgba};
use rayon::prelude::*;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, blend_from_str};
use crate::{EngineError, Result, Session};

mod art_history;
mod content_aware_move;
mod patch;
mod pattern_stamp;
pub use pattern_stamp::live_impressionist as live_pattern_impressionist;
mod periodic;
mod remove;
pub use content_aware_move::{ContentAwarePreview, preview as content_aware_move_preview};
pub use patch::preview as patch_preview;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// A single stroke's accumulated coverage must stay at or under this many pixels (8192², so a
/// stroke corner to corner across a 24–36 MP canvas fits). Larger footprints return an error
/// instead of allocating.
const MAX_STROKE_PIXELS: u64 = 67_108_864;

/// Refuse a stroke whose footprint would not fit the coverage budget, from the points and brush
/// size alone, before the renderer allocates anything for it.
fn reject_huge_stroke(cmd: &str, stroke: &Stroke) -> Result<()> {
    // Dynamics place dabs beyond the input points, so half the tip is not the whole reach:
    // Scattering offsets a centre by up to its amount times the tip size, and a Dual Brush dab
    // reaches its own radius further. Pad by what those can add so this estimate does not fall
    // short of what the renderer will cover. It is only the cheap early-out; the rendered extent
    // is admitted for real in `coverage` below.
    let b = &stroke.brush;
    let size = f64::from(b.size.max(1.0));
    let scatter = if b.scattering.enabled { size * f64::from(b.scattering.scatter.jitter.max(0.0)) } else { 0.0 };
    let dual = if b.dual_brush.enabled { f64::from(b.dual_brush.size.max(0.0)) * 0.5 } else { 0.0 };
    let pad = size.mul_add(0.5, 4.0) + scatter + dual;
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for q in &stroke.points {
        min_x = min_x.min(q.x);
        min_y = min_y.min(q.y);
        max_x = max_x.max(q.x);
        max_y = max_y.max(q.y);
    }
    if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
        return Err(bad(cmd, "point coordinates must be finite"));
    }
    let w = (max_x - min_x + pad * 2.0).ceil().max(1.0);
    let h = (max_y - min_y + pad * 2.0).ceil().max(1.0);
    if !w.is_finite() || !h.is_finite() || w > 1.0e9 || h > 1.0e9 {
        return Err(bad(cmd, format!("stroke too large (at most {MAX_STROKE_PIXELS} px)")));
    }
    let area = (w as u64).saturating_mul(h as u64);
    if area > MAX_STROKE_PIXELS {
        return Err(bad(cmd, format!("stroke too large ({w:.0}×{h:.0} px, at most {MAX_STROKE_PIXELS})")));
    }
    Ok(())
}

/// The stroke's coverage, with the **rendered** extent admitted before the dense buffer exists.
///
/// `reject_huge_stroke` estimates from the input points; this is the real bound. Rendering is
/// tiled, so it holds only the tiles the dabs touch; the extent is read from those tiles once
/// every dab is placed (tips, dynamics and scattering included) and checked against `max`, and
/// only a stroke that fits is made dense. `max` is a parameter so tests can inject a small budget.
fn coverage_within(cmd: &str, stroke: &Stroke, max: u64) -> Result<(Rect, Vec<f32>)> {
    photocraft_paint::retouch::stroke_coverage_within(stroke, max).ok_or_else(|| bad(cmd, format!("stroke too large (at most {max} px)")))
}

/// [`coverage_within`] at the shared allowance.
fn coverage(cmd: &str, stroke: &Stroke) -> Result<(Rect, Vec<f32>)> {
    coverage_within(cmd, stroke, MAX_STROKE_PIXELS)
}

/// A pixel layer, a layer with a mask, or a targeted alpha channel / Quick Mask (as the Brush).
fn has_pixel_layer(s: &Session) -> std::result::Result<(), String> {
    if crate::channel_cmds::edits_channel(s) {
        return Ok(());
    }
    let d = s.active().ok_or("no document open")?;
    let l = d.active_layer.and_then(|id| d.doc.layer(id)).ok_or("no active layer")?;
    if matches!(l.content, LayerContent::Raster(_)) || l.mask.is_some() {
        Ok(())
    } else {
        Err(format!("active layer is {} {} layer, not a pixel layer", l.content.article(), l.content.kind_name()))
    }
}

/// The stroke paints layer pixels (not a mask or channel), so other layers can be sampled.
fn targets_pixels(p: &Value) -> bool {
    crate::channel_cmds::target_of(p) == crate::channel_cmds::Target::Pixels
}

fn num(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}
fn flag(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn string<'a>(p: &'a Value, k: &str, d: &'a str) -> &'a str {
    p.get(k).and_then(Value::as_str).unwrap_or(d)
}
fn point(p: &Value, k: &str) -> Option<(f64, f64)> {
    let a = p.get(k)?.as_array()?;
    Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
}

/// The brush a retouching stroke paints with: the tool's brush (`base`, its tip, hardness and
/// shape dynamics) with the shared params of `p` applied. UI previews of a stroke (the Spot Healing
/// Brush's trail) render their coverage with it, so they match what the command changes.
pub fn stroke_brush(base: &BrushSettings, p: &Value) -> BrushSettings {
    let pct = |k: &str, d: f32, lo: f32, hi: f32| num(p, k, d).clamp(lo, hi) / 100.0;
    BrushSettings {
        size: num(p, "size", base.size).max(1.0),
        hardness: pct("hardness", base.hardness * 100.0, 0.0, 100.0),
        opacity: pct("opacity", 100.0, 1.0, 100.0),
        flow: pct("flow", 100.0, 1.0, 100.0),
        // Brush Settings › Brush Tip Shape › Spacing, as the Brush uses it (#2140).
        spacing: pct("spacing", base.spacing * 100.0, 1.0, 1000.0),
        erase: false,
        ..base.clone()
    }
}

/// Parse the shared brush params into a stroke on a layer (`None` when an alpha channel or the
/// Quick Mask is targeted).
fn parse_brush(s: &Session, p: &Value, cmd: &str) -> Result<(Stroke, Option<LayerId>)> {
    let pts: Vec<StrokePoint> = p
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| bad(cmd, "missing `points`"))?
        .iter()
        .filter_map(|v| {
            let a = v.as_array()?;
            Some(StrokePoint::new(a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2).and_then(Value::as_f64).unwrap_or(1.0) as f32))
        })
        .collect();
    if pts.is_empty() {
        return Err(bad(cmd, "`points` is empty"));
    }
    crate::brush_cmds::check_coords(&pts, cmd)?;
    let brush = stroke_brush(&s.tools.brush, p);
    crate::brush_cmds::validate_brush(&brush, cmd)?;
    if crate::channel_cmds::is_channel_target(p) {
        return Ok((Stroke { brush, points: pts }, None));
    }
    let id = match p.get("layer").and_then(Value::as_u64) {
        Some(id) => LayerId(id),
        None => s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into()))?,
    };
    Ok((Stroke { brush, points: pts }, Some(id)))
}

/// Run `f` on the targeted surface (layer pixels, layer mask, alpha channel or Quick Mask: the
/// Brush's paint target) inside one undoable step. `f` gets the pre-stroke document (a cheap
/// copy-on-write snapshot), the surface, the selection and the transparency lock.
fn run_stroke(
    s: &mut Session,
    label: &str,
    id: Option<LayerId>,
    p: &Value,
    f: impl FnOnce(&Document, &mut Surface, Option<&Surface>, bool) -> Result<Rect>,
) -> Result<Rect> {
    let dmg = s.edit(label, |doc, _| {
        let pre = doc.clone();
        let (surf, lock) = crate::channel_cmds::target_surface(doc, id, p)?;
        // No prune: retouching only rewrites pixels inside the stroke, and scanning every tile of a
        // large layer would dominate the cost of a dab.
        f(&pre, surf, pre.selection.as_ref(), lock)
    })?;
    s.hint_damage(dmg);
    Ok(dmg)
}

fn damage_json(r: Rect) -> Value {
    json!([r.x0, r.y0, r.width(), r.height()])
}

// ---------------------------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------------------------

/// The brush every retouching command shares, plus the tool's own params.
macro_rules! brush_params {
    ($extra:literal) => {
        concat!("{", r#""points":[[x,y,pressure?],…],"size":1..5000 px=tool size,"hardness":0..100=tool hardness,"opacity":1..100=100,"flow":1..100=100,"spacing":1..1000 (% of size)=Brush Settings spacing,"layer":id?=active,"target":"pixels"|"mask"|"filterMask"|"quickMask"|{"channel":i}=Channels panel target"#, $extra, "}")
    };
}

mod clone;
mod dab;
mod heal;
mod sampling;

pub use clone::{LiveRetouch, clone_preview, clone_preview_periodic};
use clone::{blend_param, clamp_samples, clone_stamp};
pub(crate) use dab::DabEffect;
#[cfg(test)]
use dab::DabTool;
pub use dab::LiveDab;
use dab::dab_cmd;
#[cfg(test)]
use heal::proximity_source_patch;
use heal::{heal_region, healing_brush, history_brush, history_source, spot_healing};
use sampling::sample;
pub(crate) use sampling::{SampleLayers, composite_region};

/// Retouching command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "paint.cloneStamp",
            label: "Clone Stamp",
            menu: &[],
            shortcut: None,
            params: brush_params!(
                r#","source":[sx,sy] (sampled under the first point) | "offset":[dx,dy],"aligned":bool=true,"sampleLayer":"current|currentAndBelow|all"="current","mode":"normal|multiply|…"="normal","wrap":bool=false (repeat source and stroke at document edges for Pattern Preview) → {"damage","offset","aligned","nextSource"}"#
            ),
            enabled: has_pixel_layer,
            run: clone_stamp,
            journal: true,
        },
        CommandSpec {
            id: "paint.patternStamp",
            label: "Pattern Stamp",
            menu: &[],
            shortcut: None,
            params: brush_params!(
                r#","pattern":id|name?=Patterns panel current,"scale":1..1000 %=100,"angle":deg=0,"aligned":bool=true,"impressionist":bool=false,"mode":"normal|multiply|…"="normal","phase":[px,py]? → {"damage","phase","aligned","pattern"}"#
            ),
            enabled: pattern_stamp::enabled,
            run: pattern_stamp::run,
            journal: true,
        },
        CommandSpec {
            id: "paint.healingBrush",
            label: "Healing Brush",
            menu: &[],
            shortcut: None,
            params: brush_params!(
                r#","source":[sx,sy] | "offset":[dx,dy],"aligned":bool=true,"sampleLayer":"current|currentAndBelow|all"="current","mode":"normal|…"="normal" → {"damage","offset","aligned","nextSource"}"#
            ),
            enabled: has_pixel_layer,
            run: healing_brush,
            journal: true,
        },
        CommandSpec {
            id: "paint.spotHealing",
            label: "Spot Healing Brush",
            menu: &[],
            shortcut: None,
            params: brush_params!(r#","type":"contentAware|createTexture|proximityMatch"="contentAware","sampleAllLayers":bool=false"#),
            enabled: has_pixel_layer,
            run: spot_healing,
            journal: true,
        },
        CommandSpec {
            id: "paint.patch",
            label: "Patch",
            menu: &[],
            shortcut: None,
            params: r#"{"offset":[dx,dy] (how far the selection was dragged),"mode":"source|destination"="source","contentAware":bool=false (fill the selection content-aware from the dragged-to place; a background job),"structure":1..7=4 (content-aware: 3..7 copy the middle exactly, 1..2 re-synthesise more of it),"color":0..10=0 (content-aware: how far the copy's level adapts to the selection's surroundings),"sampleAllLayers":bool=false (content-aware),"layer":id?=active,"target":"pixels"|"mask"|"filterMask"|"quickMask"|{"channel":i}=Channels panel target} → {"damage","offset"}"#,
            enabled: patch::enabled,
            run: patch::patch,
            journal: true,
        },
        CommandSpec {
            id: "paint.contentAwareMove",
            label: "Content-Aware Move",
            menu: &[],
            shortcut: None,
            params: r#"{"offset":[dx,dy] (how far the selection was dragged),"mode":"move|extend"="move","structure":1..7=4 (3..7 copy the content exactly but for a band about a patch wide along its edge; 2 widens the band, 1 re-synthesises the content too),"color":0..10=0 (0 keeps the content's level; higher values bring it towards its new place, by up to about 0.04 × color),"sampleAllLayers":bool=false,"scaleX":%=100,"scaleY":%=100 (Transform On Drop: 1..10000, negative flips),"rotate":deg=0 (Transform On Drop, clockwise; scale and rotation are about the selection's centre, before the offset; a transformed patch may reach past the canvas edge),"layer":id?=active,"target":"pixels"|"mask"|"filterMask"|"quickMask"|{"channel":i}=Channels panel target} → {"damage","offset","mode","transformed"?} (a background job; the selection moves, and is transformed, with the content)"#,
            enabled: content_aware_move::enabled,
            run: content_aware_move::content_aware_move,
            journal: true,
        },
        CommandSpec {
            id: "paint.remove",
            label: "Remove",
            menu: &[],
            shortcut: None,
            params: brush_params!(
                r#","strokes":[[[x,y,pressure?],…]|{"points":[…],"size"?,"hardness"?,"spacing"?},…]? (several strokes removed together, as one job and one undo step, each with its own size, hardness and spacing when given; else "points"),"closeLoops":bool=true (a stroke around an object removes what it encloses),"sampleAllLayers":bool=false,"newLayer":bool=false (the fill goes to a new layer above the target, which stays as it was) → {"damage","layer"?} (a background job)"#
            ),
            enabled: has_pixel_layer,
            run: remove::remove,
            journal: true,
        },
        CommandSpec {
            id: "paint.dodge",
            label: "Dodge",
            menu: &[],
            shortcut: None,
            params: brush_params!(r#","range":"shadows|midtones|highlights"="midtones","exposure":1..100=50,"protectTones":bool=true"#),
            enabled: has_pixel_layer,
            run: |s, p| dab_cmd(s, p, "paint.dodge"),
            journal: true,
        },
        CommandSpec {
            id: "paint.burn",
            label: "Burn",
            menu: &[],
            shortcut: None,
            params: brush_params!(r#","range":"shadows|midtones|highlights"="midtones","exposure":1..100=50,"protectTones":bool=true"#),
            enabled: has_pixel_layer,
            run: |s, p| dab_cmd(s, p, "paint.burn"),
            journal: true,
        },
        CommandSpec {
            id: "paint.sponge",
            label: "Sponge",
            menu: &[],
            shortcut: None,
            params: brush_params!(r#","mode":"desaturate|saturate"="desaturate","vibrance":bool=true (flow = sponge strength)"#),
            enabled: has_pixel_layer,
            run: |s, p| dab_cmd(s, p, "paint.sponge"),
            journal: true,
        },
        CommandSpec {
            id: "paint.blur",
            label: "Blur",
            menu: &[],
            shortcut: None,
            params: brush_params!(r#","strength":1..100=50,"sampleAllLayers":bool=false"#),
            enabled: has_pixel_layer,
            run: |s, p| dab_cmd(s, p, "paint.blur"),
            journal: true,
        },
        CommandSpec {
            id: "paint.sharpen",
            label: "Sharpen",
            menu: &[],
            shortcut: None,
            params: brush_params!(r#","strength":1..100=50,"protectDetail":bool=true,"sampleAllLayers":bool=false"#),
            enabled: has_pixel_layer,
            run: |s, p| dab_cmd(s, p, "paint.sharpen"),
            journal: true,
        },
        CommandSpec {
            id: "paint.smudge",
            label: "Smudge",
            menu: &[],
            shortcut: None,
            params: brush_params!(r#","strength":1..100=50,"fingerPainting":bool=false (starts with the foreground colour),"sampleAllLayers":bool=false"#),
            enabled: has_pixel_layer,
            run: |s, p| dab_cmd(s, p, "paint.smudge"),
            journal: true,
        },
        CommandSpec {
            id: "paint.historyBrush",
            label: "History Brush",
            menu: &[],
            shortcut: None,
            params: brush_params!(
                r#","state":row (a History panel row, as history.setSource: 0 = oldest state, then the current one, then the greyed redo states),"snapshot":id (a history.newSnapshot id; default: the History panel's source, else the oldest held state; a source of another canvas size is an error)"#
            ),
            enabled: has_pixel_layer,
            run: history_brush,
            journal: true,
        },
        CommandSpec {
            id: "paint.artHistoryBrush",
            label: "Art History Brush",
            menu: &[],
            shortcut: None,
            params: brush_params!(
                r#","state":row (a History panel row, as history.setSource),"snapshot":id (a history.newSnapshot id; default: the History panel's source, as paint.historyBrush; a source of another canvas size is an error),"style":"tightShort|tightMedium|tightLong|looseMedium|looseLong|dab|tightCurl|looseCurl"="tightMedium","area":0..100=50,"tolerance":0..100=0"#
            ),
            enabled: has_pixel_layer,
            run: art_history::run,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests;
