use super::panel::surface;
use super::{Color32, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, Vec2, pos2, theme, vec2};

/// iOS-style toggle switch.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    if t.pro {
        return checkbox(ui, on, label);
    }
    let mut resp = ui
        .horizontal(|ui| {
            let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 17.0), Sense::click());
            let how_on = ui.ctx().animate_bool(resp.id, *on);
            let bg = if *on { t.accent } else { t.field_border };
            if t.bevel {
                surface(ui, rect, if *on { t.accent } else { t.field }, false);
            } else {
                ui.painter().rect_filled(rect, 8.5, bg);
            }
            let x = egui::lerp((rect.left() + 8.5)..=(rect.right() - 8.5), how_on);
            ui.painter().circle_filled(pos2(x, rect.center().y), 6.5, Color32::WHITE);
            ui.label(egui::RichText::new(tl!(label)).color(if *on { t.text } else { t.text_dim }));
            resp
        })
        .inner;
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    resp
}

/// Big white primary button.
pub fn primary_button(ui: &mut Ui, label: &str, min_width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    button_impl(ui, label, min_width, t.primary_bg, t.primary_text, true)
}

/// The width of the widest of `labels` as a button, for a column of equal-width buttons.
pub fn button_width(ui: &Ui, labels: &[&str]) -> f32 {
    let fg = Tokens::get(ui.ctx()).text;
    labels.iter().map(|l| button_layout(ui, l, 0.0, None, fg).1.x).fold(0.0, f32::max)
}

pub fn secondary_button(ui: &mut Ui, label: &str, min_width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    button_impl(ui, label, min_width, t.field, t.text, false)
}

/// A [`secondary_button`] `height` points tall, for slim bars such as the status bar (#2823).
pub fn compact_button(ui: &mut Ui, label: &str, height: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    button_impl_h(ui, label, 0.0, Some(height), t.field, t.text, false)
}

/// What a dialog button does, which decides where the platform's button order puts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonRole {
    /// The default answer (OK, Save, Yes), drawn as the primary button.
    Default,
    /// Another answer that closes the dialog (Don't Save, No).
    Alternate,
    /// Closes the dialog without acting.
    Cancel,
    /// Acts but keeps the dialog open (Apply).
    Apply,
}

impl ButtonRole {
    /// Position from the left in the platform's order. Windows and Linux put the default action
    /// first (OK Cancel Apply, Yes No Cancel); macOS puts it last, in the corner, with Cancel beside
    /// it and the other answers further left (Don't Save, Cancel, Save).
    fn slot(self, mac: bool) -> u8 {
        match (self, mac) {
            (Self::Default, false) | (Self::Alternate, true) => 0,
            (Self::Alternate, false) | (Self::Cancel, true) => 1,
            (Self::Cancel, false) | (Self::Apply, true) => 2,
            (Self::Apply, false) | (Self::Default, true) => 3,
        }
    }
}

/// One button of a [`dialog_buttons`] row.
#[derive(Clone, Copy)]
pub struct DialogButton<'a> {
    pub role: ButtonRole,
    pub label: &'a str,
    pub min_width: f32,
    pub enabled: bool,
    /// Take keyboard focus this frame (see [`DialogButton::focus`]).
    pub focus: bool,
}

impl<'a> DialogButton<'a> {
    pub fn new(role: ButtonRole, label: &'a str, min_width: f32) -> Self {
        Self { role, label, min_width, enabled: true, focus: false }
    }

    pub fn enabled(self, enabled: bool) -> Self {
        Self { enabled, ..self }
    }

    /// Asks for keyboard focus for this button: from there the arrow keys move between the row's
    /// buttons and Enter or Space presses the focused one.
    pub fn focus(self, focus: bool) -> Self {
        Self { focus, ..self }
    }
}

/// A dialog's button row at the cursor, in the platform's order (see [`ButtonRole`]): every modal
/// draws its buttons through this, so they all agree. Inside a right-to-left row it sits at the
/// right edge. The buttons are laid out left to right, so Tab walks them in reading order.
/// Returns the role of the button clicked this frame.
pub fn dialog_buttons(ui: &mut Ui, buttons: &[DialogButton]) -> Option<ButtonRole> {
    let mac = ui.ctx().os() == egui::os::OperatingSystem::Mac;
    let gap = ui.spacing().item_spacing.x;
    let size = buttons.iter().fold(Vec2::ZERO, |acc, b| {
        let s = button_size(ui, b.label, b.min_width);
        vec2(acc.x + s.x, acc.y.max(s.y))
    }) + vec2(gap * buttons.len().saturating_sub(1) as f32, 0.0);
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        let mut hit = None;
        for slot in 0..4 {
            for b in buttons.iter().filter(|b| b.role.slot(mac) == slot) {
                let r = ui
                    .add_enabled_ui(b.enabled, |ui| {
                        if b.role == ButtonRole::Default { primary_button(ui, b.label, b.min_width) } else { secondary_button(ui, b.label, b.min_width) }
                    })
                    .inner;
                if b.focus && b.enabled {
                    r.request_focus();
                }
                if r.clicked() {
                    hit = Some(b.role);
                }
            }
        }
        hit
    })
    .inner
}

/// The size of a primary or secondary button: its label plus padding, at least `min_width` wide.
fn button_size(ui: &Ui, label: &str, min_width: f32) -> Vec2 {
    button_layout(ui, label, min_width, None, Tokens::get(ui.ctx()).text).1
}

/// The height of a primary or secondary button (and so of a dialog's button row).
pub fn button_height(ctx: &egui::Context) -> f32 {
    if Tokens::get(ctx).pro { 28.0 } else { 30.0 }
}

fn button_layout(ui: &Ui, label: &str, min_width: f32, height: Option<f32>, fg: Color32) -> (std::sync::Arc<egui::Galley>, Vec2) {
    let galley = ui.painter().layout_no_wrap(tl!(label).to_owned(), theme::medium(13.0), fg);
    let h = height.unwrap_or(button_height(ui.ctx()));
    let size = vec2((galley.size().x + 28.0).max(min_width), h);
    (galley, size)
}

fn button_impl(ui: &mut Ui, label: &str, min_width: f32, bg: Color32, fg: Color32, primary: bool) -> Response {
    button_impl_h(ui, label, min_width, None, bg, fg, primary)
}

/// [`button_impl`] with an explicit `height` (`None`: the theme's button height).
fn button_impl_h(ui: &mut Ui, label: &str, min_width: f32, height: Option<f32>, bg: Color32, fg: Color32, primary: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let (galley, size) = button_layout(ui, label, min_width, height, fg);
    let h = size.y;
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    // Painted text: name the button for accessibility (and so tests and agents can find it).
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    if resp.has_focus() {
        // Keyboard focus (Tab); clicks don't focus egui buttons.
        let r = if t.pro { h / 2.0 } else { t.radius_sm } + 2.0;
        ui.painter().rect_stroke(rect.expand(2.0), r, Stroke::new(2.0, t.accent), StrokeKind::Outside);
    }
    if t.pro {
        // Spectrum buttons: fully rounded; primary = filled accent, secondary = outline.
        let down = resp.is_pointer_button_down_on();
        let r = h / 2.0;
        if primary {
            let fill = if down {
                bg.gamma_multiply(0.8)
            } else if resp.hovered() {
                bg.gamma_multiply(0.9)
            } else {
                bg
            };
            ui.painter().rect_filled(rect, r, fill);
        } else {
            if resp.hovered() || down {
                ui.painter().rect_filled(rect, r, t.hover);
            }
            ui.painter().rect_stroke(rect, r, Stroke::new(1.5, if resp.hovered() { t.text } else { t.text_dim }), StrokeKind::Inside);
        }
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
        return resp;
    }
    let fill = if resp.is_pointer_button_down_on() {
        bg.gamma_multiply(0.85)
    } else if resp.hovered() {
        if primary { bg.gamma_multiply(0.93) } else { t.hover }
    } else {
        bg
    };
    surface(ui, rect, fill, !resp.is_pointer_button_down_on());
    if !t.bevel && !primary {
        ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp
}

/// Spectrum-style checkbox (blue when checked).
pub fn checkbox(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let (mut resp, label_resp) = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::CLICK);
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            let l = ui
                .add(egui::Label::new(egui::RichText::new(tl!(label)).color(t.text_dim)).selectable(false).sense(Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            // Pressing the label presses the box (#4476).
            let pressed = resp.is_pointer_button_down_on() || l.is_pointer_button_down_on();
            check_box(ui.painter(), rect, *on, resp.hovered() || l.hovered(), pressed, &t);
            (resp.union(l.clone()), l)
        })
        .inner;
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    // The label is the single named, focusable checkbox; the box is a mouse target.
    // Override Label's metadata instead of creating a second node with the same name.
    label_resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *on, tl!(label)));
    resp
}

/// The fill of a check box: the accent when `on`, else the field colour. While it's pressed
/// (the mouse is down on it or on its label) Photoshop shows that a release will act: an
/// unchecked box darkens a little and a checked one turns a little less opaque (#4476).
pub fn check_box_fill(on: bool, pressed: bool, t: &Tokens) -> Color32 {
    match (on, pressed) {
        (true, false) => t.accent,
        (true, true) => t.accent.gamma_multiply(0.75),
        (false, false) => t.field,
        (false, true) => t.field.lerp_to_gamma(Color32::BLACK, 0.18),
    }
}

/// Paint a check box in `rect`: [`check_box_fill`] with a white tick when `on`, else an
/// outline that brightens while `hovered` or `pressed`.
pub fn check_box(p: &egui::Painter, rect: Rect, on: bool, hovered: bool, pressed: bool, t: &Tokens) {
    p.rect_filled(rect, 2.0, check_box_fill(on, pressed, t));
    if on {
        let a = rect.left_center() + vec2(3.0, 0.5);
        let b = rect.center_bottom() + vec2(-1.0, -3.5);
        let c = rect.right_top() + vec2(-3.0, 3.5);
        let tick = Stroke::new(rect.width() * 0.125, Color32::WHITE);
        p.line_segment([a, b], tick);
        p.line_segment([b, c], tick);
    } else {
        p.rect_stroke(rect, 2.0, Stroke::new(1.5, if hovered || pressed { t.text_dim } else { t.text_faint }), StrokeKind::Inside);
    }
}
