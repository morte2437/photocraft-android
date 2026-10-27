use std::sync::Arc;

use photocraft_doc::{LayerContent, SmartObject, SmartSource};
use photocraft_engine::{EngineError, Session};
use serde_json::{Value, json};

fn deny_blur(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "filter.blur.gaussianBlur" { Err(EngineError::Other("denied".into())) } else { Ok(()) }
}

fn edits() -> [(&'static str, Value); 2] {
    [("edit.transform", json!({"scaleX": 50, "scaleY": 100})), ("image.imageSize", json!({"width": 100, "height": 50}))]
}

fn blurred_smart_object() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 20, "y": 20, "width": 100, "height": 50})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    s
}

fn cache_width(s: &Session) -> i32 {
    let st = s.active().unwrap();
    let LayerContent::Smart(sm) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("not a smart object") };
    sm.cache.as_ref().unwrap().content_bounds().width() as i32
}

fn edit_smart_object(s: &mut Session, edit: impl FnOnce(&mut SmartObject)) {
    let st = s.active_mut().unwrap();
    let id = st.active_layer.unwrap();
    let LayerContent::Smart(sm) = &mut Arc::make_mut(&mut st.doc).layer_mut(id).unwrap().content else { panic!("not a smart object") };
    edit(sm);
}

#[test]
fn a_smart_object_whose_filters_are_denied_keeps_the_resampled_fallback() {
    for (cmd, params) in edits() {
        let mut s = blurred_smart_object();
        let before = cache_width(&s);
        s.authorize = Some(deny_blur);
        s.execute(cmd, params).unwrap();
        let after = cache_width(&s);
        assert!(after < before * 3 / 4, "{cmd}: the fallback stayed at {after} px wide (was {before})");
    }
}

#[test]
fn a_smart_object_that_fails_to_re_render_keeps_the_resampled_fallback() {
    let unrenderable =
        [SmartSource::Linked { path: "missing.psd".into() }, SmartSource::Embedded { file_name: "broken.png".into(), bytes: Arc::new(vec![0; 16]) }];
    for (cmd, params) in edits() {
        for source in &unrenderable {
            let mut s = blurred_smart_object();
            let before = cache_width(&s);
            edit_smart_object(&mut s, |sm| sm.source = source.clone());
            s.execute(cmd, params.clone()).unwrap();
            let after = cache_width(&s);
            assert!(after < before * 3 / 4, "{cmd} with {source:?}: the fallback stayed at {after} px wide (was {before})");
        }
    }
}

#[test]
fn an_allowed_smart_object_still_re_renders_at_the_new_size() {
    for (cmd, params) in edits() {
        let mut s = blurred_smart_object();
        let before = cache_width(&s);
        s.execute(cmd, params).unwrap();
        let after = cache_width(&s);
        assert!(after < before * 3 / 4, "{cmd}: {after} px wide (was {before})");
    }
}
