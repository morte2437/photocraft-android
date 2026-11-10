use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

/// The slider row's visible caption, not the field or slider it names.
fn caption_rect<T>(h: &Harness<'_, T>, text: &str) -> egui::Rect {
    h.query_all(egui_kittest::kittest::by().label(text).include_labels())
        .find(|n| n.accesskit_node().role() == Role::Label)
        .unwrap_or_else(|| panic!("the `{text}` caption"))
        .rect()
}

fn minimum_brush() -> BrushSettings {
    let mut b = BrushSettings::default();
    b.shape_dynamics.size.minimum = 0.2;
    b.shape_dynamics.roundness.minimum = 0.35;
    b.transfer.opacity.minimum = 0.4;
    b.transfer.flow.minimum = 0.55;
    b.transfer.wetness.minimum = 0.6;
    b.transfer.mix.minimum = 0.75;
    b
}

fn section_harness(section: usize, b: BrushSettings) -> Harness<'static, BrushSettings> {
    section_harness_width(section, 340.0, b)
}

fn section_harness_width(section: usize, width: f32, b: BrushSettings) -> Harness<'static, BrushSettings> {
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(width, 1100.0)).build_ui_state(
        move |ui, b: &mut BrushSettings| {
            if !ready {
                crate::PhotocraftApp::setup_context(ui.ctx(), theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            section_body(ui, b, section, &[], None, Tool::MixerBrush);
        },
        b,
    );
    h.run_steps(3);
    h
}

#[test]
fn pose_axes_controls_are_independent_and_below_each_tilt_in_all_themes() {
    use egui::accesskit::Toggled;
    for kind in theme::ThemeKind::ALL {
        let b: BrushSettings = serde_json::from_value(serde_json::json!({"pose": {
            "enabled": true, "tiltX": 45.0, "tiltY": -18.0,
            "overrideTiltX": true, "overrideTiltY": false,
            "pressure": 0.71, "rotation": 67.0
        }}))
        .unwrap();
        let mut h = section_harness(7, b.clone());
        crate::PhotocraftApp::setup_context(&h.ctx, kind);
        h.run_steps(3);
        let x = h.get_by_role_and_label(Role::CheckBox, "Override Tilt X");
        let y = h.get_by_role_and_label(Role::CheckBox, "Override Tilt Y");
        assert_eq!(x.accesskit_node().toggled(), Some(Toggled::True));
        assert_eq!(y.accesskit_node().toggled(), Some(Toggled::False));
        assert!(caption_rect(&h, "Tilt X").bottom() < x.rect().top() && x.rect().bottom() < caption_rect(&h, "Tilt Y").top());
        assert!(caption_rect(&h, "Tilt Y").bottom() < y.rect().top() && y.rect().bottom() < caption_rect(&h, "Rotation").top());
        assert_eq!(h.state(), &b, "drawing preserves pose degrees and flags");
        y.focus();
        h.run_steps(1);
        h.key_press(egui::Key::Space);
        h.run_steps(2);
        let wire = serde_json::to_value(h.state()).unwrap();
        assert_eq!(wire["pose"]["overrideTiltX"], true);
        assert_eq!(wire["pose"]["overrideTiltY"], true);
        let mut expected = serde_json::to_value(b).unwrap();
        expected["pose"]["overrideTiltY"] = serde_json::json!(true);
        expected["pose"]["overrideTilt"] = serde_json::json!(true);
        assert_eq!(wire, expected, "Space changes only Y, plus the derived legacy readback");
    }
}

/// #3825: jitter is an amount, not a hue selector; it uses the themed amount track.
#[test]
fn hue_jitter_paints_an_amount_track_in_every_theme() {
    for kind in theme::ThemeKind::ALL {
        let mut b = BrushSettings::default();
        b.color_dynamics.hue_jitter = 0.37;
        let mut h = section_harness(5, b.clone());
        crate::PhotocraftApp::setup_context(&h.ctx, kind);
        h.run_steps(3);
        let label = caption_rect(&h, "Hue Jitter");
        let t = Tokens::get(&h.ctx);
        assert!(
            h.output().shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Rect(r)
                if r.rect.width() > 150.0 && (r.rect.height() - 3.0).abs() < 0.1
                && r.rect.top() > label.bottom() && r.rect.bottom() < label.bottom() + 30.0
                && r.fill == t.field_border)),
            "{kind:?}: Hue Jitter paints the same neutral track as other percentage amounts"
        );
        assert!(h.query_all_by_role(Role::SpinButton).any(|n| n.accesskit_node().numeric_value() == Some(37.0)));
        assert_eq!(h.state(), &b, "rendering does not change saved brush settings");
    }
}

struct TransferState {
    brush: BrushSettings,
    tool: Tool,
}

fn transfer_mixer_brush() -> BrushSettings {
    let mut b = BrushSettings::default();
    b.transfer.enabled = true;
    for (dynamic, jitter, minimum, fade_steps) in [
        (&mut b.transfer.opacity, 0.13, 0.17, 119),
        (&mut b.transfer.flow, 0.27, 0.29, 229),
        (&mut b.transfer.wetness, 0.41, 0.43, 339),
        (&mut b.transfer.mix, 0.63, 0.67, 449),
    ] {
        *dynamic = Dynamic { control: Control::Fade, jitter, minimum, fade_steps, ..Default::default() };
    }
    b
}

fn transfer_mixer_harness(kind: theme::ThemeKind, tool: Tool) -> Harness<'static, TransferState> {
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(340.0, 1400.0)).build_ui_state(
        move |ui, state: &mut TransferState| {
            if !ready {
                crate::PhotocraftApp::setup_context(ui.ctx(), kind);
                ready = true;
                return;
            }
            ui.add_enabled_ui(state.brush.transfer.enabled, |ui| section_body(ui, &mut state.brush, 6, &[], None, state.tool));
        },
        TransferState { brush: transfer_mixer_brush(), tool },
    );
    h.run_steps(3);
    h
}

/// #3825: Mixer-only blocks follow the tool, including sliders, minimums and Fade steps.
#[test]
fn transfer_mixer_controls_follow_the_tool_and_section_in_every_theme() {
    for kind in theme::ThemeKind::ALL {
        let mut h = transfer_mixer_harness(kind, Tool::Brush);
        let before = h.state().brush.clone();
        for tool in Tool::ALL {
            h.state_mut().tool = tool;
            h.run_steps(2);
            for (mixer_only, values) in [(false, [13.0, 17.0, 119.0, 27.0, 29.0, 229.0]), (true, [41.0, 43.0, 339.0, 63.0, 67.0, 449.0])] {
                for value in values {
                    let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().numeric_value() == Some(value)).unwrap();
                    assert_eq!(field.accesskit_node().is_disabled(), mixer_only && tool != Tool::MixerBrush, "{kind:?}, {tool:?}, {value}");
                }
            }
            for (index, node) in h.query_all_by_role(Role::ComboBox).enumerate() {
                assert_eq!(node.accesskit_node().is_disabled(), index >= 2 && tool != Tool::MixerBrush, "{kind:?}, {tool:?}: controller {index}");
            }
            assert_eq!(h.state().brush, before, "changing tool context only changes availability");
        }
        h.state_mut().tool = Tool::MixerBrush;
        h.state_mut().brush.transfer.enabled = false;
        h.run_steps(2);
        assert!(h.query_all_by_role(Role::SpinButton).all(|node| node.accesskit_node().is_disabled()), "section Off still disables every field");
        assert!(h.query_all_by_role(Role::ComboBox).all(|node| node.accesskit_node().is_disabled()));
        h.state_mut().brush.transfer.enabled = true;
        h.state_mut().brush.transfer.wetness.control = Control::Off;
        h.run_steps(2);
        let minimum = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().numeric_value() == Some(43.0)).unwrap();
        assert!(minimum.accesskit_node().is_disabled(), "Control Off still disables the applicable minimum");
    }
}

#[test]
fn transfer_mixer_controls_ignore_disabled_edits_and_keep_values_on_reactivation() {
    let mut h = transfer_mixer_harness(theme::ThemeKind::ProMedium, Tool::Brush);
    let before = h.state().brush.clone();
    for value in [41.0, 43.0, 339.0, 63.0, 67.0, 449.0] {
        let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().numeric_value() == Some(value)).unwrap();
        let at = field.rect().center();
        field.click();
        h.key_press(egui::Key::ArrowUp);
        h.event(egui::Event::Text("999".into()));
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        h.hover_at(at);
        h.drag_at(at);
        h.run_steps(1);
        h.drop_at(at + vec2(60.0, 0.0));
        h.run_steps(2);
        assert_eq!(h.state().brush, before, "inactive field {value} ignores keyboard and pointer input");
    }
    for label in ["Wetness Jitter", "Mix Jitter"] {
        let row = caption_rect(&h, label);
        let from = pos2(row.left() + 120.0, row.bottom() + 16.0);
        h.hover_at(from);
        h.drag_at(from);
        h.run_steps(1);
        h.hover_at(from + vec2(150.0, 0.0));
        h.run_steps(1);
        h.drop_at(from + vec2(150.0, 0.0));
        h.run_steps(2);
        assert_eq!(h.state().brush, before, "the inactive {label} track ignores a drag");
    }
    h.state_mut().tool = Tool::MixerBrush;
    h.run_steps(2);
    let row = caption_rect(&h, "Wetness Jitter");
    let from = pos2(row.left() + 120.0, row.bottom() + 16.0);
    h.hover_at(from);
    h.drag_at(from);
    h.run_steps(1);
    h.hover_at(from + vec2(150.0, 0.0));
    h.run_steps(1);
    h.drop_at(from + vec2(150.0, 0.0));
    h.run_steps(2);
    assert!(h.state().brush.transfer.wetness.jitter > 0.6, "the same track accepts a drag when applicable");
    h.state_mut().brush.transfer.wetness.jitter = 0.41;
    h.run_steps(2);
    let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().numeric_value() == Some(41.0)).unwrap();
    field.click();
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    assert!((h.state().brush.transfer.wetness.jitter - 0.42).abs() < 1e-6);
    let after = h.state().brush.clone();
    h.state_mut().tool = Tool::Brush;
    h.run_steps(2);
    h.state_mut().tool = Tool::MixerBrush;
    h.run_steps(2);
    assert_eq!(h.state().brush, after);
    assert!(h.query_all_by_role(Role::SpinButton).any(|node| node.accesskit_node().numeric_value() == Some(42.0) && !node.accesskit_node().is_disabled()));
}

fn set_controls(b: &mut BrushSettings, control: Control) {
    for d in [
        &mut b.shape_dynamics.size,
        &mut b.shape_dynamics.roundness,
        &mut b.transfer.opacity,
        &mut b.transfer.flow,
        &mut b.transfer.wetness,
        &mut b.transfer.mix,
    ] {
        d.control = control;
    }
}

/// #3825: all six minimum fields follow their own Control; jitter remains editable.
#[test]
fn minimum_fields_are_disabled_only_when_their_control_is_off() {
    for (section, values) in [(1, vec![20.0, 35.0]), (6, vec![40.0, 55.0, 60.0, 75.0])] {
        let mut h = section_harness(section, minimum_brush());
        for (control, _) in CONTROLS {
            set_controls(h.state_mut(), control);
            h.run_steps(2);
            for value in &values {
                let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().numeric_value() == Some(*value)).expect("minimum input");
                assert_eq!(field.accesskit_node().is_disabled(), control == Control::Off, "section {section}, minimum {value}, {control:?}");
            }
            assert!(
                h.query_all_by_role(Role::SpinButton).any(|n| n.accesskit_node().numeric_value() == Some(0.0) && !n.accesskit_node().is_disabled()),
                "jitter stays editable"
            );
        }
        set_controls(h.state_mut(), Control::PenPressure);
        if section == 1 {
            h.state_mut().shape_dynamics.size.control = Control::Off;
        } else {
            h.state_mut().transfer.opacity.control = Control::Off;
        }
        h.run_steps(2);
        for (index, value) in values.iter().enumerate() {
            let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().numeric_value() == Some(*value)).unwrap();
            assert_eq!(field.accesskit_node().is_disabled(), index == 0, "each minimum follows its own control");
        }
    }
}

fn drag(h: &mut Harness<'static, BrushSettings>, from: egui::Pos2, to: egui::Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerMoved(to));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn disabled_minimum_ignores_input_and_keeps_its_value_for_reactivation() {
    let mut h = section_harness(1, minimum_brush());
    let before = h.state().clone();
    let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().numeric_value() == Some(20.0)).unwrap();
    let center = field.rect().center();
    field.click();
    h.key_press(egui::Key::ArrowUp);
    h.event(egui::Event::Text("99".into()));
    h.key_press(egui::Key::Enter);
    // The slider below the number and its scrubby label are disabled too.
    let track = pos2(300.0, center.y + 24.0);
    drag(&mut h, track, track);
    let label = caption_rect(&h, "Minimum Diameter").center();
    drag(&mut h, label, label + vec2(60.0, 0.0));
    assert_eq!(h.state(), &before, "disabled interactions never rewrite brush settings");

    h.state_mut().shape_dynamics.size.control = Control::PenPressure;
    h.run_steps(2);
    let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().numeric_value() == Some(20.0)).unwrap();
    assert!(!field.accesskit_node().is_disabled());
    field.click();
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(h.state().shape_dynamics.size.minimum, 0.21, "active minimum is editable");
    drag(&mut h, track, track);
    let minimum = h.state().shape_dynamics.size.minimum;
    assert!(minimum > 0.8, "the active slider accepts the same pointer position: {minimum}");
    h.state_mut().shape_dynamics.size.control = Control::Off;
    h.run_steps(2);
    assert_eq!(h.state().shape_dynamics.size.minimum, minimum, "Off keeps the saved minimum");
}

#[test]
fn ellipse_widget_drags_angle_and_roundness() {
    // Pointing straight up is 90° (counter-clockwise from +x; screen y is down).
    assert_eq!(ellipse_drag(EllipseGrab::Angle, vec2(0.0, -20.0), 0.0, 1.0), (90.0, 1.0));
    assert_eq!(ellipse_drag(EllipseGrab::Angle, vec2(-20.0, 0.0), 0.0, 1.0).0.abs(), 180.0);
    // The centre itself doesn't change the angle.
    assert_eq!(ellipse_drag(EllipseGrab::Angle, vec2(0.2, 0.1), 30.0, 0.5), (30.0, 0.5));
    // At angle 0 the minor axis is vertical: half the radius up is 50 % roundness.
    let (_, r) = ellipse_drag(EllipseGrab::Roundness, vec2(0.0, -ELLIPSE_R / 2.0), 0.0, 1.0);
    assert!((r - 0.5).abs() < 1e-4);
    // Handles sit on the minor axis and are what a press there grabs.
    let [h, _] = roundness_handles(0.0, 0.5);
    assert!((h - vec2(0.0, -ELLIPSE_R / 2.0)).length() < 1e-4);
    assert_eq!(ellipse_grab(h, 0.0, 0.5), EllipseGrab::Roundness);
    assert_eq!(ellipse_grab(vec2(ELLIPSE_R, 0.0), 0.0, 0.5), EllipseGrab::Angle);
    // Rotated 90°: the minor axis is horizontal.
    let [h, _] = roundness_handles(90.0, 1.0);
    assert!((h.x.abs() - ELLIPSE_R).abs() < 1e-3 && h.y.abs() < 1e-3, "{h:?}");
    let (_, r) = ellipse_drag(EllipseGrab::Roundness, vec2(500.0, 0.0), 90.0, 0.2);
    assert_eq!(r, 1.0);
}

#[test]
fn spacing_row_has_one_label_and_a_shared_header_in_every_theme() {
    for kind in theme::ThemeKind::ALL {
        for enabled in [false, true] {
            let b = BrushSettings { spacing: 0.427, spacing_enabled: enabled, ..Default::default() };
            let mut h = section_harness(0, b.clone());
            crate::PhotocraftApp::setup_context(&h.ctx, kind);
            h.run_steps(3);
            assert_eq!(h.query_all_by_role_and_label(Role::Label, "Spacing").count(), 0, "{kind:?}: the checkbox is the single label");
            assert_eq!(h.query_all_by_role_and_label(Role::CheckBox, "Spacing").count(), 1);
            let checkbox = h.get_by_role_and_label(Role::CheckBox, "Spacing").rect();
            let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().numeric_value() == Some(43.0)).unwrap();
            assert!((checkbox.center().y - field.rect().center().y).abs() < 2.0, "{kind:?}: checkbox and percentage share a row");
            assert_eq!(field.accesskit_node().is_disabled(), !enabled);
            let slider = h.get_by_role_and_label(Role::Slider, "Spacing");
            assert_eq!(slider.accesskit_node().numeric_value(), Some(43.0), "{kind:?}: the track is a named slider");
            assert_eq!(slider.accesskit_node().is_disabled(), !enabled);
            assert_eq!(h.state(), &b, "rendering does not round the saved spacing");
        }
    }
}

fn check_scatter_header(section: usize) {
    for kind in theme::ThemeKind::ALL {
        for both_axes in [false, true] {
            let mut b = BrushSettings::default();
            b.scattering.scatter.jitter = 2.137;
            b.scattering.both_axes = both_axes;
            b.dual_brush.scatter = 2.137;
            b.dual_brush.both_axes = both_axes;
            let mut h = section_harness(section, b.clone());
            crate::PhotocraftApp::setup_context(&h.ctx, kind);
            h.run_steps(3);
            let label = caption_rect(&h, "Scatter");
            let axes = h.get_by_label("Both Axes").rect();
            let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().numeric_value() == Some(214.0)).unwrap().rect();
            assert!((label.center().y - axes.center().y).abs() < 2.0, "{kind:?}, section {section}: axes belongs on the Scatter header");
            assert!((axes.center().y - field.center().y).abs() < 2.0, "{kind:?}, section {section}: axes and percentage share a row");
            assert!(label.right() < axes.left() && axes.right() < field.left(), "{kind:?}, section {section}: label, checkbox, percentage order");
            assert_eq!(
                h.query_all_by_role_and_label(Role::Slider, "Scatter").count(),
                1,
                "{kind:?}, section {section}: the slider is named by its caption, like other slider rows"
            );
            assert_eq!(h.state(), &b, "rendering retains fractional scatter and other settings");
        }
    }
}

#[test]
fn spacing_row_retains_disabled_values_and_percentage_editing_limits() {
    let mut h = section_harness(0, BrushSettings { spacing: 0.42, spacing_enabled: false, ..Default::default() });
    let before = h.state().clone();
    let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().numeric_value() == Some(42.0)).unwrap();
    let from = pos2(field.rect().left() - 100.0, field.rect().bottom() + 9.0);
    let to = pos2(field.rect().right() - 25.0, from.y);
    field.click();
    h.key_press(egui::Key::ArrowUp);
    h.event(egui::Event::Text("999".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    drag(&mut h, from, to);
    assert_eq!(h.state(), &before, "Off ignores both field input and a track drag");
    let checkbox = h.get_by_role_and_label(Role::CheckBox, "Spacing");
    checkbox.click();
    h.run_steps(2);
    assert!(h.state().spacing_enabled);
    assert_eq!(h.state().spacing, 0.42, "enabling keeps the saved value");
    let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().numeric_value() == Some(42.0)).unwrap();
    field.click();
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(h.state().spacing, 0.43, "integer percentage edits become a fraction");
    drag(&mut h, from, to);
    assert!(h.state().spacing > 5.0, "the same track is editable when enabled");
    for (spacing, value, key) in [(0.01, 1.0, egui::Key::ArrowDown), (10.0, 1000.0, egui::Key::ArrowUp)] {
        h.state_mut().spacing = spacing;
        h.run_steps(2);
        let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().numeric_value() == Some(value)).unwrap();
        field.click();
        h.key_press(key);
        h.run_steps(2);
        assert_eq!(h.state().spacing, spacing, "the percentage stays within 1..1000");
    }
}

#[test]
fn scatter_header_scattering_places_axes_between_label_and_percentage_in_all_themes() {
    check_scatter_header(2);
}

#[test]
fn scatter_header_dual_brush_places_axes_between_label_and_percentage_in_all_themes() {
    check_scatter_header(4);
}

#[test]
fn scatter_header_preserves_values_percentage_limits_and_label_scrubbing() {
    for section in [2, 4] {
        let mut b = BrushSettings::default();
        b.scattering.scatter.jitter = 2.137;
        b.scattering.scatter.control = Control::PenPressure;
        b.scattering.count = 5;
        b.dual_brush.scatter = 2.137;
        b.dual_brush.count = 7;
        let mut h = section_harness(section, b.clone());
        h.get_by_label("Both Axes").click();
        h.run_steps(2);
        if section == 2 {
            b.scattering.both_axes = true;
        } else {
            b.dual_brush.both_axes = true;
        }
        assert_eq!(h.state(), &b, "a toggle does not round scatter or change unrelated fields");
        let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().numeric_value() == Some(214.0)).unwrap();
        field.click();
        h.key_press(egui::Key::ArrowUp);
        h.run_steps(2);
        if section == 2 {
            b.scattering.scatter.jitter = 2.15;
        } else {
            b.dual_brush.scatter = 2.15;
        }
        assert_eq!(h.state(), &b, "percentage edits preserve the other brush block");
        let label = caption_rect(&h, "Scatter").center();
        drag(&mut h, label, label + vec2(18.0, 0.0));
        let amount = if section == 2 { h.state().scattering.scatter.jitter } else { h.state().dual_brush.scatter };
        assert!(amount > 2.15, "the label still scrubs the percentage across the checkbox");
        if section == 2 {
            b.scattering.scatter.jitter = amount;
        } else {
            b.dual_brush.scatter = amount;
        }
        assert_eq!(h.state(), &b, "scrubbing changes only the scatter amount");
        let field =
            h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().numeric_value() == Some(f64::from((amount * 100.0).round()))).unwrap();
        let from = pos2(field.rect().left() - 100.0, field.rect().bottom() + 9.0);
        let to = pos2(field.rect().right() - 25.0, from.y);
        drag(&mut h, from, to);
        let amount = if section == 2 { h.state().scattering.scatter.jitter } else { h.state().dual_brush.scatter };
        assert!(amount > 5.0, "the existing amount track remains editable");
        if section == 2 {
            b.scattering.scatter.jitter = amount;
        } else {
            b.dual_brush.scatter = amount;
        }
        assert_eq!(h.state(), &b);
        for (amount, value, key) in [(0.0, 0.0, egui::Key::ArrowDown), (10.0, 1000.0, egui::Key::ArrowUp)] {
            if section == 2 {
                b.scattering.scatter.jitter = amount;
            } else {
                b.dual_brush.scatter = amount;
            }
            *h.state_mut() = b.clone();
            h.run_steps(2);
            let field = h.query_all_by_role(Role::SpinButton).find(|node| node.accesskit_node().numeric_value() == Some(value)).unwrap();
            field.click();
            h.key_press(key);
            h.run_steps(2);
            assert_eq!(h.state(), &b, "the percentage stays within 0..1000");
        }
    }
}

fn tip_geometry_center(h: &Harness<'static, BrushSettings>) -> egui::Pos2 {
    fn centre(shape: &egui::Shape) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Circle(c) if c.radius == 40.0 => Some(c.center),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(centre),
            _ => None,
        }
    }
    h.output().shapes.iter().find_map(|shape| centre(&shape.shape)).unwrap()
}

#[test]
fn tip_geometry_fields_precede_the_widget_and_labels_align_in_all_themes() {
    for kind in theme::ThemeKind::ALL {
        for width in [280.0, 340.0] {
            let b = BrushSettings { angle: -23.0, roundness: 0.473, ..Default::default() };
            let mut h = section_harness_width(0, width, b.clone());
            crate::PhotocraftApp::setup_context(&h.ctx, kind);
            h.run_steps(3);
            let centre = tip_geometry_center(&h);
            let angle = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().numeric_value() == Some(-23.0)).unwrap().rect();
            let roundness = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().numeric_value() == Some(47.0)).unwrap().rect();
            let angle_label = caption_rect(&h, "Angle:");
            let roundness_label = caption_rect(&h, "Roundness:");
            assert!(angle.right() + 2.0 < centre.x - 40.0 && roundness.right() + 2.0 < centre.x - 40.0, "{kind:?}, {width}: fields belong left of the widget");
            assert!((angle.left() - roundness.left()).abs() < 0.5, "{kind:?}, {width}: one numeric column");
            assert!((angle_label.right() - roundness_label.right()).abs() < 0.5, "{kind:?}, {width}: labels align to the right");
            assert!((angle_label.center().y - angle.center().y).abs() < 2.0 && (roundness_label.center().y - roundness.center().y).abs() < 2.0);
            assert_eq!(h.state(), &b, "layout does not round imported values");
        }
    }
}

#[test]
fn tip_geometry_numeric_edits_keep_units_limits_and_other_settings() {
    let mut b = BrushSettings { angle: -23.0, roundness: 0.473, hardness: 0.65, spacing: 0.31, flip_x: true, ..Default::default() };
    let mut h = section_harness(0, b.clone());
    let angle = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().numeric_value() == Some(-23.0)).unwrap();
    angle.click();
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    b.angle = -22.0;
    assert_eq!(h.state(), &b);
    let roundness = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().numeric_value() == Some(47.0)).unwrap();
    roundness.click();
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    b.roundness = 0.48;
    assert_eq!(h.state(), &b, "percentage edits retain degrees and the rest of the brush");
    for (angle, key) in [(-180.0, egui::Key::ArrowDown), (180.0, egui::Key::ArrowUp)] {
        b.angle = angle;
        *h.state_mut() = b.clone();
        h.run_steps(2);
        let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().numeric_value() == Some(f64::from(angle))).unwrap();
        field.click();
        h.key_press(key);
        h.run_steps(2);
        assert_eq!(h.state(), &b, "angle stays within -180..180 degrees");
    }
    for (value, key) in [(1.0, egui::Key::ArrowDown), (100.0, egui::Key::ArrowUp)] {
        b.roundness = value / 100.0;
        *h.state_mut() = b.clone();
        h.run_steps(2);
        let field = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().numeric_value() == Some(f64::from(value))).unwrap();
        field.click();
        h.key_press(key);
        h.run_steps(2);
        assert_eq!(h.state(), &b, "roundness stays within 1..100 percent");
    }
}

#[test]
fn tip_geometry_pointer_drags_keep_angle_and_roundness_independent() {
    let mut b = BrushSettings { roundness: 0.5, flip_x: true, ..Default::default() };
    let mut h = section_harness(0, b.clone());
    let c = tip_geometry_center(&h);
    drag(&mut h, c + vec2(35.0, 0.0), c + vec2(0.0, -35.0));
    b.angle = 90.0;
    assert_eq!(h.state(), &b, "the arrow changes only the angle");
    let handle = c + roundness_handles(b.angle, b.roundness)[0];
    // Acquire the dot before moving beyond its hit radius.
    h.hover_at(handle);
    h.run_steps(2);
    h.drag_at(handle);
    h.run_steps(1);
    h.hover_at(handle + vec2(-6.5, 0.0));
    h.run_steps(2);
    h.hover_at(c + vec2(-ELLIPSE_R * 0.8, 0.0));
    h.run_steps(2);
    h.drop_at(c + vec2(-ELLIPSE_R * 0.8, 0.0));
    h.run_steps(2);
    b.roundness = 0.8;
    assert!((h.state().roundness - b.roundness).abs() < 1e-5, "a real handle drag keeps percentage units");
    b.roundness = h.state().roundness;
    assert_eq!(h.state(), &b, "the dot changes only the roundness");
}
