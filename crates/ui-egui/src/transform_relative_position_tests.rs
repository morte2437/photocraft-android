//! Relative positioning in the transform options bar (#11679): a toggle between X and Y, off by
//! default; on, X and Y show the reference point's offset from where the box started and a typed
//! value moves the box by that much; off, they show its position again.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;

const TOGGLE: &str = "Use relative positioning for reference point";

/// A 200 x 120 document with a layer filled at 40,20 (60 x 40) and Free Transform open on it:
/// the reference point starts at the box's centre, 70,40.
fn transforming() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 40, "y": 20, "width": 60, "height": 40})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    crate::transform_tool::begin(&mut app, &egui::Context::default()).unwrap();
    app
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1600.0, 200.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

/// What the X and Y fields show.
fn shown(h: &Harness<'_, PhotocraftApp>) -> (f64, f64) {
    let read = |name: &str| h.get_by_role_and_label(Role::SpinButton, name).accesskit_node().numeric_value().unwrap();
    (read("X"), read("Y"))
}

/// Types `text` into the X field and commits it with Tab, as a user does.
fn type_x(h: &mut Harness<'_, PhotocraftApp>, text: &str) {
    h.get_by_role_and_label(Role::SpinButton, "X").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    for ch in text.chars() {
        h.event(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn click_toggle(h: &mut Harness<'_, PhotocraftApp>) {
    h.get_by_label(TOGGLE).click();
    h.run_steps(3);
}

fn quad(h: &Harness<'_, PhotocraftApp>) -> [[f64; 2]; 4] {
    h.state().ui.transform.as_ref().unwrap().quad
}

fn moved(q: [[f64; 2]; 4], dx: f64) -> [[f64; 2]; 4] {
    q.map(|[x, y]| [x + dx, y])
}

fn same(a: [[f64; 2]; 4], b: [[f64; 2]; 4]) -> bool {
    a.iter().flatten().zip(b.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-6)
}

#[test]
fn relative_positioning_is_off_by_default() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert!(!app.ui.tool_options.transform_relative_position);
    // An older preferences file without the field reads it as off.
    let o: crate::state::ToolOptions = serde_json::from_value(json!({})).unwrap();
    assert!(!o.transform_relative_position);
}

#[test]
fn the_toggle_sits_between_x_and_y_and_turns_a_fresh_box_into_zero() {
    let mut h = harness(transforming());
    let x = h.get_by_role_and_label(Role::SpinButton, "X").rect();
    let y = h.get_by_role_and_label(Role::SpinButton, "Y").rect();
    let toggle = h.get_by_label(TOGGLE).rect();
    assert!(x.max.x <= toggle.min.x && toggle.max.x <= y.min.x, "X {x:?}, toggle {toggle:?}, Y {y:?}");
    assert_eq!(shown(&h), (70.0, 40.0), "off: the reference point's position");
    click_toggle(&mut h);
    assert!(h.state().ui.tool_options.transform_relative_position);
    assert_eq!(shown(&h), (0.0, 0.0), "on: no offset yet");
}

#[test]
fn a_typed_x_moves_the_box_by_that_much_and_commits_as_free_transform() {
    let mut h = harness(transforming());
    click_toggle(&mut h);
    let before = quad(&h);
    type_x(&mut h, "20");
    assert!(same(quad(&h), moved(before, 20.0)), "the box moved 20 px right: {:?}", quad(&h));
    assert_eq!(shown(&h), (20.0, 0.0));
    // Typing the same offset again doesn't move it further.
    type_x(&mut h, "20");
    assert!(same(quad(&h), moved(before, 20.0)), "still 20 px: {:?}", quad(&h));
    let app = h.state_mut();
    crate::transform_tool::commit(app);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds().x0, 60, "the layer moved 20 px");
    assert_eq!(st.history.entries().last().map(ToString::to_string).as_deref(), Some("Free Transform"));
}

#[test]
fn a_drag_updates_the_offset_and_turning_it_off_shows_the_position_again() {
    let mut h = harness(transforming());
    click_toggle(&mut h);
    {
        let app = h.state_mut();
        let none = egui::Modifiers::NONE;
        crate::transform_tool::pointer(app, ToolEvent::Down { x: 50.0, y: 30.0, pressure: 1.0 }, none);
        crate::transform_tool::pointer(app, ToolEvent::Move { x: 55.0, y: 33.0, pressure: 1.0 }, none);
        crate::transform_tool::pointer(app, ToolEvent::Up { x: 55.0, y: 33.0 }, none);
    }
    h.run_steps(3);
    assert_eq!(shown(&h), (5.0, 3.0), "the offset follows the drag");
    click_toggle(&mut h);
    assert!(!h.state().ui.tool_options.transform_relative_position);
    assert_eq!(shown(&h), (75.0, 43.0), "off again: the reference point's position");
}

/// Typing X leaves Y exactly where it was, even when Y is a fraction the field's f32 can't hold
/// (CodeRabbit review on options_bar.rs:308): the field's rounding must not nudge the box.
#[test]
fn a_typed_x_leaves_a_fractional_y_exactly_where_it_was() {
    let mut app = transforming();
    let t = app.ui.transform.as_mut().unwrap();
    t.quad = t.quad.map(|[x, y]| [x, y + 0.1]);
    t.pivot[1] += 0.1;
    let mut h = harness(app);
    let before = quad(&h);
    let pivot_y = h.state().ui.transform.as_ref().unwrap().pivot[1];
    type_x(&mut h, "90");
    let after = quad(&h);
    assert!(same(after, moved(before, 20.0)), "the box moved 20 px right: {after:?}");
    assert_eq!(after.map(|p| p[1]), before.map(|p| p[1]), "Y is untouched");
    assert_eq!(h.state().ui.transform.as_ref().unwrap().pivot[1], pivot_y);
}

/// A box squashed flat has no inverse map, so the offset comes from the map's pseudo-inverse: a
/// box squashed to zero width about its reference point reads 0 / 0, and one squashed onto its
/// left edge, carrying the reference point with it, reads -30 / 0 (CodeRabbit pre-merge check).
#[test]
fn a_flattened_box_reads_the_reference_points_own_offset() {
    let mut app = transforming();
    app.ui.tool_options.transform_relative_position = true;
    let t = app.ui.transform.as_mut().unwrap();
    t.quad = [[70.0, 20.0], [70.0, 20.0], [70.0, 60.0], [70.0, 60.0]];
    t.pivot = [70.0, 40.0];
    let mut h = harness(app);
    assert_eq!(shown(&h), (0.0, 0.0), "squashed about the reference point");
    let t = h.state_mut().ui.transform.as_mut().unwrap();
    t.quad = [[40.0, 20.0], [40.0, 20.0], [40.0, 60.0], [40.0, 60.0]];
    t.pivot = [40.0, 40.0];
    h.run_steps(2);
    assert_eq!(shown(&h), (-30.0, 0.0), "squashed onto the left edge");
}
