//! On-canvas corner radius widgets for live shapes (TOOL-214-12), as in Photoshop: with Path
//! Selection, a shape tool (Rectangle, Triangle, Polygon), or the Move tool showing its transform
//! controls, every selected shape layer that is a live rectangle, triangle, polygon or star shows
//! a round widget inside each corner, at the centre of its arc. Dragging one rounds every corner
//! of every selected shape to the same radius; ⌥/Alt changes that corner only on rectangles
//! (a triangle or polygon has one radius for all its corners, as in Photoshop). The drag previews
//! the new outlines and commits one `shape.edit` per changed layer on release (`radii` for a
//! rectangle, `cornerRadius` for a polygon), as one history step; a click without movement
//! changes nothing.

use egui::{Color32, Modifiers, Pos2, Stroke};
use photocraft_doc::LayerContent;
use photocraft_doc::vector::LiveShape;
use photocraft_vector::shapes;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// A widget sits at least this many screen points inside its corner, so a sharp corner's widget
/// can still be grabbed.
const MIN_INSET: f64 = 10.0;

/// The live shape a target's corners belong to.
#[derive(Clone, Debug, PartialEq)]
enum Geom {
    /// `[x, y, w, h]` with a positive size.
    Rect([f64; 4]),
    /// The polygon's parameters and its sharp vertices, in path order.
    Polygon { rect: [f64; 4], sides: u32, star_ratio: f64, vertices: Vec<[f64; 2]> },
}

/// A selected live shape with corners: its layer, its geometry, and its radii (a rectangle's four
/// `[tl, tr, br, bl]`; a polygon's one radius, repeated).
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    layer: u64,
    geom: Geom,
    radii: [f64; 4],
}

/// One corner: its vertex, and how far its arc's centre moves per unit of radius (the centre is
/// `vertex + step * r`: `(±1, ±1)` for a rectangle, `bisector / sin(half the angle)` for a
/// polygon vertex).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Corner {
    vertex: [f64; 2],
    step: [f64; 2],
}

impl Target {
    fn corners(&self) -> usize {
        match &self.geom {
            Geom::Rect(_) => 4,
            Geom::Polygon { vertices, .. } => vertices.len(),
        }
    }

    fn is_rect(&self) -> bool {
        matches!(self.geom, Geom::Rect(_))
    }

    /// Corner `i`, `None` when it is degenerate.
    fn corner(&self, i: usize) -> Option<Corner> {
        match &self.geom {
            Geom::Rect([x, y, w, h]) => {
                let (vertex, d) = match i {
                    0 => ([*x, *y], [1.0, 1.0]),
                    1 => ([x + w, *y], [-1.0, 1.0]),
                    2 => ([x + w, y + h], [-1.0, -1.0]),
                    _ => ([*x, y + h], [1.0, -1.0]),
                };
                Some(Corner { vertex, step: d })
            }
            Geom::Polygon { vertices, .. } => {
                let f = self.fillet(i, 1.0)?;
                let sin = f.half_angle.sin();
                (sin > 1e-9).then(|| Corner { vertex: vertices.get(i).copied().unwrap_or(f.centre), step: [f.bisector[0] / sin, f.bisector[1] / sin] })
            }
        }
    }

    /// A polygon vertex's rounding with radius `r`.
    fn fillet(&self, i: usize, r: f64) -> Option<shapes::Fillet> {
        let Geom::Polygon { vertices, .. } = &self.geom else { return None };
        let n = vertices.len();
        let at = |k: usize| vertices.get(k % n).copied();
        shapes::fillet(at(i + n - 1)?, at(i)?, at(i + 1)?, r)
    }

    /// The largest radius every corner of the shape can take.
    fn max_radius(&self) -> f64 {
        match &self.geom {
            Geom::Rect(r) => r[2].min(r[3]) / 2.0,
            Geom::Polygon { vertices, .. } => (0..vertices.len()).filter_map(|i| self.fillet(i, 1.0e12)).map(|f| f.radius).fold(f64::INFINITY, f64::min),
        }
    }

    /// The radius corner `i` is drawn with for `radii`: capped at what the shape allows there.
    fn radius_at(&self, radii: [f64; 4], i: usize) -> f64 {
        match &self.geom {
            Geom::Rect(_) => radii.get(i).copied().unwrap_or(0.0).clamp(0.0, self.max_radius()),
            Geom::Polygon { .. } => self.fillet(i, radii[0]).map_or(0.0, |f| f.radius),
        }
    }

    /// Where corner `i`'s widget is drawn for `radii`: the centre of its arc, or a few screen
    /// points inside a sharper corner.
    fn widget(&self, radii: [f64; 4], i: usize, zoom: f64) -> Option<[f64; 2]> {
        let c = self.corner(i)?;
        let most = match &self.geom {
            Geom::Rect(_) => self.max_radius(),
            Geom::Polygon { .. } => self.fillet(i, 1.0e12).map_or(0.0, |f| f.radius),
        };
        // The inset is measured like a rectangle's: 10 points along each axis of a right angle.
        let len = c.step[0].hypot(c.step[1]).max(1e-9);
        let inset = MIN_INSET / zoom * std::f64::consts::SQRT_2 / len;
        let r = self.radius_at(radii, i).max(inset).min(most);
        Some([c.vertex[0] + c.step[0] * r, c.vertex[1] + c.step[1] * r])
    }

    /// The outline for `radii`.
    fn path(&self, radii: [f64; 4]) -> photocraft_doc::Path {
        match &self.geom {
            Geom::Rect([x, y, w, h]) => shapes::rounded_rect(*x, *y, *w, *h, radii),
            Geom::Polygon { rect, sides, star_ratio, .. } => {
                shapes::live_path(&LiveShape::Polygon { rect: *rect, sides: *sides, star_ratio: *star_ratio, corner_radius: radii[0] })
            }
        }
    }
}

/// A corner radius drag in progress.
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    /// Every selected live shape with corners, as at the press.
    targets: Vec<Target>,
    /// The shape whose widget was pressed (an index into `targets`) and its corner.
    grabbed: usize,
    corner: usize,
    /// Every corner (no ⌥) or the dragged one only (rectangles).
    all: bool,
    /// Where the press was.
    start: [f64; 2],
    /// Each target's radii so far.
    now: Vec<[f64; 4]>,
}

/// Do the widgets show with the current tool?
fn shows(app: &PhotocraftApp) -> bool {
    app.ui.transform.is_none()
        && match app.ui.tool {
            Tool::PathSelection | Tool::Rectangle | Tool::Triangle | Tool::Polygon => true,
            Tool::Move => app.ui.tool_options.move_show_transform,
            _ => false,
        }
}

/// A shape layer's live shape as a target, if it has corners.
fn target(layer: u64, live: &LiveShape) -> Option<Target> {
    let finite = |v: &[f64]| v.iter().all(|v| v.is_finite());
    let positive = |[x, y, w, h]: [f64; 4]| {
        let (x, w) = if w < 0.0 { (x + w, -w) } else { (x, w) };
        let (y, h) = if h < 0.0 { (y + h, -h) } else { (y, h) };
        (w > 0.0 && h > 0.0 && finite(&[x, y, w, h])).then_some([x, y, w, h])
    };
    match *live {
        LiveShape::Rect { rect, radii } => {
            let rect = positive(rect)?;
            finite(&radii).then_some(Target { layer, geom: Geom::Rect(rect), radii })
        }
        LiveShape::Polygon { rect, sides, star_ratio, corner_radius } => {
            positive(rect)?;
            if !finite(&[star_ratio, corner_radius]) {
                return None;
            }
            let sharp = shapes::polygon(rect[0], rect[1], rect[2], rect[3], sides, star_ratio);
            let vertices = sharp.subpaths.first()?.knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
            Some(Target { layer, geom: Geom::Polygon { rect, sides, star_ratio, vertices }, radii: [corner_radius.max(0.0); 4] })
        }
        LiveShape::Ellipse { .. } | LiveShape::Line { .. } => None,
    }
}

/// The selected layers that are live shapes with corners, front first.
fn targets(app: &PhotocraftApp) -> Vec<Target> {
    if !shows(app) {
        return Vec::new();
    }
    let Some(st) = app.session.active() else { return Vec::new() };
    let mut out: Vec<Target> = st
        .selected_layers()
        .into_iter()
        .filter_map(|id| {
            let l = st.doc.layer(id)?;
            let LayerContent::Shape(sh) = &l.content else { return None };
            target(l.id.0, sh.live.as_ref()?)
        })
        .collect();
    // `selected_layers` lists bottom to top; the topmost widget wins a press.
    out.reverse();
    out
}

fn zoom(app: &PhotocraftApp) -> f64 {
    f64::from(app.point_zoom().max(0.01))
}

/// The widget of `targets` within grabbing distance of document point `p`: the shape (an index
/// into `targets`) and its corner, the nearest when several are close.
fn hit(app: &PhotocraftApp, targets: &[Target], p: [f64; 2]) -> Option<(usize, usize)> {
    let z = zoom(app);
    let tol = 7.0 / z;
    targets
        .iter()
        .enumerate()
        .flat_map(|(t, g)| (0..g.corners()).filter_map(move |i| g.widget(g.radii, i, z).map(|w| (t, i, (w[0] - p[0]).hypot(w[1] - p[1])))))
        .filter(|(.., d)| *d <= tol)
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(t, i, _)| (t, i))
}

/// Is a widget under document point `p`, or one being dragged? The canvas then shows the corner
/// radius cursor.
pub fn hovers(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    app.ui.corner_radius.is_some() || hit(app, &targets(app), p).is_some()
}

/// Canvas pointer events for the corner widgets; false when the event is not theirs.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: Modifiers) -> bool {
    match ev {
        ToolEvent::Down { x, y, .. } => {
            if mods.command {
                return false;
            }
            let targets = targets(app);
            let Some((grabbed, corner)) = hit(app, &targets, [x, y]) else { return false };
            let now = targets.iter().map(|t| t.radii).collect();
            app.ui.corner_radius = Some(Drag { targets, grabbed, corner, all: !mods.alt, start: [x, y], now });
            // The press may have started the Move tool's snapping, which a radius drag has none of.
            app.prefs_rt.snap = None;
            app.prefs_rt.snap_lines.clear();
            app.move_mods = Default::default();
            true
        }
        ToolEvent::Move { x, y, .. } => {
            let Some(d) = app.ui.corner_radius.as_mut() else { return false };
            let Some(g) = d.targets.get(d.grabbed) else { return true };
            let Some(c) = g.corner(d.corner) else { return true };
            // The radius at the press plus the move projected onto the corner's step (the way its
            // arc's centre moves as the radius grows). Relative, so a click on a sharp corner's
            // inset widget changes nothing.
            let from = g.radius_at(g.radii, d.corner);
            let len2 = c.step[0] * c.step[0] + c.step[1] * c.step[1];
            let r = from + ((x - d.start[0]) * c.step[0] + (y - d.start[1]) * c.step[1]) / len2.max(1e-12);
            let r = if r.is_finite() { r.max(0.0) } else { from };
            // Every selected shape takes the same radius, within its own size.
            for (t, now) in d.targets.iter().zip(d.now.iter_mut()) {
                let r = r.min(t.max_radius());
                if d.all || !t.is_rect() {
                    *now = [r; 4];
                } else {
                    *now = t.radii;
                    if let Some(v) = now.get_mut(d.corner) {
                        *v = r;
                    }
                }
            }
            true
        }
        ToolEvent::Up { x, y } => {
            if app.ui.corner_radius.is_none() {
                return false;
            }
            pointer(app, ToolEvent::Move { x, y, pressure: 1.0 }, mods);
            let Some(d) = app.ui.corner_radius.take() else { return true };
            // One history step for every layer changed.
            let key = format!("corner-radius-{}", app.ui.alloc_id());
            for (t, now) in d.targets.iter().zip(&d.now) {
                if *now == t.radii {
                    continue;
                }
                let params = if t.is_rect() {
                    serde_json::json!({"layer": t.layer, "radii": now, "coalesce": key})
                } else {
                    serde_json::json!({"layer": t.layer, "cornerRadius": now[0], "coalesce": key})
                };
                if let Err(e) = app.run("shape.edit", params) {
                    app.ui.status = e;
                    app.ui.status_error = true;
                    break;
                }
            }
            true
        }
    }
}

/// The radius the dragged corner shows in the readout.
fn dragged_radius(d: &Drag) -> Option<f64> {
    let g = d.targets.get(d.grabbed)?;
    let now = d.now.get(d.grabbed)?;
    if g.is_rect() { now.get(d.corner).copied() } else { Some(g.radius_at(*now, d.corner)) }
}

/// The widgets of every selected live shape with corners; while one is dragged, the new outlines
/// and the radius beside the pointer (`R: 25.0 px`), as in Photoshop.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, to_scr: &dyn Fn([f64; 2]) -> Pos2, accent: Color32) {
    let drag = app.ui.corner_radius.as_ref();
    if let Some(r) = drag.and_then(dragged_radius)
        && let Some(at) = painter.ctx().input(|i| i.pointer.hover_pos())
    {
        crate::canvas::draw_value_readout(app, painter.ctx(), "corner-radius-readout", at, ["R:"], [format!("{r:.1} px")]);
    }
    let z = zoom(app);
    for t in targets(app) {
        let now = drag.and_then(|d| d.targets.iter().position(|g| g.layer == t.layer).and_then(|i| d.now.get(i)));
        let radii = now.copied().unwrap_or(t.radii);
        if now.is_some() {
            crate::vector_ui::draw_outline(painter, &t.path(radii), to_scr, accent);
        }
        for i in 0..t.corners() {
            if let Some(w) = t.widget(radii, i, z) {
                let p = to_scr(w);
                painter.circle(p, 5.0, Color32::WHITE, Stroke::new(1.5, accent));
                painter.circle_filled(p, 2.5, accent);
            }
        }
    }
}

#[cfg(test)]
#[path = "corner_radius_tests.rs"]
mod tests;
