//! #12892: ↩ ends a Pen path and deselects the Work Path, as in Photoshop: it stays in the Paths
//! panel, no outline is drawn and the Make buttons are greyed, so the next path replaces it. Esc
//! keeps it selected, and so does ↩ into a saved path or a vector mask, or with the Curvature Pen.

use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::add_anchor_options_bar_tests::harness;
use super::pen_draw_tests::click;
use super::tests::overlay_shapes;
use super::{PathRow, path_rows, work_path_shown};
use crate::PhotocraftApp;
use crate::state::Tool;

fn draw_path(h: &mut Harness<'static, PhotocraftApp>, pts: [[f32; 2]; 2], key: Key) {
    for p in pts {
        click(h, p);
    }
    h.key_press(key);
    h.run_steps(2);
}

/// The anchors of each subpath of the Work Path.
fn work_anchors(h: &Harness<'static, PhotocraftApp>) -> Vec<Vec<[f64; 2]>> {
    let doc = &h.state().session.active().unwrap().doc;
    let work = doc.work_path.as_ref().expect("a work path");
    work.subpaths.iter().map(|s| s.knots.iter().map(|k| [k.anchor.x.round(), k.anchor.y.round()]).collect()).collect()
}

fn listed(h: &Harness<'static, PhotocraftApp>) -> bool {
    let st = h.state().session.active().unwrap();
    path_rows(&st.doc, st.active_layer).iter().any(|r| r.kind == PathRow::Work)
}

fn make_buttons_enabled(h: &Harness<'static, PhotocraftApp>) -> [bool; 2] {
    ["Selection…", "Shape"].map(|label| !h.get_by_label(label).accesskit_node().is_disabled())
}

#[test]
fn enter_deselects_the_work_path_and_the_next_path_replaces_it() {
    let mut h = harness(Tool::Pen);
    draw_path(&mut h, [[20.0, 20.0], [120.0, 20.0]], Key::Enter);
    assert!(h.state().ui.pen.is_none(), "drawing ended");
    assert_eq!(work_anchors(&h), [[[20.0, 20.0], [120.0, 20.0]]]);
    assert!(listed(&h), "the Work Path is still in the Paths panel");
    assert_eq!(h.state().ui.selected_path, None, "but not selected");
    assert!(!work_path_shown(h.state()), "the Work Path is hidden");
    assert_eq!(overlay_shapes(h.state(), &egui::Context::default()), 0, "no outline on the canvas");
    assert_eq!(make_buttons_enabled(&h), [false, false], "the Make buttons are greyed");

    draw_path(&mut h, [[30.0, 60.0], [150.0, 80.0]], Key::Enter);
    assert_eq!(work_anchors(&h), [[[30.0, 60.0], [150.0, 80.0]]], "the new path replaced the old one");
    assert_eq!(h.state().ui.selected_path, None, "and is deselected in turn");
}

#[test]
fn escape_keeps_the_work_path_selected_and_the_next_path_joins_it() {
    let mut h = harness(Tool::Pen);
    draw_path(&mut h, [[20.0, 20.0], [120.0, 20.0]], Key::Escape);
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("work"));
    assert!(work_path_shown(h.state()));
    assert!(overlay_shapes(h.state(), &egui::Context::default()) > 0, "the outline stays");
    assert_eq!(make_buttons_enabled(&h), [true, true]);

    draw_path(&mut h, [[30.0, 60.0], [150.0, 80.0]], Key::Escape);
    assert_eq!(work_anchors(&h), [vec![[20.0, 20.0], [120.0, 20.0]], vec![[30.0, 60.0], [150.0, 80.0]]], "a second subpath");
}

#[test]
fn enter_keeps_a_selected_saved_path_selected() {
    let mut h = harness(Tool::Pen);
    let saved = json!({"subpaths": [{"closed": false, "knots": [{"anchor": [10, 90]}, {"anchor": [60, 90]}]}]});
    h.state_mut().run("path.set", json!({"name": "Saved", "path": saved})).unwrap();
    h.state_mut().ui.selected_path = Some("Saved".into());
    h.run_steps(2);
    draw_path(&mut h, [[20.0, 20.0], [120.0, 20.0]], Key::Enter);
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(doc.paths.iter().find(|p| p.name == "Saved").map(|p| p.path.subpaths.len()), Some(2), "the path joined the saved path");
    assert!(doc.work_path.is_none());
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("Saved"), "which stays selected");
    assert_eq!(make_buttons_enabled(&h), [true, true]);
}

#[test]
fn enter_into_a_vector_mask_keeps_the_work_path_selected() {
    let mut h = harness(Tool::Pen);
    draw_path(&mut h, [[20.0, 20.0], [120.0, 20.0]], Key::Escape);
    let app = h.state_mut();
    app.run("layer.new.layer", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let mask = json!({"subpaths": [{"closed": true, "knots": [[150, 70], [190, 70], [190, 95]]}]});
    app.run("layer.vectorMask.add", json!({"layer": id.0, "path": mask})).unwrap();
    app.ui.vector_mask_target = true;
    h.run_steps(2);
    draw_path(&mut h, [[30.0, 60.0], [100.0, 80.0]], Key::Enter);
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(doc.layer(id).unwrap().vector_mask.as_ref().map(|m| m.path.subpaths.len()), Some(2), "the path went into the mask");
    assert_eq!(work_anchors(&h), [[[20.0, 20.0], [120.0, 20.0]]], "not into the Work Path");
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("work"), "which stays selected");
    assert!(work_path_shown(h.state()));
}

#[test]
fn enter_with_the_curvature_pen_keeps_the_work_path_selected() {
    let mut h = harness(Tool::CurvaturePen);
    draw_path(&mut h, [[20.0, 20.0], [120.0, 20.0]], Key::Enter);
    assert!(h.state().ui.pen.is_none(), "drawing ended");
    assert_eq!(work_anchors(&h).len(), 1);
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("work"), "not measured: today's behaviour");
    assert!(work_path_shown(h.state()));
}

/// Not measured: with the Work Path deselected, Delete with the Pen reaches Delete Layer, as after
/// ⌘↩ (Make Selection), instead of deleting the path as after Esc (#5702).
#[test]
fn delete_after_enter_deletes_the_layer_not_the_work_path() {
    let mut h = harness(Tool::Pen);
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(2);
    let layers = |h: &Harness<'static, PhotocraftApp>| h.state().session.active().unwrap().doc.layers.len();
    let before = layers(&h);
    draw_path(&mut h, [[20.0, 20.0], [120.0, 20.0]], Key::Enter);
    h.key_press(Key::Delete);
    h.run_steps(2);
    assert_eq!(layers(&h), before - 1, "the layer is deleted");
    assert_eq!(work_anchors(&h), [[[20.0, 20.0], [120.0, 20.0]]], "the Work Path stays");
}
