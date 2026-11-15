use super::display::{canvas_display, display_image};
use super::drag::Drag;
use super::paint_thread::{PaintThread, Step};
use super::textures::{canvas_texture_options, shown_as_document};
use super::view_xform::ViewXform;
use super::{Color32, DRect, PhotocraftApp, Pos2, Stroke, Tool, json, pos2};

pub(crate) struct ClonePreviewCache {
    pub(super) key: String,
    // Reopening a file can retain its document id and revision; distinguish its new snapshot.
    // A weak reference detects that replacement without keeping a closed image's pixels alive.
    source_document: Option<std::sync::Weak<photocraft_doc::Document>>,
    /// The brush a clipped overlay took its coverage from: a hardness or tip change redraws it.
    pub(super) brush: Option<photocraft_paint::BrushSettings>,
    /// The document area the texture covers.
    pub(super) rect: DRect,
    pub(super) texture: egui::TextureHandle,
}

/// The most document pixels one dab's coverage is rendered over for the overlay: a 1000 px brush,
/// the largest the overlay draws, at hardness 0 reaches about 1.73 × its radius (`tip_reach`).
const CLONE_PREVIEW_MAX_PIXELS: u64 = 1800 * 1800;

/// The largest side `retouch_cmds::clone_preview_periodic` samples, for the overlay's area and for
/// the source area it reads (`Mapping::source_rect`: the overlay's, mapped, with a 2 px apron).
const CLONE_PREVIEW_MAX_SIDE: u32 = 1024;

/// The coverage of one dab of `brush` at `at`, including flow, tip, angle, roundness and
/// hardness (a soft tip reaches past its radius). Stroke opacity is applied when drawing. The area
/// it covers and its `width × height` coverage values in 0..1. A very large soft tip, or a source
/// the Clone Source panel scales down (which reads a larger area than it covers), is cropped
/// around `at` to the sampling budget, keeping the middle of the dab.
fn clone_dab_coverage(
    brush: &photocraft_paint::BrushSettings,
    at: [f64; 2],
    map: &photocraft_engine::presets::clone_source::Mapping,
    flow: f32,
) -> Option<(DRect, Vec<f32>)> {
    let brush = photocraft_paint::BrushSettings { opacity: 1.0, flow, erase: false, ..brush.clone() };
    let stroke = photocraft_paint::Stroke { brush, points: vec![photocraft_paint::StrokePoint::new(at[0], at[1], 1.0)] };
    let (rect, cov) = photocraft_paint::retouch::stroke_coverage_within(&stroke, CLONE_PREVIEW_MAX_PIXELS)?;
    let fits = |r: DRect| r.width() <= CLONE_PREVIEW_MAX_SIDE && r.height() <= CLONE_PREVIEW_MAX_SIDE;
    let (cx, cy) = (at[0].floor() as i32, at[1].floor() as i32);
    let around = |half: i32| rect.intersect(&DRect::new(cx.saturating_sub(half), cy.saturating_sub(half), cx.saturating_add(half), cy.saturating_add(half)));
    // The source area scales with the overlay's: shrink by what overflows, a few times for the
    // apron's rounding.
    let mut half = i32::try_from((CLONE_PREVIEW_MAX_SIDE - 4) / 2).unwrap_or(510);
    let mut window = around(half);
    for _ in 0..8 {
        let src = map.source_rect(window);
        if fits(window) && fits(src) {
            break;
        }
        let over = f64::from(src.width().max(src.height())) / f64::from(CLONE_PREVIEW_MAX_SIDE - 4);
        half = ((f64::from(half) / over.max(1.0)).floor() as i32).saturating_sub(2).max(1);
        window = around(half);
    }
    if window.is_empty() || !fits(window) || !fits(map.source_rect(window)) {
        return None;
    }
    if window == rect {
        return Some((rect, cov));
    }
    let stride = rect.width() as usize;
    let mut cropped = Vec::with_capacity(window.width() as usize * window.height() as usize);
    for y in window.y0..window.y1 {
        let row = (y - rect.y0) as usize * stride;
        let from = row + (window.x0 - rect.x0) as usize;
        cropped.extend_from_slice(cov.get(from..from + window.width() as usize)?);
    }
    Some((window, cropped))
}

/// A transient source image beneath the brush outline; never part of the document render. The
/// Clone Source panel drives the Clone Stamp and the Healing Brush alike, so both show it.
pub(super) fn draw_clone_preview(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, at: [f64; 2], output: Option<u32>) {
    use photocraft_engine::presets::clone_source::overlay_blend;
    let overlay = app.session.presets.clone.overlay.clone();
    let tool = app.ui.tool;
    if !matches!(tool, Tool::CloneStamp | Tool::Healing) || !overlay.show || (overlay.auto_hide && app.drag.is_some()) {
        return;
    }
    let slot = app.session.presets.clone.active().clone();
    let Some(map) = crate::preset_panels::clone_mapping_at(app, Some(at)) else { return };
    let radius = f64::from(app.session.tools.brush.size) / 2.0;
    if !radius.is_finite() || radius <= 0.0 || radius > 500.0 || !at.iter().all(|v| v.is_finite() && v.abs() < 1_000_000.0) {
        return;
    }
    // Both tools repeat source reads in Pattern Preview; this also refreshes a stationary hover.
    let wrap = app.ui.view.pattern_preview;
    let Some(st) = app.session.active() else { return };
    let target = paint_target(app);
    if slot.document.is_some_and(|id| id != st.doc.id) && photocraft_engine::channel_cmds::gray_target(&json!({"target": target})).is_some() {
        // Strokes reject these cross-document targets, including a mask selected only in the UI.
        return;
    }
    let source_revision =
        slot.document.and_then(|id| app.session.documents().iter().find(|st| st.doc.id == id)).map(|st| (st.doc.id, st.revision, st.active_layer));
    let source_document = slot.document.and_then(|id| app.session.documents().iter().find(|st| st.doc.id == id)).map(|st| std::sync::Arc::downgrade(&st.doc));
    let (display, display_key) = canvas_display(app, &st.doc, output);
    let key = format!(
        "{tool:?}:{source_revision:?}:{:?}:{}:{:?}:{:?}:{map:?}:{:?}:{}:{}:{}:{}:{wrap}:{}:{}",
        st.doc.id,
        st.revision,
        st.active_layer,
        at.map(f64::to_bits),
        slot.layer,
        app.ui.tool_options.clone_sample,
        display_key,
        overlay.invert,
        overlay.blend,
        overlay.clipped,
        radius.to_bits()
    );
    let brush = overlay.clipped.then_some(&app.session.tools.brush);
    if app.clone_preview.as_ref().is_none_or(|c| {
        let same_source = match (&c.source_document, &source_document) {
            (Some(a), Some(b)) => std::sync::Weak::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        c.key != key || c.brush.as_ref() != brush || !same_source
    }) {
        // Clipped (Photoshop's default): the dab the tool paints here, so a soft brush previews
        // soft and a shaped tip keeps its outline. Unclipped: the brush's whole square.
        let (rect, coverage) = match brush {
            Some(b) => {
                let flow = if tool == Tool::CloneStamp { b.flow } else { 1.0 };
                let Some((rect, coverage)) = clone_dab_coverage(b, at, &map, flow) else { return };
                (rect, Some(coverage))
            }
            None => (
                DRect::new((at[0] - radius).floor() as i32, (at[1] - radius).floor() as i32, (at[0] + radius).ceil() as i32, (at[1] + radius).ceil() as i32),
                None,
            ),
        };
        let preview = if tool == Tool::Healing {
            photocraft_engine::retouch_cmds::healing_preview_periodic(&app.session, rect, &map, &app.ui.tool_options.clone_sample, wrap)
        } else {
            photocraft_engine::retouch_cmds::clone_preview_periodic(&app.session, rect, &map, &app.ui.tool_options.clone_sample, wrap)
        };
        let Ok(mut buf) = preview else {
            return;
        };
        if overlay.invert {
            for px in &mut buf.px {
                for c in px.iter_mut().take(3) {
                    *c = 1.0 - *c;
                }
            }
        }
        // Darken, Lighten and Difference mix the source with the image under the brush (the
        // composite, whatever Sample reads), so Difference turns black where they line up.
        if overlay.blend != "normal" {
            let Ok(under) = photocraft_engine::retouch_cmds::clone_destination_preview(&app.session, rect, wrap) else {
                return;
            };
            for (px, d) in buf.px.iter_mut().zip(&under.px) {
                *px = overlay_blend(&overlay.blend, *px, *d);
            }
        }
        if let Some(coverage) = &coverage {
            for (px, c) in buf.px.iter_mut().zip(coverage) {
                px[3] *= c.clamp(0.0, 1.0);
            }
        }
        let image = display_image(display.as_deref(), &buf);
        let brush = brush.cloned();
        match app.clone_preview.as_mut() {
            Some(cache) => {
                cache.texture.set(image, canvas_texture_options(1));
                cache.key = key;
                cache.source_document = source_document;
                cache.brush = brush;
                cache.rect = rect;
            }
            None => {
                let texture = painter.ctx().load_texture("clone-preview", image, canvas_texture_options(1));
                app.clone_preview = Some(ClonePreviewCache { key, source_document, brush, rect, texture });
            }
        }
    }
    let Some(cache) = &app.clone_preview else { return };
    let rect = cache.rect;
    // Clone Stamp sends its opacity/flow with every stroke; Healing Brush uses full strength.
    let brush_alpha = if tool == Tool::CloneStamp {
        app.session.tools.brush.opacity.clamp(0.0, 1.0) * if overlay.clipped { 1.0 } else { app.session.tools.brush.flow.clamp(0.0, 1.0) }
    } else {
        1.0
    };
    let tint = Color32::from_white_alpha((overlay.opacity.clamp(0.0, 100.0) * brush_alpha * 2.55).round() as u8);
    // A textured quad over the covered area: its texels carry the clip, in mirrored and rotated
    // canvas views too.
    let mut mesh = egui::Mesh::with_texture(cache.texture.id());
    for (x, y, u, v) in [(rect.x0, rect.y0, 0.0, 0.0), (rect.x1, rect.y0, 1.0, 0.0), (rect.x1, rect.y1, 1.0, 1.0), (rect.x0, rect.y1, 0.0, 1.0)] {
        mesh.vertices.push(egui::epaint::Vertex { pos: xf.to_screen(x as f32, y as f32), uv: pos2(u, v), color: tint });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// The engine's live stroke of a tool: the Brush, Pencil and Eraser paint dabs; the Clone Stamp,
/// Healing Brush and History Brush composite their source.
pub(super) enum EngineStroke {
    Brush(Box<photocraft_engine::brush_cmds::LiveStroke>),
    Retouch(Box<photocraft_engine::retouch_cmds::LiveRetouch>),
    Dab(Box<photocraft_engine::retouch_cmds::LiveDab>),
    Mixer(Box<photocraft_engine::brush_cmds::mixer_live::LiveMixer>),
}

impl EngineStroke {
    /// The brush engine's strokes can change while the pointer is held still (see
    /// `LiveStroke::wants_time`); the retouch strokes don't.
    pub(super) fn wants_time(&self) -> bool {
        match self {
            EngineStroke::Brush(l) => l.wants_time(),
            EngineStroke::Mixer(l) => l.wants_time(),
            EngineStroke::Retouch(_) | EngineStroke::Dab(_) => false,
        }
    }

    pub(super) fn doc(&self) -> &std::sync::Arc<photocraft_doc::Document> {
        match self {
            Self::Brush(s) => &s.doc,
            Self::Retouch(s) => &s.doc,
            Self::Dab(s) => &s.doc,
            Self::Mixer(s) => &s.doc,
        }
    }
    fn bounds(&self) -> DRect {
        match self {
            Self::Brush(s) => s.bounds(),
            Self::Retouch(s) => s.bounds(),
            Self::Dab(s) => s.bounds(),
            Self::Mixer(s) => s.bounds(),
        }
    }
    pub(super) fn push(&mut self, pts: &[photocraft_engine::paint::StrokePoint]) -> photocraft_engine::Result<DRect> {
        match self {
            Self::Brush(s) => s.push(pts),
            Self::Retouch(s) => s.push(pts),
            Self::Dab(s) => s.push(pts),
            Self::Mixer(s) => s.push(pts),
        }
    }
    /// The jitter seed the commit must reuse (the Clone Stamp's comes from the session brush).
    fn seed(&self) -> Option<u64> {
        match self {
            Self::Brush(s) => Some(s.seed),
            Self::Mixer(s) => Some(s.seed),
            Self::Retouch(_) => None,
            Self::Dab(_) => None,
        }
    }
}

/// A stroke shown while it is drawn: the engine renders the real result onto a copy of the
/// document, on a thread of its own (`PaintThread`), and the canvas redraws only what each step
/// changed.
pub(crate) struct LiveStroke {
    pub(super) preview_failed: bool,
    pub(super) paint: PaintThread,
    /// The jitter seed the commit must reuse (`EngineStroke::seed`).
    pub(super) seed: Option<u64>,
    pub(super) doc: photocraft_doc::DocId,
    revision: u64,
    /// Preview key of the stroke; step `n` displays as `key + n`.
    pub(super) key: u64,
    /// Damage of each step (step 0 = the document before the stroke).
    pub(super) damage: Vec<DRect>,
    /// Drag points rendered so far.
    fed: usize,
}

impl LiveStroke {
    /// The document as the stroke last painted it.
    pub(super) fn doc(&self) -> &std::sync::Arc<photocraft_doc::Document> {
        &self.paint.shown().doc
    }

    /// See `EngineStroke::wants_time`.
    pub(super) fn wants_time(&self) -> bool {
        self.paint.shown().wants_time
    }

    /// See `photocraft_engine::brush_cmds::LiveStroke::smoothing_string`.
    pub(super) fn smoothing_string(&self) -> Option<([f64; 2], [f64; 2])> {
        self.paint.shown().smoothing_string
    }

    pub(super) fn display_key(&self) -> u64 {
        self.key + self.damage.len() as u64
    }

    /// What changed since the canvas showed preview `key` (0 = the document before the stroke).
    pub(super) fn since(&self, key: u64) -> Option<DRect> {
        let seen = if key == 0 { 0 } else { usize::try_from(key.checked_sub(self.key)?).ok()? };
        Some(self.damage.get(seen..)?.iter().fold(DRect::EMPTY, |a, r| a.union(r)))
    }
}

/// A live stroke whose pixels its commit took over (`"fromLive"`): document `doc` at `revision`
/// holds exactly the live stroke's last step, so a cache that showed step `n` of it only misses
/// the steps after `n` (step 0 = the document before the stroke: everything).
pub(crate) struct CommittedStroke {
    pub(crate) doc: photocraft_doc::DocId,
    pub(crate) revision: u64,
    key: u64,
    damage: Vec<DRect>,
    /// What the commit repaints beyond the stroke whatever a cache showed: the layer's whole
    /// composite when effects laid out over its frame moved with it (a Gradient Overlay spans the
    /// grown layer, #4053; `photocraft_compose::relayout_damage`).
    pub(super) relayout: DRect,
}

impl CommittedStroke {
    #[cfg(test)]
    pub(crate) fn for_test(key: u64, damage: Vec<DRect>) -> Self {
        Self { doc: photocraft_doc::DocId(0), revision: 0, key, damage, relayout: DRect::EMPTY }
    }

    /// The raw preview key of the live stroke's last step: what a cache showed at release.
    #[cfg(test)]
    pub(crate) fn last_key(&self) -> u64 {
        self.key + self.damage.len() as u64
    }

    /// What a cache that showed raw preview `key` of this stroke at the revision before misses.
    pub(crate) fn since(&self, key: u64) -> Option<DRect> {
        let seen = if key == 0 { 0 } else { usize::try_from(key.checked_sub(self.key)?).ok()? };
        Some(self.damage.get(seen..)?.iter().fold(DRect::EMPTY, |a, r| a.union(r)))
    }
}

/// The live stroke on document `idx`, while it is current.
pub(super) fn live_stroke(app: &PhotocraftApp, idx: usize) -> Option<&LiveStroke> {
    let st = app.session.documents().get(idx)?;
    app.live_stroke.as_ref().filter(|l| app.drag.is_some() && l.doc == st.doc.id && l.revision == st.revision)
}

/// `paint.stroke` params for a Brush/Eraser drag (shared by the live preview and the commit). The
/// stroke smoothing is the session brush's (the options bar's Smoothing %), except on a ⇧-click
/// line, which isn't smoothed (#11887).
pub(super) fn stroke_params(app: &PhotocraftApp, d: &Drag, points: &[Vec<f64>]) -> serde_json::Value {
    let tool = d.tool;
    let mut p = json!({ "points": points, "erase": d.erase, "zoom": app.point_zoom(), "target": paint_target(app) });
    unsmoothed_line(&mut p, d.line);
    if d.clear {
        p["mode"] = json!("clear");
    }
    if tool == Tool::Pencil {
        p["autoErase"] = json!(app.ui.tool_options.pencil_auto_erase);
    }
    // Block mode: the engine sizes the square from the stroke's zoom (16 / zoom px, #2770).
    if crate::eraser_ui::block_mode(app, tool) {
        p["block"] = json!(true);
    }
    p
}

/// A ⇧-click line isn't smoothed, so it ends at the click whatever Catch-up on Stroke End is set
/// to (#11887). Applied to the params of every painting tool's live preview and commit alike, so
/// a retouching stroke's commit still takes the pixels its preview drew.
pub(crate) fn unsmoothed_line(p: &mut serde_json::Value, line: bool) {
    if line && let Some(o) = p.as_object_mut() {
        o.insert("smoothing".into(), json!(0.0));
    }
}

/// Tools whose strokes the engine renders while they are drawn (`LiveStroke`).
pub(crate) fn strokes_live(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::Brush
            | Tool::Pencil
            | Tool::MixerBrush
            | Tool::Eraser
            | Tool::ColorReplacement
            | Tool::Blur
            | Tool::Sharpen
            | Tool::Smudge
            | Tool::Dodge
            | Tool::Burn
            | Tool::Sponge
    ) || live_retouch_command(tool).is_some()
}

/// The Spot Healing Brush's stroke while dragging: Photoshop shows the area it will heal as a
/// semi-opaque dark overlay (an image overlay, the same in every UI theme) and heals on release.
pub(crate) const SPOT_HEALING_TRAIL: Color32 = Color32::from_black_alpha(128);

/// The retouching tools drawn live (`LiveRetouch`; the Background Eraser's and an Impressionist
/// Pattern Stamp's dabs are laid one by one, `LiveDab`), by the command their live stroke
/// previews. The Healing Brush shows its texture while drawing; the blend comes on release.
pub(super) fn live_retouch_command(tool: Tool) -> Option<&'static str> {
    match tool {
        Tool::CloneStamp => Some("paint.cloneStamp"),
        Tool::Healing => Some("paint.healingBrush"),
        Tool::HistoryBrush => Some("paint.historyBrush"),
        Tool::PatternStamp => Some("paint.patternStamp"),
        Tool::BackgroundEraser => Some("paint.backgroundEraser"),
        _ => None,
    }
}

/// The command a live-stroking tool commits: the Pencil's (and the Pencil- or Block-mode
/// Eraser's) `paint.pencil`, else `paint.stroke`.
pub(crate) fn stroke_command(app: &PhotocraftApp, tool: Tool) -> &'static str {
    if tool == Tool::Pencil || crate::eraser_ui::pencil_mode(app, tool) || crate::eraser_ui::block_mode(app, tool) { "paint.pencil" } else { "paint.stroke" }
}

/// Starts the live preview of the current drag, from the params its tool commits. `None` without
/// an active document or drag, or when the tool's preview can't begin.
pub(super) fn begin_live_stroke(app: &PhotocraftApp) -> Option<LiveStroke> {
    static STROKES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let st = app.session.active()?;
    let d = app.drag.as_ref()?;
    let stroke = if d.tool == Tool::MixerBrush {
        let points = app.stylus.stroke_points(&d.points);
        let p = stroke_params(app, d, points.get(..1)?);
        EngineStroke::Mixer(Box::new(photocraft_engine::brush_cmds::mixer_live::LiveMixer::begin(&app.session, &p).ok()?))
    } else if d.tool == Tool::ColorReplacement {
        // The params `retouch_ui::finish_stroke` commits.
        let mut p = crate::retouch_ui::color_replacement_params(app);
        p["points"] = json!(d.points);
        unsmoothed_line(&mut p, d.line);
        EngineStroke::Dab(Box::new(photocraft_engine::brush_cmds::live_color_replacement(&app.session, &p).ok()?))
    } else if let Some((cmd, mut p)) = crate::retouch_ui::dab_params(app, d.tool) {
        // The params `retouch_ui::finish_stroke` commits; Sample All Layers isn't previewed.
        p["points"] = json!(d.points);
        p["target"] = paint_target(app);
        unsmoothed_line(&mut p, d.line);
        EngineStroke::Dab(Box::new(photocraft_engine::retouch_cmds::LiveDab::begin(&app.session, cmd, &p).ok()?))
    } else if d.tool == Tool::PatternStamp {
        // The params `retouch_ui::finish_stroke` commits.
        let mut p = crate::retouch_ui::pattern_stamp_params(app, &d.points);
        p["points"] = json!(d.points);
        p["target"] = paint_target(app);
        unsmoothed_line(&mut p, d.line);
        if app.ui.tool_options.pattern_stamp_impressionist {
            EngineStroke::Dab(Box::new(photocraft_engine::retouch_cmds::live_pattern_impressionist(&app.session, &p).ok()?))
        } else {
            EngineStroke::Retouch(Box::new(photocraft_engine::retouch_cmds::LiveRetouch::begin(&app.session, "paint.patternStamp", &p).ok()?))
        }
    } else if d.tool == Tool::BackgroundEraser {
        // The params `eraser_ui::finish_stroke` commits (layer pixels only).
        let mut p = crate::eraser_ui::background_eraser_params(app, &d.points);
        unsmoothed_line(&mut p, d.line);
        EngineStroke::Dab(Box::new(photocraft_engine::eraser_cmds::live_background_eraser(&app.session, &p).ok()?))
    } else if let Some(cmd) = live_retouch_command(d.tool) {
        // The params `retouch_ui::finish_stroke` commits.
        let mut p = if d.tool == Tool::HistoryBrush {
            d.history_params.clone().unwrap_or_else(|| crate::retouch_ui::history_brush_params(app))
        } else {
            crate::retouch_ui::clone_params(app)?
        };
        p["points"] = json!(d.points);
        p["target"] = paint_target(app);
        unsmoothed_line(&mut p, d.line);
        EngineStroke::Retouch(Box::new(photocraft_engine::retouch_cmds::LiveRetouch::begin(&app.session, cmd, &p).ok()?))
    } else {
        let p = stroke_params(app, d, &app.stylus.stroke_points(&d.points));
        EngineStroke::Brush(Box::new(photocraft_engine::brush_cmds::LiveStroke::begin_with(&app.session, stroke_command(app, d.tool), &p).ok()?))
    };
    let n = STROKES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) & 0xff_ffff;
    let damage = vec![stroke.bounds()];
    Some(LiveStroke {
        seed: stroke.seed(),
        paint: PaintThread::start(stroke),
        doc: st.doc.id,
        revision: st.revision,
        key: (1 << 44) | (n << 20),
        damage,
        fed: if d.tool == Tool::MixerBrush { 1 } else { d.points.len() },
        preview_failed: false,
    })
}

/// Cancel uncommitted mixer paint when its document, tool or input context goes away.
pub(crate) fn cancel_mixer_on_context_change(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    if !app.drag.as_ref().is_some_and(|drag| drag.tool == Tool::MixerBrush) {
        return false;
    }
    let changed = app.ui.tool != Tool::MixerBrush
        || app.live_stroke.as_ref().is_some_and(|live| !app.session.active().is_some_and(|st| st.doc.id == live.doc && st.revision == live.revision));
    if changed || ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        app.drag = None;
        app.live_stroke = None;
        return true;
    }
    false
}

/// Render the drag points the live stroke hasn't seen yet, with the pen pressure, tilt and
/// rotation the commit's `paint.stroke` gets for them.
pub(super) fn feed_live_stroke(app: &mut PhotocraftApp) {
    feed_live_stroke_waiting(app, LIVE_STROKE_WAIT);
}

/// [`feed_live_stroke`], waiting at most `wait` for the stroke to paint (tests: not at all, to
/// hand points to a stroke that is busy).
pub(super) fn feed_live_stroke_waiting(app: &mut PhotocraftApp, wait: std::time::Duration) {
    // A batched move replays several samples in one frame and calls this once at the end, so the
    // per-sample calls from `tool_event` are skipped (see `canvas_view`).
    if app.defer_live_stroke {
        return;
    }
    let (Some(l), Some(d)) = (app.live_stroke.as_mut(), app.drag.as_ref()) else { return };
    if l.preview_failed {
        return;
    }
    let pose = &app.stylus.stroke;
    // The same times `stroke_points` gives the commit (from the drag's first point).
    let t0 = app.stylus.point_ms(0);
    let pts: Vec<_> = (l.fed..d.points.len())
        .filter_map(|i| {
            let p = d.points.get(i)?;
            let t = pose.get(i).or(pose.last()).copied().unwrap_or_default();
            let mut sp = photocraft_engine::paint::StrokePoint::new(p[0], p[1], p[2] as f32);
            (sp.tilt_x, sp.tilt_y, sp.rotation) = (t[0], t[1], t[2]);
            sp.time = app.stylus.point_ms(i) - t0;
            Some(sp)
        })
        .collect();
    l.fed = d.points.len();
    let started = crate::gpu_canvas::now_ms();
    l.paint.push(pts);
    let steps = l.paint.catch_up(wait);
    let tool = d.tool;
    take_steps(app, tool, steps);
    app.perf.stage.stroke_ms += crate::gpu_canvas::now_ms() - started;
}

/// How long a frame waits for the live stroke to paint the points it was just given. A light
/// brush is done well within it and shows them in the same frame, as when it painted on the UI
/// thread; a heavy one shows them in a later frame instead of holding this one up. Tests wait for
/// every step, so they see each one.
const LIVE_STROKE_WAIT: std::time::Duration = if cfg!(test) { std::time::Duration::MAX } else { std::time::Duration::from_millis(LIVE_STROKE_WAIT_MS) };
const LIVE_STROKE_WAIT_MS: u64 = 6;

/// Record painted steps on the live stroke. A failed one drops the live stroke (the commit then
/// renders the stroke itself), except the Mixer Brush's, which keeps its last valid preview.
fn take_steps(app: &mut PhotocraftApp, tool: Tool, steps: Vec<Step>) {
    let Some(l) = app.live_stroke.as_mut() else { return };
    for step in steps {
        match step {
            Ok(r) => l.damage.push(r),
            Err(e) => {
                if tool == Tool::MixerBrush {
                    // Keep the last valid preview and its document/revision guard. All input
                    // stays in the drag; release replays it without the preview tile budget.
                    l.preview_failed = true;
                    app.ui.status = e.to_string();
                    app.ui.status_error = true;
                } else {
                    app.live_stroke = None;
                }
                return;
            }
        }
    }
}

/// Is the live stroke still painting points it was given?
pub(super) fn live_stroke_busy(app: &PhotocraftApp) -> bool {
    app.live_stroke.as_ref().is_some_and(|l| l.paint.busy())
}

impl PhotocraftApp {
    /// Is the live stroke still painting, or holding steps the canvas hasn't shown yet? Lets a
    /// test read the canvas only once what it shows is the whole stroke so far (#13477).
    pub fn live_stroke_busy(&self) -> bool {
        live_stroke_busy(self)
    }
}

/// The points the live retouching stroke has rendered.
#[cfg(test)]
pub(crate) fn live_retouch_points(app: &PhotocraftApp) -> Option<Vec<photocraft_engine::paint::StrokePoint>> {
    app.live_stroke.as_ref()?.paint.peek(|stroke| match stroke {
        EngineStroke::Retouch(l) => Some(l.points().to_vec()),
        _ => None,
    })?
}

/// Offer the Clone Stamp's or History Brush's live stroke, or a Dodge, Burn, Sponge, Blur, Sharpen
/// or Smudge one, with the drag points it hasn't rendered yet, to the command
/// `retouch_ui::finish_stroke` commits, which then takes its pixels instead of rendering the
/// stroke again.
pub(super) fn offer_live_retouch(app: &mut PhotocraftApp, d: &Drag) {
    let dab = crate::retouch_ui::dab_params(app, d.tool).is_some();
    if !dab && !matches!(d.tool, Tool::CloneStamp | Tool::HistoryBrush) {
        return;
    }
    let Some(LiveStroke { paint, fed, .. }) = app.live_stroke.take() else { return };
    let (steps, Some(stroke)) = paint.finish() else { return };
    if steps.iter().any(Result::is_err) {
        return;
    }
    let rest: Vec<_> = d.points.get(fed..).unwrap_or_default().iter().map(|q| photocraft_engine::paint::StrokePoint::new(q[0], q[1], q[2] as f32)).collect();
    // A refused offer (the brush changed mid-stroke, say) leaves the commit to render the stroke.
    match stroke {
        EngineStroke::Retouch(live) if !dab => offer_retouch(app, live, &rest),
        EngineStroke::Dab(live) if dab => offer_dab(app, live, &rest),
        _ => {}
    }
}

fn offer_retouch(app: &mut PhotocraftApp, mut live: Box<photocraft_engine::retouch_cmds::LiveRetouch>, rest: &[photocraft_engine::paint::StrokePoint]) {
    if rest.is_empty() || live.push(rest).is_ok() {
        let _ = app.session.offer_live_retouch(*live);
    }
}

fn offer_dab(app: &mut PhotocraftApp, mut live: Box<photocraft_engine::retouch_cmds::LiveDab>, rest: &[photocraft_engine::paint::StrokePoint]) {
    if rest.is_empty() || live.push(rest).is_ok() {
        let _ = app.session.offer_live_dab(*live);
    }
}

/// A pen tap (a drag still at its one press point when released) paints its dab at the highest
/// pressure the pen reached during the contact, not just the first sample's (#1798). A mouse is
/// unchanged at full pressure.
pub(super) fn tap_at_peak_pressure(app: &mut PhotocraftApp) {
    if app.stylus.sample().is_none() {
        return;
    }
    let peak = f64::from(app.stylus.tap_pressure());
    let mut refresh_mixer = false;
    if let Some(d) = app.drag.as_mut().filter(|d| d.points.len() == 1)
        && let Some(p) = d.points.first_mut()
    {
        refresh_mixer = d.tool == Tool::MixerBrush && peak > p[2];
        p[2] = p[2].max(peak);
    }
    if refresh_mixer {
        app.live_stroke = begin_live_stroke(app);
    }
}

/// While a stroke is held still, its point repeats this often (in ms) so time-based brush
/// features (airbrush build-up, smoothing catch-up) keep going, as when the pen moves.
pub(super) const HOLD_REPEAT_MS: f64 = 16.0;

/// Is the live stroke one that changes while the pointer is held still (airbrush Build-up, or
/// smoothing catching up)? Only then does a held pointer repeat its point and keep repainting.
pub(super) fn stroke_wants_time(app: &PhotocraftApp) -> bool {
    app.live_stroke.as_ref().is_some_and(LiveStroke::wants_time)
}

/// Time of sample `k` of the `n` samples a frame delivered, spread evenly after `base` (the
/// previous point) up to `now` (this frame): egui's pointer events carry no timestamps.
pub(crate) fn spread_ms(base: f64, now: f64, k: usize, n: usize) -> f64 {
    // Not after `base` (or not comparable, NaN): every sample takes the frame time.
    if n == 0 || now.partial_cmp(&base) != Some(std::cmp::Ordering::Greater) {
        return now;
    }
    base + (now - base) * (k + 1) as f64 / n as f64
}

/// Tools whose gesture follows a freehand path (a polyline of the input points), so every pointer
/// sample the OS delivered improves the result. Other tools are driven by the pointer's latest
/// position (endpoints, anchors, guides), so feeding them a whole frame's moves only repeats work.
pub(crate) fn freehand_tool(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::Brush
            | Tool::Pencil
            | Tool::MixerBrush
            | Tool::ColorReplacement
            | Tool::Eraser
            | Tool::BackgroundEraser
            | Tool::HistoryBrush
            | Tool::Remove
            | Tool::ArtHistoryBrush
            | Tool::SpotHealing
            | Tool::Healing
            | Tool::CloneStamp
            | Tool::PatternStamp
            | Tool::Blur
            | Tool::Sharpen
            | Tool::Smudge
            | Tool::Dodge
            | Tool::Burn
            | Tool::Sponge
            | Tool::Lasso
            | Tool::MagneticLasso
            | Tool::Patch
            | Tool::ContentAwareMove
            | Tool::QuickSelection
            | Tool::FreeformPen
    )
}

/// The pointer moves this frame delivered while `button` was held, in order. `down_at_start` is
/// whether the button was already down when the frame began. A move before a press or after a
/// release in the same frame is dropped, so a gesture never picks up input from outside its own
/// press..release interval.
pub(super) fn pointer_moves(events: &[egui::Event], button: egui::PointerButton, down_at_start: bool) -> Vec<Pos2> {
    let mut down = down_at_start;
    let mut out = Vec::new();
    for e in events {
        match e {
            egui::Event::PointerButton { button: b, pressed, .. } if *b == button => down = *pressed,
            egui::Event::PointerMoved(p) if down => out.push(*p),
            _ => {}
        }
    }
    out
}

/// Modifiers attached to the pointer event itself. Some input sources provide these without a
/// separate `ModifiersChanged` event, so use them for click gestures before falling back to the
/// frame-wide modifier state.
pub(super) fn pointer_button_modifiers(events: &[egui::Event], button: egui::PointerButton) -> Option<egui::Modifiers> {
    events.iter().rev().find_map(|e| match e {
        egui::Event::PointerButton { button: b, modifiers, .. } if *b == button => Some(*modifiers),
        _ => None,
    })
}

/// The line from the brush to the pointer while stroke smoothing lags (Pulled String Mode and
/// Stroke Catch-up, #3479): a dark underlay and a light line stay visible on any pixels, and a dot
/// marks the brush end.
pub(super) fn draw_smoothing_string(painter: &egui::Painter, xf: &ViewXform, (brush, pointer): ([f64; 2], [f64; 2])) {
    let (a, b) = (xf.to_screen(brush[0] as f32, brush[1] as f32), xf.to_screen(pointer[0] as f32, pointer[1] as f32));
    if a.distance(b) < 2.0 {
        return;
    }
    painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(110)));
    painter.line_segment([a, b], Stroke::new(1.0, Color32::WHITE));
    painter.circle_filled(a, 2.5, Color32::WHITE);
}

/// Commit a Brush, Pencil or Eraser drag. The live stroke already rendered it: its pixels are
/// offered to the commit (`Session::offer_live_stroke`), so release costs no second render, and
/// the canvas then refreshes only what it hadn't shown yet (`CommittedStroke`).
pub(super) fn commit_brush_stroke(app: &mut PhotocraftApp, d: &Drag) {
    let live = app.live_stroke.take();
    let mut p = stroke_params(app, d, &app.stylus.stroke_points(&d.points));
    if let Some(seed) = live.as_ref().and_then(|l| l.seed) {
        p["seed"] = json!(seed);
    }
    let Some(l) = live else {
        let _ = app.run(stroke_command(app, d.tool), p);
        return;
    };
    // Points the live stroke hasn't rendered yet (the release frame's last moves).
    let t0 = app.stylus.point_ms(0);
    let rest: Vec<_> = (l.fed..d.points.len())
        .filter_map(|i| {
            let q = d.points.get(i)?;
            let t = app.stylus.stroke.get(i).or(app.stylus.stroke.last()).copied().unwrap_or_default();
            let mut sp = photocraft_engine::paint::StrokePoint::new(q[0], q[1], q[2] as f32);
            (sp.tilt_x, sp.tilt_y, sp.rotation) = (t[0], t[1], t[2]);
            sp.time = app.stylus.point_ms(i) - t0;
            Some(sp)
        })
        .collect();
    // What the live stroke's thread was still painting, then those points.
    let LiveStroke { paint, doc, key, mut damage, .. } = l;
    let (steps, stroke) = paint.finish();
    let painted = stroke.and_then(|mut stroke| {
        for step in steps.into_iter().chain((!rest.is_empty()).then(|| stroke.push(&rest))) {
            damage.push(step.ok()?);
        }
        Some(stroke)
    });
    // The live stroke can't show these points: the commit renders the stroke itself.
    let Some(stroke) = painted else {
        let _ = app.run(stroke_command(app, d.tool), p);
        return;
    };
    let before = app.session.documents().iter().find(|st| st.doc.id == doc).map(|st| st.doc.clone());
    if let EngineStroke::Brush(stroke) = stroke
        && app.session.offer_live_stroke(*stroke).is_err()
    {
        app.session.clear_live_stroke();
    }
    let started = crate::gpu_canvas::now_ms();
    let result = app.run(stroke_command(app, d.tool), p);
    app.perf.stage.stroke_ms += crate::gpu_canvas::now_ms() - started;
    app.session.clear_live_stroke();
    let Ok(r) = result else { return };
    let after = app.session.documents().iter().find(|st| st.doc.id == doc).map(|st| (st.revision, st.doc.clone()));
    if r.get("fromLive").and_then(serde_json::Value::as_bool) == Some(true)
        && let Some((revision, after)) = after
    {
        let relayout = before.map_or(DRect::EMPTY, |b| photocraft_compose::relayout_damage(&b, &after));
        app.committed_stroke = Some(CommittedStroke { doc, revision, key, damage, relayout });
    } else {
        // Rendered again from scratch: the commit's damage rect refreshes the stroke.
        let shown = CommittedStroke { doc, revision: 0, key, damage, relayout: DRect::EMPTY };
        // Raw preview key 0 = the document itself (its colour display folded in).
        shown_as_document(app, doc, |k| shown.since(k).is_some());
    }
}

/// Pencil (and a Pencil-mode Eraser) dabs sit on the pixel grid. Clicks anywhere inside one
/// pixel must produce the same point, or a ⇧-click line changes with the sub-pixel position.
pub(super) fn snap_aliased_tip(app: &PhotocraftApp, tool: Tool, x: f64, y: f64) -> (f64, f64) {
    let pencil = tool == Tool::Pencil || crate::eraser_ui::pencil_mode(app, tool);
    if !pencil {
        return (x, y);
    }
    let (cx, cy) = photocraft_engine::paint::grid_center(x, y, app.session.tools.brush.size);
    (f64::from(cx), f64::from(cy))
}

/// "mask" when the Layers panel targets (or the canvas shows) the active layer's mask, or the layer
/// is an adjustment layer with one (it has no pixels: #4298), else "pixels".
pub fn paint_target(app: &PhotocraftApp) -> serde_json::Value {
    use photocraft_engine::channel_cmds::ChannelTarget;
    let Some(st) = app.session.active() else { return json!("pixels") };
    // A targeted alpha channel (Channels panel) or Quick Mask mode wins over the layer.
    match st.channel_view.target {
        ChannelTarget::Alpha(i) if i < st.doc.channels.len() => return json!({ "channel": i }),
        ChannelTarget::Composite if st.doc.quick_mask.is_some() => return json!("quickMask"),
        ChannelTarget::LayerMask => return json!("mask"),
        ChannelTarget::Color(_) => {
            if let Some(view) = photocraft_engine::mask_view_cmds::current(st) {
                return json!(if view.filter { "filterMask" } else { "mask" });
            }
            // A colour set edits a raster layer's own pixels. The rows stay selected across layer
            // switches, so an adjustment layer's mask and a layer that needs rasterizing first
            // keep the layer's own target below.
            let raster = st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| matches!(l.content, photocraft_doc::LayerContent::Raster(_)));
            if raster && let Some(colors) = photocraft_engine::color_channels::selected(&app.session) {
                return colors.params();
            }
        }
        _ => {}
    }
    // A smart object's targeted (or shown) filter mask takes the paint (FILE-215-3).
    if app.filter_mask_targeted() {
        return json!("filterMask");
    }
    let has_mask = st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| l.mask.is_some());
    // Viewing the mask (⌥-click its thumbnail, #196) paints the mask.
    let viewing = photocraft_engine::mask_view_cmds::current(st).is_some_and(|v| !v.filter);
    let mask = app.ui.mask_target || viewing || crate::adjustment_layer_active(st);
    json!(if mask && has_mask { "mask" } else { "pixels" })
}

/// Colour rows can remain selected behind a viewed mask; restrictions follow what is edited.
/// Several rows stop Move and transforms on any layer, which the engine refuses there too.
pub(crate) fn multiple_color_target(app: &PhotocraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    matches!(st.channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Color(_))
        && photocraft_engine::mask_view_cmds::current(st).is_none()
        && photocraft_engine::color_channels::selected(&app.session).is_some_and(|colors| colors.len() > 1)
}
