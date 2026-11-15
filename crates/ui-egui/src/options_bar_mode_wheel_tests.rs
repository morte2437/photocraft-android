//! #2061: the scroll wheel over the options bar's Mode dropdown (Brush, Pencil, Gradient, Paint
//! Bucket, the stamps, History Brush, Healing) steps through the blend modes, as over the Layers
//! panel's (#1747) and as in Photoshop.

use egui::accesskit::Role;
use egui::{Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The app with a new document and `tool` chosen.
pub(super) fn harness(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.ui.tool = tool;
        app
    });
    h.run_steps(8);
    h
}

/// The options bar's Mode dropdown: the highest one showing `Normal` (the Layers panel's is lower).
pub(super) fn mode_dropdown(h: &Harness<'_, PhotocraftApp>) -> Rect {
    h.query_all_by_role(Role::ComboBox).filter(|n| n.value().as_deref() == Some("Normal")).map(|n| n.rect()).min_by(|a, b| a.top().total_cmp(&b.top())).unwrap()
}

/// `lines` wheel lines at `at` in one event: negative is down.
pub(super) fn wheel(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, lines: f32) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, lines),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    });
    h.run_steps(3);
}

pub(super) fn brush_mode(h: &Harness<'_, PhotocraftApp>) -> BlendMode {
    h.state().session.tools.brush.mode
}

#[test]
fn the_wheel_over_the_brush_mode_dropdown_steps_the_painting_mode() {
    let mut h = harness(Tool::Brush);
    let at = mode_dropdown(&h).center();
    assert_eq!(brush_mode(&h), BlendMode::Normal);
    wheel(&mut h, at, -1.0);
    assert_eq!(brush_mode(&h), BlendMode::Dissolve);
    wheel(&mut h, at, -1.0);
    assert!(h.state().session.tools.brush.behind, "Behind follows Dissolve");
    wheel(&mut h, at, -1.0);
    assert!(h.state().ui.tool_options.brush_clear, "Clear follows Behind");
    assert!(!h.state().session.tools.brush.behind);
    wheel(&mut h, at, -1.0);
    assert!(!h.state().ui.tool_options.brush_clear);
    assert_eq!(brush_mode(&h), BlendMode::Darken);
    wheel(&mut h, at, 4.0);
    assert_eq!(brush_mode(&h), BlendMode::Normal, "up steps back and stops at the first mode");
}

#[test]
fn the_wheel_over_the_pencil_mode_dropdown_steps_the_painting_mode() {
    let mut h = harness(Tool::Pencil);
    let at = mode_dropdown(&h).center();
    wheel(&mut h, at, -1.0);
    assert_eq!(brush_mode(&h), BlendMode::Dissolve);
    wheel(&mut h, at, 1.0);
    assert_eq!(brush_mode(&h), BlendMode::Normal);
}

#[test]
fn the_wheel_over_the_gradient_mode_dropdown_steps_its_blend_mode() {
    let mut h = harness(Tool::Gradient);
    // Only the Classic gradient has a blend Mode (#12007).
    h.state_mut().ui.tool_options.gradient_classic = true;
    h.run_steps(4);
    let at = mode_dropdown(&h).center();
    wheel(&mut h, at, -2.0);
    assert_eq!(h.state().ui.tool_options.gradient_blend_mode, BlendMode::Darken);
    wheel(&mut h, at, 1.0);
    assert_eq!(h.state().ui.tool_options.gradient_blend_mode, BlendMode::Dissolve);
}

/// The stamps, the History Brush and the Healing Brush have a Mode dropdown of their own brush's
/// mode; the Healing Brush's skips the modes it doesn't have, and the Pattern Stamp's has Behind
/// after Dissolve (#12879).
#[test]
fn retouch_tools_have_a_mode_dropdown() {
    for (tool, after_two) in [(Tool::CloneStamp, BlendMode::Darken), (Tool::HistoryBrush, BlendMode::Darken), (Tool::Healing, BlendMode::Screen)] {
        let mut h = harness(tool);
        let at = mode_dropdown(&h).center();
        wheel(&mut h, at, -2.0);
        assert_eq!(brush_mode(&h), after_two, "{tool:?}");
        assert_eq!(h.state().ui.tool, tool);
    }
}

/// The Pattern Stamp's Mode dropdown has Behind after Dissolve and no Clear (#12879): the wheel
/// steps Normal, Dissolve, Behind, Darken, and stepping on leaves Behind. On a pixel layer: on the
/// Background Behind is greyed and the wheel passes it.
#[test]
fn the_pattern_stamp_mode_dropdown_has_behind() {
    let mut h = harness(Tool::PatternStamp);
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(4);
    let at = mode_dropdown(&h).center();
    wheel(&mut h, at, -1.0);
    assert_eq!(brush_mode(&h), BlendMode::Dissolve);
    assert!(!h.state().session.tools.brush.behind);
    wheel(&mut h, at, -1.0);
    assert!(h.state().session.tools.brush.behind, "Behind follows Dissolve");
    assert_eq!(brush_mode(&h), BlendMode::Normal);
    wheel(&mut h, at, -1.0);
    assert!(!h.state().session.tools.brush.behind, "Darken follows Behind: no Clear");
    assert_eq!(brush_mode(&h), BlendMode::Darken);
    wheel(&mut h, at, 1.0);
    assert!(h.state().session.tools.brush.behind, "and back");
    assert_eq!(h.state().ui.tool, Tool::PatternStamp);
}

/// The centre of a dropdown showing `value`.
fn combo(h: &Harness<'_, PhotocraftApp>, value: &str) -> Option<Pos2> {
    h.query_all_by_role(Role::ComboBox).find(|n| n.value().as_deref() == Some(value)).map(|n| n.rect().center())
}

/// In an Indexed Color document the Pattern Stamp's Mode reads "Threshold" for Normal (#12887); the
/// other entries weren't measured there, so the list and the wheel stay as today.
#[test]
fn the_pattern_stamp_mode_reads_threshold_in_an_indexed_document() {
    let mut h = harness(Tool::PatternStamp);
    h.state_mut().run("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none"})).unwrap();
    h.run_steps(4);
    let at = combo(&h, "Threshold").expect("Mode reads Threshold");
    wheel(&mut h, at, -1.0);
    assert_eq!(brush_mode(&h), BlendMode::Dissolve);
    assert!(combo(&h, "Dissolve").is_some() && combo(&h, "Threshold").is_none(), "Dissolve reads as itself");
    wheel(&mut h, at, 1.0);
    assert_eq!(brush_mode(&h), BlendMode::Normal);
    assert!(combo(&h, "Threshold").is_some());
}

/// The Paint Bucket's Mode dropdown has Clear after Dissolve, as the Brush's does.
#[test]
fn the_paint_bucket_mode_dropdown_has_clear() {
    let mut h = harness(Tool::PaintBucket);
    let at = mode_dropdown(&h).center();
    wheel(&mut h, at, -1.0);
    assert_eq!(h.state().ui.tool_options.bucket_blend_mode, BlendMode::Dissolve);
    wheel(&mut h, at, -1.0);
    assert!(h.state().ui.tool_options.bucket_clear);
    wheel(&mut h, at, -1.0);
    assert!(!h.state().ui.tool_options.bucket_clear);
    assert_eq!(h.state().ui.tool_options.bucket_blend_mode, BlendMode::Darken);
}
