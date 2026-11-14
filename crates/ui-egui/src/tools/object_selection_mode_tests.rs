//! Object Selection's New / Add / Subtract / Intersect buttons (#12853), as measured in the
//! reference application: the bar's mode (or ⇧ / ⌥ / ⇧⌥) combines each Object Finder click and
//! rectangle drag with the selection; in New mode a press inside the selection deselects (click)
//! or moves the outline (drag); ⌘ moves the pixels.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use crate::tool_feedback::{Badge, badge};

const RED: [f64; 2] = [200.0, 180.0];
const BLUE: [f64; 2] = [560.0, 300.0];

/// An 800 x 600 document with an opaque red block (80,80–320,280) and a blue disc (centre
/// 560,300, radius 120) on one layer, nothing selected, Object Selection with Object Finder on.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 80, "y": 80, "width": 240, "height": 200})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("select.rect", json!({"x": 440, "y": 180, "width": 240, "height": 240, "ellipse": true})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#0000ff"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::ObjectSelection;
    app.ui.tool_options.object_finder = true;
    app
}

fn click(app: &mut PhotocraftApp, p: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: p[0], y: p[1] }, mods);
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

fn selected(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    app.session.active().and_then(|st| st.doc.selection.as_ref()).is_some_and(|s| s.sample_channel(p[0] as i32, p[1] as i32, 0) >= 0.5)
}

fn any_selection(app: &PhotocraftApp) -> bool {
    app.session.active().and_then(|st| st.doc.selection.as_ref()).is_some_and(|s| !s.content_bounds().is_empty())
}

fn last_step(app: &PhotocraftApp) -> Option<String> {
    app.session.active()?.history.entries().last().map(ToString::to_string)
}

fn alpha_at(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)[3]
}

#[test]
fn add_mode_clicks_accumulate() {
    let mut app = app();
    app.ui.selection_mode = 1;
    click(&mut app, RED, Modifiers::NONE);
    click(&mut app, BLUE, Modifiers::NONE);
    assert!(selected(&app, RED) && selected(&app, BLUE));
}

#[test]
fn subtract_mode_removes_the_clicked_object() {
    let mut app = app();
    app.ui.selection_mode = 1;
    click(&mut app, RED, Modifiers::NONE);
    click(&mut app, BLUE, Modifiers::NONE);
    app.ui.selection_mode = 2;
    click(&mut app, RED, Modifiers::NONE);
    assert!(!selected(&app, RED), "red is removed");
    assert!(selected(&app, BLUE), "blue stays");
}

#[test]
fn intersect_mode_keeps_only_the_overlap() {
    let mut app = app();
    click(&mut app, RED, Modifiers::NONE);
    app.ui.selection_mode = 3;
    click(&mut app, RED, Modifiers::NONE);
    assert!(selected(&app, RED), "red with red is red");
    click(&mut app, BLUE, Modifiers::NONE);
    assert!(!any_selection(&app), "red with blue is empty");
    assert!(!app.ui.status_error && app.ui.dialogs.is_empty(), "an empty intersection is not an error");
}

#[test]
fn new_mode_press_inside_the_selection_deselects_or_moves_the_outline() {
    let mut app = app();
    click(&mut app, RED, Modifiers::NONE);
    assert!(selected(&app, RED));
    click(&mut app, RED, Modifiers::NONE);
    assert!(!any_selection(&app));
    assert_eq!(last_step(&app).as_deref(), Some("Deselect"));

    click(&mut app, RED, Modifiers::NONE);
    drag(&mut app, RED, [250.0, 220.0], Modifiers::NONE);
    assert_eq!(last_step(&app).as_deref(), Some("Move Selection"));
    assert!(selected(&app, [340.0, 300.0]) && !selected(&app, [100.0, 100.0]), "the outline moved by +50,+40");
    assert_eq!(alpha_at(&app, 100, 100), 1.0, "the pixels stay");
    assert_eq!(alpha_at(&app, 340, 300), 0.0);

    // Outside the selection a click replaces it.
    click(&mut app, BLUE, Modifiers::NONE);
    assert!(selected(&app, BLUE) && !selected(&app, [340.0, 300.0]));
}

#[test]
fn the_mode_stays_new_after_a_selection() {
    let mut app = app();
    click(&mut app, RED, Modifiers::NONE);
    assert_eq!(app.ui.selection_mode, 0);
}

#[test]
fn modifiers_override_the_bar() {
    let mut app = app();
    click(&mut app, RED, Modifiers::NONE);
    click(&mut app, BLUE, Modifiers::SHIFT);
    assert!(selected(&app, RED) && selected(&app, BLUE), "Shift adds");
    click(&mut app, RED, Modifiers::ALT);
    assert!(!selected(&app, RED) && selected(&app, BLUE), "Alt subtracts");

    // Shift inside the selection in New mode adds rather than deselecting.
    click(&mut app, BLUE, Modifiers::SHIFT);
    assert!(selected(&app, BLUE));

    // Shift+Alt rectangle drag intersects: both selected, a rectangle around red leaves red.
    click(&mut app, RED, Modifiers::SHIFT);
    assert!(selected(&app, RED) && selected(&app, BLUE));
    let both = Modifiers { shift: true, alt: true, ..Default::default() };
    drag(&mut app, [60.0, 60.0], [340.0, 300.0], both);
    assert!(selected(&app, RED), "red is in both");
    assert!(!selected(&app, BLUE), "blue is outside the rectangle");
}

#[test]
fn the_badge_follows_the_bar_and_modifiers() {
    let mut app = app();
    let both = Modifiers { shift: true, alt: true, ..Default::default() };
    for (mode, want) in [(0, None), (1, Some(Badge::Add)), (2, Some(Badge::Subtract)), (3, Some(Badge::Intersect))] {
        app.ui.selection_mode = mode;
        assert_eq!(badge(&app, Tool::ObjectSelection, Modifiers::NONE), want, "bar mode {mode}");
    }
    app.ui.selection_mode = 0;
    assert_eq!(badge(&app, Tool::ObjectSelection, both), Some(Badge::Intersect));
}

#[test]
fn command_drag_inside_the_selection_moves_the_pixels() {
    let mut app = app();
    click(&mut app, RED, Modifiers::NONE);
    drag(&mut app, RED, [250.0, 220.0], Modifiers::COMMAND);
    // The pixels float at the pointer (as with a marquee's ⌘-drag) and land as "Move" on the drop.
    let offset = photocraft_engine::float_cmds::floating(app.session.active().unwrap()).map(|f| f.offset);
    assert_eq!(offset, Some((50, 40)), "the selected pixels follow the drag");
    crate::menus::invoke(&mut app, &egui::Context::default(), "select.deselect", json!({})).unwrap();
    let labels: Vec<String> = app.session.active().unwrap().history.entries().into_iter().map(|e| e.to_string()).collect();
    assert!(labels.iter().any(|l| l == "Move"), "{labels:?}");
    assert_eq!(alpha_at(&app, 100, 100), 0.0, "the pixels moved away");
    assert_eq!(alpha_at(&app, 340, 300), 1.0);
}

#[test]
fn reset_tool_sets_the_mode_back_to_new() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.ui.selection_mode = 3;
    crate::menus::invoke(&mut app, &ctx, "tool.reset", json!({})).unwrap();
    assert_eq!(app.ui.selection_mode, 0);
}

#[test]
fn object_finder_off_keeps_quick_selection_and_the_rectangle_inside_the_selection() {
    let mut app = app();
    click(&mut app, RED, Modifiers::NONE);
    app.ui.tool_options.object_finder = false;
    // A New-mode click inside the selection is still a Quick Selection dab, not Deselect.
    click(&mut app, RED, Modifiers::NONE);
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("select.quick"));
    assert!(selected(&app, RED));
    // A drag inside it is still a rectangle Object Selection, not Move Selection.
    drag(&mut app, [100.0, 100.0], [300.0, 260.0], Modifiers::NONE);
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("select.object"));
    assert_ne!(last_step(&app).as_deref(), Some("Move Selection"));
}

#[test]
fn object_finder_off_dab_subtracts_with_alt_and_in_subtract_mode() {
    let mut app = app();
    app.ui.tool_options.object_finder = false;
    let shift_alt = Modifiers { shift: true, alt: true, ..Default::default() };
    for (bar, mods) in [(0, Modifiers::ALT), (0, shift_alt), (2, Modifiers::NONE)] {
        app.run("select.all", json!({})).unwrap();
        app.ui.selection_mode = bar;
        click(&mut app, RED, mods);
        assert!(!selected(&app, RED), "bar mode {bar}, {mods:?} subtracts");
        assert!(selected(&app, [20.0, 20.0]), "bar mode {bar}, {mods:?} keeps the rest");
    }
}

#[test]
fn indexed_color_alerts_instead_of_deselecting_or_moving_the_outline() {
    let mut app = app();
    click(&mut app, RED, Modifiers::NONE);
    app.run("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none"})).unwrap();
    assert!(selected(&app, RED));
    let steps = app.session.active().unwrap().history.past_len();
    let errors = |app: &PhotocraftApp| app.ui.dialogs.iter().filter(|d| d.kind == crate::state::DialogKind::Error).count();
    for gesture in 0..2 {
        if gesture == 0 {
            click(&mut app, RED, Modifiers::NONE);
        } else {
            drag(&mut app, RED, [250.0, 220.0], Modifiers::NONE);
        }
        assert_eq!(errors(&app), 1, "gesture {gesture}: the tool's alert");
        assert!(selected(&app, RED) && !selected(&app, [340.0, 300.0]), "gesture {gesture}: the selection stays");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps, "gesture {gesture}: no history step");
        app.ui.dialogs.clear();
    }
}
