//! #12917: Make Selection from a path is Photoshop's dialog: Rendering (Feather Radius in pixels,
//! Anti-aliased) and Operation (New / Add to / Subtract from / Intersect with Selection, the last
//! three greyed without a selection). OK with the defaults selects inside the path in one
//! Selection Change History step; each Operation runs `path.toSelection` with its mode. ⌘↩ and the
//! combining operations keep today's Make Selection step (not measured).

use egui::accesskit::{Role, Toggled};
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::{Harness, Node};
use serde_json::{Value, json};

use super::make_selection;
use super::tests::choose_open;
use crate::PhotocraftApp;
use crate::state::Tool;

const OPERATIONS: [&str; 4] = ["New Selection", "Add to Selection", "Subtract from Selection", "Intersect with Selection"];
const MODES: [&str; 4] = ["replace", "add", "subtract", "intersect"];

/// A document with the reference's closed triangle as the Work Path, with or without a selection
/// (a rectangle the triangle overlaps).
fn app(selection: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 500})).unwrap();
    let knots = json!([[100, 300], [300, 300], [200, 450]]);
    app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": knots}]}})).unwrap();
    if selection {
        app.run("select.rect", json!({"x": 20, "y": 20, "width": 200, "height": 320})).unwrap();
    }
    app.ui.tool = Tool::Pen;
    app
}

fn draw(ui: &mut egui::Ui, app: &mut PhotocraftApp) {
    if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
        crate::dialogs::show(app, ui.ctx());
    }
}

/// The Pen canvas menu's Make Selection… dialog, drawn as the window shows it.
fn dialog(mut app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    choose_open(&mut app, "path.toSelection");
    assert_eq!(app.ui.dialogs.len(), 1, "Make Selection… opened its dialog");
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 700.0)).build_ui_state(draw, app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn click(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn steps(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().history.entries().iter().map(|e| e.to_string()).collect()
}

fn last_step(app: &PhotocraftApp) -> Option<String> {
    steps(app).last().cloned()
}

fn selection_bounds(app: &PhotocraftApp) -> Option<[i32; 4]> {
    let r = app.session.active().unwrap().doc.selection.as_ref()?.content_bounds();
    Some([r.x0, r.y0, r.x1, r.y1])
}

/// The params of the last command run.
fn last_params(app: &PhotocraftApp) -> Value {
    let (id, params) = app.session.journal.last().cloned().unwrap();
    assert_eq!(id, "path.toSelection");
    params
}

fn enabled(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    !h.get_by_label(label).accesskit_node().is_disabled()
}

fn feather_field<'h>(h: &'h Harness<'_, PhotocraftApp>) -> Node<'h> {
    h.get_all_by_label("Feather Radius:").find(|n| n.accesskit_node().role() == Role::SpinButton).expect("the Feather Radius number field")
}

#[test]
fn the_dialog_shows_the_rendering_and_operation_groups() {
    let h = dialog(app(false));
    for label in ["Make Selection", "Rendering", "pixels", "Anti-aliased", "Operation", "OK", "Cancel"] {
        assert!(h.query_by_label(label).is_some(), "{label} is shown");
    }
    for old in ["Feather", "Anti Alias", "Mode"] {
        assert!(h.query_by_label(old).is_none(), "the schema dialog's {old} is gone");
    }
    assert_eq!(h.get_by_label("Anti-aliased").accesskit_node().toggled(), Some(Toggled::True), "Anti-aliased is on");
    assert_eq!(h.get_by_label("New Selection").accesskit_node().toggled(), Some(Toggled::True), "New Selection is chosen");
    assert_eq!(feather_field(&h).accesskit_node().numeric_value(), Some(0.0), "Feather Radius is 0");
    let tops: Vec<f32> = OPERATIONS.iter().map(|label| h.get_by_label(label).rect().top()).collect();
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "the operations are listed in order: {tops:?}");
    let rendering = h.get_by_label("Rendering").rect().top();
    let operation = h.get_by_label("Operation").rect().top();
    let feather = feather_field(&h).rect().top();
    assert!(rendering < feather && feather < operation && operation < tops[0], "Rendering holds Feather Radius, Operation the radio buttons");
}

#[test]
fn the_options_bar_button_opens_the_same_dialog() {
    let mut app = app(false);
    make_selection(&mut app).unwrap();
    let fields = &app.ui.dialogs.last().unwrap().fields;
    assert!(super::owns_make_selection(fields));
    assert_eq!(fields.get("__command"), Some(&json!("path.toSelection")));
    assert!(fields.contains_key("__deselectPath"), "OK deselects the path, as from the canvas menu");
}

#[test]
fn without_a_selection_only_new_selection_is_available() {
    let h = dialog(app(false));
    assert!(enabled(&h, "New Selection"));
    for label in &OPERATIONS[1..] {
        assert!(!enabled(&h, label), "{label} is greyed out without a selection");
    }
}

#[test]
fn with_a_selection_every_operation_is_available() {
    let h = dialog(app(true));
    for label in OPERATIONS {
        assert!(enabled(&h, label), "{label} is available with a selection");
    }
}

/// A combining mode the dialog was given (by automation, or before the selection went) falls
/// back to New Selection when there is nothing to combine with.
#[test]
fn without_a_selection_a_combining_mode_falls_back_to_new_selection() {
    let mut h = dialog(app(false));
    h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("mode".into(), json!("subtract"));
    h.run_steps(2);
    assert_eq!(h.get_by_label("New Selection").accesskit_node().toggled(), Some(Toggled::True));
    click(&mut h, "OK");
    assert_eq!(last_params(h.state()).get("mode"), Some(&json!("replace")));
    assert_eq!(selection_bounds(h.state()), Some([100, 300, 300, 450]));
}

#[test]
fn ok_with_the_defaults_selects_the_triangle_in_one_selection_change_step() {
    let mut h = dialog(app(false));
    let before = steps(h.state());
    click(&mut h, "OK");
    assert!(h.state().ui.dialogs.is_empty(), "OK closed the dialog");
    assert_eq!(selection_bounds(h.state()), Some([100, 300, 300, 450]));
    let after = steps(h.state());
    assert_eq!(after.len(), before.len() + 1, "one History step: {after:?}");
    assert_eq!(after[..before.len()], before[..], "the earlier steps keep their names");
    assert_eq!(after.last().map(String::as_str), Some("Selection Change"));
    assert_eq!(h.state().ui.selected_path, None, "OK deselects the path, as ⌘↩ does");
}

#[test]
fn each_operation_runs_make_selection_with_its_mode() {
    for (label, mode) in OPERATIONS.into_iter().zip(MODES) {
        let mut h = dialog(app(true));
        click(&mut h, label);
        click(&mut h, "OK");
        assert_eq!(last_params(h.state()).get("mode").and_then(Value::as_str), Some(mode), "{label}");
        let want = if mode == "replace" { "Selection Change" } else { "Make Selection" };
        assert_eq!(last_step(h.state()).as_deref(), Some(want), "{label}: the combining operations keep today's step name");
    }
}

#[test]
fn a_typed_feather_radius_is_sent() {
    let mut h = dialog(app(false));
    feather_field(&h).click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("4".into()));
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    click(&mut h, "OK");
    assert_eq!(last_params(h.state()).get("feather").and_then(Value::as_f64), Some(4.0));
}

#[test]
fn cancel_changes_nothing() {
    let mut h = dialog(app(true));
    let (selection, before) = (selection_bounds(h.state()), steps(h.state()));
    click(&mut h, "Intersect with Selection");
    click(&mut h, "Cancel");
    assert!(h.state().ui.dialogs.is_empty(), "Cancel closed the dialog");
    assert_eq!(selection_bounds(h.state()), selection);
    assert_eq!(steps(h.state()), before);
}

/// ⌘↩ runs the same command without the dialog: its step keeps today's name (not measured).
#[test]
fn cmd_enter_keeps_the_make_selection_step() {
    let mut app = app(false);
    crate::vector_ui::path_to_selection(&mut app, json!({})).unwrap();
    assert_eq!(selection_bounds(&app), Some([100, 300, 300, 450]));
    assert_eq!(last_step(&app).as_deref(), Some("Make Selection"));
}

/// OK that adds no step (the path went while the dialog was open) renames nothing.
#[test]
fn ok_that_fails_does_not_rename_the_previous_step() {
    let mut h = dialog(app(true));
    h.state_mut().run("path.delete", json!({"name": "work"})).unwrap();
    let before = steps(h.state());
    click(&mut h, "OK");
    assert_eq!(steps(h.state()), before);
}
