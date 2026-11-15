//! #14029: a document made with File › New starts with the History state "New", as in the
//! reference application; a file that opens keeps "Open".

use serde_json::{Value, json};

use crate::Session;

/// The History states and snapshots of the active document, as `document.history` and
/// `document.snapshots` report them.
fn history(s: &Session) -> (Value, Value) {
    let d = crate::inspect::document(s.active().unwrap());
    (d["history"].clone(), d["snapshots"].clone())
}

/// Open an 8 × 8 PNG from memory as `name`, the way File › Open admits a file.
fn open_png(s: &mut Session, name: &str) {
    let img = photocraft_codecs::Image::from_u8(8, 8, photocraft_codecs::ChannelLayout::Rgb, vec![128; 8 * 8 * 3]).unwrap();
    let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    crate::file_cmds::open_bytes_as(s, name, &bytes, None, None).unwrap();
}

#[test]
fn the_new_document_dialogs_document_has_the_state_new_under_its_snapshot() {
    // The measured case: White, 13 x 400 px at 1 ppi, created as Untitled-1.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 13, "height": 400, "resolution": 1, "background": "white", "name": "Untitled-1"})).unwrap();
    assert_eq!(history(&s), (json!(["New"]), json!([{"id": 0, "name": "Untitled-1"}])));
}

#[test]
fn an_artboard_document_has_the_state_new_too() {
    // The measured case: Artboards on, 11 x 8 in at 300 ppi, White.
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 3300, "height": 2400, "resolution": 300, "artboard": true, "name": "Untitled-1"})).unwrap();
    assert_eq!(history(&s), (json!(["New"]), json!([{"id": 0, "name": "Untitled-1"}])));
}

#[test]
fn every_background_and_mode_of_file_new_gives_new() {
    for p in [
        json!({"width": 8, "height": 8, "background": "transparent"}),
        json!({"width": 8, "height": 8, "background": "backgroundColor"}),
        json!({"width": 8, "height": 8, "background": "black", "mode": "cmyk", "depth": 16}),
        json!({"width": 8, "height": 8, "background": "#336699", "mode": "lab", "depth": 32}),
    ] {
        let mut s = Session::new();
        s.execute("file.new", p.clone()).unwrap();
        assert_eq!(history(&s).0, json!(["New"]), "{p}");
    }
}

#[test]
fn the_first_state_stays_new_through_undo_and_redo_and_is_the_default_brush_source() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let source = |s: &Session| crate::inspect::document(s.active().unwrap())["historySource"].clone();
    assert_eq!(source(&s), json!({"state": 0, "label": "New"}));
    s.execute("layer.new.layer", json!({})).unwrap();
    assert_eq!(history(&s).0, json!(["New", "New Layer"]));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(history(&s).0, json!(["New"]));
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(history(&s).0, json!(["New", "New Layer"]));
    assert_eq!(source(&s), json!({"state": 0, "label": "New"}));
}

#[test]
fn an_opened_file_keeps_open_next_to_a_new_document() {
    let mut s = Session::new();
    open_png(&mut s, "before.png");
    s.execute("file.new", json!({"width": 8, "height": 8, "name": "Untitled-1"})).unwrap();
    open_png(&mut s, "after.png");
    let states: Vec<(String, Vec<String>)> = s.documents().iter().map(|d| (d.doc.name.clone(), d.history.entries())).collect();
    assert_eq!(
        states,
        [
            ("before.png".to_string(), vec!["Open".to_string()]),
            ("Untitled-1".to_string(), vec!["New".to_string()]),
            ("after.png".to_string(), vec!["Open".to_string()]),
        ]
    );
}
