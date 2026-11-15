//! #11813: the Brush options-bar controls carry the reference application's tooltips (also their
//! accessible descriptions), the Eraser keeps today's texts, and the smoothing options popup has
//! a "Smoothing Options" title.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable, by};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::ThemeKind;

fn bar(tool: Tool) -> Harness<'static, PhotocraftApp> {
    bar_in(tool, ThemeKind::ProMedium)
}

fn bar_in(tool: Tool, theme: ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(4);
    h
}

/// The descriptions of the widgets labelled `label`, or, for the controls a caption names (Mode
/// and the fields), of every control of that kind.
fn descriptions(h: &Harness<'static, PhotocraftApp>, label: &str) -> Vec<String> {
    let of = |n: egui_kittest::Node<'_>| n.accesskit_node().description().map(|d| d.to_string());
    if label == "Mode" {
        return h.query_all_by_role(egui::accesskit::Role::ComboBox).filter_map(of).collect();
    }
    // The fields too are named by the caption before them.
    if ["Opacity", "Flow", "Smoothing"].contains(&label) {
        return h.query_all_by_role(egui::accesskit::Role::SpinButton).filter_map(of).collect();
    }
    // The non-Pro pressure switches are unnamed nodes: every description in the bar.
    if label.starts_with("Pressure for") {
        return h.query_all(by().predicate(|n| n.description().is_some())).filter_map(of).collect();
    }
    h.query_all_by_label(label).filter_map(of).collect()
}

const BRUSH: [(&str, &str); 8] = [
    ("Brush Preset picker", "Click to open the Brush Preset picker"),
    ("Mode", "Painting Mode"),
    ("Opacity", "Set opacity for stroke"),
    ("Always use pressure for opacity", "Always use Pressure for Opacity. When off, Brush Preset controls pressure"),
    ("Flow", "Set flow rate for stroke"),
    ("Smoothing", "Set smoothing for stroke\nUse higher values to reduce the shakiness of brush strokes"),
    ("Always use pressure for size", "Always use Pressure for Size. When off, Brush Preset controls pressure"),
    ("Set painting symmetry options", "Set symmetry options for Painting"),
];

#[test]
fn the_brush_controls_have_the_reference_tooltips() {
    let h = bar(Tool::Brush);
    for (label, tip) in BRUSH {
        let found = descriptions(&h, label);
        assert!(found.iter().any(|d| d == tip), "{label}: {found:?}");
    }
}

#[test]
fn the_eraser_keeps_its_texts() {
    let h = bar(Tool::Eraser);
    for (label, tip) in BRUSH {
        assert!(!descriptions(&h, label).iter().any(|d| d == tip), "{label} changed on the Eraser");
    }
}

/// The non-Pro themes' Brush bar: the same texts where it has the same controls (its pressure
/// switches are named "Pressure for Size" and "Pressure for Opacity").
const BRUSH_CLASSIC: [(&str, &str); 6] = [
    ("Mode", "Painting Mode"),
    ("Opacity", "Set opacity for stroke"),
    ("Flow", "Set flow rate for stroke"),
    ("Smoothing", "Set smoothing for stroke\nUse higher values to reduce the shakiness of brush strokes"),
    ("Pressure for Size", "Always use Pressure for Size. When off, Brush Preset controls pressure"),
    ("Pressure for Opacity", "Always use Pressure for Opacity. When off, Brush Preset controls pressure"),
];

#[test]
fn the_brush_has_the_reference_tooltips_in_other_themes_too() {
    let h = bar_in(Tool::Brush, ThemeKind::Classic);
    for (label, tip) in BRUSH_CLASSIC {
        let found = descriptions(&h, label);
        assert!(found.iter().any(|d| d == tip), "{label}: {found:?}");
    }
    let h = bar_in(Tool::Eraser, ThemeKind::Classic);
    for (label, tip) in BRUSH_CLASSIC {
        assert!(!descriptions(&h, label).iter().any(|d| d == tip), "{label} changed on the Eraser");
    }
}

#[test]
fn the_erasers_smoothing_popup_keeps_no_title() {
    let mut h = bar(Tool::Eraser);
    h.get_by_label("Set additional smoothing options").click();
    h.run_steps(3);
    assert!(h.query_by_label("Pulled String Mode").is_some());
    assert!(h.query_by_label("Smoothing Options").is_none());
}

#[test]
fn the_smoothing_options_popup_has_a_title() {
    let mut h = bar(Tool::Brush);
    h.get_by_label("Set additional smoothing options").click();
    h.run_steps(3);
    assert!(h.query_by_label("Smoothing Options").is_some());
    assert!(h.query_by_label("Pulled String Mode").is_some());
}
