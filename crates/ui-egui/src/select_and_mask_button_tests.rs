//! #7520: the options bar's Select and Mask… button is greyed out while nothing is selected.
use crate::PhotocraftApp;
use crate::state::Tool;
use egui::vec2;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::json;

fn harness(tool: Tool, select: bool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    if select {
        app.run("select.all", json!({})).unwrap();
    }
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(1400.0, 60.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::panels::options_bar(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
    h.run_steps(4);
    h
}

#[test]
fn select_and_mask_is_disabled_without_a_selection() {
    for tool in [Tool::RectMarquee, Tool::Lasso, Tool::MagicWand] {
        let h = harness(tool, false);
        assert!(h.get_by_label("Select and Mask…").accesskit_node().is_disabled(), "{tool:?}: button should be greyed out with no selection");
    }
}

#[test]
fn select_and_mask_is_enabled_with_a_selection() {
    for tool in [Tool::RectMarquee, Tool::Lasso, Tool::MagicWand] {
        let h = harness(tool, true);
        assert!(!h.get_by_label("Select and Mask…").accesskit_node().is_disabled(), "{tool:?}: button should be live with a selection");
    }
}
