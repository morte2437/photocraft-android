//! Photoshop-style Layer Style dialog: one row per effect instance on the left,
//! parameters on the right. State lives in the dialog's `fields` (JSON), so
//! automation can drive it like any other dialog:
//!
//! - `layer`: layer id; `globalLight`: the document's light angle (degrees)
//! - `selected`: `"blendingOptions"` or an instance id from `effects`
//! - `effects`: `[{id, kind, on, params, fx?}]` in the layer's effect order.
//!   `fx` is the effect snapshot the dialog loaded: the apply edits that
//!   snapshot with `params` (engine-side overlay), so everything the dialog
//!   doesn't model — gradient strokes, imported contours, PSD data — survives.
//!   An entry without `fx` is a new instance built from `params`.
//! - `p:blendingOptions`: the parameters of `layer.layerStyle.blendingOptions` (blend mode,
//!   opacity, fill opacity, channels, knockout, the Advanced Blending switches, Blend If) plus
//!   the page-only `blendIfChannel`; `channelNames`: the document's colour channels.

use egui::{Color32, RichText, Sense, Stroke, StrokeKind, vec2};
use photocraft_doc::effects::{FxPaint, GlowSource, GlowTechnique, StrokePosition};
use photocraft_doc::{Effect, Layer};
use serde_json::{Map, Value, json};

use crate::theme::Tokens;
use crate::{PhotocraftApp, widgets};

#[cfg(test)]
mod bevel_modes_tests;
pub(crate) mod color_picker;
mod contour;
#[cfg(test)]
mod gradient_method_tests;

#[derive(Clone, Copy)]
enum P {
    Slider(f32, f32, &'static str),
    Color,
    Blend,
    Choice(&'static [(&'static str, &'static str)]),
    GradientMethod,
    Check,
    Angle,
    /// A pattern from the dialog's `patternList` field (`[[id, name], …]`).
    Pattern,
    /// A contour (thumbnail, preset picker, Contour Editor), with the key of its Anti-aliased
    /// checkbox when the effect has one.
    Contour(Option<&'static str>),
}

/// Effect kinds in Photoshop's list order: (command kind, label, params).
/// The Styles preset gallery page id.
pub const STYLES_PAGE: &str = "styles";
/// The Blending Options page id (not an effect).
pub const BLENDING: &str = "blendingOptions";

pub const KINDS: &[(&str, &str)] = &[
    ("bevelEmboss", "Bevel & Emboss"),
    ("stroke", "Stroke"),
    ("innerShadow", "Inner Shadow"),
    ("innerGlow", "Inner Glow"),
    ("satin", "Satin"),
    ("colorOverlay", "Color Overlay"),
    ("gradientOverlay", "Gradient Overlay"),
    ("patternOverlay", "Pattern Overlay"),
    ("outerGlow", "Outer Glow"),
    ("dropShadow", "Drop Shadow"),
];

/// Effect kinds Photoshop offers several instances of (stored as `…Multi` lists in the PSD).
const MULTI: &[&str] = &["stroke", "dropShadow", "innerShadow", "colorOverlay", "gradientOverlay"];

fn multi(kind: &str) -> bool {
    MULTI.contains(&kind)
}

fn kind_of(e: &Effect) -> &'static str {
    photocraft_engine::layer_style::kind_of(e)
}

fn defaults(kind: &str) -> Value {
    photocraft_engine::layer_style::effect_defaults(kind)
}

/// Factory params for a new instance, with the shared light angle filled in.
fn fresh_params(kind: &str, light: f32) -> Value {
    let mut p = defaults(kind);
    if p.get("useGlobalLight").and_then(Value::as_bool) == Some(true) {
        p["angle"] = json!(light);
    }
    p
}

fn spec(kind: &str) -> &'static [(&'static str, &'static str, P)] {
    const POS: &[(&str, &str)] = &[("outside", "Outside"), ("inside", "Inside"), ("center", "Center")];
    const GSTYLE: &[(&str, &str)] = &[("linear", "Linear"), ("radial", "Radial"), ("angle", "Angle"), ("reflected", "Reflected"), ("diamond", "Diamond")];
    const BSTYLE: &[(&str, &str)] = &[("inner", "Inner Bevel"), ("outer", "Outer Bevel"), ("emboss", "Emboss"), ("pillow", "Pillow Emboss")];
    const TECHNIQUE: &[(&str, &str)] = &[("smooth", "Smooth"), ("chiselHard", "Chisel Hard"), ("chiselSoft", "Chisel Soft")];
    const DIR: &[(&str, &str)] = &[("up", "Up"), ("down", "Down")];
    const SRC: &[(&str, &str)] = &[("edge", "Edge"), ("center", "Center")];
    const GLOW_TECHNIQUE: &[(&str, &str)] = &[("softer", "Softer"), ("precise", "Precise")];
    const CONTOUR: P = P::Contour(Some("antiAlias"));
    match kind {
        "dropShadow" => &[
            ("blend", "Blend Mode", P::Blend),
            ("color", "Color", P::Color),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("angle", "Angle", P::Angle),
            ("useGlobalLight", "Use Global Light", P::Check),
            ("distance", "Distance", P::Slider(0.0, 300.0, "px")),
            ("spread", "Spread", P::Slider(0.0, 100.0, "%")),
            ("size", "Size", P::Slider(0.0, 250.0, "px")),
            ("contour", "Contour", CONTOUR),
            ("noise", "Noise", P::Slider(0.0, 100.0, "%")),
            ("knocksOut", "Layer Knocks Out Drop Shadow", P::Check),
        ],
        "innerShadow" => &[
            ("blend", "Blend Mode", P::Blend),
            ("color", "Color", P::Color),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("angle", "Angle", P::Angle),
            ("useGlobalLight", "Use Global Light", P::Check),
            ("distance", "Distance", P::Slider(0.0, 300.0, "px")),
            ("choke", "Choke", P::Slider(0.0, 100.0, "%")),
            ("size", "Size", P::Slider(0.0, 250.0, "px")),
            ("contour", "Contour", CONTOUR),
            ("noise", "Noise", P::Slider(0.0, 100.0, "%")),
        ],
        // Photoshop's order (#6698): Technique before the spread or choke, Range after the contour.
        "outerGlow" => &[
            ("blend", "Blend Mode", P::Blend),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("color", "Color", P::Color),
            ("technique", "Technique", P::Choice(GLOW_TECHNIQUE)),
            ("spread", "Spread", P::Slider(0.0, 100.0, "%")),
            ("size", "Size", P::Slider(0.0, 250.0, "px")),
            ("contour", "Contour", CONTOUR),
            ("range", "Range", P::Slider(1.0, 100.0, "%")),
            ("noise", "Noise", P::Slider(0.0, 100.0, "%")),
        ],
        "innerGlow" => &[
            ("blend", "Blend Mode", P::Blend),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("color", "Color", P::Color),
            ("technique", "Technique", P::Choice(GLOW_TECHNIQUE)),
            ("source", "Source", P::Choice(SRC)),
            ("choke", "Choke", P::Slider(0.0, 100.0, "%")),
            ("size", "Size", P::Slider(0.0, 250.0, "px")),
            ("contour", "Contour", CONTOUR),
            ("range", "Range", P::Slider(1.0, 100.0, "%")),
            ("noise", "Noise", P::Slider(0.0, 100.0, "%")),
        ],
        "stroke" => &[
            ("size", "Size", P::Slider(1.0, 250.0, "px")),
            ("position", "Position", P::Choice(POS)),
            ("blend", "Blend Mode", P::Blend),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("overprint", "Overprint", P::Check),
            ("color", "Color", P::Color),
        ],
        "colorOverlay" => &[("blend", "Blend Mode", P::Blend), ("color", "Color", P::Color), ("opacity", "Opacity", P::Slider(0.0, 100.0, "%"))],
        "gradientOverlay" => &[
            ("blend", "Blend Mode", P::Blend),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("from", "From", P::Color),
            ("to", "To", P::Color),
            ("reverse", "Reverse", P::Check),
            ("style", "Style", P::Choice(GSTYLE)),
            ("angle", "Angle", P::Angle),
            ("scale", "Scale", P::Slider(10.0, 150.0, "%")),
            ("method", "Method", P::GradientMethod),
        ],
        "patternOverlay" => &[
            ("blend", "Blend Mode", P::Blend),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("pattern", "Pattern", P::Pattern),
            ("angle", "Angle", P::Angle),
            ("scale", "Scale", P::Slider(1.0, 1000.0, "%")),
            ("link", "Link with Layer", P::Check),
        ],
        "bevelEmboss" => &[
            ("style", "Style", P::Choice(BSTYLE)),
            ("technique", "Technique", P::Choice(TECHNIQUE)),
            ("depth", "Depth", P::Slider(1.0, 1000.0, "%")),
            ("direction", "Direction", P::Choice(DIR)),
            ("size", "Size", P::Slider(0.0, 250.0, "px")),
            ("soften", "Soften", P::Slider(0.0, 16.0, "px")),
            ("angle", "Angle", P::Angle),
            ("useGlobalLight", "Use Global Light", P::Check),
            ("altitude", "Altitude", P::Slider(0.0, 90.0, "°")),
            ("glossContour", "Gloss Contour", P::Contour(Some("glossAntiAlias"))),
            ("highlightBlend", "Highlight Mode", P::Blend),
            ("highlightColor", "Highlight Color", P::Color),
            ("highlightOpacity", "Highlight Opacity", P::Slider(0.0, 100.0, "%")),
            ("shadowBlend", "Shadow Mode", P::Blend),
            ("shadowColor", "Shadow Color", P::Color),
            ("shadowOpacity", "Shadow Opacity", P::Slider(0.0, 100.0, "%")),
            // Bevel & Emboss › Contour element.
            ("contour", "Contour", P::Check),
            ("contourName", "", P::Contour(Some("contourAntiAlias"))),
            ("contourRange", "Range", P::Slider(1.0, 100.0, "%")),
        ],
        "satin" => &[
            ("blend", "Blend Mode", P::Blend),
            ("color", "Color", P::Color),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("angle", "Angle", P::Angle),
            ("distance", "Distance", P::Slider(0.0, 250.0, "px")),
            ("size", "Size", P::Slider(0.0, 250.0, "px")),
            ("contour", "Contour", CONTOUR),
            ("invert", "Invert", P::Check),
        ],
        BLENDING => &[
            ("blend", "Blend Mode", P::Blend),
            ("opacity", "Opacity", P::Slider(0.0, 100.0, "%")),
            ("fillOpacity", "Fill Opacity", P::Slider(0.0, 100.0, "%")),
        ],
        _ => &[],
    }
}

fn hex(c: &photocraft_doc::Color) -> String {
    let [r, g, b, _] = c.to_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Current values of an existing effect, in the command's parameter units —
/// only the keys this dialog models. Everything else (gradient strokes, multi-stop
/// gradients, imported contours) is left out so the apply overlay can't clobber it.
/// `light` is the document's global light angle: an effect that uses it shows that angle.
fn values_of(e: &Effect, light: f32) -> Value {
    let mut v = defaults(kind_of(e));
    let set = |v: &mut Value, k: &str, x: Value| {
        v[k] = x;
    };
    let drop_keys = |v: &mut Value, keys: &[&str]| {
        if let Some(o) = v.as_object_mut() {
            for k in keys {
                o.remove(*k);
            }
        }
    };
    match e {
        Effect::DropShadow(s) | Effect::InnerShadow(s) => {
            set(&mut v, "blend", json!(s.common.blend.label()));
            set(&mut v, "opacity", json!((s.common.opacity * 100.0).round()));
            set(&mut v, "color", json!(hex(&s.color)));
            set(&mut v, "angle", json!(if s.use_global_light { light } else { s.angle }));
            set(&mut v, "useGlobalLight", json!(s.use_global_light));
            set(&mut v, "distance", json!(s.distance));
            set(&mut v, if matches!(e, Effect::DropShadow(_)) { "spread" } else { "choke" }, json!((s.spread * 100.0).round()));
            set(&mut v, "size", json!(s.size));
            set(&mut v, "contour", photocraft_engine::layer_style::contour_param(&s.contour));
            set(&mut v, "antiAlias", json!(s.anti_alias));
            set(&mut v, "noise", json!((s.noise * 100.0).round()));
            if matches!(e, Effect::DropShadow(_)) {
                set(&mut v, "knocksOut", json!(s.knocks_out));
            }
        }
        Effect::OuterGlow(g) | Effect::InnerGlow(g) => {
            set(&mut v, "blend", json!(g.common.blend.label()));
            set(&mut v, "opacity", json!((g.common.opacity * 100.0).round()));
            if let FxPaint::Color(c) = &g.paint {
                set(&mut v, "color", json!(hex(c)));
            } else {
                drop_keys(&mut v, &["color"]);
            }
            set(&mut v, "technique", json!(if g.technique == GlowTechnique::Precise { "precise" } else { "softer" }));
            set(&mut v, if matches!(e, Effect::OuterGlow(_)) { "spread" } else { "choke" }, json!((g.spread * 100.0).round()));
            set(&mut v, "size", json!(g.size));
            set(&mut v, "range", json!((g.range * 100.0).round()));
            set(&mut v, "contour", photocraft_engine::layer_style::contour_param(&g.contour));
            set(&mut v, "antiAlias", json!(g.anti_alias));
            set(&mut v, "noise", json!((g.noise * 100.0).round()));
            if matches!(e, Effect::InnerGlow(_)) {
                set(&mut v, "source", json!(if g.source == GlowSource::Center { "center" } else { "edge" }));
            }
        }
        Effect::Stroke(s) => {
            set(&mut v, "blend", json!(s.common.blend.label()));
            set(&mut v, "opacity", json!((s.common.opacity * 100.0).round()));
            set(&mut v, "size", json!(s.size));
            set(
                &mut v,
                "position",
                json!(match s.position {
                    StrokePosition::Inside => "inside",
                    StrokePosition::Center => "center",
                    _ => "outside",
                }),
            );
            set(&mut v, "overprint", json!(s.overprint));
            if let FxPaint::Color(c) = &s.paint {
                set(&mut v, "color", json!(hex(c)));
            } else {
                // A gradient/pattern stroke: the dialog's colour field doesn't model
                // the paint, so the key is left out and the snapshot carries it.
                drop_keys(&mut v, &["color"]);
            }
        }
        Effect::ColorOverlay { common, color } => {
            set(&mut v, "blend", json!(common.blend.label()));
            set(&mut v, "opacity", json!((common.opacity * 100.0).round()));
            set(&mut v, "color", json!(hex(color)));
        }
        Effect::GradientOverlay { common, gradient: g, interpolation, .. } => {
            set(&mut v, "blend", json!(common.blend.label()));
            set(&mut v, "opacity", json!((common.opacity * 100.0).round()));
            let stops = interpolation.as_ref().map_or(&g.stops, |source| &source.stops);
            if let Some(source) = interpolation {
                set(&mut v, "method", json!(source.method));
                set(&mut v, "smoothness", json!(source.smoothness));
            }
            // Endpoint colours and geometry always load into the dialog. Recorded
            // interpolation exposes the original source ends; a baked curve without a
            // record (PSD import) keeps its interior stops in `fx`, and an edit only
            // retints its ends until a method is chosen.
            if let (Some(first), Some(last)) = (stops.first(), stops.last()) {
                set(&mut v, "from", json!(hex(&first.1)));
                set(&mut v, "to", json!(hex(&last.1)));
            }
            set(&mut v, "reverse", json!(g.reverse));
            set(
                &mut v,
                "style",
                json!(match g.style {
                    photocraft_doc::effects::GradientStyle::Radial => "radial",
                    photocraft_doc::effects::GradientStyle::Angle => "angle",
                    photocraft_doc::effects::GradientStyle::Reflected => "reflected",
                    photocraft_doc::effects::GradientStyle::Diamond => "diamond",
                    photocraft_doc::effects::GradientStyle::Linear => "linear",
                }),
            );
            set(&mut v, "angle", json!(g.angle));
            set(&mut v, "scale", json!((g.scale * 100.0).round()));
        }
        Effect::PatternOverlay { common, name, id, scale, angle, link, phase } => {
            set(&mut v, "blend", json!(common.blend.label()));
            set(&mut v, "opacity", json!((common.opacity * 100.0).round()));
            set(&mut v, "pattern", json!(if id.is_empty() { name } else { id }));
            set(&mut v, "scale", json!((scale * 100.0).round()));
            set(&mut v, "angle", json!(angle));
            set(&mut v, "link", json!(link));
            set(&mut v, "phaseX", json!(phase.0));
            set(&mut v, "phaseY", json!(phase.1));
        }
        Effect::Satin(s) => {
            set(&mut v, "blend", json!(s.common.blend.label()));
            set(&mut v, "opacity", json!((s.common.opacity * 100.0).round()));
            set(&mut v, "color", json!(hex(&s.color)));
            set(&mut v, "angle", json!(s.angle));
            set(&mut v, "distance", json!(s.distance));
            set(&mut v, "size", json!(s.size));
            set(&mut v, "invert", json!(s.invert));
            set(&mut v, "contour", photocraft_engine::layer_style::contour_param(&s.contour));
            set(&mut v, "antiAlias", json!(s.anti_alias));
        }
        Effect::BevelEmboss(b) => {
            set(
                &mut v,
                "style",
                json!(match b.style {
                    photocraft_doc::effects::BevelStyle::OuterBevel => "outer",
                    photocraft_doc::effects::BevelStyle::Emboss => "emboss",
                    photocraft_doc::effects::BevelStyle::PillowEmboss => "pillow",
                    photocraft_doc::effects::BevelStyle::StrokeEmboss => "stroke",
                    photocraft_doc::effects::BevelStyle::InnerBevel => "inner",
                }),
            );
            set(
                &mut v,
                "technique",
                json!(match b.technique {
                    photocraft_doc::effects::BevelTechnique::ChiselHard => "chiselHard",
                    photocraft_doc::effects::BevelTechnique::ChiselSoft => "chiselSoft",
                    photocraft_doc::effects::BevelTechnique::Smooth => "smooth",
                }),
            );
            set(&mut v, "depth", json!((b.depth * 100.0).round()));
            set(&mut v, "size", json!(b.size));
            set(&mut v, "soften", json!(b.soften));
            set(&mut v, "angle", json!(if b.use_global_light { light } else { b.angle }));
            set(&mut v, "useGlobalLight", json!(b.use_global_light));
            set(&mut v, "altitude", json!(b.altitude));
            set(&mut v, "direction", json!(if b.up { "up" } else { "down" }));
            set(&mut v, "glossContour", photocraft_engine::layer_style::contour_param(&b.gloss_contour));
            set(&mut v, "glossAntiAlias", json!(b.gloss_anti_alias));
            set(&mut v, "highlightBlend", json!(b.highlight.blend.label()));
            set(&mut v, "highlightColor", json!(hex(&b.highlight_color)));
            set(&mut v, "highlightOpacity", json!((b.highlight.opacity * 100.0).round()));
            set(&mut v, "shadowBlend", json!(b.shadow.blend.label()));
            set(&mut v, "shadowColor", json!(hex(&b.shadow_color)));
            set(&mut v, "shadowOpacity", json!((b.shadow.opacity * 100.0).round()));
            set(&mut v, "contour", json!(b.contour.is_some()));
            if let Some(c) = &b.contour {
                set(&mut v, "contourName", photocraft_engine::layer_style::contour_param(&c.contour));
                set(&mut v, "contourRange", json!((c.range * 100.0).round()));
                set(&mut v, "contourAntiAlias", json!(c.anti_alias));
            }
        }
    }
    v
}

/// The dialog's effect entries (JSON array of `{id, kind, on, params, fx?}`).
fn effects_of(f: &Map<String, Value>) -> &[Value] {
    f.get("effects").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn entry<'a>(f: &'a Map<String, Value>, id: &str) -> Option<&'a Value> {
    effects_of(f).iter().find(|e| e.get("id").and_then(Value::as_str) == Some(id))
}

fn entry_mut<'a>(f: &'a mut Map<String, Value>, id: &str) -> Option<&'a mut Value> {
    f.get_mut("effects").and_then(Value::as_array_mut).and_then(|a| a.iter_mut().find(|e| e.get("id").and_then(Value::as_str) == Some(id)))
}

/// Stores one edited parameter of an instance (or of the blending page).
fn set_param(f: &mut Map<String, Value>, id: &str, key: &str, value: Value) {
    if id == BLENDING || id == STYLES_PAGE {
        if let Some(o) = f.get_mut("p:blendingOptions").and_then(Value::as_object_mut) {
            o.insert(key.into(), value);
        }
        return;
    }
    // All globally lit instances share one angle, including disabled instances.
    // Keep the opening angle intact for automation's changed-value fallback in apply().
    let shared = entry(f, id).and_then(|e| e.get("params")).is_some_and(|p| {
        (key == "angle" && p.get("useGlobalLight").and_then(Value::as_bool) == Some(true)) || (key == "useGlobalLight" && value.as_bool() == Some(true))
    });
    let shared_angle =
        if shared { if key == "angle" { Some(value.clone()) } else { f.get("pendingGlobalLight").or_else(|| f.get("globalLight")).cloned() } } else { None };
    if let Some(e) = entry_mut(f, id)
        && let Some(o) = e.get_mut("params").and_then(Value::as_object_mut)
    {
        o.insert(key.into(), value);
    }
    if let Some(angle) = shared_angle {
        f.insert("pendingGlobalLight".into(), angle.clone());
        if let Some(effects) = f.get_mut("effects").and_then(Value::as_array_mut) {
            for e in effects {
                if let Some(p) = e.get_mut("params").and_then(Value::as_object_mut)
                    && p.get("useGlobalLight").and_then(Value::as_bool) == Some(true)
                {
                    p.insert("angle".into(), angle.clone());
                }
            }
        }
    }
}

/// A fresh instance id: one past the highest numbered `fxN` in the list.
fn next_id(f: &Map<String, Value>) -> String {
    let used: std::collections::HashSet<u64> = effects_of(f)
        .iter()
        .filter_map(|e| e.get("id").and_then(Value::as_str))
        .filter_map(|id| id.strip_prefix("fx"))
        .filter_map(|n| n.parse::<u64>().ok())
        .collect();
    let max = used.iter().copied().max().unwrap_or(0);
    let next = max.checked_add(1).or_else(|| (1_u64..).find(|n| !used.contains(n))).unwrap_or(1);
    format!("fx{next}")
}

/// Adds an instance of `kind`, right after the last one of its kind (the list,
/// and so the rendered stack, keeps the layer's effect order), and selects it.
fn add_instance(f: &mut Map<String, Value>, kind: &str) {
    let light = f.get("pendingGlobalLight").or_else(|| f.get("globalLight")).and_then(Value::as_f64).unwrap_or(120.0) as f32;
    let id = next_id(f);
    let new = json!({"id": id, "kind": kind, "on": true, "params": fresh_params(kind, light)});
    if let Some(a) = f.get_mut("effects").and_then(Value::as_array_mut) {
        let pos = a.iter().rposition(|e| e.get("kind").and_then(Value::as_str) == Some(kind)).map_or(a.len(), |i| i + 1);
        a.insert(pos, new);
    }
    f.insert("selected".into(), json!(id));
}

/// Removes an instance; the selection moves to a sibling of the same kind,
/// else to the blending page.
fn remove_instance(f: &mut Map<String, Value>, id: &str) {
    let kind = entry(f, id).and_then(|e| e.get("kind").and_then(Value::as_str)).map(str::to_string);
    if let Some(a) = f.get_mut("effects").and_then(Value::as_array_mut) {
        a.retain(|e| e.get("id").and_then(Value::as_str) != Some(id));
    }
    let selected = f.get("selected").and_then(Value::as_str).unwrap_or_default().to_string();
    if selected == id {
        let sibling = kind
            .as_deref()
            .and_then(|kind| effects_of(f).iter().find(|e| e.get("kind").and_then(Value::as_str) == Some(kind)))
            .and_then(|e| e.get("id").and_then(Value::as_str))
            .map(str::to_string);
        f.insert("selected".into(), json!(sibling.unwrap_or_else(|| STYLES_PAGE.into())));
    }
}

/// Dialog fields for a layer: one entry per effect instance (all of them, in
/// the layer's order), the blending options and the selected page. `select`
/// names a kind to open: its first instance is selected (and enabled), or one
/// is created. `light` is the document's global light angle.
/// Whether the selected page is a Drop Shadow or Inner Shadow, which a drag on the image moves
/// while the dialog is open, as in Photoshop (#5701; [`drag_shadow`]).
pub fn drags_shadow(f: &Map<String, Value>) -> bool {
    let selected = f.get("selected").and_then(Value::as_str).unwrap_or_default();
    entry(f, selected).and_then(|e| e.get("kind")).and_then(Value::as_str).is_some_and(|k| matches!(k, "dropShadow" | "innerShadow"))
}

/// A drag on the image with a shadow page selected: `start` is true on the press (it records the
/// shadow's angle and distance then), `total` the pointer's movement since in document pixels.
/// The shadow's offset follows the pointer, kept as Photoshop's whole-degree angle and whole-pixel
/// distance (offset = (−cos angle, sin angle) × distance, the light opposite the shadow); with
/// Use Global Light the shared angle moves too ([`set_param`]).
pub fn drag_shadow(f: &mut Map<String, Value>, start: bool, total: [f64; 2]) {
    const KEY: &str = "__shadowDrag";
    let Some(id) = f.get("selected").and_then(Value::as_str).map(str::to_string).filter(|_| drags_shadow(f)) else { return };
    let Some(p) = entry(f, &id).and_then(|e| e.get("params")) else { return };
    if start || f.get(KEY).is_none() {
        let angle = p.get("angle").and_then(Value::as_f64).unwrap_or(120.0);
        let distance = p.get("distance").and_then(Value::as_f64).unwrap_or(0.0);
        f.insert(KEY.into(), json!([angle, distance]));
    }
    let Some([angle, distance]) = f.get(KEY).and_then(Value::as_array).and_then(|a| Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])) else { return };
    let a = angle.to_radians();
    let (ox, oy) = (-a.cos() * distance + total[0], a.sin() * distance + total[1]);
    let d = ox.hypot(oy);
    if !d.is_finite() {
        return;
    }
    let new_angle = if d >= 0.5 { oy.atan2(-ox).to_degrees().round() } else { angle };
    set_param(f, &id, "angle", json!(new_angle));
    set_param(f, &id, "distance", json!(d.round().clamp(0.0, 30000.0)));
}

pub fn initial_fields(layer: &Layer, select: Option<&str>, light: f32) -> Map<String, Value> {
    let mut f = Map::new();
    f.insert("layer".into(), json!(layer.id.0));
    f.insert("globalLight".into(), json!(light));
    f.insert("preview".into(), json!(true));
    set_blending_fields(&mut f, layer, photocraft_color::ColorMode::Rgb);
    let mut effects = Vec::new();
    for (i, e) in layer.effects.items.iter().enumerate() {
        effects.push(json!({
            "id": format!("fx{}", i + 1),
            "kind": kind_of(e),
            "on": e.enabled(),
            "params": values_of(e, light),
            "fx": serde_json::to_value(e).unwrap_or(Value::Null),
        }));
    }
    f.insert("effects".into(), Value::Array(effects));
    let selected = match select {
        None => effects_of(&f).first().and_then(|e| e.get("id").and_then(Value::as_str)).map(str::to_string).unwrap_or_else(|| "dropShadow".into()),
        Some(STYLES_PAGE) => STYLES_PAGE.to_string(),
        Some(BLENDING) => BLENDING.to_string(),
        Some(kind) => {
            let existing = effects_of(&f)
                .iter()
                .find(|e| e.get("kind").and_then(Value::as_str) == Some(kind))
                .map(|e| e.get("id").and_then(Value::as_str).unwrap_or_default().to_string());
            match existing {
                Some(id) => {
                    if let Some(e) = entry_mut(&mut f, &id) {
                        e["on"] = json!(true);
                    }
                    id
                }
                None => {
                    add_instance(&mut f, kind);
                    f.get("selected").and_then(Value::as_str).unwrap_or_default().to_string()
                }
            }
        }
    };
    f.insert("selected".into(), json!(selected));
    f
}

pub fn open(app: &mut PhotocraftApp, select: Option<&str>) -> Option<u64> {
    // An open Free Transform box is applied first, as in Photoshop (#4821): the canvas shows the
    // box's preview while it is open, so the dialog's preview would not show until Enter.
    if app.ui.transform.is_some() {
        crate::transform_tool::commit(app);
        if app.ui.transform.is_some() && crate::transform_tool::committing(app).is_none() {
            return None; // refused: the box stays open, with the reason in the status bar
        }
    }
    let st = app.session.active()?;
    let layer = st.doc.layer(st.active_layer?)?.clone();
    let light = st.doc.global_light.angle;
    let mode = st.doc.mode;
    let mut f = initial_fields(&layer, select, light);
    // Channels and Blend If list the document's own colour channels.
    set_blending_fields(&mut f, &layer, mode);
    f.insert("__document".into(), json!(st.doc.id));
    f.insert("patternList".into(), pattern_list(app));
    Some(app.ui.open_dialog(crate::state::DialogKind::LayerStyle, f))
}

/// Double-click on an effect row of the Layers panel: open Layer Style for that row's layer. A
/// row of another layer selects it first, as its right-click menu does (Photoshop does the same).
pub fn open_for_layer(app: &mut PhotocraftApp, layer: photocraft_doc::LayerId, select: Option<&str>) -> Option<u64> {
    let active = app.session.active().is_some_and(|s| s.active_layer == Some(layer));
    if !active {
        app.run("layer.select", json!({"layer": layer.0})).ok()?;
    }
    open(app, select)
}

/// `[[id, name], …]` of the patterns a style can use (document's, then the library's).
pub fn pattern_list(app: &PhotocraftApp) -> Value {
    let mut out: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let doc_pats = app.session.active().map(|d| d.doc.patterns.clone()).unwrap_or_default();
    for p in doc_pats.iter().chain(app.session.patterns.items.iter()) {
        if seen.insert(p.id.clone()) {
            out.push(json!([p.id, p.display_name()]));
        }
    }
    Value::Array(out)
}

/// Apply the dialog: replace the layer's effects with the dialog's instances.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    apply(f, |id, p| app.run(id, p))
}

/// Every parameter group is edited as an object. Validate before preview or confirmation so
/// malformed values supplied through `ui.dialog.set` are reported before any command runs.
fn validate_params(f: &Map<String, Value>) -> Result<(), String> {
    let key = format!("p:{BLENDING}");
    if let Some(value) = f.get(&key)
        && !value.is_object()
    {
        return Err(format!("invalid layer style parameters: `{key}` must be an object"));
    }
    let Some(effects) = f.get("effects") else { return Ok(()) };
    let Some(effects) = effects.as_array() else {
        return Err("invalid layer style parameters: `effects` must be an array".into());
    };
    for (i, e) in effects.iter().enumerate() {
        let Some(e) = e.as_object() else {
            return Err(format!("invalid layer style parameters: `effects[{i}]` must be an object"));
        };
        if let Some(params) = e.get("params")
            && !params.is_object()
        {
            return Err(format!("invalid layer style parameters: `effects[{i}].params` must be an object"));
        }
    }
    Ok(())
}

fn effect_entry(e: &Value, kind: &str, mut params: Value, initial_light_angle: Option<f64>) -> Value {
    match e.get("fx").filter(|fx| fx.is_object()) {
        Some(fx) => {
            if kind == "bevelEmboss"
                && let Ok(original) = serde_json::from_value::<Effect>(fx.clone())
                && let Some(edits) = params.as_object_mut()
            {
                let shown = values_of(&original, initial_light_angle.unwrap_or(120.0) as f32);
                // Unedited display values must not quantize imported shading in the snapshot.
                for key in ["highlightBlend", "highlightColor", "highlightOpacity", "shadowBlend", "shadowColor", "shadowOpacity"] {
                    if edits.get(key) == shown.get(key) {
                        edits.remove(key);
                    }
                }
            }
            json!({"kind": kind, "fx": fx, "params": params})
        }
        None => {
            // A new instance: the page shows the factory defaults merged with
            // the edits, and the engine builds from the full set.
            let mut p = defaults(kind);
            if let (Some(o), Some(pv)) = (p.as_object_mut(), params.as_object()) {
                for (k, v) in pv {
                    o.insert(k.clone(), v.clone());
                }
            }
            json!({"kind": kind, "params": p})
        }
    }
}

fn find_light_angle(f: &Map<String, Value>, initial_light_angle: Option<f64>) -> Option<f64> {
    let mut light_angle: Option<f64> = None;
    for e in effects_of(f) {
        if e.get("on").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let params = e.get("params").cloned().unwrap_or_else(|| json!({}));
        if params.get("useGlobalLight").and_then(Value::as_bool) == Some(true)
            && let Some(angle) = params.get("angle").and_then(Value::as_f64)
            && initial_light_angle.is_none_or(|initial| angle != initial)
        {
            light_angle = Some(angle);
        }
    }
    light_angle
}

/// Runs the dialog's commands through `run`: blending options, then the whole
/// effect list in one `layer.layerStyle.replace` (instances keep their order;
/// their snapshots carry what the dialog doesn't model).
fn apply(f: &Map<String, Value>, mut run: impl FnMut(&str, Value) -> Result<Value, String>) -> Result<Value, String> {
    validate_params(f)?;
    let layer = f.get("layer").cloned().unwrap_or(Value::Null);
    let initial_light_angle = f.get("globalLight").and_then(Value::as_f64);
    if let Some(bo) = f.get(&format!("p:{BLENDING}")).and_then(Value::as_object) {
        let mut bo = bo.clone();
        // Which Blend If channel the page shows is dialog state, not a parameter.
        bo.remove("blendIfChannel");
        bo.insert("layer".into(), layer.clone());
        let p = Value::Object(bo);
        run("layer.layerStyle.blendingOptions", p)?;
    }
    let mut entries = Vec::new();
    for e in effects_of(f) {
        let (Some(kind), Some(_)) = (e.get("kind").and_then(Value::as_str), e.get("id").and_then(Value::as_str)) else { continue };
        let on = e.get("on").and_then(Value::as_bool).unwrap_or(true);
        let mut params = e.get("params").cloned().unwrap_or_else(|| json!({}));
        params["enabled"] = json!(on);
        entries.push(effect_entry(e, kind, params, initial_light_angle));
    }
    run("layer.layerStyle.replace", json!({"layer": layer, "effects": entries}))?;
    // An effect lit by the global light follows the document's light angle, so the Angle slider
    // drives that shared angle rather than the per-effect angle the compositor ignores.
    let light_angle = find_light_angle(f, initial_light_angle);
    if let Some(angle) = f.get("pendingGlobalLight").and_then(Value::as_f64).or(light_angle) {
        run("layer.layerStyle.globalLight", json!({"angle": angle}))?;
    }
    Ok(Value::Null)
}

/// Hash of the fields that change the rendered style (not the selected page).
pub fn preview_hash(f: &Map<String, Value>) -> u64 {
    f.iter()
        .filter(|(k, _)| matches!(k.as_str(), "layer" | "effects" | "p:blendingOptions" | "pendingGlobalLight"))
        .flat_map(|(k, v)| k.bytes().chain(v.to_string().into_bytes()))
        .fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// `doc` with the dialog's style applied, run on a scratch session (no history) for the live preview.
pub fn preview_document(
    doc: &photocraft_doc::Document,
    patterns: &photocraft_engine::pattern_cmds::PatternLibrary,
    f: &Map<String, Value>,
) -> Result<photocraft_doc::Document, String> {
    let mut s = photocraft_engine::Session::new();
    s.patterns = patterns.clone();
    s.add_document(doc.clone(), None);
    apply(f, |id, p| s.execute(id, p).map_err(|e| e.to_string()))?;
    s.active().map(|d| (*d.doc).clone()).ok_or_else(|| "no preview document".into())
}

/// Applies a style preset to the dialog fields in-place without touching the document.
/// Clicking Cancel leaves the document untouched; clicking OK commits.
fn apply_style_preset(app: &PhotocraftApp, f: &mut Map<String, Value>, name: &str) {
    let Ok(style) = photocraft_engine::presets::styles::find_style(&app.session, &json!({"preset": name}), "apply_style_preset") else { return };
    let light = f.get("pendingGlobalLight").or_else(|| f.get("globalLight")).and_then(Value::as_f64).unwrap_or(120.0) as f32;
    let mut effects = Vec::new();
    for (i, e) in style.effects.iter().enumerate() {
        effects.push(json!({
            "id": format!("fx{}", i + 1),
            "kind": kind_of(e),
            "on": e.enabled(),
            "params": values_of(e, light),
            "fx": serde_json::to_value(e).unwrap_or(Value::Null),
        }));
    }
    f.insert("effects".into(), Value::Array(effects));
    if let Some(b) = style.blend {
        set_param(f, BLENDING, "blend", json!(b.label()));
    }
    if let Some(fo) = style.fill_opacity {
        set_param(f, BLENDING, "fillOpacity", json!((fo * 100.0).round()));
    }
    if let Some(o) = style.opacity {
        set_param(f, BLENDING, "opacity", json!((o * 100.0).round()));
    }
}

/// Saves the dialog's pending state (enabled instances + blending page) as a style preset.
fn save_new_style(app: &mut PhotocraftApp, f: &Map<String, Value>) {
    let mut entries = Vec::new();
    for e in effects_of(f) {
        if e.get("on").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let Some(kind) = e.get("kind").and_then(Value::as_str) else { continue };
        let mut params = defaults(kind);
        if let (Some(o), Some(pv)) = (params.as_object_mut(), e.get("params").and_then(Value::as_object)) {
            for (k, v) in pv {
                o.insert(k.clone(), v.clone());
            }
        }
        entries.push(json!([kind, params]));
    }
    let bo = f.get("p:blendingOptions").cloned().unwrap_or_else(|| json!({}));
    if let Ok(v) = app.run(
        "style.presets.new",
        json!({
            "layer": f.get("layer").cloned().unwrap_or(Value::Null),
            "effects": entries,
            "blend": bo.get("blend").cloned().unwrap_or(Value::Null),
            "fillOpacity": bo.get("fillOpacity").cloned().unwrap_or(Value::Null),
            "opacity": bo.get("opacity").cloned().unwrap_or(Value::Null),
        }),
    ) {
        app.ui.status = crate::i18n::fmt(tl!("Style {name} saved"), &[("name", v["name"].as_str().unwrap_or("Style"))]);
    }
}

/// Disabled effects keep their muted labels even when their parameter page stays
/// selected; selecting a page is distinct from enabling its effect (#2288).
fn fx_label_color(enabled: bool, t: &Tokens) -> Color32 {
    if enabled { t.text } else { t.text_dim }
}

/// Dialog body (left list, right parameters).
/// The enable checkbox of an effect row.
fn fx_checkbox_rect(row: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(row.min + vec2(6.0, 6.0), vec2(14.0, 14.0))
}

/// An effect row's checkbox: an accent box with a tick when on, an outline when off.
fn fx_checkbox(p: &egui::Painter, cb: egui::Rect, on: bool, t: &Tokens) {
    if on {
        p.rect_filled(cb, 2.0, t.accent);
        p.line_segment([cb.left_center() + vec2(3.0, 0.5), cb.center_bottom() + vec2(-1.0, -3.5)], Stroke::new(1.8, Color32::WHITE));
        p.line_segment([cb.center_bottom() + vec2(-1.0, -3.5), cb.right_top() + vec2(-3.0, 3.5)], Stroke::new(1.8, Color32::WHITE));
    } else {
        p.rect_stroke(cb, 2.0, Stroke::new(1.5, t.text_faint), StrokeKind::Inside);
    }
}

pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let selected = f.get("selected").and_then(Value::as_str).unwrap_or("dropShadow").to_string();
    let mut style_click: Option<String> = None;
    let mut new_style = false;
    let invalid = validate_params(f).err();
    ui.horizontal_top(|ui| {
        // Left: effect list.
        ui.vertical(|ui| {
            ui.set_width(190.0);
            // Styles & Blending Options rows (matching the effect row layout & padding).
            for (id, label) in [(STYLES_PAGE, "Styles"), (BLENDING, "Blending Options")] {
                let is_sel = selected == id;
                let (rect, resp) = ui.allocate_exact_size(vec2(190.0, 26.0), Sense::click());
                if is_sel {
                    ui.painter().rect_filled(rect, t.radius_sm, t.row_selected.gamma_multiply(if t.pro { 1.0 } else { 0.0 }).max_alpha(t.hover));
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, t.radius_sm, t.hover.gamma_multiply(0.5));
                }
                ui.painter().text(
                    rect.left_center() + vec2(28.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    tl!(label),
                    egui::FontId::proportional(12.5),
                    if is_sel { t.text } else { t.text_dim },
                );
                if resp.clicked() {
                    f.insert("selected".into(), json!(id));
                }
            }
            ui.add_space(4.0);
            for &(kind, label) in KINDS {
                let ids: Vec<String> = effects_of(f)
                    .iter()
                    .filter(|e| e.get("kind").and_then(Value::as_str) == Some(kind))
                    .filter_map(|e| e.get("id").and_then(Value::as_str).map(str::to_string))
                    .collect();
                if ids.is_empty() {
                    // The effect isn't on the layer: a greyed row with an empty checkbox, as in
                    // Photoshop (#1357); a click on either adds it, turned on.
                    let (rect, resp) = ui.allocate_exact_size(vec2(190.0, 26.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, t.radius_sm, t.hover.gamma_multiply(0.5));
                    }
                    fx_checkbox(ui.painter(), fx_checkbox_rect(rect), false, &t);
                    ui.painter().text(
                        rect.left_center() + vec2(28.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        tl!(label),
                        egui::FontId::proportional(12.5),
                        t.text_faint,
                    );
                    if resp.clicked() {
                        add_instance(f, kind);
                    }
                    continue;
                }
                let count = ids.len();
                for id in &ids {
                    let Some(e) = entry(f, id) else { continue };
                    let kind_label = KINDS.iter().find(|k| k.0 == kind).map(|k| k.1).unwrap_or(label);
                    let mut on = e.get("on").and_then(Value::as_bool).unwrap_or(false);
                    let is_sel = id == &selected;
                    let (rect, resp) = ui.allocate_exact_size(vec2(190.0, 26.0), Sense::click());
                    if is_sel {
                        ui.painter().rect_filled(rect, t.radius_sm, t.row_selected.gamma_multiply(if t.pro { 1.0 } else { 0.0 }).max_alpha(t.hover));
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, t.radius_sm, t.hover.gamma_multiply(0.5));
                    }
                    let cb = fx_checkbox_rect(rect);
                    let cresp = ui.interact(cb, ui.scope_id().with(("fxcb", kind, id)), Sense::click());
                    fx_checkbox(ui.painter(), cb, on, &t);
                    ui.painter().text(
                        rect.left_center() + vec2(28.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        tl!(kind_label),
                        egui::FontId::proportional(12.5),
                        fx_label_color(on, &t),
                    );
                    // + adds another instance; − removes one once there are several.
                    if multi(kind) {
                        let btn = |ui: &egui::Ui, at: egui::Pos2, glyph: &str, tag: &str| {
                            let r = egui::Rect::from_center_size(at, vec2(18.0, 18.0));
                            let hresp = ui.interact(r, ui.scope_id().with(("fxbtn", tag, id)), Sense::click());
                            let col = if hresp.hovered() { t.accent } else { t.text_faint };
                            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(13.0), col);
                            hresp.clicked()
                        };
                        if count > 1 && btn(ui, rect.right_center() - vec2(22.0, 0.0), "\u{2212}", "minus") {
                            remove_instance(f, id);
                            continue;
                        }
                        if btn(ui, rect.right_center() - vec2(8.0, 0.0), "+", "plus") {
                            add_instance(f, kind);
                        }
                    }
                    if cresp.clicked() {
                        on = !on;
                        if let Some(e) = entry_mut(f, id) {
                            e["on"] = json!(on);
                        }
                        f.insert("selected".into(), json!(id));
                    } else if resp.clicked() {
                        f.insert("selected".into(), json!(id));
                    }
                }
            }
        });
        widgets::vline(ui, 330.0);
        // Right: parameters of the selected page.
        ui.vertical(|ui| {
            ui.set_width(330.0);
            if selected == STYLES_PAGE {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tl!("Styles")).font(crate::theme::semibold(14.0)).color(t.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::widgets::secondary_button(ui, tl!("New Style…"), 100.0).clicked() {
                            new_style = true;
                        }
                    });
                });
                ui.add_space(8.0);
                let ctx = ui.ctx().clone();
                egui::ScrollArea::vertical().max_height(340.0).id_salt("layer-style-presets-main").show(ui, |ui| {
                    for group in &app.session.presets.styles {
                        ui.label(RichText::new(&group.name).size(12.0).color(t.text_dim));
                        ui.add_space(3.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                            for st in &group.items {
                                let tex = crate::preset_panels::style_texture(app, &ctx, st);
                                let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
                                ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                                let (stroke, kind) = if resp.hovered() {
                                    (Stroke::new(1.5, t.accent), egui::StrokeKind::Outside)
                                } else {
                                    (Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside)
                                };
                                ui.painter().rect_stroke(rect, 2.0, stroke, kind);
                                if resp.on_hover_text(&st.name).clicked() {
                                    style_click = Some(st.name.clone());
                                }
                            }
                        });
                        ui.add_space(10.0);
                    }
                });
                return;
            }
            let sel_kind: String = if selected == BLENDING {
                BLENDING.to_string()
            } else {
                entry(f, &selected).and_then(|e| e.get("kind").and_then(Value::as_str)).unwrap_or("").to_string()
            };
            let label = KINDS.iter().find(|k| k.0 == sel_kind).map(|k| k.1).unwrap_or(if selected == BLENDING { "Blending Options" } else { "" });
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!(&label)).font(crate::theme::semibold(14.0)).color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut preview = f.get("preview").and_then(Value::as_bool).unwrap_or(true);
                    if widgets::checkbox(ui, &mut preview, tl!("Preview")).changed() {
                        f.insert("preview".into(), json!(preview));
                    }
                });
            });
            ui.add_space(6.0);
            // Only reachable through `ui.dialog.set`: say what is wrong once, keep the bad value.
            if let Some(error) = &invalid {
                ui.colored_label(t.danger, error);
                return;
            }
            if selected == BLENDING {
                // General Blending, Advanced Blending and Blend If, laid out like Photoshop's page.
                let names = channel_names_of(f);
                let key = format!("p:{BLENDING}");
                let Some(mut p) = f.get(&key).filter(|v| v.is_object()).cloned() else { return };
                blending_page(ui, &mut p, &names);
                f.insert(key, p);
                return;
            }
            // The page shows the factory defaults under the instance's values,
            // but only real edits reach the stored params.
            let disp = if selected == BLENDING {
                f.get("p:blendingOptions").cloned().unwrap_or_else(|| json!({}))
            } else {
                entry(f, &selected).and_then(|e| e.get("params").cloned()).unwrap_or_else(|| json!({}))
            };
            let Some(mut disp) = disp.as_object().map(|o| Value::Object(o.clone())) else { return };
            if selected != BLENDING {
                let base = defaults(&sel_kind);
                if let (Some(o), Some(b)) = (disp.as_object_mut(), base.as_object()) {
                    for (k, v) in b {
                        o.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
            }
            for &(key, label, kind_p) in spec(&sel_kind) {
                match kind_p {
                    P::Angle => {
                        let mut v = disp.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                        if widgets::angle_dial_row(ui, label, &mut v, -180.0..=180.0).changed() {
                            let value = json!(v);
                            disp[key] = value.clone();
                            set_param(f, &selected, key, value);
                        }
                    }
                    P::Slider(min, max, unit) => {
                        let mut v = disp.get(key).and_then(Value::as_f64).unwrap_or(min as f64) as f32;
                        if widgets::slider_row(ui, label, &mut v, min..=max, unit, None).changed() {
                            let value = json!(v.round());
                            disp[key] = value.clone();
                            set_param(f, &selected, key, value);
                        }
                    }
                    P::Color => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(tl!(&label)).color(t.text_dim));
                            let color = disp.get(key).and_then(Value::as_str).unwrap_or("#000000");
                            let response = widgets::color_swatch(ui, parse_hex(color)).on_hover_text(tl!("Color Picker (Layer Style Color)"));
                            if response.clicked() {
                                f.insert(color_picker::REQUEST.into(), json!({"effect": selected, "field": key}));
                            }
                        });
                    }
                    P::Blend => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(tl!(&label)).color(t.text_dim));
                            let mut cur = disp.get(key).and_then(Value::as_str).unwrap_or("Normal").to_string();
                            let opts: Vec<(String, &str)> =
                                photocraft_color::BlendMode::LAYER_MODES.iter().map(|m| (m.label().to_string(), m.label())).collect();
                            // One id per mode: Bevel & Emboss has two (Highlight, Shadow), which must not share a list (#9368).
                            if widgets::dropdown(ui, &format!("fx-blend-{sel_kind}-{key}"), tl!(&label), &mut cur, &opts, 150.0) {
                                let value = json!(cur);
                                disp[key] = value.clone();
                                set_param(f, &selected, key, value);
                            }
                        });
                    }
                    P::Choice(options) => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(tl!(&label)).color(t.text_dim));
                            let mut cur = disp.get(key).and_then(Value::as_str).unwrap_or(options[0].0).to_string();
                            let opts: Vec<(String, &str)> = options.iter().map(|(v, l)| (v.to_string(), *l)).collect();
                            if widgets::dropdown(ui, &format!("fx-{sel_kind}-{key}"), tl!(&label), &mut cur, &opts, 150.0) {
                                let value = json!(cur);
                                disp[key] = value.clone();
                                set_param(f, &selected, key, value);
                            }
                        });
                    }
                    P::GradientMethod => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(tl!("Method")).color(t.text_dim));
                            let mut current = disp.get(key).and_then(Value::as_str).unwrap_or("custom").to_string();
                            let mut options = vec![
                                ("smooth".to_string(), "Smooth"),
                                ("perceptual".to_string(), "Perceptual"),
                                ("linear".to_string(), "Linear"),
                                ("classic".to_string(), "Classic"),
                            ];
                            // Old/imported curves have no method record. Preserve them until
                            // the user explicitly chooses an interpolation method.
                            if photocraft_io::GradientMethod::from_name(&current).is_none() {
                                current = "custom".into();
                                options.insert(0, ("custom".into(), "Custom"));
                            }
                            if widgets::dropdown(ui, "fx-gradientOverlay-method", tl!("Method"), &mut current, &options, 150.0) {
                                // The older dialog deliberately omits From/To for rich imported
                                // curves. Once a method is chosen, show the real source ends.
                                let snapshot = entry(f, &selected).and_then(|e| e.get("fx")).cloned();
                                if let Some(Effect::GradientOverlay { gradient, interpolation, .. }) =
                                    snapshot.and_then(|v| serde_json::from_value::<Effect>(v).ok())
                                {
                                    let stops = interpolation.as_ref().map_or(&gradient.stops, |source| &source.stops);
                                    for (key, stop) in [("from", stops.first()), ("to", stops.last())] {
                                        if entry(f, &selected).and_then(|e| e.get("params")).and_then(|p| p.get(key)).is_none()
                                            && let Some(stop) = stop
                                        {
                                            set_param(f, &selected, key, json!(hex(&stop.1)));
                                        }
                                    }
                                }
                                let value = json!(current);
                                disp[key] = value.clone();
                                set_param(f, &selected, key, value);
                            }
                        });
                    }
                    P::Pattern => {
                        let list: Vec<(String, String)> = f
                            .get("patternList")
                            .and_then(Value::as_array)
                            .map(|a| a.iter().filter_map(|e| Some((e.get(0)?.as_str()?.to_string(), e.get(1)?.as_str()?.to_string()))).collect())
                            .unwrap_or_default();
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(tl!(&label)).color(t.text_dim));
                            let mut cur = disp
                                .get(key)
                                .and_then(Value::as_str)
                                .filter(|c| !c.is_empty())
                                .map(str::to_string)
                                .or_else(|| list.first().map(|l| l.0.clone()))
                                .unwrap_or_default();
                            let opts: Vec<(String, &str)> = list.iter().map(|(id, n)| (id.clone(), n.as_str())).collect();
                            if widgets::dropdown(ui, &format!("fx-{sel_kind}-{key}"), tl!(&label), &mut cur, &opts, 180.0)
                                || disp.get(key).and_then(Value::as_str).is_none_or(str::is_empty)
                            {
                                let value = json!(cur);
                                disp[key] = value.clone();
                                set_param(f, &selected, key, value);
                            }
                        });
                    }
                    P::Contour(aa) => contour::row(app, ui, f, &selected, key, label, aa, &mut disp),
                    P::Check => {
                        let mut b = disp.get(key).and_then(Value::as_bool).unwrap_or(false);
                        if widgets::checkbox(ui, &mut b, label).changed() {
                            let value = json!(b);
                            disp[key] = value.clone();
                            set_param(f, &selected, key, value);
                        }
                    }
                }
                ui.add_space(2.0);
            }
            // Make Default stores this page as the user default; Reset to Default
            // restores it (or the factory defaults before the first Make).
            let page_kind = sel_kind != BLENDING && entry(f, &selected).is_some();
            ui.add_space(4.0);
            // Wrap: translated labels (ru, de) can be wider than the page column.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min).with_main_wrap(true), |ui| {
                ui.add_enabled_ui(page_kind, |ui| {
                    if crate::widgets::secondary_button(ui, tl!("Reset to Default"), 130.0).clicked()
                        && let Ok(v) = app.run("layer.layerStyle.defaultFor", json!({"kind": sel_kind}))
                        && let Some(e) = entry_mut(f, &selected)
                    {
                        e["params"] = v["params"].clone();
                    }
                    if crate::widgets::secondary_button(ui, tl!("Make Default"), 110.0).clicked() {
                        let _ = app.run("layer.layerStyle.makeDefault", json!({"kind": sel_kind, "params": disp}));
                    }
                });
            });
        });
    });
    // Style swatch clicks apply to the layer (Photoshop does this immediately);
    // New Style… saves the dialog's pending state as a preset.
    if let Some(name) = style_click {
        apply_style_preset(app, f, &name);
    }
    if new_style {
        save_new_style(app, f);
    }
    contour::editor(app, ui, f);
}

/// Colour channel names of a document mode, as Blending Options lists them (Channels, Blend If).
pub fn channel_names(mode: photocraft_color::ColorMode) -> Vec<&'static str> {
    use photocraft_color::ColorMode as M;
    match mode {
        M::Rgb | M::Indexed | M::Multichannel => vec!["R", "G", "B"],
        M::Cmyk => vec!["C", "M", "Y", "K"],
        M::Lab => vec!["L", "a", "b"],
        _ => vec!["Gray"],
    }
}

const KNOCKOUT: &[(&str, &str)] = &[("none", "None"), ("shallow", "Shallow"), ("deep", "Deep")];

/// (parameter key, label) of the Advanced Blending check boxes, in Photoshop's order.
const ADVANCED_CHECKS: &[(&str, &str)] = &[
    ("blendInteriorEffectsAsGroup", "Blend Interior Effects as Group"),
    ("blendClippedLayersAsGroup", "Blend Clipped Layers as Group"),
    ("transparencyShapesLayer", "Transparency Shapes Layer"),
    ("layerMaskHidesEffects", "Layer Mask Hides Effects"),
    ("vectorMaskHidesEffects", "Vector Mask Hides Effects"),
];

/// The Blending Options page's fields for `layer` in a `mode` document (the parameters of
/// `layer.layerStyle.blendingOptions`, plus the page's Blend If channel), and the channel names.
pub fn set_blending_fields(f: &mut Map<String, Value>, layer: &Layer, mode: photocraft_color::ColorMode) {
    let names = channel_names(mode);
    let n = names.len();
    let a = &layer.advanced;
    // Blend If entries in the command's form: index 0 is Gray for colour documents; a one-channel
    // document's Gray is its channel's own entry (index 1).
    let first = if n == 1 { 1 } else { 0 };
    let blend_if: Vec<Value> = (first..=n)
        .map(|i| {
            let [this, under] = layer.blend_if.get(i);
            json!({"channel": i, "thisLayer": this.to_bytes(), "underlying": under.to_bytes()})
        })
        .collect();
    f.insert(
        format!("p:{BLENDING}"),
        json!({
            "blend": layer.blend.label(),
            "opacity": (layer.opacity * 100.0).round(),
            "fillOpacity": (layer.fill_opacity * 100.0).round(),
            "channels": (0..n).map(|i| layer.excluded_channels & (1 << i) == 0).collect::<Vec<_>>(),
            "knockout": a.knockout.name(),
            "blendInteriorEffectsAsGroup": a.blend_interior,
            "blendClippedLayersAsGroup": a.blend_clipped,
            "transparencyShapesLayer": a.transparency_shapes,
            "layerMaskHidesEffects": a.layer_mask_hides_effects,
            "vectorMaskHidesEffects": a.vector_mask_hides_effects,
            "blendIfChannel": 0,
            "blendIf": blend_if,
        }),
    );
    f.insert("channelNames".into(), json!(names));
}

fn channel_names_of(f: &Map<String, Value>) -> Vec<String> {
    f.get("channelNames")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect::<Vec<_>>())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| ["R", "G", "B"].iter().map(|s| (*s).to_string()).collect())
}

/// The Blending Options page: General Blending, Advanced Blending and Blend If, as in Photoshop.
fn blending_page(ui: &mut egui::Ui, p: &mut Value, names: &[String]) {
    let t = Tokens::get(ui.ctx());
    let num = |p: &Value, k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
    widgets::section_label(ui, "General Blending");
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Blend Mode")).color(t.text_dim));
        let mut cur = p.get("blend").and_then(Value::as_str).unwrap_or("Normal").to_string();
        // Photoshop's modes, plus the layer's own Paint.NET mode when it has one (an imported .pdn).
        let own = photocraft_color::BlendMode::PAINT_NET_MODES.into_iter().find(|m| m.label() == cur);
        let opts: Vec<(String, &str)> = photocraft_color::BlendMode::LAYER_MODES.into_iter().chain(own).map(|m| (m.label().to_string(), m.label())).collect();
        if widgets::dropdown(ui, "fx-blend-blendingOptions", tl!("Blend Mode"), &mut cur, &opts, 150.0) {
            p["blend"] = json!(cur);
        }
    });
    let mut v = num(p, "opacity", 100.0);
    if widgets::slider_row(ui, "Opacity", &mut v, 0.0..=100.0, "%", None).changed() {
        p["opacity"] = json!(v.round());
    }
    widgets::hairline(ui);
    widgets::section_label(ui, "Advanced Blending");
    let mut v = num(p, "fillOpacity", 100.0);
    if widgets::slider_row(ui, "Fill Opacity", &mut v, 0.0..=100.0, "%", None).changed() {
        p["fillOpacity"] = json!(v.round());
    }
    // Channels: one check box per colour channel (checked = blends).
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Channels:")).color(t.text_dim));
        let mut ch: Vec<bool> =
            p.get("channels").and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_bool().unwrap_or(true)).collect()).unwrap_or_default();
        ch.resize(names.len(), true);
        let mut changed = false;
        for (i, name) in names.iter().enumerate() {
            if let Some(c) = ch.get_mut(i) {
                changed |= widgets::checkbox(ui, c, name).changed();
            }
        }
        if changed {
            p["channels"] = json!(ch);
        }
    });
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Knockout")).color(t.text_dim));
        let mut cur = p.get("knockout").and_then(Value::as_str).unwrap_or("none").to_string();
        let opts: Vec<(String, &str)> = KNOCKOUT.iter().map(|(v, l)| (v.to_string(), *l)).collect();
        if widgets::dropdown(ui, "fx-blendingOptions-knockout", tl!("Knockout"), &mut cur, &opts, 150.0) {
            p["knockout"] = json!(cur);
        }
    });
    for &(key, label) in ADVANCED_CHECKS {
        let default = matches!(key, "blendClippedLayersAsGroup" | "transparencyShapesLayer");
        let mut b = p.get(key).and_then(Value::as_bool).unwrap_or(default);
        if widgets::checkbox(ui, &mut b, label).changed() {
            p[key] = json!(b);
        }
    }
    widgets::hairline(ui);
    // Blend If: the channel, then This Layer / Underlying Layer split sliders (Alt-drag splits).
    let entries = p.get("blendIf").and_then(Value::as_array).map_or(0, Vec::len);
    if entries == 0 {
        return;
    }
    let sel = p.get("blendIfChannel").and_then(Value::as_u64).map_or(0, |v| v as usize).min(entries - 1);
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Blend If:")).color(t.text_dim));
        // A colour document lists Gray first; a one-channel document only its channel (Gray).
        let label = |i: usize| -> String {
            if entries == names.len() {
                names.get(i).cloned().unwrap_or_default()
            } else if i == 0 {
                "Gray".to_string()
            } else {
                names.get(i - 1).cloned().unwrap_or_default()
            }
        };
        let labels: Vec<String> = (0..entries).map(label).collect();
        let opts: Vec<(usize, &str)> = labels.iter().enumerate().map(|(i, l)| (i, l.as_str())).collect();
        let mut cur = sel;
        if widgets::dropdown(ui, "fx-blendingOptions-blendIf", tl!("Blend If:"), &mut cur, &opts, 110.0) {
            p["blendIfChannel"] = json!(cur);
        }
    });
    for (key, label) in [("thisLayer", "This Layer:"), ("underlying", "Underlying Layer:")] {
        let mut q = blend_quad(p.get("blendIf").and_then(|a| a.get(sel)).and_then(|e| e.get(key)));
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!(label)).color(t.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let side = |lo: u8, hi: u8| if lo == hi { lo.to_string() } else { format!("{lo} / {hi}") };
                ui.label(RichText::new(format!("{}    {}", side(q[0], q[1]), side(q[2], q[3]))).color(t.text));
            });
        });
        if blend_if_bar(ui, ui.scope_id().with(("blendIf", key)), &mut q)
            && let Some(e) = p.get_mut("blendIf").and_then(|a| a.get_mut(sel))
        {
            e[key] = json!(q);
        }
        ui.add_space(4.0);
    }
}

/// A Blend If range from the dialog's JSON (`[blackLo, blackHi, whiteLo, whiteHi]`), full when absent.
fn blend_quad(v: Option<&Value>) -> [u8; 4] {
    let mut q = [0u8, 0, 255, 255];
    if let Some(a) = v.and_then(Value::as_array) {
        for (dst, x) in q.iter_mut().zip(a) {
            *dst = x.as_f64().map_or(*dst, |f| f.clamp(0.0, 255.0).round() as u8);
        }
    }
    // The sliders never cross.
    for i in 1..4 {
        q[i] = q[i].max(q[i - 1]);
    }
    q
}

/// A Blend If slider: a black-to-white ramp with a black and a white handle, each split in two
/// halves by Alt-dragging (the layer fades between them). Returns whether `q` changed.
fn blend_if_bar(ui: &mut egui::Ui, id: egui::Id, q: &mut [u8; 4]) -> bool {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width().clamp(120.0, 320.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click_and_drag());
    let ramp = egui::Rect::from_min_size(rect.min + vec2(6.0, 0.0), vec2(w - 12.0, 12.0));
    let x_of = |v: u8| ramp.left() + ramp.width() * f32::from(v) / 255.0;
    let v_of = |x: f32| (((x - ramp.left()) / ramp.width().max(1.0)) * 255.0).round().clamp(0.0, 255.0) as u8;
    // The ramp, drawn as vertical strips.
    let strips = 32;
    for s in 0..strips {
        let x0 = ramp.left() + ramp.width() * s as f32 / strips as f32;
        let x1 = ramp.left() + ramp.width() * (s + 1) as f32 / strips as f32;
        let g = (255.0 * (s as f32 + 0.5) / strips as f32) as u8;
        ui.painter().rect_filled(egui::Rect::from_x_y_ranges(x0..=x1, ramp.y_range()), 0.0, Color32::from_gray(g));
    }
    ui.painter().rect_stroke(ramp, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    // Handles: triangles under the ramp; a split handle shows two half triangles.
    let mut changed = false;
    let drag_key = id.with("handle");
    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let near = (0..4).min_by(|a, b| (x_of(q[*a]) - pos.x).abs().total_cmp(&(x_of(q[*b]) - pos.x).abs())).unwrap_or(0);
        // Between two coinciding halves, pick by the side of the pointer.
        let near = match near {
            0 | 1 if q[0] == q[1] => usize::from(pos.x > x_of(q[0])),
            2 | 3 if q[2] == q[3] => 2 + usize::from(pos.x > x_of(q[2])),
            n => n,
        };
        ui.data_mut(|d| d.insert_temp(drag_key, near));
    }
    if resp.dragged()
        && let (Some(h), Some(pos)) = (ui.data(|d| d.get_temp::<usize>(drag_key)), resp.interact_pointer_pos())
        && let Some(cur) = q.get(h).copied()
    {
        let target = v_of(pos.x);
        let split = ui.input(|i| i.modifiers.alt);
        let mut n = *q;
        let pair = if h < 2 { 0 } else { 2 };
        if split {
            n[h] = target;
        } else {
            // Unsplit: the pair moves together, keeping its spread.
            let delta = i16::from(target) - i16::from(cur);
            for v in &mut n[pair..pair + 2] {
                *v = (i16::from(*v) + delta).clamp(0, 255) as u8;
            }
        }
        // Keep the order black low <= black high <= white low <= white high, pushing the others.
        for i in 0..4 {
            if i < h {
                n[i] = n[i].min(n[h]);
            } else if i > h {
                n[i] = n[i].max(n[h]);
            }
        }
        for i in 1..4 {
            n[i] = n[i].max(n[i - 1]);
        }
        if n != *q {
            *q = n;
            changed = true;
        }
    }
    let base = ramp.bottom() + 1.0;
    let tri = |x: f32, half: i8, fill: Color32| {
        let (l, r) = match half {
            -1 => (x - 5.0, x),
            1 => (x, x + 5.0),
            _ => (x - 5.0, x + 5.0),
        };
        let pts = vec![egui::pos2(x, base), egui::pos2(r, base + 10.0), egui::pos2(l, base + 10.0)];
        ui.painter().add(egui::Shape::convex_polygon(pts, fill, Stroke::new(1.0, t.text_dim)));
    };
    let dark = Color32::from_gray(20);
    let light = Color32::from_gray(240);
    for (pair, fill) in [(0usize, dark), (2, light)] {
        let (a, b) = (q[pair], q[pair + 1]);
        if a == b {
            tri(x_of(a), 0, fill);
        } else {
            tri(x_of(a), -1, fill);
            tri(x_of(b), 1, fill);
        }
    }
    changed
}

fn parse_hex(s: &str) -> Color32 {
    let s = s.trim_start_matches('#');
    let b = |i: usize| u8::from_str_radix(s.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
    Color32::from_rgb(b(0), b(2), b(4))
}

trait MaxAlpha {
    fn max_alpha(self, other: Color32) -> Color32;
}

impl MaxAlpha for Color32 {
    /// Fall back to `other` when this colour is fully transparent (non-Pro themes have no row colour).
    fn max_alpha(self, other: Color32) -> Color32 {
        if self.a() == 0 { other } else { self }
    }
}

#[cfg(test)]
mod tests;
