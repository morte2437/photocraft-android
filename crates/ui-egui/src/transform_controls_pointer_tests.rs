//! Move tool transform controls through the control channel (#11105).

use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::control::{ControlRequest, Outcome};
use crate::state::Tool;

fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (request, _receiver) = ControlRequest::new(method, params);
    let Outcome::Done(reply) = crate::control::handle(app, ctx, &request) else { panic!("expected an immediate {method} reply") };
    reply
}

fn accepted(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) {
    let reply = call(app, ctx, method, params);
    assert_eq!(reply["ok"], true, "{method}: {reply}");
}

/// Two selected shapes share the measured controls frame (100,100)..(800,500).
fn fixture() -> (PhotocraftApp, egui::Context, [LayerId; 2]) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1000, "height": 700, "background": "white"})).unwrap();
    let mut layers = Vec::new();
    for rect in [[100, 100, 200, 100], [600, 400, 200, 100]] {
        app.run("shape.create", json!({"kind": "rect", "rect": rect, "fill": "#ff0000"})).unwrap();
        layers.push(app.session.active().unwrap().active_layer.unwrap());
    }
    app.run("layer.select", json!({"layer": layers[0].0, "mode": "add"})).unwrap();
    app.sync_views();
    app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
    app.ui.tool_options.move_auto_select = false;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let ctx = egui::Context::default();
    accepted(&mut app, &ctx, "ui.set", json!({"tool": "Move", "moveShowTransform": true, "zoom": 1.0, "center": [500, 350]}));
    (app, ctx, [layers[0], layers[1]])
}

fn bounds(app: &PhotocraftApp, id: LayerId) -> Rect {
    let state = app.session.active().unwrap();
    photocraft_engine::transform_cmds::transform_bounds(&state.doc, state.doc.layer(id).unwrap())
}

fn pointer(app: &mut PhotocraftApp, ctx: &egui::Context, kind: &str, at: [f64; 2]) {
    accepted(app, ctx, "ui.pointer", json!({"events": [{"kind": kind, "x": at[0], "y": at[1]}]}));
}

fn enter(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let (request, _receiver) = ControlRequest::new("ui.key", json!({"key": "Enter"}));
    assert!(matches!(crate::control::handle(app, ctx, &request), Outcome::AfterInput));
    let events = app.take_synthetic_step();
    assert!(!events.is_empty());
    let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
    output.textures_delta.clear(); // This CPU-only helper deliberately has no texture uploader.
}

fn assert_one_transform(app: &PhotocraftApp, steps: usize) {
    assert!(app.ui.transform.is_none(), "Enter closes Free Transform");
    let state = app.session.active().unwrap();
    assert_eq!(state.history.past_len(), steps + 1);
    assert_eq!(state.history.entries().last().map(String::as_str), Some("Free Transform"));
}

#[test]
fn corner_drag_scales_both_selected_layers_in_one_free_transform_step() {
    let (mut app, ctx, layers) = fixture();
    let before = app.session.active().unwrap().doc.clone();
    let old = layers.map(|id| bounds(&app, id));
    let steps = app.session.active().unwrap().history.past_len();
    pointer(&mut app, &ctx, "down", [800.0, 500.0]);
    let transform = app.ui.transform.as_ref().expect("Move controls open Free Transform");
    assert_eq!(transform.rect, [100.0, 100.0, 800.0, 500.0]);
    assert_eq!(transform.layers, layers.map(|id| id.0));
    pointer(&mut app, &ctx, "move", [700.0, 440.0]);
    pointer(&mut app, &ctx, "up", [700.0, 440.0]);
    assert!(app.ui.transform.is_some(), "Up ends the gesture, not the transform session");
    assert_eq!(app.session.active().unwrap().doc, before, "the preview has not changed the document");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    enter(&mut app, &ctx);
    assert_one_transform(&app, steps);
    for (id, previous) in layers.into_iter().zip(old) {
        let scaled = bounds(&app, id);
        assert!(scaled.width() < previous.width() && scaled.height() < previous.height(), "both layers scale: {previous:?} -> {scaled:?}");
    }
    let after = app.session.active().unwrap().doc.clone();
    assert!(app.session.undo());
    assert_eq!(app.session.active().unwrap().doc, before, "one Undo restores both layers");
    assert!(app.session.redo());
    assert_eq!(app.session.active().unwrap().doc, after, "one Redo restores the transform");
}

#[test]
fn rotation_zone_uses_the_existing_transform_gesture() {
    let (mut app, ctx, layers) = fixture();
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    // Fourteen points beyond the corner: outside its 12pt handle, inside the rotation band.
    pointer(&mut app, &ctx, "down", [814.0, 500.0]);
    let initial = app.ui.transform.as_ref().expect("rotation zone opens Free Transform").quad;
    pointer(&mut app, &ctx, "move", [800.0, 514.0]);
    pointer(&mut app, &ctx, "up", [800.0, 514.0]);
    let rotated = app.ui.transform.as_ref().unwrap();
    assert_ne!(rotated.quad, initial);
    assert_eq!(rotated.layers, layers.map(|id| id.0));
    assert!((rotated.quad[1][1] - rotated.quad[0][1]).abs() > 1.0, "the upper edge rotates");
    enter(&mut app, &ctx);
    assert_one_transform(&app, steps);
    assert_ne!(app.session.active().unwrap().doc, before, "Enter commits the rotation to the document");
}

#[test]
fn zoomed_control_channel_press_starts_the_visible_corner() {
    let (mut app, ctx, layers) = fixture();
    accepted(&mut app, &ctx, "ui.set", json!({"zoom": 2.0, "center": [500, 350]}));
    let old = layers.map(|id| bounds(&app, id));
    let steps = app.session.active().unwrap().history.past_len();
    pointer(&mut app, &ctx, "down", [800.0, 500.0]);
    assert_eq!(app.ui.transform.as_ref().expect("the projected corner is visible at 200%").layers, layers.map(|id| id.0));
    pointer(&mut app, &ctx, "move", [700.0, 440.0]);
    pointer(&mut app, &ctx, "up", [700.0, 440.0]);
    enter(&mut app, &ctx);
    assert_one_transform(&app, steps);
    for (id, previous) in layers.into_iter().zip(old) {
        let scaled = bounds(&app, id);
        assert!(scaled.width() < previous.width() && scaled.height() < previous.height());
    }
}

#[test]
fn off_canvas_handles_do_not_start_transform_controls() {
    for (zoom, center) in [(3.0, [500, 350]), (1.0, [0, 0])] {
        let (mut app, ctx, _) = fixture();
        accepted(&mut app, &ctx, "ui.set", json!({"zoom": zoom, "center": center}));
        let xf = crate::canvas::ViewXform::active(&app).unwrap();
        let projected_corner = xf.to_screen(800.0, 500.0);
        assert!(projected_corner.is_finite() && !xf.rect.contains(projected_corner), "the actual corner is outside the visible canvas");
        let before = app.session.active().unwrap().doc.clone();
        let steps = app.session.active().unwrap().history.past_len();
        pointer(&mut app, &ctx, "down", [800.0, 500.0]);
        assert!(app.ui.transform.is_none(), "off-canvas press cannot activate a hidden handle");
        pointer(&mut app, &ctx, "up", [800.0, 500.0]);
        assert!(app.ui.transform.is_none());
        assert_eq!(app.session.active().unwrap().doc, before);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    }
}

#[test]
fn disabled_hidden_and_missed_controls_do_not_open_a_transform() {
    for (enabled, extras, at) in [(false, true, [800.0, 500.0]), (true, false, [800.0, 500.0]), (true, true, [900.0, 600.0])] {
        let (mut app, ctx, _) = fixture();
        accepted(&mut app, &ctx, "ui.set", json!({"moveShowTransform": enabled}));
        app.ui.view.extras = extras;
        pointer(&mut app, &ctx, "down", at);
        assert!(app.ui.transform.is_none(), "enabled={enabled}, Extras={extras}, at={at:?}");
        pointer(&mut app, &ctx, "up", at);
        assert!(app.ui.transform.is_none());
    }
}

#[test]
fn nonprimary_and_space_presses_do_not_take_over_move_controls() {
    for params in [json!({"button": "right"}), json!({"button": "middle"}), json!({"space": true})] {
        let (mut app, ctx, _) = fixture();
        let mut params = params;
        params["events"] = json!([{"kind": "down", "x": 800, "y": 500}]);
        accepted(&mut app, &ctx, "ui.pointer", params);
        assert!(app.ui.transform.is_none());
    }
}

#[test]
fn interior_drag_keeps_the_ordinary_move_path() {
    let (mut app, ctx, layers) = fixture();
    let old = layers.map(|id| bounds(&app, id));
    let steps = app.session.active().unwrap().history.past_len();
    pointer(&mut app, &ctx, "down", [200.0, 150.0]);
    assert!(app.ui.transform.is_none());
    pointer(&mut app, &ctx, "move", [220.0, 160.0]);
    pointer(&mut app, &ctx, "up", [220.0, 160.0]);
    for (id, previous) in layers.into_iter().zip(old) {
        assert_eq!(bounds(&app, id), previous.translate(20, 10));
    }
    assert!(app.ui.transform.is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
}

#[test]
fn explicit_free_transform_still_works_with_move_controls_disabled() {
    let (mut app, ctx, _) = fixture();
    accepted(&mut app, &ctx, "ui.set", json!({"moveShowTransform": false}));
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    pointer(&mut app, &ctx, "down", [800.0, 500.0]);
    pointer(&mut app, &ctx, "move", [700.0, 440.0]);
    pointer(&mut app, &ctx, "up", [700.0, 440.0]);
    enter(&mut app, &ctx);
    assert_one_transform(&app, steps);
}

#[test]
fn invalid_move_show_transform_is_atomic_with_other_ui_fields() {
    for invalid in [json!("true"), json!(1), json!(null), json!([]), json!({})] {
        let (mut app, ctx, _) = fixture();
        accepted(&mut app, &ctx, "ui.set", json!({"moveShowTransform": false}));
        let before = serde_json::to_value(&app.ui).unwrap();
        let doc = app.session.active().unwrap().doc.clone();
        let steps = app.session.active().unwrap().history.past_len();
        let reply = call(&mut app, &ctx, "ui.set", json!({"tool": "Brush", "zoom": 2.0, "moveShowTransform": invalid}));
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(reply["error"].as_str().unwrap().contains("moveShowTransform"), "{reply}");
        assert_eq!(serde_json::to_value(&app.ui).unwrap(), before, "no UI field changed");
        assert_eq!(app.session.active().unwrap().doc, doc);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    }
}

#[test]
fn move_show_transform_is_retained_when_omitted_and_waits_for_all_validation() {
    let (mut app, ctx, _) = fixture();
    accepted(&mut app, &ctx, "ui.set", json!({"moveShowTransform": false}));
    let reply = call(&mut app, &ctx, "ui.set", json!({"moveShowTransform": true, "zoom": "bad"}));
    assert_eq!(reply["ok"], false);
    assert!(!app.ui.tool_options.move_show_transform);
    accepted(&mut app, &ctx, "ui.set", json!({"moveShowTransform": true}));
    accepted(&mut app, &ctx, "ui.set", json!({"tool": "Brush"}));
    assert_eq!(app.ui.tool, Tool::Brush);
    assert!(app.ui.tool_options.move_show_transform);
}

#[test]
fn invalid_later_pointer_event_does_not_begin_a_transform() {
    let (mut app, ctx, _) = fixture();
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    let reply = call(
        &mut app,
        &ctx,
        "ui.pointer",
        json!({"events": [
            {"kind": "down", "x": 800, "y": 500},
            {"kind": "unknown", "x": 700, "y": 440}
        ]}),
    );
    assert_eq!(reply["ok"], false);
    assert!(app.ui.transform.is_none());
    assert!(app.drag.is_none());
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}
