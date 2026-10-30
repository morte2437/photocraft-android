//! Brush-only Shift+right-click Mode picker. Shares the options bar's painting modes.

use egui::{Context, Id};
use photocraft_doc::BlendMode;
use serde_json::json;

use crate::{PhotocraftApp, state::Tool, theme::Tokens};

/// The options bar and canvas picker use the same order, including painting-only Clear.
pub(crate) fn choices() -> Vec<((Option<BlendMode>, bool), &'static str)> {
    let mut choices: Vec<_> = BlendMode::LAYER_MODES.iter().map(|m| ((Some(*m), false), m.label())).collect();
    let at = choices.iter().position(|o| o.0 == (Some(BlendMode::Dissolve), false)).map_or(choices.len(), |i| i + 1);
    choices.insert(at, ((None, true), "Behind"));
    choices.insert(at + 1, ((None, false), "Clear"));
    choices
}

pub(crate) fn is_gesture(tool: Tool, mods: egui::Modifiers) -> bool {
    tool == Tool::Brush && mods.shift && !mods.alt && !mods.ctrl && !mods.command && !mods.mac_cmd
}

/// The modifiers that decide a right-button gesture: the ones held when the button went down.
/// egui reports `secondary_clicked` on release, so letting go of Shift a moment before the button
/// must not turn Shift+right-click into a plain right-click (an erase dab, with the Erase
/// preference). The control path decides on the press too.
pub(crate) fn gesture_mods(ctx: &Context, mods: egui::Modifiers) -> egui::Modifiers {
    let key = menu_id().with("press-mods");
    let (pressed, down, released) = ctx.input(|i| {
        (i.pointer.button_pressed(egui::PointerButton::Secondary), i.pointer.secondary_down(), i.pointer.button_released(egui::PointerButton::Secondary))
    });
    if pressed {
        ctx.data_mut(|d| d.insert_temp(key, mods));
        return mods;
    }
    if !down && !released {
        ctx.data_mut(|d| d.remove::<egui::Modifiers>(key));
        return mods;
    }
    let held = ctx.data(|d| d.get_temp::<egui::Modifiers>(key));
    if released {
        ctx.data_mut(|d| d.remove::<egui::Modifiers>(key));
    }
    held.unwrap_or(mods)
}

pub(crate) fn menu_id() -> Id {
    Id::unique("canvas-brush-mode-menu")
}

pub(crate) fn open(app: &mut PhotocraftApp, ctx: &Context, pos: [f32; 2]) {
    if !pos.iter().all(|v| v.is_finite()) {
        return;
    }
    app.ui.brush_picker = None;
    app.ui.layer_menu = None;
    app.ui.canvas_tool_menu = None;
    app.ui.brush_mode_menu = Some(pos);
    crate::canvas_popup::mark_open(ctx, menu_id());
}

/// A press outside dismisses the popup, and consumes that entire canvas gesture through Up.
pub(crate) fn blocks_canvas(app: &PhotocraftApp, ctx: &Context) -> bool {
    let key = menu_id().with("dismiss-pointer");
    if ctx.data(|d| d.get_temp::<bool>(key)).unwrap_or(false) {
        if !ctx.input(|i| i.pointer.any_down()) {
            ctx.data_mut(|d| d.remove::<bool>(key));
        }
        return true;
    }
    if app.ui.brush_mode_menu.is_some() {
        if let Some(rect) = ctx.memory(|m| m.area_rect(menu_id()))
            && crate::canvas_popup::outside_pressed(ctx, menu_id(), rect)
        {
            ctx.data_mut(|d| d.insert_temp(key, true));
        }
        return true;
    }
    false
}

/// Document-coordinate control pointers dismiss; menu rows are chosen with screen-space ui.click.
pub(crate) fn blocks_control(app: &mut PhotocraftApp, ctx: &Context, ev: crate::canvas::ToolEvent) -> bool {
    use crate::canvas::ToolEvent;
    let key = menu_id().with("control-dismiss");
    let blocked = ctx.data(|d| d.get_temp::<bool>(key)).unwrap_or(false);
    if blocked {
        if matches!(ev, ToolEvent::Up { .. }) {
            ctx.data_mut(|d| d.remove::<bool>(key));
        }
        return true;
    }
    if app.ui.brush_mode_menu.is_some() {
        if app.ui.tool != Tool::Brush {
            app.ui.brush_mode_menu = None;
            return false;
        }
        if matches!(ev, ToolEvent::Down { .. }) {
            app.ui.brush_mode_menu = None;
            ctx.data_mut(|d| d.insert_temp(key, true));
        }
        return true;
    }
    false
}

fn choose(app: &mut PhotocraftApp, mode: (Option<BlendMode>, bool)) {
    // Clear is the existing painting-only UI flag; keep the underlying blend mode just as the
    // options bar does. Journal the choice through the same brush command.
    let selected = if mode.1 { BlendMode::Normal } else { mode.0.unwrap_or(app.session.tools.brush.mode) };
    match app.run("tools.setBrush", json!({"brush": {"mode": selected,"behind":mode.1}})) {
        Ok(_) => app.ui.tool_options.brush_clear = mode == (None, false),
        Err(e) => app.ui.status = e,
    }
    app.ui.brush_mode_menu = None;
}

pub(crate) fn show(app: &mut PhotocraftApp, ctx: &Context) {
    let Some([x, y]) = app.ui.brush_mode_menu else { return };
    if app.ui.tool != Tool::Brush || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.brush_mode_menu = None;
        // Escape also consumes the release of a right-button gesture still in flight.
        if app.ui.tool == Tool::Brush && ctx.input(|i| i.pointer.secondary_down()) {
            ctx.data_mut(|d| d.insert_temp(menu_id().with("dismiss-pointer"), true));
        }
        return;
    }
    let choices = choices();
    let current = if app.ui.tool_options.brush_clear {
        (None, false)
    } else if app.session.tools.brush.behind {
        (None, true)
    } else {
        (Some(app.session.tools.brush.mode), false)
    };
    let screen = crate::work_area::visible_rect(ctx);
    let id = menu_id();
    let margin = ctx.global_style().spacing.menu_margin.sum() + egui::Vec2::splat(2.0);
    let height = (choices.len() as f32 * 22.0).min((screen.height() - margin.y).max(0.0));
    let width = (screen.width() - margin.x).clamp(0.0, 190.0);
    let size = ctx.memory(|m| m.area_rect(id)).map_or(egui::vec2(width + margin.x, height + margin.y), |r| r.size());
    let pos = egui::pos2(x.min(screen.right() - size.x).max(screen.left()), y.min(screen.bottom() - size.y).max(screen.top()));
    let mut picked = None;
    let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).constrain_to(screen).show(ctx, |ui| {
        let t = Tokens::get(ctx);
        egui::Frame::menu(ui.style()).fill(t.card).stroke(egui::Stroke::new(1.0, t.card_border)).show(ui, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            crate::widgets::menu_scroll(ui, |ui| {
                for (mode, label) in choices {
                    let selected = mode == current;
                    let color = if selected && t.bevel { t.accent_text } else { t.text };
                    let row = egui::Button::selectable(selected, egui::RichText::new(tl!(label)).color(color)).min_size(egui::vec2(width, 22.0));
                    if ui.add(row).clicked() {
                        picked = Some(mode);
                    }
                }
            });
        });
    });
    if let Some(mode) = picked {
        choose(app, mode);
    } else if crate::canvas_popup::outside_pressed(ctx, id, area.response.rect) {
        app.ui.brush_mode_menu = None;
    }
}
