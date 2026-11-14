//! #1318: the Layers panel's Blend Mode dropdown takes the arrow keys once it is open, ahead of
//! the tool shortcuts (the Move tool's arrow-key nudge).

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

#[test]
fn arrow_keys_step_the_open_blend_mode_dropdown_without_nudging_the_layer() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.ui.tool = Tool::Move;
        app
    });
    h.run_steps(6);
    let layer = |h: &Harness<'_, PhotocraftApp>| {
        let st = h.state().session.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
        (l.blend, l.surface().unwrap().content_bounds())
    };
    let (blend, bounds) = layer(&h);
    assert_eq!(blend, BlendMode::Normal);
    let named = |n: &egui_kittest::Node<'_>| {
        let a = n.accesskit_node();
        a.value().as_deref() == Some("Normal") || a.label().as_deref() == Some("Normal")
    };
    let combo = h.query_all_by_role(Role::ComboBox).find(|n| named(n));
    combo.expect("the Layers panel's Blend Mode dropdown").click();
    h.run_steps(2);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(2);
    let (blend, moved) = layer(&h);
    assert_ne!(blend, BlendMode::Normal, "↓ picks the next mode");
    assert_eq!(moved, bounds, "the Move tool's nudge doesn't also take the key");
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(layer(&h).0, BlendMode::Normal);
}

/// #1481: with the Blend Mode list open, a letter picks the next mode starting with it (as in
/// Photoshop's pop-up menus) instead of reaching the tool shortcuts, and ↩ closes the list.
#[test]
fn letters_pick_blend_modes_in_the_open_list_without_switching_tools() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.ui.tool = Tool::Move;
        app
    });
    h.run_steps(6);
    let blend = |h: &Harness<'_, PhotocraftApp>| {
        let st = h.state().session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().blend
    };
    let type_key = |h: &mut Harness<'_, PhotocraftApp>, key: egui::Key, text: &str| {
        h.key_press(key);
        h.event(egui::Event::Text(text.into()));
        h.run_steps(2);
    };
    let named = |n: &egui_kittest::Node<'_>| {
        let a = n.accesskit_node();
        a.value().as_deref() == Some("Normal") || a.label().as_deref() == Some("Normal")
    };
    h.query_all_by_role(Role::ComboBox).find(|n| named(n)).expect("the Layers panel's Blend Mode dropdown").click();
    h.run_steps(2);
    type_key(&mut h, egui::Key::M, "m");
    assert_eq!(blend(&h), BlendMode::Multiply, "M picks Multiply");
    assert_eq!(h.state().ui.tool, Tool::Move, "M doesn't also pick the Marquee tool");
    type_key(&mut h, egui::Key::D, "d");
    assert_eq!(blend(&h), BlendMode::DarkerColor, "D picks the next mode starting with D");
    type_key(&mut h, egui::Key::D, "d");
    assert_eq!(blend(&h), BlendMode::Difference, "D again picks the one after");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(!crate::widgets::dropdown_list_open(&h.ctx), "↩ closes the list");
    assert_eq!(blend(&h), BlendMode::Difference, "↩ keeps the mode");
    // With the list closed the letters are the tools' again.
    h.key_press(egui::Key::M);
    h.event(egui::Event::Text("m".into()));
    h.run_steps(2);
    assert!(h.state().ui.tool != Tool::Move, "M picks a marquee tool once the list is closed");
}

/// #9065: a command shortcut pressed with the Blend Mode list open closes the list and runs, as
/// with Photoshop's menus (⌘1 there closes an open panel or menu bar menu and zooms to 100%).
/// Letters and arrows stay the list's (the tests above).
#[test]
fn a_command_shortcut_closes_the_open_list_and_runs() {
    command_shortcut_closes_the_open_list_and_runs(egui::Modifiers::COMMAND);
}

/// #9314 review: the same with the modifiers a real key press carries, not just egui's logical
/// `command`: Ctrl on Windows and Linux, ⌘ on macOS.
#[test]
fn a_command_shortcut_with_each_platforms_modifiers_closes_the_open_list_and_runs() {
    command_shortcut_closes_the_open_list_and_runs(egui::Modifiers::CTRL | egui::Modifiers::COMMAND);
    command_shortcut_closes_the_open_list_and_runs(egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND);
}

/// ⌘1 / Ctrl+1 (100%), pressed with `mods`, with the Blend Mode list open.
fn command_shortcut_closes_the_open_list_and_runs(mods: egui::Modifiers) {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 3000, "height": 2000})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app
    });
    h.run_steps(6);
    assert_ne!(h.state().ui.views[0].zoom, 1.0, "the document starts fitted, not at 100%");
    let named = |n: &egui_kittest::Node<'_>| {
        let a = n.accesskit_node();
        a.value().as_deref() == Some("Normal") || a.label().as_deref() == Some("Normal")
    };
    h.query_all_by_role(Role::ComboBox).find(|n| named(n)).expect("the Layers panel's Blend Mode dropdown").click();
    h.run_steps(2);
    assert!(crate::widgets::dropdown_list_open(&h.ctx), "the list is open");
    h.key_press_modifiers(mods, egui::Key::Num1);
    h.run_steps(3);
    assert_eq!(h.state().ui.views[0].zoom, 1.0, "⌘1 (100%) runs with the list open: {mods:?}");
    assert!(!crate::widgets::dropdown_list_open(&h.ctx), "and closes the list: {mods:?}");
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().blend, BlendMode::Normal, "the mode is unchanged: {mods:?}");
}

/// #9065 (CodeRabbit on #9314): with the Layers panel's options menu open, a command shortcut
/// closes the menu and runs, as Photoshop's panel flyout does (⌘1 there closes it and zooms to
/// 100%).
#[test]
fn a_command_shortcut_closes_the_open_panel_menu_and_runs() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 3000, "height": 2000})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app
    });
    h.run_steps(6);
    assert_ne!(h.state().ui.views[0].zoom, 1.0, "the document starts fitted, not at 100%");
    let group = crate::panel_docking::probe::group_rect(h.state(), &h.ctx, "layers").expect("layers group");
    h.query_all_by_label("Panel menu").find(|n| group.contains(n.rect().center())).expect("the Layers panel menu").click();
    h.run_steps(2);
    assert!(h.query_by_label("Collapse All Groups").is_some(), "the Layers panel menu is open");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num1);
    h.run_steps(3);
    assert_eq!(h.state().ui.views[0].zoom, 1.0, "⌘1 (100%) runs with the menu open");
    assert!(h.query_by_label("Collapse All Groups").is_none(), "and closes the menu");
}

/// #9065 (CodeRabbit on #9314): Layer Style's contour picker keeps its open state in the
/// dialog's fields rather than egui's popup memory; a command shortcut closes it too and runs
/// (⌘1 zooms to 100%, which Layer Style lets through).
#[test]
fn a_command_shortcut_closes_the_open_contour_picker_and_runs() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 3000, "height": 2000})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app
    });
    h.run_steps(6);
    assert_ne!(h.state().ui.views[0].zoom, 1.0, "the document starts fitted, not at 100%");
    crate::layer_style::open(h.state_mut(), Some("dropShadow")).expect("Layer Style opens");
    h.run_steps(4);
    let picker = "contourPicker";
    let d = h.state_mut().ui.dialogs.last_mut().unwrap();
    let fx = d.fields["selected"].clone();
    d.fields.insert(picker.into(), json!({"effect": fx, "key": "contour"}));
    h.run_steps(3);
    assert!(h.query_by_label("Load Contours…").is_some(), "the contour picker is open");
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num1);
    h.run_steps(3);
    assert_eq!(h.state().ui.views[0].zoom, 1.0, "⌘1 (100%) runs with the picker open");
    assert!(h.query_by_label("Load Contours…").is_none(), "and closes the picker");
    assert!(!h.state().ui.dialogs.last().unwrap().fields.contains_key(picker), "and forgets it was open");
}
