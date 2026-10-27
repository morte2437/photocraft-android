use super::*;

#[test]
fn panel_menu_lists_load_save_and_existing_action_commands() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut empty_app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    empty_app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let mut empty = Harness::builder().with_size(vec2(240.0, 280.0)).build_ui_state(|ui, app| panel_menu(app, ui), empty_app);
    empty.run_steps(2);
    assert!(empty.query_all_by_label("Load Actions…").count() > 0);
    assert!(empty.query_all_by_label("New Action…").count() > 0);
    assert!(empty.query_all_by_label("Start Recording").count() > 0);
    empty.get_by_label("New Action…").click();
    empty.run_steps(2);
    assert_eq!(empty.state().session.actions.list.len(), 1);

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.session.actions.list.push(Action::new("Make Layer", vec![("layer.new.layer".into(), json!({}))]));
    app.ui.actions.selected = Some(0);
    let mut h = Harness::builder().with_size(vec2(240.0, 320.0)).build_ui_state(|ui, app| panel_menu(app, ui), app);
    h.run_steps(2);
    for label in ["Load Actions…", "Save Actions…", "Insert Stop…", "Play", "Delete", "Start Recording"] {
        assert!(h.query_all_by_label(label).count() > 0, "{label}");
    }
    let layers = h.state().session.active().unwrap().doc.layers.len();
    h.get_by_label("Play").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers + 1);
}

#[test]
fn selected_layer_name_is_visible_during_recording_and_after_stop() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":4,"height":4})).unwrap();
    app.run("layer.setProps", json!({"name":"Background"})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.run("actions.record", json!({"name":"Select photo"})).unwrap();
    app.ui.actions.expanded = vec![true];
    app.run("layer.select", json!({"layer":layer.0})).unwrap();
    let mut h = Harness::builder().with_size(vec2(400.0, 260.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(4);
    h.get_by_label("Select Layer (Background)");
    h.state_mut().run("actions.stop", json!({})).unwrap();
    h.run_steps(4);
    h.get_by_label("Select Layer (Background)");
    assert_eq!(step_label("layer.select", &json!({"layer":26})), "Select Layer (ID 26)");
    assert_eq!(step_label("layer.select", &json!({"layer":"Background"})), "Select Layer (Background)");
}

#[test]
fn actions_list_grows_with_panel_and_keeps_footer_visible() {
    use egui_kittest::{Harness, kittest::Queryable};
    let bottom = |height: f32| {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for i in 0..50 {
            app.session.actions.list.push(Action { name: format!("Action {i}"), steps: vec![], disabled: Default::default(), ..Default::default() });
        }
        let mut h = Harness::builder().with_size(vec2(320.0, height)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        let rect = h.get_by_label("Play selection").rect();
        assert!(rect.bottom() <= height, "footer must fit: {rect:?}");
        rect.top()
    };
    assert!(bottom(700.0) - bottom(300.0) > 350.0, "list uses the additional panel height instead of stopping at 320 px");
    assert!(crate::dock::Group::History.scrolls_itself(1));
}

/// #7646: an action's function key can take Shift and Command, as in Photoshop's Action
/// Options, and only that combination plays it.
#[test]
fn a_function_key_with_shift_and_command_plays_its_action() {
    use egui::{Event, Key, Modifiers};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.session.actions.list.push(Action {
        name: "Action 1".into(),
        steps: vec![("layer.new.layer".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    let key = photocraft_engine::atn_cmds::function_key_name(2, true, true);
    assert_eq!(key, "Cmd+Shift+F2");
    assign_shortcut(&mut app, "Action 1", &key).unwrap();
    let ctx = egui::Context::default();
    let press = |app: &mut PhotocraftApp, modifiers| {
        ctx.begin_pass(egui::RawInput {
            events: vec![
                Event::ModifiersChanged(modifiers),
                Event::Key { key: Key::F2, physical_key: None, pressed: true, repeat: false, modifiers },
                Event::Key { key: Key::F2, physical_key: None, pressed: false, repeat: false, modifiers },
            ],
            ..Default::default()
        });
        crate::shortcuts::handle(app, &ctx);
        ctx.end_pass().textures_delta.clear();
    };
    let layers = |app: &PhotocraftApp| app.session.active().unwrap().doc.layers.len();
    let before = layers(&app);
    press(&mut app, Modifiers::NONE);
    press(&mut app, Modifiers::SHIFT);
    assert_eq!(layers(&app), before, "F2 and Shift+F2 alone don't play it");
    press(&mut app, Modifiers::SHIFT | Modifiers::COMMAND);
    assert_eq!(layers(&app), before + 1, "Cmd+Shift+F2 plays it");
}

#[test]
fn double_clicking_an_action_name_renames_it_in_place() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.session.actions.list.push(Action {
        name: "Action 1".into(),
        steps: vec![("layer.new.layer".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    let layers = app.session.active().unwrap().doc.layers.len();
    // 60 fps steps, so two clicks a frame apart count as a double-click.
    let mut h = Harness::builder().with_size(vec2(320.0, 260.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    let rename = |h: &mut Harness<'_, PhotocraftApp>, text: &str| {
        let name = h.get_by_label("Action 1").rect().left_center() + vec2(52.0, 0.0);
        h.event(egui::Event::PointerMoved(name));
        // Wait out the double-click window of the previous attempt.
        h.run_steps(30);
        for pressed in [true, false, true, false] {
            h.event(egui::Event::PointerButton { pos: name, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.run_steps(3);
        h.event(egui::Event::Text(text.into()));
        h.run_steps(1);
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
    };
    // Whitespace keeps the old name and reports why.
    rename(&mut h, "  ");
    assert_eq!(h.state().session.actions.list[0].name, "Action 1");
    assert!(h.state().ui.status_error);
    rename(&mut h, "Sepia");
    assert_eq!(h.state().session.actions.list[0].name, "Sepia");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers, "renaming must not play the action");
}

#[test]
fn play_button_targets_first_action_if_nothing_is_selected() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    assert_eq!(playback_selection(&app), None);
    app.session.actions.list.push(Action {
        name: "Make Layer".into(),
        steps: vec![("layer.new.layer".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    app.session.actions.list.push(Action {
        name: "Fit".into(),
        steps: vec![("view.fitOnScreen".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    assert_eq!(playback_selection(&app), Some(0));
    app.ui.actions.selected = Some(1);
    assert_eq!(playback_selection(&app), Some(1));
    app.ui.actions.selected = Some(999);
    assert_eq!(playback_selection(&app), Some(0));

    app.ui.actions.selected = None;
    use egui_kittest::{Harness, kittest::Queryable};
    let mut h = Harness::builder().with_size(vec2(400.0, 320.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    let before = h.state().session.active().unwrap().doc.layers.len();
    h.get_by_label("Play selection").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before + 1);
    assert!(!h.state().ui.status_error);
}

#[test]
fn step_check_box_excludes_the_step_from_playback_and_batch() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let steps = ["A", "B", "C"].map(|n| ("layer.new.layer".to_string(), json!({"name": n})));
    app.session.actions.list.push(Action::new("Three", steps.to_vec()));
    app.ui.actions.expanded = vec![true];
    let mut h = Harness::builder().with_size(vec2(320.0, 300.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    let checks = h.query_all_by_role(egui::accesskit::Role::CheckBox).count();
    assert_eq!(checks, 3);
    h.query_all_by_role(egui::accesskit::Role::CheckBox).nth(1).unwrap().click();
    h.run_steps(3);
    let action = &h.state().session.actions.list[0];
    assert_eq!(action.disabled.iter().copied().collect::<Vec<_>>(), [1]);
    assert_eq!(action_steps(action).len(), 2, "Batch and droplets get only the checked steps");
    assert_eq!(step_count(3, action.disabled.len()), "2/3 steps");
    h.get_by_label("Play selection").click();
    h.run_steps(3);
    let names: Vec<_> = h.state().session.active().unwrap().doc.layers.iter().skip(1).map(|l| l.name.clone()).collect();
    assert_eq!(names, ["A", "C"]);
}

#[test]
fn playback_errors_are_visible_in_status_bar() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.session.actions.list.push(Action {
        name: "Bad action".into(),
        steps: vec![("not.a.command".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    let mut h = Harness::builder().with_size(vec2(400.0, 320.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    h.get_by_label("Play selection").click();
    h.run_steps(3);
    assert!(h.state().ui.status_error);
    assert!(h.state().ui.status.contains("not.a.command") || h.state().ui.status.contains("unknown command"));
}

#[test]
fn recording_reveals_new_action_and_latest_step_in_a_long_panel() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":4,"height":4})).unwrap();
    for i in 0..30 {
        app.session.actions.list.push(Action { name: format!("Existing {i}"), steps: vec![], disabled: Default::default(), ..Default::default() });
    }
    let mut h = Harness::builder().with_size(vec2(320.0, 260.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(4);
    begin_recording(h.state_mut(), false);
    h.run_steps(20);
    let rect = h.get_by_label("Action 31").rect();
    assert!(rect.top() >= 0.0 && rect.bottom() <= h.get_by_label("Play selection").rect().top(), "new action visible: {rect:?}");
    for _ in 0..20 {
        h.state_mut().run("view.zoomOut", json!({})).unwrap();
    }
    h.state_mut().run("view.fitOnScreen", json!({})).unwrap();
    h.run_steps(20);
    let rect = h.get_by_label("Fit on Screen").rect();
    assert!(rect.top() >= 0.0 && rect.bottom() <= h.get_by_label("Play selection").rect().top(), "latest step visible: {rect:?}");
}

#[test]
fn drag_steps_and_action_headers_reorders_and_keeps_selection() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.actions.list.push(Action {
        name: "First".into(),
        steps: vec![("view.zoomOut".into(), json!({})), ("view.fitOnScreen".into(), json!({})), ("view.actualPixels".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    app.session.actions.list.push(Action { name: "Second".into(), steps: vec![], disabled: Default::default(), ..Default::default() });
    app.ui.actions.expanded = vec![true, false];
    let mut h = Harness::builder().with_size(vec2(320.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(4);
    fn drag(h: &mut Harness<'_, PhotocraftApp>, from: egui::Pos2, to: egui::Pos2) {
        h.event(egui::Event::PointerMoved(from));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(from + vec2(10.0, 0.0)));
        h.run_steps(2);
        h.event(egui::Event::PointerMoved(to));
        h.run_steps(2);
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(3);
    }
    let from = h.get_by_label("Zoom Out").rect().center();
    let target = h.get_by_label("100%").rect();
    drag(&mut h, from, pos2(target.center().x, target.bottom() - 2.0));
    assert_eq!(
        h.state().session.actions.list[0].steps.iter().map(|st| st.0.as_str()).collect::<Vec<_>>(),
        ["view.fitOnScreen", "view.actualPixels", "view.zoomOut"]
    );
    assert_eq!(h.state().ui.actions.selected_step, Some(2));
    let from = h.get_by_label("Second").rect().center();
    let target = h.get_by_label("First").rect();
    drag(&mut h, from, pos2(target.center().x, target.top() + 2.0));
    assert_eq!(h.state().session.actions.list[0].name, "Second");
    assert_eq!(h.state().ui.actions.expanded, [false, true]);
    assert_eq!(h.state().ui.actions.selected, Some(0));
}

#[test]
fn nested_shell_actions_include_view_steps_and_propagate_failures() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":40,"height":40})).unwrap();
    app.session.actions.list.push(Action {
        name: "Child".into(),
        steps: vec![("layer.new.layer".into(), json!({})), ("view.fitOnScreen".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    app.run("actions.record", json!({"name":"Parent"})).unwrap();
    app.run("actions.play", json!({"action":"Child"})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[1].steps, [("actions.play".into(), json!({"action":"Child"}))]);
    assert_eq!(step_label("actions.play", &json!({"action":"Child"})), "Play Action: Child");
    app.run("view.actualPixels", json!({})).unwrap();
    let r = app.run("actions.play", json!({"action":"Parent"})).unwrap();
    assert!(r.get("failed").is_none(), "{r}");
    assert!(app.ui.views[0].fit_pending);
    app.session.authorize =
        Some(|id, _| if id == "view.fitOnScreen" { Err(photocraft_engine::EngineError::Other("denied nested view".into())) } else { Ok(()) });
    let r = app.run("actions.play", json!({"action":"Parent"})).unwrap();
    assert!(r["failed"]["error"].as_str().unwrap().contains("denied nested view"));
    assert!(app.session.actions.playback_stack.is_empty());
    assert_eq!(app.session.actions.playing, 0);
}

#[test]
fn panel_shows_pending_steps_and_trash_deletes_only_the_clicked_step() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
    begin_recording(&mut app, false);
    app.run("view.fitOnScreen", json!({})).unwrap();
    app.run("view.zoomOut", json!({})).unwrap();
    let mut h = Harness::builder().with_size(vec2(320.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(2);
    assert!(h.state().session.actions.recording.is_some());
    h.get_by_label("Fit on Screen");
    h.get_by_label("Zoom Out").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.actions.selected_step, Some(1));
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert_eq!(visible_steps(h.state(), 0).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["view.fitOnScreen"]);
    assert_eq!(h.state().session.actions.list.len(), 1);
    assert!(h.state().session.actions.recording.is_some());
    assert!(h.state().ui.actions.selected.is_none());
    delete_selection(h.state_mut());
    assert_eq!(h.state().session.actions.list.len(), 1, "repeated trash cannot delete the action");
    h.state_mut().run("view.zoomIn", json!({})).unwrap();
    h.run_steps(2);
    h.get_by_label("Zoom In");
    h.state_mut().run("actions.stop", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(visible_steps(h.state(), 0).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["view.fitOnScreen", "view.zoomIn"]);
    h.get_by_label("Zoom In").click();
    h.run_steps(2);
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert_eq!(visible_steps(h.state(), 0).iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["view.fitOnScreen"]);
}

#[test]
fn view_menu_commands_record_append_and_replay_after_resize() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 160, "height": 100})).unwrap();
    begin_recording(&mut app, false);
    app.run("image.canvasSize", json!({"width": 80, "height": 60})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    begin_recording(&mut app, true);
    crate::menus::invoke(&mut app, &ctx, "view.fitOnScreen", json!({})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list.len(), 1);
    assert_eq!(app.session.actions.list[0].steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["image.canvasSize", "view.fitOnScreen"]);
    assert_eq!(label_of("view.fitOnScreen"), "Fit on Screen");
    // Persisted actions retain the menu step, and playback fits the new document.
    let saved = serde_json::to_string(&app.session.actions.list).unwrap();
    app.session.actions.list = serde_json::from_str(&saved).unwrap();
    app.run("file.new", json!({"width": 200, "height": 140})).unwrap();
    app.run("view.actualPixels", json!({})).unwrap();
    let result = app.run("actions.play", json!({"action": 0})).unwrap();
    assert_eq!(result["ran"], 2);
    assert!(result.get("failed").is_none(), "{result}");
    let idx = app.session.active_index().unwrap();
    assert!(app.ui.views[idx].fit_pending);
    let doc = &app.session.active().unwrap().doc;
    crate::canvas::fit_view(&mut app.ui.views[idx], doc, vec2(100.0, 100.0), 1.0, 1.0);
    assert_eq!(app.ui.views[idx].zoom, 0.75);
    assert_eq!(app.ui.views[idx].center, [40.0, 30.0]);
    assert!(!app.ui.views[idx].fit_pending);
}

#[test]
fn view_action_failure_resume_and_authorization() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.actions.list.push(Action {
        name: "Fit".into(),
        steps: vec![("view.fitOnScreen".into(), json!({})), ("view.actualPixels".into(), json!({}))],
        disabled: Default::default(),
        ..Default::default()
    });
    let result = app.run("actions.play", json!({"action": "Fit"})).unwrap();
    assert_eq!(result["ran"], 0);
    assert_eq!(result["failed"]["step"], 0);
    assert_eq!(app.session.actions.playing, 0);
    app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
    app.session.authorize =
        Some(
            |id, _| {
                if id == "view.fitOnScreen" { Err(photocraft_engine::EngineError::BadParams { cmd: id.into(), msg: "denied view".into() }) } else { Ok(()) }
            },
        );
    let result = app.run("actions.play", json!({"action": 0})).unwrap();
    assert_eq!(result["ran"], 0);
    assert!(result["failed"]["error"].as_str().unwrap().contains("denied view"));
    let result = app.run("actions.play", json!({"action": 0, "from": 1})).unwrap();
    assert_eq!(result["ran"], 1);
    assert!(!app.ui.views[0].fit_pending);
    assert!(app.run("actions.play", json!({"action": 0, "from": -1})).is_err());
    app.session.actions.playing = 1;
    assert!(app.run("actions.play", json!({"action": 0})).is_err());
}

#[test]
fn record_and_replay_on_another_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
    app.sync_views();
    app.run("actions.record", json!({})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let _ = app.run("document.pixel", json!({"x": 1, "y": 1}));
    app.run("actions.stop", json!({})).unwrap();
    let a = &app.session.actions.list[0];
    assert_eq!(a.steps.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), ["layer.new.layer", "select.rect", "edit.fill"]);
    app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
    app.sync_views();
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    assert_eq!(played["ran"], 3);
    assert!(played.get("failed").is_none(), "{played}");
    let d = &app.session.active().unwrap().doc;
    assert_eq!(d.layers.len(), 2);
    assert_eq!(d.layers[1].surface().unwrap().pixel(5, 5), vec![1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn recorded_save_as_and_save_replay_and_write_the_file() {
    let written = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let log = written.clone();
    let services = crate::Services {
        export: Some(Box::new(|doc, path, _| photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))),
        write: Some(Box::new(move |path, _| {
            log.borrow_mut().push(path.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("actions.record", json!({})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "out/result.psd"})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    let steps = &app.session.actions.list[0].steps;
    assert_eq!(steps, &[("layer.new.layer".into(), json!({})), ("file.saveAs".into(), json!({"path": "out/result.psd"})), ("file.save".into(), json!({}))]);
    written.borrow_mut().clear();
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(*written.borrow(), ["out/result.psd", "out/result.psd"]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("out/result.psd"));
}

/// An app with the real importer/exporter and writer whose Save dialogs answer `answers`.
fn disk_app(answers: Vec<Option<crate::file_dialog::FileDialogAnswer>>) -> PhotocraftApp {
    let services = crate::Services {
        import: Some(Box::new(|name, bytes, opts| {
            photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        })),
        export: Some(Box::new(|doc, path, _| photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))),
        write: Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string()))),
        file_dialog: Some(crate::file_dialog::fake(answers).0),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.background_jobs = false;
    app
}

/// Records Invert + File › Save on the file `a`, then plays it on the file `b` (given a
/// new layer first when `layered`). Returns the played result and whether `b` / `a`
/// changed on disk.
fn record_save_on_a_play_on_b(ext: &str, layered: bool) -> (Value, bool, bool) {
    use crate::file_dialog::FileDialogAnswer::SaveTo;
    let dir = std::env::temp_dir().join(format!("photocraft-rec-save-{ext}-{layered}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = |n: &str| dir.join(format!("{n}.{ext}")).to_string_lossy().into_owned();
    let (a, b) = (path("a"), path("b"));
    // The dialog confirms the suggested name, which is the document's own file.
    let mut app = disk_app(vec![Some(SaveTo(a.clone())), Some(SaveTo(a.clone()))]);
    let ctx = egui::Context::default();
    for p in [&a, &b] {
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.save_as(Some(p.clone())).unwrap();
    }
    let (before_a, before_b) = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    app.open_path(&a).unwrap();
    app.run("actions.record", json!({})).unwrap();
    app.run("image.adjustments.invert", json!({})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    app.run("actions.stop", json!({})).unwrap();
    std::fs::write(&a, &before_a).unwrap();
    app.open_path(&b).unwrap();
    if layered {
        app.run("layer.new.layer", json!({})).unwrap();
    }
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    let r = (played, std::fs::read(&b).unwrap() != before_b, std::fs::read(&a).unwrap() != before_a);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

#[test]
fn recorded_save_overwrites_the_layered_file_the_action_is_played_on() {
    let (played, b_changed, a_changed) = record_save_on_a_play_on_b("psd", false);
    assert!(played.get("failed").is_none(), "{played}");
    assert!(b_changed && !a_changed);
}

#[test]
fn recorded_save_overwrites_the_flat_file_the_action_is_played_on() {
    let (played, b_changed, a_changed) = record_save_on_a_play_on_b("png", false);
    assert!(played.get("failed").is_none(), "{played}");
    assert!(b_changed, "Save did not overwrite the file the action was played on");
    assert!(!a_changed, "Save wrote the file the action was recorded on");
}

#[test]
fn recorded_save_never_flattens_layers_over_a_flat_file() {
    // The played-on document has layers its PNG can't keep: the step fails, nothing is written (#3806).
    let (played, b_changed, a_changed) = record_save_on_a_play_on_b("png", true);
    assert!(played.get("failed").is_some(), "{played}");
    assert!(!b_changed && !a_changed);
}

/// Loads a Photoshop set holding Invert + a Save As into `dest` and plays it on a PSD.
/// Returns the played result, whether the PSD changed on disk and the files in `out`.
fn play_imported_save_as(dest: &str) -> (Value, bool, Vec<String>) {
    use photocraft_psd::atn::{AtnAction, AtnEvent, AtnFile, AtnItem};
    use photocraft_psd::descriptor::{Descriptor, UnicodeString, Value as D};
    let dir = std::env::temp_dir().join(format!("photocraft-atn-save-as-{}-{}", dest.len(), std::process::id()));
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let src = dir.join("src.psd").to_string_lossy().into_owned();
    let path = |text: &str| {
        let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let mut b = b"txtu".to_vec();
        b.extend_from_slice(&((12 + units.len() * 2) as u32).to_le_bytes());
        b.extend_from_slice(&(units.len() as u32).to_le_bytes());
        units.iter().for_each(|u| b.extend_from_slice(&u.to_le_bytes()));
        D::Path(b)
    };
    let save = Descriptor::new("null")
        .with("As  ", D::Descriptor(Descriptor::new("PNGF")))
        .with("In  ", path(&out.join(dest).to_string_lossy()))
        .with("DocI", D::Integer(53));
    let item = |code: &[u8; 4], label: &str, d: Option<Descriptor>| AtnItem {
        event: AtnEvent::Code(*code),
        dictionary: label.into(),
        descriptor: d,
        ..Default::default()
    };
    let set = AtnFile {
        name: UnicodeString::new_nul("Export"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Invert and save"),
            items: vec![item(b"Invr", "Invert", None), item(b"save", "Save", Some(save))],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut app = disk_app(vec![]);
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.save_as(Some(src.clone())).unwrap();
    let before = std::fs::read(&src).unwrap();
    app.run("actions.load", json!({"data": photocraft_engine::paint::tile::b64_encode(&photocraft_psd::atn::write(&set).unwrap())})).unwrap();
    app.open_path(&src).unwrap();
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    let mut written: Vec<String> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    written.sort();
    let r = (played, std::fs::read(&src).unwrap() != before, written);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

#[test]
fn imported_save_as_writes_the_recorded_file_not_the_played_one() {
    // #7102: the Save As imported as File › Save, which wrote over src.psd.
    let (played, src_changed, written) = play_imported_save_as("result.png");
    assert!(played.get("failed").is_none(), "{played}");
    assert!(!src_changed, "the Save As wrote over the file the action was played on");
    assert_eq!(written, ["result.png"]);
}

#[test]
fn imported_save_as_into_a_folder_stops_before_writing() {
    // A folder destination isn't mapped yet (#5430): playback stops on it, nothing is saved.
    let (played, src_changed, written) = play_imported_save_as("folder");
    assert!(played.get("failed").is_some(), "{played}");
    assert!(!src_changed);
    assert!(written.is_empty(), "{written:?}");
}

#[test]
fn set_folders_group_actions_and_collapse() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut sharpen = Action::new("Sharpen", vec![("filter.sharpen.sharpen".into(), json!({}))]);
    sharpen.set = Some("Retouch".into());
    let mut soften = Action::new("Soften", vec![("filter.blur.blur".into(), json!({}))]);
    soften.set = Some("Retouch".into());
    app.session.actions.list.push(Action::new("Loose", vec![]));
    app.session.actions.list.push(sharpen);
    app.session.actions.list.push(Action::new("Also loose", vec![]));
    app.session.actions.list.push(soften);
    let mut h = Harness::builder().with_size(vec2(360.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    // The folder row shows the set with its count; its actions sit under it, loose ones first.
    let folder = h.get_by_label("Retouch").rect();
    assert!(h.get_by_label("Loose").rect().top() < folder.top());
    assert!(h.get_by_label("Also loose").rect().top() < folder.top());
    assert!(h.get_by_label("Sharpen").rect().top() > folder.top());
    assert!(h.get_by_label("Soften").rect().top() > h.get_by_label("Sharpen").rect().top());
    assert_eq!(display_order(&[]), Vec::<usize>::new());
    // Clicking the folder collapses it; its actions disappear and the state is remembered.
    h.get_by_label("Retouch").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.actions.collapsed_sets, ["Retouch"]);
    assert!(h.query_by_label("Sharpen").is_none());
    assert!(h.query_by_label("Loose").is_some());
    h.get_by_label("Retouch").click();
    h.run_steps(3);
    assert!(h.state().ui.actions.collapsed_sets.is_empty());
    assert!(h.query_by_label("Sharpen").is_some());
}

#[test]
fn moving_an_action_next_to_another_set_joins_it() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut a = Action::new("A", vec![]);
    a.set = Some("One".into());
    let mut b = Action::new("B", vec![]);
    b.set = Some("Two".into());
    app.session.actions.list.push(a);
    app.session.actions.list.push(b);
    app.session.actions.list.push(Action::new("C", vec![]));
    // C dropped below B: it joins B's set, "Two".
    move_row(&mut app, DragRow { action: 2, step: None }, 2);
    assert_eq!(app.session.actions.list[2].set.as_deref(), Some("Two"));
    // B dropped to the top, above A: the row below decides when there is none above.
    move_row(&mut app, DragRow { action: 1, step: None }, 0);
    assert_eq!((app.session.actions.list[0].name.as_str(), app.session.actions.list[0].set.as_deref()), ("B", Some("One")));
    // C dropped between A (One) and nothing: the row above wins.
    move_row(&mut app, DragRow { action: 2, step: None }, 2);
    assert_eq!((app.session.actions.list[2].name.as_str(), app.session.actions.list[2].set.as_deref()), ("C", Some("One")));
}

#[test]
fn insert_stop_dialog_adds_a_stop_after_the_selected_step() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.session.actions.list.push(Action::new("A", vec![("select.all".into(), json!({})), ("select.inverse".into(), json!({}))]));
    assert!(insert_stop_dialog(&mut app).is_err(), "needs a selected action");
    app.ui.actions.selected = Some(0);
    app.ui.actions.selected_step = Some(0);
    let id = insert_stop_dialog(&mut app).unwrap()["dialog"].as_u64().unwrap();
    let d = app.ui.dialog_mut(id).unwrap();
    assert!(owns_insert_stop(&d.fields));
    assert_eq!(d.fields["at"], 1);
    d.fields.insert("message".into(), json!("Check the mask"));
    d.fields.insert("allowContinue".into(), json!(true));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let steps = &app.session.actions.list[0].steps;
    assert_eq!(steps[1], (photocraft_engine::atn_cmds::STOP_MESSAGE.to_string(), json!({"message": "Check the mask", "allowContinue": true})));
    assert_eq!((app.ui.actions.selected, app.ui.actions.selected_step), (Some(0), Some(1)));
    assert_eq!(step_label(&steps[1].0, &steps[1].1), "Stop: Check the mask");
    // Without a selected step the stop goes last; the panel's menu wiring is exercised by name.
    app.ui.actions.selected_step = None;
    let id = insert_stop_dialog(&mut app).unwrap()["dialog"].as_u64().unwrap();
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["at"], 3);
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert_eq!(app.session.actions.list[0].steps.len(), 3);
}

#[test]
fn dialog_worker_failure_stops_action_before_following_steps() {
    for (background, locked) in [(true, true), (false, true), (true, false)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 8, "height": 6, "background": "#203040"})).unwrap();
        let mut action = Action::new(
            "Worker Result",
            vec![
                ("layer.setProps".into(), json!({"name": "Prefix", "locks": {"pixels": locked}})),
                ("filter.blur.gaussianBlur".into(), json!({"radius": 1.5})),
                ("layer.new.layer".into(), json!({"name": "Suffix"})),
            ],
        );
        action.dialogs.insert(1);
        app.session.actions.list.push(action);
        app.background_jobs = background;
        let paused = play(&mut app, &json!({"action": "Worker Result"})).unwrap();
        let dialog = paused["paused"]["dialog"].as_u64().unwrap();
        let result = crate::dialogs::confirm(&mut app, dialog);
        let suffix = app.session.active().unwrap().doc.walk().iter().any(|(_, _, layer)| layer.name == "Suffix");

        assert_eq!(suffix, !locked, "background={background}, locked={locked}: {result:?}");
        if locked && background {
            assert!(result.as_ref().is_err_and(|e| e.contains("locked")), "failed worker must fail the dialog confirmation: {result:?}");
            assert!(app.ui.status_error, "the action must report the worker failure");
            assert!(app.ui.status.contains("locked"), "{}", app.ui.status);
            assert_eq!(app.session.jobs_with_recent()[0].state, "failed");
        } else if locked {
            assert!(result.is_err(), "the foreground command rejects the locked layer");
        } else {
            assert!(result.is_ok(), "the successful worker lets the action continue: {result:?}");
        }
        assert_eq!(app.session.actions.playing, 0);
        assert!(app.session.actions.playback_stack.is_empty());
    }
}

#[test]
fn a_dialog_toggled_step_pauses_playback_and_resumes_on_ok() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut a = Action::new(
        "Blur",
        vec![("layer.new.layer".into(), json!({})), ("filter.blur.gaussianBlur".into(), json!({"radius": 3})), ("select.all".into(), json!({}))],
    );
    a.dialogs.insert(1);
    app.session.actions.list.push(a);
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    assert_eq!(v["ran"], 1, "{v}");
    assert_eq!(v["paused"]["step"], 1);
    let id = v["paused"]["dialog"].as_u64().unwrap();
    let d = app.ui.dialog_mut(id).unwrap();
    assert_eq!(d.fields["radius"], 3, "the dialog opens on the recorded values");
    assert!(d.fields[RESUME].is_u64(), "the dialog carries only the pause's token: {}", d.fields[RESUME]);
    assert!(app.session.active().unwrap().doc.selection.is_none(), "nothing after the dialog ran yet");
    report_play(&mut app, &v);
    assert!(app.ui.status.contains("dialog"), "{}", app.ui.status);
    // OK runs the blur with the dialog's values and plays the rest.
    app.ui.dialog_mut(id).unwrap().fields.insert("radius".into(), json!(5));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.ui.dialogs.is_empty());
    assert!(app.session.active().unwrap().doc.selection.is_some(), "the step after the dialog ran");
    assert!(app.ui.status.starts_with("Played"), "{}", app.ui.status);
    // Cancel stops the action there.
    app.run("select.deselect", json!({})).unwrap();
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    let id = v["paused"]["dialog"].as_u64().unwrap();
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_none());
    assert!(app.ui.status.contains("stopped"), "{}", app.ui.status);
    // With background jobs (the desktop default) the resumed steps wait for the dialog's
    // filter to land instead of meeting a locked document.
    app.background_jobs = true;
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    let id = v["paused"]["dialog"].as_u64().unwrap();
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_some(), "the step after the background blur ran");
    assert!(app.ui.status.starts_with("Played"), "{}", app.ui.status);
    app.background_jobs = false;
    // Agents play headlessly: no pause, the recorded values run.
    app.automation_input = true;
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    assert_eq!(v["ran"], 3, "{v}");
    assert!(v.get("paused").is_none());
}

/// An 8×6 document, a Child action whose first step (a blur) shows its dialog and whose
/// second step adds a "Suffix" layer, and recording into Owner started.
fn recording_with_dialog_child() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 6, "background": "#203040"})).unwrap();
    let mut child =
        Action::new("Child", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1.5})), ("layer.new.layer".into(), json!({"name": "Suffix"}))]);
    child.dialogs.insert(0);
    app.session.actions.list.push(child);
    app.run("actions.record", json!({"name": "Owner"})).unwrap();
    app
}

fn suffixes(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.walk().iter().filter(|(_, _, l)| l.name == "Suffix").count()
}

fn child_call() -> Vec<(String, Value)> {
    vec![("actions.play".to_string(), json!({"action": "Child"}))]
}

/// Play Child and return the id of its step dialog.
fn pause_child(app: &mut PhotocraftApp) -> u64 {
    let v = play(app, &json!({"action": "Child"})).unwrap();
    v["paused"]["dialog"].as_u64().unwrap()
}

#[test]
fn a_recorded_call_paused_by_a_dialog_records_one_call_and_replays_once() {
    let mut app = recording_with_dialog_child();
    let refusal = crate::i18n::t("Confirm or cancel the open action step dialog first").to_string();

    // Confirm: the dialog command and the resumed rest stay inside the one named call.
    let live = app.session.journal.len();
    let id = pause_child(&mut app);
    assert!(app.session.actions.recording.is_none(), "recording stays suspended while the step dialog is open");
    let names = |app: &PhotocraftApp| app.session.actions.list.iter().map(|a| a.name.clone()).collect::<Vec<_>>();
    let before = names(&app);
    assert_eq!(app.run("actions.record", json!({"name": "Intruder"})), Err(refusal.clone()), "no second recording while the call is suspended");
    assert_eq!(app.run("actions.delete", json!({"action": "Owner"})), Err(refusal.clone()), "the suspended owner can't be deleted");
    assert_eq!(app.run("actions.move", json!({"action": "Owner", "to": 0})), Err(refusal.clone()), "the suspended owner can't be moved");
    assert_eq!(names(&app), before, "the refused commands leave the list alone");
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.session.actions.recording.is_some(), "recording comes back when the call completes");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[1].steps, child_call());
    assert_eq!(suffixes(&app), 1);

    // Replaying the recording repeats the call once.
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    let start = app.session.journal.len();
    app.automation_input = true;
    let v = play(&mut app, &json!({"action": "Owner"})).unwrap();
    app.automation_input = false;
    assert!(v.get("failed").is_none(), "{v}");
    assert_eq!(suffixes(&app), 1, "replay creates the suffix once");
    let edits = |from: usize| -> Vec<(String, Value)> {
        app.session.journal[from..].iter().filter(|(id, _)| !id.starts_with("actions.") && id != "file.new").cloned().collect()
    };
    let expected = vec![("filter.blur.gaussianBlur".to_string(), json!({"radius": 1.5})), ("layer.new.layer".to_string(), json!({"name": "Suffix"}))];
    assert_eq!(edits(start), expected, "replay journals the blur and the suffix once each, in order");
    assert_eq!(edits(live), expected[..].iter().chain(&expected).cloned().collect::<Vec<_>>(), "live then replay");

    // Cancel: the call ends at the dialog and is recorded once.
    app.run("actions.record", json!({"name": "Cancelled"})).unwrap();
    let id = pause_child(&mut app);
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert!(app.session.actions.recording.is_some(), "cancel restores recording");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[2].steps, child_call());
}

#[test]
fn a_failing_step_dialog_command_ends_the_recorded_call_once() {
    let mut app = recording_with_dialog_child();
    let id = pause_child(&mut app);
    assert!(app.session.actions.recording.is_none(), "recording stays suspended while the step dialog is open");
    app.ui.dialog_mut(id).unwrap().fields.insert("radius".into(), json!("not a radius"));
    assert!(crate::dialogs::confirm(&mut app, id).is_err(), "the dialog command fails");
    assert!(app.session.actions.recording.is_some(), "recording comes back when the dialog command fails");
    assert_eq!(suffixes(&app), 0, "the action stops at the failed step");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[1].steps, child_call());
}

#[test]
fn an_abandoned_step_dialog_leaves_no_paused_playback() {
    let mut app = recording_with_dialog_child();
    let id = pause_child(&mut app);
    app.ui.dialog_mut(id).unwrap().fields.insert("radius".into(), json!("not a radius"));
    assert!(crate::dialogs::confirm(&mut app, id).is_err(), "the dialog command fails");
    assert!(app.paused_playbacks.paused.is_empty(), "the failed dialog ends its pause");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[1].steps, child_call(), "the call is recorded once");
    app.run("actions.record", json!({"name": "Next"})).unwrap();
}

#[test]
fn a_failing_resumed_playback_ends_the_recorded_call_once() {
    let mut app = recording_with_dialog_child();
    let id = pause_child(&mut app);
    // The rest of the call can't be found any more: resuming it fails.
    app.session.actions.list[0].name = "Renamed".into();
    assert!(crate::dialogs::confirm(&mut app, id).is_err(), "the resumed playback fails");
    assert!(app.session.actions.recording.is_some(), "recording comes back when the resumed playback fails");
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[1].steps, child_call());
}

#[test]
fn a_suspended_call_is_recorded_into_its_action_after_the_list_changes() {
    let mut app = recording_with_dialog_child();
    let id = pause_child(&mut app);
    // Something else puts an action where Owner was.
    app.session.actions.list.insert(1, Action::new("Bystander", vec![]));
    crate::dialogs::confirm(&mut app, id).unwrap();
    app.run("actions.stop", json!({})).unwrap();
    assert!(app.session.actions.list[1].steps.is_empty(), "the action now at the old index gets nothing");
    assert_eq!(app.session.actions.list[2].name, "Owner");
    assert_eq!(app.session.actions.list[2].steps, child_call());
}

#[test]
fn escape_on_a_step_dialog_ends_the_recorded_call_once() {
    let app = recording_with_dialog_child();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(2);
    pause_child(h.state_mut());
    h.run_steps(4);
    assert!(h.state().session.actions.recording.is_none(), "recording stays suspended while the step dialog is open");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.dialogs.is_empty(), "Esc closes the step dialog");
    assert!(h.state().session.actions.recording.is_some(), "Esc restores recording");
    let app = h.state_mut();
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[1].steps, child_call());
}

#[test]
fn a_step_dialog_keeps_its_continuation_token() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for (name, steps) in [
        ("First", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1}))]),
        ("Second", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1})), ("select.all".into(), json!({}))]),
    ] {
        let mut action = Action::new(name, steps);
        action.dialogs.insert(0);
        app.session.actions.list.push(action);
    }
    let first = play(&mut app, &json!({"action": "First"})).unwrap()["paused"]["dialog"].as_u64().unwrap();
    let second = play(&mut app, &json!({"action": "Second"})).unwrap()["paused"]["dialog"].as_u64().unwrap();
    let first_token = app.ui.dialog_mut(first).unwrap().fields[RESUME].clone();
    let second_token = app.ui.dialog_mut(second).unwrap().fields[RESUME].clone();
    for token in [second_token.clone(), Value::Null, json!("invalid"), json!({"action": "Second", "from": 1})] {
        let (request, _) = crate::control::ControlRequest::new("ui.dialog.set", json!({"dialog": first, "field": RESUME, "value": token}));
        let response = crate::control::handle(&mut app, &egui::Context::default(), &request);
        assert!(matches!(response, crate::control::Outcome::Done(ref v) if v["error"].as_str().is_some_and(|e| e.contains("read-only"))));
        assert_eq!(app.ui.dialog_mut(first).unwrap().fields[RESUME], first_token);
        assert_eq!(app.ui.dialog_mut(second).unwrap().fields[RESUME], second_token);
        assert_eq!(app.paused_playbacks.paused.len(), 2);
    }
    crate::dialogs::confirm(&mut app, first).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_none(), "First must not run Second's remaining step");
    crate::dialogs::confirm(&mut app, second).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_some());
    assert!(app.paused_playbacks.paused.is_empty());
}

fn recording_with_displace_child() -> PhotocraftApp {
    let mut app = recording_with_dialog_child();
    app.background_jobs = false;
    app.session.actions.list[0].steps[0] = ("filter.distort.displace".into(), json!({}));
    app
}

fn assert_displace_call_finished(app: &mut PhotocraftApp, suffix_count: usize) {
    assert!(!app.file_dialog_open());
    assert!(app.ui.dialogs.is_empty());
    assert!(app.paused_playbacks.paused.is_empty());
    assert!(app.session.actions.recording.is_some());
    assert_eq!(suffixes(app), suffix_count);
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[1].steps, child_call(), "record the child call exactly once");
    app.run("actions.record", json!({"name": "Next"})).unwrap();
}

#[test]
fn a_displace_step_restores_recording_when_the_map_picker_fails_or_is_cancelled() {
    use crate::file_dialog::FileDialogAnswer;
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.png").to_string_lossy().into_owned();
    for answer in [
        None,
        Some(FileDialogAnswer::Unavailable("no picker available".into())),
        Some(FileDialogAnswer::Paths(Vec::new())),
        Some(FileDialogAnswer::Contents("map.png".into(), Vec::new())),
        Some(FileDialogAnswer::Paths(vec![missing])),
    ] {
        let mut app = recording_with_displace_child();
        let (show, _) = crate::file_dialog::fake(vec![answer]);
        app.services.file_dialog = Some(show);
        let id = pause_child(&mut app);
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert!(app.file_dialog_open());
        assert!(app.session.actions.recording.is_none());
        app.poll_file_dialog(&egui::Context::default(), None);
        assert_displace_call_finished(&mut app, 0);
    }
}

#[test]
fn a_displace_step_restores_recording_when_the_map_picker_cannot_start() {
    let mut app = recording_with_displace_child();
    let id = pause_child(&mut app);
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_displace_call_finished(&mut app, 0);
}

#[test]
fn a_displace_step_resumes_and_records_its_call_once_after_choosing_a_map() {
    let mut app = recording_with_displace_child();
    let dir = tempfile::tempdir().unwrap();
    let map = dir.path().join("map.png");
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "png", &Default::default()).unwrap().bytes;
    std::fs::write(&map, bytes).unwrap();
    let (show, _) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::Paths(vec![map.to_string_lossy().into_owned()]))]);
    app.services.file_dialog = Some(show);
    let id = pause_child(&mut app);
    crate::dialogs::confirm(&mut app, id).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    assert_displace_call_finished(&mut app, 1);
}

#[test]
fn a_step_dialog_takes_only_the_steps_values() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut a = Action::new("Blur", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 2, "__command": "select.all", "__label": "Other"}))]);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    let d = app.ui.dialog_mut(v["paused"]["dialog"].as_u64().unwrap()).unwrap();
    assert_eq!(d.fields["radius"], 2);
    assert_eq!(d.fields["__command"], "filter.blur.gaussianBlur");
    assert_ne!(d.fields["__label"], "Other");
}

fn deny_blur(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "filter.blur.gaussianBlur" { Err(photocraft_engine::EngineError::Other("blur denied".into())) } else { Ok(()) }
}

#[test]
fn a_step_the_gate_denies_opens_no_dialog() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut a = Action::new("Blur", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1}))]);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    app.session.authorize = Some(deny_blur);
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    app.session.authorize = None;
    assert!(v["failed"]["error"].as_str().is_some_and(|e| e.contains("blur denied")), "{v}");
    assert!(app.ui.dialogs.is_empty(), "no step dialog opened");
    assert!(app.paused_playbacks.paused.is_empty());
}

fn deny_select_all(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "select.all" { Err(photocraft_engine::EngineError::Other("select.all denied".into())) } else { Ok(()) }
}

/// An action an automation client plays (`control::handle` installs the gate, then removes
/// it) pauses at a step dialog; confirming the dialog later, here with synthetic Enter or
/// OK, runs the remaining steps under the same gate.
#[test]
fn a_paused_playback_keeps_its_automation_gate_when_the_dialog_is_confirmed() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut a = Action::new("Seeded", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1})), ("select.all".into(), json!({}))]);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    app.session.authorize = Some(deny_select_all);
    let v = play(&mut app, &json!({"action": "Seeded"})).unwrap();
    app.session.authorize = None;
    let id = v["paused"]["dialog"].as_u64().unwrap();
    app.automation_input = true;
    let _ = crate::dialogs::confirm(&mut app, id);
    app.automation_input = false;
    assert!(app.session.active().unwrap().doc.selection.is_none(), "the denied step did not run");
    assert!(app.ui.status.contains("select.all denied"), "{}", app.ui.status);
}

/// The dialog names only its pause: changing its fields can't resume another action.
#[test]
fn a_step_dialog_resumes_only_the_playback_that_paused_it() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let mut a = Action::new("Blur", vec![("filter.blur.gaussianBlur".into(), json!({"radius": 1}))]);
    a.dialogs.insert(0);
    app.session.actions.list.push(a);
    app.session.actions.list.push(Action::new("Seeded", vec![("select.all".into(), json!({}))]));
    let v = play(&mut app, &json!({"action": "Blur"})).unwrap();
    let id = v["paused"]["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert(RESUME.into(), json!({"action": "Seeded", "from": 0}));
    let r = crate::dialogs::confirm(&mut app, id);
    assert!(app.session.active().unwrap().doc.selection.is_none(), "the other action did not play");
    assert!(r.is_err_and(|e| e.contains("paused action")));
}

#[test]
fn button_mode_plays_the_clicked_action_and_hides_its_steps() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.session.actions.list.push(Action::new("Print", vec![("layer.new.layer".into(), json!({}))]));
    app.session.actions.list[0].color = "blue".into();
    app.ui.actions.expanded = vec![true];
    app.ui.actions.button_mode = true;
    let mut h = Harness::builder().with_size(vec2(420.0, 280.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    assert_eq!(h.query_all_by_role(egui::accesskit::Role::CheckBox).count(), 0, "button mode hides the step list");
    let before = h.state().session.active().unwrap().doc.layers.len();
    h.get_by_label("Print").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before + 1, "a click plays the action");
    h.get_by_label("Button Mode On").click();
    h.run_steps(2);
    assert!(!h.state().ui.actions.button_mode);
    assert!(h.query_by_label("Button Mode").is_some());
    assert_eq!(h.query_all_by_role(egui::accesskit::Role::CheckBox).count(), 1, "the step list returns when Button Mode is turned off");
}

/// An imported Close with Save: Yes plays as Save then Close. A Save Document script event
/// that closes the saved document stops playback, so the Close never reaches another dirty
/// document (#7367).
#[test]
fn save_hook_closing_the_target_stops_an_imported_close_with_save() {
    use photocraft_psd::atn::{self, AtnEvent, AtnItem};
    use photocraft_psd::descriptor::{Descriptor, Id, Value as D};
    use std::{cell::RefCell, rc::Rc};
    // A one-step .atn: Close (`Cls `) with Save: Yes.
    let seed = Action::new("Seed", vec![("layer.new.layer".into(), json!({}))]);
    let (bytes, _) = photocraft_engine::atn_cmds::export("Set", &[&seed], &[], false).unwrap();
    let mut f = atn::parse(&bytes).unwrap();
    f.actions[0].items = vec![AtnItem {
        event: AtnEvent::Code(*b"Cls "),
        dictionary: "Close".into(),
        descriptor: Some(Descriptor::new("null").with("Svng", D::Enumerated { type_id: Id::new("YsN "), value: Id::new("Ys  ") })),
        ..Default::default()
    }];
    let imported = photocraft_engine::atn_cmds::import(&atn::write(&f).unwrap()).unwrap();
    let close_with_save = imported.actions[0].clone();
    assert_eq!(close_with_save.steps, [("file.save".into(), json!({})), ("file.close".into(), json!({}))]);
    for hook in ["none", "activate", "close"] {
        let writes = Rc::new(RefCell::new(Vec::<String>::new()));
        let w = writes.clone();
        let services = crate::Services {
            export: Some(Box::new(|doc, _, _| Ok((doc.name.clone().into_bytes(), Vec::new())))),
            write: Some(Box::new(move |path, _| {
                w.borrow_mut().push(path.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        // B is dirty with an unsaved KeepB layer; A is active and has its own file.
        for name in ["B", "A"] {
            app.run("file.new", json!({"name": name, "width": 2, "height": 2})).unwrap();
        }
        assert!(app.session.set_active(0));
        app.run("layer.new.layer", json!({"name": "KeepB"})).unwrap();
        assert!(app.session.set_active(1));
        app.session.active_mut().unwrap().path = Some("A.psd".into());
        app.session.actions.list.push(close_with_save.clone());
        let steps = match hook {
            "activate" => json!([["document.activate", {"document": 0}]]),
            "close" => json!([["file.close", {}]]),
            _ => Value::Null,
        };
        if !steps.is_null() {
            app.run("file.scripts.scriptEventsManager", json!({"add": {"event": "saveDocument", "steps": steps}})).unwrap();
        }
        let r = app.run("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(*writes.borrow(), ["A.psd"], "hook={hook}: A is saved once");
        let names: Vec<Vec<String>> = app.session.documents().iter().map(|d| d.doc.layers.iter().map(|l| l.name.clone()).collect()).collect();
        assert_eq!(app.session.documents().len(), 1, "hook={hook}: only A closes; result={r}");
        assert_eq!(app.session.active().unwrap().doc.name, "B", "hook={hook}");
        assert_eq!(names, [["Background", "KeepB"]], "hook={hook}: B stays open with its unsaved layer");
        if hook == "close" {
            assert_eq!(r["failed"]["step"], 0, "{r}");
            assert_eq!(r["failed"]["id"], "file.save", "{r}");
            assert!(r["failed"]["error"].as_str().unwrap().contains("closed during the save"), "{r}");
        } else {
            assert!(r.get("failed").is_none(), "hook={hook}: {r}");
        }
        assert_eq!(app.session.actions.playing, 0);
        assert!(app.session.actions.playback_stack.is_empty());
    }
}
