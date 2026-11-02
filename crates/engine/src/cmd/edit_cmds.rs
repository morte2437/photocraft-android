//! Everyday Photoshop commands: clipboard (Cut/Copy/Copy Merged/Paste/Paste in Place), Layer via
//! Copy/Cut, Merge Visible, Auto Tone/Contrast/Color, Toggle Last State and Transform Again.

use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::adjust::LevelsChannel;
use photocraft_doc::{Adjustment, DocId, Document, Layer, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The paste commands' `"target"` param (for `concat!` into their params docs).
macro_rules! paste_target {
    () => {
        r#""target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target (not pixels: the luminosity pasted into that mask or channel)"#
    };
}
pub(crate) use paste_target;

/// Pixels on the session clipboard, with where they came from (for Paste in Place).
#[derive(Clone, Debug)]
pub struct Clip {
    /// Native colour indices for a multi-component clip, in channel order. Singletons are grayscale.
    pub components: Option<Vec<usize>>,
    pub surface: Surface,
    pub bounds: Rect,
    /// The copied layer itself, when Copy took a whole layer that Paste should recreate rather
    /// than rasterize (see [`whole_layer`]): a scratch document in the source's colour holding it.
    /// `surface` still holds its pixels, for Paste Into, masks, channels and other apps.
    pub layers: Option<Box<Document>>,
    /// The source document's resolution (ppi): File › New from the clipboard takes it, as
    /// Photoshop's Clipboard preset does. None for an image from another app.
    pub resolution: Option<f32>,
    /// The document the pixels were copied from: Paste keeps them in place only there, and
    /// centres them in any other (#8945). None for an image from another app.
    pub source: Option<DocId>,
}

/// Photoshop's layer-based copy (CC 2018 on): with no selection, Copy takes the active layer
/// whole when it is a smart object (#2428), a type or shape layer, or a group (its whole subtree)
/// (#2670), so Paste recreates it instead of its pixels. Fill and adjustment layers have no
/// pixels of their own, so they are taken whole too rather than refused (#2344). Pixel layers
/// still copy their pixels; several selected layers are [`multi_copy_ids`].
fn takes_whole(d: &crate::DocState, id: LayerId) -> bool {
    d.doc.selection.is_none()
        && d.selected_layers().len() <= 1
        && crate::layer_copy_cmds::layered(d.doc.mode)
        && d.doc.layer(id).is_some_and(|l| !matches!(l.content, LayerContent::Raster(_)))
}

/// A scratch document in the source's colour, to hold copied layers.
fn scratch_doc(doc: &Document) -> Document {
    let mut copies = Document::new("", doc.size, doc.mode, doc.depth);
    copies.icc_profile = doc.icc_profile.clone();
    copies.resolution_dpi = doc.resolution_dpi;
    copies.patterns = doc.patterns.clone();
    copies
}

/// Scope linked records to the copied layer tree: a PSD block can also hold files belonging
/// to unselected or hidden layers, which must not follow this copy into a native save.
fn copy_linked_sources(source: &Document, copies: &mut Document) -> Result<()> {
    fn collect<'a>(layers: &'a [Layer], depth: usize, paths: &mut std::collections::HashSet<&'a str>) -> Result<()> {
        if depth > photocraft_doc::MAX_GROUP_DEPTH {
            return Err(EngineError::Other("can't copy linked smart objects from layers nested too deeply".into()));
        }
        for layer in layers {
            if let LayerContent::Smart(sm) = &layer.content
                && let photocraft_doc::SmartSource::Linked { path } = &sm.source
            {
                paths.try_reserve(1).map_err(|_| EngineError::Other("not enough memory to copy smart-object source IDs".into()))?;
                paths.insert(path);
            }
            if let Some(children) = layer.children().filter(|children| !children.is_empty()) {
                collect(children, depth + 1, paths)?;
            }
        }
        Ok(())
    }
    let mut paths = std::collections::HashSet::new();
    collect(&copies.layers, 0, &mut paths)?;
    if paths.is_empty() {
        return Ok(());
    }
    for (signature, key, data) in &source.metadata.psd_global_blocks {
        if !matches!(key, b"lnk2" | b"lnk3" | b"lnkD" | b"lnkE") {
            continue;
        }
        let selected = photocraft_io::linked::select_items(data, &|uuid, _| paths.contains(uuid))
            .map_err(|e| EngineError::Other(format!("Could not copy smart-object contents: {e}; the clipboard was not changed")))?;
        let selected = selected.map(std::sync::Arc::new).unwrap_or_else(|| data.clone());
        if !selected.is_empty() {
            copies.metadata.psd_global_blocks.try_reserve(1).map_err(|_| EngineError::Other("not enough memory to copy smart-object source records".into()))?;
            copies.metadata.psd_global_blocks.push((*signature, *key, selected));
        }
    }
    Ok(())
}

/// The layer Copy takes whole ([`takes_whole`]): a scratch document in the source's colour
/// holding it.
fn whole_layer(d: &crate::DocState, id: LayerId) -> Option<Box<Document>> {
    let l = d.doc.layer(id).filter(|_| takes_whole(d, id))?;
    let mut copies = scratch_doc(&d.doc);
    copies.layers.push(l.clone());
    Some(Box::new(copies))
}

/// The layers Copy takes when several are selected and nothing is selected in the pixels
/// (Photoshop, #2478): the selected layers that no selected group already carries, bottom-to-top.
/// Empty otherwise, and for a single layer ([`takes_whole`] decides that case).
fn multi_copy_ids(d: &crate::DocState) -> Vec<LayerId> {
    if d.doc.selection.is_some() || !crate::layer_copy_cmds::layered(d.doc.mode) {
        return Vec::new();
    }
    let selected = d.selected_layers();
    if selected.len() < 2 {
        return Vec::new();
    }
    crate::layer_multi_cmds::top_level(&d.doc, &selected)
}

/// Every selected layer, whole ([`multi_copy_ids`]), in a scratch document: Paste recreates them
/// all, each with its mask, styles and content. The Background becomes an ordinary layer, as in
/// Photoshop's Duplicate Layer.
fn whole_layers(d: &crate::DocState, ids: &[LayerId]) -> Option<Box<Document>> {
    let mut copies = scratch_doc(&d.doc);
    for id in ids {
        let mut l = d.doc.layer(*id)?.clone();
        if d.doc.is_background(*id) {
            l.name = "Layer 0".into();
            l.locks = Default::default();
        }
        copies.layers.push(l);
    }
    Some(Box::new(copies))
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// Paste and New from Clipboard need only a clipboard: with no document open, they make one.
fn has_clip_only(s: &Session) -> std::result::Result<(), String> {
    s.clipboard.as_ref().map(|_| ()).ok_or_else(|| "the clipboard is empty".into())
}

fn active_layer(s: &Session) -> std::result::Result<&Layer, String> {
    let d = s.active().ok_or("no document open")?;
    d.active_layer.and_then(|id| d.doc.layer(id)).ok_or_else(|| "no active layer".into())
}

/// Cut and Layer via Cut edit the pixels, so they need a pixel layer.
fn has_pixels(s: &Session) -> std::result::Result<(), String> {
    match active_layer(s)?.content {
        LayerContent::Raster(_) => Ok(()),
        LayerContent::Smart(_) => Err("the smart object is not directly editable".into()),
        _ => Err("the active layer has no pixels".into()),
    }
}

/// Copy only reads: any layer that shows pixels will do, so a smart object, type or shape layer
/// copies what it shows (Photoshop).
fn has_layer_pixels(s: &Session) -> std::result::Result<(), String> {
    active_layer(s)?.surface().map(|_| ()).ok_or_else(|| "the active layer has no pixels".into())
}

/// Copy: a targeted alpha channel (#2307), a layer with pixels, or one taken whole (a group with
/// no selection, #2670).
fn can_copy(s: &Session) -> std::result::Result<(), String> {
    if crate::channel_clip::copies_alpha_channel(s) || s.active().is_some_and(|d| !multi_copy_ids(d).is_empty()) {
        return Ok(());
    }
    has_layer_pixels(s).or_else(|e| s.active().and_then(|d| d.active_layer.filter(|id| takes_whole(d, *id))).map(|_| ()).ok_or(e))
}

fn has_clip(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    s.clipboard.as_ref().map(|_| ()).ok_or_else(|| "the clipboard is empty".into())
}

fn active_id(s: &Session) -> Result<LayerId> {
    s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into()))
}

/// The pixels of `src` inside the selection (or all of them), as a straight-alpha surface.
///
/// Built tile by tile (#4516). A source tile that the copy takes whole (inside the canvas, fully
/// selected or no selection, already in the clip's format) is shared copy-on-write, so a plain
/// Copy or Select All, Copy costs a pointer per tile; only the other tiles are read and written,
/// one at a time, instead of the whole area as one `f32` buffer.
fn lift(src: &Surface, sel: Option<&Surface>, canvas: Rect) -> Result<Clip> {
    let fmt = src.format();
    let with_alpha = PixelFormat::new(fmt.mode, fmt.sample, true);
    let area = match sel {
        Some(m) => m.content_bounds().intersect(&canvas),
        None => src.content_bounds().intersect(&canvas),
    };
    let mut out = Surface::new(with_alpha);
    if area.is_empty() {
        return Ok(Clip { components: None, surface: out, bounds: Rect::EMPTY, layers: None, resolution: None, source: None });
    }
    // Only the selected area is read: converting the whole layer (a 24 MP Background gaining
    // alpha) cost about a second per Layer via Copy (#668).
    let mut shared = Vec::new();
    for tc in area.tiles() {
        let tr = tc.rect().intersect(&area);
        if fmt.alpha
            && tr == tc.rect()
            && let Some(tile) = src.tile(tc)
            && sel.is_none_or(|m| crate::fill_cmds::Mask::of(m, tc).full(tr))
        {
            // Reading and writing it back would give the same bytes.
            shared.push((tc, tile.clone()));
            continue;
        }
        let px = lift_region(src, sel, tr, with_alpha.channels())?;
        out.try_write_region(tr, &px).map_err(|e| EngineError::Other(format!("not enough memory while copying pixels ({e}); the document was not changed")))?;
    }
    out.put_tiles(shared);
    out.prune();
    let bounds = out.content_bounds();
    Ok(Clip { components: None, surface: out, bounds, layers: None, resolution: None, source: None })
}

/// [`lift`] over one rectangle: `src`'s pixels with alpha (`n` channels), the selection
/// multiplied into it.
fn lift_region(src: &Surface, sel: Option<&Surface>, r: Rect, n: usize) -> Result<Vec<f32>> {
    let fmt = src.format();
    let mut px = if fmt.alpha {
        crate::allocation::read_region(src, r, "copying pixels")?
    } else {
        let opaque = crate::allocation::read_region(src, r, "copying pixels")?;
        let k = fmt.channels();
        let len = opaque
            .len()
            .checked_div(k.max(1))
            .and_then(|pixels| pixels.checked_mul(n))
            .ok_or_else(|| EngineError::Other("not enough memory for copying pixels (requested size overflowed); the document was not changed".into()))?;
        let mut v = crate::allocation::filled(len, 0.0, "copying pixels")?;
        let mut at = 0;
        for p in opaque.chunks_exact(k.max(1)) {
            let Some(dst) = v.get_mut(at..at + k.max(1)) else { break };
            dst.copy_from_slice(p);
            at += k.max(1);
            if let Some(alpha) = v.get_mut(at) {
                *alpha = 1.0;
            }
            at += 1;
        }
        v
    };
    if let Some(m) = sel {
        let mask = crate::allocation::read_region(m, r, "applying the selection to copied pixels")?;
        let mk = m.format().channels().max(1);
        for (p, a) in px.chunks_exact_mut(n.max(1)).zip(mask.chunks_exact(mk)) {
            if let Some(alpha) = p.last_mut() {
                *alpha *= a[0];
            }
        }
    }
    Ok(px)
}

/// Merged composite of the visible document as a surface in the document's format.
fn merged_surface(doc: &Document) -> Surface {
    let fmt = doc.pixel_format();
    photocraft_compose::flatten_to_surface(doc, PixelFormat::new(fmt.mode, fmt.sample, true), None)
}

fn copy(s: &mut Session, merged: bool) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let clip = if merged {
        lift(&merged_surface(&d.doc), d.doc.selection.as_ref(), canvas)?
    } else {
        let id = d.active_layer.ok_or(EngineError::Other("no active layer".into()))?;
        let several = multi_copy_ids(d);
        let mut layers = if several.is_empty() { whole_layer(d, id) } else { whole_layers(d, &several) };
        if let Some(copies) = &mut layers {
            copy_linked_sources(&d.doc, copies)?;
        }
        match (d.doc.layer(id).and_then(Layer::surface).filter(|_| several.is_empty()), &layers) {
            (Some(surf), _) => Clip { components: None, layers, ..lift(surf, d.doc.selection.as_ref(), canvas)? },
            // A group or fill layer has no pixels of its own: the clipboard pixels are what it
            // shows. Adjustment layers show nothing by themselves, so their clip spans the canvas
            // they apply to: Paste places them as a full-canvas layer, and the UI still mirrors
            // the clip to the OS clipboard (an empty clip would let a stale OS image replace it).
            (None, Some(copies)) => {
                let mut shown = (**copies).clone();
                for l in &mut shown.layers {
                    l.visible = true;
                }
                let clip = lift(&merged_surface(&shown), None, canvas)?;
                let bounds = if clip.bounds.is_empty() { canvas } else { clip.bounds };
                Clip { components: None, layers, bounds, ..clip }
            }
            (None, None) => return Err(EngineError::Other("the active layer has no pixels".into())),
        }
    };
    if clip.bounds.is_empty() {
        return Err(EngineError::Other("Could not copy: the selected area is empty".into()));
    }
    let b = clip.bounds;
    let clip = Clip { resolution: Some(d.doc.resolution_dpi), source: Some(d.doc.id), ..clip };
    s.clipboard = Some(clip);
    Ok(json!({"bounds": [b.x0, b.y0, b.width(), b.height()]}))
}

/// Copy the targeted layer mask (`"target":"mask"`, #2928) or smart-filter mask
/// (`"target":"filterMask"`): the mask's values as a
/// grayscale clip, inside the selection or the whole canvas — an untouched mask copies at its
/// default value, so a plain Reveal All / Hide All mask transfers too. Paste then writes into
/// another layer's mask (targeted, #1035), in this or another document.
fn copy_mask(s: &mut Session, filter: bool) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let id = d.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
    let layer = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let mask = crate::mask_view_cmds::viewed_mask(layer, filter)
        .ok_or_else(|| EngineError::Other(if filter { "the active layer has no filter mask" } else { "the active layer has no layer mask" }.into()))?;
    let sel = d.doc.selection.as_ref();
    let area = sel.map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    if area.is_empty() {
        return Err(EngineError::Other("Could not copy: the selected area is empty".into()));
    }
    let n = mask.surface.format().channels().max(1);
    let values = crate::allocation::read_region(&mask.surface, area, "copying the mask")?;
    let len = values
        .len()
        .checked_div(n)
        .and_then(|pixels| pixels.checked_mul(2))
        .ok_or_else(|| EngineError::Other("not enough memory for copying the mask (requested size overflowed); the document was not changed".into()))?;
    let cover = sel.map(|m| (m.read_region(area), m.format().channels().max(1)));
    let mut px = crate::allocation::filled(len, 0.0, "copying the mask")?;
    for (i, p) in values.chunks_exact(n).enumerate() {
        px[i * 2] = p.first().copied().unwrap_or(0.0);
        px[i * 2 + 1] = cover.as_ref().map_or(1.0, |(m, mk)| m.get(i * mk).copied().unwrap_or(0.0));
    }
    let fmt = PixelFormat::new(ColorMode::Grayscale, mask.surface.format().sample, true);
    let mut surface = Surface::new(fmt);
    surface
        .try_write_region(area, &px)
        .map_err(|e| EngineError::Other(format!("not enough memory while copying the mask ({e}); the document was not changed")))?;
    surface.prune();
    let bounds = surface.content_bounds();
    if bounds.is_empty() {
        return Err(EngineError::Other("Could not copy: the selected area is empty".into()));
    }
    let b = bounds;
    let (resolution, source) = (Some(d.doc.resolution_dpi), Some(d.doc.id));
    s.clipboard = Some(Clip { components: None, surface, bounds: b, layers: None, resolution, source });
    Ok(json!({"bounds": [b.x0, b.y0, b.width(), b.height()], "target": if filter { "filterMask" } else { "mask" }}))
}

fn clear_selected(doc: &mut Document, id: LayerId, background: [f32; 4]) -> Result<()> {
    let sel = doc.selection.clone();
    let canvas = doc.bounds();
    let area = sel.as_ref().map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    clear_area(doc, id, area, sel.as_ref(), background)
}

/// Clear/Cut fill the selected mask samples with background gray, preserving layer pixels.
pub(crate) fn clear_mask(doc: &mut Document, id: LayerId, background: [f32; 4], params: &Value) -> Result<()> {
    let sel = doc.selection.clone();
    let canvas = doc.bounds();
    let area = sel.as_ref().map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    let mask = crate::commands::paint_surface(doc, id, params)?;
    crate::fill_cmds::fill_color(mask, area, [background[0], background[1], background[2], 1.0], sel.as_ref())
}

/// Cut / Clear on a layer: makes the selected pixels transparent. The Background can't hold
/// transparency, so there the area is filled with the background colour instead (Photoshop).
pub(crate) fn clear_area(doc: &mut Document, id: LayerId, area: Rect, sel: Option<&Surface>, background: [f32; 4]) -> Result<()> {
    doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let bg = doc.is_background(id);
    let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
    if bg {
        crate::pixels::try_fill_surface(surf, area, background, sel, true)?;
    } else {
        crate::pixels::try_clear_surface(surf, area, sel)?;
    }
    surf.prune();
    Ok(())
}

/// Paste as a new layer (or into the targeted mask or channel: [`paste_to_target`]). `in_place` keeps the original position; otherwise the pixels are centred
/// on `center` (the view centre from the UI) or the canvas, unless they already overlap the canvas.
fn paste(s: &mut Session, p: &Value, in_place: bool) -> Result<Value> {
    crate::allocation::checkpoint("pasting pixels")?;
    let clip = s.clipboard.clone().ok_or(EngineError::Other("the clipboard is empty".into()))?;
    let Some(d) = s.active() else {
        // Nothing open to paste into: the clipboard becomes a document of its own (#368).
        return new_from_clipboard(s);
    };
    if crate::channel_cmds::target_of(p) != crate::channel_cmds::Target::Pixels {
        return paste_to_target(s, p, in_place, None, "Paste");
    }
    if let Some(colors) = crate::color_channels::target(s, p)?.filter(|c| c.len() > 1 || p.get("target").is_some() || clip.components.is_some()) {
        return crate::channel_clip::paste_colors(s, p, colors, in_place, None, "Paste");
    }
    // A targeted colour channel takes the paste instead of a new layer (#2307).
    if let Some(k) = crate::channel_clip::paste_color_target(s, p) {
        return crate::channel_clip::paste(s, p, k, in_place, None, "Paste");
    }
    let canvas = d.doc.bounds();
    let fmt = d.doc.pixel_format();
    let (dx, dy) = paste_offset(&clip, d.doc.id, canvas, p, in_place);
    if let Some(layers) = clip.layers.as_deref().filter(|_| crate::layer_copy_cmds::layered(d.doc.mode)) {
        return paste_layers(s, layers, dx, dy);
    }
    let moved = shifted(&clip.surface, dx, dy);
    let target = PixelFormat::new(fmt.mode, fmt.sample, true);
    let surf = if moved.format() == target { moved } else { moved.convert(target) };
    let id = s.edit("Paste", |doc, active| {
        let mut l = Layer::raster(doc.next_layer_name("Layer"), target);
        *crate::pixels_mut(&mut l)? = surf;
        let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::AutoEditable);
        *active = Some(id);
        doc.selection = None;
        Ok(id)
    })?;
    // `edit` reports only where the pasted pixels land as damage, not the whole document (#2901).
    Ok(json!({"layer": id.0, "offset": [dx, dy]}))
}

/// Paste of a whole copied layer ([`whole_layer`]): fresh copies (a group's children too),
/// converted to the document's colour, moved as the pixels would be, above the active layer. A pasted smart object gets its
/// own contents, independent of the copied one (as across documents).
fn paste_layers(s: &mut Session, layers: &Document, dx: i32, dy: i32) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let mut copies = layers.clone();
    for l in &mut copies.layers {
        *l = l.duplicate();
    }
    crate::layer_copy_cmds::adapt_copies(s, &mut copies, &d.doc)?;
    crate::layer_copy_cmds::translate_copies(&mut copies, &d.doc, dx, dy);
    let patterns: Vec<_> = copies.patterns.iter().filter(|pat| !d.doc.patterns.iter().any(|q| q.id == pat.id)).cloned().collect();
    // A copied smart object may name a PSD-embedded linked file; land its `lnk` block too (#3584).
    let mut linked = Vec::new();
    for (signature, key, data) in copies.metadata.psd_global_blocks.iter().filter(|(_, k, _)| matches!(k, b"lnk2" | b"lnk3" | b"lnkD" | b"lnkE")) {
        if d.doc.metadata.psd_global_blocks.iter().any(|(_, dk, dd)| dk == key && std::sync::Arc::ptr_eq(dd, data)) {
            continue;
        }
        // The clipboard may contain a subset of a destination block. Compare complete records,
        // so copying one layer in the same document does not duplicate large embedded files.
        let missing = photocraft_io::linked::select_items(data, &|_, raw| {
            !d.doc.metadata.psd_global_blocks.iter().any(|(_, dk, dd)| dk == key && photocraft_io::linked::contains_item(dd, raw))
        })
        .map_err(|e| EngineError::Other(format!("Could not paste smart-object contents: {e}; the document was not changed")))?;
        let missing = missing.map(std::sync::Arc::new).unwrap_or_else(|| data.clone());
        if !missing.is_empty() {
            linked.try_reserve(1).map_err(|_| EngineError::Other("not enough memory to paste smart-object source records".into()))?;
            linked.push((*signature, *key, missing));
        }
    }
    let id = s.edit("Paste", |doc, active| {
        doc.patterns.extend(patterns);
        doc.metadata.psd_global_blocks.extend(linked);
        // The first pasted layer goes where a new layer would; the rest stack above it.
        let mut place = crate::layer_place::Place::AutoEditable;
        for l in copies.layers {
            *active = Some(crate::layer_place::insert_new(doc, *active, l, place));
            place = crate::layer_place::Place::Above;
        }
        crate::cmd::layer_multi_cmds::check_group_depth(doc, "Paste")?;
        doc.selection = None;
        active.ok_or(EngineError::Other("nothing to paste".into()))
    })?;
    Ok(json!({"layer": id.0, "offset": [dx, dy]}))
}

/// How far a paste moves the clipboard pixels: not at all in place (or when they lie on the
/// canvas and no `center` is given), else onto `center` (the view centre from the UI) or the
/// canvas centre, then kept on the canvas as Photoshop keeps it: pixels that fit stay wholly
/// inside, larger ones start at the canvas's top-left corner (#6759). `"clampToCanvas": false`
/// skips that, for Paste Into / Outside, which centre on the selection.
pub(crate) fn paste_offset(clip: &Clip, doc: DocId, canvas: Rect, p: &Value, in_place: bool) -> (i32, i32) {
    let home = clip.source == Some(doc);
    if in_place || (home && clip.bounds.intersect(&canvas) == clip.bounds && p.get("center").is_none()) {
        return (0, 0);
    }
    let c = p.get("center").and_then(Value::as_array).filter(|a| a.len() >= 2).map(|a| (a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0)));
    let (cx, cy) = c.unwrap_or((mid(canvas.x0, canvas.x1), mid(canvas.y0, canvas.y1)));
    let b = clip.bounds;
    let (dx, dy) = ((cx - mid(b.x0, b.x1)).round(), (cy - mid(b.y0, b.y1)).round());
    if p.get("clampToCanvas").and_then(Value::as_bool) == Some(false) || b.is_empty() {
        return (dx as i32, dy as i32);
    }
    (clamp_shift(dx, b.x0, b.x1, canvas.x0, canvas.x1), clamp_shift(dy, b.y0, b.y1, canvas.y0, canvas.y1))
}

/// The midpoint of `a..b`, each edge converted before adding so extreme coordinates can't overflow.
fn mid(a: i32, b: i32) -> f64 {
    (f64::from(a) + f64::from(b)) / 2.0
}

/// Shift `d` of the span `b0..b1` along one axis, changed as little as needed to keep the span
/// inside `c0..c1` when it fits; a span longer than the canvas starts at `c0`.
fn clamp_shift(d: f64, b0: i32, b1: i32, c0: i32, c1: i32) -> i32 {
    let (b0, b1, c0, c1) = (i64::from(b0), i64::from(b1), i64::from(c0), i64::from(c1));
    // `as` saturates a NaN or out-of-range shift, then the clamp brings it into range.
    let start = if b1 - b0 <= c1 - c0 { b0.saturating_add(d as i64).clamp(c0, c1 - (b1 - b0)) } else { c0 };
    i32::try_from(start - b0).unwrap_or(if start > b0 { i32::MAX } else { i32::MIN })
}

pub(crate) fn shifted(surface: &Surface, dx: i32, dy: i32) -> Surface {
    if dx == 0 && dy == 0 { surface.clone() } else { photocraft_algo::resample::translate_surface(surface, dx, dy) }
}

/// Paste into the targeted layer mask, alpha channel or Quick Mask (`"target"`) instead of as a
/// new layer (Photoshop, #1035): the pasted pixels' luminosity, placed as a paste places them,
/// over what was there. Their transparency, and `limit` (Paste Into / Outside), let it show
/// through. One history step.
pub(crate) fn paste_to_target(s: &mut Session, p: &Value, in_place: bool, limit: Option<&Surface>, label: &str) -> Result<Value> {
    crate::allocation::checkpoint("pasting pixels")?;
    let clip = s.clipboard.clone().ok_or(EngineError::Other("the clipboard is empty".into()))?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = d.doc.bounds();
    let layer = d.active_layer;
    let (dx, dy) = paste_offset(&clip, d.doc.id, canvas, p, in_place);
    let src = shifted(&clip.surface, dx, dy);
    let area = src.content_bounds().intersect(&canvas);
    s.edit(label, |doc, _| {
        // A spot channel stores ink: the paste goes over its tone, then back to ink (#5787).
        let spot = crate::channel_cmds::is_spot_target(doc, p);
        let (surf, _) = crate::channel_cmds::target_surface(doc, layer, p)?;
        if spot {
            crate::channel_cmds::invert_region(surf, area)?;
        }
        crate::fill_cmds::composite_over(surf, &src, area, limit)?;
        if spot {
            crate::channel_cmds::invert_region(surf, area)?;
        }
        surf.prune();
        doc.selection = None;
        Ok(())
    })?;
    Ok(json!({"target": p.get("target"), "offset": [dx, dy]}))
}

/// A new document the size of the clipboard image, holding it as its one layer, in the pixel
/// format it was copied in (#368).
fn new_from_clipboard(s: &mut Session) -> Result<Value> {
    let clip = s.clipboard.clone().ok_or(EngineError::Other("the clipboard is empty".into()))?;
    let b = clip.bounds;
    let (w, h) = (b.width(), b.height());
    if w == 0 || h == 0 {
        return Err(EngineError::Other("the clipboard image has no size".into()));
    }
    let (Some(dx), Some(dy)) = (b.x0.checked_neg(), b.y0.checked_neg()) else {
        return Err(EngineError::Other("the clipboard origin is outside the coordinate range for a new document".into()));
    };
    let fmt = clip.surface.format();
    let target = PixelFormat::new(fmt.mode, fmt.sample, true);
    let moved = if dx == 0 && dy == 0 { clip.surface } else { photocraft_algo::resample::translate_surface(&clip.surface, dx, dy) };
    let mut doc = Document::new("Untitled", photocraft_geom::Size::new(w, h), fmt.mode, fmt.sample);
    if let Some(r) = clip.resolution.filter(|r| r.is_finite() && *r > 0.0) {
        doc.resolution_dpi = r;
    }
    let mut l = Layer::raster(doc.next_layer_name("Layer"), target);
    *crate::pixels_mut(&mut l)? = if moved.format() == target { moved } else { moved.convert(target) };
    doc.layers.push(l);
    let i = s.add_document(doc, None);
    // Clipboard content has never been saved; closing this tab must ask before discarding it.
    if let Some(st) = s.active_mut() {
        st.saved_revision = 0;
    }
    Ok(json!({"document": i, "width": w, "height": h}))
}

fn layer_via(s: &mut Session, cut: bool) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id = active_id(s)?;
    let has_sel = d.doc.selection.is_some();
    if !has_sel && !cut && crate::layer_multi_cmds::multi(s, &Value::Null) {
        // No selection, several layers selected: copy every one of them (Photoshop, #2777).
        return crate::layer_multi_cmds::duplicate_selected(s, true, "Layer Via Copy");
    }
    if !has_sel && !cut {
        // No selection: Layer via Copy duplicates the whole layer (Photoshop).
        let nid = s.edit("Layer Via Copy", |doc, active| {
            let src = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            let mut dup = src.duplicate();
            dup.name = doc.copy_name(&src.name);
            dup.locks = Default::default();
            let nid = doc.insert_above(Some(id), dup);
            *active = Some(nid);
            Ok(nid)
        })?;
        return Ok(json!({"layer": nid.0}));
    }
    let surf = d.doc.layer(id).and_then(Layer::surface).ok_or(EngineError::Other("the active layer has no pixels".into()))?;
    let clip = lift(surf, d.doc.selection.as_ref(), d.doc.bounds())?;
    if clip.bounds.is_empty() {
        return Err(EngineError::Other("Could not complete the command: the selected area is empty".into()));
    }
    let bg = s.tools.background;
    let label = if cut { "Layer Via Cut" } else { "Layer Via Copy" };
    let nid = s.edit(label, |doc, active| {
        if cut {
            clear_selected(doc, id, bg)?;
        }
        let mut l = Layer::raster(doc.next_layer_name("Layer"), clip.surface.format());
        *crate::pixels_mut(&mut l)? = clip.surface;
        let nid = crate::layer_place::insert_new(doc, Some(id), l, crate::layer_place::Place::Above);
        *active = Some(nid);
        // Photoshop deselects: a following Free Transform moves the whole new layer (#1096).
        doc.selection = None;
        Ok(nid)
    })?;
    Ok(json!({"layer": nid.0}))
}

fn merge_visible(s: &mut Session) -> Result<Value> {
    s.edit("Merge Visible", |doc, active| {
        let visible: Vec<usize> = doc.layers.iter().enumerate().filter(|(_, l)| l.visible).map(|(i, _)| i).collect();
        if visible.len() < 2 && !visible.iter().any(|i| doc.layers[*i].is_group()) {
            return Err(EngineError::Other("there are not enough visible layers to merge".into()));
        }
        let lowest = visible[0];
        let base = doc.layers[lowest].clone();
        let is_background = lowest == 0 && doc.is_background(base.id);
        // Composite of visible layers only (hidden ones stay where they are).
        let mut solo = doc.clone();
        solo.layers.retain(|l| l.visible);
        let fmt = doc.pixel_format();
        let fmt = if is_background { fmt } else { PixelFormat::new(fmt.mode, fmt.sample, true) };
        let mut merged = Layer::raster(base.name.clone(), fmt);
        merged.locks = base.locks;
        *crate::pixels_mut(&mut merged)? = crate::pixels::composite_layers(&solo, fmt, is_background.then_some([1.0, 1.0, 1.0]));
        let mid = merged.id;
        let mut out = Vec::with_capacity(doc.layers.len());
        for (i, l) in doc.layers.drain(..).enumerate() {
            if i == lowest {
                out.push(merged.clone());
            } else if !l.visible {
                out.push(l);
            }
        }
        doc.layers = out;
        *active = Some(mid);
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Histogram of a pixel layer (8-bit bins per channel: R, G, B) inside the selection.
fn histograms(surf: &Surface, area: Rect) -> [[u64; 256]; 3] {
    let mut h = [[0u64; 256]; 3];
    let step = ((area.width() as u64 * area.height() as u64) / 4_000_000).max(1) as usize; // sample big images
    let mut row = vec![[0u8; 4]; area.width() as usize];
    for y in (area.y0..area.y1).step_by(step.isqrt().max(1)) {
        surf.read_rgba8_into(Rect::new(area.x0, y, area.x1, y + 1), &mut row);
        for p in row.iter().step_by(step.isqrt().max(1)) {
            if p[3] == 0 {
                continue;
            }
            for c in 0..3 {
                h[c][p[c] as usize] += 1;
            }
        }
    }
    h
}

/// Black and white points with `clip` (fraction) clipped at each end.
fn clip_points(h: &[u64; 256], clip: f64) -> (f32, f32) {
    let total: u64 = h.iter().sum();
    if total == 0 {
        return (0.0, 1.0);
    }
    let lim = (total as f64 * clip) as u64;
    let (mut acc, mut lo) = (0u64, 0usize);
    while lo < 254 && acc + h[lo] <= lim {
        acc += h[lo];
        lo += 1;
    }
    let (mut acc, mut hi) = (0u64, 255usize);
    while hi > lo + 1 && acc + h[hi] <= lim {
        acc += h[hi];
        hi -= 1;
    }
    (lo as f32 / 255.0, hi as f32 / 255.0)
}

fn median(h: &[u64; 256]) -> f32 {
    let total: u64 = h.iter().sum();
    let mut acc = 0;
    for (i, v) in h.iter().enumerate() {
        acc += v;
        if acc * 2 >= total {
            return i as f32 / 255.0;
        }
    }
    0.5
}

/// Image › Auto Tone / Auto Contrast / Auto Color (0.1% clipping, Photoshop's defaults).
fn auto_adjust(s: &mut Session, kind: &str) -> Result<Value> {
    let id = active_id(s)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let surf = d.doc.layer(id).and_then(Layer::surface).ok_or(EngineError::Other("the active layer has no pixels".into()))?;
    let area = d.doc.selection.as_ref().map_or(d.doc.bounds(), |m| m.content_bounds()).intersect(&d.doc.bounds());
    let h = histograms(surf, area);
    let clip = 0.001;
    let mut per = [LevelsChannel::default(), LevelsChannel::default(), LevelsChannel::default()];
    match kind {
        "contrast" => {
            // Same points for all channels (preserves colour), from the combined histogram.
            let mut all = [0u64; 256];
            for c in &h {
                for (a, v) in all.iter_mut().zip(c) {
                    *a += v;
                }
            }
            let (lo, hi) = clip_points(&all, clip);
            for p in &mut per {
                (p.in_black, p.in_white) = (lo, hi);
            }
        }
        _ => {
            for (c, p) in per.iter_mut().enumerate() {
                (p.in_black, p.in_white) = clip_points(&h[c], clip);
            }
            if kind == "color" {
                // Neutralise midtones: move each channel's median onto the average median.
                let meds: Vec<f32> = (0..3).map(|c| median(&h[c])).collect();
                let target = meds.iter().sum::<f32>() / 3.0;
                for (c, p) in per.iter_mut().enumerate() {
                    let t = ((meds[c] - p.in_black) / (p.in_white - p.in_black).max(1e-3)).clamp(0.02, 0.98);
                    let want = ((target - p.in_black) / (p.in_white - p.in_black).max(1e-3)).clamp(0.02, 0.98);
                    // out = t^(1/g) = want  →  g = ln t / ln want
                    p.gamma = (t.ln() / want.ln()).clamp(0.1, 9.99);
                }
            }
        }
    }
    let adj = Adjustment::Levels { master: LevelsChannel::default(), per_channel: per, space: Default::default(), black: LevelsChannel::default() };
    let label = match kind {
        "contrast" => "Auto Contrast",
        "color" => "Auto Color",
        _ => "Auto Tone",
    };
    s.edit(label, |doc, _| {
        let sel = doc.selection.clone();
        let mode = doc.mode;
        let surf = crate::commands::paint_surface(doc, id, &Value::Null)?;
        crate::pixels::adjust_surface(surf, &adj, sel.as_ref(), mode).map_err(EngineError::Other)?;
        Ok(())
    })?;
    Ok(Value::Null)
}

fn toggle_last_state(s: &mut Session) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let redo = st.history.can_redo();
    Ok(json!(if redo { s.redo() } else { s.undo() }))
}

/// Edit › Transform › Again: replay the last `edit.transform` on the selected layers (#6501).
fn transform_again(s: &mut Session) -> Result<Value> {
    let ids = s.active().map(crate::transform_cmds::selection_targets).unwrap_or_default();
    let p = transform_again_params(s, &ids)?;
    s.execute("edit.transform", p)
}

/// Transform Again on a copy (⌥⇧⌘T, no menu item): duplicates the active layer and repeats the
/// last transform on the copy, as one history step, so pressing it again steps and repeats
/// (#352). The transform is worked out before anything changes, so a refusal leaves no copy.
fn transform_again_copy(s: &mut Session) -> Result<Value> {
    let id = active_id(s)?;
    let p = transform_again_params(s, &[id])?;
    let journal = s.journal.len();
    // The steps are recorded (Again on a Copy is not replayable itself): with one layer selected,
    // "duplicate the current layer" plays back on any document, a layer id only on this one.
    let dup = if crate::layer_multi_cmds::multi(s, &Value::Null) { json!({"layer": id.0}) } else { json!({}) };
    s.execute("layer.duplicate", dup)?;
    match s.execute("edit.transform", p) {
        Ok(r) => {
            let st = s.active_mut().ok_or(EngineError::NoDocument)?;
            st.history.purge_last();
            st.history.set_current_label("Transform Again");
            Ok(r)
        }
        Err(e) => {
            // The copy (e.g. of a layer whose position is locked) couldn't be transformed: take
            // it back, leaving nothing to redo and no Duplicate step to record.
            s.rollback();
            s.journal.truncate(journal);
            Err(e)
        }
    }
}

/// The `edit.transform` params that repeat the last transform on the layers `ids` (their frame
/// together is the box it starts from).
fn transform_again_params(s: &Session, ids: &[LayerId]) -> Result<Value> {
    let (_, last) =
        s.journal.iter().rev().find(|(id, _)| id == "edit.transform").cloned().ok_or(EngineError::Other("there is no transform to repeat".into()))?;
    let (rect, quad) = (last.get("rect").cloned(), last.get("quad").cloned());
    let mut p = json!({"interpolation": last.get("interpolation").cloned().unwrap_or(json!("bicubic"))});
    match (rect, quad, last.get("matrix").cloned()) {
        (Some(r), Some(q), _) => {
            let v = |x: &Value| -> Vec<f64> { x.as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default() };
            let r = v(&r);
            let q: Vec<Vec<f64>> = q.as_array().map(|a| a.iter().map(v).collect()).unwrap_or_default();
            if r.len() != 4 || q.len() != 4 || q.iter().any(|row| row.len() < 2) {
                return Err(EngineError::Other("the last transform can't be repeated".into()));
            }
            let h = photocraft_algo::transform::Homography::rect_to_quad(
                [r[0], r[1], r[2], r[3]],
                [[q[0][0], q[0][1]], [q[1][0], q[1][1]], [q[2][0], q[2][1]], [q[3][0], q[3][1]]],
            )
            .ok_or(EngineError::Other("the last transform can't be repeated".into()))?;
            let d = s.active().ok_or(EngineError::NoDocument)?;
            d.doc.layer(active_id(s)?).ok_or(EngineError::Other("no active layer".into()))?;
            let b = crate::transform_cmds::transform_bounds_of(&d.doc, ids);
            if b.is_empty() {
                return Err(EngineError::Other("nothing to transform".into()));
            }
            let nr = [b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64];
            let corners = [(nr[0], nr[1]), (nr[2], nr[1]), (nr[2], nr[3]), (nr[0], nr[3])].map(|(x, y)| {
                let (a, b) = h.apply(x, y);
                [a, b]
            });
            p["rect"] = json!(nr);
            p["quad"] = json!(corners);
        }
        (_, _, Some(m)) => p["matrix"] = m,
        _ => return Err(EngineError::Other("the last transform can't be repeated".into())),
    }
    Ok(p)
}

/// Guides further than this from the canvas origin are refused: far past any document (300000 px)
/// yet well within f32 precision.
const MAX_GUIDE_PX: f64 = 1_000_000.0;

/// A guide `position` in document pixels: a number of pixels, or text with a unit as in Photoshop's
/// New Guide dialog ("50%", "2.5 cm", "12 pt"). Percent is of the document's width for a vertical
/// guide and its height for a horizontal one; points and picas follow Units & Rulers › Point Size.
/// `None` when there is no `position`.
fn guide_position(s: &Session, p: &Value, vertical: bool) -> Result<Option<f64>> {
    let Some(v) = p.get("position") else { return Ok(None) };
    let px = match (v.as_f64(), v.as_str()) {
        (Some(n), _) => n,
        (None, Some(text)) => {
            let (n, unit) = crate::prefs::Unit::parse_length(text).ok_or_else(|| {
                EngineError::Other(format!("`position` {text:?} is not a length: give a number of pixels, or a number with px, in, cm, mm, pt, pc or %"))
            })?;
            let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
            let extent = if vertical { doc.size.width } else { doc.size.height };
            unit.unwrap_or(crate::prefs::Unit::Pixels).to_px(n, doc.resolution_dpi as f64, extent as f64, s.prefs().units_and_rulers.point_size.per_inch())
        }
        _ => return Err(EngineError::Other("`position` must be a number of pixels or text with a unit".into())),
    };
    if !px.is_finite() || px.abs() > MAX_GUIDE_PX {
        return Err(EngineError::Other(format!("`position` is out of range (at most {MAX_GUIDE_PX} px from the canvas origin)")));
    }
    Ok(Some(px))
}

/// Photoshop's guide colours, in its order, by the names `view.newGuide` takes: New Guide's and
/// New Guide Layout's Color lists. Cyan (the default guide colour in Preferences, so a guide
/// dragged from a ruler reads as Cyan), Magenta and Yellow are the values New Guide Layout first
/// shipped with; the rest approximate Photoshop's on screen, rounded to web-safe steps.
pub const GUIDE_COLORS: [(&str, [u8; 3]); 9] = [
    ("lightBlue", [0x33, 0x66, 0xff]),
    ("lightRed", [0xff, 0x33, 0x33]),
    ("green", [0x33, 0xff, 0x33]),
    ("mediumBlue", [0x33, 0x33, 0xff]),
    ("yellow", [0xff, 0xcc, 0x00]),
    ("magenta", [0xff, 0x00, 0xff]),
    ("cyan", [0x4a, 0xff, 0xff]),
    ("lightGray", [0xe7, 0xe7, 0xe7]),
    ("black", [0x00, 0x00, 0x00]),
];

/// A guide `color`: one of [`GUIDE_COLORS`]' names or `"#rrggbb"`. `None` when there is no `color`
/// (the guide uses Preferences › Guides, Grid & Slices › Guide colour).
fn guide_color(p: &Value) -> Result<Option<[u8; 3]>> {
    let Some(v) = p.get("color") else { return Ok(None) };
    let text = v.as_str().ok_or(EngineError::Other("`color` must be a colour name or \"#rrggbb\"".into()))?;
    GUIDE_COLORS.iter().find(|(name, ..)| name.eq_ignore_ascii_case(text)).map(|(.., rgb)| *rgb).or_else(|| crate::prefs::parse_hex(text)).map(Some).ok_or_else(
        || EngineError::Other(format!("`color` {text:?} is not a guide colour: use \"#rrggbb\" or one of {}", GUIDE_COLORS.map(|c| c.0).join(", "))),
    )
}

/// Edit Selected Guides (the guide context menu): change one guide's orientation, position and
/// colour in one history step. A guide that keeps its orientation keeps its index; one that turns
/// goes to the end of the other list. Without `color` it keeps its own colour, or none.
fn edit_guide(s: &mut Session, p: &Value) -> Result<Value> {
    let orientation = |key: &str| match p.get(key).map(|v| v.as_str()) {
        None => Ok(None),
        Some(Some("horizontal")) => Ok(Some(false)),
        Some(Some("vertical")) => Ok(Some(true)),
        Some(_) => Err(EngineError::Other(format!("`{key}` must be \"horizontal\" or \"vertical\""))),
    };
    let from = orientation("orientation")?.unwrap_or(false);
    let to = orientation("newOrientation")?.unwrap_or(from);
    let index = p.get("index").and_then(Value::as_u64).ok_or(EngineError::Other("missing `index`".into()))?;
    let pos = guide_position(s, p, to)?;
    let color = guide_color(p)?;
    s.edit("Edit Guide", |doc, _| {
        let guides = &mut doc.guides;
        let list = if from { &guides.vertical } else { &guides.horizontal };
        let i = usize::try_from(index).ok().filter(|i| *i < list.len()).ok_or(EngineError::Other("no such guide".into()))?;
        let old = list[i];
        let at = pos.unwrap_or(old);
        let rgb = color.or_else(|| guides.color(from, old));
        if from == to {
            let list = if from { &mut guides.vertical } else { &mut guides.horizontal };
            list[i] = at;
        } else {
            (if from { &mut guides.vertical } else { &mut guides.horizontal }).remove(i);
            (if to { &mut guides.vertical } else { &mut guides.horizontal }).push(at);
        }
        guides.prune_colors();
        if rgb.is_some() {
            guides.set_color(to, at, rgb);
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// View › New Guide / guide moves (undoable, like Photoshop's "New Guide"/"Move Guide" states).
fn guide_cmd(s: &mut Session, p: &Value, op: &str) -> Result<Value> {
    let vertical = p.get("orientation").and_then(Value::as_str).unwrap_or("horizontal") == "vertical";
    // Move Guide can turn the guide (⌥ while dragging it, as in Photoshop): `toOrientation`.
    let to_vertical = match p.get("toOrientation").map(|v| v.as_str()) {
        None => vertical,
        Some(Some("vertical")) => true,
        Some(Some("horizontal")) => false,
        Some(_) => return Err(EngineError::Other("`toOrientation` must be \"horizontal\" or \"vertical\"".into())),
    };
    if to_vertical != vertical && op != "move" {
        return Err(EngineError::Other("`toOrientation` applies to view.moveGuide only".into()));
    }
    // A % position is of the axis the guide ends up on.
    let pos = guide_position(s, p, to_vertical)?;
    let color = if op == "new" { guide_color(p)? } else { None };
    let index = p.get("index").and_then(Value::as_u64).map(|v| v as usize);
    let label = match op {
        "new" => "New Guide",
        "move" => "Move Guide",
        "delete" => "Delete Guide",
        _ => "Clear Guides",
    };
    s.edit(label, |doc, _| {
        let guides = &mut doc.guides;
        let g = if vertical { &mut guides.vertical } else { &mut guides.horizontal };
        match op {
            "new" => {
                let at = pos.ok_or(EngineError::Other("missing `position`".into()))?;
                g.push(at);
                if color.is_some() {
                    guides.set_color(vertical, at, color);
                }
            }
            "move" => {
                let i = index.filter(|i| *i < g.len()).ok_or(EngineError::Other("no such guide".into()))?;
                let to = pos.ok_or(EngineError::Other("missing `position`".into()))?;
                let from = if to_vertical == vertical {
                    std::mem::replace(&mut g[i], to)
                } else {
                    let from = g.remove(i);
                    if to_vertical {
                        guides.vertical.push(to)
                    } else {
                        guides.horizontal.push(to)
                    }
                    from
                };
                // The guide takes its colour along; another guide left at the old position keeps it.
                if let Some(rgb) = guides.color(vertical, from) {
                    guides.set_color(to_vertical, to, Some(rgb));
                }
                guides.prune_colors();
            }
            "delete" => {
                let i = index.filter(|i| *i < g.len()).ok_or(EngineError::Other("no such guide".into()))?;
                g.remove(i);
                guides.prune_colors();
            }
            _ => {
                guides.horizontal.clear();
                guides.vertical.clear();
                guides.colors.clear();
            }
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Farthest the ruler zero point may sit from the image, in document pixels.
const MAX_RULER_ORIGIN: f64 = 1.0e7;

/// Move the rulers' zero point (`view.setRulerOrigin`), or put it back at the image's top-left
/// (`view.resetRulerOrigin`, `p` = None). Undoable like the guides.
fn ruler_origin_cmd(s: &mut Session, p: Option<&Value>) -> Result<Value> {
    let origin = match p {
        None => [0.0, 0.0],
        Some(p) => {
            let axis = |k: &str| {
                p.get(k)
                    .and_then(Value::as_f64)
                    .filter(|v| v.is_finite() && v.abs() <= MAX_RULER_ORIGIN)
                    .ok_or_else(|| EngineError::Other(format!("`{k}` must be a number of pixels within ±{MAX_RULER_ORIGIN}")))
            };
            [axis("x")?, axis("y")?]
        }
    };
    let label = if p.is_some() { "Set Ruler Origin" } else { "Reset Ruler Origin" };
    s.edit(label, |doc, _| {
        doc.ruler_origin = origin;
        Ok(())
    })?;
    Ok(json!({"x": origin[0], "y": origin[1]}))
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $sc:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: $sc, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    vec![
        spec!(
            "edit.cut",
            "Cut",
            &["Edit"],
            Some("Cmd+X"),
            r##"{"target":"mask|filterMask"|{"color":i}|{"colors":[i,…]}?=Channels panel target,"layer":id|name?=active (colour targets only)} (a targeted mask or colour channel cuts that channel only; a colour set cuts its selected components)"##,
            has_pixels,
            |s, p| {
                if crate::commands::is_mask_target(p) {
                    let id = active_id(s)?;
                    crate::commands::check_all_unlocked(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
                    let result = copy_mask(s, crate::channel_cmds::target_of(p) == crate::channel_cmds::Target::FilterMask)?;
                    let bg = s.tools.background;
                    s.edit("Cut Mask", |doc, _| clear_mask(doc, id, bg, p))?;
                    return Ok(result);
                }
                // Refuse a locked layer before copying, so a refused Cut leaves the clipboard alone.
                let colors = crate::color_channels::target(s, p)?.filter(|c| c.len() > 1 || p.get("target").is_some() || p.get("layer").is_some());
                // Composite Cut retains Copy's active-layer semantics. A native colour target uses
                // the same resolved destination for both the copied planes and the cleared samples.
                let id = if colors.is_some() { crate::commands::layer_param(s, p)? } else { active_id(s)? };
                crate::commands::check_pixels_unlocked(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
                if let Some(colors) = colors {
                    let result = crate::channel_clip::copy_colors(s, p, colors)?;
                    let bg = s.tools.background;
                    s.edit("Cut Pixels", |doc, _| {
                        for k in colors.indices() {
                            crate::channel_cmds::clear_color_channel(doc, id, k, bg)?;
                        }
                        Ok(())
                    })?;
                    return Ok(result);
                }
                // A targeted colour channel cuts that channel only (#2698).
                if let Some(k) = crate::channel_clip::paste_color_target(s, p) {
                    return crate::channel_clip::cut(s, id, k);
                }
                let r = copy(s, false)?;
                let bg = s.tools.background;
                s.edit("Cut Pixels", |doc, _| clear_selected(doc, id, bg))?;
                Ok(r)
            }
        ),
        spec!(
            "edit.copy",
            "Copy",
            &["Edit"],
            Some("Cmd+C"),
            r##"{"target":"mask|filterMask"|{"color":i}|{"colors":[i,…]}?=Channels panel target,"layer":id|name?=active (colour targets only)} (a targeted mask, colour or alpha channel copies that channel as grayscale; a colour set copies its selected components)"##,
            can_copy,
            |s, p| {
                if crate::commands::is_mask_target(p) {
                    return copy_mask(s, crate::channel_cmds::target_of(p) == crate::channel_cmds::Target::FilterMask);
                }
                match crate::channel_clip::copy(s, p)? {
                    Some(r) => Ok(r),
                    None => copy(s, false),
                }
            }
        ),
        spec!("edit.copyMerged", "Copy Merged", &["Edit"], Some("Cmd+Shift+C"), "{}", has_doc, |s, _| copy(s, true)),
        spec!(
            "edit.paste",
            "Paste",
            &["Edit"],
            Some("Cmd+V"),
            concat!(
                r#"{"center":[x,y]? (view centre; default keeps the position when it overlaps the canvas),"clampToCanvas":bool=true (keep the paste on the canvas, or at its top-left when larger),"#,
                paste_target!(),
                "} (with no document open: a new document from the clipboard)"
            ),
            has_clip_only,
            |s, p| paste(s, p, false)
        ),
        spec!(
            "file.newFromClipboard",
            "New from Clipboard",
            &["File"],
            None,
            "{} (a new document the size of the clipboard image, holding it as one layer)",
            has_clip_only,
            |s, _| new_from_clipboard(s)
        ),
        spec!(
            "edit.pasteSpecial.pasteInPlace",
            "Paste in Place",
            &["Edit", "Paste Special"],
            Some("Cmd+Shift+V"),
            concat!("{", paste_target!(), "}"),
            has_clip,
            |s, p| paste(s, p, true)
        ),
        spec!("layer.new.layerViaCopy", "Layer via Copy", &["Layer", "New"], Some("Cmd+J"), "{}", has_doc, |s, _| layer_via(s, false)),
        spec!("layer.new.layerViaCut", "Layer via Cut", &["Layer", "New"], Some("Cmd+Shift+J"), "{}", has_pixels, |s, _| layer_via(s, true)),
        spec!("layer.mergeVisible", "Merge Visible", &["Layer"], Some("Cmd+Shift+E"), "{}", has_doc, |s, _| merge_visible(s)),
        spec!("image.autoTone", "Auto Tone", &["Image"], Some("Cmd+Shift+L"), "{}", has_pixels, |s, _| auto_adjust(s, "tone")),
        spec!("image.autoContrast", "Auto Contrast", &["Image"], Some("Cmd+Alt+Shift+L"), "{}", has_pixels, |s, _| auto_adjust(s, "contrast")),
        spec!("image.autoColor", "Auto Color", &["Image"], Some("Cmd+Shift+B"), "{}", has_pixels, |s, _| auto_adjust(s, "color")),
        spec!("edit.toggleLastState", "Toggle Last State", &["Edit"], Some("Cmd+Alt+Z"), "{}", has_doc, |s, _| toggle_last_state(s)),
        spec!("edit.transform.again", "Again", &["Edit", "Transform"], Some("Cmd+Shift+T"), "{}", has_doc, |s, _| transform_again(s)),
        spec!(
            "edit.transform.againCopy",
            "Transform Again on a Copy",
            &[],
            Some("Cmd+Alt+Shift+T"),
            "{} (duplicates the active layer and repeats the last transform on the copy: step and repeat)",
            has_doc,
            |s, _| transform_again_copy(s)
        ),
        spec!(
            "view.newGuide",
            "New Guide…",
            &["View"],
            None,
            r##"{"orientation":"horizontal|vertical","position":px or "<number><unit>","color":"lightBlue|lightRed|green|mediumBlue|yellow|magenta|cyan|lightGray|black" or "#rrggbb"?} (units px, in, cm, mm, pt, pc or %; % is of the width for a vertical guide, the height for a horizontal one; without a color the guide uses the guide colour in Preferences)"##,
            has_doc,
            |s, p| guide_cmd(s, p, "new")
        ),
        spec!(
            "view.moveGuide",
            "Move Guide",
            &[],
            None,
            r##"{"orientation":"horizontal|vertical","index":n,"position":px or "<number><unit>","toOrientation":"horizontal|vertical"?} (units as for view.newGuide; toOrientation turns the guide, its position then on the new axis)"##,
            has_doc,
            |s, p| guide_cmd(s, p, "move")
        ),
        spec!(
            "view.editGuide",
            "Edit Guide",
            &[],
            None,
            r##"{"orientation":"horizontal|vertical","index":n,"newOrientation":"horizontal|vertical"?,"position":px or "<number><unit>"?,"color":name or "#rrggbb"?} (Edit Selected Guides: units and colours as for view.newGuide; % is of the new orientation's extent; whatever is left out stays as it was)"##,
            has_doc,
            edit_guide
        ),
        spec!("view.deleteGuide", "Delete Guide", &[], None, r##"{"orientation":"horizontal|vertical","index":n}"##, has_doc, |s, p| guide_cmd(s, p, "delete")),
        spec!("view.clearGuides", "Clear Guides", &["View"], None, "{}", has_doc, |s, p| guide_cmd(s, p, "clear")),
        spec!(
            "view.setRulerOrigin",
            "Set Ruler Origin",
            &[],
            None,
            r##"{"x":px,"y":px} (the rulers' zero point in document pixels; the rulers and Info panel measure from it)"##,
            has_doc,
            |s, p| ruler_origin_cmd(s, Some(p))
        ),
        spec!("view.resetRulerOrigin", "Reset Ruler Origin", &[], None, "{} (zero point back at the image's top-left)", has_doc, |s, _| ruler_origin_cmd(
            s, None
        )),
    ]
}

#[cfg(test)]
mod tests;
