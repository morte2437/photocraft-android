//! #10881: the Layers panel's Attribute filter, against the cases measured in Photoshop.

use std::collections::BTreeSet;

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{BlendRange, Document, Group, Knockout, Layer, LayerContent, LayerMask, VectorMask};
use serde_json::json;

use super::attribute::Attribute;
use super::{Filter, FilterBy};
use crate::PhotocraftApp;
use crate::state::UiState;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

/// One layer per measured case. Every pixel layer has pixels except "empty"; the adjustment, fill
/// and shape layers come from their commands, so they carry the masks those give them.
fn session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let pixel_layers = [
        "pixels",
        "hidden",
        "position lock",
        "all locked",
        "empty",
        "linked a",
        "linked b",
        "base",
        "masked",
        "vector masked",
        "fx",
        "fx off",
        "fill 50",
        "adv channels",
        "adv knockout",
        "adv interior",
        "adv clipped",
        "adv shapes",
        "adv layer mask",
        "adv vector mask",
        "adv blend if",
    ];
    for name in pixel_layers {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
        match name {
            "base" => {
                s.execute("layer.newAdjustmentLayer.levels", json!({})).unwrap();
                rename_active(&mut s, "clipped");
                s.execute("layer.createClippingMask", json!({})).unwrap();
            }
            "fx" => {
                s.execute("layer.layerStyle.dropShadow", json!({})).unwrap();
            }
            "fx off" => {
                s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": "dropShadow", "params": {"enabled": false}}]})).unwrap();
            }
            _ => {}
        }
    }
    s.execute("layer.newFillLayer.solidColor", json!({"color": "#00ff00"})).unwrap();
    rename_active(&mut s, "fill");
    s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 20], "fill": "#ff0000"})).unwrap();
    rename_active(&mut s, "shape");

    let doc = std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let full = photocraft_geom::Rect::new(0, 0, 64, 48);
    for name in pixel_layers.into_iter().filter(|n| *n != "empty") {
        let l = layer(doc, name);
        let LayerContent::Raster(surface) = &mut l.content else { panic!("{name} is a pixel layer") };
        surface.fill_rect(full, &RED);
    }
    layer(doc, "hidden").visible = false;
    layer(doc, "position lock").locks.position = true;
    layer(doc, "all locked").locks.all = true;
    layer(doc, "linked a").link_group = Some(7);
    layer(doc, "linked b").link_group = Some(7);
    layer(doc, "masked").mask = Some(LayerMask::reveal_all());
    layer(doc, "vector masked").vector_mask = Some(VectorMask { path: Default::default(), enabled: true, linked: true, density: 1.0, feather: 0.0 });
    layer(doc, "fill 50").fill_opacity = 0.5;
    layer(doc, "adv channels").excluded_channels = 0b010;
    layer(doc, "adv knockout").advanced.knockout = Knockout::Shallow;
    layer(doc, "adv interior").advanced.blend_interior = true;
    layer(doc, "adv clipped").advanced.blend_clipped = false;
    layer(doc, "adv shapes").advanced.transparency_shapes = false;
    layer(doc, "adv layer mask").advanced.layer_mask_hides_effects = true;
    layer(doc, "adv vector mask").advanced.vector_mask_hides_effects = true;
    layer(doc, "adv blend if").blend_if.ranges = vec![[BlendRange { black: [40, 40], white: [255, 255] }, BlendRange::FULL]];
    // A visible group holding a pixel layer, and an empty group.
    let mut inside = Layer::new("inside", LayerContent::Raster(photocraft_raster::Surface::new(photocraft_doc::PixelFormat::RGBA8)));
    if let LayerContent::Raster(surface) = &mut inside.content {
        surface.fill_rect(full, &RED);
    }
    doc.layers.push(Layer::new("group", LayerContent::Group(Group { children: vec![inside], expanded: true, artboard: None, frame_stroke: None })));
    doc.layers.push(Layer::new("empty group", LayerContent::Group(Group { children: Vec::new(), expanded: true, artboard: None, frame_stroke: None })));
    // A hidden group whose child is visible: Visible goes by the layer's own flag.
    let mut child = Layer::new("in hidden group", LayerContent::Raster(photocraft_raster::Surface::new(photocraft_doc::PixelFormat::RGBA8)));
    if let LayerContent::Raster(surface) = &mut child.content {
        surface.fill_rect(full, &RED);
    }
    let mut hidden_group = Layer::new("hidden group", LayerContent::Group(Group { children: vec![child], expanded: true, artboard: None, frame_stroke: None }));
    hidden_group.visible = false;
    doc.layers.push(hidden_group);
    s
}

fn layer<'a>(doc: &'a mut Document, name: &str) -> &'a mut Layer {
    let id = doc.walk().into_iter().find(|(_, _, l)| l.name == name).map(|(_, _, l)| l.id).unwrap_or_else(|| panic!("no layer {name}"));
    doc.layer_mut(id).unwrap()
}

fn rename_active(s: &mut photocraft_engine::Session, name: &str) {
    let d = s.active_mut().unwrap();
    let id = d.active_layer.unwrap();
    std::sync::Arc::make_mut(&mut d.doc).layer_mut(id).unwrap().name = name.into();
}

/// The names `attribute` lists.
fn listed(doc: &Document, attribute: Attribute) -> BTreeSet<String> {
    let mut filter = Filter::of(&UiState { layer_filter_by: FilterBy::Attribute, layer_filter_attribute: attribute, ..UiState::default() });
    // As the panel does ([`Filter::of_app`]): Empty knows the canvas.
    filter.empty = Some(std::sync::Arc::new(super::attribute::empty_layers_of(doc)));
    crate::layer_tree_ui::display_rows(doc, filter.active()).into_iter().filter(|(_, l)| filter.shows(l)).map(|(_, l)| l.name.clone()).collect()
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|n| n.to_string()).collect()
}

fn all(doc: &Document) -> BTreeSet<String> {
    doc.walk().into_iter().map(|(_, _, l)| l.name.clone()).collect()
}

#[test]
fn each_attribute_lists_the_measured_layers() {
    let s = session();
    let doc = &s.active().unwrap().doc;
    let everything = all(doc);
    let but = |gone: &[&str]| -> BTreeSet<String> { everything.difference(&names(gone)).cloned().collect() };
    // Every visible layer and every visible group.
    assert_eq!(listed(doc, Attribute::Visible), but(&["hidden", "hidden group"]));
    assert!(listed(doc, Attribute::Visible).contains("in hidden group"), "a visible child of a hidden group");
    // Any lock counts, the Background's too.
    assert_eq!(listed(doc, Attribute::Locked), names(&["position lock", "all locked", "Background"]));
    // A pixel layer without pixels; the empty group isn't listed.
    assert_eq!(listed(doc, Attribute::Empty), names(&["empty"]));
    assert_eq!(listed(doc, Attribute::Linked), names(&["linked a", "linked b"]));
    // The clipped Levels layer, not its base.
    assert_eq!(listed(doc, Attribute::Clipped), names(&["clipped"]));
    // The adjustment and fill layers count with the masks they're created with.
    assert_eq!(listed(doc, Attribute::LayerMask), names(&["masked", "clipped", "fill"]));
    // A shape layer counts as having a vector mask.
    assert_eq!(listed(doc, Attribute::VectorMask), names(&["vector masked", "shape"]));
    // A layer whose only effect is turned off counts.
    assert_eq!(listed(doc, Attribute::LayerEffects), names(&["fx", "fx off"]));
    // Each Advanced Blending option on its own; Fill opacity alone doesn't count.
    assert_eq!(
        listed(doc, Attribute::AdvancedBlending),
        names(&["adv channels", "adv knockout", "adv interior", "adv clipped", "adv shapes", "adv layer mask", "adv vector mask", "adv blend if"])
    );
}

#[test]
fn a_layer_erased_completely_is_empty() {
    // The Eraser only clears alpha, so the erased pixels keep their colour bytes.
    for depth in [8, 16, 32] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth, "background": "transparent"})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let name = s.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l.name.clone()).next().unwrap();
        let erase = |s: &mut photocraft_engine::Session, x: i32| {
            s.execute("paint.stroke", json!({"points": [[x, -10], [x, 40]], "size": 30, "erase": true, "brush": {"hardness": 1.0}})).unwrap();
        };
        erase(&mut s, 10);
        let doc = &s.active().unwrap().doc;
        assert_eq!(listed(doc, Attribute::Empty), names(&[]), "{depth}: partly erased");
        assert_eq!(listed(doc, Attribute::NotEmpty), names(&[&name]), "{depth}");
        erase(&mut s, 30);
        let doc = &s.active().unwrap().doc;
        assert_eq!(listed(doc, Attribute::Empty), names(&[&name]), "{depth}: erased completely");
        assert_eq!(listed(doc, Attribute::NotEmpty), names(&[]), "{depth}");
    }
}

#[test]
fn a_layer_with_an_opaque_default_pixel_is_not_empty() {
    // No stored tiles, but the default pixel covers the whole canvas.
    let mut doc = Document::new("Opaque default", photocraft_geom::Size::new(40, 30), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let opaque = photocraft_raster::Surface::with_default(photocraft_doc::PixelFormat::RGBA8, &RED);
    doc.layers.push(Layer::new("opaque", LayerContent::Raster(opaque)));
    doc.layers.push(Layer::new("clear", LayerContent::Raster(photocraft_raster::Surface::new(photocraft_doc::PixelFormat::RGBA8))));
    assert_eq!(listed(&doc, Attribute::Empty), names(&["clear"]));
    assert_eq!(listed(&doc, Attribute::NotEmpty), names(&["opaque"]));
}

#[test]
fn an_opaque_default_layer_erased_on_the_whole_canvas_is_empty() {
    // 300 px wide: the second column of tiles reaches past the canvas, where the default stays.
    let mut doc = Document::new("Opaque erased", photocraft_geom::Size::new(300, 30), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let clear = [1.0, 0.0, 0.0, 0.0];
    for (name, erased) in [("erased", photocraft_geom::Rect::new(0, 0, 300, 30)), ("half erased", photocraft_geom::Rect::new(0, 0, 200, 30))] {
        let mut s = photocraft_raster::Surface::with_default(photocraft_doc::PixelFormat::RGBA8, &RED);
        s.fill_rect(erased, &clear);
        doc.layers.push(Layer::new(name, LayerContent::Raster(s)));
    }
    let mut dot = photocraft_raster::Surface::with_default(photocraft_doc::PixelFormat::RGBA8, &RED);
    dot.fill_rect(photocraft_geom::Rect::new(0, 0, 300, 30), &clear);
    dot.fill_rect(photocraft_geom::Rect::new(299, 29, 300, 30), &RED);
    doc.layers.push(Layer::new("one pixel", LayerContent::Raster(dot)));
    // The half erased layer has no tile over x 256..300, so the default shows there.
    assert_eq!(listed(&doc, Attribute::Empty), names(&["erased"]));
    assert_eq!(listed(&doc, Attribute::NotEmpty), names(&["half erased", "one pixel"]));
}

#[test]
fn the_empty_layers_are_found_once_per_document() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "transparent"})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.ui.layer_filter_by = FilterBy::Attribute;
    app.ui.layer_filter_attribute = Attribute::Empty;
    let first = Filter::of_app(&app).empty.expect("found for Empty");
    assert!(first.is_empty(), "the filled layer has pixels");
    let again = Filter::of_app(&app).empty.unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &again), "the same document reuses the result");
    // Selecting a layer moves the revision but changes no pixels.
    let id = app.session.active().unwrap().doc.layers[0].id;
    app.session.select_layer(id).unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &Filter::of_app(&app).empty.unwrap()), "a selection reuses the result");
    app.session.execute("paint.stroke", json!({"points": [[20, -10], [20, 40]], "size": 60, "erase": true, "brush": {"hardness": 1.0}})).unwrap();
    let after = Filter::of_app(&app);
    assert_eq!(after.empty.as_ref().map(|set| set.len()), Some(1), "an edit finds them again");
    let doc = &app.session.active().unwrap().doc;
    assert!(doc.walk().into_iter().all(|(_, _, l)| after.shows(l)), "the erased layer is listed");
    app.ui.layer_filter_attribute = Attribute::NotEmpty;
    assert!(Filter::of_app(&app).empty.is_some(), "Not Empty uses them too");
    // Another document, with no empty layer, gets its own result.
    app.session.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    assert_eq!(Filter::of_app(&app).empty.map(|set| set.len()), Some(0), "the new document's layers");
    app.ui.layer_filter_attribute = Attribute::Visible;
    assert!(Filter::of_app(&app).empty.is_none(), "other attributes don't read pixels");
}

#[test]
fn each_opposite_lists_the_rest_groups_included() {
    let s = session();
    let doc = &s.active().unwrap().doc;
    let everything = all(doc);
    for (positive, negative) in Attribute::ALL[..9].iter().zip(&Attribute::ALL[9..]) {
        let rest: BTreeSet<String> = everything.difference(&listed(doc, *positive)).cloned().collect();
        assert_eq!(listed(doc, *negative), rest, "{negative:?} lists what {positive:?} doesn't");
    }
    // The three measured ones, spelled out.
    assert_eq!(listed(doc, Attribute::NotVisible), names(&["hidden", "hidden group"]));
    assert!(listed(doc, Attribute::NotLocked).contains("group"), "groups are listed too");
    assert!(!listed(doc, Attribute::NotLocked).contains("Background"));
    assert_eq!(listed(doc, Attribute::NotLinked), everything.difference(&names(&["linked a", "linked b"])).cloned().collect());
}

#[test]
fn attribute_filters_at_once_with_visible() {
    let f = Filter::of(&UiState { layer_filter_by: FilterBy::Attribute, ..UiState::default() });
    assert_eq!(f.attribute, Attribute::Visible);
    assert!(f.active(), "Attribute filters as soon as it's picked");
}

#[test]
fn ui_set_picks_the_attribute_by_id() {
    let mut ui = UiState::default();
    for attribute in Attribute::ALL {
        let f = super::patched(&ui, &json!({"by": "attribute", "attribute": attribute.id()})).unwrap();
        super::apply(&mut ui, f);
        assert_eq!((ui.layer_filter_by, ui.layer_filter_attribute), (FilterBy::Attribute, attribute));
    }
    assert_eq!(Attribute::NoAdvancedBlending.id(), "noAdvancedBlending");
    let err = super::patched(&ui, &json!({"attribute": "hidden"})).unwrap_err();
    assert!(err.contains("hidden") && err.contains("notVisible") && err.contains("noAdvancedBlending"), "{err}");
    assert!(super::patched(&ui, &json!({"attribute": 3})).is_err());
    assert!(super::patched(&ui, &json!({"attribute": null})).is_err());
}

#[test]
fn saved_settings_without_the_attribute_still_load() {
    let mut saved = serde_json::to_value(UiState { layer_filter_attribute: Attribute::NotLinked, ..UiState::default() }).unwrap();
    assert_eq!(saved["layer_filter_attribute"], "notLinked");
    saved.as_object_mut().unwrap().remove("layer_filter_attribute");
    let ui: UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(ui.layer_filter_attribute, Attribute::Visible);
}

/// The names of the rows the Layers panel drew.
fn drawn(h: &Harness<'_, PhotocraftApp>) -> BTreeSet<String> {
    let doc = &h.state().session.active().unwrap().doc;
    let rows = crate::layer_row_ui::recorded(&h.ctx);
    rows.iter().filter_map(|r| doc.walk().into_iter().find(|(_, _, l)| l.id.0 == r.layer).map(|(_, _, l)| l.name.clone())).collect()
}

#[test]
fn the_panel_filters_by_the_attribute_picked_in_the_dropdown() {
    let session = session();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(session, crate::Services::default())
    });
    h.run_steps(8);
    h.get_by_value("Kind").click();
    h.run_steps(4);
    h.query_all_by_label("Attribute").last().expect("Attribute in the open list").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.layer_filter_by, FilterBy::Attribute);
    assert!(!drawn(&h).contains("hidden"), "Visible at first");
    h.get_by_value("Visible").click();
    h.run_steps(4);
    h.query_all_by_label("Not Visible").last().expect("Not Visible in the open list").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.layer_filter_attribute, Attribute::NotVisible);
    assert_eq!(drawn(&h), names(&["hidden", "hidden group"]));
}
