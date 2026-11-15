//! The Perspective Crop tool (C, in the Crop flyout), as in Photoshop:
//!
//! - drag a box, or click four corners, to make a quad (`UiState::perspective_crop`, TL, TR, BR,
//!   BL in document px; clicked corners are put in that order);
//! - drag a corner to move it alone, onto an edge of a skewed object (a photographed page, a
//!   facade); drag inside the quad to move it;
//! - the quad shows its perspective grid (Show Grid) and turns red while it isn't a convex quad,
//!   which can't be committed; one flipped on one axis (wound the other way) commits mirrored;
//! - ↵ (or ✓) commits: `image.perspectiveCrop` maps the quad onto an upright rectangle, the whole
//!   document in one undo step. Its size comes from the options bar's W x H (and the resolution),
//!   which are the Crop tool's fields (`crop_size`), or with them empty from the rectangle the
//!   quad shows in perspective. Esc (or ⊘) cancels.
//!
//! A pending quad belongs to the document it was drawn on, like the Crop tool's frame.

use egui::{Color32, Pos2, Stroke};
use photocraft_algo::transform::{Homography, convex_quad, convex_quad_any_winding};
use photocraft_engine::prefs::Unit;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::state::Tool;

/// A drag shorter than this (screen px) is a click: it places a corner.
const CLICK_PX: f64 = 3.0;

#[derive(Clone, Copy, Debug)]
pub enum Drag {
    /// Drawing a box from `start`; ⇧ makes it a square, ⌥ centres it on `start` (#11802).
    New { start: [f64; 2], cur: [f64; 2], square: bool, centred: bool },
    /// Moving one corner; `offset` is the press's distance from it, `press` where the press was
    /// and `quad` the quad then (⇧ keeps one axis, ⌥ keeps a rectangle one, #11802).
    Corner { index: usize, offset: [f64; 2], press: [f64; 2], quad: [[f64; 2]; 4] },
    /// Moving the whole quad.
    Move { start: [f64; 2], quad: [[f64; 2]; 4] },
}

#[derive(Default)]
pub struct PerspectiveCrop {
    pub drag: Option<Drag>,
    /// Corners clicked so far, before the fourth makes the quad.
    pub clicks: Vec<[f64; 2]>,
    /// The pointer between clicks (button up), the preview quad's free corner (#11804).
    pub hover: Option<[f64; 2]>,
    /// The document the pending quad belongs to.
    doc: Option<photocraft_doc::DocId>,
}

fn active(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::PerspectiveCrop && app.session.active().is_some()
}

/// A quad or corners clicked or a drag in progress.
pub fn pending(app: &PhotocraftApp) -> bool {
    app.ui.perspective_crop.is_some() || !app.pcrop.clicks.is_empty() || app.pcrop.drag.is_some()
}

/// A box, corner or quad drag in progress: held past the canvas edge, it scrolls the view (#4183).
/// A press that hasn't moved further than a click doesn't: scrolling would turn a corner-placing
/// click near the edge into a box.
pub fn dragging(app: &PhotocraftApp) -> bool {
    app.ui.tool == Tool::PerspectiveCrop
        && match app.pcrop.drag {
            Some(Drag::New { start, cur, .. }) => !is_click(app, start, cur),
            Some(_) => true,
            None => false,
        }
}

/// A new-box press released within [`CLICK_PX`] screen points of where it began places a corner.
fn is_click(app: &PhotocraftApp, start: [f64; 2], cur: [f64; 2]) -> bool {
    let click = crate::distort_ui::tolerance(app) * CLICK_PX / 8.0;
    (cur[0] - start[0]).abs() <= click && (cur[1] - start[1]).abs() <= click
}

pub fn cancel(app: &mut PhotocraftApp) {
    app.ui.perspective_crop = None;
    app.pcrop.drag = None;
    app.pcrop.clicks.clear();
    app.pcrop.hover = None;
}

/// Another document became active: its pending quad is dropped (also from `sync_views`, so it is
/// never drawn on the other document).
pub(crate) fn cancel_stale(app: &mut PhotocraftApp) {
    let doc = app.session.active().map(|st| st.doc.id);
    if pending(app) && app.pcrop.doc.is_some() && app.pcrop.doc != doc {
        cancel(app);
    }
    app.pcrop.doc = doc;
}

/// Four points in quad order: clockwise on screen around their centre, starting top-left.
pub fn ordered(points: [[f64; 2]; 4]) -> [[f64; 2]; 4] {
    let c = [points.iter().map(|p| p[0]).sum::<f64>() / 4.0, points.iter().map(|p| p[1]).sum::<f64>() / 4.0];
    let mut v = points;
    // y grows downwards, so increasing atan2 runs clockwise on screen.
    v.sort_by(|a, b| (a[1] - c[1]).atan2(a[0] - c[0]).total_cmp(&(b[1] - c[1]).atan2(b[0] - c[0])));
    let first = (0..4).min_by(|&i, &j| (v[i][0] + v[i][1]).total_cmp(&(v[j][0] + v[j][1]))).unwrap_or(0);
    v.rotate_left(first);
    v
}

fn inside(q: &[[f64; 2]; 4], p: [f64; 2]) -> bool {
    (0..4).all(|i| {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= 0.0
    })
}

/// What a press at a point does, which the hover cursor shows (#11974).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    /// No quad yet: a press starts a box or places a corner.
    Idle,
    /// Within the corner tolerance: a press drags that corner.
    Corner(usize),
    /// Inside the quad: a press moves it.
    Inside,
    /// Elsewhere with a quad: a press does nothing yet (edge handles and rotate are #11801).
    Outside,
}

/// The zone at `p` (document px): the hit-testing of a press in [`pointer`].
pub fn zone(app: &PhotocraftApp, p: [f64; 2]) -> Zone {
    let Some(q) = app.ui.perspective_crop else {
        return Zone::Idle;
    };
    let tol = crate::distort_ui::tolerance(app);
    let dist = |i: usize| (q[i][0] - p[0]).hypot(q[i][1] - p[1]);
    match (0..4).filter(|&i| dist(i) <= tol).min_by(|&i, &j| dist(i).total_cmp(&dist(j))) {
        Some(index) => Zone::Corner(index),
        None if inside(&q, p) => Zone::Inside,
        None => Zone::Outside,
    }
}

/// The hover cursor's zone at `p`: what a press there does. While a drag is held it stays the
/// zone the drag started in, as the Crop tool keeps its drag's cursor (not measured).
pub fn cursor(app: &PhotocraftApp, p: [f64; 2]) -> Option<Zone> {
    if !active(app) || !p[0].is_finite() || !p[1].is_finite() {
        return None;
    }
    Some(match app.pcrop.drag {
        Some(Drag::New { .. }) => Zone::Idle,
        Some(Drag::Corner { index, .. }) => Zone::Corner(index),
        Some(Drag::Move { .. }) => Zone::Inside,
        None => zone(app, p),
    })
}

/// The box a new drag from `start` to `cur` draws (document px, x0, y0, x1, y1): ⇧ makes it a
/// square of the shorter drag side in the drag's direction, ⌥ centres it on `start` at twice the
/// drag, both together a centred square (#11802). `None` without area.
pub(crate) fn new_box(start: [f64; 2], cur: [f64; 2], square: bool, centred: bool) -> Option<[[f64; 2]; 4]> {
    let (mut dx, mut dy) = (cur[0] - start[0], cur[1] - start[1]);
    if square {
        let side = dx.abs().min(dy.abs());
        (dx, dy) = (side.copysign(dx), side.copysign(dy));
    }
    let (a, b) = if centred { ([start[0] - dx, start[1] - dy], [start[0] + dx, start[1] + dy]) } else { (start, [start[0] + dx, start[1] + dy]) };
    let (x0, y0, x1, y1) = (a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1]));
    // A straight drag has no area: no box (it could neither be committed nor replaced by a new
    // drag).
    (x1 > x0 && y1 > y0).then_some([[x0, y0], [x1, y0], [x1, y1], [x0, y1]])
}

/// Whether every side of `q` is either horizontal or vertical: a rectangle on the image's axes,
/// which ⌥ keeps one while a corner is dragged.
fn axis_rect(q: &[[f64; 2]; 4]) -> bool {
    (0..4).all(|i| {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        (a[0] == b[0]) != (a[1] == b[1])
    })
}

/// Pointer events (document px) with the tool and the held modifiers. True when consumed.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    if app.ui.tool != Tool::PerspectiveCrop {
        app.pcrop.drag = None;
        return false;
    }
    let p = match ev {
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => [x, y],
    };
    if app.session.active().is_none() || !p[0].is_finite() || !p[1].is_finite() {
        app.pcrop.hover = None;
        return true;
    }
    cancel_stale(app);
    match ev {
        ToolEvent::Down { .. } => {
            // A new quad's first press drops an active selection as its own "Deselect" step, as in
            // Photoshop (#11975): it is gone after the commit and after Esc too. Refused (a job
            // locks the document, its reason in the status bar), the press starts no quad.
            if app.ui.perspective_crop.is_none()
                && app.pcrop.clicks.is_empty()
                && app.session.active().is_some_and(|d| d.doc.selection.is_some())
                && app.run("select.deselect", json!({})).is_err()
            {
                return true;
            }
            app.pcrop.drag = match (zone(app, p), app.ui.perspective_crop) {
                (Zone::Corner(index), Some(q)) => Some(Drag::Corner { index, offset: [p[0] - q[index][0], p[1] - q[index][1]], press: p, quad: q }),
                (Zone::Inside, Some(q)) => Some(Drag::Move { start: p, quad: q }),
                (Zone::Idle, _) => Some(Drag::New { start: p, cur: p, square: mods.shift, centred: mods.alt }),
                _ => None,
            };
        }
        ToolEvent::Move { .. } => {
            // Between corner clicks the preview's free corner follows the pointer (#11804).
            app.pcrop.hover = (app.pcrop.drag.is_none() && !app.pcrop.clicks.is_empty()).then_some(p);
            drag_to(app, p, mods);
        }
        ToolEvent::Up { .. } => {
            drag_to(app, p, mods);
            if let Some(Drag::New { start, cur, square, centred }) = app.pcrop.drag {
                if is_click(app, start, cur) {
                    app.pcrop.clicks.push(start);
                    if let [a, b, c, d] = app.pcrop.clicks[..] {
                        app.ui.perspective_crop = Some(ordered([a, b, c, d]));
                        app.pcrop.clicks.clear();
                    }
                } else if let Some(quad) = new_box(start, cur, square, centred) {
                    // Only a drag that built a box replaces the corners clicked so far. One that
                    // made none (a start the snap moved onto its release point) leaves them, so the
                    // next click still finishes the quad (#13065).
                    app.pcrop.clicks.clear();
                    app.ui.perspective_crop = Some(quad);
                }
            }
            app.pcrop.drag = None;
        }
    }
    true
}

fn drag_to(app: &mut PhotocraftApp, p: [f64; 2], mods: egui::Modifiers) {
    match app.pcrop.drag {
        // The modifiers held now shape the box, as the Frame tool's (not measured mid-drag).
        Some(Drag::New { start, .. }) => app.pcrop.drag = Some(Drag::New { start, cur: p, square: mods.shift, centred: mods.alt }),
        Some(Drag::Corner { index, offset, press, quad }) => {
            let Some(from) = quad.get(index).copied() else { return };
            let mut c = [p[0] - offset[0], p[1] - offset[1]];
            // ⇧: along the axis the drag mostly follows since the press; the other stays.
            if mods.shift {
                if (p[0] - press[0]).abs() >= (p[1] - press[1]).abs() {
                    c[1] = from[1];
                } else {
                    c[0] = from[0];
                }
            }
            let Some(q) = app.ui.perspective_crop.as_mut() else { return };
            // Each move starts from the quad at the press, so ⌥ released mid-drag puts the
            // neighbours back.
            *q = quad;
            // ⌥ on a rectangle: the neighbours follow, one coordinate each, the opposite corner
            // stays. On any other quad (not measured) the corner moves alone.
            if mods.alt && axis_rect(&quad) {
                for j in [(index + 1) % 4, (index + 3) % 4] {
                    if quad[j][0] == from[0] {
                        q[j][0] = c[0];
                    } else {
                        q[j][1] = c[1];
                    }
                }
            }
            if let Some(corner) = q.get_mut(index) {
                *corner = c;
            }
        }
        Some(Drag::Move { start, quad }) => {
            let (dx, dy) = (p[0] - start[0], p[1] - start[1]);
            app.ui.perspective_crop = Some(quad.map(|c| [c[0] + dx, c[1] + dy]));
        }
        None => {}
    }
}

/// `image.perspectiveCrop`'s size params from W, H and the resolution (the Crop tool's fields):
/// lengths in physical units convert at the typed resolution, else the document's.
pub(crate) fn size_params(app: &PhotocraftApp, p: &mut Value) {
    let o = &app.ui.tool_options;
    let res = crate::crop_size::resolution_ppi(o);
    let dpi = res.unwrap_or_else(|| app.session.active().map_or(72.0, |st| f64::from(st.doc.resolution_dpi)));
    // Points and picas per Preferences › Point/Pica Size.
    let pt_per_in = app.session.prefs().units_and_rulers.point_size.per_inch();
    let px = |text: &str| {
        let (v, u) = crate::crop_size::parse_length(text, Unit::Pixels)?;
        let px = u.to_px(v, dpi, 0.0, pt_per_in).round();
        (px.is_finite() && px >= 1.0).then_some(px)
    };
    if let Some(w) = px(&o.crop_width) {
        p["width"] = json!(w);
    }
    if let Some(h) = px(&o.crop_height) {
        p["height"] = json!(h);
    }
    if let Some(r) = res {
        p["resolution"] = json!(r);
    }
}

/// ↵: crops to the pending quad. An invalid quad stays for fixing, with the engine's reason.
pub fn commit(app: &mut PhotocraftApp) {
    cancel_stale(app);
    let Some(q) = app.ui.perspective_crop else { return };
    let mut p = json!({"corners": q});
    size_params(app, &mut p);
    if app.run("image.perspectiveCrop", p).is_ok() {
        cancel(app);
    }
}

/// ↵ commits and Esc cancels while the tool has something pending. True when a key was taken.
pub fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    if !active(app) || !pending(app) {
        return false;
    }
    use egui::{Key, Modifiers};
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        commit(app);
        return true;
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        cancel(app);
        return true;
    }
    false
}

/// Grid lines per side for a quad about `side` screen px across (Photoshop's grid gets denser as
/// the quad grows).
fn grid_steps(side: f32) -> usize {
    ((side / 48.0).round() as usize).clamp(2, 12)
}

/// The quad that follows the pointer while corners are clicked (#11804): after two clicks the two
/// points and the pointer as the third and fourth corner (a triangle), after three the three points
/// and the pointer as the fourth. `None` with fewer clicks or no pointer position.
pub(crate) fn click_preview(app: &PhotocraftApp) -> Option<[[f64; 2]; 4]> {
    let h = app.pcrop.hover.filter(|h| h.iter().all(|v| v.is_finite()))?;
    match app.pcrop.clicks[..] {
        [a, b] => Some([a, b, h, h]),
        [a, b, c] => Some([a, b, c, h]),
        _ => None,
    }
}

/// The grid lines of the quad `q` (document px, `n` steps): its perspective grid, or for a quad
/// with two corners on one point (the two-click preview) the lines between matching points of
/// opposite sides, which meet at that point.
fn grid_lines(q: [[f64; 2]; 4], n: usize) -> Vec<[[f64; 2]; 2]> {
    let lerp = |a: [f64; 2], b: [f64; 2], t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    let homography = if convex_quad(&q) { Homography::rect_to_quad([0.0, 0.0, 1.0, 1.0], q) } else { None };
    let at = |u: f64, v: f64| match &homography {
        Some(h) => {
            let (x, y) = h.apply(u, v);
            [x, y]
        }
        None => lerp(lerp(q[0], q[1], u), lerp(q[3], q[2], u), v),
    };
    (1..n)
        .flat_map(|k| {
            let t = k as f64 / n as f64;
            [[at(t, 0.0), at(t, 1.0)], [at(0.0, t), at(1.0, t)]]
        })
        .collect()
}

/// The shield over the image outside a pending quad (#11972): grey 89 at 18 % opacity, which shows
/// red 255, 0, 0 as 225, 16, 16, as measured in the reference application.
pub(crate) const SHIELD: Color32 = Color32::from_rgba_premultiplied(16, 16, 16, 46);

/// The canvas `canvas` (a convex polygon, document px) minus the convex quad `q`, as up to four
/// disjoint convex pieces: each is what is left of the canvas outside one quad edge and inside the
/// edges before it. Empty for a degenerate quad.
pub(crate) fn shield_pieces(canvas: [[f64; 2]; 4], q: [[f64; 2]; 4]) -> Vec<Vec<[f64; 2]>> {
    let area: f64 = (0..4).map(|i| q[i][0] * q[(i + 1) % 4][1] - q[(i + 1) % 4][0] * q[i][1]).sum();
    if !area.is_finite() || area.abs() < 1e-9 {
        return Vec::new();
    }
    let side = area.signum();
    let mut rest = canvas.to_vec();
    let mut out = Vec::new();
    for i in 0..4 {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        let outside = clip_half_plane(&rest, a, b, -side);
        if outside.len() >= 3 {
            out.push(outside);
        }
        rest = clip_half_plane(&rest, a, b, side);
    }
    out
}

/// One Sutherland–Hodgman step: the part of the convex polygon `poly` on the `side` (the sign of
/// the cross product) of the line from `a` to `b`.
fn clip_half_plane(poly: &[[f64; 2]], a: [f64; 2], b: [f64; 2], side: f64) -> Vec<[f64; 2]> {
    let d = |p: [f64; 2]| side * ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]));
    let mut out = Vec::with_capacity(poly.len() + 1);
    for (i, &p) in poly.iter().enumerate() {
        let n = poly[(i + 1) % poly.len()];
        let (dp, dn) = (d(p), d(n));
        if dp >= 0.0 {
            out.push(p);
        }
        if (dp >= 0.0) != (dn >= 0.0) && (dp - dn).abs() > f64::EPSILON {
            let t = dp / (dp - dn);
            out.push([p[0] + (n[0] - p[0]) * t, p[1] + (n[1] - p[1]) * t]);
        }
    }
    out
}

/// The quad, its perspective grid and corner handles; corners clicked so far; the box being drawn.
pub fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    if app.ui.tool != Tool::PerspectiveCrop {
        return;
    }
    let scr = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    let accent = crate::theme::Tokens::get(painter.ctx()).accent;
    let quad = match app.pcrop.drag {
        Some(Drag::New { start, cur, square, centred }) => new_box(start, cur, square, centred),
        _ => app.ui.perspective_crop,
    };
    if let Some(q) = quad.filter(|q| q.iter().flatten().all(|v| v.is_finite())) {
        // A quad flipped on one axis is wound counter-clockwise and commits mirrored (#12489).
        let ok = convex_quad_any_winding(&q);
        let color = if ok { accent } else { Color32::from_rgb(0xe5, 0x3e, 0x3e) };
        // The image outside the quad is dimmed (#11972), following the quad while it is dragged.
        if ok && let Some(size) = app.session.active().map(|d| d.doc.size) {
            let (w, h) = (f64::from(size.width), f64::from(size.height));
            for piece in shield_pieces([[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]], q) {
                painter.add(egui::Shape::convex_polygon(piece.iter().map(|c| scr(*c)).collect(), SHIELD, Stroke::NONE));
            }
        }
        let pts: Vec<Pos2> = q.iter().map(|c| scr(*c)).collect();
        if ok
            && app.ui.tool_options.perspective_crop_grid
            && let Some(h) = Homography::rect_to_quad([0.0, 0.0, 1.0, 1.0], q)
        {
            let side = pts.iter().zip(pts.iter().skip(1)).map(|(a, b)| a.distance(*b)).fold(0.0, f32::max);
            let n = grid_steps(side);
            let thin = Stroke::new(0.75, Color32::from_rgba_unmultiplied(255, 255, 255, 150));
            let at = |u: f64, v: f64| {
                let (x, y) = h.apply(u, v);
                scr([x, y])
            };
            for k in 1..n {
                let t = k as f64 / n as f64;
                painter.line_segment([at(t, 0.0), at(t, 1.0)], thin);
                painter.line_segment([at(0.0, t), at(1.0, t)], thin);
            }
        }
        painter.add(egui::Shape::closed_line(pts.clone(), Stroke::new(1.5, color)));
        if !matches!(app.pcrop.drag, Some(Drag::New { .. })) {
            for c in pts {
                let r = egui::Rect::from_center_size(c, egui::vec2(8.0, 8.0));
                painter.rect_filled(r, 0.0, Color32::WHITE);
                painter.rect_stroke(r, 0.0, Stroke::new(1.25, color), egui::StrokeKind::Inside);
            }
        }
    }
    let clicks: Vec<Pos2> = app.pcrop.clicks.iter().map(|c| scr(*c)).collect();
    if let Some(q) = click_preview(app) {
        // The quad to the pointer with its grid, in the finished quad's style (#11804). A third
        // click that makes it cross turns it red as the finished quad (not measured); the
        // two-click triangle is drawn as a quad.
        let ok = clicks.len() == 2 || convex_quad(&q);
        let color = if ok { accent } else { Color32::from_rgb(0xe5, 0x3e, 0x3e) };
        let pts: Vec<Pos2> = q.iter().map(|c| scr(*c)).collect();
        if ok && app.ui.tool_options.perspective_crop_grid {
            let side = pts.iter().zip(pts.iter().skip(1)).map(|(a, b)| a.distance(*b)).fold(0.0, f32::max);
            let thin = Stroke::new(0.75, Color32::from_rgba_unmultiplied(255, 255, 255, 150));
            for [a, b] in grid_lines(q, grid_steps(side)) {
                painter.line_segment([scr(a), scr(b)], thin);
            }
        }
        painter.add(egui::Shape::closed_line(pts, Stroke::new(1.5, color)));
    } else if clicks.len() > 1 {
        painter.add(egui::Shape::line(clicks.clone(), Stroke::new(1.5, accent)));
    }
    // The clicked corners: small square handles, as the reference application marks them.
    for c in clicks {
        let r = egui::Rect::from_center_size(c, egui::vec2(8.0, 8.0));
        painter.rect_filled(r, 0.0, Color32::WHITE);
        painter.rect_stroke(r, 0.0, Stroke::new(1.25, accent), egui::StrokeKind::Inside);
    }
}

/// Options bar: W, ⇄, H, the resolution (the Crop tool's fields), Front Image, Clear, Show Grid,
/// and cancel / commit.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let doc_dpi = app.session.active().map_or(72.0, |st| f64::from(st.doc.resolution_dpi));
    let pt_per_in = app.session.prefs().units_and_rulers.point_size.per_inch();
    // Out-of-range numbers and text get the reference application's alerts (#11977).
    if let Some(message) = crate::crop_size::perspective_fields(&mut app.ui.tool_options, ui, doc_dpi, pt_per_in) {
        app.ui.open_dialog(crate::state::DialogKind::Error, json!({ "message": message }).as_object().cloned().unwrap_or_default());
    }
    if crate::widgets::secondary_button(ui, tl!("Front Image"), 0.0).clicked()
        && let Some((size, dpi)) = app.session.active().map(|st| (st.doc.size, f64::from(st.doc.resolution_dpi)))
    {
        let o = &mut app.ui.tool_options;
        o.crop_width = crate::crop_size::format_length(f64::from(size.width), Unit::Pixels);
        o.crop_height = crate::crop_size::format_length(f64::from(size.height), Unit::Pixels);
        let per_cm = o.crop_resolution_unit == crate::crop_size::PX_PER_CM;
        // Within the field's limits, so a 30000 ppi document's resolution still commits.
        o.crop_resolution = crate::crop_size::fit_resolution(if per_cm { dpi / 2.54 } else { dpi }, per_cm);
    }
    if crate::widgets::secondary_button(ui, tl!("Clear"), 0.0).clicked() {
        crate::crop_size::clear(&mut app.ui.tool_options);
        // Clear also puts the resolution unit back to px/in (#11977).
        app.ui.tool_options.crop_resolution_unit = crate::crop_size::PX_PER_IN.into();
    }
    crate::widgets::vline(ui, 22.0);
    crate::widgets::checkbox(ui, &mut app.ui.tool_options.perspective_crop_grid, tl!("Show Grid"));
    // Commit and Cancel show only during a session: a quad, clicked corners, or a drag (#11978).
    if !pending(app) {
        return;
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let commit_tip = crate::i18n::fmt(tl!("Commit current crop operation  ({key})"), &[("key", &crate::shortcuts::pretty("Enter"))]);
        if crate::icons::button(ui, "check", 26.0, false, &commit_tip).clicked() {
            commit(app);
        }
        if crate::icons::button(ui, "ban", 26.0, false, tl!("Cancel current crop operation  (Esc)")).clicked() {
            cancel(app);
        }
    });
}

#[cfg(test)]
#[path = "perspective_crop_fields_tests.rs"]
mod perspective_crop_fields_tests;

#[cfg(test)]
#[path = "perspective_crop_click_preview_tests.rs"]
mod perspective_crop_click_preview_tests;

#[cfg(test)]
#[path = "perspective_crop_bar_buttons_tests.rs"]
mod perspective_crop_bar_buttons_tests;

#[cfg(test)]
#[path = "perspective_crop_modifiers_tests.rs"]
mod perspective_crop_modifiers_tests;

#[cfg(test)]
#[path = "perspective_crop_shield_tests.rs"]
mod perspective_crop_shield_tests;

#[cfg(test)]
#[path = "perspective_crop_deselect_tests.rs"]
mod perspective_crop_deselect_tests;

#[cfg(test)]
#[path = "perspective_crop_mirrored_tests.rs"]
mod perspective_crop_mirrored_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};

    /// A straight drag (no width or no height) leaves no zero-area box that only Esc could clear,
    /// and a plain click still counts as a corner click.
    #[test]
    fn a_drag_without_area_leaves_no_quad() {
        let doc = Document::with_background("p", Size::new(200, 150), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut s = photocraft_engine::Session::new();
        s.add_document(doc, None);
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.ui.tool = Tool::PerspectiveCrop;
        let ev = |app: &mut PhotocraftApp, e| assert!(pointer(app, e, egui::Modifiers::NONE));
        for (x1, y1) in [(10.0, 60.0), (90.0, 10.0)] {
            ev(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 });
            ev(&mut app, ToolEvent::Move { x: x1, y: y1, pressure: 1.0 });
            ev(&mut app, ToolEvent::Up { x: x1, y: y1 });
            assert_eq!(app.ui.perspective_crop, None, "drag to ({x1}, {y1})");
            assert!(app.pcrop.clicks.is_empty());
        }
        ev(&mut app, ToolEvent::Down { x: 30.0, y: 30.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 30.0, y: 30.0 });
        assert_eq!(app.pcrop.clicks, vec![[30.0, 30.0]]);
        assert_eq!(app.ui.perspective_crop, None);
    }

    /// Real egui input: a corner dragged past the canvas edge and held there scrolls the view, as a
    /// marquee drag does (#4183), so the corner can reach the part of the image that was off screen.
    #[test]
    fn a_corner_held_past_the_canvas_edge_scrolls_the_view() {
        use egui::{Event, Modifiers, PointerButton, pos2};
        let doc = Document::with_background("p", Size::new(200, 100), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut s = photocraft_engine::Session::new();
        s.add_document(doc, None);
        let mut a = PhotocraftApp::new(s, crate::Services::default());
        a.ui.tool = Tool::PerspectiveCrop;
        a.ui.extras.snap = false;
        a.ui.extras.rulers = false;
        // 200x100 at 400% in a 600x400 canvas: document x 25..175 shows.
        a.ui.views = vec![crate::state::View {
            zoom: 4.0,
            center: [100.0, 50.0],
            fit_pending: false,
            fill_pending: false,
            doc_size: [200, 100],
            rotation: 0.0,
            zoom_animating: false,
            zoom_anchor: None,
            display_camera: None,
            zoom_transition: None,
        }];
        a.ui.perspective_crop = Some([[40.0, 10.0], [100.0, 10.0], [100.0, 40.0], [40.0, 40.0]]);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 400.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let v = app.ui.views[0].clone();
                crate::canvas::canvas_view(app, ui, 0, ui.max_rect(), v, true);
            },
            a,
        );
        h.run_steps(2);
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
        let xf = crate::canvas::ViewXform { rect, zoom: 4.0, center: [100.0, 50.0], flip: false, rotation: 0.0, aspect: 1.0 };
        // The top-right corner: document (100, 10).
        let grab = xf.to_screen(100.0, 10.0);
        h.event(Event::PointerMoved(grab));
        h.event(button(grab, true));
        h.step();
        let out = pos2(650.0, grab.y);
        for p in [pos2(grab.x + 40.0, grab.y), out] {
            h.event(Event::PointerMoved(p));
            h.step();
        }
        let before = h.state().ui.views[0].center;
        h.run_steps(10);
        let st = h.state();
        assert!(matches!(st.pcrop.drag, Some(Drag::Corner { index: 1, .. })), "{:?}", st.pcrop.drag);
        assert!(st.ui.views[0].center[0] > before[0], "the view scrolled right: {:?} -> {:?}", before, st.ui.views[0].center);
        assert!(st.ui.perspective_crop.is_some_and(|q| q[1][0] > 175.0), "the corner reached past what showed: {:?}", st.ui.perspective_crop);
        h.event(button(out, false));
        h.step();
    }

    /// ⌘Space (Ctrl+Space off the Mac) pressed during a corner drag held past the canvas edge
    /// selects the temporary Zoom, which takes the pointer from the drag: the view no longer
    /// scrolls for the corner the Zoom froze.
    #[test]
    fn a_temporary_zoom_during_a_corner_drag_stops_the_scrolling() {
        use egui::{Event, Key, Modifiers, PointerButton, pos2};
        let doc = Document::with_background("p", Size::new(800, 100), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut s = photocraft_engine::Session::new();
        s.add_document(doc, None);
        let mut a = PhotocraftApp::new(s, crate::Services::default());
        a.ui.tool = Tool::PerspectiveCrop;
        a.ui.extras.snap = false;
        a.ui.extras.rulers = false;
        // 800x100 at 400% in a 600x400 canvas: document x 25..175 shows, with room to scroll right.
        a.ui.views = vec![crate::state::View {
            zoom: 4.0,
            center: [100.0, 50.0],
            fit_pending: false,
            fill_pending: false,
            doc_size: [800, 100],
            rotation: 0.0,
            zoom_animating: false,
            zoom_anchor: None,
            display_camera: None,
            zoom_transition: None,
        }];
        a.ui.perspective_crop = Some([[40.0, 10.0], [100.0, 10.0], [100.0, 40.0], [40.0, 40.0]]);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(600.0, 400.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let v = app.ui.views[0].clone();
                crate::canvas::canvas_view(app, ui, 0, ui.max_rect(), v, true);
            },
            a,
        );
        h.run_steps(2);
        let button = |pos, pressed, modifiers| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
        let xf = crate::canvas::ViewXform { rect, zoom: 4.0, center: [100.0, 50.0], flip: false, rotation: 0.0, aspect: 1.0 };
        let grab = xf.to_screen(100.0, 10.0);
        h.event(Event::PointerMoved(grab));
        h.event(button(grab, true, Modifiers::NONE));
        h.step();
        let out = pos2(650.0, grab.y);
        for p in [pos2(grab.x + 40.0, grab.y), out] {
            h.event(Event::PointerMoved(p));
            h.step();
        }
        // Scrolling while the crop tool has the pointer.
        let before = h.state().ui.views[0].center;
        h.run_steps(2);
        assert!(h.state().ui.views[0].center[0] > before[0], "the corner drag scrolls: {:?} -> {:?}", before, h.state().ui.views[0].center);
        let zoom = Modifiers::COMMAND;
        let zoom = if cfg!(target_os = "macos") { Modifiers { mac_cmd: true, ..zoom } } else { Modifiers { ctrl: true, ..zoom } };
        h.event(Event::ModifiersChanged(zoom));
        h.event(Event::Key { key: Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: zoom });
        h.step();
        assert_eq!(crate::hold_keys::held_tool(h.state(), &h.ctx), Some(crate::hold_keys::Temporary::ZoomIn));
        assert!(h.state().pcrop.drag.is_some(), "the corner drag is still recorded");
        let before = h.state().ui.views[0].center;
        h.run_steps(10);
        assert_eq!(h.state().ui.views[0].center, before, "the temporary Zoom's press doesn't scroll for the corner");
        h.event(button(out, false, zoom));
        h.event(Event::Key { key: Key::Space, physical_key: None, pressed: false, repeat: false, modifiers: zoom });
        h.event(Event::ModifiersChanged(Modifiers::NONE));
        h.step();
    }

    /// A corner-placing click pressed just inside the canvas edge that slips a point past it doesn't
    /// scroll the view (scrolling would turn it into a box); a new box dragged past the edge does.
    #[test]
    fn a_click_at_the_canvas_edge_places_a_corner_without_scrolling() {
        use egui::{Event, Modifiers, PointerButton, pos2};
        let doc = Document::with_background("p", Size::new(200, 100), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut s = photocraft_engine::Session::new();
        s.add_document(doc, None);
        let mut a = PhotocraftApp::new(s, crate::Services::default());
        a.ui.tool = Tool::PerspectiveCrop;
        a.ui.extras.snap = false;
        a.ui.extras.rulers = false;
        // 200x100 at 400% in a 600x400 canvas 100 points from the window's left: document x 25..175
        // shows.
        a.ui.views = vec![crate::state::View {
            zoom: 4.0,
            center: [100.0, 50.0],
            fit_pending: false,
            fill_pending: false,
            doc_size: [200, 100],
            rotation: 0.0,
            zoom_animating: false,
            zoom_anchor: None,
            display_camera: None,
            zoom_transition: None,
        }];
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 400.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let v = app.ui.views[0].clone();
                let rect = egui::Rect::from_min_size(egui::pos2(100.0, 0.0), egui::vec2(600.0, 400.0));
                crate::canvas::canvas_view(app, ui, 0, rect, v, true);
            },
            a,
        );
        h.run_steps(2);
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        // Pressed a point inside the left edge (the scrollbars sit right and bottom), slipping two
        // points past it: still a click.
        let press = pos2(101.0, 200.0);
        let slip = pos2(99.0, 200.0);
        h.event(Event::PointerMoved(press));
        h.event(button(press, true));
        h.step();
        // Held past egui's click time, the press becomes a drag that hasn't left the click size.
        for p in [slip, pos2(98.5, 200.0), slip] {
            h.event(Event::PointerMoved(p));
            h.run_steps(4);
        }
        assert_eq!(h.state().ui.views[0].center, [100.0, 50.0], "a click doesn't scroll the view");
        h.event(button(slip, false));
        h.step();
        let st = h.state();
        assert_eq!(st.pcrop.clicks.len(), 1, "the click placed a corner: {:?}", st.pcrop.clicks);
        assert_eq!(st.ui.perspective_crop, None);
        // A new box dragged from inside the canvas past its edge does scroll.
        let start = pos2(200.0, 200.0);
        h.event(Event::PointerMoved(start));
        h.event(button(start, true));
        h.step();
        for p in [pos2(140.0, 220.0), pos2(50.0, 220.0)] {
            h.event(Event::PointerMoved(p));
            h.step();
        }
        let before = h.state().ui.views[0].center;
        h.run_steps(10);
        let st = h.state();
        assert!(matches!(st.pcrop.drag, Some(Drag::New { .. })), "{:?}", st.pcrop.drag);
        assert!(st.ui.views[0].center[0] < before[0], "the box drag scrolled left: {:?} -> {:?}", before, st.ui.views[0].center);
        h.event(button(pos2(50.0, 220.0), false));
        h.step();
    }

    /// A box drag makes the quad, corners drag alone, and ↵'s commit crops the document to the
    /// quad's size; four clicks in any order make a quad in TL, TR, BR, BL order.
    #[test]
    fn drag_a_box_move_a_corner_and_commit() {
        let doc = Document::with_background("p", Size::new(200, 150), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut s = photocraft_engine::Session::new();
        s.add_document(doc, None);
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.ui.tool = Tool::PerspectiveCrop;
        let ev = |app: &mut PhotocraftApp, e| assert!(pointer(app, e, egui::Modifiers::NONE));
        ev(&mut app, ToolEvent::Down { x: 20.0, y: 10.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Move { x: 120.0, y: 90.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 120.0, y: 90.0 });
        assert_eq!(app.ui.perspective_crop, Some([[20.0, 10.0], [120.0, 10.0], [120.0, 90.0], [20.0, 90.0]]));
        ev(&mut app, ToolEvent::Down { x: 121.0, y: 89.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 141.0, y: 99.0 });
        assert_eq!(app.ui.perspective_crop.unwrap()[2], [140.0, 100.0]);
        commit(&mut app);
        assert!(app.ui.perspective_crop.is_none());
        let size = app.session.active().unwrap().doc.size;
        assert_eq!((size.width, size.height), (120, 92), "the quad's longest edges: no camera at the centre can see this rectangle");

        assert_eq!(ordered([[10.0, 90.0], [100.0, 5.0], [0.0, 0.0], [90.0, 100.0]]), [[0.0, 0.0], [100.0, 5.0], [90.0, 100.0], [10.0, 90.0]]);
    }

    /// ↵ with W and H empty gives the command's perspective size, not the quad's edges (#11798).
    #[test]
    fn commit_with_w_and_h_empty_takes_the_perspective_size() {
        let doc = Document::with_background("p", Size::new(800, 500), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut s = photocraft_engine::Session::new();
        s.add_document(doc, None);
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.ui.tool = Tool::PerspectiveCrop;
        app.ui.perspective_crop = Some([[100.0, 50.0], [700.0, 80.0], [760.0, 450.0], [40.0, 420.0]]);
        commit(&mut app);
        assert!(app.ui.perspective_crop.is_none());
        let size = app.session.active().unwrap().doc.size;
        assert!(size.width.abs_diff(731) <= 1 && size.height.abs_diff(588) <= 1, "{} x {}", size.width, size.height);
    }

    /// Clicking ⇄ in the rendered options bar swaps W and H, although Perspective Crop never sets
    /// the Crop tool's W x H x Resolution mode (#7059).
    #[test]
    fn the_options_bar_swap_button_swaps_w_and_h() {
        use egui_kittest::kittest::Queryable;
        let doc = Document::with_background("p", Size::new(200, 150), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut s = photocraft_engine::Session::new();
        s.add_document(doc, None);
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.ui.tool = Tool::PerspectiveCrop;
        app.ui.tool_options.crop_width = "4 in".into();
        app.ui.tool_options.crop_height = "6 in".into();
        assert_ne!(app.ui.tool_options.crop_ratio, crate::crop_size::WHR);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 60.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                crate::panels::options_bar(app, ui);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(4);
        h.get_by_label("Swaps height and width").click();
        h.run_steps(2);
        let o = &h.state().ui.tool_options;
        assert_eq!((o.crop_width.as_str(), o.crop_height.as_str()), ("6 in", "4 in"));
    }

    /// The pending quad belongs to its document (#1918): switching documents drops it, so it is
    /// never drawn on the other one; on its own document it stays.
    #[test]
    fn switching_documents_drops_the_pending_quad() {
        let mut s = photocraft_engine::Session::new();
        for name in ["a", "b"] {
            s.add_document(Document::with_background(name, Size::new(200, 150), ColorMode::Rgb, SampleType::U8, Color::WHITE), None);
        }
        s.set_active(0);
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.ui.tool = Tool::PerspectiveCrop;
        let ev = |app: &mut PhotocraftApp, e| assert!(pointer(app, e, egui::Modifiers::NONE));
        ev(&mut app, ToolEvent::Down { x: 20.0, y: 10.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Move { x: 120.0, y: 90.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 120.0, y: 90.0 });
        app.sync_views();
        assert!(app.ui.perspective_crop.is_some(), "same document: the quad stays");
        app.session.set_active(1);
        app.sync_views();
        assert_eq!(app.ui.perspective_crop, None);
        assert!(app.pcrop.drag.is_none() && app.pcrop.clicks.is_empty());
    }
}
