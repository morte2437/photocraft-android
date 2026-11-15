//! #13777: File › Export › Paths to Illustrator opens its Paths dropdown on the row that is
//! selected in the Paths panel, as the reference application does: a saved path or the Work
//! Path when that row is selected, All Paths when no row is (or the row is gone), and the
//! choice is not remembered between openings. The shape layer's path is not an entry yet
//! (#13774), so that row keeps today's All Paths.

use crate::PhotocraftApp;
use serde_json::{Value, json};

const COMMAND: &str = "file.export.pathsToIllustrator";

/// A document with a Work Path and two saved paths, "Curve A" and "Curve B".
fn app_with_paths() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    let path = |x: f64| json!({"subpaths": [{"closed": true, "knots": [[x, 10.0], [x + 40.0, 10.0], [x + 40.0, 50.0]]}]});
    for (name, x) in [("Curve A", 10.0), ("Curve B", 60.0), ("work", 110.0)] {
        app.run("path.set", json!({"name": name, "path": path(x)})).unwrap();
    }
    app
}

/// Opens the dialog as the menu item does and returns its id and the preselected entry.
fn open(app: &mut PhotocraftApp) -> (u64, Value) {
    let r = super::file_ui::invoke(app, &egui::Context::default(), COMMAND, &json!({})).unwrap().unwrap();
    let id = r["dialog"].as_u64().unwrap();
    let paths = app.ui.dialog_mut(id).unwrap().fields["paths"].clone();
    (id, paths)
}

#[test]
fn a_selected_saved_path_is_preselected() {
    let mut app = app_with_paths();
    app.ui.selected_path = Some("Curve B".into());
    assert_eq!(open(&mut app).1, "Curve B");
}

#[test]
fn the_selected_work_path_is_preselected() {
    let mut app = app_with_paths();
    app.ui.selected_path = Some("work".into());
    assert_eq!(open(&mut app).1, "work");
}

#[test]
fn no_selected_row_preselects_all_paths() {
    let mut app = app_with_paths();
    app.ui.selected_path = None;
    assert_eq!(open(&mut app).1, "all");
}

#[test]
fn a_row_the_document_no_longer_has_preselects_all_paths() {
    let mut app = app_with_paths();
    app.ui.selected_path = Some("Deleted path".into());
    assert_eq!(open(&mut app).1, "all");
    app.run("path.delete", json!({"name": "work"})).unwrap();
    app.ui.selected_path = Some("work".into());
    assert_eq!(open(&mut app).1, "all", "a selected Work Path the document no longer has");
}

/// The layer path entry does not exist yet (#13774): that row keeps today's default.
#[test]
fn the_shape_layer_path_row_keeps_all_paths() {
    let mut app = app_with_paths();
    app.ui.selected_path = Some("layer".into());
    assert_eq!(open(&mut app).1, "all");
}

#[test]
fn the_choice_is_not_remembered_between_openings() {
    let mut app = app_with_paths();
    app.ui.selected_path = Some("Curve A".into());
    let (id, paths) = open(&mut app);
    assert_eq!(paths, "Curve A");
    app.ui.dialog_mut(id).unwrap().fields.insert("paths".into(), json!("work"));
    app.ui.close_dialog(id);
    app.ui.selected_path = Some("Curve B".into());
    assert_eq!(open(&mut app).1, "Curve B");
}

/// The dropdown's entries and the command's parameter values stay as they are.
#[test]
fn the_entries_keep_their_parameter_values() {
    let mut app = app_with_paths();
    let (id, _) = open(&mut app);
    let choices = app.ui.dialog_mut(id).unwrap().fields["__choices"]["paths"].clone();
    // Document Bounds comes first (#13771).
    assert_eq!(choices, json!(["documentBounds", "all", "work", "Curve A", "Curve B"]));
}
