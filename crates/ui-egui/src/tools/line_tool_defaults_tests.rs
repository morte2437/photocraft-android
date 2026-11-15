//! #13241: the Line tool starts, and resets to, Weight 1 px with arrowheads 5 px wide and 10 px
//! long (Start and End off, Concavity 0 %), as the reference application does.

use egui::Modifiers;
use photocraft_doc::LayerContent;
use photocraft_doc::vector::LiveShape;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{Tool, ToolOptions};

fn assert_line_defaults(o: &ToolOptions) {
    assert_eq!(o.line_weight, 1.0, "Weight");
    assert!(!o.arrow_start && !o.arrow_end, "Start and End off");
    assert_eq!(o.arrow_width, 5.0, "arrowhead Width");
    assert_eq!(o.arrow_length, 10.0, "arrowhead Length");
    assert_eq!(o.arrow_concavity, 0.0, "arrowhead Concavity");
}

#[test]
fn fresh_settings_give_the_reference_line_defaults() {
    assert_line_defaults(&ToolOptions::default());
}

#[test]
fn reset_tool_gives_the_reference_line_defaults() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.tool = Tool::Line;
    let o = &mut app.ui.tool_options;
    (o.line_weight, o.arrow_start, o.arrow_end, o.arrow_width, o.arrow_length, o.arrow_concavity) = (3.0, true, true, 52.0, 50.0, 40.0);
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tools::tool_reset::RESET, json!({})).unwrap();
    assert_line_defaults(&app.ui.tool_options);
}

#[test]
fn a_default_drag_with_end_on_makes_a_1_px_shaft_and_a_10_by_5_px_head() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.ui.tool = Tool::Line;
    app.ui.tool_options.arrow_end = true;
    let none = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 400.0, y: 300.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Move { x: 600.0, y: 300.0, pressure: 1.0 }, none);
    tool_event(&mut app, ToolEvent::Up { x: 600.0, y: 300.0 }, none);

    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let LayerContent::Shape(shape) = &layer.content else { panic!("a shape layer: {:?}", layer.content) };
    let Some(LiveShape::ArrowLine { from, to, weight, start, end, width, length, concavity }) = shape.live.clone() else {
        panic!("a live arrow line: {:?}", shape.live)
    };
    assert_eq!((weight, start, end, width, length, concavity), (1.0, false, true, 5.0, 10.0, 0.0));
    assert_eq!(to[0], 600.0, "the head's tip is at the drag's end");
    assert_eq!(from[1], to[1], "a horizontal line");

    // The shaft, then the head: its tip at 600, 10 px long and 5 px wide.
    let [shaft, head] = &shape.path.subpaths[..] else { panic!("a shaft and one head: {:?}", shape.path) };
    let bounds = |sp: &photocraft_doc::vector::Subpath| photocraft_doc::vector::Path::new(vec![sp.clone()]).control_bounds().unwrap();
    let (_, sy0, _, sy1) = bounds(shaft);
    assert_eq!(sy1 - sy0, 1.0, "a 1 px shaft");
    let (hx0, hy0, hx1, hy1) = bounds(head);
    assert_eq!((hx0, hx1), (590.0, 600.0), "a head 10 px long ending at the tip");
    assert_eq!(hy1 - hy0, 5.0, "a head 5 px wide");
}

#[test]
fn line_options_saved_before_the_new_defaults_keep_their_values() {
    let saved = json!({"line_weight": 3.0, "arrow_end": true, "arrow_width": 52.0, "arrow_length": 50.0, "arrow_concavity": 25.0});
    let o: ToolOptions = serde_json::from_value(saved).unwrap();
    assert_eq!((o.line_weight, o.arrow_end, o.arrow_width, o.arrow_length, o.arrow_concavity), (3.0, true, 52.0, 50.0, 25.0));
    assert!(!o.arrow_start, "a field the file lacks takes its default");
}

#[test]
fn line_options_missing_from_a_saved_file_take_the_new_defaults() {
    let o: ToolOptions = serde_json::from_value(json!({"tolerance": 10.0})).unwrap();
    assert_line_defaults(&o);
}
