//! #12744: the Convert Point tool's right-click opens the same path menu as the Delete Anchor
//! Point tool's (#12771), with no Convert Point entry. The shared steps live in
//! `delete_anchor_menu_tests`.

use egui::Modifiers;
use egui_kittest::kittest::Queryable;

use super::delete_anchor_menu_tests::{add_on_the_edge, delete_the_corner, empty_canvas, opening_changes_nothing, right_clicked, saved, setup};
use super::tests::{harness, right_click, screen_of};
use crate::canvas::{ToolEvent, tool_event};
use crate::direct_select::PathRef;
use crate::state::Tool;

#[test]
fn right_click_on_an_anchor_offers_delete_anchor_point_on_top_and_it_deletes() {
    delete_the_corner(Tool::ConvertPoint);
}

#[test]
fn right_click_on_a_segment_offers_add_anchor_point_on_top() {
    add_on_the_edge(Tool::ConvertPoint);
}

#[test]
fn right_click_on_empty_canvas_opens_the_path_menu_without_a_top_entry() {
    empty_canvas(Tool::ConvertPoint);
}

#[test]
fn the_menu_has_no_convert_point_entry_and_converts_nothing() {
    let before = saved(&setup(Tool::ConvertPoint));
    for at in [[300.0, 300.0], [200.0, 300.0], [500.0, 100.0]] {
        let h = right_clicked(Tool::ConvertPoint, at);
        assert!(h.state().ui.canvas_tool_menu.is_some(), "the menu opened at {at:?}");
        assert!(h.query_by_label("Convert Point").is_none(), "no Convert Point entry at {at:?}");
        assert_eq!(saved(h.state()), before, "no point converted at {at:?}");
    }
}

#[test]
fn opening_the_menu_adds_no_history_step_and_converts_no_point() {
    opening_changes_nothing(Tool::ConvertPoint);
}

/// Every anchor selected (a Direct Selection marquee), then Delete Anchor Point from the menu on
/// (300,300): the anchors shift, so no old anchor index stays selected, as after a Delete Anchor
/// Point click: `app.run` drops a Direct Selection whose revision moved on (`cancel_stale`).
#[test]
fn deleting_from_the_menu_leaves_no_shifted_anchor_selected() {
    let mut app = setup(Tool::DirectSelection);
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 250.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 750.0, y: 550.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 750.0, y: 550.0 }, Modifiers::NONE);
    assert!(app.ui.direct_selection.anchors.len() > 3, "every anchor is selected");
    app.ui.tool = Tool::ConvertPoint;
    let mut h = harness(app);
    assert!(!h.state().ui.direct_selection.anchors.is_empty(), "Convert Point keeps the selection");
    let p = screen_of(h.state(), [300.0, 300.0]);
    right_click(&mut h, p, Modifiers::NONE);
    // Chosen without a frame after it, which would drop a selection with no owner, as for a click.
    let ctx = h.ctx.clone();
    super::choose(h.state_mut(), &ctx, "path.deleteAnchor");
    assert_eq!(saved(h.state()).subpaths[0].knots.len(), 3, "the anchor is gone");
    let ds = &h.state().ui.direct_selection;
    assert!(ds.target.is_none_or(|t| t == PathRef::Saved(0)), "the selection is on no other path");
    assert!(ds.anchors.is_empty(), "no stale anchor index: {:?}", ds.anchors);
}
