//! #12768: a click with the Delete Anchor Point tool on an anchor of a live shape asks "This
//! operation will turn a live shape into a regular path. Continue?" first: Yes deletes it, No
//! changes nothing, and "Don't show again" skips the question later.

use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use super::delete_anchor_targeted_path_tests::{click, select_layer, select_saved_path, setup_asking, shape_path, steps};
use crate::PhotocraftApp;
use crate::state::Tool;

const QUESTION: &str = "This operation will turn a live shape into a regular path. Continue?";
/// The live rectangle's bottom-right corner.
const CORNER: [f64; 2] = [650.0, 250.0];

/// The tool with the live rectangle active.
fn live(tool: Tool) -> (PhotocraftApp, u64) {
    let (mut app, shape) = setup_asking(tool);
    select_layer(&mut app, shape);
    (app, shape)
}

fn info(app: &mut PhotocraftApp, shape: u64) -> Value {
    app.run("shape.info", json!({"layer": shape})).unwrap()
}

fn question(app: &PhotocraftApp) -> Option<u64> {
    app.ui.dialogs.iter().find(|d| d.fields.get("__command").and_then(Value::as_str) == Some("path.deleteAnchor")).map(|d| d.id)
}

fn corners(app: &mut PhotocraftApp, shape: u64) -> usize {
    shape_path(app, shape)["subpaths"][0]["knots"].as_array().unwrap().len()
}

#[test]
fn a_click_on_a_live_shape_anchor_asks_with_yes_no_and_an_unticked_box() {
    let (mut app, shape) = live(Tool::DeleteAnchorPoint);
    let (before, n) = (info(&mut app, shape), steps(&app));
    click(&mut app, CORNER);
    let id = question(&app).expect("the question");
    assert_eq!(info(&mut app, shape), before, "nothing goes before the answer");
    assert_eq!(steps(&app), n);
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["skip"], json!(false));
    // Like the Preferences kittest: the dialog draws in the semibold family, which a build without the fonts lacks.
    let has_fonts = |ctx: &egui::Context| ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into())));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(
        move |ui, app| {
            if has_fonts(ui.ctx()) {
                crate::dialogs::show(app, ui.ctx());
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(5);
    if !has_fonts(&h.ctx) {
        return;
    }
    h.get_by_label(QUESTION);
    h.get_by_label("Yes");
    h.get_by_label("No");
    h.get_by_label("Don't show again");
    // Enter is Yes.
    h.key_press(egui::Key::Enter);
    h.run_steps(5);
    let app = h.state_mut();
    assert!(question(app).is_none());
    assert_eq!(info(app, shape)["live"], Value::Null);
}

#[test]
fn yes_deletes_the_anchor_and_the_layer_is_no_longer_live() {
    let (mut app, shape) = live(Tool::DeleteAnchorPoint);
    let n = steps(&app);
    click(&mut app, CORNER);
    let id = question(&app).expect("the question");
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(corners(&mut app, shape), 3);
    let info = info(&mut app, shape);
    assert_eq!((&info["live"], &info["kind"]), (&Value::Null, &json!("path")));
    assert_eq!(steps(&app), n + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
}

/// The question belongs to the document it was asked in: Yes after another document became active
/// is refused and changes nothing there (layer ids are unique in a session, so this guards the
/// binding the way the Color Picker's `__document` does), and the question's own document keeps
/// its shape.
#[test]
fn yes_after_another_document_became_active_deletes_nothing() {
    let (mut app, shape) = live(Tool::DeleteAnchorPoint);
    click(&mut app, CORNER);
    let id = question(&app).expect("the question");
    let first = app.session.active_index();
    app.session.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    let other = app.run("shape.create", json!({"kind": "rect", "rect": [450, 100, 200, 150], "fill": "#3070c0"})).unwrap()["layer"].as_u64().unwrap();
    assert_ne!(app.session.active_index(), first, "the new document is active");
    let (before, n) = (shape_path(&mut app, other), steps(&app));
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!((shape_path(&mut app, other), steps(&app)), (before, n));
    assert!(app.session.set_active(first.unwrap()));
    app.sync_views();
    assert_eq!(corners(&mut app, shape), 4, "the first document's shape keeps its corner");
}

/// The question's knot index belongs to the path as it was asked: an anchor added before the
/// corner meanwhile (through the control channel; the question is modal on screen) shifts it, so Yes
/// is refused instead of deleting another anchor.
#[test]
fn yes_after_the_path_changed_deletes_nothing() {
    let (mut app, shape) = live(Tool::DeleteAnchorPoint);
    click(&mut app, CORNER);
    let id = question(&app).expect("the question");
    app.run("path.addAnchor", json!({"name": "layer", "layer": shape, "subpath": 0, "knot": 0, "t": 0.5})).unwrap();
    let (before, n) = (shape_path(&mut app, shape), steps(&app));
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!((shape_path(&mut app, shape), steps(&app)), (before, n));
}

#[test]
fn no_leaves_the_shape_its_parameters_and_history_unchanged() {
    let (mut app, shape) = live(Tool::DeleteAnchorPoint);
    let (before, n) = (info(&mut app, shape), steps(&app));
    click(&mut app, CORNER);
    let id = question(&app).expect("the question");
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert_eq!(info(&mut app, shape), before);
    assert_ne!(before["live"], Value::Null);
    assert_eq!(steps(&app), n);
    // Not recorded: the next click asks again.
    click(&mut app, CORNER);
    assert!(question(&app).is_some());
}

#[test]
fn yes_with_dont_show_again_deletes_later_clicks_at_once() {
    let (mut app, shape) = live(Tool::DeleteAnchorPoint);
    click(&mut app, CORNER);
    let id = question(&app).expect("the question");
    app.ui.dialog_mut(id).unwrap().fields.insert("skip".into(), json!(true));
    crate::dialogs::confirm(&mut app, id).unwrap();
    // A second live shape: its anchor goes at once.
    let other = app.run("shape.create", json!({"kind": "rect", "rect": [100, 100, 100, 100], "fill": "#c03030"})).unwrap()["layer"].as_u64().unwrap();
    assert_ne!(info(&mut app, other)["live"], Value::Null);
    let n = steps(&app);
    click(&mut app, [200.0, 200.0]);
    assert!(question(&app).is_none());
    assert_eq!(corners(&mut app, other), 3);
    assert_eq!(info(&mut app, other)["live"], Value::Null);
    assert_eq!(steps(&app), n + 1);
    assert_eq!(corners(&mut app, shape), 3);
}

#[test]
fn a_shape_that_is_not_live_or_a_saved_path_deletes_without_the_question() {
    let (mut app, shape) = live(Tool::DeleteAnchorPoint);
    let path = json!({"subpaths": [{"closed": true, "knots": [[450, 100], [650, 100], [650, 250], [450, 250]]}]});
    app.run("shape.edit", json!({"layer": shape, "path": path})).unwrap();
    assert_eq!(info(&mut app, shape)["live"], Value::Null);
    let n = steps(&app);
    click(&mut app, CORNER);
    assert!(question(&app).is_none());
    assert_eq!(corners(&mut app, shape), 3);
    assert_eq!(steps(&app), n + 1);
    // A saved path.
    let (mut app, _) = live(Tool::DeleteAnchorPoint);
    select_saved_path(&mut app);
    let n = steps(&app);
    click(&mut app, [150.0, 550.0]);
    assert!(question(&app).is_none());
    assert_eq!(steps(&app), n + 1);
}

#[test]
fn add_anchor_point_on_a_live_shape_converts_it_without_the_question() {
    let (mut app, shape) = live(Tool::AddAnchorPoint);
    let n = steps(&app);
    click(&mut app, [550.0, 100.0]);
    assert!(question(&app).is_none());
    assert_eq!(corners(&mut app, shape), 5);
    assert_eq!(info(&mut app, shape)["live"], Value::Null);
    assert_eq!(steps(&app), n + 1);
}
