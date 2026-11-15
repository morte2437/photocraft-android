//! #13702: Contact Sheet II with Flatten All Layers ends as one normal layer named
//! "Contact Sheet II" (history `Flatten Image | Make Layer`), not as a Background.

use super::tests::{images, tmp};
use super::*;
use photocraft_doc::{BlendMode, Locks};

fn sheet(s: &mut Session, dir: &str, extra: Value) -> Vec<usize> {
    let mut p = json!({"input": dir, "units": "pixels", "width": 300, "height": 200, "resolution": 72, "columns": 2, "rows": 2, "caption": false});
    if let (Some(p), Value::Object(extra)) = (p.as_object_mut(), extra) {
        p.extend(extra);
    }
    let r = s.execute("file.automate.contactSheetII", p).unwrap();
    r["documents"].as_array().unwrap().iter().map(|d| d.as_u64().unwrap() as usize).collect()
}

#[test]
fn a_flattened_sheet_ends_as_one_normal_layer_named_contact_sheet_ii() {
    let dir = tmp("flatten-layer");
    images(&dir, 3, 60, 30);
    let mut s = Session::new();
    let idx = sheet(&mut s, &dir, json!({"flatten": true, "caption": true, "fontSize": 10}))[0];
    let st = &s.documents()[idx];
    let d = &st.doc;
    assert_eq!(d.layers.len(), 1);
    let l = &d.layers[0];
    assert_eq!(l.name, "Contact Sheet II");
    assert!(d.background().is_none() && !d.is_background(l.id), "a normal layer, not a Background");
    assert_eq!(l.locks, Locks::default(), "nothing locked");
    assert!(matches!(l.content, LayerContent::Raster(_)) && l.visible && l.mask.is_none());
    assert_eq!((l.opacity, l.fill_opacity, l.blend), (1.0, 1.0, BlendMode::Normal));
    assert_eq!(st.active_layer, Some(l.id));
    let h = st.history.entries();
    assert_eq!(h[h.len() - 2..], ["Flatten Image", "Make Layer"], "{h:?}");
}

#[test]
fn flattening_keeps_the_pixels_of_the_unflattened_sheet() {
    let dir = tmp("flatten-pixels");
    images(&dir, 3, 60, 30);
    let mut s = Session::new();
    let layered = sheet(&mut s, &dir, json!({}))[0];
    let flat = sheet(&mut s, &dir, json!({"flatten": true}))[0];
    let (layered, flat) = (&s.documents()[layered].doc, &s.documents()[flat].doc);
    assert_eq!(flat.layers[0].surface().unwrap().content_bounds(), flat.bounds(), "covers the whole sheet");
    assert_eq!(photocraft_compose::flatten(flat), photocraft_compose::flatten(layered));
}

#[test]
fn every_page_of_a_flattened_multi_page_run_gets_the_layer() {
    let dir = tmp("flatten-pages");
    images(&dir, 5, 60, 30);
    let mut s = Session::new();
    let docs = sheet(&mut s, &dir, json!({"flatten": true}));
    assert_eq!(docs.len(), 2);
    for idx in docs {
        let d = &s.documents()[idx].doc;
        let names: Vec<&str> = d.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Contact Sheet II"], "page {idx}");
        assert!(d.background().is_none(), "page {idx}");
    }
}

#[test]
fn an_unflattened_sheet_keeps_its_background_and_image_layers() {
    let dir = tmp("flatten-off");
    images(&dir, 3, 60, 30);
    let mut s = Session::new();
    let idx = sheet(&mut s, &dir, json!({"flatten": false}))[0];
    let st = &s.documents()[idx];
    let names: Vec<&str> = st.doc.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Background", "img0", "img1", "img2"]);
    assert!(st.doc.is_background(st.doc.layers[0].id));
    assert!(!st.history.entries().iter().any(|e| e == "Make Layer"));
}
