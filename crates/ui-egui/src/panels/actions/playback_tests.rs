//! #5212: Playback Options. Step by Step and Pause For play the whole action from one Play, with a
//! frame (and the wait) between steps, in one history state when Create a single history state is
//! on; Esc and the Stop button end them keeping the steps applied. Measured in the reference
//! application 2026-11-05.
use super::*;
use web_time::{Duration, Instant};

fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.walk().iter().map(|(_, _, l)| l.name.clone()).filter(|n| n != "Background").collect()
}

/// `Three` makes layers A, B and C.
fn app_with(performance: &str, seconds: u8, single: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    app.run("actions.setPlayback", json!({"performance": performance, "seconds": seconds, "singleState": single})).unwrap();
    let steps = ["A", "B", "C"].map(|n| ("layer.new.layer".to_string(), json!({"name": n}))).to_vec();
    app.session.actions.list.push(Action::new("Three", steps));
    app
}

/// One frame of the app's loop as far as pacing goes, with `events` as its input.
fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { events, ..Default::default() };
    ctx.run_ui(raw, |ui| playback::tick(app, ui.ctx())).drop_without_applying_deltas();
}

/// Let the wait run out, then one frame.
fn next_step(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if let Some(t) = app.paused_playbacks.timer.as_mut() {
        t.due = Instant::now();
    }
    frame(app, ctx, Vec::new());
}

fn esc() -> egui::Event {
    egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
}

fn assert_idle(app: &PhotocraftApp) {
    assert_eq!(app.session.actions.playing, 0);
    assert!(app.session.actions.playback_stack.is_empty());
    assert!(app.paused_playbacks.paused.is_empty());
    assert!(app.paused_playbacks.timer.is_none());
}

/// No continuation keeps the history policy leased once a playback ended.
fn assert_released(app: &mut PhotocraftApp) {
    let single = app.session.prefs().actions.create_single_history_state;
    app.run("actions.setHistory", json!({"singleState": single})).unwrap();
}

#[test]
fn step_by_step_plays_every_step_from_one_play_with_a_frame_between() {
    for single in [true, false] {
        let mut app = app_with("step", 1, single);
        let ctx = egui::Context::default();
        let past = app.session.active().unwrap().history.past_len();
        let v = play(&mut app, &json!({"action": "Three"})).unwrap();
        assert_eq!(v["ran"], 1, "{v}");
        assert_eq!(names(&app), ["A"], "the first step is shown before the next runs");
        let due = app.paused_playbacks.timer.unwrap().due.saturating_duration_since(Instant::now());
        assert!(due > Duration::from_millis(500) && due <= playback::STEP_GAP, "{due:?}");
        frame(&mut app, &ctx, Vec::new());
        assert_eq!(names(&app), ["A"], "nothing runs before the wait is over");
        next_step(&mut app, &ctx);
        assert_eq!(names(&app), ["A", "B"]);
        next_step(&mut app, &ctx);
        assert_eq!(names(&app), ["A", "B", "C"]);
        assert!(app.paused_playbacks.timer.is_none(), "no wait after the last step");
        assert_idle(&app);
        assert_released(&mut app);
        assert_eq!(app.ui.status, "Played 3 steps");
        let st = app.session.active().unwrap();
        if single {
            assert_eq!(st.history.past_len(), past + 1, "one history state for the whole play");
            assert_eq!(st.history.undo_label(), Some("Action: Three"));
            assert!(app.session.undo());
            assert!(names(&app).is_empty(), "one undo takes back every step");
        } else {
            assert_eq!(st.history.past_len(), past + 3, "one state per step with the option off");
        }
    }
}

#[test]
fn pause_for_waits_its_seconds_after_each_step_but_the_last() {
    let mut app = app_with("pause", 5, true);
    let ctx = egui::Context::default();
    play(&mut app, &json!({"action": "Three"})).unwrap();
    assert_eq!(names(&app), ["A"], "no wait before the first step");
    let due = app.paused_playbacks.timer.unwrap().due.saturating_duration_since(Instant::now());
    assert!(due > Duration::from_millis(5500) && due <= playback::STEP_GAP + Duration::from_secs(5), "{due:?}");
    frame(&mut app, &ctx, Vec::new());
    assert_eq!(names(&app), ["A"]);
    next_step(&mut app, &ctx);
    assert_eq!(names(&app), ["A", "B"]);
    assert!(app.paused_playbacks.timer.is_some(), "waits again between B and C");
    next_step(&mut app, &ctx);
    assert_eq!(names(&app), ["A", "B", "C"]);
    assert_idle(&app);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Action: Three"));
}

#[test]
fn accelerated_and_agents_play_at_once() {
    let mut app = app_with("accelerated", 1, true);
    assert_eq!(play(&mut app, &json!({"action": "Three"})).unwrap()["ran"], 3);
    assert_idle(&app);
    let mut app = app_with("pause", 60, true);
    app.automation_input = true;
    assert_eq!(play(&mut app, &json!({"action": "Three"})).unwrap()["ran"], 3, "agents never wait");
    assert_idle(&app);
}

#[test]
fn esc_or_stop_ends_the_playback_and_keeps_the_steps_applied() {
    for how in ["esc", "stop button", "list edited", "new play"] {
        let mut app = app_with("pause", 3, true);
        let ctx = egui::Context::default();
        let past = app.session.active().unwrap().history.past_len();
        play(&mut app, &json!({"action": "Three"})).unwrap();
        next_step(&mut app, &ctx);
        assert_eq!(names(&app), ["A", "B"]);
        match how {
            "esc" => frame(&mut app, &ctx, vec![esc()]),
            "stop button" => assert!(playback::stop(&mut app)),
            "list edited" => {
                app.run("actions.newSet", json!({"name": "Other"})).unwrap();
                frame(&mut app, &ctx, Vec::new());
            }
            _ => {
                app.session.actions.list.push(Action::new("One", vec![("layer.new.layer".into(), json!({"name": "D"}))]));
                app.run("actions.setPlayback", json!({"performance": "accelerated"})).unwrap();
                play(&mut app, &json!({"action": "One"})).unwrap();
            }
        }
        next_step(&mut app, &ctx);
        let expected: &[&str] = if how == "new play" { &["A", "B", "D"] } else { &["A", "B"] };
        assert_eq!(names(&app), expected, "{how}: C never runs");
        assert_idle(&app);
        assert_released(&mut app);
        let st = app.session.active().unwrap();
        let added = if how == "new play" { 2 } else { 1 };
        assert_eq!(st.history.past_len(), past + added, "{how}: the applied steps stay one history state");
        if how != "new play" {
            assert_eq!(app.ui.status, "Action playback stopped", "{how}");
        }
    }
}

#[test]
fn an_edit_between_steps_ends_the_playback_without_joining_it() {
    let mut app = app_with("step", 1, true);
    let ctx = egui::Context::default();
    play(&mut app, &json!({"action": "Three"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Mine"})).unwrap();
    next_step(&mut app, &ctx);
    assert_eq!(names(&app), ["A", "Mine"], "the action doesn't go on over a document someone changed");
    assert!(app.ui.status_error, "{}", app.ui.status);
    assert_idle(&app);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("New Layer"));
}

/// A called action is paced too, and its caller goes on after it in the same playback.
#[test]
fn nested_and_set_playback_stay_one_play() {
    let mut app = app_with("step", 1, true);
    let ctx = egui::Context::default();
    app.session.actions.list.push(Action::new(
        "Caller",
        vec![
            ("layer.new.layer".into(), json!({"name": "Before"})),
            ("actions.play".into(), json!({"action": "Three"})),
            ("layer.new.layer".into(), json!({"name": "After"})),
        ],
    ));
    let past = app.session.active().unwrap().history.past_len();
    play_targets(&mut app, &[1], 0).unwrap();
    // One step per frame, also into and out of the called action.
    let mut seen = vec![names(&app)];
    while app.paused_playbacks.timer.is_some() {
        next_step(&mut app, &ctx);
        seen.push(names(&app));
    }
    assert_eq!(seen.iter().map(Vec::len).collect::<Vec<_>>(), [1, 2, 3, 4, 5], "{seen:?}");
    assert_eq!(names(&app), ["Before", "A", "B", "C", "After"]);
    assert_idle(&app);
    assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Action: Caller"));
    // Counted as Accelerated counts it: the Play Action step is one step of its caller.
    assert_eq!(app.ui.status, "Played 3 steps");
    let mut fast = app_with("accelerated", 1, true);
    fast.session.actions.list.push(app.session.actions.list[1].clone());
    let v = play_targets(&mut fast, &[1], 0).unwrap().unwrap();
    assert_eq!(v["ran"], 3, "{v}");
}

#[test]
fn the_panel_menu_opens_playback_options_and_ok_saves_them() {
    let mut app = app_with("accelerated", 1, true);
    let opened = playback::open_dialog(&mut app).unwrap();
    let id = opened["dialog"].as_u64().unwrap();
    let d = app.ui.dialog_mut(id).unwrap();
    assert_eq!(crate::dialogs::title(d), "Playback Options");
    assert_eq!(d.fields["performance"], "accelerated");
    assert_eq!(d.fields["seconds"], 1);
    assert_eq!(d.fields["singleState"], true);
    d.fields.insert("performance".into(), json!("pause"));
    d.fields.insert("seconds".into(), json!(75));
    d.fields.insert("singleState".into(), json!(false));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let a = &app.session.prefs().actions;
    assert_eq!(a.performance, photocraft_engine::prefs::PlaybackPerformance::Pause);
    assert_eq!(a.pause_seconds, 60, "a number past the range takes the closest one");
    assert!(!a.create_single_history_state);
    // Cancel keeps them.
    let id = playback::open_dialog(&mut app).unwrap()["dialog"].as_u64().unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("performance".into(), json!("step"));
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert_eq!(app.session.prefs().actions.performance, photocraft_engine::prefs::PlaybackPerformance::Pause);
}

#[test]
fn the_dialog_shows_the_reference_layout() {
    use egui_kittest::{Harness, kittest::Queryable};
    let app = app_with("step", 9, true);
    let mut h = Harness::builder().with_size(egui::vec2(520.0, 420.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    playback::open_dialog(h.state_mut()).unwrap();
    h.run_steps(3);
    for label in ["Performance", "Accelerated", "Step by Step", "Pause For:", "seconds", "History", "Create a single history state", "OK", "Cancel"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    h.get_by_label("Pause For:").click();
    h.run_steps(2);
    let id = h.state().ui.dialogs[0].id;
    assert_eq!(h.state_mut().ui.dialog_mut(id).unwrap().fields["performance"], "pause");
    assert_eq!(h.state_mut().ui.dialog_mut(id).unwrap().fields["seconds"], 9, "Pause For shows the kept seconds");
    h.get_by_label("Create a single history state").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(2);
    let a = &h.state().session.prefs().actions;
    assert_eq!((a.performance, a.pause_seconds, a.create_single_history_state), (photocraft_engine::prefs::PlaybackPerformance::Pause, 9, false));
}

#[test]
fn a_paced_play_clears_an_earlier_playback_error() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = app_with("step", 1, true);
    app.ui.status = "Step 2 (layer.select) failed: no such layer".into();
    app.ui.status_error = true;
    let mut h = Harness::builder().with_size(egui::vec2(400.0, 320.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    h.get_by_label("Play selection").click();
    h.run_steps(3);
    assert_eq!(names(h.state()), ["A"], "the play waits after its first step");
    assert!(h.state().paused_playbacks.timer.is_some());
    assert!(!h.state().ui.status_error, "the old failure is gone: {}", h.state().ui.status);
    assert!(h.state().ui.status.is_empty(), "{}", h.state().ui.status);
    // A status that isn't an error stays (a step's own message, say).
    let mut app = app_with("step", 1, true);
    app.ui.status = "Saved".into();
    play_targets(&mut app, &[0], 0).map(|v| report_play(&mut app, &v.unwrap())).unwrap();
    assert_eq!(app.ui.status, "Saved");
}

#[test]
fn playback_options_ok_refuses_values_automation_set_that_the_dialog_cannot_show() {
    use photocraft_engine::prefs::PlaybackPerformance;
    let control = |app: &mut PhotocraftApp, method: &str, params: Value| {
        let (request, _) = crate::control::ControlRequest::new(method, params);
        match crate::control::handle(app, &egui::Context::default(), &request) {
            crate::control::Outcome::Done(v) => v,
            _ => panic!("{method} answers at once"),
        }
    };
    for (field, value) in
        [("performance", Value::Null), ("performance", json!("fast")), ("performance", json!(2)), ("singleState", Value::Null), ("singleState", json!("yes"))]
    {
        let mut app = app_with("step", 4, false);
        let id = playback::open_dialog(&mut app).unwrap()["dialog"].as_u64().unwrap();
        assert_eq!(control(&mut app, "ui.dialog.set", json!({"dialog": id, "field": field, "value": value}))["ok"], true);
        let r = control(&mut app, "ui.dialog.confirm", json!({"dialog": id}));
        assert!(r["error"].is_string(), "{field} = {value}: {r}");
        let a = &app.session.prefs().actions;
        assert_eq!(
            (a.performance, a.pause_seconds, a.create_single_history_state),
            (PlaybackPerformance::StepByStep, 4, false),
            "{field} = {value} saves nothing"
        );
    }
    // Valid values through the same path are saved.
    let mut app = app_with("step", 4, false);
    let id = playback::open_dialog(&mut app).unwrap()["dialog"].as_u64().unwrap();
    control(&mut app, "ui.dialog.set", json!({"dialog": id, "field": "performance", "value": "accelerated"}));
    control(&mut app, "ui.dialog.set", json!({"dialog": id, "field": "singleState", "value": true}));
    assert_eq!(control(&mut app, "ui.dialog.confirm", json!({"dialog": id}))["ok"], true);
    let a = &app.session.prefs().actions;
    assert_eq!((a.performance, a.create_single_history_state), (PlaybackPerformance::Accelerated, true));
}
