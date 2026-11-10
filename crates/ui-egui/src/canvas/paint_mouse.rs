//! Mouse buttons on the canvas. Tools follow the left button. The right button never paints with
//! the tool: with a painting tool it opens the Brush Preset picker at the pointer (Photoshop), or,
//! with Preferences › Tools › Right-click with painting tools set to Erase, a right drag with the
//! Brush erases with the current brush (Krita, Paint). Each right stroke is one `paint.stroke`
//! with `"erase": true`, so one undo step, with the pen pressure and tilt of a normal stroke.

use egui::{PointerButton, Pos2, Response};
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
        // The Art History Brush also starts at full opacity (#10263): its Opacity is its own.
        // The Background Eraser, the Healing Brush and the Burn tool start with their own measured
        // tips instead, as the whole brush Reset Tool gives them (#11911, #12099, #12671).
        None if matches!(tool, Tool::BackgroundEraser | Tool::Healing | Tool::Burn) => fresh_brush(tool, &current),
        None => {
            let opacity = if tool == Tool::ArtHistoryBrush { BrushSettings::default().opacity } else { current.opacity };
            BrushSettings { smoothing: first_smoothing(), mode: photocraft_color::BlendMode::Normal, opacity, ..current }
        }
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

/// The Brush tool's tip after Reset Tool, as the reference measures it (#11810): size, hardness
/// and spacing of a 13 px soft round at 25 % spacing.
const BRUSH_TOOL_TIP: (f32, f32, f32) = (13.0, 0.0, 0.25);
/// The Background Eraser's tip after Reset Tool (#11911): a 13 px hard round at 25 % spacing.
const BACKGROUND_ERASER_TIP: (f32, f32, f32) = (13.0, 1.0, 0.25);
/// The Healing Brush's tip after Reset Tool (#12099): a 19 px hard round at 25 % spacing.
const HEALING_BRUSH_TIP: (f32, f32, f32) = (19.0, 1.0, 0.25);
/// The Burn tool's tip after Reset Tool (#12671): a 65 px soft round at 25 % spacing.
const BURN_TOOL_TIP: (f32, f32, f32) = (65.0, 0.0, 0.25);

/// `tool`'s brush as it starts out (Reset Tool, #2825): the default brush at Photoshop's
/// smoothing, with the session's own fields of `current`. The Brush tool's tip is
/// [`BRUSH_TOOL_TIP`], the Background Eraser's [`BACKGROUND_ERASER_TIP`], the Healing Brush's
/// [`HEALING_BRUSH_TIP`], the Burn tool's [`BURN_TOOL_TIP`]; the other tools keep the default
/// tip.
pub(crate) fn fresh_brush(tool: Tool, current: &BrushSettings) -> BrushSettings {
    let mut brush = BrushSettings { smoothing: first_smoothing(), ..BrushSettings::default() };
    match tool {
        Tool::Brush => (brush.size, brush.hardness, brush.spacing) = BRUSH_TOOL_TIP,
        Tool::BackgroundEraser => (brush.size, brush.hardness, brush.spacing) = BACKGROUND_ERASER_TIP,
        Tool::Healing => (brush.size, brush.hardness, brush.spacing) = HEALING_BRUSH_TIP,
        Tool::Burn => (brush.size, brush.hardness, brush.spacing) = BURN_TOOL_TIP,
        _ => {}
    }
    session_fields(current, &brush)
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

/// A right-click on the canvas, as the system reads one: the right button pressed on the canvas
/// and released over it, the pointer having moved no more than a few pixels while it was down
/// (`right_click_reach`). egui reports `secondary_clicked` only within its click distance of the
/// press, and a high-DPI mouse can move that far just from the button going down; egui then sees
/// a drag (#10183). A right press that a gesture used (an Alt+right brush resize, a right-erase
/// stroke, a pan, another button pressed with it, a move of the view under it), or that Escape
/// cancelled, is no click.
pub fn right_clicked(response: &Response) -> bool {
    right_click_at(response).is_some()
}

/// Where a `right_clicked` click points, in the canvas's coordinates: every canvas menu hit-tests
/// its target there (a guide, the layers under it, the pasteboard, a pending crop) and opens there.
///
/// A click egui reports itself (`secondary_clicked`, within its click distance and click time)
/// counts as it always has, at the pointer. Otherwise the canvas's own record of the press
/// (`track_right_press`) decides, and the release is a right-click when all of these hold:
/// - the press went down on the canvas itself: inside it, under no popup or window, and on no
///   widget drawn over it (a scrollbar);
/// - the pointer never went farther from the press than `right_click_reach` (three times egui's
///   click distance, 18 points by default) while the button was down, at any point between the
///   press and the release, also within one frame's moves: past it the press is a drag;
/// - no other mouse button was down when it went down, and none was pressed while it was held;
/// - no gesture used it: an Alt+right brush resize, a right-erase stroke, a pan (Space's too);
/// - the view did not move under it while it was held (a scroll, a zoom, a turn), so it still
///   points at what was pressed (`track_right_press_view`);
/// - Escape was not pressed while it was held;
/// - it came up on the canvas.
///
/// Such a click points where the button went down, at what was right-clicked, not where those few
/// pixels of movement took the pointer by the release. The record also covers a slow frame where
/// the move and the release arrive together, which egui reports as neither a click nor a drag,
/// and a still press held past egui's click time, which egui no longer calls a click: that counts
/// on release, as a system context menu does.
pub fn right_click_at(response: &Response) -> Option<Pos2> {
    if response.secondary_clicked() {
        return response.interact_pointer_pos();
    }
    let ctx = &response.ctx;
    let down = ctx.data(|d| d.get_temp::<Option<Pos2>>(right_pressed_id(response))).flatten()?;
    let up = ctx.input(|i| i.pointer.button_released(PointerButton::Secondary).then(|| i.pointer.latest_pos()).flatten())?;
    on_canvas(response, up).then(|| to_layer(response, down))
}

/// Is a right press down on this canvas that may still be a right-click (`right_click_at`)?
pub fn right_click_pending(response: &Response) -> bool {
    response.ctx.data(|d| d.get_temp::<Option<Pos2>>(right_pressed_id(response))).flatten().is_some()
}

/// The right press now down was used by a gesture, so its release is no right-click.
pub fn right_press_used(response: &Response) {
    response.ctx.data_mut(|d| d.insert_temp::<Option<Pos2>>(right_pressed_id(response), None));
}

/// How far, in points, the pointer may stray from a right press while the button is down for its
/// release to still be a right-click: three times egui's click distance (6 points by default, so
/// 18). That is the "few pixels" a high-DPI mouse moves as its button goes down, or a slow frame's
/// move, at any display scale (points follow it), and well short of a deliberate drag, which is
/// no click: a long right drag opens nothing, as before #10183, and the selected Hand's pans.
fn right_click_reach(ctx: &egui::Context) -> f32 {
    3.0 * ctx.options(|o| o.input_options.max_click_dist)
}

/// A screen point in the coordinates of `response`'s layer (its `rect`'s).
fn to_layer(response: &Response, p: Pos2) -> Pos2 {
    response.ctx.layer_transform_from_global(response.layer_id).map_or(p, |t| t * p)
}

/// Is screen point `p` on the canvas itself, so that a press there is the canvas's: inside it,
/// under no popup or window, and on no widget drawn over it (a scrollbar). egui only hit-tests
/// where the pointer is at the end of the frame, which a slow frame can have moved away from the
/// press; this asks the same of last frame's widgets at `p`.
fn on_canvas(response: &Response, p: Pos2) -> bool {
    let ctx = &response.ctx;
    let q = to_layer(response, p);
    if !response.rect.contains(q) || ctx.layer_id_at(p).is_some_and(|l| l != response.layer_id) {
        return false;
    }
    // A layer's widgets are listed back to front: the last one there that senses the press is on top.
    let top = ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .get_layer(response.layer_id)
            .filter(|w| w.enabled && (w.sense.senses_click() || w.sense.senses_drag()) && w.interact_rect.contains(q))
            .last()
            .map(|w| w.id)
    });
    top.is_none_or(|id| id == response.id)
}

/// Temp-data key, one per canvas: where the right press now down (or released this frame) went
/// down, in screen points, when that was on this canvas and nothing has used it since; `None` once
/// it is no right-click.
fn right_pressed_id(response: &Response) -> egui::Id {
    response.id.with("canvas-right-pressed")
}

/// Keeps `right_pressed_id` for this frame: set on a right press (on the canvas or not), dropped
/// once the button is up again. A right-click is the right button on its own: a press made while
/// another button is down is none, and another button pressed while it is held uses it. Escape
/// pressed while it is held cancels it, and so does a move farther than `right_click_reach` from
/// the press: every pointer position this frame's events had while the button was down counts,
/// not just where the pointer ends up, so a drag out and back is no click either. The canvas runs
/// this every frame, panning or not, before the open brush mode menu skips the buttons, so a
/// press made while the menu is open is not mistaken for an older one.
pub fn track_right_press(response: &Response) {
    let id = right_pressed_id(response);
    let reach = right_click_reach(&response.ctx);
    let tracked = response.ctx.data(|d| d.get_temp::<Option<Pos2>>(id)).flatten();
    let (pressed, released, held, origin, others, escape, far) = response.ctx.input(|i| {
        let right_press = |e: &egui::Event| matches!(e, egui::Event::PointerButton { button: PointerButton::Secondary, pressed: true, .. });
        // Where the button went down. With the release in the same frame, the pointer is
        // already where it came up and egui has dropped the press origin: the event has it.
        let last_press = i.events.iter().rposition(right_press);
        let pressed_at = last_press.and_then(|k| i.events.get(k)).and_then(|e| match e {
            egui::Event::PointerButton { pos, .. } => Some(*pos),
            _ => None,
        });
        let pressed = i.pointer.button_pressed(PointerButton::Secondary);
        let origin = pressed_at.or_else(|| i.pointer.press_origin());
        // The pointer while the button was down this frame: after its press, up to its release.
        let start = if pressed { last_press.map_or(0, |k| k.saturating_add(1)) } else { 0 };
        let from = if pressed { origin } else { tracked };
        let mut far = false;
        for e in i.events.iter().skip(start) {
            let (p, up) = match e {
                egui::Event::PointerMoved(p) => (*p, false),
                egui::Event::PointerButton { pos, button: PointerButton::Secondary, pressed: false, .. } => (*pos, true),
                _ => continue,
            };
            far |= from.is_some_and(|o| o.distance(p) > reach);
            if up {
                break;
            }
        }
        let others = [PointerButton::Primary, PointerButton::Middle, PointerButton::Extra1, PointerButton::Extra2]
            .into_iter()
            .any(|b| i.pointer.button_down(b) || i.pointer.button_pressed(b));
        // The frame's own events: a shortcut, a closing popup or the HUD may already have
        // consumed this Escape from `events`, and it still cancels the press.
        let escape = i.raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. }));
        let b = PointerButton::Secondary;
        (pressed, i.pointer.button_released(b), i.pointer.secondary_down(), origin, others, escape, far)
    });
    if pressed {
        let at = origin.filter(|&p| !others && !escape && !far && on_canvas(response, p));
        response.ctx.data_mut(|d| {
            d.insert_temp(id, at);
            d.remove_temp::<ViewKey>(right_view_id(response));
        });
    } else if !released && !held {
        response.ctx.data_mut(|d| d.remove_temp::<Option<Pos2>>(id));
    } else if others || escape || far {
        right_press_used(response);
    }
}

/// A canvas's camera and place on screen, as bits: the zoom, the document point at the centre, the
/// rotation, the flip and the canvas rect.
type ViewKey = [u32; 9];

/// Temp-data key, one per canvas: the `ViewKey` the tracked right press went down in.
fn right_view_id(response: &Response) -> egui::Id {
    right_pressed_id(response).with("view")
}

/// Ties the tracked right press to the view it went down in. A scroll, a zoom or a turn of the
/// view while the button is held moves the image under the press, so its release is no
/// right-click: it would point at whatever is under the press's screen point now, not at what was
/// pressed. `view` and `flip` are this frame's camera; the canvas calls this once the frame's
/// navigation has moved it, before its menus read the click.
pub fn track_right_press_view(response: &Response, view: &crate::state::View, flip: bool) {
    let r = response.rect;
    let now: ViewKey =
        [view.zoom, view.center[0], view.center[1], view.rotation, f32::from(u8::from(flip)), r.min.x, r.min.y, r.max.x, r.max.y].map(f32::to_bits);
    let key = right_view_id(response);
    let tracked = response.ctx.data(|d| d.get_temp::<Option<Pos2>>(right_pressed_id(response))).flatten().is_some();
    let then = response.ctx.data_mut(|d| {
        if !tracked {
            d.remove_temp::<ViewKey>(key);
            return None;
        }
        let then = d.get_temp::<ViewKey>(key);
        if then.is_none() {
            d.insert_temp(key, now);
        }
        then
    });
    if then.is_some_and(|v| v != now) {
        right_press_used(response);
    }
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
    let mode_menu =
        crate::brush_mode_menu::opens_on(app, tool, crate::workspace_ui::sticky_mods(app, crate::brush_mode_menu::gesture_mods(&response.ctx, mods)));
    if mode_menu && !resizing && app.drag.is_none() {
        app.secondary_erase = false;
        if let Some(p) = right_click_at(response) {
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
    if right_start || resize_start || resizing {
        right_press_used(response);
    }
    let right_at = right_click_at(response);
    let right_click = right_at.is_some();
    if !erase
        && !layer_menu
        && !crate::color_hud::is_open(&app.ui)
        && right_click_picker(tool)
        && let Some(p) = right_at
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
    if crate::brush_mode_menu::opens_on(app, tool, mods) && app.drag.is_none() && app.brush_resize.is_none() {
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
    let opened_id = crate::brush_picker::opened_id();
    let Some([x, y]) = app.ui.brush_picker else {
        ctx.data_mut(|d| d.remove_temp::<crate::brush_picker::Opened>(opened_id));
        return;
    };
    if !has_brush_picker(app.ui.tool) {
        app.ui.brush_picker = None;
        ctx.data_mut(|d| d.remove_temp::<crate::brush_picker::Opened>(opened_id));
        return;
    }
    let mut opened = ctx.data_mut(|d| d.get_temp_mut_or_insert_with(opened_id, || crate::brush_picker::Opened::new(&app.session.tools.brush)).clone());
    // Escape and Enter close it once this frame's edits are in (a typed size applies), unless they
    // end a rename in its rename bar. Escape then reverts what was typed (#11816).
    let typing_done = app.ui.brush_picker_list.renaming.is_none();
    let escape = typing_done && ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let key_close = escape || (typing_done && ctx.input(|i| i.key_pressed(egui::Key::Enter)));
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
        let frame = egui::Frame::popup(ui.style()).show(ui, |ui| {
            // The Background Eraser has its own compact popup, without presets (#11910).
            if app.ui.tool == Tool::BackgroundEraser {
                crate::bg_eraser_popup::body(ui, &mut b, &mut app.ui.tool_options.bg_tolerance_control);
                Vec::new()
            } else {
                crate::brush_picker::body(ui, &mut b, &app.session.tools.presets, app.session.tools.current_preset.as_deref(), list, content)
            }
        });
        let picks = frame.inner;
        // The corner grip resizes the picker, and the size is remembered across opens.
        if let Some(new) = crate::brush_picker::resize_grip(ui, frame.response.rect, content) {
            app.ui.brush_picker_size = Some([new.x, new.y]);
        }
        crate::brush_panel::commit_gesture(app, ui.ctx(), &before, &b);
        if b != before {
            opened.typed(&b);
        }
        let closes = picks.iter().any(crate::brush_picker::Pick::closes);
        let picked =
            picks.iter().any(|p| matches!(p, crate::brush_picker::Pick::List(crate::brushes_tab::Action::Select(_) | crate::brushes_tab::Action::Choose(_))));
        crate::brush_picker::apply(app, ui.ctx(), picks);
        // A preset picked stays picked, Escape or not.
        if picked {
            opened = crate::brush_picker::Opened::new(&app.session.tools.brush);
        }
        closes
    });
    ctx.data_mut(|d| d.insert_temp(opened_id, opened.clone()));
    if escape {
        let now = app.session.tools.brush.clone();
        let back = opened.revert(&now);
        crate::brush_panel::commit_gesture(app, ctx, &now, &back);
    }
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
