//! Ctrl+/ (Cmd+/ on the Mac) opens Layer > Lock Layers..., and the Layer menu and the Layers panel
//! menu show the command's current key. Plain / still toggles Lock Transparent Pixels (#11397).

use egui::{Key, Modifiers};
use serde_json::json;

use crate::PhotocraftApp;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 20, "height": 10})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app
}

fn press(app: &mut PhotocraftApp, key: Key, modifiers: Modifiers) {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

fn locks(app: &PhotocraftApp) -> photocraft_doc::Locks {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().locks
}

fn set_shortcut(app: &mut PhotocraftApp, value: &str) {
    app.session.prefs.edit(|prefs| prefs.shortcuts.insert("layer.lockLayers".into(), value.into()));
}

#[test]
fn lock_layers_defaults_to_command_slash_in_the_engine_and_the_menu() {
    assert_eq!(photocraft_engine::commands::find("layer.lockLayers").unwrap().shortcut, Some("Cmd+/"));
    let row = crate::menu_catalog::CATALOG.iter().find(|c| c.3 == "layer.lockLayers").unwrap();
    assert_eq!(row.2, Some("Cmd+/"));
    assert_eq!(crate::shortcuts::default_shortcut("layer.lockLayers").as_deref(), Some("Cmd+/"));
}

#[test]
fn command_slash_runs_what_the_menu_item_runs() {
    let mut via_key = app();
    press(&mut via_key, Key::Slash, Modifiers::COMMAND);
    let mut via_menu = app();
    crate::menus::invoke(&mut via_menu, &egui::Context::default(), "layer.lockLayers", json!({})).unwrap();
    assert_eq!(locks(&via_key), locks(&via_menu));
    assert!(locks(&via_key).all, "Lock Layers... with no choices locks the layer");
}

#[test]
fn command_slash_does_nothing_when_the_menu_item_is_greyed() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert!(!crate::menus::is_enabled(&app, "layer.lockLayers"));
    press(&mut app, Key::Slash, Modifiers::COMMAND);
    assert!(app.session.active().is_none());
    assert!(app.ui.layers_panel_dialog.is_none());
}

/// Every text the Layers panel menu paints in one frame (the row labels and their shortcut texts).
fn painted_text(app: &mut PhotocraftApp) -> Vec<String> {
    fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput::default());
    let mut ui = egui::Ui::new(
        ctx.clone(),
        egui::Id::unique("lock-layers-panel-menu"),
        egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 900.0))),
    );
    crate::panels::layer_row_ui::panel_menu(app, &mut ui);
    // The texture deltas go nowhere in a test: clear them, or epaint's drop check complains.
    let mut pass = ctx.end_pass();
    pass.textures_delta.clear();
    let mut out = Vec::new();
    for clipped in pass.shapes {
        collect(&clipped.shape, &mut out);
    }
    out
}

#[test]
fn panel_menu_row_shows_the_current_key_and_follows_a_change() {
    let mut app = app();
    let default_key = crate::shortcuts::pretty("Cmd+/");
    assert_eq!(crate::shortcuts::shortcut_label(&app, "layer.lockLayers"), Some(default_key.clone()));
    // The panel menu paints the key beside Lock Layers…
    let texts = painted_text(&mut app);
    assert!(texts.iter().any(|t| t == "Lock Layers…"), "{texts:?}");
    assert!(texts.contains(&default_key), "the default key is painted: {texts:?}");
    // A changed key is painted instead of the default.
    set_shortcut(&mut app, "Cmd+Shift+L");
    let changed = crate::shortcuts::pretty("Cmd+Shift+L");
    assert_eq!(crate::shortcuts::shortcut_label(&app, "layer.lockLayers"), Some(changed.clone()));
    let texts = painted_text(&mut app);
    assert!(texts.contains(&changed), "{texts:?}");
    assert!(!texts.contains(&default_key), "the old key is gone: {texts:?}");
    // A removed key paints no key at all.
    let mut app = app_with_removed_key();
    let texts = painted_text(&mut app);
    assert!(texts.iter().any(|t| t == "Lock Layers…"), "{texts:?}");
    assert!(!texts.contains(&default_key) && !texts.contains(&changed), "{texts:?}");
}

#[test]
fn the_key_can_be_changed_and_removed() {
    let mut app = app();
    set_shortcut(&mut app, "Cmd+Shift+L");
    press(&mut app, Key::Slash, Modifiers::COMMAND);
    assert!(!locks(&app).all, "the old key no longer runs the command");
    press(&mut app, Key::L, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(locks(&app).all, "the new key does");
    let mut app = app_with_removed_key();
    press(&mut app, Key::Slash, Modifiers::COMMAND);
    assert!(!locks(&app).all);
    assert_eq!(crate::shortcuts::shortcut_label(&app, "layer.lockLayers"), None);
}

fn app_with_removed_key() -> PhotocraftApp {
    let mut app = app();
    set_shortcut(&mut app, "");
    app
}

#[test]
fn plain_slash_toggles_lock_transparency_and_command_slash_does_not() {
    let mut app = app();
    press(&mut app, Key::Slash, Modifiers::NONE);
    assert!(locks(&app).transparency && !locks(&app).all);
    press(&mut app, Key::Slash, Modifiers::NONE);
    assert!(!locks(&app).transparency);
    press(&mut app, Key::Slash, Modifiers::COMMAND);
    assert!(locks(&app).all);
    assert!(!locks(&app).transparency, "Ctrl+/ leaves Lock Transparent Pixels alone");
}
