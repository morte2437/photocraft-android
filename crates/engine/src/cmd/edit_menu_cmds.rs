//! The rest of Photoshop's Edit menu: Fade, Purge, Content-Aware Fill and Scale, Define Brush
//! Preset / Custom Shape, Find and Replace Text, the Preset Manager and preset export/import.
//! Auto-Align and Auto-Blend Layers live in [`crate::align_cmds`].
//!
//! Check Spelling is not implemented: it needs a word list per language, and no permissively
//! licensed dictionary small enough to bundle (or a pure-Rust checker with one) was available.

use photocraft_color::{BlendMode, PixelFormat};
use photocraft_doc::vector::Path;
use photocraft_doc::{DocId, Document, Layer, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba_into, to_rgba};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::channel_cmds::Target;
use crate::commands::{CommandSpec, blend_from_str, int, layer_param};
use crate::{EngineError, Result, Session};

// ------------------------------------------------------------------ state

/// What Edit › Fade would fade: the result of the last filter, adjustment or paint command.
#[derive(Clone, Debug, PartialEq)]
pub struct FadeSource {
    pub doc: DocId,
    /// Document revision right after the command; any later change disables Fade.
    pub revision: u64,
    pub target: FadeTarget,
    pub label: String,
}

/// What the faded command changed, and so what Fade blends back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FadeTarget {
    /// A layer's pixels.
    Pixels(LayerId),
    /// A layer's layer mask.
    Mask(LayerId),
    /// An alpha or spot channel (index into [`Document::channels`]).
    Channel(usize),
    /// The Quick Mask.
    QuickMask,
}

/// A custom shape (Edit › Define Custom Shape), usable by the Custom Shape tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomShape {
    pub name: String,
    pub path: Path,
}

/// Edit-menu state kept by the session.
#[derive(Clone, Debug, Default)]
pub struct EditState {
    pub fade: Option<FadeSource>,
    /// Custom shape library (the vector tools read it).
    pub custom_shapes: Vec<CustomShape>,
    /// Find and Replace Text position: (layer, byte offset after the current match).
    pub find_cursor: Option<(LayerId, usize, usize)>,
}

/// Commands whose result Edit › Fade can blend back against the previous state.
fn fadeable(id: &str) -> bool {
    id.starts_with("filter.")
        || id.starts_with("image.adjustments.")
        || id.starts_with("paint.")
        || matches!(id, "edit.fill" | "edit.stroke" | "edit.contentAwareFill" | "image.autoTone" | "image.autoContrast" | "image.autoColor")
}

/// Called after every successful command: remember a fadeable result.
pub(crate) fn after_command(s: &mut Session, id: &str, params: &Value) {
    if !fadeable(id) {
        return;
    }
    let Some(st) = s.active() else { return };
    let label = st.history.undo_label().unwrap_or(id).to_string();
    let prev = st.history.state(st.history.past_len().wrapping_sub(1));
    let prev = prev.as_deref();
    let surf_eq = |a: Option<&Surface>, b: Option<&Surface>| match (a, b) {
        (Some(a), Some(b)) => a.format() == b.format() && a.default_pixel() == b.default_pixel() && crate_fingerprint(a) == crate_fingerprint(b),
        (None, None) => true,
        _ => false,
    };
    // An alpha channel or the Quick Mask took the edit instead of the layer when the command ran
    // on it (#7382): its explicit `target`, or the one the Channels panel gave it. Only commands
    // routed to channels edit them, and some routed ones (Edit › Stroke, the Magic and Background
    // Erasers) still edit the layer whatever `target` says, so the channel must have changed.
    let run = crate::channel_cmds::inject_target(s, id, params.clone());
    let channel = match crate::channel_cmds::target_of(&run) {
        _ if !crate::channel_cmds::routed(id) => None,
        Target::Alpha(i) if i < st.doc.channels.len() => Some(FadeTarget::Channel(i)),
        Target::QuickMask if st.doc.quick_mask.is_some() => Some(FadeTarget::QuickMask),
        _ => None,
    };
    fn channel_surface<'a>(doc: &'a Document, target: &FadeTarget) -> Option<&'a Surface> {
        match target {
            FadeTarget::Channel(i) => doc.channels.get(*i).map(|c| &c.surface),
            _ => doc.quick_mask.as_ref().map(|c| &c.surface),
        }
    }
    let channel = channel.filter(|target| prev.is_none_or(|prev| !surf_eq(channel_surface(&st.doc, target), channel_surface(prev, target))));
    if let Some(target) = channel {
        s.edit_state.fade = Some(FadeSource { doc: st.doc.id, revision: st.revision, target, label });
        return;
    }
    let Some(layer) = st.active_layer else { return };
    let mask = st.doc.layer(layer).is_some_and(|l| l.mask.is_some())
        && prev.is_some_and(|prev| {
            // The mask changed and the pixels didn't: the command painted the mask.
            let now = st.doc.layer(layer);
            let was = prev.layer(layer);
            surf_eq(now.and_then(|l| l.surface()), was.and_then(|l| l.surface()))
                && !surf_eq(now.and_then(|l| l.mask.as_ref().map(|m| &m.surface)), was.and_then(|l| l.mask.as_ref().map(|m| &m.surface)))
        });
    let target = if mask { FadeTarget::Mask(layer) } else { FadeTarget::Pixels(layer) };
    s.edit_state.fade = Some(FadeSource { doc: st.doc.id, revision: st.revision, target, label });
}

/// Tile identity of a surface (pointer equality of its COW tiles).
fn crate_fingerprint(s: &Surface) -> u64 {
    s.tiles().fold(s.tile_count() as u64, |h, (c, t)| {
        (h ^ std::sync::Arc::as_ptr(t) as usize as u64 ^ ((c.tx as u64) << 32 | c.ty as u32 as u64)).wrapping_mul(0x100_0000_01b3)
    })
}

// ------------------------------------------------------------------ helpers

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn f32_or(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}

fn str_or<'a>(p: &'a Value, k: &str, d: &'a str) -> &'a str {
    p.get(k).and_then(Value::as_str).unwrap_or(d)
}

fn bool_or(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn pixel_layer(s: &Session) -> std::result::Result<LayerId, String> {
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    pixel_layer_id(s, id)
}

/// `id` if it is an unlocked pixel layer of the active document.
fn pixel_layer_id(s: &Session, id: LayerId) -> std::result::Result<LayerId, String> {
    let d = s.active().ok_or("no document open")?;
    let l = d.doc.layer(id).ok_or("no active layer")?;
    if !matches!(l.content, LayerContent::Raster(_)) {
        return Err(format!("active layer is {} {} layer, not a pixel layer", l.content.article(), l.content.kind_name()));
    }
    let locks = d.doc.effective_locks(id);
    if locks.all || locks.pixels {
        return Err(format!("the layer \"{}\" is locked", l.name));
    }
    Ok(id)
}

fn writable_surface(doc: &mut Document, id: LayerId) -> Result<&mut Surface> {
    let locks = doc.effective_locks(id);
    let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
    if locks.all || locks.pixels {
        return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
    }
    l.surface_mut().ok_or_else(|| EngineError::Other("not a pixel layer".into()))
}

fn surface_of(doc: &Document, target: FadeTarget) -> Option<&Surface> {
    match target {
        FadeTarget::Pixels(id) => doc.layer(id)?.surface(),
        FadeTarget::Mask(id) => doc.layer(id)?.mask.as_ref().map(|m| &m.surface),
        FadeTarget::Channel(i) => doc.channels.get(i).map(|c| &c.surface),
        FadeTarget::QuickMask => doc.quick_mask.as_ref().map(|c| &c.surface),
    }
}

fn surface_of_mut(doc: &mut Document, target: FadeTarget) -> Option<&mut Surface> {
    match target {
        FadeTarget::Pixels(id) => doc.layer_mut(id)?.surface_mut(),
        FadeTarget::Mask(id) => doc.layer_mut(id)?.mask.as_mut().map(|m| &mut m.surface),
        FadeTarget::Channel(i) => doc.channels.get_mut(i).map(|c| &mut c.surface),
        FadeTarget::QuickMask => doc.quick_mask.as_mut().map(|c| &mut c.surface),
    }
}

fn export_import(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "edit.presets.exportImportPresets";
    let kinds: Vec<String> = p
        .get("kinds")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_else(|| ["brushes", "customShapes", "tools"].iter().map(ToString::to_string).collect());
    let want = |k: &str| kinds.iter().any(|x| x == k);
    match str_or(p, "action", "export") {
        "export" => {
            let file = PresetFile {
                format: PRESET_FORMAT.into(),
                version: 1,
                brushes: if want("brushes") {
                    let mut out: Vec<photocraft_paint::BrushPreset> =
                        s.tools.presets.iter().filter(|b| !b.builtin || bool_or(p, "includeBuiltins", false)).cloned().collect();
                    // The file embeds every bitmap: load the tips the preset store keeps (#1843).
                    for b in &mut out {
                        s.load_brush_tips(&mut b.brush).map_err(|e| bad(cmd, format!("brush preset `{}`: {e}", b.name)))?;
                    }
                    out
                } else {
                    Vec::new()
                },
                custom_shapes: if want("customShapes") { s.edit_state.custom_shapes.clone() } else { Vec::new() },
                tool_presets: if want("tools") {
                    let builtins = crate::presets::tools::builtin();
                    s.presets.tool_presets.iter().filter(|preset| bool_or(p, "includeBuiltins", false) || !builtins.contains(preset)).cloned().collect()
                } else {
                    Vec::new()
                },
            };
            Ok(json!({"data": file, "brushes": file.brushes.len(), "customShapes": file.custom_shapes.len(), "tools": file.tool_presets.len()}))
        }
        "import" => {
            let data = p.get("data").cloned().ok_or_else(|| bad(cmd, "missing `data` (an exported preset file)"))?;
            let data = match data {
                Value::String(text) => serde_json::from_str(&text).map_err(|e| bad(cmd, format!("not a preset file: {e}")))?,
                v => v,
            };
            let file: PresetFile = serde_json::from_value(data).map_err(|e| bad(cmd, format!("not a preset file: {e}")))?;
            if file.format != PRESET_FORMAT {
                return Err(bad(cmd, format!("not a preset file (format `{}`)", file.format)));
            }
            // Validate the whole tool import before changing any preset library.
            let mut tools = s.presets.tool_presets.clone();
            let nt = if want("tools") { file.tool_presets.len() } else { 0 };
            if nt > crate::presets::tools::MAX_PRESETS {
                return Err(bad(cmd, "too many tool presets"));
            }
            if want("tools") {
                for mut preset in file.tool_presets {
                    preset.name = crate::presets::tools::checked_name(&preset.name, cmd)?;
                    if preset.tool.trim().is_empty() || !preset.options.is_object() {
                        return Err(bad(cmd, "tool presets require a non-empty tool and an options object"));
                    }
                    if let Some(old) = tools.iter_mut().find(|old| old.name == preset.name && old.tool.eq_ignore_ascii_case(&preset.tool)) {
                        *old = preset;
                    } else if tools.iter().any(|old| old.name == preset.name) {
                        return Err(bad(cmd, "a preset of that name belongs to another tool; rename it before importing"));
                    } else if tools.len() < crate::presets::tools::MAX_PRESETS {
                        tools.push(preset);
                    } else {
                        return Err(bad(cmd, "too many tool presets"));
                    }
                }
            }
            let (mut nb, mut ns) = (0, 0);
            if want("brushes") {
                for mut b in file.brushes {
                    // Nested folders as the commands bound them (a hand-edited file can't build an
                    // absurd tree).
                    b.folder.retain(|f| !f.trim().is_empty());
                    b.folder.truncate(crate::brush_preset_cmds::MAX_FOLDER_DEPTH);
                    match s.tools.presets.iter_mut().find(|x| x.name == b.name) {
                        Some(x) => *x = b,
                        None => s.tools.presets.push(b),
                    }
                    nb += 1;
                }
                s.brush_presets_changed();
            }
            if want("customShapes") {
                for c in file.custom_shapes {
                    match s.edit_state.custom_shapes.iter_mut().find(|x| x.name == c.name) {
                        Some(x) => *x = c,
                        None => s.edit_state.custom_shapes.push(c),
                    }
                    ns += 1;
                }
            }
            if want("tools") {
                s.presets.tool_presets = tools;
                s.presets_changed();
            }
            Ok(json!({"brushes": nb, "customShapes": ns, "tools": nt}))
        }
        other => Err(bad(cmd, format!("unknown action `{other}` (export|import)"))),
    }
}

// ------------------------------------------------------------------ specs

macro_rules! spec {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: true }
    };
}

mod content_aware;
mod content_aware_scale;
mod fade;
mod find_replace;
mod presets;
mod purge;

pub(crate) use content_aware::channel_mask;
pub use content_aware::{CafPlan, FillEdit, MAX_CAF_EDITS, MAX_CAF_POINTS, SelOp, content_aware_plan, content_aware_plan_with};
use content_aware::{can_caf, content_aware_fill, delete_and_fill};
pub use content_aware_scale::ContentAwareScalePreview;
use content_aware_scale::content_aware_scale;
pub use fade::fade_surface;
use fade::{can_fade, fade};
pub use find_replace::find_matches;
use find_replace::{find_replace, has_type_layer};
pub(crate) use presets::current_path;
pub use presets::{PRESET_FORMAT, PresetFile};
use presets::{can_define_shape, define_brush_preset, define_custom_shape, preset_manager};
use purge::{can_purge_all, can_purge_clipboard, can_purge_histories, can_purge_undo, purge};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "edit.fade",
            "Fade…",
            ["Edit"],
            Some("Cmd+Shift+F"),
            r##"{"opacity":0..100=100,"mode":"normal|multiply|screen|overlay|softLight|hardLight|darken|lighten|difference|color|luminosity"}"##,
            can_fade,
            fade
        ),
        spec!("edit.purge.undo", "Undo", ["Edit", "Purge"], None, "{}", can_purge_undo, |s, _| purge(s, "undo")),
        spec!("edit.purge.clipboard", "Clipboard", ["Edit", "Purge"], None, "{}", can_purge_clipboard, |s, _| purge(s, "clipboard")),
        spec!("edit.purge.histories", "Histories", ["Edit", "Purge"], None, "{}", can_purge_histories, |s, _| purge(s, "histories")),
        spec!("edit.purge.videoCache", "Video Cache", ["Edit", "Purge"], None, "{}", always, |_, _| Ok(
            json!({"purged": [], "bytes": 0, "message": "nothing to purge (no video layers)"})
        )),
        spec!("edit.purge.all", "All", ["Edit", "Purge"], None, "{}", can_purge_all, |s, _| purge(s, "all")),
        spec!(
            "edit.contentAwareFill",
            "Content-Aware Fill…",
            ["Edit"],
            None,
            r##"{"sampling":"auto|rectangular|custom","margin":px?,"area":[x,y,w,h]?,"channel":index|name?,"samplingStrokes":[{"mode":"add|subtract","size":px,"points":[[x,y],…]}]? (the workspace's Sampling Brush, after the sampling option's area; added strokes may lie outside the window),"fillEdits":[{"op":"new|add|subtract|intersect","points":[[x,y],…]}|{"op":"expand|contract","by":px}]? (the workspace's lasso edits of the fill area, in order),"sampleAllLayers":bool=false,"colorAdaptation":"default|none|high|veryHigh","rotationAdaptation":"none|low|medium|high|full","scale":bool=false,"mirror":bool=false,"output":"current|new|duplicate","layer":id?=active (the layer filled and sampled),"seed":u64=1}"##,
            can_caf,
            content_aware_fill
        ),
        spec!("edit.deleteAndFillSelection", "Delete and Fill Selection", [], None, "{}", can_caf, delete_and_fill),
        spec!(
            "edit.contentAwareScale",
            "Content-Aware Scale",
            ["Edit"],
            Some("Cmd+Alt+Shift+C"),
            r##"{"width":px?,"height":px?,"scaleX":1..400=100,"scaleY":1..400=100,"x":px?,"y":px?,"amount":0..100=100,"protect":"none"|channel index|name,"protectSkinTones":bool=false}"##,
            |s| pixel_layer(s).map(|_| ()),
            content_aware_scale
        ),
        spec!(
            "edit.defineBrushPreset",
            "Define Brush Preset…",
            ["Edit"],
            None,
            r##"{"name":text}"##,
            |s| if s.is_enabled("brush.defineFromSelection") { Ok(()) } else { Err("select pixels to define a brush from".into()) },
            define_brush_preset
        ),
        spec!(
            "edit.defineCustomShape",
            "Define Custom Shape…",
            ["Edit"],
            None,
            r##"{"name":text,"path":"work|<saved path name>"?}"##,
            can_define_shape,
            define_custom_shape
        ),
        spec!(
            "edit.findAndReplaceText",
            "Find and Replace Text…",
            ["Edit"],
            None,
            r##"{"find":text,"replace":text,"action":"changeAll|find|change|changeFind","caseSensitive":bool=false,"wholeWord":bool=false,"forward":bool=true,"allLayers":bool=true}"##,
            has_type_layer,
            find_replace
        ),
        spec!(
            "edit.presets.presetManager",
            "Preset Manager…",
            ["Edit", "Presets"],
            None,
            r##"{"action":"list|rename|delete|move","kind":"brushes|customShapes|patterns","index":n?,"name":str?,"newName":str?,"to":n?}"##,
            always,
            preset_manager
        ),
        spec!(
            "edit.presets.exportImportPresets",
            "Export/Import Presets…",
            ["Edit", "Presets"],
            None,
            r##"{"action":"export|import","kinds":["brushes","customShapes","tools"]?,"data":json (import),"includeBuiltins":bool=false}"##,
            always,
            export_import
        ),
    ]
}

/// The custom shape library (for the Custom Shape tool).
pub fn custom_shapes(s: &Session) -> &[CustomShape] {
    &s.edit_state.custom_shapes
}

#[cfg(test)]
#[path = "edit_menu_cmds/tests.rs"]
mod tests;
