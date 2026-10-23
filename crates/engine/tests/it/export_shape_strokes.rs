//! Regression coverage for #8341: canvas resizing must scale shape strokes too.
use photocraft_doc::{LayerContent, LayerId};
use photocraft_engine::{
    Session,
    web_cmds::{WebSettings, web_document},
};
use serde_json::json;

fn shape_session(depth: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1024, "height": 1024, "depth": depth, "background": "transparent"})).unwrap();
    let id = s.execute("shape.create", json!({"rect": [256, 256, 512, 512], "fill": null, "stroke": {"width": 32, "color": "#000000"}})).unwrap()["layer"]
        .as_u64()
        .unwrap();
    (s, LayerId(id))
}

#[test]
fn web_downscale_keeps_stroke_only_shape_proportions_at_every_depth() {
    for depth in [8, 16, 32] {
        let (s, id) = shape_session(depth);
        let original = &s.active().unwrap().doc;
        for params in [json!({"percent": 25}), json!({"width": 256, "height": 256})] {
            let (scaled, sx, sy) = web_document(original, &params, &WebSettings::default()).unwrap();
            assert_eq!((scaled.size.width, scaled.size.height), (256, 256));
            assert_eq!((sx, sy), (0.25, 0.25));
            let LayerContent::Shape(sh) = &scaled.layer(id).unwrap().content else {
                panic!("shape lost");
            };
            assert_eq!(sh.stroke.as_ref().unwrap().width, 8.0, "{depth}-bit stroke");
            let pixels = photocraft_compose::flatten(&scaled);
            let alpha = |x: usize| pixels.px[128 * 256 + x][3];
            assert!(alpha(62) > 0.99, "scaled stroke is visible");
            assert!(alpha(52) < 0.01, "stroke must not retain its original width");
            assert!(alpha(128) < 0.01, "no fill is introduced");
        }
        let LayerContent::Shape(sh) = &original.layer(id).unwrap().content else {
            panic!("shape lost");
        };
        assert_eq!(sh.stroke.as_ref().unwrap().width, 32.0, "export leaves the source untouched");
    }
}

#[test]
fn image_size_scales_strokes_once_and_undo_restores_them() {
    let (mut s, id) = shape_session(8);
    s.execute("shape.edit", json!({"layer": id.0, "stroke": {"dashes": [2, 1], "dashOffset": 0.5}})).unwrap();
    let before = s.active().unwrap().doc.layer(id).unwrap().clone();
    s.execute("image.imageSize", json!({"width": 512, "height": 512})).unwrap();
    let LayerContent::Shape(sh) = &s.active().unwrap().doc.layer(id).unwrap().content else {
        panic!("shape lost");
    };
    assert_eq!(sh.stroke.as_ref().unwrap().width, 16.0);
    assert_eq!(sh.stroke.as_ref().unwrap().dashes, vec![2.0, 1.0], "dash lengths stay in stroke-width units");
    assert_eq!(sh.stroke.as_ref().unwrap().dash_offset, 0.5);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer(id).unwrap(), &before);
    s.execute("edit.redo", json!({})).unwrap();
    let LayerContent::Shape(sh) = &s.active().unwrap().doc.layer(id).unwrap().content else {
        panic!("shape lost");
    };
    assert_eq!(sh.stroke.as_ref().unwrap().width, 16.0);
}

#[test]
fn grouped_strokes_scale_up_and_unchanged_size_is_a_noop() {
    let (mut s, id) = shape_session(8);
    s.edit("Group", |doc, _| {
        let children = std::mem::take(&mut doc.layers);
        doc.layers.push(photocraft_doc::Layer::group("Outer", vec![photocraft_doc::Layer::group("Inner", children)]));
        Ok(())
    })
    .unwrap();
    let original = &s.active().unwrap().doc;
    for (width, expected) in [(1024, 32.0), (1280, 40.0), (256, 8.0)] {
        let (scaled, _, _) = web_document(original, &json!({"width": width}), &WebSettings::default()).unwrap();
        let LayerContent::Shape(sh) = &scaled.layer(id).unwrap().content else {
            panic!("shape lost");
        };
        assert_eq!(sh.stroke.as_ref().unwrap().width, expected);
    }
}

#[test]
fn png_export_contains_the_scaled_stroke_and_keeps_the_source_document() {
    let (mut s, _) = shape_session(8);
    let before = s.active().unwrap().doc.clone();
    let mut bytes = Vec::new();
    photocraft_engine::web_cmds::save_for_web_with_writer(
        &mut s,
        &json!({"format": "png24", "path": "stroke.png", "width": 256, "height": 256}),
        &mut |_, encoded| {
            bytes = encoded.to_vec();
            Ok(())
        },
    )
    .unwrap();
    let image = photocraft_codecs::decode(&bytes).unwrap().convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
    assert_eq!(image.dimensions(), (256, 256));
    let alpha = |x: usize| image.data()[(128 * 256 + x) * 4 + 3];
    assert_eq!(alpha(62), 255);
    assert_eq!(alpha(52), 0);
    assert_eq!(alpha(128), 0);
    assert_eq!(s.active().unwrap().doc, before);
}
