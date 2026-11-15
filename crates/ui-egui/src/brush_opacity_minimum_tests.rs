//! The Brush's Opacity starts at 1 %, as in Photoshop (#11812): its options-bar field takes whole
//! numbers from 1 to 100, the digit keys `00` set 100 %, and `tools.setBrush` floors opacity at 1 %.

use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::opacity_keys::press;
use crate::state::Tool;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.tool = tool;
    app
}

fn options_bar(tool: Tool, theme: crate::theme::ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(1400.0, 60.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                crate::theme::install_fonts(ui.ctx());
                crate::theme::apply(ui.ctx(), theme);
                ready = true;
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app(tool),
    );
    h.run_steps(4);
    h
}

/// Types `text` into the options bar's Opacity field and presses Enter.
fn type_opacity(h: &mut Harness<'_, PhotocraftApp>, text: &str) -> f32 {
    // "Opacity:" in the Photoshop-look themes, "Opacity" in the others.
    let field = h
        .query_all(egui_kittest::kittest::by().role(egui::accesskit::Role::SpinButton))
        .find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == "Opacity"))
        .unwrap_or_else(|| panic!("the {:?} Opacity field", h.state().ui.tool))
        .rect();
    h.hover_at(field.center());
    h.run();
    h.drag_at(field.center());
    h.run();
    h.drop_at(field.center());
    h.run_steps(2);
    h.event(Event::Key { key: Key::A, physical_key: Some(Key::A), pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
    h.run();
    h.event(Event::Text(text.into()));
    h.run_steps(2);
    h.event(Event::Key { key: Key::Enter, physical_key: Some(Key::Enter), pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    h.state().session.tools.brush.opacity
}

/// In the Photoshop-look themes and the others (their options bars lay the Brush out apart); the
/// Pencil and Eraser fields take 1-100 whole numbers in Photoshop too.
#[test]
fn the_brush_opacity_field_takes_whole_numbers_from_1_to_100() {
    use crate::theme::ThemeKind;
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        for tool in [Tool::Brush, Tool::Pencil, Tool::Eraser] {
            let mut h = options_bar(tool, theme);
            for (typed, want) in [("0", 0.01), ("-10", 0.01), ("150", 1.0), ("45", 0.45), ("33.7", 0.34)] {
                let got = type_opacity(&mut h, typed);
                assert!((got - want).abs() < 1e-6, "{theme:?} {tool:?} typed {typed}: {got}, want {want}");
            }
        }
    }
}

#[test]
fn double_zero_sets_the_brush_to_100_percent() {
    let mut a = app(Tool::Brush);
    a.run("tools.setBrush", json!({"brush": {"opacity": 0.3}})).unwrap();
    press(&mut a, 0, false, 0.0);
    press(&mut a, 0, false, 100.0);
    assert_eq!(a.session.tools.brush.opacity, 1.0, "00 = 100%");
    // Other pairs are the exact value; single digits are tens; 0 alone is 100%.
    for (digits, want) in [([0, 7], 0.07), ([4, 5], 0.45)] {
        press(&mut a, digits[0], false, 10_000.0);
        press(&mut a, digits[1], false, 10_100.0);
        assert!((a.session.tools.brush.opacity - want).abs() < 1e-6, "{digits:?}");
    }
    for (d, want) in [(3, 0.3), (9, 0.9), (0, 1.0), (1, 0.1)] {
        press(&mut a, d, false, 20_000.0 + f64::from(d) * 10_000.0);
        assert!((a.session.tools.brush.opacity - want).abs() < 1e-6, "{d}");
    }
    // With build-up on, ⇧ digits set the opacity: `00` is 100% there too.
    a.session.tools.brush.build_up = true;
    press(&mut a, 2, true, 200_000.0);
    press(&mut a, 0, true, 300_000.0);
    press(&mut a, 0, true, 300_100.0);
    assert_eq!(a.session.tools.brush.opacity, 1.0);
}

#[test]
fn double_zero_on_flow_keeps_its_one_percent() {
    let mut a = app(Tool::Brush);
    press(&mut a, 0, true, 0.0);
    press(&mut a, 0, true, 100.0);
    assert_eq!(a.session.tools.brush.flow, 0.01, "⇧00 is Flow's 1%, unchanged");
}

#[test]
fn set_brush_floors_opacity_at_one_percent() {
    let mut a = app(Tool::Brush);
    for params in [json!({"brush": {"opacity": 0}}), json!({"opacity": 0.0}), json!({"brush": {"opacity": 0.004}})] {
        a.run("tools.setBrush", json!({"brush": {"opacity": 0.5}})).unwrap();
        a.run("tools.setBrush", params.clone()).unwrap();
        assert_eq!(a.session.tools.brush.opacity, 0.01, "{params}");
    }
    a.run("tools.setBrush", json!({"brush": {"opacity": 0.37}})).unwrap();
    assert!((a.session.tools.brush.opacity - 0.37).abs() < 1e-6, "values above the floor are kept");
}

/// The Pencil's `00` is 100 % too (the Pencil has no Flow, so ⇧ digits are not involved).
#[test]
fn double_zero_sets_the_pencil_to_100_percent() {
    let mut a = app(Tool::Pencil);
    a.run("tools.setBrush", json!({"brush": {"opacity": 0.3}})).unwrap();
    press(&mut a, 0, false, 0.0);
    press(&mut a, 0, false, 100.0);
    assert_eq!(a.session.tools.brush.opacity, 1.0, "00 = 100%");
}

/// An opacity set outside the field (a `tools.setBrush` fraction, an older saved 0) shows as the
/// whole percent in 1-100 the field can hold.
#[test]
fn the_opacity_field_shows_outside_values_as_whole_percents() {
    let mut h = options_bar(Tool::Brush, crate::theme::ThemeKind::ProMedium);
    for (stored, shown) in [(0.337, 34.0), (0.0, 1.0), (0.004, 1.0)] {
        h.state_mut().session.tools.brush.opacity = stored;
        h.run_steps(2);
        let value = h
            .query_all(egui_kittest::kittest::by().role(egui::accesskit::Role::SpinButton))
            .find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == "Opacity"))
            .and_then(|n| n.accesskit_node().numeric_value());
        assert_eq!(value, Some(shown), "stored {stored}");
        assert_eq!(h.state().session.tools.brush.opacity, stored, "showing it does not change it");
    }
}
