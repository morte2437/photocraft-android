//! #13771: Export Paths to File's dropdown starts with Document Bounds, and in a document
//! without paths it holds only Document Bounds, selected.

use crate::PhotocraftApp;
use serde_json::{Value, json};

const COMMAND: &str = "file.export.pathsToIllustrator";

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app
}

/// Opens the dialog as the menu item does and returns the dropdown's entries and selection.
fn open(app: &mut PhotocraftApp) -> (Vec<String>, Value) {
    let r = super::file_ui::invoke(app, &egui::Context::default(), COMMAND, &json!({})).unwrap().unwrap();
    let fields = &app.ui.dialog_mut(r["dialog"].as_u64().unwrap()).unwrap().fields;
    let names = fields["__choices"]["paths"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect();
    (names, fields["paths"].clone())
}

#[test]
fn a_document_without_paths_offers_only_document_bounds() {
    let mut app = app();
    assert!(app.session.is_enabled(COMMAND));
    let (names, paths) = open(&mut app);
    assert_eq!(names, ["documentBounds"]);
    assert_eq!(paths, "documentBounds");
    // A Paths panel selection the document doesn't have changes nothing.
    app.ui.selected_path = Some("work".into());
    assert_eq!(open(&mut app), (vec!["documentBounds".to_owned()], json!("documentBounds")));
}

#[test]
fn document_bounds_comes_before_the_paths() {
    let mut app = app();
    let path = json!({"subpaths": [{"closed": true, "knots": [[10.0, 10.0], [50.0, 10.0], [50.0, 50.0]]}]});
    app.run("path.set", json!({"name": "Rect A", "path": path})).unwrap();
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app.ui.selected_path = None;
    let (names, paths) = open(&mut app);
    assert_eq!(names, ["documentBounds", "all", "work", "Rect A"]);
    assert_eq!(paths, "all", "the default stays All Paths (#13777)");
}

#[test]
fn ok_with_document_bounds_exports_the_frame() {
    let mut app = app();
    let (_, paths) = open(&mut app);
    let r = app.run(COMMAND, json!({"paths": paths})).unwrap();
    assert_eq!(r["paths"], 0);
    assert!(r["ai"].as_str().unwrap().contains("%%BoundingBox: 0 0 300 200\n"));
}
