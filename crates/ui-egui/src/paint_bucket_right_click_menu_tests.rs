//! #12530: with the Paint Bucket a plain right-click on the image opens its Mode list at the
//! pointer, the options bar's entries with none checked; picking one sets the Mode, adds no history
//! and fills nothing. The pasteboard keeps its colour menu, Shift+right-click keeps its checked
//! menu and other tools keep their right-click.

use egui::{Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::BlendMode;
use serde_json::{Value, json};

use super::eraser_block_right_click_tests::{app, call, harness, right_click, steps};
use crate::PhotocraftApp;
use crate::state::Tool;

fn bucket() -> PhotocraftApp {
    let mut app = app("brush");
    app.ui.tool = Tool::PaintBucket;
    app
}

/// The options bar's Mode list for the Paint Bucket, Clear included.
fn options_bar_modes() -> Vec<&'static str> {
    crate::blend_keys::tool_modes(Tool::PaintBucket).unwrap().into_iter().map(|m| m.map_or("Clear", BlendMode::label)).collect()
}

fn checked_rows(inspect: &Value) -> Vec<Value> {
    let items = inspect["result"]["brushModeMenu"]["items"].as_array().unwrap();
    items.iter().filter(|i| i["checked"] == true).map(|i| i["label"].clone()).collect()
}

#[test]
fn a_plain_right_click_lists_the_options_bar_modes_with_none_checked() {
    let ctx = egui::Context::default();
    let mut app = bucket();
    right_click(&mut app, &ctx, json!({}));
    assert!(app.ui.brush_mode_menu.is_some(), "the Mode list opens");
    assert!(app.ui.brush_picker.is_none() && app.ui.canvas_tool_menu.is_none());
    let inspect = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert_eq!(inspect["result"]["brushModeMenu"]["choices"], json!(options_bar_modes()), "{inspect}");
    assert_eq!(checked_rows(&inspect), Vec::<Value>::new(), "no entry is checked");
    assert_eq!(inspect["result"]["brushModeMenu"]["mode"], "Normal", "the tool's mode is still reported");
}

#[test]
fn picking_an_entry_sets_the_mode_and_adds_no_history() {
    let ctx = egui::Context::default();
    let mut app = bucket();
    let (n, journal) = (steps(&app), app.session.journal.len());
    right_click(&mut app, &ctx, json!({}));
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "Multiply"}))["ok"], true);
    assert_eq!(app.ui.tool_options.bucket_blend_mode, BlendMode::Multiply);
    assert!(!app.ui.tool_options.bucket_clear);
    assert!(app.ui.brush_mode_menu.is_none());
    assert_eq!((steps(&app), app.session.journal.len()), (n, journal), "nothing is filled");
}

#[test]
fn shift_right_click_still_checks_the_current_mode() {
    let ctx = egui::Context::default();
    let mut app = bucket();
    app.ui.tool_options.bucket_blend_mode = BlendMode::Screen;
    right_click(&mut app, &ctx, json!({"shift": true}));
    let inspect = call(&mut app, &ctx, "ui.inspect", json!({}));
    assert_eq!(checked_rows(&inspect), vec![json!("Screen")], "{inspect}");
}

#[test]
fn other_tools_keep_their_plain_right_click() {
    let ctx = egui::Context::default();
    for tool in [Tool::Brush, Tool::Pencil, Tool::CloneStamp, Tool::Gradient, Tool::Eraser] {
        let mut app = bucket();
        app.ui.tool = tool;
        right_click(&mut app, &ctx, json!({}));
        assert!(app.ui.brush_mode_menu.is_none(), "{tool:?}");
    }
}

/// A real right-click at `at` with `mods` held.
fn mouse_right_click(h: &mut Harness<'static, PhotocraftApp>, at: Pos2, mods: Modifiers) {
    h.event(egui::Event::ModifiersChanged(mods));
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: mods });
        h.run_steps(2);
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

fn row_checked(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

/// The real mouse: the menu opens at the pointer with no row checked; Escape and a click outside
/// close it without a change; a row sets the Mode. Nothing is filled.
#[test]
fn a_real_right_click_opens_the_unchecked_menu_at_the_pointer() {
    let mut h = harness(bucket());
    let (n, journal) = (steps(h.state()), h.state().session.journal.len());
    let c = h.state().last_canvas_rect.center();
    mouse_right_click(&mut h, c, Modifiers::NONE);
    assert_eq!(h.state().ui.brush_mode_menu, Some([c.x, c.y]), "the menu at the pointer");
    assert!(!row_checked(&h, "Normal"), "the current mode is not checked");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.brush_mode_menu.is_none(), "Escape closes it");
    mouse_right_click(&mut h, c, Modifiers::NONE);
    let outside = c - egui::vec2(20.0, 0.0);
    h.event(egui::Event::PointerMoved(outside));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: outside, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
    assert!(h.state().ui.brush_mode_menu.is_none(), "a click outside closes it");
    assert_eq!(h.state().ui.tool_options.bucket_blend_mode, BlendMode::Normal);
    mouse_right_click(&mut h, c, Modifiers::NONE);
    h.get_by_label("Multiply").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.tool_options.bucket_blend_mode, BlendMode::Multiply);
    assert!(h.state().ui.brush_mode_menu.is_none());
    assert_eq!((steps(h.state()), h.state().session.journal.len()), (n, journal), "nothing is filled");
}

#[test]
fn a_real_shift_right_click_checks_the_current_mode() {
    let mut h = harness(bucket());
    let c = h.state().last_canvas_rect.center();
    mouse_right_click(&mut h, c, Modifiers::SHIFT);
    assert!(h.state().ui.brush_mode_menu.is_some());
    assert!(row_checked(&h, "Normal"));
}

#[test]
fn a_real_right_click_on_the_pasteboard_opens_its_colour_menu() {
    let mut h = harness(bucket());
    let c = h.state().last_canvas_rect.center();
    mouse_right_click(&mut h, c + egui::vec2(300.0, 0.0), Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.pasteboard), "the pasteboard colour menu");
    assert!(h.state().ui.brush_mode_menu.is_none(), "not the Mode list");
}

/// An open Free Transform box keeps its transform menu: the Mode list does not open over it.
#[test]
fn a_real_right_click_on_a_free_transform_box_opens_only_the_transform_menu() {
    let mut app = bucket();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 4, "y": 6, "width": 10, "height": 8})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let mut h = harness(app);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    h.run_steps(2);
    let c = h.state().last_canvas_rect.center();
    mouse_right_click(&mut h, c, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.transform), "the transform menu");
    assert!(h.state().ui.brush_mode_menu.is_none(), "not the Mode list");
}

/// Window › Arrange: each document's canvas is a cell, and a right-click is placed on the image or
/// the pasteboard by that cell's view, not by the whole work area's.
#[test]
fn arranged_cells_place_the_right_click_by_their_own_view() {
    let mut h = harness(bucket());
    h.state_mut().run("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    h.state_mut().ui.view.arrange = "twoUpVertical".into();
    h.run_steps(4);
    let full = h.state().last_canvas_rect;
    let active = h.state().session.active_index().unwrap();
    let cell = crate::canvas::arranged_cells(h.state(), full).unwrap().into_iter().find(|(i, _)| *i == active).unwrap().1;
    mouse_right_click(&mut h, cell.center(), Modifiers::NONE);
    assert!(h.state().ui.brush_mode_menu.is_some(), "the image in the active cell: the Mode list");
    assert!(h.state().ui.canvas_tool_menu.is_none());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    // The pasteboard at the cell's inner edge, where the whole work area's view has the image.
    let edge = cell.clamp(full.center());
    let pasteboard = edge + (cell.center() - edge).normalized() * 10.0;
    mouse_right_click(&mut h, pasteboard, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.pasteboard), "the pasteboard colour menu");
    assert!(h.state().ui.brush_mode_menu.is_none(), "not the Mode list");
}
