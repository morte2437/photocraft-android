//! #12088: with the Vertical Type Mask tool the options bar's alignment buttons are Top / Center /
//! Bottom, idle and while editing vertical mask text, and Left / Center / Right once the mask text
//! is toggled horizontal. They still set left / center / right.

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{LayerId, text::TextAlign};

use super::super::pointer_up;
use super::super::vertical_align_buttons_tests::{HORIZONTAL, VERTICAL, align_of, bar, lit, new_app, shown};
use crate::PhotocraftApp;
use crate::state::Tool;

/// The Vertical Type Mask tool typing a new mask; returns its hidden working layer.
fn typing_a_mask(app: &mut PhotocraftApp) -> LayerId {
    pointer_up(app, [400.0, 100.0], [400.0, 100.0]);
    let ed = app.ui.text_edit.as_ref().filter(|ed| ed.mask).expect("typing a mask");
    LayerId(ed.layer)
}

#[test]
fn idle_the_bar_shows_the_vertical_buttons_with_top_lit() {
    let h = bar(new_app(Tool::VerticalTypeMask));
    assert_eq!(shown(&h), VERTICAL);
    assert!(lit(&h, "Top align text"), "the default, left, lights Top");
}

#[test]
fn editing_vertical_mask_text_shows_the_vertical_buttons() {
    let mut app = new_app(Tool::VerticalTypeMask);
    typing_a_mask(&mut app);
    assert_eq!(shown(&bar(app)), VERTICAL);
}

#[test]
fn toggling_the_mask_text_horizontal_shows_left_center_and_right() {
    let mut app = new_app(Tool::VerticalTypeMask);
    typing_a_mask(&mut app);
    let mut h = bar(app);
    assert_eq!(shown(&h), VERTICAL, "before the toggle");
    h.get_by_role_and_label(Role::Button, "Toggle text orientation").click();
    h.run_steps(2);
    assert!(h.state().ui.text_edit.is_some(), "still editing");
    assert_eq!(shown(&h), HORIZONTAL);
}

#[test]
fn the_vertical_buttons_set_left_center_and_right_on_the_mask_text_and_the_tool() {
    for (label, align, value) in
        [("Bottom align text", TextAlign::Right, "right"), ("Center text", TextAlign::Center, "center"), ("Top align text", TextAlign::Left, "left")]
    {
        let mut app = new_app(Tool::VerticalTypeMask);
        app.ui.tool_options.type_align = "center".into();
        let id = typing_a_mask(&mut app);
        let mut h = bar(app);
        h.get_by_role_and_label(Role::Button, label).click();
        h.run_steps(2);
        assert_eq!(align_of(h.state(), id), align, "{label}: the mask text");
        assert_eq!(h.state().ui.tool_options.type_align, value, "{label}: the tool default");
        for other in VERTICAL {
            assert_eq!(lit(&h, other), other == label, "{label} clicked: {other}");
        }
    }
}

#[test]
fn idle_the_vertical_buttons_set_the_tool_default() {
    let mut h = bar(new_app(Tool::VerticalTypeMask));
    h.get_by_role_and_label(Role::Button, "Bottom align text").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.tool_options.type_align, "right");
    assert!(lit(&h, "Bottom align text"));
}
