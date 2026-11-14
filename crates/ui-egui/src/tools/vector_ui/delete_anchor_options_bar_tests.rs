//! #12827: the Delete Anchor Point options bar holds the preset picker and "No options for the
//! Delete Anchor Point Tool." centred in the rest of the bar, as in Photoshop.

use egui_kittest::kittest::Queryable;

use super::add_anchor_options_bar_tests::{assert_centred_after_picker, harness};
use crate::state::Tool;

const NO_OPTIONS: &str = "No options for the Delete Anchor Point Tool.";

#[test]
fn delete_anchor_point_bar_says_it_has_no_options() {
    let h = harness(Tool::DeleteAnchorPoint);
    assert!(h.query_by_label_contains("Click an anchor point to delete it").is_none(), "the Alt hint is gone");
    assert!(h.query_by_label("No options for the Add Anchor Point Tool.").is_none());
    assert_centred_after_picker(&h, Tool::DeleteAnchorPoint, NO_OPTIONS);
}

#[test]
fn other_pen_group_bars_do_not_say_it() {
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen, Tool::AddAnchorPoint, Tool::ConvertPoint] {
        assert!(harness(tool).query_by_label(NO_OPTIONS).is_none(), "{tool:?}");
    }
}
