//! A click on a Type options bar button after the font menus lands when the fonts finish loading
//! between the press and the release: the buttons keep their place and ids when the "Loading
//! fonts…" box gives way to the menus.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::super::fonts::FontsLoading;
use super::super::vertical_align_buttons_tests::new_app;
use crate::PhotocraftApp;
use crate::state::Tool;

/// The options bar with the fonts loading until the first pass that sees a pointer press ends.
fn bar_loading_until_a_press(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut loading = Some(FontsLoading::start());
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 60.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            ui.horizontal(|ui| super::options_bar(app, ui));
            if ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. }))) {
                drop(loading.take());
            }
        },
        new_app(tool),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h
}

#[test]
fn a_click_lands_when_the_fonts_finish_loading_between_the_press_and_the_release() {
    let _ = photocraft_text::shared();
    for (tool, label) in [(Tool::Type, "Right align text"), (Tool::VerticalTypeMask, "Bottom align text")] {
        let mut h = bar_loading_until_a_press(tool);
        assert!(h.query_by_label("Style").is_none(), "{label}: no style menu while the fonts load");
        let button = h.get_by_role_and_label(Role::Button, label);
        let (rect, id) = (button.rect(), button.accesskit_node().id());
        button.click();
        h.run_steps(2);
        assert!(h.query_by_label("Style").is_some(), "{label}: the style menu once the fonts are loaded");
        assert_eq!(h.state().ui.tool_options.type_align, "right", "{label}: the click lands");
        let button = h.get_by_role_and_label(Role::Button, label);
        assert_eq!((button.rect(), button.accesskit_node().id()), (rect, id), "{label}: the button keeps its place and id");
    }
}
