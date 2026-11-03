use photocraft_color::BlendMode;
use photocraft_doc::{LabelColor, Layer, LayerContent, LayerId};
use serde_json::json;

use super::*;

/// Background, then `A` and `B` (`B` active).
fn app() -> (PhotocraftApp, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 30})).unwrap();
    let a = LayerId(app.run("layer.new.layer", json!({"name": "A"})).unwrap()["layer"].as_u64().unwrap());
    let b = LayerId(app.run("layer.new.layer", json!({"name": "B"})).unwrap()["layer"].as_u64().unwrap());
    (app, a, b)
}

fn groups(app: &PhotocraftApp) -> Vec<Layer> {
    app.session.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l).filter(|l| l.is_group()).cloned().collect()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn menu_invocations_open_the_dialog_but_explicit_params_run_directly() {
    let (mut app, _, _) = app();
    let ctx = egui::Context::default();
    for (command, title) in [(NEW_GROUP, "New Group…"), (FROM_LAYERS, "New Group from Layers…")] {
        let opened = crate::menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        let id = opened["dialog"].as_u64().unwrap();
        let fields = &app.ui.dialogs.last().unwrap().fields;
        assert!(owns(fields));
        assert_eq!((fields["__command"].as_str(), fields["__label"].as_str()), (Some(command), Some(title)));
        assert!(groups(&app).is_empty(), "nothing happens until OK");
        app.ui.close_dialog(id);
        // `ui.dialog.open {kind: "command"}` gets the same dialog.
        let id = crate::dialogs::open_command_dialog(&mut app, command, "Group…");
        assert!(owns(&app.ui.dialogs.last().unwrap().fields));
        app.ui.close_dialog(id);
    }
    // Explicit params (Actions, CLI, MCP) run the engine without asking.
    crate::menus::invoke(&mut app, &ctx, NEW_GROUP, json!({"name": "Direct"})).unwrap();
    assert!(app.ui.dialogs.is_empty());
    assert_eq!(groups(&app)[0].name, "Direct");
    // Group Layers (⌘G) never asks.
    crate::menus::invoke(&mut app, &ctx, "layer.groupLayers", json!({})).unwrap();
    assert!(app.ui.dialogs.is_empty());
    // Without a document nothing opens.
    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    assert!(crate::menus::invoke(&mut empty, &ctx, NEW_GROUP, json!({})).is_err());
    assert!(crate::menus::invoke(&mut empty, &ctx, FROM_LAYERS, json!({})).is_err());
    assert!(empty.ui.dialogs.is_empty());
}

#[test]
fn defaults_are_photoshops_and_private_fields_do_not_leak() {
    let (app, _, _) = app();
    let f = fields(&app, NEW_GROUP);
    let out = params(&f);
    assert_eq!(out, json!({"name": "Group 1", "color": "none", "mode": "passThrough", "opacity": 100.0}));
    assert!(out.as_object().unwrap().keys().all(|k| !k.starts_with("__")));
}

#[test]
fn ok_creates_the_group_with_the_options_in_one_step_and_cancel_changes_nothing() {
    let (mut app, _, b) = app();
    let initial = steps(&app);
    let cancelled = open(&mut app, NEW_GROUP);
    app.ui.dialog_mut(cancelled).unwrap().fields.insert("name".into(), json!("Nope"));
    app.ui.close_dialog(cancelled);
    assert_eq!(steps(&app), initial);
    assert!(groups(&app).is_empty());

    let id = open(&mut app, NEW_GROUP);
    let d = app.ui.dialog_mut(id).unwrap();
    d.fields.insert("name".into(), json!("Sky"));
    d.fields.insert("color".into(), json!("blue"));
    d.fields.insert("mode".into(), json!("multiply"));
    d.fields.insert("opacity".into(), json!(60.0));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(steps(&app), initial + 1, "one history step");
    let g = &groups(&app)[0];
    assert_eq!((g.name.as_str(), g.label, g.blend), ("Sky", LabelColor::Blue, BlendMode::Multiply));
    assert!((g.opacity - 0.6).abs() < 1e-6);
    let top: Vec<LayerId> = app.session.active().unwrap().doc.layers.iter().map(|l| l.id).collect();
    assert_eq!(top.last(), Some(&g.id), "an empty group above the active layer B");
    assert_eq!(top[top.len() - 2], b);
}

#[test]
fn ok_from_layers_groups_the_selection_collapsed() {
    let (mut app, a, b) = app();
    app.run("layer.select", json!({"layer": a.0})).unwrap();
    app.run("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    let id = open(&mut app, FROM_LAYERS);
    app.ui.dialog_mut(id).unwrap().fields.insert("name".into(), json!("Pair"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let g = &groups(&app)[0];
    assert_eq!(g.name, "Pair");
    assert_eq!(g.children().unwrap().iter().map(|l| l.id).collect::<Vec<_>>(), vec![a, b]);
    assert!(matches!(&g.content, LayerContent::Group(g) if !g.expanded), "collapsed, as in Photoshop");
}

/// The dialog draws its fields, opens with the name selected so typing replaces it, and Enter
/// runs OK.
#[test]
fn typing_replaces_the_selected_name_and_enter_confirms() {
    use egui_kittest::kittest::Queryable;
    let (app, _, _) = app();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    open(h.state_mut(), NEW_GROUP);
    h.run_steps(4);
    for label in ["OK", "Cancel"] {
        assert!(h.query_by_label(label).is_some(), "{label} is shown");
    }
    let name = h.get_by_role(egui::accesskit::Role::TextInput);
    assert!(name.is_focused(), "the dialog opens on the name");
    name.type_text("Clouds");
    h.run_steps(2);
    assert_eq!(h.state().ui.dialogs.last().unwrap().fields["name"], "Clouds");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(groups(h.state())[0].name, "Clouds");
}
