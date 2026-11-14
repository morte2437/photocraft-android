//! #12650: the Blur tool's Strength takes whole numbers 1..100; a value typed past the range becomes
//! the closest one and, once the field is left, the reference's "Closest value inserted" alert with
//! OK says so. Sharpen (#12949) and Smudge (#12962) do the same.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::eraser_range_alert_tests::{alerts, bar, ok, type_into};
use crate::PhotocraftApp;
use crate::eraser_ui::TOLERANCE_RANGE_ALERT;
use crate::state::Tool;

/// Clicks into Strength and gives back where it sits.
fn click_strength(h: &mut Harness<'static, PhotocraftApp>) -> egui::Rect {
    let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Strength:")).expect("the Strength field");
    let rect = field.rect();
    field.click();
    h.run_steps(1);
    rect
}

fn strength(h: &Harness<'static, PhotocraftApp>) -> f32 {
    h.state().ui.tool_options.strength
}

#[test]
fn blur_strength_past_the_range_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("150", 100.0), ("0", 1.0)] {
        let mut h = bar(Tool::Blur);
        type_into(&mut h, "Strength", typed);
        assert_eq!(strength(&h), want, "{typed}");
        assert_eq!(alerts(&h), vec![TOLERANCE_RANGE_ALERT.to_string()], "{typed}: one alert");
        ok(&mut h);
        assert!(alerts(&h).is_empty(), "{typed}: OK closes the alert");
        assert_eq!(strength(&h), want, "{typed}: the closest value is kept");
    }
}

#[test]
fn blur_strength_in_range_stores_a_whole_number_without_an_alert() {
    for (typed, want) in [("40", 40.0), ("33.6", 34.0), ("1", 1.0), ("100", 100.0)] {
        let mut h = bar(Tool::Blur);
        type_into(&mut h, "Strength", typed);
        assert_eq!(strength(&h), want, "{typed}");
        assert!(alerts(&h).is_empty(), "{typed} is in range");
    }
}

#[test]
fn blur_strength_that_is_no_number_keeps_the_value_without_an_alert() {
    let mut h = bar(Tool::Blur);
    type_into(&mut h, "Strength", "abc");
    assert_eq!(strength(&h), 50.0, "the previous value stays");
    assert!(alerts(&h).is_empty(), "no number, no alert");
}

#[test]
fn a_caption_scrub_past_either_end_does_not_alert() {
    for (start, dx, want) in [("90", 50.0, 100.0), ("10", -50.0, 1.0)] {
        let mut h = bar(Tool::Blur);
        type_into(&mut h, "Strength", start);
        // The field keeps the keyboard while its caption is scrubbed past the end.
        let rect = click_strength(&mut h);
        // The caption sits just left of the field.
        let from = egui::pos2(rect.left() - 25.0, rect.center().y);
        h.event(egui::Event::PointerMoved(from));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.run_steps(1);
        for k in 1..=6 {
            h.event(egui::Event::PointerMoved(from + vec2(dx * k as f32, 0.0)));
            h.run_steps(1);
        }
        let to = from + vec2(dx * 6.0, 0.0);
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(3);
        h.key_press(egui::Key::Tab);
        h.run_steps(3);
        assert_eq!(strength(&h), want, "{start}: the scrub reached the end");
        assert!(alerts(&h).is_empty(), "{start}: a scrub stops at the end without an alert");
    }
}

#[test]
fn opening_the_blur_bar_keeps_a_sharpen_fraction() {
    let mut h = bar(Tool::Sharpen);
    h.state_mut().ui.tool_options.strength = 33.6;
    h.run_steps(2);
    h.state_mut().ui.tool = Tool::Blur;
    h.run_steps(4);
    h.state_mut().ui.tool = Tool::Sharpen;
    h.run_steps(2);
    assert_eq!(strength(&h), 33.6, "the Blur bar only shows the shared value");
    assert!(alerts(&h).is_empty());
}
