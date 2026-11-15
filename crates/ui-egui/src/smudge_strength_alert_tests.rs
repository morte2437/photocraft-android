//! #12962: the Smudge tool's Strength takes whole numbers 1..100, like Blur's (#12650) and Sharpen's
//! (#12949); a value typed past the range becomes the closest one and, once the field is left, the
//! reference's "Closest value inserted" alert with OK says so. Text that is no number and a scrub to
//! either end stay silent.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::eraser_range_alert_tests::{alerts, bar, ok, type_into};
use crate::PhotocraftApp;
use crate::eraser_ui::TOLERANCE_RANGE_ALERT;
use crate::state::Tool;

fn strength(h: &Harness<'static, PhotocraftApp>) -> f32 {
    h.state().ui.tool_options.strength
}

/// Drags the Strength caption by `dx` in six steps while the field holds the keyboard, then leaves it.
fn scrub_strength(h: &mut Harness<'static, PhotocraftApp>, dx: f32) {
    let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Strength:")).expect("the Strength field");
    let rect = field.rect();
    field.click();
    h.run_steps(1);
    let primary = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    // The caption sits just left of the field.
    let from = egui::pos2(rect.left() - 25.0, rect.center().y);
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(primary(from, true));
    h.run_steps(1);
    let mut at = from;
    for _ in 0..6 {
        at.x += dx;
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(1);
    }
    h.event(primary(at, false));
    h.run_steps(3);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

#[test]
fn smudge_strength_past_the_range_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("150", 100.0), ("0", 1.0)] {
        let mut h = bar(Tool::Smudge);
        type_into(&mut h, "Strength", typed);
        assert_eq!(strength(&h), want, "{typed}");
        assert_eq!(alerts(&h), vec![TOLERANCE_RANGE_ALERT.to_string()], "{typed}: one alert");
        ok(&mut h);
        assert!(alerts(&h).is_empty(), "{typed}: OK closes the alert");
        assert_eq!(strength(&h), want, "{typed}: the closest value is kept");
    }
}

#[test]
fn smudge_strength_in_range_stores_a_whole_number_without_an_alert() {
    for (typed, want) in [("37", 37.0), ("33.6", 34.0), ("1", 1.0), ("100", 100.0)] {
        let mut h = bar(Tool::Smudge);
        type_into(&mut h, "Strength", typed);
        assert_eq!(strength(&h), want, "{typed}");
        assert!(alerts(&h).is_empty(), "{typed} is in range");
    }
}

#[test]
fn smudge_strength_that_is_no_number_keeps_the_value_without_an_alert() {
    let mut h = bar(Tool::Smudge);
    type_into(&mut h, "Strength", "abc");
    assert_eq!(strength(&h), 50.0, "the previous value stays");
    assert!(alerts(&h).is_empty(), "no number, no alert");
}

#[test]
fn a_smudge_strength_scrub_past_either_end_does_not_alert() {
    for (start, dx, want) in [("90", 50.0, 100.0), ("10", -50.0, 1.0)] {
        let mut h = bar(Tool::Smudge);
        type_into(&mut h, "Strength", start);
        scrub_strength(&mut h, dx);
        assert_eq!(strength(&h), want, "{start}: the scrub reached the end");
        assert!(alerts(&h).is_empty(), "{start}: a scrub stops at the end without an alert");
    }
}

#[test]
fn showing_the_smudge_bar_keeps_a_shared_fraction() {
    let mut h = bar(Tool::Blur);
    h.state_mut().ui.tool_options.strength = 33.6;
    h.state_mut().ui.tool = Tool::Smudge;
    h.run_steps(4);
    assert_eq!(strength(&h), 33.6, "the Smudge bar only shows the shared value");
    assert!(alerts(&h).is_empty());
}
