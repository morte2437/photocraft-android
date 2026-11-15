//! #13364: with the Triangle tool, ⌥ (Alt) held at the press with no shape layer active picks the
//! colour under the pointer as the foreground and the tool's Fill instead of drawing or opening
//! Create Triangle; ⇧⌥ does the same. ⌥ pressed only after the drag started still draws from the
//! centre, and with a shape layer active ⌥ at the press does not sample.

use egui::Modifiers;
use photocraft_doc::{Color, Fill, LayerContent};
use serde_json::json;

use super::shape_alt_samples;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

const RED: [u8; 3] = [220, 30, 30];
const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];
/// A point on the red layer, and one on the white Background only.
const ON_RED: [f64; 2] = [200.0, 175.0];
const ON_WHITE: [f64; 2] = [500.0, 350.0];

/// The issue's test document: 800 × 600, white Background, a red (220,30,30) pixel layer at
/// 100,100 to 300,250 (active), black foreground, the Triangle tool in Shape mode.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 150})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Triangle;
    app
}

/// What a press could change: history states, layers, open dialogs and colour samplers.
fn document_state(app: &PhotocraftApp) -> (usize, usize, usize, usize) {
    let st = app.session.active().unwrap();
    (st.history.past_len(), st.doc.layers.len(), app.ui.dialogs.len(), st.color_samplers.len())
}

/// The foreground colour in 8-bit RGB.
fn foreground(app: &PhotocraftApp) -> [u8; 3] {
    let c = app.session.tools.foreground;
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

/// The tool's Fill colour: a solid Fill follows the foreground (`vector_ui::shapes::tool_paint`).
fn tool_fill(app: &PhotocraftApp) -> [u8; 3] {
    let o = &app.ui.tool_options;
    assert!(o.shape_fill && o.shape_fill_gradient.is_none(), "a solid Fill");
    foreground(app)
}

/// Press at `from` with `press` held, move to `to` with `rest`, release there with `rest`.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], press: Modifiers, rest: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, press);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, rest);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, rest);
}

/// The active layer's Fill colour in 8-bit RGB, which must be a shape layer with a solid Fill.
fn active_shape_fill(app: &PhotocraftApp) -> [u8; 3] {
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let LayerContent::Shape(shape) = &layer.content else { panic!("a shape layer, not {:?}", layer.name) };
    let Some(Fill::Solid(c)) = &shape.fill else { panic!("a solid Fill, not {:?}", shape.fill) };
    let [r, g, b, _] = c.to_rgba8();
    [r, g, b]
}

#[test]
fn alt_click_on_the_red_layer_picks_it_as_foreground_and_fill() {
    for press in [Modifiers::ALT, Modifiers::SHIFT | Modifiers::ALT] {
        let mut app = app();
        let before = document_state(&app);
        drag(&mut app, ON_RED, ON_RED, press, press);
        assert_eq!(foreground(&app), RED, "{press:?}: foreground #DC1E1E");
        assert_eq!(tool_fill(&app), RED, "{press:?}: Fill 220,30,30");
        assert_eq!(document_state(&app), before, "{press:?}: no History state, layer, dialog or colour sampler");
        assert!(!app.alt_sampling, "{press:?}: the release ends the sampling");
        // The next triangle is drawn in that Fill.
        drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE, Modifiers::NONE);
        assert_eq!(active_shape_fill(&app), RED, "{press:?}: the next triangle's Fill");
    }
}

#[test]
fn an_alt_drag_keeps_the_colour_under_the_release_point() {
    for press in [Modifiers::ALT, Modifiers::SHIFT | Modifiers::ALT] {
        let mut app = app();
        let before = document_state(&app);
        // The last move is still on red; the release is on white.
        tool_event(&mut app, ToolEvent::Down { x: ON_RED[0], y: ON_RED[1], pressure: 1.0 }, press);
        tool_event(&mut app, ToolEvent::Move { x: 250.0, y: 200.0, pressure: 1.0 }, press);
        assert_eq!(foreground(&app), RED, "{press:?}: the drag samples as it goes");
        tool_event(&mut app, ToolEvent::Up { x: ON_WHITE[0], y: ON_WHITE[1] }, press);
        assert_eq!(foreground(&app), WHITE, "{press:?}: foreground");
        assert_eq!(tool_fill(&app), WHITE, "{press:?}: Fill");
        assert_eq!(document_state(&app), before, "{press:?}: no shape and no History state");
    }
}

#[test]
fn a_gradient_or_no_fill_becomes_the_sampled_solid_fill() {
    let gradient = Fill::Gradient {
        stops: vec![(0.0, Color::BLACK), (1.0, Color::WHITE)],
        angle: 90.0,
        scale: 1.0,
        style: Default::default(),
        reverse: false,
        opacity_stops: Vec::new(),
        midpoints: Vec::new(),
        offset: (0.0, 0.0),
        dither: false,
        align: true,
    };
    for (fill, fill_gradient) in [(true, Some(gradient)), (false, None)] {
        let mut app = app();
        app.ui.tool_options.shape_fill = fill;
        app.ui.tool_options.shape_fill_gradient = fill_gradient;
        drag(&mut app, ON_RED, ON_RED, Modifiers::ALT, Modifiers::ALT);
        assert_eq!(tool_fill(&app), RED, "fill {fill}");
    }
}

#[test]
fn alt_pressed_after_the_drag_started_still_draws_from_the_centre() {
    let mut app = app();
    let layers = document_state(&app).1;
    drag(&mut app, [400.0, 300.0], [500.0, 350.0], Modifiers::NONE, Modifiers::ALT);
    assert_eq!(foreground(&app), BLACK, "nothing sampled");
    assert_eq!(document_state(&app).1, layers + 1, "a triangle was drawn");
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let LayerContent::Shape(shape) = &layer.content else { panic!("a shape layer") };
    let anchors: Vec<[f64; 2]> = shape.path.subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
    assert_eq!(anchors, [[400.0, 250.0], [500.0, 350.0], [300.0, 350.0]], "centred on the press");
}

#[test]
fn a_shape_layer_active_does_not_sample() {
    let mut app = app();
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE, Modifiers::NONE);
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    assert!(!shape_alt_samples(&mut app, Modifiers::ALT, ON_RED[0], ON_RED[1]), "⌥ is left to the subtract");
    tool_event(&mut app, ToolEvent::Down { x: ON_RED[0], y: ON_RED[1], pressure: 1.0 }, Modifiers::ALT);
    assert!(!app.alt_sampling);
    tool_event(&mut app, ToolEvent::Up { x: ON_RED[0], y: ON_RED[1] }, Modifiers::ALT);
    assert_eq!(foreground(&app), BLACK);
}

#[test]
fn unmeasured_cases_keep_drawing() {
    let mut app = app();
    assert!(shape_alt_samples(&mut app, Modifiers::ALT, ON_RED[0], ON_RED[1]));
    assert!(shape_alt_samples(&mut app, Modifiers::SHIFT | Modifiers::ALT, ON_RED[0], ON_RED[1]));
    assert!(!shape_alt_samples(&mut app, Modifiers::NONE, ON_RED[0], ON_RED[1]));
    assert!(!shape_alt_samples(&mut app, Modifiers::ALT | Modifiers::CTRL, ON_RED[0], ON_RED[1]), "Control+⌥ is the brush resize");
    assert!(!shape_alt_samples(&mut app, Modifiers::ALT, -40.0, -40.0), "nothing to sample off the image");
    for mode in ["path", "pixels"] {
        app.ui.tool_options.shape_mode = mode.into();
        assert!(!shape_alt_samples(&mut app, Modifiers::ALT, ON_RED[0], ON_RED[1]), "{mode} mode wasn't measured");
    }
    app.ui.tool_options.shape_mode = "shape".into();
    app.ui.tool = Tool::Rectangle;
    assert!(!shape_alt_samples(&mut app, Modifiers::ALT, ON_RED[0], ON_RED[1]), "the other shape tools are their own items");
    // Off the image the press draws from the centre, as before.
    app.ui.tool = Tool::Triangle;
    let layers = document_state(&app).1;
    drag(&mut app, [-40.0, -40.0], [60.0, 60.0], Modifiers::ALT, Modifiers::ALT);
    assert_eq!(foreground(&app), BLACK, "nothing sampled");
    assert_eq!(document_state(&app).1, layers + 1, "a triangle was drawn");
}
