//! #12057: the Type options bar's font family menu, font style menu, font size icon and field,
//! and text colour swatch carry the reference application's tooltips (also their accessible
//! descriptions). The controls keep their names, and the Character panel's font menu, which
//! shares the family picker, keeps its text.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;

const FAMILY_TIP: &str = "Search and select fonts";

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();
    app
}

/// `draw` in a row, once the app's fonts are set up (the first pass has none).
fn harness(draw: fn(&mut PhotocraftApp, &mut egui::Ui)) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1400.0, 600.0)).build_ui_state(
        move |ui, app| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal(|ui| draw(app, ui));
            }
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    super::options_bar::options_bar(app, ui);
}

fn character_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.vertical(|ui| super::panel::character_panel(app, ui, false));
}

fn description(n: egui_kittest::Node<'_>) -> Option<String> {
    n.accesskit_node().description().map(|d| d.to_string())
}

fn described_by_label(h: &Harness<'static, PhotocraftApp>, label: &str) -> Vec<String> {
    h.query_all_by_label(label).filter_map(description).collect()
}

fn described_by_role(h: &Harness<'static, PhotocraftApp>, role: Role) -> Vec<String> {
    h.query_all_by_role(role).filter_map(description).collect()
}

/// Every text shape drawn this frame, tooltips included.
fn drawn(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(shape) => text.push(shape.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, text)),
            _ => {}
        }
    }
    let mut text = Vec::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, &mut text);
    }
    text
}

#[test]
fn the_font_family_menu_has_the_reference_tooltip() {
    let h = harness(options_bar);
    // The name field (the bar's only text input) and its arrow, which keeps the name "Font".
    assert_eq!(described_by_role(&h, Role::TextInput), [FAMILY_TIP]);
    assert_eq!(described_by_label(&h, "Font"), [FAMILY_TIP]);
}

#[test]
fn the_font_style_menu_has_the_reference_tooltip_and_keeps_its_name() {
    let h = harness(options_bar);
    assert_eq!(described_by_label(&h, "Style"), ["Set font style"]);
}

#[test]
fn the_font_size_field_has_the_reference_tooltip() {
    let h = harness(options_bar);
    // The bar's only number field.
    assert_eq!(described_by_role(&h, Role::SpinButton), ["Set font size"]);
}

#[test]
fn the_font_size_icon_shows_the_reference_tooltip_and_keeps_its_name() {
    let mut h = harness(options_bar);
    // The icon itself: the size field beside it is labelled by it, so it is named "Font size" too.
    h.get_by_role_and_label(Role::Label, "Font size").hover();
    h.run_steps(8);
    let text = drawn(&h);
    assert!(text.iter().any(|t| t == "Set font size"), "{text:?}");
    assert!(!text.iter().any(|t| t == "Font size"), "old tooltip: {text:?}");
}

#[test]
fn the_text_color_swatch_has_the_reference_tooltip_and_keeps_its_name() {
    let h = harness(options_bar);
    let found = described_by_label(&h, "Set the text color");
    assert_eq!(found, ["Set text color"]);
}

/// A guard, so it passes without the fix too: it fails if the tooltip moves into the picker that
/// the panels share.
#[test]
fn the_character_panel_font_menu_keeps_its_text() {
    let h = harness(character_panel);
    assert!(h.query_by_label("Font").is_some(), "the panel draws its font menu");
    let found: Vec<String> = described_by_label(&h, "Font").into_iter().chain(described_by_role(&h, Role::TextInput)).collect();
    assert!(!found.iter().any(|d| d == FAMILY_TIP), "{found:?}");
}
