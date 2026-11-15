//! #13343: the Rectangle tool's gear: Unconstrained / Square / Fixed Size / Proportional and From
//! Center, as the reference application measures them on an 800 × 600 document. ⇧ during a drag
//! turns an active Square off and ⌥ an active From Center, instead of adding to them.

use egui::accesskit::{Role, Toggled};
use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{Tool, ToolOptions};

const GEAR: &str = "Set additional shape and path options";
const RADIOS: [&str; 4] = ["Unconstrained", "Square", "Fixed Size", "Proportional"];
const FIXED: [&str; 2] = ["Fixed Size W:", "Fixed Size H:"];
const RATIO: [&str; 2] = ["Proportional W:", "Proportional H:"];

/// An 800 × 600 document with `tool` active and its options set by `set`.
fn app(tool: Tool, set: impl FnOnce(&mut ToolOptions)) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    set(&mut app.ui.tool_options);
    app
}

/// A canvas drag from `from` to `to`, pressed with `press` held and moved and released with `held`
/// (a key pressed after the drag starts), as the canvas sends it.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], press: Modifiers, held: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, press);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, held);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, held);
}

/// The live rectangle a drag with `tool` and options `set` draws, as `[x0, y0, x1, y1]`.
fn drawn(tool: Tool, set: impl FnOnce(&mut ToolOptions), from: [f64; 2], to: [f64; 2], held: Modifiers) -> [f64; 4] {
    let mut app = app(tool, set);
    let layers = layer_count(&app);
    drag(&mut app, from, to, Modifiers::NONE, held);
    assert_eq!(layer_count(&app), layers + 1, "the drag draws one shape layer");
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    let info = app.session.execute("shape.info", json!({"layer": id})).unwrap();
    let r: [f64; 4] = serde_json::from_value(info["live"]["rect"].clone()).unwrap_or_else(|_| panic!("a live box: {}", info["live"]));
    [r[0], r[1], r[0] + r[2], r[1] + r[3]]
}

fn layer_count(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

/// The Rectangle tool with the gear's `constraint`.
fn constraint(c: &'static str) -> impl FnOnce(&mut ToolOptions) {
    move |o| o.rect_constraint = c.into()
}

#[test]
fn square_takes_the_shorter_side_of_the_drag() {
    // D1 and D1b.
    assert_eq!(drawn(Tool::Rectangle, constraint("square"), [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [400.0, 300.0, 500.0, 400.0]);
    assert_eq!(drawn(Tool::Rectangle, constraint("square"), [400.0, 300.0], [450.0, 500.0], Modifiers::NONE), [400.0, 300.0, 450.0, 350.0]);
    // Towards the top left, the square still grows from the press point.
    assert_eq!(drawn(Tool::Rectangle, constraint("square"), [400.0, 300.0], [200.0, 250.0], Modifiers::NONE), [350.0, 250.0, 400.0, 300.0]);
}

#[test]
fn shift_during_the_drag_turns_square_off_and_still_squares_an_unconstrained_one() {
    // D6.
    assert_eq!(drawn(Tool::Rectangle, constraint("square"), [400.0, 300.0], [600.0, 400.0], Modifiers::SHIFT), [400.0, 300.0, 600.0, 400.0]);
    assert_eq!(drawn(Tool::Rectangle, constraint("unconstrained"), [400.0, 300.0], [600.0, 400.0], Modifiers::SHIFT), [400.0, 300.0, 500.0, 400.0]);
    assert_eq!(drawn(Tool::Rectangle, constraint("unconstrained"), [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [400.0, 300.0, 600.0, 400.0]);
}

/// Fixed Size `w` × `h`.
fn fixed(w: f32, h: f32) -> impl FnOnce(&mut ToolOptions) {
    move |o| (o.rect_constraint, o.rect_fixed_width, o.rect_fixed_height) = ("fixedSize".into(), w, h)
}

#[test]
fn fixed_size_puts_its_top_left_corner_at_the_release_point() {
    // D2, and the same from a drag the other way: the corner follows the pointer, not the press.
    assert_eq!(drawn(Tool::Rectangle, fixed(120.0, 60.0), [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [600.0, 400.0, 720.0, 460.0]);
    assert_eq!(drawn(Tool::Rectangle, fixed(120.0, 60.0), [600.0, 400.0], [400.0, 300.0], Modifiers::NONE), [400.0, 300.0, 520.0, 360.0]);
    // A field left empty (as both start) draws the drag as Unconstrained; not measured.
    assert_eq!(drawn(Tool::Rectangle, fixed(120.0, 0.0), [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [400.0, 300.0, 600.0, 400.0]);
}

#[test]
fn a_click_with_fixed_size_still_opens_create_rectangle() {
    // D2b: the dialog, and no fixed-size rectangle dropped.
    let mut app = app(Tool::Rectangle, fixed(120.0, 60.0));
    let layers = layer_count(&app);
    tool_event(&mut app, ToolEvent::Down { x: 400.0, y: 300.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 400.0, y: 300.0 }, Modifiers::NONE);
    let [dialog] = app.ui.dialogs.as_slice() else { panic!("expected one Create Rectangle dialog") };
    assert_eq!(dialog.fields["__label"], "Create Rectangle");
    assert_eq!((dialog.fields["width"].as_f64(), dialog.fields["height"].as_f64()), (Some(100.0), Some(100.0)));
    assert_eq!(layer_count(&app), layers);
}

/// Proportional `w` : `h`.
fn ratio(w: f32, h: f32) -> impl FnOnce(&mut ToolOptions) {
    move |o| (o.rect_constraint, o.rect_ratio_width, o.rect_ratio_height) = ("proportional".into(), w, h)
}

#[test]
fn proportional_draws_the_largest_box_of_its_ratio_inside_the_drag() {
    // D3 and D3b.
    assert_eq!(drawn(Tool::Rectangle, ratio(2.0, 1.0), [400.0, 300.0], [600.0, 500.0], Modifiers::NONE), [400.0, 300.0, 600.0, 400.0]);
    assert_eq!(drawn(Tool::Rectangle, ratio(2.0, 1.0), [400.0, 300.0], [450.0, 400.0], Modifiers::NONE), [400.0, 300.0, 450.0, 325.0]);
    // From the press point towards the top left, and a ratio that is not whole.
    assert_eq!(drawn(Tool::Rectangle, ratio(2.0, 1.0), [400.0, 300.0], [200.0, 100.0], Modifiers::NONE), [200.0, 200.0, 400.0, 300.0]);
    assert_eq!(drawn(Tool::Rectangle, ratio(1.5, 2.5), [100.0, 100.0], [400.0, 600.0], Modifiers::NONE), [100.0, 100.0, 400.0, 600.0]);
    assert_eq!(drawn(Tool::Rectangle, ratio(1.5, 2.5), [100.0, 100.0], [400.0, 400.0], Modifiers::NONE), [100.0, 100.0, 280.0, 400.0]);
}

#[test]
fn from_center_draws_from_the_press_point_and_alt_turns_it_off() {
    let from_center = |o: &mut ToolOptions| o.rect_from_center = true;
    // D4 and D5.
    assert_eq!(drawn(Tool::Rectangle, from_center, [400.0, 300.0], [500.0, 350.0], Modifiers::NONE), [300.0, 250.0, 500.0, 350.0]);
    assert_eq!(drawn(Tool::Rectangle, from_center, [400.0, 300.0], [500.0, 350.0], Modifiers::ALT), [400.0, 300.0, 500.0, 350.0]);
    // Off, ⌥ still draws from the centre.
    assert_eq!(drawn(Tool::Rectangle, |_| {}, [400.0, 300.0], [500.0, 350.0], Modifiers::ALT), [300.0, 250.0, 500.0, 350.0]);
    // From Center with a constraint: the constrained box about the press point.
    let square_from_center = |o: &mut ToolOptions| (o.rect_constraint, o.rect_from_center) = ("square".into(), true);
    assert_eq!(drawn(Tool::Rectangle, square_from_center, [400.0, 300.0], [500.0, 350.0], Modifiers::NONE), [350.0, 250.0, 450.0, 350.0]);
}

#[test]
fn the_gear_settings_are_the_rectangles_alone() {
    let gear = |o: &mut ToolOptions| (o.rect_constraint, o.rect_from_center) = ("square".into(), true);
    assert_eq!(drawn(Tool::EllipseShape, gear, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE), [400.0, 300.0, 600.0, 400.0]);
}

#[test]
fn path_mode_draws_the_constrained_outline() {
    let mut app = app(Tool::Rectangle, |o| (o.rect_constraint, o.shape_mode) = ("square".into(), "path".into()));
    drag(&mut app, [400.0, 300.0], [600.0, 400.0], Modifiers::NONE, Modifiers::NONE);
    let path = app.session.active().unwrap().doc.work_path.clone().expect("the work path");
    let pts: Vec<_> = path.subpaths.iter().flat_map(|s| s.knots.iter().map(|k| k.anchor)).collect();
    let xs = pts.iter().map(|p| p.x);
    let ys = pts.iter().map(|p| p.y);
    let bounds = [xs.clone().fold(f64::MAX, f64::min), ys.clone().fold(f64::MAX, f64::min), xs.fold(f64::MIN, f64::max), ys.fold(f64::MIN, f64::max)];
    assert_eq!(bounds, [400.0, 300.0, 500.0, 400.0]);
}

// ---------------------------------------------------------------------------------------------
// The gear popup

/// True once `setup_context` installed the app's fonts (the first frame runs before it).
fn fonts_ready(ui: &egui::Ui) -> bool {
    ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into())))
}

/// The options bar with `tool` active.
fn bar(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(2400.0, 400.0)).build_ui_state(
        |ui, app| {
            if fonts_ready(ui) {
                crate::panels::options_bar(app, ui);
            }
        },
        app(tool, |_| {}),
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// The Rectangle tool's options bar with its gear popup open.
fn gear() -> Harness<'static, PhotocraftApp> {
    let mut h = bar(Tool::Rectangle);
    h.get_by_label(GEAR).click();
    h.run_steps(4);
    h
}

fn spin<'a>(h: &'a Harness<'static, PhotocraftApp>, label: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("the {label} field"))
}

fn checked(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().toggled() == Some(Toggled::True)
}

/// Whether each of `labels`' fields can be edited.
fn enabled(h: &Harness<'static, PhotocraftApp>, labels: [&str; 2]) -> [bool; 2] {
    labels.map(|l| !spin(h, l).accesskit_node().is_disabled())
}

/// What `label`'s field shows as a number (None when empty).
fn shown(h: &Harness<'static, PhotocraftApp>, label: &str) -> Option<f64> {
    spin(h, label).accesskit_node().numeric_value().filter(|_| spin(h, label).accesskit_node().value().as_deref() != Some(""))
}

/// Types `text` into `label`'s field, then Tabs out of it.
fn type_into(h: &mut Harness<'static, PhotocraftApp>, label: &str, text: &str) {
    spin(h, label).click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

#[test]
fn only_the_rectangle_has_the_gear_after_its_path_arrangement() {
    let h = bar(Tool::Rectangle);
    let (gear, arrange) = (h.get_by_label(GEAR).rect(), h.get_by_label("Path arrangement").rect());
    assert!(arrange.right() <= gear.left() && (gear.center().y - arrange.center().y).abs() < 4.0, "{gear:?} after {arrange:?}");
    for tool in [Tool::EllipseShape, Tool::Triangle, Tool::Polygon] {
        assert!(bar(tool).query_by_label(GEAR).is_none(), "{tool:?}");
    }
}

#[test]
fn the_popup_shows_the_controls_with_their_defaults() {
    let h = bar(Tool::Rectangle);
    assert!(h.query_by_label("From Center").is_none(), "only in the gear's popup");
    let h = gear();
    let radios: Vec<_> = RADIOS.iter().map(|l| h.get_by_role_and_label(Role::RadioButton, l).rect()).collect();
    assert!(radios.windows(2).all(|w| w[0].top() < w[1].top()), "top to bottom: {radios:?}");
    assert!(h.get_by_label("From Center").rect().top() > radios[3].top(), "From Center comes last");
    assert_eq!(RADIOS.map(|l| checked(&h, l)), [true, false, false, false], "Unconstrained by default");
    assert!(!checked(&h, "From Center"));
    assert_eq!(FIXED.map(|l| shown(&h, l)), [None, None], "Fixed Size starts empty");
    assert_eq!(RATIO.map(|l| shown(&h, l)), [Some(1.0), Some(1.0)], "Proportional starts at 1 : 1");
    assert_eq!(enabled(&h, FIXED), [false, false], "greyed unless Fixed Size is chosen");
    assert_eq!(enabled(&h, RATIO), [false, false], "greyed unless Proportional is chosen");
}

#[test]
fn a_radio_enables_its_own_fields_and_they_take_typed_sizes() {
    let mut h = gear();
    h.get_by_role_and_label(Role::RadioButton, "Fixed Size").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.rect_constraint, "fixedSize");
    assert_eq!((enabled(&h, FIXED), enabled(&h, RATIO)), ([true, true], [false, false]));
    type_into(&mut h, FIXED[0], "120");
    type_into(&mut h, FIXED[1], "60");
    let o = &h.state().ui.tool_options;
    assert_eq!((o.rect_fixed_width, o.rect_fixed_height), (120.0, 60.0));
    assert_eq!(FIXED.map(|l| shown(&h, l)), [Some(120.0), Some(60.0)]);

    h.get_by_role_and_label(Role::RadioButton, "Proportional").click();
    h.run_steps(3);
    assert_eq!((enabled(&h, FIXED), enabled(&h, RATIO)), ([false, false], [true, true]));
    type_into(&mut h, RATIO[0], "2");
    let o = &h.state().ui.tool_options;
    assert_eq!((o.rect_constraint.as_str(), o.rect_ratio_width, o.rect_ratio_height), ("proportional", 2.0, 1.0));
    assert_eq!((o.rect_fixed_width, o.rect_fixed_height), (120.0, 60.0), "Fixed Size keeps its size");

    h.get_by_label("From Center").click();
    h.run_steps(3);
    assert!(h.state().ui.tool_options.rect_from_center);
}

#[test]
fn escape_closes_the_popup_and_keeps_the_change() {
    let mut h = gear();
    h.get_by_role_and_label(Role::RadioButton, "Square").click();
    h.run_steps(3);
    assert!(h.query_by_label("From Center").is_some(), "a radio click leaves the popup open");
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert!(h.query_by_label("From Center").is_none(), "Escape closes the popup");
    assert_eq!(h.state().ui.tool_options.rect_constraint, "square");
}

#[test]
fn the_settings_are_saved_and_reset_tool_restores_them() {
    // Saved with the tool options; a save from before the gear reads as its defaults.
    let mut v = serde_json::to_value(ToolOptions::default()).unwrap();
    let saved = json!({"rect_constraint": "unconstrained", "rect_fixed_width": 0.0, "rect_fixed_height": 0.0,
        "rect_ratio_width": 1.0, "rect_ratio_height": 1.0, "rect_from_center": false});
    for (k, want) in saved.as_object().unwrap() {
        assert_eq!(&v[k], want, "{k}");
    }
    v["rect_constraint"] = json!("fixedSize");
    v["rect_fixed_width"] = json!(120.0);
    v["rect_from_center"] = json!(true);
    let on: ToolOptions = serde_json::from_value(v.clone()).unwrap();
    assert_eq!((on.rect_constraint.as_str(), on.rect_fixed_width, on.rect_from_center), ("fixedSize", 120.0, true));
    for k in saved.as_object().unwrap().keys() {
        v.as_object_mut().unwrap().remove(k);
    }
    let old: ToolOptions = serde_json::from_value(v).unwrap();
    assert_eq!((old.rect_constraint.as_str(), old.rect_ratio_width, old.rect_from_center), ("unconstrained", 1.0, false));

    // Reset Tool: the Ellipse's leaves the Rectangle's gear alone; the Rectangle's restores it.
    let ctx = egui::Context::default();
    let mut app = app(Tool::EllipseShape, |o| {
        (o.rect_constraint, o.rect_fixed_width, o.rect_fixed_height, o.rect_from_center) = ("fixedSize".into(), 120.0, 60.0, true);
    });
    crate::menus::invoke(&mut app, &ctx, "tool.reset", json!({})).unwrap();
    assert_eq!((app.ui.tool_options.rect_constraint.as_str(), app.ui.tool_options.rect_from_center), ("fixedSize", true));
    app.ui.tool = Tool::Rectangle;
    crate::menus::invoke(&mut app, &ctx, "tool.reset", json!({})).unwrap();
    let (o, d) = (&app.ui.tool_options, ToolOptions::default());
    assert_eq!((o.rect_constraint.as_str(), o.rect_from_center), ("unconstrained", false));
    assert_eq!((o.rect_fixed_width, o.rect_fixed_height, o.rect_ratio_width, o.rect_ratio_height), (d.rect_fixed_width, d.rect_fixed_height, 1.0, 1.0));
}
