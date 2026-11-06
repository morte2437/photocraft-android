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

    // The frame being edited takes the entry away.
    h.state_mut().crop.default_frame = false;
    h.run();
    assert!(!drawn(&h).iter().any(|t| t == "Selection"), "edited frame: {:?}", drawn(&h));
}

/// Choosing "Selection" (measured in the reference application, #10872): the frame stays, the
/// crop becomes live with free Ratio, and the entry is gone.
#[test]
fn choosing_selection_makes_the_frame_a_live_crop() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;

    let mut selected = app();
    selected.run("select.rect", json!({"x": 50, "y": 20, "width": 100, "height": 60})).expect("select");
    crop_ui::ensure_frame(&mut selected);
    let frame = selected.ui.crop_rect;
    assert_eq!(frame, Some([50.0, 20.0, 150.0, 80.0]));
    let mut h = dropdown_harness(selected);
    h.run();
    h.get_by_role(Role::ComboBox).click();
    h.run();
    h.get_by_label("Selection").click();
    h.run();
    let app = h.state();
    assert_eq!(app.ui.crop_rect, frame, "the frame stays");
    assert!(app.ui.tool_options.crop_ratio.is_empty(), "free Ratio");
    assert!(!app.crop.default_frame && app.crop.editing, "a live crop");
    assert!(!crop_presets::shows_selection(app), "the entry is gone");
}

/// A ratio picked on the untouched selection frame replaces it with the largest centred frame of
/// that ratio on the canvas, not one fitted into the selection (measured, #10872).
#[test]
fn a_ratio_picked_on_the_selection_frame_fits_the_canvas() {
    let mut selected = app();
    selected.run("select.rect", json!({"x": 10, "y": 10, "width": 60, "height": 40})).expect("select");
    crop_ui::ensure_frame(&mut selected);
    selected.ui.tool_options.crop_ratio = "1:1".into();
    crop_ui::apply_ratio(&mut selected);
    assert_eq!(selected.ui.crop_rect, Some([50.0, 0.0, 150.0, 100.0]));
    assert!(!crop_presets::shows_selection(&selected));
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

/// The closed dropdown names the checked entry: "Selection" while the untouched frame is the
/// selection's (as the reference application's options bar shows), the mode once it is edited.
#[test]
fn the_closed_preset_dropdown_reads_selection_while_it_stands() {
    let mut selected = app();
    selected.ui.tool_options.crop_ratio.clear();
    selected.run("select.all", json!({})).expect("select all");
    crop_ui::ensure_frame(&mut selected);
    let mut h = dropdown_harness(selected);
    h.run();
    assert!(drawn(&h).iter().any(|t| t == "Selection"), "closed: {:?}", drawn(&h));
    h.state_mut().crop.default_frame = false;
    h.run();
    assert!(!drawn(&h).iter().any(|t| t == "Selection"), "edited frame: {:?}", drawn(&h));
    // A picked ratio takes the entry's place, on the button too.
    let mut ratio = app();
    ratio.ui.tool_options.crop_ratio = "1:1".into();
    ratio.run("select.all", json!({})).expect("select all");
    crop_ui::ensure_frame(&mut ratio);
    let mut h = dropdown_harness(ratio);
    h.run();
    assert!(!drawn(&h).iter().any(|t| t == "Selection"), "ratio: {:?}", drawn(&h));
}

/// A saved free-Ratio crop preset matches the options while the Selection entry stands, but only
/// Selection is checked; once the frame is edited the preset is checked again.
#[test]
fn only_selection_is_checked_while_it_stands() {
    use egui::accesskit::{Role, Toggled};
    use egui_kittest::kittest::{NodeT as _, Queryable};

    let mut selected = app();
    selected.ui.tool_options.crop_ratio.clear();
    selected.run("tool.presets.new", json!({"name": "Free Crop", "tool": "crop", "options": {"toolOptions": {"crop_ratio": ""}}})).expect("preset");
    selected.run("select.all", json!({})).expect("select all");
    crop_ui::ensure_frame(&mut selected);
    let mut h = dropdown_harness(selected);
    h.run();
    h.get_by_role(Role::ComboBox).click();
    h.run();
    assert_eq!(h.get_by_label("Selection").accesskit_node().toggled(), Some(Toggled::True));
    assert_eq!(h.get_by_label("Free Crop").accesskit_node().toggled(), Some(Toggled::False), "one current entry");
    h.state_mut().crop.default_frame = false;
    h.run();
    assert_eq!(h.get_by_label("Free Crop").accesskit_node().toggled(), Some(Toggled::True), "no Selection entry: the preset is current");
}

/// Undo and Redo of the box put back whether its default frame was the selection's: undoing a
/// Reset to a selection back to the whole-canvas frame has no Selection entry.
#[test]
fn box_undo_restores_whether_the_frame_came_from_a_selection() {
    use crate::canvas::{ToolEvent, tool_event};
    let mut app = app();
    crop_ui::ensure_frame(&mut app);
    let canvas = app.ui.crop_rect;
    assert_eq!(canvas, Some([0.0, 0.0, 200.0, 100.0]));
    assert!(!crop_presets::shows_selection(&app));
    // Edit the frame: drag its bottom-right corner in.
    tool_event(&mut app, ToolEvent::Down { x: 200.0, y: 100.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 80.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 80.0 }, egui::Modifiers::NONE);
    assert!(!app.crop.default_frame, "edited");
    app.run("select.rect", json!({"x": 50, "y": 20, "width": 100, "height": 60})).expect("select");
    crop_ui::reset(&mut app);
    assert_eq!(app.ui.crop_rect, Some([50.0, 20.0, 150.0, 80.0]));
    assert!(crop_presets::shows_selection(&app), "reset to the selection's frame");
    app.run("edit.undo", json!({})).expect("undo");
    app.run("edit.undo", json!({})).expect("undo");
    assert_eq!(app.ui.crop_rect, canvas, "back to the whole-canvas frame");
    assert!(app.crop.default_frame);
    assert!(!crop_presets::shows_selection(&app), "the canvas frame has no Selection entry");
    app.run("edit.redo", json!({})).expect("redo");
    app.run("edit.redo", json!({})).expect("redo");
    assert_eq!(app.ui.crop_rect, Some([50.0, 20.0, 150.0, 80.0]));
    assert!(crop_presets::shows_selection(&app), "redo brings the selection's frame back");
}

/// The Fill entry's default labels are translated in every catalog that translated the old
/// "Background (Default)" label, so no language falls back to English.
#[test]
fn the_fill_labels_are_translated_where_the_old_label_was() {
    use crate::i18n::{Lang, has};
    for lang in Lang::all().filter(|l| has(*l, "Background (Default)")) {
        for label in ["Background (default)", "Transparent (default)"] {
            assert!(has(lang, label), "{} lacks {label}", lang.code());
        }
    }
}
