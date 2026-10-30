//! Edit › Free Transform / Transform: projective transforms of layers (and the selection).

use photocraft_algo::transform::{Homography, Interp, WARP_REACH, warp_output_clipped, warp_output_size, warp_surface, warp_surface_with};
use photocraft_color::PixelFormat;
#[cfg(test)]
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, Locks};
use photocraft_geom::{Affine, Rect};
use photocraft_raster::{Interrupt, Surface};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    s.active().and_then(|d| d.active_layer).map(|_| ()).ok_or_else(|| "no active layer".into())
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "edit.transform".into(), msg: msg.into() }
}

/// Layers a transform moves as a whole, even with a selection: groups, and the kinds that keep the
/// transform instead of pixels (type, shapes, smart objects). Only pixel layers move just the
/// selected pixels.
pub fn moves_whole(layer: &Layer) -> bool {
    matches!(layer.content, LayerContent::Group(_) | LayerContent::Text(_) | LayerContent::Shape(_) | LayerContent::Smart(_))
}

/// Document-space bounds a transform of `layer` starts from (what Free Transform frames): the
/// content within the selection, or all of it for a layer that [`moves_whole`] (#2630).
/// Content scans are cached per tile: snapping asks for every layer's bounds per Move drag.
pub fn transform_bounds(doc: &Document, layer: &Layer) -> Rect {
    let content = whole_layer_bounds(doc, layer);
    match &doc.selection {
        Some(sel) if !moves_whole(layer) => content.intersect(&sel.content_bounds()),
        _ => content,
    }
}

/// The box Free Transform opens on `layer`, and the Move tool's transform controls show (#5297).
/// With a selection that limits the transform it is the selection's own bounds, as in Photoshop:
/// the box and the marching ants are one rectangle, however little of the selection the layer
/// fills. Empty, like [`transform_bounds`], when the selection holds none of the layer.
pub fn transform_frame(doc: &Document, layer: &Layer) -> Rect {
    let b = transform_bounds(doc, layer);
    if moves_whole(layer) { b } else { framed_by_selection(doc, b) }
}

/// `bounds` (what a selection left of a transform's content), or the selection's own bounds.
fn framed_by_selection(doc: &Document, bounds: Rect) -> Rect {
    match &doc.selection {
        Some(sel) if !bounds.is_empty() => sel.content_bounds(),
        _ => bounds,
    }
}

/// A group moves all its descendants, so its frame must not clip raster children to the selection.
pub(crate) fn whole_layer_bounds(doc: &Document, layer: &Layer) -> Rect {
    let content = match &layer.content {
        LayerContent::Group(g) => g.children.iter().map(|c| whole_layer_bounds(doc, c)).fold(Rect::EMPTY, |a, b| a.union(&b)),
        // A soft shape (Density below 100 % or a Feather) renders past its path, over the whole
        // canvas below 100 %: its frame follows the path and stroke, as in Photoshop (#5846).
        LayerContent::Shape(sh) if sh.has_soft_mask() => photocraft_compose::soft_shape_frame(sh, doc.pixel_format(), doc.bounds()).unwrap_or(Rect::EMPTY),
        _ => layer.surface().map_or(Rect::EMPTY, photocraft_compose::bounds::content_bounds),
    };
    if content.is_empty() { layer.mask.as_ref().map_or(Rect::EMPTY, |m| photocraft_compose::bounds::content_bounds(&m.surface)) } else { content }
}

/// Warp a surface whose pixels outside the content read as `default` (masks, selections):
/// warp the content with alpha, then flatten back onto the default value.
pub(crate) fn warp_gray(s: &Surface, h: &Homography, interp: Interp) -> Result<Surface> {
    let default = s.default_pixel().first().copied().unwrap_or(0.0);
    let fmt = s.format();
    let src = s.content_bounds();
    let mut out = Surface::with_default(fmt, &[default]);
    if src.is_empty() {
        return Ok(out);
    }
    // Content-only copy (default 0 + alpha) so the "outside" is transparent during the warp.
    let with_alpha = PixelFormat::new(fmt.mode, fmt.sample, true);
    let mut tmp = Surface::new(with_alpha);
    let v = crate::allocation::read_region(s, src, "transforming a mask or channel")?;
    let mut lifted =
        crate::allocation::filled(v.len().checked_mul(2).ok_or_else(|| bad("the mask is too large to transform"))?, 0.0, "transforming a mask or channel")?;
    let (lifted_pairs, _) = lifted.as_chunks_mut::<2>();
    for (g, px) in v.iter().zip(lifted_pairs) {
        px.copy_from_slice(&[*g, 1.0]);
    }
    tmp.try_write_region(src, &lifted)?;
    let w = warp_surface(&tmp, src, h, interp)?;
    // Clear the old content region, then composite the warped content over it. These are written as
    // two sparse regions rather than one dense `src ∪ warped` block: a transform that moves the
    // content far away (e.g. a huge translation) would otherwise allocate a buffer spanning both and
    // hang/OOM. `Surface` is tile-sparse, so distant regions cost only their own tiles.
    let old_len = (src.width() as usize).checked_mul(src.height() as usize).ok_or_else(|| bad("the mask is too large to transform"))?;
    let old = crate::allocation::filled(old_len, default, "transforming a mask or channel")?;
    out.try_write_region(src, &old)?;
    let b = w.content_bounds();
    if !b.is_empty() {
        let px = crate::allocation::read_region(&w, b, "flattening a transformed mask or channel")?;
        let mut flat = crate::allocation::filled(px.len() / 2, 0.0, "flattening a transformed mask or channel")?;
        for (p, f) in px.as_chunks::<2>().0.iter().zip(&mut flat) {
            *f = p[0] * p[1] + default * (1.0 - p[1]);
        }
        out.try_write_region(b, &flat)?;
    }
    out.prune();
    Ok(out)
}

/// Split a single-channel surface whose outside reads as its default (a mask, a channel) by a
/// selection: (the selected values as grey + alpha = selection, the surface with the selected
/// values replaced by the default). `None` for surfaces that aren't a single grey channel.
pub fn split_gray_selected(s: &Surface, sel: &Surface) -> Result<Option<(Surface, Surface)>> {
    let fmt = s.format();
    if fmt.alpha || fmt.channels() != 1 {
        return Ok(None);
    }
    let default = s.default_pixel().first().copied().unwrap_or(0.0);
    let mut rest = s.clone();
    let mut lifted = Surface::new(PixelFormat::new(fmt.mode, fmt.sample, true));
    // Everything selected moves, including default-valued parts outside the painted content, so
    // the moved region stays aligned with the moved pixels.
    let area = sel.content_bounds();
    if area.is_empty() {
        return Ok(Some((lifted, rest)));
    }
    let v = crate::allocation::read_region(s, area, "transforming a selected mask or channel")?;
    let w = area.width() as usize;
    let mut lp = crate::allocation::capacity(
        v.len().checked_mul(2).ok_or_else(|| bad("the selected mask is too large to transform"))?,
        "transforming a selected mask or channel",
    )?;
    let mut rp = crate::allocation::capacity(v.len(), "transforming a selected mask or channel")?;
    for (i, g) in v.iter().enumerate() {
        let (x, y) = (area.x0 + (i % w) as i32, area.y0 + (i / w) as i32);
        let k = sel.sample_channel(x, y, 0);
        lp.extend([*g, k]);
        rp.push(g * (1.0 - k) + default * k);
    }
    lifted.try_write_region(area, &lp)?;
    lifted.prune();
    rest.try_write_region(area, &rp)?;
    rest.prune();
    Ok(Some((lifted, rest)))
}

/// [`warp_gray`] limited to a selection: only the selected values move (the vacated area reads as
/// the default), as the selected pixels of the layer do.
pub(crate) fn warp_gray_selected(s: &Surface, sel: &Surface, h: &Homography, interp: Interp) -> Result<Surface> {
    move_gray_selected(s, sel, &|lifted, src| Ok(warp_surface(lifted, src, h, interp)?), &|s| warp_gray(s, h, interp))
}

/// [`warp_gray_selected`] with any geometric warp: `warp` moves the lifted grey + alpha values
/// (given their bounds), `whole` moves `s` when it isn't a single grey channel.
pub(crate) fn move_gray_selected(
    s: &Surface,
    sel: &Surface,
    warp: &dyn Fn(&Surface, Rect) -> Result<Surface>,
    whole: &dyn Fn(&Surface) -> Result<Surface>,
) -> Result<Surface> {
    let Some((lifted, mut out)) = split_gray_selected(s, sel)? else { return whole(s) };
    let src = lifted.content_bounds();
    if src.is_empty() {
        return Ok(out);
    }
    let w = warp(&lifted, src)?;
    let b = w.content_bounds();
    if !b.is_empty() {
        let moved = crate::allocation::read_region(&w, b, "compositing transformed mask pixels")?;
        let under = crate::allocation::read_region(&out, b, "compositing transformed mask pixels")?;
        let mut flat = crate::allocation::filled(under.len(), 0.0, "compositing transformed mask pixels")?;
        for (i, (p, u)) in moved.as_chunks::<2>().0.iter().zip(&under).enumerate() {
            flat[i] = p[0] * p[1] + u * (1.0 - p[1]);
        }
        out.try_write_region(b, &flat)?;
    }
    out.prune();
    Ok(out)
}

/// The surface Free Transform moves by itself when the params target one (`"target"`): an
/// unlinked layer mask, an alpha channel or the Quick Mask. `None` means the layer, with its
/// linked masks (a targeted *linked* mask moves together with its layer, as in Photoshop).
pub fn lone_target<'a>(doc: &'a Document, layer: Option<LayerId>, p: &Value) -> Result<Option<&'a Surface>> {
    use crate::channel_cmds::Target;
    match crate::channel_cmds::target_of(p) {
        Target::Pixels => Ok(None),
        Target::Mask => {
            let id = layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            let m = l.mask.as_ref().ok_or_else(|| EngineError::Other("layer has no mask".into()))?;
            Ok((!m.linked).then_some(&m.surface))
        }
        // A filter mask is not transformed on its own: the smart object transforms, as in Photoshop.
        Target::FilterMask => Ok(None),
        Target::Alpha(i) => doc
            .channels
            .get(i)
            .map(|c| Some(&c.surface))
            .ok_or_else(|| EngineError::Other(format!("no alpha channel {i} (document has {})", doc.channels.len()))),
        Target::QuickMask => doc.quick_mask.as_ref().map(|c| Some(&c.surface)).ok_or_else(|| EngineError::Other("not in Quick Mask mode".into())),
    }
}

/// [`lone_target`], mutably.
pub fn lone_target_mut<'a>(doc: &'a mut Document, layer: Option<LayerId>, p: &Value) -> Result<Option<&'a mut Surface>> {
    if lone_target(doc, layer, p)?.is_none() {
        return Ok(None);
    }
    crate::channel_cmds::target_surface(doc, layer, p).map(|(s, _)| Some(s))
}

/// The frame Free Transform starts from on a lone target: its painted content (the whole canvas
/// when nothing is painted), within the selection.
pub fn target_bounds(doc: &Document, surf: &Surface) -> Rect {
    let content = surf.content_bounds();
    let content = if content.is_empty() { doc.bounds() } else { content };
    match &doc.selection {
        Some(sel) => content.intersect(&sel.content_bounds()),
        None => content,
    }
}

/// The box Free Transform opens on a lone target: like [`transform_frame`], the selection's own
/// bounds when there is one (#5297).
pub fn target_frame(doc: &Document, surf: &Surface) -> Rect {
    framed_by_selection(doc, target_bounds(doc, surf))
}

/// The locks `id` inherits from the groups around it.
pub(crate) fn group_locks(doc: &Document, id: LayerId) -> Locks {
    doc.path_of(id).and_then(|p| Some(doc.locks_at(p.split_last()?.1))).unwrap_or_default()
}

/// The locks a transform of `l` meets: its own and `group`'s (those it inherits). The Background's
/// position lock doesn't count: Photoshop turns it into a normal layer first.
fn transform_locks(l: &Layer, group: Locks, background: bool) -> Locks {
    let mut own = l.locks;
    if background {
        own.position = false;
        own.transparency = false;
    }
    own.union(group)
}

/// Would a transform of the layer `id` be refused because it, a group around it, or (for a group)
/// a layer inside it is locked in place? The same rule [`transform_layer`] applies at commit, so
/// Free Transform can refuse before its box opens, as Photoshop does.
pub fn locked_for_transform(doc: &Document, id: LayerId) -> bool {
    let Some(l) = doc.layer(id) else { return false };
    // An explicit stack rather than recursion: documents can nest deeply.
    let mut stack = vec![(l, group_locks(doc, id), doc.is_background(id))];
    while let Some((l, group, background)) = stack.pop() {
        let locks = transform_locks(l, group, background);
        if locks.position || locks.all {
            return true;
        }
        if let LayerContent::Group(g) = &l.content {
            stack.extend(g.children.iter().map(|c| (c, locks, false)));
        }
    }
    false
}

/// The layers Free Transform moves with the Layers panel's selection, bottom-to-top: with several
/// layers selected, every one that no selected group already carries, as in Photoshop (#6501), but
/// the Background, which Show Transform Controls doesn't frame either. With one layer selected (or
/// only the Background besides it), that layer; the active layer when nothing else is.
pub fn selection_targets(st: &crate::DocState) -> Vec<LayerId> {
    targets_of(&st.doc, &st.selected_layers(), st.active_layer)
}

/// [`selection_targets`] for the layers `selected`, `active` the active one: what an `edit.transform`
/// without `layer` or `layers` moves, so a recorded step can drop a `layers` list that matches it.
pub(crate) fn targets_of(doc: &Document, selected: &[LayerId], active: Option<LayerId>) -> Vec<LayerId> {
    if selected.len() < 2 {
        return active.into_iter().collect();
    }
    let ids: Vec<LayerId> = crate::layer_multi_cmds::top_level(doc, selected).into_iter().filter(|id| !doc.is_background(*id)).collect();
    if ids.is_empty() { active.into_iter().collect() } else { ids }
}

/// The `layers` param: the layers one box transforms together (at most 10000, each in the
/// document), without those a listed group carries. `None` when it isn't given.
fn layers_param(doc: &Document, p: &Value) -> Result<Option<Vec<LayerId>>> {
    let a = match p.get("layers") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(a)) if !a.is_empty() && a.len() <= 10_000 => a,
        Some(v) => return Err(bad(format!("`layers` is a non-empty list of at most 10000 layer ids, got {v}"))),
    };
    let mut ids = Vec::with_capacity(a.len());
    for v in a {
        let id = v.as_u64().map(LayerId).ok_or_else(|| bad(format!("`layers` holds layer ids, got {v}")))?;
        if doc.layer(id).is_none() {
            return Err(EngineError::NoLayer(id));
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(Some(crate::layer_multi_cmds::top_level(doc, &ids)))
}

/// The box Free Transform opens on the layers `ids` together: their frames ([`transform_frame`])
/// joined, as Show Transform Controls draws it. Empty when none of them has anything to move.
pub fn transform_frame_of(doc: &Document, ids: &[LayerId]) -> Rect {
    ids.iter().filter_map(|id| doc.layer(*id)).map(|l| transform_frame(doc, l)).filter(|b| !b.is_empty()).fold(Rect::EMPTY, |a, b| a.union(&b))
}

/// [`transform_bounds`] of the layers `ids` together.
pub fn transform_bounds_of(doc: &Document, ids: &[LayerId]) -> Rect {
    ids.iter().filter_map(|id| doc.layer(*id)).map(|l| transform_bounds(doc, l)).filter(|b| !b.is_empty()).fold(Rect::EMPTY, |a, b| a.union(&b))
}

/// The memory (bytes) a single Free Transform warp's result may take: Preferences › Performance ›
/// Memory Usage, which [`Session::apply_prefs`] sets, rather than a fixed 2³⁰ px, so a stitched
/// layer of a few gigapixels transforms on a machine that has the memory for it (#7807). Only the
/// size of the result counts, not where it lands, so moving a layer far off the canvas stays
/// allowed as far as the warp reaches ([`WARP_REACH`]). A scale past it is refused rather than
/// clipped (#3773): the warp would otherwise fill every tile of a result up to 2²¹ px on a side
/// before anything could fail.
static BUDGET: AtomicU64 = AtomicU64::new((crate::prefs::DEFAULT_MEMORY_USAGE_MB as u64) << 20);

/// Set the memory (bytes) one transform's result may take (see [`budget`]).
pub fn set_budget(bytes: u64) {
    BUDGET.store(bytes, Ordering::Relaxed);
}

/// The memory (bytes) one transform's result may take: Memory Usage.
pub fn budget() -> u64 {
    BUDGET.load(Ordering::Relaxed)
}

/// What [`check_budget`] holds a result to: [`budget`].
#[cfg(not(test))]
fn active_budget() -> u64 {
    budget()
}

/// Tests use a far lower budget, so proving a refusal never comes near allocating the real one:
/// 2²⁰ px of the widest format (32-bit CMYK with alpha, 20 MB), so a result within the old 2²⁰ px
/// test limit still fits at any depth.
#[cfg(test)]
fn active_budget() -> u64 {
    (1 << 20) * PixelFormat::new(ColorMode::Cmyk, SampleType::F32, true).bytes_per_pixel() as u64
}

/// Refuse a warp of the content inside `src` (pixels of `format`) by `h` whose result would take
/// more memory than the budget, or reach past where the warp places pixels (it would be clipped),
/// before anything is built. `what` names the surface in the message, which names the setting or
/// the reach rather than a shortage: nothing has been allocated yet.
pub(crate) fn check_budget(src: Rect, h: &Homography, what: &str, format: PixelFormat) -> Result<()> {
    check_budget_with_limit(src, h, what, format, active_budget())
}

/// [`check_budget`] against `budget` bytes (the tests price the production budget with it).
fn check_budget_with_limit(src: Rect, h: &Homography, what: &str, format: PixelFormat, budget: u64) -> Result<()> {
    let Some((w, ht)) = warp_output_size(src, h) else { return Ok(()) };
    // The warp writes each tile into the result as it goes, so the result is what it holds; it
    // adds alpha to a format without it.
    let bpp = PixelFormat::new(format.mode, format.sample, true).bytes_per_pixel() as u64;
    let bytes = w.saturating_mul(ht).saturating_mul(bpp);
    if bytes > budget {
        let mb = |b: u64| b.div_ceil(1 << 20);
        return Err(EngineError::Other(format!(
            "too large to transform: the transformed {what} would be {w} x {ht} px ({} MB), more than the {} MB Memory Usage allows (Preferences › Performance); the document was not changed",
            mb(bytes),
            budget >> 20
        )));
    }
    if warp_output_clipped(src, h) {
        return Err(EngineError::Other(format!(
            "too large to transform: the transformed {what} would reach past ±{WARP_REACH} px, the farthest a transform places pixels; the document was not changed"
        )));
    }
    Ok(())
}

/// [`check_budget`] for every warp [`transform_layer`] would make of `l` (for a group, of every
/// layer inside it), so a transform that doesn't fit is refused before any layer is built and a
/// group that partly fits isn't half-transformed.
fn check_layer_budget(doc_sel: Option<&Surface>, l: &Layer, h: &Homography) -> Result<()> {
    if h.is_identity() {
        return Ok(());
    }
    // An explicit stack rather than recursion: documents can nest deeply.
    let mut stack = vec![(l, doc_sel)];
    while let Some((l, sel)) = stack.pop() {
        match &l.content {
            LayerContent::Group(g) => stack.extend(g.children.iter().map(|c| (c, None))),
            LayerContent::Text(_) | LayerContent::Shape(_) => {}
            LayerContent::Smart(sm) => {
                if let Some(c) = &sm.cache {
                    check_budget(c.content_bounds(), h, "smart object", c.format())?;
                }
                if let Some(m) = &sm.filter_mask {
                    check_budget(m.surface.content_bounds(), h, "smart filter mask", m.surface.format())?;
                }
            }
            _ => {
                if let Some(surf) = l.surface() {
                    // With a selection only the lifted pixels are warped (`split_selected`).
                    let src = match sel {
                        Some(sel) => surf.content_bounds().intersect(&sel.content_bounds()),
                        None => surf.content_bounds(),
                    };
                    check_budget(src, h, &format!("layer \"{}\"", l.name), surf.format())?;
                }
            }
        }
        if let Some(m) = &l.mask
            && m.linked
        {
            // With a selection only its region of the mask moves (`warp_gray_selected`).
            let src = match sel.filter(|_| !moves_whole(l)) {
                Some(sel) => sel.content_bounds(),
                None => m.surface.content_bounds(),
            };
            check_budget(src, h, "layer mask", m.surface.format())?;
        }
    }
    Ok(())
}

/// `group` holds the locks `l` inherits from the groups around it ([`group_locks`]). A Background
/// is turned into a normal layer by the caller first ([`crate::extra_cmds::background_to_layer`]).
/// Refused, with `l` untouched, when a result would take more than [`budget`].
pub(crate) fn transform_layer(doc_sel: Option<&Surface>, group: Locks, l: &mut Layer, h: &Homography, affine: Option<Affine>, interp: Interp) -> Result<()> {
    transform_layer_with(doc_sel, group, l, h, affine, interp, &Interrupt::NONE)
}

/// [`transform_layer`] with progress and cancellation for the pixel warp (the slow part on a
/// large layer, #1015).
fn transform_layer_with(
    doc_sel: Option<&Surface>,
    group: Locks,
    l: &mut Layer,
    h: &Homography,
    affine: Option<Affine>,
    interp: Interp,
    ctl: &Interrupt,
) -> Result<()> {
    check_layer_budget(doc_sel, l, h)?;
    apply_transform(doc_sel, group, l, h, affine, interp, ctl)
}

fn apply_transform(
    doc_sel: Option<&Surface>,
    group: Locks,
    l: &mut Layer,
    h: &Homography,
    affine: Option<Affine>,
    interp: Interp,
    ctl: &Interrupt,
) -> Result<()> {
    crate::allocation::checkpoint("transforming pixels")?;
    let locks = transform_locks(l, group, false);
    if locks.position || locks.all {
        return Err(EngineError::Other(format!("layer \"{}\" is locked", l.name)));
    }
    if h.is_identity() {
        if let LayerContent::Group(g) = &mut l.content {
            for c in g.children.iter_mut() {
                apply_transform(None, locks, c, h, affine, interp, ctl)?;
            }
        }
        return Ok(());
    }
    // With a selection only the selected pixels move, and so only the same region of a linked
    // mask (#205). Groups, type, shapes and smart objects move whole.
    let mask_sel = doc_sel.filter(|_| !moves_whole(l));
    // The masks (and so a fill's frame) move, but a fill layer's cached render isn't warped:
    // drop it, the fill re-renders from its model (as `transform_layer_geometry` does).
    l.fill_cache = None;
    match &mut l.content {
        LayerContent::Group(g) => {
            for c in g.children.iter_mut() {
                apply_transform(None, locks, c, h, affine, interp, ctl)?;
            }
        }
        LayerContent::Text(t) => {
            let Some(a) = affine else {
                return Err(EngineError::Other("Distort and Perspective need rasterized type (Type › Rasterize Type Layer)".into()));
            };
            t.transform = a.mul(&t.transform);
        }
        LayerContent::Shape(sh) => match affine {
            Some(a) => crate::vector_cmds::transform_shape(sh, &a),
            // Distort and Perspective map the path and keep the shape (#3009).
            None => crate::vector_cmds::map_shape(sh, Some(h), None)?,
        },
        LayerContent::Smart(sm) => {
            // Smart objects keep the transform and re-render from their source afterwards
            // (`refresh_text`), so repeated transforms don't degrade the pixels. Distort and
            // Perspective keep the full projective map, so the fourth corner survives re-renders.
            match affine {
                Some(a) => crate::smart_cmds::transform_placement(sm, &a),
                None => crate::smart_cmds::set_placement(sm, h.mul(&crate::smart_cmds::placement(sm))),
            }
            // Fallback appearance for sources that can't be re-rendered.
            if let Some(c) = &mut sm.cache {
                let src = c.content_bounds();
                *c = warp_surface(c, src, h, interp)?;
            }
            if let Some(m) = &mut sm.filter_mask {
                m.surface = warp_gray(&m.surface, h, interp)?;
            }
        }
        _ => {
            if let Some(surf) = l.surface_mut() {
                *surf = match doc_sel {
                    // Only the selected pixels move: lift them, clear them, warp and paste back. The
                    // lifted pixels' own bounds are the edge a stretch repeats (#3773).
                    Some(sel) => {
                        let (lifted, mut rest) = split_selected(surf, sel)?;
                        let moved = warp_surface_with(&lifted, lifted.content_bounds(), h, interp, ctl)?.ok_or(EngineError::Cancelled)?;
                        composite_over(&mut rest, &moved);
                        rest.prune();
                        rest
                    }
                    None => {
                        let src = surf.content_bounds();
                        warp_surface_with(surf, src, h, interp, ctl)?.ok_or(EngineError::Cancelled)?
                    }
                };
            }
        }
    }
    if let Some(m) = l.mask.as_mut()
        && m.linked
    {
        m.surface = match mask_sel {
            Some(sel) => warp_gray_selected(&m.surface, sel, h, interp)?,
            None => warp_gray(&m.surface, h, interp)?,
        };
    }
    if let Some(vm) = l.vector_mask.as_mut()
        && vm.linked
    {
        vm.path = match affine {
            Some(a) => vm.path.transform(&a),
            None => crate::vector_cmds::map_path(&vm.path, Some(h), None)?,
        };
    }
    Ok(())
}

/// Split a layer surface by a selection: (selected pixels, everything else), both with alpha.
pub fn split_selected(surf: &Surface, sel: &Surface) -> Result<(Surface, Surface)> {
    let fmt = surf.format();
    let with_alpha = PixelFormat::new(fmt.mode, fmt.sample, true);
    let mut lifted = Surface::new(with_alpha);
    let mut rest = surf.try_convert(with_alpha)?;
    let src = surf.content_bounds();
    if src.is_empty() {
        return Ok((lifted, rest));
    }
    let n = with_alpha.channels();
    let px = crate::allocation::read_region(&rest, src, "transforming selected pixels")?;
    let mut lp = crate::allocation::filled(px.len(), 0.0, "transforming selected pixels")?;
    lp.copy_from_slice(&px);
    let mut rp = px;
    let w = src.width() as usize;
    // A pixel the selection fully lifts out is reset to the surface's (transparent) default;
    // keeping its colour would make `content_bounds` still count it, so the layer's bounds would
    // include the area the pixels were moved from.
    let mut rest_clear = rest.default_pixel();
    if rest_clear.last().is_some_and(|a| *a > 0.0) {
        rest_clear.fill(0.0);
    }
    for (i, (l, r)) in lp.chunks_exact_mut(n).zip(rp.chunks_exact_mut(n)).enumerate() {
        let (x, y) = (src.x0 + (i % w) as i32, src.y0 + (i / w) as i32);
        let k = sel.sample_channel(x, y, 0);
        l[n - 1] *= k;
        r[n - 1] *= 1.0 - k;
        // Likewise an unselected pixel leaves nothing in the lifted surface, so its bounds are the
        // selected pixels' (what a stretch repeats at the edge, #3773).
        if l[n - 1] <= 0.0 {
            l.fill(0.0);
        }
        if r[n - 1] <= 0.0 {
            r.iter_mut().zip(&rest_clear).for_each(|(d, c)| *d = *c);
        }
    }
    lifted.try_write_region(src, &lp)?;
    lifted.prune();
    rest.try_write_region(src, &rp)?;
    Ok((lifted, rest))
}

pub(crate) fn refresh_text(doc: &Document, l: &mut Layer) {
    match &mut l.content {
        LayerContent::Text(t) => crate::type_cmds::refresh(doc, t),
        LayerContent::Shape(sh) => crate::vector_cmds::refresh_shape(doc, sh),
        LayerContent::Smart(_) => {
            // Unavailable sources keep the warped cache.
            let _ = crate::smart_cmds::refresh_layer(doc, l);
        }
        LayerContent::Group(g) => g.children.iter_mut().for_each(|c| refresh_text(doc, c)),
        _ => {}
    }
}

/// Normal-blend `top` over `dst` (both straight alpha, same format).
/// Works tile by tile in parallel on `top`'s allocated tiles: fully opaque tiles share their
/// `Arc<Tile>` directly with `dst`, and opaque/transparent pixels on border tiles copy encoded
/// bytes without a whole-region `f32` round trip.
pub(crate) fn composite_over(dst: &mut Surface, top: &Surface) {
    use photocraft_color::{SampleType, read_sample};
    use photocraft_geom::TileCoord;
    use photocraft_raster::Tile;
    use rayon::prelude::*;
    use std::sync::Arc;

    let fmt = dst.format();
    let n = fmt.channels();
    if n == 0 || !fmt.alpha || top.format() != fmt {
        return;
    }
    let a = n - 1;
    if top.default_pixel().get(a).is_none_or(|&ta| ta <= 0.0) {
        let top_tiles: Vec<(TileCoord, Arc<Tile>)> = top.tiles().map(|(&tc, t)| (tc, t.clone())).collect();
        if top_tiles.is_empty() {
            return;
        }
        let bpp = fmt.bytes_per_pixel();
        let sb = fmt.sample.bytes();
        let alpha_off = bpp - sb;
        let dst_empty_default = dst.default_bytes().iter().all(|&b| b == 0);
        let dst_ref = &*dst;
        let edits: Vec<(TileCoord, Arc<Tile>)> = top_tiles
            .into_par_iter()
            .filter_map(|(tc, top_tile)| {
                let tb = top_tile.bytes();
                let all_opaque = match fmt.sample {
                    SampleType::U8 => tb.chunks_exact(bpp).all(|px| px.get(alpha_off) == Some(&u8::MAX)),
                    SampleType::U16 => tb.chunks_exact(bpp).all(|px| px.get(alpha_off..bpp) == Some(&[u8::MAX, u8::MAX])),
                    SampleType::F32 => tb.chunks_exact(bpp).all(|px| read_sample(px, SampleType::F32, a) == 1.0),
                };
                if all_opaque {
                    return Some((tc, top_tile));
                }
                let all_transparent = match fmt.sample {
                    SampleType::U8 => tb.chunks_exact(bpp).all(|px| px.get(alpha_off) == Some(&0)),
                    SampleType::U16 => tb.chunks_exact(bpp).all(|px| px.get(alpha_off..bpp) == Some(&[0, 0])),
                    SampleType::F32 => tb.chunks_exact(bpp).all(|px| read_sample(px, SampleType::F32, a) <= 0.0),
                };
                if all_transparent {
                    return None;
                }
                if dst_ref.tile(tc).is_none() && dst_empty_default {
                    let clean_zero = match fmt.sample {
                        SampleType::U8 => tb.chunks_exact(bpp).all(|px| px.get(alpha_off) != Some(&0) || px.iter().all(|&b| b == 0)),
                        SampleType::U16 => tb.chunks_exact(bpp).all(|px| px.get(alpha_off..bpp) != Some(&[0, 0]) || px.iter().all(|&b| b == 0)),
                        SampleType::F32 => tb.chunks_exact(bpp).all(|px| read_sample(px, SampleType::F32, a) > 0.0 || px.iter().all(|&b| b == 0)),
                    };
                    if clean_zero {
                        return Some((tc, top_tile));
                    }
                }
                let mut out_tile = match dst_ref.tile(tc) {
                    Some(dt) => (**dt).clone(),
                    None => dst_ref.try_default_tile().ok()?,
                };
                let mut tp = [0.0f32; 8];
                let mut dp = [0.0f32; 8];
                for (out_px, top_px) in out_tile.bytes_mut().chunks_exact_mut(bpp).zip(tb.chunks_exact(bpp)) {
                    match fmt.sample {
                        SampleType::U8 => {
                            let ta_b = top_px[alpha_off];
                            if ta_b == 0 {
                                continue;
                            }
                            if ta_b == u8::MAX {
                                out_px.copy_from_slice(top_px);
                                continue;
                            }
                        }
                        SampleType::U16 => {
                            if top_px[alpha_off..bpp] == [0, 0] {
                                continue;
                            }
                            if top_px[alpha_off..bpp] == [u8::MAX, u8::MAX] {
                                out_px.copy_from_slice(top_px);
                                continue;
                            }
                        }
                        SampleType::F32 => {
                            let ta = read_sample(top_px, SampleType::F32, a);
                            if ta <= 0.0 {
                                continue;
                            }
                            if ta == 1.0 {
                                out_px.copy_from_slice(top_px);
                                continue;
                            }
                        }
                    }
                    for c in 0..n {
                        tp[c] = read_sample(top_px, fmt.sample, c);
                        dp[c] = read_sample(out_px, fmt.sample, c);
                    }
                    let (ta, da) = (tp[a], dp[a]);
                    let oa = ta + da * (1.0 - ta);
                    if oa > 0.0 {
                        for c in 0..a {
                            dp[c] = (tp[c] * ta + dp[c] * da * (1.0 - ta)) / oa;
                        }
                    }
                    dp[a] = oa;
                    photocraft_raster::encode_pixel(&fmt, &dp[..n], out_px);
                }
                Some((tc, Arc::new(out_tile)))
            })
            .collect();
        dst.put_tiles(edits);
        return;
    }
    let b = top.content_bounds();
    if b.is_empty() {
        return;
    }
    let t = top.read_region(b);
    let mut d = dst.read_region(b);
    for (dp, tp) in d.chunks_exact_mut(n).zip(t.chunks_exact(n)) {
        let (ta, da) = (tp[a], dp[a]);
        let oa = ta + da * (1.0 - ta);
        if oa > 0.0 {
            for c in 0..a {
                dp[c] = (tp[c] * ta + dp[c] * da * (1.0 - ta)) / oa;
            }
        }
        dp[a] = oa;
    }
    dst.write_region(b, &d);
}

/// The relative form (`scaleX`, `scaleY` in percent, `rotate` in degrees clockwise, `dx`, `dy`
/// in pixels), as Photoshop records a Free Transform: scale and rotate about the frame's
/// centre, then move. `None` when none of them is given.
fn relative_matrix(p: &Value, rect: [f64; 4]) -> Result<Option<Homography>> {
    let num = |k: &str| -> Result<Option<f64>> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_f64().filter(|f| f.is_finite()).map(Some).ok_or_else(|| bad(format!("`{k}` must be a finite number"))),
        }
    };
    let (sx, sy, rot, dx, dy) = (num("scaleX")?, num("scaleY")?, num("rotate")?, num("dx")?, num("dy")?);
    if sx.is_none() && sy.is_none() && rot.is_none() && dx.is_none() && dy.is_none() {
        return Ok(None);
    }
    let (sx, sy) = (sx.unwrap_or(100.0) / 100.0, sy.unwrap_or(100.0) / 100.0);
    if sx.abs() < 1e-6 || sy.abs() < 1e-6 {
        return Err(bad("`scaleX` and `scaleY` must not be 0"));
    }
    let (cx, cy) = ((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0);
    let (sin, cos) = rot.unwrap_or(0.0).to_radians().sin_cos();
    // (x, y) → translate(-c) → scale → rotate → translate(c + d).
    let (a, b, c, d) = (sx * cos, sx * sin, -sy * sin, sy * cos);
    let e = cx + dx.unwrap_or(0.0) - a * cx - c * cy;
    let f = cy + dy.unwrap_or(0.0) - b * cx - d * cy;
    Ok(Some(Homography([a, c, e, b, d, f, 0.0, 0.0, 1.0])))
}

fn quad_param(p: &Value) -> Option<[[f64; 2]; 4]> {
    let a = p.get("quad")?.as_array()?;
    if a.len() != 4 {
        return None;
    }
    let mut q = [[0.0; 2]; 4];
    for (i, v) in a.iter().enumerate() {
        let c = v.as_array()?;
        q[i] = [c.first()?.as_f64()?, c.get(1)?.as_f64()?];
    }
    Some(q)
}

/// `p`, with the Channels panel's targeted alpha channel or Quick Mask as its `"target"` when it
/// names neither a target nor a layer (#2835).
fn with_panel_target(s: &Session, p: &Value) -> Value {
    let mut p = if p.is_object() { p.clone() } else { json!({}) };
    if p.get("target").is_none()
        && p.get("layer").is_none()
        && p.get("layers").is_none()
        && let (Some(m), Some(Value::Object(t))) = (p.as_object_mut(), crate::channel_cmds::targeted_channel(s))
    {
        m.extend(t);
    }
    p
}

fn transform(s: &mut Session, p: &Value) -> Result<Value> {
    let p = &with_panel_target(s, p);
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let many = layers_param(&st.doc, p)?;
    if many.is_some()
        && (!matches!(p.get("layer"), None | Some(Value::Null)) || !matches!(crate::channel_cmds::target_of(p), crate::channel_cmds::Target::Pixels))
    {
        return Err(bad("`layers` moves layers: pass it without `layer` or a mask or channel `target`"));
    }
    let explicit = crate::commands::explicit_layer(s, p)?;
    let id = explicit.or(st.active_layer);
    let lone = if many.is_some() { None } else { lone_target(&st.doc, id, p)? };
    let id = match (id, lone.is_some() && crate::channel_cmds::is_channel_target(p)) {
        (_, true) => None,
        (Some(id), false) => Some(id),
        (None, false) if many.is_some() => None,
        (None, false) => return Err(EngineError::Other("no active layer".into())),
    };
    // The layers the box moves: those listed, the one named, or else the layer selection (#6501).
    let ids: Vec<LayerId> = match (many, explicit) {
        (Some(ids), _) => ids,
        _ if lone.is_some() => Vec::new(),
        (None, Some(id)) => vec![id],
        (None, None) => selection_targets(st),
    };
    if let Some(missing) = ids.iter().find(|id| st.doc.layer(**id).is_none()) {
        return Err(EngineError::NoLayer(*missing));
    }
    let rect = match p.get("rect").and_then(Value::as_array) {
        Some(r) if r.len() == 4 => {
            let v: Vec<f64> = r.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect();
            [v[0], v[1], v[2], v[3]]
        }
        _ => {
            let b = match lone {
                Some(surf) => target_bounds(&st.doc, surf),
                None => transform_bounds_of(&st.doc, &ids),
            };
            if b.is_empty() {
                return Err(EngineError::Other("nothing to transform".into()));
            }
            [b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64]
        }
    };
    let h = if let Some(q) = quad_param(p) {
        Homography::rect_to_quad(rect, q).ok_or_else(|| bad("degenerate quad"))?
    } else if let Some(m) = p.get("matrix").and_then(Value::as_array) {
        let v: Vec<f64> = m.iter().filter_map(Value::as_f64).collect();
        if v.len() != 6 {
            return Err(bad("matrix must be [a, b, c, d, e, f]"));
        }
        // Affine [a b c d e f] maps (x, y) → (a·x + c·y + e, b·x + d·y + f).
        Homography([v[0], v[2], v[4], v[1], v[3], v[5], 0.0, 0.0, 1.0])
    } else if let Some(m) = relative_matrix(p, rect)? {
        m
    } else {
        return Err(bad("pass `quad` (where the rect's corners go), `matrix`, or `scaleX`/`scaleY`/`rotate`/`dx`/`dy`"));
    };
    if h.inverse().is_none() {
        return Err(bad("the transform collapses the layer"));
    }
    let m = h.0;
    let affine =
        (m[6].abs() < 1e-12 && m[7].abs() < 1e-12).then(|| Affine { m: [m[0] / m[8], m[3] / m[8], m[1] / m[8], m[4] / m[8], m[2] / m[8], m[5] / m[8]] });
    let interp = Interp::parse(p.get("interpolation").and_then(Value::as_str).unwrap_or("bicubic"));
    let lone = lone.is_some();
    let layers: Vec<u64> = ids.iter().map(|id| id.0).collect();
    let p = p.clone();
    // A background job when started with `Session::start` (#1015): the warp of a large layer runs
    // on a worker with progress, and can be cancelled.
    crate::jobs::edit_job(
        s,
        "Free Transform",
        move |doc, _, ctx| ctx.stage(0.0, 1.0, "Transforming", |ctl| transform_doc(doc, id, &ids, &p, lone, &h, affine, interp, ctl)),
        move |()| json!({"rect": rect, "layers": layers}),
    )
}

/// The edit of [`transform`]: the layers `ids` (or the lone mask or channel of `id`) and the
/// selection outline.
#[allow(clippy::too_many_arguments)]
fn transform_doc(
    doc: &mut Document,
    id: Option<LayerId>,
    ids: &[LayerId],
    p: &Value,
    lone: bool,
    h: &Homography,
    affine: Option<Affine>,
    interp: Interp,
    ctl: &Interrupt,
) -> Result<()> {
    let sel = doc.selection.clone();
    // The selection outline moves too: price it before anything is built.
    if let Some(sel) = sel.as_ref().filter(|_| !h.is_identity()) {
        check_budget(sel.content_bounds(), h, "selection", sel.format())?;
    }
    if lone {
        // A targeted unlinked mask, alpha channel or Quick Mask transforms by itself.
        if !h.is_identity() {
            let (surf, _) = crate::channel_cmds::target_surface(doc, id, p)?;
            let src = sel.as_ref().map_or_else(|| surf.content_bounds(), Surface::content_bounds);
            check_budget(src, h, "mask or channel", surf.format())?;
            *surf = match &sel {
                Some(sel) => warp_gray_selected(surf, sel, h, interp)?,
                None => warp_gray(surf, h, interp)?,
            };
            if let Some(sel) = &doc.selection {
                doc.selection = Some(warp_gray(sel, h, Interp::Bilinear)?).filter(|s| !s.content_bounds().is_empty());
            }
        }
        return Ok(());
    }
    if ids.is_empty() {
        return Err(EngineError::Other("no active layer".into()));
    }
    // A group moves whole: the selection doesn't limit what is inside it.
    let layer_sel = |l: &Layer| if l.is_group() { None } else { sel.as_ref() };
    // Every layer is priced before any is built, so one that doesn't fit leaves all as they were.
    for &id in ids {
        let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        check_layer_budget(layer_sel(l), l, h)?;
    }
    let mut taken = Vec::new();
    for &id in ids {
        let group = group_locks(doc, id);
        // Photoshop turns the Background into a normal layer before transforming it.
        crate::extra_cmds::background_to_layer(doc, id);
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let doc_sel = layer_sel(l);
        if !h.is_identity() {
            taken.extend(crate::smart_cmds::take_caches(std::slice::from_mut(l)));
        }
        apply_transform(doc_sel, group, l, h, affine, interp, ctl)?;
    }
    if !h.is_identity() {
        // Type layers re-render from their new transform.
        let snapshot = doc.clone();
        for &id in ids {
            if let Some(l) = doc.layer_mut(id) {
                refresh_text(&snapshot, l);
            }
        }
        crate::smart_cmds::restore_unrendered(doc, taken, |c| warp_surface(c, c.content_bounds(), h, interp))?;
        // The selection outline moves with the pixels.
        if let Some(sel) = &doc.selection {
            doc.selection = Some(warp_gray(sel, h, Interp::Bilinear)?).filter(|s| !s.content_bounds().is_empty());
        }
    }
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "edit.transform",
        label: "Free Transform",
        menu: &[],
        shortcut: None,
        params: r##"{"layer":id?,"layers":[id,…]? (several layers moved by one box, one step; default with neither = the selected layers but the Background, as Photoshop's Free Transform with several selected),"rect":[x0,y0,x1,y1]? (source frame; default = the layers' content ∩ selection),"quad":[[x,y]×4]? (where the frame's corners go, clockwise from top-left),"matrix":[a,b,c,d,e,f]? (affine alternative),"scaleX":%?,"scaleY":%?,"rotate":deg?,"dx":px?,"dy":px? (relative alternative: scale and rotate about the frame's centre, then move, as a Photoshop action records a Free Transform),"interpolation":"bicubic|bilinear|nearest|bicubicSharper|bicubicSmoother|bicubicAutomatic"="bicubic" (Photoshop's bicubic variants render as Bicubic),"target":"pixels"|"mask"|"quickMask"|{"channel":i}=Channels panel target (an unlinked mask, an alpha channel or the Quick Mask transforms alone; a linked mask moves with its layer)}"##,
        enabled: has_layer,
        journal: true,
        run: transform,
    }]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod multi_tests;
