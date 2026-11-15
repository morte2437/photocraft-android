//! #13219: Direct Selection's options bar is the Path Selection bar: the same controls in the same
//! order, the Path operations button included, instead of a hint. Its operations act on the
//! components that have selected anchors.

use egui::accesskit::Toggled;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{Path, PathOp};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const SAVED: &str = "Path 1";

/// A saved path of two squares (the second combined), targeted, with Direct Selection.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    let square = |x: f64, op: &str| json!({"closed": true, "op": op, "knots": [[x, 50.0], [x + 80.0, 50.0], [x + 80.0, 130.0], [x, 130.0]]});
    app.run("path.set", json!({"name": SAVED, "path": {"subpaths": [square(40.0, "combine"), square(200.0, "combine")]}})).unwrap();
    app.ui.selected_path = Some(SAVED.into());
    app.ui.tool = Tool::DirectSelection;
    app
}

fn saved(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths.iter().find(|p| p.name == SAVED).unwrap().path.clone()
}

/// The vector tools' options bar for `tool`, laid out once.
fn bar(app: PhotocraftApp, tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1400.0, 400.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal(|ui| crate::vector_ui::options_bar(app, ui, tool));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

fn open_path_operations(h: &mut Harness<'_, PhotocraftApp>) {
    h.get_by_label("Path operations").click();
    h.run_steps(3);
}

/// With a saved path targeted and nothing selected, as measured: the Path operations button with
/// New Layer greyed and Combine Shapes checked; no hint text.
#[test]
fn the_bar_shows_path_operations_instead_of_the_hint() {
    let mut h = bar(app(), Tool::DirectSelection);
    assert!(h.query_by_label_contains("Click: select anchor").is_none(), "no hint text");
    open_path_operations(&mut h);
    assert!(h.get_by_label("New Layer").accesskit_node().is_disabled(), "Direct Selection makes no shape layer");
    assert_eq!(h.get_by_label("Combine Shapes").accesskit_node().toggled(), Some(Toggled::True));
    assert!(!h.get_by_label("Merge Shape Components").accesskit_node().is_disabled(), "two components to merge");
}

/// New Layer stays greyed when the Pen was last in Shape mode: that setting is the Pen's.
#[test]
fn new_layer_stays_greyed_with_the_pen_in_shape_mode() {
    let mut app = app();
    app.ui.tool_options.vector_mode = "shape".into();
    let mut h = bar(app, Tool::DirectSelection);
    open_path_operations(&mut h);
    assert!(h.get_by_label("New Layer").accesskit_node().is_disabled());
    assert_eq!(h.get_by_label("Combine Shapes").accesskit_node().toggled(), Some(Toggled::True), "not the greyed New Layer");
}

/// With a saved path targeted both tools draw Path operations at the same place.
#[test]
fn the_bar_matches_path_selection_without_a_shape_layer() {
    let x = |tool: Tool| {
        let mut app = app();
        app.ui.tool = tool;
        bar(app, tool).get_by_label("Path operations").rect().min.x
    };
    assert_eq!(x(Tool::DirectSelection), x(Tool::PathSelection));
}

/// With a shape layer selected both tools draw the same controls at the same places.
#[test]
fn the_bar_matches_path_selection_with_a_shape_layer() {
    let labels = ["Path operations", "Mode", "Fill", "Fill color", "Stroke color"];
    let xs = |tool: Tool| {
        let mut app = app();
        app.ui.selected_path = None;
        app.run("shape.create", json!({"rect": [20, 20, 220, 170], "stroke": null})).unwrap();
        app.ui.tool = tool;
        let h = bar(app, tool);
        labels.map(|l| h.get_by_label(l).rect().min.x)
    };
    let (path, direct) = (xs(Tool::PathSelection), xs(Tool::DirectSelection));
    assert!(path.is_sorted_by(|a, b| a < b), "left to right: {path:?}");
    assert_eq!(direct, path);
}

/// A shape layer of two squares at `y`, 80 px each, active; its id.
fn two_component_shape(app: &mut PhotocraftApp, y: f64) -> u64 {
    app.run("shape.create", json!({"rect": [40, y, 80, 80], "stroke": null})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    app.run("shape.create", json!({"rect": [200, y, 80, 80], "addTo": id, "op": "combine"})).unwrap();
    id
}

fn shape_path(app: &PhotocraftApp, id: u64) -> Path {
    let st = app.session.active().unwrap();
    match &st.doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
        photocraft_doc::LayerContent::Shape(sh) => sh.path.clone(),
        _ => panic!("not a shape layer"),
    }
}

/// Merge Shape Components merges the selected shape layer's components, as with Path Selection.
#[test]
fn merge_shape_components_acts_on_the_selected_shape_layer() {
    let mut app = app();
    app.ui.selected_path = None;
    two_component_shape(&mut app, 20.0);
    let steps = app.session.active().unwrap().history.past_len();
    let mut h = bar(app, Tool::DirectSelection);
    open_path_operations(&mut h);
    h.get_by_label("Merge Shape Components").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "{}", h.state().ui.status);
}

/// An anchor clicked on the second square selects it for the menu: Subtract Front Shape turns
/// that component into a hole, one History step; the first square keeps its operation.
#[test]
fn a_picked_operation_acts_on_the_component_with_a_selected_anchor() {
    let mut app = app();
    let mods = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 280.0, y: 50.0, pressure: 1.0 }, mods);
    tool_event(&mut app, ToolEvent::Up { x: 280.0, y: 50.0 }, mods);
    assert_eq!(app.ui.direct_selection.anchors, [[1, 1]], "the anchor is selected");
    let steps = app.session.active().unwrap().history.past_len();
    let mut h = bar(app, Tool::DirectSelection);
    open_path_operations(&mut h);
    h.get_by_label("Subtract Front Shape").click();
    h.run_steps(3);
    let app = h.state();
    assert_eq!(saved(app).subpaths.iter().map(|s| s.op).collect::<Vec<_>>(), [PathOp::Combine, PathOp::Subtract]);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    assert_eq!(app.ui.direct_selection.anchors, [[1, 1]], "the anchor stays selected");
    assert_eq!(app.ui.tool_options.pen_path_op, "subtract", "the tool keeps it for the next path, as Path Selection does");
}

/// The saved path is targeted, but the anchor is selected on the shape layer: Merge Shape
/// Components merges the shape layer's components and leaves the saved path alone.
#[test]
fn merge_shape_components_acts_on_the_path_with_selected_anchors() {
    let mut app = app();
    let id = two_component_shape(&mut app, 180.0);
    app.ui.selected_path = Some(SAVED.into());
    let mods = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 40.0, y: 180.0, pressure: 1.0 }, mods);
    tool_event(&mut app, ToolEvent::Up { x: 40.0, y: 180.0 }, mods);
    assert_eq!(app.ui.direct_selection.target, Some(crate::direct_select::PathRef::Layer(id)), "the shape's anchor is selected");
    assert_eq!(app.ui.selected_path.as_deref(), Some(SAVED), "the saved path stays targeted");
    let (before, shape) = (saved(&app), shape_path(&app, id));
    let mut h = bar(app, Tool::DirectSelection);
    open_path_operations(&mut h);
    h.get_by_label("Merge Shape Components").click();
    h.run_steps(3);
    assert_eq!(saved(h.state()), before, "the saved path is unchanged");
    assert_ne!(shape_path(h.state(), id), shape, "{}", h.state().ui.status);
}

/// A marquee selects anchors on a shape layer that is not the active one: the picked operation
/// acts on that layer's component, not on the active layer.
#[test]
fn a_picked_operation_acts_on_anchors_marqueed_on_an_inactive_shape_layer() {
    let mut app = app();
    app.ui.selected_path = None;
    let inactive = two_component_shape(&mut app, 20.0);
    let active = two_component_shape(&mut app, 180.0);
    let mods = egui::Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 190.0, y: 10.0, pressure: 1.0 }, mods);
    tool_event(&mut app, ToolEvent::Move { x: 290.0, y: 110.0, pressure: 1.0 }, mods);
    tool_event(&mut app, ToolEvent::Up { x: 290.0, y: 110.0 }, mods);
    assert_eq!(app.ui.direct_selection.target, Some(crate::direct_select::PathRef::Layer(inactive)), "the marquee selects its anchors");
    assert_eq!(app.session.active().unwrap().active_layer.map(|l| l.0), Some(active), "the other layer stays active");
    let other = shape_path(&app, active);
    let mut h = bar(app, Tool::DirectSelection);
    open_path_operations(&mut h);
    h.get_by_label("Subtract Front Shape").click();
    h.run_steps(3);
    let app = h.state();
    assert_eq!(shape_path(app, inactive).subpaths[1].op, PathOp::Subtract, "{}", app.ui.status);
    assert_eq!(shape_path(app, active), other, "the active layer is unchanged");
}
