//! #12022: the Hand tool's canvas right-click lists only Fit on Screen, 100%, 200% and Print Size;
//! the Zoom tool's menu is unchanged.

use egui::Context;
use serde_json::json;

use super::{FILL_SCREEN, HAND_MENU, Row, VIEW_MENU, choose, open_view, rows};
use crate::PhotocraftApp;
use crate::state::Tool;

fn labels(rows: &[Row]) -> Vec<Option<&'static str>> {
    rows.iter().map(|r| r.map(|(label, _)| label)).collect()
}

/// A 400 × 300 document, optionally with everything selected, and the view menu of `tool` open.
fn menu_app(tool: Tool, selection: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    if selection {
        app.run("select.all", json!({})).unwrap();
    }
    app.sync_views();
    assert!(open_view(&mut app, &Context::default(), tool, [100.0, 100.0], [300.0, 100.0]));
    app
}

#[test]
fn the_hand_menu_lists_only_the_fixed_zooms() {
    for selection in [false, true] {
        let app = menu_app(Tool::Hand, selection);
        let menu = app.ui.canvas_tool_menu.clone().unwrap();
        assert_eq!(labels(rows(&menu)), vec![Some("Fit on Screen"), Some("100%"), Some("200%"), Some("Print Size")], "selection {selection}");
    }
}

#[test]
fn the_zoom_menu_is_unchanged() {
    let app = menu_app(Tool::Zoom, false);
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(rows(&menu) == VIEW_MENU);
    assert_eq!(
        labels(rows(&menu)),
        vec![Some("Fit on Screen"), Some("Fill Screen"), Some("100%"), Some("200%"), Some("Print Size"), None, Some("Zoom In"), Some("Zoom Out")]
    );
}

#[test]
fn the_hand_entries_zoom_and_add_no_history() {
    let mut app = menu_app(Tool::Hand, false);
    let before = app.session.active().unwrap().history.past_len();
    choose(&mut app, &Context::default(), "view.twoHundredPercent");
    assert_eq!(app.ui.views[0].zoom, 2.0);
    assert_eq!(app.session.active().unwrap().history.past_len(), before);
    // Rows that left the Hand's menu can't be run through it.
    for gone in [FILL_SCREEN, "view.zoomIn", "view.zoomOut"] {
        let mut app = menu_app(Tool::Hand, false);
        let zoom = app.ui.views[0].zoom;
        choose(&mut app, &Context::default(), gone);
        assert_eq!(app.ui.views[0].zoom, zoom, "{gone}");
        assert!(!app.ui.views[0].fill_pending, "{gone}");
    }
    assert!(HAND_MENU.iter().all(Option::is_some), "no separator");
}
