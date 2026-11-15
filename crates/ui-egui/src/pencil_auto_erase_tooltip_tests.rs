//! #11968: hovering the Pencil's Auto Erase checkbox shows "Draws background color over
//! foreground colors", in the Pro and the other themes.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::ThemeKind;

const TIP: &str = "Draws background color over foreground colors";

fn bar(theme: ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.ui.tool = Tool::Pencil;
    let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(4);
    h
}

#[test]
fn auto_erase_has_the_reference_tooltip() {
    for theme in [ThemeKind::ProMedium, ThemeKind::Classic] {
        let mut h = bar(theme);
        // Hovering the checkbox's label shows it.
        h.get_by_label("Auto Erase").hover();
        h.run_steps(60);
        assert!(h.query_by_label(TIP).is_some(), "{theme:?}: no tooltip on hover");
    }
}
