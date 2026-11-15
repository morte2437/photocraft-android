//! The contextual task bar (#10890): a small floating bar on the canvas that offers the next steps
//! for what is being edited, as in the reference application. A context (today the Crop tool,
//! `crop_task_bar`) supplies only the controls in the middle; this module owns the rest, so other
//! contexts (Remove Background #2815, selections #2708, #7633, #8106) can reuse it:
//!
//! - the **position rule**: centred under the context's anchor (the crop frame's screen bounds)
//!   with its top [`GAP`] points below the anchor's bottom edge, following the anchor while it
//!   changes; dragged by its grip (or pinned), it stays where it is, its left edge fixed while it
//!   grows; always clamped inside the canvas area;
//! - the **grip**: two columns of three dots on the left, drawn in code;
//! - the **⋯ menu**: Hide bar, Reset bar position, Pin bar position (the reference's Watch quick
//!   video entry is left out);
//! - the **state**: [`TaskBarPlace`] in `Panels` (kept across restarts with the panel layout,
//!   read and set over the control channel), and Hide bar clears Window › Contextual Task Bar.
//!
//! As in the reference application its buttons have no tooltips, only a hover highlight; they
//! still carry accessible names.

use egui::{Color32, CursorIcon, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

use crate::state::TaskBarPlace;
use crate::theme::Tokens;

/// The bar's top below its anchor's bottom edge, in points (measured: 11 screen px at 100%).
pub const GAP: f32 = 11.0;

/// Height of the bar's buttons, in points.
pub const BUTTON: f32 = 26.0;

/// The bar's one area: only one context shows a bar at a time.
fn area_id() -> Id {
    Id::unique("pc-context-task-bar")
}

/// The bar's ⋯ button, handed to the contents so they place it: the Crop tool shows Cancel and
/// Done after it. Contents that never call [`Self::show`] get it at the end.
pub struct MoreMenu {
    place: TaskBarPlace,
    shown: bool,
    hide: bool,
    picked: Option<TaskBarPlace>,
}

impl MoreMenu {
    /// Draws the ⋯ button (and its menu while open) here.
    pub fn show(&mut self, ui: &mut Ui) {
        if std::mem::replace(&mut self.shown, true) {
            return;
        }
        let (hide, picked) = more_menu(ui, &self.place);
        self.hide |= hide;
        self.picked = picked.or(self.picked);
    }
}

/// What the bar did this frame.
pub struct Shown<R> {
    /// The bar's screen rect.
    pub rect: Rect,
    /// Hide bar was picked: the caller turns Window › Contextual Task Bar off.
    pub hide: bool,
    /// What the context's contents returned.
    pub inner: R,
}

/// The size the bar had when it was last drawn (its contents decide it), or a guess before.
fn last_size(ctx: &egui::Context) -> Vec2 {
    ctx.memory(|m| m.area_rect(area_id())).map_or(vec2(300.0, BUTTON + 8.0), |r| r.size())
}

/// Where the bar's top-left goes: dragged or pinned at `place.at`, else centred [`GAP`] below
/// `anchor`; clamped so a bar of `size` stays inside `area`. A non-finite saved place (a
/// hand-edited preferences file, `ui.set`) counts as not placed.
pub fn position(place: &TaskBarPlace, anchor: Rect, area: Rect, size: Vec2) -> Pos2 {
    let at = place.at.filter(|p| p.iter().all(|v| v.is_finite())).map(|[x, y]| area.min + vec2(x, y));
    let p = at.unwrap_or_else(|| pos2(anchor.center().x - size.x / 2.0, anchor.bottom() + GAP));
    clamp_into(p, size, area)
}

/// `p` moved so a rect of `size` at it lies inside `area` (its left/top edge wins when it can't fit).
fn clamp_into(p: Pos2, size: Vec2, area: Rect) -> Pos2 {
    let fit = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.min(hi).max(lo) } else { lo };
    pos2(fit(p.x, area.left(), area.right() - size.x), fit(p.y, area.top(), area.bottom() - size.y))
}

/// Draws the bar for `anchor` (screen rect) inside the canvas `area`: grip, then `contents`, which
/// place the ⋯ button with [`MoreMenu::show`]. `place` is updated by a drag on the grip and by the
/// menu; `None` when `area` is degenerate.
pub fn show<R>(
    ctx: &egui::Context,
    place: &mut TaskBarPlace,
    anchor: Rect,
    area: Rect,
    contents: impl FnOnce(&mut Ui, &mut MoreMenu) -> R,
) -> Option<Shown<R>> {
    if !(anchor.is_finite() && area.is_finite() && area.is_positive()) {
        return None;
    }
    let size = last_size(ctx);
    let at = position(place, anchor, area, size);
    // Pin bar position, or a pin set over the control channel: the bar freezes where it is now.
    if place.pinned && place.at.is_none() {
        place.at = Some((at - area.min).into());
    }
    let t = Tokens::get(ctx);
    let mut moved = None;
    let mut menu = MoreMenu { place: *place, shown: false, hide: false, picked: None };
    let out = egui::Area::new(area_id()).order(egui::Order::Middle).fixed_pos(at).constrain(false).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(t.card)
            .stroke(Stroke::new(1.0, t.field_border))
            .corner_radius(t.radius)
            .inner_margin(egui::Margin::symmetric(4, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if let Some(d) = grip(ui, t) {
                        moved = Some(d);
                    }
                    let inner = contents(ui, &mut menu);
                    menu.show(ui);
                    inner
                })
                .inner
            })
            .inner
    });
    let rect = out.response.rect;
    if let Some(d) = moved {
        // A dragged bar stays where it is dropped and no longer follows the frame.
        let p = clamp_into(at + d, rect.size(), area);
        place.at = Some((p - area.min).into());
    }
    if let Some(p) = menu.picked {
        *place = p;
        if place.pinned {
            place.at = Some((at - area.min).into());
        }
    }
    // The first frame only measures the contents: draw again at the measured size.
    if rect.size() != size {
        ctx.request_repaint();
    }
    Some(Shown { rect, hide: menu.hide, inner: out.inner })
}

/// The grip: two columns of three dots. Returns the drag's movement this frame.
fn grip(ui: &mut Ui, t: Tokens) -> Option<Vec2> {
    let (rect, resp) = ui.allocate_exact_size(vec2(12.0, BUTTON), Sense::drag());
    let resp = resp.on_hover_cursor(CursorIcon::Grab);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Move bar")));
    let dot: Color32 = if resp.hovered() || resp.dragged() { t.text } else { t.icon };
    for col in [-2.0, 2.0] {
        for row in [-4.0, 0.0, 4.0] {
            ui.painter().circle_filled(rect.center() + vec2(col, row), 1.1, dot);
        }
    }
    if resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        return Some(resp.drag_delta()).filter(|d| *d != Vec2::ZERO);
    }
    None
}

/// The ⋯ button and its menu. Returns whether Hide bar was picked and the place Reset or Pin
/// asks for.
fn more_menu(ui: &mut Ui, place: &TaskBarPlace) -> (bool, Option<TaskBarPlace>) {
    let r = crate::icons::button_with_icon_size_tip(ui, "ellipsis", BUTTON, 14.0, false, tl!("More options"), false);
    let mut out = (false, None);
    egui::Popup::menu(&r).show(|ui| {
        ui.set_min_width(190.0);
        let item = |ui: &mut Ui, icon: &str, label: &str| {
            let mut atoms = egui::Atoms::default();
            atoms.push_right(crate::icons::image(icon, 14.0, Tokens::get(ui.ctx()).icon));
            atoms.push_right(label.to_owned());
            ui.add(egui::Button::new(atoms)).clicked()
        };
        if item(ui, "eye-off", tl!("Hide bar")) {
            out.0 = true;
            ui.close();
        }
        if item(ui, "rotate-ccw", tl!("Reset bar position")) {
            out.1 = Some(TaskBarPlace::default());
            ui.close();
        }
        if item(ui, "pin", tl!("Pin bar position")) {
            out.1 = Some(TaskBarPlace { pinned: true, ..*place });
            ui.close();
        }
        // The reference's Watch quick video entry opens a tutorial video: not offered.
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect { min: pos2(0.0, 40.0), max: pos2(800.0, 640.0) };
    const SIZE: Vec2 = vec2(300.0, 34.0);

    #[test]
    fn follows_the_anchor_centred_eleven_points_below() {
        let anchor = Rect::from_min_max(pos2(100.0, 100.0), pos2(500.0, 400.0));
        assert_eq!(position(&TaskBarPlace::default(), anchor, AREA, SIZE), pos2(150.0, 411.0));
    }

    #[test]
    fn stays_inside_the_canvas_area() {
        // The frame's bottom edge past the window: the bar sits on the area's bottom edge.
        let low = Rect::from_min_max(pos2(-400.0, 300.0), pos2(100.0, 900.0));
        assert_eq!(position(&TaskBarPlace::default(), low, AREA, SIZE), pos2(0.0, 606.0));
        let right = Rect::from_min_max(pos2(700.0, 100.0), pos2(1200.0, 200.0));
        assert_eq!(position(&TaskBarPlace::default(), right, AREA, SIZE).x, 500.0);
    }

    #[test]
    fn a_placed_bar_keeps_its_place_and_its_left_edge() {
        let anchor = Rect::from_min_max(pos2(100.0, 100.0), pos2(500.0, 400.0));
        let place = TaskBarPlace { pinned: false, at: Some([20.0, 30.0]) };
        assert_eq!(position(&place, anchor, AREA, SIZE), pos2(20.0, 70.0));
        let moved = anchor.translate(vec2(90.0, 50.0));
        assert_eq!(position(&place, moved, AREA, vec2(420.0, 34.0)), pos2(20.0, 70.0), "grows to the right, stays put");
    }

    #[test]
    fn hostile_places_never_leave_the_area() {
        let anchor = Rect::from_min_max(pos2(100.0, 100.0), pos2(500.0, 400.0));
        for at in [[f32::NAN, 0.0], [f32::INFINITY, 1.0], [1e30, -1e30]] {
            let p = position(&TaskBarPlace { pinned: true, at: Some(at) }, anchor, AREA, SIZE);
            assert!(AREA.contains(p), "{at:?} -> {p:?}");
        }
        // A bar wider than the area keeps its left edge on the area's.
        assert_eq!(position(&TaskBarPlace::default(), anchor, AREA, vec2(1200.0, 34.0)).x, 0.0);
    }
}
