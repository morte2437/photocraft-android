use photocraft_color::{PixelFormat, SampleType};

use super::{Document, Layer, LayerContent, Rect, Session, VideoData};

/// Calls `f` with the rect of every tile that differs between two surfaces of the same format and
/// default pixel: a tile only one side has, or a different copy-on-write tile pointer at the same
/// coordinate. A merge walk of the two sorted tile maps: pointer compares, no pixel reads. Both
/// documents stay alive during the comparison, so tile addresses cannot be reused under it.
/// `false` (and no calls) when the format or default pixel differs: every pixel may have changed.
fn each_changed_tile(a: &photocraft_raster::Surface, b: &photocraft_raster::Surface, mut f: impl FnMut(Rect)) -> bool {
    if a.format() != b.format() || a.default_bytes() != b.default_bytes() {
        return false;
    }
    let (mut xs, mut ys) = (a.tiles().peekable(), b.tiles().peekable());
    loop {
        match (xs.peek(), ys.peek()) {
            (None, None) => return true,
            (Some((ca, ta)), Some((cb, tb))) if ca == cb => {
                if !std::sync::Arc::ptr_eq(ta, tb) {
                    f(ca.rect());
                }
                xs.next();
                ys.next();
            }
            (Some((ca, _)), Some((cb, _))) if ca < cb => {
                f(ca.rect());
                xs.next();
            }
            (Some((ca, _)), None) => {
                f(ca.rect());
                xs.next();
            }
            (_, Some((cb, _))) => {
                f(cb.rect());
                ys.next();
            }
        }
    }
}

/// The pixels that differ between two surfaces, to tile precision: the bounding box of the tiles
/// [`each_changed_tile`] finds (empty when the pixels are identical), or `None` when the format
/// or default pixel differs.
fn surface_damage(a: &photocraft_raster::Surface, b: &photocraft_raster::Surface) -> Option<Rect> {
    let mut out = Rect::EMPTY;
    each_changed_tile(a, b, |r| out = out.union(&r)).then_some(out)
}

/// Whether two surfaces hold identical pixels (see [`surface_damage`]).
fn surfaces_equal(a: &photocraft_raster::Surface, b: &photocraft_raster::Surface) -> bool {
    surface_damage(a, b) == Some(Rect::EMPTY)
}

/// How [`layer_pixels_cmp`] compares a pair of surfaces a layer draws from, given the feather (in
/// pixels) the compositor blurs it by: `true` when the pair counts as equal for the caller.
type SurfaceCmp<'f> = &'f mut dyn FnMut(&photocraft_raster::Surface, &photocraft_raster::Surface, f32) -> bool;

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

/// Layer masks (also the smart-filter mask): scalars plus the surface, compared by `surf`.
fn masks_cmp(a: &Option<photocraft_doc::LayerMask>, b: &Option<photocraft_doc::LayerMask>, surf: SurfaceCmp) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            x.enabled == y.enabled && x.linked == y.linked && x.density == y.density && x.feather == y.feather && surf(&x.surface, &y.surface, x.feather)
        }
        _ => false,
    }
}

fn texts_cmp(a: &photocraft_doc::TextLayer, b: &photocraft_doc::TextLayer, surf: SurfaceCmp) -> bool {
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
        && blobs_equal(&a.psd_raw, &b.psd_raw)
        && match (&a.cache, &b.cache) {
            (None, None) => true,
            (Some(x), Some(y)) => surf(x, y, 0.0),
            _ => false,
        }
}

fn smarts_cmp(a: &photocraft_doc::SmartObject, b: &photocraft_doc::SmartObject, surf: SurfaceCmp) -> bool {
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
        && blobs_equal(&a.psd_raw, &b.psd_raw)
        // The filter mask shapes the filtered result the cache holds: compared exactly.
        && masks_cmp(&a.filter_mask, &b.filter_mask, &mut |x, y, _| surfaces_equal(x, y))
        && match (&a.cache, &b.cache) {
            (None, None) => true,
            (Some(x), Some(y)) => surf(x, y, 0.0),
            _ => false,
        }
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
    layer_pixels_cmp(a, b, &mut |x, y, _| surfaces_equal(x, y))
}

/// [`layer_pixels_equal`] with the surfaces the layer draws from (its pixels, mask, type and
/// smart-object caches, fill cache) compared by `surf`; everything else compares exactly, before
/// any surface, so `surf` sees a pair only when every scalar the compositor reads is equal.
fn layer_pixels_cmp(a: &Layer, b: &Layer, surf: SurfaceCmp) -> bool {
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
        frame,
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
        && *vector_mask == b.vector_mask
        && *frame == b.frame
        && *effects == b.effects
        && *excluded_channels == b.excluded_channels
        && *blend_if == b.blend_if
        && *advanced == b.advanced
        && match (fill_cache, &b.fill_cache) {
            (None, None) => true,
            (Some(x), Some(y)) => x.fill == y.fill,
            _ => false,
        }
        && match (content, &b.content) {
            (LayerContent::Raster(_), LayerContent::Raster(_)) => true,
            // Children are walked separately; only the group's own clipping frame counts.
            (LayerContent::Group(x), LayerContent::Group(y)) => x.artboard == y.artboard,
            (LayerContent::Fill(x), LayerContent::Fill(y)) => x == y,
            (LayerContent::Adjustment(x), LayerContent::Adjustment(y)) => x == y,
            (LayerContent::Text(x), LayerContent::Text(y)) => texts_cmp(x, y, &mut |_, _, _| true),
            (LayerContent::Shape(x), LayerContent::Shape(y)) => x == y,
            (LayerContent::Smart(x), LayerContent::Smart(y)) => smarts_cmp(x, y, &mut |_, _, _| true),
            _ => false,
        }
        // Every scalar is equal: now the surfaces.
        && masks_cmp(mask, &b.mask, surf)
        && match (content, &b.content) {
            (LayerContent::Raster(x), LayerContent::Raster(y)) => surf(x, y, 0.0),
            (LayerContent::Text(x), LayerContent::Text(y)) => texts_cmp(x, y, surf),
            (LayerContent::Smart(x), LayerContent::Smart(y)) => smarts_cmp(x, y, surf),
            _ => true,
        }
        && match (fill_cache, &b.fill_cache) {
            (Some(x), Some(y)) => surf(&x.surface, &y.surface, 0.0),
            _ => true,
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
///
/// A layer whose only change is in the pixels of its surfaces (raster content, mask, type or
/// smart-object cache, fill cache) damages only the tiles that changed ([`tile_damage`]), so
/// undoing a brush dab on a full-canvas layer redraws the dab's tiles, not the canvas (#9329).
pub fn step_damage(a: &Document, b: &Document) -> Option<Rect> {
    if !doc_pixels_equal(a, b) {
        return None;
    }
    /// The list holding a sibling list's parent group, and the group's index there.
    type Outer<'d> = Option<(&'d [Layer], usize)>;
    /// What the groups around a sibling list add to a change inside it: the summed reach of
    /// their effects, and whether any lays effects out over its frame.
    #[derive(Clone, Copy)]
    struct Around {
        reach: i32,
        spans_frame: bool,
    }
    fn walk(xs: &[Layer], ys: &[Layer], (ox, oy): (Outer, Outer), around: Around, a: &Document, b: &Document, out: &mut Rect) -> bool {
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
            let tiles = tile_damage(x, y);
            if tiles != Some(Rect::EMPTY) {
                // Both sides' extents: a move changes pixels where it left and where it landed.
                // A clipped adjustment reaches as far as its base (#6535).
                let (Some(bx), Some(by)) =
                    (photocraft_compose::change_bounds_in(xs, xk, ox, a.bounds()), photocraft_compose::change_bounds_in(ys, yk, oy, b.bounds()))
                else {
                    return false;
                };
                let whole = bx.union(&by);
                // Only pixels changed: the changed tiles, grown by what the groups around reach,
                // never more than the whole extents. Effects laid out over a frame repaint it
                // everywhere when it moves (#4053), so then the whole extents.
                let frame_moved = around.spans_frame
                    || (photocraft_compose::effects::spans_frame(y)
                        && photocraft_compose::effects_frame(x, a.bounds()) != photocraft_compose::effects_frame(y, b.bounds()));
                let d = match tiles {
                    Some(t) if !frame_moved => t.inflate(around.reach).intersect(&whole.inflate(around.reach)).intersect(&b.bounds()),
                    _ => whole,
                };
                *out = out.union(&d);
            }
            match (x.children(), y.children()) {
                (Some(xc), Some(yc)) => {
                    let reach = |l: &Layer| if photocraft_compose::effects::has_effects(l) { photocraft_compose::effects::margin(l) } else { 0 };
                    let inner = Around {
                        reach: around.reach.saturating_add(reach(x).max(reach(y))),
                        spans_frame: around.spans_frame || photocraft_compose::effects::spans_frame(x) || photocraft_compose::effects::spans_frame(y),
                    };
                    if !walk(xc, yc, (Some((xs, xk)), Some((ys, yk))), inner, a, b, out) {
                        return false;
                    }
                }
                (None, None) => {}
                _ => return false,
            }
        }
    }
    let mut out = Rect::EMPTY;
    if walk(&a.layers, &b.layers, (None, None), Around { reach: 0, spans_frame: false }, a, b, &mut out) { Some(out) } else { None }
}

/// How many tiles that differ between `before` and `after` (in the surfaces of layers both have)
/// `hint` misses: a command's own damage rect that misses a changed tile is too narrow (#4053).
#[cfg(any(test, debug_assertions))]
pub(crate) fn hint_misses(before: &Document, after: &Document, hint: Rect) -> usize {
    let mut misses = 0;
    for (_, _, y) in after.walk() {
        let Some(x) = before.layer(y.id) else { continue };
        layer_pixels_cmp(x, y, &mut |p, q, _| {
            each_changed_tile(p, q, |t| misses += usize::from(t.intersect(&hint).is_empty()));
            true
        });
    }
    misses
}

/// The pixels a change from layer `a` to layer `b` (the same layer on either side of a step) can
/// have changed when only the pixels of its surfaces changed: the tiles that differ
/// ([`surface_damage`]), a mask's grown by its feather, all grown by the layer's own effects.
/// Empty when the layers render identically. `None` when anything else changed (a property, the
/// content kind, a surface's format or default pixel): the caller then takes the layer's whole
/// extents. Children of groups are not compared (the caller walks them).
fn tile_damage(a: &Layer, b: &Layer) -> Option<Rect> {
    let mut tiles = Rect::EMPTY;
    let only_pixels = layer_pixels_cmp(a, b, &mut |x, y, feather| {
        let Some(r) = surface_damage(x, y) else { return false };
        let r = if feather > 0.0 { photocraft_vector::feather::grow(r, photocraft_vector::feather::feather_sigma(feather)) } else { r };
        tiles = tiles.union(&r);
        true
    });
    if !only_pixels {
        return None;
    }
    // Effects are equal on both sides here (they are compared before any surface).
    let margin = if photocraft_compose::effects::has_effects(b) { photocraft_compose::effects::margin(b) } else { 0 };
    Some(if tiles.is_empty() || margin == 0 { tiles } else { tiles.inflate(margin) })
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

/// Report the pixels a command made of more than one edit changed since `before` ([`step_damage`]),
/// so the canvas recomposites only where its new layers draw instead of the whole document.
/// A single [`Session::edit`] already reports its own damage: don't call this after one (#5695).
pub(crate) fn note_insert(s: &mut Session, before: &Document) {
    let Some(st) = s.active_mut() else { return };
    st.last_damage = step_damage(before, &st.doc);
}

/// Content bounds of a layer's pixels as stored (type and shape layers: their cached render), or
/// the union of a group's children. Fill and adjustment layers have none. Cheap: Layers list,
/// artboards, copy and slices call it for every layer.
pub fn layer_bounds(l: &Layer) -> Option<Rect> {
    let r = match &l.content {
        LayerContent::Group(g) => g.children.iter().filter_map(layer_bounds).fold(Rect::EMPTY, |a, b| a.union(&b)),
        _ => l.surface().map(|s| s.content_bounds()).unwrap_or(Rect::EMPTY),
    };
    (!r.is_empty()).then_some(r)
}

/// The bounds Align and Distribute move by, in a document of bounds `canvas`: [`layer_bounds`],
/// except that a shape layer is measured over everything it draws, beyond the canvas too (its
/// cache is clipped to the canvas, #8660), by rendering it again. Only those commands pay for that
/// render, once per click; `canvas` is the frame a shape's gradients are laid out in, so an opacity
/// stop measures as it renders, and `sample` is the document's sample type (see
/// [`shape_pixel_bounds`]).
pub(crate) fn align_bounds(l: &Layer, canvas: Rect, sample: SampleType) -> Option<Rect> {
    let r = match &l.content {
        LayerContent::Group(g) => g.children.iter().filter_map(|c| align_bounds(c, canvas, sample)).fold(Rect::EMPTY, |a, b| a.union(&b)),
        LayerContent::Shape(sh) => shape_pixel_bounds(sh, canvas, sample).unwrap_or_else(|| {
            let cached = l.surface().map(|s| s.content_bounds()).unwrap_or(Rect::EMPTY);
            // A fill that shows everywhere covers the canvas, rendered yet or not.
            if cached.is_empty() && unbounded_fill(sh) { canvas } else { cached }
        }),
        _ => return layer_bounds(l),
    };
    (!r.is_empty()).then_some(r)
}

/// Whether the shape's fill shows everywhere (an inverted path, or below 100 % density), so it has
/// no extent of its own.
fn unbounded_fill(sh: &photocraft_doc::ShapeLayer) -> bool {
    sh.fill.is_some() && (sh.path.inverted || (sh.density.is_finite() && sh.density < 1.0))
}

/// Largest area [`shape_pixel_bounds`] renders to find a shape's extent; beyond it the path and
/// stroke bounds stand in.
const SHAPE_BOUNDS_MAX_PX: u64 = 64 << 20;

/// Pixels [`shape_pixel_bounds`] renders at a time.
const SHAPE_BOUNDS_BAND_PX: u64 = 1 << 20;

/// The pixels a shape layer draws, wherever they fall. Its cache is rendered clipped to the
/// canvas, so a shape partly off the canvas measured from it looked smaller and Align moved it
/// by the wrong amount, a little more on each click (#8660). The shape is rendered again over
/// everything it can draw (its path, stroke and feather), unclipped, and measured from that. A fill
/// that shows everywhere (an inverted path, or below 100 % density) has no extent of its own and
/// keeps the canvas-clipped measure (`None`).
pub(super) fn shape_pixel_bounds(sh: &photocraft_doc::ShapeLayer, canvas: Rect, sample: SampleType) -> Option<Rect> {
    if unbounded_fill(sh) {
        return None;
    }
    let reach = photocraft_compose::shape_bounds(sh)?.inflate(1);
    if u64::from(reach.width()) * u64::from(reach.height()) > SHAPE_BOUNDS_MAX_PX {
        return photocraft_compose::hard_shape_bounds(sh);
    }
    // Gradients lay out in the document's frame (as the cached render does); the feather and the
    // clip cover the shape's whole reach, beyond the canvas too.
    let shape = photocraft_vector::CompiledShape::with_frames(sh, photocraft_vector::DEFAULT_TOLERANCE, reach, canvas);
    // Alpha at the document's own precision, as the document stores it: an 8-bit render drops the
    // faint stroke of a 16-bit or float document (the opaque body then aligns by a different
    // extent than the document draws), and a float one would find alpha an 8-bit document never
    // stores. A band of rows at a time keeps the working copy a few MB however large the shape.
    let format = PixelFormat::new(photocraft_color::ColorMode::Grayscale, sample, true);
    let band_rows = i32::try_from((SHAPE_BOUNDS_BAND_PX / u64::from(reach.width().max(1))).max(1)).unwrap_or(i32::MAX);
    let mut bounds = Rect::EMPTY;
    let mut top = reach.y0;
    while top < reach.y1 {
        let bottom = top.saturating_add(band_rows).min(reach.y1);
        bounds = bounds.union(&shape.render(format, Rect::new(reach.x0, top, reach.x1, bottom)).content_bounds());
        top = bottom;
    }
    (!bounds.is_empty()).then_some(bounds)
}
