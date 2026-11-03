//! #10889: every Crop overlay has its own embedded glyph, the options bar's overlay button paints
//! the one of the overlay in use, and the overlay menu lists a glyph per entry.

use std::collections::HashSet;

use egui_kittest::Harness;
use photocraft_ui_egui::crop_overlay::CropOverlay;
use photocraft_ui_egui::icons;
use photocraft_ui_egui::state::ToolOptions;

fn collect(shape: &egui::Shape, ids: &mut HashSet<egui::TextureId>) {
    match shape {
        egui::Shape::Mesh(m) => {
            ids.insert(m.texture_id);
        }
        // Images (the SVG glyphs) are rects filled with a texture in egui 0.37.
        egui::Shape::Rect(r) => {
            if let Some(brush) = r.brush.as_ref()
                && brush.fill_texture_id != egui::TextureId::default()
            {
                ids.insert(brush.fill_texture_id);
            }
        }
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, ids)),
        _ => {}
    }
}

fn drawn_textures(h: &Harness<'_, ToolOptions>) -> HashSet<egui::TextureId> {
    let mut ids = HashSet::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, &mut ids);
    }
    ids
}

fn harness() -> Harness<'static, ToolOptions> {
    let h = Harness::builder()
        .with_size(egui::vec2(320.0, 520.0))
        .with_max_steps(64)
        .with_accessibility_check(false)
        .build_ui_state(|ui, o: &mut ToolOptions| photocraft_ui_egui::crop_overlay::options_button(o, ui), ToolOptions::default());
    // The SVG icon loader lives in the app's context setup; without it no glyph is rasterized.
    photocraft_ui_egui::PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

#[test]
fn every_overlay_has_its_own_embedded_icon() {
    let mut names: Vec<&str> = CropOverlay::ALL.iter().map(|k| k.icon()).collect();
    for name in &names {
        assert!(icons::exists(name), "{name} is not embedded");
    }
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), CropOverlay::ALL.len(), "one icon per overlay");
}

#[test]
fn the_overlay_button_paints_the_current_overlays_icon() {
    // One context: the SVG loader gives every new file its own texture, so the id changes as the
    // overlay (and therefore the button's glyph) changes.
    let mut h = Harness::builder()
        .with_size(egui::vec2(60.0, 60.0))
        .with_max_steps(64)
        .with_accessibility_check(false)
        .build_ui_state(|ui, o: &mut ToolOptions| photocraft_ui_egui::crop_overlay::options_button(o, ui), ToolOptions::default());
    photocraft_ui_egui::PhotocraftApp::setup_context(&h.ctx, Default::default());
    let mut glyphs = Vec::new();
    for overlay in CropOverlay::ALL {
        h.state_mut().crop_overlay = overlay;
        h.run();
        let ids = drawn_textures(&h);
        assert_eq!(ids.len(), 1, "{overlay:?}: one glyph on the button, got {ids:?}");
        glyphs.push(*ids.iter().next().expect("a glyph"));
    }
    let unique: HashSet<_> = glyphs.iter().copied().collect();
    assert_eq!(unique.len(), CropOverlay::ALL.len(), "the button follows the overlay: {glyphs:?}");
}

#[test]
fn the_overlay_menu_lists_a_glyph_per_entry() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;

    let mut h = harness();
    h.run();
    h.get_by_role(Role::Button).click();
    h.run();
    let ids = drawn_textures(&h);
    assert!(ids.len() >= CropOverlay::ALL.len(), "one glyph per overlay entry plus the button: {ids:?}");
}
