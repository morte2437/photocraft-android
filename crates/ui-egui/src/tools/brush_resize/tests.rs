use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use super::*;
use crate::canvas::tool_event;

const CTRL_ALT: Modifiers = Modifiers { alt: true, ctrl: true, shift: false, mac_cmd: false, command: false };

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200, "background": "transparent"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 40, "hardness": 0.2}})).unwrap();
    app.ui.tool = tool;
    app
}

fn history(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn mapping_is_bounded() {
    assert_eq!(resized((40.0, 0.5), 10.0, 0.0, 1.0), (60.0, 0.5), "the edge follows the pointer");
    assert_eq!(resized((40.0, 0.5), 0.0, 50.0, 2.0), (40.0, 1.0), "down is harder, in screen points");
    assert_eq!(resized((40.0, 0.5), -1e9, -1e9, 1.0), (1.0, 0.0));
    assert_eq!(resized((40.0, 0.5), 1e9, 1e9, 1.0), (MAX_BRUSH_SIZE, 1.0));
    assert_eq!(resized((40.0, 0.5), f64::NAN, f64::INFINITY, f32::NAN), (40.0, 0.5));
    assert!(is_gesture(CTRL_ALT) && !is_gesture(Modifiers::ALT) && !is_gesture(Modifiers::CTRL) && !is_gesture(Modifiers::SHIFT));
}

#[test]
fn ctrl_alt_drag_resizes_every_brush_tool_without_painting_or_history() {
    let tools: Vec<Tool> = Tool::ALL.into_iter().filter(|t| applies(*t)).collect();
    assert!(tools.len() > 10 && tools.contains(&Tool::Brush) && tools.contains(&Tool::Eraser) && tools.contains(&Tool::CloneStamp));
    for tool in tools {
        let mut app = app(tool);
        let (h0, j0) = (history(&app), app.session.journal.len());
        tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, CTRL_ALT);
        assert!(app.brush_resize.is_some() && app.drag.is_none(), "{tool:?}");
        // Right grows the size.
        tool_event(&mut app, ToolEvent::Move { x: 130.0, y: 100.0, pressure: 1.0 }, CTRL_ALT);
        assert_eq!(app.session.tools.brush.size, 100.0, "{tool:?}");
        // Down raises the hardness (at 100 %, 200 points take it from 0 to 100 %).
        tool_event(&mut app, ToolEvent::Move { x: 130.0, y: 160.0, pressure: 1.0 }, CTRL_ALT);
        assert_eq!(app.session.tools.brush.hardness, 0.5, "{tool:?}");
        // Letting go of the keys mid-drag keeps resizing until the button is released.
        tool_event(&mut app, ToolEvent::Up { x: 120.0, y: 180.0 }, Modifiers::NONE);
        let b = &app.session.tools.brush;
        assert_eq!((b.size, b.hardness), (80.0, 0.6), "{tool:?}");
        assert!(app.brush_resize.is_none() && app.drag.is_none());
        // No history step, no paint: one coalesced tools.setBrush in the journal.
        assert_eq!(history(&app), h0, "{tool:?}");
        let new: Vec<&str> = app.session.journal[j0..].iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(new, ["tools.setBrush"], "{tool:?}");
        assert_eq!(app.session.journal.last().unwrap().1["brush"], json!({"size": 80.0f32, "hardness": 0.6f32}));
    }
}

#[test]
fn other_tools_and_strokes_in_progress_are_untouched() {
    let mut a = app(Tool::Move);
    tool_event(&mut a, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, CTRL_ALT);
    assert!(a.brush_resize.is_none());
    tool_event(&mut a, ToolEvent::Up { x: 10.0, y: 10.0 }, CTRL_ALT);
    let mut a = app(Tool::Brush);
    tool_event(&mut a, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut a, ToolEvent::Move { x: 50.0, y: 10.0, pressure: 1.0 }, CTRL_ALT);
    assert!(a.brush_resize.is_none(), "a stroke in progress isn't taken over");
    tool_event(&mut a, ToolEvent::Up { x: 50.0, y: 10.0 }, CTRL_ALT);
    assert!(a.session.journal.iter().any(|(id, _)| id == "paint.stroke"));
    assert_eq!(a.session.tools.brush.size, 40.0);
}

#[test]
fn canvas_drag_shows_the_circle_and_readout() {
    let mut a = app(Tool::Brush);
    a.sync_views();
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        a,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    h.run_steps(4);
    let c = h.state().last_canvas_rect.center();
    let press = |h: &mut Harness<'static, PhotocraftApp>, pos: Pos2, pressed: bool| {
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: CTRL_ALT });
        h.run();
    };
    h.event(egui::Event::ModifiersChanged(CTRL_ALT));
    h.event(egui::Event::PointerMoved(c));
    h.run();
    press(&mut h, c, true);
    let size0 = h.state().session.tools.brush.size;
    let undo0 = history(h.state());
    for k in 1..=6 {
        h.event(egui::Event::PointerMoved(c + vec2(10.0 * k as f32, 8.0 * k as f32)));
        h.run();
    }
    assert!(h.state().brush_resize.is_some());
    assert!(h.state().session.tools.brush.size > size0, "dragging right grew the brush");
    assert!(h.state().session.tools.brush.hardness > 0.2, "dragging down hardened it");
    assert!(h.ctx.memory(|m| m.area_rect(egui::Id::unique("brush-resize-readout"))).is_some(), "the readout shows mid-drag");
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "the pointer hides");
    press(&mut h, c + vec2(60.0, 48.0), false);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    assert!(h.state().brush_resize.is_none());
    assert!(!h.state().session.journal.iter().any(|(id, _)| id == "paint.stroke"), "nothing painted");
    assert_eq!(history(h.state()), undo0, "no undo step");
}

fn canvas(a: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        a,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    h.run_steps(4);
    h
}

fn button(h: &mut Harness<'static, PhotocraftApp>, pos: Pos2, button: PointerButton, pressed: bool, modifiers: Modifiers) {
    h.event(egui::Event::PointerButton { pos, button, pressed, modifiers });
    h.run_steps(2);
}

/// Alt+right-drag from the canvas centre, six steps of `step` screen points.
fn alt_right_drag(h: &mut Harness<'static, PhotocraftApp>, step: egui::Vec2) -> Pos2 {
    let c = h.state().last_canvas_rect.center();
    h.event(egui::Event::ModifiersChanged(Modifiers::ALT));
    h.event(egui::Event::PointerMoved(c));
    h.run_steps(1);
    button(h, c, PointerButton::Secondary, true, Modifiers::ALT);
    let mut p = c;
    for _ in 0..6 {
        p += step;
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(1);
    }
    p
}

#[test]
fn alt_right_drag_resizes_with_the_readout_and_never_paints() {
    for erase_pref in [false, true] {
        let mut a = app(Tool::Brush);
        if erase_pref {
            a.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "erase"})).unwrap();
        }
        a.sync_views();
        let mut h = canvas(a);
        let (undo0, size0) = (history(h.state()), h.state().session.tools.brush.size);
        let end = alt_right_drag(&mut h, vec2(10.0, 8.0));
        let r = h.state().brush_resize.expect("resizing");
        assert!(r.secondary);
        assert!(h.state().session.tools.brush.size > size0, "right grows the brush");
        assert!(h.state().session.tools.brush.hardness > 0.2, "down hardens it");
        assert!(h.ctx.memory(|m| m.area_rect(egui::Id::unique("brush-resize-readout"))).is_some(), "the readout shows");
        assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None);
        button(&mut h, end, PointerButton::Secondary, false, Modifiers::ALT);
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(2);
        let app = h.state();
        assert!(app.brush_resize.is_none() && !app.brush_resize_armed && app.ui.brush_picker.is_none());
        assert!(!app.session.journal.iter().any(|(id, _)| id == "paint.stroke"), "nothing painted or erased (erase pref {erase_pref})");
        assert_eq!(history(app), undo0, "no undo step");
        let sets = app.session.journal.iter().filter(|(id, _)| id == "tools.setBrush").count();
        assert_eq!(sets, 2, "the setup call plus one coalesced resize");
    }
}

/// Alt-right dragging changes tip size without loading paint or touching document history.
#[test]
fn mixer_alt_right_drag_resizes_without_sampling_or_painting() {
    let mut a = app(Tool::MixerBrush);
    let before = a.session.tools.mixer.clone();
    a.sync_views();
    let mut h = canvas(a);
    let (undo0, size0) = (history(h.state()), h.state().session.tools.brush.size);
    let end = alt_right_drag(&mut h, vec2(10.0, 8.0));
    assert!(h.state().brush_resize.is_some_and(|resize| resize.secondary));
    assert!(h.state().session.tools.brush.size > size0);
    button(&mut h, end, PointerButton::Secondary, false, Modifiers::ALT);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    let a = h.state();
    assert!(a.brush_resize.is_none() && !a.brush_resize_armed);
    assert_eq!(a.session.tools.mixer, before);
    assert_eq!(history(a), undo0);
    assert!(!a.session.journal.iter().any(|(id, _)| id == "brush.mixer.sample" || id == "paint.mixerBrush"));
}

#[test]
fn plain_right_click_still_opens_the_picker() {
    let mut a = app(Tool::Brush);
    a.sync_views();
    let mut h = canvas(a);
    let c = h.state().last_canvas_rect.center();
    h.event(egui::Event::PointerMoved(c));
    h.run_steps(1);
    button(&mut h, c, PointerButton::Secondary, true, Modifiers::NONE);
    button(&mut h, c, PointerButton::Secondary, false, Modifiers::NONE);
    assert!(h.state().ui.brush_picker.is_some() && h.state().brush_resize.is_none());
    // ⌥ alone on the left button stays the eyedropper, not a resize.
    assert!(!is_gesture(Modifiers::ALT) && is_right_gesture(Modifiers::ALT));
}

#[test]
fn a_missed_release_never_leaves_the_resize_stuck() {
    let mut a = app(Tool::Brush);
    a.sync_views();
    let mut h = canvas(a);
    // An Alt+right resize whose button-up never reached the canvas (released over another
    // window, focus lost): the next canvas frame sees the right button up and ends it.
    h.state_mut().brush_resize_armed = true;
    tool_event(h.state_mut(), ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(h.state().brush_resize.is_some_and(|r| r.secondary));
    h.run_steps(2);
    assert!(h.state().brush_resize.is_none(), "released when the right button is up");
    // A plain left drag paints again.
    let c = h.state().last_canvas_rect.center();
    h.event(egui::Event::WindowFocused(true));
    h.event(egui::Event::PointerMoved(c));
    h.run_steps(1);
    button(&mut h, c, PointerButton::Primary, true, Modifiers::NONE);
    for k in 1..=5 {
        h.event(egui::Event::PointerMoved(c + vec2(12.0 * k as f32, 0.0)));
        h.run_steps(1);
    }
    button(&mut h, c + vec2(60.0, 0.0), PointerButton::Primary, false, Modifiers::NONE);
    assert!(h.state().session.journal.iter().any(|(id, _)| id == "paint.stroke"), "the stroke paints");
}

#[test]
fn an_armed_press_consumed_by_another_handler_disarms() {
    // Free Transform is active: it takes every press, including an armed Alt+right one.
    let mut a = app(Tool::Brush);
    a.run("layer.new.layer", json!({})).unwrap();
    a.run("edit.fill", json!({"color": [1.0, 0.0, 0.0, 1.0]})).unwrap();
    let ctx = egui::Context::default();
    crate::transform_tool::begin(&mut a, &ctx).unwrap();
    assert!(a.ui.transform.is_some());
    a.brush_resize_armed = true;
    tool_event(&mut a, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut a, ToolEvent::Up { x: 50.0, y: 50.0 }, Modifiers::ALT);
    assert!(!a.brush_resize_armed, "the armed flag never outlives its press");
    assert!(a.brush_resize.is_none());
    crate::transform_tool::cancel(&mut a);
    assert!(a.ui.transform.is_none());
    // The next plain press paints rather than resizing.
    tool_event(&mut a, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(a.brush_resize.is_none());
    tool_event(&mut a, ToolEvent::Move { x: 60.0, y: 20.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut a, ToolEvent::Up { x: 60.0, y: 20.0 }, Modifiers::NONE);
    assert!(a.session.journal.iter().any(|(id, _)| id == "paint.stroke"));
}

#[test]
fn agents_resize_with_an_alt_right_pointer_drag() {
    let mut a = app(Tool::Brush);
    let ctx = egui::Context::default();
    let events = json!([{"kind": "down", "x": 100, "y": 100}, {"kind": "move", "x": 120, "y": 100}, {"kind": "up", "x": 130, "y": 100}]);
    let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"button": "right", "alt": true, "events": events}));
    let _ = crate::control::handle(&mut a, &ctx, &req);
    assert_eq!(a.session.tools.brush.size, 100.0, "40 px + 2 × 30");
    assert!(a.brush_resize.is_none() && !a.brush_resize_armed);
    assert!(!a.session.journal.iter().any(|(id, _)| id == "paint.stroke"));
    // Without Alt, a right press with the Brush opens the picker and paints nothing.
    let events = json!([{"kind": "down", "x": 10, "y": 10}, {"kind": "up", "x": 10, "y": 10}]);
    let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"button": "right", "events": events}));
    let _ = crate::control::handle(&mut a, &ctx, &req);
    assert_eq!(a.session.tools.brush.size, 100.0);
    assert!(a.ui.brush_picker.is_some());
}

/// Preferences ▸ Tools ▸ Vary Round Brush Hardness on HUD (on by default): off, the HUD's
/// vertical movement takes the brush opacity instead of its hardness.
#[test]
fn vary_round_brush_hardness_on_hud_switches_the_vertical_target() {
    for vary in [true, false] {
        let mut app = app(Tool::Brush);
        app.run("tools.setBrush", json!({"brush": {"opacity": 0.5}})).unwrap();
        if !vary {
            app.run("prefs.set", json!({"path": "tools.varyRoundBrushHardnessOnHud", "value": false})).unwrap();
        }
        let (h0, o0) = (app.session.tools.brush.hardness, app.session.tools.brush.opacity);
        tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, CTRL_ALT);
        tool_event(&mut app, ToolEvent::Move { x: 100.0, y: 140.0, pressure: 1.0 }, CTRL_ALT);
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 140.0 }, CTRL_ALT);
        let b = &app.session.tools.brush;
        if vary {
            assert_eq!((b.hardness, b.opacity), (h0 + 0.2, o0), "hardness moves with the drag");
            assert_eq!(app.session.journal.last().unwrap().1["brush"], json!({"size": 40.0f32, "hardness": 0.4f32}));
        } else {
            assert_eq!((b.hardness, b.opacity), (h0, o0 + 0.2), "opacity moves instead");
            assert_eq!(app.session.journal.last().unwrap().1["brush"], json!({"size": 40.0f32, "opacity": 0.7f32}));
        }
    }
}

/// One Control+Alt drag by `(dx, dy)` document pixels from (100, 100) in a view turned by
/// `rotation` degrees (flipped with `flip`): the resulting (size, hardness).
fn drag_in_view(rotation: f32, flip: bool, dx: f64, dy: f64) -> (f32, f32) {
    let mut app = app(Tool::Brush);
    app.sync_views();
    app.ui.views[0].rotation = rotation;
    app.ui.view.flip_horizontal = flip;
    tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, CTRL_ALT);
    tool_event(&mut app, ToolEvent::Up { x: 100.0 + dx, y: 100.0 + dy }, CTRL_ALT);
    let b = &app.session.tools.brush;
    (b.size, b.hardness)
}

/// #4228: the HUD is screen-relative, as in Photoshop: horizontal on screen is the size and
/// vertical the hardness however the view is rotated or flipped.
#[test]
fn a_rotated_view_keeps_horizontal_for_size_and_vertical_for_hardness() {
    // Unrotated, with and without View › Flip Horizontal: unchanged.
    assert_eq!(drag_in_view(0.0, false, 30.0, 0.0), (100.0, 0.2), "right grows");
    assert_eq!(drag_in_view(0.0, false, 0.0, 40.0), (40.0, 0.4), "down hardens");
    assert_eq!(drag_in_view(0.0, true, -30.0, 0.0), (100.0, 0.2), "flipped: screen right grows");
    assert_eq!(drag_in_view(0.0, true, 0.0, 40.0), (40.0, 0.4), "flipped: down hardens");
    // 90° clockwise: document +y points screen-left and document +x screen-down.
    assert_eq!(drag_in_view(90.0, false, 0.0, 10.0), (20.0, 0.2), "screen-left shrinks, the hardness stays");
    assert_eq!(drag_in_view(90.0, false, 0.0, -30.0), (100.0, 0.2), "screen-right grows");
    assert_eq!(drag_in_view(90.0, false, 40.0, 0.0), (40.0, 0.4), "screen-down hardens, the size stays");
    // 180°: document -x is screen right.
    assert_eq!(drag_in_view(180.0, false, -30.0, 0.0), (100.0, 0.2), "screen right still grows");
    assert_eq!(drag_in_view(180.0, false, 0.0, -40.0), (40.0, 0.4), "screen down still hardens");
    // 90° and flipped: document +y points screen-right.
    assert_eq!(drag_in_view(90.0, true, 0.0, 30.0), (100.0, 0.2), "rotated and flipped: screen right grows");
}

/// Preferences ▸ Cursors ▸ Brush Preview colour tints the HUD's tip preview (red by default).
#[test]
fn the_tip_preview_uses_the_brush_preview_colour() {
    let mut app = app(Tool::Brush);
    assert_eq!(preview_tint(&app), Color32::from_rgba_unmultiplied(255, 0, 0, 110));
    app.run("prefs.set", json!({"path": "cursors.brushPreviewColor", "value": "#00ff80"})).unwrap();
    assert_eq!(preview_tint(&app), Color32::from_rgba_unmultiplied(0, 255, 128, 110));
    app.session.prefs.edit(|p| p.cursors.brush_preview_color = "nonsense".into());
    assert_eq!(preview_tint(&app), Color32::from_rgba_unmultiplied(255, 0, 0, 110), "unreadable values keep the red");
}

#[test]
fn stylus_barrel_button_resizes_with_alt_and_never_samples_color() {
    let mut a = app(Tool::Brush);
    a.services.right_button_down = Some(Box::new(|| true));
    a.sync_views();
    let mut h = canvas(a);
    let size0 = h.state().session.tools.brush.size;
    let c = h.state().last_canvas_rect.center();
    h.event(egui::Event::ModifiersChanged(Modifiers::ALT));
    h.event(egui::Event::PointerMoved(c));
    h.run_steps(1);
    button(&mut h, c, PointerButton::Primary, true, Modifiers::ALT);
    let mut p = c;
    for _ in 0..6 {
        p += vec2(10.0, 8.0);
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(1);
    }
    let r = h.state().brush_resize.expect("resizing via stylus barrel button");
    assert!(r.secondary);
    assert!(h.state().session.tools.brush.size > size0, "right grows the brush");
    assert!(h.state().session.tools.brush.hardness > 0.2, "down hardens the brush");
    assert!(!h.state().alt_sampling, "eyedropper never triggered");
    assert!(!h.state().session.journal.iter().any(|(id, _)| id == "paint.stroke"), "nothing painted");
    button(&mut h, p, PointerButton::Primary, false, Modifiers::ALT);
    h.run_steps(2);
    assert!(h.state().brush_resize.is_none());
}
