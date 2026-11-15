//! #12906: the Pen's Mode dropdown lists Shape, Path and Pixels, in that order, as in Photoshop.
//! Pixels is greyed and can't be picked, by the pointer or the keys; the default stays Path, and
//! Reset Tool sets Path again. The Freeform and Curvature Pens keep their Path, Shape list.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::add_anchor_options_bar_tests::harness;
use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

/// The options bar's Mode dropdown (the topmost one named Mode).
fn combo<'a>(h: &'a Harness<'static, PhotocraftApp>) -> egui_kittest::Node<'a> {
    let combos = h.query_all_by_role(Role::ComboBox).filter(|n| n.accesskit_node().label().as_deref() == Some("Mode"));
    combos.min_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).expect("the Mode dropdown")
}

/// Opens the options bar's Mode dropdown.
fn open(h: &mut Harness<'static, PhotocraftApp>) {
    combo(h).click();
    h.run_steps(3);
    assert!(crate::widgets::dropdown_list_open(&h.ctx), "the list is open");
}

/// The open list's rows top to bottom (the mode buttons below the dropdown), each with whether
/// it is greyed.
fn rows(h: &Harness<'static, PhotocraftApp>) -> Vec<(String, bool)> {
    let combo = combo(h).rect();
    let mut rows: Vec<_> = h
        .query_all_by_role(Role::Button)
        .filter(|n| n.rect().top() >= combo.bottom() && n.accesskit_node().label().is_some_and(|l| ["Shape", "Path", "Pixels"].contains(&l.as_str())))
        .map(|n| (n.rect().top(), n.accesskit_node().label().unwrap_or_default(), n.accesskit_node().is_disabled()))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, label, greyed)| (label, greyed)).collect()
}

/// Clicks the open list's row `label`.
fn pick(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    let combo = combo(h).rect();
    let row = h.query_all_by_label(label).find(|n| n.rect().top() >= combo.bottom()).expect("the row");
    row.click();
    h.run_steps(3);
}

fn mode(h: &Harness<'static, PhotocraftApp>) -> String {
    h.state().ui.tool_options.vector_mode.clone()
}

#[test]
fn the_pens_list_is_shape_path_and_a_greyed_pixels() {
    let mut h = harness(Tool::Pen);
    open(&mut h);
    let expected = [("Shape".to_string(), false), ("Path".to_string(), false), ("Pixels".to_string(), true)];
    assert_eq!(rows(&h), expected);
}

#[test]
fn the_default_mode_is_path_and_reset_tool_sets_it_again() {
    assert_eq!(ToolOptions::default().vector_mode, "path");
    let mut h = harness(Tool::Pen);
    assert_eq!(mode(&h), "path");
    assert_eq!(combo(&h).value().as_deref(), Some("Path"), "the button shows Path");

    open(&mut h);
    pick(&mut h, "Shape");
    assert_eq!(mode(&h), "shape", "Shape can be picked");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    assert_eq!(mode(&h), "path", "Reset Tool from Shape mode");
}

#[test]
fn clicking_pixels_leaves_the_mode() {
    let mut h = harness(Tool::Pen);
    open(&mut h);
    pick(&mut h, "Pixels");
    assert_eq!(mode(&h), "path");
}

#[test]
fn the_keys_never_land_on_pixels() {
    let mut h = harness(Tool::Pen);
    open(&mut h);
    // ↓ from Path, the last enabled row, would be Pixels.
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(2);
    assert_eq!(mode(&h), "path", "↓");
    // The next row starting with P after Path is Pixels.
    h.key_press(egui::Key::P);
    h.event(egui::Event::Text("p".into()));
    h.run_steps(2);
    assert_eq!(mode(&h), "path", "type-ahead P");
    // The keys still step between the enabled rows.
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(2);
    assert_eq!(mode(&h), "shape", "↑");
}

#[test]
fn the_freeform_and_curvature_pens_keep_path_and_shape() {
    for tool in [Tool::FreeformPen, Tool::CurvaturePen] {
        let mut h = harness(tool);
        open(&mut h);
        let expected = [("Path".to_string(), false), ("Shape".to_string(), false)];
        assert_eq!(rows(&h), expected, "{tool:?}");
    }
}
