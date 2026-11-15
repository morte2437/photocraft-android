//! #11400: View › Guides holds the nine guide commands in the reference's order, and its two
//! Selected Guides rows act on the selected guide.

use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas_tool_menu::{GUIDE_CLEAR, GUIDE_EDIT, GUIDE_MENU};

const ORDER: [(&str, &str); 9] = [
    ("Edit Selected Guides", "view.editSelectedGuides"),
    ("Lock Guides", "view.lockGuides"),
    ("Clear Guides", "view.clearGuides"),
    ("Clear Selected Guides", "view.clearSelectedGuides"),
    ("Clear Selected Artboard Guides", "view.clearSelectedArtboardGuides"),
    ("Clear Canvas Guides", "view.clearCanvasGuides"),
    ("New Guide…", "view.newGuide"),
    ("New Guide Layout…", "view.newGuideLayout"),
    ("New Guides From Shape", "view.newGuidesFromShape"),
];

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 10})).unwrap();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 20})).unwrap();
    app
}

fn item(app: &PhotocraftApp, id: &str) -> crate::menus::MenuItem {
    crate::menus::menu_items(app).into_iter().find(|i| i.id == id).unwrap_or_else(|| panic!("{id} is not in the menu"))
}

fn vertical(app: &PhotocraftApp) -> Vec<f64> {
    app.session.active().unwrap().doc.guides.vertical.clone()
}

#[test]
fn guides_submenu_lists_the_nine_rows_in_order() {
    let app = app();
    let path = vec!["View".to_string(), "Guides".to_string()];
    let rows: Vec<(String, String)> = crate::menus::menu_items(&app).into_iter().filter(|i| i.path == path).map(|i| (i.label, i.id)).collect();
    let want: Vec<(String, String)> = ORDER.iter().map(|(l, i)| (l.to_string(), i.to_string())).collect();
    assert_eq!(rows, want);
    assert_eq!(item(&app, "view.lockGuides").shortcut.as_deref(), Some("Cmd+Alt+;"));
}

#[test]
fn no_guide_row_is_left_flat_in_view() {
    let app = app();
    let items = crate::menus::menu_items(&app);
    let guides = vec!["View".to_string(), "Guides".to_string()];
    for (_, id) in ORDER {
        let rows: Vec<_> = items.iter().filter(|i| i.id == id).collect();
        assert_eq!(rows.len(), 1, "{id} appears exactly once");
        assert_eq!(rows[0].path, guides, "{id} is in View > Guides, not flat in View");
    }
}

#[test]
fn the_menu_cache_follows_the_selected_guide() {
    let mut app = app();
    let none = crate::native_menu::state_hash(&app, None);
    crate::guide_select::select(&mut app, true, 0);
    let one = crate::native_menu::state_hash(&app, None);
    assert_ne!(none, one, "selecting a guide changes the menu state");
    assert!(item(&app, "view.clearSelectedGuides").enabled);
    crate::guide_select::clear(&mut app);
    assert_eq!(crate::native_menu::state_hash(&app, None), none);
    assert!(!item(&app, "view.clearSelectedGuides").enabled);
}

#[test]
fn selected_rows_are_greyed_without_a_selected_guide_and_enabled_with_one() {
    let mut app = app();
    for id in ["view.editSelectedGuides", "view.clearSelectedGuides"] {
        assert!(!item(&app, id).enabled, "{id} with no guide selected");
        assert!(crate::menus::invoke(&mut app, &egui::Context::default(), id, json!({})).is_err(), "{id} errors, not a crash");
    }
    crate::guide_select::select(&mut app, true, 0);
    for id in ["view.editSelectedGuides", "view.clearSelectedGuides"] {
        assert!(item(&app, id).enabled, "{id} with a guide selected");
    }
}

#[test]
fn clear_selected_guides_removes_only_the_selected_guide() {
    let mut app = app();
    crate::guide_select::select(&mut app, true, 1);
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "view.clearSelectedGuides", json!({})).unwrap();
    assert_eq!(vertical(&app), vec![10.0]);
}

#[test]
fn edit_selected_guides_opens_the_right_click_menus_dialog() {
    let mut app = app();
    crate::guide_select::select(&mut app, true, 0);
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "view.editSelectedGuides", json!({})).unwrap();
    assert_eq!(app.ui.dialogs.len(), 1);
    let from_menu = app.ui.dialogs[0].fields.clone();
    let id = app.ui.dialogs[0].id;
    crate::dialogs::cancel(&mut app, id).unwrap();
    let id = crate::new_guide_ui::open_edit(&mut app).unwrap();
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields, from_menu);
}

#[test]
fn right_click_guide_menu_keeps_both_selected_rows() {
    let ids: Vec<&str> = GUIDE_MENU.iter().flatten().map(|(_, id)| *id).collect();
    assert!(ids.contains(&GUIDE_EDIT) && ids.contains(&GUIDE_CLEAR));
    assert_eq!((GUIDE_EDIT, GUIDE_CLEAR), ("view.editSelectedGuides", "view.clearSelectedGuides"));
}
