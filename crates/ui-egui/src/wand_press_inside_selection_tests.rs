//! The Magic Wand pressed inside its selection (#11782), as measured in the reference
//! application: in New mode a click deselects and a drag moves the outline (the pixels stay),
//! the pointer shows the move-selection cursor, and a click on the pasteboard deselects. Add mode
//! and ⌘ drags keep today's behaviour.

use egui::Modifiers;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{SelCursor, ToolEvent, selection_cursor, tool_event};
use crate::state::Tool;

/// A 500 x 400 document with an opaque red block at 100,100–300,250 on its own layer, selected
/// with the wand.
fn selected() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 500, "height": 400})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 150})).unwrap();
    app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::MagicWand;
    app.ui.tool_options.anti_alias = false;
    click(&mut app, [150.0, 150.0], Modifiers::NONE);
    assert_eq!(selection(&app), Some(Rect::new(100, 100, 300, 250)), "the wand selected the block");
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

fn selection(app: &PhotocraftApp) -> Option<Rect> {
    let st = app.session.active()?;
    st.doc.selection.as_ref().map(|s| s.content_bounds()).filter(|b| !b.is_empty())
}

fn last_step(app: &PhotocraftApp) -> Option<String> {
    app.session.active()?.history.entries().last().map(ToString::to_string)
}

fn alpha_at(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)[3]
}

#[test]
fn a_new_mode_click_inside_the_selection_deselects() {
    let mut app = selected();
    click(&mut app, [200.0, 200.0], Modifiers::NONE);
    assert_eq!(selection(&app), None, "nothing is selected");
    assert_eq!(last_step(&app).as_deref(), Some("Deselect"));
}

#[test]
fn a_new_mode_drag_inside_the_selection_moves_the_outline_not_the_pixels() {
    let mut app = selected();
    drag(&mut app, [200.0, 200.0], [300.0, 250.0], Modifiers::NONE);
    assert_eq!(selection(&app), Some(Rect::new(200, 150, 400, 300)), "the outline moved by +100,+50");
    assert_eq!(last_step(&app).as_deref(), Some("Move Selection"));
    assert_eq!(alpha_at(&app, 150, 150), 1.0, "the pixels stay where they are");
    assert_eq!(alpha_at(&app, 350, 280), 0.0);
}

#[test]
fn the_pointer_over_the_selection_shows_the_move_selection_cursor_in_new_mode_only() {
    let mut app = selected();
    assert_eq!(selection_cursor(&app, Tool::MagicWand, [200.0, 200.0], Modifiers::NONE), Some(SelCursor::Outline));
    assert_eq!(selection_cursor(&app, Tool::MagicWand, [400.0, 350.0], Modifiers::NONE), None, "outside: the wand");
    assert_eq!(selection_cursor(&app, Tool::MagicWand, [200.0, 200.0], Modifiers::SHIFT), None, "Shift adds");
    app.ui.selection_mode = 1;
    assert_eq!(selection_cursor(&app, Tool::MagicWand, [200.0, 200.0], Modifiers::NONE), None, "Add mode");
}

#[test]
fn a_click_on_the_pasteboard_deselects() {
    let mut app = selected();
    click(&mut app, [-30.0, -30.0], Modifiers::NONE);
    assert_eq!(selection(&app), None);
    assert_eq!(last_step(&app).as_deref(), Some("Deselect"));
}

#[test]
fn add_mode_and_shift_keep_the_selection_and_move_nothing() {
    for (bar, mods) in [(1, Modifiers::NONE), (0, Modifiers::SHIFT)] {
        let mut app = selected();
        app.ui.selection_mode = bar;
        click(&mut app, [200.0, 200.0], mods);
        assert_eq!(selection(&app), Some(Rect::new(100, 100, 300, 250)), "mode {bar}, {mods:?}: click");
        drag(&mut app, [200.0, 200.0], [300.0, 250.0], mods);
        assert_eq!(selection(&app), Some(Rect::new(100, 100, 300, 250)), "mode {bar}, {mods:?}: drag");
        assert_eq!(alpha_at(&app, 150, 150), 1.0);
    }
}

#[test]
fn a_new_mode_click_outside_the_selection_selects_that_region() {
    let mut app = selected();
    click(&mut app, [450.0, 350.0], Modifiers::NONE);
    let b = selection(&app).expect("the transparent area is selected");
    assert!(!b.contains(150, 150) || b.width() == 500, "a new selection of the clicked region: {b:?}");
    assert_eq!(last_step(&app).as_deref(), Some("Magic Wand"));
}
