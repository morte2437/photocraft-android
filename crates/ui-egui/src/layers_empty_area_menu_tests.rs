//! #11404: a right-click on the empty area below the last layer row opens the thumbnail size and
//! clipping choices; a left click there still deselects, and a layer row keeps its own menu.

use egui::{PointerButton, Pos2, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::LayerThumbnailSize;

const SIZES: [&str; 4] = ["No Thumbnails", "Small Thumbnails", "Medium Thumbnails", "Large Thumbnails"];
const CLIPS: [&str; 2] = ["Clip Thumbnails to Layer Bounds", "Clip Thumbnails to Document Bounds"];

fn panel(open_document: bool) -> Harness<'static, PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    if open_document {
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
    }
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(6);
    h
}

/// A point in the Layers panel well below its last row (above the panel's bottom buttons).
fn empty_point(h: &Harness<'static, PhotocraftApp>) -> Pos2 {
    let button = h.get_by_label("Create a new layer").rect();
    pos2(button.center().x - 20.0, button.top() - 80.0)
}

fn click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, button: PointerButton) {
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
}

fn open_menu(h: &mut Harness<'static, PhotocraftApp>) {
    let p = empty_point(h);
    click(h, p, PointerButton::Secondary);
}

fn pick(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(4);
}

fn menu_open(h: &Harness<'static, PhotocraftApp>) -> bool {
    SIZES.iter().chain(CLIPS.iter()).any(|l| h.query_by_label(l).is_some())
}

fn checked(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

fn size(h: &Harness<'static, PhotocraftApp>) -> LayerThumbnailSize {
    h.state().ui.layers_panel_options.thumbnail_size
}

fn clip(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.state().session.prefs().interface.clip_thumbnails_to_layer_bounds
}

#[test]
fn the_menu_lists_the_six_entries_in_order_with_a_separator() {
    let mut h = panel(true);
    open_menu(&mut h);
    let ys: Vec<f32> = SIZES.iter().chain(CLIPS.iter()).map(|l| h.get_by_label(l).rect().top()).collect();
    assert_eq!(ys.len(), 6);
    assert!(ys.windows(2).all(|w| w[0] < w[1]), "entries top to bottom in the issue's order: {ys:?}");
    let step = ys[1] - ys[0];
    assert!(ys[4] - ys[3] > step + 1.0, "the separator sits between Large Thumbnails and the clip entries: {ys:?}");
}

#[test]
fn the_current_size_and_clip_choice_are_the_ones_checked() {
    let mut h = panel(true);
    h.state_mut().ui.layers_panel_options.thumbnail_size = LayerThumbnailSize::Large;
    h.state_mut().run("prefs.set", json!({"path": "interface.clipThumbnailsToLayerBounds", "value": true})).unwrap();
    h.run_steps(2);
    open_menu(&mut h);
    assert!(menu_open(&h));
    assert!(checked(&h, "Large Thumbnails"), "Large is the current size");
    for other in ["No Thumbnails", "Small Thumbnails", "Medium Thumbnails"] {
        assert!(!checked(&h, other), "{other}");
    }
    assert!(checked(&h, "Clip Thumbnails to Layer Bounds"));
    assert!(!checked(&h, "Clip Thumbnails to Document Bounds"));

    pick(&mut h, "Clip Thumbnails to Document Bounds");
    assert!(!clip(&h));
    open_menu(&mut h);
    assert!(checked(&h, "Clip Thumbnails to Document Bounds"));
    assert!(!checked(&h, "Clip Thumbnails to Layer Bounds"));
}

#[test]
fn each_size_pick_sets_the_panel_option_and_saves_it() {
    for (label, want) in [
        ("No Thumbnails", LayerThumbnailSize::None),
        ("Small Thumbnails", LayerThumbnailSize::Small),
        ("Large Thumbnails", LayerThumbnailSize::Large),
        ("Medium Thumbnails", LayerThumbnailSize::Medium),
    ] {
        let mut h = panel(true);
        h.state_mut().ui.layers_panel_options.thumbnail_size =
            if want == LayerThumbnailSize::Large { LayerThumbnailSize::Small } else { LayerThumbnailSize::Large };
        h.run_steps(2);
        open_menu(&mut h);
        pick(&mut h, label);
        assert_eq!(size(&h), want, "{label}");
        assert!(!menu_open(&h), "{label}: the menu closes");
        let saved = h.state().session.prefs().dialogs.get("ui.layersPanelOptions").cloned();
        let saved = saved.expect("the choice is saved as Panel Options saves it");
        assert_eq!(saved, serde_json::to_value(&h.state().ui.layers_panel_options).unwrap(), "{label}");
    }
}

/// Picking the size that is already checked closes the menu and leaves the saved options as they
/// were. The harness cannot observe a skipped write directly, so this asserts the saved value is
/// unchanged.
#[test]
fn picking_the_checked_size_closes_the_menu_and_changes_nothing() {
    let mut h = panel(true);
    h.state_mut().ui.layers_panel_options.thumbnail_size = LayerThumbnailSize::Large;
    h.run_steps(2);
    crate::panels::layers_panel_ui::persist_options(h.state_mut());
    let saved = |h: &Harness<'static, PhotocraftApp>| h.state().session.prefs().dialogs.get("ui.layersPanelOptions").cloned();
    let before = saved(&h);
    assert!(before.is_some(), "an entry is saved to begin with");
    open_menu(&mut h);
    assert!(checked(&h, "Large Thumbnails"));
    pick(&mut h, "Large Thumbnails");
    assert!(!menu_open(&h), "the menu closes");
    assert_eq!(size(&h), LayerThumbnailSize::Large);
    assert_eq!(saved(&h), before, "the saved options are unchanged");
}

/// The saved entry can be missing (a preferences reset, an older file) while the panel still
/// shows the size; picking the checked row writes it back.
#[test]
fn picking_the_checked_size_restores_a_missing_saved_entry() {
    let mut h = panel(true);
    h.state_mut().ui.layers_panel_options.thumbnail_size = LayerThumbnailSize::Large;
    h.run_steps(2);
    h.state_mut().session.prefs.edit(|p| {
        p.dialogs.remove("ui.layersPanelOptions");
    });
    let saved = |h: &Harness<'static, PhotocraftApp>| h.state().session.prefs().dialogs.get("ui.layersPanelOptions").cloned();
    assert!(saved(&h).is_none(), "the saved entry is gone");
    open_menu(&mut h);
    assert!(checked(&h, "Large Thumbnails"));
    pick(&mut h, "Large Thumbnails");
    assert!(!menu_open(&h), "the menu closes");
    let restored = saved(&h).expect("the pick writes the entry back");
    assert_eq!(restored, serde_json::to_value(&h.state().ui.layers_panel_options).unwrap());
}

#[test]
fn each_clip_pick_sets_the_preference() {
    let mut h = panel(true);
    h.state_mut().run("prefs.set", json!({"path": "interface.clipThumbnailsToLayerBounds", "value": false})).unwrap();
    h.run_steps(2);
    assert!(!clip(&h));
    open_menu(&mut h);
    pick(&mut h, "Clip Thumbnails to Layer Bounds");
    assert!(clip(&h));
    assert_eq!(crate::panels::layers_panel_ui::thumbnail_contents(h.state()), crate::state::LayerThumbnailContents::LayerBounds, "Panel Options agrees");
    open_menu(&mut h);
    pick(&mut h, "Clip Thumbnails to Document Bounds");
    assert!(!clip(&h));
    assert_eq!(crate::panels::layers_panel_ui::thumbnail_contents(h.state()), crate::state::LayerThumbnailContents::EntireDocument);
}

#[test]
fn a_left_click_on_the_empty_area_still_deselects_and_a_right_click_does_not() {
    let mut h = panel(true);
    let selected = |h: &Harness<'static, PhotocraftApp>| h.state().session.active().unwrap().selected_layers().len();
    h.run_steps(3);
    let before = selected(&h);
    assert!(before > 0);
    open_menu(&mut h);
    assert!(menu_open(&h));
    assert_eq!(selected(&h), before, "a right-click does not deselect");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    let p = empty_point(&h);
    click(&mut h, p, PointerButton::Primary);
    assert_eq!(selected(&h), 0, "a left click deselects every layer");
}

#[test]
fn with_no_document_nothing_opens() {
    // The Layers panel is not drawn without a document, so take its rect from the same layout
    // with one open.
    let rect = crate::panel_docking::probe::panel_rect(&panel(true).ctx, "layers").expect("the Layers panel is drawn");
    let mut h = panel(false);
    let p = rect.center();
    assert!(rect.contains(p));
    click(&mut h, p, PointerButton::Secondary);
    assert!(!menu_open(&h));
}

#[test]
fn a_right_click_on_a_layer_row_still_opens_the_layer_menu() {
    let mut h = panel(true);
    let row = h.get_all_by_label("Layer 1").next().expect("the new layer's row").rect();
    click(&mut h, row.center(), PointerButton::Secondary);
    assert!(h.query_by_label("Duplicate Layer…").is_some() || h.query_by_label("Duplicate Layer").is_some(), "the layer menu opened");
    assert!(h.query_by_label("No Thumbnails").is_none(), "not the empty-area menu");
}

/// A filter that matches no layer draws no row: the whole list is empty space, and its
/// right-click still opens the menu.
#[test]
fn with_no_row_shown_by_the_filter_the_right_click_still_opens_the_menu() {
    let mut h = panel(true);
    h.state_mut().ui.layer_filter_by = crate::layer_filter_ui::FilterBy::Name;
    h.state_mut().ui.layer_filter_name = "no layer has this name".into();
    h.run_steps(4);
    assert!(h.query_by_label("Layer 1").is_none(), "the filter hides every row");
    open_menu(&mut h);
    assert!(menu_open(&h), "the empty list's right-click opens the menu");
}
