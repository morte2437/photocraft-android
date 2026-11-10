//! #11939: at Smoothing 0 % the gear's four Smoothing Options are greyed and can't be toggled, but
//! keep their values; above 0 % they work again. For the Brush and the Eraser, the tools with this
//! gear.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const OPTIONS: [&str; 4] = ["Pulled String Mode", "Stroke Catch-up", "Catch-up on Stroke End", "Adjust for Zoom"];

fn bar(tool: Tool, smoothing: f32) -> Harness<'static, PhotocraftApp> {
    bar_with(tool, smoothing, None)
}

/// [`bar`] with the four options set to `options` (in [`OPTIONS`] order) before the first frame.
fn bar_with(tool: Tool, smoothing: f32, options: Option<[bool; 4]>) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(&mut app);
    let s = &mut app.session.tools.brush.smoothing;
    s.amount = smoothing;
    if let Some([a, b, c, d]) = options {
        (s.pulled_string, s.catch_up, s.catch_up_on_end, s.adjust_for_zoom) = (a, b, c, d);
    }
    let mut h = Harness::builder().with_size(vec2(1800.0, 400.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h.get_by_label("Set additional smoothing options").click();
    h.run_steps(3);
    h
}

fn checkbox<'a>(h: &'a Harness<'static, PhotocraftApp>, label: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::CheckBox).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("{label}"))
}

fn values(h: &Harness<'static, PhotocraftApp>) -> [bool; 4] {
    let s = &h.state().session.tools.brush.smoothing;
    [s.pulled_string, s.catch_up, s.catch_up_on_end, s.adjust_for_zoom]
}

#[test]
fn at_zero_the_four_are_greyed_and_keep_their_values() {
    for tool in [Tool::Brush, Tool::Eraser] {
        let mut h = bar(tool, 0.0);
        let before = values(&h);
        for label in OPTIONS {
            assert!(checkbox(&h, label).accesskit_node().is_disabled(), "{tool:?}: {label} greyed at 0 %");
        }
        for label in OPTIONS {
            checkbox(&h, label).click();
            h.run_steps(2);
        }
        assert_eq!(values(&h), before, "{tool:?}: clicks change nothing at 0 %");
    }
}

#[test]
fn above_zero_they_work_again() {
    for tool in [Tool::Brush, Tool::Eraser] {
        let mut h = bar(tool, 0.1);
        for label in OPTIONS {
            assert!(!checkbox(&h, label).accesskit_node().is_disabled(), "{tool:?}: {label} enabled");
        }
        let before = values(&h);
        checkbox(&h, "Pulled String Mode").click();
        h.run_steps(2);
        assert_ne!(values(&h)[0], before[0], "{tool:?}: Pulled String Mode toggles");
    }
}

#[test]
fn a_smoothing_that_shows_as_zero_greys_them_too() {
    // 0.4 % (from a preset or a script) reads 0 % in the field, so the four follow the field.
    for tool in [Tool::Brush, Tool::Eraser] {
        let h = bar(tool, 0.004);
        for label in OPTIONS {
            assert!(checkbox(&h, label).accesskit_node().is_disabled(), "{tool:?}: {label} greyed at a displayed 0 %");
        }
    }
}

/// The opposite of each option's default (Pulled String off, Stroke Catch-up on, Catch-up on
/// Stroke End off, Adjust for Zoom on).
const SET: [bool; 4] = [true, false, true, false];

/// Types `text` into the options bar's Smoothing field, then Tabs out of it.
fn type_smoothing(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == "Smoothing"))
        .expect("the Smoothing field")
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

#[test]
fn a_smoothing_that_shows_as_one_enables_them() {
    // 0.9 % reads 1 % in the field, so the four are on.
    for tool in [Tool::Brush, Tool::Eraser] {
        let h = bar(tool, 0.009);
        for label in OPTIONS {
            assert!(!checkbox(&h, label).accesskit_node().is_disabled(), "{tool:?}: {label} enabled at a displayed 1 %");
        }
    }
}

#[test]
fn smoothing_to_zero_and_back_keeps_the_four_values() {
    for tool in [Tool::Brush, Tool::Eraser] {
        let mut h = bar_with(tool, 0.1, Some(SET));
        type_smoothing(&mut h, "0");
        assert_eq!(h.state().session.tools.brush.smoothing.amount, 0.0, "{tool:?}: Smoothing 0 %");
        assert_eq!(values(&h), SET, "{tool:?}: kept at 0 %");
        type_smoothing(&mut h, "10");
        assert_eq!(values(&h), SET, "{tool:?}: kept back above 0 %");
    }
}

#[test]
fn at_zero_each_checkbox_shows_its_value() {
    use egui::accesskit::Toggled;
    for tool in [Tool::Brush, Tool::Eraser] {
        for set in [SET, SET.map(|v| !v)] {
            let h = bar_with(tool, 0.0, Some(set));
            for (label, on) in OPTIONS.into_iter().zip(set) {
                let shown = checkbox(&h, label).accesskit_node().toggled();
                assert_eq!(shown, Some(if on { Toggled::True } else { Toggled::False }), "{tool:?}: {label} shows {on} at 0 %");
            }
        }
    }
}
