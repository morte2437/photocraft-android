//! The command registry. Ids follow Photoshop's menu structure (see the parity checklist).

use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Adjustment, Document, Fill, Layer, LayerContent, LayerId, LayerMask, Size};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Map, Value, json};

use crate::{EngineError, Result, Session, inspect, pixels};

type Run = fn(&mut Session, &Value) -> Result<Value>;
type Enabled = fn(&Session) -> std::result::Result<(), String>;

/// Metadata + implementation for one command.
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Menu placement, e.g. `["Layer", "New"]`. Empty = not in menus.
    pub menu: &'static [&'static str],
    /// Default shortcut in egui-style notation (`Cmd+Shift+N`), mapped per platform by the UI.
    pub shortcut: Option<&'static str>,
    /// Human/agent-readable parameter description (JSON-ish).
    pub params: &'static str,
    pub enabled: Enabled,
    pub run: Run,
    /// Record in the session journal (false for queries).
    pub journal: bool,
}

pub(crate) fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}
fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}
fn has_layer(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    d.active_layer.filter(|id| d.doc.layer(*id).is_some()).map(|_| ()).ok_or_else(|| "no active layer".into())
}
fn has_pixel_layer(s: &Session) -> std::result::Result<(), String> {
    let l = crate::active_layer_of(s)?;
    if matches!(l.content, LayerContent::Raster(_)) {
        Ok(())
    } else {
        Err(format!("active layer is {} {} layer, not a pixel layer", l.content.article(), l.content.kind_name()))
    }
}
/// A pixel layer, or a targeted alpha channel / Quick Mask (adjustments and fills apply to it).
fn has_pixel_or_channel(s: &Session) -> std::result::Result<(), String> {
    if crate::channel_cmds::edits_channel(s) {
        return Ok(());
    }
    has_pixel_layer(s)
}
/// A layer that can be painted: pixels, or any layer with a mask (paint with `"target":"mask"`).
pub(crate) fn has_paintable(s: &Session) -> std::result::Result<(), String> {
    if crate::channel_cmds::edits_channel(s) {
        return Ok(());
    }
    let l = crate::active_layer_of(s)?;
    if matches!(l.content, LayerContent::Raster(_)) || l.mask.is_some() {
        Ok(())
    } else {
        Err(format!("active layer is {} {} layer without a mask", l.content.article(), l.content.kind_name()))
    }
}

/// Surface a paint command writes to: the layer's pixels, or its mask with `"target":"mask"`.
pub(crate) fn paint_surface<'a>(doc: &'a mut Document, id: LayerId, p: &Value) -> Result<&'a mut photocraft_raster::Surface> {
    if is_mask_target(p) {
        check_all_unlocked(doc, id)?;
    } else {
        check_pixels_unlocked(doc, id)?;
    }
    let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
    if is_mask_target(p) {
        l.mask.as_mut().map(|m| &mut m.surface).ok_or_else(|| EngineError::Other("layer has no mask".into()))
    } else {
        l.surface_mut().ok_or_else(|| EngineError::Other("not a pixel layer".into()))
    }
}

fn check_all_unlocked(doc: &Document, id: LayerId) -> Result<()> {
    let locks = doc.effective_locks(id);
    let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    if locks.all {
        return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
    }
    Ok(())
}

/// Refuse a layer whose pixels are locked (directly or through a group), with Photoshop's message.
pub(crate) fn check_pixels_unlocked(doc: &Document, id: LayerId) -> Result<()> {
    let locks = doc.effective_locks(id);
    let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    if locks.pixels || locks.all {
        return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
    }
    Ok(())
}

pub(crate) fn is_mask_target(p: &Value) -> bool {
    p.get("target").and_then(Value::as_str) == Some("mask")
}

fn can_undo(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.history.can_undo()).map(|_| ()).ok_or_else(|| "nothing to undo".into())
}
fn can_redo(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.history.can_redo()).map(|_| ()).ok_or_else(|| "nothing to redo".into())
}
fn has_selection(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|d| d.doc.selection.is_some()).map(|_| ()).ok_or_else(|| "no selection".into())
}

/// Image › Image Rotation by a right angle or a flip (pixels, vectors, guides, … all move).
fn turn(s: &mut Session, label: &str, t: crate::canvas_geom::Turn) -> Result<Value> {
    if let Some(d) = s.active() {
        let a = t.affine(f64::from(d.doc.size.width), f64::from(d.doc.size.height));
        // Sampler points are pixel indices, not pixel edges: map the pixel's centre, so the point
        // lands on the same pixel `Turn::pixel_map` moves the pixels to (`w - 1 - x`).
        let half = photocraft_geom::Affine::translate(0.5, 0.5);
        s.set_canvas_map(a, half.then(&a).then(&photocraft_geom::Affine::translate(-0.5, -0.5)));
    }
    s.edit(label, |doc, _| {
        crate::canvas_geom::turn_canvas(doc, t);
        Ok(())
    })?;
    Ok(Value::Null)
}

macro_rules! cmd {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: true }
    };
}

pub fn command_specs() -> &'static [CommandSpec] {
    &registry().specs
}

/// One `command.list` / `engine.commands` row for `c`, given why it can't run now (`None` = it can).
/// `disabledReason` is that message; `needsDocument` is true when the reason is that no document is
/// open (#3664), so an agent can tell "open or create a document first" from a precondition that a
/// document alone won't meet (a selection, an empty clipboard).
pub fn list_entry(s: &Session, c: &CommandSpec, reason: Option<String>) -> Value {
    let needs_document =
        s.active().is_none() && reason.as_deref().is_some_and(|r| ["no document", "no active document", "no open document"].iter().any(|p| r.starts_with(p)));
    let mut entry = json!({
        "id": c.id, "label": c.label, "menu": c.menu, "shortcut": c.shortcut, "params": c.params,
        "enabled": reason.is_none(), "disabledReason": reason, "needsDocument": needs_document,
    });
    if let (Some(k), Some(o)) = (search_keywords(c.id), entry.as_object_mut()) {
        o.insert("keywords".into(), json!(k));
    }
    entry
}

/// Other names a command is looked for by, beyond its id and label: the Photoshop tools and
/// options it stands for (#5060). `select.rect` is also the Elliptical and Single Row/Column
/// Marquee, which an agent searches for as "ellipse" or "marquee".
pub(crate) const SEARCH_KEYWORDS: &[(&str, &str)] =
    &[("select.rect", "Rectangular Marquee, Elliptical Marquee (\"ellipse\":true), ellipse, oval, circle, Single Row/Column Marquee (\"line\")")];

/// [`SEARCH_KEYWORDS`] of command `id`, if it has any.
pub fn search_keywords(id: &str) -> Option<&'static str> {
    SEARCH_KEYWORDS.iter().find(|(k, _)| *k == id).map(|(_, words)| *words)
}

/// Does a [`list_entry`] row match a command-list filter (`commands --filter`, `engine.commands`,
/// MCP `command_list`)? Its id, label or keywords contain `needle`, ignoring case.
pub fn list_entry_matches(entry: &Value, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    ["id", "label", "keywords"].iter().any(|k| entry.get(*k).and_then(Value::as_str).is_some_and(|v| v.to_lowercase().contains(&needle)))
}

pub fn find(id: &str) -> Option<&'static CommandSpec> {
    let registry = registry();
    registry.by_id.get(id).and_then(|&index| registry.specs.get(index))
}

struct Registry {
    specs: Vec<CommandSpec>,
    by_id: std::collections::HashMap<&'static str, usize>,
}

impl Registry {
    fn new(specs: Vec<CommandSpec>) -> Self {
        let mut by_id = std::collections::HashMap::with_capacity(specs.len());
        for (index, spec) in specs.iter().enumerate() {
            // Preserve the linear lookup's first-match behavior if an id is duplicated.
            by_id.entry(spec.id).or_insert(index);
        }
        Self { specs, by_id }
    }
}

fn registry() -> &'static Registry {
    static REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| Registry::new(build()))
}

// ---------- param helpers ----------

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}
/// Integer parameter that also accepts JSON floats (UIs send `12.0`); rounds.
pub(crate) fn int(p: &Value, key: &str) -> Option<i64> {
    p.get(key).and_then(|v| v.as_i64().or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f.round() as i64)))
}

/// [`int`] narrowed to `i32` for geometry: `None` when absent, and a bad-params error when the
/// value would wrap through `as i32` — out-of-range integers truncate by multiples of 2^32,
/// which silently relocates geometry (`dx = 2^32 + 50` moves 50 px, `3e9` goes negative)
/// instead of erroring.
pub(crate) fn int_i32(cmd: &str, p: &Value, key: &str) -> Result<Option<i32>> {
    match int(p, key) {
        None => Ok(None),
        Some(v) => i32::try_from(v).map(Some).map_err(|_| bad(cmd, format!("`{key}` = {v} is outside the 32-bit coordinate range"))),
    }
}

/// An id (layer comp, slice, style…) from a JSON value: a bad-params error unless it is a whole
/// number in `u32` range. `as u32` would wrap `2^32 + 1` to `1` and target a real item.
pub(crate) fn u32_id(cmd: &str, key: &str, v: &Value) -> Result<u32> {
    v.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| bad(cmd, format!("`{key}` = {v} is not a valid id (0..={})", u32::MAX)))
}

/// [`u32_id`] of `p[key]`: `None` when absent.
pub(crate) fn u32_id_param(cmd: &str, p: &Value, key: &str) -> Result<Option<u32>> {
    p.get(key).map(|v| u32_id(cmd, key, v)).transpose()
}

/// JSON numbers may be finite f64 values but overflow when stored in a document's f32 field.
pub(crate) fn finite_f32(cmd: &str, key: &str, value: f64) -> Result<f32> {
    let narrowed = value as f32;
    if narrowed.is_finite() { Ok(narrowed) } else { Err(bad(cmd, format!("`{key}` must fit a finite 32-bit number"))) }
}

/// Effect/type parameters contain nested colour arrays and snapshots as well as scalars.
/// Validate before parsing or editing, including numbers hidden in an effect snapshot.
pub(crate) fn check_f32_values(cmd: &str, value: &Value) -> Result<()> {
    // Bound the call stack, without allocating a work list proportional to an input array.
    fn check(cmd: &str, value: &Value, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err(bad(cmd, "parameters nested deeper than 64 levels"));
        }
        match value {
            Value::Number(n) => {
                finite_f32(cmd, "number", n.as_f64().ok_or_else(|| bad(cmd, "invalid number"))?)?;
            }
            Value::Array(a) => {
                for v in a {
                    check(cmd, v, depth + 1)?;
                }
            }
            Value::Object(o) => {
                for v in o.values() {
                    check(cmd, v, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    check(cmd, value, 0)
}

pub(crate) fn color_param(cmd: &str, p: &Value, key: &str, default: [f32; 4]) -> Result<[f32; 4]> {
    Ok(match p.get(key) {
        Some(Value::Array(a)) if a.len() >= 3 => {
            let mut out = [0.0, 0.0, 0.0, 1.0];
            for (slot, value) in out.iter_mut().zip(a) {
                *slot = finite_f32(cmd, key, value.as_f64().unwrap_or(0.0))?;
            }
            out
        }
        Some(Value::String(s)) => parse_hex(s).unwrap_or(default),
        _ => default,
    })
}
pub(crate) fn parse_hex(s: &str) -> Option<[f32; 4]> {
    let s = s.trim_start_matches('#');
    let b = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok().map(|v| v as f32 / 255.0);
    match s.len() {
        6 => Some([b(0)?, b(2)?, b(4)?, 1.0]),
        8 => Some([b(0)?, b(2)?, b(4)?, b(6)?]),
        _ => None,
    }
}
/// Layer › Create / Release Clipping Mask. With a `layer` param, just that layer. Otherwise, as in
/// Photoshop, the selection (#1248): Create clips every selected layer except the lowest, which
/// becomes the base; Release releases every selected layer. Releasing a clipped layer also
/// releases the clipped layers above it in its group, as Adobe documents ("removes the selected
/// layer and any layers above it from the clipping mask"), so none is left clipped to a layer
/// that is no longer clipped. One history step either way.
fn set_clipped(s: &mut Session, p: &Value, clip: bool) -> Result<Value> {
    let label = if clip { "Create Clipping Mask" } else { "Release Clipping Mask" };
    let ids: Vec<LayerId> = if p.get("layer").is_some() {
        vec![layer_param(s, p)?]
    } else {
        let selected = s.active().ok_or(EngineError::NoDocument)?.selected_layers();
        match (clip, selected.as_slice()) {
            (_, []) => vec![layer_param(s, p)?],
            (_, [one]) => vec![*one],
            (true, [_base, rest @ ..]) => rest.to_vec(),
            (false, all) => all.to_vec(),
        }
    };
    let ids = if clip {
        ids
    } else {
        let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
        let mut all = ids.clone();
        for id in ids.iter().filter(|id| doc.layer(**id).is_some_and(|l| l.clipped)) {
            for above in crate::clip_group::clipped_above(doc, *id) {
                if !all.contains(&above) {
                    all.push(above);
                }
            }
        }
        all
    };
    s.edit(label, |doc, _| {
        for id in &ids {
            doc.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?.clipped = clip;
        }
        Ok(())
    })?;
    Ok(json!({"layers": ids.iter().map(|id| id.0).collect::<Vec<_>>()}))
}

/// The layer a command's `"layer"` param names: an id, or a name (the topmost layer of that
/// name, as Photoshop's "select layer by name" action step finds it, so a loaded action replays
/// in another document; a name that matches no layer exactly matches regardless of case, so
/// `"background"` finds the Background). `None` when the param is absent.
pub(crate) fn explicit_layer(s: &Session, p: &Value) -> Result<Option<LayerId>> {
    match p.get("layer") {
        Some(Value::Number(n)) => {
            n.as_u64().map(|id| Some(LayerId(id))).ok_or_else(|| EngineError::Other(format!("layer id {n} is not a non-negative integer")))
        }
        Some(Value::String(name)) => {
            let d = s.active().ok_or(EngineError::NoDocument)?;
            let layers = d.doc.walk();
            let mut top_down = layers.iter().rev();
            top_down
                .clone()
                .find(|(_, _, l)| l.name == *name)
                .or_else(|| top_down.find(|(_, _, l)| l.name.to_lowercase() == name.to_lowercase()))
                .map(|(_, _, l)| Some(l.id))
                .ok_or_else(|| EngineError::Other(format!("no layer named \"{name}\"; {}", layer_listing(&d.doc))))
        }
        Some(Value::Null) | None => Ok(None),
        Some(other) => Err(EngineError::Other(format!("`layer` must be an id or a name, not {other}"))),
    }
}

/// The layers of `doc` as a caller can name them, for an error about a layer that isn't there:
/// `the document's layers are 2 "Background", 3 "Layer 1"` (top-down, at most 20).
pub(crate) fn layer_listing(doc: &Document) -> String {
    const MAX: usize = 20;
    let layers = doc.walk();
    if layers.is_empty() {
        return "the document has no layers".into();
    }
    // Names are unbounded input: each one is cut at NAME characters so the error stays short.
    const NAME: usize = 64;
    let name = |n: &str| match n.char_indices().nth(NAME) {
        Some((cut, _)) => format!("{}…", &n[..cut]),
        None => n.to_string(),
    };
    let mut list: Vec<String> = layers.iter().rev().take(MAX).map(|(_, _, l)| format!("{} {:?}", l.id.0, name(&l.name))).collect();
    if layers.len() > MAX {
        list.push(format!("… {} more (see `document.inspect`)", layers.len() - MAX));
    }
    format!("the document's layers are {} (a `layer` param takes an id or a name)", list.join(", "))
}

/// The layer a command targets: [`explicit_layer`], else the active layer.
pub(crate) fn layer_param(s: &Session, p: &Value) -> Result<LayerId> {
    match explicit_layer(s, p)? {
        Some(id) => Ok(id),
        None => s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into())),
    }
}
pub fn blend_from_str(s: &str) -> Option<BlendMode> {
    let norm = |x: &str| x.to_ascii_lowercase().replace([' ', '_', '-', '(', ')'], "");
    let want = norm(s);
    std::iter::once(BlendMode::PassThrough).chain(BlendMode::layer_modes()).find(|m| norm(m.label()) == want || norm(&format!("{m:?}")) == want)
}

/// The active selection as the mask of a layer being created, or None without a selection.
/// Photoshop masks a new adjustment or fill layer with the selection, so the adjustment acts on
/// the selected area only (#1250); the selection itself stays, as Reveal Selection leaves it.
pub(crate) fn selection_mask(doc: &Document) -> Result<Option<LayerMask>> {
    doc.selection.as_ref().map(|sel| mask_at_depth(LayerMask { surface: sel.clone(), ..LayerMask::reveal_all() }, doc.depth)).transpose()
}

/// New pixel masks use the document's depth, as PSD saves/imports do. Otherwise a later soft
/// brush stroke quantizes differently depending on whether the document reopened as PSD or
/// native. Converting a selection keeps its coverage and does not alter the selection itself.
pub(crate) fn mask_at_depth(mut mask: LayerMask, depth: SampleType) -> Result<LayerMask> {
    let format = mask.surface.format().with_sample(depth);
    if mask.surface.format() != format {
        mask.surface = mask.surface.try_convert(format)?;
    }
    Ok(mask)
}

/// Options from the Layers Panel flyout live in preferences so headless/keyboard commands
/// behave exactly like pointer-triggered commands. Old preferences preserve Photoshop defaults.
pub(crate) fn layer_panel_option(s: &Session, key: &str, fallback: bool) -> bool {
    s.prefs().dialogs.get("ui.layersPanelOptions").and_then(|v| v.get(key)).and_then(Value::as_bool).unwrap_or(fallback)
}

/// Mask a fill or adjustment layer being created, as Photoshop does: the path selected in the
/// Paths panel (`"path"`: `"work"`, a saved path's name or a path object) becomes its vector
/// mask, which makes a Solid Color fill a shape (#1419); without one, the selection becomes its
/// layer mask ([`selection_mask`]); without either, a white mask reveals the whole layer (#1768).
/// A path that isn't there is an error.
pub(crate) fn mask_new_layer(doc: &Document, l: &mut Layer, p: &Value, cmd: &str, default_mask: bool) -> Result<()> {
    let path = match p.get("path") {
        None | Some(Value::Null) => None,
        Some(Value::String(n)) if crate::vector_cmds::is_work(Some(n)) => Some(doc.work_path.clone().ok_or_else(|| bad(cmd, "no work path"))?),
        Some(Value::String(n)) => {
            Some(doc.paths.iter().find(|sp| sp.name == *n).map(|sp| sp.path.clone()).ok_or_else(|| bad(cmd, format!("no path named \"{n}\"")))?)
        }
        Some(v) => Some(crate::vector_cmds::parse_path(v).map_err(|e| bad(cmd, e))?),
    };
    match path {
        Some(path) => l.vector_mask = Some(photocraft_doc::VectorMask::new(path)),
        None => {
            l.mask = selection_mask(doc)?.or_else(|| default_mask.then(LayerMask::reveal_all)).map(|mask| mask_at_depth(mask, doc.depth)).transpose()?;
        }
    }
    Ok(())
}

/// The `"path"` param of the new fill and adjustment layer commands ([`mask_new_layer`]).
pub(crate) const NEW_LAYER_PATH: &str =
    r##""path":"work"|saved path name|{…path}? (the active path: becomes the vector mask; else the selection is the layer mask, or white with no selection)"##;

/// Add an adjustment layer above the active one; it takes the mask `mask_new_layer` builds from `p` (a path, the selection, or the default white mask).
fn new_adjustment(s: &mut Session, adj: Adjustment, p: &Value, cmd: &str) -> Result<Value> {
    let label = format!("New {} Layer", adj.label());
    let name = adj.label().to_string();
    let id = s.edit(&label, |doc, active| {
        let mut l = Layer::new(doc.next_layer_name(&name), LayerContent::Adjustment(adj));
        mask_new_layer(doc, &mut l, p, cmd, true)?;
        let id = doc.insert_above(*active, l);
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({ "layer": id.0 }))
}

/// Apply `adj` in place to the targeted layer mask, the targeted alpha channel or Quick Mask, or else the active layer's pixels, as one history step named `label`.
fn destructive_adjust(s: &mut Session, label: &str, adj: Adjustment, p: &Value) -> Result<Value> {
    if is_mask_target(p) {
        // The targeted layer mask (#780): ⌘I inverts it, as in Photoshop.
        let id = layer_param(s, &Value::Null)?;
        return s.edit(label, |doc, _| {
            let sel = doc.selection.clone();
            let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            let mask = l.mask.as_mut().ok_or_else(|| EngineError::Other("layer has no mask".into()))?;
            pixels::adjust_mask(&mut mask.surface, &adj, sel.as_ref());
            Ok(Value::Null)
        });
    }
    if crate::channel_cmds::is_channel_target(p) {
        // Alpha channel / Quick Mask target: the adjustment runs on the grayscale channel as on
        // a layer mask, its unstored black included, so ⌘I in Quick Mask inverts it all (#6095).
        return s.edit(label, |doc, _| {
            let sel = doc.selection.clone();
            if let Some(surf) = crate::channel_cmds::channel_surface_for_filter(doc, None, p)? {
                pixels::adjust_mask(surf, &adj, sel.as_ref());
            }
            Ok(Value::Null)
        });
    }
    let id = layer_param(s, &Value::Null)?;
    s.edit(label, |doc, _| {
        let sel = doc.selection.clone();
        let mode = doc.mode;
        let surf = paint_surface(doc, id, &Value::Null)?;
        pixels::adjust_surface(surf, &adj, sel.as_ref(), mode);
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Lenient adjustment from params for previews: bad params fall back to the kind's defaults.
/// Commands use the checked [`crate::adjust_params::from_params`].
pub fn adjustment_from_params(kind: &str, p: &Value) -> Adjustment {
    crate::adjust_params::from_params(kind, p, None, ColorMode::Rgb)
        .or_else(|_| crate::adjust_params::default_for(kind, ColorMode::Rgb))
        .unwrap_or(Adjustment::Invert)
}

fn doc_mode(s: &Session) -> ColorMode {
    s.active().map_or(ColorMode::Rgb, |d| d.doc.mode)
}

/// The colour mode a destructive adjustment parses its tone space in: a targeted layer mask,
/// saved channel or Quick Mask holds gray levels whatever the document's mode, so Curves and
/// Levels stay in the neutral space instead of Lab (where the composite is ignored) or CMYK
/// (where the curve runs on the inks) (#5335).
fn adjust_mode(s: &Session, p: &Value) -> ColorMode {
    if is_mask_target(p) || crate::channel_cmds::is_channel_target(p) { ColorMode::Grayscale } else { doc_mode(s) }
}

/// Tone params for a gray plane target: the document-space channel records are dropped, because
/// those keys select the Lab or CMYK tone space and a plane only holds levels (#5335).
fn gray_tone_params(p: &Value) -> Value {
    match p {
        Value::Object(m) => {
            let mut o = Map::new();
            for (k, v) in m {
                if !matches!(k.as_str(), "cyan" | "magenta" | "yellow" | "black" | "lightness" | "a" | "b") {
                    o.insert(k.clone(), v.clone());
                }
            }
            Value::Object(o)
        }
        other => other.clone(),
    }
}

// ---------- the table ----------

#[allow(clippy::too_many_lines)]
fn build() -> Vec<CommandSpec> {
    let mut v = vec![
        // File
        cmd!(
            "file.new",
            "New…",
            ["File"],
            Some("Cmd+N"),
            r##"{"width":1..300000=1920,"height":1..300000=1080 (pixel counts are rounded),"mode":"rgb|gray|grayscale|cmyk|lab"="rgb","depth":8|16|32=8,"background":"white|black|backgroundColor|transparent|#rrggbb"="white","backgroundColor":[r,g,b]? (0..1; defaults to toolbox colour),"resolution":1..30000=72 (ppi),"name":str,"profile":"working"|built-in id|path to an .icc file="working" (the colour profile it is tagged with; it must suit the mode)}"##,
            always,
            |s, p| {
                use crate::document_preset_cmds::{MAX_DIMENSION, MAX_RESOLUTION, background_color, color_mode, sample_type};
                crate::document_preset_cmds::validate_new_params(s, p)?;
                // A size given as a float (`512.0`, as JSON from a UI field) is still that size (#254).
                let px = |k: &str, d: u32| match p.get(k) {
                    Some(v) => v
                        .as_u64()
                        .map(|n| n.clamp(1, MAX_DIMENSION as u64) as u32)
                        .or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f.round().clamp(1.0, MAX_DIMENSION as f64) as u32))
                        .unwrap_or(d),
                    None => d,
                };
                let (w, h) = (px("width", 1920), px("height", 1080));
                let mode = color_mode(p.get("mode").and_then(Value::as_str).unwrap_or("rgb")).unwrap_or(ColorMode::Rgb);
                let depth = sample_type(p.get("depth").and_then(Value::as_u64).unwrap_or(8)).unwrap_or(SampleType::U8);
                let name = p.get("name").and_then(Value::as_str).unwrap_or("Untitled").to_string();
                let res = p.get("resolution").and_then(Value::as_f64).unwrap_or(72.0).clamp(1.0, MAX_RESOLUTION) as f32;
                let mut doc = match p.get("background").and_then(Value::as_str).unwrap_or("white") {
                    "backgroundColor" => {
                        let bgc = background_color(p, s.tools.background, "file.new")?;
                        Document::with_background(name, Size::new(w, h), mode, depth, Color::rgba(bgc[0], bgc[1], bgc[2], 1.0))
                    }
                    "transparent" => {
                        let mut d = Document::new(name, Size::new(w, h), mode, depth);
                        d.layers.push(Layer::raster("Layer 1", d.pixel_format()));
                        d
                    }
                    "black" => Document::with_background(name, Size::new(w, h), mode, depth, Color::BLACK),
                    other => {
                        let c = parse_hex(other).unwrap_or([1.0; 4]);
                        Document::with_background(name, Size::new(w, h), mode, depth, Color::rgba(c[0], c[1], c[2], c[3]))
                    }
                };
                doc.resolution_dpi = res;
                // Photoshop's New Document › Color Profile: tag the document with another profile
                // than the working space (#321). Checked before the document opens.
                if let Some(spec) = p.get("profile").and_then(Value::as_str).filter(|spec| !matches!(*spec, "working" | "none")) {
                    let prof = s.color.resolve(spec, Some(&doc), None)?;
                    if prof.color_space != crate::color_cmds::mode_space(doc.mode) {
                        return Err(EngineError::BadParams {
                            cmd: "file.new".into(),
                            msg: format!("profile `{}` is {:?}; the document is {:?}", prof.description, prof.color_space, doc.mode),
                        });
                    }
                    doc.icc_profile = Some(prof.to_bytes());
                }
                let i = s.add_document(doc, None);
                Ok(json!({ "document": i }))
            }
        ),
        cmd!("file.close", "Close", ["File"], Some("Cmd+W"), r##"{"document":index?}"##, has_doc, |s, p| {
            let i = p.get("document").and_then(Value::as_u64).map(|v| v as usize).or(s.active_index()).ok_or(EngineError::NoDocument)?;
            s.close(i).ok_or(EngineError::NoDocument)?;
            Ok(Value::Null)
        }),
        // Edit
        cmd!("edit.undo", "Undo", ["Edit"], Some("Cmd+Z"), "{}", can_undo, |s, _| Ok(json!(s.undo()))),
        cmd!("edit.redo", "Redo", ["Edit"], Some("Cmd+Shift+Z"), "{}", can_redo, |s, _| Ok(json!(s.redo()))),
        cmd!(
            "edit.fill",
            "Fill…",
            ["Edit"],
            Some("Shift+F5"),
            r##"{"contents":"foreground|background|color|contentAware|pattern|history|black|gray|white"="color","color":"#rrggbb|[r,g,b,a]"=foreground (contents=color),"pattern":id|name (contents=pattern),"scale":%=100,"angle":deg,"state":row?,"snapshot":id? (contents=history: a History panel row as history.setSource, redo rows last, or a snapshot; default the History Brush source),"colorAdaptation":bool=true (contents=contentAware),"mode":"normal|multiply|…"="normal","opacity":0..100=100,"preserveTransparency":bool=false,"target":"pixels"|"mask"|"quickMask"|{"channel":i}?}"##,
            has_pixel_or_channel,
            crate::fill_cmds::fill
        ),
        cmd!("edit.clear", "Clear", ["Edit"], Some("Delete"), "{} (a targeted channel or Quick Mask is cleared alone)", has_pixel_or_channel, |s, p| {
            let bg = s.tools.background;
            // A targeted alpha channel or Quick Mask is filled instead (#2734).
            if let Some(t) = crate::channel_cmds::targeted_channel(s) {
                s.edit("Clear", |doc, _| crate::channel_cmds::clear_channel(doc, &t, bg))?;
                return Ok(Value::Null);
            }
            let id = layer_param(s, p)?;
            // A targeted colour channel clears that channel only (#2720).
            if let Some(k) = crate::channel_cmds::targeted_color(s) {
                s.edit("Clear", |doc, _| crate::channel_cmds::clear_color_channel(doc, id, k, bg))?;
                return Ok(Value::Null);
            }
            s.edit("Clear", |doc, _| {
                let sel = doc.selection.clone();
                let area = sel.as_ref().map(|m| m.content_bounds()).unwrap_or(doc.bounds());
                crate::edit_cmds::clear_area(doc, id, area, sel.as_ref(), bg)
            })?;
            Ok(Value::Null)
        }),
        // Select
        cmd!("select.all", "All", ["Select"], Some("Cmd+A"), "{}", has_doc, |s, _| {
            s.edit("Select All", |doc, _| {
                let mut m = Surface::new(photocraft_color::PixelFormat::GRAY8);
                m.fill_rect(doc.bounds(), &[1.0]);
                doc.selection = Some(m);
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!("select.deselect", "Deselect", ["Select"], Some("Cmd+D"), "{}", has_selection, |s, _| {
            // Nothing selected: nothing to do and no history step, as in Photoshop (#6744).
            if s.active().is_none_or(|d| d.doc.selection.is_none()) {
                return Ok(Value::Null);
            }
            s.edit("Deselect", |doc, _| {
                doc.selection = None;
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!("select.inverse", "Inverse", ["Select"], Some("Cmd+Shift+I"), "{}", has_selection, |s, _| {
            s.edit("Inverse", |doc, _| {
                let old = doc.selection.take().unwrap_or_else(|| Surface::new(photocraft_color::PixelFormat::GRAY8));
                let b = doc.bounds();
                let data: Vec<f32> = old.read_region(b).into_iter().map(|v| 1.0 - v).collect();
                let mut m = Surface::new(photocraft_color::PixelFormat::GRAY8);
                m.write_region(b, &data);
                m.prune();
                doc.selection = Some(m);
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!(
            "select.rect",
            "Rectangular Selection",
            [],
            None,
            r##"{"x":i32,"y":i32,"width":u32,"height":u32,"mode":"replace|add|subtract|intersect"="replace","ellipse":bool=false,"line":"row|column"?,"antiAlias":bool=true,"feather":px=0}"##,
            has_doc,
            |s, p| {
                let get = |k: &str| int_i32("select.rect", p, k).and_then(|v| v.ok_or_else(|| bad("select.rect", format!("missing `{k}`"))));
                let r = Rect::from_xywh(get("x")?, get("y")?, get("width")?.max(0) as u32, get("height")?.max(0) as u32);
                let mode = p.get("mode").and_then(Value::as_str).unwrap_or("replace").to_string();
                let ellipse = p.get("ellipse").and_then(Value::as_bool).unwrap_or(false);
                // The History step is named after the tool: the Single Row / Single Column
                // Marquee send `line` (Photoshop).
                let step = match p.get("line").and_then(Value::as_str) {
                    Some("row") => "Single Row Marquee",
                    Some("column") => "Single Column Marquee",
                    Some(other) => return Err(bad("select.rect", format!("unknown `line` {other:?} (row|column)"))),
                    None if ellipse => "Elliptical Marquee",
                    None => "Rectangular Marquee",
                };
                // Options bar: anti-aliased ellipse edges (4x4 supersampled) and Feather (applied to the new shape only).
                let aa = p.get("antiAlias").and_then(Value::as_bool).unwrap_or(true);
                let feather = p.get("feather").and_then(Value::as_f64).unwrap_or(0.0).clamp(0.0, 1000.0) as f32;
                s.edit(step, |doc, _| {
                    let area = doc.bounds();
                    // A marquee dragged past the canvas stops at its edge (Photoshop); the ellipse
                    // keeps the dragged shape and is only cut there.
                    use photocraft_algo::selection as sel;
                    let shape = if ellipse {
                        sel::ellipse_surface(r, area, aa)
                    } else {
                        let mut s = Surface::new(photocraft_color::PixelFormat::GRAY8);
                        s.fill_rect(r.intersect(&area), &[1.0]);
                        s
                    };
                    if feather > 0.0 {
                        let m = sel::feather(&sel::mask_from_surface(Some(&shape), area), area.width() as usize, area.height() as usize, feather);
                        doc.selection = sel::combine(doc.selection.as_ref(), &m, area, sel::SelectionMode::parse(&mode));
                        return Ok(());
                    }
                    let old = doc.selection.take();
                    let combined = match (mode.as_str(), old) {
                        ("add", Some(o)) => sel::combine_surfaces(&o, &shape, area, |a, b| a.max(b)),
                        ("subtract", Some(o)) => sel::combine_surfaces(&o, &shape, area, |a, b| a * (1.0 - b)),
                        ("intersect", Some(o)) => sel::combine_surfaces(&o, &shape, area, |a, b| a.min(b)),
                        // With no existing selection, neither operation can select new pixels.
                        ("subtract" | "intersect", None) => Surface::new(photocraft_color::PixelFormat::GRAY8),
                        _ => shape,
                    };
                    // Nothing selected on the canvas (e.g. dragged entirely outside it) deselects.
                    doc.selection = (!combined.content_bounds().is_empty()).then_some(combined);
                    Ok(())
                })?;
                Ok(Value::Null)
            }
        ),
        // Layer
        cmd!(
            "layer.new.layer",
            "Layer…",
            ["Layer", "New"],
            Some("Cmd+Shift+N"),
            r##"{"name":str?,"clipToPrevious":bool=false (Use Previous Layer to Create Clipping Mask; inside a clip group the layer joins it anyway),"color":"none|red|orange|yellow|green|seafoam|blue|indigo|magenta|fuchsia|violet|gray"="none","mode":"Normal|Multiply|…"="Normal","opacity":0..1=1,"fillNeutral":bool=false (fill with the mode's neutral color: modes without one are an error),"position":"above|below"="above" (below: the Layers panel's Cmd/Ctrl-click; never under the Background)}"##,
            has_doc,
            crate::new_layer::run
        ),
        cmd!("layer.new.group", "Group…", ["Layer", "New"], None, r##"{"name":str?}"##, has_doc, |s, p| {
            let id = s.edit("New Group", |doc, active| {
                let name = p.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| doc.next_layer_name("Group"));
                let id = doc.insert_above(*active, Layer::group(name, vec![]));
                *active = Some(id);
                Ok(id)
            })?;
            crate::layer_multi_cmds::note_inert_insert(s, id);
            Ok(json!({ "layer": id.0 }))
        }),
        cmd!(
            "layer.groupLayers",
            "Group Layers",
            ["Layer"],
            Some("Cmd+G"),
            r##"{"layer":id?,"name":str?} (no layer: every selected layer)"##,
            has_layer,
            crate::layer_multi_cmds::group_layers
        ),
        cmd!(
            "layer.duplicate",
            "Duplicate Layer…",
            ["Layer"],
            None,
            r##"{"layer":id?,"inPlace":bool=false (artboards: keep the copy on the original instead of beside it),"name":str? (one layer: the copy's name),"document":index|"new"=same document (Photoshop's Destination: "new" makes a new document the size of this canvas holding just the copies),"documentName":str? (the new document's name)} (no layer: every selected layer)"##,
            has_layer,
            |s, p| {
                if let Some(d) = p.get("document") {
                    if d.as_str() == Some("new") {
                        return crate::layer_copy_cmds::duplicate_to_new_document(s, p);
                    }
                    let same = d.as_u64().is_some_and(|i| s.active_index().is_some_and(|a| i == a as u64));
                    if !same {
                        return Err(EngineError::BadParams {
                            cmd: "layer.duplicate".into(),
                            msg: if d.as_u64().is_some() {
                                "the destination is another open document (use layer.copyToDocument)".into()
                            } else {
                                "`document` must be a document index or \"new\"".into()
                            },
                        });
                    }
                }
                let in_place = p.get("inPlace").and_then(Value::as_bool).unwrap_or(false);
                let name = p.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string);
                if crate::layer_multi_cmds::multi(s, p) {
                    return crate::layer_multi_cmds::duplicate_selected(s, in_place, "Duplicate Layers");
                }
                let id = layer_param(s, p)?;
                let add_copy = layer_panel_option(s, "addCopyToCopiedLayersAndGroups", true);
                let nid = s.edit("Duplicate Layer", |doc, active| {
                    let mut dup = layer_copy(doc, id, add_copy)?;
                    if let Some(name) = &name {
                        dup.name.clone_from(name);
                    }
                    let nid = doc.insert_above(Some(id), dup);
                    if !in_place {
                        crate::artboard_cmds::place_copy(doc, nid)?;
                    }
                    *active = Some(nid);
                    Ok(nid)
                })?;
                Ok(json!({ "layer": nid.0 }))
            }
        ),
        cmd!("layer.delete", "Delete Layer", ["Layer", "Delete"], None, r##"{"layer":id?} (no layer: every selected layer)"##, has_layer, |s, p| {
            if crate::layer_multi_cmds::multi(s, p) {
                return crate::layer_multi_cmds::delete_selected(s);
            }
            let id = layer_param(s, p)?;
            s.edit("Delete Layer", |doc, active| {
                let neighbours = crate::layer_multi_cmds::deletion_neighbours(doc, id);
                let before = crate::clip_group::Before::read(doc, &[id]);
                doc.remove(id).ok_or(EngineError::NoLayer(id))?;
                crate::clip_group::settle(doc, &[], &before, false);
                if doc.layers.is_empty() {
                    return Err(EngineError::Other("a document must keep at least one layer".into()));
                }
                if active.is_some_and(|current| doc.layer(current).is_none()) {
                    *active = neighbours.into_iter().find(|candidate| doc.layer(*candidate).is_some());
                }
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!(
            "layer.select",
            "Select Layer",
            [],
            None,
            r##"{"layer":id|str | "name":str,"occurrence":index?,"mode":"replace|toggle|range|add|remove"="replace"} (name occurrence: zero-based document walk order; required for duplicate names; toggle = ⌘-click, range = ⇧-click, remove = deselect it, keeping at least one)"##,
            has_doc,
            crate::layer_multi_cmds::select
        ),
        cmd!(
            "layer.setProps",
            "Layer Properties",
            [],
            None,
            r##"{"layer":id?,"layers":[id,…]? (the same change to each, one step: the Layers panel's controls with several layers selected),"name":str? (one layer only),"visible":bool?,"opacity":0..1?,"fill":0..1?,"blend":"Multiply|…"?,"clipped":bool?,"locked":bool?,"locks":{"transparency","pixels","position","artboard","all":bool}?,"channels":[bool,…]? (Advanced Blending: which colour channels blend, R G B / C M Y K / L a b)}"##,
            has_layer,
            |s, p| {
                // Several layers at once (the Layers panel with several selected, as in Photoshop),
                // else the given or active one.
                let ids: Vec<LayerId> = match p.get("layers") {
                    None | Some(Value::Null) => vec![layer_param(s, p)?],
                    Some(Value::Array(a)) if !a.is_empty() && a.len() <= 10_000 => {
                        let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
                        let mut ids = Vec::with_capacity(a.len());
                        for v in a {
                            let id = v.as_u64().map(LayerId).ok_or_else(|| bad("layer.setProps", format!("`layers` holds layer ids, got {v}")))?;
                            if doc.layer(id).is_none() {
                                return Err(EngineError::NoLayer(id));
                            }
                            if !ids.contains(&id) {
                                ids.push(id);
                            }
                        }
                        ids
                    }
                    Some(v) => return Err(bad("layer.setProps", format!("`layers` is a non-empty list of layer ids, got {v}"))),
                };
                if ids.len() > 1 && p.get("name").is_some() {
                    return Err(bad("layer.setProps", "`name` renames one layer, not several"));
                }
                // Opacity and Fill are fractions here, while the Layer Style commands take
                // percentages: a value outside 0..1 is refused, never clamped to 100 % (#5242).
                let unit = |key: &str| -> Result<Option<f32>> {
                    let Some(v) = p.get(key) else { return Ok(None) };
                    match v.as_f64() {
                        Some(x) if (0.0..=1.0).contains(&x) => Ok(Some(x as f32)),
                        _ => Err(bad("layer.setProps", format!("{key} must be a number in 0..1 (1 = 100 %), got {v}"))),
                    }
                };
                let (opacity, fill) = (unit("opacity")?, unit("fill")?);
                // Photoshop's History names for a single kind of change; a mix is "Layer Properties".
                let changed: Vec<&str> = p
                    .as_object()
                    .map(|o| o.keys().map(String::as_str).filter(|k| !matches!(*k, "layer" | "layers" | "coalesce")).collect())
                    .unwrap_or_default();
                let label = match changed.as_slice() {
                    ["visible"] => "Layer Visibility",
                    ["opacity"] => "Opacity Change",
                    ["fill"] => "Fill Opacity Change",
                    ["blend"] => "Blending Change",
                    ["name"] => "Rename Layer",
                    _ => "Layer Properties",
                };
                let before = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
                // Clipping and channel changes reach the layers around it: recomposite everything.
                let local = p.get("clipped").is_none() && p.get("channels").is_none();
                s.edit(label, |doc, _| {
                    // The Background has no opacity, fill or blend mode: like Photoshop, setting
                    // one that changes its look turns it into a normal layer ("Layer 0") first.
                    let num = |k: &str| p.get(k).and_then(Value::as_f64).is_some_and(|v| v < 1.0);
                    let blends = p.get("blend").and_then(Value::as_str).and_then(blend_from_str).is_some_and(|b| b != photocraft_color::BlendMode::Normal);
                    for &id in &ids {
                        if num("opacity") || num("fill") || blends {
                            crate::extra_cmds::background_to_layer(doc, id);
                        }
                        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
                        if let Some(v) = p.get("name").and_then(Value::as_str) {
                            l.name = v.to_string();
                        }
                        if let Some(v) = p.get("visible").and_then(Value::as_bool) {
                            l.visible = v;
                        }
                        if let Some(v) = opacity {
                            l.opacity = v;
                        }
                        if let Some(v) = fill {
                            l.fill_opacity = v;
                        }
                        if let Some(v) = p.get("blend").and_then(Value::as_str) {
                            let b = blend_from_str(v).ok_or_else(|| bad("layer.setProps", format!("unknown blend mode `{v}`")))?;
                            // Pass Through is a group's mode only, as in Photoshop.
                            if b == photocraft_color::BlendMode::PassThrough && !l.is_group() {
                                return Err(bad("layer.setProps", format!("Pass Through applies to groups only; layer {} isn't one", l.id.0)));
                            }
                            l.blend = b;
                        }
                        if let Some(v) = p.get("clipped").and_then(Value::as_bool) {
                            l.clipped = v;
                        }
                        if let Some(v) = p.get("locked").and_then(Value::as_bool) {
                            l.locks.all = v;
                        }
                        if let Some(Value::Array(ch)) = p.get("channels") {
                            // Blending Options › Advanced Blending › Channels: unchecked = left out.
                            l.excluded_channels = ch.iter().take(32).enumerate().filter(|(_, v)| v.as_bool() == Some(false)).fold(0, |m, (i, _)| m | 1 << i);
                        }
                        if let Some(Value::Object(m)) = p.get("locks") {
                            for (k, v) in m {
                                let v = v.as_bool().unwrap_or(false);
                                if !l.locks.set(k, v) {
                                    return Err(bad("layer.setProps", format!("unknown lock `{k}`")));
                                }
                            }
                        }
                    }
                    Ok(())
                })?;
                if local {
                    crate::layer_multi_cmds::note_damage(s, &before, &ids);
                }
                Ok(Value::Null)
            }
        ),
        cmd!("layer.arrange.bringForward", "Bring Forward", ["Layer", "Arrange"], Some("Cmd+]"), r##"{"layer":id?}"##, has_layer, |s, p| arrange(s, p, 1)),
        cmd!("layer.arrange.sendBackward", "Send Backward", ["Layer", "Arrange"], Some("Cmd+["), r##"{"layer":id?}"##, has_layer, |s, p| arrange(s, p, -1)),
        cmd!("layer.arrange.bringToFront", "Bring to Front", ["Layer", "Arrange"], Some("Cmd+Shift+]"), r##"{"layer":id?}"##, has_layer, |s, p| arrange(
            s,
            p,
            i32::MAX
        )),
        cmd!("layer.arrange.sendToBack", "Send to Back", ["Layer", "Arrange"], Some("Cmd+Shift+["), r##"{"layer":id?}"##, has_layer, |s, p| arrange(
            s,
            p,
            i32::MIN
        )),
        cmd!(
            "layer.createClippingMask",
            "Create Clipping Mask",
            ["Layer"],
            Some("Cmd+Alt+G"),
            r##"{"layer":id?} (no layer: every selected layer but the lowest clips to the layer below it)"##,
            has_layer,
            |s, p| set_clipped(s, p, true)
        ),
        cmd!(
            "layer.releaseClippingMask",
            "Release Clipping Mask",
            ["Layer"],
            None,
            r##"{"layer":id?} (no layer: every selected layer)"##,
            has_layer,
            |s, p| set_clipped(s, p, false)
        ),
        cmd!(
            "layer.layerMask.revealAll",
            "Reveal All",
            ["Layer", "Layer Mask"],
            None,
            r##"{"layer":id?} (no layer: every selected layer that can take a mask)"##,
            has_layer,
            |s, p| set_mask(s, p, "Add Layer Mask", Some(LayerMask::reveal_all()))
        ),
        cmd!(
            "layer.layerMask.hideAll",
            "Hide All",
            ["Layer", "Layer Mask"],
            None,
            r##"{"layer":id?} (no layer: every selected layer that can take a mask)"##,
            has_layer,
            |s, p| set_mask(s, p, "Add Layer Mask", Some(LayerMask::hide_all()))
        ),
        cmd!(
            "layer.layerMask.revealSelection",
            "Reveal Selection",
            ["Layer", "Layer Mask"],
            None,
            r##"{"layer":id?} (no layer: every selected layer that can take a mask)"##,
            has_selection,
            |s, p| {
                let sel = s.active().and_then(|d| d.doc.selection.clone()).ok_or(EngineError::Other("no selection".into()))?;
                let mask = LayerMask { surface: sel, ..LayerMask::reveal_all() };
                // The selection becomes the mask, so Photoshop drops it (as Hide Selection does): a
                // following Invert (⌘I) on the targeted mask then inverts all of it, not just the
                // part inside the old selection. Same undo step as the mask.
                set_mask_and_deselect(s, p, "Add Layer Mask", mask)
            }
        ),
        cmd!("layer.layerMask.delete", "Delete", ["Layer", "Layer Mask"], None, r##"{"layer":id?}"##, has_layer, |s, p| set_mask(
            s,
            p,
            "Delete Layer Mask",
            None
        )),
        cmd!("layer.mergeDown", "Merge Down", ["Layer"], None, r##"{"layer":id?}"##, has_layer, |s, p| {
            let id = layer_param(s, p)?;
            s.edit("Merge Down", |doc, active| {
                let path = doc.path_of(id).ok_or(EngineError::NoLayer(id))?;
                let Some((&idx, parent)) = path.split_last() else { return Err(EngineError::NoLayer(id)) };
                if idx == 0 {
                    return Err(EngineError::Other("no layer below to merge into".into()));
                }
                let mut below = parent.to_vec();
                below.push(idx - 1);
                let lower = doc.layer_at(&below).ok_or(EngineError::NoLayer(id))?.clone();
                let upper = doc.layer(id).ok_or(EngineError::NoLayer(id))?.clone();
                let merged = pixels::merge_down(doc, &lower, &upper);
                doc.remove(id);
                *doc.layer_at_mut(&below).ok_or(EngineError::NoLayer(lower.id))? = merged;
                *active = Some(lower.id);
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!("layer.mergeGroup", "Merge Group", ["Layer"], None, r##"{"layer":id?}"##, has_layer, |s, p| {
            let id = layer_param(s, p)?;
            if !s.active().and_then(|d| d.doc.layer(id)).is_some_and(|l| matches!(l.content, LayerContent::Group(_))) {
                return Err(EngineError::Other("Merge Group needs a group".into()));
            }
            merge_group(s, id)
        }),
        cmd!("layer.flattenImage", "Flatten Image", ["Layer"], None, "{}", has_doc, |s, _| {
            s.edit("Flatten Image", |doc, active| {
                let fmt = doc.pixel_format();
                let mut bg = Layer::raster("Background", fmt);
                bg.locks.transparency = true;
                bg.locks.position = true;
                *crate::pixels_mut(&mut bg)? = photocraft_compose::flatten_to_surface(doc, fmt, Some([1.0, 1.0, 1.0]));
                *active = Some(bg.id);
                doc.layers = vec![bg];
                Ok(())
            })?;
            Ok(Value::Null)
        }),
        cmd!(
            "layer.newFillLayer.solidColor",
            "Solid Color…",
            ["Layer", "New Fill Layer"],
            None,
            r##"{"color":"#rrggbb"=foreground,"path":"work"|saved path name|{…path}? (the active path: becomes the vector mask, making a shape; else the selection is the layer mask)}"##,
            has_doc,
            |s, p| {
                let c = color_param("layer.newFillLayer.solidColor", p, "color", s.tools.foreground)?;
                let default_mask = layer_panel_option(s, "useDefaultMasksOnFillLayers", true);
                let id = s.edit("New Color Fill Layer", |doc, active| {
                    let fill = Fill::Solid(Color::rgba(c[0], c[1], c[2], c[3])).in_mode(doc.mode);
                    let mut l = Layer::new(doc.next_layer_name("Color Fill"), LayerContent::Fill(fill));
                    mask_new_layer(doc, &mut l, p, "layer.newFillLayer.solidColor", default_mask)?;
                    let id = doc.insert_above(*active, l);
                    *active = Some(id);
                    Ok(id)
                })?;
                Ok(json!({ "layer": id.0 }))
            }
        ),
        cmd!(
            "layer.newFillLayer.gradient",
            "Gradient…",
            ["Layer", "New Fill Layer"],
            None,
            r##"{"from":"#rrggbb","to":"#rrggbb","angle":deg=90,"style":"linear|radial|angle|reflected|diamond","reverse":bool,"path":"work"|saved path name|{…path}? (the active path: becomes the vector mask; else the selection is the layer mask)}"##,
            has_doc,
            |s, p| {
                let a = color_param("layer.newFillLayer.gradient", p, "from", s.tools.foreground)?;
                let b = color_param("layer.newFillLayer.gradient", p, "to", s.tools.background)?;
                let angle = crate::gradient_fill_cmds::angle_param("layer.newFillLayer.gradient", p, 90.0)?;
                let style = crate::layer_style::gradient_style(p.get("style").and_then(Value::as_str).unwrap_or("linear"));
                let reverse = p.get("reverse").and_then(Value::as_bool).unwrap_or(false);
                let default_mask = layer_panel_option(s, "useDefaultMasksOnFillLayers", true);
                let id = s.edit("New Gradient Fill Layer", |doc, active| {
                    let fill = Fill::gradient(
                        vec![(0.0, Color::rgba(a[0], a[1], a[2], a[3])), (1.0, Color::rgba(b[0], b[1], b[2], b[3]))],
                        angle,
                        1.0,
                        style,
                        reverse,
                    )
                    .in_mode(doc.mode);
                    let mut l = Layer::new(doc.next_layer_name("Gradient Fill"), LayerContent::Fill(fill));
                    mask_new_layer(doc, &mut l, p, "layer.newFillLayer.gradient", default_mask)?;
                    let id = doc.insert_above(*active, l);
                    *active = Some(id);
                    Ok(id)
                })?;
                Ok(json!({ "layer": id.0 }))
            }
        ),
        cmd!(
            "layer.setAdjustment",
            "Adjustment Properties",
            [],
            None,
            r##"{"layer":id?, …params of that adjustment kind} (Levels, Curves: "eyedropper":{"point":"black|gray|white","at":[x,y],"size":1..101=1} or {"point",…,"color":[r,g,b] 0..1} = the Properties panel's Set Black/Neutral Gray/White Point, sampling the image below the layer, RGB and Grayscale (no gray) only)"##,
            has_layer,
            |s, p| {
                let id = layer_param(s, p)?;
                let eyedropped = crate::adjust_params::layer_eyedropper(s, id, p)?;
                s.edit("Modify Adjustment", |doc, _| {
                    let mode = doc.mode;
                    let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
                    let LayerContent::Adjustment(adj) = &mut l.content else { return Err(EngineError::Other("not an adjustment layer".into())) };
                    if matches!(adj, Adjustment::Unsupported { .. }) {
                        return Err(EngineError::Other("this adjustment can't be edited (kept as imported)".into()));
                    }
                    let kind = adjustment_kind(adj);
                    // No params resets to the defaults; otherwise params merge over the current values.
                    *adj = if let Some(eyedropped) = eyedropped {
                        eyedropped
                    } else if crate::adjust_params::user_keys(p).is_empty() {
                        crate::adjust_params::default_for(kind, mode)?
                    } else {
                        crate::adjust_params::from_params(kind, p, Some(adj), mode)?
                    };
                    Ok(())
                })?;
                Ok(Value::Null)
            }
        ),
        cmd!(
            "layer.moveTo",
            "Reorder Layer",
            [],
            None,
            r##"{"layer":id?| "layers":[id,…]?, "target":id,"position":"above|below|into"="above","copy":bool=false} (layers: one undoable move, document order preserved; copy: place duplicates there and leave the layers, as ⌥-dragging a Layers panel row)"##,
            has_layer,
            crate::layer_multi_cmds::move_to
        ),
        cmd!(
            "layer.translate",
            "Move Layer",
            [],
            None,
            r##"{"layer":id?,"dx":i32,"dy":i32,"units":"px"|"pt"="px" (pt: points, whole pixels at the document's resolution),"target":"pixels"|"mask"="pixels","selection":bool=false} (no layer: every selected layer; linked layers follow; target mask: a layer whose mask is unlinked moves that mask alone, a linked mask moves with the layer; selection: the selection moves too, in the same step)"##,
            has_layer,
            crate::layer_multi_cmds::translate
        ),
        // Image
        cmd!("image.imageRotation.flipCanvasHorizontal", "Flip Canvas Horizontal", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Flip Canvas Horizontal", crate::canvas_geom::Turn::FlipHorizontal)
        }),
        cmd!("image.imageRotation.flipCanvasVertical", "Flip Canvas Vertical", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Flip Canvas Vertical", crate::canvas_geom::Turn::FlipVertical)
        }),
        cmd!("image.imageRotation.180", "180°", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Rotate Canvas", crate::canvas_geom::Turn::Rotate180)
        }),
        cmd!("image.imageRotation.90cw", "90° Clockwise", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Rotate Canvas", crate::canvas_geom::Turn::Cw90)
        }),
        cmd!("image.imageRotation.90ccw", "90° Counter Clockwise", ["Image", "Image Rotation"], None, "{}", has_doc, |s, _| {
            turn(s, "Rotate Canvas", crate::canvas_geom::Turn::Ccw90)
        }),
        cmd!("image.adjustments.desaturate", "Desaturate", ["Image", "Adjustments"], Some("Cmd+Shift+U"), "{}", has_pixel_layer, |s, _| {
            destructive_adjust(
                s,
                "Desaturate",
                Adjustment::HueSaturation {
                    hue: 0.0,
                    saturation: -100.0,
                    lightness: 0.0,
                    colorize: false,
                    ranges: photocraft_doc::adjust::HueRange::defaults(),
                },
                &Value::Null,
            )
        }),
        // Paint
        cmd!(
            "paint.stroke",
            "Brush Stroke",
            [],
            None,
            r##"{"points":[[x,y,pressure?,tiltX?,tiltY?,rotation?,timeMs?,wheel?],…],"brush":{…BrushSettings}?,"preset":name?,"size":0.5..5000 px?,"hardness":0..1?,"opacity":0..1?,"flow":0..1?,"spacing":0..10?,"color":"#rrggbb"?=foreground,"mode":"normal|multiply|screen|…|clear (erases with this brush)"="normal","erase":bool?,"smoothing":0..1?,"zoom":number=1,"seed":u64?,"target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target}"##,
            has_paintable,
            crate::brush_cmds::paint_stroke
        ),
        cmd!(
            "tools.setColors",
            "Set Colors",
            [],
            None,
            r##"{"foreground":"#rrggbb"|[r,g,b,a?]?,"background":same?,"foregroundCmyk":[c,m,y,k]? (inks 0..1 the foreground was picked as: a CMYK document is filled with exactly these),"backgroundCmyk":same?}"##,
            always,
            |s, p| {
                let cmyk = |key: &str| -> Result<Option<[f32; 4]>> {
                    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
                    let inks: Option<Vec<f32>> = v
                        .as_array()
                        .filter(|a| a.len() == 4)
                        .and_then(|a| a.iter().map(|x| x.as_f64().filter(|x| x.is_finite()).map(|x| x.clamp(0.0, 1.0) as f32)).collect());
                    match inks.as_deref() {
                        Some(&[c, m, y, k]) => Ok(Some([c, m, y, k])),
                        _ => Err(bad("tools.setColors", format!("`{key}` must be [c, m, y, k] numbers 0..1"))),
                    }
                };
                let (fg_cmyk, bg_cmyk) = (cmyk("foregroundCmyk")?, cmyk("backgroundCmyk")?);
                let foreground = color_param("tools.setColors", p, "foreground", s.tools.foreground)?;
                let background = color_param("tools.setColors", p, "background", s.tools.background)?;
                s.tools.foreground = foreground;
                s.tools.background = background;
                if let Some(k) = fg_cmyk {
                    let [r, g, b, _] = s.tools.foreground;
                    s.tools.remember_cmyk([r, g, b], k);
                }
                if let Some(k) = bg_cmyk {
                    let [r, g, b, _] = s.tools.background;
                    s.tools.remember_cmyk([r, g, b], k);
                }
                Ok(Value::Null)
            }
        ),
        cmd!("tools.swapColors", "Switch Foreground and Background Colors", [], Some("X"), "{}", always, |s, _| {
            std::mem::swap(&mut s.tools.foreground, &mut s.tools.background);
            Ok(Value::Null)
        }),
        cmd!("tools.defaultColors", "Default Foreground and Background Colors", [], Some("D"), "{}", always, |s, _| {
            if s.tools.mask_targeted {
                s.tools.foreground = crate::ToolState::MASK_COLORS[0];
                s.tools.background = crate::ToolState::MASK_COLORS[1];
            } else {
                s.tools.foreground = [0.0, 0.0, 0.0, 1.0];
                s.tools.background = [1.0, 1.0, 1.0, 1.0];
            }
            Ok(Value::Null)
        }),
        // Queries (not journaled)
        CommandSpec {
            id: "session.inspect",
            label: "Inspect Session",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |s, _| Ok(inspect::session(s)),
            journal: false,
        },
        CommandSpec {
            id: "document.inspect",
            label: "Inspect Document",
            menu: &[],
            shortcut: None,
            params: r##"{"document":index?}"##,
            enabled: has_doc,
            run: |s, p| {
                let d = match p.get("document") {
                    Some(v) => {
                        let i = v.as_u64().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad("document.inspect", "`document` must be an index"))?;
                        s.documents().get(i).ok_or_else(|| EngineError::Other(format!("no document at index {i}")))?
                    }
                    None => s.active().ok_or(EngineError::NoDocument)?,
                };
                Ok(inspect::document(d))
            },
            journal: false,
        },
        CommandSpec {
            id: "document.activate",
            label: "Activate Document",
            menu: &[],
            shortcut: None,
            params: r##"{"document":index} or {"offset":int} (the open document that many tabs after the active one, before it when negative: a Photoshop action's switch to another document) → {document}"##,
            enabled: has_doc,
            run: |s, p| {
                // A null key counts as omitted.
                let given = |k: &str| p.get(k).filter(|v| !v.is_null());
                let idx = match (given("document"), given("offset")) {
                    (Some(_), Some(_)) => return Err(bad("document.activate", "give `document` or `offset`, not both")),
                    (None, None) => return Err(bad("document.activate", "needs `document` (a tab index) or `offset`")),
                    // Past the first or last tab is refused rather than wrapped, so an action that
                    // copies from one document never silently pastes into the wrong one (#7103).
                    (None, Some(o)) => {
                        let o = o.as_i64().ok_or_else(|| bad("document.activate", "`offset` must be an integer"))?;
                        let (active, len) = (s.active_index().ok_or(EngineError::NoDocument)?, s.documents().len());
                        let target = i64::try_from(active).ok().and_then(|a| a.checked_add(o)).and_then(|i| usize::try_from(i).ok());
                        let Some(i) = target.filter(|&i| i < len) else {
                            let side = if o < 0 { "before" } else { "after" };
                            let msg = format!("no document {} {side} the active one, which is {} of {len}", o.unsigned_abs(), active + 1);
                            return Err(EngineError::Other(msg));
                        };
                        i
                    }
                    (Some(d), None) => {
                        let i = d.as_u64().ok_or_else(|| bad("document.activate", "`document` must be a tab index"))?;
                        usize::try_from(i).map_err(|_| bad("document.activate", "`document` out of range"))?
                    }
                };
                if s.set_active(idx) { Ok(json!({"document": idx})) } else { Err(EngineError::Other(format!("no document at index {idx}"))) }
            },
            journal: false,
        },
        CommandSpec {
            id: "document.move",
            label: "Move Document",
            menu: &[],
            shortcut: None,
            params: r##"{"document":index?,"to":index}"##,
            enabled: has_doc,
            run: |s, p| {
                let index = |key: &str| {
                    p.get(key)
                        .map(|v| v.as_u64().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad("document.move", format!("`{key}` must be a tab index"))))
                };
                let from = match index("document") {
                    Some(i) => i?,
                    None => s.active_index().ok_or(EngineError::NoDocument)?,
                };
                let to = index("to").ok_or_else(|| bad("document.move", "missing `to`"))??;
                let to = s.move_document(from, to).ok_or(EngineError::NoDocument)?;
                Ok(json!({"document": to}))
            },
            journal: false,
        },
        CommandSpec {
            id: "command.list",
            label: "List Commands",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |s, _| Ok(Value::Array(command_specs().iter().map(|c| list_entry(s, c, (c.enabled)(s).err())).collect())),
            journal: false,
        },
        CommandSpec {
            id: "document.pixel",
            label: "Read Composite Pixel",
            menu: &[],
            shortcut: None,
            params: r##"{"x":i32,"y":i32}"##,
            enabled: has_doc,
            run: |s, p| {
                let coord = |k: &str| i32::try_from(int(p, k).unwrap_or(0)).map_err(|_| bad("document.pixel", format!("{k} must fit in 32 bits")));
                let (x, y) = (coord("x")?, coord("y")?);
                let d = s.active().ok_or(EngineError::NoDocument)?;
                // At i32::MAX the 1x1 rect saturates to empty: a pixel that far out is transparent.
                let px = photocraft_compose::render(&d.doc, Rect::from_xywh(x, y, 1, 1)).px.first().copied().unwrap_or_default();
                Ok(json!(px))
            },
            journal: false,
        },
    ];

    // Adjustment layers + destructive adjustments, generated from one list.
    // Levels/Curves channel keys follow the document: red/green/blue (RGB), gray (Grayscale),
    // cyan/magenta/yellow/black (CMYK), lightness/a/b (Lab); see `adjust_params`.
    const ADJ: &[(&str, &str, &str)] = &[
        ("brightnessContrast", "Brightness/Contrast…", r##"{"brightness":-150..150=0,"contrast":-50..100=0,"legacy":bool=false}"##),
        (
            "levels",
            "Levels…",
            r##"{"inBlack":0..253=0,"gamma":0.01..9.99=1,"inWhite":2..255=255,"outBlack":0..255=0,"outWhite":0..255=255,"red":json,"green":json,"blue":json,"eyedropper":json} (top level = composite; per channel {"inBlack","gamma","inWhite","outBlack","outWhite"} under red/green/blue, gray, cyan/magenta/yellow/black or lightness/a/b; eyedropper = the dialog's Set Black/Neutral Gray/White Point picker {"point":"black|gray|white","at":[x,y],"size":1..101=1} or {"point",…,"color":[r,g,b] 0..1}: black/white set each channel's input black/white to the sample, gray each channel's gamma so the sample turns neutral at its luma; RGB and Grayscale (no gray) only, applied over the given levels)"##,
        ),
        (
            "curves",
            "Curves…",
            r##"{"points":json,"red":json,"green":json,"blue":json,"pencil":bool=false,"smooth":0..10=0,"eyedropper":json} (curves as [[in,out],…] in 0..255, 2..19 points: points = composite; red/green/blue, gray, cyan/magenta/yellow/black or lightness/a/b per channel; pencil = drawn curves (Draw to modify the curve): each curve is 256 output levels in 0..255 (point curves given are drawn in, and turning pencil off fits points to the drawing); smooth = Smooth passes over the drawn curves; eyedropper = the dialog's Set Black/Neutral Gray/White Point picker {"point":"black|gray|white","at":[x,y],"size":1..101=1} or {"point",…,"color":[r,g,b] 0..1}, RGB and Grayscale (no gray) only, applied over the given curves)"##,
        ),
        ("exposure", "Exposure…", r##"{"exposure":-20..20=0,"offset":-0.5..0.5=0,"gamma":0.01..9.99=1}"##),
        ("vibrance", "Vibrance…", r##"{"vibrance":-100..100=0,"saturation":-100..100=0}"##),
        (
            "hueSaturation",
            "Hue/Saturation…",
            r##"{"hue":-180..180=0,"saturation":-100..100=0,"lightness":-100..100=0,"colorize":bool=false,"reds":json,"yellows":json,"greens":json,"cyans":json,"blues":json,"magentas":json} (per range {"hue","saturation","lightness","range":[4 hue degrees]}; colorize: hue 0..360, saturation 0..100)"##,
        ),
        (
            "colorBalance",
            "Color Balance…",
            r##"{"shadows":json,"midtones":json,"highlights":json,"preserveLuminosity":bool=true} (each tone [cyan-red, magenta-green, yellow-blue] in -100..100)"##,
        ),
        (
            "blackWhite",
            "Black & White…",
            r##"{"reds":-200..300=40,"yellows":-200..300=60,"greens":-200..300=40,"cyans":-200..300=60,"blues":-200..300=20,"magentas":-200..300=80,"tint":bool=false,"tintColor":"#rrggbb"}"##,
        ),
        (
            "photoFilter",
            "Photo Filter…",
            r##"{"filter":"warming85|warmingLBA|warming81|cooling80|coolingLBB|cooling82|red|orange|yellow|green|cyan|blue|violet|magenta|sepia|deepRed|deepBlue|deepEmerald|deepYellow|underwater","color":"#rrggbb","density":0..100=25,"preserveLuminosity":bool=true}"##,
        ),
        (
            "channelMixer",
            "Channel Mixer…",
            r##"{"red":json,"green":json,"blue":json,"gray":json,"monochrome":bool=false} (per output channel [red %, green %, blue %, constant %] in -200..200; gray = the monochrome mix)"##,
        ),
        ("invert", "Invert", "{}"),
        ("posterize", "Posterize…", r##"{"levels":2..255=4}"##),
        ("threshold", "Threshold…", r##"{"level":1..255=128}"##),
        ("gradientMap", "Gradient Map…", r##"{"stops":json,"reverse":bool=false,"dither":bool=false} (stops [[location 0..1, "#rrggbb"], …], 2..64)"##),
        (
            "selectiveColor",
            "Selective Color…",
            r##"{"method":"relative|absolute"="relative","colors":"reds|yellows|greens|cyans|blues|magentas|whites|neutrals|blacks"="reds","cyan":-100..100=0,"magenta":-100..100=0,"yellow":-100..100=0,"black":-100..100=0,"reds":json} (per-range [c,m,y,k] arrays: reds yellows greens cyans blues magentas whites neutrals blacks)"##,
        ),
        (
            "colorLookup",
            "Color Lookup…",
            r##"{"lut":"none|warm|cool|tealOrange|bleachBypass|fadedFilm|dayForNight|monoContrast|crossProcess"="none","file":text,"interpolation":"trilinear|tetrahedral"="trilinear","dither":bool=false,"data":json} (file: .cube/.3dl/.look path; data: file text + "fileName")"##,
        ),
    ];
    for &(kind, label, params) in ADJ {
        let layer_id: &'static str = Box::leak(format!("layer.newAdjustmentLayer.{kind}").into_boxed_str());
        let image_id: &'static str = Box::leak(format!("image.adjustments.{kind}").into_boxed_str());
        v.push(CommandSpec {
            id: layer_id,
            label,
            menu: &["Layer", "New Adjustment Layer"],
            shortcut: None,
            params: Box::leak(format!("{params} + {{{NEW_LAYER_PATH}}}").into_boxed_str()),
            enabled: has_doc,
            run: |s, p| {
                let kind = p.get("__kind").and_then(Value::as_str).unwrap_or("invert").to_string();
                let eyedropped = if matches!(kind.as_str(), "levels" | "curves") { crate::adjust_params::eyedropper_from_params(s, &kind, p)? } else { None };
                let adj = match eyedropped {
                    Some(adj) => adj,
                    None => crate::adjust_params::from_params(&kind, p, None, doc_mode(s))?,
                };
                new_adjustment(s, adj, p, &format!("layer.newAdjustmentLayer.{kind}"))
            },
            journal: true,
        });
        v.push(CommandSpec {
            id: image_id,
            label,
            menu: &["Image", "Adjustments"],
            shortcut: match kind {
                "levels" => Some("Cmd+L"),
                "curves" => Some("Cmd+M"),
                "hueSaturation" => Some("Cmd+U"),
                "colorBalance" => Some("Cmd+B"),
                "invert" => Some("Cmd+I"),
                "blackWhite" => Some("Cmd+Alt+Shift+B"),
                _ => None,
            },
            params,
            enabled: crate::adjust_cmds::has_adjustable,
            run: |s, p| {
                let kind = p.get("__kind").and_then(Value::as_str).unwrap_or("invert").to_string();
                let eyedropped = if matches!(kind.as_str(), "levels" | "curves") { crate::adjust_params::eyedropper_from_params(s, &kind, p)? } else { None };
                // A smart filter re-runs its params, so it records what an eyedropper resolved
                // to, not the sample (#3127).
                let recorded = eyedropped.as_ref().map(|adj| {
                    let mut q = p.clone();
                    if let (Value::Object(q), Value::Object(resolved)) = (&mut q, crate::adjust_params::to_params(adj)) {
                        q.remove("eyedropper");
                        q.extend(resolved);
                    }
                    q
                });
                let adj = match eyedropped {
                    Some(adj) => adj,
                    None => {
                        // A gray plane target keeps the neutral tone space even over params
                        // carrying Lab or CMYK records (#5335).
                        let tone = if is_mask_target(p) || crate::channel_cmds::is_channel_target(p) {
                            if matches!(kind.as_str(), "levels" | "curves") { gray_tone_params(p) } else { p.clone() }
                        } else {
                            p.clone()
                        };
                        crate::adjust_params::from_params(&kind, &tone, None, adjust_mode(s, p))?
                    }
                };
                let label = adj.label().to_string();
                // A smart object records the adjustment as a smart filter (Photoshop), which
                // re-runs from the source instead of baking into a copy of the pixels.
                if crate::adjust_cmds::adjust_as_smart_filter(s, &kind, &label, recorded.as_ref().unwrap_or(p))? {
                    return Ok(Value::Null);
                }
                destructive_adjust(s, &label, adj, p)
            },
            journal: true,
        });
    }
    v.extend(crate::layer_style::specs());
    v.extend(crate::contour_lib_cmds::specs());
    v.extend(crate::layer_list_cmds::specs());
    v.extend(crate::filters::specs());
    v.extend(crate::filters_ext::specs());
    v.extend(crate::color_to_alpha_cmds::specs());
    v.extend(crate::gallery_cmds::specs());
    v.extend(crate::gradient_fill_cmds::specs());
    v.extend(crate::solid_fill_cmds::specs());
    v.extend(crate::pattern_fill_cmds::specs());
    v.extend(crate::type_cmds::specs());
    v.extend(crate::type_mask_cmds::specs());
    v.extend(crate::transform_cmds::specs());
    v.extend(crate::float_cmds::specs());
    v.extend(crate::vector_cmds::specs());
    v.extend(crate::path_edit_cmds::specs());
    v.extend(crate::smartselect_cmds::specs());
    v.extend(crate::cutout_cmds::specs());
    v.extend(crate::symmetry_cmds::specs());
    v.extend(crate::edit_cmds::specs());
    v.extend(crate::color_cmds::specs());
    v.extend(crate::brush_cmds::specs());
    v.extend(crate::brush_preset_cmds::specs());
    v.extend(crate::eraser_cmds::specs());
    v.extend(crate::preset_import_cmds::specs());
    v.extend(crate::atn_cmds::specs());
    v.extend(crate::actions_edit_cmds::specs());
    v.extend(crate::swatch_cmds::specs());
    v.extend(crate::retouch_cmds::specs());
    v.extend(crate::redeye_cmds::specs());
    v.extend(crate::image_cmds::specs());
    v.extend(crate::perspective_crop_cmds::specs());
    v.extend(crate::selection_cmds::specs());
    v.extend(crate::magnetic_cmds::specs());
    v.extend(crate::select_extra_cmds::specs());
    v.extend(crate::paint_cmds::specs());
    v.extend(crate::extra_cmds::specs());
    v.extend(crate::background_cmds::specs());
    v.extend(crate::file_cmds::specs());
    v.extend(crate::exr_cmds::specs());
    v.extend(crate::type_extra_cmds::specs());
    v.extend(crate::type_styles_cmds::specs());
    v.extend(crate::type_spell_cmds::specs());
    v.extend(crate::type_caret_cmds::specs());
    v.extend(crate::smart_cmds::specs());
    v.extend(crate::layer_multi_cmds::specs());
    v.extend(crate::layer_copy_cmds::specs());
    v.extend(crate::prefs::specs());
    v.extend(crate::edit_menu_cmds::specs());
    v.extend(crate::fill_key_cmds::specs());
    v.extend(crate::sample_cmds::specs());
    v.extend(crate::face_cmds::specs());
    v.extend(crate::sampler_cmds::specs());
    v.extend(crate::brush_key_cmds::specs());
    v.extend(crate::stamp_cmds::specs());
    v.extend(crate::align_cmds::specs());
    v.extend(crate::photo_cmds::specs());
    v.extend(crate::lens_cmds::specs());
    v.extend(crate::vp_cmds::specs());
    v.extend(crate::channel_cmds::specs());
    v.extend(crate::adjust_cmds::specs());
    v.extend(crate::layer_menu_cmds::specs());
    v.extend(crate::layer_label_cmds::specs());
    v.extend(crate::layer_xmp_cmds::specs());
    v.extend(crate::mode_cmds::specs());
    v.extend(crate::multichannel_cmds::specs());
    v.extend(crate::pattern_cmds::specs());
    v.extend(crate::warp_cmds::specs());
    v.extend(crate::comps_cmds::specs());
    v.extend(crate::artboard_cmds::specs());
    v.extend(crate::distort_cmds::specs());
    v.extend(crate::analysis_cmds::specs());
    v.extend(crate::notes_cmds::specs());
    v.extend(crate::history_cmds::specs());
    v.extend(crate::history_menu_cmds::specs());
    v.extend(crate::history_source_cmds::specs());
    v.extend(crate::history_select_cmds::specs());
    v.extend(crate::proof_sim::specs());
    v.extend(crate::presets::specs());
    v.extend(crate::document_preset_cmds::specs());
    v.extend(crate::export_preset_cmds::specs());
    v.extend(crate::render_cmds::specs());
    v.extend(crate::slice_cmds::specs());
    v.extend(crate::web_cmds::specs());
    v.extend(crate::automate_cmds::specs());
    v.extend(crate::print_cmds::specs());
    v.extend(crate::installed_profiles::specs());
    v.extend(crate::pick_cmds::specs());
    v.extend(crate::layer_nav_cmds::specs());
    v.extend(crate::frame_cmds::specs());
    v.extend(crate::migrate_cmds::specs());
    v.extend(crate::trap_cmds::specs());
    v.extend(crate::timeline_cmds::specs());
    v.extend(crate::video_cmds::specs());
    v.extend(crate::jobs::specs());
    v.extend(crate::wia_cmds::specs());
    v.extend(crate::rasterize_style_cmds::specs());
    v.extend(crate::variables_cmds::specs());
    v.extend(crate::plugin_cmds::specs());
    v.extend(crate::group_view_cmds::specs());
    v.extend(crate::fx_view_cmds::specs());
    v.extend(crate::fx_visibility_cmds::specs());
    v.extend(crate::mask_view_cmds::specs());
    v.extend(crate::layer_mask_props_cmds::specs());
    v.extend(crate::actions_cmds::specs());
    // The dispatch guard's unit test needs a command that panics on purpose (REL-3).
    #[cfg(test)]
    v.push(test_panic_command());
    v
}

/// A command whose `run` panics on purpose, so the last-resort guard in `jobs::dispatch`
/// can be tested (REL-3). Test builds only: `commands::build` pushes it under `cfg(test)`.
#[cfg(test)]
fn test_panic_command() -> CommandSpec {
    #[allow(clippy::panic)]
    fn run(_: &mut Session, _: &Value) -> Result<Value> {
        panic!("test: this command panicked on purpose")
    }
    CommandSpec { id: "test.panic", label: "Test Panic", menu: &[], shortcut: None, params: r##"{}"##, enabled: |_: &Session| Ok(()), run, journal: false }
}

/// Commands generated from a table share one `run` fn; the kind is recovered from the id.
pub(crate) fn inject_kind(id: &str, params: Value) -> Value {
    let kind = id.strip_prefix("layer.newAdjustmentLayer.").or_else(|| id.strip_prefix("image.adjustments."));
    match (kind, params) {
        (Some(k), Value::Object(mut m)) => {
            m.insert("__kind".into(), Value::String(k.into()));
            Value::Object(m)
        }
        (Some(k), _) => json!({ "__kind": k }),
        (None, p) => p,
    }
}

pub fn adjustment_kind(a: &Adjustment) -> &'static str {
    match a {
        Adjustment::BrightnessContrast { .. } => "brightnessContrast",
        Adjustment::Levels { .. } => "levels",
        Adjustment::Curves { .. } => "curves",
        Adjustment::Exposure { .. } => "exposure",
        Adjustment::Vibrance { .. } => "vibrance",
        Adjustment::HueSaturation { .. } => "hueSaturation",
        Adjustment::ColorBalance { .. } => "colorBalance",
        Adjustment::BlackWhite { .. } => "blackWhite",
        Adjustment::PhotoFilter { .. } => "photoFilter",
        Adjustment::ChannelMixer { .. } => "channelMixer",
        Adjustment::Posterize { .. } => "posterize",
        Adjustment::Threshold { .. } => "threshold",
        Adjustment::GradientMap { .. } => "gradientMap",
        Adjustment::SelectiveColor { .. } => "selectiveColor",
        Adjustment::ColorLookup { .. } => "colorLookup",
        Adjustment::Unsupported { .. } => "unsupported",
        _ => "invert",
    }
}

fn arrange(s: &mut Session, p: &Value, delta: i32) -> Result<Value> {
    let id = layer_param(s, p)?;
    s.edit("Arrange", |doc, _| {
        // The Background stays at the bottom: it doesn't move and nothing goes below it.
        let bg = crate::background_cmds::background_id(doc);
        if bg == Some(id) {
            return Err(EngineError::Other("the Background layer can't be moved".into()));
        }
        let steps = match delta {
            i32::MAX => 10_000,
            i32::MIN => -10_000,
            d => d,
        };
        let dir = steps.signum();
        let before = crate::clip_group::Before::read(doc, &[id]);
        let mut moved = false;
        for _ in 0..steps.abs() {
            if !doc.shift(id, dir) {
                break;
            }
            if crate::background_cmds::keep_background_at_bottom(bg, doc).is_err() {
                doc.shift(id, -dir);
                break;
            }
            moved = true;
        }
        if !moved {
            return Err(EngineError::Other("layer can't move further".into()));
        }
        crate::clip_group::settle(doc, &[id], &before, false);
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Add (`Some`) or remove (`None`) a layer mask. Adding acts on every selected layer that can take
/// one ([`crate::layer_multi_cmds::mask_targets`]), in one history step.
fn set_mask(s: &mut Session, p: &Value, label: &str, mask: Option<LayerMask>) -> Result<Value> {
    set_mask_with(s, p, label, mask, false)
}

/// [`set_mask`] that also drops the selection the mask was made from, in the same history step.
fn set_mask_and_deselect(s: &mut Session, p: &Value, label: &str, mask: LayerMask) -> Result<Value> {
    set_mask_with(s, p, label, Some(mask), true)
}

fn set_mask_with(s: &mut Session, p: &Value, label: &str, mask: Option<LayerMask>, deselect: bool) -> Result<Value> {
    let ids = if mask.is_some() { crate::layer_multi_cmds::mask_targets(s, p)? } else { vec![layer_param(s, p)?] };
    s.edit(label, |doc, _| {
        let mask = mask.clone().map(|mask| mask_at_depth(mask, doc.depth)).transpose()?;
        for id in &ids {
            if mask.is_some() {
                crate::extra_cmds::background_to_layer_for_mask(doc, *id);
            }
            doc.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?.mask = mask.clone();
        }
        if deselect {
            doc.selection = None;
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Move a layer's pixels, linked mask and type by whole pixels (vectors move via
/// `vector_cmds::translate_vectors`).
pub(crate) fn translate_layer(doc: &Document, l: &mut Layer, dx: i32, dy: i32) {
    use crate::layer_multi_cmds::shift_surface as translate_surface;
    // Linked patterns in the layer's effects move with it.
    if let Some(r) = &mut l.effects.reference {
        *r = (r.0 + f64::from(dx), r.1 + f64::from(dy));
    }
    if let Some(m) = &mut l.mask
        && m.linked
    {
        m.surface = translate_surface(&m.surface, dx, dy);
    }
    match &mut l.content {
        LayerContent::Raster(s) => *s = translate_surface(s, dx, dy),
        LayerContent::Text(t) => {
            t.transform = photocraft_geom::Affine::translate(dx as f64, dy as f64).mul(&t.transform);
            // A whole-pixel move shifts the rendered text and its PSD position as they are.
            // Re-rendering would replace Photoshop's rendering of imported type, and drop the
            // text when its font is missing (artboard export, moving type with the Move tool).
            let raw = t.psd_raw.as_deref().map(|r| photocraft_text::psd::translate_tysh(r, f64::from(dx), f64::from(dy)));
            match (&mut t.cache, raw) {
                (Some(c), Some(Some(r))) => {
                    *c = translate_surface(c, dx, dy);
                    t.psd_raw = Some(std::sync::Arc::new(r));
                }
                (Some(c), None) => *c = translate_surface(c, dx, dy),
                _ => crate::type_cmds::refresh(doc, t),
            }
        }
        LayerContent::Group(g) => {
            // Moving an artboard moves the board with its contents.
            if let Some(a) = &mut g.artboard {
                a.rect = a.rect.translate(dx, dy);
            }
            for c in &mut g.children {
                translate_layer(doc, c, dx, dy);
            }
        }
        LayerContent::Smart(sm) => crate::smart_cmds::shift_smart(sm, dx, dy),
        _ => {}
    }
}

/// A copy of layer `id` as Layer › Duplicate Layer makes it, not yet in the document: a fresh id,
/// "… copy" appended to the name, and a copy of the Background layer is an ordinary, unlocked
/// layer (Photoshop).
pub(crate) fn layer_copy(doc: &Document, id: LayerId, add_copy: bool) -> Result<Layer> {
    let src = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    // A copy of the Background layer is an ordinary, unlocked layer (Photoshop).
    let from_background = doc.is_background(id);
    let mut dup = src.duplicate();
    if add_copy {
        dup.name = doc.copy_name(&dup.name);
    }
    if from_background {
        dup.locks = Default::default();
    }
    Ok(dup)
}

/// Layer › Merge Group (⌘E with a group selected, #3832): the group's visible contents, as they
/// show, become one pixel layer in its place, named after it. Like Photoshop, the layer keeps the
/// group's opacity, blend mode (Pass Through becomes Normal), clipping and locks; the group's mask
/// and style are applied to the pixels.
fn merge_group(s: &mut Session, id: LayerId) -> Result<Value> {
    s.edit("Merge Group", |doc, active| {
        let group = doc.layer(id).ok_or(EngineError::NoLayer(id))?.clone();
        if !group.visible {
            return Err(EngineError::Other("the group is hidden".into()));
        }
        let fmt = doc.pixel_format();
        let fmt = photocraft_color::PixelFormat::new(fmt.mode, fmt.sample, true);
        let mut solo = group.clone();
        solo.opacity = 1.0;
        solo.blend = photocraft_doc::BlendMode::Normal;
        solo.clipped = false;
        let mut tmp = doc.clone();
        tmp.layers = vec![solo];
        let mut merged = Layer::new(group.name.clone(), LayerContent::Raster(pixels::composite_layers_uncropped(&tmp, fmt)));
        merged.id = group.id;
        merged.opacity = group.opacity;
        merged.blend = if group.blend == photocraft_doc::BlendMode::PassThrough { photocraft_doc::BlendMode::Normal } else { group.blend };
        merged.clipped = group.clipped;
        merged.locks = group.locks;
        *doc.layer_mut(id).ok_or(EngineError::NoLayer(id))? = merged;
        *active = Some(id);
        Ok(())
    })?;
    Ok(json!({ "layer": id.0 }))
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn duplicate_ids_keep_the_first_spec_and_listing_order() {
        let mut built = build().into_iter();
        let first = built.next().unwrap();
        let mut second = built.next().unwrap();
        let id = first.id;
        second.id = id;
        let registry = Registry::new(vec![first, second]);
        assert_eq!(registry.by_id.get(id), Some(&0));
        assert_eq!(registry.specs.len(), 2);
    }
}

#[cfg(test)]
mod set_props_tests {
    use super::*;

    #[test]
    fn opacity_and_fill_outside_zero_to_one_are_refused_not_clamped() {
        // #5242: `55` (a percentage, as the Layer Style commands take) used to become 100 %.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("layer.setProps", json!({"opacity": 0.55, "fill": 0.4})).unwrap();
        let layer = |s: &Session| {
            let d = s.active().unwrap();
            let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
            (l.opacity, l.fill_opacity)
        };
        assert_eq!(layer(&s), (0.55, 0.4));
        for bad in [json!({"opacity": 55}), json!({"opacity": -0.1}), json!({"fill": 1.5}), json!({"opacity": "50%"}), json!({"opacity": 0.5, "fill": 40})] {
            let e = s.execute("layer.setProps", bad.clone()).unwrap_err().to_string();
            assert!(e.contains("0..1"), "{bad}: {e}");
            assert_eq!(layer(&s), (0.55, 0.4), "{bad} left the layer alone");
        }
        s.execute("layer.setProps", json!({"opacity": 0, "fill": 1})).unwrap();
        assert_eq!(layer(&s), (0.0, 1.0));
    }
}
