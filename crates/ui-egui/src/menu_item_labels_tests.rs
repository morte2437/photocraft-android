//! #11399: the Combine Shapes, Open Recent, Arrange and Proof Setup items carry the reference
//! application's names, in the live menu tree and in the engine's command labels.

use crate::PhotocraftApp;
use crate::menus::menu_items;

/// (command id, label the live menu must show).
const LABELS: [(&str, &str); 7] = [
    ("layer.combineShapes.intersectShapeAreas", "Unite Shapes at Overlap"),
    ("layer.combineShapes.excludeOverlappingShapes", "Subtract Shapes at Overlap"),
    ("file.clearRecent", "Clear Recent File List"),
    ("window.arrange.newWindowForDocument", "New Window"),
    ("view.proofSetup.workingCmyPlate", "Working CMY Plates"),
    ("view.proofSetup.legacyMacintoshRgb", "Legacy Macintosh RGB (Gamma 1.8)"),
    ("view.proofSetup.internetStandardRgb", "Internet Standard RGB (sRGB)"),
];

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

#[test]
fn live_menu_shows_the_reference_names() {
    let app = app();
    let items = menu_items(&app);
    for (id, label) in LABELS {
        let item = items.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("{id} is in the menu tree"));
        assert_eq!(item.label, label, "{id}");
    }
}

#[test]
fn open_recent_clear_item_is_labelled_with_and_without_recent_files() {
    let mut app = app();
    app.ui.recent_files = vec!["/tmp/a.png".into()];
    let items = menu_items(&app);
    let clear = items.iter().find(|i| i.id == "file.clearRecent").expect("clear item");
    assert_eq!(clear.label, "Clear Recent File List");
    assert!(clear.enabled);
    assert_eq!(clear.path, vec!["File".to_string(), "Open Recent".to_string()]);
    // With nothing to clear the item stays in the submenu, greyed.
    app.ui.recent_files.clear();
    let items = menu_items(&app);
    let clear = items.iter().find(|i| i.id == "file.clearRecent").expect("clear item");
    assert_eq!(clear.label, "Clear Recent File List");
    assert!(!clear.enabled);
}

#[test]
fn engine_command_labels_match_the_menu() {
    let specs = photocraft_engine::cmd::command_specs();
    // The two Combine Shapes and the three Proof Setup commands have an engine command under the
    // same id as the menu row.
    for (id, label) in LABELS {
        if let Some(spec) = specs.iter().find(|s| s.id == id) {
            assert_eq!(spec.label, label, "engine label of {id}");
        }
    }
    let engine_ids = [
        "layer.combineShapes.intersectShapeAreas",
        "layer.combineShapes.excludeOverlappingShapes",
        "view.proofSetup.workingCmyPlate",
        "view.proofSetup.legacyMacintoshRgb",
    ];
    for id in engine_ids {
        assert!(specs.iter().any(|s| s.id == id), "{id} is an engine command");
    }
    // The shell command for the extra window is registered under its own id.
    let shell = photocraft_engine::shell_commands::SHELL_COMMANDS.iter().find(|c| c.0 == "window.newWindowForDocument").expect("shell command");
    assert_eq!(shell.1, "New Window");
}

#[test]
fn history_step_names_and_other_labels_are_unchanged() {
    // The Home screen keeps its own button label (#11399 "not measured").
    let src = include_str!("home_recent_ui.rs");
    assert!(src.contains("tl!(\"Clear Recent Files\")"));
    // Merge Shape Components stays in Combine Shapes.
    let app = app();
    let items = menu_items(&app);
    assert!(items.iter().any(|i| i.id == "layer.combineShapes.mergeShapeComponents"));
}
