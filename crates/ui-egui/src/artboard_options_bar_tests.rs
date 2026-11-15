//! The Artboard tool's options bar (#11385): the iPhone 8/7/6 default size, the background for the
//! boards it draws, the Align panel and the gear menu, and what Reset Tool puts back.

use egui::Modifiers;
use photocraft_doc::{ArtboardBackground, LayerId};
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::artboard_tool::ArtboardToolUi;
use crate::artboard_tool::bar::{self, ALIGN_PANEL, GEAR, Gear};
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app_with_depth(depth: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600, "depth": depth})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app.ui.tool = Tool::Artboard;
    app
}

fn app() -> PhotocraftApp {
    app_with_depth(8)
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

fn boards(app: &PhotocraftApp) -> Vec<(LayerId, Rect, ArtboardBackground)> {
    app.session.active().unwrap().doc.artboards().iter().map(|b| (b.0, b.2.rect, b.2.background)).collect()
}

fn reset_tool(app: &mut PhotocraftApp) {
    crate::menus::invoke(app, &egui::Context::default(), crate::tool_reset::RESET, json!({})).unwrap();
}

#[test]
fn the_presets_list_has_iphone_8_7_6() {
    assert!(photocraft_engine::artboard_cmds::PRESETS.contains(&("iPhone 8/7/6", 750, 1334)));
    assert!(photocraft_engine::artboard_cmds::PRESETS.contains(&("iPhone 14", 390, 844)), "the existing entries stay");
    // The engine takes the new preset by name.
    let mut app = app();
    let r = app.run("layer.new.artboard", json!({"preset": "iPhone 8/7/6", "x": 0, "y": 0})).unwrap();
    assert_eq!(r["rect"], json!([0, 0, 750, 1334]));
}

#[test]
fn a_fresh_bar_has_the_measured_defaults() {
    let app = app();
    let o = &app.ui.artboard_tool;
    assert_eq!((o.preset.as_str(), o.width, o.height), ("iPhone 8/7/6", 750, 1334));
    assert_eq!((o.background.as_str(), o.background_color.as_str()), ("white", "#ffffff"));
    assert!(GEAR.iter().all(|(g, _)| g.checked(o)), "all four gear entries are checked");
    assert_eq!(bar::DEFAULT_SIZE, ("iPhone 8/7/6", 750, 1334));
}

#[test]
fn reset_tool_restores_the_options_and_only_for_the_artboard_tool() {
    let mut app = app();
    bar::set_size(&mut app, 200.0, 300.0);
    bar::set_custom_background(&mut app, "#336699").unwrap();
    app.ui.artboard_tool.auto_nest = false;
    app.ui.artboard_tool.shrinkwrap_on_save = false;
    // Another tool's Reset Tool leaves them.
    app.ui.tool = Tool::Move;
    reset_tool(&mut app);
    assert_eq!(app.ui.artboard_tool.width, 200);
    // The Artboard tool's puts every one back.
    app.ui.tool = Tool::Artboard;
    reset_tool(&mut app);
    assert_eq!(app.ui.artboard_tool, ArtboardToolUi::default());
    assert_eq!(app.ui.artboard_tool.height, 1334);
    // Reset All Tools too.
    bar::set_preset(&mut app, "A4");
    app.ui.artboard_tool.auto_size_canvas = false;
    crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET_ALL, json!({"confirmed": true})).unwrap();
    assert_eq!(app.ui.artboard_tool, ArtboardToolUi::default());
}

#[test]
fn the_options_are_serde_tool_options() {
    let mut app = app();
    bar::set_preset(&mut app, "Letter");
    app.ui.artboard_tool.keep_relative_position = false;
    let v = serde_json::to_value(&app.ui.artboard_tool).unwrap();
    assert_eq!(v["preset"], "Letter");
    assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(612), Some(792)));
    assert_eq!(v["keep_relative_position"], false);
    assert_eq!(v["auto_nest"], true);
    assert!(v.get("drag").is_none(), "the drag in progress is not saved");
    let back: ArtboardToolUi = serde_json::from_value(v).unwrap();
    assert_eq!(back, app.ui.artboard_tool);
    // Settings saved before these options existed load with the defaults.
    let old: ArtboardToolUi = serde_json::from_value(json!({})).unwrap();
    assert_eq!(old, ArtboardToolUi::default());
    // The whole UI state carries them, so `ui.inspect` reaches them.
    let ui = serde_json::to_value(&app.ui).unwrap();
    assert_eq!(ui["artboard_tool"]["preset"], "Letter");
}

#[test]
fn width_and_height_edit_the_tool_option_when_no_board_is_selected() {
    let mut app = app();
    assert!(boards(&app).is_empty());
    assert_eq!(bar::bar_size(&app), [750, 1334]);
    bar::set_size(&mut app, 400.0, 500.0);
    assert_eq!((app.ui.artboard_tool.width, app.ui.artboard_tool.height), (400, 500));
    assert_eq!(app.ui.artboard_tool.preset, "", "a size that is no preset's reads Custom");
    assert_eq!(bar::bar_size(&app), [400, 500]);
    // Back to a preset's size: the preset reads again only when chosen; choosing it sets the size.
    bar::set_preset(&mut app, "iPhone 14");
    assert_eq!((app.ui.artboard_tool.width, app.ui.artboard_tool.height), (390, 844));
    assert_eq!(app.ui.artboard_tool.preset, "iPhone 14");
    // Out of range and not a number are held to 1..=300000.
    bar::set_size(&mut app, 0.0, 1.0e9);
    assert_eq!((app.ui.artboard_tool.width, app.ui.artboard_tool.height), (1, 300_000));
    bar::set_size(&mut app, f32::NAN, 5.0);
    assert_eq!(app.ui.artboard_tool.width, 1);
    // No document change.
    assert_eq!(app.session.active().unwrap().doc.size.width, 800);
    assert!(boards(&app).is_empty());
    assert!(!app.session.active().unwrap().history.entries().iter().any(|e| e.contains("Artboard")));
}

#[test]
fn with_a_board_selected_the_bar_size_is_the_boards_and_the_options_stay() {
    let mut app = app();
    bar::set_size(&mut app, 400.0, 500.0);
    drag(&mut app, [100.0, 100.0], [300.0, 300.0]);
    assert_eq!(boards(&app).first().map(|b| b.1), Some(Rect::new(100, 100, 300, 300)));
    // A 200 x 200 board reads 200 x 200 (Custom), not the options.
    assert_eq!(bar::bar_size(&app), [200, 200]);
    let id = boards(&app)[0].0;
    let preset = app.session.active().unwrap().doc.layer(id).unwrap().artboard().unwrap().preset.clone();
    assert_eq!(preset, "", "Custom");
    assert_eq!((app.ui.artboard_tool.width, app.ui.artboard_tool.height), (400, 500), "the tool option is untouched");
    // Choosing iPhone 8/7/6 resizes the board in one step and keeps its top-left corner.
    let steps = app.session.active().unwrap().history.entries().len();
    app.run("layer.artboard.set", json!({"layer": id.0, "preset": "iPhone 8/7/6"})).unwrap();
    assert_eq!(boards(&app)[0].1, Rect::new(100, 100, 850, 1434));
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 1);
    assert_eq!(bar::bar_size(&app), [750, 1334]);
}

#[test]
fn bar_size_helper_prefers_the_selected_board() {
    let mut app = app();
    assert_eq!(bar::bar_size(&app), [750, 1334]);
    app.run("layer.new.artboard", json!({"rect": [0, 0, 120, 90]})).unwrap();
    assert_eq!(bar::bar_size(&app), [120, 90]);
}

/// The background reaches `layer.new.artboard` for a board the tool draws, at 8 and 16 bit.
#[test]
fn a_drawn_board_gets_the_bars_background() {
    for depth in [8, 16] {
        for (name, want) in [("white", ArtboardBackground::White), ("black", ArtboardBackground::Black), ("transparent", ArtboardBackground::Transparent)] {
            let mut app = app_with_depth(depth);
            app.ui.artboard_tool.background = name.to_string();
            drag(&mut app, [50.0, 50.0], [150.0, 120.0]);
            assert!(!app.ui.status_error, "{depth} bit {name}: {}", app.ui.status);
            let b = boards(&app);
            assert_eq!(b.len(), 1, "{depth} bit {name}");
            assert_eq!((b[0].1, b[0].2), (Rect::new(50, 50, 150, 120), want), "{depth} bit {name}");
        }
        // Other…: the picked colour.
        let mut app = app_with_depth(depth);
        bar::set_custom_background(&mut app, "#ff0000").unwrap();
        drag(&mut app, [50.0, 50.0], [150.0, 120.0]);
        let ArtboardBackground::Custom(c) = boards(&app)[0].2 else { panic!("{depth} bit: a custom background") };
        assert_eq!(c.to_rgba8(), [255, 0, 0, 255], "{depth} bit");
    }
}

/// A board placed by the Add New Artboard click takes the bar's background too.
#[test]
fn a_placed_board_gets_the_bars_background() {
    let place = |app: &mut PhotocraftApp| {
        app.ui.artboard_tool.adding = true;
        let m = Modifiers::NONE;
        tool_event(app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Up { x: 20.0, y: 30.0 }, m);
        assert!(!app.ui.status_error, "{}", app.ui.status);
    };
    let mut app = self::app();
    place(&mut app);
    assert_eq!(boards(&app).len(), 1);
    assert_eq!(boards(&app)[0].2, ArtboardBackground::White, "the default setting keeps white");
    let mut app = self::app();
    app.ui.artboard_tool.background = "transparent".to_string();
    place(&mut app);
    assert_eq!(boards(&app)[0].2, ArtboardBackground::Transparent);
    let mut app = self::app();
    bar::set_custom_background(&mut app, "#00ff00").unwrap();
    place(&mut app);
    let ArtboardBackground::Custom(c) = boards(&app)[0].2 else { panic!("a custom background") };
    assert_eq!(c.to_rgba8(), [0, 255, 0, 255]);
}

/// With no board selected, a placed board takes the size the bar shows (the tool option), not the canvas's.
#[test]
fn a_placed_board_takes_the_bars_size_with_no_board_selected() {
    let mut app = app();
    assert!(boards(&app).is_empty());
    bar::set_size(&mut app, 333.0, 222.0);
    app.ui.artboard_tool.adding = true;
    let m = Modifiers::NONE;
    tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 30.0, pressure: 1.0 }, m);
    tool_event(&mut app, ToolEvent::Up { x: 20.0, y: 30.0 }, m);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(boards(&app).first().map(|b| b.1), Some(Rect::new(20, 30, 353, 252)));
}

/// A placed board keeps the bar's preset name, so selected it shows that preset, not Custom.
#[test]
fn a_placed_board_keeps_the_bars_preset_name() {
    let mut app = app();
    let place = |app: &mut PhotocraftApp, x: f64| {
        app.ui.artboard_tool.adding = true;
        let m = Modifiers::NONE;
        tool_event(app, ToolEvent::Down { x, y: 30.0, pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Up { x, y: 30.0 }, m);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        let (id, rect, _) = *boards(app).last().unwrap();
        app.run("layer.select", json!({"layer": id.0})).unwrap();
        (id, rect)
    };
    let (id, rect) = place(&mut app, 20.0);
    assert_eq!((rect.width(), rect.height()), (750, 1334));
    assert_eq!(bar::board_preset(&app, id), "iPhone 8/7/6");
    let stored = app.session.active().unwrap().doc.layer(id).unwrap().artboard().unwrap().preset.clone();
    assert_eq!(stored, "iPhone 8/7/6");
    // Deselect, edit the size by hand: the preset is Custom and so is the next placed board.
    app.session.active_mut().unwrap().active_layer = None;
    bar::set_size(&mut app, 333.0, 222.0);
    let (id2, rect2) = place(&mut app, 2000.0);
    assert_eq!((rect2.width(), rect2.height()), (333, 222));
    assert_eq!(bar::board_preset(&app, id2), "");
}

/// OK in the picker with the colour the swatch shows leaves the board's own colour alone.
#[test]
fn ok_in_the_picker_without_a_change_leaves_the_boards_alone() {
    let mut app = app();
    let a = app.run("layer.new.artboard", json!({"rect": [0, 0, 100, 100]})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.artboard.set", json!({"layer": a, "background": "custom", "color": [0.2, 0.4, 0.6, 0.5]})).unwrap();
    app.run("layer.select", json!({"layer": a})).unwrap();
    let before = (steps(&app), boards(&app)[0].2);
    let hex = bar::shown_background(&app).1;
    let id = crate::color_picker_ui::open_for_target(&mut app, "artboardBackground", "Color Picker (Color)", crate::color_picker_ui::parse_hex(&hex).unwrap());
    let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    crate::color_picker_ui::confirm(&mut app, &fields).unwrap();
    assert_eq!((steps(&app), boards(&app)[0].2), before, "no history step, colour bit-exact");
    // A real change still recolours.
    bar::set_custom_background(&mut app, "#00ff00").unwrap();
    assert_eq!(steps(&app), before.0 + 1);
    assert_ne!(boards(&app)[0].2, before.1);
}

/// Two boards, both selected.
fn two_boards(app: &mut PhotocraftApp) -> (u64, u64) {
    let a = app.run("layer.new.artboard", json!({"rect": [0, 0, 100, 100]})).unwrap()["layer"].as_u64().unwrap();
    let b = app.run("layer.new.artboard", json!({"rect": [300, 0, 100, 100]})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.select", json!({"layer": a})).unwrap();
    app.run("layer.select", json!({"layer": b, "mode": "add"})).unwrap();
    (a, b)
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn last_label(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.entries().last().cloned()
}

/// Two selected boards with different custom colours; the first board's colour is the swatch's.
fn two_coloured_boards(app: &mut PhotocraftApp) -> (u64, u64) {
    let (a, b) = two_boards(app);
    app.run("layer.artboard.set", json!({"layer": a, "background": "custom", "color": "#336699"})).unwrap();
    app.run("layer.artboard.set", json!({"layer": b, "background": "custom", "color": "#cc3300"})).unwrap();
    (a, b)
}

/// Open the Color Picker from the swatch (as the bar does) and OK it with `color`, or as opened.
fn pick_from_swatch(app: &mut PhotocraftApp, color: Option<&str>) {
    let hex = bar::shown_background(app).1;
    let id = crate::color_picker_ui::open_for_target(app, "artboardBackground", "Color Picker (Color)", crate::color_picker_ui::parse_hex(&hex).unwrap());
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    if let Some(c) = color {
        fields.insert("color".into(), json!(c));
    }
    crate::color_picker_ui::confirm(app, &fields).unwrap();
}

fn colours(app: &PhotocraftApp) -> Vec<ArtboardBackground> {
    boards(app).iter().map(|b| b.2).collect()
}

#[test]
fn an_unchanged_ok_with_boards_of_different_colours_leaves_every_board_alone() {
    let mut app = app();
    two_coloured_boards(&mut app);
    let before = (steps(&app), colours(&app));
    pick_from_swatch(&mut app, None);
    assert_eq!((steps(&app), colours(&app)), before, "no history step, both colours bit-exact");
    assert_eq!((app.ui.artboard_tool.background.as_str(), app.ui.artboard_tool.background_color.as_str()), ("custom", "#336699"));
}

#[test]
fn a_changed_ok_with_boards_of_different_colours_recolours_both_in_one_step() {
    let mut app = app();
    two_coloured_boards(&mut app);
    let steps0 = steps(&app);
    pick_from_swatch(&mut app, Some("#00ff00"));
    assert_eq!(steps(&app), steps0 + 1);
    assert_eq!(last_label(&app).as_deref(), Some("Edit Artboards"));
    let green = |c: &ArtboardBackground| matches!(c, ArtboardBackground::Custom(c) if c.to_rgba8()[..3] == [0, 255, 0]);
    assert!(colours(&app).iter().all(green));
}

#[test]
fn an_unchanged_ok_with_only_web_colors_leaves_the_boards_alone() {
    let mut app = app();
    app.ui.color_panel.web_only = true;
    let (a, _) = two_boards(&mut app);
    app.run("layer.artboard.set", json!({"layer": a, "background": "custom", "color": [0.2, 0.4, 0.61, 1.0]})).unwrap();
    let before = (steps(&app), colours(&app));
    pick_from_swatch(&mut app, None);
    assert_eq!((steps(&app), colours(&app)), before, "no history step, colours bit-exact");
}

#[test]
fn a_colour_some_boards_already_have_recolours_only_the_others() {
    let mut app = app();
    let (_, b) = two_coloured_boards(&mut app);
    let b_before = colours(&app)[1];
    let steps0 = steps(&app);
    pick_from_swatch(&mut app, Some("#cc3300"));
    assert_eq!(steps(&app), steps0 + 1);
    assert_eq!(last_label(&app).as_deref(), Some("Edit Artboard"), "the label follows the one board that changed");
    assert_eq!(colours(&app)[1], b_before, "board {b} keeps its own value");
    assert!(matches!(colours(&app)[0], ArtboardBackground::Custom(c) if c.to_rgba8()[..3] == [0xcc, 0x33, 0x00]));
}

#[test]
fn with_a_board_selected_the_bar_shows_its_background() {
    let mut app = app();
    let a = app.run("layer.new.artboard", json!({"rect": [0, 0, 100, 100]})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.artboard.set", json!({"layer": a, "background": "black"})).unwrap();
    app.run("layer.select", json!({"layer": a})).unwrap();
    assert_eq!(app.ui.artboard_tool.background, "white", "the tool option is still white");
    assert_eq!(bar::shown_background(&app).0, "black");
    app.run("layer.artboard.set", json!({"layer": a, "background": "custom", "color": "#336699"})).unwrap();
    assert_eq!(bar::shown_background(&app), ("custom".to_string(), "#336699".to_string()));
}

#[test]
fn a_background_choice_recolours_every_selected_board_in_one_step() {
    let mut app = app();
    let (a, b) = two_boards(&mut app);
    let before = steps(&app);
    bar::set_background(&mut app, "transparent", "#ffffff").unwrap();
    assert_eq!(steps(&app), before + 1);
    assert_eq!(last_label(&app).as_deref(), Some("Edit Artboards"));
    assert!(boards(&app).iter().all(|x| x.2 == ArtboardBackground::Transparent));
    assert_eq!(boards(&app).iter().map(|x| x.0.0).collect::<Vec<_>>(), vec![a, b]);
    assert_eq!(app.ui.artboard_tool.background, "transparent", "also the next drawn board's");
    // Other...: OK in the picker recolours them too.
    bar::set_custom_background(&mut app, "#00ff00").unwrap();
    assert_eq!(steps(&app), before + 2);
    for x in boards(&app) {
        let ArtboardBackground::Custom(c) = x.2 else { panic!("a custom background") };
        assert_eq!(c.to_rgba8(), [0, 255, 0, 255]);
    }
    assert_eq!((app.ui.artboard_tool.background.as_str(), app.ui.artboard_tool.background_color.as_str()), ("custom", "#00ff00"));
}

#[test]
fn one_selected_board_is_edit_artboard() {
    let mut app = app();
    let a = app.run("layer.new.artboard", json!({"rect": [0, 0, 100, 100]})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.select", json!({"layer": a})).unwrap();
    let before = steps(&app);
    bar::set_background(&mut app, "black", "#ffffff").unwrap();
    assert_eq!(steps(&app), before + 1);
    assert_eq!(last_label(&app).as_deref(), Some("Edit Artboard"));
    assert_eq!(boards(&app)[0].2, ArtboardBackground::Black);
}

#[test]
fn with_no_board_selected_the_background_edits_only_the_tool_option() {
    let mut app = app();
    let before = steps(&app);
    bar::set_background(&mut app, "black", "#ffffff").unwrap();
    assert_eq!(app.ui.artboard_tool.background, "black");
    assert_eq!(steps(&app), before, "no history step");
    assert!(boards(&app).is_empty());
}

#[test]
fn the_background_param_carries_color_only_for_other() {
    let mut app = app();
    assert_eq!(bar::with_background(&app, json!({"rect": [0, 0, 5, 5]})), json!({"rect": [0, 0, 5, 5], "background": "white"}));
    bar::set_custom_background(&mut app, "#00ff00").unwrap();
    assert_eq!(bar::with_background(&app, json!({})), json!({"background": "custom", "color": "#00ff00"}));
    assert!(bar::set_custom_background(&mut app, "green").is_err());
    assert_eq!(app.ui.artboard_tool.background_color, "#00ff00", "a bad colour changes nothing");
}

#[test]
fn other_opens_the_color_picker_and_ok_sets_the_option() {
    let mut app = app();
    app.ui.artboard_tool.background = "black".to_string();
    // What the dropdown's Other… and the swatch do.
    app.ui.artboard_tool.background_color = "#123456".to_string();
    let before = app.ui.dialogs.len();
    let rgb = crate::color_picker_ui::parse_hex("#123456").unwrap();
    let id = crate::color_picker_ui::open_for_target(&mut app, "artboardBackground", "Color Picker (Color)", rgb);
    assert_eq!(app.ui.dialogs.len(), before + 1);
    // Cancel leaves the option alone; OK sets it.
    assert_eq!(app.ui.artboard_tool.background, "black");
    let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    crate::color_picker_ui::confirm(&mut app, &fields).unwrap();
    assert_eq!((app.ui.artboard_tool.background.as_str(), app.ui.artboard_tool.background_color.as_str()), ("custom", "#123456"));
}

#[test]
fn the_align_panel_has_the_eight_entries_in_the_measured_order() {
    let labels: Vec<Vec<&str>> = ALIGN_PANEL.iter().map(|g| g.iter().map(|e| e.0).collect()).collect();
    assert_eq!(
        labels,
        vec![
            vec!["Left Edges", "Horizontal Centers", "Right Edges"],
            vec!["Top Edges", "Vertical Centers", "Bottom Edges"],
            vec!["Distribute Widths", "Distribute Heights"],
        ]
    );
    let specs = photocraft_engine::command_specs();
    for (label, cmd) in ALIGN_PANEL.iter().flat_map(|g| g.iter()) {
        match cmd {
            Some(c) => {
                assert!(specs.iter().any(|s| s.id == *c), "{c} is a command");
                assert!(crate::align_bar::ALIGN.iter().any(|a| a.0 == *c && a.2 == *label), "{label} is the Move tool's");
            }
            None => assert!(label.starts_with("Distribute"), "{label} has no command yet"),
        }
    }
    // Distribute Widths and Heights run nothing.
    assert!(bar::run_align(&mut app(), 2, 0).is_none());
    assert!(bar::run_align(&mut app(), 2, 1).is_none());
}

/// A filled square at `r` (`x, y, w, h`); the engine aligns a board by its contents.
fn square(app: &mut PhotocraftApp, r: [i64; 4]) -> u64 {
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": r[0], "y": r[1], "width": r[2], "height": r[3]})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.session.active().unwrap().active_layer.unwrap().0
}

fn wrap(app: &mut PhotocraftApp, layer: u64) -> u64 {
    app.run("layer.select", json!({"layer": layer})).unwrap();
    app.run("layer.new.artboardFromLayers", json!({})).unwrap()["layer"].as_u64().unwrap()
}

#[test]
fn an_align_entry_moves_two_selected_boards_in_one_step() {
    let mut app = app();
    let (sa, sb) = (square(&mut app, [40, 10, 100, 100]), square(&mut app, [300, 200, 80, 100]));
    let (a, b) = (wrap(&mut app, sa), wrap(&mut app, sb));
    app.run("layer.select", json!({"layer": a})).unwrap();
    app.run("layer.select", json!({"layer": b, "mode": "add"})).unwrap();
    assert!(app.session.is_enabled("layer.align.leftEdges"));
    let before: Vec<Rect> = boards(&app).iter().map(|b| b.1).collect();
    assert_eq!(before.len(), 2);
    let steps = app.session.active().unwrap().history.entries().len();
    let r = bar::run_align(&mut app, 0, 0);
    assert!(matches!(r, Some(Ok(_))), "{r:?}");
    let after: Vec<Rect> = boards(&app).iter().map(|b| b.1).collect();
    // Left Edges: the second board (with its contents) moves left to the first one's edge.
    assert_eq!(after[0], before[0]);
    assert_eq!((after[1].x0, after[1].y0), (before[0].x0, before[1].y0));
    assert_eq!((after[1].width(), after[1].height()), (before[1].width(), before[1].height()));
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps + 1, "one history step");
    // Top Edges (second group, first entry) moves it up to the first board's top.
    assert!(matches!(bar::run_align(&mut app, 1, 0), Some(Ok(_))));
    let after: Vec<Rect> = boards(&app).iter().map(|b| b.1).collect();
    assert_eq!((after[1].x0, after[1].y0), (before[0].x0, before[0].y0));
}

#[test]
fn the_gear_menu_has_four_entries_all_checked_and_only_auto_nest_live() {
    let labels: Vec<&str> = GEAR.iter().map(|g| g.1).collect();
    assert_eq!(labels, vec!["Auto-nest Layers", "Auto-size Canvas", "Keep Relative Position During Layer Reordering", "Shrinkwrap Canvas on Save"]);
    let live: Vec<Gear> = GEAR.iter().map(|g| g.0).filter(|g| g.enabled()).collect();
    assert_eq!(live, vec![Gear::AutoNest]);
    let mut o = ArtboardToolUi::default();
    for (g, _) in GEAR {
        assert!(g.checked(&o));
    }
    o.auto_nest = false;
    o.shrinkwrap_on_save = false;
    let checked: Vec<bool> = GEAR.iter().map(|g| g.0.checked(&o)).collect();
    assert_eq!(checked, vec![false, true, true, false]);
}

/// Auto-nest Layers off: a layer dragged with the Move tool onto another board stays where it was.
#[test]
fn auto_nest_layers_turns_the_move_tools_nesting_on_and_off() {
    for (nest, joins) in [(true, true), (false, false)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 300, "height": 100, "background": "transparent"})).unwrap();
        app.sync_views();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.session.execute("select.deselect", json!({})).unwrap();
        let red = app.session.active().unwrap().active_layer.unwrap();
        app.session.execute("layer.new.artboardFromLayers", json!({"name": "A"})).unwrap();
        app.session.execute("layer.artboard.set", json!({"width": 40, "height": 40})).unwrap();
        let b = LayerId(app.session.execute("layer.new.artboard", json!({"rect": [200, 20, 40, 40], "name": "B"})).unwrap()["layer"].as_u64().unwrap());
        app.session.execute("layer.select", json!({"layer": red.0})).unwrap();
        app.ui.tool = Tool::Move;
        app.ui.artboard_tool.auto_nest = nest;
        crate::canvas::move_ui::finish(&mut app, 190.0, 10.0);
        let board = app.session.active().unwrap().doc.artboard_of(red);
        assert_eq!(board == Some(b), joins, "auto_nest {nest}");
    }
}
