//! #2849: a complete secondary tap can arrive in one frame. Keep its opening press from
//! dismissing a canvas menu whose position rounds to a physical pixel.

use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::{PhotocraftApp, Services, Tool};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Popup {
    Tool,
    Layers,
    Brush,
}

fn harness(tool: Tool, selection: bool, scale: f32) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_pixels_per_point(scale).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("layer.new.layer", json!({"name": "Color"})).unwrap();
        app.run("edit.fill", json!({"color": "#cc6644"})).unwrap();
        if selection {
            app.run("select.rect", json!({"x": 40, "y": 40, "width": 200, "height": 100})).unwrap();
        }
        app.ui.tool = tool;
        app
    });
    h.run_steps(5);
    h
}

fn fractional_point(h: &Harness<'_, PhotocraftApp>, scale: f32) -> Pos2 {
    let center = crate::rulers::content_rect(h.state(), h.state().last_canvas_rect).center();
    pos2(((center.x * scale).floor() + 0.75) / scale, ((center.y * scale).floor() + 0.75) / scale)
}

fn press(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, button: PointerButton, pressed: bool, modifiers: Modifiers) {
    h.input_mut().events.push(Event::PointerButton { pos: at, button, pressed, modifiers });
}

fn tap(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, modifiers: Modifiers, same_frame: bool) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(at));
    h.run_steps(2);
    for pressed in [true, false] {
        // Harness::event separates queued events into frames; RawInput preserves a quick tap.
        press(h, at, PointerButton::Secondary, pressed, modifiers);
        if !same_frame {
            h.run_steps(1);
        }
    }
    h.run_steps(3);
}

fn document_state(h: &Harness<'_, PhotocraftApp>) -> impl PartialEq + use<> {
    let state = h.state().session.active().unwrap();
    let strokes = h.state().session.journal.iter().filter(|(id, _)| id == "paint.stroke").count();
    (state.doc.clone(), state.revision, state.history.past_len(), state.active_layer, state.selected_layers(), strokes)
}

fn is_open(h: &Harness<'_, PhotocraftApp>, popup: Popup) -> bool {
    let ui = &h.state().ui;
    match popup {
        Popup::Tool => ui.canvas_tool_menu.is_some(),
        Popup::Layers => ui.layer_menu.is_some(),
        Popup::Brush => ui.brush_picker.is_some(),
    }
}

fn dismiss_outside(h: &mut Harness<'_, PhotocraftApp>, popup: Popup) {
    // The status-bar corner is outside every popup and does not edit the document.
    let at = pos2(1190.0, 790.0);
    h.event(Event::PointerMoved(at));
    h.run_steps(2);
    press(h, at, PointerButton::Primary, true, Modifiers::NONE);
    h.run_steps(1);
    assert!(!is_open(h, popup), "a later outside press closes immediately");
    press(h, at, PointerButton::Primary, false, Modifiers::NONE);
    h.run_steps(2);
}

fn check_reopening(tool: Tool, modifiers: Modifiers, popup: Popup) {
    for scale in [1.0, 2.0] {
        for selection in [false, true] {
            let mut h = harness(tool, selection, scale);
            let center = fractional_point(&h, scale);
            let aligned = pos2((center.x * scale).round() / scale, (center.y * scale).round() / scale);
            let mut points = vec![center, center + vec2(40.0, 20.0), center - vec2(40.0, 20.0), aligned];
            if popup == Popup::Brush {
                let canvas = crate::rulers::content_rect(h.state(), h.state().last_canvas_rect);
                points.extend([canvas.center(), canvas.left_top() + vec2(40.0, 40.0), canvas.right_bottom() - vec2(40.0, 40.0)]);
            }
            for at in points {
                for same_frame in [false, true] {
                    for attempt in 0..3 {
                        let before = document_state(&h);
                        tap(&mut h, at, modifiers, same_frame);
                        assert!(is_open(&h, popup), "{tool:?} scale={scale} selection={selection} same_frame={same_frame} reopen={attempt} at {at:?}");
                        assert!(document_state(&h) == before, "opening preserves pixels, selection, layers and history without painting");
                        if popup == Popup::Tool {
                            let menu = h.state().ui.canvas_tool_menu.as_ref().unwrap();
                            assert_eq!(menu.has_selection, selection);
                            assert!(!menu.pasteboard);
                        }
                        dismiss_outside(&mut h, popup);
                        assert!(document_state(&h) == before, "outside dismissal preserves the document and history");
                    }
                }
            }
            tap(&mut h, center, modifiers, true);
            h.key_press(egui::Key::Escape);
            h.run_steps(2);
            assert!(!is_open(&h, popup));
        }
    }
}

#[test]
fn secondary_taps_keep_selection_menus_open() {
    // Quick Selection too, though it has a brush: Photoshop's right-click there is the selection
    // menu (#7268), the brush is set from the options bar.
    for tool in [
        Tool::RectMarquee,
        Tool::EllipseMarquee,
        Tool::Lasso,
        Tool::PolygonLasso,
        Tool::MagneticLasso,
        Tool::MagicWand,
        Tool::ObjectSelection,
        Tool::QuickSelection,
    ] {
        check_reopening(tool, Modifiers::NONE, Popup::Tool);
    }
}

#[test]
fn secondary_taps_keep_canvas_layer_menus_open() {
    check_reopening(Tool::Move, Modifiers::NONE, Popup::Layers);
    check_reopening(Tool::RectMarquee, Modifiers::COMMAND, Popup::Layers);
}

/// A macOS Control-click (control_click.rs) reaches the canvas as a right-click that still holds
/// Control: it opens the same menu as a plain right-click and paints nothing.
#[test]
fn control_clicks_open_the_right_click_menus() {
    for (tool, popup) in [(Tool::RectMarquee, Popup::Tool), (Tool::Pen, Popup::Tool), (Tool::Move, Popup::Layers), (Tool::Brush, Popup::Brush)] {
        let mut h = harness(tool, true, 1.0);
        let at = fractional_point(&h, 1.0);
        let before = document_state(&h);
        let mut clicks = crate::control_click::ControlClick::default();
        h.event(Event::ModifiersChanged(Modifiers::CTRL));
        h.event(Event::PointerMoved(at));
        h.run_steps(2);
        for pressed in [true, false] {
            let mut events = vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::CTRL }];
            clicks.normalize(&mut events, true);
            h.input_mut().events.extend(events);
            h.run_steps(1);
        }
        h.run_steps(3);
        assert!(is_open(&h, popup), "{tool:?}");
        assert!(document_state(&h) == before, "{tool:?}: a Control-click paints nothing");
    }
}

#[test]
fn reopened_canvas_menus_still_choose_their_actions() {
    let mut h = harness(Tool::RectMarquee, true, 1.0);
    let at = fractional_point(&h, 1.0);
    tap(&mut h, at, Modifiers::NONE, true);
    assert!(is_open(&h, Popup::Tool));
    dismiss_outside(&mut h, Popup::Tool);
    tap(&mut h, at, Modifiers::NONE, true);
    assert!(is_open(&h, Popup::Tool));
    h.get_by_label(crate::i18n::t("Deselect")).click();
    h.run_steps(3);
    assert!(!is_open(&h, Popup::Tool));
    assert!(h.state().session.active().unwrap().doc.selection.is_none());
    h.state_mut().ui.tool = Tool::Move;
    h.run_steps(2);
    tap(&mut h, at, Modifiers::NONE, true);
    assert!(is_open(&h, Popup::Layers));
    dismiss_outside(&mut h, Popup::Layers);
    tap(&mut h, at, Modifiers::NONE, true);
    assert!(is_open(&h, Popup::Layers));
    let menu_rect = h.ctx.memory(|m| m.area_rect(egui::Id::unique("canvas-layer-menu"))).unwrap();
    h.query_all_by_label("Background").find(|node| menu_rect.contains(node.rect().center())).expect("Background in the layer menu").click();
    h.run_steps(3);
    assert!(!is_open(&h, Popup::Layers));
    let state = h.state().session.active().unwrap();
    assert_eq!(state.doc.layer(state.active_layer.unwrap()).unwrap().name, "Background");
}

#[test]
fn secondary_taps_keep_other_canvas_tool_menus_open() {
    for scale in [1.0, 2.0] {
        for tool in [Tool::Pen, Tool::RectMarquee, Tool::Move] {
            let mut h = harness(tool, true, scale);
            let center = fractional_point(&h, scale);
            let pasteboard = tool != Tool::Pen;
            let at = if pasteboard { center + vec2(240.0, 0.0) } else { center };
            for _ in 0..3 {
                tap(&mut h, at, Modifiers::NONE, true);
                let menu = h.state().ui.canvas_tool_menu.as_ref().expect("Pen or pasteboard menu stays open");
                assert_eq!(menu.pasteboard, pasteboard);
                dismiss_outside(&mut h, Popup::Tool);
            }
        }
        let mut h = harness(Tool::Move, false, scale);
        let ctx = h.ctx.clone();
        crate::transform_tool::begin(h.state_mut(), &ctx).unwrap();
        h.run_steps(3);
        let at = fractional_point(&h, scale);
        for _ in 0..3 {
            tap(&mut h, at, Modifiers::NONE, true);
            assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.transform));
            dismiss_outside(&mut h, Popup::Tool);
            assert!(h.state().ui.transform.is_some(), "dismissing its menu keeps the transform session");
        }
    }
}

fn control_tap(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, popup: Popup) {
    use crate::control::{ControlRequest, Outcome, handle};

    let [x, y] = crate::canvas::ViewXform::active(h.state()).unwrap().to_doc(at);
    let (req, _) = ControlRequest::new("ui.pointer", json!({"button":"secondary", "events":[{"kind":"down", "x":x, "y":y}, {"kind":"up", "x":x, "y":y}]}));
    let ctx = h.ctx.clone();
    let mut input = h.input_mut().clone();
    input.events = vec![
        Event::PointerMoved(at),
        Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
        Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE },
    ];
    let mut reply = None;
    let mut stayed_open = true;
    ctx.run_ui(input, |ui| {
        if reply.is_none()
            && let Outcome::Done(result) = handle(h.state_mut(), ui.ctx(), &req)
        {
            reply = Some(result);
        }
        match popup {
            Popup::Tool => crate::canvas_tool_menu::show(h.state_mut(), ui.ctx()),
            Popup::Layers => crate::layer_pick_ui::show(h.state_mut(), ui.ctx()),
            Popup::Brush => crate::paint_mouse::show_picker(h.state_mut(), ui.ctx()),
        }
        stayed_open &= is_open(h, popup);
    })
    .drop_without_applying_deltas();
    assert_eq!(reply.as_ref().map(|r| &r["ok"]), Some(&json!(true)), "{reply:?}");
    assert!(stayed_open, "the control opening press does not dismiss {popup:?}");
    h.run_steps(3);
    assert!(is_open(h, popup));
}

#[test]
fn control_channel_opening_uses_the_same_dismissal_guard() {
    for (tool, popup, pasteboard) in [
        (Tool::RectMarquee, Popup::Tool, false),
        (Tool::Move, Popup::Layers, false),
        (Tool::RectMarquee, Popup::Tool, true),
        (Tool::Brush, Popup::Brush, false),
        (Tool::Eraser, Popup::Brush, false),
        // Its right-click is the selection menu (#7268).
        (Tool::QuickSelection, Popup::Tool, false),
    ] {
        for scale in [1.0, 2.0] {
            let mut h = harness(tool, true, scale);
            let at = fractional_point(&h, scale) + if pasteboard { vec2(240.0, 0.0) } else { egui::Vec2::ZERO };
            for _ in 0..3 {
                let before = document_state(&h);
                control_tap(&mut h, at, popup);
                assert!(document_state(&h) == before, "pixels, selection, layers and history are unchanged");
                if popup == Popup::Tool {
                    assert_eq!(h.state().ui.canvas_tool_menu.as_ref().unwrap().pasteboard, pasteboard);
                }
                dismiss_outside(&mut h, popup);
                assert!(document_state(&h) == before, "pixels, selection, layers and history are unchanged");
            }
        }
    }
}

/// #7324: an agent's right-click on committed type opens the type layer menu, as on the canvas.
#[test]
fn control_channel_right_click_on_committed_type_opens_the_type_layer_menu() {
    let mut h = harness(Tool::Type, false, 1.0);
    h.state_mut().run("type.create", json!({"text": "Hello", "size": 72, "x": 100, "y": 180})).unwrap();
    h.run_steps(3);
    let at = fractional_point(&h, 1.0);
    let before = document_state(&h);
    control_tap(&mut h, at, Popup::Tool);
    let menu = h.state().ui.canvas_tool_menu.clone().unwrap();
    assert!(menu.type_layer && !menu.text);
    assert!(h.state().ui.text_edit.is_none());
    // The new type is already the active layer, so the menu selects nothing either.
    assert!(document_state(&h) == before, "opening the menu changes nothing");
}

#[test]
fn brush_picker_taps_preserve_pixels_selection_and_history_after_reopening() {
    for tool in [Tool::Brush, Tool::Eraser] {
        check_reopening(tool, Modifiers::NONE, Popup::Brush);
    }
}

#[test]
fn brush_chip_toggles_after_canvas_and_control_opening() {
    for tool in [Tool::Brush, Tool::Eraser] {
        for scale in [1.0, 2.0] {
            let mut h = harness(tool, true, scale);
            let before = document_state(&h);
            let at = fractional_point(&h, scale);
            tap(&mut h, at, Modifiers::NONE, true);
            assert!(h.state().ui.brush_picker.is_some());
            for _ in 0..3 {
                h.get_by_label(crate::i18n::t("Brush Preset picker")).click();
                h.run_steps(3);
                assert!(h.state().ui.brush_picker.is_none(), "the chip closes the open picker");
                h.get_by_label(crate::i18n::t("Brush Preset picker")).click();
                h.run_steps(3);
                assert!(h.state().ui.brush_picker.is_some(), "the chip reopens the picker");
                assert!(document_state(&h) == before, "pixels, selection, layers and history are unchanged");
            }
            dismiss_outside(&mut h, Popup::Brush);
            control_tap(&mut h, at, Popup::Brush);
            h.get_by_label(crate::i18n::t("Brush Preset picker")).click();
            h.run_steps(3);
            assert!(h.state().ui.brush_picker.is_none());
            assert!(document_state(&h) == before, "pixels, selection, layers and history are unchanged");
        }
    }
}

/// #7268: the Quick Selection tool's brush chip still opens and closes the Brush Preset picker;
/// only its canvas right-click is the selection menu.
#[test]
fn quick_selection_chip_toggles_the_picker_and_right_click_opens_the_selection_menu() {
    let mut h = harness(Tool::QuickSelection, true, 1.0);
    let before = document_state(&h);
    for _ in 0..2 {
        h.get_by_label(crate::i18n::t("Brush Preset picker")).click();
        h.run_steps(3);
        assert!(h.state().ui.brush_picker.is_some(), "the chip opens the picker");
        h.get_by_label(crate::i18n::t("Brush Preset picker")).click();
        h.run_steps(3);
        assert!(h.state().ui.brush_picker.is_none(), "the chip closes it");
    }
    let at = fractional_point(&h, 1.0);
    tap(&mut h, at, Modifiers::NONE, true);
    assert!(h.state().ui.brush_picker.is_none(), "no picker on a right-click");
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.has_selection), "the selection menu");
    assert!(document_state(&h) == before);
}

#[test]
fn brush_preset_menu_action_outside_the_picker_keeps_it_open() {
    let mut h = harness(Tool::Brush, true, 1.0);
    let before = document_state(&h);
    let at = fractional_point(&h, 1.0);
    tap(&mut h, at, Modifiers::NONE, true);
    let picker = h.ctx.memory(|m| m.area_rect(egui::Id::unique("canvas-brush-picker"))).unwrap();
    let name = h.state().session.tools.presets.first().unwrap().name.clone();
    let card = h.query_all_by_label(&name).find(|n| picker.contains(n.rect().center())).expect("visible preset card").rect();
    // Open by the right edge so the action lands outside the picker on a foreground menu.
    tap(&mut h, pos2(card.right() - 20.0, card.center().y), Modifiers::NONE, false);
    assert!(egui::Popup::is_any_open(&h.ctx), "preset menu at {card:?} in picker {picker:?}");
    let rename = h.get_by_label(crate::i18n::t("Rename Brush…"));
    assert!(!picker.contains(rename.rect().center()), "exercise the floating-menu exception outside the picker");
    rename.click();
    h.run_steps(3);
    assert!(h.state().ui.brush_picker.is_some());
    assert!(h.state().ui.brush_picker_list.renaming.is_some());
    assert!(document_state(&h) == before, "pixels, selection, layers and history are unchanged");
    dismiss_outside(&mut h, Popup::Brush);
    assert!(h.state().ui.brush_picker_list.renaming.is_none());
    assert!(document_state(&h) == before, "pixels, selection, layers and history are unchanged");
}
