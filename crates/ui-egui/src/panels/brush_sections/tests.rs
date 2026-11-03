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
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(340.0, 1100.0)).build_ui_state(
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
