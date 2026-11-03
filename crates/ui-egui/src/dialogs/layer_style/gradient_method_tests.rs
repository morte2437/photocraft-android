use egui_kittest::kittest::Queryable;

use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app
}

fn effect(app: &PhotocraftApp) -> &Effect {
    let state = app.session.active().unwrap();
    &state.doc.layer(state.active_layer.unwrap()).unwrap().effects.items[0]
}

fn fields(app: &PhotocraftApp) -> Map<String, Value> {
    let state = app.session.active().unwrap();
    initial_fields(state.doc.layer(state.active_layer.unwrap()).unwrap(), Some("gradientOverlay"), 120.0)
}

#[test]
fn method_picker_is_pending_until_confirm_and_survives_reopening() {
    let mut app = app();
    app.run("layer.layerStyle.gradientOverlay", json!({"from": "#ff0000", "to": "#0000ff"})).unwrap();
    let original = effect(&app).clone();
    let pending_fields = fields(&app);
    let mut harness = egui_kittest::Harness::builder().with_size(vec2(1000.0, 1000.0)).build_ui_state(
        |ui, state: &mut (PhotocraftApp, Map<String, Value>)| {
            // The harness draws once before setup_context installs the app fonts.
            if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                body(&mut state.0, ui, &mut state.1);
            }
        },
        (app, pending_fields),
    );
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
    harness.run_steps(3);
    harness.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Method").click();
    harness.run_steps(2);
    harness.get_by_label("Smooth").click();
    harness.run_steps(2);
    assert_eq!(effect(&harness.state().0), &original, "cancel can still discard all pending edits");
    assert_eq!(harness.state().1["effects"][0]["params"]["method"], "smooth");
    let (app, pending) = harness.state_mut();
    confirm(app, pending).unwrap();
    let saved = effect(app).clone();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(effect(app), &original);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(effect(app), &saved);
    let reopened = fields(app);
    assert_eq!(reopened["effects"][0]["params"]["method"], "smooth");
    assert_eq!(reopened["effects"][0]["params"]["stops"][0][1], "#ff0000");
    confirm(app, &reopened).unwrap();
    assert_eq!(effect(app), &saved, "OK without edits leaves the rendered curve exact");
}

#[test]
fn all_four_methods_roundtrip_through_the_dialog() {
    for method in ["classic", "linear", "perceptual", "smooth"] {
        let mut app = app();
        app.run("layer.layerStyle.gradientOverlay", json!({"from": [0.123456, 0.234567, 0.345678], "to": "#abcdef", "method": method})).unwrap();
        let before = effect(&app).clone();
        let fields = fields(&app);
        assert_eq!(fields["effects"][0]["params"]["method"], method);
        confirm(&mut app, &fields).unwrap();
        assert_eq!(effect(&app), &before);
    }
}

#[test]
fn baked_overlay_is_unchanged_until_method_selection_then_edits_the_whole_curve() {
    use photocraft_color::Color;
    use photocraft_io::{GradientMethod, bake_gradient};

    let mut app = app();
    app.run("layer.layerStyle.gradientOverlay", json!({})).unwrap();
    // PSD import and older method commands leave a baked curve without a source record.
    let state = app.session.active_mut().unwrap();
    let fx = &mut std::sync::Arc::make_mut(&mut state.doc).layer_mut(state.active_layer.unwrap()).unwrap().effects.items[0];
    let Effect::GradientOverlay { gradient, interpolation, .. } = fx else { panic!("expected overlay") };
    gradient.stops = bake_gradient(vec![(0.0, Color::BLACK), (1.0, Color::WHITE)], &[0.5], 1.0, GradientMethod::Smooth);
    *interpolation = None;
    let source_ends = vec![(0.0, gradient.stops.first().unwrap().1), (1.0, gradient.stops.last().unwrap().1)];

    let original = effect(&app).clone();
    let mut pending = fields(&app);
    let id = pending["effects"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(pending["effects"][0]["params"]["method"], "custom");
    confirm(&mut app, &pending).unwrap();
    assert_eq!(effect(&app), &original);

    set_param(&mut pending, &id, "method", json!("linear"));
    confirm(&mut app, &pending).unwrap();
    let selected = effect(&app).clone();
    let Effect::GradientOverlay { gradient, interpolation, .. } = &selected else { panic!("expected overlay") };
    assert_eq!(gradient.stops, bake_gradient(source_ends, &[0.5], 1.0, GradientMethod::Linear));
    assert_eq!(interpolation.as_ref().unwrap().stops.len(), 2);

    let mut reopened = fields(&app);
    assert_eq!(reopened["effects"][0]["params"]["method"], "linear");
    set_param(&mut reopened, &id, "from", json!("#ff0000"));
    set_param(&mut reopened, &id, "to", json!("#0000ff"));
    confirm(&mut app, &reopened).unwrap();
    let edited = effect(&app).clone();
    let Effect::GradientOverlay { gradient, .. } = &edited else { panic!("expected overlay") };
    assert_eq!(gradient.stops, bake_gradient(vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))], &[0.5], 1.0, GradientMethod::Linear));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(effect(&app), &selected);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(effect(&app), &edited);
    let reopened = fields(&app);
    confirm(&mut app, &reopened).unwrap();
    assert_eq!(effect(&app), &edited);
}
