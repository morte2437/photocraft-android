//! Mouse buttons on the canvas. Tools follow the left button. The right button never paints with
//! the tool: with a painting tool it opens the Brush Preset picker at the pointer (Photoshop), or,
//! with Preferences › Tools › Right-click with painting tools set to Erase, a right drag with the
//! Brush erases with the current brush (Krita, Paint). Each right stroke is one `paint.stroke`
//! with `"erase": true`, so one undo step, with the pen pressure and tilt of a normal stroke.

use egui::{PointerButton, Response};
use photocraft_engine::BrushSettings;
use photocraft_engine::prefs::RightClickPaint;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The brush is a tool option, kept per tool like Photoshop's options bar: switching from the
/// Brush to the Eraser saves the session brush for the old tool and loads the new tool's, so each
/// keeps its own size, hardness, mode, opacity, dynamics and smoothing. A tool seen for the
/// first time takes what the brush already has, at Photoshop's 10 % smoothing. Other tools leave
/// the brush alone.
///
/// The foreground and background colours, the erase flag and the stroke seed belong to the
/// session rather than to the tool, so a swap never carries them between tools: picking a colour
/// or painting with the Eraser works with any tool's brush.
pub fn sync_tool_brush(app: &mut PhotocraftApp) {
    let tool = app.ui.tool;
    if !has_brush_picker(tool) || app.ui.brush_tool == tool {
        return;
    }
    // The live brush is the outgoing tool's, so save it there before the new one takes over: any
    // edit it made since it was last active is in it, so its copy never goes stale.
    let current = app.session.tools.brush.clone();
    save_brush(app, app.ui.brush_tool, current.clone());
    app.session.tools.brush = match app.ui.tool_brushes.iter().find(|(t, _)| *t == tool) {
        Some((_, b)) => {
            // A brush remembered from the previous run holds its stored tips as references:
            // load their full bitmaps (a missing one keeps painting with its preview).
            let mut b = session_fields(&current, b);
            let _ = app.session.load_brush_tips(&mut b);
            b
        }
        // A tool seen for the first time takes what the brush has, at Photoshop's smoothing, but
        // in Normal mode: each tool's Mode starts at Normal, so a Multiply Brush doesn't make the
        // first Clone Stamp stroke multiply (or give Healing a mode its menu doesn't have).
        None => BrushSettings { smoothing: first_smoothing(), mode: photocraft_color::BlendMode::Normal, ..current },
    };
    // The picked preset was the outgoing tool's brush: forget it, so Update Brush can't overwrite
    // that preset with this tool's brush.
    app.session.tools.current_preset = None;
    app.session.tools.brush_changed();
    app.ui.brush_tool = tool;
}

/// The brush `tool` had when it was last active.
fn save_brush(app: &mut PhotocraftApp, tool: Tool, brush: BrushSettings) {
    match app.ui.tool_brushes.iter_mut().find(|(t, _)| *t == tool) {
        Some(e) => e.1 = brush,
        None => app.ui.tool_brushes.push((tool, brush)),
    }
}

/// The session's own brush fields, which a tool swap never carries: the colours, the erase flag
/// and the stroke seed (a stroke's replay seed is the command's, not the tool's).
fn session_fields(current: &BrushSettings, brush: &BrushSettings) -> BrushSettings {
    BrushSettings { color: current.color, background: current.background, erase: current.erase, seed: current.seed, ..brush.clone() }
}

/// A tool's brush as it starts out (Reset Tool, #2825): the default brush at Photoshop's
/// smoothing, with the session's own fields of `current`.
pub(crate) fn fresh_brush(current: &BrushSettings) -> BrushSettings {
    session_fields(current, &BrushSettings { smoothing: first_smoothing(), ..BrushSettings::default() })
}

/// Photoshop's 10 % stroke smoothing, what a tool's brush starts at.
fn first_smoothing() -> photocraft_engine::paint::brush::Smoothing {
    photocraft_engine::paint::brush::Smoothing { amount: 0.1, ..Default::default() }
}

/// Does a stroke with `tool` starting now paint in Photoshop's Clear mode, erasing with its own
/// brush (#3439)? Clear chosen in its Mode dropdown, or `~` held (`hold_keys::erase_held`).
pub fn clears(app: &PhotocraftApp, tool: Tool) -> bool {
    let chosen = match tool {
        Tool::Brush => app.ui.tool_options.brush_clear,
        Tool::Pencil => app.ui.tool_options.pencil_clear,
        _ => false,
    };
    crate::hold_keys::erases_while_held(tool) && (chosen || app.held_erase)
}

/// Which tool events this frame's canvas response produces.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Buttons {
    pub started: bool,
    pub dragged: bool,
    pub stopped: bool,
    pub clicked: bool,
}

/// Tools with the options-bar brush chip, which opens the Brush Preset picker.
pub fn has_brush_picker(tool: Tool) -> bool {
    tool.is_brushlike() || tool == Tool::QuickSelection
}

/// Tools whose canvas right-click opens the Brush Preset picker: the painting tools. The Quick
/// Selection tool has the picker too, but its right-click opens the selection menu (#7268).
pub fn right_click_picker(tool: Tool) -> bool {
    has_brush_picker(tool) && !crate::canvas_tool_menu::applies(tool)
}

/// Does a right-button drag with `tool` erase (rather than open the picker)?
pub fn right_erases(app: &PhotocraftApp, tool: Tool) -> bool {
    matches!(tool, Tool::Brush | Tool::Eraser) && app.session.prefs().tools.right_click_with_painting_tools == RightClickPaint::Erase
}

/// Route the canvas response's buttons: the left one drives the tool; the right one resizes the
/// brush with Alt held (#297), erases (Erase preference) or opens the Brush Preset picker. Arms
/// `secondary_erase` or `brush_resize_armed` for this frame's `Down`.
pub fn canvas_buttons(app: &mut PhotocraftApp, response: &Response, tool: Tool) -> Buttons {
    if crate::brush_mode_menu::blocks_canvas(app, &response.ctx) {
        app.secondary_erase = false;
        return Buttons::default();
    }
    let physical_right = app.services.right_button_down.as_ref().is_some_and(|f| f());
    let (mods, mut right_down) = response.ctx.input(|i| (i.modifiers, i.pointer.secondary_down()));
    right_down |= physical_right;
    let right_started = response.drag_started_by(PointerButton::Secondary) || (physical_right && response.drag_started());
    let right_dragged = response.dragged_by(PointerButton::Secondary) || (physical_right && response.dragged());
    let resizing = app.brush_resize.is_some_and(|r| r.secondary);
    let right_stopped = response.drag_stopped_by(PointerButton::Secondary) || (resizing && response.drag_stopped());

    if physical_right && crate::brush_resize::is_right_gesture(crate::workspace_ui::sticky_mods(app, mods)) {
        app.alt_sampling = false;
    }

    // Alt+right-drag resizes the brush (brush_resize.rs); its events reach `tool_event` like a
    // left drag's, and nothing paints. A resize whose release was missed ends here.
    crate::brush_resize::release_stale(app, right_down || response.drag_stopped_by(PointerButton::Secondary) || response.drag_stopped());
    let resize_start = crate::brush_resize::applies(tool)
        && app.drag.is_none()
        && crate::brush_resize::is_right_gesture(crate::workspace_ui::sticky_mods(app, mods))
        && (right_started || (physical_right && response.dragged() && app.brush_resize.is_none()));
    let resizing = app.brush_resize.is_some_and(|r| r.secondary);
    app.brush_resize_armed = resize_start || (physical_right && resizing);
    // ⌘/Ctrl+right-click lists the layers under the pointer instead (layer_pick_ui.rs, #307).
    let layer_menu = crate::layer_pick_ui::is_gesture(tool, mods);
    // Decided by the modifiers held when the button went down, not by those of the release frame that reports the click.
    let mode_menu = crate::brush_mode_menu::is_gesture(tool, crate::workspace_ui::sticky_mods(app, crate::brush_mode_menu::gesture_mods(&response.ctx, mods)));
    if mode_menu && !resizing && app.drag.is_none() {
        app.secondary_erase = false;
        if response.secondary_clicked()
            && let Some(p) = response.interact_pointer_pos()
        {
            crate::brush_mode_menu::open(app, &response.ctx, [p.x, p.y]);
        }
        return Buttons {
            started: response.drag_started_by(PointerButton::Primary),
            dragged: response.dragged_by(PointerButton::Primary),
            stopped: response.drag_stopped_by(PointerButton::Primary),
            clicked: response.clicked(),
        };
    }
    let erase = right_erases(app, tool) && !resize_start && !resizing && !layer_menu;
    // Shift+Alt+right-press opens Photoshop's HUD Color Picker (#6098) instead of the picker.
    // Tools whose right-click is the canvas tool menu (Quick Selection, #7268) keep that menu.
    let sticky = crate::workspace_ui::sticky_mods(app, mods);
    let hud_gesture = right_click_picker(tool) && !layer_menu && !erase && sticky.shift && sticky.alt;
    let right_press = response.ctx.input(|i| i.pointer.button_pressed(PointerButton::Secondary)) && response.contains_pointer();
    if hud_gesture
        && right_press
        && let Some(p) = response.ctx.input(|i| i.pointer.hover_pos())
    {
        crate::color_hud::open(app, [p.x, p.y], true);
    }
    let right_stroke = erase && app.drag.is_some();
    let right_start = erase && right_started;
    let right_click = response.secondary_clicked();
    if right_click
        && !erase
        && !layer_menu
        && !crate::color_hud::is_open(&app.ui)
        && right_click_picker(tool)
        && let Some(p) = response.interact_pointer_pos()
    {
        crate::brush_picker::open(&mut app.ui, &response.ctx, [p.x, p.y]);
    }
    let erase_click = erase && right_click;
    app.secondary_erase = right_start || erase_click;
    let right_drag = right_stroke || resizing;
    Buttons {
        started: response.drag_started_by(PointerButton::Primary) || right_start || resize_start,
        dragged: response.dragged_by(PointerButton::Primary) || (right_drag && right_dragged),
        stopped: response.drag_stopped_by(PointerButton::Primary) || (right_drag && right_stopped),
        clicked: response.clicked() || erase_click,
    }
}

/// `ui.pointer` with `"button": "secondary"`: true when its events should reach the tool (an
/// Alt+right-drag brush resize, `brush_resize_armed` for its `Down`; or an erasing right stroke,
/// `secondary_erase` armed for its `Down`). Otherwise a right-click with a painting tool opens
/// the Brush Preset picker at screen point `at`, and nothing paints.
pub fn pointer_secondary(app: &mut PhotocraftApp, ctx: &egui::Context, down: bool, mods: egui::Modifiers, at: [f32; 2]) -> bool {
    let tool = app.ui.tool;
    // Shift+Alt+right is Photoshop's HUD Color Picker (#6098): the down opens it, the up closes
    // it. As on the canvas, the right-erase preference wins over the HUD, and a tool whose
    // right-click is the canvas tool menu keeps that menu (handled by the caller).
    let hud_gesture = right_click_picker(tool) && mods.shift && mods.alt && !right_erases(app, tool);
    // A control-channel request carries one modifier set, so the Up need not repeat the Down's:
    // the caller may release Shift and Alt first. Any Up therefore closes a non-held HUD that is
    // still open; `release` leaves a real-mouse (held) one to its button release.
    let hud_up = !down && app.ui.color_hud.is_some_and(|h| !h.hold);
    if hud_gesture || hud_up {
        if down {
            crate::color_hud::open(app, at, false);
        } else {
            crate::color_hud::release(&mut app.ui);
        }
        return false;
    }
    if crate::brush_resize::applies(tool) && (crate::brush_resize::is_right_gesture(mods) || app.brush_resize.is_some_and(|r| r.secondary)) {
        app.brush_resize_armed = down && app.drag.is_none();
        return true;
    }
    if crate::brush_mode_menu::is_gesture(tool, mods) && app.drag.is_none() && app.brush_resize.is_none() {
        app.secondary_erase = false;
        if down {
            crate::brush_mode_menu::open(app, ctx, at);
        }
        return false;
    }
    if right_erases(app, tool) {
        app.secondary_erase = down;
        return true;
    }
    if down && right_click_picker(tool) {
        crate::brush_picker::open(&mut app.ui, ctx, at);
    }
    false
}

/// The Brush Preset picker a right-click or the options-bar brush chip opened, at the pointer. It
/// edits the session brush like the Brushes panel; a press outside, Escape, Enter or a
/// double-click on a preset closes it.
pub fn show_picker(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some([x, y]) = app.ui.brush_picker else { return };
    if !has_brush_picker(app.ui.tool) {
        app.ui.brush_picker = None;
        return;
    }
    // Escape and Enter close it once this frame's edits are in (a typed size applies), unless they
    // end a rename in its rename bar.
    let key_close = app.ui.brush_picker_list.renaming.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter));
    let screen = ctx.content_rect();
    // The picker's content size: the one its grip was dragged to, or the one it last closed with
    // (kept in egui's own memory, which outlives the app's), or the default. The frame's chrome
    // sits around it, and together they keep the whole picker on screen.
    let id = crate::brush_picker::picker_id();
    let size_id = id.with("content-size");
    let content = app.ui.brush_picker_size.or_else(|| ctx.data_mut(|d| d.get_persisted::<[f32; 2]>(size_id))).map_or_else(
        || egui::vec2(crate::brush_picker::DEFAULT_SIZE[0], crate::brush_picker::DEFAULT_SIZE[1]),
        |s| crate::brush_picker::clamp_size(s, screen.size()),
    );
    // Remember it for the picker's next open, and — egui's memory being saved — the next run.
    ctx.data_mut(|d| d.insert_persisted(size_id, [content.x, content.y]));
    let style = ctx.global_style();
    let size = content + style.spacing.menu_margin.sum() + egui::Vec2::splat(2.0 * style.visuals.window_stroke().width);
    let pos = egui::pos2(x.min(screen.right() - size.x).max(screen.left()), y.min(screen.bottom() - size.y).max(screen.top()));
    let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        let before = app.session.tools.brush.clone();
        let mut b = before.clone();
        let list = &mut app.ui.brush_picker_list;
        let frame = egui::Frame::popup(ui.style())
            .show(ui, |ui| crate::brush_picker::body(ui, &mut b, &app.session.tools.presets, app.session.tools.current_preset.as_deref(), list, content));
        let picks = frame.inner;
        // The corner grip resizes the picker, and the size is remembered across opens.
        if let Some(new) = crate::brush_picker::resize_grip(ui, frame.response.rect, content) {
            app.ui.brush_picker_size = Some([new.x, new.y]);
        }
        crate::brush_panel::commit_gesture(app, ui.ctx(), &before, &b);
        let closes = picks.iter().any(crate::brush_picker::Pick::closes);
        crate::brush_picker::apply(app, ui.ctx(), picks);
        closes
    });
    // Not a press on a menu the picker opened (the gear, a preset's context menu), nor on the
    // options-bar chip, whose click toggles the picker. A menu floats over the picker in the
    // foreground order, so the press is told apart by the layer it lands on, not by any popup
    // just being open: a press elsewhere closes the picker in the same click that closes the menu.
    let press = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten());
    let on_menu = press.is_some_and(|p| ctx.layer_id_at(p).is_some_and(|l| l.order == egui::Order::Foreground));
    let outside = crate::canvas_popup::outside_pressed(ctx, id, area.response.rect) && press.is_some_and(|p| !crate::brush_picker::on_chip(ctx, p)) && !on_menu;
    if outside || key_close || area.inner {
        crate::brush_picker::close(&mut app.ui);
    }
}

#[cfg(test)]
mod tests;
