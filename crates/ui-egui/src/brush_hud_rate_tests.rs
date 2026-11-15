//! #11891: the Alt+right-drag brush HUD changes hardness at Photoshop's rate (50 screen points
//! down from 0 % give 13 %), and its readout has three lines: "Diameter :", "Hardness :" and
//! "Opacity :". The horizontal size change is unchanged.

use egui::{Modifiers, PointerButton, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::brush_resize::{readout_label, resized};
use crate::state::Tool;

#[test]
fn fifty_points_down_from_zero_give_thirteen_percent() {
    for zoom in [0.5, 1.0, 2.0] {
        // The drag is in document pixels; 50 screen points is 50 / zoom of them.
        let (_, hardness) = resized((13.0, 0.0), 0.0, f64::from(50.0 / zoom), zoom);
        assert!((hardness - 0.13).abs() <= 0.02, "zoom {zoom}: {hardness}");
    }
    assert_eq!(resized((13.0, 0.0), 0.0, -50.0, 1.0).1, 0.0, "up from 0 % stays 0 %");
}

#[test]
fn the_size_change_is_unchanged() {
    assert_eq!(resized((13.0, 0.0), 50.0, 0.0, 1.0).0, 113.0);
    assert_eq!(resized((13.0, 0.0), -278.0, 0.0, 1.0).0, 1.0);
}

#[test]
fn labels_get_a_space_before_the_colon() {
    assert_eq!(readout_label("Diameter:"), "Diameter :");
    assert_eq!(readout_label("Diamètre :"), "Diamètre :", "a translation's own spacing stays");
    assert_eq!(readout_label("Size"), "Size");
}

#[test]
fn the_readout_shows_diameter_hardness_and_opacity() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 13, "hardness": 0.0, "opacity": 1.0}})).unwrap();
    app.ui.tool = Tool::Brush;
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let c = h.state().last_canvas_rect.center();
    let alt = Modifiers::ALT;
    h.event(egui::Event::ModifiersChanged(alt));
    h.event(egui::Event::PointerMoved(c));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: c, button: PointerButton::Secondary, pressed: true, modifiers: alt });
    h.run_steps(1);
    for i in 1..=5 {
        h.event(egui::Event::PointerMoved(c + vec2(0.0, 10.0 * i as f32)));
        h.run_steps(1);
    }
    let hardness = h.state().session.tools.brush.hardness;
    assert!((hardness - 0.13).abs() <= 0.02, "{hardness}");
    for label in ["Diameter :", "Hardness :", "Opacity :"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    assert!(h.query_by_label("13%").is_some() && h.query_by_label("100%").is_some());
    h.event(egui::Event::PointerButton { pos: c + vec2(0.0, 50.0), button: PointerButton::Secondary, pressed: false, modifiers: alt });
    h.run_steps(2);
}
