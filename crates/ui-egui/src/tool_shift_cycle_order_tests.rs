//! Shift+letter steps through a key's tool group in the toolbar flyout's order, wrapping, and the
//! anchor point tools have no letter so P never reaches them.

use egui::{Key, Modifiers};
use serde_json::json;

use crate::PhotocraftApp;
use crate::panels::toolbar::{TOOL_SECTIONS, toolbar_position};
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app
}

fn shift_press(app: &mut PhotocraftApp, key: Key) {
    let modifiers = Modifiers::SHIFT;
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

/// Every lettered tool group in toolbar order, with its letter.
fn groups() -> Vec<(char, Vec<Tool>)> {
    let mut keys: Vec<char> = Tool::ALL.iter().map(|t| t.key()).filter(|k| *k != '\0').collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .map(|k| {
            let mut group: Vec<Tool> = Tool::ALL.iter().copied().filter(|t| t.key() == k && toolbar_position(*t) != usize::MAX).collect();
            group.sort_by_key(|t| toolbar_position(*t));
            (k, group)
        })
        .collect()
}

fn key_for(letter: char) -> Key {
    crate::spring_tools::key_of(letter).unwrap()
}

fn sequence(letter: char, steps: usize) -> Vec<Tool> {
    let mut app = app();
    app.ui.tool = Tool::Hand;
    (0..steps)
        .map(|_| {
            shift_press(&mut app, key_for(letter));
            app.ui.tool
        })
        .collect()
}

#[test]
fn from_every_tool_shift_letter_lands_on_the_next_tool_in_toolbar_order() {
    for (letter, group) in groups() {
        for (i, from) in group.iter().enumerate() {
            let mut app = app();
            app.ui.tool = *from;
            shift_press(&mut app, key_for(letter));
            let want = group[(i + 1) % group.len()];
            assert_eq!(app.ui.tool, want, "Shift+{letter} from {from:?}");
        }
    }
}

#[test]
fn shift_letter_wraps_from_the_last_tool_to_the_first() {
    for (letter, group) in groups() {
        let mut app = app();
        app.ui.tool = *group.last().unwrap();
        shift_press(&mut app, key_for(letter));
        assert_eq!(app.ui.tool, group[0], "Shift+{letter} wraps");
    }
}

#[test]
fn the_first_shift_letter_from_another_tool_lands_on_the_groups_first_tool() {
    for (letter, group) in groups() {
        let mut app = app();
        app.ui.tool = Tool::Hand;
        if group.contains(&Tool::Hand) {
            continue;
        }
        shift_press(&mut app, key_for(letter));
        assert_eq!(app.ui.tool, group[0], "Shift+{letter} from the Hand tool");
    }
}

#[test]
fn w_cycles_object_selection_quick_selection_magic_wand() {
    assert_eq!(sequence('W', 4), vec![Tool::ObjectSelection, Tool::QuickSelection, Tool::MagicWand, Tool::ObjectSelection]);
}

#[test]
fn t_cycles_horizontal_vertical_vertical_mask_horizontal_mask() {
    assert_eq!(sequence('T', 5), vec![Tool::Type, Tool::VerticalType, Tool::VerticalTypeMask, Tool::HorizontalTypeMask, Tool::Type]);
}

#[test]
fn p_cycles_the_three_pens_and_never_reaches_an_anchor_point_tool() {
    let seen = sequence('P', 7);
    assert_eq!(&seen[..4], &[Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen, Tool::Pen]);
    for t in seen {
        assert!(!matches!(t, Tool::AddAnchorPoint | Tool::DeleteAnchorPoint | Tool::ConvertPoint), "{t:?}");
    }
}

#[test]
fn the_anchor_point_tools_have_no_letter_but_stay_in_the_pen_flyout() {
    for t in [Tool::AddAnchorPoint, Tool::DeleteAnchorPoint, Tool::ConvertPoint] {
        assert_eq!(t.key(), '\0', "{t:?}");
        assert!(TOOL_SECTIONS.iter().flat_map(|s| s.iter()).any(|slot| slot.contains(&t)), "{t:?} stays in the toolbar");
    }
}

#[test]
fn the_cycle_order_is_the_flattened_toolbar_order() {
    let flat: Vec<Tool> = TOOL_SECTIONS.iter().flat_map(|s| s.iter()).flat_map(|slot| slot.iter().copied()).collect();
    for (i, t) in flat.iter().enumerate() {
        assert_eq!(toolbar_position(*t), i, "{t:?}");
    }
}

#[test]
fn with_the_shift_switch_preference_off_plain_w_keeps_the_old_order() {
    let mut app = app();
    app.session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = false);
    app.ui.tool = Tool::MagicWand;
    let mut seen = Vec::new();
    for _ in 0..3 {
        let ctx = egui::Context::default();
        let modifiers = Modifiers::NONE;
        let raw = egui::RawInput {
            events: vec![egui::Event::Key { key: Key::W, physical_key: None, pressed: true, repeat: false, modifiers }],
            ..Default::default()
        };
        ctx.begin_pass(raw);
        crate::shortcuts::handle(&mut app, &ctx);
        ctx.end_pass().textures_delta.clear();
        seen.push(app.ui.tool);
    }
    assert_eq!(seen, vec![Tool::QuickSelection, Tool::ObjectSelection, Tool::MagicWand]);
}

#[test]
fn a_plain_w_from_outside_the_group_keeps_selecting_what_it_selected_before() {
    let mut app = app();
    app.ui.tool = Tool::Move;
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![egui::Event::Key { key: Key::W, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(&mut app, &ctx);
    ctx.end_pass().textures_delta.clear();
    assert_eq!(app.ui.tool, Tool::ObjectSelection, "fresh W selects the first tool in the flyout");
}
