//! Native Convert to Profile can flatten source-space appearance or freeze placed pixels.

use std::sync::Arc;

use photocraft_cms::Builtin;
use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    Document, Effect, Effects, FxCommon, FxPaint, Layer, LayerContent, LayerId, LayerMask, Path, SmartFilter, SmartObject, SmartSource, StrokeFx,
    StrokePosition, Subpath, VectorMask,
};
use photocraft_engine::{Session, smart_cmds};
use photocraft_geom::{Affine, Rect, Size};
use photocraft_raster::Surface;
use serde_json::{Value, json};

const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="6" height="4"><rect width="3" height="4" fill="#ff0000"/><rect x="3" width="3" height="2" fill="#00ff00"/></svg>"##;

fn convert_params() -> Value {
    json!({"profile": "linear-srgb", "intent": "relative", "bpc": false})
}

/// Half-opacity white over black separates source-space blending from linear-space blending.
fn layered(depth: u32, mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6, "depth": depth, "mode": mode, "background": "transparent"})).unwrap();
    s.execute("edit.fill", json!({"contents": "black"})).unwrap();
    let id = LayerId(s.execute("layer.new.layer", json!({"name": "Half white"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("edit.fill", json!({"contents": "white"})).unwrap();
    s.edit("Layer opacity", |doc, _| {
        doc.layer_mut(id).unwrap().opacity = 0.5;
        Ok(())
    })
    .unwrap();
    let profile = match mode {
        "cmyk" => "coated-cmyk",
        "lab" => "lab-d50",
        _ => "srgb",
    };
    s.execute("edit.assignProfile", json!({"profile": profile})).unwrap();
    s
}

fn from_document(doc: Document, active: Option<LayerId>) -> Session {
    let mut s = Session::new();
    s.add_document(doc, None);
    if let Some(id) = active {
        s.select_layer(id).unwrap();
    }
    s
}

fn composed(s: &Session) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(&s.active().unwrap().doc).px
}

/// The public source-first control must differ from conversion before flattening.
#[test]
fn flatten_uses_source_space_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = layered(depth, "rgb");
        let before = s.active().unwrap().doc.clone();
        let active = s.active().unwrap().active_layer;
        let past = s.active().unwrap().history.past_len();
        let mut source_first = from_document((*before).clone(), active);
        source_first.execute("layer.flattenImage", json!({})).unwrap();
        source_first.execute("edit.convertToProfile", convert_params()).unwrap();
        let expected = composed(&source_first);
        let mut destination_first = from_document((*before).clone(), active);
        destination_first.execute("edit.convertToProfile", convert_params()).unwrap();
        destination_first.execute("layer.flattenImage", json!({})).unwrap();
        assert!((expected[0][0] - composed(&destination_first)[0][0]).abs() > 0.1, "order fixture at {depth}-bit");

        let mut params = convert_params();
        params["flatten"] = json!(true);
        s.execute("edit.convertToProfile", params).unwrap();
        assert_eq!(composed(&s), expected, "flatten must precede profile conversion at {depth}-bit");
        let after = s.active().unwrap().doc.clone();
        assert_eq!(after.layers.len(), 1);
        assert!(matches!(after.layers[0].content, LayerContent::Raster(_)));
        assert!(after.layers[0].locks.position && after.layers[0].locks.transparency);
        assert_eq!(s.active().unwrap().active_layer, Some(after.layers[0].id));
        assert_eq!(s.active().unwrap().history.past_len(), past + 1);
        assert_eq!(after.icc_profile, source_first.active().unwrap().doc.icc_profile);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().active_layer, active);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, after);
    }
}

/// Native CMYK and Lab flattening retain the existing compositor and conversion path.
#[test]
fn flatten_keeps_native_source_color_models() {
    for mode in ["cmyk", "lab"] {
        let mut s = layered(8, mode);
        let before = s.active().unwrap().doc.clone();
        let mut control = from_document((*before).clone(), s.active().unwrap().active_layer);
        control.execute("layer.flattenImage", json!({})).unwrap();
        control.execute("edit.convertToProfile", convert_params()).unwrap();
        let mut params = convert_params();
        params["flatten"] = json!(true);
        s.execute("edit.convertToProfile", params).unwrap();
        assert_eq!(s.active().unwrap().doc.layers.len(), 1);
        assert_eq!(composed(&s), composed(&control), "source model {mode}");
    }
}

/// Omitted, false and null options retain the old per-layer conversion behavior.
#[test]
fn default_options_preserve_editable_layers_and_reject_wrong_types() {
    let s = layered(8, "rgb");
    let before = s.active().unwrap().doc.clone();
    let active = s.active().unwrap().active_layer;
    let mut control = from_document((*before).clone(), active);
    control.execute("edit.convertToProfile", convert_params()).unwrap();
    for value in [None, Some(json!(false)), Some(Value::Null)] {
        let mut actual = from_document((*before).clone(), active);
        let mut params = convert_params();
        if let Some(value) = value {
            params["flatten"] = value.clone();
            params["rasterizePlaced"] = value;
        }
        actual.execute("edit.convertToProfile", params).unwrap();
        assert_eq!(actual.active().unwrap().doc, control.active().unwrap().doc);
        assert_eq!(actual.active().unwrap().active_layer, active);
    }
    for key in ["flatten", "rasterizePlaced"] {
        for value in [json!(1), json!("false"), json!([]), json!({})] {
            let mut actual = from_document((*before).clone(), active);
            let mut params = convert_params();
            params[key] = value;
            assert_rejected(&mut actual, params);
        }
    }
}

/// Offset pixels with distinct alpha expose a second placement, filter or mask application.
fn shown_pixels() -> Surface {
    let mut surface = Surface::new(PixelFormat::RGBA32F);
    surface.fill_rect(Rect::new(7, 5, 12, 9), &[0.0, 0.0, 1.0, 0.75]);
    surface.fill_rect(Rect::new(12, 7, 15, 11), &[0.0, 1.0, 1.0, 0.5]);
    surface
}

fn embedded() -> SmartSource {
    SmartSource::Embedded { file_name: "profile.svg".into(), bytes: Arc::new(SVG.to_vec()) }
}

/// Keep source, placement and filter data deliberately different from the displayed cache.
fn placed(source: SmartSource, cache: Option<Surface>, unsupported_filter: bool) -> Layer {
    let mut smart = SmartObject::new(source, Affine { m: [2.0, 0.0, 0.0, 2.0, 7.0, 5.0] }, cache);
    smart.smart_filters.push(SmartFilter {
        command: if unsupported_filter { "psd.unsupportedFilter" } else { "filter.other.offset" }.into(),
        params: if unsupported_filter { json!({"name": "Cache sentinel"}) } else { json!({"horizontal": 3, "vertical": 1}) },
        blend: BlendMode::Normal,
        opacity: 0.7,
        visible: true,
    });
    let mut filter_mask = LayerMask::reveal_all();
    filter_mask.surface.fill_rect(Rect::new(7, 5, 10, 11), &[0.25]);
    smart.filter_mask = Some(filter_mask);
    smart.psd_raw = Some(Arc::new(vec![1, 2, 3, 4]));
    let mut layer = Layer::new("Placed", LayerContent::Smart(smart));
    layer.opacity = 0.6;
    layer.fill_opacity = 0.8;
    let mut mask = LayerMask::reveal_all();
    mask.surface.fill_rect(Rect::new(7, 5, 10, 11), &[0.5]);
    mask.density = 0.75;
    layer.mask = Some(mask);
    layer.vector_mask =
        Some(VectorMask::new(Path { subpaths: vec![Subpath::polygon(&[(8.0, 4.0), (14.0, 4.0), (14.0, 12.0), (8.0, 12.0)])], ..Default::default() }));
    layer.effects = Effects {
        enabled: true,
        items: vec![Effect::Stroke(StrokeFx {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            size: 1.0,
            position: StrokePosition::Outside,
            paint: FxPaint::Color(Color::rgb(1.0, 1.0, 0.0)),
            overprint: false,
        })],
        ..Default::default()
    };
    for key in [*b"SoLd", *b"SoLE", *b"PlLd", *b"zzzz"] {
        layer.psd_blocks.push((key, Arc::new(vec![9, 8, 7])));
    }
    layer
}

/// Nest placed objects under two groups with an ordinary active sibling.
fn smart_scene(children: Vec<Layer>) -> Session {
    let mut doc = Document::new("Placed profile", Size::new(24, 18), ColorMode::Rgb, SampleType::F32);
    doc.icc_profile = Some(Builtin::Srgb.profile().to_bytes());
    let ordinary = Layer::raster("Ordinary", PixelFormat::RGBA32F);
    let active = ordinary.id;
    doc.layers = vec![Layer::group("Outer", vec![Layer::group("Inner", children)]), ordinary];
    from_document(doc, Some(active))
}

fn missing_source() -> SmartSource {
    SmartSource::Linked { path: "missing-profile-smart-source-7ecb9a.svg".into() }
}

/// Convert manually substituted cached pixels through the unchanged command path.
fn cached_control(before: &Document, ids: &[LayerId], active: Option<LayerId>) -> Session {
    let mut expected = before.clone();
    let format = expected.pixel_format();
    for id in ids {
        let LayerContent::Smart(smart) = &expected.layer(*id).unwrap().content else { panic!("smart object missing") };
        let cache = smart.cache.clone().unwrap();
        let layer = expected.layer_mut(*id).unwrap();
        layer.content = LayerContent::Raster(if cache.format() == format { cache } else { cache.convert(format) });
        layer.psd_blocks.retain(|(key, _)| key != b"SoLd" && key != b"SoLE" && key != b"PlLd");
    }
    let mut s = from_document(expected, active);
    s.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "relative", "bpc": false})).unwrap();
    s
}

/// Rasterization freezes nested and unselected caches without rebaking masks or transforms.
#[test]
fn rasterize_placed_freezes_displayed_caches_and_restores_models_on_undo() {
    let first = placed(missing_source(), Some(shown_pixels()), true);
    let mut second = placed(embedded(), Some(shown_pixels()), false);
    second.visible = false;
    let ids = [first.id, second.id];
    let mut s = smart_scene(vec![first, second]);
    s.execute("shape.create", json!({"rect": [0, 0, 3, 3], "fill": "#336699"})).unwrap();
    let active = s.active().unwrap().doc.layers.last().unwrap().id;
    s.select_layer(active).unwrap();
    let before = s.active().unwrap().doc.clone();
    let past = s.active().unwrap().history.past_len();
    let selection = s.active().unwrap().selected_layers.clone();
    let control = cached_control(&before, &ids, Some(active));
    s.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "relative", "bpc": false, "rasterizePlaced": true})).unwrap();
    let after = s.active().unwrap().doc.clone();
    for id in ids {
        assert!(matches!(after.layer(id).unwrap().content, LayerContent::Raster(_)), "rasterizePlaced must replace every placed model");
        assert_eq!(after.layer(id), control.active().unwrap().doc.layer(id));
        assert_eq!(after.layer(id).unwrap().surface().unwrap().rgba(9, 6)[3], 0.75);
        assert_eq!(after.layer(id).unwrap().surface().unwrap().rgba(13, 8)[3], 0.5);
    }
    assert!(after.walk().iter().any(|(_, _, layer)| matches!(layer.content, LayerContent::Shape(_))));
    assert_eq!(composed(&s), composed(&control));
    assert_eq!(s.active().unwrap().active_layer, Some(active));
    assert_eq!(s.active().unwrap().selected_layers, selection);
    assert_eq!(s.active().unwrap().history.past_len(), past + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc, before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc, after);
}

/// A refusal must preserve the document, selection, revision and undo/redo state.
fn assert_rejected(s: &mut Session, params: Value) {
    let state = s.active().unwrap();
    let before = state.doc.clone();
    let revision = state.revision;
    let active = state.active_layer;
    let selection = state.selected_layers.clone();
    let history =
        (state.history.past_len(), state.history.can_redo(), state.history.undo_label().map(str::to_owned), state.history.redo_label().map(str::to_owned));
    assert!(s.execute("edit.convertToProfile", params).is_err());
    let state = s.active().unwrap();
    assert_eq!(state.doc, before);
    assert_eq!(state.revision, revision);
    assert_eq!(state.active_layer, active);
    assert_eq!(state.selected_layers, selection);
    assert_eq!(
        (state.history.past_len(), state.history.can_redo(), state.history.undo_label().map(str::to_owned), state.history.redo_label().map(str::to_owned)),
        history
    );
}

/// Missing or unrenderable visible pixels cannot become a successful blank conversion.
#[test]
fn unavailable_placed_pixels_and_oversize_flattening_fail_atomically() {
    for unavailable in [placed(missing_source(), None, false), placed(embedded(), None, true)] {
        for (flatten, rasterize) in [(false, true), (true, false), (true, true)] {
            let mut s = smart_scene(vec![placed(embedded(), Some(shown_pixels()), false), unavailable.clone()]);
            s.execute("layer.new.layer", json!({"name": "Redo sentinel"})).unwrap();
            s.execute("edit.undo", json!({})).unwrap();
            assert!(s.active().unwrap().history.can_redo());
            assert_rejected(&mut s, json!({"profile": "display-p3", "flatten": flatten, "rasterizePlaced": rasterize}));
        }
    }
    let mut large = Session::new();
    large.execute("file.new", json!({"width": 40000, "height": 30000, "background": "transparent"})).unwrap();
    assert_rejected(&mut large, json!({"profile": "srgb", "flatten": true}));
}

/// A missing cache can render from an available source; hidden discarded content needs no read.
#[test]
fn available_uncached_sources_render_and_flatten_skips_hidden_sources() {
    let mut layer = Layer::new("Available", LayerContent::Smart(SmartObject::new(embedded(), Affine { m: [1.0, 0.0, 0.0, 1.0, 3.0, 4.0] }, None)));
    let id = layer.id;
    let original = smart_scene(vec![layer.clone()]);
    let mut rendered = (*original.active().unwrap().doc).clone();
    assert!(smart_cmds::refresh_layer(&rendered, &mut layer).unwrap());
    let LayerContent::Smart(smart) = &layer.content else { panic!("smart object missing") };
    assert!(smart.cache.as_ref().unwrap().rgba(4, 5)[3] > 0.0);
    rendered.layer_mut(id).unwrap().content = layer.content;
    for flatten in [false, true] {
        let active = original.active().unwrap().active_layer;
        let mut s = from_document((*original.active().unwrap().doc).clone(), active);
        let mut control = if flatten { from_document(rendered.clone(), active) } else { cached_control(&rendered, &[id], active) };
        if flatten {
            control.execute("layer.flattenImage", json!({})).unwrap();
            control.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "relative", "bpc": false})).unwrap();
        }
        s.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "relative", "bpc": false, "flatten": flatten, "rasterizePlaced": !flatten}))
            .unwrap();
        assert_eq!(composed(&s), composed(&control));
        assert!(s.active().unwrap().doc.walk().iter().all(|(_, _, layer)| !matches!(layer.content, LayerContent::Smart(_))));
    }
    for (flatten, rasterize) in [(true, false), (true, true), (false, true)] {
        let mut hidden = placed(missing_source(), None, false);
        hidden.visible = false;
        let mut s = smart_scene(vec![hidden]);
        if flatten {
            s.execute("edit.convertToProfile", json!({"profile": "srgb", "flatten": true, "rasterizePlaced": rasterize})).unwrap();
            assert_eq!(s.active().unwrap().doc.layers.len(), 1);
        } else {
            assert_rejected(&mut s, json!({"profile": "srgb", "rasterizePlaced": true}));
        }
    }
}
