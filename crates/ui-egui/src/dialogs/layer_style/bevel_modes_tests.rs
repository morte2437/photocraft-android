//! #9368: Bevel & Emboss's Highlight Mode and Shadow Mode are two dropdowns, each changing its own
//! mode, as in Photoshop. They shared one widget id, so the Shadow Mode list opened both lists and
//! a pick changed the Highlight Mode.
use egui::vec2;
use egui_kittest::kittest::Queryable;

use super::*;

#[test]
fn highlight_and_shadow_modes_are_independent_dropdowns() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.layerStyle.bevelEmboss", json!({})).unwrap();
    let fields = {
        let state = app.session.active().unwrap();
        initial_fields(state.doc.layer(state.active_layer.unwrap()).unwrap(), Some("bevelEmboss"), 120.0)
    };
    let mut harness = egui_kittest::Harness::builder().with_size(vec2(1000.0, 1200.0)).build_ui_state(
        |ui, state: &mut (PhotocraftApp, Map<String, Value>)| {
            if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                body(&mut state.0, ui, &mut state.1);
            }
        },
        (app, fields),
    );
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
    harness.run_steps(3);
    let params = |h: &egui_kittest::Harness<'_, (PhotocraftApp, Map<String, Value>)>| h.state().1["effects"][0]["params"].clone();
    assert_eq!((params(&harness)["highlightBlend"].as_str(), params(&harness)["shadowBlend"].as_str()), (Some("Screen"), Some("Multiply")));
    // Shadow Mode's list: one list opens, and its pick is the Shadow Mode's.
    harness.get_by_value("Multiply").click();
    harness.run_steps(3);
    assert_eq!(harness.query_all_by_label("Dissolve").count(), 1, "only the Shadow Mode list is open");
    harness.get_by_label("Dissolve").click();
    harness.run_steps(3);
    assert_eq!(params(&harness)["shadowBlend"], "Dissolve");
    assert_eq!(params(&harness)["highlightBlend"], "Screen", "the Highlight Mode is unchanged");
    // And the Highlight Mode's list changes the Highlight Mode.
    harness.get_by_value("Screen").click();
    harness.run_steps(3);
    harness.get_by_label("Overlay").click();
    harness.run_steps(3);
    assert_eq!(params(&harness)["highlightBlend"], "Overlay");
    assert_eq!(params(&harness)["shadowBlend"], "Dissolve");
}
