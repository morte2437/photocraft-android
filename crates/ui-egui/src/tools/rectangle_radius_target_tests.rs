//! #13338: the Rectangle tool's options-bar Radius sets the next rectangle, not the active shape.
//! It shows the tool's own radius. The Path Selection / Direct Selection Radius still edits the
//! active shape.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LiveShape};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// A document with Rectangle 1 (radius 20) as the active layer, and `tool` current.
fn bar_with(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.create", json!({"kind": "roundedRect", "rect": [10, 10, 80, 60], "radii": [20, 20, 20, 20], "fill": "#ff0000"})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn type_radius(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Radius:")).expect("the Radius field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn history_len(h: &Harness<'static, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

/// The first radius of every rectangle shape layer, bottom to top.
fn radii(h: &Harness<'static, PhotocraftApp>) -> Vec<f64> {
    let st = h.state().session.active().unwrap();
    st.doc
        .walk()
        .into_iter()
        .filter_map(|(_, _, l)| match &l.content {
            LayerContent::Shape(sh) => match &sh.live {
                Some(LiveShape::Rect { radii, .. }) => Some(radii[0]),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let mods = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

#[test]
fn the_rectangle_tools_radius_field_leaves_the_active_shape_alone() {
    let mut h = bar_with(Tool::Rectangle);
    let before = history_len(&h);
    type_radius(&mut h, "5000");
    assert_eq!(radii(&h), vec![20.0], "Rectangle 1 keeps its radius");
    assert_eq!(history_len(&h), before, "no History state");
    assert_eq!(h.state().ui.tool_options.corner_radius, 5000.0);
}

#[test]
fn the_next_rectangle_takes_the_typed_radius() {
    let mut h = bar_with(Tool::Rectangle);
    type_radius(&mut h, "5000");
    drag(h.state_mut(), [100.0, 100.0], [180.0, 170.0]);
    let all = radii(&h);
    assert_eq!(all.len(), 2, "a second rectangle was drawn: {all:?}");
    assert_eq!(all[0], 20.0);
    assert_eq!(all[1], 5000.0);
}

#[test]
fn the_rectangle_tools_radius_field_shows_the_tools_radius() {
    let mut h = bar_with(Tool::Rectangle);
    h.state_mut().ui.tool_options.corner_radius = 0.0;
    h.run_steps(3);
    let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Radius:")).expect("the Radius field");
    let shown = field
        .accesskit_node()
        .numeric_value()
        .map(|v| v as f32)
        .or_else(|| field.accesskit_node().value().and_then(|v| v.trim_end_matches(" px").trim().parse().ok()));
    assert_eq!(shown, Some(0.0), "the tool's radius, not the shape's 20");
}

#[test]
fn the_path_selection_radius_field_still_edits_the_active_shape() {
    for tool in [Tool::PathSelection, Tool::DirectSelection] {
        let mut h = bar_with(tool);
        let before = history_len(&h);
        type_radius(&mut h, "30");
        assert_eq!(radii(&h), vec![30.0], "{tool:?}: the active shape takes it");
        assert_eq!(history_len(&h), before + 1, "{tool:?}: one History state");
    }
}
