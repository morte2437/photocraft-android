//! Image Size can preserve layer-style sizes while resampling the document's geometry.

use std::sync::Arc;

use photocraft_color::{BlendMode, Color};
use photocraft_doc::{Effect, Effects, FxCommon, FxPaint, Layer, LayerContent, LayerId, StrokeFx, StrokePosition};
use photocraft_engine::Session;
use serde_json::json;

fn styled_shape(depth: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 40, "depth": depth, "background": "transparent"})).unwrap();
    let id = s.execute("shape.create", json!({"rect": [20, 10, 40, 20], "fill": "#ff0000", "stroke": {"width": 4, "color": "#000000"}})).unwrap()["layer"]
        .as_u64()
        .unwrap();
    let id = LayerId(id);
    s.edit("Styles", |doc, _| {
        let effects = Effects {
            enabled: true,
            items: vec![Effect::Stroke(StrokeFx {
                common: FxCommon::new(BlendMode::Normal, 1.0),
                size: 20.0,
                position: StrokePosition::Outside,
                paint: FxPaint::Color(Color::rgb(0.0, 0.0, 1.0)),
                overprint: false,
            })],
            psd_raw: Some(Arc::new(b"preserved layer style".to_vec())),
            ..Default::default()
        };
        doc.layer_mut(id).unwrap().effects = effects.clone();
        let children = std::mem::take(&mut doc.layers);
        let mut outer = Layer::group("Outer", vec![Layer::group("Inner", children)]);
        outer.effects = effects;
        doc.layers.push(outer);
        Ok(())
    })
    .unwrap();
    (s, id)
}

fn shape_stroke(s: &Session, id: LayerId) -> f32 {
    let LayerContent::Shape(shape) = &s.active().unwrap().doc.layer(id).unwrap().content else { panic!("shape lost") };
    shape.stroke.as_ref().unwrap().width
}

/// Native omission keeps the old default; false preserves style sizes and raw PSD effects.
#[test]
fn image_size_scale_styles_is_optional_at_every_depth() {
    for depth in [8, 16, 32] {
        for option in [None, Some(true), Some(false)] {
            let (mut s, id) = styled_shape(depth);
            let before = s.active().unwrap().doc.clone();
            let group = before.layers[0].id;
            let past = s.active().unwrap().history.past_len();
            let mut params = json!({"percent": 50, "resample": "nearest"});
            if let Some(value) = option {
                params["scaleStyles"] = json!(value);
            }
            s.execute("image.imageSize", params).unwrap();
            let after = s.active().unwrap().doc.clone();
            assert_eq!((after.size.width, after.size.height), (40, 20));
            assert_eq!(shape_stroke(&s, id), 2.0, "{depth}/{option:?}: geometric strokes always scale");
            assert_eq!(s.active().unwrap().history.past_len(), past + 1);
            for layer in [group, id] {
                let effects = &after.layer(layer).unwrap().effects;
                let Effect::Stroke(stroke) = &effects.items[0] else { panic!("style lost") };
                assert_eq!(stroke.size, if option == Some(false) { 20.0 } else { 10.0 }, "{depth}/{option:?}: layer style stroke");
                if option == Some(false) {
                    assert_eq!(effects, &before.layer(layer).unwrap().effects, "{depth}: preserve every style setting and raw block");
                } else {
                    assert!(effects.psd_raw.is_none(), "scaled effects invalidate their old PSD block");
                }
            }
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.size, before.size);
            assert_eq!(shape_stroke(&s, id), 4.0);
            for layer in [group, id] {
                assert_eq!(s.active().unwrap().doc.layer(layer).unwrap().effects, before.layer(layer).unwrap().effects);
            }
            s.execute("edit.redo", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.size, after.size);
            assert_eq!(shape_stroke(&s, id), 2.0);
            for layer in [group, id] {
                assert_eq!(s.active().unwrap().doc.layer(layer).unwrap().effects, after.layer(layer).unwrap().effects);
            }
        }
    }
}

/// Resolution-only and unchanged-size commands leave styles intact even when scaling is enabled.
#[test]
fn image_size_without_pixel_resampling_preserves_styles() {
    for mut params in [json!({"percent": 50, "resolution": 144, "resample": "none"}), json!({"percent": 100, "resample": "nearest"})] {
        for option in [false, true] {
            let (mut s, id) = styled_shape(8);
            let before = s.active().unwrap().doc.clone();
            params["scaleStyles"] = json!(option);
            s.execute("image.imageSize", params.clone()).unwrap();
            let after = &s.active().unwrap().doc;
            assert_eq!(after.size, before.size);
            assert_eq!(shape_stroke(&s, id), 4.0);
            for layer in [before.layers[0].id, id] {
                assert_eq!(after.layer(layer).unwrap().effects, before.layer(layer).unwrap().effects);
            }
            if params["resample"] == "none" {
                assert_eq!(after.resolution_dpi, 144.0);
            }
        }
    }
}

/// Crop's shared resampling path retains its existing style-scaling default.
#[test]
fn crop_target_keeps_scaling_styles() {
    let (mut s, id) = styled_shape(8);
    let group = s.active().unwrap().doc.layers[0].id;
    s.execute("image.crop", json!({"x": 0, "y": 0, "width": 80, "height": 40, "targetWidth": 40, "targetHeight": 20, "resample": "nearest"})).unwrap();
    assert_eq!(shape_stroke(&s, id), 2.0);
    for layer in [group, id] {
        let effects = &s.active().unwrap().doc.layer(layer).unwrap().effects;
        let Effect::Stroke(stroke) = &effects.items[0] else { panic!("style lost") };
        assert_eq!(stroke.size, 10.0);
    }
}
