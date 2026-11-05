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

/// The measured union-box gesture must commit both layer bounds as one undoable transform.
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

/// Both committed layer bounds must follow the preview homography; a translation or one-layer rotation fails.
#[test]
fn rotation_zone_uses_the_existing_transform_gesture() {
    let (mut app, ctx, layers) = fixture();
    let before = app.session.active().unwrap().doc.clone();
    let old = layers.map(|id| bounds(&app, id));
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
    let preview = photocraft_algo::transform::Homography::rect_to_quad(rotated.rect, rotated.quad).unwrap();
    let expected = old.map(|r| {
        let points = [[r.x0, r.y0], [r.x1, r.y0], [r.x1, r.y1], [r.x0, r.y1]].map(|[x, y]| preview.apply(f64::from(x), f64::from(y)));
        points
            .into_iter()
            .fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, (x, y)| [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)])
    });
    enter(&mut app, &ctx);
    assert_one_transform(&app, steps);
    assert_ne!(app.session.active().unwrap().doc, before, "Enter commits the rotation to the document");
    for ((id, previous), expected) in layers.into_iter().zip(old).zip(expected) {
        assert!((expected[2] - expected[0] - f64::from(previous.width())).abs() > 2.0, "this fixture distinguishes rotation from translation");
        let actual = bounds(&app, id);
        for (actual, expected) in [actual.x0, actual.y0, actual.x1, actual.y1].into_iter().zip(expected) {
            // The preview frame is continuous; committed raster bounds round to whole pixels.
            assert!((f64::from(actual) - expected).abs() <= 1.0, "layer {id:?}: committed bound {actual} does not match preview {expected}");
        }
    }
}

/// Non-unit zoom and a nonzero center expose missing document-to-screen conversion.
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

/// Geometric handles outside the active view must be unreachable through synthetic input.
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

/// Visibility and setting gates must reject entry even when the point geometrically hits a handle.
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

/// Secondary buttons and Space-pan retain their established input ownership.
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

/// An interior press must use Move without creating a transform session.
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

/// Disabling Move controls must not disable the explicit Free Transform command.
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

/// A malformed boolean must reject the complete patch before a valid sibling field changes.
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

/// Omission retains the setting, and later validation failure cannot partially apply a new value.
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

/// A malformed later sample must be rejected before the first valid Down can open a session.
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

/// A modal owns the whole gesture, including ordinary Move with transform controls disabled.
#[test]
fn modal_dialogs_do_not_start_move_transform_controls() {
    for command in ["file.new", "edit.preferences.general"] {
        let (mut app, ctx, _) = fixture();
        crate::menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        assert_eq!(app.ui.dialogs.len(), 1, "{command} opened its modal");
        let before = app.session.active().unwrap().doc.clone();
        let steps = app.session.active().unwrap().history.past_len();
        let selected = app.session.active().unwrap().selected_layers();
        let active = app.session.active().unwrap().active_layer;
        for controls in [true, false] {
            app.ui.tool_options.move_show_transform = controls;
            for from in [[800.0, 500.0], [200.0, 150.0]] {
                let to = [from[0] + 20.0, from[1] + 10.0];
                for (kind, at) in [("down", from), ("move", to), ("up", to)] {
                    pointer(&mut app, &ctx, kind, at);
                    assert!(app.ui.transform.is_none(), "{command} owns input before the Move handle entry");
                    assert!(app.drag.is_none(), "{command} must not start an ordinary Move drag");
                    assert!(app.layer_box.is_none());
                    assert!(app.guide_drag.is_none());
                }
                assert_eq!(app.session.active().unwrap().doc, before);
                assert_eq!(app.session.active().unwrap().history.past_len(), steps);
                assert_eq!(app.session.active().unwrap().selected_layers(), selected);
                assert_eq!(app.session.active().unwrap().active_layer, active);
                assert_eq!(app.ui.tool, Tool::Move);
                assert_eq!(app.ui.tool_options.move_show_transform, controls);
                assert_eq!(app.ui.dialogs.len(), 1, "the gesture leaves the modal open");
            }
        }
    }
}

/// A dialog opened after Down must not strand the Move drag or replace its starting point.
#[test]
fn ordinary_move_gesture_can_finish_after_a_dialog_opens() {
    let (mut app, ctx, layers) = fixture();
    let old = layers.map(|id| bounds(&app, id));
    let steps = app.session.active().unwrap().history.past_len();
    pointer(&mut app, &ctx, "down", [200.0, 150.0]);
    assert!(app.drag.is_some());
    accepted(&mut app, &ctx, "ui.dialog.open", json!({"kind": "about"}));
    let dialog = app.ui.dialogs.last().unwrap().id;
    pointer(&mut app, &ctx, "down", [250.0, 175.0]);
    assert_eq!(app.drag.as_ref().unwrap().start, [200.0, 150.0], "the modal rejects a replacement Down");
    pointer(&mut app, &ctx, "move", [220.0, 160.0]);
    pointer(&mut app, &ctx, "up", [220.0, 160.0]);
    assert!(app.drag.is_none());
    assert!(app.ui.transform.is_none());
    for (id, previous) in layers.into_iter().zip(old) {
        assert_eq!(bounds(&app, id), previous.translate(20, 10));
    }
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    assert_eq!(app.ui.dialogs.last().unwrap().id, dialog);
    accepted(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": dialog}));
    let after = app.session.active().unwrap().doc.clone();
    pointer(&mut app, &ctx, "move", [270.0, 190.0]);
    pointer(&mut app, &ctx, "up", [270.0, 190.0]);
    assert_eq!(app.session.active().unwrap().doc, after, "later input cannot resume the finished gesture");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
}

/// A modal may interrupt a handle drag, but Up still ends its gesture before Enter commits.
#[test]
fn transform_handle_gesture_can_finish_after_a_dialog_opens() {
    let (mut app, ctx, layers) = fixture();
    let before = app.session.active().unwrap().doc.clone();
    let old = layers.map(|id| bounds(&app, id));
    let steps = app.session.active().unwrap().history.past_len();
    pointer(&mut app, &ctx, "down", [800.0, 500.0]);
    let initial = app.ui.transform.as_ref().expect("the handle starts Free Transform").quad;
    accepted(&mut app, &ctx, "ui.dialog.open", json!({"kind": "about"}));
    let dialog = app.ui.dialogs.last().unwrap().id;
    pointer(&mut app, &ctx, "down", [200.0, 150.0]);
    assert_eq!(app.ui.transform.as_ref().unwrap().quad, initial, "the modal cannot move the preview on Down");
    pointer(&mut app, &ctx, "move", [700.0, 440.0]);
    pointer(&mut app, &ctx, "up", [700.0, 440.0]);
    let preview = app.ui.transform.as_ref().unwrap().quad;
    assert_ne!(preview, initial, "the existing handle gesture still updates its preview");
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert_eq!(app.ui.dialogs.last().unwrap().id, dialog);
    accepted(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": dialog}));
    pointer(&mut app, &ctx, "move", [650.0, 410.0]);
    pointer(&mut app, &ctx, "up", [650.0, 410.0]);
    assert_eq!(app.ui.transform.as_ref().unwrap().quad, preview, "later input cannot resume the released handle");
    enter(&mut app, &ctx);
    assert_one_transform(&app, steps);
    for (id, previous) in layers.into_iter().zip(old) {
        let scaled = bounds(&app, id);
        assert!(scaled.width() < previous.width() && scaled.height() < previous.height());
    }
}

/// Full-window dialogs with embedded previews must not forward ordinary Move gestures.
#[test]
fn modal_workspaces_without_tool_event_controllers_do_not_move_layers() {
    for command in ["filter.filterGallery", "filter.adaptiveWideAngle"] {
        let (mut app, ctx, layers) = fixture();
        app.run("layer.rasterize.layer", json!({"layer": layers[0].0})).unwrap();
        app.run("layer.select", json!({"layer": layers[0].0})).unwrap();
        crate::menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        assert_eq!(crate::distort_ui::modal_workspace(&app), Some(command));
        let before = app.session.active().unwrap().doc.clone();
        let steps = app.session.active().unwrap().history.past_len();
        for controls in [true, false] {
            app.ui.tool_options.move_show_transform = controls;
            for (kind, at) in [("down", [200.0, 150.0]), ("move", [220.0, 160.0]), ("up", [220.0, 160.0])] {
                pointer(&mut app, &ctx, kind, at);
                assert!(app.ui.transform.is_none());
                assert!(app.drag.is_none());
            }
            assert_eq!(app.session.active().unwrap().doc, before);
            assert_eq!(app.session.active().unwrap().history.past_len(), steps);
            assert_eq!(app.ui.tool, Tool::Move);
            assert_eq!(crate::distort_ui::modal_workspace(&app), Some(command));
        }
    }
}

/// Modal canvas samplers retain ownership even when Move and its controls are selected.
#[test]
fn move_pointer_still_samples_into_the_color_picker() {
    let (mut app, ctx, _) = fixture();
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    let id = crate::color_picker_ui::open(&mut app, "foreground");
    for (kind, at) in [("down", [200.0, 150.0]), ("move", [220.0, 160.0]), ("up", [220.0, 160.0])] {
        pointer(&mut app, &ctx, kind, at);
    }
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["color"], "#ff0000");
    assert!(app.ui.transform.is_none());
    assert!(app.drag.is_none());
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

/// A full-window editor receives its own stroke instead of a Move or Free Transform gesture.
#[test]
fn move_pointer_still_reaches_the_liquify_workspace() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 120, "height": 80, "depth": 8})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.sync_views();
    let ctx = egui::Context::default();
    accepted(&mut app, &ctx, "ui.set", json!({"tool": "Move", "moveShowTransform": true}));
    crate::menus::invoke(&mut app, &ctx, "filter.liquify", json!({})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    for (kind, at) in [("down", [50.0, 40.0]), ("move", [60.0, 40.0]), ("up", [60.0, 40.0])] {
        pointer(&mut app, &ctx, kind, at);
    }
    assert_eq!(app.distort.liquify.as_ref().unwrap().strokes.len(), 1);
    assert!(app.ui.transform.is_none());
    assert!(app.drag.is_none());
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

/// A sampling dialog owns its release without leaving an earlier transform drag armed.
#[test]
fn color_picker_release_ends_an_existing_transform_handle_drag() {
    let (mut app, ctx, layers) = fixture();
    let before = app.session.active().unwrap().doc.clone();
    let old = layers.map(|id| bounds(&app, id));
    let steps = app.session.active().unwrap().history.past_len();
    pointer(&mut app, &ctx, "down", [800.0, 500.0]);
    let initial = app.ui.transform.as_ref().unwrap().quad;
    pointer(&mut app, &ctx, "move", [700.0, 440.0]);
    let preview = app.ui.transform.as_ref().unwrap().quad;
    assert_ne!(preview, initial);
    accepted(&mut app, &ctx, "ui.dialog.open", json!({"kind": "colorPicker"}));
    let dialog = crate::color_picker_ui::top(&app).unwrap();
    pointer(&mut app, &ctx, "down", [200.0, 150.0]);
    pointer(&mut app, &ctx, "move", [220.0, 160.0]);
    assert_eq!(app.ui.dialog_mut(dialog).unwrap().fields["color"], "#ff0000");
    pointer(&mut app, &ctx, "up", [999.0, 690.0]);
    assert_eq!(app.ui.dialog_mut(dialog).unwrap().fields["color"], "#ff0000", "release does not resample");
    assert_eq!(app.ui.transform.as_ref().unwrap().quad, preview, "dialog-owned coordinates do not update the preview");
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    accepted(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": dialog}));
    pointer(&mut app, &ctx, "move", [650.0, 410.0]);
    pointer(&mut app, &ctx, "up", [650.0, 410.0]);
    assert_eq!(app.ui.transform.as_ref().unwrap().quad, preview, "later input needs a new press");
    enter(&mut app, &ctx);
    assert_one_transform(&app, steps);
    for (id, previous) in layers.into_iter().zip(old) {
        let scaled = bounds(&app, id);
        assert!(scaled.width() < previous.width() && scaled.height() < previous.height());
    }
}

/// ⌘ makes another tool the Move tool for the press: a modal blocks that layer drag too.
#[test]
fn modal_dialogs_block_a_command_held_move_from_another_tool() {
    let (mut app, ctx, _) = fixture();
    accepted(&mut app, &ctx, "ui.set", json!({"tool": "Brush"}));
    crate::menus::invoke(&mut app, &ctx, "file.new", json!({})).unwrap();
    assert_eq!(app.ui.dialogs.len(), 1);
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    for (kind, at) in [("down", [200.0, 150.0]), ("move", [220.0, 160.0]), ("up", [220.0, 160.0])] {
        accepted(&mut app, &ctx, "ui.pointer", json!({"command": true, "events": [{"kind": kind, "x": at[0], "y": at[1]}]}));
        assert!(app.drag.is_none(), "{kind}: the modal owns the ⌘ Move press");
    }
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert_eq!(app.ui.tool, Tool::Brush);
}

/// The smart guide segments `snap_ui::draw` paints on the canvas.
fn drawn_snap_lines(app: &mut PhotocraftApp) -> usize {
    let xf = crate::canvas::ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0)),
        zoom: 1.0,
        center: [500.0, 350.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| crate::snap_ui::draw(app, ui.painter(), &xf));
    output.textures_delta.clear();
    let mut stack: Vec<egui::Shape> = output.shapes.into_iter().map(|c| c.shape).collect();
    let mut lines = 0;
    while let Some(shape) = stack.pop() {
        match shape {
            egui::Shape::Vec(v) => stack.extend(v),
            egui::Shape::LineSegment { .. } => lines += 1,
            _ => {}
        }
    }
    lines
}

/// The Color Picker takes the release of a snapped handle drag: its alignment lines go with it.
#[test]
fn color_picker_release_clears_the_handle_drags_snap_lines() {
    let (mut app, ctx, _) = fixture();
    app.ui.extras.snap = true;
    app.ui.view.show.smart_guides = true;
    pointer(&mut app, &ctx, "down", [800.0, 500.0]);
    assert!(app.ui.transform.is_some());
    // 2 px from the document's centre (500, 350): the corner snaps there and the guides show.
    pointer(&mut app, &ctx, "move", [502.0, 352.0]);
    assert!(drawn_snap_lines(&mut app) > 0, "{:?}", app.prefs_rt.snap_lines);
    accepted(&mut app, &ctx, "ui.dialog.open", json!({"kind": "colorPicker"}));
    pointer(&mut app, &ctx, "up", [999.0, 690.0]);
    assert_eq!(drawn_snap_lines(&mut app), 0, "the released drag draws no alignment lines");
    assert!(app.prefs_rt.snap.is_none());
}

/// Every sampler that takes the primary release ends a transform handle drag begun before it,
/// not only the Color Picker: the Curves eyedroppers and Color Range too. A move after the
/// sampler closes needs a new press.
#[test]
fn other_samplers_releases_end_an_existing_transform_handle_drag() {
    for sampler in ["curves", "colorRange"] {
        let (mut app, ctx, _) = fixture();
        pointer(&mut app, &ctx, "down", [800.0, 500.0]);
        pointer(&mut app, &ctx, "move", [700.0, 440.0]);
        let preview = app.ui.transform.as_ref().unwrap().quad;
        let dialog = match sampler {
            "curves" => {
                let id = crate::adjust_dialog::open(&mut app, "image.adjustments.curves").unwrap();
                let d = app.ui.dialog_mut(id).unwrap();
                let base = crate::filter_dialog::params_of(&d.fields);
                d.fields.insert("__curvePicker".into(), json!("black"));
                d.fields.insert("__curvePickerBase".into(), base);
                assert!(crate::adjust_dialog::picker_armed(&app), "{sampler}");
                id
            }
            _ => crate::color_range_ui::open(&mut app),
        };
        pointer(&mut app, &ctx, "down", [200.0, 150.0]);
        pointer(&mut app, &ctx, "up", [999.0, 690.0]);
        assert_eq!(app.ui.transform.as_ref().unwrap().quad, preview, "{sampler}: the sampler's coordinates do not move the box");
        app.ui.dialogs.retain(|d| d.id != dialog);
        pointer(&mut app, &ctx, "move", [650.0, 410.0]);
        assert_eq!(app.ui.transform.as_ref().unwrap().quad, preview, "{sampler}: later moves need a new press");
    }
}

/// The Color Picker takes the release of an ordinary Move drag: the drag ends where it last was, and
/// moving after the picker closes needs a new press.
#[test]
fn color_picker_release_ends_an_ordinary_move_drag() {
    let (mut app, ctx, layers) = fixture();
    let old = layers.map(|id| bounds(&app, id));
    let steps = app.session.active().unwrap().history.past_len();
    pointer(&mut app, &ctx, "down", [200.0, 150.0]);
    pointer(&mut app, &ctx, "move", [220.0, 160.0]);
    assert!(app.drag.is_some());
    accepted(&mut app, &ctx, "ui.dialog.open", json!({"kind": "colorPicker"}));
    let dialog = crate::color_picker_ui::top(&app).unwrap();
    pointer(&mut app, &ctx, "up", [999.0, 690.0]);
    assert!(app.drag.is_none(), "the picker's release ends the drag");
    accepted(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": dialog}));
    pointer(&mut app, &ctx, "move", [270.0, 190.0]);
    pointer(&mut app, &ctx, "up", [270.0, 190.0]);
    for (id, previous) in layers.into_iter().zip(old) {
        assert_eq!(bounds(&app, id), previous.translate(20, 10), "moved only as far as before the picker opened");
    }
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
}
