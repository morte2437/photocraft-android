//! #8386: a Stop step with its dialog toggle on shows its message with Continue (when allowed)
//! and Stop; with the toggle off it ends the playback without a word; a pause inside a called
//! action continues into its callers. Behaviour measured in the reference application
//! (2026-11-05).
use super::*;
use photocraft_engine::atn_cmds::STOP_MESSAGE;

const MESSAGE: &str = "Check the mask before continuing";

fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.walk().iter().map(|(_, _, l)| l.name.clone()).filter(|n| n != "Background").collect()
}

fn count(app: &PhotocraftApp, name: &str) -> usize {
    names(app).iter().filter(|n| *n == name).count()
}

/// `Stop Test` = Prefix, Stop, Suffix, saved to `.atn` and loaded back, so the step and its dialog
/// toggle come the way a recorded file brings them.
fn stop_app(allow: bool, with_dialog: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    let mut action = Action::new(
        "Stop Test",
        vec![
            ("layer.new.layer".into(), json!({"name": "Prefix"})),
            (STOP_MESSAGE.into(), json!({"message": MESSAGE, "allowContinue": allow})),
            ("layer.new.layer".into(), json!({"name": "Suffix"})),
        ],
    );
    if with_dialog {
        action.dialogs.insert(1);
    }
    let (bytes, _) = photocraft_engine::atn_cmds::export("Stops", &[&action], &[], false).unwrap();
    app.run("actions.load", json!({"data": photocraft_engine::paint::tile::b64_encode(&bytes)})).unwrap();
    assert_eq!(app.session.actions.list[0].dialogs.contains(&1), with_dialog);
    app
}

/// Grandparent and Parent each make a prefix layer, call the next one in, then make a suffix.
fn nested_app(allow: bool) -> PhotocraftApp {
    let mut app = stop_app(allow, true);
    for (name, child) in [("Parent", "Stop Test"), ("Grandparent", "Parent")] {
        app.session.actions.list.push(Action::new(
            name,
            vec![
                ("layer.new.layer".into(), json!({"name": format!("{name} prefix")})),
                ("actions.play".into(), json!({"action": child})),
                ("layer.new.layer".into(), json!({"name": format!("{name} suffix")})),
            ],
        ));
    }
    app
}

fn assert_idle(app: &PhotocraftApp) {
    assert_eq!(app.session.actions.playing, 0);
    assert!(app.session.actions.playback_stack.is_empty());
    assert!(app.paused_playbacks.paused.is_empty());
    assert!(app.ui.dialogs.is_empty());
}

#[test]
fn a_stop_with_its_dialog_shows_the_message_and_continue_or_stop_decides() {
    for allow in [true, false] {
        for confirm in [true, false] {
            let mut app = stop_app(allow, true);
            let v = play(&mut app, &json!({"action": "Stop Test"})).unwrap();
            assert_eq!(v["ran"], 1, "{v}");
            assert_eq!(v["paused"]["step"], 1, "{v}");
            let id = v["paused"]["dialog"].as_u64().unwrap();
            let d = app.ui.dialog_mut(id).unwrap();
            assert!(owns_stop_message(&d.fields));
            assert_eq!(d.fields["message"], MESSAGE);
            assert_eq!(d.fields["allowContinue"], allow);
            assert_eq!(crate::dialogs::title(d), "Message");
            assert_eq!(names(&app), ["Prefix"], "the steps before the Stop stay applied");
            if confirm {
                crate::dialogs::confirm(&mut app, id).unwrap();
            } else {
                crate::dialogs::cancel(&mut app, id).unwrap();
            }
            let continued = allow && confirm;
            assert_eq!(names(&app), if continued { vec!["Prefix", "Suffix"] } else { vec!["Prefix"] }, "allow {allow}, confirm {confirm}");
            assert!(!app.ui.status_error, "{}", app.ui.status);
            assert_idle(&app);
            // One history state for the whole play, Continue or not.
            assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Action: Stop Test"));
            assert!(app.session.undo());
            assert!(names(&app).is_empty());
        }
    }
}

#[test]
fn automation_cannot_turn_a_hard_stop_into_continue() {
    let mut app = stop_app(false, true);
    let v = play(&mut app, &json!({"action": "Stop Test"})).unwrap();
    let id = v["paused"]["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("allowContinue".into(), json!(true));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(names(&app), ["Prefix"], "the step's own Allow Continue decides");
    assert_idle(&app);
}

#[test]
fn a_stop_with_its_dialog_off_ends_the_playback_silently() {
    for allow in [true, false] {
        let mut app = stop_app(allow, false);
        let v = play(&mut app, &json!({"action": "Stop Test"})).unwrap();
        assert!(v.get("failed").is_none(), "no error: {v}");
        assert_eq!(v["stopped"]["step"], 1, "{v}");
        assert_eq!(names(&app), ["Prefix"]);
        report_play(&mut app, &v);
        assert!(!app.ui.status_error);
        assert_eq!(app.ui.status, "Action playback stopped");
        assert_idle(&app);
    }
}

#[test]
fn agents_keep_the_engine_stop_semantics() {
    for allow in [true, false] {
        for with_dialog in [true, false] {
            let mut app = stop_app(allow, with_dialog);
            app.automation_input = true;
            let v = play(&mut app, &json!({"action": "Stop Test"})).unwrap();
            assert!(v.get("paused").is_none() && v.get("stopped").is_none(), "{v}");
            assert_eq!(v.get("failed").is_none(), allow, "{v}");
            assert_eq!(count(&app, "Suffix"), usize::from(allow));
            assert_idle(&app);
        }
    }
}

#[test]
fn the_stop_dialog_shows_its_buttons_and_enter_answers_the_default_only_once_shown() {
    use egui_kittest::{Harness, kittest::Queryable};
    for (allow, button) in [(true, Some("Continue")), (true, Some("Stop")), (false, None)] {
        let app = stop_app(allow, true);
        let mut h = Harness::builder().with_size(vec2(900.0, 600.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        play(h.state_mut(), &json!({"action": "Stop Test"})).unwrap();
        // The Enter that started the play arrives with the dialog's first frame: it answers nothing.
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert_eq!(h.state().ui.dialogs.len(), 1, "allow {allow}");
        assert!(h.query_by_label(MESSAGE).is_some());
        assert_eq!(h.query_by_role_and_label(egui::accesskit::Role::Button, "Continue").is_some(), allow);
        assert!(h.query_by_role_and_label(egui::accesskit::Role::Button, "Stop").is_some());
        match button {
            Some(label) => h.get_by_role_and_label(egui::accesskit::Role::Button, label).click(),
            None => h.key_press(egui::Key::Enter),
        }
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty(), "allow {allow}, {button:?}");
        let continued = button == Some("Continue");
        assert_eq!(count(h.state(), "Suffix"), usize::from(continued));
    }
}

#[test]
fn a_stop_restores_a_suspended_recording_once_for_every_exit() {
    for allow in [true, false] {
        for confirm in [true, false] {
            let mut app = stop_app(allow, true);
            app.run("actions.record", json!({"name": "Owner"})).unwrap();
            let v = play(&mut app, &json!({"action": "Stop Test"})).unwrap();
            let id = v["paused"]["dialog"].as_u64().unwrap();
            assert!(app.session.actions.recording.is_none());
            if confirm {
                crate::dialogs::confirm(&mut app, id).unwrap();
            } else {
                crate::dialogs::cancel(&mut app, id).unwrap();
            }
            assert!(app.session.actions.recording.is_some());
            app.run("actions.stop", json!({})).unwrap();
            let owner = app.session.actions.list.iter().find(|a| a.name == "Owner").unwrap();
            assert_eq!(owner.steps, vec![("actions.play".into(), json!({"action": "Stop Test"}))]);
            assert_idle(&app);
        }
    }
}

#[test]
fn continue_inside_a_called_action_finishes_every_caller_once() {
    let mut app = nested_app(true);
    // A second pause in Parent, after the call, keeps Grandparent's continuation too.
    let parent = app.session.actions.list.iter_mut().find(|a| a.name == "Parent").unwrap();
    parent.steps.insert(2, (STOP_MESSAGE.into(), json!({"message": "Parent pause", "allowContinue": true})));
    parent.dialogs.insert(2);
    let history = app.session.active().unwrap().history.past_len();
    app.run("actions.record", json!({"name": "Owner"})).unwrap();
    let v = play(&mut app, &json!({"action": "Grandparent"})).unwrap();
    let id = v["paused"]["dialog"].as_u64().unwrap();
    assert_eq!(names(&app), ["Grandparent prefix", "Parent prefix", "Prefix"]);
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(count(&app, "Suffix"), 1, "the child's suffix runs once");
    assert_eq!(app.ui.dialogs.len(), 1, "Parent goes on into its own Stop");
    assert_eq!(app.ui.dialogs[0].fields["message"], "Parent pause");
    assert!(app.session.actions.recording.is_none(), "recording stays suspended through both pauses");
    let id = app.ui.dialogs[0].id;
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_idle(&app);
    let edits: Vec<_> = app.session.journal.iter().filter(|(id, _)| id == "layer.new.layer").map(|(_, p)| p["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(edits, ["Grandparent prefix", "Parent prefix", "Prefix", "Suffix", "Parent suffix", "Grandparent suffix"]);
    // One history state for the whole call, named after the action that was played.
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), history + 1);
    assert_eq!(st.history.undo_label(), Some("Action: Grandparent"));
    assert!(app.session.actions.recording.is_some());
    app.run("actions.stop", json!({})).unwrap();
    let owner = app.session.actions.list.iter().find(|a| a.name == "Owner").unwrap();
    assert_eq!(owner.steps, vec![("actions.play".into(), json!({"action": "Grandparent"}))]);
}

#[test]
fn stop_a_hard_stop_or_a_failing_child_ends_every_caller() {
    for (allow, confirm, fail) in [(true, false, false), (false, true, false), (true, true, true)] {
        let mut app = nested_app(allow);
        if fail {
            app.session.actions.list[0].steps[2] = ("layer.select".into(), json!({"layer": 999_999}));
        }
        let v = play(&mut app, &json!({"action": "Grandparent"})).unwrap();
        let id = v["paused"]["dialog"].as_u64().unwrap();
        if confirm {
            let _ = crate::dialogs::confirm(&mut app, id);
        } else {
            crate::dialogs::cancel(&mut app, id).unwrap();
        }
        assert!(!names(&app).iter().any(|n| n.ends_with("uffix")), "{allow} {confirm} {fail}: {:?}", names(&app));
        assert_eq!(app.ui.status_error, fail, "{}", app.ui.status);
        assert_idle(&app);
    }
}

#[test]
fn a_silent_stop_inside_a_called_action_ends_its_callers() {
    let mut app = nested_app(true);
    app.session.actions.list[0].dialogs.clear();
    let v = play(&mut app, &json!({"action": "Grandparent"})).unwrap();
    assert!(v.get("failed").is_none(), "{v}");
    assert!(v["stopped"].is_object(), "{v}");
    assert_eq!(names(&app), ["Grandparent prefix", "Parent prefix", "Prefix"]);
    assert_idle(&app);
}

#[test]
fn a_resumed_called_action_still_refuses_to_call_its_paused_callers() {
    let mut app = nested_app(true);
    app.session.actions.list[0].steps[2] = ("actions.play".into(), json!({"action": "Grandparent"}));
    let v = play(&mut app, &json!({"action": "Grandparent"})).unwrap();
    let id = v["paused"]["dialog"].as_u64().unwrap();
    let _ = crate::dialogs::confirm(&mut app, id);
    assert!(app.ui.status_error);
    assert!(app.ui.status.contains("recursive"), "{}", app.ui.status);
    assert_eq!(count(&app, "Grandparent prefix"), 1, "no caller's prefix runs again");
    assert!(!names(&app).iter().any(|n| n.ends_with("suffix")));
    assert_idle(&app);
}

#[test]
fn a_filter_dialog_inside_a_called_action_continues_its_callers_after_ok() {
    let mut app = nested_app(true);
    app.session.actions.list[0].steps[1] = ("filter.blur.gaussianBlur".into(), json!({"radius": 1.5}));
    app.background_jobs = true;
    let v = play(&mut app, &json!({"action": "Grandparent"})).unwrap();
    let id = v["paused"]["dialog"].as_u64().unwrap();
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(names(&app), ["Grandparent prefix", "Parent prefix", "Prefix", "Suffix", "Parent suffix", "Grandparent suffix"]);
    assert!(app.ui.status.starts_with("Played"), "{}", app.ui.status);
    assert_idle(&app);
}

#[test]
fn a_caller_chain_is_bounded() {
    let mut app = stop_app(true, true);
    let v = play(&mut app, &json!({"action": "Stop Test"})).unwrap();
    let dialog = v["paused"]["dialog"].clone();
    let token = paused_token(&mut app, &dialog).unwrap();
    app.paused_playbacks.paused.get_mut(&token).unwrap().callers = vec![("Missing".into(), 1); MAX_CALLERS];
    let r = called(&mut app, "One more", 0, "actions.play", &json!({"paused": v["paused"].clone()}), None);
    assert!(matches!(r, Called::Failed(_)), "a caller past the bound fails the call");
    // A caller that no longer exists ends the resumed playback with an error, nothing else runs.
    app.paused_playbacks.paused.get_mut(&token).unwrap().callers = vec![("Missing".into(), 1)];
    let id = dialog.as_u64().unwrap();
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert_eq!(count(&app, "Suffix"), 0);
    assert_idle(&app);
}
