//! The Artboard tool's Add New Artboard placing mode (#11386): with the mode on, a click on empty
//! canvas puts a board of the bar's size with its top-left corner at the click.

use egui::Modifiers;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The measured document: two boards, the 750 x 1334 one selected, the other one apart from it.
fn app_with_selected_board() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [2000, 0, 300, 300]})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [900, 0, 750, 1334]})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app.ui.tool = Tool::Artboard;
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x, y }, m);
}

fn boards(app: &PhotocraftApp) -> Vec<Rect> {
    app.session.active().unwrap().doc.artboards().iter().map(|b| b.2.rect).collect()
}

fn last_step(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.entries().last().cloned()
}

#[test]
fn a_click_with_the_mode_on_puts_a_bar_sized_board_at_the_click() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    let before = boards(&app);
    click(&mut app, 222.0, 600.0);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let after = boards(&app);
    assert_eq!(after.len(), before.len() + 1);
    // 750 x 1334, top-left at the click, over the selected board (it did not move aside).
    assert_eq!(after.last().copied(), Some(Rect::new(222, 600, 972, 1934)));
    assert_eq!(after.get(..before.len()), Some(before.as_slice()));
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers.last().map(|l| l.name.as_str()), Some("Artboard 3"));
    assert_eq!(st.active_layer, Some(st.doc.artboards().last().unwrap().0));
    assert_eq!(last_step(&app).as_deref(), Some("New Artboard"));
    // One undo step.
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(boards(&app), before);
}

#[test]
fn the_click_is_rounded_to_whole_pixels() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    click(&mut app, 222.4, 600.6);
    assert_eq!(boards(&app).last().copied(), Some(Rect::new(222, 601, 972, 1935)));
}

#[test]
fn the_mode_stays_on_after_a_board_is_placed() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    click(&mut app, 222.0, 600.0);
    assert!(app.ui.artboard_tool.adding);
    // The next click is another placement, at the new selected board's size.
    click(&mut app, 3000.0, 100.0);
    assert_eq!(boards(&app).last().copied(), Some(Rect::new(3000, 100, 3750, 1434)));
}

#[test]
fn a_click_with_the_mode_off_still_places_nothing() {
    let mut app = app_with_selected_board();
    let before = boards(&app);
    click(&mut app, 222.0, 600.0);
    assert_eq!(boards(&app), before);
}

#[test]
fn with_no_board_the_click_uses_the_bars_tool_option_size() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Artboard;
    app.ui.artboard_tool.adding = true;
    click(&mut app, 40.0, 50.0);
    // The size the bar shows (the iPhone 8/7/6 default), not the canvas's 800 x 600.
    assert_eq!(boards(&app), vec![Rect::new(40, 50, 790, 1384)]);
}

#[test]
fn a_drag_with_the_mode_on_still_draws_the_dragged_board() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 80.0, y: 70.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Move { x: 120.0, y: 110.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 120.0, y: 110.0 }, m);
    assert_eq!(boards(&app).last().copied(), Some(Rect::new(20, 30, 120, 110)));
}

#[test]
fn a_click_on_an_existing_board_selects_it_and_places_nothing() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    let before = boards(&app);
    click(&mut app, 2100.0, 100.0);
    assert_eq!(boards(&app), before);
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer, Some(st.doc.artboards()[0].0));
}

#[test]
fn another_tool_turns_the_mode_off() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    assert_eq!(serde_json::to_value(&app.ui.artboard_tool).unwrap()["adding"], json!(true));
    app.ui.tool = Tool::Move;
    click(&mut app, 3000.0, 100.0);
    assert!(!app.ui.artboard_tool.adding);
    app.ui.tool = Tool::Artboard;
    let before = boards(&app);
    click(&mut app, 3000.0, 100.0);
    assert_eq!(boards(&app), before);
}

#[test]
fn ui_inspect_reports_the_mode_on_and_off() {
    let ctx = egui::Context::default();
    let mut app = app_with_selected_board();
    let adding = |app: &mut PhotocraftApp| {
        let (req, _) = crate::control::ControlRequest::new("ui.inspect", json!({}));
        let crate::control::Outcome::Done(v) = crate::control::handle(app, &ctx, &req) else { panic!("inspect") };
        v["result"]["artboardTool"]["adding"].clone()
    };
    assert_eq!(adding(&mut app), json!(false));
    app.ui.artboard_tool.adding = true;
    assert_eq!(adding(&mut app), json!(true));
}

#[test]
fn selecting_another_tool_and_coming_back_leaves_the_mode_off() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    // Both changes in one pass, with no pointer or options-bar call between them.
    crate::tool_memory::select(&mut app, Tool::Move);
    crate::tool_memory::select(&mut app, Tool::Artboard);
    assert!(!app.ui.artboard_tool.adding);
    let before = boards(&app);
    click(&mut app, 222.0, 600.0);
    assert_eq!(boards(&app), before);
}

#[test]
fn the_add_new_artboard_button_toggles_the_mode_and_adds_no_board() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    let mut app = app_with_selected_board();
    app.ui.tool = Tool::Artboard;
    let before = boards(&app);
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 200.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::panels::options_bar(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(4);
    assert!(!h.state().ui.artboard_tool.adding);
    h.get_by_label("Add New Artboard").click();
    h.run_steps(4);
    assert!(h.state().ui.artboard_tool.adding, "the first click turns the mode on");
    assert_eq!(boards(h.state()), before);
    h.get_by_label("Add New Artboard").click();
    h.run_steps(4);
    assert!(!h.state().ui.artboard_tool.adding, "the second click turns it off");
    assert_eq!(boards(h.state()), before);
}

#[test]
fn the_tool_shortcuts_away_and_back_leave_the_mode_off() {
    let mut app = app_with_selected_board();
    app.ui.artboard_tool.adding = true;
    let ctx = egui::Context::default();
    let press = |app: &mut PhotocraftApp, modifiers| {
        let key = egui::Key::V;
        let raw = egui::RawInput {
            events: vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
                egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
            ],
            ..Default::default()
        };
        ctx.begin_pass(raw);
        crate::shortcuts::handle(app, &ctx);
        ctx.end_pass().textures_delta.clear();
    };
    // Shift+V leaves the Artboard tool for Move, and Shift+V again comes back, with no pointer or options-bar call between.
    press(&mut app, Modifiers::SHIFT);
    assert_eq!(app.ui.tool, Tool::Move);
    press(&mut app, Modifiers::SHIFT);
    assert_eq!(app.ui.tool, Tool::Artboard);
    assert!(!app.ui.artboard_tool.adding);
    let before = boards(&app);
    click(&mut app, 222.0, 600.0);
    assert_eq!(boards(&app), before);
}
