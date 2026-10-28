use super::{Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, pos2, theme, vec2};

/// Match the Spectrum application-menu padding and blue/white hover state in
/// secondary menus (status bar, dock hamburger, layer context menus; #2187).
/// The setting is local to this popup's Ui and leaves other themes unchanged.
pub fn style_spectrum_popup_menu(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // The 6 pt gap between rows was outside both buttons, so a drag across a menu
    // dropped the highlight (#4602). Padding stays inside the row, which is the hit target.
    ui.spacing_mut().item_spacing.y = 0.0;
    if !t.pro {
        return;
    }
    ui.spacing_mut().button_padding = vec2(10.0, 4.0);
    let v = &mut ui.style_mut().visuals;
    v.widgets.hovered.weak_bg_fill = t.accent;
    v.widgets.hovered.bg_fill = t.accent;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.inactive.corner_radius = CornerRadius::ZERO;
    v.widgets.hovered.corner_radius = CornerRadius::ZERO;
    v.widgets.active.corner_radius = CornerRadius::ZERO;
}

/// An accent insertion line on one edge of `r` while a drag hovers it (vertical: on its left or,
/// `after`, right edge; else on its top or bottom).
pub fn drop_line(ui: &Ui, r: Rect, after: bool, vertical: bool, t: &Tokens) {
    let s = Stroke::new(2.0, t.accent);
    if vertical {
        let x = if after { r.right() } else { r.left() };
        ui.painter().line_segment([pos2(x, r.top() + 2.0), pos2(x, r.bottom() - 2.0)], s);
    } else {
        let y = if after { r.bottom() } else { r.top() };
        ui.painter().line_segment([pos2(r.left() + 4.0, y), pos2(r.right() - 4.0, y)], s);
    }
}

/// Draw a bevelled box (Classic theme) or a flat rounded box.
pub fn surface(ui: &Ui, rect: Rect, fill: Color32, raised: bool) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    p.rect_filled(rect, t.radius_sm, fill);
    if t.bevel {
        let (hi, lo) = if raised { (Color32::WHITE, Color32::from_gray(64)) } else { (Color32::from_gray(64), Color32::WHITE) };
        p.line_segment([rect.left_bottom(), rect.left_top()], Stroke::new(1.0, hi));
        p.line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, hi));
        p.line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(1.0, lo));
        p.line_segment([rect.right_bottom(), rect.left_bottom()], Stroke::new(1.0, lo));
    }
}

/// Inner margin of a dock panel's body: a Pro panel's, or a Studio card's below its tab strip.
pub(crate) fn body_margin(pro: bool) -> egui::Margin {
    if pro { egui::Margin::same(8) } else { egui::Margin { left: 8, right: 8, top: 6, bottom: 10 } }
}

/// Height of a panel footer's bar ([`panel_footer`]), under its 1 px line.
pub const FOOTER_BAR: f32 = 30.0;

/// The height a [`panel_footer`] takes from the panel body: its line, and its bar down to the
/// body's bottom margin (the bar runs on over the margin).
pub fn footer_height(ui: &Ui) -> f32 {
    1.0 + FOOTER_BAR - body_margin(Tokens::get(ui.ctx()).pro).bottom as f32
}

/// A panel footer as in Photoshop's Layers and History panels: a bar of buttons under a line that
/// runs from edge to edge of the panel, the buttons centred in the bar's height and laid out from
/// the right. Drawn last in a body that fills its group (leave it [`footer_height`]), the bar ends
/// at the panel's bottom edge.
pub fn panel_footer<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    let m = body_margin(t.pro);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), footer_height(ui)), Sense::hover());
    let y = r.top() + 0.5;
    ui.painter().line_segment([pos2(r.left() - m.left as f32, y), pos2(r.right() + m.right as f32, y)], Stroke::new(1.0, t.separator));
    let bar = Rect::from_min_size(pos2(r.left(), r.top() + 1.0), vec2(r.width(), FOOTER_BAR));
    let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)));
    // The bar runs on over the body's bottom margin, past where the body's own clip ends: without
    // this its buttons were cut off there, a 26 px hover highlight drawn about 21 px tall.
    bar_ui.set_clip_rect(bar_ui.clip_rect().union(bar));
    bar_ui.spacing_mut().item_spacing.x = 2.0;
    add(&mut bar_ui)
}

/// The panel menu button (hamburger in Pro, ellipsis in Studio), matching the dock and cards.
pub fn panel_menu_button(ui: &mut Ui) -> Response {
    let t = Tokens::get(ui.ctx());
    let size = vec2(20.0, 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let tint = crate::icons::button_chrome(ui, rect, false, resp.hovered());
    if t.pro {
        for k in 0..3 {
            let y = rect.center().y - 3.5 + k as f32 * 3.5;
            ui.painter().line_segment([pos2(rect.center().x - 5.0, y), pos2(rect.center().x + 5.0, y)], Stroke::new(1.0, tint));
        }
    } else {
        crate::icons::paint(ui, rect, "ellipsis", 14.0, tint);
    }
    let tip = tl!("Panel menu");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    resp.on_hover_text(tip)
}

pub fn pill_tab(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let translated = tl!(label);
    let font = theme::medium(12.5);
    let galley = ui.painter().layout_no_wrap(translated.to_string(), font, t.text);
    let size = vec2(galley.size().x + 20.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if selected {
        surface(ui, rect, t.hover, true);
        if !t.bevel {
            ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
        }
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, t.radius_sm, t.hover.gamma_multiply(0.6));
    }
    let color = if selected { t.text } else { t.text_dim };
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, translated));
    resp
}

/// Small caps section label.
pub fn section_label(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!(text)).font(theme::medium(11.5)).color(t.text_faint));
}

/// Hairline separator.
pub fn hairline(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().line_segment([r.left_center(), r.right_center()], Stroke::new(1.0, t.separator));
}

/// Vertical hairline for horizontal layouts.
pub fn vline(ui: &mut Ui, height: f32) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(9.0, height), Sense::hover());
    ui.painter().line_segment([r.center_top(), r.center_bottom()], Stroke::new(1.0, t.separator));
}

/// Small chevron for dropdowns (replaces egui's filled triangle).
pub fn chevron_icon(ui: &Ui, rect: Rect, visuals: &egui::style::WidgetVisuals, _open: bool) {
    let c = rect.center();
    let s = 3.2;
    let stroke = Stroke::new(1.3, visuals.fg_stroke.color.gamma_multiply(0.8));
    ui.painter().line_segment([c + vec2(-s, -s * 0.5), c + vec2(0.0, s * 0.5)], stroke);
    ui.painter().line_segment([c + vec2(0.0, s * 0.5), c + vec2(s, -s * 0.5)], stroke);
}
