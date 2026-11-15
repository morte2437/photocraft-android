//! #13243: ⇧ while dragging with the Line tool snaps to multiples of 45° as Photoshop does: a
//! diagonal line ends at the shorter of the drag's two distances on both axes, a horizontal or
//! vertical one keeps the full distance along its axis (and its 0.5 px centring on the pixel).
//! The drag preview shows the same line as the layer, and ⇧ pressed after the start gives the
//! same line as ⇧ held from the press.

use egui::Modifiers;
use photocraft_doc::{LayerContent, LiveShape};
use serde_json::json;

use super::shape_preview;
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The five measured drags from 400,300 and the line Photoshop draws for each: a flat line's
/// ends move 0.5 px across it, onto the middle of the pixel.
const CASES: [([f64; 2], [[f64; 2]; 2]); 5] = [
    ([600.0, 400.0], [[400.0, 299.5], [600.0, 299.5]]),
    ([600.0, 480.0], [[400.0, 300.0], [580.0, 480.0]]),
    ([600.0, 520.0], [[400.0, 300.0], [600.0, 500.0]]),
    ([300.0, 420.0], [[400.0, 300.0], [300.0, 400.0]]),
    ([450.0, 500.0], [[399.5, 300.0], [399.5, 500.0]]),
];

const START: [f64; 2] = [400.0, 300.0];

/// A new 800 × 600 document with the Line tool picked.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Line;
    app
}

/// A drag from `from` to `to`, with `at_press` held at the press and `later` from the first move.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], at_press: Modifiers, later: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, at_press);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, later);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, later);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, later);
}

/// The active layer's shape: its live line's ends, and its outline.
fn line(app: &PhotocraftApp) -> ([f64; 2], [f64; 2], photocraft_doc::vector::Path) {
    let st = app.session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Shape(s) => match &s.live {
            Some(LiveShape::Line { from, to, .. } | LiveShape::ArrowLine { from, to, .. }) => (*from, *to, s.path.clone()),
            other => panic!("a live line, got {other:?}"),
        },
        _ => panic!("a shape layer"),
    }
}

/// The measured ⇧-drags end where Photoshop ends them.
#[test]
fn shift_drags_end_where_photoshop_ends_them() {
    for (to, want) in CASES {
        let mut app = app();
        drag(&mut app, START, to, Modifiers::SHIFT, Modifiers::SHIFT);
        let (from, end, _) = line(&app);
        assert_eq!([from, end], want, "⇧-drag 400,300 → {to:?}");
    }
}

/// ⇧ pressed after the drag started constrains the line as ⇧ held from the press does.
#[test]
fn shift_pressed_after_the_start_gives_the_same_line() {
    for (to, _) in CASES {
        let mut held = app();
        drag(&mut held, START, to, Modifiers::SHIFT, Modifiers::SHIFT);
        let mut later = app();
        drag(&mut later, START, to, Modifiers::NONE, Modifiers::SHIFT);
        assert_eq!(line(&held), line(&later), "⇧ at the press or after it, → {to:?}");
    }
}

/// The drag preview's outline is the outline of the layer the drag makes.
#[test]
fn the_preview_shows_the_line_the_drag_makes() {
    for (to, _) in CASES {
        let mut app = app();
        let preview = shape_preview(&app, Tool::Line, START, to, Modifiers::SHIFT).expect("a preview");
        drag(&mut app, START, to, Modifiers::NONE, Modifiers::SHIFT);
        assert_eq!(preview.path, line(&app).2, "preview of → {to:?}");
    }
}

/// Up-left, up-right and down-left drags snap the same way, mirrored.
#[test]
fn shift_snaps_every_direction() {
    let o = app().ui.tool_options.clone();
    for (to, want) in [([200.0, 180.0], [280.0, 180.0]), ([520.0, 100.0], [520.0, 180.0]), ([380.0, 100.0], [399.5, 100.0]), ([200.0, 310.0], [200.0, 299.5])] {
        let g = super::shape_geometry(&o, Tool::Line, START, to, Modifiers::SHIFT).expect("a line");
        assert_eq!(g["to"], json!(want), "⇧-drag 400,300 → {to:?}: {g}");
    }
}

/// CodeRabbit on #13283: a short fractional ⇧-drag still draws a one-pixel diagonal. Snapped to
/// 0.65 × 0.65 it is shorter than a pixel, but the drag itself is not, and its ends round to a
/// whole-pixel 45° step.
#[test]
fn a_short_fractional_shift_drag_still_draws_a_diagonal() {
    let o = app().ui.tool_options.clone();
    let g = super::shape_geometry(&o, Tool::Line, [10.0, 10.0], [11.1, 10.65], Modifiers::SHIFT).expect("a line");
    assert_eq!((g["from"].clone(), g["to"].clone()), (json!([10.0, 10.0]), json!([11.0, 11.0])), "{g}");
}

/// CodeRabbit on #13283: a short ⇧-drag near an axis still draws a one-pixel line. With
/// Preferences › Tools › Snap Vector Tools and Transforms to Pixel Grid off, 0.96 × 0.30 snaps
/// flat, and its 0.96 px along the axis would round both ends to x 201; the preview and the layer
/// both show a line one pixel long.
#[test]
fn a_short_shift_drag_near_an_axis_still_draws_a_line() {
    for (from, to, want) in [
        ([200.51, 100.0], [201.47, 100.30], [[201.0, 99.5], [202.0, 99.5]]),
        ([200.0, 100.51], [200.30, 101.47], [[199.5, 101.0], [199.5, 102.0]]),
        ([201.47, 100.0], [200.51, 100.30], [[201.0, 99.5], [200.0, 99.5]]),
    ] {
        let mut app = app();
        app.run("prefs.set", json!({"path": "tools.snapVectorToolsAndTransformsToPixelGrid", "value": false})).unwrap();
        app.ui.views[0].zoom = 16.0;
        let preview = shape_preview(&app, Tool::Line, from, to, Modifiers::SHIFT).expect("a preview");
        drag(&mut app, from, to, Modifiers::SHIFT, Modifiers::SHIFT);
        let (a, b, path) = line(&app);
        assert_eq!([a, b], want, "⇧-drag {from:?} → {to:?}");
        assert_eq!(preview.path, path, "preview of {from:?} → {to:?}");
    }
}
