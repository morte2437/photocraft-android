//! Free Transform's rules for the Layers panel's selection (#11101): the Background among several
//! selected layers refuses with the locked-layer alert, and any change to the selected layers
//! while the box is open applies the box first, as in Photoshop.

use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::transform_tool::LOCKED;

/// A 200 × 200 document on a white Background: "A" with a red block at x 10..50, y 10..30, and
/// "B" with a blue one at x 100..140, y 100..120. Only B is selected.
fn app() -> (PhotocraftApp, LayerId, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    let bg = app.session.active().unwrap().doc.layers[0].id;
    let mut block = |name: &str, rect: [i32; 4], color: &str| {
        let id = LayerId(app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap());
        app.run("select.rect", json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3]})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        id
    };
    let a = block("A", [10, 10, 40, 20], "#ff0000");
    let b = block("B", [100, 100, 40, 20], "#0000ff");
    (app, bg, a, b)
}

fn bounds(app: &PhotocraftApp, id: LayerId) -> Rect {
    let st = app.session.active().unwrap();
    photocraft_engine::transform_cmds::transform_bounds(&st.doc, st.doc.layer(id).unwrap())
}

fn select(app: &mut PhotocraftApp, id: LayerId, mode: &str) {
    app.run("layer.select", json!({"layer": id.0, "mode": mode})).unwrap();
}

fn invoke(app: &mut PhotocraftApp, id: &str) -> Result<(), String> {
    crate::menus::invoke(app, &egui::Context::default(), id, json!({})).map(|_| ())
}

fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.ui.dialogs.iter().filter(|d| d.kind == crate::state::DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(String::from)).collect()
}

/// Drag the open box `dx` px right, with nearest neighbour so the blocks keep their edges.
fn nudge(app: &mut PhotocraftApp, dx: f64) {
    let t = app.ui.transform.as_mut().expect("a box is open");
    t.interpolation = "nearest".into();
    t.quad = t.quad.map(|[x, y]| [x + dx, y]);
}

#[test]
fn the_background_among_several_layers_shows_the_locked_alert_and_opens_no_box() {
    for id in ["edit.freeTransform", "edit.transform.scale", "edit.transform.rotate"] {
        let (mut app, bg, a, _) = app();
        select(&mut app, a, "replace");
        select(&mut app, bg, "add");
        let before = app.session.active().unwrap().doc.clone();
        assert_eq!(invoke(&mut app, id), Err(LOCKED.into()), "{id}");
        assert!(app.ui.transform.is_none(), "{id}: no box opens");
        assert_eq!(alerts(&app), [LOCKED], "{id}: the same alert as a position-locked layer");
        assert!(app.session.active().unwrap().doc == before, "{id}: nothing moved");
    }
}

#[test]
fn the_background_is_refused_whichever_of_them_is_active() {
    let (mut app, bg, a, _) = app();
    select(&mut app, a, "replace");
    select(&mut app, bg, "add");
    select(&mut app, a, "add");
    assert_eq!(app.session.active().unwrap().active_layer, Some(a));
    assert_eq!(invoke(&mut app, "edit.freeTransform"), Err(LOCKED.into()));
    assert!(app.ui.transform.is_none());
}

#[test]
fn the_engines_default_targets_refuse_the_background_but_an_explicit_list_is_kept() {
    let (mut app, bg, a, b) = app();
    select(&mut app, a, "replace");
    select(&mut app, bg, "add");
    let err = app.run("edit.transform", json!({"matrix": [1, 0, 0, 1, 5, 0]})).unwrap_err();
    assert!(err.contains("locked"), "{err}");
    // A recorded or scripted step naming its layers moves those, as before.
    app.run("edit.transform", json!({"layers": [a.0, b.0], "matrix": [1, 0, 0, 1, 5, 0]})).unwrap();
    assert_eq!(bounds(&app, a), Rect::new(15, 10, 55, 30));
    assert_eq!(bounds(&app, b), Rect::new(105, 100, 145, 120));
}

#[test]
fn several_layers_without_the_background_still_open_one_box() {
    let (mut app, _, a, b) = app();
    select(&mut app, a, "add");
    invoke(&mut app, "edit.freeTransform").unwrap();
    let t = app.ui.transform.as_ref().unwrap();
    assert_eq!(t.layers, [a.0, b.0]);
    assert!(alerts(&app).is_empty());
}

#[test]
fn taking_a_layer_out_of_the_selection_applies_the_box_to_both_first() {
    // The reporter's case: the box over A and B (B active), then ⌘-click A in the Layers panel.
    let (mut app, _, a, b) = app();
    select(&mut app, a, "replace");
    select(&mut app, b, "add");
    assert_eq!(app.session.active().unwrap().active_layer, Some(b));
    invoke(&mut app, "edit.freeTransform").unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    nudge(&mut app, 10.0);
    select(&mut app, a, "toggle");
    assert_eq!(app.session.active().unwrap().active_layer, Some(b), "the active layer didn't change");
    crate::transform_tool::end_if_left(&mut app);
    assert!(app.ui.transform.is_none(), "the box is applied and not opened again");
    assert_eq!(bounds(&app, a), Rect::new(20, 10, 60, 30), "A moved with the box");
    assert_eq!(bounds(&app, b), Rect::new(110, 100, 150, 120));
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 1, "one Free Transform step");
    assert_eq!(st.selected_layers(), [b], "then A leaves the selection");
    // Nudging B now moves B alone: the old box is gone.
    invoke(&mut app, "edit.freeTransform").unwrap();
    assert!(app.ui.transform.as_ref().unwrap().layers.is_empty(), "a box over B alone");
}

#[test]
fn adding_a_layer_to_the_selection_applies_the_box_first() {
    let (mut app, _, a, b) = app();
    invoke(&mut app, "edit.freeTransform").unwrap();
    nudge(&mut app, 10.0);
    // ⌘-click A, then make B active again: the set changed though the active layer is B.
    select(&mut app, a, "add");
    select(&mut app, b, "add");
    assert_eq!(app.session.active().unwrap().active_layer, Some(b));
    crate::transform_tool::end_if_left(&mut app);
    assert!(app.ui.transform.is_none());
    assert_eq!(bounds(&app, b), Rect::new(110, 100, 150, 120), "B moved");
    assert_eq!(bounds(&app, a), Rect::new(10, 10, 50, 30), "A wasn't in the box");
    assert_eq!(app.session.active().unwrap().selected_layers(), [a, b]);
}

#[test]
fn an_unchanged_selection_keeps_the_box_open() {
    let (mut app, _, a, b) = app();
    select(&mut app, a, "add");
    invoke(&mut app, "edit.freeTransform").unwrap();
    nudge(&mut app, 10.0);
    // Selecting what is selected already, or editing a moving layer, isn't a change.
    select(&mut app, a, "add");
    app.run("layer.setProps", json!({"layer": b.0, "opacity": 0.5})).unwrap();
    crate::transform_tool::follow_edits(&mut app);
    crate::transform_tool::end_if_left(&mut app);
    assert!(app.ui.transform.is_some(), "the box stays open");
}

#[test]
fn the_background_alone_keeps_todays_box() {
    let (mut app, bg, _, _) = app();
    select(&mut app, bg, "replace");
    let r = invoke(&mut app, "edit.freeTransform");
    assert_ne!(r, Err(LOCKED.into()), "not the several-layers alert");
    assert!(alerts(&app).is_empty());
}

#[test]
fn a_box_over_a_linked_layer_stays_open_while_the_selection_stays() {
    // The box over A also moves B, linked to it (#11217); the selection is still A alone, so
    // nothing changed and the box stays open.
    let (mut app, _, a, b) = app();
    select(&mut app, a, "add");
    app.run("layer.linkLayers", json!({})).unwrap();
    select(&mut app, a, "replace");
    invoke(&mut app, "edit.freeTransform").unwrap();
    assert_eq!(app.ui.transform.as_ref().unwrap().layers, [a.0, b.0], "the box moves the link group");
    nudge(&mut app, 10.0);
    crate::transform_tool::end_if_left(&mut app);
    assert!(app.ui.transform.is_some(), "the box stays open");
    // Adding B to the selection is a change, though the box already moves B.
    select(&mut app, b, "add");
    select(&mut app, a, "add");
    crate::transform_tool::end_if_left(&mut app);
    assert!(app.ui.transform.is_none(), "the box is applied");
    assert_eq!(bounds(&app, a), Rect::new(20, 10, 60, 30));
    assert_eq!(bounds(&app, b), Rect::new(110, 100, 150, 120));
}

#[test]
fn taking_a_child_of_a_selected_group_out_of_the_selection_applies_the_box() {
    // The box over group G moves its child A whether A is selected or not; ⌘-clicking A out of
    // the selection still changes the selection, so the box is applied first.
    let (mut app, _, a, b) = app();
    select(&mut app, a, "replace");
    let g = LayerId(app.run("layer.new.groupFromLayers", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap());
    select(&mut app, g, "replace");
    select(&mut app, a, "add");
    select(&mut app, b, "add");
    select(&mut app, g, "add");
    invoke(&mut app, "edit.freeTransform").unwrap();
    nudge(&mut app, 10.0);
    select(&mut app, a, "toggle");
    assert_eq!(app.session.active().unwrap().active_layer, Some(g));
    crate::transform_tool::end_if_left(&mut app);
    assert!(app.ui.transform.is_none(), "the box is applied and not opened again");
    assert_eq!(bounds(&app, a), Rect::new(20, 10, 60, 30), "A moved with its group");
    assert_eq!(bounds(&app, b), Rect::new(110, 100, 150, 120));
}

/// An unselected layer "C" under the frame's bottom-right handle (x, y 180..200), so a press there
/// that reached the Move tool's Auto-Select would select it.
fn corner_layer(app: &mut PhotocraftApp) -> LayerId {
    let c = LayerId(app.run("layer.new.layer", json!({"name": "C"})).unwrap()["layer"].as_u64().unwrap());
    app.run("select.rect", json!({"x": 180, "y": 180, "width": 20, "height": 20})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#00ff00"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    c
}

#[test]
fn a_refused_handle_press_on_the_canvas_reaches_no_move_tool() {
    // The Background among the selected layers: a press on the Move tool's transform handle shows
    // the locked-layer alert and goes nowhere else. Auto-Select (on) used to take it and select
    // C under the handle, a click and a drag alike.
    use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
    for drag in [false, true] {
        let (mut app, bg, a, _) = app();
        corner_layer(&mut app);
        select(&mut app, a, "replace");
        select(&mut app, bg, "add");
        app.ui.tool = crate::state::Tool::Move;
        app.ui.tool_options.move_show_transform = true;
        app.ui.tool_options.move_auto_select = true;
        app.sync_views();
        let mut view = app.ui.views[0].clone();
        view.zoom = 1.0;
        view.center = [100.0, 100.0];
        view.fit_pending = false;
        view.fill_pending = false;
        let rect = egui::Rect::from_min_size(Pos2::ZERO, vec2(300.0, 300.0));
        let ctx = egui::Context::default();
        let mut frame = |app: &mut PhotocraftApp, events: Vec<Event>| {
            let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                view = crate::canvas::canvas_view(app, ui, 0, rect, view.clone(), true);
            });
            out.textures_delta.clear();
        };
        let press = |pos: Pos2, pressed: bool| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        // The frame holds the Background, the whole canvas: its bottom-right handle is at doc
        // (200, 200), screen (250, 250); the press is inside its grab area, over C.
        let at = Pos2::new(246.0, 246.0);
        let before = app.session.active().unwrap().doc.clone();
        let selected = app.session.active().unwrap().selected_layers();
        frame(&mut app, vec![Event::PointerMoved(at)]);
        frame(&mut app, vec![press(at, true)]);
        let mut end = at;
        if drag {
            for i in 1..=6 {
                end = at + vec2(-8.0 * i as f32, -8.0 * i as f32);
                frame(&mut app, vec![Event::PointerMoved(end)]);
            }
        }
        frame(&mut app, vec![press(end, false)]);
        frame(&mut app, vec![]);
        assert!(app.ui.transform.is_none(), "drag {drag}: no box opens");
        assert!(alerts(&app).iter().any(|m| m == "Could not transform because the layer is locked."), "drag {drag}: {:?}", alerts(&app));
        let st = app.session.active().unwrap();
        assert_eq!(st.selected_layers(), selected, "drag {drag}: the selection is unchanged");
        assert!(st.doc == before, "drag {drag}: the document is unchanged");
    }
}

#[test]
fn a_refused_handle_press_over_the_control_channel_reaches_no_move_tool() {
    // `ui.pointer` takes the same path: the alert, and the press is not forwarded to the Move tool.
    use crate::control::{ControlRequest, Outcome};
    let (mut app, bg, a, _) = app();
    corner_layer(&mut app);
    select(&mut app, a, "replace");
    select(&mut app, bg, "add");
    app.sync_views();
    app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 300.0));
    app.ui.tool_options.move_auto_select = true;
    let ctx = egui::Context::default();
    let call = |app: &mut PhotocraftApp, method: &str, params: serde_json::Value| {
        let (request, _receiver) = ControlRequest::new(method, params);
        let Outcome::Done(reply) = crate::control::handle(app, &ctx, &request) else { panic!("expected an immediate {method} reply") };
        assert_eq!(reply["ok"], true, "{method}: {reply}");
    };
    call(&mut app, "ui.set", json!({"tool": "Move", "moveShowTransform": true, "zoom": 1.0, "center": [100, 100]}));
    let before = app.session.active().unwrap().doc.clone();
    let selected = app.session.active().unwrap().selected_layers();
    call(
        &mut app,
        "ui.pointer",
        json!({"events": [{"kind": "down", "x": 196, "y": 196}, {"kind": "move", "x": 180, "y": 180}, {"kind": "up", "x": 180, "y": 180}]}),
    );
    assert!(app.ui.transform.is_none(), "no box opens");
    assert!(alerts(&app).iter().any(|m| m == "Could not transform because the layer is locked."), "{:?}", alerts(&app));
    let st = app.session.active().unwrap();
    assert_eq!(st.selected_layers(), selected, "the selection is unchanged");
    assert!(st.doc == before, "the document is unchanged");
}
