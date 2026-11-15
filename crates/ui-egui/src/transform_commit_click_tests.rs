//! Free Transform commit rules for clicks (#11100): a click outside the box on the image keeps the
//! box open, a click on the pasteboard commits, a double-click inside the box commits and one
//! outside does nothing.

use egui::{Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use photocraft_doc::LayerId;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform, tool_event};

/// A 200 × 200 document with a red block (x 20..60, y 20..60) on layer "Paint", Free Transform
/// open on it, and the box moved 10 px to the right as in the measurement.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    let paint = LayerId(app.run("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.select", json!({"layer": paint.0})).unwrap();
    app.run("select.rect", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
    let t = app.ui.transform.as_mut().unwrap();
    t.quad = t.quad.map(|[x, y]| [x + 10.0, y]);
    t.pivot[0] += 10.0;
    app
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn click(app: &mut PhotocraftApp, p: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: p[0], y: p[1] }, Modifiers::NONE);
}

#[test]
fn a_click_outside_the_box_on_the_image_keeps_it_open() {
    let mut app = app();
    let (before, quad) = (steps(&app), app.ui.transform.as_ref().unwrap().quad);
    click(&mut app, [150.0, 150.0]);
    assert!(app.ui.transform.is_some(), "the box stays open");
    assert_eq!(app.ui.transform.as_ref().unwrap().quad, quad, "and unchanged");
    assert_eq!(steps(&app), before);
}

#[test]
fn a_jittery_click_outside_the_box_leaves_it_unchanged() {
    let mut app = app();
    let (quad, pivot) = (app.ui.transform.as_ref().unwrap().quad, app.ui.transform.as_ref().unwrap().pivot);
    tool_event(&mut app, ToolEvent::Down { x: 150.0, y: 150.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 151.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 151.0 }, Modifiers::NONE);
    let t = app.ui.transform.as_ref().unwrap();
    assert_eq!((t.quad, t.pivot), (quad, pivot));
}

#[test]
fn a_click_on_the_pasteboard_commits_one_free_transform_step() {
    for p in [[-30.0, 100.0], [100.0, -30.0], [230.0, 100.0], [100.0, 230.0]] {
        let mut app = app();
        let before = steps(&app);
        click(&mut app, p);
        assert!(app.ui.transform.is_none(), "{p:?}");
        assert_eq!(steps(&app), before + 1, "{p:?}");
        assert_eq!(app.session.active().unwrap().history.entries().last().map(ToString::to_string).as_deref(), Some("Free Transform"));
    }
}

/// #11100, measured in the reference application: a click outside the box on another layer's
/// pixels commits one Free Transform step, and the transformed layer stays the only selected one
/// (the click doesn't pick the layer under it).
#[test]
fn a_click_on_another_layers_pixels_commits_and_keeps_the_selection() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    // "Other" has a blue block at x 120..160, y 120..160; "Paint" a red one at x 20..60, y 20..60.
    for (name, at, color) in [("Other", 120, "#0000ff"), ("Paint", 20, "#ff0000")] {
        let id = app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.select", json!({"layer": id})).unwrap();
        app.run("select.rect", json!({"x": at, "y": at, "width": 40, "height": 40})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
    }
    let paint = app.session.active().unwrap().active_layer.unwrap();
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
    let t = app.ui.transform.as_mut().unwrap();
    t.quad = t.quad.map(|[x, y]| [x + 10.0, y]);
    let before = steps(&app);
    click(&mut app, [140.0, 140.0]);
    assert!(app.ui.transform.is_none(), "committed");
    assert_eq!(steps(&app), before + 1);
    assert_eq!(app.session.active().unwrap().history.entries().last().map(ToString::to_string).as_deref(), Some("Free Transform"));
    assert_eq!(app.session.active().unwrap().selected_layers(), vec![paint], "the selection doesn't change");
}

/// A layer linked to the transformed one after the box opened moves with the commit
/// (`edit.transform` takes the link group along), so a click on its pixels is not a click on
/// another layer: the box stays open.
#[test]
fn a_click_on_a_layer_linked_after_the_box_opened_keeps_it_open() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    let mut ids = Vec::new();
    for (name, at, color) in [("Other", 120, "#0000ff"), ("Paint", 20, "#ff0000")] {
        let id = app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.select", json!({"layer": id})).unwrap();
        app.run("select.rect", json!({"x": at, "y": at, "width": 40, "height": 40})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        ids.push(LayerId(id));
    }
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
    let t = app.ui.transform.as_mut().unwrap();
    t.quad = t.quad.map(|[x, y]| [x + 10.0, y]);
    app.session
        .edit("Link Layers", |doc, _| {
            for id in &ids {
                doc.layer_mut(*id).unwrap().link_group = Some(7);
            }
            Ok(())
        })
        .unwrap();
    click(&mut app, [140.0, 140.0]);
    assert!(app.ui.transform.is_some(), "the linked layer moves with the box: the click doesn't commit");
}

/// Only the frontmost layer under the click counts: where a grouped layer (not measured, so the
/// box stays open there) covers a top-level layer's pixels, the click doesn't commit through the
/// covered layer.
#[test]
fn a_click_on_a_grouped_layer_covering_another_layer_keeps_the_box_open() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    // "Paint" has a red block at x 20..60, y 20..60; "Other" a blue one at x 120..160, y 120..160,
    // covered by a green one on "Cover", which then goes into a group.
    let mut ids = Vec::new();
    for (name, at, color) in [("Paint", 20, "#ff0000"), ("Other", 120, "#0000ff"), ("Cover", 120, "#00ff00")] {
        let id = app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap();
        app.run("layer.select", json!({"layer": id})).unwrap();
        app.run("select.rect", json!({"x": at, "y": at, "width": 40, "height": 40})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        ids.push(id);
    }
    app.run("layer.groupLayers", json!({})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    assert!(doc.layers.iter().all(|l| l.id.0 != ids[2]), "Cover is grouped");
    assert!(doc.layers.iter().any(|l| l.id.0 == ids[1]), "Other is top-level");
    app.run("layer.select", json!({"layer": ids[0]})).unwrap();
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
    let t = app.ui.transform.as_mut().unwrap();
    t.quad = t.quad.map(|[x, y]| [x + 10.0, y]);
    let before = steps(&app);
    click(&mut app, [140.0, 140.0]);
    assert!(app.ui.transform.is_some(), "the box stays open");
    assert_eq!(steps(&app), before);
}

/// The press decides where a click lands: one on the image that jitters past the canvas edge
/// before the release keeps the box open, one on the pasteboard that jitters onto the image commits.
#[test]
fn a_click_counts_where_it_was_pressed() {
    let mut app = app();
    let (before, quad) = (steps(&app), app.ui.transform.as_ref().unwrap().quad);
    tool_event(&mut app, ToolEvent::Down { x: 199.5, y: 150.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 200.5, y: 150.0 }, Modifiers::NONE);
    assert!(app.ui.transform.is_some(), "pressed on the image: the box stays open");
    assert_eq!(app.ui.transform.as_ref().unwrap().quad, quad);
    assert_eq!(steps(&app), before);

    tool_event(&mut app, ToolEvent::Down { x: 200.5, y: 150.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 199.5, y: 150.0 }, Modifiers::NONE);
    assert!(app.ui.transform.is_none(), "pressed on the pasteboard: committed");
    assert_eq!(steps(&app), before + 1);
}

#[test]
fn a_drag_outside_the_box_still_rotates() {
    let mut app = app();
    let quad = app.ui.transform.as_ref().unwrap().quad;
    tool_event(&mut app, ToolEvent::Down { x: 150.0, y: 100.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 140.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 140.0 }, Modifiers::NONE);
    assert!(app.ui.transform.is_some());
    assert_ne!(app.ui.transform.as_ref().unwrap().quad, quad);
}

/// 60 frames a second, so the second click of a double-click lands inside egui's delay.
fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
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

fn screen(app: &PhotocraftApp, p: [f64; 2]) -> Pos2 {
    let v = &app.ui.views[0];
    let xf = ViewXform {
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: v.zoom / app.canvas_ppp(),
        center: v.center,
        flip: app.ui.view.flip_horizontal,
        rotation: v.rotation,
        aspect: app.ui.view.display_aspect(),
    };
    xf.to_screen(p[0] as f32, p[1] as f32)
}

fn double_click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2) {
    h.hover_at(p);
    h.run_steps(1);
    for pressed in [true, false, true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.run_steps(2);
}

#[test]
fn a_double_click_inside_the_box_commits() {
    let mut h = harness(app());
    let before = steps(h.state());
    // The box is x 30..70, y 20..60 now; its middle is clear of the handles.
    let at = screen(h.state(), [50.0, 40.0]);
    double_click(&mut h, at);
    assert!(h.state().ui.transform.is_none(), "committed");
    assert_eq!(steps(h.state()), before + 1);
}

#[test]
fn a_double_click_outside_the_box_does_nothing() {
    let mut h = harness(app());
    let before = steps(h.state());
    let at = screen(h.state(), [150.0, 150.0]);
    double_click(&mut h, at);
    assert!(h.state().ui.transform.is_some(), "the box stays open");
    assert_eq!(steps(h.state()), before);
}

/// With Toggle Reference Point off, an ⌥-click outside the box moves the hidden reference point
/// there; a double-click on that spot is still outside the box and must not commit.
#[test]
fn a_double_click_on_a_hidden_reference_point_outside_the_box_does_nothing() {
    let mut app = app();
    app.ui.tool_options.transform_reference_point = false;
    tool_event(&mut app, ToolEvent::Down { x: 150.0, y: 150.0, pressure: 1.0 }, Modifiers::ALT);
    tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 150.0 }, Modifiers::ALT);
    assert_eq!(app.ui.transform.as_ref().unwrap().pivot, [150.0, 150.0], "the ⌥-click moved the reference point");
    let mut h = harness(app);
    let before = steps(h.state());
    let at = screen(h.state(), [150.0, 150.0]);
    double_click(&mut h, at);
    assert!(h.state().ui.transform.is_some(), "the box stays open");
    assert_eq!(steps(h.state()), before);
}
