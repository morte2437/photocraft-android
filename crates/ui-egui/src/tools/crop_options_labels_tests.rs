//! #10872: the Crop options bar's Fill entry follows Delete Cropped Pixels, and the preset
//! dropdown shows the temporary "Selection" entry while the frame is the untouched selection's.
//!
//! Kept in its own module so the batch that lands the crop-parity items does not touch the shared
//! `tests.rs` files.

use egui_kittest::Harness;
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use serde_json::json;

use crate::state::Tool;
use crate::{PhotocraftApp, Services, crop_presets, crop_ui};

fn app() -> PhotocraftApp {
    let doc = Document::with_background("crop labels", Size::new(200, 100), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut s = photocraft_engine::Session::new();
    s.add_document(doc, None);
    let mut app = PhotocraftApp::new(s, Services::default());
    app.ui.tool = Tool::Crop;
    app
}

/// Every text shape the harness drew this frame, popups included.
fn drawn<State>(h: &Harness<'_, State>) -> Vec<String> {
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

/// The preset dropdown alone, so the only ComboBox in the harness is ours.
fn dropdown_harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    Harness::builder().with_size(egui::vec2(460.0, 900.0)).with_max_steps(64).build_ui_state(|ui, app: &mut PhotocraftApp| crop_presets::dropdown(app, ui), app)
}

#[test]
fn the_fill_label_follows_delete_cropped_pixels() {
    assert_eq!(crop_ui::fill_label(true), "Background (default)");
    assert_eq!(crop_ui::fill_label(false), "Transparent (default)");
}

#[test]
fn the_options_bar_shows_the_fill_label_for_the_checkbox() {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 300.0))
        .with_max_steps(64)
        // The whole options bar has number fields the app itself does not name for accessibility.
        .with_accessibility_check(false)
        .build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
            app.run("file.new", json!({"name": "crop fill", "width": 200, "height": 100})).expect("new document");
            app.ui.tool = Tool::Crop;
            app
        });
    h.run_steps(6);
    h.state_mut().ui.tool_options.crop_delete = false;
    h.run_steps(2);
    assert!(drawn(&h).iter().any(|t| t == "Transparent (default)"), "off: {:?}", drawn(&h));
    h.state_mut().ui.tool_options.crop_delete = true;
    h.run_steps(2);
    assert!(drawn(&h).iter().any(|t| t == "Background (default)"), "on: {:?}", drawn(&h));
}

#[test]
fn the_selection_entry_stands_only_on_the_untouched_selection_frame() {
    // A selection when the Crop tool is picked: the default frame is its bounds, and the entry
    // names it.
    let mut selected = app();
    selected.run("select.all", json!({})).expect("select all");
    crop_ui::ensure_frame(&mut selected);
    assert!(selected.crop.default_frame && selected.crop.frame_from_selection);
    assert!(crop_presets::shows_selection(&selected));

    // Editing the frame clears the default-frame state, as the first press or drag does.
    selected.crop.default_frame = false;
    assert!(!crop_presets::shows_selection(&selected));

    // No selection: the default frame is the whole canvas, which has no Selection entry.
    let mut plain = app();
    crop_ui::ensure_frame(&mut plain);
    assert!(plain.crop.default_frame && !plain.crop.frame_from_selection);
    assert!(!crop_presets::shows_selection(&plain));

    // Cancelling drops the frame, so the entry goes with it.
    let mut cancelled = app();
    cancelled.run("select.all", json!({})).expect("select all");
    crop_ui::ensure_frame(&mut cancelled);
    crop_ui::cancel(&mut cancelled);
    assert!(!crop_presets::shows_selection(&cancelled));
}

/// The dropdown itself lists "Selection" above Ratio while it stands, and not after the frame is
/// edited.
#[test]
fn the_preset_dropdown_lists_selection_while_the_frame_is_the_selections() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable};

    let mut selected = app();
    selected.run("select.all", json!({})).expect("select all");
    crop_ui::ensure_frame(&mut selected);
    let mut h = dropdown_harness(selected);
    h.run();
    h.get_by_role(Role::ComboBox).click();
    h.run();
    assert!(drawn(&h).iter().any(|t| t == "Selection"), "open list: {:?}", drawn(&h));
    // Above the Ratio presets, and checked: the entry names the frame, so it must read as the
    // current one (#10872).
    let selection = h.get_by_label("Selection");
    let presets = h.get_by_label("W x H x Resolution");
    assert!(selection.rect().top() < presets.rect().top(), "Selection sits above the Ratio presets");
    assert_eq!(selection.accesskit_node().toggled(), Some(egui::accesskit::Toggled::True), "Selection is checked");

    // Picking it is a label, not a preset: the frame and the ratio stay as they are (#10872).
    let before = (h.state().ui.crop_rect, h.state().ui.tool_options.crop_ratio.clone());
    h.get_by_label("Selection").click();
    h.run();
    assert_eq!((h.state().ui.crop_rect, h.state().ui.tool_options.crop_ratio.clone()), before, "choosing Selection changes neither the frame nor the ratio");

    // The pick closes the list; reopen it, so the edited-frame check below looks at a drawn list.
    h.get_by_role(Role::ComboBox).click();
    h.run();
    assert!(drawn(&h).iter().any(|t| t == "Selection"), "reopened list: {:?}", drawn(&h));
    h.state_mut().crop.default_frame = false;
    h.run();
    assert!(!drawn(&h).iter().any(|t| t == "Selection"), "edited frame: {:?}", drawn(&h));
}

/// A picked ratio takes the check mark instead of the temporary entry.
#[test]
fn a_picked_ratio_hides_the_selection_entry() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;

    let mut selected = app();
    selected.run("select.all", json!({})).expect("select all");
    crop_ui::ensure_frame(&mut selected);
    selected.ui.tool_options.crop_ratio = "1:1".into();
    // The options bar fits the untouched frame to the picked ratio; it is no longer the
    // selection's bounds.
    crop_ui::apply_ratio(&mut selected);
    assert!(!crop_presets::shows_selection(&selected), "a fitted frame is not the selection's");
    // Clearing the ratio leaves the fitted frame, so the entry must not come back.
    selected.ui.tool_options.crop_ratio.clear();
    assert!(!crop_presets::shows_selection(&selected), "the entry does not return");
    let mut h = dropdown_harness(selected);
    h.run();
    h.get_by_role(Role::ComboBox).click();
    h.run();
    assert!(!drawn(&h).iter().any(|t| t == "Selection"), "a picked ratio: {:?}", drawn(&h));
}
