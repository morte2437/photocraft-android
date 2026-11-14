//! #12196: a live Gradient drag (`gradient.fill.create`) masks the new fill layer with the
//! selection and drops the selection in the same History step, while `new_layer` (used by the
//! canvas preview during the drag) masks the preview without mutating the document's selection.

use photocraft_geom::Rect;
use serde_json::json;

use super::{CREATE, LayerId, new_layer};
use crate::Session;

/// Creates an 800x600 white-background test session at the given bit depth.
fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "depth": depth, "background": "white"})).unwrap();
    s
}

/// Verifies that `new_layer` previews a selection-masked gradient without dropping the selection,
/// `gradient.fill.create` masks the new layer and drops the selection, and `edit.undo` restores it.
#[test]
fn create_masks_with_the_selection_drops_it_and_undo_restores_it() {
    let bounds = Rect::new(200, 200, 600, 400);
    let drag = json!({"from": [200, 200], "to": [600, 400], "stops": [[0, "#000000"], [1, "#000000"]]});
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        s.execute("select.rect", json!({"x": 200, "y": 200, "width": 400, "height": 200})).unwrap();

        // The live drag preview calls `new_layer`: it is masked by the selection, renders only
        // inside the selection, and leaves the document's selection intact while dragging.
        let preview = new_layer(&s, &s.active().unwrap().doc, &drag).unwrap();
        let preview_mask = preview.mask.as_ref().expect("preview is masked by the selection");
        assert_eq!(preview_mask.surface.content_bounds(), bounds, "{depth}-bit preview mask bounds");
        let mut preview_doc = (*s.active().unwrap().doc).clone();
        preview_doc.layers.push(preview);
        let flat = photocraft_compose::flatten(&preview_doc);
        assert_eq!(flat.px[300 * 800 + 300], [0.0, 0.0, 0.0, 1.0], "{depth}-bit preview inside selection");
        assert_eq!(flat.px[100 * 800 + 100], [1.0, 1.0, 1.0, 1.0], "{depth}-bit preview outside selection");
        assert!(s.active().unwrap().doc.selection.is_some(), "{depth}-bit preview keeps selection");

        let r = s.execute(CREATE, drag.clone()).unwrap();
        let id = LayerId(r["layer"].as_u64().unwrap());
        let d = s.active().unwrap();
        let l = d.doc.layer(id).unwrap();
        assert_eq!(l.name, "Gradient Fill 1", "{depth}-bit");
        let mask = l.mask.as_ref().expect("selection becomes the fill layer's mask");
        assert_eq!(mask.surface.content_bounds(), bounds, "{depth}-bit mask bounds");
        assert_eq!(mask.value(300, 300), 1.0, "{depth}-bit inside selection");
        assert_eq!(mask.value(100, 100), 0.0, "{depth}-bit outside selection");
        assert!(d.doc.selection.is_none(), "{depth}-bit selection is dropped");

        // One undo removes the layer and brings the selection back.
        s.execute("edit.undo", json!({})).unwrap();
        let d = s.active().unwrap();
        assert!(d.doc.layer(id).is_none(), "{depth}-bit undo removes the layer");
        let sel = d.doc.selection.as_ref().expect("undo brings the selection back");
        assert_eq!(sel.content_bounds(), bounds, "{depth}-bit restored selection bounds");

        // A soft/feathered selection's fractional coverage goes into the mask and survives undo.
        s.execute("select.rect", json!({"x": 200, "y": 200, "width": 400, "height": 200, "feather": 5.0})).unwrap();
        let canvas = s.active().unwrap().doc.bounds();
        let before_sel = s.active().unwrap().doc.selection.as_ref().unwrap().read_region(canvas);
        assert!(before_sel.iter().any(|&v| v > 0.0 && v < 1.0), "{depth}-bit has fractional coverage");
        let r = s.execute(CREATE, drag.clone()).unwrap();
        let feathered_id = LayerId(r["layer"].as_u64().unwrap());
        let feathered_mask = s.active().unwrap().doc.layer(feathered_id).unwrap().mask.as_ref().unwrap();
        assert_eq!(feathered_mask.surface.read_region(canvas), before_sel, "{depth}-bit mask carries fractional coverage");
        assert!(s.active().unwrap().doc.selection.is_none(), "{depth}-bit feathered selection dropped");
        s.execute("edit.undo", json!({})).unwrap();
        let after_sel = s.active().unwrap().doc.selection.as_ref().unwrap().read_region(canvas);
        assert_eq!(after_sel, before_sel, "{depth}-bit undo restores fractional selection coverage");
    }
}

/// Verifies that `gradient.fill.create` without an active selection leaves the new layer unmasked.
#[test]
fn create_without_a_selection_adds_an_unmasked_fill_layer() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let r = s.execute(CREATE, json!({"from": [100, 100], "to": [700, 500]})).unwrap();
        let id = LayerId(r["layer"].as_u64().unwrap());
        let d = s.active().unwrap();
        let l = d.doc.layer(id).unwrap();
        assert!(l.mask.is_none(), "{depth}-bit without selection has no mask");
        assert!(d.doc.selection.is_none(), "{depth}-bit still has no selection");
    }
}
