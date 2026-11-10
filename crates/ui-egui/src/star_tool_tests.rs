//! The Star Tool (#11377): in the shape slot between Polygon and Line, on U, and a drag makes a
//! live star shape layer with PhotoCraft's star defaults (5 points, star ratio 0.5).

use egui::{Key, Modifiers};
use photocraft_doc::{Color, Fill, LayerContent, LiveShape, ShapeLayer};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::panels::toolbar::TOOL_SECTIONS;
use crate::state::Tool;

const SHAPES: [Tool; 7] = [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Star, Tool::Line, Tool::CustomShape];

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app
}

fn press(app: &mut PhotocraftApp, key: Key, modifiers: Modifiers) {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

fn shape_layer(app: &PhotocraftApp) -> Option<ShapeLayer> {
    let st = app.session.active()?;
    match &st.doc.layer(st.active_layer?)?.content {
        LayerContent::Shape(s) => Some(s.clone()),
        _ => None,
    }
}

fn live_shape(app: &PhotocraftApp) -> Option<LiveShape> {
    shape_layer(app)?.live
}

#[test]
fn star_sits_in_the_shape_slot_after_polygon() {
    let slot = TOOL_SECTIONS.iter().flat_map(|s| s.iter()).find(|slot| slot.contains(&Tool::Star)).copied();
    assert_eq!(slot, Some(&SHAPES[..]));
    let pos = |t: Tool| Tool::ALL.iter().position(|x| *x == t);
    assert_eq!(pos(Tool::Star), pos(Tool::Polygon).map(|p| p + 1), "Tool::ALL keeps the slot order");
}

#[test]
fn star_is_a_shape_tool_on_u_with_its_label_icon_and_name() {
    assert!(crate::tools::vector_ui::is_shape_tool(Tool::Star));
    assert_eq!(Tool::Star.key(), 'U');
    assert_eq!(Tool::Star.label(), "Star Tool");
    assert_eq!(Tool::from_name("Star Tool"), Some(Tool::Star));
    assert_eq!(Tool::from_name("star"), Some(Tool::Star));
    assert_eq!(crate::icons::tool_icon(Tool::Star), "star");
}

#[test]
fn shift_u_steps_from_polygon_to_star_to_line() {
    let mut app = app();
    app.ui.tool = Tool::Polygon;
    press(&mut app, Key::U, Modifiers::SHIFT);
    assert_eq!(app.ui.tool, Tool::Star);
    press(&mut app, Key::U, Modifiers::SHIFT);
    assert_eq!(app.ui.tool, Tool::Line);
}

#[test]
fn a_drag_makes_a_live_star_shape_layer_with_one_undo_step() {
    let mut app = app();
    app.ui.tool = Tool::Star;
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
    app.ui.tool_options.shape_fill = true;
    app.ui.tool_options.stroke_width = 5.0;
    let before = app.session.active().unwrap().history.past_len();
    drag(&mut app, [20.0, 30.0], [120.0, 90.0], Modifiers::NONE);
    match live_shape(&app) {
        Some(LiveShape::Polygon { rect, sides, star_ratio, .. }) => {
            assert_eq!((sides, star_ratio), (5, 0.5));
            assert_eq!(rect, [20.0, 30.0, 100.0, 60.0]);
        }
        other => panic!("a live star shape, got {other:?}"),
    }
    let shape = shape_layer(&app).expect("a star shape layer");
    let solid = |r, g, b| Fill::Solid(Color::rgba(r, g, b, 1.0));
    assert_eq!(shape.fill, Some(solid(1.0, 0.0, 0.0)), "filled with the foreground colour");
    let stroke = shape.stroke.expect("the star keeps the tool's stroke");
    assert_eq!((stroke.paint, stroke.width), (solid(0.0, 0.0, 1.0), 5.0), "stroked with the background colour at the tool's width");
    assert_eq!(app.session.active().unwrap().history.past_len(), before + 1, "one history step");
    app.run("edit.undo", json!({})).unwrap();
    assert!(live_shape(&app).is_none(), "undo takes the star away");
}

#[test]
fn shift_and_alt_constrain_and_centre_the_star_like_the_polygon_tool() {
    let mut app = app();
    app.ui.tool = Tool::Star;
    drag(&mut app, [100.0, 100.0], [140.0, 120.0], Modifiers::SHIFT | Modifiers::ALT);
    match live_shape(&app) {
        Some(LiveShape::Polygon { rect, .. }) => assert_eq!(rect, [60.0, 60.0, 80.0, 80.0], "a square box drawn from the centre"),
        other => panic!("a live star shape, got {other:?}"),
    }
}
