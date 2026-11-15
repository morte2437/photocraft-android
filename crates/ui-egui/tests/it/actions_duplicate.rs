//! The panel duplicates the selected action/step and keeps its new row selected.
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_engine::{Session, actions_cmds::Action, atn_cmds};
use photocraft_ui_egui::{PhotocraftApp, actions, dialogs};
use serde_json::json;

#[test]
fn duplicate_menu_selects_the_new_action_and_then_the_new_step() {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    let mut a = Action::new("Source", vec![("select.all".into(), json!({}))]);
    a.disabled.insert(0);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    app.ui.actions.selected = Some(0);
    app.ui.actions.expanded = vec![true];
    let mut h = Harness::builder().with_size(egui::vec2(360.0, 460.0)).build_ui_state(|ui, app| actions::panel_menu(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    h.get_by_label("Duplicate").click();
    h.run_steps(3);
    assert_eq!(h.state().session.actions.list.len(), 2);
    assert_eq!(h.state().session.actions.list[1].name, "Source copy");
    assert_eq!(h.state().ui.actions.selected, Some(1));
    assert_eq!(h.state().ui.actions.selected_step, None);
    assert_eq!(h.state().ui.actions.expanded, [true, true]);
    h.state_mut().ui.actions.selected_step = Some(0);
    h.get_by_label("Duplicate").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.actions.selected, Some(1));
    assert_eq!(h.state().ui.actions.selected_step, Some(1));
    let copied = &h.state().session.actions.list[1];
    assert_eq!(copied.steps.len(), 2);
    assert_eq!(copied.disabled.iter().copied().collect::<Vec<_>>(), [0, 1]);
    assert_eq!(copied.dialogs.iter().copied().collect::<Vec<_>>(), [0, 1]);
}

#[test]
fn duplicate_cannot_shift_the_owner_of_a_suspended_recording() {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    app.session.actions.list.push(Action::new("Menu", vec![(atn_cmds::MENU_ITEM.into(), json!({"id": "image.imageSize"}))]));
    app.run("actions.record", json!({"name": "Owner"})).unwrap();
    let result = app.run("actions.play", json!({"action": "Menu"})).unwrap();
    let dialog = result["paused"]["dialog"].as_u64().unwrap();
    let list = app.session.actions.list.clone();
    assert!(app.session.actions.recording.is_none());
    let error = app.run("actions.duplicate", json!({"action": "Menu"})).unwrap_err();
    assert!(error.contains("Confirm or cancel"), "{error}");
    assert_eq!(app.session.actions.list, list);
    dialogs::cancel(&mut app, dialog).unwrap();
    assert_eq!(app.session.actions.recording.map(|(_, i)| i), Some(1));
    app.run("actions.stop", json!({})).unwrap();
}
