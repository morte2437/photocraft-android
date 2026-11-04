//! #10889: every Crop overlay has its own embedded glyph, the options bar's overlay button paints
//! the one of the overlay in use, and the overlay menu lists a glyph per entry.

use std::collections::HashSet;

use egui_kittest::Harness;
use photocraft_ui_egui::crop_overlay::CropOverlay;
use photocraft_ui_egui::icons;
use photocraft_ui_egui::state::{Tool, ToolOptions};

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

fn drawn_textures<State>(h: &Harness<'_, State>) -> HashSet<egui::TextureId> {
    let mut ids = HashSet::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, &mut ids);
    }
    ids
}

/// Every glyph texture and the rectangle it covers, in paint order.
fn glyph_rects(shape: &egui::Shape, out: &mut Vec<(egui::TextureId, egui::Rect)>) {
    match shape {
        egui::Shape::Rect(r) => {
            if let Some(brush) = r.brush.as_ref()
                && brush.fill_texture_id != egui::TextureId::default()
            {
                out.push((brush.fill_texture_id, r.rect));
            }
        }
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| glyph_rects(shape, out)),
        _ => {}
    }
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

/// The six files are different drawings, not one picture saved under six names: a duplicate would
/// still get six texture ids and pass the texture checks above.
#[test]
fn the_overlay_glyphs_are_six_different_drawings() {
    let art: [(&str, &[u8]); 6] = [
        ("crop-overlay-thirds", include_bytes!("../../../../assets/icons/crop-overlay-thirds.svg")),
        ("crop-overlay-grid", include_bytes!("../../../../assets/icons/crop-overlay-grid.svg")),
        ("crop-overlay-diagonal", include_bytes!("../../../../assets/icons/crop-overlay-diagonal.svg")),
        ("crop-overlay-triangle", include_bytes!("../../../../assets/icons/crop-overlay-triangle.svg")),
        ("crop-overlay-golden-ratio", include_bytes!("../../../../assets/icons/crop-overlay-golden-ratio.svg")),
        ("crop-overlay-golden-spiral", include_bytes!("../../../../assets/icons/crop-overlay-golden-spiral.svg")),
    ];
    let icons: Vec<&str> = CropOverlay::ALL.iter().map(|k| k.icon()).collect();
    assert_eq!(art.iter().map(|(name, _)| *name).collect::<Vec<_>>(), icons, "one file per overlay, in ALL order");
    for (i, (name, bytes)) in art.iter().enumerate() {
        assert!(art[..i].iter().all(|(_, other)| other != bytes), "{name} repeats another overlay's drawing");
    }
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

/// The glyph texture its row drew, found by row bounds.
fn row_glyph<State>(h: &Harness<'_, State>, label: &str) -> egui::TextureId {
    use egui_kittest::kittest::Queryable;

    let mut drawn = Vec::new();
    for shape in &h.output().shapes {
        glyph_rects(&shape.shape, &mut drawn);
    }
    h.get_all_by_label(label)
        .map(|node| node.rect())
        .find_map(|row| drawn.iter().find(|(_, rect)| row.contains_rect(*rect)).map(|(id, _)| *id))
        .unwrap_or_else(|| panic!("no {label} row draws a glyph"))
}

/// The canvas right-click Crop menu's overlay rows carry the same glyphs (#10889), each row its
/// own: a swapped icon still adds six textures, so the count alone proves nothing. Checked per
/// row, and pairwise distinct. (Comparing against a freshly loaded 14 px glyph does not work:
/// egui frees icon textures at the end of every pass, so the reload gets a new id.)
#[test]
fn the_canvas_crop_menu_lists_a_glyph_per_overlay() {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).with_accessibility_check(false).build_eframe(|cc| {
        photocraft_ui_egui::PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = photocraft_ui_egui::PhotocraftApp::new(photocraft_engine::Session::new(), photocraft_ui_egui::Services::default());
        app.run("file.new", serde_json::json!({"name": "crop menu", "width": 400, "height": 300})).expect("new document");
        app.ui.tool = Tool::Crop;
        photocraft_ui_egui::crop_ui::ensure_frame(&mut app);
        // The menu opens on a crop the user has started, so nudge the untouched frame once.
        photocraft_ui_egui::crop_ui::nudge(&mut app, 1.0, 0.0);
        app
    });
    h.run();
    let ctx = h.ctx.clone();
    let opened = photocraft_ui_egui::canvas::canvas_tool_menu::open_crop(h.state_mut(), &ctx, Tool::Crop, [600.0, 400.0]);
    assert!(opened, "the crop menu opens on a pending crop");
    h.run();
    let glyphs: Vec<_> = CropOverlay::ALL.iter().map(|kind| row_glyph(&h, kind.label())).collect();
    let unique: HashSet<_> = glyphs.iter().copied().collect();
    assert_eq!(unique.len(), CropOverlay::ALL.len(), "one glyph per row: {glyphs:?}");
}
