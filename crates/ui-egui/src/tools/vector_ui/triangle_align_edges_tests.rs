//! #13370: the Triangle tool's Align Edges checkbox. On (the default) the drawn box snaps to whole
//! pixels; off it keeps the drag's fractional coordinates. It is greyed outside Shape mode.

use egui::accesskit::Toggled;
use egui_kittest::kittest::{NodeT, Queryable};

use super::add_anchor_options_bar_tests::harness;
use super::shapes::shape_geometry;
use crate::state::{Tool, ToolOptions};

const START: [f64; 2] = [10.3, 10.6];
const END: [f64; 2] = [40.7, 30.2];

fn rect(o: &ToolOptions, tool: Tool, start: [f64; 2], end: [f64; 2]) -> Option<[f64; 4]> {
    let g = shape_geometry(o, tool, start, end, egui::Modifiers::NONE)?;
    serde_json::from_value(g["rect"].clone()).ok()
}

#[test]
fn align_edges_is_on_by_default_and_saved() {
    assert!(ToolOptions::default().align_edges);
    let off = ToolOptions { align_edges: false, ..Default::default() };
    let back: ToolOptions = serde_json::from_str(&serde_json::to_string(&off).unwrap()).unwrap();
    assert!(!back.align_edges, "a saved off survives a round trip");
    let empty: ToolOptions = serde_json::from_str("{}").unwrap();
    assert!(empty.align_edges, "an older file without the field gets the default");
}

#[test]
fn triangle_box_snaps_with_align_edges_and_stays_fractional_without() {
    let mut o = ToolOptions::default();
    let r = rect(&o, Tool::Triangle, START, END).unwrap();
    assert!(r.iter().all(|v| v.fract() == 0.0), "whole pixels: {r:?}");
    o.align_edges = false;
    let r = rect(&o, Tool::Triangle, START, END).unwrap();
    for (got, want) in r.iter().zip([10.3, 10.6, 30.4, 19.6]) {
        assert!((got - want).abs() < 1e-9, "{r:?}");
    }
}

#[test]
fn other_shape_tools_always_snap() {
    let o = ToolOptions { align_edges: false, ..Default::default() };
    for tool in [Tool::Polygon, Tool::EllipseShape, Tool::Rectangle, Tool::Star] {
        let r = rect(&o, tool, START, END).unwrap();
        assert!(r.iter().all(|v| v.fract() == 0.0), "{tool:?}: {r:?}");
    }
}

#[test]
fn a_greyed_checkbox_keeps_snapping_in_path_mode() {
    let o = ToolOptions { align_edges: false, shape_mode: "path".into(), ..Default::default() };
    let r = rect(&o, Tool::Triangle, START, END).unwrap();
    assert!(r.iter().all(|v| v.fract() == 0.0), "Path mode: {r:?}");
}

#[test]
fn a_sub_pixel_triangle_drag_still_draws_nothing() {
    let o = ToolOptions { align_edges: false, ..Default::default() };
    assert!(rect(&o, Tool::Triangle, [10.0, 10.0], [10.4, 30.0]).is_none());
    assert!(rect(&o, Tool::Triangle, [10.0, 10.0], [30.0, 10.9]).is_none());
}

#[test]
fn options_bar_shows_align_edges_for_the_triangle_only() {
    let mut h = harness(Tool::Triangle);
    let node = h.get_by_label("Align Edges");
    assert!(!node.accesskit_node().is_disabled(), "enabled in Shape mode");
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
    h.get_by_label("Align Edges").click();
    h.run_steps(3);
    assert!(!h.state().ui.tool_options.align_edges, "clicking toggles the option");
    assert_eq!(h.get_by_label("Align Edges").accesskit_node().toggled(), Some(Toggled::False));

    h.state_mut().ui.tool_options.shape_mode = "path".into();
    h.run_steps(4);
    assert!(h.get_by_label("Align Edges").accesskit_node().is_disabled(), "greyed in Path mode");

    let h = harness(Tool::Rectangle);
    assert!(h.query_by_label("Align Edges").is_none());
}
