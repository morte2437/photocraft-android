//! #14593: Image › Duplicate leaves the copy without a selection, as the reference application
//! does, for a full copy and a merged-layers-only copy. The source keeps its selection, and the
//! copy keeps the source's alpha channels, guides and paths.

use photocraft_doc::Document;
use photocraft_geom::Rect;
use serde_json::json;

use crate::Session;

/// A source at `depth` with the issue's selection (10,10-110,60), one alpha channel saved from
/// it, a guide each way and a saved path.
fn source(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100, "depth": depth, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    s.execute("select.saveSelection", json!({"name": "Alpha 1"})).unwrap();
    s.execute("view.newGuide", json!({"orientation": "vertical", "position": 40})).unwrap();
    s.execute("view.newGuide", json!({"orientation": "horizontal", "position": 20})).unwrap();
    s.execute("path.set", json!({"name": "Saved", "path": {"subpaths": [{"knots": [[2, 2], [12, 2], [12, 8]]}]}})).unwrap();
    s
}

fn doc(s: &Session, index: usize) -> &Document {
    &s.documents()[index].doc
}

fn selection(d: &Document) -> Option<Rect> {
    d.selection.as_ref().map(|sel| sel.content_bounds())
}

#[test]
fn a_duplicate_has_no_selection_and_the_source_keeps_its_own() {
    for depth in [8, 16, 32] {
        for merged in [false, true] {
            let mut s = source(depth);
            let before = selection(doc(&s, 0));
            assert_eq!(before, Some(Rect::from_xywh(10, 10, 100, 50)), "{depth}-bit source has the selection");
            let r = s.execute("image.duplicate", json!({"mergedOnly": merged})).unwrap();
            assert_eq!(r["document"], json!(1));
            assert_eq!(s.active_index(), Some(1), "the copy is active");
            let copy = doc(&s, 1);
            assert!(copy.selection.is_none(), "{depth}-bit copy (mergedOnly {merged}) has no selection");
            assert_eq!(selection(doc(&s, 0)), before, "{depth}-bit source (mergedOnly {merged}) keeps its selection");
        }
    }
}

#[test]
fn a_duplicate_keeps_alpha_channels_guides_and_paths() {
    for merged in [false, true] {
        let mut s = source(8);
        s.execute("image.duplicate", json!({"mergedOnly": merged})).unwrap();
        let (src, copy) = (doc(&s, 0), doc(&s, 1));
        assert_eq!(copy.channels.len(), 1, "mergedOnly {merged}: the alpha channel is copied");
        assert_eq!(copy.channels[0].name, src.channels[0].name);
        assert_eq!(copy.guides.vertical, vec![40.0], "mergedOnly {merged}: vertical guide copied");
        assert_eq!(copy.guides.horizontal, vec![20.0], "mergedOnly {merged}: horizontal guide copied");
        assert_eq!(copy.paths.len(), src.paths.len(), "mergedOnly {merged}: paths copied");
        assert!(copy.paths.iter().any(|p| p.name == "Saved"), "mergedOnly {merged}: the saved path is copied");
    }
}
