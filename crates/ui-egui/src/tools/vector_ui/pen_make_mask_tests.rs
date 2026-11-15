//! #12904: the Pen options bar's Path mode shows "Make:" followed by Selection…, Mask and Shape,
//! as in Photoshop. Mask adds a vector mask made from the path to the active layer as one History
//! step, Add Vector Mask; the Background becomes Layer 0 inside that step.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::Path;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::vector_ui::{pen_down, pen_up};

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.ui.tool = Tool::Pen;
    let mut h = Harness::builder().with_size(vec2(1400.0, 400.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal(|ui| super::options_bar(app, ui, Tool::Pen));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(4);
    h
}

/// Clicks a closed square with the Pen and returns the Work Path it made.
fn square(h: &mut Harness<'static, PhotocraftApp>) -> Path {
    let app = h.state_mut();
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 120.0), (20.0, 120.0)] {
        pen_down(app, x, y, false);
        pen_up(app);
    }
    pen_down(app, 20.5, 20.5, false); // close
    pen_up(app);
    h.run_steps(2);
    h.state().session.active().unwrap().doc.work_path.clone().expect("a work path")
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn disabled(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

#[test]
fn make_row_is_a_caption_and_three_buttons_greyed_without_a_path() {
    let mut h = harness();
    let x = |label: &str| h.get_by_label(label).rect().min.x;
    let order = [x("Make:"), x("Selection…"), x("Mask"), x("Shape")];
    assert!(order.is_sorted_by(|a, b| a < b), "Make: Selection… Mask Shape, left to right: {order:?}");
    assert!(h.query_by_label("Make Selection").is_none() && h.query_by_label("Make Shape").is_none(), "the caption carries \"Make\"");
    for label in ["Selection…", "Mask", "Shape"] {
        assert!(disabled(&h, label), "{label}: no path yet");
    }
    square(&mut h);
    for label in ["Selection…", "Mask", "Shape"] {
        assert!(!disabled(&h, label), "{label}: the Work Path is the active path");
    }
    // Shape mode has no Make row.
    h.state_mut().ui.tool_options.vector_mode = "shape".into();
    h.run_steps(2);
    assert!(h.query_by_label("Make:").is_none() && h.query_by_label("Mask").is_none());
}

#[test]
fn mask_adds_a_vector_mask_of_the_path_to_a_pixel_layer_in_one_step() {
    let mut h = harness();
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    let drawn = square(&mut h);
    let (before, layer) = (steps(h.state()), h.state().session.active().unwrap().active_layer.unwrap());
    h.get_by_label("Mask").click();
    h.run_steps(3);
    let app = h.state();
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer, Some(layer), "the active layer is the one masked");
    assert_eq!(st.doc.layer(layer).unwrap().vector_mask.as_ref().map(|m| &m.path), Some(&drawn));
    assert_eq!(steps(app), before + 1, "one History step");
    assert_eq!(st.history.undo_label(), Some("Add Vector Mask"));
    assert_eq!(st.doc.work_path.as_ref(), Some(&drawn), "the Work Path stays");
    h.state_mut().session.undo();
    let st = h.state().session.active().unwrap();
    assert!(st.doc.layer(layer).unwrap().vector_mask.is_none(), "one Undo takes the mask off");
}

#[test]
fn mask_on_the_background_makes_it_layer_0_in_the_same_step() {
    let mut h = harness();
    let drawn = square(&mut h);
    let st = h.state().session.active().unwrap();
    let (before, layer) = (steps(h.state()), st.active_layer.unwrap());
    assert!(st.doc.is_background(layer));
    h.get_by_label("Mask").click();
    h.run_steps(3);
    let app = h.state();
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let l = st.doc.layer(layer).unwrap();
    assert!(!st.doc.is_background(layer), "no longer the Background");
    assert_eq!(l.name, "Layer 0");
    assert_eq!(l.vector_mask.as_ref().map(|m| &m.path), Some(&drawn));
    assert_eq!(steps(app), before + 1, "one History step");
    assert_eq!(st.history.undo_label(), Some("Add Vector Mask"));
    h.state_mut().session.undo();
    let st = h.state().session.active().unwrap();
    assert!(st.doc.is_background(layer) && st.doc.layer(layer).unwrap().vector_mask.is_none(), "one Undo gives the Background back");
}

/// A path still being drawn is finished into the Work Path first, as the canvas menu's Create
/// Vector Mask does, and then masks the layer.
#[test]
fn mask_finishes_a_path_being_drawn_first() {
    let mut h = harness();
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    for (x, y) in [(10.0, 150.0), (60.0, 150.0), (60.0, 190.0)] {
        pen_down(h.state_mut(), x, y, false);
        pen_up(h.state_mut());
    }
    h.run_steps(2);
    assert!(!disabled(&h, "Mask"));
    h.get_by_label("Mask").click();
    h.run_steps(3);
    let app = h.state();
    assert!(app.ui.pen.is_none(), "the draft was finished");
    let st = app.session.active().unwrap();
    let mask = st.active_layer.and_then(|id| st.doc.layer(id)).and_then(|l| l.vector_mask.as_ref()).expect("a vector mask");
    assert_eq!(mask.path.subpaths[0].knots.len(), 3);
    assert_eq!(st.history.undo_label(), Some("Add Vector Mask"));
}
