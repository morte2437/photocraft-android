//! Photoshop's Color panel modes, picked from its panel menu and the same in every theme (#6000,
//! #2569): Hue Cube and Brightness Cube (drawn by [`crate::panels::color_field`]), Color Wheel (HSB
//! sliders over a hue ring around a saturation/brightness triangle), and the Grayscale, RGB, HSB,
//! CMYK, Lab and Web Color sliders over a colour ramp. The panel menu also has Copy Color as HTML,
//! Copy Color's Hex Code, the ramp choices (RGB Spectrum, CMYK Spectrum, Grayscale Ramp, Current
//! Colors) and Make Ramp Web Safe. Shift-click on the ramp cycles the ramps, Alt-click picks the
//! colour the panel isn't editing, and a right-click opens the ramp menu.

use std::f32::consts::TAU;
use std::sync::Arc;

use egui::{Color32, Mesh, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

use crate::PhotocraftApp;
use crate::color_picker_ui::{hex, not_web_safe, out_of_gamut};
use crate::panels::{color_field_fill_id, field_chips, field_hsva, srgb_hsva};
use crate::state::{ColorPanelMode, ColorRamp};
use crate::theme::Tokens;
use crate::widgets;
use photocraft_color::convert;
use photocraft_color::convert::{hsb_to_rgb, rgb_to_hsb};

/// Width of the chips column, as the cube field's.
const CHIPS_W: f32 = 38.0;
/// Height of one slider row.
const ROW_H: f32 = 22.0;
/// Width of a slider row's value field.
const FIELD_W: f32 = 44.0;
/// Width of the unit after a field ("°", "%").
const UNIT_W: f32 = 14.0;
/// Width of the white and black boxes at the right end of the ramp.
const BOXES_W: f32 = 16.0;

/// The Color tab: whatever the panel menu picked.
pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    match app.ui.color_panel.mode {
        ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube => crate::panels::color_field(app, ui),
        ColorPanelMode::ColorWheel => wheel(app, ui),
        mode => sliders(app, ui, mode),
    }
    crate::apply_color::track_focus(ui.ctx());
}

/// The colour model a slider mode edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Model {
    Gray,
    Rgb,
    Hsb,
    Cmyk,
    Lab,
    Web,
}

impl Model {
    pub(crate) fn of(mode: ColorPanelMode) -> Model {
        match mode {
            ColorPanelMode::GrayscaleSlider => Model::Gray,
            ColorPanelMode::HsbSliders | ColorPanelMode::ColorWheel => Model::Hsb,
            ColorPanelMode::CmykSliders => Model::Cmyk,
            ColorPanelMode::LabSliders => Model::Lab,
            ColorPanelMode::WebColorSliders => Model::Web,
            ColorPanelMode::RgbSliders | ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube => Model::Rgb,
        }
    }

    /// Each slider: label, range and unit, in Photoshop's order.
    pub(crate) fn channels(self) -> &'static [(&'static str, f32, f32, &'static str)] {
        match self {
            Model::Gray => &[("K", 0.0, 100.0, "%")],
            Model::Rgb | Model::Web => &[("R", 0.0, 255.0, ""), ("G", 0.0, 255.0, ""), ("B", 0.0, 255.0, "")],
            Model::Hsb => &[("H", 0.0, 360.0, "°"), ("S", 0.0, 100.0, "%"), ("B", 0.0, 100.0, "%")],
            Model::Cmyk => &[("C", 0.0, 100.0, "%"), ("M", 0.0, 100.0, "%"), ("Y", 0.0, 100.0, "%"), ("K", 0.0, 100.0, "%")],
            Model::Lab => &[("L", 0.0, 100.0, ""), ("a", -128.0, 127.0, ""), ("b", -128.0, 127.0, "")],
        }
    }

    /// The sRGB colour of slider values `v` (in the units the fields show).
    pub(crate) fn to_rgb(self, v: [f32; 4]) -> [f32; 3] {
        let [a, b, c, d] = v.map(|x| if x.is_finite() { x } else { 0.0 });
        let rgb = match self {
            Model::Gray => [1.0 - a / 100.0; 3],
            Model::Rgb | Model::Web => [a / 255.0, b / 255.0, c / 255.0],
            Model::Hsb => hsb_to_rgb([a, b / 100.0, c / 100.0]),
            Model::Cmyk => convert::cmyk_to_rgb([a / 100.0, b / 100.0, c / 100.0, d / 100.0]),
            Model::Lab => convert::lab_to_srgb([a, b, c]),
        };
        rgb.map(|x| if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 })
    }

    /// Slider values of `rgb`, rounded to whole numbers as Photoshop's fields show them.
    pub(crate) fn values_of(self, rgb: [f32; 3]) -> [f32; 4] {
        let v = match self {
            Model::Gray => [(1.0 - convert::rgb_to_gray(rgb)) * 100.0, 0.0, 0.0, 0.0],
            Model::Rgb | Model::Web => [rgb[0] * 255.0, rgb[1] * 255.0, rgb[2] * 255.0, 0.0],
            Model::Hsb => {
                let [h, s, b] = rgb_to_hsb(rgb);
                [h, s * 100.0, b * 100.0, 0.0]
            }
            Model::Cmyk => convert::rgb_to_cmyk(rgb).map(|x| x * 100.0),
            Model::Lab => {
                let [l, a, b] = convert::srgb_to_lab(rgb);
                [l, a, b, 0.0]
            }
        };
        v.map(|x| if x.is_finite() { x.round() } else { 0.0 })
    }
}

/// The colour the panel edits.
fn active(app: &PhotocraftApp) -> [f32; 3] {
    let c = if app.ui.color_panel.background { app.session.tools.background } else { app.session.tools.foreground };
    [c[0], c[1], c[2]].map(|x| x.clamp(0.0, 1.0))
}

/// Set the background colour, or the foreground and recolour what is selected as the tool in hand
/// says (`apply_color`); the picks of one drag or typed value are one history step.
pub(crate) fn set_color(app: &mut PhotocraftApp, ctx: &egui::Context, background: bool, rgb: [f32; 3]) {
    let c = [rgb[0], rgb[1], rgb[2], 1.0];
    if background {
        app.session.tools.background = c;
    } else {
        app.session.tools.foreground = c;
        crate::apply_color::foreground_changed(app, crate::apply_color::gesture(ctx));
    }
}

/// The key of the slider values last shown for `model`, kept while they still give the colour:
/// a grey has no hue, and CMYK or Lab values don't survive an sRGB round trip (#3228).
fn values_key(model: Model, background: bool) -> egui::Id {
    egui::Id::unique(("color-panel-values", model, background))
}

/// The key of the Hue Cube's remembered hue and saturation (see [`field_hsva`]).
fn field_key(background: bool) -> egui::Id {
    egui::Id::unique(("color-field-hue", background))
}

/// Slider values for the colour the panel edits: the ones last shown while they still give it.
fn current_values(ui: &egui::Ui, model: Model, background: bool, rgb: [f32; 3]) -> [f32; 4] {
    let stored: Option<[f32; 4]> = ui.data(|d| d.get_temp(values_key(model, background)));
    if let Some(v) = stored.filter(|v| hex(model.to_rgb(*v)) == hex(rgb)) {
        return v;
    }
    if model == Model::Hsb {
        // Share the cube's hue and saturation, so a grey keeps its hue across modes.
        let c = [rgb[0], rgb[1], rgb[2], 1.0];
        let hsva = srgb_hsva(c);
        let remembered: (f32, f32) = ui.data(|d| d.get_temp(field_key(background))).unwrap_or((hsva.h, hsva.s));
        let h = field_hsva(hsva, remembered);
        return [h.h * 360.0, h.s * 100.0, h.v * 100.0, 0.0].map(f32::round);
    }
    model.values_of(rgb)
}

/// Apply edited slider values: the colour, and the values (and the cube's hue) remembered.
fn apply_values(app: &mut PhotocraftApp, ui: &egui::Ui, model: Model, v: [f32; 4]) {
    let background = app.ui.color_panel.background;
    let rgb = model.to_rgb(v);
    set_color(app, ui.ctx(), background, rgb);
    ui.data_mut(|d| d.insert_temp(values_key(model, background), v));
    if model == Model::Hsb && v[2] > 0.5 {
        ui.data_mut(|d| d.insert_temp(field_key(background), (v[0] / 360.0, v[1] / 100.0)));
    }
}

/// Track colours of slider `i` at `steps` + 1 points, the other sliders at `v`.
fn track_stops(model: Model, v: [f32; 4], i: usize, steps: usize) -> Vec<Color32> {
    let Some(&(_, lo, hi, _)) = model.channels().get(i) else { return Vec::new() };
    (0..=steps)
        .map(|k| {
            let mut w = v;
            if let Some(slot) = w.get_mut(i) {
                *slot = lo + (hi - lo) * k as f32 / steps as f32;
            }
            // HSB's hue track is the full spectrum, whatever the saturation and brightness.
            if model == Model::Hsb && i == 0 {
                w = [w[0], 100.0, 100.0, 0.0];
            }
            rgb32(model.to_rgb(w))
        })
        .collect()
}

pub(crate) fn rgb32(c: [f32; 3]) -> Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(q(c[0]), q(c[1]), q(c[2]))
}

/// The slider rows of `model`. Returns the edited values, if any.
fn slider_rows(ui: &mut egui::Ui, model: Model, v: [f32; 4], background: bool, dynamic: bool) -> Option<[f32; 4]> {
    let t = Tokens::get(ui.ctx());
    // Dynamic Color Sliders off: the tracks keep the colours they had when a drag began.
    let stops_key = egui::Id::unique(("color-panel-stops", model, background));
    let dragging = ui.input(|i| i.pointer.any_down());
    let stops_from = if dynamic || !dragging { v } else { ui.data(|d| d.get_temp::<[f32; 4]>(stops_key)).unwrap_or(v) };
    if !dragging {
        ui.data_mut(|d| d.insert_temp(stops_key, v));
    }
    let mut out = None;
    let mut values = v;
    for (i, &(label, lo, hi, unit)) in model.channels().iter().enumerate() {
        let Some(&current) = values.get(i) else { continue };
        let mut x = current;
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.set_height(ROW_H);
            let (lr, _) = ui.allocate_exact_size(vec2(12.0, ROW_H), Sense::hover());
            ui.painter().text(lr.left_center(), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(12.0), t.text_dim);
            let track_w = (ui.available_width() - FIELD_W - UNIT_W - 8.0).max(24.0);
            let (tr, resp) = ui.allocate_exact_size(vec2(track_w, ROW_H), Sense::click_and_drag());
            let stops = track_stops(model, stops_from, i, 24);
            changed |= track(ui, tr, &resp, &mut x, lo, hi, &stops, model == Model::Web);
            resp.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(x), label));
            if model == Model::Web {
                changed |= hex_pair(ui, ui.scope_id().with(("web-field", i, background)), &mut x);
            } else if widgets::value_field(ui, &mut x, lo..=hi, "", FIELD_W).changed() {
                changed = true;
            }
            let (ur, _) = ui.allocate_exact_size(vec2(UNIT_W, ROW_H), Sense::hover());
            ui.painter().text(ur.left_center(), egui::Align2::LEFT_CENTER, unit, egui::FontId::proportional(12.0), t.text_faint);
        });
        if changed && let Some(slot) = values.get_mut(i) {
            *slot = x.clamp(lo, hi).round();
            out = Some(values);
        }
    }
    out
}

/// A slider track with Photoshop's triangle thumb under it. Returns true when dragged or clicked
/// to a new value; Web Color tracks snap to the web steps (00, 33, 66, 99, CC, FF).
#[allow(clippy::too_many_arguments)]
fn track(ui: &egui::Ui, r: Rect, resp: &egui::Response, value: &mut f32, lo: f32, hi: f32, stops: &[Color32], web: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let bar = Rect::from_min_max(pos2(r.left() + 4.0, r.center().y - 6.0), pos2(r.right() - 4.0, r.center().y + 1.0));
    let mut mesh = Mesh::default();
    let n = stops.len().saturating_sub(1).max(1);
    for (k, c) in stops.iter().enumerate() {
        let x = bar.left() + bar.width() * k as f32 / n as f32;
        mesh.colored_vertex(pos2(x, bar.top()), *c);
        mesh.colored_vertex(pos2(x, bar.bottom()), *c);
    }
    for k in 0..stops.len().saturating_sub(1) as u32 {
        let a = k * 2;
        mesh.add_triangle(a, a + 1, a + 3);
        mesh.add_triangle(a, a + 3, a + 2);
    }
    ui.painter().add(mesh);
    ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    if web {
        // Ticks at the web steps.
        for k in 0..=5 {
            let x = bar.left() + bar.width() * k as f32 / 5.0;
            ui.painter().line_segment([pos2(x, bar.bottom() + 1.0), pos2(x, bar.bottom() + 3.0)], Stroke::new(1.0, t.text_faint));
        }
    }
    let mut changed = false;
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
        && bar.width() > 0.0
        && hi > lo
    {
        let f = ((p.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
        let mut v = lo + (hi - lo) * f;
        if web {
            v = (v / 51.0).round() * 51.0;
        }
        let v = v.round().clamp(lo, hi);
        if v != *value {
            *value = v;
            changed = true;
        }
    }
    let f = if hi > lo { ((*value - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
    let x = bar.left() + bar.width() * f;
    let y = bar.bottom() + 1.0;
    let thumb = vec![pos2(x, y), pos2(x + 5.0, y + 7.0), pos2(x - 5.0, y + 7.0)];
    ui.painter().add(egui::Shape::convex_polygon(thumb, t.text, Stroke::new(1.0, t.field_border)));
    changed
}

/// A Web Color slider's field: the channel as two hex digits. Valid input sets `value` (0..255)
/// and returns true; while focused it shows what is typed, and an invalid entry snaps back.
fn hex_pair(ui: &mut egui::Ui, id: egui::Id, value: &mut f32) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(FIELD_W, ROW_H), Sense::hover());
    widgets::surface(ui, rect, t.field, false);
    if !t.bevel {
        ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    }
    let byte = value.clamp(0.0, 255.0).round() as u8;
    let typing = if ui.memory(|m| m.has_focus(id)) { ui.data(|d| d.get_temp::<String>(id)) } else { None };
    let mut text = typing.unwrap_or_else(|| format!("{byte:02X}"));
    let inner = rect.shrink2(vec2(5.0, 2.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let resp = child
        .add(egui::TextEdit::singleline(&mut text).id(id).char_limit(2).desired_width(inner.width()).frame(egui::Frame::NONE).font(crate::theme::mono(12.0)));
    if resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    // Two hex digits only: `from_str_radix` would also take a sign ("+F").
    if resp.changed()
        && text.len() == 2
        && text.bytes().all(|b| b.is_ascii_hexdigit())
        && let Ok(b) = u8::from_str_radix(&text, 16)
    {
        *value = f32::from(b);
        return true;
    }
    false
}

/// The slider modes: chips (with the out-of-gamut and web-colour alerts under them) beside the
/// sliders, over the colour ramp.
fn sliders(app: &mut PhotocraftApp, ui: &mut egui::Ui, mode: ColorPanelMode) {
    let model = Model::of(mode);
    let background = app.ui.color_panel.background;
    let rgb = active(app);
    let v = current_values(ui, model, background, rgb);
    let dynamic = app.session.prefs().interface.dynamic_color_sliders;
    let rows_h = model.channels().len() as f32 * (ROW_H + ui.spacing().item_spacing.y);
    let top_h = rows_h.max(CHIPS_W + 40.0);
    let mut edited = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (col, _) = ui.allocate_exact_size(vec2(CHIPS_W, top_h), Sense::hover());
        field_chips(app, ui, col);
        alerts(app, ui, col, rgb, model == Model::Web);
        ui.vertical(|ui| {
            edited = slider_rows(ui, model, v, background, dynamic);
        });
    });
    if let Some(v) = edited {
        apply_values(app, ui, model, v);
    }
    ui.add_space(6.0);
    let fill = ui.data(|d| d.get_temp::<f32>(color_field_fill_id()));
    let gaps = 2.0 * ui.spacing().item_spacing.y + 6.0;
    let ramp_h = fill.filter(|f| f.is_finite()).map_or(36.0, |f| (f - top_h - gaps).clamp(28.0, 2000.0));
    ramp(app, ui, ramp_h);
}

/// Photoshop's alerts under the chips: ⚠ when the colour can't print (click: the nearest CMYK
/// colour) and, for Web Color sliders, the cube when it isn't a web colour (click: the nearest).
fn alerts(app: &mut PhotocraftApp, ui: &mut egui::Ui, col: Rect, rgb: [f32; 3], web: bool) {
    let t = Tokens::get(ui.ctx());
    let background = app.ui.color_panel.background;
    let mut y = col.top() + CHIPS_W + 2.0;
    let warnings = [
        (out_of_gamut(rgb), "Out of gamut for printing: click for the nearest printable colour", false),
        (if web { not_web_safe(rgb) } else { None }, "Not a web colour: click for the nearest web colour", true),
    ];
    for (slot, (near, tip, cube)) in warnings.into_iter().enumerate() {
        let Some(near) = near else { continue };
        let icon = Rect::from_min_size(pos2(col.left() + 1.0, y), vec2(14.0, 13.0));
        let chip = Rect::from_min_size(pos2(col.left() + 18.0, y), vec2(13.0, 13.0));
        if cube {
            crate::color_picker_ui::web_cube(ui.painter(), icon.shrink(0.5), t.text_dim);
        } else {
            crate::color_picker_ui::warning_triangle(ui.painter(), icon, t.text_dim, t.card);
        }
        ui.painter().rect_filled(chip, 0.0, rgb32(near));
        ui.painter().rect_stroke(chip, 0.0, Stroke::new(1.0, t.text_dim), StrokeKind::Outside);
        let resp = ui.interact(icon.union(chip), ui.scope_id().with(("color-panel-alert", slot)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tip));
        if resp.on_hover_text(tl!(tip)).clicked() {
            set_color(app, ui.ctx(), background, near);
        }
        y += 18.0;
    }
}

/// The ramp's colour at (x, y) in 0..1 (y down). RGB Spectrum: hue across, white at the top
/// through the full colour to black at the bottom; CMYK Spectrum: the same in the CMYK gamut;
/// Grayscale Ramp: white to black; Current Colors: the foreground to the background.
pub(crate) fn ramp_color(ramp: ColorRamp, web_safe: bool, fg: [f32; 3], bg: [f32; 3], x: f32, y: f32) -> [f32; 3] {
    let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
    let spectrum = || {
        let (s, v) = if y < 0.5 { (y * 2.0, 1.0) } else { (1.0, 1.0 - (y - 0.5) * 2.0) };
        hsb_to_rgb([x.min(0.9999) * 360.0, s, v])
    };
    let c = match ramp {
        ColorRamp::RgbSpectrum => spectrum(),
        ColorRamp::CmykSpectrum => convert::cmyk_to_rgb(convert::rgb_to_cmyk(spectrum())),
        ColorRamp::GrayscaleRamp => [1.0 - x; 3],
        ColorRamp::CurrentColors => [0, 1, 2].map(|i| fg[i] + (bg[i] - fg[i]) * x),
    };
    let c = c.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    if web_safe { c.map(|v| (v * 5.0).round() / 5.0) } else { c }
}

/// Cells across and down the ramp's mesh.
const RAMP_NX: usize = 48;
const RAMP_NY: usize = 8;

/// The ramp's colours, cached until the ramp, its inputs or the CMYK space change: the CMYK
/// Spectrum takes a CMYK round trip per vertex, too slow to redo every frame.
fn ramp_colors(ui: &egui::Ui, ramp: ColorRamp, web_safe: bool, fg: [f32; 3], bg: [f32; 3]) -> Arc<Vec<Color32>> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (ramp as u8, web_safe).hash(&mut h);
    if ramp == ColorRamp::CurrentColors {
        (hex(fg), hex(bg)).hash(&mut h);
    }
    if ramp == ColorRamp::CmykSpectrum {
        convert::active_cmyk_space().map(|s| Arc::as_ptr(&s) as usize).hash(&mut h);
    }
    let sig = h.finish();
    let key = egui::Id::unique("color-panel-ramp-cache");
    if let Some((s, colors)) = ui.data(|d| d.get_temp::<(u64, Arc<Vec<Color32>>)>(key))
        && s == sig
    {
        return colors;
    }
    // Web-safe ramps are flat cells (sampled at their centres), the others shaded vertices.
    let (nx, ny, half) = if web_safe { (RAMP_NX, RAMP_NY, 0.5) } else { (RAMP_NX + 1, RAMP_NY + 1, 0.0) };
    let mut colors = Vec::with_capacity(nx * ny);
    for j in 0..ny {
        for i in 0..nx {
            let (x, y) = ((i as f32 + half) / RAMP_NX as f32, (j as f32 + half) / RAMP_NY as f32);
            colors.push(rgb32(ramp_color(ramp, web_safe, fg, bg, x, y)));
        }
    }
    let colors = Arc::new(colors);
    ui.data_mut(|d| d.insert_temp(key, (sig, colors.clone())));
    colors
}

/// The ramp mesh over `r` from [`ramp_colors`].
fn ramp_mesh(r: Rect, colors: &[Color32], web_safe: bool) -> Mesh {
    let mut m = Mesh::default();
    let at = |i: usize, j: usize| pos2(r.left() + r.width() * i as f32 / RAMP_NX as f32, r.top() + r.height() * j as f32 / RAMP_NY as f32);
    if web_safe {
        for j in 0..RAMP_NY {
            for i in 0..RAMP_NX {
                let Some(&c) = colors.get(j * RAMP_NX + i) else { continue };
                let a = m.vertices.len() as u32;
                for p in [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)] {
                    m.colored_vertex(p, c);
                }
                m.add_triangle(a, a + 1, a + 2);
                m.add_triangle(a, a + 2, a + 3);
            }
        }
        return m;
    }
    for j in 0..=RAMP_NY {
        for i in 0..=RAMP_NX {
            m.colored_vertex(at(i, j), colors.get(j * (RAMP_NX + 1) + i).copied().unwrap_or(Color32::BLACK));
        }
    }
    let row = (RAMP_NX + 1) as u32;
    for j in 0..RAMP_NY as u32 {
        for i in 0..RAMP_NX as u32 {
            let a = j * row + i;
            m.add_triangle(a, a + 1, a + row + 1);
            m.add_triangle(a, a + row + 1, a + row);
        }
    }
    m
}

/// The colour ramp with the white and black boxes at its right end. A click or drag picks; with
/// Alt it picks the colour the panel isn't editing; Shift-click cycles the ramps; a right-click
/// opens the ramp menu.
fn ramp(app: &mut PhotocraftApp, ui: &mut egui::Ui, height: f32) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (all, _) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
    let bar = Rect::from_min_max(all.min, pos2(all.right() - BOXES_W - 2.0, all.bottom()));
    let boxes = Rect::from_min_max(pos2(all.right() - BOXES_W, all.top()), all.max);
    let cp = app.ui.color_panel;
    let ramp_id = ui.scope_id().with("color-panel-ramp");
    // A pick changes the foreground or background, so a drag keeps the Current Colors ends it began
    // with; otherwise the ramp would slide under the pointer and a held pick would drift.
    let ends_id = ramp_id.with("ends");
    let live = (rgb3(app.session.tools.foreground), rgb3(app.session.tools.background));
    let (fg, bg) = if ui.ctx().is_being_dragged(ramp_id) {
        ui.data(|d| d.get_temp::<([f32; 3], [f32; 3])>(ends_id)).unwrap_or(live)
    } else {
        ui.data_mut(|d| d.insert_temp(ends_id, live));
        live
    };
    let colors = ramp_colors(ui, cp.ramp, cp.web_safe_ramp, fg, bg);
    ui.painter().add(ramp_mesh(bar, &colors, cp.web_safe_ramp));
    ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    let (white, black) =
        (Rect::from_min_max(boxes.min, pos2(boxes.right(), boxes.center().y)), Rect::from_min_max(pos2(boxes.left(), boxes.center().y), boxes.max));
    ui.painter().rect_filled(white, 0.0, Color32::WHITE);
    ui.painter().rect_filled(black, 0.0, Color32::BLACK);
    ui.painter().rect_stroke(boxes, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    let resp = ui.interact(bar, ramp_id, Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Unknown, ui.is_enabled(), "Color Ramp"));
    let mods = ui.input(|i| i.modifiers);
    let target = cp.background != mods.alt;
    if resp.clicked() && mods.shift {
        app.ui.color_panel.ramp = cp.ramp.next();
    } else if (resp.dragged() || resp.clicked())
        && !mods.shift
        && let Some(p) = resp.interact_pointer_pos()
        && bar.width() > 0.0
        && bar.height() > 0.0
    {
        let (mut x, mut y) = ((p.x - bar.left()) / bar.width(), (p.y - bar.top()) / bar.height());
        if cp.web_safe_ramp {
            // Pick the cell drawn under the pointer: web-safe cells show their centre's colour.
            let centre = |f: f32, n: usize| ((f * n as f32).floor().clamp(0.0, (n - 1) as f32) + 0.5) / n as f32;
            (x, y) = (centre(x, RAMP_NX), centre(y, RAMP_NY));
        }
        let c = ramp_color(cp.ramp, cp.web_safe_ramp, fg, bg, x, y);
        set_color(app, ui.ctx(), target, c);
    }
    resp.context_menu(|ui| {
        widgets::style_spectrum_popup_menu(ui);
        ramp_menu(app, ui);
    });
    for (r, c, name) in [(white, [1.0; 3], "White"), (black, [0.0; 3], "Black")] {
        let b = ui.interact(r, ui.scope_id().with(("color-panel-box", name)), Sense::click());
        b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), name));
        if b.clicked() {
            set_color(app, ui.ctx(), target, c);
        }
    }
}

fn rgb3(c: [f32; 4]) -> [f32; 3] {
    [c[0], c[1], c[2]].map(|v| v.clamp(0.0, 1.0))
}

/// The Color Wheel: HSB sliders over a hue ring (red at the right, running counter-clockwise
/// through yellow and green) around a triangle whose tip is the hue, with white and black at its
/// other corners; the add-to-swatches button sits under it.
fn wheel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let background = app.ui.color_panel.background;
    let rgb = active(app);
    let v = current_values(ui, Model::Hsb, background, rgb);
    let dynamic = app.session.prefs().interface.dynamic_color_sliders;
    let rows_h = 3.0 * (ROW_H + ui.spacing().item_spacing.y);
    let top_h = rows_h.max(CHIPS_W);
    let mut edited = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (col, _) = ui.allocate_exact_size(vec2(CHIPS_W, top_h), Sense::hover());
        field_chips(app, ui, col);
        ui.vertical(|ui| {
            edited = slider_rows(ui, Model::Hsb, v, background, dynamic);
        });
    });
    let mut v = edited.unwrap_or(v);
    let mut changed = edited.is_some();
    ui.add_space(6.0);
    let fill = ui.data(|d| d.get_temp::<f32>(color_field_fill_id()));
    let gaps = 2.0 * ui.spacing().item_spacing.y + 6.0;
    let w = ui.available_width();
    let side = fill.filter(|f| f.is_finite()).map_or(w.min(220.0), |f| (f - top_h - gaps).min(w)).max(80.0);
    let (area, resp) = ui.allocate_exact_size(vec2(w, side), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Unknown, ui.is_enabled(), "Color Wheel"));
    let (center, inner, outer) = wheel_geometry(area);
    let ring_w = outer - inner;
    let (h, s, b) = (v[0] / 360.0, v[1] / 100.0, v[2] / 100.0);
    ui.painter().add(ring_mesh(center, inner, outer));
    let tri = triangle(center, inner - 1.0, h);
    ui.painter().add(triangle_mesh(tri, h));
    // Which part a drag began on: the ring sets the hue, the triangle saturation and brightness.
    // The ring or the triangle, from where the press began (kept for frames without a press
    // origin), so a drag that leaves the ring keeps turning the hue.
    let drag_key = ui.scope_id().with("color-wheel-drag");
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        // A click is where it landed; a drag goes by where its press began.
        let origin = if resp.clicked() { Some(p) } else { ui.input(|i| i.pointer.press_origin()) };
        // `None`: the press began beside the wheel (the area spans the panel), which picks nothing.
        let part = match origin {
            Some(origin) => {
                let d = origin.distance(center);
                // The hue knob reaches past the ring's edge on a small wheel.
                let part = (d <= outer + 7.0).then_some(d >= inner - 2.0);
                ui.data_mut(|data| data.insert_temp(drag_key, part));
                part
            }
            None => ui.data(|d| d.get_temp::<Option<bool>>(drag_key)).unwrap_or(Some(false)),
        };
        match part {
            Some(true) => {
                let a = (center.y - p.y).atan2(p.x - center.x).rem_euclid(TAU);
                v[0] = (a / TAU * 360.0).round().rem_euclid(360.0);
                changed = true;
            }
            Some(false) => {
                if let Some((s2, b2)) = triangle_sv(tri, p) {
                    v[1] = (s2 * 100.0).round();
                    v[2] = (b2 * 100.0).round();
                }
                changed = true;
            }
            None => {}
        }
    }
    // Knobs: on the ring at the hue, in the triangle at the saturation and brightness.
    let a = h * TAU;
    let ring_knob = center + vec2(a.cos(), -a.sin()) * (inner + ring_w / 2.0);
    let tri_knob = triangle_point(tri, s, b);
    for k in [ring_knob, tri_knob] {
        ui.painter().circle_stroke(k, 5.0, Stroke::new(1.5, Color32::WHITE));
        ui.painter().circle_stroke(k, 6.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
    }
    // Photoshop's add-to-swatches button under the wheel.
    let add = Rect::from_min_size(pos2(area.right() - 16.0, area.bottom() - 16.0), vec2(14.0, 14.0));
    ui.painter().rect_stroke(add, 1.0, Stroke::new(1.0, t.text_dim), StrokeKind::Inside);
    ui.painter().line_segment([pos2(add.center().x, add.top() + 3.5), pos2(add.center().x, add.bottom() - 3.5)], Stroke::new(1.0, t.text_dim));
    ui.painter().line_segment([pos2(add.left() + 3.5, add.center().y), pos2(add.right() - 3.5, add.center().y)], Stroke::new(1.0, t.text_dim));
    let add_resp = ui.interact(add, ui.scope_id().with("color-wheel-add"), Sense::click());
    add_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Add to Swatches"));
    if add_resp.on_hover_text(tl!("Create new swatch")).clicked()
        && let Err(e) = app.run("swatches.add", json!({"color": hex(rgb)}))
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    if changed {
        apply_values(app, ui, Model::Hsb, v);
    }
}

/// The wheel in `area`: its centre and the hue ring's inner and outer radii.
pub(crate) fn wheel_geometry(area: Rect) -> (Pos2, f32, f32) {
    let outer = (area.width().min(area.height()) / 2.0 - 2.0).max(1.0);
    let ring_w = (outer * 0.09).clamp(6.0, 22.0).min(outer * 0.5);
    (area.center(), outer - ring_w, outer)
}

/// The wheel's hue ring between radii `inner` and `outer`.
fn ring_mesh(center: Pos2, inner: f32, outer: f32) -> Mesh {
    let mut m = Mesh::default();
    let steps = 96u32;
    for k in 0..=steps {
        let f = k as f32 / steps as f32;
        let a = f * TAU;
        let c = rgb32(hsb_to_rgb([f * 360.0, 1.0, 1.0]));
        let dir = vec2(a.cos(), -a.sin());
        m.colored_vertex(center + dir * inner, c);
        m.colored_vertex(center + dir * outer, c);
    }
    for k in 0..steps {
        let a = k * 2;
        m.add_triangle(a, a + 1, a + 3);
        m.add_triangle(a, a + 3, a + 2);
    }
    m
}

/// The triangle's corners for hue `h` (0..1): the pure hue at the hue's angle, white 120° on and
/// black 240° on, inside radius `r`.
fn triangle(center: Pos2, r: f32, h: f32) -> [Pos2; 3] {
    let at = |turns: f32| {
        let a = (h + turns) * TAU;
        center + vec2(a.cos(), -a.sin()) * r
    };
    [at(0.0), at(1.0 / 3.0), at(2.0 / 3.0)]
}

/// The point of saturation `s` and brightness `b` in the triangle: black + b·(white − black) +
/// b·s·(hue − white).
fn triangle_point([hue, white, black]: [Pos2; 3], s: f32, b: f32) -> Pos2 {
    black + (white - black) * b + (hue - white) * (b * s)
}

/// Saturation and brightness at `p`, clamped into the triangle; `None` for a degenerate one or a
/// non-finite `p`.
pub(crate) fn triangle_sv([hue, white, black]: [Pos2; 3], p: Pos2) -> Option<(f32, f32)> {
    let (a, c, d) = (white - black, hue - white, p - black);
    let det = a.x * c.y - a.y * c.x;
    if det.abs() < 1e-6 {
        return None;
    }
    let (b, bs) = ((d.x * c.y - d.y * c.x) / det, (a.x * d.y - a.y * d.x) / det);
    // A NaN `b` would panic as a clamp bound.
    if !b.is_finite() || !bs.is_finite() {
        return None;
    }
    let b = b.clamp(0.0, 1.0);
    let bs = bs.clamp(0.0, b);
    Some((if b > 0.0 { bs / b } else { 0.0 }, b))
}

/// The triangle shaded through saturation and brightness at hue `h`.
fn triangle_mesh(tri: [Pos2; 3], h: f32) -> Mesh {
    let n = 16usize;
    let mut m = Mesh::default();
    for j in 0..=n {
        for i in 0..=n {
            let (s, b) = (i as f32 / n as f32, j as f32 / n as f32);
            m.colored_vertex(triangle_point(tri, s, b), rgb32(hsb_to_rgb([h * 360.0, s, b])));
        }
    }
    let row = (n + 1) as u32;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = j * row + i;
            m.add_triangle(a, a + 1, a + row + 1);
            m.add_triangle(a, a + row + 1, a + row);
        }
    }
    m
}

/// `color="#rrggbb"`, what Copy Color as HTML puts on the clipboard.
pub(crate) fn html_color(rgb: [f32; 3]) -> String {
    format!("color=\"{}\"", hex(rgb))
}

/// `rrggbb`, what Copy Color's Hex Code puts on the clipboard.
pub(crate) fn hex_code(rgb: [f32; 3]) -> String {
    hex(rgb).trim_start_matches('#').to_string()
}

/// A panel menu row with Photoshop's check mark column.
fn check_item(ui: &mut egui::Ui, on: bool, label: &str) -> bool {
    ui.button(format!("{}{}", if on { "✔ " } else { "   " }, tl!(label))).clicked()
}

/// The ramp choices and Make Ramp Web Safe (the panel menu's last section and the ramp's
/// right-click menu).
fn ramp_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    for r in ColorRamp::ALL {
        if check_item(ui, app.ui.color_panel.ramp == r, r.label()) {
            app.ui.color_panel.ramp = r;
            ui.close();
        }
    }
    ui.separator();
    if check_item(ui, app.ui.color_panel.web_safe_ramp, "Make Ramp Web Safe") {
        app.ui.color_panel.web_safe_ramp = !app.ui.color_panel.web_safe_ramp;
        ui.close();
    }
}

/// The Color panel's panel menu, Photoshop's: the modes, the copy commands, the ramps.
pub(crate) fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // One section, as Photoshop's: no separator between Color Wheel and Grayscale Slider.
    for mode in ColorPanelMode::ALL {
        if check_item(ui, app.ui.color_panel.mode == mode, mode.label()) {
            app.ui.color_panel.mode = mode;
            ui.close();
        }
    }
    ui.separator();
    let rgb = active(app);
    if ui.button(tl!("Copy Color as HTML")).clicked() {
        ui.ctx().copy_text(html_color(rgb));
        ui.close();
    }
    if ui.button(tl!("Copy Color's Hex Code")).clicked() {
        ui.ctx().copy_text(hex_code(rgb));
        ui.close();
    }
    ui.separator();
    ramp_menu(app, ui);
}
