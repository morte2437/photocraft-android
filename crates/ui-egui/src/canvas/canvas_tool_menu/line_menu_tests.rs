//! #13249: in Shape mode a right-click on the canvas with the Line tool opens the shape and path
//! menu, as measured in the reference application over a pixel layer and over a line shape layer.
//! Path and Pixels mode, and other layer kinds, keep opening nothing.

use egui::{Context, Modifiers};
use egui_kittest::Harness;
use serde_json::{Value, json};

use super::tests::{app, harness, right_click};
use super::{available, choose, open};
use crate::PhotocraftApp;
use crate::state::Tool;

/// A pixel layer filled red, active, with the Line tool selected in Shape mode.
fn pixel_layer() -> PhotocraftApp {
    let mut app = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.ui.tool = Tool::Line;
    assert_eq!(app.ui.tool_options.shape_mode, "shape", "Shape is the default mode");
    app
}

/// A line drawn with the Line tool near the top of the canvas, with a fill and a 1 px stroke as
/// in the measured document: its shape layer is active.
fn line_layer() -> PhotocraftApp {
    let mut app = app();
    app.ui.tool = Tool::Line;
    app.ui.tool_options.stroke_width = 1.0;
    crate::vector_ui::finish_shape(&mut app, Tool::Line, [4.0, 4.0], [28.0, 4.0], Modifiers::NONE);
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert!(matches!(layer.content, photocraft_doc::LayerContent::Shape(_)), "the line made a shape layer");
    app
}

/// Right-click the middle of the canvas (empty, away from the line): the open menu's entries as
/// `ui.inspect` lists them.
fn right_click_entries(h: &mut Harness<'static, PhotocraftApp>) -> Vec<Value> {
    let center = h.state().last_canvas_rect.center();
    right_click(h, center, Modifiers::NONE);
    let inspected = crate::control::inspect(h.state(), &h.ctx);
    assert_eq!(inspected["canvasToolMenu"]["tool"], json!("Line"));
    inspected["canvasToolMenu"]["entries"].as_array().cloned().unwrap_or_default()
}

/// The entries' labels, a separator as `None`.
fn shown(entries: &[Value]) -> Vec<Option<String>> {
    entries.iter().map(|e| if e["separator"] == json!(true) { None } else { e["label"].as_str().map(String::from) }).collect()
}

/// The labels of the greyed entries.
fn greyed(entries: &[Value]) -> Vec<String> {
    entries.iter().filter(|e| e["enabled"] == json!(false)).filter_map(|e| e["label"].as_str().map(String::from)).collect()
}

/// The issue's notation: rows joined by " · ", separators written " ― ".
fn menu(spec: &str) -> Vec<Option<String>> {
    spec.split(" ― ")
        .enumerate()
        .flat_map(|(i, group)| (i > 0).then_some(None).into_iter().chain(group.split(" · ").map(|label| Some(label.to_owned()))))
        .collect()
}

fn list(labels: &str) -> Vec<String> {
    labels.split(" · ").map(String::from).collect()
}

#[test]
fn over_a_pixel_layer_the_menu_lists_the_reference_rows() {
    let mut h = harness(pixel_layer());
    let entries = right_click_entries(&mut h);
    assert_eq!(
        shown(&entries),
        menu(
            "Define Custom Shape… ― Blending Options… ― Make Selection… · New Guides From Shape · Fill Path… · Stroke Path… ― \
             Rasterize Layer · Rasterize Layer Style · Rasterize Vector Mask ― Free Transform ― Unite Shapes · Subtract Front Shape · \
             Unite Shapes at Overlap · Subtract Shapes at Overlap ― Copy Fill · Copy Complete Stroke ― Paste Fill · Paste Complete Stroke ― \
             Isolate Layers"
        )
    );
    assert_eq!(
        greyed(&entries),
        list(
            "Define Custom Shape… · Make Selection… · Fill Path… · Stroke Path… · Rasterize Layer · Rasterize Layer Style · \
             Rasterize Vector Mask · Unite Shapes · Subtract Front Shape · Unite Shapes at Overlap · Subtract Shapes at Overlap · \
             Copy Fill · Copy Complete Stroke · Paste Fill · Paste Complete Stroke"
        )
    );
}

#[test]
fn over_a_line_shape_layer_the_menu_lists_the_reference_rows() {
    let mut h = harness(line_layer());
    let entries = right_click_entries(&mut h);
    assert_eq!(
        shown(&entries),
        menu(
            "Define Custom Shape… ― Blending Options… ― Make Selection… · New Guides From Shape · Fill Path… · Stroke Path… ― \
             Rasterize Layer · Rasterize Layer Style · Rasterize Vector Mask ― Free Transform Path ― Unite Shapes · Subtract Front Shape · \
             Unite Shapes at Overlap · Subtract Shapes at Overlap ― Copy Fill · Copy Complete Stroke ― Paste Fill · Paste Complete Stroke ― \
             Isolate Layers"
        )
    );
    assert_eq!(
        greyed(&entries),
        list(
            "Fill Path… · Stroke Path… · Rasterize Layer Style · Rasterize Vector Mask · Unite Shapes · Subtract Front Shape · \
             Unite Shapes at Overlap · Subtract Shapes at Overlap · Paste Fill · Paste Complete Stroke"
        )
    );
}

#[test]
fn path_and_pixels_mode_open_no_menu() {
    for mode in ["path", "pixels"] {
        for mut app in [pixel_layer(), line_layer()] {
            app.ui.tool_options.shape_mode = mode.into();
            let mut h = harness(app);
            let center = h.state().last_canvas_rect.center();
            right_click(&mut h, center, Modifiers::NONE);
            let inspected = crate::control::inspect(h.state(), &h.ctx);
            assert_eq!(inspected["canvasToolMenu"], Value::Null, "{mode} mode");
            assert_eq!(inspected["layerMenu"], Value::Null, "{mode} mode");
        }
    }
}

#[test]
fn unmeasured_layer_kinds_open_no_menu() {
    let mut app = app();
    app.run("type.create", json!({"x": 4, "y": 20, "text": "A", "size": 12})).unwrap();
    app.ui.tool = Tool::Line;
    assert!(!open(&mut app, &Context::default(), Tool::Line, [10.0, 10.0]), "type layer");
    assert!(app.ui.canvas_tool_menu.is_none());
}

/// A row that acts on the path opens the dialog or runs the command the Pen's menu does for the
/// same layer.
#[test]
fn path_rows_run_as_from_the_pen() {
    for command in ["edit.defineCustomShape", "path.toSelection", "path.transform", "path.style.copyFill", "select.isolateLayers"] {
        let mut by_tool = Vec::new();
        for tool in [Tool::Pen, Tool::Line] {
            let mut app = line_layer();
            app.ui.tool = tool;
            assert!(open(&mut app, &Context::default(), tool, [10.0, 10.0]), "{tool:?}");
            let menu = app.ui.canvas_tool_menu.clone().unwrap();
            assert!(available(&app, &menu, command), "{tool:?}: {command} is enabled");
            let before = app.session.journal.len();
            choose(&mut app, &Context::default(), command);
            assert!(!app.ui.status_error, "{tool:?}: {command}: {}", app.ui.status);
            let dialog = app.ui.dialogs.last().map(|d| d.fields.clone());
            let ran: Vec<_> = app.session.journal[before..].iter().map(|(id, _)| id.clone()).collect();
            by_tool.push((dialog, ran, app.session.path_fill_clipboard.is_some()));
        }
        assert_eq!(by_tool[0], by_tool[1], "{command}");
    }
}

/// The layer rows run the menu bar's command.
#[test]
fn layer_rows_run_the_menu_bar_command() {
    let mut app = line_layer();
    assert!(open(&mut app, &Context::default(), Tool::Line, [10.0, 10.0]));
    choose(&mut app, &Context::default(), "layer.rasterize.layer");
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("layer.rasterize.layer"));
    let st = app.session.active().unwrap();
    assert!(matches!(st.doc.layer(st.active_layer.unwrap()).unwrap().content, photocraft_doc::LayerContent::Raster(_)), "rasterized");

    // Over a pixel layer with no path, Free Transform starts the box on the layer.
    let mut app = pixel_layer();
    assert!(open(&mut app, &Context::default(), Tool::Line, [10.0, 10.0]));
    choose(&mut app, &Context::default(), "edit.freeTransform");
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(app.ui.transform.is_some(), "Free Transform started");
}

/// Greyed rows run nothing.
#[test]
fn greyed_rows_run_nothing() {
    let mut app = pixel_layer();
    assert!(open(&mut app, &Context::default(), Tool::Line, [10.0, 10.0]));
    let before = app.session.journal.len();
    for command in ["path.toSelection", "layer.rasterize.layer", "path.style.pasteFill", "path.transform"] {
        choose(&mut app, &Context::default(), command);
        assert!(open(&mut app, &Context::default(), Tool::Line, [10.0, 10.0]));
    }
    assert_eq!(app.session.journal.len(), before);
    assert!(app.ui.dialogs.is_empty());
}
