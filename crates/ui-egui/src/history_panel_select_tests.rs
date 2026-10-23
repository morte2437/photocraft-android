//! #2966: a click on a History panel state returns the document there with one command
//! (`history.selectState`), not one Undo / Redo per state, so the canvas refreshes once and only
//! where the jump changed pixels.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::*;

/// Open, then New Layer, Fill, New Layer, Fill.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    for (cmd, p) in
        [("layer.new.layer", json!({})), ("edit.fill", json!({"color": "#ff0000"})), ("layer.new.layer", json!({})), ("edit.fill", json!({"color": "#00ff00"}))]
    {
        app.run(cmd, p).unwrap();
    }
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            history(app, ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

fn state(h: &Harness<'_, PhotocraftApp>) -> (usize, usize, u64) {
    let st = h.state().session.active().unwrap();
    (st.history.past_len(), st.history.redo_labels().count(), st.revision)
}

#[test]
fn clicking_a_state_jumps_there_with_one_command() {
    let mut h = harness();
    let (past, redo, rev) = state(&h);
    assert_eq!((past, redo), (4, 0));
    // Back three states: one command, one revision (the canvas sees one change).
    h.get_all_by_label("New Layer").next().expect("the first New Layer row").click();
    h.run_steps(2);
    assert_eq!(state(&h), (1, 3, rev + 1));
    assert_eq!(h.state().perf.last_command, "history.selectState");
    // Forward to the last (dimmed) state, again in one step.
    h.get_all_by_label("Fill").last().expect("the last Fill row").click();
    h.run_steps(2);
    assert_eq!(state(&h), (4, 0, rev + 2));
    // The current state: nothing to do.
    h.get_all_by_label("Fill").last().expect("the last Fill row").click();
    h.run_steps(2);
    assert_eq!(state(&h), (4, 0, rev + 2));
}

#[test]
fn new_history_steps_scroll_the_current_state_into_view() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    for n in 0..18 {
        app.run("layer.new.layer", json!({"name": format!("Layer {n}")})).unwrap();
    }
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(280.0, 120.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            history(app, ui);
        },
        app,
    );
    h.run_steps(4);
    let row = h.get_all_by_label("New Layer").last().expect("the latest New Layer state").rect();
    let view = h.ctx.input(|i| i.viewport_rect());
    assert!(view.contains_rect(row), "the current history row must scroll into view: row {row:?} view {view:?}",);
}

/// Open plus 18 New Layer states in a dock that shows only a few rows.
fn long_harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    for n in 0..18 {
        app.run("layer.new.layer", json!({"name": format!("Layer {n}")})).unwrap();
    }
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(280.0, 220.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            history(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// Top of the `n`th New Layer row from the end (0 = the last).
fn row_top(h: &Harness<'_, PhotocraftApp>, n: usize) -> f32 {
    let rows: Vec<_> = h.get_all_by_label("New Layer").map(|r| r.rect()).collect();
    rows[rows.len() - 1 - n].top()
}

/// The rows' visible band: below the window top, above the footer.
fn shown(h: &Harness<'_, PhotocraftApp>, n: usize) -> bool {
    let footer = h.get_by_label("Delete current state").rect().top();
    let top = row_top(h, n);
    top >= h.ctx.input(|i| i.viewport_rect()).top() && top + 24.0 <= footer + 0.5
}

#[test]
fn clicking_a_visible_state_leaves_the_list_still() {
    let mut h = long_harness();
    assert!(shown(&h, 0) && shown(&h, 1), "the newest states are in view");
    let before = row_top(&h, 1);
    // A click on the state above the current one goes back one step; the row stays under the pointer.
    let rows: Vec<_> = h.get_all_by_label("New Layer").collect();
    rows[rows.len() - 2].click();
    h.run_steps(4);
    assert_eq!(h.state().session.active().unwrap().history.redo_labels().count(), 1);
    assert_eq!(row_top(&h, 1), before, "the clicked row must not move");
}

#[test]
fn undo_and_a_layer_click_keep_the_redo_state_in_view() {
    let mut h = long_harness();
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(4);
    assert!(shown(&h, 0), "the greyed redo state stays in view after Undo");
    let before = row_top(&h, 0);
    // Selecting a layer bumps the revision but adds no state: the list stays where it is.
    let id = h.state().session.active().unwrap().doc.layers[0].id;
    h.state_mut().session.select_layer(id).unwrap();
    h.run_steps(4);
    assert_eq!(row_top(&h, 0), before, "a layer click must not scroll History");
}

#[test]
fn scrolling_away_survives_a_change_that_adds_no_state() {
    let mut h = long_harness();
    let view = h.ctx.input(|i| i.viewport_rect());
    h.event(egui::Event::PointerMoved(view.center()));
    for _ in 0..3 {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, 3.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        h.run_steps(1);
    }
    h.run_steps(4);
    let scrolled = row_top(&h, 0);
    assert!(!shown(&h, 0), "the wheel scrolled the current state out of view");
    let id = h.state().session.active().unwrap().doc.layers[0].id;
    h.state_mut().session.select_layer(id).unwrap();
    h.run_steps(4);
    assert_eq!(row_top(&h, 0), scrolled, "the user's scroll position is kept");
    // A new state brings the current row back into view.
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(4);
    assert!(shown(&h, 0), "a new state scrolls into view");
}

#[test]
fn clicking_a_snapshot_keeps_it_in_view() {
    let mut h = long_harness();
    h.state_mut().run("history.newSnapshot", json!({})).unwrap();
    h.run_steps(2);
    // Scroll to the top, where the snapshots are.
    let view = h.ctx.input(|i| i.viewport_rect());
    h.event(egui::Event::PointerMoved(view.center()));
    for _ in 0..10 {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, 5.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        h.run_steps(1);
    }
    h.run_steps(4);
    let before = h.get_all_by_label("Snapshot 1").next().expect("the snapshot row").rect().top();
    // Selecting it adds a "Snapshot 1" state at the bottom; the selected snapshot row must not scroll away.
    h.get_all_by_label("Snapshot 1").next().expect("the snapshot row").click();
    h.run_steps(4);
    assert_eq!(h.state().session.active().unwrap().history.entries().last().map(String::as_str), Some("Snapshot 1"));
    assert_eq!(h.get_all_by_label("Snapshot 1").next().expect("the snapshot row").rect().top(), before, "the clicked snapshot stays put");
}

#[test]
fn a_new_state_at_the_history_limit_still_scrolls_into_view() {
    let mut h = long_harness();
    // At the limit a new step drops the oldest: the count and the (same) name stay as they were.
    let st = h.state_mut().session.active_mut().unwrap();
    st.history.max_states = st.history.past_len();
    let view = h.ctx.input(|i| i.viewport_rect());
    h.event(egui::Event::PointerMoved(view.center()));
    for _ in 0..3 {
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, 3.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        h.run_steps(1);
    }
    h.run_steps(4);
    assert!(!shown(&h, 0), "the wheel scrolled the current state out of view");
    let past = h.state().session.active().unwrap().history.past_len();
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    assert_eq!(h.state().session.active().unwrap().history.past_len(), past, "the oldest state was dropped");
    h.run_steps(4);
    assert!(shown(&h, 0), "a new state at the limit scrolls into view");
}
