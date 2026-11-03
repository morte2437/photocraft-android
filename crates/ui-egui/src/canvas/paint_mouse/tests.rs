use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use super::*;
use crate::canvas::{ToolEvent, tool_event};

fn harness(prefs: Option<&str>) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 20, "hardness": 1.0}})).unwrap();
    if let Some(v) = prefs {
        app.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": v})).unwrap();
    }
    app.ui.tool = Tool::Brush;
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h
}

fn press(h: &mut Harness<'static, PhotocraftApp>, pos: Pos2, button: PointerButton, pressed: bool) {
    h.event(egui::Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// Drag across the canvas centre with `button`; returns the document points it covered.
fn drag(h: &mut Harness<'static, PhotocraftApp>, button: PointerButton) -> (Pos2, Pos2) {
    let c = h.state().last_canvas_rect.center();
    let (a, b) = (c - vec2(80.0, 0.0), c + vec2(80.0, 0.0));
    h.event(egui::Event::PointerMoved(a));
    h.run_steps(1);
    press(h, a, button, true);
    for i in 1..=8 {
        h.event(egui::Event::PointerMoved(a + (b - a) * (i as f32 / 8.0)));
        h.run_steps(1);
    }
    press(h, b, button, false);
    (a, b)
}

fn alpha_at(h: &Harness<'static, PhotocraftApp>, screen: Pos2) -> f32 {
    let app = h.state();
    let v = app.ui.views[0].clone();
    let r = app.last_canvas_rect;
    let d = (screen - r.center()) / v.zoom;
    let (x, y) = ((d.x + v.center[0]) as i32, (d.y + v.center[1]) as i32);
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)[3]
}

fn strokes(h: &Harness<'static, PhotocraftApp>) -> Vec<serde_json::Value> {
    h.state().session.journal.iter().filter(|(id, _)| id == "paint.stroke").map(|(_, p)| p.clone()).collect()
}

fn mode_right_click(h: &mut Harness<'static, PhotocraftApp>, mods: Modifiers) {
    let p = h.state().last_canvas_rect.center();
    h.event(egui::Event::ModifiersChanged(mods));
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: mods });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: mods });
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

#[test]
fn shift_right_mode_picker_selects_journals_and_clear_erases() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness(Some("erase"));
    let history = h.state().session.active().unwrap().history.past_len();
    let journal = h.state().session.journal.len();
    mode_right_click(&mut h, Modifiers::SHIFT);
    assert!(h.state().ui.brush_mode_menu.is_some());
    assert!(h.state().ui.brush_picker.is_none() && h.state().drag.is_none());
    assert_eq!(h.state().session.journal.len(), journal);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    h.get_by_label("Multiply").click();
    h.run_steps(2);
    assert!(h.state().ui.brush_mode_menu.is_none());
    assert_eq!(h.state().session.tools.brush.mode, photocraft_doc::BlendMode::Multiply);
    assert_eq!(h.state().session.journal.last().unwrap(), &("tools.setBrush".into(), json!({"brush": {"mode": "Multiply", "behind": false}})));
    let (a, b) = drag(&mut h, PointerButton::Primary);
    let mid = a + (b - a) * 0.5;
    assert!(alpha_at(&h, mid) > 0.9);
    mode_right_click(&mut h, Modifiers::SHIFT);
    h.get_by_label("Clear").click();
    h.run_steps(2);
    assert!(h.state().ui.tool_options.brush_clear);
    assert_eq!(h.state().session.tools.brush.mode, photocraft_doc::BlendMode::Multiply, "Clear keeps the underlying mode, as the options bar does");
    drag(&mut h, PointerButton::Primary);
    assert_eq!(alpha_at(&h, mid), 0.0);
    assert!(strokes(&h).last().unwrap()["erase"].as_bool().unwrap());
}

#[test]
fn mode_picker_outside_dismissal_does_not_paint_or_erase() {
    for button in [PointerButton::Primary, PointerButton::Secondary] {
        let mut h = harness(Some("erase"));
        drag(&mut h, PointerButton::Primary);
        let journal = h.state().session.journal.len();
        let history = h.state().session.active().unwrap().history.past_len();
        mode_right_click(&mut h, Modifiers::SHIFT);
        // A full outside drag must only dismiss, including its release click.
        drag(&mut h, button);
        assert!(h.state().ui.brush_mode_menu.is_none());
        assert_eq!(h.state().session.journal.len(), journal, "dismissal must not paint/erase with {button:?}");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn mode_control_can_pick_a_row_with_ui_click() {
    use crate::control::{ControlRequest, Outcome, handle};
    use egui_kittest::kittest::Queryable;
    let mut h = harness(Some("erase"));
    let ctx = h.ctx.clone();
    let before = h.state().session.journal.len();
    let (req, _) = ControlRequest::new(
        "ui.pointer",
        json!({"button": "right", "shift": true,
        "events": [{"kind": "down", "x": 200, "y": 150}, {"kind": "up", "x": 200, "y": 150}]}),
    );
    assert!(matches!(handle(h.state_mut(), &ctx, &req), Outcome::Done(_)));
    h.run_steps(2);
    assert!(h.state().ui.brush_mode_menu.is_some());
    assert_eq!(h.state().session.journal.len(), before);
    let at = h.get_by_label("Darken").rect().center();
    let (req, _) = ControlRequest::new("ui.click", json!({"x": at.x, "y": at.y}));
    assert!(matches!(handle(h.state_mut(), &ctx, &req), Outcome::AfterInput));
    // The app injects these queued control events into its next frame's input.
    let events = std::mem::take(&mut h.state_mut().synthetic);
    for event in events {
        h.event(event);
    }
    h.run_steps(2);
    assert!(h.state().ui.brush_mode_menu.is_none());
    assert_eq!(h.state().session.tools.brush.mode, photocraft_doc::BlendMode::Darken);
    assert_eq!(h.state().session.journal.last().unwrap(), &("tools.setBrush".into(), json!({"brush": {"mode": "Darken", "behind": false}})));
}

#[test]
fn shift_right_mode_picker_cancels_and_other_gestures_keep_their_routes() {
    let mut h = harness(None);
    let journal = h.state().session.journal.len();
    mode_right_click(&mut h, Modifiers::SHIFT);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.brush_mode_menu.is_none());
    mode_right_click(&mut h, Modifiers::SHIFT);
    let outside = h.state().last_canvas_rect.min + vec2(2.0, 2.0);
    h.event(egui::Event::PointerMoved(outside));
    press(&mut h, outside, PointerButton::Secondary, true);
    press(&mut h, outside, PointerButton::Secondary, false);
    assert!(h.state().ui.brush_mode_menu.is_none());
    mode_right_click(&mut h, Modifiers::SHIFT);
    h.state_mut().ui.tool = Tool::Pencil;
    h.run_steps(2);
    assert!(h.state().ui.brush_mode_menu.is_none());
    // Spot Healing has no Shift+right-click menu here: its right-click keeps the picker.
    h.state_mut().ui.tool = Tool::SpotHealing;
    h.run_steps(1);
    mode_right_click(&mut h, Modifiers::SHIFT);
    assert!(h.state().ui.brush_mode_menu.is_none() && h.state().ui.brush_picker.is_some());
    h.state_mut().ui.brush_picker = None;
    h.state_mut().ui.tool = Tool::Brush;
    mode_right_click(&mut h, Modifiers::NONE);
    assert!(h.state().ui.brush_picker.is_some() && h.state().ui.brush_mode_menu.is_none());
    assert_eq!(h.state().session.journal.len(), journal, "opening/cancelling never changes brush or pixels");
}

#[test]
fn escape_dismissal_consumes_a_held_secondary_release() {
    for release_shift in [false, true] {
        let mut h = harness(Some("erase"));
        let journal = h.state().session.journal.len();
        let history = h.state().session.active().unwrap().history.past_len();
        let p = h.state().last_canvas_rect.center();
        h.event(egui::Event::ModifiersChanged(Modifiers::SHIFT));
        h.event(egui::Event::PointerMoved(p));
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::SHIFT });
        h.run_steps(1);
        // A quick second press can arrive in the frame that opens the menu for the first
        // click's release, leaving the secondary button held while the picker is visible.
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::SHIFT });
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::SHIFT });
        h.run_steps(1);
        assert!(h.state().ui.brush_mode_menu.is_some());
        assert!(h.ctx.input(|i| i.pointer.secondary_down()));
        h.key_press(egui::Key::Escape);
        h.run_steps(1);
        assert!(h.state().ui.brush_mode_menu.is_none());
        let mods = if release_shift { Modifiers::NONE } else { Modifiers::SHIFT };
        h.event(egui::Event::ModifiersChanged(mods));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: mods });
        h.run_steps(2);
        assert!(h.state().ui.brush_mode_menu.is_none(), "Escape must consume the pending release");
        assert!(h.state().ui.brush_picker.is_none() && h.state().drag.is_none() && !h.state().secondary_erase);
        assert_eq!(h.state().session.journal.len(), journal);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
        mode_right_click(&mut h, Modifiers::SHIFT);
        assert!(h.state().ui.brush_mode_menu.is_some(), "the next gesture must still work");
    }
}

#[test]
fn shift_released_before_the_right_button_still_opens_the_mode_picker() {
    // egui reports `secondary_clicked` on release. With Erase on, a click that lost Shift a
    // frame early must open the menu, not erase a dab.
    let mut h = harness(Some("erase"));
    let history = h.state().session.active().unwrap().history.past_len();
    let journal = h.state().session.journal.len();
    let p = h.state().last_canvas_rect.center();
    h.event(egui::Event::ModifiersChanged(Modifiers::SHIFT));
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::SHIFT });
    h.run_steps(1);
    // Shift goes up in a frame of its own while the right button is still down.
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().ui.brush_mode_menu.is_some(), "the press decided the gesture");
    assert!(h.state().drag.is_none() && !h.state().secondary_erase);
    assert_eq!(h.state().session.journal.len(), journal, "no erase dab");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
}

#[test]
fn left_drag_paints() {
    let mut h = harness(None);
    let (a, b) = drag(&mut h, PointerButton::Primary);
    assert_eq!(strokes(&h).len(), 1);
    assert!(alpha_at(&h, a + (b - a) * 0.5) > 0.9);
}

#[test]
fn shift_click_connects_to_the_previous_brush_stroke() {
    use egui::{Event, Modifiers};

    let mut h = harness(None);
    let (_, end) = drag(&mut h, PointerButton::Primary);
    let previous = h
        .state()
        .session
        .journal
        .iter()
        .rev()
        .find(|(id, _)| id == "paint.stroke")
        .map(|(_, p)| p["points"].as_array().unwrap().last().unwrap().clone())
        .unwrap();
    let target = end + vec2(120.0, 80.0);
    h.event(Event::PointerMoved(target));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed, modifiers: Modifiers::SHIFT });
        h.run_steps(2);
    }

    let strokes = strokes(&h);
    assert_eq!(strokes.len(), 2, "a Shift-click commits one connected stroke");
    let points = strokes[1]["points"].as_array().unwrap();
    // The previous endpoint, then the clicked one. While the button is held, smoothing that is
    // still catching up repeats the clicked point with later times.
    assert_eq!(points[0][0], previous[0]);
    assert_eq!(points[0][1], previous[1]);
    let clicked = &points[1];
    assert!(points[1..].iter().all(|p| p[0] == clicked[0] && p[1] == clicked[1]), "{points:?}");
    assert!(alpha_at(&h, end + vec2(60.0, 40.0)) > 0.9, "the segment between the strokes is painted");
}

#[test]
fn right_click_opens_the_brush_picker_and_never_paints() {
    let mut h = harness(None);
    let undo = h.state().session.active().unwrap().history.past_len();
    let c = h.state().last_canvas_rect.center();
    h.event(egui::Event::PointerMoved(c));
    h.run_steps(1);
    press(&mut h, c, PointerButton::Secondary, true);
    press(&mut h, c, PointerButton::Secondary, false);
    assert_eq!(h.state().ui.brush_picker, Some([c.x, c.y]), "opened at the pointer");
    assert!(strokes(&h).is_empty());
    // The picker shows over the canvas at the pointer, moved up just enough to stay on screen.
    h.run_steps(2);
    let shown = h.ctx.memory(|m| m.area_rect(egui::Id::unique("canvas-brush-picker"))).expect("picker shown");
    let screen = h.ctx.content_rect();
    assert!((shown.min.x - c.x).abs() < 1.0 && shown.min.y <= c.y + 1.0, "{shown:?}");
    assert!(screen.contains_rect(shown.shrink(0.5)) && shown.width() > 250.0 && shown.height() > 150.0, "{shown:?}");
    // It is the real preset library, not a fixed set of round tips (#258).
    let names: Vec<String> = h.state().session.tools.presets.iter().take(3).map(|p| p.name.clone()).collect();
    {
        use egui_kittest::kittest::Queryable;
        for n in &names {
            assert!(h.query_by_label(n).is_some(), "{n} listed");
        }
    }
    // A right drag doesn't paint either.
    drag(&mut h, PointerButton::Secondary);
    assert!(strokes(&h).is_empty(), "right-drag never paints with the brush picker preference");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), undo, "no edit");
    // Escape closes it.
    h.state_mut().ui.brush_picker = Some([c.x, c.y]);
    h.run_steps(1);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().ui.brush_picker, None);
}

#[test]
fn click_outside_closes_the_picker() {
    let mut h = harness(None);
    let c = h.state().last_canvas_rect.center();
    h.state_mut().ui.brush_picker = Some([c.x, c.y]);
    h.run_steps(2);
    let far = c - vec2(300.0, 200.0);
    h.event(egui::Event::PointerMoved(far));
    press(&mut h, far, PointerButton::Primary, true);
    press(&mut h, far, PointerButton::Primary, false);
    assert_eq!(h.state().ui.brush_picker, None);
}

/// #1031: the right-click picker picks brushes, not just the size. A click picks a preset and
/// keeps the picker open, a double-click picks one and closes it, and Enter applies a typed
/// size before it closes the picker.
#[test]
fn picker_click_picks_a_preset_and_double_click_closes_it() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness(None);
    let c = h.state().last_canvas_rect.center();
    h.event(egui::Event::PointerMoved(c));
    h.run_steps(1);
    press(&mut h, c, PointerButton::Secondary, true);
    press(&mut h, c, PointerButton::Secondary, false);
    h.run_steps(2);
    let names: Vec<String> = h.state().session.tools.presets.iter().skip(1).take(2).map(|p| p.name.clone()).collect();
    let picks = |h: &Harness<'static, PhotocraftApp>, name: &str| {
        h.state().session.journal.iter().filter(|(id, p)| id == "tools.setBrush" && *p == json!({ "preset": name })).count()
    };
    h.get_by_label(&names[0]).click();
    h.run_steps(3);
    assert_eq!(picks(&h, &names[0]), 1);
    assert!(h.state().ui.brush_picker.is_some(), "a click keeps the picker open");
    let at = h.get_by_label(&names[1]).rect().center();
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    // Two clicks 2 frames apart; well after the first click, so not a triple click.
    h.run_steps(40);
    for pressed in [true, false, true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(2);
    assert_eq!(picks(&h, &names[1]), 1, "picked once: the second click only closes");
    assert_eq!(h.state().ui.brush_picker, None, "a double-click closes the picker");
    assert!(strokes(&h).is_empty(), "nothing painted under the picker");
    // Enter applies the typed size, then closes the picker.
    h.state_mut().ui.brush_picker = Some([c.x, c.y]);
    h.run_steps(2);
    h.get_all_by_role(egui::accesskit::Role::SpinButton).next().expect("the Size field").click();
    h.run_steps(1);
    h.event(egui::Event::Text("30*2".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!((h.state().session.tools.brush.size, h.state().ui.brush_picker), (60.0, None));
}

/// A sampled tip has no hardness: the picker shows only its size (Photoshop).
#[test]
fn picker_shows_hardness_only_for_round_tips() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness(None);
    let c = h.state().last_canvas_rect.center();
    h.state_mut().ui.brush_picker = Some([c.x, c.y]);
    h.run_steps(2);
    assert!(h.query_all_by_label("Hardness").next().is_some());
    let sampled = h.state().session.tools.presets.iter().find(|p| p.brush.tip != photocraft_engine::paint::TipShape::Round).map(|p| p.name.clone());
    h.state_mut().run("tools.setBrush", json!({ "preset": sampled.expect("a sampled preset") })).unwrap();
    h.run_steps(2);
    assert!(h.query_all_by_label("Hardness").next().is_none());
    assert!(h.state().ui.brush_picker.is_some());
}

/// The gear menu's New Brush Preset… opens the New Brush Preset dialog (#4490), which saves
/// the preset under the typed name on OK.
#[test]
fn picker_gear_menu_saves_and_names_a_new_preset() {
    use egui_kittest::kittest::Queryable;
    let mut h = harness(None);
    let c = h.state().last_canvas_rect.center();
    h.state_mut().ui.brush_picker = Some([c.x, c.y]);
    h.run_steps(2);
    h.get_by_label("Brush Preset Options").click();
    h.run_steps(2);
    let before = h.state().session.tools.presets.len();
    h.get_by_label("New Brush Preset…").click();
    h.run_steps(3);
    let dialog = h.state().ui.dialogs.iter().find(|d| crate::brush_preset_dialog::owns(&d.fields)).map(|d| d.id).expect("the dialog asks for a name");
    assert_eq!(h.state().session.tools.presets.len(), before, "nothing is saved before OK");
    assert!(h.state().ui.brush_picker.is_none(), "the dialog takes the picker's place");
    // This harness draws the canvas only: OK as automation does (the dialog's own tests type).
    h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("name".into(), json!("Inky"));
    crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
    h.run_steps(2);
    assert!(h.state().session.tools.presets.iter().any(|p| p.name == "Inky"), "saved under the typed name");
    assert_eq!(h.state().session.tools.current_preset.as_deref(), Some("Inky"));
    h.state_mut().ui.brush_picker = Some([c.x, c.y]);
    h.run_steps(2);
    // The gear's boxes pick the card parts; the last one on can't be turned off.
    h.get_by_label("Brush Preset Options").click();
    h.run_steps(2);
    h.get_by_label("Brush Tip").click();
    h.run_steps(2);
    assert!(!h.state().ui.brush_picker_list.show_tip);
    h.get_by_label("Brush Stroke").click();
    h.run_steps(2);
    h.get_by_label("Brush Name").click();
    h.run_steps(2);
    assert!(h.state().ui.brush_picker_list.show_name, "the last part on stays on");
    assert!(!h.state().ui.brush_picker_list.show_stroke && !h.state().ui.brush_picker_list.show_tip);
    assert!(h.state().ui.brush_picker.is_some());
    // A rename left open goes with the picker.
    h.state_mut().ui.brush_picker_list.renaming =
        Some(crate::brush_panel::Renaming { group: false, name: "Inky".into(), folder: Vec::new(), text: String::new() });
    h.run_steps(2);
    let far = c - vec2(300.0, 200.0);
    h.event(egui::Event::PointerMoved(far));
    press(&mut h, far, PointerButton::Primary, true);
    press(&mut h, far, PointerButton::Primary, false);
    assert!(h.state().ui.brush_picker.is_none() && h.state().ui.brush_picker_list.renaming.is_none());
}

#[test]
fn right_drag_erases_with_the_erase_preference() {
    let mut h = harness(Some("erase"));
    let (a, b) = drag(&mut h, PointerButton::Primary);
    let mid = a + (b - a) * 0.5;
    assert!(alpha_at(&h, mid) > 0.9);
    let undo = h.state().session.active().unwrap().history.past_len();
    drag(&mut h, PointerButton::Secondary);
    let s = strokes(&h);
    assert_eq!(s.len(), 2);
    assert_eq!(s[1]["erase"], json!(true), "the right stroke erases");
    assert_eq!(alpha_at(&h, mid), 0.0, "erased to transparency");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), undo + 1, "one undo step");
    assert_eq!(h.state().ui.brush_picker, None, "no picker in erase mode");
    // The next left stroke paints again.
    drag(&mut h, PointerButton::Primary);
    assert_eq!(strokes(&h)[2]["erase"], json!(false));
    assert!(alpha_at(&h, mid) > 0.9);
}

#[test]
fn right_stroke_erases_with_pen_pressure_through_the_control_channel_path() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "erase"})).unwrap();
    app.ui.tool = Tool::Brush;
    let m = Modifiers::NONE;
    assert!(pointer_secondary(&mut app, &egui::Context::default(), true, egui::Modifiers::NONE, [0.0, 0.0]));
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: 0.4 }, m);
    assert!(!app.secondary_erase, "armed for one stroke only");
    tool_event(&mut app, ToolEvent::Move { x: 100.0, y: 40.0, pressure: 0.8 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 40.0 }, m);
    let p = app.session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").map(|(_, p)| p.clone()).unwrap();
    assert_eq!(p["erase"], json!(true));
    assert_eq!(p["points"][0], json!([10.0, 40.0, 0.4f32 as f64]), "pressure reaches the erasing stroke");
    // On the Background the erasing stroke paints the background colour (#76).
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers[0].surface().unwrap().rgba(50, 40), [1.0, 1.0, 1.0, 1.0]);
    // With the default preference the right button opens the picker and nothing paints.
    app.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "brushPicker"})).unwrap();
    assert!(!pointer_secondary(&mut app, &egui::Context::default(), true, egui::Modifiers::NONE, [0.0, 0.0]));
    assert!(app.ui.brush_picker.is_some() && !app.secondary_erase);
}

#[test]
fn shift_alt_right_through_the_control_channel_respects_the_erase_preference() {
    // As on the canvas, with the right-erase preference the erase stroke wins over the HUD
    // for Brush/Eraser; without it, Shift+Alt+right opens the HUD (#6098).
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.ui.tool = Tool::Brush;
    let shift_alt = egui::Modifiers { shift: true, alt: true, ..Default::default() };
    app.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "erase"})).unwrap();
    assert!(pointer_secondary(&mut app, &egui::Context::default(), true, shift_alt, [10.0, 10.0]), "the events reach the tool");
    assert!(app.ui.color_hud.is_none(), "no HUD with the erase preference");
    assert!(app.secondary_erase, "the down arms the erasing stroke");
    pointer_secondary(&mut app, &egui::Context::default(), false, shift_alt, [10.0, 10.0]);
    // With the default preference the same gesture opens the HUD instead.
    app.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "brushPicker"})).unwrap();
    assert!(!pointer_secondary(&mut app, &egui::Context::default(), true, shift_alt, [10.0, 10.0]));
    assert!(app.ui.color_hud.is_some_and(|h| !h.hold), "the control-channel HUD opened");
}

#[test]
fn a_control_channel_up_closes_the_hud_without_repeating_its_modifiers() {
    // Each control-channel request carries its own modifiers, so an Up sent after the keys
    // are released arrives without Shift+Alt: it must still close the HUD it opened, or the
    // HUD stays up and keeps picking from the hover (#6098 review).
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.ui.tool = Tool::Brush;
    let shift_alt = egui::Modifiers { shift: true, alt: true, ..Default::default() };
    assert!(!pointer_secondary(&mut app, &egui::Context::default(), true, shift_alt, [10.0, 10.0]));
    assert!(app.ui.color_hud.is_some_and(|h| !h.hold), "the control-channel HUD opened");
    assert!(!pointer_secondary(&mut app, &egui::Context::default(), false, egui::Modifiers::NONE, [10.0, 10.0]));
    assert!(app.ui.color_hud.is_none(), "the Up closes it whatever the modifiers say");
    // A real-mouse HUD belongs to the canvas gesture and waits for its own button release.
    crate::color_hud::open(&mut app, [10.0, 10.0], true);
    pointer_secondary(&mut app, &egui::Context::default(), false, egui::Modifiers::NONE, [10.0, 10.0]);
    assert!(app.ui.color_hud.is_some_and(|h| h.hold), "a held HUD is left to the canvas");
}

#[test]
fn the_brush_is_a_per_tool_option_that_presets_keep() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let amount = |app: &PhotocraftApp| app.session.tools.brush.smoothing.amount;
    app.ui.tool = Tool::Brush;
    sync_tool_brush(&mut app);
    assert_eq!(amount(&app), 0.1, "Photoshop's default");
    // Picking a preset (whose own smoothing is 0) keeps the tool's smoothing.
    let preset = app.session.tools.presets.iter().find(|p| p.brush.smoothing.amount == 0.0).map(|p| p.name.clone()).unwrap();
    app.run("tools.setBrush", json!({"preset": preset})).unwrap();
    assert_eq!(amount(&app), 0.1);
    // A user value (the options-bar field) survives a preset too, mode included.
    app.session.tools.brush.smoothing.amount = 0.42;
    app.session.tools.brush.smoothing.pulled_string = true;
    app.run("tools.setBrush", json!({"preset": preset})).unwrap();
    assert_eq!((amount(&app), app.session.tools.brush.smoothing.pulled_string), (0.42, true));
    // The Eraser has its own (10 % at first); switching back restores the Brush's.
    app.ui.tool = Tool::Eraser;
    sync_tool_brush(&mut app);
    assert_eq!(amount(&app), 0.1);
    app.session.tools.brush.smoothing.amount = 0.0;
    app.ui.tool = Tool::Brush;
    sync_tool_brush(&mut app);
    assert_eq!((amount(&app), app.session.tools.brush.smoothing.pulled_string), (0.42, true));
    // Other tools leave it alone; the Eraser kept its 0 %.
    app.ui.tool = Tool::Move;
    sync_tool_brush(&mut app);
    assert_eq!(amount(&app), 0.42);
    app.ui.tool = Tool::Eraser;
    // A stroke picks up the tool's brush even without a frame in between.
    tool_event(&mut app, ToolEvent::Down { x: 5.0, y: 5.0, pressure: 1.0 }, Modifiers::NONE);
    assert_eq!(amount(&app), 0.0);
    tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 5.0 }, Modifiers::NONE);
}

/// #218: Photoshop keeps a brush per painting tool, so the size, hardness, mode, opacity and
/// dynamics of one tool never leak into another's, and each tool gets its own back.
#[test]
fn each_tool_keeps_its_own_brush() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let brush = |app: &PhotocraftApp| app.session.tools.brush.clone();
    let size = |b: &BrushSettings| b.size;
    let hardness = |b: &BrushSettings| b.hardness;
    let opacity = |b: &BrushSettings| b.opacity;
    let mode = |b: &BrushSettings| b.mode;

    app.ui.tool = Tool::Brush;
    sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 40, "hardness": 0.2, "opacity": 0.7, "mode": "Multiply"}})).unwrap();
    assert_eq!(size(&brush(&app)), 40.0);

    // A tool seen for the first time starts from the brush the old tool left (Photoshop's
    // 10 % smoothing), so nothing is lost by switching.
    app.ui.tool = Tool::Eraser;
    sync_tool_brush(&mut app);
    assert_eq!(size(&brush(&app)), 40.0, "a new tool inherits the current brush");

    // The Eraser edits its own size; the Brush keeps 40 px.
    app.run("tools.setBrush", json!({"brush": {"size": 8, "hardness": 1.0}})).unwrap();
    app.ui.tool = Tool::Brush;
    sync_tool_brush(&mut app);
    assert_eq!(size(&brush(&app)), 40.0, "the Brush keeps its own size");
    assert_eq!(hardness(&brush(&app)), 0.2, "and its hardness");
    app.ui.tool = Tool::Eraser;
    sync_tool_brush(&mut app);
    assert_eq!(size(&brush(&app)), 8.0, "the Eraser keeps its own size");
    assert_eq!(hardness(&brush(&app)), 1.0, "and its hardness");

    // Mode, opacity, flow and the dynamics sections are per tool too.
    app.ui.tool = Tool::CloneStamp;
    sync_tool_brush(&mut app);
    assert_eq!(mode(&brush(&app)), photocraft_color::BlendMode::Normal, "a new tool starts in Normal, not the Brush's Multiply");
    app.run("tools.setBrush", json!({"brush": {"mode": "Screen", "opacity": 0.3}})).unwrap();
    app.ui.tool = Tool::Brush;
    sync_tool_brush(&mut app);
    assert_eq!((mode(&brush(&app)), opacity(&brush(&app))), (photocraft_color::BlendMode::Multiply, 0.7), "the Brush's own mode and opacity");
    app.ui.tool = Tool::CloneStamp;
    sync_tool_brush(&mut app);
    assert_eq!((mode(&brush(&app)), opacity(&brush(&app))), (photocraft_color::BlendMode::Screen, 0.3), "the Clone Stamp keeps its own");

    // The colours, the erase flag and the seed belong to the session, so a swap keeps what is
    // live now instead of restoring another tool's copy (a picked colour reaches every tool).
    app.run("tools.setBrush", json!({"brush": {"color": [0.0, 1.0, 0.0, 1.0], "background": [1.0, 0.0, 1.0, 1.0], "erase": true, "seed": 42}})).unwrap();
    app.ui.tool = Tool::Brush;
    sync_tool_brush(&mut app);
    let b = brush(&app);
    assert_eq!(
        (b.color, b.background, b.erase, b.seed),
        ([0.0, 1.0, 0.0, 1.0], [1.0, 0.0, 1.0, 1.0], true, 42),
        "the session's own brush fields survive the swap"
    );

    // Tools with no brush of their own (the Move) leave the brush alone.
    let before = brush(&app);
    app.ui.tool = Tool::Move;
    sync_tool_brush(&mut app);
    assert_eq!(brush(&app), before);
}

#[test]
fn eraser_tool_on_the_background_paints_the_background_colour() {
    // #76: the Eraser on the Background (locked transparency) paints the background colour.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.run("tools.setColors", json!({"background": "#ff0000"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0}})).unwrap();
    app.ui.tool = Tool::Eraser;
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 100.0, y: 40.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 40.0 }, m);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers[0].name, "Background");
    assert_eq!(st.doc.layers[0].surface().unwrap().rgba(50, 40), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(st.history.undo_label(), Some("Eraser"));
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

/// Assert the one committed stroke contains a point at each queued screen position.
fn assert_committed_points(h: &Harness<'static, PhotocraftApp>, queued: &[Pos2]) {
    let s = strokes(h);
    assert_eq!(s.len(), 1);
    let points = s[0]["points"].as_array().unwrap();
    let app = h.state();
    let v = app.ui.views[0].clone();
    let r = app.last_canvas_rect;
    for p in queued {
        let d = (*p - r.center()) / v.zoom;
        let want = [d.x as f64 + v.center[0] as f64, d.y as f64 + v.center[1] as f64];
        assert!(
            points.iter().any(|q| {
                let q = q.as_array().unwrap();
                (q[0].as_f64().unwrap() - want[0]).abs() < 1.0 && (q[1].as_f64().unwrap() - want[1]).abs() < 1.0
            }),
            "queued move {want:?} missing from {points:?}"
        );
    }
}

#[test]
fn one_frame_feeds_every_pointer_move_not_just_the_latest() {
    // Fast strokes used to lose the moves between two frames: the canvas read only
    // `interact_pointer_pos()` once per frame, so the stroke was a coarse polyline. Every
    // `PointerMoved` egui-winit delivered in one frame must now reach the stroke.
    let mut h = harness(None);
    let c = h.state().last_canvas_rect.center();
    let a = c - vec2(90.0, 0.0);
    h.event(egui::Event::PointerMoved(a));
    h.run_steps(1);
    press(&mut h, a, PointerButton::Primary, true);
    // Cross egui's drag threshold on its own frame.
    h.event(egui::Event::PointerMoved(a + vec2(20.0, 0.0)));
    h.run_steps(1);
    // Several moves in ONE frame: `Harness::event` would make one frame per event, so push
    // them straight into the next frame's input. All of them must reach the committed stroke.
    let queued: Vec<Pos2> = (1..=6).map(|i| a + vec2(20.0 + i as f32 * 8.0, 0.0)).collect();
    for p in &queued {
        h.input_mut().events.push(egui::Event::PointerMoved(*p));
    }
    h.run_steps(1);
    let end = *queued.last().unwrap();
    press(&mut h, end, PointerButton::Primary, false);
    assert_committed_points(&h, &queued);
}

/// Shift+right-click with `tool` and the labels of the menu it opened (empty: none opened).
fn open_tool_menu(h: &mut Harness<'static, PhotocraftApp>, tool: Tool) -> Vec<&'static str> {
    h.state_mut().ui.tool = tool;
    h.run_steps(1);
    mode_right_click(h, Modifiers::SHIFT);
    if h.state().ui.brush_mode_menu.is_none() { Vec::new() } else { crate::brush_mode_menu::labels(tool) }
}

fn choose_row(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    use egui_kittest::kittest::Queryable;
    h.get_by_label(label).click();
    h.run_steps(2);
    assert!(h.state().ui.brush_mode_menu.is_none(), "choosing {label} closes the menu");
}

#[test]
fn shift_right_menu_lists_each_tools_own_rows_in_order() {
    use crate::brush_mode_menu::{Item, items};
    use egui_kittest::kittest::Queryable;
    let blend: Vec<&'static str> = photocraft_doc::BlendMode::LAYER_MODES.iter().map(|m| m.label()).collect();
    let with = |extra: &[&'static str]| {
        let mut v = blend.clone();
        for (i, l) in extra.iter().enumerate() {
            v.insert(2 + i, l);
        }
        v
    };
    let edit = |rest: Vec<&'static str>| std::iter::once("Edit Brush…").chain(rest).collect::<Vec<_>>();
    for (tool, want) in [
        (Tool::Brush, edit(with(&["Behind", "Clear"]))),
        (Tool::Pencil, edit(with(&["Behind", "Clear"]))),
        (Tool::CloneStamp, edit(with(&[]))),
        (Tool::Eraser, vec!["Brush", "Pencil", "Block"]),
        (Tool::Dodge, edit(vec!["Shadows", "Midtones", "Highlights"])),
        (Tool::Sponge, edit(vec!["Desaturate", "Saturate"])),
        (Tool::PaintBucket, with(&["Clear"])),
        (Tool::MixerBrush, vec!["Edit Brush…"]),
        (Tool::Healing, vec!["Normal", "Multiply", "Screen", "Darken", "Lighten", "Color", "Luminosity"]),
        (Tool::SpotHealing, vec![]),
        (Tool::MagicEraser, vec![]),
    ] {
        let mut h = harness(None);
        let journal = h.state().session.journal.len();
        assert_eq!(open_tool_menu(&mut h, tool), want, "{tool:?}");
        assert_eq!(h.state().session.journal.len(), journal, "{tool:?}: opening changes nothing");
        if want.is_empty() {
            continue;
        }
        assert!(h.state().ui.brush_picker.is_none(), "{tool:?}");
        // A separator after Edit Brush…, and the blend modes in their groups.
        let rows = items(tool);
        if rows.first() == Some(&Item::EditBrush) && rows.len() > 1 {
            assert_eq!(rows.get(1), Some(&Item::Separator), "{tool:?}");
        }
        if want.contains(&"Darken") && want.contains(&"Dissolve") {
            let at = rows.iter().position(|i| *i == Item::Blend(photocraft_doc::BlendMode::Darken)).unwrap();
            assert_eq!(rows[at - 1], Item::Separator, "{tool:?}");
        }
        // The rows the menu shows first are drawn, top to bottom, in that order.
        let tops: Vec<f32> = want.iter().take(4).map(|l| h.get_by_label(l).rect().top()).collect();
        assert!(tops.windows(2).all(|w| w[0] < w[1]), "{tool:?}: {tops:?}");
    }
}

#[test]
fn shift_right_menu_rows_set_what_the_options_bar_sets() {
    use photocraft_doc::BlendMode;
    let mut h = harness(None);
    // Pencil: Clear is the Pencil's own flag, Behind its brush's, both through tools.setBrush.
    open_tool_menu(&mut h, Tool::Pencil);
    choose_row(&mut h, "Clear");
    assert!(h.state().ui.tool_options.pencil_clear && !h.state().ui.tool_options.brush_clear);
    open_tool_menu(&mut h, Tool::Pencil);
    choose_row(&mut h, "Behind");
    assert!(!h.state().ui.tool_options.pencil_clear && h.state().session.tools.brush.behind);
    assert_eq!(h.state().session.journal.last().unwrap(), &("tools.setBrush".into(), json!({"brush": {"mode": "Normal", "behind": true}})));
    // Clone Stamp: its own brush's mode, as the options bar's Mode dropdown.
    open_tool_menu(&mut h, Tool::CloneStamp);
    choose_row(&mut h, "Multiply");
    assert_eq!(h.state().session.tools.brush.mode, BlendMode::Multiply);
    assert_eq!(h.state().session.journal.last().unwrap(), &("tools.setBrush".into(), json!({"brush": {"mode": "Multiply"}})));
    assert_eq!(crate::blend_keys::tool_mode(h.state_mut()), Some(BlendMode::Multiply));
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(1);
    assert_eq!(h.state().session.tools.brush.mode, BlendMode::Normal, "the Brush keeps its own mode");
    // Healing Brush: the short list.
    open_tool_menu(&mut h, Tool::Healing);
    choose_row(&mut h, "Luminosity");
    assert_eq!(h.state().session.tools.brush.mode, BlendMode::Luminosity);
    // Tool options: Eraser Mode, Dodge Range, Sponge Mode, Paint Bucket and Gradient modes.
    open_tool_menu(&mut h, Tool::Eraser);
    choose_row(&mut h, "Block");
    assert_eq!(h.state().ui.tool_options.eraser_mode, "block");
    open_tool_menu(&mut h, Tool::Dodge);
    choose_row(&mut h, "Highlights");
    assert_eq!(h.state().ui.tool_options.tone_range, "highlights");
    open_tool_menu(&mut h, Tool::Sponge);
    choose_row(&mut h, "Saturate");
    assert_eq!(h.state().ui.tool_options.sponge_mode, "saturate");
    open_tool_menu(&mut h, Tool::PaintBucket);
    choose_row(&mut h, "Clear");
    assert!(h.state().ui.tool_options.bucket_clear);
    open_tool_menu(&mut h, Tool::PaintBucket);
    choose_row(&mut h, "Screen");
    assert!(!h.state().ui.tool_options.bucket_clear);
    assert_eq!(h.state().ui.tool_options.bucket_blend_mode, BlendMode::Screen);
    open_tool_menu(&mut h, Tool::Gradient);
    choose_row(&mut h, "Overlay");
    assert_eq!(h.state().ui.tool_options.gradient_blend_mode, BlendMode::Overlay);
    // The checked row is the current one.
    open_tool_menu(&mut h, Tool::Eraser);
    assert_eq!(crate::brush_mode_menu::current(h.state(), Tool::Eraser), Some(crate::brush_mode_menu::Item::Eraser("block")));
}

#[test]
fn edit_brush_opens_the_brush_preset_picker_at_the_click() {
    for tool in [Tool::Brush, Tool::CloneStamp, Tool::MixerBrush, Tool::Dodge] {
        let mut h = harness(None);
        open_tool_menu(&mut h, tool);
        let at = h.state().ui.brush_mode_menu.unwrap();
        let journal = h.state().session.journal.len();
        choose_row(&mut h, "Edit Brush…");
        assert_eq!(h.state().ui.brush_picker, Some(at), "{tool:?}");
        assert_eq!(h.state().session.journal.len(), journal, "{tool:?}");
        h.run_steps(2);
        assert!(h.state().ui.brush_picker.is_some(), "{tool:?}: the picker stays open");
    }
}

#[test]
fn behind_and_clear_are_greyed_on_the_background_layer() {
    use crate::brush_mode_menu::{Item, enabled, find};
    use egui_kittest::kittest::Queryable;
    let mut h = harness(None);
    let (bg, top) = {
        let doc = &h.state().session.active().unwrap().doc;
        (doc.layers[0].id, doc.layers[1].id)
    };
    h.state_mut().session.active_mut().unwrap().active_layer = Some(bg);
    for tool in [Tool::Brush, Tool::Pencil, Tool::PaintBucket] {
        open_tool_menu(&mut h, tool);
        assert!(!enabled(h.state(), Item::Clear), "{tool:?}");
        assert!(find(h.state(), "Clear").is_err(), "{tool:?}");
        assert!(enabled(h.state(), Item::Blend(photocraft_doc::BlendMode::Multiply)));
        let journal = h.state().session.journal.len();
        h.get_by_label("Clear").click();
        h.run_steps(2);
        let o = &h.state().ui.tool_options;
        assert!(!o.brush_clear && !o.pencil_clear && !o.bucket_clear, "{tool:?}");
        assert_eq!(h.state().session.journal.len(), journal, "{tool:?}: a greyed row does nothing");
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
    }
    assert!(!enabled(h.state(), Item::Behind));
    // On a normal layer they are available again.
    h.state_mut().session.active_mut().unwrap().active_layer = Some(top);
    assert!(enabled(h.state(), Item::Behind) && enabled(h.state(), Item::Clear));
}
