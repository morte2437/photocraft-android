//! Placement markers for the active layer's type path.
use crate::{PhotocraftApp, canvas::ViewXform, state::Tool};
use std::{
    collections::VecDeque,
    sync::{Arc, Weak},
};

use egui::{Pos2, Stroke, Vec2};
use photocraft_doc::{Document, LayerContent, TextLayer, text::TextShape};
use photocraft_geom::{Point, Rect};
use photocraft_text::warp::PathGeometry;

/// Returns the active text layer when a path selection tool is active and targeted at the type path.
pub(crate) fn active_text(app: &PhotocraftApp) -> Option<&TextLayer> {
    if !matches!(app.ui.tool, Tool::PathSelection | Tool::DirectSelection)
        || app.ui.transform.is_some()
        || app.ui.vector_mask_target
        || app.ui.selected_path.as_deref().is_some_and(|name| name != "layer")
    {
        return None;
    }
    let st = app.session.active()?;
    if st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::VectorMask {
        return None;
    }
    let id = st.active_layer?;
    if !st.doc.shown_at(&st.doc.path_of(id)?) {
        return None;
    }
    let layer = st.doc.layer(id)?;
    match &layer.content {
        LayerContent::Text(text) if matches!(text.shape, TextShape::Path { .. }) => Some(text),
        _ => None,
    }
}

/// Returns the normalized paragraph alignment weight (0.0 for left, 0.5 for center, 1.0 for right).
fn alignment_factor(align: photocraft_doc::text::TextAlign) -> f64 {
    use photocraft_doc::text::TextAlign;
    match align {
        TextAlign::Center | TextAlign::JustifyCenter => 0.5,
        TextAlign::Right | TextAlign::JustifyRight => 1.0,
        _ => 0.0,
    }
}

/// Computes the start and end arc-length interval of path text, converting legacy alignment anchors to explicit spans.
fn effective_range(text: &TextLayer, geometry: &PathGeometry) -> Option<[f64; 2]> {
    let TextShape::Path { path, start, end, .. } = &text.shape else { return None };
    let length = geometry.length();
    let mut start = f64::from(*start);
    if !length.is_finite() || length <= 0.0 || !start.is_finite() {
        return None;
    }
    let range = if let Some(end) = end {
        let mut end = f64::from(*end);
        if !end.is_finite() || end < start {
            return None;
        }
        if !path.closed {
            if start < 0.0 {
                return None;
            }
            start = start.min(length);
            end = end.min(length);
        }
        [start, end]
    } else {
        // Different paragraph alignments cannot share one equivalent interval.
        let runs = text.paragraph_runs();
        let f = alignment_factor(runs.first()?.style.align);
        if runs.iter().any(|run| alignment_factor(run.style.align) != f) {
            return None;
        }
        if path.closed {
            [start - f * length, start + (1.0 - f) * length]
        } else if start < 0.0 {
            return None;
        } else {
            let start = start.min(length);
            if f == 0.0 {
                [start, length]
            } else if f == 1.0 {
                [0.0, start]
            } else if start <= length / 2.0 {
                [0.0, 2.0 * start]
            } else {
                [2.0 * start - length, length]
            }
        }
    };
    if range.iter().any(|n| !n.is_finite()) || range[1] < range[0] || (!path.closed && (range[0] < 0.0 || range[1] > length)) {
        return None;
    }
    Some(range)
}

/// Single-entry cache for flattened type path geometry keyed by document snapshot and shape identity.
#[derive(Clone, Debug)]
pub struct CachedGeometry {
    pub(crate) doc: Weak<Document>,
    pub(crate) layer: photocraft_doc::LayerId,
    pub(crate) path: photocraft_doc::vector::Subpath,
    pub(crate) flip: bool,
    pub(crate) geometry: Arc<PathGeometry>,
    pub(crate) reversed: Arc<PathGeometry>,
}

impl PartialEq for CachedGeometry {
    fn eq(&self, other: &Self) -> bool {
        self.doc.ptr_eq(&other.doc) && self.layer == other.layer && self.flip == other.flip && self.path == other.path
    }
}

/// Bounded single-entry geometry cache wrapper implementing equality and interior mutability.
#[derive(Clone, Debug, Default)]
pub struct GeometryCache(pub(crate) Arc<std::sync::Mutex<Option<CachedGeometry>>>);

impl PartialEq for GeometryCache {
    fn eq(&self, _other: &Self) -> bool {
        // Cached geometry is transient and does not change logical UI state.
        true
    }
}

impl Eq for GeometryCache {}

/// Retrieves or creates cached path geometry, reusing active drag geometry or single-entry cache across frames.
pub(crate) fn active_path_geometry(app: &PhotocraftApp, text: &TextLayer) -> Option<Arc<PathGeometry>> {
    let TextShape::Path { path, flip, .. } = &text.shape else { return None };
    if let Some(drag) = app.ui.type_path_drag.as_ref().filter(|d| d.valid(app)) {
        if *flip == drag.original_flip {
            return Some(Arc::clone(&drag.geometry));
        } else {
            return Some(Arc::clone(&drag.reversed_geometry));
        }
    }
    let st = app.session.active()?;
    let layer = st.active_layer?;
    let mut guard = app.ui.type_path_geometry_cache.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(cached) = guard.as_ref()
        && cached.doc.ptr_eq(&Arc::downgrade(&st.doc))
        && cached.layer == layer
        && cached.path == *path
    {
        if cached.flip == *flip {
            return Some(Arc::clone(&cached.geometry));
        } else {
            return Some(Arc::clone(&cached.reversed));
        }
    }
    let geometry = Arc::new(PathGeometry::new(path, *flip)?);
    let reversed = Arc::new(geometry.flipped());
    *guard = Some(CachedGeometry { doc: Arc::downgrade(&st.doc), layer, path: path.clone(), flip: *flip, geometry: Arc::clone(&geometry), reversed });
    Some(geometry)
}

/// Computes screen-space positions and tangents for start and end placement markers.
#[cfg(test)]
pub(crate) fn markers(text: &TextLayer, xf: &ViewXform) -> Option<[(Pos2, Vec2); 2]> {
    let TextShape::Path { path, flip, .. } = &text.shape else { return None };
    let geometry = PathGeometry::new(path, *flip)?;
    markers_with_geometry(text, &geometry, xf)
}

/// Computes placement markers using existing or cached path geometry.
pub(crate) fn markers_with_geometry(text: &TextLayer, geometry: &PathGeometry, xf: &ViewXform) -> Option<[(Pos2, Vec2); 2]> {
    let [start, finish] = effective_range(text, geometry)?;
    let sample = |arc| {
        let ((x, y), (tx, ty)) = geometry.sample(arc)?;
        let a = text.transform.apply(Point::new(x, y));
        let b = text.transform.apply(Point::new(x + tx, y + ty));
        let p = xf.to_screen(a.x as f32, a.y as f32);
        let q = xf.to_screen(b.x as f32, b.y as f32);
        let direction = q - p;
        (p.is_finite() && direction.is_finite() && direction.length_sq() > 1e-10).then_some((p, direction.normalized()))
    };
    Some([sample(start)?, sample(finish)?])
}

/// Screen radius of the beginning (`x`) and ending (`o`) markers drawn on the path.
const MARKER_RADIUS: f32 = 4.0;
/// Screen distance from a marker within which a press grabs it.
const MARKER_REACH: f32 = 6.0;

/// Hit-tests a screen point against the start and end markers, which sit on the path itself.
fn marker_hit(ends: [(Pos2, Vec2); 2], p: Pos2) -> Option<(usize, f32)> {
    ends.into_iter()
        .enumerate()
        .map(|(index, (point, _))| (index, point.distance(p)))
        .filter(|(_, distance)| *distance <= MARKER_REACH)
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// Screen distance along the path from coincident full-lap markers within which a press moves the
/// whole text rather than one end.
const LAP_CENTER: f32 = 1.5;

/// What a press at screen point `p` on marker `index` grabs: `(endpoint, either)`. Where the two
/// markers coincide on screen:
///
/// - On a closed path's full lap (the default for text attached to a closed path) the text
///   begins just ahead of the point and ends just behind it, so a press ahead along the path
///   takes the beginning, one behind takes the end, and one on the point moves the whole text.
///   Either end can then be dragged to shorten the lap.
/// - An empty or end-clamped range waits for the first motion to pick a marker (`either`):
///   backward takes the beginning and forward the end, so the range can always open.
fn marker_grab(ends: [(Pos2, Vec2); 2], index: usize, p: Pos2, closed: bool, range: [f64; 2], length: f64) -> (Option<usize>, bool) {
    let [(point, tangent), (other, _)] = ends;
    if point.distance(other) >= 1.0 {
        return (Some(index), false);
    }
    if !(closed && range[1] - range[0] >= length - 1e-3) {
        return (None, true);
    }
    let along = (p - point).dot(tangent);
    if along > LAP_CENTER {
        (Some(0), false)
    } else if along < -LAP_CENTER {
        (Some(1), false)
    } else {
        (None, false)
    }
}

/// What the pointer would move: the cursor shows an I-beam with a right-pointing triangle for the
/// beginning, a left-pointing one for the end, and both for the whole text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Grab {
    /// The beginning marker.
    Start,
    /// The ending marker.
    End,
    /// The text body: both boundaries move together.
    Body,
}

impl Grab {
    /// The grab a gesture with `endpoint` (marker index, or `None` for the body) performs.
    fn of(endpoint: Option<usize>) -> Self {
        match endpoint {
            Some(0) => Self::Start,
            Some(_) => Self::End,
            None => Self::Body,
        }
    }
}

/// What a press at screen point `p` would grab, for the hover cursor (the active gesture's grab
/// while dragging).
pub(crate) fn cursor(app: &mut PhotocraftApp, xf: &ViewXform, p: Pos2) -> Option<Grab> {
    if let Some(drag) = app.ui.type_path_drag.as_ref().filter(|drag| drag.valid(app)) {
        return Some(Grab::of(drag.endpoint));
    }
    if !app.ui.view.shows(app.ui.view.show.target_path) {
        return None;
    }
    let st = app.session.active()?;
    let layer = st.active_layer?;
    let path = st.doc.path_of(layer)?;
    if (st.doc.locks_at(&path).all || st.doc.locks_at(&path).position) || !st.doc.shown_at(&path) {
        return None;
    }
    let text = active_text(app)?;
    let geometry = active_path_geometry(app, text)?;
    let ends = markers_with_geometry(text, &geometry, xf)?;
    let d = xf.to_doc(p);
    if let Some((index, _)) = marker_hit(ends, p) {
        let TextShape::Path { path, .. } = &text.shape else { return None };
        let range = effective_range(text, &geometry)?;
        let (endpoint, _) = marker_grab(ends, index, p, path.closed, range, geometry.length());
        return Some(Grab::of(endpoint));
    }
    (crate::type_tool::hit_layer(app, d[0], d[1]) == Some(layer)).then_some(Grab::Body)
}

/// Draws the placement cursor: an I-beam with a filled triangle on the side `grab` points to.
pub(crate) fn draw_cursor(ctx: &egui::Context, p: Pos2, grab: Grab) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("type-path-cursor")));
    let beam = [
        (p + Vec2::new(0.0, -7.0), p + Vec2::new(0.0, 7.0)),
        (p + Vec2::new(-3.0, -7.0), p + Vec2::new(3.0, -7.0)),
        (p + Vec2::new(-3.0, 7.0), p + Vec2::new(3.0, 7.0)),
    ];
    for (a, b) in beam {
        painter.line_segment([a, b], Stroke::new(3.0, egui::Color32::BLACK));
        painter.line_segment([a, b], Stroke::new(1.0, egui::Color32::WHITE));
    }
    let sides: &[f32] = match grab {
        Grab::Start => &[1.0],
        Grab::End => &[-1.0],
        Grab::Body => &[-1.0, 1.0],
    };
    for &sign in sides {
        let triangle = vec![p + Vec2::new(3.0 * sign, -4.0), p + Vec2::new(8.0 * sign, 0.0), p + Vec2::new(3.0 * sign, 4.0)];
        painter.add(egui::Shape::convex_polygon(triangle, egui::Color32::BLACK, Stroke::new(1.0, egui::Color32::WHITE)));
    }
}

/// Checks whether text glyphs exceed the current path range, caching by document snapshot, revision, and placement.
fn overflows(app: &PhotocraftApp, ctx: &egui::Context, text: &TextLayer) -> bool {
    let Some(st) = app.session.active() else { return false };
    let TextShape::Path { start, end, flip, .. } = &text.shape else { return false };
    let key = (st.revision, start.to_bits(), end.map(f32::to_bits), *flip);
    let source = Arc::downgrade(&st.doc);
    let id = egui::Id::unique(("path-text-overflow", st.doc.id.0, st.active_layer));
    if let Some((_, _, value)) = ctx
        .data(|data| data.get_temp::<((u64, u32, Option<u32>, bool), Weak<Document>, bool)>(id))
        .filter(|(cached, doc, _)| *cached == key && doc.ptr_eq(&source))
    {
        return value;
    }
    let mut engine = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let layout = engine.layout(text, st.doc.resolution_dpi);
    let value = photocraft_text::render::path_overflows(&layout, &text.shape);
    ctx.data_mut(|data| data.insert_temp(id, (key, source, value)));
    value
}

/// Renders placement start/end markers and overflow indicators for the active type path.
pub(crate) fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(text) = active_text(app) else { return };
    let mut preview = text.clone();
    if let Some(drag) = app.ui.type_path_drag.as_ref().filter(|d| d.valid(app) && (d.proposed != d.original || d.flipped)) {
        let (range, side) = drag.placement();
        if let TextShape::Path { start, end, flip, .. } = &mut preview.shape {
            *start = range[0] as f32;
            *end = Some(range[1] as f32);
            *flip = side;
        }
    }
    let Some(geometry) = active_path_geometry(app, &preview) else { return };
    let Some(markers) = markers_with_geometry(&preview, &geometry, xf) else { return };
    let stroke = Stroke::new(1.5, crate::theme::Tokens::get(painter.ctx()).accent);
    let overflow = overflows(app, painter.ctx(), &preview);
    let [(start, tangent), (end, end_tangent)] = markers;
    // The beginning is an `x` across the path, square to its direction.
    let normal = Vec2::new(-tangent.y, tangent.x);
    for diagonal in [tangent + normal, tangent - normal] {
        let arm = diagonal * (MARKER_RADIUS / std::f32::consts::SQRT_2);
        painter.line_segment([start - arm, start + arm], stroke);
    }
    // The end is a small circle, with a plus inside while the range hides glyphs.
    painter.circle_stroke(end, MARKER_RADIUS, stroke);
    if overflow {
        let normal = Vec2::new(-end_tangent.y, end_tangent.x);
        let arm = MARKER_RADIUS - 1.5;
        painter.line_segment([end - end_tangent * arm, end + end_tangent * arm], stroke);
        painter.line_segment([end - normal * arm, end + normal * arm], stroke);
    }
}

#[derive(Clone, Debug)]
struct Preview {
    range: [f64; 2],
    flip: bool,
    shown: Arc<Document>,
    key: u64,
}

pub(crate) struct Damage {
    document: photocraft_doc::DocId,
    revision: u64,
    original_area: Rect,
    areas: VecDeque<(u64, Rect)>,
}

/// Calculates canvas invalidation damage bounding the union of preview text regions across display revisions.
pub(crate) fn damage(app: &PhotocraftApp, doc: photocraft_doc::DocId, revision: u64, before: u64, after: u64) -> Option<Rect> {
    let p = app.type_path_damage.as_ref().filter(|p| p.document == doc && p.revision == revision)?;
    let area = |key| if key == 0 { Some(p.original_area) } else { p.areas.iter().find(|(k, _)| *k == key).map(|(_, r)| *r) };
    Some(area(before)?.union(&area(after)?))
}

/// Returns a transient document snapshot with live placement preview modifications and tracks damage bounding boxes.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let drag = app.ui.type_path_drag.as_ref()?;
    if !drag.valid(app) {
        return None;
    }
    let st = app.session.documents().get(idx)?;
    if st.doc.id != drag.document || st.revision != drag.revision {
        return None;
    }
    if drag.proposed == drag.original && !drag.flipped {
        return None;
    }
    let (range, flip) = drag.placement();
    if let Some(preview) = &drag.preview
        && preview.range == range
        && preview.flip == flip
    {
        return Some((preview.shown.clone(), preview.key));
    }
    let source = st.doc.clone();
    let layer = drag.layer;
    let original_area = source.layer(layer)?.surface().map_or(Rect::EMPTY, |s| s.tile_bounds());
    let mut shown = (*source).clone();
    let LayerContent::Text(text) = &mut shown.layer_mut(layer)?.content else { return None };
    let TextShape::Path { start, end, flip: side, .. } = &mut text.shape else { return None };
    *start = range[0] as f32;
    *end = Some(range[1] as f32);
    *side = flip;
    photocraft_engine::type_cmds::refresh(&source, text);
    let area = text.cache.as_ref().map_or(Rect::EMPTY, |s| s.tile_bounds());
    let shown = Arc::new(photocraft_engine::mode_cmds::display_document(&shown).unwrap_or(shown));
    const BASE: u64 = 1 << 50;
    let key = BASE | (app.ui.alloc_id() & (BASE - 1));
    let mut damage = app.type_path_damage.take().filter(|p| p.document == source.id && p.revision == st.revision).unwrap_or_else(|| Damage {
        document: source.id,
        revision: st.revision,
        original_area,
        areas: VecDeque::new(),
    });
    damage.original_area = damage.original_area.union(&original_area);
    if damage.areas.len() >= 128 {
        damage.areas.pop_front();
    }
    damage.areas.push_back((key, area));
    app.type_path_damage = Some(damage);
    app.ui.type_path_drag.as_mut()?.preview = Some(Preview { range, flip, shown: shown.clone(), key });
    Some((shown, key))
}

/// One marker gesture. The document is changed only on release, so Escape is lossless.
#[derive(Clone, Debug)]
pub struct Drag {
    pub(crate) viewport: Option<egui::ViewportId>,
    document: photocraft_doc::DocId,
    source: Weak<Document>,
    layer: photocraft_doc::LayerId,
    revision: u64,
    tool: Tool,
    inverse: photocraft_doc::Affine,
    pub(crate) geometry: Arc<PathGeometry>,
    pub(crate) reversed_geometry: Arc<PathGeometry>,
    closed: bool,
    endpoint: Option<usize>,
    /// Coincident markers were pressed: the first motion picks the beginning (backward) or the
    /// end (forward) as `endpoint`.
    either: bool,
    alignment: f64,
    original: [f64; 2],
    proposed: [f64; 2],
    previous: f64,
    delta: f64,
    original_flip: bool,
    flipped: bool,
    initial_side: f64,
    transform: photocraft_doc::Affine,
    xf: ViewXform,
    preview: Option<Preview>,
}

// Geometry is fixed by the source document revision; cached preview pixels are derived state.
impl PartialEq for Drag {
    /// Compares gesture state identity, ignoring transient preview cache artifacts.
    fn eq(&self, other: &Self) -> bool {
        self.source.ptr_eq(&other.source)
            && (self.document, self.layer, self.revision, self.tool) == (other.document, other.layer, other.revision, other.tool)
            && self.inverse == other.inverse
            && self.closed == other.closed
            && self.endpoint == other.endpoint
            && self.either == other.either
            && self.alignment == other.alignment
            && self.original == other.original
            && self.proposed == other.proposed
            && self.previous == other.previous
            && self.delta == other.delta
            && self.original_flip == other.original_flip
            && self.flipped == other.flipped
            && self.initial_side == other.initial_side
            && self.transform == other.transform
            && self.xf.rect == other.xf.rect
            && self.xf.zoom == other.xf.zoom
            && self.xf.center == other.xf.center
            && self.xf.flip == other.xf.flip
            && self.xf.rotation == other.xf.rotation
            && self.xf.aspect == other.xf.aspect
    }
}

impl Drag {
    /// Checks whether the drag gesture remains bound to the active document revision, layer, and tool.
    fn valid(&self, app: &PhotocraftApp) -> bool {
        active_text(app).is_some()
            && app.ui.tool == self.tool
            && app.session.active().is_some_and(|st| {
                st.doc.id == self.document
                    && self.source.ptr_eq(&Arc::downgrade(&st.doc))
                    && st.active_layer == Some(self.layer)
                    && st.revision == self.revision
            })
    }

    /// Computes the proposed start and end arc lengths and flip state, remapping intervals when flipped across the path.
    fn placement(&self) -> ([f64; 2], bool) {
        let range = if self.flipped {
            let length = self.geometry.length();
            let start = length - self.proposed[1];
            let start = if self.closed { start.rem_euclid(length) } else { start };
            [start, start + self.proposed[1] - self.proposed[0]]
        } else {
            self.proposed
        };
        (range, self.original_flip ^ self.flipped)
    }

    /// Projects pointer motion onto the path geometry, updating proposed placement range and detecting flip crossings.
    fn update(&mut self, x: f64, y: f64) {
        let point = self.inverse.apply(Point::new(x, y));
        let Some((arc, distance)) = self.geometry.project_near(point.x, point.y, self.previous) else { return };
        let length = self.geometry.length();
        if let Some(((px, py), (tx, ty))) = self.geometry.sample(arc) {
            let a = self.transform.apply(Point::new(px, py));
            let b = self.transform.apply(Point::new(px + tx, py + ty));
            let c = self.transform.apply(Point::new(px - ty, py + tx));
            let a = self.xf.to_screen(a.x as f32, a.y as f32);
            let b = self.xf.to_screen(b.x as f32, b.y as f32) - a;
            let c = self.xf.to_screen(c.x as f32, c.y as f32) - a;
            let screen_distance = if b.length() > 1e-6 { distance * f64::from((b.x * c.y - b.y * c.x).abs() / b.length()) } else { 0.0 };
            if screen_distance.abs() > 4.0 {
                self.flipped = screen_distance.signum() != self.initial_side;
            }
        }
        let mut delta = arc - self.previous;
        if self.closed {
            delta = (delta + length / 2.0).rem_euclid(length) - length / 2.0;
        }
        self.delta += delta;
        self.previous = arc;
        if self.either && self.delta != 0.0 {
            self.endpoint = Some(if self.delta < 0.0 { 0 } else { 1 });
            self.either = false;
        }
        if self.delta == 0.0 && self.proposed == self.original {
            return;
        }
        let [start, end] = self.original;
        self.proposed = if self.endpoint == Some(0) {
            [(start + self.delta).clamp(if self.closed { end - length } else { 0.0 }, end), end]
        } else if self.endpoint == Some(1) {
            [start, (end + self.delta).clamp(start, if self.closed { start + length } else { length })]
        } else if self.closed {
            [start + self.delta, end + self.delta]
        } else {
            moved_open_range(self.original, self.delta, self.alignment, length)
        };
    }
}

// Preserve the paragraph anchor when an open range reaches an endpoint. Keeping
// the whole span fixed would prevent a range ending at the path endpoint from moving.
/// Translates an open path range while preserving paragraph alignment anchor positions when hitting path endpoints.
fn moved_open_range([start, end]: [f64; 2], delta: f64, alignment: f64, length: f64) -> [f64; 2] {
    let span = end - start;
    let anchor = (start + span * alignment + delta).clamp(0.0, length);
    if alignment == 0.0 {
        [anchor, (anchor + span).min(length)]
    } else if alignment == 1.0 {
        [(anchor - span).max(0.0), anchor]
    } else {
        let half = (span / 2.0).min(anchor).min(length - anchor);
        [anchor - half, anchor + half]
    }
}

/// Discard a capture when its original document or tool is no longer active.
pub(crate) fn cancel_stale(app: &mut PhotocraftApp) {
    if app.ui.type_path_drag.as_ref().is_some_and(|drag| !drag.valid(app)) {
        cancel(app);
    }
}

/// Cancels active path placement capture when the owning viewport loses focus or the primary button was released externally.
pub(crate) fn cancel_missing_release(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(drag) = &app.ui.type_path_drag else { return };
    if let Some(owner) = drag.viewport
        && ctx.viewport_id() != owner
    {
        return;
    }
    if ctx.input(|input| !input.focused || !input.pointer.primary_down()) {
        cancel(app);
    }
}

/// Cancels any active type path placement drag gesture and discards live preview state.
pub(crate) fn cancel(app: &mut PhotocraftApp) -> bool {
    app.ui.type_path_drag.take().is_some()
}

/// Helper entry point for pointer events without explicit view context in tests.
#[cfg(test)]
pub(crate) fn pointer(app: &mut PhotocraftApp, event: crate::canvas::ToolEvent) -> bool {
    pointer_with_context(app, event, None, None)
}

/// Helper entry point for pointer events with an explicit view transform in tests.
#[cfg(test)]
pub(crate) fn pointer_with_view(app: &mut PhotocraftApp, event: crate::canvas::ToolEvent, view: Option<&ViewXform>) -> bool {
    pointer_with_context(app, event, view, None)
}

/// Handles canvas pointer events for type path marker and body drag gestures, managing capture and preview updates.
pub(crate) fn pointer_with_context(
    app: &mut PhotocraftApp,
    event: crate::canvas::ToolEvent,
    view: Option<&ViewXform>,
    viewport: Option<egui::ViewportId>,
) -> bool {
    use crate::canvas::ToolEvent;
    if matches!(event, ToolEvent::Down { .. })
        && let Some(drag) = &app.ui.type_path_drag
    {
        if drag.viewport.is_none() || drag.viewport == viewport {
            cancel(app);
        } else {
            return false;
        }
    }
    if let Some(mut drag) = app.ui.type_path_drag.take() {
        if drag.viewport.is_some_and(|owner| Some(owner) != viewport) {
            app.ui.type_path_drag = Some(drag);
            return false;
        }
        if !drag.valid(app) {
            return true;
        }
        if let Some(xf) = view {
            drag.xf = *xf;
        }
        match event {
            ToolEvent::Move { x, y, .. } => {
                drag.update(x, y);
                app.ui.type_path_drag = Some(drag);
            }
            ToolEvent::Up { x, y } => {
                drag.update(x, y);
                if drag.proposed != drag.original || drag.flipped {
                    let (mut range, flip) = drag.placement();
                    if !drag.closed {
                        let canonical = drag.geometry.canonical_length_f32() as f64;
                        range[1] = range[1].min(canonical);
                        range[0] = range[0].clamp(0.0, range[1]);
                    }
                    if let Err(error) = app.run(
                        "type.setPathPlacement",
                        serde_json::json!({
                            "layer": drag.layer.0, "start": range[0], "end": range[1], "flip": flip
                        }),
                    ) {
                        app.ui.status = error;
                        app.ui.status_error = true;
                    }
                }
            }
            ToolEvent::Down { .. } => {}
        }
        return true;
    }
    let ToolEvent::Down { x, y, .. } = event else { return false };
    if !app.ui.view.shows(app.ui.view.show.target_path) {
        return false;
    }
    let Some(text) = active_text(app).cloned() else { return false };
    let TextShape::Path { path, flip, .. } = &text.shape else { return false };
    let Some(geometry) = active_path_geometry(app, &text) else { return false };
    let Some(inverse) = text.transform.inverse() else { return false };
    let Some(original) = effective_range(&text, &geometry) else { return false };
    let Some(xf) = view.copied().or_else(|| ViewXform::active(app)) else { return false };
    let p = xf.to_screen(x as f32, y as f32);
    let best = markers_with_geometry(&text, &geometry, &xf).and_then(|ends| Some((ends, marker_hit(ends, p)?)));
    let active_layer = app.session.active().and_then(|st| st.active_layer);
    let (endpoint, either) = if let Some((ends, (index, _))) = best {
        marker_grab(ends, index, p, path.closed, original, geometry.length())
    } else if crate::type_tool::hit_layer(app, x, y) == active_layer && active_layer.is_some() {
        (None, false)
    } else {
        return false;
    };
    let Some(st) = app.session.active() else { return false };
    let Some(layer) = st.active_layer else { return false };
    if st.doc.effective_locks(layer).all || st.doc.effective_locks(layer).position {
        return false;
    }
    let point = inverse.apply(Point::new(x, y));
    let Some((previous, distance)) = geometry.project(point.x, point.y) else { return false };
    let reversed_geometry = Arc::new(geometry.flipped());
    app.ui.type_path_drag = Some(Drag {
        viewport,
        document: st.doc.id,
        source: Arc::downgrade(&st.doc),
        layer,
        revision: st.revision,
        tool: app.ui.tool,
        inverse,
        geometry,
        reversed_geometry,
        closed: path.closed,
        endpoint,
        either,
        alignment: text.paragraph_runs().first().map_or(0.0, |run| alignment_factor(run.style.align)),
        original,
        proposed: original,
        previous,
        delta: 0.0,
        original_flip: *flip,
        flipped: false,
        initial_side: if distance.abs() > 1e-6 { distance.signum() } else { -1.0 },
        transform: text.transform,
        xf,
        preview: None,
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::ToolEvent;
    use serde_json::json;

    /// Sets up a test harness with a document, path, and attached type layer for placement tests.
    fn app(tool: Tool) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.sync_views();
        app.ui.tool = tool;
        app.run("path.set", json!({"name":"work", "path":{"subpaths":[{"closed":false,"knots":[[20,100],[180,100]]}]}})).unwrap();
        app.ui.selected_path = Some("work".into());
        app.run("type.create", json!({"text":"ABC", "x":20, "y":100})).unwrap();
        app.run("type.attachToPath", json!({"start":20})).unwrap();
        app.run("type.setPathPlacement", json!({"end":100})).unwrap();
        app.ui.views[0].zoom = 1.0;
        app
    }

    /// Proves drag capture is preserved while the pointer is held and cancelled cleanly when release is lost during focus loss.
    #[test]
    fn missing_release_cleanup_preserves_held_capture_and_cancels_lost_release() {
        let mut app = app(Tool::PathSelection);
        let ctx = egui::Context::default();
        let pos = egui::pos2(1000.0, 1000.0);
        ctx.begin_pass(egui::RawInput {
            focused: true,
            events: vec![
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerGone,
            ],
            ..Default::default()
        });
        down(&mut app, 40.0, 100.0);
        cancel_missing_release(&mut app, &ctx);
        assert!(app.ui.type_path_drag.is_some());
        ctx.end_pass().textures_delta.clear();
        ctx.begin_pass(egui::RawInput {
            focused: true,
            events: vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }],
            ..Default::default()
        });
        let revision = app.session.active().unwrap().revision;
        cancel_missing_release(&mut app, &ctx);
        assert!(app.ui.type_path_drag.is_none());
        assert_eq!(app.session.active().unwrap().revision, revision);
        down(&mut app, 40.0, 100.0);
        assert!(app.ui.type_path_drag.is_some());
        ctx.end_pass().textures_delta.clear();
        ctx.begin_pass(egui::RawInput {
            focused: true,
            events: vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }],
            ..Default::default()
        });
        assert!(ctx.input(|i| i.pointer.primary_down()));
        ctx.end_pass().textures_delta.clear();
        ctx.begin_pass(egui::RawInput { focused: false, ..Default::default() });
        assert!(ctx.input(|i| i.pointer.primary_down()), "primary button must be held during focus loss");
        cancel_missing_release(&mut app, &ctx);
        assert!(app.ui.type_path_drag.is_none());
        ctx.end_pass().textures_delta.clear();
    }

    /// Verifies switching tools cancels any in-progress marker drag capture before returning to selection tools.
    #[test]
    fn selecting_another_tool_cancels_capture_before_switching_back() {
        let mut app = app(Tool::PathSelection);
        down(&mut app, 40.0, 100.0);
        assert!(app.ui.type_path_drag.is_some());
        crate::tool_memory::select(&mut app, Tool::Move);
        crate::tool_memory::select(&mut app, Tool::PathSelection);
        assert!(app.ui.type_path_drag.is_none());
    }

    /// Ensures attaching type to a work path targets the layer's path for editing without mutating the source work path.
    #[test]
    fn attaching_type_selects_its_path_and_preserves_the_source() {
        let mut app = app(Tool::PathSelection);
        assert_eq!(app.ui.selected_path.as_deref(), Some("layer"));
        assert!(active_text(&app).is_some());
        let source = app.session.active().unwrap().doc.work_path.clone();
        app.ui.selected_path = Some("work".into());
        app.session.active_mut().unwrap().channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::VectorMask;
        assert!(active_text(&app).is_none());
        app.run("type.attachToPath", json!({"start":20})).unwrap();
        assert_eq!(app.session.active().unwrap().channel_view.target, photocraft_engine::channel_cmds::ChannelTarget::Composite);
        assert_eq!(app.ui.selected_path.as_deref(), Some("layer"));
        assert!(active_text(&app).is_some());
        assert_eq!(app.session.active().unwrap().doc.work_path, source);
    }

    /// Returns the text shape of the active text layer in test assertions.
    fn shape(app: &PhotocraftApp) -> TextShape {
        active_text(app).unwrap().shape.clone()
    }

    /// Returns the (start, end, flip) placement parameters of the active path text layer.
    fn placement(app: &PhotocraftApp) -> (f32, Option<f32>, bool) {
        let TextShape::Path { start, end, flip, .. } = shape(app) else { panic!("expected path") };
        (start, end, flip)
    }

    /// Returns the count of committed history entries in the active session.
    fn steps(app: &PhotocraftApp) -> usize {
        app.session.active().unwrap().history.entries().len()
    }

    /// Dispatches a pointer down event at document coordinates.
    fn down(app: &mut PhotocraftApp, x: f64, y: f64) {
        assert!(pointer(app, ToolEvent::Down { x, y, pressure: 1.0 }));
    }
    /// Dispatches a pointer move event to document coordinates.
    fn move_to(app: &mut PhotocraftApp, x: f64, y: f64) {
        assert!(pointer(app, ToolEvent::Move { x, y, pressure: 1.0 }));
    }
    /// Dispatches a pointer up event at document coordinates.
    fn up(app: &mut PhotocraftApp, x: f64, y: f64) {
        assert!(pointer(app, ToolEvent::Up { x, y }));
    }

    /// Verifies converting legacy single-point anchor alignment to explicit ranges preserves exact glyph rasterization coordinates.
    #[test]
    fn legacy_alignment_conversion_preserves_glyph_positions() {
        use photocraft_doc::text::{ParagraphStyle, TextAlign};
        use photocraft_doc::vector::Subpath;
        use photocraft_text::warp::Warp;
        let mut engine = photocraft_text::TextEngine::new();
        for closed in [false, true] {
            for flip in [false, true] {
                for align in [TextAlign::Left, TextAlign::Center, TextAlign::Right] {
                    let path = if closed {
                        Subpath::polygon(&[(0.0, 0.0), (200.0, 0.0), (200.0, 200.0), (0.0, 200.0)])
                    } else {
                        Subpath::polyline(&[(0.0, 0.0), (500.0, 0.0)])
                    };
                    let mut text = TextLayer {
                        text: "ABC".into(),
                        paragraphs: vec![photocraft_doc::text::ParagraphRun { len: 3, style: ParagraphStyle { align, ..Default::default() } }],
                        ..Default::default()
                    };
                    text.shape = TextShape::Path { path: path.clone(), start: 200.0, end: None, flip };
                    let old = engine.layout(&text, 72.0);
                    let old_warp = Warp::on_path_with_text_start(&text.shape, f64::from(old.lines[0].x0)).unwrap();
                    let geometry = PathGeometry::new(&path, flip).unwrap();
                    let range = effective_range(&text, &geometry).unwrap();
                    text.shape = TextShape::Path { path, start: range[0] as f32, end: Some(range[1] as f32), flip };
                    let new = engine.layout(&text, 72.0);
                    let new_warp = Warp::on_path_with_text_start(&text.shape, f64::from(new.lines[0].x0)).unwrap();
                    assert_eq!(old.glyphs.len(), new.glyphs.len());
                    for (old, new) in old.glyphs.iter().zip(&new.glyphs) {
                        let a = old_warp.apply(f64::from(old.x), f64::from(old.y));
                        let b = new_warp.apply(f64::from(new.x), f64::from(new.y));
                        assert!((a.0 - b.0).hypot(a.1 - b.1) < 0.02, "{closed} {flip} {align:?}: {a:?} != {b:?}");
                    }
                }
            }
        }
    }

    /// Ensures hidden text layers reject marker interactions and pointer gesture captures.
    #[test]
    fn hidden_active_layer_rejects_placement() {
        let mut app = app(Tool::PathSelection);
        let id = app.session.active().unwrap().active_layer.unwrap();
        app.session
            .edit("hide", |doc, _| {
                doc.layer_mut(id).unwrap().visible = false;
                Ok(())
            })
            .unwrap();
        assert!(active_text(&app).is_none());
        assert!(!pointer(&mut app, ToolEvent::Down { x: 40.0, y: 90.0, pressure: 1.0 }));
    }

    /// Proves clicking and releasing a placement marker without dragging preserves the document shape and appends no history step.
    #[test]
    fn legacy_click_and_release_preserves_model_and_history() {
        let mut app = app(Tool::PathSelection);
        app.run("type.setPathPlacement", json!({"end":null})).unwrap();
        let before = shape(&app);
        let count = steps(&app);
        down(&mut app, 40.0, 90.0);
        up(&mut app, 40.0, 90.0);
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), count);
    }

    /// Verifies marker hit testing and cursor feedback match across rotated, zoomed, and non-uniformly scaled canvas views.
    #[test]
    fn marker_cursor_and_press_agree_in_transformed_views() {
        for rotation in [0.0, 37.0, 90.0] {
            for mirror in [false, true] {
                let mut app = app(Tool::DirectSelection);
                let id = app.session.active().unwrap().active_layer.unwrap();
                app.session
                    .edit("skew text", |doc, _| {
                        let LayerContent::Text(text) = &mut doc.layer_mut(id).unwrap().content else { panic!("text") };
                        text.transform = photocraft_doc::Affine { m: [1.8, 0.3, 0.6, 0.9, 12.0, -7.0] };
                        Ok(())
                    })
                    .unwrap();
                app.ui.views[0].rotation = rotation;
                app.ui.view.flip_horizontal = mirror;
                let xf = ViewXform::active(&app).unwrap();
                let ends = markers(active_text(&app).unwrap(), &xf).unwrap();
                for (index, (point, _)) in ends.into_iter().enumerate() {
                    let screen = point;
                    assert_eq!(cursor(&mut app, &xf, screen), Some(Grab::of(Some(index))));
                    let doc = xf.to_doc(screen);
                    down(&mut app, doc[0], doc[1]);
                    assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, Some(index));
                    assert!(cancel(&mut app));
                }
            }
        }
    }

    /// Proves marker drags routed through explicit view transforms correctly update placement start and end boundaries.
    #[test]
    fn explicit_view_routes_marker_gesture() {
        let mut app = app(Tool::DirectSelection);
        let mut xf = ViewXform::active(&app).unwrap();
        xf.zoom = 2.0;
        xf.rotation = 37.0;
        xf.aspect = 1.5;
        let (point, _) = markers(active_text(&app).unwrap(), &xf).unwrap()[0];
        let from = xf.to_doc(point);
        assert!(pointer_with_view(&mut app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Some(&xf)));
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, Some(0));
        let to = [from[0] + 10.0, from[1]];
        assert!(pointer_with_view(&mut app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, Some(&xf)));
        assert!(pointer_with_view(&mut app, ToolEvent::Up { x: to[0], y: to[1] }, Some(&xf)));
        let (start, end, flip) = placement(&app);
        assert!((start - 30.0).abs() < 0.01);
        assert_eq!((end, flip), (Some(100.0), false));
    }

    /// Verifies crossing threshold for flipping path text uses perpendicular screen distance under affine shears.
    #[test]
    fn shear_flip_uses_perpendicular_screen_distance() {
        let mut app = app(Tool::PathSelection);
        let id = app.session.active().unwrap().active_layer.unwrap();
        app.session
            .edit("shear text", |doc, _| {
                let LayerContent::Text(text) = &mut doc.layer_mut(id).unwrap().content else { panic!("text") };
                text.transform = photocraft_doc::Affine { m: [1.0, 0.0, 10.0, 1.0, 0.0, 0.0] };
                Ok(())
            })
            .unwrap();
        let xf = ViewXform::active(&app).unwrap();
        let (point, _) = markers(active_text(&app).unwrap(), &xf).unwrap()[0];
        let from = xf.to_doc(point);
        assert!(pointer_with_view(&mut app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Some(&xf)));
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, Some(0));
        let text = active_text(&app).unwrap();
        let TextShape::Path { path, flip: path_flip, .. } = &text.shape else { panic!("text") };
        let geometry = PathGeometry::new(path, *path_flip).unwrap();
        let transform = text.transform;
        let arc = 40.0;
        let ((px, py), (tx, ty)) = geometry.sample(arc).unwrap();
        let normal = Point::new(-ty, tx);

        // Same side as initial press (- normal): 5 screen pts stays un-flipped
        let same_side = transform.apply(Point::new(px - normal.x * 5.0, py - normal.y * 5.0));
        assert!(pointer_with_view(&mut app, ToolEvent::Move { x: same_side.x, y: same_side.y, pressure: 1.0 }, Some(&xf)));
        assert!(!app.ui.type_path_drag.as_ref().unwrap().flipped);

        // Opposite side (+ normal), 3 screen points (below 4.0 threshold)
        let near = transform.apply(Point::new(px + normal.x * 3.0, py + normal.y * 3.0));
        assert!(pointer_with_view(&mut app, ToolEvent::Move { x: near.x, y: near.y, pressure: 1.0 }, Some(&xf)));
        assert!(!app.ui.type_path_drag.as_ref().unwrap().flipped);

        // Opposite side (+ normal), 5 screen points (above 4.0 threshold)
        let far = transform.apply(Point::new(px + normal.x * 5.0, py + normal.y * 5.0));
        assert!(pointer_with_view(&mut app, ToolEvent::Move { x: far.x, y: far.y, pressure: 1.0 }, Some(&xf)));
        assert!(app.ui.type_path_drag.as_ref().unwrap().flipped);
        assert!(pointer_with_view(&mut app, ToolEvent::Up { x: far.x, y: far.y }, Some(&xf)));
        assert!(placement(&app).2);
    }

    /// Ensures in-progress placement dragging caches transient preview snapshots without modifying committed history.
    #[test]
    fn preview_is_cached_and_does_not_change_history() {
        let mut app = app(Tool::PathSelection);
        let before = shape(&app);
        let n = steps(&app);
        down(&mut app, 40.0, 90.0);
        assert!(display_doc(&mut app, 0).is_none());
        move_to(&mut app, 50.0, 90.0);
        let (shown, key) = display_doc(&mut app, 0).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let LayerContent::Text(text) = &shown.layer(layer).unwrap().content else { panic!("expected text") };
        let TextShape::Path { start, .. } = &text.shape else { panic!("expected path") };
        assert_eq!(*start, 30.0);
        let (again, same_key) = display_doc(&mut app, 0).unwrap();
        assert_eq!(key, same_key);
        assert!(Arc::ptr_eq(&shown, &again));
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), n);
        assert!(cancel(&mut app));
        assert!(display_doc(&mut app, 0).is_none());
        down(&mut app, 40.0, 90.0);
        move_to(&mut app, 50.0, 90.0);
        up(&mut app, 50.0, 90.0);
        assert!(display_doc(&mut app, 0).is_none());
    }

    /// Proves preview damage calculations bound union regions across successive preview revisions and restore correctly on cancel.
    #[test]
    fn damage_tracks_preview_and_return() {
        let mut app = app(Tool::PathSelection);
        let st = app.session.active().unwrap();
        let (doc, revision) = (st.doc.id, st.revision);
        down(&mut app, 40.0, 90.0);
        move_to(&mut app, 50.0, 90.0);
        let (first_doc, first) = display_doc(&mut app, 0).unwrap();
        move_to(&mut app, 60.0, 90.0);
        let (second_doc, second) = display_doc(&mut app, 0).unwrap();
        let layer_id = app.session.active().unwrap().active_layer.unwrap();
        let original_bounds = app.session.active().unwrap().doc.layer(layer_id).unwrap().surface().map_or(Rect::EMPTY, |s| s.tile_bounds());
        assert!(!original_bounds.is_empty(), "original rendered text surface bounds must be non-empty");

        let first_bounds = first_doc.layer(layer_id).unwrap().surface().map_or(Rect::EMPTY, |s| s.tile_bounds());
        assert!(!first_bounds.is_empty(), "first preview rendered text surface bounds must be non-empty");

        let second_bounds = second_doc.layer(layer_id).unwrap().surface().map_or(Rect::EMPTY, |s| s.tile_bounds());
        assert!(!second_bounds.is_empty(), "second preview rendered text surface bounds must be non-empty");

        assert_eq!(damage(&app, doc, revision, 0, first), Some(original_bounds.union(&first_bounds)));
        assert_eq!(damage(&app, doc, revision, first, second), Some(first_bounds.union(&second_bounds)));
        assert!(cancel(&mut app));
        assert_eq!(damage(&app, doc, revision, second, 0), Some(second_bounds.union(&original_bounds)));
        assert_eq!(damage(&app, doc, revision + 1, second, 0), None);
    }

    /// Verifies dragging placement markers commits exactly one history step on release and cleanly rolls back on undo.
    #[test]
    fn marker_drags_commit_once_and_undo() {
        for tool in [Tool::PathSelection, Tool::DirectSelection] {
            for (from, to, expected) in
                [([40.0, 100.0], [50.0, 100.0], (30.0, Some(100.0), false)), ([120.0, 100.0], [130.0, 100.0], (20.0, Some(110.0), false))]
            {
                let mut app = app(tool);
                let before = shape(&app);
                let n = steps(&app);
                down(&mut app, from[0], from[1]);
                move_to(&mut app, to[0], to[1]);
                assert_eq!(shape(&app), before);
                assert_eq!(steps(&app), n);
                up(&mut app, to[0], to[1]);
                assert_eq!(placement(&app), expected);
                let TextShape::Path { path: old, .. } = &before else { panic!("expected path") };
                let TextShape::Path { path: new, .. } = shape(&app) else { panic!("expected path") };
                assert_eq!(*old, new);
                assert_eq!(steps(&app), n + 1);
                assert!(app.session.undo());
                assert_eq!(shape(&app), before);
            }
        }
    }

    /// Locates an interior document point on the glyph baseline of the active path text.
    fn body_point(app: &mut PhotocraftApp) -> Point {
        let id = app.session.active().unwrap().active_layer.unwrap();
        let (layout, _, _) = crate::type_tool::layout(app, id).unwrap();
        let cluster = layout.clusters.last().unwrap();
        let line = &layout.lines[cluster.line];
        let text = active_text(app).unwrap();
        let warp = photocraft_text::warp::Warp::on_path(&text.shape).unwrap();
        let (x, y) = warp.apply(f64::from(cluster.x + cluster.advance / 2.0), f64::from(line.baseline - line.ascent * 0.4));
        text.transform.apply(Point::new(x, y))
    }

    /// The markers sit on the path: the beginning at its start offset and the end at its end
    /// offset. Hovering the text body shows the double-headed cursor, and a gesture keeps the
    /// cursor of what it grabbed.
    #[test]
    fn markers_sit_on_the_path_and_body_hover_grabs_both_ends() {
        let mut app = app(Tool::PathSelection);
        let xf = ViewXform::active(&app).unwrap();
        let ends = markers(active_text(&app).unwrap(), &xf).unwrap();
        for ((point, _), x) in ends.into_iter().zip([40.0, 120.0]) {
            let doc = xf.to_doc(point);
            assert!((doc[0] - x).abs() < 1e-3 && (doc[1] - 100.0).abs() < 1e-3, "{doc:?}");
        }
        // Off the marker but on the path below it: nothing to grab there.
        assert!(marker_hit(ends, ends[0].0 + Vec2::new(0.0, MARKER_REACH + 1.0)).is_none());
        let point = body_point(&mut app);
        let screen = xf.to_screen(point.x as f32, point.y as f32);
        assert_eq!(cursor(&mut app, &xf, screen), Some(Grab::Body));
        down(&mut app, point.x, point.y);
        assert_eq!(cursor(&mut app, &xf, ends[1].0), Some(Grab::Body), "the gesture's grab wins while dragging");
        assert!(cancel(&mut app));
    }

    /// An open path's default end marker sits on its last anchor. Both selection tools move the
    /// marker there, as the reference editor does, even when Direct Selection has selected the anchor.
    #[test]
    fn a_marker_on_an_anchor_is_grabbed_by_both_selection_tools() {
        for tool in [Tool::DirectSelection, Tool::PathSelection] {
            for select_anchor in [false, true] {
                let mut app = app(tool);
                app.run("type.setPathPlacement", json!({"end": null})).unwrap();
                if select_anchor {
                    // A marquee round the last anchor selects it first.
                    let mods = egui::Modifiers::NONE;
                    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 195.0, y: 115.0, pressure: 1.0 }, mods);
                    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 170.0, y: 90.0, pressure: 1.0 }, mods);
                    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 170.0, y: 90.0 }, mods);
                }
                let xf = ViewXform::active(&app).unwrap();
                let end = markers(active_text(&app).unwrap(), &xf).unwrap()[1].0;
                let doc = xf.to_doc(end);
                assert!((doc[0] - 180.0).abs() < 1e-3, "end marker on the last anchor: {doc:?}");
                assert_eq!(cursor(&mut app, &xf, end), Some(Grab::End), "{tool:?} selected {select_anchor}");
                assert!(pointer(&mut app, ToolEvent::Down { x: doc[0], y: doc[1], pressure: 1.0 }), "{tool:?} selected {select_anchor}");
                assert_eq!(app.ui.type_path_drag.as_ref().map(|d| d.endpoint), Some(Some(1)), "{tool:?} selected {select_anchor}");
            }
        }
    }

    /// On a closed path whose range is a full lap the two markers coincide; pressing them moves the
    /// whole text round the path (the double cursor) instead of collapsing the lap.
    #[test]
    fn coincident_markers_on_a_full_lap_move_the_whole_text() {
        let mut app = app(Tool::PathSelection);
        let id = app.session.active().unwrap().active_layer.unwrap();
        app.session
            .edit("full lap", |doc, _| {
                let LayerContent::Text(text) = &mut doc.layer_mut(id).unwrap().content else { panic!("expected text") };
                text.shape = TextShape::Path {
                    path: photocraft_doc::vector::Subpath::polygon(&[(20.0, 20.0), (180.0, 20.0), (180.0, 180.0), (20.0, 180.0)]),
                    start: 20.0,
                    end: Some(660.0),
                    flip: false,
                };
                Ok(())
            })
            .unwrap();
        let xf = ViewXform::active(&app).unwrap();
        let ends = markers(active_text(&app).unwrap(), &xf).unwrap();
        assert!(ends[0].0.distance(ends[1].0) < 1.0, "{ends:?}");
        assert_eq!(cursor(&mut app, &xf, ends[0].0), Some(Grab::Body));
        let doc = xf.to_doc(ends[0].0);
        down(&mut app, doc[0], doc[1]);
        assert_eq!(app.ui.type_path_drag.as_ref().map(|d| (d.endpoint, d.either)), Some((None, false)));
        move_to(&mut app, doc[0] + 10.0, doc[1]);
        up(&mut app, doc[0] + 10.0, doc[1]);
        let (start, end, flip) = placement(&app);
        assert!((start - 30.0).abs() < 1e-3 && end.is_some_and(|end| (end - start - 640.0).abs() < 1e-3) && !flip, "{start} {end:?}");
    }

    /// Text attached to a closed path fills one lap (`end: None`), so its markers coincide. A press
    /// just ahead along the path grabs the beginning and one just behind grabs the end; dragging
    /// either shortens the lap without reshaping the path, and the centre still moves the whole
    /// text.
    #[test]
    fn default_full_lap_markers_can_be_pulled_apart() {
        let square = photocraft_doc::vector::Subpath::polygon(&[(20.0, 20.0), (180.0, 20.0), (180.0, 180.0), (20.0, 180.0)]);
        // Ahead of the point: the beginning moves forward. Behind it: the end moves back.
        for (offset, grab, drag, expected) in [(4.0, Grab::Start, 30.0, [130.0, 740.0]), (-4.0, Grab::End, -30.0, [100.0, 710.0])] {
            let mut app = app(Tool::PathSelection);
            let id = app.session.active().unwrap().active_layer.unwrap();
            app.session
                .edit("closed path", |doc, _| {
                    let LayerContent::Text(text) = &mut doc.layer_mut(id).unwrap().content else { panic!("expected text") };
                    // Start mid-edge, so the drags below stay on the top edge.
                    text.shape = TextShape::Path { path: square.clone(), start: 100.0, end: None, flip: false };
                    Ok(())
                })
                .unwrap();
            let n = steps(&app);
            let xf = ViewXform::active(&app).unwrap();
            let ends = markers(active_text(&app).unwrap(), &xf).unwrap();
            assert!(ends[0].0.distance(ends[1].0) < 1.0, "{ends:?}");
            let (point, tangent) = ends[0];
            assert_eq!(cursor(&mut app, &xf, point), Some(Grab::Body));
            let press = point + tangent * offset;
            assert_eq!(cursor(&mut app, &xf, press), Some(grab));
            let from = xf.to_doc(press);
            down(&mut app, from[0], from[1]);
            assert_eq!(Grab::of(app.ui.type_path_drag.as_ref().unwrap().endpoint), grab);
            let to = xf.to_doc(press + tangent * drag);
            move_to(&mut app, to[0], to[1]);
            up(&mut app, to[0], to[1]);
            let (start, end, flip) = placement(&app);
            let end = end.expect("an explicit range");
            assert!((start - expected[0] as f32).abs() < 0.01 && (end - expected[1] as f32).abs() < 0.01, "{grab:?}: {start} {end}");
            assert!(end - start < 640.0 - 1.0, "shorter than one lap");
            assert!(!flip, "a drag along the path does not flip");
            let TextShape::Path { path, .. } = shape(&app) else { panic!("expected path") };
            assert_eq!(path, square, "the path is not reshaped");
            assert_eq!(steps(&app), n + 1);
        }
    }

    /// Proves dragging text body translates the placement range even when one boundary touches the open path endpoint.
    #[test]
    fn body_drag_can_move_text_when_range_reaches_the_open_endpoint() {
        let mut app = app(Tool::PathSelection);
        app.run("type.setPathPlacement", json!({"end": null})).unwrap();
        for expected in [30.0, 40.0] {
            let point = body_point(&mut app);
            down(&mut app, point.x, point.y);
            assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, None);
            move_to(&mut app, point.x + 10.0, point.y);
            up(&mut app, point.x + 10.0, point.y);
            assert_eq!(placement(&app), (expected, Some(160.0), false));
        }
    }

    /// Verifies dragging the text body shifts the entire range by delta and commits a single undoable command.
    #[test]
    fn body_drag_moves_range_once_and_undoes() {
        for tool in [Tool::PathSelection, Tool::DirectSelection] {
            let mut app = app(tool);
            let before = shape(&app);
            let n = steps(&app);
            let point = body_point(&mut app);
            let text = active_text(&app).unwrap();
            let xf = ViewXform::active(&app).unwrap();
            assert!(marker_hit(markers(text, &xf).unwrap(), xf.to_screen(point.x as f32, point.y as f32)).is_none());
            down(&mut app, point.x, point.y);
            assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, None);
            move_to(&mut app, point.x + 10.0, point.y);
            up(&mut app, point.x + 10.0, point.y);
            assert_eq!(placement(&app), (30.0, Some(110.0), false));
            let TextShape::Path { path: old, .. } = &before else { panic!("expected path") };
            let TextShape::Path { path: new, .. } = shape(&app) else { panic!("expected path") };
            assert_eq!(*old, new);
            assert_eq!(steps(&app), n + 1);
            assert!(app.session.undo());
            assert_eq!(shape(&app), before);
        }
    }

    /// Ensures position locks on text layers reject marker drag gestures and command executions.
    #[test]
    fn position_lock_rejects_marker_and_command() {
        for tool in [Tool::PathSelection, Tool::DirectSelection] {
            let mut app = app(tool);
            let id = app.session.active().unwrap().active_layer.unwrap();
            app.session
                .edit("lock position", |doc, _| {
                    doc.layer_mut(id).unwrap().locks.position = true;
                    Ok(())
                })
                .unwrap();
            let before = shape(&app);
            let n = steps(&app);
            let xf = ViewXform::active(&app).unwrap();
            let (point, _) = markers(active_text(&app).unwrap(), &xf).unwrap()[0];
            let screen = point;
            let doc = xf.to_doc(screen);
            assert!(!pointer(&mut app, ToolEvent::Down { x: doc[0], y: doc[1], pressure: 1.0 }));
            assert!(app.session.execute("type.setPathPlacement", json!({"start":30})).is_err());
            assert_eq!(shape(&app), before);
            assert_eq!(steps(&app), n);
        }
    }

    /// Proves dragging past the curve flip threshold reverses side orientation and remaps start/end intervals.
    #[test]
    fn crossing_the_path_flips_and_remaps_the_interval() {
        for tool in [Tool::PathSelection, Tool::DirectSelection] {
            let mut app = app(tool);
            down(&mut app, 40.0, 90.0);
            move_to(&mut app, 40.0, 110.0);
            up(&mut app, 40.0, 110.0);
            assert_eq!(placement(&app), (60.0, Some(140.0), true));
        }
    }

    /// Verifies cancelling or switching documents cleanly clears active drag state without committing changes.
    #[test]
    fn cancel_and_document_switch_discard_the_gesture() {
        for switch in [false, true] {
            let mut app = app(Tool::PathSelection);
            let before = shape(&app);
            let n = steps(&app);
            down(&mut app, 40.0, 90.0);
            move_to(&mut app, 50.0, 90.0);
            if switch {
                let source = app.session.active().unwrap().doc.clone();
                app.session.add_document((*source).clone(), None);
                let new_before = shape(&app);
                let new_steps = steps(&app);
                up(&mut app, 50.0, 90.0);
                assert_eq!(shape(&app), new_before);
                assert_eq!(steps(&app), new_steps);
                assert_eq!(app.session.documents()[0].history.entries().len(), n);
            } else {
                assert!(cancel(&mut app));
                assert_eq!(shape(&app), before);
                assert_eq!(steps(&app), n);
            }
        }
    }
    /// Proves reallocating document instance memory invalidates stale weak pointers and cancels active drag gestures.
    #[test]
    fn replacing_snapshot_with_same_id_and_revision_cancels_capture() {
        let mut app = app(Tool::PathSelection);
        let before = shape(&app);
        let count = steps(&app);
        down(&mut app, 40.0, 90.0);
        move_to(&mut app, 50.0, 90.0);
        let st = app.session.active_mut().unwrap();
        let id = st.doc.id;
        let revision = st.revision;
        let original = st.doc.clone();
        st.doc = Arc::new((*original).clone());
        assert_eq!(st.doc.id, id);
        assert_eq!(st.revision, revision);
        assert!(!Arc::ptr_eq(&st.doc, &original));
        cancel_stale(&mut app);
        assert!(app.ui.type_path_drag.is_none());
        assert!(!pointer(&mut app, ToolEvent::Up { x: 50.0, y: 90.0 }));
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), count);
    }

    /// Ensures clicking on a closed path range exceeding one lap preserves the full multi-lap span without truncation.
    #[test]
    fn closed_range_over_one_lap_click_preserves_range_and_history() {
        let mut app = app(Tool::PathSelection);
        let st = app.session.active_mut().unwrap();
        let id = st.active_layer.unwrap();
        let layer = Arc::make_mut(&mut st.doc).layer_mut(id).unwrap();
        let LayerContent::Text(text) = &mut layer.content else { panic!("expected text") };
        text.shape = TextShape::Path {
            path: photocraft_doc::vector::Subpath::polygon(&[(20.0, 20.0), (180.0, 20.0), (180.0, 180.0), (20.0, 180.0)]),
            start: 20.0,
            end: Some(700.0),
            flip: false,
        };
        let before = shape(&app);
        let count = steps(&app);
        let xf = ViewXform::active(&app).unwrap();
        let ends = markers(active_text(&app).unwrap(), &xf).unwrap();
        let tip = ends[1].0;
        let point = xf.to_doc(tip);
        down(&mut app, point[0], point[1]);
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, Some(1));
        up(&mut app, point[0], point[1]);
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), count);
    }

    /// Proves dragging across self-intersecting path crossings stays locked to the current branch based on previous arc length.
    #[test]
    fn pointer_crossing_gesture_stays_on_branch() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.sync_views();
        app.ui.tool = Tool::PathSelection;
        app.run("path.set", json!({"name":"work", "path":{"subpaths":[{"closed":true,"knots":[[20,20],[180,180],[20,180],[180,20]]}]}})).unwrap();
        app.ui.selected_path = Some("work".into());
        app.run("type.create", json!({"text":"ABC", "x":20, "y":20})).unwrap();
        app.run("type.attachToPath", json!({"start":20})).unwrap();
        app.run("type.setPathPlacement", json!({"end":80})).unwrap();
        app.ui.views[0].zoom = 1.0;

        let xf = ViewXform::active(&app).unwrap();
        let ends = markers(active_text(&app).unwrap(), &xf).unwrap();
        let tip = ends[1].0;
        let from = xf.to_doc(tip);
        assert!(pointer_with_view(&mut app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Some(&xf)));
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, Some(1));
        assert!(pointer_with_view(&mut app, ToolEvent::Move { x: 100.0, y: 100.0, pressure: 1.0 }, Some(&xf)));
        assert!(pointer_with_view(&mut app, ToolEvent::Move { x: 140.0, y: 140.0, pressure: 1.0 }, Some(&xf)));
        assert!(pointer_with_view(&mut app, ToolEvent::Up { x: 140.0, y: 140.0 }, Some(&xf)));

        let (start, end, flip) = placement(&app);
        assert_eq!(start, 20.0);
        let end = end.expect("explicit end");
        assert!((end - 169.71).abs() < 1.0, "end should stay on branch: {end}");
        assert!(!flip);
    }

    /// Verifies that hiding an ancestor layer group excludes child path text from marker rendering and pointer captures.
    #[test]
    fn hidden_ancestor_excludes_markers_and_capture() {
        use photocraft_doc::LayerId;
        let mut app = app(Tool::PathSelection);
        let text_id = app.session.active().unwrap().active_layer.unwrap();
        let group_id = LayerId(app.run("layer.groupLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
        app.run("layer.select", json!({"layer": text_id.0, "mode": "replace"})).unwrap();
        assert_eq!(app.session.active().unwrap().active_layer, Some(text_id));
        assert!(active_text(&app).is_some());

        app.session
            .edit("hide group", |doc, _| {
                doc.layer_mut(group_id).unwrap().visible = false;
                Ok(())
            })
            .unwrap();

        assert!(active_text(&app).is_none());
        let xf = ViewXform::active(&app).unwrap();
        assert!(cursor(&mut app, &xf, egui::pos2(40.0, 90.0)).is_none());
        assert!(!pointer(&mut app, ToolEvent::Down { x: 40.0, y: 90.0, pressure: 1.0 }));
        assert!(app.ui.type_path_drag.is_none());
    }

    /// Proves overflow marker caches invalidate when a document Arc is replaced with modified content at the same revision.
    #[test]
    fn overflow_cache_invalidates_on_replacement_arc_with_same_id_and_revision() {
        let mut app = app(Tool::PathSelection);
        let ctx = egui::Context::default();
        let text = active_text(&app).unwrap().clone();
        let initial = overflows(&app, &ctx, &text);
        assert!(!initial, "short text should not overflow range [20, 100]");

        // 1. Replacement Arc with same docid/revision but different text content:
        let st = app.session.active_mut().unwrap();
        let doc_id = st.doc.id;
        let revision = st.revision;
        let original_arc = st.doc.clone();
        let mut modified = (*original_arc).clone();
        let layer_id = st.active_layer.unwrap();
        let LayerContent::Text(t) = &mut modified.layer_mut(layer_id).unwrap().content else { panic!("expected text") };
        t.text = "A very long sentence on a path that is guaranteed to overflow the assigned span".into();
        st.doc = Arc::new(modified);
        assert_eq!(st.doc.id, doc_id);
        assert_eq!(st.revision, revision);
        assert!(!Arc::ptr_eq(&st.doc, &original_arc));

        let new_text = active_text(&app).unwrap().clone();
        let updated = overflows(&app, &ctx, &new_text);
        assert!(updated, "overflow cache must invalidate on replacement Arc with different text content");

        // 2. Replacement Arc with same docid/revision but different range content:
        let st = app.session.active_mut().unwrap();
        let prev_arc = st.doc.clone();
        let mut modified = (*prev_arc).clone();
        let LayerContent::Text(t) = &mut modified.layer_mut(layer_id).unwrap().content else { panic!("expected text") };
        t.text = "ABC".into();
        let TextShape::Path { end, .. } = &mut t.shape else { panic!("expected path") };
        *end = Some(21.0);
        st.doc = Arc::new(modified);
        assert_eq!(st.doc.id, doc_id);
        assert_eq!(st.revision, revision);
        assert!(!Arc::ptr_eq(&st.doc, &prev_arc));

        let tiny_range_text = active_text(&app).unwrap().clone();
        let tiny_overflow = overflows(&app, &ctx, &tiny_range_text);
        assert!(tiny_overflow, "overflow cache must invalidate on replacement Arc with tiny range");

        // 3. Expanding range on replacement Arc so it no longer overflows:
        let st = app.session.active_mut().unwrap();
        let prev_arc = st.doc.clone();
        let mut modified = (*prev_arc).clone();
        let LayerContent::Text(t) = &mut modified.layer_mut(layer_id).unwrap().content else { panic!("expected text") };
        let TextShape::Path { end, .. } = &mut t.shape else { panic!("expected path") };
        *end = Some(180.0);
        st.doc = Arc::new(modified);
        assert_eq!(st.doc.id, doc_id);
        assert_eq!(st.revision, revision);

        let wide_range_text = active_text(&app).unwrap().clone();
        let wide_overflow = overflows(&app, &ctx, &wide_range_text);
        assert!(!wide_overflow, "overflow cache must invalidate on replacement Arc with expanded range");
    }

    /// Ensures releasing a placement drag outside canvas bounds cleanly commits exactly once and updates placement.
    #[test]
    fn ordinary_offcanvas_up_commits_once() {
        let mut app = app(Tool::PathSelection);
        let before = shape(&app);
        let n = steps(&app);
        down(&mut app, 40.0, 90.0);
        assert!(app.ui.type_path_drag.is_some());
        move_to(&mut app, 500.0, 500.0);
        up(&mut app, 500.0, 500.0);
        assert!(app.ui.type_path_drag.is_none(), "drag ended on release");
        assert_eq!(steps(&app), n + 1, "exactly one commit step");
        assert_ne!(shape(&app), before, "placement was modified");
        assert!(!pointer(&mut app, ToolEvent::Up { x: 500.0, y: 500.0 }));
        assert_eq!(steps(&app), n + 1);
        assert!(app.session.undo());
        assert_eq!(shape(&app), before);
    }

    /// Proves non-integer path lengths whose `f32` conversion rounds upward keep placement markers visible without clamping errors.
    #[test]
    fn type_path_placement_noninteger_line_rounding_upward_keeps_markers_visible() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.sync_views();
        app.ui.tool = Tool::PathSelection;
        // Line from (20, 20) to (27, 27) with length 7 * sqrt2 ≈ 9.899495.
        app.run("path.set", json!({"name":"work", "path":{"subpaths":[{"closed":false,"knots":[[20,20],[27,27]]}]}})).unwrap();
        app.ui.selected_path = Some("work".into());
        app.run("type.create", json!({"text":"AB", "x":20, "y":20})).unwrap();
        app.run("type.attachToPath", json!({"start":0})).unwrap();
        app.ui.views[0].zoom = 1.0;

        let xf = ViewXform::active(&app).unwrap();
        let geom = PathGeometry::new(&photocraft_doc::vector::Subpath::polyline(&[(20.0, 20.0), (27.0, 27.0)]), false).unwrap();
        let length_f32 = geom.length() as f32;
        assert!(f64::from(length_f32) > geom.length(), "scale 7 * sqrt2 must round up in f32");

        // Endpoint command keeps markers visible.
        app.run("type.setPathPlacement", json!({"end": length_f32})).unwrap();
        assert!(markers(active_text(&app).unwrap(), &xf).is_some(), "markers must remain visible after endpoint command");

        // Zero-capacity endpoint range keeps markers visible.
        app.run("type.setPathPlacement", json!({"start": length_f32, "end": length_f32})).unwrap();
        assert!(markers(active_text(&app).unwrap(), &xf).is_some(), "zero-capacity endpoint markers must remain visible");

        // Flipped case keeps markers visible.
        app.run("type.setPathPlacement", json!({"start": 0.0, "end": length_f32, "flip": true})).unwrap();
        assert!(markers(active_text(&app).unwrap(), &xf).is_some(), "flipped endpoint markers must remain visible");

        // Reset to initial placement and test actual drag gesture to end.
        app.run("type.setPathPlacement", json!({"start": 0.0, "end": 5.0, "flip": false})).unwrap();
        let ends = markers(active_text(&app).unwrap(), &xf).unwrap();
        let tip = ends[1].0;
        let from = xf.to_doc(tip);
        assert!(pointer_with_view(&mut app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Some(&xf)));
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, Some(1));
        // Drag far past the endpoint.
        assert!(pointer_with_view(&mut app, ToolEvent::Move { x: 50.0, y: 50.0, pressure: 1.0 }, Some(&xf)));
        assert!(pointer_with_view(&mut app, ToolEvent::Up { x: 50.0, y: 50.0 }, Some(&xf)));
        assert!(markers(active_text(&app).unwrap(), &xf).is_some(), "markers must remain visible after actual drag release at endpoint");
    }

    /// Verifies viewport ownership guards prevent secondary viewports from discarding an active drag belonging to another viewport.
    #[test]
    fn cancel_missing_release_viewport_ownership() {
        let mut app = app(Tool::PathSelection);
        let owner_vp = egui::ViewportId::from_hash_of("owner-viewport");
        let nonowner_vp = egui::ViewportId::from_hash_of("nonowner-viewport");

        let ctx_owner = egui::Context::default();
        let ctx_nonowner = egui::Context::default();

        let pos = egui::pos2(1000.0, 1000.0);

        let make_raw = |viewport: egui::ViewportId, focused: bool, pressed: bool| {
            let mut raw = egui::RawInput {
                viewport_id: viewport,
                focused,
                events: if pressed {
                    vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }]
                } else {
                    vec![]
                },
                ..Default::default()
            };
            raw.viewports.insert(viewport, egui::ViewportInfo { focused: Some(focused), ..Default::default() });
            raw
        };

        // Set owner ctx focused with primary down.
        ctx_owner.begin_pass(make_raw(owner_vp, true, true));

        // Set nonowner ctx unfocused without primary pressed.
        ctx_nonowner.begin_pass(make_raw(nonowner_vp, false, false));

        // Start drag in owner viewport.
        let xf = ViewXform::active(&app).unwrap();
        let ends = markers(active_text(&app).unwrap(), &xf).unwrap();
        let tip = ends[1].0;
        let from = xf.to_doc(tip);
        assert!(pointer_with_context(&mut app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Some(&xf), Some(owner_vp)));
        assert!(app.ui.type_path_drag.is_some());
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().viewport, Some(owner_vp));

        // Verify separate viewport identity and focus/held assertions.
        assert_eq!(ctx_owner.viewport_id(), owner_vp);
        assert_eq!(ctx_nonowner.viewport_id(), nonowner_vp);
        assert!(ctx_owner.input(|i| i.focused && i.pointer.primary_down()));
        assert!(ctx_nonowner.input(|i| !i.focused && !i.pointer.primary_down()));

        // Unfocused nonowner cannot cancel.
        cancel_missing_release(&mut app, &ctx_nonowner);
        assert!(app.ui.type_path_drag.is_some(), "unfocused nonowner must not cancel owner drag");

        // Focused held owner stays.
        cancel_missing_release(&mut app, &ctx_owner);
        assert!(app.ui.type_path_drag.is_some(), "focused held owner must retain drag");

        ctx_owner.end_pass().textures_delta.clear();

        // Owner focus loss cancels.
        ctx_owner.begin_pass(make_raw(owner_vp, false, false));
        assert_eq!(ctx_owner.viewport_id(), owner_vp);
        assert!(ctx_owner.input(|i| !i.focused));
        cancel_missing_release(&mut app, &ctx_owner);
        assert!(app.ui.type_path_drag.is_none(), "owner focus loss must cancel drag");
        ctx_owner.end_pass().textures_delta.clear();

        // Automation drag without owner retains cancellation on lost release.
        assert!(pointer(&mut app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }));
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().viewport, None);
        cancel_missing_release(&mut app, &ctx_nonowner);
        assert!(app.ui.type_path_drag.is_none(), "automation fallback cleans up without owner");
        ctx_nonowner.end_pass().textures_delta.clear();
    }

    #[test]
    fn owned_viewport_rejects_foreign_or_none_events_and_finishes() {
        let mut app = app(Tool::PathSelection);
        let owner_vp = egui::ViewportId::from_hash_of("owner-viewport");
        let nonowner_vp = egui::ViewportId::from_hash_of("nonowner-viewport");

        let xf = ViewXform::active(&app).unwrap();
        let ends = markers(active_text(&app).unwrap(), &xf).unwrap();
        let tip = ends[1].0;
        let from = xf.to_doc(tip);

        assert!(pointer_with_context(&mut app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Some(&xf), Some(owner_vp)));
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().viewport, Some(owner_vp));
        let history_count = steps(&app);

        assert!(!pointer_with_context(&mut app, ToolEvent::Move { x: from[0] + 10.0, y: from[1] + 10.0, pressure: 1.0 }, Some(&xf), None));
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().viewport, Some(owner_vp));

        assert!(!pointer_with_context(&mut app, ToolEvent::Move { x: from[0] + 10.0, y: from[1] + 10.0, pressure: 1.0 }, Some(&xf), Some(nonowner_vp)));
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().viewport, Some(owner_vp));

        assert!(!pointer_with_context(&mut app, ToolEvent::Up { x: from[0] + 10.0, y: from[1] + 10.0 }, Some(&xf), None));
        assert!(app.ui.type_path_drag.is_some());
        assert_eq!(steps(&app), history_count);

        assert!(!pointer_with_context(&mut app, ToolEvent::Up { x: from[0] + 10.0, y: from[1] + 10.0 }, Some(&xf), Some(nonowner_vp)));
        assert!(app.ui.type_path_drag.is_some());
        assert_eq!(steps(&app), history_count);

        assert!(pointer_with_context(&mut app, ToolEvent::Move { x: from[0] + 10.0, y: from[1] + 10.0, pressure: 1.0 }, Some(&xf), Some(owner_vp)));
        assert!(pointer_with_context(&mut app, ToolEvent::Up { x: from[0] + 10.0, y: from[1] + 10.0 }, Some(&xf), Some(owner_vp)));
        assert!(app.ui.type_path_drag.is_none());
        assert_eq!(steps(&app), history_count + 1);

        let ends = markers(active_text(&app).unwrap(), &xf).unwrap();
        let tip = ends[1].0;
        let from_updated = xf.to_doc(tip);

        assert!(pointer_with_context(&mut app, ToolEvent::Down { x: from_updated[0], y: from_updated[1], pressure: 1.0 }, Some(&xf), None));
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().viewport, None);
        assert!(pointer_with_context(&mut app, ToolEvent::Move { x: from_updated[0] + 10.0, y: from_updated[1] + 10.0, pressure: 1.0 }, Some(&xf), None));
        assert!(pointer_with_context(&mut app, ToolEvent::Up { x: from_updated[0] + 10.0, y: from_updated[1] + 10.0 }, Some(&xf), None));
        assert!(app.ui.type_path_drag.is_none());
    }

    #[test]
    fn marker_geometry_cache_reuse_and_invalidation() {
        let mut app = app(Tool::PathSelection);
        let text = active_text(&app).unwrap().clone();

        let geom1 = active_path_geometry(&app, &text).expect("initial geometry");
        let initial_length = geom1.length();
        assert!(app.ui.type_path_geometry_cache.0.lock().unwrap().is_some());

        let geom2 = active_path_geometry(&app, &text).expect("cached geometry");
        assert!(Arc::ptr_eq(&geom1, &geom2));

        let st = app.session.active_mut().unwrap();
        let cloned_doc = (*st.doc).clone();
        st.doc = Arc::new(cloned_doc);
        let geom_doc_replaced = active_path_geometry(&app, &text).expect("recomputed geometry");
        assert!(!Arc::ptr_eq(&geom1, &geom_doc_replaced));

        app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[0, 0], [200, 0]]}]}})).unwrap();
        app.run("type.attachToPath", json!({"start": 0})).unwrap();
        let text_after_path = active_text(&app).unwrap();
        let geom_new_path = active_path_geometry(&app, text_after_path).expect("new path geometry");
        assert_ne!(geom_new_path.length(), initial_length);
        assert!(!Arc::ptr_eq(&geom_doc_replaced, &geom_new_path));

        app.run("type.setPathPlacement", json!({"flip": true})).unwrap();
        let text_after_flip = active_text(&app).unwrap();
        let geom_flipped = active_path_geometry(&app, text_after_flip).expect("flipped geometry");
        assert!(!Arc::ptr_eq(&geom_new_path, &geom_flipped));
        let geom_flipped2 = active_path_geometry(&app, text_after_flip).expect("cached flipped geometry");
        assert!(Arc::ptr_eq(&geom_flipped, &geom_flipped2));
        assert_eq!(geom_flipped.sample(10.0).unwrap().1.0, -geom_new_path.sample(10.0).unwrap().1.0);
    }

    #[test]
    fn pointer_perturbed_crossing_stays_on_branch_and_releases() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.sync_views();
        app.ui.tool = Tool::PathSelection;
        app.run("path.set", json!({"name":"work", "path":{"subpaths":[{"closed":true,"knots":[[20,20],[180.2,179.8],[20,179.8],[180.2,20]]}]}})).unwrap();
        app.ui.selected_path = Some("work".into());
        app.run("type.create", json!({"text":"ABC", "x":20, "y":20})).unwrap();
        app.run("type.attachToPath", json!({"start":20})).unwrap();
        app.run("type.setPathPlacement", json!({"end":80})).unwrap();
        app.ui.views[0].zoom = 1.0;

        let xf = ViewXform::active(&app).unwrap();
        let ends = markers(active_text(&app).unwrap(), &xf).unwrap();
        let tip = ends[1].0;
        let from = xf.to_doc(tip);
        assert!(pointer_with_view(&mut app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Some(&xf)));
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, Some(1));
        assert!(pointer_with_view(&mut app, ToolEvent::Move { x: 100.1, y: 99.9, pressure: 1.0 }, Some(&xf)));
        assert!(pointer_with_view(&mut app, ToolEvent::Move { x: 140.0, y: 140.0, pressure: 1.0 }, Some(&xf)));
        assert!(pointer_with_view(&mut app, ToolEvent::Up { x: 140.0, y: 140.0 }, Some(&xf)));

        let (start, end, flip) = placement(&app);
        assert_eq!(start, 20.0);
        let end = end.expect("explicit end");
        assert!((end - 169.8).abs() < 2.0, "end must stay on branch 1 without jumping: {end}");
        assert!(!flip);
    }

    /// Proves markers and placement gestures work on shortened open geometry with stale explicit end, cancel preserves the original, and edits commit valid ranges.
    #[test]
    fn shortened_path_stale_explicit_end_preserves_markers_cancellation_and_commits_valid_range() {
        let mut app = app(Tool::PathSelection);
        let id = app.session.active().unwrap().active_layer.unwrap();
        // Shorten path from length 160 (20 to 180) to length 60 (20 to 80).
        let shortened = photocraft_doc::vector::Subpath::polyline(&[(20.0, 100.0), (80.0, 100.0)]);
        app.session
            .edit("shorten path", |doc, _| {
                let LayerContent::Text(text) = &mut doc.layer_mut(id).unwrap().content else { panic!("expected text") };
                let TextShape::Path { path, .. } = &mut text.shape else { panic!("expected path") };
                *path = shortened;
                Ok(())
            })
            .unwrap();

        // Stored document retains stale explicit end (100.0) past shortened path length (60.0).
        let (stored_start, stored_end, stored_flip) = placement(&app);
        assert_eq!(stored_start, 20.0);
        assert_eq!(stored_end, Some(100.0));
        assert!(!stored_flip);

        // Markers remain visible on shortened path with bounded effective range.
        let xf = ViewXform::active(&app).unwrap();
        let ends = markers(active_text(&app).unwrap(), &xf).expect("markers must remain visible after shortening");
        let tip0 = ends[0].0;
        let tip1 = ends[1].0;
        assert_eq!(cursor(&mut app, &xf, tip0), Some(Grab::Start));
        assert_eq!(cursor(&mut app, &xf, tip1), Some(Grab::End));

        // Click and release without dragging preserves stored document and history count.
        let before = shape(&app);
        let initial_steps = steps(&app);
        let doc0 = xf.to_doc(tip0);
        down(&mut app, doc0[0], doc0[1]);
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, Some(0));
        up(&mut app, doc0[0], doc0[1]);
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), initial_steps);

        // Cancelling drag gesture preserves stored document and history count.
        down(&mut app, doc0[0], doc0[1]);
        assert!(cancel(&mut app));
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), initial_steps);

        // Dragging end marker commits a valid bounded endpoint on the shortened path.
        let doc1 = xf.to_doc(tip1);
        down(&mut app, doc1[0], doc1[1]);
        assert_eq!(app.ui.type_path_drag.as_ref().unwrap().endpoint, Some(1));
        // The layer is translated, so move relative to the press: 10 px back along the path.
        move_to(&mut app, doc1[0] - 10.0, doc1[1]);
        up(&mut app, doc1[0] - 10.0, doc1[1]);
        assert_eq!(steps(&app), initial_steps + 1);
        let (new_start, new_end, new_flip) = placement(&app);
        assert_eq!(new_start, 20.0);
        assert_eq!(new_end, Some(50.0));
        assert!(!new_flip);

        // Undo cleanly rolls back to the preserved original stale document.
        assert!(app.session.undo());
        assert_eq!(shape(&app), before);
    }

    /// Proves markers remain visible and start can be dragged onto the path when both endpoints exceed shortened path length.
    #[test]
    fn shortened_path_start_beyond_new_length_preserves_markers_cancellation_and_commits_valid_range() {
        let mut app = app(Tool::PathSelection);
        let id = app.session.active().unwrap().active_layer.unwrap();
        // Shorten path to length 40 (20 to 60) and place stale start at 50.0 and end at 80.0.
        let shortened = photocraft_doc::vector::Subpath::polyline(&[(20.0, 100.0), (60.0, 100.0)]);
        app.session
            .edit("shorten path beyond start", |doc, _| {
                let LayerContent::Text(text) = &mut doc.layer_mut(id).unwrap().content else { panic!("expected text") };
                text.shape = TextShape::Path { path: shortened, start: 50.0, end: Some(80.0), flip: false };
                Ok(())
            })
            .unwrap();

        // Stored document retains stale range beyond new length.
        let (stored_start, stored_end, stored_flip) = placement(&app);
        assert_eq!(stored_start, 50.0);
        assert_eq!(stored_end, Some(80.0));
        assert!(!stored_flip);

        // Markers remain visible, bounded to the endpoint of the shortened path.
        let xf = ViewXform::active(&app).unwrap();
        let ends = markers(active_text(&app).unwrap(), &xf).expect("markers must remain visible when start is beyond length");
        let tip0 = ends[0].0;
        let tip1 = ends[1].0;
        // Both are clamped to the path's end, so they coincide: either can be pulled out.
        assert!(tip0.distance(tip1) < 1.0);
        assert_eq!(cursor(&mut app, &xf, tip0), Some(Grab::Body));
        assert_eq!(cursor(&mut app, &xf, tip1), Some(Grab::Body));

        // Click and release or cancel preserves original stored shape.
        let before = shape(&app);
        let initial_steps = steps(&app);
        let doc0 = xf.to_doc(tip0);
        down(&mut app, doc0[0], doc0[1]);
        assert_eq!(app.ui.type_path_drag.as_ref().map(|d| (d.endpoint, d.either)), Some((None, true)));
        assert!(cancel(&mut app));
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), initial_steps);

        down(&mut app, doc0[0], doc0[1]);
        up(&mut app, doc0[0], doc0[1]);
        assert_eq!(shape(&app), before);
        assert_eq!(steps(&app), initial_steps);

        // Dragging the coincident markers backward pulls the start out and commits valid endpoints.
        down(&mut app, doc0[0], doc0[1]);
        // The layer is translated, so move relative to the press: 25 px back along the path.
        move_to(&mut app, doc0[0] - 25.0, doc0[1]);
        assert_eq!(app.ui.type_path_drag.as_ref().map(|d| (d.endpoint, d.either)), Some((Some(0), false)));
        assert_eq!(cursor(&mut app, &xf, tip0), Some(Grab::Start));
        up(&mut app, doc0[0] - 25.0, doc0[1]);
        assert_eq!(steps(&app), initial_steps + 1);
        let (new_start, new_end, new_flip) = placement(&app);
        assert_eq!(new_start, 15.0);
        assert_eq!(new_end, Some(40.0));
        assert!(!new_flip);

        // Undo restores original shape.
        assert!(app.session.undo());
        assert_eq!(shape(&app), before);
    }

    /// Verifies malformed, non-finite, and inverted ranges continue to be rejected by effective_range.
    #[test]
    fn effective_range_safeguards_reject_malformed_and_inverted_ranges() {
        use photocraft_doc::vector::Subpath;
        let path = Subpath::polyline(&[(0.0, 0.0), (100.0, 0.0)]);
        let geometry = PathGeometry::new(&path, false).unwrap();
        let make_text = |start: f32, end: Option<f32>| TextLayer {
            text: "ABC".into(),
            shape: TextShape::Path { path: path.clone(), start, end, flip: false },
            ..Default::default()
        };

        // Inverted explicit range (start > end) must return None.
        assert!(effective_range(&make_text(50.0, Some(20.0)), &geometry).is_none());

        // Negative start on open path must return None.
        assert!(effective_range(&make_text(-5.0, Some(50.0)), &geometry).is_none());
        assert!(effective_range(&make_text(-5.0, None), &geometry).is_none());

        // Non-finite values must return None.
        assert!(effective_range(&make_text(f32::NAN, Some(50.0)), &geometry).is_none());
        assert!(effective_range(&make_text(10.0, Some(f32::NAN)), &geometry).is_none());
        assert!(effective_range(&make_text(10.0, Some(f32::INFINITY)), &geometry).is_none());
    }
}
