//! #12192: with no selection, a Content-Aware Move drag that encloses no area (a straight line or a
//! click) warns "No pixels are selected." with "Don't show again", adds no History state and
//! selects nothing. A drag that encloses an area still makes the selection; the Lasso tool and a
//! drag with a selection keep their behaviour.

use std::cell::RefCell;
use std::rc::Rc;

use egui::Modifiers;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{DialogKind, Tool};

/// A 200 x 100 white document with nothing selected and the Content-Aware Move tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.ui.tool = Tool::ContentAwareMove;
    app
}

/// A drag through `points` (the first is the press), released at the last.
fn drag(app: &mut PhotocraftApp, points: &[[f64; 2]]) {
    let m = Modifiers::NONE;
    let [x, y] = points[0];
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
    for &[x, y] in &points[1..] {
        tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
    }
    let [x, y] = points[points.len() - 1];
    tool_event(app, ToolEvent::Up { x, y }, m);
}

const LINE: &[[f64; 2]] = &[[20.0, 50.0], [60.0, 50.0], [100.0, 50.0], [140.0, 50.0]];
const DIAGONAL: &[[f64; 2]] = &[[20.0, 20.0], [50.0, 35.0], [80.0, 50.0], [110.0, 65.0]];
const AREA: &[[f64; 2]] = &[[20.0, 20.0], [80.0, 20.0], [80.0, 80.0], [20.0, 80.0]];

/// The open "No pixels are selected." alerts.
fn alerts(app: &PhotocraftApp) -> Vec<u64> {
    app.ui
        .dialogs
        .iter()
        .filter(|d| d.kind == DialogKind::Error && d.fields.get("message").and_then(Value::as_str) == Some("No pixels are selected."))
        .map(|d| d.id)
        .collect()
}

/// Whether the document has a selection.
fn selected(app: &PhotocraftApp) -> bool {
    app.session.active().unwrap().doc.selection.is_some()
}

/// The number of History states.
fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn a_straight_line_or_a_click_warns_and_changes_nothing() {
    for (name, points) in [("line", LINE), ("diagonal", DIAGONAL), ("click", &[[40.0, 40.0]][..]), ("two points", &[[40.0, 40.0], [90.0, 40.0]][..])] {
        let mut app = app();
        let before = steps(&app);
        drag(&mut app, points);
        assert_eq!(alerts(&app).len(), 1, "{name}: the alert");
        assert!(!selected(&app), "{name}: nothing selected");
        assert_eq!(steps(&app), before, "{name}: no History state");
    }
}

#[test]
fn a_second_drag_while_the_alert_is_open_adds_no_second_one() {
    let mut app = app();
    drag(&mut app, LINE);
    drag(&mut app, LINE);
    assert_eq!(alerts(&app).len(), 1);
}

#[test]
fn ok_closes_it_and_the_next_drag_warns_again() {
    let mut app = app();
    drag(&mut app, LINE);
    let id = alerts(&app)[0];
    // The alert's OK button answers as Cancel does (its only button); a script may confirm it.
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert!(alerts(&app).is_empty());
    drag(&mut app, LINE);
    assert_eq!(alerts(&app).len(), 1, "the box was not ticked, so it warns again");
    let id = alerts(&app)[0];
    crate::dialogs::confirm(&mut app, id).unwrap();
    drag(&mut app, LINE);
    assert_eq!(alerts(&app).len(), 1);
}

#[test]
fn dont_show_again_stops_the_warning_and_is_kept_with_the_preferences() {
    for confirm in [false, true] {
        let mut app = app();
        let saved = Rc::new(RefCell::new(String::new()));
        let store = saved.clone();
        app.services.save_prefs = Some(Box::new(move |text| {
            *store.borrow_mut() = text.to_string();
            Ok(())
        }));
        drag(&mut app, LINE);
        let id = alerts(&app)[0];
        app.ui.dialog_mut(id).unwrap().fields.insert("skip".into(), json!(true));
        if confirm {
            crate::dialogs::confirm(&mut app, id).unwrap();
        } else {
            crate::dialogs::cancel(&mut app, id).unwrap();
        }
        let text = saved.borrow().clone();
        let stored: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("confirm {confirm}: the preferences were saved ({e})"));
        assert_eq!(stored["dialogs"]["contentAwareMove.noPixelsSelected"], json!({"skip": true}), "confirm {confirm}: written to the store");
        let before = steps(&app);
        drag(&mut app, LINE);
        assert!(alerts(&app).is_empty(), "confirm {confirm}: no alert any more");
        assert!(!selected(&app) && steps(&app) == before, "confirm {confirm}: still nothing selected and no History state");
        // A restart reads it back from the store.
        let services = crate::Services { load_prefs: Some(Box::new(move || Some(text.clone()))), ..Default::default() };
        let mut fresh = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        fresh.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
        fresh.ui.tool = Tool::ContentAwareMove;
        drag(&mut fresh, LINE);
        assert!(alerts(&fresh).is_empty(), "confirm {confirm}: no alert after a restart");
    }
}

#[test]
fn a_tiny_lasso_that_covers_part_of_a_pixel_still_selects() {
    // Under half a pixel of area, but anti-aliased it covers part of pixel 10,10.
    let mut app = app();
    app.ui.tool_options.anti_alias = true;
    let before = steps(&app);
    drag(&mut app, &[[10.0, 10.0], [11.0, 10.0], [10.0, 10.75]]);
    assert!(alerts(&app).is_empty());
    assert!(selected(&app));
    assert_eq!(steps(&app), before + 1);
}

#[test]
fn without_anti_alias_a_lasso_over_pixel_centres_selects() {
    let mut app = app();
    app.ui.tool_options.anti_alias = false;
    let before = steps(&app);
    drag(&mut app, &[[10.0, 20.0], [13.0, 20.0], [13.0, 23.0], [10.0, 23.0]]);
    assert!(alerts(&app).is_empty());
    assert!(selected(&app));
    assert_eq!(steps(&app), before + 1);
}

#[test]
fn a_thin_lasso_between_pixel_centres_warns_without_anti_alias() {
    // 21 px² of area, but between the centres of row 20 (20.5) and the row above: no pixel.
    let mut app = app();
    app.ui.tool_options.anti_alias = false;
    let before = steps(&app);
    drag(&mut app, &[[10.0, 20.1], [150.0, 20.1], [150.0, 20.4]]);
    assert_eq!(alerts(&app).len(), 1);
    assert!(!selected(&app));
    assert_eq!(steps(&app), before);
}

#[test]
fn a_drag_that_encloses_an_area_still_selects_it() {
    let mut app = app();
    let before = steps(&app);
    drag(&mut app, AREA);
    assert!(alerts(&app).is_empty());
    assert!(selected(&app));
    assert_eq!(steps(&app), before + 1);
}

#[test]
fn the_lasso_tool_does_not_warn() {
    let mut app = app();
    app.ui.tool = Tool::Lasso;
    drag(&mut app, LINE);
    drag(&mut app, &[[40.0, 40.0]]);
    assert!(alerts(&app).is_empty());
}

#[test]
fn with_a_selection_a_line_outside_it_keeps_todays_behaviour() {
    let mut app = app();
    app.run("select.rect", json!({"x": 150, "y": 10, "width": 30, "height": 30})).unwrap();
    drag(&mut app, LINE);
    assert!(alerts(&app).is_empty(), "not measured with a selection: no alert");
}

#[test]
fn other_alerts_have_no_dont_show_again() {
    let mut app = app();
    let mut fields = serde_json::Map::new();
    fields.insert("message".into(), json!("Something else."));
    let id = app.ui.open_dialog(DialogKind::Error, fields);
    app.ui.dialog_mut(id).unwrap().fields.insert("skip".into(), json!(true));
    let before = app.session.prefs().dialogs.clone();
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.prefs().dialogs, before, "an alert without the box keeps no preference");
}
