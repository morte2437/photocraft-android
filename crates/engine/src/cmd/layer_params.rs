use super::params::bad;
use super::{BlendMode, Document, EngineError, Layer, LayerId, LayerMask, Result, SampleType, Session, Value};

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
