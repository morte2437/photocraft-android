//! TOOL-214-12: a live rectangle's corner radius widgets, as in Photoshop.

use egui::Modifiers;
use photocraft_doc::LayerContent;
use photocraft_doc::vector::LiveShape;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// A 200 × 200 document at 100 % with a live rectangle at (40, 40), 120 × 80, radii 10.
fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.run("shape.create", json!({"kind": "roundedRect", "rect": [40, 40, 120, 80], "radii": 10})).unwrap();
    app.ui.tool = tool;
    app
}

fn radii(app: &PhotocraftApp) -> Option<[f64; 4]> {
    let st = app.session.active()?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id))?;
    match &l.content {
        LayerContent::Shape(sh) => match sh.live {
            Some(LiveShape::Rect { radii, .. }) => Some(radii),
            _ => None,
        },
        _ => None,
    }
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

#[test]
fn dragging_a_widget_rounds_every_corner_in_one_step() {
    let mut app = app(Tool::PathSelection);
    let n = steps(&app);
    // The top-left widget sits at the arc centre (50, 50); dragging it 15 px along the diagonal
    // adds 15 to the radius.
    drag(&mut app, [50.0, 50.0], [65.0, 65.0], Modifiers::NONE);
    assert_eq!(radii(&app), Some([25.0; 4]));
    assert_eq!(steps(&app), n + 1, "one history step");
    assert!(app.ui.corner_radius.is_none());
    assert!(app.session.undo());
    assert_eq!(radii(&app), Some([10.0; 4]));
}

#[test]
fn alt_drags_one_corner_and_radii_stay_within_half_the_short_side() {
    let mut app = app(Tool::Rectangle);
    // The bottom-right widget, with ⌥: only that corner, capped at 40 (half of the height 80).
    drag(&mut app, [150.0, 110.0], [20.0, 20.0], Modifiers::ALT);
    assert_eq!(radii(&app), Some([10.0, 10.0, 40.0, 10.0]));
    // Dragging out past the corner makes it sharp.
    drag(&mut app, [150.0, 50.0], [190.0, 10.0], Modifiers::ALT);
    assert_eq!(radii(&app), Some([10.0, 0.0, 40.0, 10.0]));
    // The Rectangle tool drew no new shape.
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 2);
}

#[test]
fn a_sharp_corner_keeps_a_grabbable_widget_and_a_click_changes_nothing() {
    let mut app = app(Tool::PathSelection);
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    app.run("shape.edit", json!({"layer": id, "radii": 0})).unwrap();
    let n = steps(&app);
    // At 100 % the widget of a sharp corner is 10 px inside it.
    drag(&mut app, [50.0, 50.0], [50.0, 50.0], Modifiers::NONE);
    assert_eq!((radii(&app), steps(&app)), (Some([0.0; 4]), n), "a click commits nothing");
    drag(&mut app, [50.0, 50.0], [60.0, 60.0], Modifiers::NONE);
    assert_eq!(radii(&app), Some([10.0; 4]));
}

#[test]
fn only_live_rectangles_with_the_right_tools_take_the_press() {
    for tool in [Tool::Move, Tool::DirectSelection, Tool::Brush] {
        let mut a = app(tool);
        assert!(!super::pointer(&mut a, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE), "{tool:?}");
    }
    // ⌘/Ctrl leaves the press to the tool.
    let mut a = app(Tool::PathSelection);
    assert!(!super::pointer(&mut a, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::COMMAND));
    // An ellipse has no corners.
    let mut a = app_with(json!({"kind": "ellipse", "rect": [40, 40, 120, 80]}));
    assert!(!super::pointer(&mut a, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE));
    // A press off the widgets is the tool's.
    let mut a = app(Tool::PathSelection);
    assert!(!super::pointer(&mut a, ToolEvent::Down { x: 100.0, y: 80.0, pressure: 1.0 }, Modifiers::NONE));
}

fn app_with(shape: serde_json::Value) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.run("shape.create", shape).unwrap();
    app.ui.tool = Tool::PathSelection;
    app
}

/// Three live rectangles: (20, 20) 60 × 40 radii 5, (110, 20) 70 × 50 radii 0, and (20, 120)
/// 80 × 60 radii 0, the last one left unselected. Returns the ids, bottom first.
fn three(tool: Tool) -> (PhotocraftApp, Vec<photocraft_doc::LayerId>) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    let mut ids = Vec::new();
    for (rect, r) in [([20, 20, 60, 40], 5), ([110, 20, 70, 50], 0), ([20, 120, 80, 60], 0)] {
        app.run("shape.create", json!({"kind": "rect", "rect": rect, "radii": r})).unwrap();
        ids.push(app.session.active().unwrap().active_layer.unwrap());
    }
    app.run("layer.select", json!({"layer": ids[0].0})).unwrap();
    app.run("layer.select", json!({"layer": ids[1].0, "mode": "add"})).unwrap();
    app.ui.tool = tool;
    (app, ids)
}

fn radii_of(app: &PhotocraftApp, id: photocraft_doc::LayerId) -> [f64; 4] {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => match sh.live {
            Some(LiveShape::Rect { radii, .. }) => radii,
            _ => panic!("live rect"),
        },
        _ => panic!("shape"),
    }
}

#[test]
fn one_widget_rounds_every_selected_rectangle_alike_in_one_step() {
    let (mut app, ids) = three(Tool::PathSelection);
    let n = steps(&app);
    // The second rectangle's top-left widget (sharp: 10 px inside, at (120, 30)), dragged 15 px
    // in each direction: radius 15 on every corner of both selected rectangles.
    drag(&mut app, [120.0, 30.0], [135.0, 45.0], Modifiers::NONE);
    assert_eq!(radii_of(&app, ids[0]), [15.0; 4]);
    assert_eq!(radii_of(&app, ids[1]), [15.0; 4]);
    assert_eq!(radii_of(&app, ids[2]), [0.0; 4], "the unselected one is untouched");
    assert_eq!(steps(&app), n + 1, "one history step for both layers");
    assert!(app.session.undo());
    assert_eq!((radii_of(&app, ids[0]), radii_of(&app, ids[1])), ([5.0; 4], [0.0; 4]));
}

#[test]
fn alt_changes_that_corner_on_each_and_each_stays_within_its_own_size() {
    let (mut app, ids) = three(Tool::Rectangle);
    // ⌥ on the first rectangle's top-left widget (radius 5, so drawn 10 px inside at (30, 30)),
    // dragged far in: each rectangle's top-left corner only, capped at half of its own short side.
    drag(&mut app, [30.0, 30.0], [100.0, 100.0], Modifiers::ALT);
    assert_eq!(radii_of(&app, ids[0]), [20.0, 5.0, 5.0, 5.0]);
    assert_eq!(radii_of(&app, ids[1]), [25.0, 0.0, 0.0, 0.0]);
}

#[test]
fn the_move_tool_shows_and_drags_the_widgets_with_its_transform_controls() {
    let (mut app, ids) = three(Tool::Move);
    assert!(!super::pointer(&mut app, ToolEvent::Down { x: 120.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE), "not without the controls");
    app.ui.tool_options.move_show_transform = true;
    let before = app.session.active().unwrap().selected_layers();
    drag(&mut app, [120.0, 30.0], [130.0, 40.0], Modifiers::NONE);
    assert_eq!((radii_of(&app, ids[0]), radii_of(&app, ids[1])), ([10.0; 4], [10.0; 4]));
    assert_eq!(app.session.active().unwrap().selected_layers(), before, "the press neither moved nor reselected layers");
}

#[test]
fn the_cursor_and_readout_follow_the_widgets() {
    let mut app = app(Tool::PathSelection);
    // The corner radius cursor shows over a widget, not elsewhere.
    assert!(super::hovers(&app, [50.0, 50.0]));
    assert!(!super::hovers(&app, [100.0, 80.0]));
    // While dragging it stays, and the readout shows the dragged corner's radius.
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 62.0, y: 62.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(super::hovers(&app, [100.0, 80.0]));
    assert_eq!(app.ui.corner_radius.as_ref().and_then(super::dragged_radius), Some(22.0));
    tool_event(&mut app, ToolEvent::Up { x: 62.0, y: 62.0 }, Modifiers::NONE);
    assert!(!super::hovers(&app, [100.0, 80.0]));
}

/// The live polygon of layer `id`: its corner radius.
fn corner_radius_of(app: &PhotocraftApp, id: photocraft_doc::LayerId) -> f64 {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => match sh.live {
            Some(LiveShape::Polygon { corner_radius, .. }) => corner_radius,
            _ => panic!("live polygon"),
        },
        _ => panic!("shape"),
    }
}

/// A live triangle (a 3-sided polygon) in (40, 40) 120 × 120, as the Triangle tool draws it.
fn triangle(tool: Tool, radius: f64) -> (PhotocraftApp, photocraft_doc::LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.run("shape.create", json!({"kind": "polygon", "rect": [40, 40, 120, 120], "sides": 3, "cornerRadius": radius})).unwrap();
    app.ui.tool = tool;
    let id = app.session.active().unwrap().active_layer.unwrap();
    (app, id)
}

/// Where the widget of corner `i` of the first target is, and the way its arc's centre moves.
fn widget_of(app: &PhotocraftApp, i: usize) -> ([f64; 2], [f64; 2]) {
    let t = &super::targets(app)[0];
    let z = f64::from(app.point_zoom());
    (t.widget(t.radii, i, z).unwrap(), t.corner(i).unwrap().step)
}

#[test]
fn a_triangle_rounds_all_its_corners_from_any_widget_in_one_step() {
    for tool in [Tool::Triangle, Tool::Polygon, Tool::PathSelection] {
        let (mut app, id) = triangle(tool, 6.0);
        assert_eq!(super::targets(&app)[0].corners(), 3, "{tool:?}");
        let n = steps(&app);
        // Drag the bottom-right widget inwards by 4 radius units along its step: 6 + 4 = 10.
        let (w, step) = widget_of(&app, 1);
        let to = [w[0] + step[0] * 4.0, w[1] + step[1] * 4.0];
        drag(&mut app, w, to, Modifiers::NONE);
        assert!((corner_radius_of(&app, id) - 10.0).abs() < 1e-9, "{tool:?}: {}", corner_radius_of(&app, id));
        assert_eq!(steps(&app), n + 1, "one history step");
        // ⌥ makes no difference: a triangle has one radius for all its corners.
        let (w, step) = widget_of(&app, 0);
        drag(&mut app, w, [w[0] + step[0] * 2.0, w[1] + step[1] * 2.0], Modifiers::ALT);
        assert!((corner_radius_of(&app, id) - 12.0).abs() < 1e-9);
        assert!(app.session.undo());
        assert!((corner_radius_of(&app, id) - 10.0).abs() < 1e-9);
    }
}

#[test]
fn a_triangle_radius_is_capped_and_a_click_on_a_sharp_corner_changes_nothing() {
    let (mut app, id) = triangle(Tool::PathSelection, 0.0);
    let n = steps(&app);
    let (w, step) = widget_of(&app, 2);
    drag(&mut app, w, w, Modifiers::NONE);
    assert_eq!((corner_radius_of(&app, id), steps(&app)), (0.0, n), "a click commits nothing");
    // Dragged far in: the radius stops where the arcs meet halfway along the edges.
    drag(&mut app, w, [w[0] + step[0] * 1000.0, w[1] + step[1] * 1000.0], Modifiers::NONE);
    let max = super::targets(&app)[0].max_radius();
    assert!(max > 0.0 && (corner_radius_of(&app, id) - max).abs() < 1e-9, "{} vs {max}", corner_radius_of(&app, id));
}

#[test]
fn rectangles_and_triangles_selected_together_take_the_same_radius() {
    let (mut app, tri) = triangle(Tool::PathSelection, 0.0);
    app.run("shape.create", json!({"kind": "rect", "rect": [10, 170, 100, 20], "radii": 0})).unwrap();
    let rect = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.select", json!({"layer": tri.0, "mode": "add"})).unwrap();
    // The rectangle's top-left widget (10 px inside, at (20, 180)), 4 px in each direction: 4.
    drag(&mut app, [20.0, 180.0], [24.0, 184.0], Modifiers::NONE);
    assert_eq!(radii(&app), None, "the active layer is the triangle");
    let rr = match &app.session.active().unwrap().doc.layer(rect).unwrap().content {
        LayerContent::Shape(sh) => match sh.live {
            Some(LiveShape::Rect { radii, .. }) => radii,
            _ => panic!(),
        },
        _ => panic!(),
    };
    assert_eq!(rr, [4.0; 4]);
    assert!((corner_radius_of(&app, tri) - 4.0).abs() < 1e-9);
}
