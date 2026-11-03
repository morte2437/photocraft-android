//! #10878: the Kind and Mode filters list the same layers as the reference application. Fill
//! layers (solid colour without a vector shape, gradient, pattern) match no Kind button, a solid
//! colour fill with a vector mask lists as a shape, and the Mode list has no Pass Through.

use photocraft_color::BlendMode;
use photocraft_doc::Document;
use serde_json::json;

use super::{Filter, FilterBy, KINDS};
use crate::state::UiState;

fn new_session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s
}

/// Make a layer with `cmd`, name it `name` and return its id.
fn make(s: &mut photocraft_engine::Session, cmd: &str, params: serde_json::Value, name: &str) -> u64 {
    let id = s.execute(cmd, params).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": id, "name": name})).unwrap();
    id
}

/// The names `filter` lists, top to bottom.
fn listed(doc: &Document, filter: &Filter) -> Vec<String> {
    crate::layer_tree_ui::display_rows(doc, filter.active()).into_iter().filter(|(_, l)| filter.shows(l)).map(|(_, l)| l.name.clone()).collect()
}

fn kinds(kinds: &[&str]) -> Filter {
    Filter::of(&UiState { layer_filter_by: FilterBy::Kind, layer_filter: kinds.iter().map(|k| k.to_string()).collect(), ..UiState::default() })
}

fn mode(mode: BlendMode) -> Filter {
    Filter::of(&UiState { layer_filter_by: FilterBy::Mode, layer_filter_mode: mode, ..UiState::default() })
}

/// Every kind of fill layer, a solid fill with a vector mask, and a Gradient Map adjustment.
fn fills() -> photocraft_engine::Session {
    let mut s = new_session();
    make(&mut s, "layer.newFillLayer.solidColor", json!({"color": "#112233"}), "Solid");
    let shaped = make(&mut s, "layer.newFillLayer.solidColor", json!({"color": "#445566"}), "Solid shape");
    s.execute("layer.vectorMask.revealAll", json!({"layer": shaped})).unwrap();
    make(&mut s, "layer.newFillLayer.gradient", json!({}), "Gradient");
    make(&mut s, "layer.newFillLayer.pattern", json!({"pattern": "Checkerboard"}), "Pattern");
    make(&mut s, "layer.newAdjustmentLayer.gradientMap", json!({}), "Gradient Map");
    s
}

#[test]
fn fill_layers_match_no_kind_button_and_a_shaped_solid_fill_is_a_shape() {
    let s = fills();
    let doc = &s.active().unwrap().doc;
    // Each button on its own: the measured table, row by row.
    assert_eq!(listed(doc, &kinds(&["adjustment"])), ["Gradient Map"], "only the adjustment layer is an adjustment");
    assert_eq!(listed(doc, &kinds(&["shape"])), ["Solid shape"], "a solid fill with a vector mask lists as a shape");
    assert!(listed(doc, &kinds(&["pixel"])).iter().all(|n| n == "Background"), "no fill layer is a pixel layer");
    // With all five buttons on, the plain solid, gradient and pattern fills are still not listed.
    let all = listed(doc, &kinds(&KINDS));
    for fill in ["Solid", "Gradient", "Pattern"] {
        assert!(!all.iter().any(|n| n == fill), "{fill} matches no Kind button: {all:?}");
    }
    assert!(all.iter().any(|n| n == "Solid shape") && all.iter().any(|n| n == "Gradient Map"));
}

#[test]
fn a_normal_group_is_listed_by_mode_and_a_pass_through_group_never_is() {
    let mut s = new_session();
    make(&mut s, "layer.new.layer", json!({}), "Child");
    let normal = s.execute("layer.groupLayers", json!({"name": "Normal group"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": normal, "blend": "Normal"})).unwrap();
    let child = {
        let doc = &s.active().unwrap().doc;
        doc.walk().into_iter().find(|(_, _, l)| l.name == "Child").map(|(_, _, l)| l.id.0).unwrap()
    };
    s.execute("layer.setProps", json!({"layer": child, "blend": "Multiply"})).unwrap();
    make(&mut s, "layer.new.layer", json!({}), "Other");
    s.execute("layer.groupLayers", json!({"name": "Pass group"})).unwrap();
    let doc = &s.active().unwrap().doc;
    let pass = doc.walk().into_iter().find(|(_, _, l)| l.name == "Pass group").map(|(_, _, l)| l.blend).unwrap();
    assert_eq!(pass, BlendMode::PassThrough, "a new group passes through");

    let normal_rows = listed(doc, &mode(BlendMode::Normal));
    assert!(normal_rows.iter().any(|n| n == "Normal group"), "a Normal group is listed under Normal: {normal_rows:?}");
    assert!(!normal_rows.iter().any(|n| n == "Child"), "its Multiply child is not");
    assert!(listed(doc, &mode(BlendMode::Multiply)).iter().any(|n| n == "Child"), "a Multiply layer inside a group is listed under Multiply");
    assert!(!normal_rows.iter().any(|n| n == "Pass group"), "a Pass Through group is not");
    for m in BlendMode::LAYER_MODES {
        assert!(!listed(doc, &mode(m)).iter().any(|n| n == "Pass group"), "no mode lists the Pass Through group ({m:?})");
    }
}

#[test]
fn the_mode_filter_takes_no_pass_through() {
    let ui = UiState::default();
    let err = super::patched(&ui, &json!({"mode": "Pass Through"})).unwrap_err();
    assert!(err.contains("Pass Through"), "the error names what was asked for: {err}");
    for m in BlendMode::LAYER_MODES {
        assert!(err.contains(m.label()), "the error names the valid mode {}: {err}", m.label());
    }
    // The 27 layer modes still work.
    assert_eq!(BlendMode::LAYER_MODES.len(), 27);
    assert_eq!(super::patched(&ui, &json!({"mode": "Multiply"})).unwrap().mode, BlendMode::Multiply);
    assert_eq!(super::patched(&ui, &json!({"mode": "Normal"})).unwrap().mode, BlendMode::Normal);
}

#[test]
fn a_stored_pass_through_filter_mode_loads_as_normal() {
    let mut stored = serde_json::to_value(UiState::default()).unwrap();
    stored["layer_filter_mode"] = serde_json::to_value(BlendMode::PassThrough).unwrap();
    let ui: UiState = serde_json::from_value(stored.clone()).unwrap();
    assert_eq!(ui.layer_filter_mode, BlendMode::Normal);
    // Any other stored mode is kept as it was.
    stored["layer_filter_mode"] = serde_json::to_value(BlendMode::Screen).unwrap();
    let ui: UiState = serde_json::from_value(stored).unwrap();
    assert_eq!(ui.layer_filter_mode, BlendMode::Screen);
}
