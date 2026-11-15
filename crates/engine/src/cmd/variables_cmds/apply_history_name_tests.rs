//! #14498: Apply Data Set adds one history state named exactly "Apply Data Set", whatever the set is called.

use super::*;
use photocraft_doc::{Layer, TextLayer};
use photocraft_geom::Rect;

const NAME: &str = "Apply Data Set";

/// A document with a raster "photo", a togglable raster "badge" and a text layer "title".
fn session() -> (Session, LayerId, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let ids = s
        .edit("setup", |doc, _| {
            let fmt = doc.pixel_format();
            let mut photo = Layer::raster("photo", fmt);
            photo.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 64, 48), &photocraft_raster::from_rgba(&fmt, [0.5, 0.5, 0.5, 1.0]));
            let mut badge = Layer::raster("badge", fmt);
            badge.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 10, 10), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
            let title = Layer::new("title", LayerContent::Text(TextLayer { text: "Old".into(), ..Default::default() }));
            let (p, b, t) = (photo.id, badge.id, title.id);
            doc.layers.push(photo);
            doc.layers.push(badge);
            doc.layers.push(title);
            Ok((p, b, t))
        })
        .unwrap();
    (s, ids.0, ids.1, ids.2)
}

fn entries(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries().into_iter().map(|e| e.to_string()).collect()
}

fn define_visibility(s: &mut Session, badge: LayerId) {
    s.execute("image.variables.define", json!({"defs": [{"name": "showBadge", "layer": badge.0, "type": "visibility"}]})).unwrap();
}

fn define_sets(s: &mut Session, names: &[&str]) {
    let sets: Vec<Value> = names.iter().map(|n| json!({"name": n, "values": [{"variable": "showBadge", "kind": "visibility", "value": false}]})).collect();
    s.execute("image.variables.dataSets", json!({"dataSets": sets})).unwrap();
}

#[test]
fn applying_by_name_adds_one_state_named_apply_data_set() {
    for set in ["Data Set 2", "Zed"] {
        let (mut s, _p, badge, _t) = session();
        define_visibility(&mut s, badge);
        define_sets(&mut s, &["First", set]);
        let before = entries(&s);
        s.execute("image.applyDataSet", json!({"name": set})).unwrap();
        let after = entries(&s);
        assert_eq!(after.len(), before.len() + 1, "{set}: exactly one new state");
        assert_eq!(after.last().map(String::as_str), Some(NAME), "{set}");
    }
}

#[test]
fn applying_by_index_adds_one_state_named_apply_data_set() {
    let (mut s, _p, badge, _t) = session();
    define_visibility(&mut s, badge);
    define_sets(&mut s, &["First", "Data Set 2"]);
    let before = entries(&s);
    s.execute("image.applyDataSet", json!({"index": 1})).unwrap();
    let after = entries(&s);
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(after.last().map(String::as_str), Some(NAME));
}

#[test]
fn one_undo_reverts_visibility_text_and_pixels_together() {
    let (mut s, photo, badge, title) = session();
    let dir = std::env::temp_dir().join(format!("pc-vars-{}-apply-history-name", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("blue.png").to_string_lossy().into_owned();
    {
        let mut src = Session::new();
        src.execute("file.new", json!({"width": 8, "height": 8, "background": "#0000ff"})).unwrap();
        let bytes = photocraft_io::export(&src.active().unwrap().doc, "png", &photocraft_io::ExportOptions::default()).unwrap().bytes;
        std::fs::write(&png, bytes).unwrap();
    }
    s.execute(
        "image.variables.define",
        json!({"defs": [
            {"name": "showBadge", "layer": badge.0, "type": "visibility"},
            {"name": "headline", "layer": title.0, "type": "textReplacement"},
            {"name": "img", "layer": photo.0, "type": "pixelReplacement", "method": "conform"},
        ]}),
    )
    .unwrap();
    s.execute(
        "image.variables.dataSets",
        json!({"dataSets": [{"name": "Data Set 2", "values": [
            {"variable": "showBadge", "kind": "visibility", "value": false},
            {"variable": "headline", "kind": "text", "value": "Hello"},
            {"variable": "img", "kind": "pixels", "value": png},
        ]}]}),
    )
    .unwrap();
    let pixel_before = s.execute("document.pixel", json!({"x": 32, "y": 24})).unwrap();
    let count = entries(&s).len();

    s.execute("image.applyDataSet", json!({"name": "Data Set 2"})).unwrap();
    assert_eq!(entries(&s).len(), count + 1);
    assert_eq!(entries(&s).last().map(String::as_str), Some(NAME));
    assert_ne!(s.execute("document.pixel", json!({"x": 32, "y": 24})).unwrap(), pixel_before, "pixels replaced");

    assert!(s.undo());
    let d = &s.active().unwrap().doc;
    assert!(d.layer(badge).unwrap().visible, "visibility reverted");
    match &d.layer(title).unwrap().content {
        LayerContent::Text(t) => assert_eq!(t.text, "Old", "text reverted"),
        _ => panic!("not a text layer"),
    }
    assert_eq!(s.execute("document.pixel", json!({"x": 32, "y": 24})).unwrap(), pixel_before, "pixels reverted");
    assert_eq!(entries(&s).len(), count, "the apply was one state");
    let _ = std::fs::remove_dir_all(&dir);
}
