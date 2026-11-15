//! #13295: Path Selection's pointer over the canvas is its own arrow with the tip on the pointer,
//! not the crosshair: white with a black outline idle and with ⌘, the solid black one with ⌥ over
//! a component (of the target path or of a shape layer). The type-on-path and shape-box cursors still win. Driven through real egui input
//! on the real canvas (`document_area`).

use egui::epaint::ColorMode;
use egui::{Color32, CursorIcon, Modifiers, PointerButton, Pos2, Shape};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::guide_cursor_tests::{harness, hover, screen};
use crate::state::Tool;

/// Path Selection over a work path with one square component, 60–160 on both axes (away from the
/// harness's guide at x = 200).
fn square() -> Harness<'static, PhotocraftApp> {
    let mut h = harness(Tool::PathSelection);
    h.state_mut()
        .run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": [[60, 60], [160, 60], [160, 160], [60, 160]]}]}}))
        .unwrap();
    h.run_steps(1);
    h
}

/// Which arrow is painted: white with a black outline, or solid black with a light one.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Arrow {
    White,
    Black,
}

fn flatten<'a>(shape: &'a Shape, out: &mut Vec<&'a Shape>) {
    match shape {
        Shape::Vec(inner) => inner.iter().for_each(|s| flatten(s, out)),
        s => out.push(s),
    }
}

/// Whether the top left corner of `points` is within a pixel of `tip`.
fn tip_on(points: impl Iterator<Item = Pos2>, tip: Pos2) -> bool {
    let min = points.fold(Pos2::new(f32::MAX, f32::MAX), |m, p| m.min(p));
    (min.x - tip.x).abs() <= 1.0 && (min.y - tip.y).abs() <= 1.0
}

/// The painted arrow whose tip is within a pixel of `tip`: its fill (a seven-corner mesh of one
/// colour) and its closed seven-corner outline, read together.
fn arrow_at(h: &Harness<'_, PhotocraftApp>, tip: Pos2) -> Option<Arrow> {
    let out = h.output();
    let mut shapes = Vec::new();
    out.shapes.iter().for_each(|c| flatten(&c.shape, &mut shapes));
    let fill = shapes.iter().find_map(|s| match s {
        Shape::Mesh(mesh) if mesh.vertices.len() == 7 && tip_on(mesh.vertices.iter().map(|v| v.pos), tip) => {
            let fill = mesh.vertices.first()?.color;
            mesh.vertices.iter().all(|v| v.color == fill).then_some(fill)
        }
        _ => None,
    })?;
    let edge = shapes.iter().find_map(|s| match s {
        Shape::Path(path) if path.closed && path.points.len() == 7 && tip_on(path.points.iter().copied(), tip) => match path.stroke.color {
            ColorMode::Solid(c) => Some(c),
            _ => None,
        },
        _ => None,
    })?;
    let (dark, light) = (edge.r() < 60 && edge.a() > 128, edge.r() > 200 && edge.a() > 128);
    match fill {
        Color32::WHITE if dark => Some(Arrow::White),
        Color32::BLACK if light => Some(Arrow::Black),
        _ => None,
    }
}

/// The OS cursor and the arrow painted at doc point (`x`, `y`) with `m` held.
fn pointer(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, m: Modifiers) -> (CursorIcon, Option<Arrow>) {
    let icon = hover(h, x, y, m);
    (icon, arrow_at(h, screen(h, x, y)))
}

const WHITE_ARROW: (CursorIcon, Option<Arrow>) = (CursorIcon::None, Some(Arrow::White));
const BLACK_ARROW: (CursorIcon, Option<Arrow>) = (CursorIcon::None, Some(Arrow::Black));

/// Two shape layers, 40,40–140,120 and 230,180–350,260, the second one active.
fn two_shapes(h: &mut Harness<'static, PhotocraftApp>) -> u64 {
    let app = h.state_mut();
    app.run("shape.create", json!({"kind": "rect", "rect": [40, 40, 100, 80], "fill": "#e02020"})).unwrap();
    let first = app.session.active().unwrap().active_layer.unwrap();
    app.run("shape.create", json!({"kind": "rect", "rect": [230, 180, 120, 80], "fill": "#2050e0"})).unwrap();
    first.0
}

#[test]
fn idle_and_with_command_the_pointer_is_the_white_arrow() {
    let mut h = square();
    assert_eq!(pointer(&mut h, 300.0, 250.0, Modifiers::NONE), WHITE_ARROW, "over empty canvas");
    assert_eq!(pointer(&mut h, 60.0, 110.0, Modifiers::NONE), WHITE_ARROW, "over the subpath's outline");
    assert_eq!(pointer(&mut h, 110.0, 110.0, Modifiers::NONE), WHITE_ARROW, "inside the component");
    assert_eq!(pointer(&mut h, 60.0, 60.0, Modifiers::COMMAND), WHITE_ARROW, "⌘ over an anchor");
}

#[test]
fn alt_over_a_component_is_the_solid_black_arrow() {
    let mut h = square();
    assert_eq!(pointer(&mut h, 60.0, 110.0, Modifiers::ALT), BLACK_ARROW, "⌥ over the subpath's outline");
    assert_eq!(pointer(&mut h, 110.0, 110.0, Modifiers::ALT), BLACK_ARROW, "⌥ inside the component");
    assert_eq!(pointer(&mut h, 300.0, 250.0, Modifiers::ALT), WHITE_ARROW, "⌥ off every component was not measured: the white arrow");
    // Window › Modifier Keys: a latched ⌥ is ⌥ too.
    h.state_mut().ui.shell.sticky_alt = true;
    assert_eq!(pointer(&mut h, 110.0, 110.0, Modifiers::NONE), BLACK_ARROW, "sticky ⌥ inside the component");
    h.state_mut().ui.shell.sticky_alt = false;
    assert_eq!(pointer(&mut h, 110.0, 110.0, Modifiers::ALT | Modifiers::COMMAND), WHITE_ARROW, "⌘⌥ was not measured: the white arrow");
}

/// An ⌥-drag that starts on the component keeps the solid arrow once the pointer leaves the
/// component's stored outline: it moves only on release.
#[test]
fn an_alt_drag_keeps_the_solid_black_arrow_off_the_component() {
    let mut h = square();
    hover(&mut h, 110.0, 110.0, Modifiers::ALT);
    let p = screen(&h, 110.0, 110.0);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT });
    h.run_steps(1);
    for k in 1..=4 {
        let t = k as f32 / 4.0;
        h.event(egui::Event::PointerMoved(screen(&h, 110.0 + 190.0 * t, 110.0 + 140.0 * t)));
        h.run_steps(1);
    }
    h.run_steps(1);
    assert!(h.state().drag.is_some(), "still dragging");
    assert_eq!(arrow_at(&h, screen(&h, 300.0, 250.0)), Some(Arrow::Black));
    let end = screen(&h, 300.0, 250.0);
    h.event(egui::Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::ALT });
    h.run_steps(2);
}

/// A press with ⌥ over another shape layer selects that layer and lands on its component, so the
/// cursor shows the solid arrow there too.
#[test]
fn alt_over_an_inactive_shape_layer_is_the_solid_black_arrow() {
    let mut h = harness(Tool::PathSelection);
    two_shapes(&mut h);
    assert_eq!(pointer(&mut h, 90.0, 80.0, Modifiers::ALT), BLACK_ARROW, "⌥ inside the inactive shape");
    assert_eq!(pointer(&mut h, 290.0, 220.0, Modifiers::ALT), BLACK_ARROW, "⌥ inside the active shape");
    assert_eq!(pointer(&mut h, 300.0, 80.0, Modifiers::ALT), WHITE_ARROW, "⌥ off both shapes");
}

#[test]
fn precise_other_cursors_and_caps_lock_keep_the_crosshair() {
    let mut h = square();
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
    assert_eq!(pointer(&mut h, 110.0, 110.0, Modifiers::NONE), (CursorIcon::Crosshair, None), "Precise: the crosshair, as today");
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "standard"})).unwrap();
    h.state_mut().caps_lock = true;
    assert_eq!(pointer(&mut h, 110.0, 110.0, Modifiers::NONE), (CursorIcon::Crosshair, None), "Caps Lock: the crosshair, as today");
}

#[test]
fn the_type_on_path_cursor_keeps_priority() {
    let mut h = harness(Tool::PathSelection);
    let app = h.state_mut();
    app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": false, "knots": [[20, 100], [180, 100]]}]}})).unwrap();
    app.ui.selected_path = Some("work".into());
    app.run("type.create", json!({"text": "ABC", "x": 20, "y": 100})).unwrap();
    app.run("type.attachToPath", json!({"start": 20})).unwrap();
    app.run("type.setPathPlacement", json!({"end": 100})).unwrap();
    assert_eq!(pointer(&mut h, 40.0, 100.0, Modifiers::NONE), (CursorIcon::None, None), "over the type: the placement I-beam, not the arrow");
    let xf = crate::canvas::ViewXform::active(h.state()).unwrap();
    let p = screen(&h, 40.0, 100.0);
    assert!(crate::type_path_placement::cursor(h.state_mut(), &xf, p).is_some(), "the placement cursor applies there");
    assert_eq!(pointer(&mut h, 300.0, 250.0, Modifiers::NONE), WHITE_ARROW, "off it: the arrow");
}

#[test]
fn the_shape_box_handle_cursor_keeps_priority() {
    let mut h = harness(Tool::PathSelection);
    let first = two_shapes(&mut h);
    h.state_mut().run("layer.select", json!({"layer": first, "mode": "toggle"})).unwrap();
    assert_eq!(pointer(&mut h, 350.0, 260.0, Modifiers::NONE), (CursorIcon::ResizeNwSe, None), "over the box's corner handle: its scale cursor");
    assert_eq!(pointer(&mut h, 300.0, 120.0, Modifiers::NONE), WHITE_ARROW, "off the box: the arrow");
}
