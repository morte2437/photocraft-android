//! #12551: with Remove strokes waiting (Remove after each stroke off), a right-click on the canvas
//! opens nothing, neither the Brush Preset picker nor a menu, and the strokes, their tint and the
//! document stay as they were. With nothing waiting, and with the other painting tools, it still
//! opens the picker.

use egui::{PointerButton, Pos2};
use egui_kittest::Harness;
use serde_json::{Value, json};

use super::tests::{drag, harness, press};
use crate::PhotocraftApp;
use crate::control::{ControlRequest, Outcome, handle};
use crate::state::Tool;

/// The Remove tool with Remove after each stroke off, nothing painted yet.
fn remove_harness() -> Harness<'static, PhotocraftApp> {
    let mut h = harness(None);
    h.state_mut().ui.tool = Tool::Remove;
    h.state_mut().ui.tool_options.remove_after_each_stroke = false;
    h.run_steps(2);
    h
}

/// One Remove stroke painted with the mouse, waiting.
fn waiting() -> Harness<'static, PhotocraftApp> {
    let mut h = remove_harness();
    drag(&mut h, PointerButton::Primary);
    assert_eq!(h.state().remove_pending.len(), 1, "one stroke waits");
    assert!(h.state().remove_trail.is_some(), "tinted");
    h
}

/// The waiting strokes, their tint along the stroke's row, the history and the journal.
fn snapshot(app: &PhotocraftApp) -> (Vec<Value>, Option<Vec<u8>>, usize, usize) {
    let st = app.session.active().unwrap();
    let row = st.doc.size.height as usize / 2;
    let tint = app.remove_trail.as_ref().map(|t| (0..st.doc.size.width as usize).map(|x| t.coverage(x, row)).collect());
    (app.remove_pending.clone(), tint, st.history.past_len(), app.session.journal.len())
}

fn nothing_open(app: &PhotocraftApp) -> bool {
    app.ui.brush_picker.is_none() && app.ui.brush_mode_menu.is_none() && app.ui.canvas_tool_menu.is_none() && !crate::color_hud::is_open(&app.ui)
}

fn mouse_right_click(h: &mut Harness<'static, PhotocraftApp>) -> Pos2 {
    let c = h.state().last_canvas_rect.center();
    h.event(egui::Event::PointerMoved(c));
    h.run_steps(1);
    press(h, c, PointerButton::Secondary, true);
    press(h, c, PointerButton::Secondary, false);
    c
}

/// `ui.pointer` with `"button": "secondary"`: a down and an up on the image's centre.
fn pointer_right_click(h: &mut Harness<'static, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    let events = json!([{"kind": "down", "x": 200, "y": 150}, {"kind": "up", "x": 200, "y": 150}]);
    let (req, _rx) = ControlRequest::new("ui.pointer", json!({"button": "secondary", "events": events}));
    assert!(matches!(handle(h.state_mut(), &ctx, &req), Outcome::Done(v) if v["ok"] == true));
    h.run_steps(2);
}

#[test]
fn a_mouse_right_click_with_strokes_waiting_opens_nothing_and_keeps_them() {
    let mut h = waiting();
    let before = snapshot(h.state());
    mouse_right_click(&mut h);
    assert!(nothing_open(h.state()), "no picker, no menu");
    assert_eq!(snapshot(h.state()), before, "the strokes, their tint and the document stay");
}

#[test]
fn a_control_channel_right_click_with_strokes_waiting_opens_nothing_and_keeps_them() {
    let mut h = waiting();
    let before = snapshot(h.state());
    pointer_right_click(&mut h);
    assert!(nothing_open(h.state()), "no picker, no menu");
    assert_eq!(snapshot(h.state()), before, "the strokes, their tint and the document stay");
}

/// A key press through the app's shortcut handling: ↩ applies the waiting strokes, Esc drops them.
fn key(h: &mut Harness<'static, PhotocraftApp>, key: egui::Key) {
    let ctx = egui::Context::default();
    let event = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    ctx.begin_pass(egui::RawInput { events: vec![event], ..Default::default() });
    crate::shortcuts::handle(h.state_mut(), &ctx);
    ctx.end_pass().textures_delta.clear();
    h.run_steps(2);
}

/// The mouse opens the picker at the pointer, Escape closes it, and `ui.pointer` opens it again.
fn assert_picker_opens(h: &mut Harness<'static, PhotocraftApp>, case: &str) {
    let c = mouse_right_click(h);
    assert_eq!(h.state().ui.brush_picker, Some([c.x, c.y]), "{case}: the mouse, at the pointer");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.brush_picker.is_none(), "{case}: closed");
    pointer_right_click(h);
    assert!(h.state().ui.brush_picker.is_some(), "{case}: ui.pointer");
}

#[test]
fn with_nothing_waiting_the_remove_tool_still_opens_the_picker() {
    assert_picker_opens(&mut remove_harness(), "nothing painted");
    for (k, case) in [(egui::Key::Enter, "applied"), (egui::Key::Escape, "dropped")] {
        let mut h = waiting();
        key(&mut h, k);
        assert!(h.state().remove_pending.is_empty(), "{case}: nothing waits");
        assert_picker_opens(&mut h, case);
    }
}

#[test]
fn other_painting_tools_keep_opening_the_picker() {
    for tool in [Tool::Brush, Tool::Pencil, Tool::CloneStamp, Tool::SpotHealing] {
        let mut h = harness(None);
        h.state_mut().ui.tool = tool;
        h.run_steps(2);
        assert_picker_opens(&mut h, &format!("{tool:?}"));
    }
}
