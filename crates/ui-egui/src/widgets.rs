//! Custom widgets for the PhotoCraft look: cards with pill tabs, thin sliders with round knobs,
//! monospace value fields with dimmed units, toggle switches, primary/secondary buttons.

use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use crate::theme::{self, Tokens};

mod angle_dial;
mod color_count;
pub use angle_dial::angle_dial_row;
pub use color_count::color_count_row;
mod type_ahead;
pub use type_ahead::list_open as dropdown_list_open;

mod buttons;
mod color;
mod dropdown;
pub mod field_tab;
pub mod field_wheel;
mod fields;
pub(super) mod numeric_expression;
mod panel;
pub mod point_curve;
pub mod press_menu;
mod scrub;
mod slider;
pub mod swap_arrow;
pub(super) mod tab_strip;
mod text_field;
pub mod text_field_menu;
pub mod tone;
mod units;

pub use buttons::{
    ButtonRole, DialogButton, button_height, button_width, check_box, check_box_fill, checkbox, checkbox_wrapped, compact_button, dialog_buttons, primary_button,
    secondary_button, toggle,
};
pub(crate) use color::color_swatch;
pub use color::{
    checker, checker_rounded, color_edit_button_rgb, color_edit_button_srgb, color_edit_button_srgba, color_swatch_button, hue_color, hue_stops, pick_pos,
    rgb_histogram,
};
#[cfg(test)]
use color::{checker_tile_id, checker_uv};
pub use dropdown::{dropdown, dropdown_hovered, dropdown_literal, dropdown_wheel_hovered, dropdown_with_tooltips, menu_scroll, swatch_popup};
pub(crate) use fields::compact_wrapping_value_field;
#[cfg(test)]
use fields::{FieldOpts, NUMERIC_INPUT_LIMIT, value_field_in};
pub use fields::{
    POPUP_ARROW_W, PopupFieldResponse, VALUE_FIELD_H, compact_value_field, elided_label, elided_label_sensed, popup_value_field, value_field, value_field_named,
};
pub(crate) use panel::body_margin;
pub use panel::{
    FOOTER_BAR, chevron_icon, drop_line, footer_height, hairline, panel_footer, panel_menu_button, pill_tab, section_label, style_spectrum_popup_menu, surface,
    vline,
};
pub use scrub::{scrub_label, scrub_option_icon, scrub_option_label};
#[cfg(test)]
use slider::DOUBLE_CLICK_S;
pub use slider::{RowGestures, second_click, slider, slider_row, slider_row_prec, slider_row_with};
pub(crate) use slider::{count_notches, wheel_notches_of};
#[cfg(test)]
pub(crate) use slider::{curve_fraction, curve_value};
pub use text_field::{INPUT_EVENT_LIMIT, TEXT_FIELD_H, TEXT_FIELD_LIMIT, cap_text_events, focus_first_field, text_field, text_field_ex};
pub use units::{
    FieldUnits, fmt_num, fmt_num_prec, fmt_num2, parse_num, set_field_resolution, unit_decimals, unit_of_key, value_field_extent, value_field_prec,
    value_field_units,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod screen_color_tests {
    use super::*;
    use crate::screen_picker::{Capture, Pending};
    use egui_kittest::{Harness, kittest::Queryable};
    #[test]
    fn float_picker_keeps_precision_and_converts_screen_srgb_to_linear() {
        let services = crate::Services {
            screen_pick: Some(Box::new(|_| {
                let (tx, receiver) = std::sync::mpsc::channel();
                tx.send(Ok(Capture::Color(Some([0.5, 0.25, 1.0])))).unwrap();
                Pending { receiver, cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)) }
            })),
            ..Default::default()
        };
        let original = [0.123456, 0.234567, 0.345678];
        let app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let mut h = Harness::builder().with_size(vec2(600.0, 500.0)).build_ui_state(
            |ui, state: &mut (crate::PhotocraftApp, [f32; 3])| {
                crate::screen_picker::tick(&mut state.0, ui.ctx());
                color_edit_button_rgb(ui, &mut state.1);
            },
            (app, original),
        );
        h.run_steps(2);
        assert_eq!(h.state().1, original);
        h.get_by_role(egui::accesskit::Role::ColorWell).click();
        h.run_steps(3);
        for (actual, expected) in h.state().1.iter().zip(original) {
            assert!((actual - expected).abs() < 0.000001, "opening the float popup must not quantize to 8 bits");
        }
        h.get_by_label("Pick screen color").click();
        h.run_steps(3);
        for (actual, expected) in h.state().1.iter().zip([0.21404114, 0.05087609, 1.0]) {
            assert!((actual - expected).abs() < 0.000001);
        }
    }
    #[test]
    fn compact_picker_delivers_screen_color_after_popup_closes_and_keeps_alpha() {
        for (original, expected) in [
            (Color32::from_rgba_unmultiplied(40, 50, 60, 128), Color32::from_rgba_unmultiplied(255, 0, 128, 128)),
            (Color32::from_rgb_additive(40, 50, 60), Color32::from_rgb_additive(255, 0, 128)),
        ] {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut receiver = Some(rx);
            let services = crate::Services {
                screen_pick: Some(Box::new(move |_| Pending {
                    receiver: receiver.take().unwrap(),
                    cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                })),
                ..Default::default()
            };
            let app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), services);
            let mut h = Harness::builder().with_size(vec2(600.0, 500.0)).build_ui_state(
                |ui, state: &mut (crate::PhotocraftApp, Color32, bool)| {
                    crate::screen_picker::tick(&mut state.0, ui.ctx());
                    state.2 = color_edit_button_srgba(ui, &mut state.1).changed();
                },
                (app, original, false),
            );
            h.get_by_role(egui::accesskit::Role::ColorWell).click();
            h.run_steps(3);
            h.get_by_label("Pick screen color").click();
            h.run_steps(2);
            assert_eq!(h.state().1, original);
            egui::Popup::close_all(&h.ctx);
            tx.send(Ok(Capture::Color(Some([1.0, 0.0, 0.5])))).unwrap();
            h.run_steps(1);
            assert!(h.state().2);
            assert_eq!(h.state().1.a(), original.a());
            assert_eq!(h.state().1, expected);
            h.run_steps(1);
            assert!(!h.state().2, "the pick commits once");
        }
    }
}

#[cfg(test)]
#[path = "widgets/numeric_input_tests.rs"]
mod numeric_input_tests;
