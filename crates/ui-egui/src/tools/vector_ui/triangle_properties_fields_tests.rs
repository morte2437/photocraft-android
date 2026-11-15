//! #13360: a triangle's Properties Shape section shows one Corner radius field and no Sides and no
//! Star ratio field; Polygon tool (even with 3 sides) and Star tool shapes keep theirs. The
//! behaviour landed with the live triangle kind (#13578); these cases pin the issue's Done-when.

use egui::accesskit::Role;
use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LayerId, LiveShape};
use serde_json::{Value, json};

use super::{finish_shape, shape_properties};
use crate::PhotocraftApp;
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app
}

fn active_id(app: &PhotocraftApp) -> LayerId {
    app.session.active().unwrap().active_layer.unwrap()
}

fn fonts_ready(ui: &egui::Ui) -> bool {
    ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into())))
}

fn harness(app: PhotocraftApp, id: LayerId) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(400.0, 900.0)).build_ui_state(
        move |ui, app| {
            if fonts_ready(ui) {
                shape_properties(app, ui, id);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn labels(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.query_all_by_role(Role::SpinButton).filter_map(|n| n.accesskit_node().label()).map(|l| l.to_string()).collect()
}

fn only_corner_radius(h: &Harness<'static, PhotocraftApp>) {
    let l = labels(h);
    assert!(l.iter().any(|s| s == "Corner radius"), "{l:?}");
    assert!(!l.iter().any(|s| s == "Sides" || s == "Star ratio"), "{l:?}");
}

fn drag(tool: Tool, sides: u32) -> (PhotocraftApp, LayerId) {
    let mut app = app();
    app.ui.tool = tool;
    app.ui.tool_options.polygon_sides = sides;
    finish_shape(&mut app, tool, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE);
    let id = active_id(&app);
    (app, id)
}

fn info(app: &mut PhotocraftApp) -> Value {
    let id = active_id(app).0;
    app.session.execute("shape.info", json!({"layer": id})).unwrap()
}

fn knots(v: &Value) -> usize {
    v["path"]["subpaths"][0]["knots"].as_array().unwrap().len()
}

fn live(app: &PhotocraftApp, id: LayerId) -> LiveShape {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.live.clone().expect("a live shape"),
        _ => panic!("a shape layer"),
    }
}

fn corner_radius(app: &PhotocraftApp, id: LayerId) -> f64 {
    match live(app, id) {
        LiveShape::Triangle { corner_radius, .. } => corner_radius,
        other => panic!("a live triangle, got {other:?}"),
    }
}

fn type_radius(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Corner radius")).expect("the Corner radius field").click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

#[test]
fn a_dragged_triangle_shows_only_corner_radius() {
    let (app, id) = drag(Tool::Triangle, 5);
    assert!(matches!(live(&app, id), LiveShape::Triangle { .. }));
    only_corner_radius(&harness(app, id));
}

#[test]
fn a_create_triangle_shape_shows_only_corner_radius() {
    // Runs the command Create Triangle, OK sends (same geometry as the dialog, see
    // triangle_box_geometry_tests) rather than driving the dialog.
    let mut app = app();
    let fields = json!({"__createShape": "triangle", "__at": [400.0, 300.0], "width": 100.0, "height": 100.0, "fromCenter": false});
    let Value::Object(fields) = fields else { panic!("an object") };
    let geometry = crate::shape_dialog::geometry(&fields).unwrap();
    app.run("shape.create", geometry).unwrap();
    let id = active_id(&app);
    assert!(matches!(live(&app, id), LiveShape::Triangle { .. }));
    only_corner_radius(&harness(app, id));
}

#[test]
fn editing_corner_radius_rounds_the_triangle() {
    let (mut app, id) = drag(Tool::Triangle, 5);
    let before = knots(&info(&mut app));
    let mut h = harness(app, id);
    type_radius(&mut h, "10");
    assert_eq!(corner_radius(h.state(), id), 10.0);
    let after = knots(&info(h.state_mut()));
    assert!(after > before, "{before} -> {after}");
    only_corner_radius(&h);
}

#[test]
fn polygon_with_three_sides_and_star_keep_sides_and_star_ratio() {
    for (tool, sides) in [(Tool::Polygon, 3), (Tool::Polygon, 6), (Tool::Star, 5)] {
        let (app, id) = drag(tool, sides);
        let h = harness(app, id);
        let l = labels(&h);
        assert!(l.iter().any(|s| s == "Sides") && l.iter().any(|s| s == "Star ratio") && l.iter().any(|s| s == "Corner radius"), "{tool:?} {sides}: {l:?}");
    }
}

#[test]
fn the_triangle_stays_a_triangle_after_undo_redo_and_a_pcraft_round_trip() {
    let (app, id) = drag(Tool::Triangle, 5);
    let mut h = harness(app, id);
    type_radius(&mut h, "10");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(corner_radius(h.state(), id), 0.0);
    only_corner_radius(&h);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(corner_radius(h.state(), id), 10.0);
    only_corner_radius(&h);

    // Document-level round trip with the format crate.
    let bytes = photocraft_format::save_to_bytes(&h.state().session.active().unwrap().doc, &Default::default()).unwrap();
    let doc = photocraft_format::load_from_bytes(&bytes).unwrap();
    let loaded = match &doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.live.clone(),
        _ => panic!("a shape layer"),
    };
    assert!(matches!(loaded, Some(LiveShape::Triangle { corner_radius, .. }) if corner_radius == 10.0), "{loaded:?}");
}
