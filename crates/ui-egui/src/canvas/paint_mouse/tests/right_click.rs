//! What counts as a right-click on the canvas (#10183): a right press that went down on the
//! canvas itself, released over it, with no gesture using it in between.

use egui::pos2;

use super::*;

/// The open popup's position: the Pen's canvas tool menu, or else the Brush Preset picker.
fn popup_at(h: &Harness<'static, PhotocraftApp>, pen: bool) -> Option<[f32; 2]> {
    if pen { h.state().ui.canvas_tool_menu.as_ref().map(|m| m.pos) } else { h.state().ui.brush_picker }
}

/// A right press on a popup over the canvas (the Brush Preset picker, the Pen's menu) is the
/// popup's, even when the move off it and the release arrive in the same frame: it never reopens
/// the popup where the button came up.
#[test]
fn a_right_press_on_a_popup_over_the_canvas_is_no_canvas_click() {
    for pen in [false, true] {
        for split in [true, false] {
            let mut h = harness(None);
            if pen {
                h.state_mut().ui.tool = Tool::Pen;
                h.run_steps(2);
            }
            let c = h.state().last_canvas_rect.center();
            right_click_in_one_frame(&mut h, c, c, true);
            assert_eq!(popup_at(&h, pen), Some([c.x, c.y]), "pen {pen}: the popup opens at the click");
            let id = if pen { egui::Id::unique("canvas-selection-menu") } else { crate::brush_picker::picker_id() };
            let popup = h.ctx.memory(|m| m.area_rect(id)).expect("the popup is shown");
            let canvas = h.state().last_canvas_rect;
            // Just inside the popup's left edge and just outside it: a move of a few pixels.
            let (on, off) = (pos2(popup.left() + 4.0, popup.center().y), pos2(popup.left() - 6.0, popup.center().y));
            assert!(canvas.contains(on) && !popup.contains(off));
            right_click_in_one_frame(&mut h, on, off, split);
            for p in [on, off] {
                assert_ne!(popup_at(&h, pen), Some([p.x, p.y]), "pen {pen}, split {split}: a press on the popup is no canvas right-click");
            }
            assert!(strokes(&h).is_empty());
        }
    }
}

/// The scrollbars are drawn over the canvas edges and take the pointer there: a right-click on
/// one, still or moved, opens neither the Brush Preset picker nor a canvas menu.
#[test]
fn a_right_click_on_a_canvas_scrollbar_opens_no_menu() {
    for tool in [Tool::Brush, Tool::Pen] {
        let mut h = harness(None);
        h.state_mut().ui.tool = tool;
        h.state_mut().ui.views[0].zoom = 6.0;
        h.run_steps(4);
        let r = h.state().last_canvas_rect;
        let bars = [(pos2(r.center().x, r.bottom() - 3.0), vec2(9.0, 0.0)), (pos2(r.right() - 3.0, r.center().y), vec2(0.0, 9.0))];
        for (p, along) in bars {
            for (to, split) in [(p, true), (p + along, true), (p + along, false)] {
                right_click_in_one_frame(&mut h, p, to, split);
                assert_eq!(h.state().ui.brush_picker, None, "{tool:?} at {p:?} to {to:?}, split {split}: no picker");
                assert!(h.state().ui.canvas_tool_menu.is_none(), "{tool:?} at {p:?} to {to:?}, split {split}: no menu");
            }
        }
        // Inside the bars a right-click is still one.
        right_click_in_one_frame(&mut h, r.center(), r.center(), true);
        assert!(h.state().ui.brush_picker.is_some() || h.state().ui.canvas_tool_menu.is_some(), "{tool:?}: a right-click on the image opens its menu");
    }
}

/// One pointer button event in its own frame.
fn button(h: &mut Harness<'static, PhotocraftApp>, pos: Pos2, button: PointerButton, pressed: bool) {
    h.event(egui::Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

/// A pointer move by `by` from `from`, in `steps` frames; returns where it ends.
fn move_by(h: &mut Harness<'static, PhotocraftApp>, from: Pos2, by: egui::Vec2, steps: usize) -> Pos2 {
    for i in 1..=steps {
        h.event(egui::Event::PointerMoved(from + by * (i as f32 / steps as f32)));
        h.run_steps(1);
    }
    from + by
}

fn space(h: &mut Harness<'static, PhotocraftApp>, pressed: bool) {
    h.event(egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

/// A harness zoomed in far enough to pan, with `tool`.
fn zoomed(tool: Tool, prefs: Option<&str>) -> Harness<'static, PhotocraftApp> {
    let mut h = harness(prefs);
    h.state_mut().ui.tool = tool;
    h.state_mut().ui.views[0].zoom = 4.0;
    h.run_steps(2);
    h
}

fn nothing_opened(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.state().ui.brush_picker.is_none() && h.state().ui.canvas_tool_menu.is_none()
}

/// Space is the Hand over any tool, and a right drag with it pans, as before: only the selected
/// Hand's right button without Space is its view menu. The pan opens nothing and paints nothing.
#[test]
fn space_and_a_right_drag_pan_with_any_tool() {
    for (tool, prefs) in [(Tool::Brush, None), (Tool::Brush, Some("erase")), (Tool::Pen, None)] {
        let mut h = zoomed(tool, prefs);
        let center = h.state().ui.views[0].center;
        space(&mut h, true);
        right_drag_by(&mut h, vec2(60.0, 36.0), Modifiers::NONE);
        space(&mut h, false);
        assert_ne!(h.state().ui.views[0].center, center, "{tool:?} {prefs:?}: Space and a right drag pan");
        assert!(nothing_opened(&h), "{tool:?} {prefs:?}: and open nothing");
        assert!(strokes(&h).is_empty(), "{tool:?} {prefs:?}: nor paint");
    }
}

/// A Space+right pan whose Space comes up before the right button: the release is the end of a
/// pan, not a right-click, so it neither erases (Erase preference) nor opens the picker.
#[test]
fn letting_go_of_space_before_the_right_button_ends_the_pan_only() {
    for prefs in [Some("erase"), None] {
        let mut h = zoomed(Tool::Brush, prefs);
        h.state_mut().run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        h.run_steps(2);
        let center = h.state().ui.views[0].center;
        let c = h.state().last_canvas_rect.center();
        space(&mut h, true);
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, PointerButton::Secondary, true);
        let p = move_by(&mut h, c, vec2(10.0, 0.0), 4);
        space(&mut h, false);
        let p = move_by(&mut h, p, vec2(6.0, 0.0), 2);
        button(&mut h, p, PointerButton::Secondary, false);
        h.run_steps(2);
        assert!(strokes(&h).is_empty(), "{prefs:?}: the release erases nothing");
        assert!(alpha_at(&h, p) > 0.0, "{prefs:?}: the layer is untouched where it came up");
        assert!(nothing_opened(&h), "{prefs:?}: and opens nothing");
        assert_ne!(h.state().ui.views[0].center, center, "{prefs:?}: the view panned while Space was held");
    }
}

/// With the Hand, a left pan and the right button chorded in, either way round: the right button
/// took part in the pan, so its release opens no view menu, and the view pans.
#[test]
fn a_hand_pan_with_the_right_button_chorded_in_opens_no_menu() {
    for right_first in [false, true] {
        let mut h = zoomed(Tool::Hand, None);
        let center = h.state().ui.views[0].center;
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        let (first, second) = if right_first { (PointerButton::Secondary, PointerButton::Primary) } else { (PointerButton::Primary, PointerButton::Secondary) };
        button(&mut h, c, first, true);
        let p = move_by(&mut h, c, vec2(8.0, 0.0), 4);
        button(&mut h, p, second, true);
        let p = move_by(&mut h, p, vec2(8.0, 0.0), 4);
        // The right button comes up first while the left still pans, or last.
        let order = if right_first { [PointerButton::Primary, PointerButton::Secondary] } else { [PointerButton::Secondary, PointerButton::Primary] };
        for b in order {
            button(&mut h, p, b, false);
        }
        h.run_steps(2);
        assert_ne!(h.state().ui.views[0].center, center, "right first {right_first}: the view pans");
        assert!(h.state().ui.canvas_tool_menu.is_none(), "right first {right_first}: no view menu");
    }
}

/// The selected Hand pans with the back and forward buttons, as with any button but the right.
#[test]
fn the_hand_pans_with_the_extra_mouse_buttons() {
    for b in [PointerButton::Extra1, PointerButton::Extra2] {
        let mut h = zoomed(Tool::Hand, None);
        let center = h.state().ui.views[0].center;
        drag(&mut h, b);
        assert_ne!(h.state().ui.views[0].center, center, "{b:?} pans");
        assert!(h.state().ui.canvas_tool_menu.is_none());
    }
}

/// A right press while the left button is painting is no right-click of the canvas's own: only
/// egui's, a still click, opens the picker then, as it always has. The stroke commits whole.
#[test]
fn a_right_press_moved_during_a_left_stroke_opens_no_picker() {
    for moved in [true, false] {
        let mut h = harness(None);
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, PointerButton::Primary, true);
        let p = move_by(&mut h, c, vec2(40.0, 0.0), 4);
        button(&mut h, p, PointerButton::Secondary, true);
        let q = if moved { move_by(&mut h, p, vec2(0.0, 12.0), 4) } else { p };
        button(&mut h, q, PointerButton::Secondary, false);
        h.run_steps(1);
        assert_eq!(h.state().ui.brush_picker.is_some(), !moved, "moved {moved}: the picker opens only for a still click");
        h.state_mut().ui.brush_picker = None;
        let end = move_by(&mut h, q, vec2(20.0, 0.0), 2);
        button(&mut h, end, PointerButton::Primary, false);
        h.run_steps(2);
        assert_eq!(strokes(&h).len(), 1, "moved {moved}: one stroke");
    }
}

/// One key going down and up, each in its own frame, with `mods` held.
fn key(h: &mut Harness<'static, PhotocraftApp>, key: egui::Key, mods: Modifiers) {
    for pressed in [true, false] {
        h.event(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods });
        h.run_steps(1);
    }
}

/// Escape pressed while the right button is down cancels the right-click: a moved release opens
/// neither the Brush Preset picker nor the canvas tool menu. That also holds after Escape closed a
/// Shift+Alt+right HUD Color Picker, whose button is still down.
#[test]
fn escape_while_the_right_button_is_down_cancels_the_right_click() {
    for (tool, mods) in [(Tool::Brush, Modifiers::NONE), (Tool::Pen, Modifiers::NONE), (Tool::Brush, Modifiers::SHIFT | Modifiers::ALT)] {
        let mut h = harness(None);
        h.state_mut().ui.tool = tool;
        h.run_steps(2);
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::ModifiersChanged(mods));
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: c, button: PointerButton::Secondary, pressed: true, modifiers: mods });
        h.run_steps(1);
        let hud = mods.alt;
        assert_eq!(crate::color_hud::is_open(&h.state().ui), hud, "{tool:?} {mods:?}: the HUD opens only for Shift+Alt");
        let p = move_by(&mut h, c, vec2(8.0, 0.0), 2);
        key(&mut h, egui::Key::Escape, mods);
        assert!(!crate::color_hud::is_open(&h.state().ui), "{tool:?} {mods:?}: Escape closes the HUD");
        let p = move_by(&mut h, p, vec2(6.0, 4.0), 2);
        button(&mut h, p, PointerButton::Secondary, false);
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(2);
        assert!(nothing_opened(&h), "{tool:?} {mods:?}: Escape cancelled the right-click");
        assert!(strokes(&h).is_empty(), "{tool:?} {mods:?}: and nothing paints");
        // The next right-click is one again.
        right_click_in_one_frame(&mut h, c, c + vec2(14.0, 0.0), true);
        assert!(!nothing_opened(&h), "{tool:?} {mods:?}: the next right-click opens its menu");
    }
}

/// Space is the Hand's pan over the selected Hand too: with Space held, the Hand's right drag pans
/// and opens no view menu. Without Space it opens the menu without panning (#10183).
#[test]
fn space_and_a_right_drag_pan_with_the_hand_selected() {
    let mut h = zoomed(Tool::Hand, None);
    let center = h.state().ui.views[0].center;
    space(&mut h, true);
    right_drag_by(&mut h, vec2(60.0, 36.0), Modifiers::NONE);
    space(&mut h, false);
    assert_ne!(h.state().ui.views[0].center, center, "Space and a right drag pan with the Hand");
    assert!(h.state().ui.canvas_tool_menu.is_none(), "and open no view menu");
    // A fresh view: the Space pan can glide on (flick panning).
    let mut h = zoomed(Tool::Hand, None);
    let center = h.state().ui.views[0].center;
    right_drag_by(&mut h, vec2(14.0, 8.0), Modifiers::NONE);
    assert_eq!(h.state().ui.views[0].center, center, "without Space the right drag does not pan");
    assert_eq!(h.state().ui.canvas_tool_menu.as_ref().map(|m| m.tool), Some(Tool::Hand), "it opens the view menu");
}

/// A right-click that moved past the click distance points where the button went down, at what
/// the user right-clicked: a guide under the press gets the guide menu, at the press, even when the
/// pointer came up a few pixels off the guide (#10183).
#[test]
fn a_moved_right_click_on_a_guide_opens_the_guide_menu() {
    for split in [true, false] {
        let mut h = harness(None);
        h.state_mut().ui.tool = Tool::Move;
        let x = h.state().ui.views[0].center[0];
        h.state_mut().run("view.newGuide", json!({"orientation": "vertical", "position": x})).unwrap();
        h.run_steps(2);
        let c = h.state().last_canvas_rect.center();
        right_click_in_one_frame(&mut h, c, c + vec2(8.0, 0.0), split);
        let menu = h.state().ui.canvas_tool_menu.as_ref().map(|m| (m.guide, m.pos));
        assert_eq!(menu, Some((true, [c.x, c.y])), "split {split}: the guide menu opens at the press");
        assert!(h.state().ui.layer_menu.is_none(), "split {split}: not the layers under the release");
    }
}

/// A right drag is no right-click: once the pointer has gone farther than a few pixels from the
/// press while the button is down, the release opens nothing, as before #10183. That holds over
/// frames, out and back near the press, and with the whole drag in one slow frame. A Space+right
/// flick with the Hand selected is a pan too short for egui to see, and opens no view menu either.
/// The selected Hand's right drag within a right-click's reach is its view menu, not a pan; past
/// the reach it is a drag after all, and pans as it did before #10183, opening nothing.
#[test]
fn the_hands_right_drag_past_a_few_pixels_pans() {
    let mut h = zoomed(Tool::Hand, None);
    let center = h.state().ui.views[0].center;
    right_drag_by(&mut h, vec2(80.0, 40.0), Modifiers::NONE);
    assert_ne!(h.state().ui.views[0].center, center, "a long right drag with the Hand pans");
    assert!(nothing_opened(&h), "and opens no view menu");
    let mut h = zoomed(Tool::Hand, None);
    let center = h.state().ui.views[0].center;
    right_drag_by(&mut h, vec2(14.0, 7.0), Modifiers::NONE);
    assert_eq!(h.state().ui.views[0].center, center, "a right drag within the reach does not pan");
    assert_eq!(h.state().ui.canvas_tool_menu.as_ref().map(|m| m.tool), Some(Tool::Hand), "it opens the view menu");
}

#[test]
fn a_right_drag_past_a_few_pixels_opens_nothing() {
    for tool in [Tool::Brush, Tool::Pen, Tool::Hand] {
        let mut h = harness(None);
        h.state_mut().ui.tool = tool;
        h.run_steps(2);
        right_drag_by(&mut h, vec2(-400.0, -250.0), Modifiers::NONE);
        assert!(nothing_opened(&h), "{tool:?}: a long right drag opens nothing");
        // Out past the reach and back near the press, over frames.
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, PointerButton::Secondary, true);
        let p = move_by(&mut h, c, vec2(60.0, 0.0), 3);
        let p = move_by(&mut h, p, vec2(-55.0, 0.0), 3);
        button(&mut h, p, PointerButton::Secondary, false);
        h.run_steps(2);
        assert!(nothing_opened(&h), "{tool:?}: a drag out and back opens nothing");
        // The same within one frame, the press in it or in the frame before.
        for split in [true, false] {
            h.event(egui::Event::PointerMoved(c));
            h.run_steps(1);
            let back = c + vec2(5.0, 0.0);
            let press = egui::Event::PointerButton { pos: c, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE };
            let rest = [
                egui::Event::PointerMoved(c + vec2(70.0, 40.0)),
                egui::Event::PointerMoved(back),
                egui::Event::PointerButton { pos: back, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE },
            ];
            if split {
                h.input_mut().events.push(press);
                h.run_steps(1);
                h.input_mut().events.extend(rest);
            } else {
                h.input_mut().events.extend([press].into_iter().chain(rest));
            }
            h.run_steps(2);
            assert!(nothing_opened(&h), "{tool:?}, split {split}: a drag out and back in one frame opens nothing");
        }
        // Within the reach it is still a right-click.
        right_click_in_one_frame(&mut h, c, c + vec2(12.0, 8.0), true);
        assert!(!nothing_opened(&h), "{tool:?}: a right-click that moved a few pixels opens its menu");
    }
    for split in [true, false] {
        let mut h = zoomed(Tool::Hand, None);
        let c = h.state().last_canvas_rect.center();
        space(&mut h, true);
        right_click_in_one_frame(&mut h, c, c + vec2(60.0, 36.0), split);
        space(&mut h, false);
        assert!(h.state().ui.canvas_tool_menu.is_none(), "split {split}: a Space+right flick opens no view menu");
    }
}

/// With the Hand selected, a right drag that Space made a pan stays one until the right button
/// comes up: letting go of Space first neither starts a flick glide while the button is held nor
/// stops the mouse panning, and the release opens no view menu.
#[test]
fn letting_go_of_space_mid_right_drag_with_the_hand_keeps_panning() {
    let mut h = zoomed(Tool::Hand, None);
    let start = h.state().ui.views[0].center;
    let c = h.state().last_canvas_rect.center();
    space(&mut h, true);
    h.event(egui::Event::PointerMoved(c));
    h.run_steps(1);
    button(&mut h, c, PointerButton::Secondary, true);
    let p = move_by(&mut h, c, vec2(80.0, 40.0), 4);
    let panned = h.state().ui.views[0].center;
    assert_ne!(panned, start, "Space and the right drag pan");
    space(&mut h, false);
    h.run_steps(30);
    assert_eq!(h.state().ui.views[0].center, panned, "no glide while the button is held");
    let p = move_by(&mut h, p, vec2(30.0, 0.0), 3);
    let moved = h.state().ui.views[0].center;
    assert!(moved[0] < panned[0], "the mouse still pans after Space comes up");
    button(&mut h, p, PointerButton::Secondary, false);
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "the release opens no view menu");
}

/// A scroll or a zoom while the right button is held moves the image under the press, so the
/// release is no right-click: it would point at whatever is under the press now. Without it the
/// same right-click opens its menu.
#[test]
fn a_view_change_while_the_right_button_is_held_cancels_the_right_click() {
    for tool in [Tool::Brush, Tool::Pen] {
        for change in [None, Some("scroll"), Some("zoom")] {
            let mut h = harness(None);
            h.state_mut().ui.tool = tool;
            h.state_mut().ui.views[0].zoom = 2.0;
            h.run_steps(2);
            let c = h.state().last_canvas_rect.center();
            h.event(egui::Event::PointerMoved(c));
            h.run_steps(1);
            button(&mut h, c, PointerButton::Secondary, true);
            let view = h.state().ui.views[0].clone();
            match change {
                Some("scroll") => {
                    let delta = vec2(-80.0, 0.0);
                    let unit = egui::MouseWheelUnit::Point;
                    h.event(egui::Event::MouseWheel {
                        source: egui::MouseWheelSource::Unknown,
                        unit,
                        delta,
                        phase: egui::TouchPhase::Move,
                        modifiers: Modifiers::NONE,
                    });
                    h.run_steps(1);
                }
                Some(_) => {
                    h.state_mut().ui.views[0].zoom = 3.0;
                    h.run_steps(1);
                }
                None => {}
            }
            let p = move_by(&mut h, c, vec2(8.0, 0.0), 2);
            assert_eq!(
                h.state().ui.views[0].center != view.center || h.state().ui.views[0].zoom != view.zoom,
                change.is_some(),
                "{tool:?} {change:?}: the view moved"
            );
            button(&mut h, p, PointerButton::Secondary, false);
            h.run_steps(2);
            assert_eq!(nothing_opened(&h), change.is_some(), "{tool:?} {change:?}: the right-click opens its menu only with the view still");
        }
    }
}

/// Every painting tool's Shift+right-click menu (Edit Brush… and its Mode list) reads the same
/// right-click as the Brush's: one that moved a few pixels opens the tool's own menu where the
/// button went down, and nothing else; a long Shift+right drag opens nothing. Spot Healing has no
/// such menu and keeps its Brush Preset picker.
#[test]
fn a_moved_shift_right_click_opens_each_painting_tools_own_menu() {
    let tools = [
        Tool::Brush,
        Tool::Pencil,
        Tool::CloneStamp,
        Tool::Eraser,
        Tool::Dodge,
        Tool::Sponge,
        Tool::PaintBucket,
        Tool::Gradient,
        Tool::MixerBrush,
        Tool::Healing,
    ];
    for tool in tools {
        let mut h = harness(None);
        h.state_mut().ui.tool = tool;
        h.run_steps(2);
        let journal = h.state().session.journal.len();
        let at = right_drag_by(&mut h, vec2(9.0, 5.0), Modifiers::SHIFT);
        assert_eq!(h.state().ui.brush_mode_menu, Some([at.x, at.y]), "{tool:?}: the menu opens where the button went down");
        assert!(nothing_opened(&h), "{tool:?}: only the Shift+right-click menu opens");
        assert_eq!(h.state().session.journal.len(), journal, "{tool:?}: and nothing runs");
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        right_drag_by(&mut h, vec2(120.0, 60.0), Modifiers::SHIFT);
        assert!(h.state().ui.brush_mode_menu.is_none() && nothing_opened(&h), "{tool:?}: a long Shift+right drag opens nothing");
        assert!(strokes(&h).is_empty(), "{tool:?}");
    }
    let mut h = harness(None);
    h.state_mut().ui.tool = Tool::SpotHealing;
    h.run_steps(2);
    let at = right_drag_by(&mut h, vec2(9.0, 5.0), Modifiers::SHIFT);
    assert!(h.state().ui.brush_mode_menu.is_none());
    assert_eq!(h.state().ui.brush_picker, Some([at.x, at.y]), "Spot Healing keeps its picker");
}

/// The Shift+Alt+right HUD Color Picker opens with the current colour under the pointer. A press
/// that opened it is the HUD's gesture: released after a small move, on the HUD or just off it
/// on the canvas, it closes the HUD and opens neither the Brush Preset picker nor a canvas menu.
#[test]
fn a_hud_color_picker_press_moved_a_few_pixels_opens_no_menu() {
    let shift_alt = Modifiers::SHIFT | Modifiers::ALT;
    // The harness paints in blue, the HUD square's top-right corner: up is off the HUD, down
    // and left is on it.
    for by in [vec2(0.0, -10.0), vec2(-10.0, 10.0)] {
        let mut h = harness(None);
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::ModifiersChanged(shift_alt));
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: c, button: PointerButton::Secondary, pressed: true, modifiers: shift_alt });
        h.run_steps(2);
        assert!(crate::color_hud::is_open(&h.state().ui), "{by:?}: the HUD opens");
        let p = move_by(&mut h, c, by, 2);
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: shift_alt });
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(2);
        assert!(!crate::color_hud::is_open(&h.state().ui), "{by:?}: the release closes the HUD");
        assert!(nothing_opened(&h) && h.state().ui.brush_mode_menu.is_none(), "{by:?}: and opens no menu");
        assert!(strokes(&h).is_empty(), "{by:?}");
    }
}
