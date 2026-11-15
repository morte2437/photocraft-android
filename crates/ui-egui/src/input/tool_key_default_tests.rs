use super::{Key, Modifiers, PhotocraftApp, Tool, handle};
use crate::panels::toolbar::TOOL_SECTIONS;
use crate::spring_tools::key_of;
use std::collections::HashSet;

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
}

fn press(app: &mut PhotocraftApp, ctx: &egui::Context, key: Key, modifiers: Modifiers) {
    let raw = egui::RawInput {
        events: vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
            egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
        ],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    handle(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

/// A fresh app enters each default letter group at its first toolbar tool under either preference.
#[test]
fn fresh_tool_letters_pick_the_first_tool_in_toolbar_order() {
    for shift_switch in [true, false] {
        let mut checked = HashSet::new();
        for tool in TOOL_SECTIONS.iter().flat_map(|section| section.iter()).flat_map(|slot| slot.iter()).copied() {
            let Some(key) = key_of(tool.key()) else { continue };
            if !checked.insert(key) {
                continue;
            }

            let mut app = app();
            app.session.edit_prefs(|prefs| prefs.tools.use_shift_key_for_tool_switch = shift_switch);
            if app.ui.tool.key() == tool.key() {
                app.ui.tool = Tool::Brush;
            }
            press(&mut app, &egui::Context::default(), key, Modifiers::NONE);
            assert_eq!(app.ui.tool, tool, "{key:?}, Shift switch: {shift_switch}");
        }
        assert_eq!(checked, Tool::ALL.into_iter().filter_map(|tool| key_of(tool.key())).collect::<HashSet<_>>());
    }
}

/// A remembered Magic Wand takes precedence over the fresh W default, even when hidden.
#[test]
fn plain_w_returns_to_the_remembered_magic_wand() {
    for hidden in [false, true] {
        let mut app = app();
        let ctx = egui::Context::default();
        if hidden {
            app.session.edit_prefs(|prefs| prefs.toolbar.hidden = vec!["MagicWand".to_owned()]);
        }
        app.ui.tool = Tool::MagicWand;
        press(&mut app, &ctx, Key::B, Modifiers::NONE);
        assert_eq!(app.ui.tool, Tool::Brush);
        press(&mut app, &ctx, Key::W, Modifiers::NONE);
        assert_eq!(app.ui.tool, Tool::MagicWand, "hidden: {hidden}");
    }
}

/// Shift+W follows the flyout order and wraps. With the default preference, plain W keeps
/// the current tool when it already belongs to the group.
#[test]
fn shift_w_cycles_in_toolbar_order() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.ui.tool = Tool::ObjectSelection;

    for expected in [Tool::QuickSelection, Tool::MagicWand, Tool::ObjectSelection] {
        press(&mut app, &ctx, Key::W, Modifiers::SHIFT);
        assert_eq!(app.ui.tool, expected);
    }
    press(&mut app, &ctx, Key::W, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::ObjectSelection);
}

#[test]
fn remapped_first_tool_is_skipped_by_its_default_letter() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("prefs.set", serde_json::json!({"path": "shortcuts.tools.select.objectSelection", "value": "F6"})).unwrap();
    press(&mut app, &ctx, Key::W, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::QuickSelection);
    press(&mut app, &ctx, Key::F6, Modifiers::NONE);
    assert_eq!(app.ui.tool, Tool::ObjectSelection);
}
