//! Photoshop's Fill and Stroke popover for shapes (the options bar's Fill/Stroke swatches and the
//! Properties panel's Appearance, [`crate::shape_paint_ui::swatch`]): the paint type along the top
//! (No Color, Solid Color, Gradient, Pattern) with the Color Picker button at the right, then the
//! Recently Used Colors row and the swatch library in folders for Solid Color, or the Gradient tab
//! ([`crate::shape_gradient_ui`]) for Gradient.
//!
//! The Color Picker button opens the full Color Picker dialog (HSB, RGB, Lab, CMYK, hex and the
//! eyedropper); the popover only reports it ([`PaintChange::OpenPicker`]), since opening a dialog
//! needs the app. Recently Used Colors live in `Preferences::recent_colors`, so they survive a
//! restart.

use std::sync::Arc;

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::PhotocraftApp;
use crate::shape_gradient_ui::{self as shape_gradient, GradientChoices};
use crate::shape_paint_ui::PaintChange;
use crate::theme::Tokens;

/// How many Recently Used Colors are kept (one row of the popover).
pub(crate) const MAX_RECENT: usize = 15;
/// Side of a swatch cell, as Photoshop's small swatches.
const CELL: f32 = 16.0;
/// Room between swatch cells.
const GAP: f32 = 2.0;
/// The popover's content width: fifteen cells and their gaps.
const WIDTH: f32 = MAX_RECENT as f32 * (CELL + GAP) - GAP;
/// Height of the swatch folders' scrolling list.
const LIST_HEIGHT: f32 = 200.0;
/// Height of a type button along the top.
const TYPE_BUTTON: f32 = 26.0;

/// The popover's paint types, in Photoshop's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    None,
    Solid,
    Gradient,
    Pattern,
}

/// A folder of the library: its name and (name, display colour, sRGB) per swatch.
pub(crate) struct SwatchGroup {
    name: String,
    items: Vec<(String, Color32, [f32; 3])>,
}

/// What the Solid Color tab lists, cached per library revision, colour settings and recent
/// colours (see [`library`]).
pub(crate) struct Library {
    recent: Vec<[f32; 3]>,
    swatches: Vec<SwatchGroup>,
}

/// The Recently Used Colors, newest first (at most [`MAX_RECENT`]; unreadable entries skipped).
pub(crate) fn recent_colors(session: &photocraft_engine::Session) -> Vec<[f32; 3]> {
    session.prefs().recent_colors.iter().filter_map(|h| crate::color_picker_ui::parse_hex(h)).take(MAX_RECENT).collect()
}

/// Put `rgb` at the front of the Recently Used Colors (once), as Photoshop does for a colour that
/// was used.
pub(crate) fn remember(app: &mut PhotocraftApp, rgb: [f32; 3]) {
    if rgb.iter().any(|c| !c.is_finite()) {
        return;
    }
    let hex = crate::color_picker_ui::hex(rgb);
    let mut next: Vec<String> = vec![hex.clone()];
    next.extend(recent_colors(&app.session).into_iter().map(crate::color_picker_ui::hex).filter(|h| *h != hex));
    next.truncate(MAX_RECENT);
    if app.session.prefs().recent_colors != next {
        app.session.prefs.edit(|p| p.recent_colors = next);
    }
}

fn c32(c: [f32; 3]) -> Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(q(c[0]), q(c[1]), q(c[2]))
}

/// The popover's lists. Rebuilt only when what they show changes, so drawing the swatches every
/// frame costs a lookup.
pub(crate) fn library(session: &photocraft_engine::Session, ctx: &egui::Context) -> Arc<Library> {
    let c = &session.color.settings;
    let recent = &session.prefs().recent_colors;
    let key = egui::Id::unique(("paint-picker-library", session.presets.rev, &c.working_rgb, &c.working_cmyk, &c.working_gray, recent));
    if let Some(v) = ctx.data(|d| d.get_temp::<Arc<Library>>(key)) {
        return v;
    }
    let conv = photocraft_engine::presets::swatches::SwatchConv::new(session);
    let swatches = session
        .presets
        .swatches
        .iter()
        .map(|g| SwatchGroup {
            name: g.name.clone(),
            items: g
                .items
                .iter()
                .map(|s| {
                    let rgb = conv.rgb(&s.color);
                    (s.name.clone(), c32(rgb), rgb)
                })
                .collect(),
        })
        .collect();
    let lib = Arc::new(Library { recent: recent_colors(session), swatches });
    ctx.data_mut(|d| {
        // Only the newest table is kept.
        let latest = egui::Id::unique("paint-picker-library-key");
        if let Some(old) = d.get_temp::<egui::Id>(latest) {
            d.remove::<Arc<Library>>(old);
        }
        d.insert_temp(latest, key);
        d.insert_temp(key, lib.clone());
    });
    lib
}

/// The paint type the popover opens on: the shape's current paint.
fn kind_of(fill: Option<&photocraft_doc::Fill>) -> Kind {
    match fill {
        None => Kind::None,
        Some(photocraft_doc::Fill::Solid(_)) => Kind::Solid,
        Some(photocraft_doc::Fill::Gradient { .. }) => Kind::Gradient,
        Some(photocraft_doc::Fill::Pattern { .. }) => Kind::Pattern,
    }
}

/// One of the type buttons: its icon drawn by `draw`, an accent frame when `on`.
fn type_button(ui: &mut egui::Ui, on: bool, enabled: bool, label: &str, tip: &str, draw: impl FnOnce(&egui::Ui, Rect, &Tokens)) -> bool {
    let t = Tokens::get(ui.ctx());
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (r, resp) = ui.allocate_exact_size(vec2(TYPE_BUTTON + 6.0, TYPE_BUTTON), sense);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, on, label));
    // Photoshop frames the selected type; the chrome paints the selected and hover states.
    crate::icons::button_chrome(ui, r, on, resp.hovered() && enabled);
    draw(ui, Rect::from_center_size(r.center(), vec2(16.0, 16.0)), &t);
    if !enabled {
        // Greyed out: the popover's background over the icon at half strength.
        ui.painter().rect_filled(r.shrink(1.0), t.radius_sm, ui.visuals().window_fill.gamma_multiply(0.6));
    }
    resp.on_hover_text(tip).clicked()
}

/// A small white square crossed by a red diagonal, Photoshop's "No Color".
fn draw_no_color(ui: &egui::Ui, r: Rect, t: &Tokens) {
    let p = ui.painter();
    p.rect_filled(r, 0.0, Color32::WHITE);
    p.rect_stroke(r, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    p.line_segment([r.left_bottom() + vec2(1.5, -1.5), r.right_top() + vec2(-1.5, 1.5)], Stroke::new(1.5, t.danger));
}

fn draw_gradient(ui: &egui::Ui, r: Rect, t: &Tokens) {
    let p = ui.painter();
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), Color32::from_gray(30));
    mesh.colored_vertex(r.right_top(), Color32::from_gray(235));
    mesh.colored_vertex(r.right_bottom(), Color32::from_gray(235));
    mesh.colored_vertex(r.left_bottom(), Color32::from_gray(30));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(mesh);
    p.rect_stroke(r, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
}

fn draw_pattern(ui: &egui::Ui, r: Rect, t: &Tokens) {
    let p = ui.painter();
    let s = r.width() / 4.0;
    for j in 0..4 {
        for i in 0..4 {
            if (i + j) % 2 == 0 {
                let cell = Rect::from_min_size(r.min + vec2(i as f32 * s, j as f32 * s), vec2(s, s));
                p.rect_filled(cell, 0.0, t.text);
            }
        }
    }
    p.rect_stroke(r, 0.0, Stroke::new(1.0, t.text_dim), StrokeKind::Inside);
}

/// The Color Picker button: a hue ramp fading to white, as Photoshop's rainbow square.
fn picker_button(ui: &mut egui::Ui) -> bool {
    let t = Tokens::get(ui.ctx());
    let label = tl!("Color Picker");
    let (r, resp) = ui.allocate_exact_size(vec2(TYPE_BUTTON, TYPE_BUTTON), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    let inner = r.shrink(4.0);
    let mut mesh = egui::Mesh::default();
    const STEPS: usize = 6;
    for i in 0..=STEPS {
        let x = inner.left() + inner.width() * i as f32 / STEPS as f32;
        let hue = photocraft_color::convert::hsb_to_rgb([360.0 * i as f32 / STEPS as f32, 1.0, 1.0]);
        mesh.colored_vertex(pos2(x, inner.top()), c32(hue));
        mesh.colored_vertex(pos2(x, inner.bottom()), Color32::WHITE);
        if i > 0 {
            let k = (i as u32) * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k, k + 1);
        }
    }
    ui.painter().add(mesh);
    let border = if resp.hovered() || resp.has_focus() { t.accent } else { t.text };
    ui.painter().rect_stroke(inner, 0.0, Stroke::new(1.5, border), StrokeKind::Outside);
    resp.on_hover_text(label).clicked()
}

/// One swatch cell; true when clicked.
fn cell(ui: &mut egui::Ui, color: Color32, label: &str, selected: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(CELL, CELL), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    ui.painter().rect_filled(r, 0.0, color);
    if selected || resp.hovered() || resp.has_focus() {
        ui.painter().rect_stroke(r, 0.0, Stroke::new(1.5, if selected { t.accent } else { t.text }), StrokeKind::Inside);
    }
    resp.on_hover_text(label).clicked()
}

/// A folder row (disclosure triangle, folder icon, name); flips `open` when clicked.
fn folder_row(ui: &mut egui::Ui, name: &str, open: &mut bool) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::DisclosureTriangle, ui.is_enabled(), *open, name));
    if resp.hovered() {
        ui.painter().rect_filled(r, t.radius_sm, t.hover);
    }
    let chevron = Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(16.0, 16.0));
    crate::icons::paint(ui, chevron, if *open { "chevron-down" } else { "chevron-right" }, 12.0, t.text_dim);
    let folder = Rect::from_min_size(r.min + vec2(22.0, 3.0), vec2(18.0, 18.0));
    crate::icons::paint(ui, folder, "folder", 16.0, t.text_dim);
    ui.painter().text(pos2(r.left() + 46.0, r.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(12.5), t.text);
    if resp.clicked() {
        *open = !*open;
    }
}

/// A wrapped grid of `n` cells drawn by `draw`, `WIDTH` wide.
fn grid(ui: &mut egui::Ui, n: usize, mut draw: impl FnMut(&mut egui::Ui, usize)) {
    ui.scope(|ui| {
        ui.set_max_width(WIDTH);
        ui.spacing_mut().item_spacing = vec2(GAP, GAP);
        ui.horizontal_wrapped(|ui| {
            for i in 0..n {
                draw(ui, i);
            }
        });
    });
}

/// The popover's body for a paint (`fill`; `None` is No Color) of swatch `id`. `original` is the
/// solid colour a pick keeps the alpha of; `gradients` feeds the Gradient tab.
pub(crate) fn body(
    ui: &mut egui::Ui,
    lib: &Library,
    fill: Option<&photocraft_doc::Fill>,
    original: [f32; 4],
    gradients: &GradientChoices,
    id: egui::Id,
) -> Option<PaintChange> {
    let t = Tokens::get(ui.ctx());
    let mut out = None;
    let current = kind_of(fill);
    // The tab on show: the paint's own type when the popover opens, then the one clicked.
    let tab_id = id.with("paint-picker-tab");
    let mut tab = ui.data(|d| d.get_temp::<(Kind, Kind)>(tab_id)).filter(|(was, _)| *was == current).map_or(current, |(_, tab)| tab);
    let solid_now = match fill {
        Some(photocraft_doc::Fill::Solid(c)) => Some(c.to_rgb()),
        _ => None,
    };
    ui.set_width(WIDTH);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if type_button(ui, tab == Kind::None, true, tl!("No Color"), tl!("No Color"), draw_no_color) {
            tab = Kind::None;
            if current != Kind::None {
                out = Some(PaintChange::None);
            }
        }
        if type_button(ui, tab == Kind::Solid, true, tl!("Solid Color"), tl!("Solid Color"), |ui, r, t| crate::icons::paint(ui, r, "grid-3x3", 16.0, t.text)) {
            tab = Kind::Solid;
            if current != Kind::Solid {
                out = Some(PaintChange::Color(original));
            }
        }
        if type_button(ui, tab == Kind::Gradient, true, tl!("Gradient"), tl!("Gradient"), draw_gradient) {
            tab = Kind::Gradient;
            if current != Kind::Gradient {
                out = Some(PaintChange::Gradient(gradients.default_fill()));
            }
        }
        // Shapes don't render pattern paint yet: the type shows, as in Photoshop, but is off.
        type_button(ui, tab == Kind::Pattern, false, tl!("Pattern"), tl!("Shapes can't use patterns yet"), draw_pattern);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if picker_button(ui) {
                out = Some(PaintChange::OpenPicker);
            }
        });
    });
    ui.add_space(6.0);
    match (tab, fill) {
        (Kind::Gradient, Some(g @ photocraft_doc::Fill::Gradient { .. })) => {
            if let Some(edited) = shape_gradient::gradient_tab(ui, g, gradients, id) {
                out = Some(PaintChange::Gradient(edited));
            }
        }
        // With No Color the colours still show, as in Photoshop: a click makes the paint solid.
        (Kind::Solid | Kind::None, _) => {
            ui.label(egui::RichText::new(tl!("Recently Used Colors")).color(t.text_dim).size(12.0));
            ui.add_space(2.0);
            // One row, kept the same height while it is still empty.
            ui.allocate_ui(vec2(WIDTH, CELL), |ui| {
                ui.set_min_height(CELL);
                grid(ui, lib.recent.len(), |ui, i| {
                    let Some(&rgb) = lib.recent.get(i) else { return };
                    let selected = solid_now.is_some_and(|c| crate::color_picker_ui::hex(c) == crate::color_picker_ui::hex(rgb));
                    if cell(ui, c32(rgb), &crate::color_picker_ui::hex(rgb), selected) {
                        out = Some(PaintChange::Color([rgb[0], rgb[1], rgb[2], original[3]]));
                    }
                });
            });
            ui.add_space(6.0);
            folders(ui, &lib.swatches, |ui, g, i| {
                let Some((name, color, rgb)) = g.items.get(i) else { return };
                let selected = solid_now.is_some_and(|c| crate::color_picker_ui::hex(c) == crate::color_picker_ui::hex(*rgb));
                if cell(ui, *color, name, selected) {
                    out = Some(PaintChange::Color([rgb[0], rgb[1], rgb[2], original[3]]));
                }
            });
        }
        // The Gradient type was just clicked: the gradient shows once the paint holds it.
        _ => {}
    }
    ui.data_mut(|d| d.insert_temp(tab_id, (current, tab)));
    out
}

/// The swatch library's folders in a scrolling list, each opening to a grid of its swatches.
fn folders(ui: &mut egui::Ui, groups: &[SwatchGroup], mut item: impl FnMut(&mut egui::Ui, &SwatchGroup, usize)) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::new().fill(t.field).corner_radius(t.radius_sm).inner_margin(4.0).show(ui, |ui| {
        // A fixed height, as Photoshop's list: opening a folder scrolls instead of growing the
        // popover under the pointer.
        egui::ScrollArea::vertical().max_height(LIST_HEIGHT).min_scrolled_height(LIST_HEIGHT).auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(WIDTH - 8.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            for (gi, g) in groups.iter().enumerate() {
                let (name, n) = (&g.name, g.items.len());
                let open_id = ui.scope_id().with(("paint-picker-folder", gi, name));
                let mut open = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
                folder_row(ui, name, &mut open);
                ui.data_mut(|d| d.insert_temp(open_id, open));
                if open {
                    ui.indent(open_id, |ui| grid(ui, n, |ui, i| item(ui, g, i)));
                    ui.add_space(4.0);
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remember_moves_a_colour_to_the_front_once_and_caps_the_row() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        for i in 0..20 {
            remember(&mut app, [i as f32 / 255.0, 0.0, 0.0]);
        }
        let recent = recent_colors(&app.session);
        assert_eq!(recent.len(), MAX_RECENT);
        assert_eq!(crate::color_picker_ui::hex(recent[0]), "#130000", "newest first");
        remember(&mut app, [10.0 / 255.0, 0.0, 0.0]);
        let recent = recent_colors(&app.session);
        assert_eq!(crate::color_picker_ui::hex(recent[0]), "#0a0000");
        assert_eq!(recent.iter().filter(|c| crate::color_picker_ui::hex(**c) == "#0a0000").count(), 1, "no duplicates");
        // Non-finite colours and hand-edited junk never reach the row.
        remember(&mut app, [f32::NAN, 0.0, 0.0]);
        app.session.prefs.edit(|p| p.recent_colors.insert(0, "not a colour é".into()));
        assert_eq!(crate::color_picker_ui::hex(recent_colors(&app.session)[0]), "#0a0000");
    }
}
