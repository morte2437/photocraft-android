//! #12959: the Smudge options bar's Mode dropdown (Normal, Darken, Lighten, Hue, Saturation,
//! Color, Luminosity), its tooltip, what the stroke sends and what Reset Tool does. Blur and
//! Sharpen share the bar's branch and keep theirs.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_color::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const NAMES: [&str; 7] = ["Normal", "Darken", "Lighten", "Hue", "Saturation", "Color", "Luminosity"];

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    app
}

fn bar(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder()
        .with_size(vec2(1600.0, 400.0))
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ProMedium))
        .build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app(tool));
    h.run_steps(4);
    h
}

/// The bar's Mode dropdown, if it has one.
fn mode<'a>(h: &'a Harness<'static, PhotocraftApp>) -> Option<egui_kittest::Node<'a>> {
    h.query_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().label().as_deref() == Some("Mode:"))
}

#[test]
fn the_smudge_bar_lists_the_seven_modes_in_order_and_starts_at_normal() {
    let mut h = bar(Tool::Smudge);
    assert_eq!(h.state().ui.tool_options.smudge_mode, BlendMode::Normal);
    let dropdown = mode(&h).expect("Smudge has a Mode dropdown");
    assert_eq!(dropdown.accesskit_node().value().as_deref(), Some("Normal"));
    // It comes before Strength, right after the brush controls.
    let strength = h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Strength:")).expect("Strength");
    assert!(dropdown.rect().right() <= strength.rect().left(), "Mode is left of Strength");
    dropdown.click();
    h.run_steps(4);
    assert!(egui::Popup::is_any_open(&h.ctx), "the list opens");
    let tops: Vec<f32> = NAMES.iter().map(|name| h.query_all_by_label(name).map(|n| n.rect().top()).fold(f32::MIN, f32::max)).collect();
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "the entries are in the reference order: {tops:?}");
    for other in ["Multiply", "Screen", "Overlay", "Difference", "Dissolve"] {
        assert!(h.query_by_label(other).is_none(), "{other} is not a Smudge mode");
    }
    h.get_by_label("Luminosity").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.tool_options.smudge_mode, BlendMode::Luminosity);
}

#[test]
fn the_list_has_a_line_above_darken_and_above_hue() {
    assert_eq!(crate::retouch_ui::SMUDGE_MODES.map(BlendMode::label), NAMES);
    let grouped: Vec<BlendMode> = crate::retouch_ui::SMUDGE_MODES.into_iter().filter(|m| crate::retouch_ui::smudge_mode_starts_group(*m)).collect();
    assert_eq!(grouped, [BlendMode::Darken, BlendMode::Hue]);
}

#[test]
fn hovering_the_dropdown_shows_painting_mode() {
    let mut h = bar(Tool::Smudge);
    assert!(h.query_by_label("Painting Mode").is_none(), "no tooltip before hover");
    let center = mode(&h).expect("Mode").rect().center();
    h.hover_at(center);
    h.run_steps(60);
    assert!(h.query_by_label("Painting Mode").is_some(), "the tooltip shows");
}

#[test]
fn blur_and_sharpen_keep_their_bar() {
    for tool in [Tool::Blur, Tool::Sharpen] {
        let h = bar(tool);
        assert!(mode(&h).is_none(), "{tool:?} has no Mode dropdown here");
    }
}

#[test]
fn the_stroke_sends_the_mode_and_only_smudge_does() {
    let mut app = app(Tool::Smudge);
    let (cmd, p) = crate::retouch_ui::dab_params(&app, Tool::Smudge).unwrap();
    assert_eq!((cmd, &p["mode"]), ("paint.smudge", &json!("Normal")));
    for (mode, name) in crate::retouch_ui::SMUDGE_MODES.into_iter().zip(NAMES) {
        app.ui.tool_options.smudge_mode = mode;
        let (_, p) = crate::retouch_ui::dab_params(&app, Tool::Smudge).unwrap();
        assert_eq!(p["mode"], json!(name));
        // The engine takes every name the bar sends.
        let mut stroke = p.clone();
        stroke["points"] = json!([[20, 40], [60, 40]]);
        app.run(cmd, stroke).unwrap_or_else(|e| panic!("{name}: {e}"));
        for tool in [Tool::Blur, Tool::Sharpen] {
            let (_, p) = crate::retouch_ui::dab_params(&app, tool).unwrap();
            assert!(p.get("mode").is_none(), "{tool:?} sends no mode");
        }
    }
}

#[test]
fn reset_tool_sets_normal_and_other_tools_leave_it() {
    let mut app = app(Tool::Smudge);
    app.ui.tool_options.smudge_mode = BlendMode::Hue;
    for tool in [Tool::Blur, Tool::Sharpen, Tool::Sponge] {
        crate::tool_reset::reset_options(&mut app.ui.tool_options, tool);
        assert_eq!(app.ui.tool_options.smudge_mode, BlendMode::Hue, "resetting {tool:?} keeps the Smudge mode");
    }
    crate::tool_reset::reset_options(&mut app.ui.tool_options, Tool::Smudge);
    assert_eq!(app.ui.tool_options.smudge_mode, BlendMode::Normal);
}
