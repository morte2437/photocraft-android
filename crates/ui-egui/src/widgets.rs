//! Custom widgets for the PhotoCraft look: cards with pill tabs, thin sliders with round knobs,
//! monospace value fields with dimmed units, toggle switches, primary/secondary buttons.

use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use crate::theme::{self, Tokens};

mod angle_dial;
mod color_count;
pub use angle_dial::{angle_dial_line, angle_dial_row};
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
    ButtonRole, DialogButton, button_height, button_width, check_box, check_box_fill, checkbox, compact_button, dialog_buttons, primary_button,
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
pub use slider::{RowGestures, row_label, second_click, slider, slider_line, slider_row, slider_row_prec, slider_row_with};
pub(crate) use slider::{count_notches, wheel_notches_of};
#[cfg(test)]
pub(crate) use slider::{curve_fraction, curve_value};
pub use text_field::{
    INPUT_EVENT_LIMIT, TEXT_FIELD_H, TEXT_FIELD_LIMIT, cap_text_events, focus_first_field, text_field, text_field_ex, text_field_with_trailing,
};
pub use units::{
    FieldUnits, fmt_num, fmt_num_prec, fmt_num2, parse_num, set_field_resolution, unit_decimals, unit_of_key, value_field_extent, value_field_prec,
    value_field_units,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "widgets/color_button_tests.rs"]
mod color_button_tests;

#[cfg(test)]
#[path = "widgets/numeric_input_tests.rs"]
mod numeric_input_tests;
