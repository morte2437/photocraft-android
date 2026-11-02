//! The colour swatch buttons open the full Color Picker dialog.
use super::*;
use egui_kittest::{Harness, kittest::Queryable};

/// The app, the swatch's value, and whether the swatch committed this frame (changed with its
/// picker closed: a host commits then).
type State<T> = (crate::PhotocraftApp, T, bool);

/// Draw `swatch` with the dialogs, the frame loop's [`crate::color_picker_ui::widget_tick`] first.
fn harness<T: 'static>(value: T, swatch: fn(&mut Ui, &mut T) -> Response) -> Harness<'static, State<T>> {
    let app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let mut h = Harness::builder().with_size(vec2(900.0, 600.0)).build_ui_state(
        move |ui, state: &mut State<T>| {
            crate::color_picker_ui::widget_tick(&mut state.0, ui.ctx());
            let r = swatch(ui, &mut state.1);
            state.2 = r.changed() && !crate::color_picker_ui::widget_picker_open(ui.ctx(), r.id);
            crate::dialogs::show(&mut state.0, ui.ctx());
        },
        (app, value, false),
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(2);
    h
}

/// Open the swatch's picker and show `hex` in it.
fn show<T>(h: &mut Harness<'static, State<T>>, hex: &str) {
    h.get_by_label("Color Picker (Glow Color)").click();
    h.run_steps(3);
    let d = h.state().0.ui.dialogs.last().expect("the swatch opens the Color Picker dialog");
    assert_eq!(d.fields["__label"], "Color Picker (Glow Color)");
    let id = d.id;
    h.state_mut().0.ui.dialog_mut(id).unwrap().fields.insert("color".into(), serde_json::json!(hex));
    h.run_steps(1);
}

fn pick<T>(h: &mut Harness<'static, State<T>>, hex: &str, ok: bool) {
    show(h, hex);
    h.get_by_label(if ok { "OK" } else { "Cancel" }).click();
    h.run_steps(1);
}

#[test]
fn swatch_opens_the_full_picker_and_ok_sets_the_colour_once_keeping_alpha() {
    for (original, expected) in [
        (Color32::from_rgba_unmultiplied(40, 50, 60, 128), Color32::from_rgba_unmultiplied(255, 0, 128, 128)),
        (Color32::from_rgb_additive(40, 50, 60), Color32::from_rgb_additive(255, 0, 128)),
    ] {
        let mut h = harness(original, |ui, c| color_edit_button_srgba(ui, c, "Color Picker (Glow Color)"));
        pick(&mut h, "#336699", false);
        h.run_steps(3);
        assert_eq!(h.state().1, original, "Cancel leaves the colour");
        assert!(h.state().0.ui.dialogs.is_empty());
        pick(&mut h, "#ff0080", true);
        let mut changed = 0;
        for _ in 0..4 {
            h.step();
            changed += usize::from(h.state().2);
        }
        assert_eq!(changed, 1, "the pick commits once, when the picker closes");
        assert_eq!(h.state().1, expected);
        assert_eq!(crate::paint_picker::recent_colors(&h.state().0.session).first().copied().map(crate::color_picker_ui::hex).as_deref(), Some("#ff0080"));
    }
}

#[test]
fn float_swatch_keeps_precision_until_a_new_colour_and_stores_linear() {
    let original = [0.123456, 0.234567, 0.345678];
    let mut h = harness(original, |ui, c| color_edit_button_rgb(ui, c, "Color Picker (Glow Color)"));
    h.get_by_label("Color Picker (Glow Color)").click();
    h.run_steps(3);
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert_eq!(h.state().1, original, "OK on the opening colour must not quantize to 8 bits");
    pick(&mut h, "#8040ff", true);
    h.run_steps(3);
    for (actual, expected) in h.state().1.iter().zip([0.21586053, 0.051269468, 1.0]) {
        assert!((actual - expected).abs() < 0.00001, "{actual} vs {expected}");
    }
}

#[test]
fn eight_bit_swatch_is_opaque() {
    let mut h = harness([10u8, 20, 30], |ui, c| color_edit_button_srgb(ui, c, "Color Picker (Glow Color)"));
    pick(&mut h, "#abcdef", true);
    h.run_steps(3);
    assert_eq!(h.state().1, [0xab, 0xcd, 0xef]);
}

#[test]
fn swatch_follows_the_open_picker_and_cancel_restores_the_exact_value() {
    let original = [0.123456, 0.234567, 0.345678];
    let mut h = harness(original, |ui, c| color_edit_button_rgb(ui, c, "Color Picker (Glow Color)"));
    show(&mut h, "#8040ff");
    h.run_steps(2);
    assert_eq!(h.state().0.ui.dialogs.len(), 1, "the picker is still open");
    for (actual, expected) in h.state().1.iter().zip([0.21586053, 0.051269468, 1.0]) {
        assert!((actual - expected).abs() < 0.00001, "the swatch previews the picker's colour: {actual} vs {expected}");
    }
    h.get_by_label("Cancel").click();
    let mut committed = 0;
    for _ in 0..4 {
        h.step();
        committed += usize::from(h.state().2);
    }
    assert_eq!(h.state().1, original, "Cancel restores the float exactly");
    assert_eq!(committed, 1, "closing the picker is one commit frame");
    // OK on the opening colour after a preview also goes back to the exact value.
    show(&mut h, "#8040ff");
    h.state_mut()
        .0
        .ui
        .dialogs
        .last_mut()
        .unwrap()
        .fields
        .insert("color".into(), serde_json::json!(crate::color_picker_ui::hex(original.map(egui::ecolor::gamma_from_linear))));
    h.run_steps(1);
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert_eq!(h.state().1, original, "OK unchanged keeps the float exactly");
}
