use super::*;

/// The document pixels an edit of the layers `ids` (moved, shown, hidden or restyled; contents
/// otherwise as they were) can have changed from `before` to `after`, or `None` for anywhere.
pub fn layers_damage(before: &Document, after: &Document, ids: &[LayerId]) -> Option<Rect> {
    let mut out = Rect::EMPTY;
    for d in [before, after] {
        let canvas = d.bounds();
        for id in ids {
            // A layer gone from one side (never for a move) changes who knows what.
            let (siblings, k, outer) = siblings_of(d, *id)?;
            let b = photocraft_compose::change_bounds_in(siblings, k, outer, canvas)?;
            if !b.is_empty() {
                out = if out.is_empty() { b } else { out.union(&b) };
            }
        }
    }
    Some(out)
}

/// The sibling list holding layer `id` in `doc` (bottom→top), its index there, and the list
/// holding its parent group with the group's index (`None` at the top level).
type Siblings<'d> = (&'d [Layer], usize, Option<(&'d [Layer], usize)>);

fn siblings_of(doc: &Document, id: LayerId) -> Option<Siblings<'_>> {
    let path = doc.path_of(id)?;
    let (&k, parent) = path.split_last()?;
    let Some((&g, grand)) = parent.split_last() else { return Some((doc.layers.as_slice(), k, None)) };
    let outer = if grand.is_empty() { doc.layers.as_slice() } else { doc.layer_at(grand)?.children()? };
    Some((outer.get(g)?.children()?, k, Some((outer, g))))
}

/// Report [`layers_damage`] as the active document's last change, so the canvas recomposites
/// only that (grown by the effect reach around it) instead of everything.
pub(crate) fn note_damage(s: &mut Session, before: &Document, ids: &[LayerId]) {
    let Some(st) = s.active_mut() else { return };
    if before.size != st.doc.size {
        return;
    }
    st.last_damage = layers_damage(before, &st.doc, ids);
}

/// Whether two surfaces hold identical pixels: same format and default pixel, and the same
/// copy-on-write tile pointers at the same coordinates. Both documents stay alive during the
/// comparison, so tile addresses cannot be reused under it.
fn surfaces_equal(a: &photocraft_raster::Surface, b: &photocraft_raster::Surface) -> bool {
    a.format() == b.format()
        && a.default_pixel() == b.default_pixel()
        && a.tile_count() == b.tile_count()
        && a.tiles().zip(b.tiles()).all(|((ca, ta), (cb, tb))| ca == cb && std::sync::Arc::ptr_eq(ta, tb))
}

/// Byte blobs kept in `Arc`s (PSD raw blocks, embedded smart-object sources): pointer-identical
/// or content-equal (they are small compared to pixels, and undo steps rarely re-parse them).
fn blob_equal(x: &std::sync::Arc<Vec<u8>>, y: &std::sync::Arc<Vec<u8>>) -> bool {
    std::sync::Arc::ptr_eq(x, y) || (x.len() == y.len() && x.as_slice() == y.as_slice())
}

fn blobs_equal(a: &Option<std::sync::Arc<Vec<u8>>>, b: &Option<std::sync::Arc<Vec<u8>>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => blob_equal(x, y),
        _ => false,
    }
}

/// Layer masks (also the smart-filter mask): scalars plus tile-pointer surface identity.
fn masks_equal(a: &Option<photocraft_doc::LayerMask>, b: &Option<photocraft_doc::LayerMask>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            x.enabled == y.enabled && x.linked == y.linked && x.density == y.density && x.feather == y.feather && surfaces_equal(&x.surface, &y.surface)
        }
        _ => false,
    }
}

fn texts_equal(a: &photocraft_doc::TextLayer, b: &photocraft_doc::TextLayer) -> bool {
    a.text == b.text
        && a.font_family == b.font_family
        && a.size_pt == b.size_pt
        && a.color == b.color
        && a.transform == b.transform
        && a.runs == b.runs
        && a.paragraphs == b.paragraphs
        && a.shape == b.shape
        && a.orientation == b.orientation
        && a.antialias == b.antialias
        && a.warp == b.warp
        && match (&a.cache, &b.cache) {
            (None, None) => true,
            (Some(x), Some(y)) => surfaces_equal(x, y),
            _ => false,
        }
        && blobs_equal(&a.psd_raw, &b.psd_raw)
}

fn smarts_equal(a: &photocraft_doc::SmartObject, b: &photocraft_doc::SmartObject) -> bool {
    a.filters_enabled == b.filters_enabled
        && a.transform == b.transform
        && a.smart_filters == b.smart_filters
        && a.stack_mode == b.stack_mode
        && a.perspective == b.perspective
        && a.warp == b.warp
        && match (&a.source, &b.source) {
            (photocraft_doc::SmartSource::Embedded { file_name: x, bytes: xb }, photocraft_doc::SmartSource::Embedded { file_name: y, bytes: yb }) => {
                x == y && blob_equal(xb, yb)
            }
            (photocraft_doc::SmartSource::Linked { path: x }, photocraft_doc::SmartSource::Linked { path: y }) => x == y,
            _ => false,
        }
        && match (&a.cache, &b.cache) {
            (None, None) => true,
            (Some(x), Some(y)) => surfaces_equal(x, y),
            _ => false,
        }
        && blobs_equal(&a.psd_raw, &b.psd_raw)
        && masks_equal(&a.filter_mask, &b.filter_mask)
}

/// Extra alpha/spot channels (the image itself in Multichannel mode).
fn alpha_channels_equal(a: &photocraft_doc::AlphaChannel, b: &photocraft_doc::AlphaChannel) -> bool {
    a.name == b.name && a.spot == b.spot && a.color == b.color && a.opacity == b.opacity && a.indicates == b.indicates && surfaces_equal(&a.surface, &b.surface)
}

/// Whether two layers render to identical pixels in place. Everything the compositor reads is
/// compared structurally — no hashing, a hash collision would under-report damage; pixels are
/// identified by their copy-on-write tile pointers. Group children are deliberately *not*
/// compared here: the caller walks into them, so an edit inside a group damages only the
/// edited child's extent, not the whole group's. The destructure is exhaustive, so a new
/// `Layer` field fails to compile until it is classified: compared, or explicitly ignored
/// (name, locks, label, `psd_blocks`, `psd_id`, link group, XMP — none of them composite; video
/// bails out earlier in the walk).
fn layer_pixels_equal(a: &Layer, b: &Layer) -> bool {
    let Layer {
        id,
        name: _,
        visible,
        locks: _,
        blend,
        opacity,
        fill_opacity,
        clipped,
        mask,
        vector_mask,
        effects,
        label: _,
        content,
        psd_blocks: _,
        psd_id: _,
        fill_cache,
        link_group: _,
        excluded_channels,
        blend_if,
        advanced,
        video: _,
        xmp: _,
    } = a;
    *id == b.id
        && *visible == b.visible
        && *blend == b.blend
        && *opacity == b.opacity
        && *fill_opacity == b.fill_opacity
        && *clipped == b.clipped
        && masks_equal(mask, &b.mask)
        && *vector_mask == b.vector_mask
        && *effects == b.effects
        && *excluded_channels == b.excluded_channels
        && *blend_if == b.blend_if
        && *advanced == b.advanced
        && match (content, &b.content) {
            (LayerContent::Raster(x), LayerContent::Raster(y)) => surfaces_equal(x, y),
            // Children are walked separately; only the group's own clipping frame counts.
            (LayerContent::Group(x), LayerContent::Group(y)) => x.artboard == y.artboard,
            (LayerContent::Fill(x), LayerContent::Fill(y)) => x == y,
            (LayerContent::Adjustment(x), LayerContent::Adjustment(y)) => x == y,
            (LayerContent::Text(x), LayerContent::Text(y)) => texts_equal(x, y),
            (LayerContent::Shape(x), LayerContent::Shape(y)) => x == y,
            (LayerContent::Smart(x), LayerContent::Smart(y)) => smarts_equal(x, y),
            _ => false,
        }
        && match (fill_cache, &b.fill_cache) {
            (None, None) => true,
            (Some(x), Some(y)) => x.fill == y.fill && surfaces_equal(&x.surface, &y.surface),
            _ => false,
        }
}

/// Document-level fields the compositor reads: all must match for a bounded damage claim. The
/// destructure is exhaustive, so a new `Document` field fails to compile until it is classified
/// here — compared when it composites, or explicitly ignored (the pixel-irrelevant allowlist:
/// identity, name, selection, guides, paths, layer comps, annotations, automation state).
fn doc_pixels_equal(a: &Document, b: &Document) -> bool {
    let Document {
        id: _,
        name: _,
        size,
        resolution_dpi,
        mode,
        depth,
        icc_profile,
        layers: _,
        channels,
        guides: _,
        ruler_origin: _,
        selection: _,
        metadata: _,
        global_light,
        paths: _,
        work_path: _,
        clipping_path: _,
        quick_mask,
        quick_mask_unselected: _,
        patterns,
        color_table,
        duotone,
        layer_comps: _,
        last_applied_comp: _,
        last_document_state: _,
        measurement: _,
        notes: _,
        text_styles: _,
        slices: _,
        variables: _,
        timeline: _,
        file_encoding: _,
    } = a;
    *size == b.size
        && resolution_dpi.to_bits() == b.resolution_dpi.to_bits()
        && *mode == b.mode
        && *depth == b.depth
        && icc_profile.as_deref() == b.icc_profile.as_deref()
        && channels.len() == b.channels.len()
        && channels.iter().zip(&b.channels).all(|(x, y)| alpha_channels_equal(x, y))
        && *global_light == b.global_light
        && match (quick_mask, &b.quick_mask) {
            (None, None) => true,
            (Some(x), Some(y)) => alpha_channels_equal(x, y),
            _ => false,
        }
        && *patterns == b.patterns
        && *color_table == b.color_table
        && *duotone == b.duotone
}

/// Whether `a` and `b` composite to the same pixels: they differ at most in fields the
/// compositor never reads (selection, guides, paths, names, …), by the same classification as
/// [`step_damage`] but without computing any bounds. Cheap: pixels compare by tile pointer.
/// Conservative (`false`) past a nesting depth no real document reaches.
pub fn same_pixels(a: &Document, b: &Document) -> bool {
    pixels_equal_with(a, b, |_, _| false)
}

/// [`same_pixels`], with video layers equal when `video_equal` says their frame stacks are.
fn pixels_equal_with(a: &Document, b: &Document, video_equal: fn(&VideoData, &VideoData) -> bool) -> bool {
    fn walk(xs: &[Layer], ys: &[Layer], depth: usize, video_equal: fn(&VideoData, &VideoData) -> bool) -> bool {
        depth < 256
            && xs.len() == ys.len()
            && xs.iter().zip(ys).all(|(x, y)| {
                layer_pixels_equal(x, y)
                    && match (&x.video, &y.video) {
                        (None, None) => true,
                        (Some(vx), Some(vy)) => video_equal(vx, vy),
                        _ => false,
                    }
                    && match (x.children(), y.children()) {
                        (Some(xc), Some(yc)) => walk(xc, yc, depth + 1, video_equal),
                        (None, None) => true,
                        _ => false,
                    }
            })
    }
    doc_pixels_equal(a, b) && walk(&a.layers, &b.layers, 0, video_equal)
}

/// Whether two video layers hold the same frames: the same settings and stack shape
/// (`VideoData`'s own, shallow equality) and the same tile pointers in every frame, painted and
/// original. Pointer compares only, like [`surfaces_equal`].
fn video_frames_equal(a: &VideoData, b: &VideoData) -> bool {
    let frames = |x: &[photocraft_raster::Surface], y: &[photocraft_raster::Surface]| x.len() == y.len() && x.iter().zip(y).all(|(f, g)| surfaces_equal(f, g));
    *a == *b
        && frames(&a.frames, &b.frames)
        && match (&a.original_frames, &b.original_frames) {
            (Some(x), Some(y)) => frames(x, y),
            (None, None) => true,
            _ => false,
        }
}

/// Whether `a` and `b` differ at most in their selection (#6865): a selection step, which must
/// not mark a clean document modified. Pixels compare by tile pointer (as in [`same_pixels`],
/// plus every frame of a video layer, whose own equality is shallow); every other field compares
/// structurally, where copy-on-write `Arc`s of `Eq` data short-circuit on pointer identity. The
/// destructure is exhaustive, so a new `Document` field fails to compile until it is classified.
pub fn selection_only_change(a: &Document, b: &Document) -> bool {
    let Document {
        id,
        name,
        size: _,
        resolution_dpi: _,
        mode: _,
        depth: _,
        icc_profile: _,
        layers,
        channels: _,
        guides,
        ruler_origin,
        selection: _,
        metadata,
        global_light: _,
        paths,
        work_path,
        clipping_path,
        quick_mask: _,
        quick_mask_unselected,
        patterns: _,
        color_table: _,
        duotone: _,
        layer_comps,
        last_applied_comp,
        last_document_state,
        measurement,
        notes,
        text_styles,
        slices,
        variables,
        timeline,
        file_encoding,
    } = a;
    // The ignored fields above are the pixel ones `pixels_equal_with` already compares.
    pixels_equal_with(a, b, video_frames_equal)
        && *id == b.id
        && *name == b.name
        && *layers == b.layers
        && *guides == b.guides
        && *ruler_origin == b.ruler_origin
        && *metadata == b.metadata
        && *paths == b.paths
        && *work_path == b.work_path
        && *clipping_path == b.clipping_path
        && *quick_mask_unselected == b.quick_mask_unselected
        && *layer_comps == b.layer_comps
        && *last_applied_comp == b.last_applied_comp
        && *last_document_state == b.last_document_state
        && *measurement == b.measurement
        && *notes == b.notes
        && *text_styles == b.text_styles
        && *slices == b.slices
        && *variables == b.variables
        && *timeline == b.timeline
        && *file_encoding == b.file_encoding
}

/// Pixels a single history step can have changed between `a` and `b` (the documents on either
/// side of an edit, its undo or its redo): the union of the change bounds of every layer whose
/// pixels differ, so a local edit recomposites only that area instead of the whole canvas. A
/// layer only one side has (a duplicate, a paste, a deleted layer) counts where it draws (see
/// [`one_sided_bounds`]); one that draws nothing ([`inert_at`]) changes nothing. `None`
/// (composite everything) when the step touched anything the layer walk cannot bound:
/// document-level fields the compositor reads (canvas size, mode, depth, profile, global light,
/// patterns, channels, palettes), layer order and kind changes, layers added or removed that
/// reach outside their own bounds, layer properties that do (clipping, excluded channels —
/// what `layer.setProps` already reports as unbounded), and video layers. Inserting a copy of a
/// 12 MP layer used to recomposite every layer of the document (#2883, P8/P9 in
/// `perf/budgets.toml`).
pub fn step_damage(a: &Document, b: &Document) -> Option<Rect> {
    if !doc_pixels_equal(a, b) {
        return None;
    }
    /// The list holding a sibling list's parent group, and the group's index there.
    type Outer<'d> = Option<(&'d [Layer], usize)>;
    fn walk(xs: &[Layer], ys: &[Layer], (ox, oy): (Outer, Outer), a: &Document, b: &Document, out: &mut Rect) -> bool {
        // Pair the layers by id, stepping over layers only one side has: one that draws nothing
        // changes no pixels (a new empty layer or group, #1771), any other only where it draws.
        let one_sided = |layers: &[Layer], k: usize, outer: Outer, canvas: Rect, out: &mut Rect| {
            let Some(r) = one_sided_bounds(layers, k, outer, canvas) else { return false };
            if !r.is_empty() {
                *out = if out.is_empty() { r } else { out.union(&r) };
            }
            true
        };
        let (mut i, mut j) = (0, 0);
        loop {
            let (x, y) = match (xs.get(i), ys.get(j)) {
                (None, None) => return true,
                (Some(x), Some(y)) if x.id == y.id => (x, y),
                _ if inert_at(ys, j) => {
                    j += 1;
                    continue;
                }
                _ if inert_at(xs, i) => {
                    i += 1;
                    continue;
                }
                (_, Some(y)) if xs.iter().all(|x| x.id != y.id) => {
                    if !one_sided(ys, j, oy, b.bounds(), out) {
                        return false;
                    }
                    j += 1;
                    continue;
                }
                (Some(x), _) if ys.iter().all(|y| y.id != x.id) => {
                    if !one_sided(xs, i, ox, a.bounds(), out) {
                        return false;
                    }
                    i += 1;
                    continue;
                }
                // Both sides have the layer, in another order.
                _ => return false,
            };
            let (xk, yk) = (i, j);
            i += 1;
            j += 1;
            if x.video.is_some() || y.video.is_some() {
                return false;
            }
            // Clipping and channel exclusion change how *other* layers render, outside this
            // layer's own bounds; they have no bounded damage.
            if x.clipped != y.clipped || x.excluded_channels != y.excluded_channels {
                return false;
            }
            if !layer_pixels_equal(x, y) {
                // Both sides' extents: a move changes pixels where it left and where it landed.
                // A clipped adjustment reaches as far as its base (#6535).
                let (Some(bx), Some(by)) =
                    (photocraft_compose::change_bounds_in(xs, xk, ox, a.bounds()), photocraft_compose::change_bounds_in(ys, yk, oy, b.bounds()))
                else {
                    return false;
                };
                *out = if out.is_empty() { bx.union(&by) } else { out.union(&bx).union(&by) };
            }
            match (x.children(), y.children()) {
                (Some(xc), Some(yc)) => {
                    if !walk(xc, yc, (Some((xs, xk)), Some((ys, yk))), a, b, out) {
                        return false;
                    }
                }
                (None, None) => {}
                _ => return false,
            }
        }
    }
    let mut out = Rect::EMPTY;
    if walk(&a.layers, &b.layers, (None, None), a, b, &mut out) { Some(out) } else { None }
}

/// The pixels adding or removing `layers[k]` can change, when that is bounded: its change bounds
/// (`photocraft_compose::change_bounds_in`: not for an unclipped adjustment, or a pass-through
/// group holding one; a clipped adjustment's are its base's), provided it isn't the clipping base
/// of clipped layers above it (they would change base), and neither it nor anything inside it is
/// a video layer or excludes channels (both reach outside the layer's bounds, see
/// [`step_damage`]).
fn one_sided_bounds(layers: &[Layer], k: usize, outer: Option<(&[Layer], usize)>, canvas: Rect) -> Option<Rect> {
    fn contained(l: &Layer, depth: usize) -> bool {
        depth < 256 && l.video.is_none() && l.excluded_channels == 0 && l.children().is_none_or(|c| c.iter().all(|c| contained(c, depth + 1)))
    }
    let l = layers.get(k)?;
    let new_base = !l.clipped && layers.get(k + 1).is_some_and(|above| above.clipped);
    if new_base || !contained(l, 0) {
        return None;
    }
    photocraft_compose::change_bounds_in(layers, k, outer, canvas)
}

/// Whether `layers[k]` draws nothing wherever it sits, so adding or removing it leaves the
/// composite as it was: an empty pixel layer (no tiles, transparent default) or an empty group,
/// in a Normal or Pass Through blend with no mask, vector mask, effects or Advanced Blending.
/// A clipping base that would gain or lose the clipped layers above it is not inert.
fn inert_at(layers: &[Layer], k: usize) -> bool {
    let Some(l) = layers.get(k) else { return false };
    let empty = match &l.content {
        LayerContent::Raster(s) => s.tile_count() == 0 && s.format().alpha && s.default_bytes().iter().all(|b| *b == 0),
        LayerContent::Group(g) => g.children.is_empty() && g.artboard.is_none(),
        _ => false,
    };
    empty
        && matches!(l.blend, photocraft_doc::BlendMode::Normal | photocraft_doc::BlendMode::PassThrough)
        && l.mask.is_none()
        && l.vector_mask.is_none()
        && !photocraft_compose::effects::has_effects(l)
        && l.fill_cache.is_none()
        && l.video.is_none()
        && l.advanced == photocraft_doc::AdvancedBlending::default()
        && (l.clipped || layers.get(k + 1).is_none_or(|above| !above.clipped))
}

/// Report that adding layer `id` to the active document changed no pixels when it draws
/// nothing ([`inert_at`]), so the canvas doesn't recomposite every layer for it (#1771). Looks
/// only at the new layer and its neighbour, never at other layers' tiles.
pub(crate) fn note_inert_insert(s: &mut Session, id: LayerId) {
    let Some(st) = s.active_mut() else { return };
    let Some(path) = st.doc.path_of(id) else { return };
    let Some((&k, parent)) = path.split_last() else { return };
    let siblings = if parent.is_empty() { Some(st.doc.layers.as_slice()) } else { st.doc.layer_at(parent).and_then(Layer::children) };
    if siblings.is_some_and(|sib| inert_at(sib, k)) {
        st.last_damage = Some(Rect::EMPTY);
    }
}

/// Report the pixels a command made of more than one edit changed since `before` ([`step_damage`]),
/// so the canvas recomposites only where its new layers draw instead of the whole document.
/// A single [`Session::edit`] already reports its own damage: don't call this after one (#5695).
pub(crate) fn note_insert(s: &mut Session, before: &Document) {
    let Some(st) = s.active_mut() else { return };
    st.last_damage = step_damage(before, &st.doc);
}

/// Content bounds used by Align/Distribute: the layer's pixels (type and shape layers use their
/// rendered pixels), or the union of a group's children. Fill and adjustment layers have none.
pub fn layer_bounds(l: &Layer) -> Option<Rect> {
    let r = match &l.content {
        LayerContent::Group(g) => g.children.iter().filter_map(layer_bounds).fold(Rect::EMPTY, |a, b| a.union(&b)),
        _ => l.surface().map(|s| s.content_bounds()).unwrap_or(Rect::EMPTY),
    };
    (!r.is_empty()).then_some(r)
}
