use photocraft_doc::{LayerContent, LayerId, SmartFilter};
use photocraft_engine::{Session, smart_cmds};
use serde_json::json;

const STORED: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

fn mockup() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("imported filter", |doc, _| {
        let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("not a smart object") };
        sm.smart_filters.push(SmartFilter {
            command: "psd.unsupportedFilter".into(),
            params: json!({"name": "Sharpen", "filterID": 1399353968, "psd": ""}),
            blend: photocraft_color::BlendMode::Normal,
            opacity: 1.0,
            visible: true,
        });
        let c = sm.cache.as_mut().unwrap();
        let r = c.content_bounds();
        c.fill_rect(r, &STORED);
        Ok(())
    })
    .unwrap();
    (s, id)
}

fn cache(s: &Session, id: LayerId) -> photocraft_raster::Surface {
    let LayerContent::Smart(sm) = &s.active().unwrap().doc.layer(id).unwrap().content else { panic!("not a smart object") };
    sm.cache.clone().unwrap()
}

fn png() -> Vec<u8> {
    let image = photocraft_codecs::Image::from_u8(8, 8, photocraft_codecs::ChannelLayout::Rgb, vec![0; 8 * 8 * 3]).unwrap();
    photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap()
}

#[test]
fn geometry_edits_keep_the_stored_render_of_an_unsupported_filter() {
    for (cmd, params) in [("edit.transform", json!({"scaleX": 50, "scaleY": 50})), ("image.imageSize", json!({"width": 20, "height": 15}))] {
        let (mut s, id) = mockup();
        let before = cache(&s, id).content_bounds();
        s.execute(cmd, params).unwrap();
        let after = cache(&s, id);
        let r = after.content_bounds();
        assert!(r.width() < before.width(), "{cmd} resized the stored render");
        assert_eq!(after.rgba((r.x0 + r.x1) / 2, (r.y0 + r.y1) / 2), STORED, "{cmd} kept the stored render");
    }
}

#[test]
fn replacing_contents_reports_the_filters_it_cannot_apply() {
    let (mut s, id) = mockup();
    let r = smart_cmds::replace_contents_bytes(&mut s, id, "new.png", png()).unwrap();
    let warnings = r["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{r}");
    assert!(warnings[0].as_str().unwrap().contains("Sharpen"), "{r}");
}

#[test]
fn supported_filters_still_re_render_after_a_transform() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("stale cache", |doc, _| {
        let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!("not a smart object") };
        let c = sm.cache.as_mut().unwrap();
        let r = c.content_bounds();
        c.fill_rect(r, &STORED);
        Ok(())
    })
    .unwrap();
    s.execute("edit.transform", json!({"scaleX": 50, "scaleY": 50})).unwrap();
    let c = cache(&s, id);
    let r = c.content_bounds();
    assert_ne!(c.rgba((r.x0 + r.x1) / 2, (r.y0 + r.y1) / 2), STORED, "a supported filter re-renders from the source");
}
