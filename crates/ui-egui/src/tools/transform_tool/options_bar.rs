use super::begin::{enter_warp, interpolation_preference, leave_warp, warps_type};
use super::commit::{cancel, commit};
use super::pointer::point_on_quad;
use super::warp::edit_session_warp;
use super::{BezierMesh, Homography, PhotocraftApp, SplitTool, Stroke, TransformSession, Warp, WarpStyle, json, vec2};

/// How much the box enlarges its pixels: the longest edge relative to the original (picks the
/// preview's texture level and filter).
pub(super) fn quad_scale(t: &TransformSession) -> f32 {
    let (w0, h0) = ((t.rect[2] - t.rect[0]).max(1e-9), (t.rect[3] - t.rect[1]).max(1e-9));
    let len = |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]).hypot(b[1] - a[1]);
    let q = &t.quad;
    let s = (len(q[0], q[1]) / w0).max(len(q[3], q[2]) / w0).max(len(q[0], q[3]) / h0).max(len(q[1], q[2]) / h0);
    if s.is_finite() { s as f32 } else { 1.0 }
}

/// W and H (%), the angle (°) and the horizontal skew (°) the options bar shows for the box.
/// A mirrored box reads as a negative W or H, whichever keeps the angle nearer 0 (Photoshop:
/// Flip Horizontal is W -100 % at 0°, Flip Vertical H -100 % at 0°). Both are the same box:
/// negating W and H and turning by a half turn cancel out.
pub(super) fn readout(t: &TransformSession) -> (f64, f64, f64, f64) {
    let p = Parts::of(t);
    let wrap = |a: f64| {
        let a = a.rem_euclid(std::f64::consts::TAU);
        if a > std::f64::consts::PI { a - std::f64::consts::TAU } else { a }
    };
    let turned = wrap(p.angle + std::f64::consts::PI);
    let (sx, sy, angle) = if p.sy < 0.0 && turned.abs() < wrap(p.angle).abs() { (-p.sx, -p.sy, turned) } else { (p.sx, p.sy, p.angle) };
    (sx * 100.0, sy * 100.0, angle.to_degrees(), p.skew_h.to_degrees())
}

/// `v`, kept off zero with its sign: a divisor for a W or H scale that may be negative.
fn nonzero(v: f64) -> f64 {
    if v.abs() < 1e-9 { 1e-9f64.copysign(v) } else { v }
}

/// The box's affine part read as Photoshop's options bar does: the frame scaled (W, H), skewed
/// (H, V) and then rotated about the reference point. Angles are radians; positive turns
/// clockwise on screen, so a positive H skew leans the box's sides like the angle field would
/// turn them, and a positive V skew its top and bottom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Parts {
    pub(super) sx: f64,
    /// Negative when the box is flipped vertically relative to its rotation.
    pub(super) sy: f64,
    pub(super) angle: f64,
    pub(super) skew_h: f64,
    pub(super) skew_v: f64,
}

/// The skew fields' reach: a skew of ±90° would flatten the box.
pub(super) const SKEW_LIMIT_DEG: f32 = 89.0;

impl Parts {
    /// The box's columns: where the frame's unit X and Y edges went (from the top-left corner).
    fn columns(t: &TransformSession) -> Option<[[f64; 2]; 2]> {
        let (w0, h0) = (t.rect[2] - t.rect[0], t.rect[3] - t.rect[1]);
        if w0.abs() < 1e-9 || h0.abs() < 1e-9 {
            return None;
        }
        let q = &t.quad;
        let m = [[(q[1][0] - q[0][0]) / w0, (q[1][1] - q[0][1]) / w0], [(q[3][0] - q[0][0]) / h0, (q[3][1] - q[0][1]) / h0]];
        m.iter().flatten().all(|v| v.is_finite()).then_some(m)
    }

    /// The box as scale, skew and rotation, keeping the session's typed V skew (any box can be
    /// written with any V skew short of ±90°).
    fn of(t: &TransformSession) -> Parts {
        let skew_v = t.skew_v.to_radians();
        let none = Parts { sx: 1.0, sy: 1.0, angle: 0.0, skew_h: 0.0, skew_v };
        Parts::columns(t).map_or(none, |m| Parts::from_columns(m, skew_v))
    }

    pub(super) fn from_columns([c0, c1]: [[f64; 2]; 2], skew_v: f64) -> Parts {
        // X edge = sx · R(angle) · (1, tan v): its direction is the angle plus v.
        let angle = c0[1].atan2(c0[0]) - skew_v;
        let sx = c0[0].hypot(c0[1]) * skew_v.cos();
        // Y edge = sy · R(angle) · (−tan h, 1).
        let (gx, gy) = unturn(c1, angle);
        let skew_h = if gy.abs() < 1e-12 { 0.0 } else { (-gx / gy).atan() };
        Parts { sx, sy: gy, angle, skew_h, skew_v }
    }

    pub(super) fn columns_of(self) -> [[f64; 2]; 2] {
        let (sn, cs) = self.angle.sin_cos();
        let rot = |x: f64, y: f64| [x * cs - y * sn, x * sn + y * cs];
        let c0 = rot(self.sx, self.sx * self.skew_v.tan());
        let c1 = rot(-self.sy * self.skew_h.tan(), self.sy);
        [c0, c1]
    }
}

/// `c` turned back by `angle` (radians).
fn unturn(c: [f64; 2], angle: f64) -> (f64, f64) {
    let (sn, cs) = angle.sin_cos();
    (c[0] * cs + c[1] * sn, -c[0] * sn + c[1] * cs)
}

/// Sets the box's H and V skew (degrees), keeping its scale, angle and reference point: the
/// options bar's skew fields.
fn skew_about_pivot(app: &mut PhotocraftApp, skew_h: f64, skew_v: f64) {
    let Some(s) = app.ui.transform.as_mut() else { return };
    let Some(m) = Parts::columns(s) else { return };
    let limit = f64::from(SKEW_LIMIT_DEG);
    let (skew_h, skew_v) = (skew_h.clamp(-limit, limit), skew_v.clamp(-limit, limit));
    let from = Parts::from_columns(m, s.skew_v.to_radians());
    let to = Parts { skew_h: skew_h.to_radians(), skew_v: skew_v.to_radians(), ..from }.columns_of();
    // The map from the old box to the new one: to · m⁻¹.
    let det = m[0][0] * m[1][1] - m[1][0] * m[0][1];
    if det.abs() < 1e-12 {
        return;
    }
    let inv = [[m[1][1] / det, -m[0][1] / det], [-m[1][0] / det, m[0][0] / det]];
    let l = |x: f64, y: f64| {
        let (u, v) = (inv[0][0] * x + inv[1][0] * y, inv[0][1] * x + inv[1][1] * y);
        [to[0][0] * u + to[1][0] * v, to[0][1] * u + to[1][1] * v]
    };
    let c = s.pivot;
    let quad = s.quad.map(|q| {
        let d = l(q[0] - c[0], q[1] - c[1]);
        [c[0] + d[0], c[1] + d[1]]
    });
    if quad.iter().flatten().all(|v| v.is_finite()) {
        s.quad = quad;
        s.skew_v = skew_v;
    }
}

/// Width of the mode, cancel and commit cluster kept on the right of the options bar.
const ACTIONS_W: f32 = 140.0;

/// Width of the action buttons when they follow the Warp fields (one more than the box bar: Reset).
const WARP_ACTIONS_W: f32 = 176.0;

/// Reach of the reference point X/Y fields: the largest document side (`image.canvasSize`'s limit).
pub(super) const POSITION_LIMIT_PX: f32 = 300_000.0;

/// Reach of the W/H fields. The engine takes any scale; this lets a 30 px layer span the largest
/// document (#1267).
pub(super) const SCALE_LIMIT_PCT: f32 = 1_000_000.0;

/// The W or H field's limit for a box side of `side` px: as far as the result stays within the
/// largest document side, so a typed scale can't ask for an image too large to allocate.
pub(super) fn scale_limit_pct(side: f64) -> f32 {
    let side = side.abs();
    if !side.is_finite() || side < 1e-6 {
        return SCALE_LIMIT_PCT;
    }
    ((f64::from(POSITION_LIMIT_PX) / side * 100.0) as f32).clamp(100.0, SCALE_LIMIT_PCT)
}

/// Where Free Transform remembers the unit of its W and H fields, picked by right-clicking either
/// of them as in Photoshop (Percent until another is picked).
pub(super) const SIZE_UNIT_KEY: &str = "edit.freeTransform";

/// The W/H unit, with the document's resolution and the points per inch it converts with.
pub(super) fn size_unit(app: &PhotocraftApp) -> crate::layer_props_ui::LenUnit {
    use photocraft_engine::prefs::Unit;
    let unit = app.session.prefs().dialogs.get(SIZE_UNIT_KEY).and_then(|v| v.get("sizeUnit")).and_then(serde_json::Value::as_str).and_then(Unit::parse);
    crate::layer_props_ui::LenUnit { unit: unit.unwrap_or(Unit::Percent), ..crate::layer_props_ui::LenUnit::of(app) }
}

pub(super) fn set_size_unit(app: &mut PhotocraftApp, name: &str) {
    app.session.prefs.edit(|p| {
        let entry = p.dialogs.entry(SIZE_UNIT_KEY.to_string()).or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({});
        }
        entry["sizeUnit"] = json!(name);
    });
}

/// A W or H value shown in `u`: the box side `side` px wide at the original size, scaled by `pct`.
/// Percent stays the scale itself (100 % is the original size), lengths are the side's length.
pub(super) fn size_shown(u: &crate::layer_props_ui::LenUnit, pct: f64, side: f64) -> f32 {
    u.shown(pct / 100.0 * side, side)
}

/// The scale (%) a W or H value typed in `u` asks for, for a box side `side` px wide originally.
pub(super) fn size_pct(u: &crate::layer_props_ui::LenUnit, v: f32, side: f64) -> f64 {
    let side = side.abs().max(1e-9);
    u.px(v, side) / side * 100.0
}

/// Options bar while transforming: the fields (reference point X/Y, W/H %, angle, interpolation,
/// or the warp controls), then the mode switch, cancel and commit right after them. A bar too
/// narrow for both pins the buttons to the right edge and clips the fields, so Commit and Cancel
/// are always reachable.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(t) = app.ui.transform.clone() else { return };
    let bar = ui.available_rect_before_wrap();
    let warping = t.warp.is_some();
    let buttons_w = if warping { WARP_ACTIONS_W } else { ACTIONS_W };
    let split = (bar.right() - buttons_w).max(bar.left());
    let fields = egui::Rect::from_min_max(bar.min, egui::pos2(split, bar.bottom()));
    let mut row = ui.new_child(egui::UiBuilder::new().id_salt("transform-fields").max_rect(fields).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.set_clip_rect(fields);
    if let Some(w) = t.warp.clone() {
        warp_fields(app, &mut row, &w);
    } else if t.content_aware.is_some() {
        crate::cas_transform::fields(app, &mut row, &t);
    } else {
        transform_fields(app, &mut row, &t);
    }
    crate::widgets::vline(&mut row, 22.0);
    let start = row.min_rect().right().min(split);
    let actions = egui::Rect::from_min_max(egui::pos2(start, bar.top()), egui::pos2(start + buttons_w, bar.bottom()));
    let mut btns = ui.new_child(egui::UiBuilder::new().id_salt("transform-actions").max_rect(actions).layout(egui::Layout::right_to_left(egui::Align::Center)));
    transform_actions(app, &mut btns, warping, t.content_aware.is_none());
}

fn transform_fields(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &TransformSession) {
    let tk = crate::theme::Tokens::get(ui.ctx());
    let (_, _, angle, skew_h) = readout(t);
    let lbl = |ui: &mut egui::Ui, s: &str| {
        ui.label(egui::RichText::new(s).color(tk.text_dim).size(12.0));
    };
    position_size_fields(app, ui, t);
    crate::widgets::vline(ui, 22.0);
    let mut a = angle as f32;
    let (r, resp) = ui.allocate_exact_size(vec2(18.0, 22.0), egui::Sense::hover());
    crate::icons::paint(ui, r, "rotate-cw", 13.0, tk.text_dim);
    resp.on_hover_text(tl!("Rotate"));
    if crate::widgets::value_field(ui, &mut a, -180.0..=180.0, "°", 60.0).accessible_name(tl!("Rotate")).changed() {
        rotate_about_pivot(app, (a as f64 - angle).to_radians());
    }
    // Photoshop's H and V skew fields: the box's sides lean by H, its top and bottom by V.
    let (mut h, mut v) = (skew_h as f32, t.skew_v as f32);
    let lim = -SKEW_LIMIT_DEG..=SKEW_LIMIT_DEG;
    lbl(ui, "H:");
    let rh =
        crate::widgets::value_field(ui, &mut h, lim.clone(), "°", 56.0).accessible_name(tl!("Set horizontal skew")).on_hover_text(tl!("Set horizontal skew"));
    lbl(ui, "V:");
    let rv = crate::widgets::value_field(ui, &mut v, lim, "°", 56.0).accessible_name(tl!("Set vertical skew")).on_hover_text(tl!("Set vertical skew"));
    if rh.changed() || rv.changed() {
        skew_about_pivot(app, f64::from(h), f64::from(v));
    }
    crate::widgets::vline(ui, 22.0);
    lbl(ui, tl!("Interpolation:"));
    let mut interp = t.interpolation.clone();
    let opts = [("bicubic".to_string(), tl!("Bicubic")), ("bilinear".to_string(), tl!("Bilinear")), ("nearest".to_string(), tl!("Nearest Neighbor"))];
    if crate::widgets::dropdown(ui, "transform-interp", tl!("Interpolation"), &mut interp, &opts, 130.0)
        && let Some(preference) = interpolation_preference(&interp)
        && let Some(s) = app.ui.transform.as_mut()
    {
        s.interpolation = interp;
        app.ui.tool_options.transform_interpolation = Some(preference);
    }
}

pub(super) fn matching_anchor(quad: [[f64; 2]; 4], pivot: [f64; 2]) -> Option<usize> {
    (0..9).find(|&i| {
        let p = point_on_quad(quad, i);
        (p[0] - pivot[0]).abs() <= 0.05 && (p[1] - pivot[1]).abs() <= 0.05
    })
}

fn anchor_grid_widget(ui: &mut egui::Ui, current_anchor: Option<usize>) -> Option<usize> {
    let size = egui::vec2(16.0, 16.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let tk = crate::theme::Tokens::get(ui.ctx());
    let painter = ui.painter();
    painter.rect_stroke(rect, 1.0, egui::Stroke::new(1.0, tk.card_border), egui::StrokeKind::Inside);
    let mut clicked = None;
    let pointer = resp.interact_pointer_pos();
    for row in 0..3 {
        for col in 0..3 {
            let idx = row * 3 + col;
            let cx = rect.min.x + (col as f32 + 0.5) * (rect.width() / 3.0);
            let cy = rect.min.y + (row as f32 + 0.5) * (rect.height() / 3.0);
            let pt_rect = egui::Rect::from_center_size(egui::pos2(cx, cy), egui::vec2(3.5, 3.5));
            if resp.clicked() && pointer.is_some_and(|p| (p.x - cx).abs() <= 3.0 && (p.y - cy).abs() <= 3.0) {
                clicked = Some(idx);
            }
            if current_anchor == Some(idx) {
                painter.rect_filled(pt_rect, 0.5, tk.accent);
            } else {
                painter.rect_filled(pt_rect, 0.5, tk.text_dim);
            }
        }
    }
    resp.on_hover_text(tl!("Reference point location"));
    clicked
}

/// The reference point's X/Y and the W/H scale fields (with the aspect link) of the options bar.
pub(crate) fn position_size_fields(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &TransformSession) {
    let tk = crate::theme::Tokens::get(ui.ctx());
    let (sx, sy, _, _) = readout(t);
    let lbl = |ui: &mut egui::Ui, s: &str| {
        ui.label(egui::RichText::new(s).color(tk.text_dim).size(12.0));
    };
    let current_anchor = matching_anchor(t.quad, t.pivot);
    let mut pivot = t.pivot;
    if let Some(new_anchor) = anchor_grid_widget(ui, current_anchor) {
        let new_p = point_on_quad(t.quad, new_anchor);
        if let Some(s) = app.ui.transform.as_mut() {
            s.pivot = new_p;
        }
        pivot = new_p;
    }
    ui.add_space(4.0);
    let mut px = pivot[0] as f32;
    let mut py = pivot[1] as f32;
    lbl(ui, "X:");
    let rx = crate::widgets::value_field(ui, &mut px, -POSITION_LIMIT_PX..=POSITION_LIMIT_PX, "px", 72.0).accessible_name("X");
    lbl(ui, "Y:");
    let ry = crate::widgets::value_field(ui, &mut py, -POSITION_LIMIT_PX..=POSITION_LIMIT_PX, "px", 72.0).accessible_name("Y");
    if (rx.changed() || ry.changed())
        && let Some(s) = app.ui.transform.as_mut()
    {
        let (dx, dy) = (px as f64 - s.pivot[0], py as f64 - s.pivot[1]);
        s.quad = s.quad.map(|q| [q[0] + dx, q[1] + dy]);
        s.pivot = [px as f64, py as f64];
    }
    crate::widgets::vline(ui, 22.0);
    // W and H in the unit picked from their right-click menu: the scale in percent, or the box's
    // length (Photoshop's units, at the document's resolution).
    let u = size_unit(app);
    let (side_w, side_h) = (t.rect[2] - t.rect[0], t.rect[3] - t.rect[1]);
    let (wlim, hlim) = (scale_limit_pct(side_w), scale_limit_pct(side_h));
    let (wlim_u, hlim_u) = (size_shown(&u, f64::from(wlim), side_w), size_shown(&u, f64::from(hlim), side_h));
    let (mut w, mut h) = (size_shown(&u, sx, side_w), size_shown(&u, sy, side_h));
    let (suffix, places) = (u.unit.suffix(), u.unit.decimals() as u32);
    // Lengths need more room than a percentage ("2116.67 mm").
    let size_w = if u.unit == photocraft_engine::prefs::Unit::Percent { 66.0 } else { 84.0 };
    // Whatever their unit, W and H take any other typed in (`500 px` or `2 in` in percent, `50%`
    // in a length), converted at the document's resolution against the box's original side (#6284).
    let units = crate::widgets::FieldUnits { unit: u.unit, dpi: u.dpi, ppi: u.ppi };
    lbl(ui, "W:");
    let rw = crate::widgets::value_field_extent(ui, &mut w, -wlim_u..=wlim_u, suffix, size_w, places, units, side_w).accessible_name(tl!("Width"));
    let link = !app.ui.transform_unlinked;
    if crate::icons::button(ui, if link { "link" } else { "unlink" }, 22.0, link, tl!("Maintain aspect ratio")).clicked() {
        app.ui.transform_unlinked = link;
    }
    lbl(ui, "H:");
    let rh = crate::widgets::value_field_extent(ui, &mut h, -hlim_u..=hlim_u, suffix, size_w, places, units, side_h).accessible_name(tl!("Height"));
    for r in [&rw, &rh] {
        r.context_menu(|ui| {
            if let Some(name) = crate::rulers::unit_menu(ui, u.unit) {
                set_size_unit(app, name);
            }
        });
    }
    let (w, h) = (size_pct(&u, w, side_w) as f32, size_pct(&u, h, side_h) as f32);
    if rw.changed() || rh.changed() {
        let (kx, ky) = if link {
            let k = if rw.changed() { w as f64 / nonzero(sx) } else { h as f64 / nonzero(sy) };
            // Linked, the other side follows: keep it within its own limit too.
            let k = k.min(f64::from(wlim) / sx.abs().max(1e-9)).min(f64::from(hlim) / sy.abs().max(1e-9));
            (k, k)
        } else {
            (w as f64 / nonzero(sx), h as f64 / nonzero(sy))
        };
        scale_about_pivot(app, kx, ky);
    }
}

/// Options bar fields in Warp mode: style, bend, the split icons and the grid menu.
fn warp_fields(app: &mut PhotocraftApp, ui: &mut egui::Ui, w: &Warp) {
    let tk = crate::theme::Tokens::get(ui.ctx());
    let lbl = |ui: &mut egui::Ui, s: &str| {
        ui.label(egui::RichText::new(s).color(tk.text_dim).size(12.0));
    };
    // Type takes only the styles (#3004): Split and Grid are greyed, and there is no Custom.
    let on_type = warps_type(app);
    // Split, Grid, then the warp style with its bend fields.
    ui.add_enabled_ui(!on_type, |ui| warp_mesh_fields(app, ui, w));
    crate::widgets::vline(ui, 22.0);
    lbl(ui, tl!("Warp:"));
    let mut style = w.style;
    let opts: Vec<(WarpStyle, &str)> = WarpStyle::all().filter(|s| !(on_type && *s == WarpStyle::Custom)).map(|s| (s, s.label())).collect();
    if crate::widgets::dropdown(ui, "warp-style", tl!("Warp"), &mut style, &opts, 120.0)
        && let Some(t) = app.ui.transform.as_mut()
    {
        let b = t.rect;
        t.warp = Some(match style {
            WarpStyle::Custom => Warp::custom(w.to_mesh(1, 1), b),
            WarpStyle::None if on_type => Warp::none(b),
            WarpStyle::None => Warp::custom(BezierMesh::identity(b, 1, 1), b),
            s => Warp { style: s, bend: if w.style.is_preset() { w.bend } else { 50.0 }, ..Warp::none(b) },
        });
    }
    // Orientation and the bend fields are always there, and only act on a preset style.
    let preset = w.style.is_preset();
    ui.add_enabled_ui(preset, |ui| {
        let orient = crate::icons::button(ui, "arrow-left-right", 26.0, w.vertical, tl!("Vertical"));
        orient.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, preset, "Change warp orientation"));
        if orient.clicked()
            && let Some(Some(cur)) = app.ui.transform.as_mut().map(|t| t.warp.as_mut())
        {
            cur.vertical = !cur.vertical;
        }
        let fields: [(&str, f64); 3] = [(tl!("Bend:"), w.bend), ("H:", w.h_distort), ("V:", w.v_distort)];
        for (k, (name, v)) in fields.iter().enumerate() {
            lbl(ui, name);
            let mut f = *v as f32;
            if crate::widgets::value_field(ui, &mut f, -100.0..=100.0, "%", 58.0).accessible_name(*name).changed()
                && let Some(Some(cur)) = app.ui.transform.as_mut().map(|t| t.warp.as_mut())
            {
                match k {
                    0 => cur.bend = f64::from(f),
                    1 => cur.h_distort = f64::from(f),
                    _ => cur.v_distort = f64::from(f),
                }
            }
        }
    });
}

/// The Split icons and the Grid menu of the Warp options bar.
fn warp_mesh_fields(app: &mut PhotocraftApp, ui: &mut egui::Ui, w: &Warp) {
    let tk = crate::theme::Tokens::get(ui.ctx());
    let lbl = |ui: &mut egui::Ui, s: &str| {
        ui.label(egui::RichText::new(s).color(tk.text_dim).size(12.0));
    };
    lbl(ui, tl!("Split:"));
    let armed = app.transform_preview.as_ref().and_then(|p| p.split_tool);
    for (tool, tip) in [
        (SplitTool::Cross, tl!("Split Warp Crosswise")),
        (SplitTool::Vertical, tl!("Split Warp Vertically")),
        (SplitTool::Horizontal, tl!("Split Warp Horizontally")),
    ] {
        let on = armed == Some(tool);
        if split_icon(ui, tool.icon(), tip, on).clicked()
            && let Some(pv) = app.transform_preview.as_mut()
        {
            pv.split_tool = if on { None } else { Some(tool) };
            pv.split_placing = false;
            pv.split_pointer = None;
            pv.split_quick = false;
        }
    }
    crate::widgets::vline(ui, 22.0);
    lbl(ui, tl!("Grid:"));
    let mesh = w.mesh.clone().unwrap_or_else(|| BezierMesh::identity(w.bounds, 1, 1));
    let mut grid = warp_grid_id(&mesh).to_string();
    let opts = [
        ("custom".to_string(), tl!("Custom")),
        ("default".to_string(), tl!("Default")),
        ("3".to_string(), tl!("3 x 3")),
        ("4".to_string(), tl!("4 x 4")),
        ("5".to_string(), tl!("5 x 5")),
    ];
    if crate::widgets::dropdown(ui, "warp-grid", tl!("Grid"), &mut grid, &opts, 92.0)
        && let Some(n) = match grid.as_str() {
            "default" => Some(1),
            "3" => Some(3),
            "4" => Some(4),
            "5" => Some(5),
            _ => None,
        }
    {
        let _ = edit_session_warp(app, "edit.transform.warpGrid", &json!({ "size": n }));
    }
}

/// Back to the untouched box: the default one-patch grid over the box, no bend.
pub fn reset_warp(app: &mut PhotocraftApp) {
    let Some(t) = app.ui.transform.as_mut() else { return };
    if t.warp.is_none() {
        return;
    }
    let b = t.rect;
    t.warp = Some(Warp::custom(BezierMesh::identity(b, 1, 1), b));
    if warps_type(app)
        && let Some(t) = app.ui.transform.as_mut()
    {
        t.warp = Some(Warp::none(b));
    }
    if let Some(pv) = app.transform_preview.as_mut() {
        pv.warp_drag = None;
        pv.split_tool = None;
        pv.split_pointer = None;
        pv.split_placing = false;
        pv.split_quick = false;
    }
}

/// Grid menu readout: Default is 1×1, the numbered entries are uniform n×n patches, and anything
/// split by hand reads as Custom.
pub(super) fn warp_grid_id(mesh: &BezierMesh) -> &'static str {
    let cols = mesh.us.len().saturating_sub(1);
    let rows = mesh.vs.len().saturating_sub(1);
    let even = |k: &[f64]| {
        let n = k.len().saturating_sub(1);
        n > 0 && k.iter().enumerate().all(|(i, v)| (v - i as f64 / n as f64).abs() < 1e-4)
    };
    if cols == rows && even(&mesh.us) && even(&mesh.vs) {
        match cols {
            1 => "default",
            3 => "3",
            4 => "4",
            5 => "5",
            _ => "custom",
        }
    } else {
        "custom"
    }
}

/// One of the three Split icons. `on` is the armed tool: the line then follows the pointer.
fn split_icon(ui: &mut egui::Ui, kind: &str, tip: &str, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
    let ink = crate::icons::button_chrome(ui, rect, on, resp.hovered());
    let c = rect.center();
    let stroke = Stroke::new(1.15, ink);
    let box_r = egui::Rect::from_center_size(c, vec2(13.0, 13.0));
    ui.painter().rect_stroke(box_r, 2.0, stroke, egui::StrokeKind::Inside);
    let h = 5.2;
    match kind {
        "vertical" => {
            ui.painter().line_segment([c - vec2(0.0, h), c + vec2(0.0, h)], stroke);
        }
        "horizontal" => {
            ui.painter().line_segment([c - vec2(h, 0.0), c + vec2(h, 0.0)], stroke);
        }
        _ => {
            ui.painter().line_segment([c - vec2(h, 0.0), c + vec2(h, 0.0)], stroke);
            ui.painter().line_segment([c - vec2(0.0, h), c + vec2(0.0, h)], stroke);
        }
    }
    if kind == "cross" {
        ui.painter().circle_filled(c, 1.5, ink);
    } else {
        ui.painter().circle_stroke(c, 2.1, stroke);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    resp.on_hover_text(tip)
}

/// Warp switch, cancel and commit, kept on the right of the options bar.
/// `mode_switch`: the Free Transform / Warp switch (not for Content-Aware Scale, which only scales).
fn transform_actions(app: &mut PhotocraftApp, ui: &mut egui::Ui, warping: bool, mode_switch: bool) {
    ui.add_space(8.0);
    let commit_tip = if warping {
        crate::i18n::fmt(tl!("Commit warp ({key})"), &[("key", &crate::shortcuts::pretty("Enter"))])
    } else {
        crate::i18n::fmt(tl!("Commit transform ({key})"), &[("key", &crate::shortcuts::pretty("Enter"))])
    };
    let commit_btn = crate::icons::button(ui, "check", 26.0, false, &commit_tip);
    let label = "Commit transform";
    commit_btn.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!(label)));
    if commit_btn.clicked() {
        commit(app);
    }
    let cancel_tip = if warping { tl!("Cancel warp (Esc)") } else { tl!("Cancel transform (Esc)") };
    let cancel_btn = crate::icons::button(ui, "ban", 26.0, false, cancel_tip);
    let label = "Cancel transform";
    cancel_btn.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!(label)));
    if cancel_btn.clicked() {
        cancel(app);
    }
    if warping {
        let reset = crate::icons::button(ui, "undo-2", 26.0, false, tl!("Reset"));
        reset.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Reset warp"));
        if reset.clicked() {
            reset_warp(app);
        }
    }
    if !mode_switch {
        return;
    }
    crate::widgets::vline(ui, 22.0);
    let mode = crate::icons::button(ui, "grid-3x3", 26.0, warping, tl!("Switch between free transform and warp modes"));
    mode.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Switch between free transform and warp modes"));
    if mode.clicked() {
        if warping {
            leave_warp(app);
        } else {
            enter_warp(app, &ui.ctx().clone());
        }
    }
}

/// Scale the box along its own axes about the reference point.
pub(super) fn scale_about_pivot(app: &mut PhotocraftApp, kx: f64, ky: f64) {
    let Some(s) = app.ui.transform.as_mut() else { return };
    let Some(h) = Homography::rect_to_quad([0.0, 0.0, 1.0, 1.0], s.quad) else { return };
    let Some(inv) = h.inverse() else { return };
    let (pu, pv) = inv.apply(s.pivot[0], s.pivot[1]);
    let m = |u: f64, v: f64| {
        let (x, y) = h.apply(pu + (u - pu) * kx, pv + (v - pv) * ky);
        [x, y]
    };
    s.quad = [m(0.0, 0.0), m(1.0, 0.0), m(1.0, 1.0), m(0.0, 1.0)];
}

pub(super) fn rotate_about_pivot(app: &mut PhotocraftApp, da: f64) {
    let Some(s) = app.ui.transform.as_mut() else { return };
    let (sn, cs) = da.sin_cos();
    let c = s.pivot;
    s.quad = s.quad.map(|q| {
        let (x, y) = (q[0] - c[0], q[1] - c[1]);
        [c[0] + x * cs - y * sn, c[1] + x * sn + y * cs]
    });
}
