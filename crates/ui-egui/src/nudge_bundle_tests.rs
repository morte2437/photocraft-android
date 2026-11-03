//! #2259: arrow-key nudges through the whole app (panels, shortcuts, the real frame loop and its
//! clock) are one History state per run, and Undo takes the run back.

use egui_kittest::Harness;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.ui.tool = Tool::Move;
        app
    });
    h.run_steps(6);
    h
}

fn bounds(h: &Harness<'_, PhotocraftApp>) -> Rect {
    let st = h.state().session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds()
}

fn steps(h: &Harness<'_, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

fn nudge(h: &mut Harness<'_, PhotocraftApp>, key: egui::Key, frames: usize) {
    h.key_press(key);
    h.run_steps(frames);
}

#[test]
fn tapping_an_arrow_key_ten_times_is_one_history_step() {
    let mut h = harness();
    let (start, n0) = (bounds(&h), steps(&h));
    for _ in 0..10 {
        nudge(&mut h, egui::Key::ArrowRight, 1);
    }
    assert_eq!(bounds(&h).x0, start.x0 + 10);
    assert_eq!(steps(&h), n0 + 1, "ten taps, one state");
    let entries = h.state().session.active().unwrap().history.entries();
    assert_eq!(entries.last().map(String::as_str), Some("Move"));
    assert_eq!(entries.iter().filter(|e| *e == "Move").count(), 1);
    h.state_mut().session.undo();
    assert_eq!(bounds(&h), start, "one Undo takes all ten pixels back");
}

#[test]
fn a_held_key_is_one_history_step() {
    let mut h = harness();
    let (start, n0) = (bounds(&h), steps(&h));
    nudge(&mut h, egui::Key::ArrowDown, 1);
    // The key stays down: the OS repeats it after a pause of about half a second.
    h.run_steps(2);
    for _ in 0..15 {
        h.event(egui::Event::Key { key: egui::Key::ArrowDown, physical_key: None, pressed: true, repeat: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    h.event(egui::Event::Key { key: egui::Key::ArrowDown, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    assert_eq!(bounds(&h).y0, start.y0 + 16);
    assert_eq!(steps(&h), n0 + 1);
    h.state_mut().session.undo();
    assert_eq!(bounds(&h), start);
}

#[test]
fn a_pause_starts_a_new_step_and_the_preference_sets_how_long() {
    let mut h = harness();
    let n0 = steps(&h);
    nudge(&mut h, egui::Key::ArrowRight, 1);
    nudge(&mut h, egui::Key::ArrowRight, 1);
    // The test clock runs a quarter of a second per frame: eight frames is two seconds of quiet.
    h.run_steps(8);
    nudge(&mut h, egui::Key::ArrowRight, 1);
    assert_eq!(steps(&h), n0 + 2, "two seconds of quiet end the run");
    h.state_mut().run("prefs.set", json!({"path": "tools.nudgeBundlePauseMs", "value": 5000})).unwrap();
    h.run_steps(8);
    nudge(&mut h, egui::Key::ArrowRight, 1);
    assert_eq!(steps(&h), n0 + 2, "with a five second pause, two seconds of quiet go on with the run");
    h.run_steps(24);
    nudge(&mut h, egui::Key::ArrowRight, 1);
    assert_eq!(steps(&h), n0 + 3, "six seconds of quiet end it");
}

#[test]
fn the_preference_turns_bundling_off_and_on_again() {
    let mut h = harness();
    let (start, n0) = (bounds(&h), steps(&h));
    h.state_mut().run("prefs.set", json!({"path": "tools.bundleNudges", "value": false})).unwrap();
    for _ in 0..4 {
        nudge(&mut h, egui::Key::ArrowRight, 1);
    }
    assert_eq!(steps(&h), n0 + 4, "a state per press");
    h.state_mut().session.undo();
    assert_eq!(bounds(&h).x0, start.x0 + 3);
    h.state_mut().run("prefs.set", json!({"path": "tools.bundleNudges", "value": true})).unwrap();
    for _ in 0..4 {
        nudge(&mut h, egui::Key::ArrowRight, 1);
    }
    assert_eq!(steps(&h), n0 + 4, "the next four bundle: 3 earlier states and one run");
    assert_eq!(bounds(&h).x0, start.x0 + 7);
}

/// Bad values are rejected, not stored (the control channel and the Preferences dialog share the
/// same ranges).
#[test]
fn the_pause_preference_stays_within_its_range() {
    let mut h = harness();
    for bad in [json!(0), json!(99), json!(10_001), json!("long"), json!(-5)] {
        assert!(h.state_mut().run("prefs.set", json!({"path": "tools.nudgeBundlePauseMs", "value": bad.clone()})).is_err(), "{bad}");
    }
    assert_eq!(h.state().session.prefs().tools.nudge_bundle_pause_ms, 1000);
}
