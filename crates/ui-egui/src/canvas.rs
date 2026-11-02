//! Document canvas: display texture cache, view transform, tool input, extra document windows.
//!
//! Rendering is CPU (`photocraft-compose`) for now, uploaded into an egui texture. Brush strokes
//! update only their damage rectangle (`set_partial`). The wgpu compositor (M5) will replace this
//! with direct GPU rendering behind the same `CanvasCache` interface.

use egui::{Color32, Mesh, PointerButton, Pos2, Rect, Sense, Stroke, TextureOptions, Vec2, pos2, vec2};
use photocraft_doc::{Document, LayerContent};
use photocraft_geom::Rect as DRect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{Tool, View};

pub(super) mod ants;
pub mod artboard_ui;
pub(crate) mod background_redraw;
pub(crate) mod cam_preview;
pub mod canvas_autoscroll;
pub(super) mod canvas_popup;
pub mod canvas_tool_menu;
pub mod channel_view;
mod cursors;
mod display;
mod drag;
mod filter_preview;
mod gesture;
pub mod gpu_canvas;
pub mod guide_select;
pub mod layer_pick_ui;
pub mod move_ui;
mod navigator;
pub mod outline;
mod paint;
pub mod paint_mouse;
pub mod patch_preview;
pub mod pattern_preview;
pub mod pixel_grid;
pub mod quick_pick;
pub mod rotate_view;
pub mod rulers;
pub mod scrollbars;
pub mod select_mask_view;
mod selection;
pub mod snap_ui;
mod start_screen;
mod strokes;
mod tabs;
mod textures;
pub(super) mod tile_overlay;
pub(super) mod tool_cursor;
pub mod tool_feedback;
mod tool_input;
mod transform_controls;
mod view_xform;
pub mod wheel_nav;
pub mod zoom_field;
pub mod zoom_hold;
pub mod zoom_levels;
pub mod zoom_window_fit;

#[cfg(test)]
pub(crate) use cursors::eyedropper_ring_colors;
pub(crate) use cursors::{alt_picks, alt_samples, composite_color, paint_bucket_cursor, pen_cursor, pencil_cursor_rect, pencil_tip_is_round, pipette_cursor};
use cursors::{brush_tip_centre, eyedropper_ring, precise_other_cursor, visible_crosshair};
#[cfg(test)]
pub(crate) use display::GPU_OUTPUT;
#[cfg(test)]
use display::texture_key;
pub(crate) use display::{cache_key, canvas_display, display_doc, display_image};
use display::{checker, hdr_preview, sync_display_lut};
pub use drag::{Drag, fixed_size_px, marquee_corners, marquee_px, marquee_readout, readout_in, size_readout, square_px};
pub(crate) use drag::{angle_readout, draw_readout, draw_value_readout};
#[cfg(test)]
pub(crate) use drag::{marquee_drag_ants, marquee_preview_px};
#[cfg(test)]
pub(crate) use filter_preview::committed_filter_preview;
use filter_preview::{ensure_adjust_proxy, ensure_proxy_preview, filter_preview_display_doc};
pub(crate) use filter_preview::{ensure_filter_preview, filter_channel_preview, filter_preview_factor};
#[cfg(test)]
pub(crate) use gesture::constrained_polygon_point;
pub use gesture::{commit_crop, commit_polygon, polygon_cancel_stale, polygon_replaces_selection, polygon_retract};
pub(crate) use gesture::{finish_gesture, selection_mode};
pub(crate) use navigator::NavigatorCache;
#[cfg(test)]
use navigator::navigator_waits_for_drag;
pub use navigator::{NAVIGATOR_SIDE, navigator_gpu_view, navigator_texture, navigator_waits_for_gpu};
#[cfg(test)]
use paint::draw_tool_state;
pub(crate) use paint::{clip_line, marching_ants_line_shapes, marching_ants_mapped};
use paint::{
    draw_beyond_canvas, draw_drag_preview, marching_ants_segment_shapes, paint_cpu_detail, paint_cpu_document, paint_gpu_view, paint_mapped_image_at,
    pattern_xf,
};
use selection::selection_shown_offset;
pub use selection::{SelCursor, selection_cursor};
pub(crate) use selection::{floating_outline, selection_drag_delta};
#[cfg(test)]
use start_screen::start_screen_drop_hint;
pub(crate) use strokes::multiple_color_target;
pub use strokes::paint_target;
pub(crate) use strokes::{ClonePreviewCache, CommittedStroke, LiveStroke, cancel_mixer_on_context_change, freehand_tool, spread_ms, strokes_live};
use strokes::{HOLD_REPEAT_MS, draw_clone_preview, feed_live_stroke, pointer_button_modifiers, pointer_moves, stroke_wants_time, tap_at_peak_pressure};
#[cfg(test)]
pub(crate) use strokes::{SPOT_HEALING_TRAIL, live_retouch_points};
pub(crate) use tabs::arranged_cells;
pub(crate) use tabs::tab_title_at;
#[cfg(test)]
use tabs::{TEAR_OFF, pro_tab_title, tab_context_items, tab_drop_index, tab_move_target, tabs, torn_off};
pub use tabs::{TabStrip, document_area, extra_windows};
use tabs::{dots, paint_dots};
#[cfg(test)]
use tabs::{main_canvas_document, move_target_after_filter};
#[cfg(test)]
pub(crate) use textures::detail_plan;
pub(crate) use textures::{DetailPlan, damage_since, effect_reach, ensure_texture_in_view, retain_gpu_documents, shown_as_document};
pub use textures::{MAX_TEXTURE, ensure_texture};
#[cfg(test)]
use textures::{clamp_texture_side, preview_bounds, texture_factor};
use textures::{ensure_gpu, redraw_focus, visible_doc_rect};
pub use tool_input::{ToolEvent, selection_drag_kind, tool_event};
pub(crate) use tool_input::{command_guide_at, command_moves_layer, event_tool};
pub(crate) use transform_controls::{LAYER_EDGES, layer_edges_rect};
use transform_controls::{begin_transform_controls_at, draw_transform_controls, transform_controls_cursor, transform_controls_rect, transform_controls_turn};
#[cfg(test)]
use transform_controls::{transform_controls_hit, transform_controls_layers};
pub use view_xform::{ViewXform, doc_point_at, fill_view, fit_view};
use view_xform::{animated_view, wheel_anchor, zoom_about};

pub struct CanvasCache {
    pub revision: u64,
    pub texture: Option<egui::TextureHandle>,
    /// Display pixels per document pixel of the cached image (≤ 1).
    pub scale: f32,
    /// Hash of the live-adjust preview used for this render (0 = none).
    pub preview_key: u64,
    /// The current image lives in the GPU canvas (`gpu_canvas`) rather than `texture`.
    pub on_gpu: bool,
    /// Revision and preview key `texture` was rendered at. Tracked apart from the GPU canvas's
    /// `revision`, so the Navigator never shows a stale CPU image after GPU-path edits.
    pub tex_revision: u64,
    pub tex_preview_key: u64,
    /// All uploaded texels are opaque. Screen coverage is checked separately before omitting
    /// the checker. Updated with uploads, never by scanning during an unchanged frame.
    pub texture_opaque: bool,
    /// The CPU path's pixel grid for the part of the document in view (pixel_grid.rs).
    pub grid: Option<crate::pixel_grid::GridLines>,
    /// The CPU path's sharper textures of the part in view, over a reduced `texture` (#7568),
    /// by `InView::key` (two windows of a document can show different parts).
    pub detail: Vec<(u64, DetailTexture)>,
}

/// [`CanvasCache::detail`]: the part of the document `DetailPlan` chose, uploaded at its factor.
pub struct DetailTexture {
    texture: egui::TextureHandle,
    plan: DetailPlan,
    /// The texels it holds, in the plan's reduced pixels (`render_reduced_damage`'s output rect).
    out: DRect,
    /// Revision and preview key (display key folded in) it was rendered at.
    revision: u64,
    preview_key: u64,
    /// Pixels were composited for it this call (for the perf span).
    rendered: bool,
}

/// The CPU canvas textures to draw: the whole document, and over it the sharper part in view.
pub(crate) struct CpuTextures {
    pub texture: egui::TextureId,
    pub scale: f32,
    /// The document area the texture covers: the document's bounds, or for a filter preview that
    /// changes the canvas's dimensions (Image › Rotation) its result centred on the document, as
    /// the GPU canvas draws it.
    pub bounds: DRect,
    /// The detail texture and the document area it covers (x0, y0, x1, y1 in document pixels).
    pub detail: Option<(egui::TextureId, [f32; 4])>,
}

/// The part of a CPU canvas view in sight: its document area and zoom (physical pixels per
/// document pixel), for [`ensure_texture_in_view`]'s detail texture.
#[derive(Clone, Copy, Debug)]
pub(crate) struct InView {
    pub visible: DRect,
    pub zoom: f32,
    /// Which view this is, so each keeps its own detail texture.
    pub key: u64,
}

/// Photoshop-style zoom label: "33.3", "100", "12.5".
pub fn fmt_zoom(pct: f32) -> String {
    if (pct - pct.round()).abs() < 0.05 { format!("{}", pct.round() as i64) } else { format!("{pct:.1}") }
}

pub fn mode_label(doc: &Document) -> &'static str {
    match doc.mode {
        photocraft_doc::ColorMode::Rgb => "RGB",
        photocraft_doc::ColorMode::Grayscale => "Gray",
        photocraft_doc::ColorMode::Cmyk => "CMYK",
        photocraft_doc::ColorMode::Lab => "Lab",
        photocraft_doc::ColorMode::Indexed => "Indexed",
        photocraft_doc::ColorMode::Bitmap => "Bitmap",
        photocraft_doc::ColorMode::Duotone => "Duotone",
        photocraft_doc::ColorMode::Multichannel => "Multichannel",
    }
}

/// Draw one canvas view and handle its input. `primary` = main window (tools active).
struct CanvasPaintInput<'a> {
    ctx: &'a egui::Context,
    rect: Rect,
    painter: &'a egui::Painter,
    ppp: f32,
    border: photocraft_engine::prefs::CanvasBorder,
    doc: &'a std::sync::Arc<Document>,
    idx: usize,
    output: Option<u32>,
    size: [u32; 2],
    primary: bool,
    flip: bool,
    aspect: f32,
    render_zoom: f32,
    target_xf: ViewXform,
    pixel_grid: bool,
}

struct SelectionPaintInput<'a> {
    ctx: &'a egui::Context,
    painter: &'a egui::Painter,
    ui: &'a egui::Ui,
    view: &'a View,
    xf: ViewXform,
    rect: Rect,
    doc: &'a Document,
    idx: usize,
}

/// The marching ants of the selection, traced from this frame's snapshot like the image under
/// them, returned to be painted over the grid and guides (#10671): a guide on the same device
/// pixel as an edge used to hide it, at some zoom levels and not others.
fn paint_selection_outline(app: &mut PhotocraftApp, input: SelectionPaintInput<'_>) -> Vec<egui::Shape> {
    let mut ants = Vec::new();
    // Select and Mask outlines its refined selection (on its preview proxy, reduced by `k`) in
    // the Marching Ants view and hides the ants in the others.
    let refined = crate::select_mask_view::ants(app, input.idx);
    // A floating piece's ants hug the pixels being moved.
    let floated = floating_outline(app, input.idx);
    let (outlined, k) = match &refined {
        None => (floated.as_ref().or(input.doc.selection.as_ref()), 1),
        Some(None) => (None, 1),
        Some(Some((proxy, k))) => (proxy.selection.as_ref(), (*k).max(1)),
    };
    // Selection outline: true boundary, animated marching ants (cached per revision).
    if let Some(sel) = outlined.filter(|_| app.ui.view.shows(app.ui.view.show.selection_edges) && !polygon_replaces_selection(app)) {
        // Trace at display resolution over the visible part only; key by the mask's tile identity
        // (not the document revision) so unrelated edits don't re-trace it. The step is in
        // device pixels per document pixel (`View::zoom`, not the point zoom), so a 1 px
        // selection still traces at 100% on a scaled display.
        let step = (1.0 / (input.view.zoom * k as f32).max(1e-3)).log2().floor().exp2().clamp(1.0, 64.0) as u32;
        let corners = [input.rect.min, pos2(input.rect.max.x, input.rect.min.y), input.rect.max, pos2(input.rect.min.x, input.rect.max.y)];
        let docs = [input.xf.to_doc(corners[0]), input.xf.to_doc(corners[1]), input.xf.to_doc(corners[2]), input.xf.to_doc(corners[3])];
        let mut x0 = docs[0][0];
        let mut y0 = docs[0][1];
        let mut x1 = x0;
        let mut y1 = y0;
        for d in docs {
            x0 = x0.min(d[0]);
            y0 = y0.min(d[1]);
            x1 = x1.max(d[0]);
            y1 = y1.max(d[1]);
        }
        let (x0, y0, x1, y1) = (x0 / f64::from(k), y0 / f64::from(k), x1 / f64::from(k), y1 / f64::from(k));
        let q = 256 * step as i32; // quantise the region so small pans reuse the cache
        // The view centre can be any finite number (`ui.set`, #980). Past ±2³⁰ there is no document
        // to outline, and the padding below (at most 2 · 256 · 64) can't overflow i32 from there.
        let px = |v: f64| v.clamp(-f64::from(1 << 30), f64::from(1 << 30)) as i32;
        // A Free Transform carries the selection to wherever its box goes (#5297), so all of
        // it is outlined: the part now in view may have started out of it.
        let carried = app.session.active_index() == Some(input.idx) && crate::transform_tool::carries_selection_of(app, input.doc.id);
        let vis = if carried {
            photocraft_geom::Rect::new(-(1 << 30), -(1 << 30), 1 << 30, 1 << 30)
        } else {
            photocraft_geom::Rect::new(
                px(x0.floor()).div_euclid(q) * q - q,
                px(y0.floor()).div_euclid(q) * q - q,
                (px(x1.ceil()).div_euclid(q) + 2) * q,
                (px(y1.ceil()).div_euclid(q) + 2) * q,
            )
        };
        let bounds_key = if k > 1 { u64::MAX / 2 - input.doc.id.0 } else { u64::MAX - input.doc.id.0 };
        let carried_bounds = carried.then(|| app.cached_bounds(bounds_key, sel).intersect(&vis));
        let step = carried_bounds.map_or(step, |b| crate::outline::bounded_step(b, step));
        let key = crate::surface_fingerprint(sel)
            ^ u64::from(step).rotate_left(56)
            ^ input.doc.id.0.rotate_left(17)
            ^ (vis.x0 as u64) << 8
            ^ (vis.y0 as u64) << 24
            ^ (vis.x1 as u64) << 36
            ^ (vis.y1 as u64) << 48
            ^ u64::from(k).rotate_left(5);
        let fresh = matches!(&app.outline_cache, Some((d, k, _)) if *d == input.doc.id && *k == key);
        if !fresh {
            let t0 = crate::gpu_canvas::now_ms();
            let b = carried_bounds.unwrap_or_else(|| app.cached_bounds(bounds_key, sel).intersect(&vis));
            // Filters can occupy every worker in Rayon's global pool. A parallel scan here would
            // block the UI thread waiting for those workers, preventing job cancellation (#1554).
            let mut segs =
                if app.session.has_jobs() { crate::outline::outline_scaled_serial(sel, b, step) } else { crate::outline::outline_scaled(sel, b, step) };
            if k > 1 {
                let s = i32::try_from(k).unwrap_or(1);
                let up = |p: [i32; 2]| [p[0].saturating_mul(s), p[1].saturating_mul(s)];
                segs.iter_mut().for_each(|(a, b)| (*a, *b) = (up(*a), up(*b)));
            }
            app.perf.span("outline", crate::gpu_canvas::now_ms() - t0);
            app.outline_cache = Some((input.doc.id, key, std::sync::Arc::new(segs)));
        }
        // Under Content-Aware Move's Transform On Drop box the ants are drawn where it puts them.
        if let Some((_, _, segs)) = app.outline_cache.as_ref().filter(|_| !crate::cam_transform::active(app)) {
            let time = input.ui.input(|i| i.time);
            // A floating piece (and a selection being dragged) draws its ants where it is shown.
            let mut shown = input.xf;
            if let Some((dx, dy)) = selection_shown_offset(app) {
                shown.center = [input.xf.center[0] - dx as f32, input.xf.center[1] - dy as f32];
            }
            // Under a Free Transform box they are where the box puts them (#5297).
            let boxed = carried.then(|| crate::transform_tool::selection_ants(app, input.painter, &input.xf, input.doc.id, segs, time)).flatten();
            ants = boxed.unwrap_or_else(|| marching_ants_segment_shapes(input.painter, &shown, segs, time));
            input.ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
    ants
}

/// Paints the document, and returns whether the pixels past the canvas show, the Layer Edges slot,
/// the canvas edge on screen, and the selection's ants still to be painted ([`paint_selection_outline`]).
fn paint_canvas(
    app: &mut PhotocraftApp,
    ui: &egui::Ui,
    view: &View,
    xf: ViewXform,
    input: &CanvasPaintInput<'_>,
) -> (bool, egui::layers::ShapeIdx, Rect, Vec<egui::Shape>) {
    let ctx = input.ctx;
    let rect = input.rect;
    let painter = input.painter;
    let ppp = input.ppp;
    let border = input.border;
    let doc = input.doc;
    let idx = input.idx;
    let output = input.output;
    let size = input.size;
    let primary = input.primary;
    let flip = input.flip;
    let aspect = input.aspect;
    let render_zoom = input.render_zoom;
    let target_xf = input.target_xf;
    let pixel_grid = input.pixel_grid;

    let point_zoom = view.zoom / ppp;
    let drop_shadow = border == photocraft_engine::prefs::CanvasBorder::DropShadow;
    // Drop shadow, checkerboard, document image.
    let img_rect = xf.doc_rect(doc.bounds());
    // The canvas edge the Line border outlines: a size-changing preview's own (CPU canvas).
    let mut edge_rect = img_rect;
    // The document area the image is drawn over: a size-changing preview's own on the CPU
    // canvas (Image › Rotation). Overlays drawn from the same preview follow it.
    let mut shown_area = doc.bounds();
    // Crop tool: the layers' pixels past the canvas while a frame is edited, under the canvas
    // (whose edge blends into them) and without its shadow.
    let beyond = draw_beyond_canvas(app, painter, &xf, idx, output);
    // View › Pattern Preview (#1067): copies of the document around it, from the same texture.
    let pattern = if app.ui.view.pattern_preview {
        let v = visible_doc_rect(&xf);
        crate::pattern_preview::copies([v.x0, v.y0, v.x1, v.y1].map(f64::from), size)
    } else {
        Vec::new()
    };
    let drop_shadow = drop_shadow && !beyond && !app.ui.view.pattern_preview;
    // Live adjustment previews on big documents use a downsampled proxy (see proxy.rs), unless
    // Preferences › Performance › Low Resolution Previews is off.
    let mut on_gpu = false;
    // A flipped or pixel-aspect-corrected view draws through the CPU path (the GPU canvas shader
    // has no mirroring or non-square pixels). Rotation stays on the GPU: it is a view uniform.
    let gpu_ok = crate::gpu_canvas::gpu_view_ok(flip, view.rotation, aspect);
    if app.gpu.is_some() {
        if gpu_ok {
            crate::render_path::view_ok(app, doc.id.0);
        } else {
            crate::render_path::view_cpu(app, doc.id.0);
        }
    }
    if app.gpu.is_some()
        && gpu_ok
        && let Some((k, key, preview_size)) =
            ensure_adjust_proxy(app, idx, render_zoom).or_else(|| ensure_filter_preview(app, idx, ctx)).or_else(|| ensure_proxy_preview(app, idx, render_zoom))
    {
        on_gpu = true;
        app.navigator_gpu.remove(&doc.id);
        let original_size = [doc.size.width.div_ceil(k), doc.size.height.div_ceil(k)];
        let shown = filter_preview_display_doc(app, doc, key);
        let params = crate::gpu_canvas::ViewParams {
            doc: key,
            doc_size: preview_size,
            zoom: view.zoom * k as f32 / ppp,
            // A size-changing preview grows around the old image center; keep the view's pan.
            center: [
                xf.center[0] / k as f32 + (preview_size[0] as f32 - original_size[0] as f32) / 2.0,
                xf.center[1] / k as f32 + (preview_size[1] as f32 - original_size[1] as f32) / 2.0,
            ],
            rotation: if view.rotation.is_finite() { view.rotation.to_radians() } else { 0.0 },
            shadow: {
                let t = crate::theme::Tokens::get(ctx);
                !t.bevel && !t.pro && drop_shadow
            },
            pixel_grid: false,
            view_key: egui::Id::unique(("pc-canvas-proxy", ctx.viewport_id(), idx)).value(),
            display: sync_display_lut(app, &shown, key, output),
            output: output.unwrap_or(0),
            hdr: hdr_preview(app, doc),
            offset: [0.0, 0.0],
        };
        paint_gpu_view(painter, rect, params, &pattern);
    } else if gpu_ok && ensure_gpu(app, ctx, idx, redraw_focus(&target_xf, &xf)) {
        on_gpu = true;
        app.perf.gpu = true;
        // Shadow, checkerboard, document and pixel grid in one custom shader (gpu_canvas.rs).
        let params = crate::gpu_canvas::ViewParams {
            doc: doc.id.0,
            doc_size: [doc.size.width, doc.size.height],
            zoom: point_zoom,
            center: xf.center,
            rotation: if view.rotation.is_finite() { view.rotation.to_radians() } else { 0.0 },
            shadow: {
                let t = crate::theme::Tokens::get(ctx);
                !t.bevel && !t.pro && drop_shadow
            },
            pixel_grid,
            view_key: egui::Id::unique(("pc-canvas", ctx.viewport_id(), idx)).value(),
            display: sync_display_lut(app, doc, doc.id.0, output),
            output: output.unwrap_or(0),
            hdr: hdr_preview(app, doc),
            offset: [0.0, 0.0],
        };
        if app.navigator_plain.contains(&doc.id) {
            app.navigator_gpu.insert(doc.id, params);
        } else {
            app.navigator_gpu.remove(&doc.id);
        }
        paint_gpu_view(painter, rect, params, &pattern);
    } else {
        app.navigator_gpu.remove(&doc.id);
        // A document too large for one full-resolution texture also gets the part in view at
        // the screen's resolution, drawn over the reduced whole (#7568).
        let in_view =
            InView { visible: visible_doc_rect(&xf), zoom: view.zoom, key: egui::Id::unique(("pc-canvas-detail", ctx.viewport_id(), idx, primary)).value() };
        let textures = ensure_texture_in_view(app, ctx, idx, output, Some(in_view));
        let tex = textures.as_ref().map(|t| t.texture);
        // The area the image covers: a size-changing preview's own (Image › Rotation).
        let area = textures.as_ref().map_or(doc.bounds(), |t| t.bounds);
        shown_area = area;
        if area != doc.bounds() {
            edge_rect = xf.doc_rect(area);
        }
        if !crate::theme::Tokens::get(ctx).bevel && drop_shadow {
            let shadow_rect = edge_rect;
            painter.add(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(150) }.as_shape(shadow_rect, 0));
        }
        let rotated = view.rotation.abs() > 0.05;
        let checkerboard = app.session.prefs().transparency_and_gamut.square().map(|square| (checker(app, ctx), square));
        // Repeated-quad coverage is not proven identical on all Pattern Preview adapters.
        // Retain its legacy checker submissions, including the central document's.
        let opaque = !app.ui.view.pattern_preview && app.canvases.get(&cache_key(doc.id, output)).is_some_and(|c| c.texture_opaque) && tex.is_some();
        paint_cpu_document(painter, &xf, area, rotated, checkerboard, tex, opaque);
        if let Some((detail, area)) = textures.and_then(|t| t.detail) {
            paint_cpu_detail(painter, &xf, doc.bounds(), rotated, checkerboard, detail, area);
        }
        // Pattern Preview tiles what is shown: a size-changing preview's area, from its origin.
        let (tile, copies) = if area == doc.bounds() || !app.ui.view.pattern_preview {
            (size, pattern.clone())
        } else {
            let tile = [area.width(), area.height()];
            let v = visible_doc_rect(&xf).translate(-area.x0, -area.y0);
            (tile, crate::pattern_preview::copies([v.x0, v.y0, v.x1, v.y1].map(f64::from), tile))
        };
        for &c in &copies {
            paint_cpu_document(painter, &pattern_xf(&xf, c, tile), area, rotated, checkerboard, tex, opaque);
        }
    }
    // Channels panel: alpha / Quick Mask overlays and single-channel views (channel_view.rs).
    if let Some(tex) = crate::channel_view::ensure(app, ctx, idx) {
        // Rendered from the live preview, so over the area the preview is shown in.
        let a = shown_area;
        if view.rotation.abs() > 0.05 {
            paint_mapped_image_at(
                painter,
                &xf,
                tex,
                Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                [a.x0 as f32, a.y0 as f32],
                [a.width() as f32, a.height() as f32],
                Color32::WHITE,
            );
        } else {
            let uv = if flip { Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0)) } else { Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)) };
            painter.image(tex, xf.doc_rect(a), uv, Color32::WHITE);
        }
    }
    crate::color_range_ui::paint_selection_preview(app, ctx, painter, doc.id, img_rect, flip);
    // Artboards: pasteboard between the boards, outlines and names (artboard_ui.rs).
    if doc.has_artboards() {
        let t = crate::theme::Tokens::get(ctx);
        let pasteboard = crate::prefs_ui::pasteboard_color(app);
        let dot_tex = dots(ctx, &t);
        // Cut around the layers as they are shown, so a loose layer dragged with the Move tool
        // (or otherwise previewed) is not covered where it now draws.
        let shown = display_doc(app, idx).0;
        // Each piece is painted as its own quad (the painter clips to the canvas), so a
        // rotated view does not paint a piece's bounding box over the boards or loose work.
        let fill = |q: &[Pos2; 4], tex: Option<egui::TextureId>, color: Color32| {
            let mut mesh = tex.map_or_else(egui::Mesh::default, egui::Mesh::with_texture);
            for &p in q {
                // Same phase as the dots around the document.
                let uv = if tex.is_some() { ((p - rect.min) / 22.0).to_pos2() } else { egui::epaint::WHITE_UV };
                mesh.vertices.push(egui::epaint::Vertex { pos: p, uv, color });
            }
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(mesh);
        };
        for q in crate::artboard_ui::pasteboard_quads(&xf, &shown) {
            if !Rect::from_points(&q).intersects(rect) {
                continue;
            }
            match pasteboard {
                Some(c) => fill(&q, None, c),
                None => {
                    fill(&q, None, t.canvas);
                    if let Some(id) = dot_tex {
                        fill(&q, Some(id), Color32::WHITE);
                    }
                }
            }
        }
        if primary {
            crate::artboard_ui::draw_frames(app, painter, &xf);
        }
    }

    // Pixel grid at high zoom, over pixels with content only (the GPU path draws its own).
    if !on_gpu && pixel_grid && crate::pixel_grid::shows_at(view.zoom) {
        crate::pixel_grid::paint(app, painter, &xf, idx, output, visible_doc_rect(&xf));
    }

    // Developer tile grid (`ui.set {"tileGrid": true}`): the active layer's storage tiles.
    // Not tied to View › Extras (⌘H), which hides only the View › Show overlays.
    if app.ui.view.tile_grid {
        crate::tile_overlay::paint(app, ctx, painter, &xf, idx, visible_doc_rect(&xf));
    }

    // View › Show › Layer Edges: drawn at this depth, but only once this frame's tool events are
    // in (`layer_edges_rect`), so a press that Auto-Selects a layer outlines that layer.
    let layer_edges = painter.add(egui::Shape::Noop);

    let ants = paint_selection_outline(app, SelectionPaintInput { ctx, painter, ui, view, xf, rect, doc, idx });

    (beyond, layer_edges, edge_rect, ants)
}

struct CanvasNavigationInput<'a> {
    ctx: &'a egui::Context,
    response: &'a egui::Response,
    free_hover: bool,
    wheel_owner: egui::Id,
    xf: ViewXform,
    ppp: f32,
    idx: usize,
}

fn update_canvas_navigation(app: &mut PhotocraftApp, view: &mut View, input: CanvasNavigationInput<'_>) {
    // Navigation (wheel_nav.rs): scroll pans (⇧ or ⌘/Ctrl sideways); pinch and ⌥-scroll zoom around the pointer.
    // The wheel also scrolls over the scrollbars drawn on top of the canvas (last frame's hover).
    let bars_id = egui::Id::unique(("pc-canvas-bars-hover", input.idx));
    let over_bars = input.ctx.data(|d| d.get_temp::<bool>(bars_id)).unwrap_or(false);
    let nav_hovered = input.response.hovered() || input.free_hover || over_bars;
    // Interface › Enable Animations off: the wheel moves the canvas without a glide (#5721).
    // General › Animated Zoom: the wheel zoom eases over the frames of a notch, or is one instant step.
    // Enhanced Controls › Enable Gestures off: two fingers on a touch screen neither zoom nor
    // turn the view (#204); the mouse and trackpad are unaffected.
    let touch_gestures = app.session.prefs().enhanced_controls.touch_gestures;
    let wheel = crate::wheel_nav::read_with(
        input.ctx,
        app.session.prefs().general.zoom_with_scroll_wheel,
        app.session.prefs().enhanced_controls.zoom_with_trackpad_pinch,
        app.session.prefs().general.animated_zoom,
        input.wheel_owner,
        nav_hovered,
        app.session.prefs().interface.enable_animations,
        touch_gestures,
    );
    if nav_hovered {
        let pointer = input.ctx.input(|i| i.pointer.hover_pos());
        match (wheel, pointer) {
            (Some(crate::wheel_nav::Wheel::Zoom(f)), Some(p)) => {
                // At a limit the view stays put, as in Photoshop: a notch past 12800 % neither
                // zooms nor slides the image towards the pointer.
                let nz = crate::zoom_levels::clamp(view.zoom * f, view.doc_size);
                if nz != view.zoom * f {
                    crate::wheel_nav::settle(input.ctx, input.wheel_owner);
                }
                if nz != view.zoom {
                    // Over the pasteboard the anchor is the image's nearest point (#3720).
                    let at = wheel_anchor(&input.xf, p, view.doc_size);
                    zoom_about(view, &input.xf, at, nz, false, input.ppp);
                }
            }
            (Some(crate::wheel_nav::Wheel::SnapZoom(n)), Some(p)) => {
                let dir = if n > 0 { 1 } else { -1 };
                let mut nz = view.zoom;
                for _ in 0..n.unsigned_abs() {
                    nz = crate::zoom_levels::step(nz, dir, view.doc_size);
                }
                if nz != view.zoom {
                    zoom_about(view, &input.xf, p, nz, false, input.ppp);
                }
            }
            (Some(crate::wheel_nav::Wheel::Pan(scroll)), _) => {
                let d = input.xf.unmap_vec(scroll) / (view.zoom / input.ppp);
                view.center[0] -= d.x;
                view.center[1] -= d.y;
            }
            _ => {}
        }
        let rotate_trackpad = app.session.prefs().enhanced_controls.rotate_view_with_trackpad;
        if let Some(delta) = crate::rotate_view::trackpad_delta(input.ctx, rotate_trackpad, touch_gestures) {
            view.zoom_animating = false;
            view.zoom_anchor = None;
            let next = view.rotation + crate::rotate_view::screen_delta(delta, input.xf.flip);
            if next.is_finite() {
                view.rotation = crate::rotate_view::wrap_deg(next);
            }
        }
    }
}

struct DialogCanvasInput<'a> {
    ctx: &'a egui::Context,
    view: &'a mut View,
    xf: &'a ViewXform,
    rect: Rect,
    painter: &'a egui::Painter,
    ppp: f32,
    primary: bool,
    middle: bool,
    space_pan: bool,
    temporary: Option<crate::hold_keys::Temporary>,
    free_hover: bool,
    img_rect: Rect,
}

fn handle_dialog_canvas_input(app: &mut PhotocraftApp, input: DialogCanvasInput<'_>) {
    let sticky_alt = app.ui.shell.sticky_alt;
    let zoom_out = |alt: bool| match input.temporary {
        Some(crate::hold_keys::Temporary::ZoomOut) => true,
        Some(crate::hold_keys::Temporary::ZoomIn) => false,
        _ => alt || sticky_alt,
    };

    // With a colour dialog picker armed the image is its eyedropper, whatever the tool; Space
    // and the middle button still pan. Curves and Replace Color use the same merged-composite
    // sampler as the Color Picker, never the reduced adjustment preview.
    let curves_picking = crate::adjust_dialog::picker_armed(app);
    let replace_picking = crate::filter_dialog::color_eyedropper_armed(app);
    let picking = input.primary && (crate::color_picker_ui::top(app).is_some() || curves_picking || replace_picking);
    let range_picking =
        input.primary && app.ui.dialogs.last().is_some_and(|d| crate::color_range_ui::owns(&d.fields) && crate::color_range_ui::controls(&d.fields).sampling);
    // Select and Mask's Refine Edge Brush paints on the image.
    let refine_brushing =
        input.primary && app.ui.dialogs.last().is_some_and(|d| crate::select_mask_view::owns(&d.fields) && crate::select_mask_view::brush_on(&d.fields));
    // Field Blur's pins are placed and dragged on the image (`blur_pins`).
    let pinning = input.primary && app.ui.dialogs.last().is_some_and(|d| crate::blur_pins::owns(&d.fields));
    // Layer Style on a Drop or Inner Shadow page: a drag on the image moves the shadow (#5701).
    let shadow_drag =
        !picking && app.ui.dialogs.last().is_some_and(|d| d.kind == crate::state::DialogKind::LayerStyle && crate::layer_style::drags_shadow(&d.fields));
    // ⌘/Ctrl-click with Curves open: a point at the pixel's tone (#7325), whatever the tool. The
    // press's own modifiers decide, and only the press adds: a drag would add one per frame.
    let curve_point_press = (input.primary && !input.middle && crate::adjust_dialog::curves_on_top(app))
        .then(|| {
            input.ctx.input(|i| {
                i.events.iter().rev().find_map(|e| match e {
                    egui::Event::PointerButton { button: PointerButton::Primary, pressed: true, modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
            })
        })
        .flatten()
        .map(|m| crate::workspace_ui::sticky_mods(app, m))
        .filter(|m| m.command);
    let dialog_hand = app.ui.tool == Tool::Hand && !picking && !range_picking && !refine_brushing && !pinning && !shadow_drag && curve_point_press.is_none();
    // ⌘Space / ⌥Space zoom under the dialog as they do without one, before Space pans (#6460).
    // With the Hand tool, ⌘ alone is Zoom In (#6434); under Curves that press is the dialog's
    // point (#7325), not a zoom.
    let curve_point_with_hand = curve_point_press.is_some() && app.ui.tool == Tool::Hand;
    let held_zoom =
        !input.middle && !curve_point_with_hand && matches!(input.temporary, Some(crate::hold_keys::Temporary::ZoomIn | crate::hold_keys::Temporary::ZoomOut));
    if held_zoom {
        let alt = input.ctx.input(|i| i.modifiers.alt);
        if input.free_hover {
            input.ctx.set_cursor_icon(crate::tool_cursor::zoom(input.ctx, zoom_out(alt)));
        }
        let b = crate::dialogs::free_buttons(input.ctx, input.rect);
        let pointer = input.ctx.input(|i| i.pointer.latest_pos());
        crate::zoom_tool::drag(app, input.ctx, input.view, input.xf, &b, pointer);
        if b.clicked
            && let Some(p) = pointer
        {
            let nz = crate::zoom_levels::step(input.view.zoom, if zoom_out(alt) { -1 } else { 1 }, input.view.doc_size);
            zoom_about(input.view, input.xf, p, nz, app.session.prefs().tools.zoom_clicked_point_to_center, input.ppp);
        }
    } else if let Some(d) = crate::dialogs::pan_delta(input.ctx, input.rect, dialog_hand) {
        let d = input.xf.unmap_vec(d) / (input.view.zoom / input.ppp);
        input.view.center[0] -= d.x;
        input.view.center[1] -= d.y;
        input.ctx.set_cursor_icon(crate::tool_cursor::hand(input.ctx, true));
    } else if input.free_hover && (input.space_pan || dialog_hand) {
        input.ctx.set_cursor_icon(crate::tool_cursor::hand(input.ctx, false));
    } else if shadow_drag && !input.middle && crate::dialogs::free_pointer_over(input.ctx, input.rect).is_some() {
        input.ctx.set_cursor_icon(egui::CursorIcon::Move);
        if let Some(p) = crate::dialogs::free_press(input.ctx, input.rect) {
            let (start, origin) = input.ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.press_origin().unwrap_or(p)));
            let total = input.xf.unmap_vec(p - origin) / (input.view.zoom / input.ppp);
            if let Some(d) = app.ui.dialogs.last_mut() {
                crate::layer_style::drag_shadow(&mut d.fields, start, [f64::from(total.x), f64::from(total.y)]);
            }
        }
    } else if picking && let Some(p) = crate::dialogs::free_pointer_over(input.ctx, input.rect) {
        if app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise {
            input.ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        } else {
            input.ctx.set_cursor_icon(pipette_cursor(input.ctx, p));
        }
        if let Some(p) = crate::dialogs::free_press(input.ctx, input.rect) {
            let d = input.xf.to_doc(p);
            if curves_picking {
                crate::adjust_dialog::sample_at(app, d[0], d[1]);
            } else if replace_picking {
                crate::filter_dialog::color_eyedropper_sample(app, d[0], d[1]);
            } else {
                crate::color_picker_ui::sample_at(app, d[0], d[1]);
            }
        }
    } else if pinning
        && !input.middle
        && let Some(p) = crate::dialogs::free_pointer_over(input.ctx, input.rect)
    {
        let press = crate::dialogs::free_press(input.ctx, input.rect);
        crate::blur_pins::canvas(app, input.ctx, input.xf, p, press);
    } else if refine_brushing
        && !input.middle
        && let Some(p) = crate::dialogs::free_pointer_over(input.ctx, input.rect)
    {
        let press = crate::dialogs::free_press(input.ctx, input.rect).map(|q| input.xf.to_doc(q));
        crate::select_mask_view::canvas_brush(app, input.painter, p, press, input.xf.zoom_x());
    } else if let Some(mods) = curve_point_press
        && let Some(p) = crate::dialogs::free_press(input.ctx, input.rect)
    {
        let d = input.xf.to_doc(p);
        crate::adjust_dialog::add_point_at(app, input.ctx, d[0], d[1], mods.shift);
    } else if input.primary
        && !input.middle
        && let Some(p) = crate::dialogs::free_pointer_over(input.ctx, input.rect).filter(|p| input.img_rect.contains(*p))
    {
        // Color Range samples colours with its eyedropper on the image itself.
        let press = crate::dialogs::free_press(input.ctx, input.rect)
            .filter(|p| input.img_rect.contains(*p) && input.ctx.input(|i| i.pointer.primary_pressed() || i.pointer.delta() != egui::Vec2::ZERO))
            .map(|q| input.xf.to_doc(q));
        crate::color_range_ui::canvas_eyedropper(app, input.ctx, p, press);
    }
}

struct CanvasToolInput<'a> {
    ctx: &'a egui::Context,
    response: &'a egui::Response,
    xf: &'a ViewXform,
    rect: Rect,
    full: Rect,
    tool: Tool,
    idx: usize,
    primary: bool,
    transform_owns_pointer: bool,
    hand_pan: bool,
    ppp: f32,
    under_dialog: bool,
    zoom_out: &'a dyn Fn(bool) -> bool,
    display_zoom: f32,
}

fn handle_primary_canvas_input(app: &mut PhotocraftApp, ui: &mut egui::Ui, view: &mut View, input: &CanvasToolInput<'_>) {
    let ctx = input.ctx;
    let response = input.response;
    let xf = input.xf;
    let rect = input.rect;
    let tool = input.tool;
    let idx = input.idx;
    let under_dialog = input.under_dialog;
    let mods = ui.input(|i| i.modifiers);
    // An ⌥-click the window manager took never arrives; say so (alt_grab.rs).
    crate::alt_grab::watch(app, ctx, egui::Id::unique(("pc-alt-grab", idx)), rect, tool, !under_dialog);
    // Tools follow the left button; the right one opens the Brush Preset picker or erases
    // (Preferences › Tools, `paint_mouse`).
    crate::paint_mouse::sync_tool_brush(app);
    let mut buttons = crate::paint_mouse::canvas_buttons(app, response, tool);
    // The Crop tool's frame is edited from the press (before egui's drag threshold), so the
    // pixels past the canvas show at once, as in Photoshop.
    if tool == Tool::Crop && response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_down()) && crate::crop_ui::press(app) {
        ctx.request_repaint();
    }
    // Capture intent at the press, before egui's drag threshold or click replay on release:
    // Type transforms keep Command; selections keep Add/Subtract if Shift/Alt is released.
    let capture_press =
        tool.is_type() || matches!(tool, Tool::RectMarquee | Tool::EllipseMarquee | Tool::SingleRowMarquee | Tool::SingleColumnMarquee | Tool::ObjectSelection);
    let tool_press = egui::Id::unique(("tool-pointer-press-modifiers", idx));
    if capture_press
        && let Some((p, press_mods)) = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers } if rect.contains(*pos) => Some((*pos, *modifiers)),
                _ => None,
            })
        })
    {
        let press_mods = crate::workspace_ui::sticky_mods(app, press_mods);
        ctx.data_mut(|d| d.insert_temp(tool_press, press_mods));
        if tool.is_type() && press_mods.command && app.ui.text_edit.is_some() {
            let d = xf.to_doc(p);
            tool_event(app, ToolEvent::Down { x: d[0], y: d[1], pressure: app.stylus.pressure() }, press_mods);
        }
    }

    handle_canvas_tool_menus(app, input, mods, &mut buttons);
    handle_canvas_tool_navigation(app, ui, view, input, &mut buttons);
    handle_canvas_tool_stroke(app, ui, input, mods, capture_press, tool_press, &mut buttons);
    handle_canvas_tool_clicks(app, ui, view, input, mods, tool_press, &buttons);
}

fn handle_canvas_tool_menus(app: &mut PhotocraftApp, input: &CanvasToolInput<'_>, mods: egui::Modifiers, buttons: &mut crate::paint_mouse::Buttons) {
    let ctx = input.ctx;
    let response = input.response;
    let xf = input.xf;
    let tool = input.tool;
    let transform_owns_pointer = input.transform_owns_pointer;
    // Right-click while transforming: switch the box's mode (Free Transform, Scale, Rotate,
    // Skew, Distort, Perspective, Warp), or rotate or flip the box.
    let transforming = app.ui.transform.as_ref().is_some_and(|t| t.warp.is_none());
    if response.secondary_clicked()
        && transforming
        && let Some(p) = response.interact_pointer_pos()
    {
        crate::canvas_tool_menu::open_transform(app, ctx, [p.x, p.y]);
    }
    let lasso_retracted = response.secondary_clicked()
        && match tool {
            Tool::Lasso | Tool::FreeformPen => crate::lasso_ui::undo_last_vertex(app),
            Tool::PolygonLasso => polygon_retract(app),
            _ => false,
        };
    // The Lasso (and the Freeform Pen, which draws the same way) reads its own presses and
    // drops the moves of a drag it didn't start, so an open Free Transform box takes the
    // usual drag path instead (#2153).
    if crate::lasso_ui::owns(tool) && !transform_owns_pointer {
        crate::lasso_ui::canvas_input(app, ctx, xf, response);
        (buttons.started, buttons.dragged, buttons.stopped, buttons.clicked) = (false, false, false, false);
    }
    // Right-click on a guide where it could be dragged (the Move tool, or ⌘ with a selection
    // tool): select it and open the guide menu, as Photoshop does.
    let guide_menu = response.secondary_clicked()
        && !lasso_retracted
        && !transforming
        && response.interact_pointer_pos().is_some_and(|p| crate::canvas_tool_menu::open_guide(app, ctx, tool, mods, [p.x, p.y], xf.to_doc(p)));
    // Right-click with the Move tool, or ⌘/Ctrl+right-click: the layers under the pointer.
    if response.secondary_clicked()
        && !lasso_retracted
        && !transforming
        && !guide_menu
        && crate::layer_pick_ui::is_gesture(tool, mods)
        && let Some(p) = response.interact_pointer_pos()
    {
        let d = xf.to_doc(p);
        app.ui.canvas_tool_menu = None;
        // No layers there (the pasteboard, with the Move tool): its colour menu instead.
        if !crate::layer_pick_ui::open(app, ctx, [p.x, p.y], d[0], d[1]) && !mods.command && crate::canvas_tool_menu::pasteboard_at(app, tool, d) {
            crate::canvas_tool_menu::open_pasteboard(app, ctx, tool, [p.x, p.y]);
        }
    }
    if response.secondary_clicked()
        && !lasso_retracted
        && !transforming
        && !guide_menu
        && !crate::layer_pick_ui::is_gesture(tool, mods)
        && let Some(p) = response.interact_pointer_pos()
    {
        // A pending crop: its menu, also outside the image, where the frame can reach (#6561).
        // Outside the image: the pasteboard colour menu (#2462), else the tool's own. The view
        // menu belongs to the selected Zoom or Hand, not to Space or a held zoom key. A crop
        // frame or Straighten line still dragged by the left button gets no menu over it: the
        // gesture and the held button both, as either alone can outlive a lost release. A
        // finger held still (egui's press-and-hold right-click) is down too, but drags nothing.
        let crop_gesture = tool == Tool::Crop
            && !response.long_touched()
            && (app.crop.drag.is_some() || app.crop.straighten.line.is_some())
            && ctx.input(|i| i.pointer.primary_down());
        if crop_gesture || crate::canvas_tool_menu::open_crop(app, ctx, tool, [p.x, p.y]) {
        } else if crate::canvas_tool_menu::pasteboard_at(app, tool, xf.to_doc(p)) {
            crate::canvas_tool_menu::open_pasteboard(app, ctx, tool, [p.x, p.y]);
        } else if crate::canvas_tool_menu::is_view_tool(tool) {
            if tool == app.ui.tool {
                crate::canvas_tool_menu::open_view(app, ctx, tool, [p.x, p.y], xf.to_doc(p));
            }
        } else {
            crate::canvas_tool_menu::open_at(app, ctx, tool, [p.x, p.y], Some(xf.to_doc(p)));
        }
    }
}

fn handle_canvas_tool_navigation(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    view: &mut View,
    input: &CanvasToolInput<'_>,
    buttons: &mut crate::paint_mouse::Buttons,
) {
    let ctx = input.ctx;
    let response = input.response;
    let xf = input.xf;
    let tool = input.tool;
    let idx = input.idx;
    let primary = input.primary;
    let transform_owns_pointer = input.transform_owns_pointer;
    let hand_pan = input.hand_pan;
    let ppp = input.ppp;
    let zoom_out = input.zoom_out;
    // The (temporary) Hand pans above; its gestures never reach the tool underneath. While a
    // Free Transform box is open the Hand does not pan, so the box gets these events (#2302).
    if hand_pan {
        (buttons.started, buttons.dragged, buttons.stopped) = (false, false, false);
    }
    // Rotate View: drag around the view centre. Compass clicks are not a rotate drag.
    let over_compass = crate::rotate_view::compass_visible(view.rotation)
        && response.interact_pointer_pos().is_some_and(|p| crate::rotate_view::compass_rect(xf.rect).contains(p));
    if tool == Tool::RotateView && !over_compass && crate::rotate_view::drag(ctx, view, xf, buttons, response.interact_pointer_pos()) {
        (buttons.started, buttons.dragged, buttons.stopped, buttons.clicked) = (false, false, false, false);
    }
    // Animated Zoom: a Zoom press held still zooms continuously (zoom_hold.rs); while it does,
    // it owns the press: no scrub, and its release is no click step.
    if tool == Tool::Zoom && !transform_owns_pointer {
        let out = zoom_out(ui.input(|i| i.modifiers.alt));
        let on_canvas = !input.under_dialog && !over_compass && response.is_pointer_button_down_on();
        let hold = crate::zoom_hold::hold_id(idx, primary, ctx.viewport_id());
        if crate::zoom_hold::update(ctx, view, xf, app.session.prefs().general.animated_zoom, out, ppp, on_canvas, hold) {
            (buttons.started, buttons.dragged, buttons.stopped, buttons.clicked) = (false, false, false, false);
        }
    }
    // Zoom tool drags: scrubby zoom or a zoom rectangle (zoom_tool.rs); clicks step below.
    // Like the Hand, the selected Zoom yields to an open Free Transform box (#4799).
    if tool == Tool::Zoom && !transform_owns_pointer && crate::zoom_tool::drag(app, ctx, view, xf, buttons, response.interact_pointer_pos()) {
        (buttons.started, buttons.dragged, buttons.stopped) = (false, false, false);
    }
}

fn handle_canvas_tool_stroke(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    input: &CanvasToolInput<'_>,
    mods: egui::Modifiers,
    capture_press: bool,
    tool_press: egui::Id,
    buttons: &mut crate::paint_mouse::Buttons,
) {
    let ctx = input.ctx;
    let response = input.response;
    let xf = input.xf;
    let rect = input.rect;
    let full = input.full;
    let tool = input.tool;
    // A drag is only recognised once the pointer has moved past egui's click distance: the
    // gesture starts where the button went down, not where it is now (#123).
    let gesture_active_before = app.drag.is_some();
    // Stroke timing: egui's events carry no timestamps, so a press takes the frame time and
    // the moves below are spread between the previous point and this frame (`spread_ms`).
    let now_ms = ui.input(|i| i.time) * 1000.0;
    app.stylus.clock_ms = now_ms;
    // Live painting tools start on the press, not once the pointer passes egui's click
    // distance: the first dab shows at once. The drag recognised later continues that stroke,
    // and a click (or a long press that never moved) just ends it.
    let physical_right = app.services.right_button_down.as_ref().is_some_and(|f| f());
    let right_down = ui.input(|i| i.pointer.secondary_down()) || physical_right;
    if strokes_live(tool)
        && !right_down
        && app.drag.is_none()
        && response.is_pointer_button_down_on()
        && ui.input(|i| i.pointer.primary_pressed())
        && !crate::brush_mode_menu::blocks_canvas(app, ctx)
        && let Some(p) = ui.input(|i| i.pointer.press_origin()).filter(|p| rect.contains(*p))
    {
        let d = xf.to_doc(p);
        // The press's own modifiers: a ⇧ that arrives with the click still connects the line.
        let press_mods = ui.input(|i| pointer_button_modifiers(&i.events, PointerButton::Primary)).unwrap_or(mods);
        tool_event(app, ToolEvent::Down { x: d[0], y: d[1], pressure: app.stylus.pressure() }, press_mods);
        app.press_stroke = app.drag.is_some();
    }
    let press_stroke = app.press_stroke;
    // A live tool's press that went to the ⌥ eyedropper keeps sampling for the whole press: the
    // drag recognised later must not decide again with ⌥ already let go and paint (#7802).
    if press_stroke || (strokes_live(tool) && (app.alt_sampling || app.mixer_sampling)) {
        buttons.started = false;
    }
    if buttons.started
        && let Some(p) = ui.input(|i| i.pointer.press_origin()).filter(|p| rect.contains(*p)).or(response.interact_pointer_pos())
    {
        if tool == Tool::Move && app.ui.transform.is_none() {
            begin_transform_controls_at(app, ctx, xf, p);
        }
        let d = xf.to_doc(p);
        let press_mods = if capture_press { ctx.data(|d| d.get_temp(tool_press)).unwrap_or(mods) } else { mods };
        tool_event(app, ToolEvent::Down { x: d[0], y: d[1], pressure: app.stylus.pressure() }, press_mods);
    }
    // A stroke started on the press takes its moves from the press on, not only once egui
    // recognises a drag (past its click distance): otherwise the first few points are lost
    // and the stroke starts with a straight segment and no pressure ramp.
    let press_stroke_moving = press_stroke && app.drag.is_some();
    if buttons.dragged || buttons.stopped || press_stroke_moving {
        // Feed every pointer move the OS delivered this frame, not just the latest position, so
        // a fast stroke is sampled densely and renders as a smooth curve instead of a coarse
        // polyline. egui-winit pushes one `PointerMoved` per `CursorMoved`, and they accumulate
        // while a frame is slow, so reading them all recovers the moves a per-frame
        // `interact_pointer_pos()` would drop. Only freehand tools take the whole batch; the
        // rest follow the pointer's latest position. The moves are bounded to the gesture's own
        // press..release interval (`pointer_moves`), so the start and stop frames keep their
        // valid samples without leaking a move from outside the gesture.
        let events = ui.input(|i| i.events.clone());
        let button = if response.dragged_by(PointerButton::Secondary)
            || response.drag_started_by(PointerButton::Secondary)
            || response.drag_stopped_by(PointerButton::Secondary)
        {
            PointerButton::Secondary
        } else {
            PointerButton::Primary
        };
        let press_this_frame = events.iter().any(|e| matches!(e, egui::Event::PointerButton { button: b, pressed: true, .. } if *b == button));
        let down_at_start = !press_this_frame && (gesture_active_before || buttons.started);
        let mut positions = if app.brush_resize.is_some() {
            response.interact_pointer_pos().into_iter().collect()
        } else if freehand_tool(tool) {
            pointer_moves(&events, button, down_at_start)
        } else if buttons.dragged {
            response.interact_pointer_pos().into_iter().collect()
        } else {
            Vec::new()
        };
        // A frame with no raw move still tracks a held gesture (a still pointer, a modifier
        // change): fall back to the latest position, as the old code always did.
        if positions.is_empty()
            && (buttons.dragged || press_stroke_moving)
            && let Some(p) = response.interact_pointer_pos().or(ui.input(|i| i.pointer.latest_pos()))
        {
            positions.push(p);
        }
        app.defer_live_stroke = true;
        let base = app.stylus.last_point_ms().map_or(now_ms, |t| t.min(now_ms));
        let n = positions.len();
        for (k, p) in positions.into_iter().enumerate() {
            app.stylus.clock_ms = spread_ms(base, now_ms, k, n);
            // This move's share of the pen samples the frame brought (pressure, tilt, rotation).
            app.stylus.select(k, n);
            let d = xf.to_doc(p);
            tool_event(app, ToolEvent::Move { x: d[0], y: d[1], pressure: app.stylus.pressure() }, mods);
        }
        app.stylus.clear_selection();
        app.stylus.clock_ms = now_ms;
        // A held pen keeps the clock running for airbrush build-up and smoothing catch-up.
        if app.drag.is_some() && stroke_wants_time(app) {
            ctx.request_repaint_after(std::time::Duration::from_millis(HOLD_REPEAT_MS as u64));
        }
        app.defer_live_stroke = false;
        // One live-stroke update for the whole frame, not one per recovered sample.
        feed_live_stroke(app);
        crate::magnetic_lasso_ui::flush(app);
    }
    if buttons.stopped {
        // The live painting press already created the stroke and this release commits it.
        // egui may also report clicked on the same release; replaying that click paints a
        // second full-pressure dab over the real (possibly light-pressure) pen dab.
        if press_stroke {
            buttons.clicked = false;
        }
        let at = response.interact_pointer_pos();
        // #6286: a guide let go over a ruler is removed, whichever ruler it is. Tested here in
        // screen space, where the rulers are: on the guide's own axis the release can still be
        // inside the document, which is what `finish_drag` sees.
        if app.guide_drag.is_some() {
            let on_ruler = at.is_some_and(|p| crate::rulers::dropped_on_a_ruler(app, full, p));
            if let Some(g) = app.guide_drag.as_mut() {
                g.over_ruler = on_ruler;
            }
        }
        let p = at.map(|p| xf.to_doc(p)).or_else(|| app.drag.as_ref().and_then(|d| d.points.last().map(|q| [q[0], q[1]])));
        if let Some(d) = p {
            tap_at_peak_pressure(app);
            tool_event(app, ToolEvent::Up { x: d[0], y: d[1] }, mods);
        }
    }
    // A stroke started on the press ends with the release, whether egui saw a drag, a click,
    // or neither (a long press that never moved).
    if press_stroke && !buttons.stopped && ui.input(|i| i.pointer.primary_released()) {
        buttons.clicked = false;
        let p = response.interact_pointer_pos().map(|p| xf.to_doc(p)).or_else(|| app.drag.as_ref().and_then(|d| d.points.last().map(|q| [q[0], q[1]])));
        if let Some(d) = p
            && app.drag.is_some()
        {
            tap_at_peak_pressure(app);
            tool_event(app, ToolEvent::Up { x: d[0], y: d[1] }, mods);
        }
    }
    if press_stroke && (app.drag.is_none() || !ui.input(|i| i.pointer.primary_down())) {
        app.press_stroke = false;
    }
}

fn handle_canvas_tool_clicks(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    view: &mut View,
    input: &CanvasToolInput<'_>,
    mods: egui::Modifiers,
    tool_press: egui::Id,
    buttons: &crate::paint_mouse::Buttons,
) {
    let ctx = input.ctx;
    let response = input.response;
    let xf = input.xf;
    let rect = input.rect;
    let tool = input.tool;
    let transform_owns_pointer = input.transform_owns_pointer;
    let ppp = input.ppp;
    let zoom_out = input.zoom_out;
    let display_zoom = input.display_zoom;
    if buttons.clicked
        && let Some(p) = response.interact_pointer_pos()
    {
        let click_button = if app.secondary_erase { PointerButton::Secondary } else { PointerButton::Primary };
        let click_mods = ui.input(|i| pointer_button_modifiers(&i.events, click_button)).unwrap_or(mods);
        let click_mods = if tool == Tool::ObjectSelection && click_button == PointerButton::Primary {
            ctx.data(|d| d.get_temp(tool_press)).unwrap_or(click_mods)
        } else {
            click_mods
        };
        let d = xf.to_doc(p);
        match tool {
            Tool::Zoom if !transform_owns_pointer => {
                let nz = crate::zoom_levels::step(view.zoom, if zoom_out(click_mods.alt) { -1 } else { 1 }, view.doc_size);
                let center = app.session.prefs().tools.zoom_clicked_point_to_center;
                let doc_anchor = xf.to_doc(p);
                let screen_anchor = p;
                zoom_about(view, xf, p, nz, center, ppp);
                view.request_zoom_animation(app.session.prefs().general.animated_zoom && nz != display_zoom);
                view.zoom_anchor = (view.zoom_animating && !center).then_some((doc_anchor, [screen_anchor.x, screen_anchor.y]));
            }
            // A click with the (temporary) Hand does nothing, never the tool underneath.
            Tool::Hand | Tool::RotateView => {}
            // Double-clicking closes the polygonal lasso: the first click placed the last
            // vertex (or already closed it), so the second never starts a new polygon. egui
            // reports it as a triple click when a vertex went down shortly before.
            Tool::PolygonLasso if response.double_clicked() || response.triple_clicked() => commit_polygon(app),
            // The same for the Magnetic Lasso: along the edges, or straight with ⌥.
            Tool::MagneticLasso if response.double_clicked() || response.triple_clicked() => crate::magnetic_lasso_ui::close(app, mods.alt),
            // The first click placed (or grabbed) the point; the second toggles it.
            Tool::CurvaturePen if response.double_clicked() => crate::vector_ui::curvature_toggle(app, d[0], d[1]),
            // Double-clicking a guide opens Preferences › Guides, Grid & Slices, as in Photoshop.
            Tool::Move if response.double_clicked() && crate::rulers::guide_at(app, d[0], d[1]).is_some() => {
                if let Err(e) = crate::menus::invoke(app, ctx, "edit.preferences.guidesGridAndSlices", json!({})) {
                    app.ui.status = e;
                }
            }
            _ => {
                if tool == Tool::Move && app.ui.transform.is_none() {
                    begin_transform_controls_at(app, ctx, xf, p);
                }
                // If this tool was not handled on pointer-down, fall back to a
                // click-sized gesture while preserving pen pressure (mouse stays at 1).
                tool_event(app, ToolEvent::Down { x: d[0], y: d[1], pressure: app.stylus.pressure() }, click_mods);
                tap_at_peak_pressure(app);
                tool_event(app, ToolEvent::Up { x: d[0], y: d[1] }, click_mods);
            }
        }
    }
    // Magnetic Lasso: between clicks the border follows the pointer with the button up.
    if tool == Tool::MagneticLasso && app.ui.magnetic.active() && !(buttons.started || buttons.dragged || buttons.stopped) && response.hovered() {
        let moves: Vec<Pos2> = ui.input(|i| i.events.iter().filter_map(|e| if let egui::Event::PointerMoved(p) = e { Some(*p) } else { None }).collect());
        app.defer_live_stroke = true;
        for p in moves.into_iter().filter(|p| rect.contains(*p)) {
            let d = xf.to_doc(p);
            tool_event(app, ToolEvent::Move { x: d[0], y: d[1], pressure: app.stylus.pressure() }, mods);
        }
        app.defer_live_stroke = false;
        crate::magnetic_lasso_ui::flush(app);
    }
    // Move tool: a double-click on type edits it, as in Photoshop (#1132). Only the selected
    // Move tool, not ⌘ held over another tool; an open transform box keeps its double-click
    // (commit, below).
    if tool == Tool::Move
        && app.ui.tool == Tool::Move
        && app.ui.transform.is_none()
        && response.double_clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        let d = xf.to_doc(p);
        crate::type_tool::edit_at(app, d[0], d[1]);
    }
    if app.ui.transform.is_some() && response.double_clicked() {
        crate::transform_tool::commit(app);
    }
    if crate::cam_transform::active(app) && response.double_clicked() {
        crate::cam_transform::commit(app);
    }
    // Crop: a double-click inside the frame commits it, as ↵ does (#1792).
    if tool == Tool::Crop && response.double_clicked() && response.interact_pointer_pos().is_some_and(|p| crate::crop_ui::commits_at(app, xf.to_doc(p))) {
        commit_crop(app);
    }
    // Type: 2 clicks select a word, 3 the line, 4 the paragraph, 5 all text (Photoshop).
    if tool.is_type()
        && response.clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        if crate::type_transform::visible(app, crate::workspace_ui::sticky_mods(app, mods)) {
            app.type_clicks = None;
        } else {
            let n = if response.triple_clicked() {
                3
            } else if response.double_clicked() {
                2
            } else {
                1
            };
            let d = xf.to_doc(p);
            crate::type_tool::multi_click(app, n, d[0], d[1]);
        }
    }
}

struct CanvasOverlayInput<'a> {
    ctx: &'a egui::Context,
    painter: &'a egui::Painter,
    xf: &'a ViewXform,
    response: &'a egui::Response,
    doc: &'a std::sync::Arc<Document>,
    tool: Tool,
    idx: usize,
    output: Option<u32>,
    border: photocraft_engine::prefs::CanvasBorder,
    beyond: bool,
    edge_rect: Rect,
    panning: bool,
    zoom_out: &'a dyn Fn(bool) -> bool,
}

fn paint_primary_overlays(app: &mut PhotocraftApp, ui: &mut egui::Ui, view: &mut View, input: &CanvasOverlayInput<'_>, ants: Vec<egui::Shape>) {
    let ctx = input.ctx;
    let painter = input.painter;
    let xf = input.xf;
    let response = input.response;
    let doc = input.doc;
    let idx = input.idx;
    let border = input.border;
    let beyond = input.beyond;
    let edge_rect = input.edge_rect;
    // The transform preview is image content: it goes under the grid and guides, which
    // stay visible across a layer being transformed (#5086); the box is drawn later.
    crate::transform_tool::draw_content(app, painter, xf);
    // So are Puppet Warp's, Perspective Warp's and Vanishing Point's warped pixels (#7781).
    crate::distort_ui::draw_content(app, painter, xf);
    if app.ui.extras.grid && app.ui.view.extras {
        crate::rulers::draw_grid(app, painter, xf, doc);
    }
    if app.ui.view.shows(app.ui.view.show.canvas_guides) || crate::guide_layout_ui::preview(app, doc).is_some() {
        crate::rulers::draw_guides(app, painter, xf, doc);
    }
    // The selection's ants go over the grid and guides, as in Photoshop (#10671).
    painter.extend(ants);
    // The pending crop frame shows on its own document only, not on the other tiles (#1918).
    let foreign_crop = if app.session.active_index() == Some(idx) { None } else { app.ui.crop_rect.take() };
    draw_drag_preview(app, painter, xf);
    if app.session.active_index() == Some(idx) {
        crate::move_box::draw(app, painter, xf);
    }
    if foreign_crop.is_some() {
        app.ui.crop_rect = foreign_crop;
    }
    crate::zoom_tool::draw(ctx, painter);
    let resizing = crate::brush_resize::draw(app, painter, xf);
    draw_transform_controls(app, painter, xf);
    crate::move_hover::draw(app, painter, xf, idx, response.hover_pos());
    crate::object_finder::draw(app, painter, xf, idx, response.hover_pos());
    crate::layer_pick_ui::show(app, ctx);
    let shown = app.ui.views.get(idx).cloned();
    crate::canvas_tool_menu::show(app, ctx);
    // A view menu row (Zoom In, Fit on Screen…) changed the camera: it wins over this
    // frame's copy, which is written back at the end.
    if let Some(now) = app.ui.views.get(idx)
        && Some(now) != shown.as_ref()
    {
        *view = now.clone();
        ctx.request_repaint();
    }
    crate::snap_ui::draw(app, painter, xf);
    crate::move_distance::draw(app, painter, xf, idx, response.hover_pos());
    // The canvas edge isn't outlined while the pixels past it show (Photoshop's crop preview),
    // nor while the Crop tool hides everything outside its box (Show Cropped Area off).
    if border == photocraft_engine::prefs::CanvasBorder::Line && !beyond && !crate::crop_shield::hides_outside(app) {
        painter.rect_stroke(edge_rect, 0.0, Stroke::new(1.0, Color32::from_gray(20)), egui::StrokeKind::Outside);
    }
    crate::type_tool::draw_overlay(app, painter, xf);
    crate::transform_tool::draw_overlay(app, painter, xf);
    crate::cam_transform::draw(app, painter, xf);
    crate::cam_preview::wake(app, painter.ctx());
    crate::distort_ui::draw_overlay(app, painter, xf);
    crate::perspective_crop_ui::draw_overlay(app, painter, xf);
    crate::retouch_ui::draw_source_marker(app, painter, xf);
    crate::vector_ui::draw_overlay(app, painter, xf, doc);
    crate::symmetry_ui::draw(app, painter, xf);
    crate::analysis_ui::draw_overlay(app, painter, xf);
    crate::gradient_ui::draw_overlay(app, painter, xf);
    crate::slice_ui::draw_overlay(app, painter, xf);
    crate::frame_tool::draw_overlay(app, painter, xf);
    crate::artboard_tool::draw_overlay(app, painter, xf);

    draw_canvas_tool_cursor(app, ui, input, resizing);
}

fn draw_canvas_tool_cursor(app: &mut PhotocraftApp, ui: &mut egui::Ui, input: &CanvasOverlayInput<'_>, resizing: bool) {
    let ctx = input.ctx;
    let xf = input.xf;
    let response = input.response;
    let tool = input.tool;
    let panning = input.panning;
    // Tool cursors (Photoshop-style).
    let guide_hover = response.hover_pos().and_then(|p| {
        let d = xf.to_doc(p);
        if tool == Tool::Move {
            return crate::rulers::guide_at(app, d[0], d[1]);
        }
        command_guide_at(app, tool, d, crate::workspace_ui::sticky_mods(app, ui.input(|i| i.modifiers)))
    });
    if panning {
        // The fist set with the pan above stays (#2196).
    } else if crate::tool_cursor::pointer_elsewhere(ctx, &crate::panel_docking::native_viewports(app, ctx)) {
        // The pointer is over a floating panel's own window; this window's last position of
        // it is stale, so no tool cursor or brush outline is painted at it.
    } else if let Some((vertical, _)) = guide_hover {
        ui.ctx().set_cursor_icon(crate::rulers::guide_cursor(xf, vertical));
    } else if let Some((p, mods, c)) = response.hover_pos().and_then(|p| {
        let mods = crate::workspace_ui::sticky_mods(app, ui.input(|i| i.modifiers));
        let c = crate::transform_tool::cursor(app, xf, xf.to_doc(p), mods)?;
        Some((p, mods, c))
    }) {
        ui.ctx().set_cursor_icon(c);
        // CursorIcon::None hides the pointer; the curved arrow has to be drawn or the
        // rotate zone looks like a plain cursor (#4050).
        let d = xf.to_doc(p);
        if let Some(k) = crate::transform_tool::rotate_cursor_dir(app, d) {
            let toward = xf.to_screen((d[0] + k[0] * 16.0) as f32, (d[1] + k[1] * 16.0) as f32) - p;
            crate::crop_ui::draw_turn_cursor(ctx, p, toward);
        } else if c == egui::CursorIcon::None {
            // Likewise the distort and skew pointers over a handle (#5405).
            crate::transform_tool::draw_handle_cursor(app, ctx, xf, p, mods);
        }
    } else if let Some(c) = response.hover_pos().and_then(|p| crate::symmetry_ui::cursor(app, xf, xf.to_doc(p))) {
        ui.ctx().set_cursor_icon(c);
    } else if let Some(c) = response.hover_pos().and_then(|p| crate::cam_transform::cursor(app, ctx, xf, p)) {
        ui.ctx().set_cursor_icon(c);
    } else if let Some(p) = response.hover_pos().filter(|p| crate::corner_radius::hovers(app, xf.to_doc(*p))) {
        // Over a live rectangle's corner widget (or dragging one): the arrow with a rounded
        // corner badge, as Photoshop's corner radius cursor.
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
        crate::icons::cursor_badge(ctx, "scan", p);
    } else if let Some((r, p, c)) =
        response.hover_pos().and_then(|p| transform_controls_rect(app, xf).and_then(|r| transform_controls_cursor(r, p).map(|c| (r, p, c))))
    {
        // The Move tool draws handles without opening a Free Transform session.
        // Give them their resize/rotate cursors before the first pointer press (#2871).
        ui.ctx().set_cursor_icon(c);
        if let Some(toward) = transform_controls_turn(r, p) {
            crate::crop_ui::draw_turn_cursor(ctx, p, toward);
        }
    } else if let Some(c) = response.hover_pos().filter(|_| tool.is_type()).and_then(|p| {
        let mods = crate::workspace_ui::sticky_mods(app, ui.input(|i| i.modifiers));
        crate::type_transform::cursor(app, ctx, xf, p, mods)
    }) {
        ui.ctx().set_cursor_icon(c);
    } else if let Some(c) = response.hover_pos().filter(|_| tool == Tool::Artboard).and_then(|p| crate::artboard_tool::cursor(app, xf, xf.to_doc(p))) {
        ui.ctx().set_cursor_icon(c);
    } else if let Some((p, c)) = response.hover_pos().filter(|_| tool == Tool::Crop).and_then(|p| Some((p, crate::crop_ui::cursor(app, xf.to_doc(p))?))) {
        ui.ctx().set_cursor_icon(c);
        let d = xf.to_doc(p);
        if let Some(k) = crate::crop_ui::turn_cursor_dir(app, d) {
            let toward = xf.to_screen((d[0] + k[0] * 16.0) as f32, (d[1] + k[1] * 16.0) as f32) - p;
            crate::crop_ui::draw_turn_cursor(ctx, p, toward);
        }
    } else if let Some(p) = response.hover_pos()
        && let Some(c) = selection_cursor(app, tool, xf.to_doc(p), crate::workspace_ui::sticky_mods(app, ui.input(|i| i.modifiers)))
    {
        // Over a selection (Photoshop's cursors): the move cursor where a press drags the
        // outline (#1428), the arrow with a badge for what a press does to the selected
        // pixels, a hollow arrowhead while they are dragged, a plain arrow over a floating
        // piece.
        let icon = match c {
            SelCursor::Outline => egui::CursorIcon::Move,
            SelCursor::Dragging => {
                // The tip of the arrowhead is at (4, 4) of its 24-unit box.
                crate::icons::cursor(ui.ctx(), "mouse-pointer-2", p, vec2(4.0, 4.0) / 24.0, 18.0);
                egui::CursorIcon::None
            }
            SelCursor::Piece => egui::CursorIcon::Default,
            SelCursor::Cut => {
                crate::icons::cursor_badge(ui.ctx(), "scissors", p);
                egui::CursorIcon::Default
            }
            SelCursor::Copy => {
                crate::icons::cursor_copy_badge(ui.ctx(), p);
                egui::CursorIcon::Default
            }
        };
        ui.ctx().set_cursor_icon(icon);
    } else if let Some(p) = response.hover_pos() {
        draw_default_tool_cursor(app, ui, input, resizing, p);
    }
}

fn draw_default_tool_cursor(app: &mut PhotocraftApp, ui: &mut egui::Ui, input: &CanvasOverlayInput<'_>, resizing: bool, p: Pos2) {
    let painter = input.painter;
    let xf = input.xf;
    let response = input.response;
    let tool = input.tool;
    let output = input.output;
    let zoom_out = input.zoom_out;

    let alt = ui.input(|i| i.modifiers.alt);
    if !resizing && !alt && !app.ui.shell.sticky_alt {
        draw_clone_preview(app, painter, xf, xf.to_doc(p), output);
    }
    // The Type tool over a path shows Photoshop's area-type or type-on-a-path pointer (#218).
    // Not while a type edit is open or a text box is dragged out, nor over type a click
    // would edit (`hover_path_target`).
    let type_path = if matches!(tool, Tool::Type | Tool::VerticalType) {
        let [x, y] = xf.to_doc(p);
        let shift = crate::workspace_ui::sticky_mods(app, ui.input(|i| i.modifiers)).shift;
        crate::type_tool::hover_path_target(app, x, y, shift)
    } else {
        None
    };
    let icon = match tool {
        // Resizing the brush: the circle stays where the drag began (`brush_resize`).
        t if resizing && crate::brush_resize::applies(t) => egui::CursorIcon::None,
        // ⌥ turns a painting tool into the Eyedropper (`alt_eyedropper`): its cursor too,
        // unless Preferences › Cursors › Other Cursors asks for the precise crosshair.
        t if app.alt_sampling
            || (app.drag.is_none()
                && !app.services.right_button_down.as_ref().is_some_and(|f| f())
                && alt_picks(t, crate::workspace_ui::sticky_mods(app, ui.input(|i| i.modifiers)))) =>
        {
            // While it samples (button held), the comparison ring replaces the cursor (#213).
            if let Some(icon) = eyedropper_ring(app, painter, xf, p, response.is_pointer_button_down_on()) {
                icon
            } else if app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise {
                egui::CursorIcon::Crosshair
            } else {
                pipette_cursor(ui.ctx(), p)
            }
        }
        t if t.is_brushlike() || t == Tool::QuickSelection => {
            // Preferences › Cursors: brush tip outline (normal = the 50% contour, or
            // full size), precise crosshair, or the standard pointer.
            use photocraft_engine::prefs::PaintingCursor;
            let cur = app.session.prefs().cursors.clone();
            // ⇪ forces the precise crosshair, whatever the cursor preference (#1758).
            if app.caps_lock {
                crate::tool_cursor::crosshair(painter, p, 6.0, 0.0)
            } else {
                let painting = app.drag.is_some();
                let brush = &app.session.tools.brush;
                let full = (brush.size / 2.0 * xf.zoom).max(1.0);
                match cur.painting {
                    PaintingCursor::Standard => egui::CursorIcon::Default,
                    PaintingCursor::Precise => crate::tool_cursor::crosshair(painter, p, 6.0, 0.0),
                    _ if painting && cur.show_only_crosshair_while_painting => crate::tool_cursor::crosshair(painter, p, 5.0, 0.0),
                    // The Pencil (and the Eraser in Pencil mode): the outline of the
                    // whole pixels its dab fills, on the pixel grid. A round tip is a
                    // disc, a tiny one the whole square (#2662). The Block-mode Eraser:
                    // its square, 16 screen pixels at any zoom (#2770).
                    _ if tool == Tool::Pencil || crate::eraser_ui::pencil_mode(app, tool) || crate::eraser_ui::block_mode(app, tool) => {
                        let block = crate::eraser_ui::block_mode(app, tool);
                        let size = if block { crate::eraser_ui::block_size(app) } else { brush.size };
                        let ppp = painter.ctx().pixels_per_point();
                        let sq = pencil_cursor_rect(xf, xf.to_doc(p), size, ppp);
                        let px = 1.0 / ppp;
                        let (dark, light) = (Stroke::new(px, Color32::from_black_alpha(160)), Stroke::new(px, Color32::from_white_alpha(230)));
                        if !block && pencil_tip_is_round(size) {
                            let r = sq.width() / 2.0;
                            painter.circle_stroke(sq.center(), r + px / 2.0, dark);
                            painter.circle_stroke(sq.center(), (r - px / 2.0).max(0.0), light);
                        } else {
                            painter.rect_stroke(sq, 0.0, dark, egui::StrokeKind::Outside);
                            painter.rect_stroke(sq, 0.0, light, egui::StrokeKind::Inside);
                        }
                        // Too small to see where it is: the hotspot as well.
                        if cur.show_crosshair_in_brush_tip || sq.width() < 6.0 {
                            crate::tool_cursor::crosshair(painter, p, 4.0, 0.0);
                        }
                        egui::CursorIcon::None
                    }
                    // A shaped tip (sampled bitmap, or a rotated/flattened round tip)
                    // traces its outline, turned and flipped with the view; a plain round
                    // tip keeps the circle.
                    _ => {
                        let centre = brush_tip_centre(tool, alt || app.ui.shell.sticky_alt, cur.show_crosshair_in_brush_tip);
                        let axes = crate::brush_cursor::pen_axes(xf, brush, app.stylus.sample());
                        crate::brush_cursor::cursor(painter, p, brush, full, axes, cur.painting == PaintingCursor::NormalTip, centre)
                    }
                }
            }
        }
        // The Eyedropper: the comparison ring follows the pointer while it samples (#213);
        // otherwise its pipette, or the crosshair for precise Other Cursors.
        Tool::Eyedropper => {
            if let Some(icon) = eyedropper_ring(app, painter, xf, p, response.is_pointer_button_down_on()) {
                icon
            } else if precise_other_cursor(app) {
                egui::CursorIcon::Crosshair
            } else {
                pipette_cursor(ui.ctx(), p)
            }
        }
        // The Color Sampler: a pipette, or the crosshair for precise Other Cursors.
        Tool::ColorSampler => {
            if precise_other_cursor(app) {
                egui::CursorIcon::Crosshair
            } else {
                pipette_cursor(ui.ctx(), p)
            }
        }
        // The Paint Bucket: a bucket, or the crosshair for precise Other Cursors (#3968).
        Tool::PaintBucket => {
            if precise_other_cursor(app) {
                egui::CursorIcon::Crosshair
            } else {
                paint_bucket_cursor(ui.ctx(), p)
            }
        }
        // Over the first anchor, the close-path pointer (#4596), before the pens' own cursors.
        Tool::Pen | Tool::CurvaturePen
            if {
                let [x, y] = xf.to_doc(p);
                crate::vector_ui::closes_path_at(app, tool, x, y)
            } =>
        {
            crate::tool_cursor::close_path(painter, p)
        }
        // The pen tools: their own icon with its nib on the point, or the crosshair for
        // precise Other Cursors (#4639).
        Tool::Pen | Tool::FreeformPen | Tool::CurvaturePen | Tool::AddAnchorPoint | Tool::DeleteAnchorPoint | Tool::ConvertPoint => {
            if precise_other_cursor(app) { egui::CursorIcon::Crosshair } else { pen_cursor(ui.ctx(), tool, p) }
        }
        // Preferences › Cursors › Other Cursors: Precise shows a crosshair for every tool.
        Tool::Move if crate::move_mods::copying(app, alt) => egui::CursorIcon::Copy,
        Tool::Move | Tool::Type | Tool::VerticalType | Tool::HorizontalTypeMask | Tool::VerticalTypeMask
            if app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise =>
        {
            egui::CursorIcon::Crosshair
        }
        // Photoshop's Move tool keeps the standard arrow pointer (#5646): the movable
        // layer under it is marked by the hover outline, not by a different cursor.
        Tool::Move => egui::CursorIcon::Default,
        // Dragging is handled with the pan itself, above.
        Tool::Hand => crate::tool_cursor::hand(ui.ctx(), false),
        Tool::RotateView => crate::rotate_view::cursor(response.dragged()),
        Tool::Zoom => crate::tool_cursor::zoom(ui.ctx(), zoom_out(alt)),
        Tool::Type | Tool::VerticalType if type_path.is_some() => match type_path {
            Some(crate::type_tool::PathClick::Area { .. }) => crate::tool_cursor::area_type(painter, p),
            _ => crate::tool_cursor::path_type(painter, p),
        },
        Tool::Type | Tool::VerticalType | Tool::HorizontalTypeMask | Tool::VerticalTypeMask => egui::CursorIcon::Text,
        Tool::MagneticLasso => crate::magnetic_lasso_ui::cursor(app, painter, p, xf.zoom),
        Tool::RedEye => {
            let pupil = app.ui.tool_options.red_eye_pupil_size.clamp(1.0, 100.0);
            let r_doc = photocraft_algo::redeye::search_radius(pupil);
            let r = (r_doc * xf.zoom).max(2.0);
            painter.circle_stroke(p, r + 0.5, Stroke::new(1.0, Color32::from_black_alpha(140)));
            painter.circle_stroke(p, r, Stroke::new(1.0, Color32::from_white_alpha(220)));
            for (w, c) in [(2.5, Color32::from_black_alpha(140)), (1.0, Color32::from_white_alpha(220))] {
                painter.line_segment([p - vec2(3.0, 0.0), p + vec2(3.0, 0.0)], Stroke::new(w, c));
                painter.line_segment([p - vec2(0.0, 3.0), p + vec2(0.0, 3.0)], Stroke::new(w, c));
            }
            egui::CursorIcon::None
        }
        _ => egui::CursorIcon::Crosshair,
    };
    let icon = visible_crosshair(icon, painter, p, cfg!(not(target_arch = "wasm32")));
    ui.ctx().set_cursor_icon(icon);
    // Selection tools: + / − / × badge for the effective mode (#170). A gesture keeps the
    // mode it started with (⇧ then constrains the marquee instead of adding).
    let held = crate::workspace_ui::sticky_mods(app, ui.input(|i| i.modifiers));
    let mods = app.drag.as_ref().map_or(held, |d| d.modifiers);
    if let Some(b) = crate::tool_feedback::badge(app, tool, mods) {
        crate::tool_feedback::draw_badge(painter, p, b, tool == Tool::QuickSelection);
    }
    // ⇧ after a stroke: the straight line a click would paint (#257).
    crate::stroke_constraint::draw_line_preview(app, painter, xf, p, tool, held.shift);
}

struct CanvasTailInput<'a> {
    ctx: &'a egui::Context,
    painter: &'a egui::Painter,
    response: &'a egui::Response,
    doc: &'a std::sync::Arc<Document>,
    xf: ViewXform,
    rect: Rect,
    full: Rect,
    idx: usize,
    primary: bool,
    tool: Tool,
    middle: bool,
    flip: bool,
    aspect: f32,
    bars_id: egui::Id,
    layer_edges: egui::layers::ShapeIdx,
    camera: Option<([f32; 2], f32)>,
}

fn finish_canvas_view(app: &mut PhotocraftApp, ui: &mut egui::Ui, view: &mut View, input: &CanvasTailInput<'_>) {
    let ctx = input.ctx;
    let painter = input.painter;
    let response = input.response;
    let doc = input.doc;
    let xf = input.xf;
    let rect = input.rect;
    let full = input.full;
    let idx = input.idx;
    let primary = input.primary;
    let tool = input.tool;
    let middle = input.middle;
    let flip = input.flip;
    let aspect = input.aspect;
    let bars_id = input.bars_id;
    let layer_edges = input.layer_edges;
    let camera = input.camera;
    app.tool_override = None;
    if let Some(b) = layer_edges_rect(app, idx) {
        painter.set(layer_edges, egui::Shape::rect_stroke(xf.doc_rect(b), 0, Stroke::new(1.0, LAYER_EDGES), egui::StrokeKind::Outside));
    }
    // A tool drag held past the canvas edge scrolls the view toward the pointer (#4183); the
    // drag's next move then extends into what scrolled in.
    if primary
        && app.drag.is_some()
        && tool != Tool::Hand
        && !middle
        && let Some(p) = ui.input(|i| i.pointer.primary_down().then(|| i.pointer.latest_pos()).flatten())
    {
        let step = crate::canvas_autoscroll::edge_step(p, rect, ui.input(|i| i.stable_dt));
        if crate::canvas_autoscroll::scroll(view, step, xf) {
            ctx.request_repaint();
        }
    }
    // Scrollbars (scrollbars.rs): drawn over the canvas edges, they take the pointer there.
    let t0 = crate::gpu_canvas::now_ms();
    let before = view.center;
    let over_bars = app.ui.view.shows_scrollbars()
        && crate::scrollbars::show(ui, rect, view, flip, aspect, app.session.prefs().tools.overscroll, egui::Id::unique(("pc-canvas", idx)));
    ctx.data_mut(|d| d.insert_temp(bars_id, over_bars));
    if view.center != before {
        ctx.request_repaint();
    }
    if primary {
        app.perf.span("scrollbars", crate::gpu_canvas::now_ms() - t0);
        app.hover_doc = response.hover_pos().map(|p| xf.to_doc(p));
    }
    if primary && app.ui.extras.rulers {
        let before = app.guide_drag;
        crate::rulers::draw_rulers(app, ui, full, &xf);
        // A guide dragged out of a ruler moved after this frame drew it: draw it again where it is.
        if app.guide_drag != before {
            ctx.request_repaint();
        }
    } else if app.ui.extras.rulers {
        crate::rulers::paint_rulers(app, ui, full, &xf, doc);
    }
    // After the ruler drags above, so the readout shows where the guide is now, and not after a
    // release ended the drag.
    if app.session.active_index() == Some(idx) {
        crate::rulers::draw_guide_drag_readout(app, ctx);
    }
    crate::rotate_view::overlay(app, ctx, painter, &xf, view, response);
    if primary {
        // Tool input can change the stored camera while navigation edits the local view.
        // Merge changed camera values so write-back preserves tool-driven pan and rotation.
        if let (Some((center, rotation)), Some(now)) = (camera, app.ui.views.get(idx)) {
            if now.center != center {
                view.center = now.center;
                ctx.request_repaint();
            }
            if now.rotation != rotation {
                view.rotation = now.rotation;
                ctx.request_repaint();
            }
        }
        app.ui.views[idx] = (*view).clone();
    }
}

/// A Properties Levels/Curves eyedropper (layer_picker.rs) owns the image until Esc or another
/// tool; Space and the middle button still pan. It samples on the press.
fn layer_picker_eyedropper(app: &mut PhotocraftApp, ui: &egui::Ui, ctx: &egui::Context, response: &egui::Response, xf: &ViewXform) {
    if response.hovered()
        && let Some(p) = ui.input(|i| i.pointer.hover_pos())
    {
        if app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise {
            ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        } else {
            ctx.set_cursor_icon(pipette_cursor(ctx, p));
        }
    }
    if response.is_pointer_button_down_on()
        && ui.input(|i| i.pointer.primary_pressed())
        && let Some(p) = ui.input(|i| i.pointer.interact_pos())
    {
        let d = xf.to_doc(p);
        crate::layer_picker::sample_at(app, d[0], d[1]);
    }
}

/// Per-frame reconciliation of the camera with its document: resize re-centre, pending fit/fill,
/// and the overscroll clamp. Returns the pixel aspect the view is drawn with.
fn sync_view_to_document(app: &PhotocraftApp, ctx: &egui::Context, view: &mut View, doc: &Document, rect: Rect, ppp: f32) -> f32 {
    let size = [doc.size.width, doc.size.height];
    if view.doc_size != size {
        // Like Photoshop: keep the zoom level, re-centre the resized document.
        if view.doc_size != [0, 0] {
            view.center = [size[0] as f32 / 2.0, size[1] as f32 / 2.0];
        }
        view.doc_size = size;
    }
    // View › Pixel Aspect Ratio Correction: document pixels are shown this much wider.
    let aspect = app.ui.view.display_aspect();
    if view.fit_pending && rect.width() > 50.0 {
        fit_view(view, doc, rect.size(), ppp, aspect);
    }
    if view.fill_pending && rect.width() > 50.0 {
        fill_view(view, doc, rect.size(), ppp, aspect);
    }
    // Preferences › Tools › Overscroll off: clamp before anything is drawn (scrollbars.rs).
    if !app.session.prefs().tools.overscroll && crate::scrollbars::clamp_view_with_aspect(view, rect.size(), ppp, aspect) {
        view.zoom_anchor = None;
        ctx.request_repaint();
    }
    aspect
}

fn update_polygon_camera(app: &mut PhotocraftApp, ctx: &egui::Context, (idx, primary): (usize, bool), rect: Rect, view: &mut View, ppp: f32, aspect: f32) {
    let previous_center = view.center;
    if primary {
        polygon_cancel_stale(app);
        polygon_edge_scroll(app, ctx, idx, rect, view, ppp);
    }
    // Bounds belong to an actual edge-scroll update, not an idle/finished polygon.
    if view.center != previous_center && !app.session.prefs().tools.overscroll {
        crate::scrollbars::clamp_view_with_aspect(view, rect.size(), ppp, aspect);
    }
    if view.center != previous_center {
        ctx.request_repaint();
    }
}

pub fn canvas_view(app: &mut PhotocraftApp, ui: &mut egui::Ui, idx: usize, rect: Rect, mut view: View, primary: bool) -> View {
    let ctx = ui.ctx().clone();
    // The camera as the app holds it before this frame's input. The canvas works on its own copy
    // (`view`) and writes it back at the end; a tool that moves the camera itself meanwhile (the
    // Crop tool's default mode pans and turns the view with the image, `crop_mode`) must not be
    // undone by that write-back.
    let camera = if primary { app.ui.views.get(idx).map(|v| (v.center, v.rotation)) } else { None };
    // `View::zoom` is device pixels per document pixel; this canvas's geometry is in its own
    // viewport's egui points. A document window can sit on another display with a different
    // scale, so only the primary canvas records `app.ppp` for the frame's shared helpers
    // (`point_zoom`, the navigator, control-channel coordinate mapping). `pixels_per_point`
    // folds in the display scale and Interface › UI Scale alike, so dividing by it keeps 100%
    // one document pixel per physical display pixel at any UI scale (#2121).
    let ppp = ctx.pixels_per_point();
    if primary {
        app.ppp = ppp;
    }
    // The display this window is on: its monitor profile (#569). A document window on a display
    // the last reading didn't know asks for a new one.
    let output = crate::monitor_status::view_display(app, &ctx);
    if !primary {
        crate::monitor_status::check_window(app, &ctx);
    }
    let full = rect;
    // Every document view has its rulers, the tiles of Window › Arrange and floating windows
    // too, as in Photoshop (#6017).
    let rect = crate::rulers::content_rect(app, rect);
    // Both wheel reads share one owner so snap-zoom input is accumulated only once per frame.
    let wheel_owner = ui.scope_id().with(("canvas-wheel", idx));
    // Direct navigation starts from the authoritative camera. This also means a click or drag
    // never maps through a camera that is still moving under the pointer.
    let (pointer_here, hover_here) = ctx.input(|i| {
        (
            (i.pointer.any_pressed() || i.pointer.any_down()) && i.pointer.press_origin().is_some_and(|p| rect.contains(p)),
            i.pointer.hover_pos().is_some_and(|p| rect.contains(p)),
        )
    });
    let direct_input = pointer_here
        || (hover_here
            && crate::wheel_nav::read_with(
                &ctx,
                app.session.prefs().general.zoom_with_scroll_wheel,
                app.session.prefs().enhanced_controls.zoom_with_trackpad_pinch,
                app.session.prefs().general.animated_zoom,
                wheel_owner,
                hover_here,
                app.session.prefs().interface.enable_animations,
                app.session.prefs().enhanced_controls.touch_gestures,
            )
            .is_some());
    if direct_input {
        view.zoom_animating = false;
        view.zoom_anchor = None;
    }
    let doc = app.session.documents()[idx].doc.clone();
    let size = [doc.size.width, doc.size.height];
    let aspect = sync_view_to_document(app, &ctx, &mut view, &doc, rect, ppp);
    update_polygon_camera(app, &ctx, (idx, primary), rect, &mut view, ppp, aspect);

    let flip = app.ui.view.flip_horizontal;
    // Document rendering follows the target; only shader uniforms and overlay geometry ease.
    let target_xf = ViewXform { rect, zoom: view.zoom / ppp, center: view.center, flip, rotation: view.rotation, aspect };
    let render_zoom = view.zoom;
    let mut display_view = animated_view(&mut view, target_xf, ppp, app.session.prefs().general.animated_zoom, ctx.input(|i| i.time));
    if primary && let Some(stored) = app.ui.views.get_mut(idx) {
        stored.zoom_animating = view.zoom_animating;
        stored.display_camera = view.display_camera;
    }
    if view.zoom_animating {
        ctx.request_repaint();
    }
    let xf = ViewXform { zoom: display_view.zoom / ppp, center: display_view.center, ..target_xf }.pixel_aligned_for(app, ppp);
    let pixel_grid = app.ui.view.shows(app.ui.view.show.pixel_grid);
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let stroke_dismissal = primary && crate::shape_stroke_ui::dismiss_from_canvas(app, &response);
    let painter = ui.painter_at(rect);

    match crate::prefs_ui::pasteboard_color(app) {
        Some(c) => {
            ui.painter_at(rect).rect_filled(rect, 0.0, c);
        }
        None => paint_dots(ui, rect, app.session.prefs().interface.show_canvas_dots),
    }
    let border = app.session.prefs().interface.canvas_border;
    // Default-mode Crop changes its camera during input. Defer its image until the new camera
    // is known; other tools keep their existing paint order and texture/outline snapshots.
    let crop_camera = primary && crate::crop_mode::moves_image(app);
    let paint_input = CanvasPaintInput {
        ctx: &ctx,
        rect,
        painter: &painter,
        ppp,
        border,
        doc: &doc,
        idx,
        output,
        size,
        primary,
        flip,
        aspect,
        render_zoom,
        target_xf,
        pixel_grid,
    };
    let painted = if crop_camera { None } else { Some(paint_canvas(app, ui, &display_view, xf, &paint_input)) };
    let img_rect = xf.doc_rect(doc.bounds());

    // Under an open dialog the canvas widget is inert, but the image still pans and zooms.
    let under_dialog = !app.ui.dialogs.is_empty();
    let free_hover = under_dialog && crate::dialogs::free_pointer_over(&ctx, rect).is_some();
    // A press on the canvas while a tool letter is held makes that tool temporary (#2672).
    if !under_dialog && ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| rect.contains(p))) {
        crate::spring_tools::note_canvas_press(app);
    }
    let bars_id = egui::Id::unique(("pc-canvas-bars-hover", idx));
    update_canvas_navigation(app, &mut view, CanvasNavigationInput { ctx: &ctx, response: &response, free_hover, wheel_owner, xf, ppp, idx });

    // Pen pressure/tilt for this frame's tool events (mouse = 1.0), unless Preferences › Tools ›
    // Use Tablet Pressure is off; the pen's eraser end selects the Eraser.
    app.stylus.use_pressure = app.session.prefs().tools.use_tablet_pressure;
    let curve = app.session.prefs().tools.pressure_curve.clone();
    app.stylus.set_pressure_curve(&curve);
    app.stylus.update_for_frame(ui.ctx().cumulative_frame_nr(), &ui.input(|i| i.events.clone()));
    crate::stylus::Stylus::sync_eraser_tool(app);
    // A Magnetic Lasso border left behind by a tool or document switch is dropped.
    crate::magnetic_lasso_ui::frame(app);
    // A Properties Levels/Curves eyedropper left behind by a tool or layer switch is dropped.
    crate::layer_picker::prune(app);
    // Held keys (hold_keys.rs): Space repositions a crop frame, marquee, lasso or shape being
    // drawn; otherwise Space is the Hand and ⌘Space / ⌥Space the Zoom tool while held.
    let reposition = crate::hold_keys::reposition_held(app, &ctx);
    crate::crop_ui::set_space(app, reposition);
    // ⌘ (Ctrl) held: a temporary Straighten with the Crop tool.
    crate::crop_straighten::set_held(app, !ctx.text_edit_focused() && ui.input(|i| i.modifiers.command));
    // A tab switched this frame drops the other document's pending crop (#1918).
    crate::crop_ui::cancel_stale(app);
    crate::crop_ui::ensure_frame(app);
    let mut drawing = crate::crop_ui::active(app);
    if let Some(pen) = app.ui.pen.as_mut().filter(|pen| pen.dragging && !pen.curvature) {
        pen.reposition = reposition;
        drawing = true;
    }
    if let Some(d) = app.drag.as_mut().filter(|d| crate::hold_keys::repositions(d.tool)) {
        d.reposition = reposition;
        drawing = true;
    }
    let temporary = crate::hold_keys::for_frame(app, &ctx, drawing || crate::type_transform::active(app));
    let space_pan = temporary == Some(crate::hold_keys::Temporary::Hand);
    let middle = ui.input(|i| i.pointer.middle_down());
    let tool = match temporary {
        Some(t) => t.tool(),
        None if middle => Tool::Hand,
        None => app.ui.tool,
    };
    // ⌘ held: this frame's gestures go to the Move tool (`PhotocraftApp::active_tool`). The Hand
    // and Zoom never reach `tool_event`, so only the Move needs the state machine to know.
    app.tool_override = (temporary == Some(crate::hold_keys::Temporary::Move)).then_some(Tool::Move);
    // `~` held: a Brush or Pencil stroke that starts now erases (`tool_event`'s `Down`).
    app.held_erase = crate::hold_keys::erase_held(app, &ctx);
    // Zoom direction: the temporary zoom key decides, else ⌥ (Zoom tool), held or latched in
    // Window › Modifier Keys (the way around a desktop that takes Alt+click, `alt_grab`).
    let sticky_alt = app.ui.shell.sticky_alt;
    let zoom_out = |alt: bool| match temporary {
        Some(crate::hold_keys::Temporary::ZoomOut) => true,
        Some(crate::hold_keys::Temporary::ZoomIn) => false,
        _ => alt || sticky_alt,
    };

    if under_dialog {
        handle_dialog_canvas_input(
            app,
            DialogCanvasInput {
                ctx: &ctx,
                view: &mut view,
                xf: &xf,
                rect,
                painter: &painter,
                ppp,
                primary,
                middle,
                space_pan,
                temporary,
                free_hover,
                img_rect,
            },
        );
    }

    // The selected Hand yields to an open Free Transform box in this (primary) window: the box
    // owns the pointer, so the tool behind it cannot steal the resize drag (#2302). The held Hand
    // (Space) and a middle-button drag still pan, and a secondary window's Hand never stops, as the
    // box is only drawn and driven on the primary canvas.
    let transform_owns_pointer = primary && app.ui.transform.is_some() && temporary.is_none() && !middle;
    let hand_pan = tool == Tool::Hand && !transform_owns_pointer;
    // Preferences › Tools › Enable Flick Panning: a Hand drag let go while moving glides on.
    let flick_id = egui::Id::unique(("pc-flick-pan", idx, primary, ctx.viewport_id()));
    let flick_on = app.session.prefs().tools.enable_flick_panning;
    if let Some(d) = crate::flick_pan::frame(&ctx, flick_id, hand_pan && response.dragged() && !stroke_dismissal, response.drag_delta(), flick_on) {
        let d = xf.unmap_vec(d) / (view.zoom / ppp);
        if d.is_finite() {
            view.zoom_animating = false;
            view.zoom_anchor = None;
            view.center[0] -= d.x;
            view.center[1] -= d.y;
        }
        // Overscroll off: the glide stops at the pan limit, as the scrollbars do.
        if !app.session.prefs().tools.overscroll && crate::scrollbars::clamp_view_with_aspect(&mut view, rect.size(), ppp, aspect) {
            ctx.data_mut(|m| m.get_temp_mut_or_default::<crate::flick_pan::Flick>(flick_id).stop());
        }
    }
    if hand_pan && response.dragged() && !stroke_dismissal {
        let d = xf.unmap_vec(response.drag_delta()) / (view.zoom / ppp);
        view.center[0] -= d.x;
        view.center[1] -= d.y;
        // The tool cursors below are skipped while panning: the fist is set here (#2196).
        ctx.set_cursor_icon(crate::tool_cursor::hand(&ctx, true));
    } else if primary && !stroke_dismissal && temporary.is_none() && !middle && crate::layer_picker::armed(app).is_some() {
        layer_picker_eyedropper(app, ui, &ctx, &response, &xf);
    } else if primary && !stroke_dismissal {
        let input = CanvasToolInput {
            ctx: &ctx,
            response: &response,
            xf: &xf,
            rect,
            full,
            tool,
            idx,
            primary,
            transform_owns_pointer,
            hand_pan,
            ppp,
            under_dialog,
            zoom_out: &zoom_out,
            display_zoom: display_view.zoom,
        };
        handle_primary_canvas_input(app, ui, &mut view, &input);
    }
    if crop_camera && let (Some((center, rotation)), Some(now)) = (camera, app.ui.views.get(idx)) {
        if now.center != center {
            view.center = now.center;
            ctx.request_repaint();
        }
        if now.rotation != rotation {
            view.rotation = now.rotation;
            ctx.request_repaint();
        }
    }
    let xf = if crop_camera {
        if view.center != target_xf.center {
            display_view.center = view.center;
        }
        display_view.rotation = view.rotation;
        ViewXform { center: display_view.center, rotation: display_view.rotation, ..xf }.pixel_aligned_for(app, ppp)
    } else {
        xf
    };
    let (beyond, layer_edges, edge_rect, ants) = painted.unwrap_or_else(|| paint_canvas(app, ui, &display_view, xf, &paint_input));
    // A Hand drag (the tool, Space or the middle button) keeps every overlay on screen, as in
    // Photoshop: the Pen path being drawn, paths, guides, grid, transform box (#6592). Only the
    // tool cursors below give way to the fist.
    let panning = hand_pan && response.dragged();
    if primary {
        let input = CanvasOverlayInput {
            ctx: &ctx,
            painter: &painter,
            xf: &xf,
            response: &response,
            doc: &doc,
            tool,
            idx,
            output,
            border,
            beyond,
            edge_rect,
            panning,
            zoom_out: &zoom_out,
        };
        paint_primary_overlays(app, ui, &mut view, &input, ants);
    } else {
        painter.extend(ants);
    }
    let tail = CanvasTailInput {
        ctx: &ctx,
        painter: &painter,
        response: &response,
        doc: &doc,
        xf,
        rect,
        full,
        idx,
        primary,
        tool,
        middle,
        flip,
        aspect,
        bars_id,
        layer_edges,
        camera,
    };
    finish_canvas_view(app, ui, &mut view, &tail);
    view
}

#[cfg(test)]
mod transform_outline_clip_tests {
    use super::*;

    #[test]
    fn near_horizon_lines_keep_viewport_precision() {
        let r = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 600.0));
        for (a, b, start, end) in [
            (pos2(-1e9, 100.0), pos2(1e9, 100.0), pos2(0.0, 100.0), pos2(800.0, 100.0)),
            (pos2(100.0, -1e9), pos2(100.0, 1e9), pos2(100.0, 0.0), pos2(100.0, 600.0)),
            (pos2(-1e9, -1e9), pos2(1e9, 1e9), pos2(0.0, 0.0), pos2(600.0, 600.0)),
        ] {
            let (x, y) = clip_line(r, a, b).unwrap();
            assert!(x.distance(start) < 0.01 && y.distance(end) < 0.01, "{x:?}–{y:?}, expected {start:?}–{end:?}");
        }
        assert!(clip_line(r, pos2(-1e9, -100.0), pos2(1e9, -100.0)).is_none());
    }
}

#[cfg(test)]
mod tap_pressure_tests {
    #[test]
    fn fallback_click_pressure_matches_pen_or_mouse() {
        let mut s = crate::stylus::Stylus::default();
        assert_eq!(s.pressure(), 1.0, "mouse clicks paint at full pressure");
        s.feed.set(Some(crate::stylus::PenSample { pressure: 0.18, ..Default::default() }));
        assert_eq!(s.pressure(), 0.18, "a tap without a canvas drag still has pen pressure");
        s.use_pressure = false;
        assert_eq!(s.pressure(), 1.0, "Use Tablet Pressure off stays full pressure");
    }
}

#[cfg(test)]
mod tabs_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod transform_controls_tests;

#[cfg(test)]
mod layer_edges_tests;

#[cfg(test)]
mod guide_ants_tests;

#[cfg(test)]
mod rotation_preview_isolation_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn channel_overlay_shows_filter_preview_and_restores_on_cancel() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        app.run("file.new", json!({"width":16,"height":16})).unwrap();
        app.run("channel.new", json!({"name":"Alpha","fill":"black"})).unwrap();
        app.session
            .edit("Fixture", |doc, _| {
                for y in 0..16 {
                    for x in 0..8 {
                        doc.channels[0].surface.write_pixel(x, y, &[1.0]);
                    }
                }
                Ok(())
            })
            .unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let ctx = egui::Context::default();
        let texture = crate::channel_view::ensure(&mut app, &ctx, 0).unwrap();
        ctx.tex_manager().write().take_delta().clear();
        let dialog = crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
        app.ui.dialogs.last_mut().unwrap().fields.insert("radius".into(), json!(2.0));
        ensure_filter_preview(&mut app, 0, &ctx).unwrap();
        crate::channel_view::ensure(&mut app, &ctx, 0);
        let mut delta = ctx.tex_manager().write().take_delta();
        let egui::ImageData::Color(image) = &delta.set.get(&texture).expect("filter preview must update channel overlay")[0].image;
        let st = app.session.active().unwrap();
        let original =
            crate::channel_view::render(&before, &st.channel_view, before.bounds(), 1, crate::channel_view::Look::default(), st.active_layer).unwrap();
        assert_ne!(image.pixels, original, "a nontrivial filter must be visible before OK");
        delta.clear();
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        app.ui.dialogs.last_mut().unwrap().fields.insert("__preview".into(), json!(false));
        crate::channel_view::ensure(&mut app, &ctx, 0);
        let mut delta = ctx.tex_manager().write().take_delta();
        let egui::ImageData::Color(image) = &delta.set.get(&texture).expect("Preview off restores channel")[0].image;
        assert_eq!(image.pixels, original);
        delta.clear();
        app.ui.dialogs.last_mut().unwrap().fields.insert("__preview".into(), json!(true));
        crate::channel_view::ensure(&mut app, &ctx, 0);
        ctx.tex_manager().write().take_delta().clear();
        crate::dialogs::cancel(&mut app, dialog).unwrap();
        crate::channel_view::ensure(&mut app, &ctx, 0);
        let mut delta = ctx.tex_manager().write().take_delta();
        let egui::ImageData::Color(image) = &delta.set.get(&texture).expect("Cancel restores channel")[0].image;
        assert_eq!(image.pixels, original);
        delta.clear();
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    }

    fn assert_filter_preview_target(target: photocraft_engine::channel_cmds::ChannelTarget) {
        for depth in [8, 16, 32] {
            let mut session = photocraft_engine::Session::new();
            session.execute("file.new", json!({"width":16,"height":16,"depth":depth})).unwrap();
            session.execute("channel.new", json!({"name":"Alpha","fill":"black"})).unwrap();
            session
                .edit("Fixture", |doc, active| {
                    let layer = doc.layer_mut(active.unwrap()).unwrap();
                    layer.mask = Some(photocraft_doc::LayerMask::hide_all());
                    for y in 0..16 {
                        for x in 0..8 {
                            layer.surface_mut().unwrap().write_pixel(x, y, &[0.0, 0.25, 0.5, 1.0]);
                            layer.mask.as_mut().unwrap().surface.write_pixel(x, y, &[1.0]);
                        }
                    }
                    for y in 0..16 {
                        for x in 0..8 {
                            doc.channels[0].surface.write_pixel(x, y, &[1.0]);
                        }
                    }
                    Ok(())
                })
                .unwrap();
            session.active_mut().unwrap().channel_view.target = target;
            let mut app = PhotocraftApp::new(session, crate::Services::default());
            app.background_jobs = false;
            let dialog = crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
            app.ui.dialogs.last_mut().unwrap().fields.insert("radius".into(), json!(2.0));
            ensure_filter_preview(&mut app, 0, &egui::Context::default()).unwrap();
            let preview = app.filter_preview.as_ref().unwrap().result.as_ref().unwrap().clone();
            crate::dialogs::confirm(&mut app, dialog).unwrap();
            let final_doc = &app.session.active().unwrap().doc;
            let bounds = final_doc.bounds();
            assert_eq!(
                preview.layers[0].surface().unwrap().read_region(bounds),
                final_doc.layers[0].surface().unwrap().read_region(bounds),
                "RGB: {depth} {target:?}"
            );
            assert_eq!(preview.channels[0].surface.read_region(bounds), final_doc.channels[0].surface.read_region(bounds), "alpha: {depth} {target:?}");
            assert_eq!(
                preview.layers[0].mask.as_ref().unwrap().surface.read_region(bounds),
                final_doc.layers[0].mask.as_ref().unwrap().surface.read_region(bounds),
                "mask: {depth} {target:?}"
            );
        }
    }

    #[test]
    fn filter_preview_matches_applied_alpha_target() {
        assert_filter_preview_target(photocraft_engine::channel_cmds::ChannelTarget::Alpha(0));
    }

    #[test]
    fn filter_preview_matches_applied_color_target() {
        assert_filter_preview_target(photocraft_engine::channel_cmds::ChannelTarget::Color(0));
    }

    #[test]
    fn filter_preview_matches_applied_layer_mask_target() {
        assert_filter_preview_target(photocraft_engine::channel_cmds::ChannelTarget::LayerMask);
    }

    #[test]
    fn filter_preview_refreshes_when_channel_target_changes() {
        use photocraft_engine::channel_cmds::ChannelTarget;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = false;
        app.run("file.new", json!({"width":16,"height":16})).unwrap();
        for name in ["First", "Second"] {
            app.run("channel.new", json!({"name":name,"fill":"black"})).unwrap();
        }
        app.session
            .edit("Fixture", |doc, _| {
                for y in 0..16 {
                    for x in 0..8 {
                        doc.channels[1].surface.write_pixel(x, y, &[1.0]);
                    }
                }
                Ok(())
            })
            .unwrap();
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(0);
        let revision = app.session.active().unwrap().revision;
        let dialog = crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
        app.ui.dialogs.last_mut().unwrap().fields.insert("radius".into(), json!(2.0));
        let ctx = egui::Context::default();
        let (first, first_key) = filter_channel_preview(&mut app, 0, &ctx).unwrap();
        app.session.active_mut().unwrap().channel_view.target = ChannelTarget::Alpha(1);
        let (second, second_key) = filter_channel_preview(&mut app, 0, &ctx).unwrap();
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_ne!(first_key, second_key);
        assert_ne!(first.channels[1].surface.read_region(first.bounds()), second.channels[1].surface.read_region(second.bounds()));
        let (unchanged, unchanged_key) = filter_channel_preview(&mut app, 0, &ctx).unwrap();
        assert!(std::sync::Arc::ptr_eq(&second, &unchanged));
        assert_eq!(second_key, unchanged_key);
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        let applied = &app.session.active().unwrap().doc;
        assert_eq!(second.channels[1].surface.read_region(second.bounds()), applied.channels[1].surface.read_region(applied.bounds()));
    }

    #[test]
    fn filter_preview_overlay_matches_quick_mask_and_mixed_color_views() {
        for quick_mask in [false, true] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.background_jobs = false;
            app.run("file.new", json!({"width":16,"height":16})).unwrap();
            app.session
                .edit("Fixture", |doc, active| {
                    if quick_mask {
                        doc.quick_mask =
                            Some(photocraft_doc::AlphaChannel::new("Quick Mask", photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8)));
                    }
                    for y in 0..16 {
                        for x in 0..8 {
                            if let Some(mask) = &mut doc.quick_mask {
                                mask.surface.write_pixel(x, y, &[1.0]);
                            } else {
                                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().write_pixel(x, y, &[0.0, 0.25, 0.5, 1.0]);
                            }
                        }
                    }
                    Ok(())
                })
                .unwrap();
            if !quick_mask {
                let view = &mut app.session.active_mut().unwrap().channel_view;
                view.target = photocraft_engine::channel_cmds::ChannelTarget::Color(0);
                view.set_color_visible(2, false);
            }
            let ctx = egui::Context::default();
            let texture = crate::channel_view::ensure(&mut app, &ctx, 0).unwrap();
            ctx.tex_manager().write().take_delta().clear();
            let dialog = crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
            app.ui.dialogs.last_mut().unwrap().fields.insert("radius".into(), json!(2.0));
            crate::channel_view::ensure(&mut app, &ctx, 0).unwrap();
            let mut delta = ctx.tex_manager().write().take_delta();
            let egui::ImageData::Color(image) = &delta.set.get(&texture).expect("preview must refresh overlay")[0].image;
            crate::dialogs::confirm(&mut app, dialog).unwrap();
            let st = app.session.active().unwrap();
            let expected =
                crate::channel_view::render(&st.doc, &st.channel_view, st.doc.bounds(), 1, crate::channel_view::Look::default(), st.active_layer).unwrap();
            assert_eq!(image.pixels, expected, "quick_mask={quick_mask}");
            delta.clear();
        }
    }

    /// Assign and Convert to Profile have Photoshop's Preview. Assign changes only the profile, so
    /// its preview must be shown through the profile it assigns (through the document's own it
    /// looked unchanged); Convert keeps the look of in-gamut colours. Their `working` is the
    /// working space from Color Settings, not the built-in default.
    #[test]
    fn profile_dialogs_preview_through_the_profile_they_choose() {
        let mut session = photocraft_engine::Session::new();
        session.color.settings.monitor_profile = "srgb".into();
        session.color.settings.working_rgb = "display-p3".into();
        session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        session.execute("edit.assignProfile", json!({"profile": "srgb"})).unwrap();
        session
            .edit("Fixture", |doc, _| {
                let layer = doc.layers.last_mut().unwrap();
                for y in 0..4 {
                    for x in 0..4 {
                        layer.surface_mut().unwrap().write_pixel(x, y, &[0.6, 0.35, 0.25, 1.0]);
                    }
                }
                Ok(())
            })
            .unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        let shown_pixels = |app: &PhotocraftApp, doc: &Document| {
            let (display, _) = canvas_display(app, doc, None);
            display_image(display.as_deref(), &photocraft_compose::flatten(doc)).pixels[0]
        };
        let original = app.session.active().unwrap().doc.clone();
        let before = shown_pixels(&app, &original);
        let preview = |app: &mut PhotocraftApp, command: &str, profile: &str| {
            crate::filter_dialog::open(app, command).unwrap();
            let fields = &mut app.ui.dialogs.last_mut().unwrap().fields;
            assert_eq!(fields.get("__preview"), Some(&json!(true)), "{command} has a Preview checkbox, on");
            fields.insert("profile".into(), json!(profile));
            ensure_filter_preview(app, 0, &egui::Context::default()).expect("a preview");
            let result = app.filter_preview.as_ref().unwrap().result.clone().unwrap();
            let key = original.id.0 ^ (1u64 << 61);
            assert_eq!(filter_preview_display_doc(app, &original, key).icc_profile, result.icc_profile, "the canvas shows it through its own profile");
            assert_eq!(app.session.active().unwrap().doc.icc_profile, original.icc_profile, "previewing changes nothing");
            app.ui.dialogs.clear();
            app.filter_preview = None;
            result
        };
        let description = |doc: &Document| photocraft_engine::color_cmds::profile_from_bytes(doc.icc_profile.as_ref().unwrap()).unwrap().description.clone();

        let assigned = preview(&mut app, "edit.assignProfile", "prophoto-compat");
        assert_eq!(description(&assigned), photocraft_cms::Builtin::ProPhotoCompat.description());
        let shown = shown_pixels(&app, &assigned);
        assert!(shown.r().abs_diff(before.r()) > 10 || shown.b().abs_diff(before.b()) > 10, "the assigned profile shows: {before:?} → {shown:?}");

        let converted = preview(&mut app, "edit.convertToProfile", "working");
        assert_eq!(description(&converted), photocraft_cms::Builtin::DisplayP3.description(), "`working` is Color Settings' working space");
        let shown = shown_pixels(&app, &converted);
        for (a, b) in [(shown.r(), before.r()), (shown.g(), before.g()), (shown.b(), before.b())] {
            assert!(a.abs_diff(b) <= 2, "converting keeps the look: {before:?} → {shown:?}");
        }
    }

    /// A CPU-only app must be able to generate the actual rotation preview without a
    /// GPU adapter; checking the result also verifies Preview off and document integrity.
    #[test]
    fn rotation_preview_is_computed_headlessly_without_mutating_document() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 96, "height": 48})).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        assert!(app.gpu.is_none());
        crate::filter_dialog::open(&mut app, "image.rotation.arbitrary").unwrap();
        let dialog = app.ui.dialogs.last_mut().unwrap();
        dialog.fields.insert("angle".into(), json!(90.0));
        dialog.fields.insert("direction".into(), json!("cw"));

        let (factor, _, size) = ensure_filter_preview(&mut app, 0, &egui::Context::default()).expect("CPU preview computed");
        assert_eq!(factor, 1);
        assert_eq!(size, [48, 96]);
        let original = &app.session.active().unwrap().doc;
        assert_eq!([original.size.width, original.size.height], [96, 48]);

        app.ui.dialogs.last_mut().unwrap().fields.insert("__preview".into(), json!(false));
        assert!(ensure_filter_preview(&mut app, 0, &egui::Context::default()).is_none(), "disabling Preview must hide the preview");
        assert_eq!([app.session.active().unwrap().doc.size.width, app.session.active().unwrap().doc.size.height], [96, 48]);
    }

    /// A dialog operates on the active document and cannot render a preview into a
    /// background tab, even if that tab has a valid canvas index.
    #[test]
    fn filter_preview_is_not_returned_for_inactive_document() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 96, "height": 48})).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        crate::filter_dialog::open(&mut app, "image.rotation.arbitrary").unwrap();
        app.ui.dialogs.last_mut().unwrap().fields.insert("angle".into(), json!(90.0));
        assert!(ensure_filter_preview(&mut app, 0, &egui::Context::default()).is_some());

        app.session.execute("file.new", json!({"width": 40, "height": 20})).unwrap();
        assert_eq!(app.session.active_index(), Some(1));
        assert!(ensure_filter_preview(&mut app, 0, &egui::Context::default()).is_none(), "an inactive tab must never inherit the command preview");
    }
}

#[cfg(test)]
mod zoom_animation_tests {
    use super::*;

    /// Screen geometry shared by the zoom-transition regressions.
    fn camera(view: &View) -> ViewXform {
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0)), zoom: view.zoom, center: view.center, flip: false, rotation: 0.0, aspect: 1.0 }
    }

    /// A settled camera whose transition state starts empty.
    fn view() -> View {
        View { zoom: 1.0, center: [200.0, 150.0], doc_size: [400, 300], fit_pending: false, ..Default::default() }
    }

    /// Deferred uploads prioritize both visible areas throughout a recentering zoom.
    #[test]
    fn deferred_redraw_focus_covers_target_and_displayed_cameras() {
        for rotation in [0.0, 37.0] {
            let mut v = view();
            let mut initial = camera(&v);
            initial.rotation = rotation;
            animated_view(&mut v, initial, 1.0, true, 0.0);
            v.zoom = 2.0;
            v.center = [1600.0, 900.0];
            v.request_zoom_animation(true);
            let target = ViewXform { zoom: v.zoom, center: v.center, ..initial };
            let target_visible = visible_doc_rect(&target);
            for time in [0.0, 0.11, 0.22] {
                let shown = animated_view(&mut v, target, 1.0, true, time);
                let displayed = ViewXform { zoom: shown.zoom, center: shown.center, ..target };
                let displayed_visible = visible_doc_rect(&displayed);
                let focus = redraw_focus(&target, &displayed);
                assert_eq!(focus.intersect(&target_visible), target_visible);
                assert_eq!(focus.intersect(&displayed_visible), displayed_visible);
                if time == 0.0 {
                    assert_ne!(focus, target_visible, "the old view must also receive priority");
                }
            }
            assert_eq!(redraw_focus(&target, &target), target_visible, "settled views keep the same focus");
        }
    }

    #[test]
    fn disabling_animated_zoom_snaps_a_secondary_canvas_on_its_next_frame() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.sync_views();
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let mut secondary = view();
        let frame = |app: &mut PhotocraftApp, view: View, time: f64| {
            let input = egui::RawInput { screen_rect: Some(rect), time: Some(time), ..Default::default() };
            let mut shown = view.clone();
            let mut out = ctx.run_ui(input, |ui| {
                shown = canvas_view(app, ui, 0, rect, view.clone(), false);
            });
            out.textures_delta.clear();
            shown
        };
        secondary = frame(&mut app, secondary, 0.0);
        secondary.zoom = 2.0;
        secondary.request_zoom_animation(true);
        secondary = frame(&mut app, secondary, 0.01);
        secondary = frame(&mut app, secondary, 0.12);
        assert!(secondary.zoom_animating);
        assert!(secondary.display_camera.unwrap().0 < secondary.zoom);
        app.run("prefs.set", serde_json::json!({"values": {"general.animatedZoom": false}})).unwrap();
        secondary = frame(&mut app, secondary, 0.13);
        assert_eq!(secondary.display_camera, Some((secondary.zoom, secondary.center)));
        assert_eq!(secondary.zoom, 2.0);
        assert!(!secondary.zoom_animating && secondary.zoom_transition.is_none() && secondary.zoom_anchor.is_none());
    }

    #[test]
    fn animated_zoom_off_cancels_each_views_presentation_camera() {
        for target in [(2.0, [210.0, 170.0]), (3.0, [250.0, 180.0])] {
            let mut v = view();
            let xf = camera(&v);
            animated_view(&mut v, xf, 1.0, true, 0.0);
            v.zoom = target.0;
            v.center = target.1;
            v.request_zoom_animation(true);
            animated_view(&mut v, xf, 1.0, true, 0.0);
            let halfway = animated_view(&mut v, xf, 1.0, true, 0.11);
            assert_ne!((halfway.zoom, halfway.center), target);
            v.zoom_anchor = Some(([200.0, 150.0], [500.0, 350.0]));
            let shown = animated_view(&mut v, xf, 1.0, false, 0.11);
            assert_eq!((shown.zoom, shown.center), target);
            assert_eq!(v.display_camera, Some(target));
            assert!(!v.zoom_animating && v.zoom_anchor.is_none() && v.zoom_transition.is_none());
        }
    }

    #[test]
    fn zoom_animation_obeys_clamped_targets_and_eases_recentering() {
        let mut v = view();
        let xf = camera(&v);
        animated_view(&mut v, xf, 1.0, true, 0.0);
        let pointer = xf.to_screen(390.0, 150.0);
        zoom_about(&mut v, &xf, pointer, 2.0, false, 1.0);
        v.zoom_anchor = Some(([390.0, 150.0], [pointer.x, pointer.y]));
        v.zoom_animating = true;
        crate::scrollbars::clamp_view(&mut v, xf.rect.size(), 1.0);
        let xf = camera(&v);
        animated_view(&mut v, xf, 1.0, true, 0.0);
        assert!(v.zoom_anchor.is_none());
        let done = animated_view(&mut v, xf, 1.0, true, 0.22);
        assert_eq!((done.zoom, done.center), (v.zoom, v.center));
        assert!(!v.zoom_animating);

        let mut v = view();
        let xf = camera(&v);
        animated_view(&mut v, xf, 1.0, true, 0.0);
        zoom_about(&mut v, &xf, pointer, 2.0, true, 1.0);
        v.zoom_animating = true;
        let target = v.center;
        let xf = camera(&v);
        let start = animated_view(&mut v, xf, 1.0, true, 0.0);
        assert_eq!(start.center, [200.0, 150.0], "recentering must not teleport");
        let halfway = animated_view(&mut v, xf, 1.0, true, 0.11);
        assert!(halfway.center[0] > start.center[0] && halfway.center[0] < target[0]);
        assert_eq!(animated_view(&mut v, xf, 1.0, true, 0.22).center, target);
    }

    #[test]
    fn zoom_animation_views_are_independent_and_direct_navigation_snaps() {
        let mut a = view();
        let mut b = view();
        let xf = camera(&a);
        animated_view(&mut a, xf, 1.0, true, 0.0);
        animated_view(&mut b, xf, 1.0, true, 0.0);
        a.zoom = 2.0;
        a.zoom_animating = true;
        animated_view(&mut a, xf, 1.0, true, 0.0);
        b.zoom = 3.0;
        b.zoom_animating = true;
        animated_view(&mut b, xf, 1.0, true, 0.11);
        assert_eq!(animated_view(&mut a, xf, 1.0, true, 0.22).zoom, 2.0);
        let shown = animated_view(&mut b, xf, 1.0, true, 0.22);
        assert!(shown.zoom > 1.0 && shown.zoom < 3.0);
        assert!(b.zoom_animating && !a.zoom_animating);
        b.zoom_animating = false;
        b.center = [220.0, 170.0];
        let shown = animated_view(&mut b, xf, 1.0, true, 0.23);
        assert_eq!((shown.zoom, shown.center), (b.zoom, b.center));
        assert!(b.zoom_transition.is_none() && b.zoom_anchor.is_none());
    }
}
#[cfg(test)]
mod display_memo_tests;

#[cfg(test)]
mod cpu_background_tests;

#[cfg(test)]
#[path = "canvas/blend_viewport_tests.rs"]
mod blend_viewport_regression;

#[cfg(test)]
#[path = "canvas/pixel_alignment_tests.rs"]
mod pixel_alignment_tests;
#[cfg(test)]
mod lasso_preview_tests {
    use super::*;

    fn xf() -> ViewXform {
        ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 }
    }

    /// What `draw` paints: (black dashes, black solid paths).
    fn painted(app: &mut PhotocraftApp, draw: impl Fn(&mut PhotocraftApp, &egui::Painter)) -> (usize, usize) {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| draw(app, ui.painter()));
        output.textures_delta.clear();
        let mut flat = Vec::new();
        for clipped in output.shapes {
            let mut stack = vec![clipped.shape];
            while let Some(shape) = stack.pop() {
                match shape {
                    egui::Shape::Vec(v) => stack.extend(v),
                    s => flat.push(s),
                }
            }
        }
        let dashes = flat.iter().filter(|s| matches!(s, egui::Shape::LineSegment { stroke, .. } if stroke.color == Color32::BLACK)).count();
        let solid = flat.iter().filter(|s| matches!(s, egui::Shape::Path(p) if p.stroke.color == egui::epaint::ColorMode::Solid(Color32::BLACK))).count();
        (dashes, solid)
    }

    fn document() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.sync_views();
        app
    }

    /// #7495: as in Photoshop, the Lasso's, Polygonal Lasso's and Magnetic Lasso's outline is a
    /// solid line until the selection is made; a Patch area dragged out keeps its marching ants.
    #[test]
    fn lasso_outlines_are_solid_until_the_selection_is_made() {
        let points = vec![[20.0, 20.0, 1.0], [120.0, 30.0, 1.0], [150.0, 140.0, 1.0], [40.0, 160.0, 1.0]];
        for tool in [Tool::Lasso, Tool::Patch] {
            let mut app = document();
            app.ui.tool = tool;
            app.drag = Some(Drag::new(tool, [20.0, 20.0], points.clone(), egui::Modifiers::NONE, false));
            let (dashes, solid) = painted(&mut app, |app, p| draw_drag_preview(app, p, &xf()));
            if tool == Tool::Lasso {
                assert_eq!((dashes, solid), (0, 1), "{tool:?}: one solid outline, no ants");
            } else {
                assert!(dashes > 10 && solid == 0, "{tool:?}: still marching ants ({dashes} dashes)");
            }
        }

        let mut app = document();
        app.ui.tool = Tool::PolygonLasso;
        app.ui.polygon = vec![[20.0, 20.0], [120.0, 30.0], [150.0, 140.0]];
        let (dashes, solid) = painted(&mut app, |app, p| draw_tool_state(app, p, &xf(), Some(pos2(60.0, 150.0))));
        assert_eq!((dashes, solid), (0, 1), "Polygonal Lasso: its sides and rubber band are one solid line");

        let mut app = document();
        app.ui.tool = Tool::MagneticLasso;
        app.ui.magnetic.path = vec![[20.0, 20.0], [80.0, 25.0], [120.0, 60.0]];
        app.ui.magnetic.anchors = vec![0, 2];
        app.ui.magnetic.live = vec![[120.0, 60.0], [130.0, 100.0]];
        let (dashes, solid) = painted(&mut app, |app, p| draw_tool_state(app, p, &xf(), None));
        assert_eq!((dashes, solid), (0, 1), "Magnetic Lasso: the traced border is solid");
    }
}

#[cfg(test)]
mod multi_channel_ui_tests {
    use super::*;
    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width":64,"height":64})).unwrap();
        app.sync_views();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        app
    }
    #[test]
    fn pixel_tools_capture_the_whole_channel_set_and_gradients_stay_on_pixels() {
        let mut app = app();
        assert_eq!(paint_target(&app), json!({"colors":[0,2]}));
        app.ui.tool = Tool::Gradient;
        app.ui.tool_options.gradient_classic = false;
        assert!(!crate::gradient_ui::live_mode(&app));
    }
    #[test]
    fn viewing_a_layer_mask_overrides_colour_targets_for_live_brush_strokes() {
        for depth in [8, 16, 32] {
            for view in ["gray", "overlay"] {
                for colors in [vec![0], vec![0, 2]] {
                    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                    app.run("file.new", json!({"width":64,"height":64,"depth":depth})).unwrap();
                    app.run("layer.layerMask.revealAll", json!({})).unwrap();
                    app.run("channel.target", json!({"colors":colors})).unwrap();
                    app.run("view.layerMask", json!({"mode":view})).unwrap();
                    assert_eq!(paint_target(&app), json!("mask"), "{depth}-bit {view} {colors:?}");
                    assert!(app.layer_mask_targeted());
                    app.run("tools.setColors", json!({"foreground":[0.0,0.0,0.0,1.0]})).unwrap();
                    app.run("tools.setBrush", json!({"brush":{"size":12,"hardness":1.0,"smoothing":{"amount":0.0}}})).unwrap();
                    app.ui.tool = Tool::Brush;
                    let st = app.session.active().unwrap();
                    let layer = st.active_layer.unwrap();
                    let original = st.doc.layer(layer).unwrap().surface().unwrap().clone();
                    let steps = st.history.past_len();
                    tool_event(&mut app, ToolEvent::Down { x: 12.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
                    tool_event(&mut app, ToolEvent::Move { x: 32.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
                    let shown = display_doc(&mut app, 0).0;
                    let shown_layer = shown.layer(layer).unwrap();
                    assert_eq!(shown_layer.surface().unwrap(), &original);
                    assert!(shown_layer.mask.as_ref().unwrap().surface.sample_channel(28, 32, 0) < 0.1);
                    tool_event(&mut app, ToolEvent::Up { x: 32.0, y: 32.0 }, egui::Modifiers::NONE);
                    let st = app.session.active().unwrap();
                    let committed = st.doc.layer(layer).unwrap();
                    assert_eq!(committed.surface().unwrap(), &original);
                    assert_eq!(committed.mask.as_ref().unwrap().surface, shown_layer.mask.as_ref().unwrap().surface);
                    assert_eq!(st.history.past_len(), steps + 1);
                    app.run("channel.target", json!({"colors":colors})).unwrap();
                    assert_eq!(paint_target(&app), json!({"colors":colors}));
                    assert!(!app.layer_mask_targeted());
                }
            }
        }
    }
    /// The colour rows stay selected when another layer becomes active. An adjustment layer has no
    /// pixels, so the Brush paints its mask, as it does without colour rows selected.
    #[test]
    fn colour_rows_left_selected_still_paint_an_adjustment_layers_mask() {
        for colors in [vec![0], vec![0, 2]] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width":64,"height":64})).unwrap();
            app.run("channel.target", json!({"colors":colors})).unwrap();
            app.run("layer.newAdjustmentLayer.levels", json!({})).unwrap();
            app.sync_views();
            assert_eq!(paint_target(&app), json!("mask"), "{colors:?}");
            app.run("tools.setColors", json!({"foreground":[0.0,0.0,0.0,1.0]})).unwrap();
            app.run("tools.setBrush", json!({"brush":{"size":12,"hardness":1.0,"smoothing":{"amount":0.0}}})).unwrap();
            app.ui.tool = Tool::Brush;
            let st = app.session.active().unwrap();
            let layer = st.active_layer.unwrap();
            let steps = st.history.past_len();
            tool_event(&mut app, ToolEvent::Down { x: 12.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
            tool_event(&mut app, ToolEvent::Move { x: 32.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
            tool_event(&mut app, ToolEvent::Up { x: 32.0, y: 32.0 }, egui::Modifiers::NONE);
            let st = app.session.active().unwrap();
            assert_eq!(st.history.past_len(), steps + 1, "{colors:?}: one stroke");
            assert!(st.doc.layer(layer).unwrap().mask.as_ref().unwrap().surface.sample_channel(28, 32, 0) < 0.1, "{colors:?}: the mask was painted");
        }
    }

    /// Targeting a layer mask with colour rows selected (its thumbnail, or adding the mask) moves
    /// the Channels panel onto the mask, as in Photoshop: tools, dialogs and menu commands then
    /// all edit the mask. Several rows still stop the Move tool on a layer that needs
    /// rasterizing, which paints only after the prompt.
    #[test]
    fn targeting_a_mask_over_colour_rows_edits_the_mask_everywhere() {
        let ctx = egui::Context::default();
        let set = |app: &mut PhotocraftApp, p: serde_json::Value| {
            let (req, _) = crate::control::ControlRequest::new("ui.set", p);
            assert!(matches!(crate::control::handle(app, &ctx, &req), crate::control::Outcome::Done(v) if v["ok"] != false));
        };
        for colors in [vec![0], vec![0, 2]] {
            for add in [false, true] {
                let mut app = self::app();
                if !add {
                    app.run("layer.layerMask.revealAll", json!({})).unwrap();
                    set(&mut app, json!({"maskTarget": false}));
                }
                app.run("channel.target", json!({"colors":colors})).unwrap();
                app.sync_views();
                assert!(paint_target(&app).get("colors").is_some(), "{colors:?}: the rows are the target first");
                let journal = app.session.journal.len();
                if add {
                    app.run("layer.layerMask.revealAll", json!({})).unwrap();
                } else {
                    set(&mut app, json!({"maskTarget": true}));
                }
                app.sync_views();
                assert_eq!(app.session.journal.len(), journal + usize::from(add), "{colors:?} add={add}: no extra recorded step");
                assert_eq!(paint_target(&app), json!("mask"), "{colors:?} add={add}");
                assert!(app.layer_mask_targeted(), "{colors:?} add={add}");
                assert_eq!(app.with_mask_target("filter.blur.gaussianBlur", json!({}))["target"], "mask");
                assert!(crate::distort_ui::colour_set_target(&app).is_null());
                assert!(!multiple_color_target(&app));
                // Clicking a colour row again takes the rows back.
                app.run("channel.target", json!({"colors":colors})).unwrap();
                app.sync_views();
                assert!(paint_target(&app).get("colors").is_some(), "{colors:?} add={add}: the rows again");
            }
        }
        // A mask added to another layer, or a fill layer's mask, leaves the rows targeted.
        let mut app = self::app();
        let active = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let other = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.select", json!({"layer": active.0})).unwrap();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        app.run("layer.layerMask.revealAll", json!({"layer": other.0})).unwrap();
        app.sync_views();
        assert_eq!(paint_target(&app), json!({"colors":[0,2]}), "a mask on another layer");
        app.run("layer.newFillLayer.solidColor", json!({})).unwrap();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        set(&mut app, json!({"maskTarget": true}));
        app.sync_views();
        assert!(matches!(app.session.active().unwrap().channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Color(_)), "a fill layer");
        let mut app = self::app();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        app.sync_views();
        assert_eq!(paint_target(&app), json!("pixels"));
        assert!(crate::rasterize_prompt::needed(&app).is_some());
        assert!(multiple_color_target(&app));
        let refused = crate::liquify_ui::open(&mut app, &ctx).unwrap_err();
        assert!(refused.contains("Rasterize"), "{refused}");
    }

    #[test]
    fn subset_does_not_block_guides_or_vector_path_transforms() {
        let mut app = app();
        app.ui.tool = Tool::Move;
        app.run("view.newGuide", json!({"orientation":"vertical","position":12})).unwrap();
        tool_event(&mut app, ToolEvent::Down { x: 12.0, y: 12.0, pressure: 1.0 }, egui::Modifiers::default());
        assert!(app.guide_drag.is_some());
        app.guide_drag = None;
        app.run("path.set", json!({"name":"work","path":{"subpaths":[{"closed":true,"knots":[[10,10],[30,10],[30,20],[10,20]]}]}})).unwrap();
        app.ui.tool = Tool::PathSelection;
        assert!(crate::menus::is_enabled(&app, "edit.freeTransform"));
        crate::transform_tool::begin(&mut app, &egui::Context::default()).unwrap();
        assert!(app.ui.transform.as_ref().unwrap().path.is_some());
    }
}

pub(crate) use drag::cancel_stale_vector_drag;

#[cfg(test)]
#[path = "pen_gesture_owner_tests.rs"]
mod pen_gesture_owner_tests;
#[cfg(test)]
#[path = "pen_path_visual_tests.rs"]
mod pen_path_visual_tests;

/// Between Polygonal Lasso clicks, reveal more of the image near or beyond the viewport edge.
/// Only the view moves: fixed vertices stay in document coordinates, including while button-up.
fn polygon_edge_scroll(app: &PhotocraftApp, ctx: &egui::Context, idx: usize, rect: Rect, view: &mut View, ppp: f32) {
    if app.ui.polygon.is_empty()
        || app.ui.tool != Tool::PolygonLasso
        || app.session.active_index() != Some(idx)
        || !app.ui.dialogs.is_empty()
        || app.camera_raw.is_some()
        || app.discard.is_some()
        || app.drag.is_some()
        || !crate::menus::modal_allows(app, "select.lasso")
        || app.ui.transform.is_some()
        || crate::distort_ui::modal_workspace(app).is_some()
        || ctx.text_edit_focused()
        || egui::Popup::is_any_open(ctx)
        || crate::hold_keys::for_frame(app, ctx, false).is_some()
    {
        return;
    }
    let Some((pointer, dt)) = ctx.input(|i| (i.focused && !i.pointer.middle_down()).then(|| i.pointer.hover_pos().map(|p| (p, i.stable_dt))).flatten()) else {
        return;
    };
    let zoom = view.zoom / ppp;
    if !pointer.x.is_finite() || !pointer.y.is_finite() || !dt.is_finite() || dt <= 0.0 || !zoom.is_finite() || zoom <= 0.0 {
        return;
    }
    // Moving over a neighbouring document is not a request to pan either tiled view.
    if arranged_cells(app, app.last_canvas_rect).is_some_and(|cells| cells.iter().any(|(other, cell)| *other != idx && cell.contains(pointer))) {
        return;
    }
    let edge = 32.0_f32.min(rect.width() * 0.25).min(rect.height() * 0.25);
    if edge <= 0.0 {
        return;
    }
    let speed = |p: f32, lo: f32, hi: f32| {
        let direction = ((edge - (hi - p)) / edge).clamp(0.0, 1.0) - ((edge - (p - lo)) / edge).clamp(0.0, 1.0);
        if direction == 0.0 { 0.0 } else { direction.signum() * (80.0 + 520.0 * direction.abs()) }
    };
    let velocity = vec2(speed(pointer.x, rect.left(), rect.right()), speed(pointer.y, rect.top(), rect.bottom()));
    let velocity = velocity * (600.0 / velocity.length().max(600.0));
    let xf = ViewXform { rect, zoom, center: view.center, flip: app.ui.view.flip_horizontal, rotation: view.rotation, aspect: app.ui.view.display_aspect() };
    let delta = xf.unmap_vec(velocity) * dt.min(0.05) / zoom;
    view.center[0] += delta.x;
    view.center[1] += delta.y;
}
