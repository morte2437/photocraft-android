//! #12546: the Remove tool's checkboxes read "Sample all layers", "Remove after each stroke",
//! "Create new layer", left to right, each with the reference tooltip.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::burn_bar_tooltips_tests::{bar, control, hover};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::i18n::Lang;
use crate::state::Tool;

const BOXES: [(&str, &str); 3] = [
    ("Sample all layers", "Sample content from all visible layers"),
    ("Remove after each stroke", "Remove area after a single stroke"),
    ("Create new layer", "Create new layer with results"),
];

fn remove_bar() -> Harness<'static, PhotocraftApp> {
    bar(Tool::Remove, Lang::EN, 1600.0, 1.0, false)
}

fn checked(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    control(h, Role::CheckBox, label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

#[test]
fn the_checkboxes_read_the_reference_labels_left_to_right() {
    let h = remove_bar();
    let mut boxes: Vec<(f32, String)> =
        h.query_all_by_role(Role::CheckBox).map(|n| (n.rect().left(), n.accesskit_node().label().unwrap_or_default())).collect();
    boxes.sort_by(|a, b| a.0.total_cmp(&b.0));
    let labels: Vec<&str> = boxes.iter().map(|(_, l)| l.as_str()).collect();
    assert_eq!(labels, BOXES.map(|(label, _)| label));
}

#[test]
fn hovering_each_checkbox_shows_its_tooltip() {
    for (label, tip) in BOXES {
        let mut h = remove_bar();
        assert!(h.query_by_label(tip).is_none(), "{label}: no tooltip before the hover");
        let before = h.state().ui.tool_options.clone();
        hover(&mut h, Role::CheckBox, label, tip);
        let o = &h.state().ui.tool_options;
        assert_eq!(
            (o.sample_all.remove, o.remove_after_each_stroke, o.remove_new_layer),
            (before.sample_all.remove, before.remove_after_each_stroke, before.remove_new_layer)
        );
    }
}

#[test]
fn defaults_show_only_remove_after_each_stroke_on_and_reset_tool_restores_them() {
    let mut h = remove_bar();
    let defaults = [false, true, false];
    assert_eq!(BOXES.map(|(label, _)| checked(&h, label)), defaults);
    for (label, _) in BOXES {
        control(&h, Role::CheckBox, label).click();
        h.run_steps(2);
    }
    assert_eq!(BOXES.map(|(label, _)| checked(&h, label)), defaults.map(|on| !on), "each click toggles its own box");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(BOXES.map(|(label, _)| checked(&h, label)), defaults);
}

#[test]
fn create_new_layer_greys_when_a_mask_or_colour_subset_is_targeted() {
    let mut h = remove_bar();
    assert!(!control(&h, Role::CheckBox, "Create new layer").accesskit_node().is_disabled());
    let app = h.state_mut();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.mask_target = true;
    h.run_steps(2);
    assert!(control(&h, Role::CheckBox, "Create new layer").accesskit_node().is_disabled(), "a targeted mask greys it");
    let app = h.state_mut();
    app.ui.mask_target = false;
    app.run("channel.target", json!({"colors": [0, 2]})).unwrap();
    h.run_steps(2);
    assert!(control(&h, Role::CheckBox, "Create new layer").accesskit_node().is_disabled(), "a colour subset greys it");
    for (label, _) in &BOXES[..2] {
        assert!(!control(&h, Role::CheckBox, label).accesskit_node().is_disabled(), "{label} stays enabled");
    }
}

#[test]
fn the_boxes_ticked_on_the_bar_reach_the_removal_options() {
    let mut h = remove_bar();
    for label in ["Sample all layers", "Create new layer"] {
        control(&h, Role::CheckBox, label).click();
        h.run_steps(2);
    }
    let app = h.state_mut();
    let m = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: 30.0, y: 18.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: 30.0, y: 42.0 }, m);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let (id, p) = app.session.journal.last().cloned().unwrap();
    assert_eq!((id.as_str(), p["sampleAllLayers"].as_bool(), p["newLayer"].as_bool()), ("paint.remove", Some(true), Some(true)));
}
