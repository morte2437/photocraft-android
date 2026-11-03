//! View extras: rulers (⌘R), grid (⌘'), guides (⌘;). Guides are dragged out of the rulers and
//! moved with the Move tool (drag one off the canvas or onto a ruler to delete it), all through
//! undoable engine commands (`view.newGuide` / `view.moveGuide` / `view.deleteGuide`).

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use photocraft_doc::Document;
use photocraft_engine::prefs::{GuidesGridAndSlices, Unit};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::theme::Tokens;

pub const RULER: f32 = 16.0;

/// A `#rrggbb` preference colour.
fn pref_color(s: &str, fallback: Color32) -> Color32 {
    photocraft_engine::prefs::parse_hex(s).map_or(fallback, |c| Color32::from_rgb(c[0], c[1], c[2]))
}

/// Draw a line in a Guides/Grid preference style (lines, dashed lines, dots).
fn styled_line(painter: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke, style: photocraft_engine::prefs::LineStyle) {
    use photocraft_engine::prefs::LineStyle;
    match style {
        LineStyle::Lines => {
            painter.line_segment([a, b], stroke);
        }
        LineStyle::Dashed => {
            painter.extend(egui::Shape::dashed_line(&[a, b], stroke, 4.0, 3.0));
        }
        LineStyle::Dots => {
            painter.extend(egui::Shape::dotted_line(&[a, b], stroke.color, 3.0, 0.6));
        }
    }
}

/// Ruler step in units so labelled ticks are at least `min` units apart: …0.1, 0.25, 0.5, 1, 2, 5,
/// 10… (whole steps only for pixels).
fn nice_step(min: f64, whole: bool) -> f64 {
    let mut step = if whole { 1.0 } else { 0.001 };
    loop {
        for m in [1.0, 2.0, 2.5, 5.0] {
            if (m != 2.5 || !whole) && step * m >= min {
                return step * m;
            }
        }
        step *= 10.0;
    }
}

/// English display name for a length unit, as shown in Preferences › Units & Rulers.
fn unit_label(u: Unit) -> &'static str {
    match u {
        Unit::Pixels => "Pixels",
        Unit::Inches => "Inches",
        Unit::Centimeters => "Centimeters",
        Unit::Millimeters => "Millimeters",
        Unit::Points => "Points",
        Unit::Picas => "Picas",
        Unit::Percent => "Percent",
    }
}

/// Every length unit, in the Preferences › Units & Rulers order.
const UNITS: [Unit; 7] = [Unit::Pixels, Unit::Inches, Unit::Centimeters, Unit::Millimeters, Unit::Points, Unit::Picas, Unit::Percent];

/// The units Photoshop's Create Rectangle (and the other shape dialogs) offer on a right-click of
/// Width or Height: the lengths, without Percent.
pub(crate) const LENGTH_UNITS: [Unit; 6] = [Unit::Pixels, Unit::Inches, Unit::Centimeters, Unit::Millimeters, Unit::Points, Unit::Picas];

/// The ruler right-click menu: every unit of `units` with its (translated) label and whether it's
/// the one in effect. Picking one changes the ruler unit exactly as the preference pane does.
fn unit_menu_items(current: Unit, units: &[Unit]) -> Vec<(String, &'static str, bool)> {
    units.iter().map(|&u| (tl!(unit_label(u)).to_string(), u.name(), u == current)).collect()
}

/// The length units menu that right-clicking a ruler (or Free Transform's W and H fields) opens;
/// the chosen unit's name is returned (`None` if the menu is dismissed without a pick).
pub(crate) fn unit_menu(ui: &mut egui::Ui, current: Unit) -> Option<&'static str> {
    unit_menu_of(ui, current, &UNITS)
}

/// [`unit_menu`] with only `units` in it.
pub(crate) fn unit_menu_of(ui: &mut egui::Ui, current: Unit, units: &[Unit]) -> Option<&'static str> {
    crate::widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(150.0);
        let mut chosen = None;
        for (label, name, checked) in unit_menu_items(current, units) {
            let label = if checked { format!("✓ {label}") } else { label };
            if ui.button(label).clicked() {
                ui.close();
                chosen = Some(name);
            }
        }
        chosen
    })
}

/// A guide being dragged: from a ruler (new) or an existing one (index).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GuideDrag {
    pub vertical: bool,
    pub index: Option<usize>,
    pub pos: f64,
    /// Where the pointer went down, on the guide's axis: released there, an existing guide is
    /// clicked (selected), not moved.
    pub press: f64,
    /// Whether the pointer was over a ruler when the guide was let go (#6286): a guide released
    /// there is removed, whatever its own axis reads.
    pub over_ruler: bool,
    pub flipped: bool,
}

impl GuideDrag {
    pub fn shows_vertical(&self) -> bool {
        self.vertical != self.flipped
    }
    pub fn track(&mut self, at: [f64; 2], alt: bool) {
        self.flipped = alt;
        self.pos = if self.shows_vertical() { at[0] } else { at[1] };
    }

    /// An existing guide taken at document point `p`.
    pub fn existing(vertical: bool, index: usize, p: [f64; 2]) -> Self {
        let at = if vertical { p[0] } else { p[1] };
        Self { vertical, index: Some(index), pos: at, press: at, over_ruler: false, flipped: false }
    }
}

/// Canvas area left after the rulers.
pub fn content_rect(app: &PhotocraftApp, rect: Rect) -> Rect {
    if app.ui.extras.rulers { Rect::from_min_max(rect.min + vec2(RULER, RULER), rect.max) } else { rect }
}

/// Whether the screen point `p` is over a ruler, inside the full canvas rect `full`. A guide let go
/// there is removed, as in Photoshop, and either ruler removes it (#6286): the top ruler measures
/// document x and the left one y, so the ruler the guide was not dragged out of can lie over a
/// coordinate that is still inside the document on the guide's own axis.
pub(crate) fn dropped_on_a_ruler(app: &PhotocraftApp, full: Rect, p: Pos2) -> bool {
    if !app.ui.extras.rulers {
        return false;
    }
    // The rulers take the strips [`content_rect`] trims off the canvas rect, and nothing else: a
    // canvas point on the boundary is canvas.
    let content = content_rect(app, full);
    full.contains(p) && (p.x < content.min.x || p.y < content.min.y)
}

/// Tick step (document px) so major ticks are at least ~60 screen px apart: 1, 2, 5, 10, 20, 50…
#[cfg(test)]
fn tick_step(zoom: f32) -> f64 {
    nice_step(60.0 / zoom.max(1e-4) as f64, true)
}

/// Grid spacing in document pixels. Percentages use each axis's own extent.
pub(crate) fn grid_major_px(g: &GuidesGridAndSlices, dpi: f64, size: [f64; 2], ppi: f64) -> [f64; 2] {
    size.map(|extent| g.major_px(dpi, extent, ppi))
}

/// View › Show › Grid, from Preferences › Guides, Grid & Slices (spacing, subdivisions, colour,
/// style). Photoshop's default is a gridline every inch with 4 subdivisions.
pub fn draw_grid(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, doc: &Document) {
    let g = &app.session.prefs().guides_grid_and_slices;
    let ppi = app.session.prefs().units_and_rulers.point_size.per_inch();
    let (w, h) = (doc.size.width as f64, doc.size.height as f64);
    let major = grid_major_px(g, doc.resolution_dpi.max(1.0) as f64, [w, h], ppi);
    let minor = major.map(|step| step / g.subdivisions.max(1) as f64);
    let base = pref_color(&g.grid_color, Color32::from_gray(140));
    for (steps, alpha) in [(minor, 0.45f32), (major, 1.0)] {
        if alpha < 1.0 && g.subdivisions <= 1 {
            continue;
        }
        let st = Stroke::new(1.0, base.gamma_multiply(alpha));
        let [step_x, step_y] = steps;
        if step_x * (xf.zoom_x() as f64) >= 6.0 {
            let mut x = step_x;
            while x < w {
                styled_line(painter, xf.to_screen(x as f32, 0.0), xf.to_screen(x as f32, h as f32), st, g.grid_style);
                x += step_x;
            }
        }
        if step_y * (xf.zoom as f64) >= 6.0 {
            let mut y = step_y;
            while y < h {
                styled_line(painter, xf.to_screen(0.0, y as f32), xf.to_screen(w as f32, y as f32), st, g.grid_style);
                y += step_y;
            }
        }
    }
}

/// Screen end points of a guide across `clip`: the document line x = `pos` (vertical) or
/// y = `pos`, spanning every document point the clip rect shows. Guides belong to the document,
/// so on a rotated view they turn with the image (#3277); unrotated this is the clip's full
/// height (or width).
fn guide_segment(xf: &ViewXform, clip: Rect, vertical: bool, pos: f64) -> [Pos2; 2] {
    let axis = usize::from(vertical);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for c in [clip.left_top(), clip.right_top(), clip.right_bottom(), clip.left_bottom()] {
        let d = xf.to_doc(c)[axis];
        lo = lo.min(d);
        hi = hi.max(d);
    }
    let (pos, lo, hi) = (pos as f32, lo as f32, hi as f32);
    if vertical { [xf.to_screen(pos, lo), xf.to_screen(pos, hi)] } else { [xf.to_screen(lo, pos), xf.to_screen(hi, pos)] }
}

/// The cursor over a guide: the arrow across it as the view shows it. A vertical guide moves left
/// and right in the document; a rotated view turns that, as it turns the guide (#3277).
pub fn guide_cursor(xf: &ViewXform, vertical: bool) -> egui::CursorIcon {
    crate::tool_cursor::resize(if vertical { [1.0, 0.0] } else { [0.0, 1.0] }, xf)
}

pub fn draw_guides(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, doc: &Document) {
    let clip = painter.clip_rect();
    let drag = app.guide_drag;
    let g = &app.session.prefs().guides_grid_and_slices;
    let preview = crate::guide_layout_ui::preview(app, doc);
    let guides = preview.as_ref().map(|(guides, _)| guides).unwrap_or(&doc.guides);
    let (guide, style) =
        (preview.as_ref().map(|(_, color)| *color).unwrap_or_else(|| pref_color(&g.guide_color, Color32::from_rgb(74, 255, 255))), g.guide_style);
    let line = |vertical: bool, pos: f64, color: Color32| {
        let [a, b] = guide_segment(xf, clip, vertical, pos);
        styled_line(painter, a, b, Stroke::new(1.0, color), style);
    };
    // A guide given a colour in New Guide keeps it; the rest use the preference colour (or, while
    // New Guide Layout previews, the colour it will set). The selected guide is Photoshop's blue.
    let own = |vertical: bool, p: f64| {
        if crate::guide_select::is_selected(app, vertical, p) {
            return crate::guide_select::SELECTED_COLOR;
        }
        guides.color(vertical, p).map_or(guide, |[r, g, b]| Color32::from_rgb(r, g, b))
    };
    if app.ui.extras.guides || preview.is_some() {
        for (vertical, list) in [(true, &guides.vertical), (false, &guides.horizontal)] {
            for (i, p) in list.iter().enumerate() {
                if drag.is_some_and(|d| d.vertical == vertical && d.index == Some(i)) {
                    continue;
                }
                line(vertical, *p, own(vertical, *p));
            }
        }
    }
    if let Some(d) = drag {
        // A guide being moved shows its own colour; a new one dragged from a ruler the preference's.
        let list = if d.vertical { &doc.guides.vertical } else { &doc.guides.horizontal };
        line(d.shows_vertical(), d.pos, d.index.and_then(|i| list.get(i)).map_or(guide, |p| own(d.vertical, *p)));
    }
}

/// The position readout of a guide being dragged: `X:` for a vertical guide, `Y:` for a horizontal
/// one, measured from the ruler zero point in the ruler unit. Pixels keep one decimal, as
/// Photoshop's "X: 103.2 px".
pub fn guide_drag_readout(app: &PhotocraftApp, d: GuideDrag) -> Option<(&'static str, String)> {
    let doc = &app.session.active()?.doc;
    let ur = &app.session.prefs().units_and_rulers;
    let axis = usize::from(!d.shows_vertical());
    let px = d.pos - doc.ruler_origin.get(axis).copied().unwrap_or(0.0);
    let extent = if d.shows_vertical() { doc.size.width } else { doc.size.height } as f64;
    let value = match ur.rulers {
        Unit::Pixels => photocraft_engine::prefs::fmt_decimals(px, 1),
        _ => ur.format(px, f64::from(doc.resolution_dpi), extent),
    };
    Some((if d.shows_vertical() { "X:" } else { "Y:" }, format!("{value} {}", ur.rulers.suffix())))
}

/// Draw [`guide_drag_readout`] beside the pointer while a guide is dragged.
pub fn draw_guide_drag_readout(app: &PhotocraftApp, ctx: &egui::Context) {
    let (Some(d), Some(at)) = (app.guide_drag, ctx.pointer_latest_pos()) else { return };
    if let Some((label, value)) = guide_drag_readout(app, d) {
        crate::canvas::draw_value_readout(app, ctx, "guide-drag-readout", at, [label], [value]);
    }
}

/// Existing guide under a document point (within 4 screen px).
pub fn guide_at(app: &PhotocraftApp, x: f64, y: f64) -> Option<(bool, usize)> {
    // Hidden canvas guides are not interactive: the Move tool must reach the layer below.
    // Keep hit testing consistent with the visibility gate in canvas::draw.
    if !app.ui.extras.guides || !app.ui.view.shows(app.ui.view.show.canvas_guides) || app.ui.extras.lock_guides {
        return None;
    }
    let doc = &app.session.active()?.doc;
    let tol = 4.0 / app.point_zoom().max(0.01) as f64;
    for (i, g) in doc.guides.vertical.iter().enumerate() {
        if (*g - x).abs() <= tol {
            return Some((true, i));
        }
    }
    for (i, g) in doc.guides.horizontal.iter().enumerate() {
        if (*g - y).abs() <= tol {
            return Some((false, i));
        }
    }
    None
}

/// Finish a guide drag: create, move, or delete (dropped outside the canvas or on a ruler). An
/// existing guide released where it was pressed is a click: it's selected and stays put, as in
/// Photoshop; a moved one stays selected.
pub fn finish_drag(app: &mut PhotocraftApp, d: GuideDrag) {
    let Some(size) = app.session.active().map(|s| s.doc.size) else { return };
    // A drop on a ruler removes the guide even when the drag held its own axis, which otherwise
    // reads as a click: dragged sideways onto the left ruler, a horizontal guide never moves in y.
    if let Some(i) = d.index
        && !d.over_ruler
        && !d.flipped
        && (d.pos - d.press).abs() * f64::from(app.point_zoom().max(0.01)) < 0.5
    {
        crate::guide_select::select(app, d.vertical, i);
        return;
    }
    let shows = d.shows_vertical();
    let extent = if shows { size.width } else { size.height } as f64;
    let orientation = if d.vertical { "vertical" } else { "horizontal" };
    let inside = !d.over_ruler && d.pos >= 0.0 && d.pos <= extent;
    let pos = (d.pos * 1000.0).round() / 1000.0;
    let _ = match (d.index, inside) {
        (None, true) => app.run("view.newGuide", json!({"orientation": if shows {"vertical"}else{"horizontal"}, "position": pos})),
        (Some(i), true) => app
            .run("view.moveGuide", json!({"orientation": orientation,"toOrientation":if shows {"vertical"}else{"horizontal"}, "index": i, "position": pos}))
            .inspect(|_| {
                let index = if d.flipped {
                    app.session.active().map_or(0, |st| if shows { st.doc.guides.vertical.len() } else { st.doc.guides.horizontal.len() }).saturating_sub(1)
                } else {
                    i
                };
                crate::guide_select::select(app, shows, index);
            }),
        (Some(i), false) => app.run("view.deleteGuide", json!({"orientation": orientation, "index": i})),
        (None, false) => Ok(serde_json::Value::Null),
    };
    if d.index.is_none() && inside {
        app.ui.extras.guides = true;
    }
}

/// The mapping the rulers measure with. Rulers are screen chrome that stays level while
/// View › Rotate View turns the canvas, so they use the view without its rotation: the same zoom,
/// flip and pixel aspect (ticks never stretch), anchored so that 0 sits at the screen position of
/// the document origin as the rotated view draws it, as in Photoshop. Tilted guides then meet the
/// rulers near their numbers. Measuring along the rotated axes instead squeezed the ticks into a
/// few pixels (or none at 90°) and left the rest of the ruler empty (#3233).
///
/// The returned mapping measures from the rulers' zero point `zero` (document px, #2955): its
/// document coordinates are distances from `zero`, and 0 sits where the canvas draws `zero`.
fn ruler_xform(xf: &ViewXform, zero: [f64; 2]) -> ViewXform {
    let level = ViewXform { rotation: 0.0, ..*xf };
    let (zx, zy) = (zero[0] as f32, zero[1] as f32);
    // The level view is an axis-aligned scale, so moving the zero point by `shift` on screen moves
    // every point by it: re-centre on the document point that sat `shift` before the centre, then
    // count from the zero point.
    let shift = xf.to_screen(zx, zy) - level.to_screen(zx, zy);
    let center = level.to_doc(xf.rect.center() - shift);
    let center = [center[0] as f32 - zx, center[1] as f32 - zy];
    if !(center[0].is_finite() && center[1].is_finite()) {
        return level;
    }
    ViewXform { center, ..level }
}

/// Visible ruler range in ruler units, normalised to `(min, max)` so the tick loop runs for
/// flipped views too (#1814), measured from the zero point `zero`. `xf` is the canvas mapping;
/// rotation is ignored (see [`ruler_xform`]). Shared by the renderer and its tests.
fn ruler_range(xf: &ViewXform, a: Pos2, b: Pos2, axis: usize, px_per_unit: f64, zero: [f64; 2]) -> (f64, f64) {
    let rxf = ruler_xform(xf, zero);
    let d0 = rxf.to_doc(a)[axis];
    let d1 = rxf.to_doc(b)[axis];
    let (u0, u1) = (d0 / px_per_unit, d1 / px_per_unit);
    (u0.min(u1), u0.max(u1))
}

/// Snap a document position to the nearest ruler tick, measured from the zero point `origin`
/// (`tick` px apart). Shift-dragging the zero point lands it on a tick, as in Photoshop.
fn snap_px_to_tick(px: f64, origin: f64, tick: f64) -> f64 {
    if !(tick.is_finite() && tick > 0.0) {
        return px;
    }
    origin + ((px - origin) / tick).round() * tick
}

/// Where a zero-point drag from the ruler corner lands for the pointer at document `d`: on the
/// nearest ruler tick (`ticks`, px per axis) with Shift, else View › Snap To targets.
fn origin_drop(app: &mut PhotocraftApp, d: [f64; 2], shift: bool, origin: [f64; 2], ticks: [f64; 2]) -> [f64; 2] {
    if shift {
        [snap_px_to_tick(d[0], origin[0], ticks[0]), snap_px_to_tick(d[1], origin[1], ticks[1])]
    } else {
        [crate::snap_ui::snap_guide(app, true, d[0]), crate::snap_ui::snap_guide(app, false, d[1])]
    }
}

/// Screen coordinate along the ruler (x for the top ruler, y for the left one) of the value `u`
/// in ruler units from the zero point `zero`. The inverse of [`ruler_range`]'s mapping.
fn ruler_pos(xf: &ViewXform, vertical: bool, u: f64, px_per_unit: f64, zero: [f64; 2]) -> f32 {
    let px = (u * px_per_unit) as f32;
    let rxf = ruler_xform(xf, zero);
    if vertical { rxf.to_screen(0.0, px).y } else { rxf.to_screen(px, 0.0).x }
}

/// Document pixels per ruler unit and the labelled tick step (in ruler units) of the top ruler
/// (`y_axis` false) or the left one, from Preferences › Units & Rulers and the zoom. Minor ticks
/// sit at tenths of the step.
fn ruler_scale(app: &PhotocraftApp, xf: &ViewXform, y_axis: bool) -> (f64, f64) {
    ruler_scale_of(app, app.session.active().map(|d| &*d.doc), xf, y_axis)
}

/// [`ruler_scale`] for the view of `doc`.
fn ruler_scale_of(app: &PhotocraftApp, doc: Option<&photocraft_doc::Document>, xf: &ViewXform, y_axis: bool) -> (f64, f64) {
    let ur = &app.session.prefs().units_and_rulers;
    let (unit, ppi) = (ur.rulers, ur.point_size.per_inch());
    let (dpi, size) = doc.map_or((72.0, [1.0, 1.0]), |d| (d.resolution_dpi.max(1.0) as f64, [d.size.width as f64, d.size.height as f64]));
    let px_per_unit = unit.to_px(1.0, dpi, if y_axis { size[1] } else { size[0] }, ppi).max(1e-9);
    // The top ruler runs along document x, which pixel aspect correction stretches.
    let zoom = if y_axis { xf.zoom } else { xf.zoom_x() };
    (px_per_unit, nice_step(60.0 / (zoom as f64 * px_per_unit).max(1e-6), unit == Unit::Pixels))
}

/// `pos` (document px) moved to the nearest tick mark the ruler draws (counted from the zero point): where a guide lands when
/// Shift is held while dragging it out of a ruler, as in Photoshop (#3770).
fn snap_to_tick(app: &PhotocraftApp, xf: &ViewXform, vertical: bool, pos: f64) -> f64 {
    // A vertical guide sits at an x position, which the top ruler measures.
    let (px_per_unit, step) = ruler_scale(app, xf, !vertical);
    // Ticks count from the zero point (#2955), which a vertical guide measures along x.
    let origin = app.session.active().map_or(0.0, |d| d.doc.ruler_origin[usize::from(!vertical)]);
    snap_px_to_tick(pos, origin, step / 10.0 * px_per_unit)
}

/// The number a ruler prints at the tick `v` (in ruler units). Like Photoshop, rulers show the
/// distance from the origin, so values left of / above it read without a minus sign.
fn ruler_label(v: f64, step: f64, whole: bool) -> String {
    let v = v.abs();
    if step >= 1.0 || whole { format!("{}", v.round() as i64) } else { crate::widgets::fmt_num2(v) }
}

/// The rulers of `doc`'s view in `full`: background, ticks and labels from its zero point.
/// Returns the top, left and corner rects, the painter, the zero point and the minor tick sizes.
#[allow(clippy::type_complexity)]
fn paint(
    app: &PhotocraftApp,
    ui: &egui::Ui,
    full: Rect,
    xf: &ViewXform,
    doc: &photocraft_doc::Document,
) -> (Rect, Rect, Rect, egui::Painter, [f64; 2], [f64; 2]) {
    let t = Tokens::get(ui.ctx());
    let top = Rect::from_min_max(pos2(full.left() + RULER, full.top()), pos2(full.right(), full.top() + RULER));
    let left = Rect::from_min_max(pos2(full.left(), full.top() + RULER), pos2(full.left() + RULER, full.bottom()));
    let corner = Rect::from_min_size(full.min, vec2(RULER, RULER));
    let p = ui.painter_at(full);
    let bg = t.chrome;
    for r in [top, left, corner] {
        p.rect_filled(r, 0.0, bg);
    }
    p.line_segment([top.left_bottom(), top.right_bottom()], Stroke::new(1.0, t.separator));
    p.line_segment([left.right_top(), left.right_bottom()], Stroke::new(1.0, t.separator));
    let font = egui::FontId::proportional(9.0);
    let tick = Stroke::new(1.0, t.text_faint);
    // Preferences › Units & Rulers: labels in the ruler unit (percent of the document's side).
    let unit = app.session.prefs().units_and_rulers.rulers;
    let whole = unit == Unit::Pixels;
    // The zero point (#2955): rulers read 0 there and count the distance from it.
    let origin = doc.ruler_origin;
    let mut ticks = [1.0; 2];
    for vertical in [false, true] {
        let p = p.with_clip_rect(if vertical { left } else { top });
        let (px_per_unit, step) = ruler_scale_of(app, Some(doc), xf, vertical);
        ticks[usize::from(vertical)] = step * px_per_unit / 10.0;
        let (u0, u1) = if vertical {
            ruler_range(xf, left.left_top(), left.left_bottom(), 1, px_per_unit, origin)
        } else {
            ruler_range(xf, top.left_top(), top.right_top(), 0, px_per_unit, origin)
        };
        let mut v = (u0 / step).floor() * step;
        while v <= u1 {
            let at = |u: f64| -> f32 { ruler_pos(xf, vertical, u, px_per_unit, origin) };
            let c = at(v);
            if vertical {
                p.line_segment([pos2(left.right() - RULER, c), pos2(left.right(), c)], tick);
                // Labels stacked, as in Photoshop, each character centred in the ruler's width
                // (a narrow "1" used to hug the left edge while a "0" filled the strip).
                for (i, ch) in ruler_label(v, step, whole).chars().enumerate() {
                    p.text(pos2(left.left() + RULER * 0.5, c + 2.0 + i as f32 * 8.5), Align2::CENTER_TOP, ch, font.clone(), t.text_dim);
                }
            } else {
                p.line_segment([pos2(c, top.bottom() - RULER), pos2(c, top.bottom())], tick);
                p.text(pos2(c + 2.0, top.top() + 1.0), Align2::LEFT_TOP, ruler_label(v, step, whole), font.clone(), t.text_dim);
            }
            for k in 1..10 {
                let m = at(v + step * k as f64 / 10.0);
                let len = if k == 5 { 6.0 } else { 3.0 };
                if vertical {
                    p.line_segment([pos2(left.right() - len, m), pos2(left.right(), m)], tick);
                } else {
                    p.line_segment([pos2(m, top.bottom() - len), pos2(m, top.bottom())], tick);
                }
            }
            v += step;
        }
    }
    (top, left, corner, p, origin, ticks)
}

/// Rulers for a document view that isn't the active one (another tile of Window › Arrange, a
/// floating window): shown as the active view's are, but guides and the zero point are dragged
/// in the active view (#6017).
pub fn paint_rulers(app: &PhotocraftApp, ui: &egui::Ui, full: Rect, xf: &ViewXform, doc: &photocraft_doc::Document) {
    paint(app, ui, full, xf, doc);
}

/// Rulers along the top and left of `full` (the canvas rect before `content_rect`), with the
/// pointer position marked; dragging out of a ruler creates a guide.
pub fn draw_rulers(app: &mut PhotocraftApp, ui: &mut egui::Ui, full: Rect, xf: &ViewXform) {
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    let (top, left, corner, p, origin, ticks) = paint(app, ui, full, xf, &doc);
    let t = Tokens::get(ui.ctx());
    let unit = app.session.prefs().units_and_rulers.rulers;
    // Pointer position markers.
    if let Some(h) = ui.ctx().pointer_hover_pos().filter(|h| full.contains(*h)) {
        let m = Stroke::new(1.0, t.text);
        p.with_clip_rect(top).line_segment([pos2(h.x, top.top()), pos2(h.x, top.bottom())], m);
        p.with_clip_rect(left).line_segment([pos2(left.left(), h.y), pos2(left.right(), h.y)], m);
    }
    // Drag a new guide out of a ruler; right-click to switch the ruler unit.
    let mut chosen_unit: Option<&'static str> = None;
    for (r, vertical, salt) in [(top, false, "ruler-top"), (left, true, "ruler-left"), (corner, false, "ruler-corner")] {
        // Guides drag out of the top/left rulers and the zero point out of the corner; all three
        // sense clicks so the right-click unit menu opens anywhere on the ruler.
        let resp = ui.interact(r, ui.scope_id().with(salt), Sense::click_and_drag());
        if salt == "ruler-corner" {
            corner_origin(app, ui, &resp, &p, xf, origin, ticks);
        } else if resp.hovered() {
            ui.ctx().set_cursor_icon(if vertical { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
        }
        if let Some(pos) = resp.interact_pointer_pos().filter(|_| salt != "ruler-corner" && (resp.dragged() || resp.drag_started())) {
            // ⌥/Alt turns the guide the other way, as in Photoshop: a vertical guide out of the top
            // ruler, a horizontal one out of the left (#5698), switching as Alt goes up and down.
            let vertical = vertical != ui.input(|i| i.modifiers.alt);
            let d = xf.to_doc(pos);
            let along = if vertical { d[0] } else { d[1] };
            let pos = if ui.input(|i| i.modifiers.shift) { snap_to_tick(app, xf, vertical, along) } else { crate::snap_ui::snap_guide(app, vertical, along) };
            app.guide_drag = Some(GuideDrag { vertical, index: None, pos, press: pos, over_ruler: false, flipped: false });
        }
        if resp.drag_stopped()
            && let Some(mut d) = app.guide_drag.take()
        {
            // Let go over a ruler (the other one, say) the drag is discarded, not placed (#6286).
            d.over_ruler = resp.interact_pointer_pos().is_some_and(|p| dropped_on_a_ruler(app, full, p));
            finish_drag(app, d);
        }
        resp.context_menu(|ui| {
            if let Some(name) = unit_menu(ui, unit) {
                chosen_unit = Some(name);
            }
        });
    }
    if let Some(name) = chosen_unit {
        let _ = app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": name}));
    }
    let _ = Pos2::ZERO;
}

/// The ruler corner: drag the zero point out of it (cross hairs mark where it lands; Shift snaps
/// to the ruler ticks), double-click it to put the zero point back at the image's top-left.
fn corner_origin(app: &mut PhotocraftApp, ui: &egui::Ui, resp: &egui::Response, p: &egui::Painter, xf: &ViewXform, origin: [f64; 2], ticks: [f64; 2]) {
    let id = resp.id.with("origin-drag");
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if resp.double_clicked() {
        let _ = app.run("view.resetRulerOrigin", json!({}));
        return;
    }
    // Dropped back on the corner (or a slow click), the zero point stays where it was.
    if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.drag_started()) {
        if resp.rect.contains(pos) {
            ui.data_mut(|d| d.remove_temp::<[f64; 2]>(id));
            return;
        }
        let shift = ui.input(|i| i.modifiers.shift);
        let at = origin_drop(app, xf.to_doc(pos), shift, origin, ticks);
        ui.data_mut(|d| d.insert_temp(id, at));
        // Cross hairs along the document axes through the new zero point, drawn like guides so
        // they turn with a rotated view.
        let t = Tokens::get(ui.ctx());
        for (vertical, pos) in [(true, at[0]), (false, at[1])] {
            p.line_segment(guide_segment(xf, p.clip_rect(), vertical, pos), Stroke::new(1.0, t.text));
        }
    }
    if resp.drag_stopped()
        && let Some(at) = ui.data_mut(|d| d.remove_temp::<[f64; 2]>(id))
    {
        let round = |v: f64| (v * 1000.0).round() / 1000.0;
        let _ = app.run("view.setRulerOrigin", json!({"x": round(at[0]), "y": round(at[1])}));
    }
}

#[cfg(test)]
mod tests;
