//! `path.deleteAnchor` on a path's last anchor (#12764): a saved path is left empty and stays
//! listed; the Work Path and a vector mask keep refusing it.

use photocraft_doc::text::TextShape;
use photocraft_doc::{Document, LayerContent, LayerId, Path, Subpath};
use serde_json::{Value, json};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 400, "background": "white"})).unwrap();
    s
}

fn lone() -> Value {
    json!({"subpaths": [{"closed": false, "knots": [[100, 300]]}]})
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

/// "Path 1" with one subpath of one anchor, emptied by deleting that anchor.
fn emptied() -> Session {
    let mut s = session();
    s.execute("path.set", json!({"name": "Path 1", "path": lone()})).unwrap();
    s.execute("path.deleteAnchor", json!({"name": "Path 1", "subpath": 0, "knot": 0})).unwrap();
    s
}

#[test]
fn deleting_a_saved_paths_last_anchor_leaves_it_empty_in_one_undoable_step() {
    let mut s = session();
    s.execute("path.set", json!({"name": "Path 1", "path": lone()})).unwrap();
    let before = doc(&s).paths[0].path.clone();
    let steps = s.active().unwrap().history.entries().len();
    let r = s.execute("path.deleteAnchor", json!({"name": "Path 1", "subpath": 0, "knot": 0})).unwrap();
    assert_eq!(r["subpathRemoved"], json!(true));
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Delete Anchor Point"));
    let list = s.execute("path.list", json!({})).unwrap();
    assert_eq!(list["paths"], json!([{"name": "Path 1", "knots": 0, "subpaths": 0}]), "Path 1 stays in the Paths panel");
    let info = s.execute("path.info", json!({"name": "Path 1"})).unwrap();
    assert_eq!(info["path"]["subpaths"], json!([]));
    assert!(s.undo());
    assert_eq!(doc(&s).paths[0].path, before, "Undo brings the anchor back");
    assert!(s.redo());
    assert!(doc(&s).paths[0].path.subpaths.is_empty());
    assert_eq!(s.execute("path.list", json!({})).unwrap()["paths"], json!([{"name": "Path 1", "knots": 0, "subpaths": 0}]));
}

/// ⌫ / Delete on the Add Anchor Point tool's selected anchor (`segments`, #12703) follows the same
/// rule for the path's only anchor, which has no segments.
#[test]
fn deleting_the_only_anchor_with_its_segments_empties_a_saved_path_and_refuses_the_work_path() {
    let s = &mut session();
    s.execute("path.set", json!({"name": "Path 1", "path": lone()})).unwrap();
    let r = s.execute("path.deleteAnchor", json!({"name": "Path 1", "subpath": 0, "knot": 0, "segments": true})).unwrap();
    assert_eq!(r["subpathRemoved"], json!(true));
    assert!(doc(s).paths[0].path.subpaths.is_empty());
    s.execute("path.set", json!({"path": lone()})).unwrap();
    let e = s.execute("path.deleteAnchor", json!({"subpath": 0, "knot": 0, "segments": true})).unwrap_err();
    assert!(e.to_string().contains("only anchor"), "{e}");
    assert_eq!(doc(s).work_path.as_ref().map(|p| p.subpaths.len()), Some(1));
}

#[test]
fn the_work_paths_and_a_vector_masks_last_anchor_are_still_refused() {
    let mut s = session();
    s.execute("path.set", json!({"path": lone()})).unwrap();
    let steps = s.active().unwrap().history.entries().len();
    let e = s.execute("path.deleteAnchor", json!({"subpath": 0, "knot": 0})).unwrap_err();
    assert!(e.to_string().contains("only anchor"), "{e}");
    assert_eq!(s.active().unwrap().history.entries().len(), steps);
    assert_eq!(doc(&s).work_path.as_ref().map(|p| p.subpaths.len()), Some(1));
    s.execute("layer.new.layer", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap().0;
    s.execute("layer.vectorMask.add", json!({"layer": id, "path": lone()})).unwrap();
    let steps_mask = s.active().unwrap().history.entries().len();
    let e = s.execute("path.deleteAnchor", json!({"name": "layer", "layer": id, "subpath": 0, "knot": 0})).unwrap_err();
    assert!(e.to_string().contains("only anchor"), "{e}");
    assert_eq!(doc(&s).layer(LayerId(id)).unwrap().vector_mask.as_ref().unwrap().path.subpaths.len(), 1);
    assert_eq!(s.active().unwrap().history.entries().len(), steps_mask);
}

#[test]
fn an_empty_saved_path_selects_fills_and_strokes_nothing() {
    let mut s = emptied();
    s.execute("layer.new.layer", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    let pixels = |s: &Session| doc(s).layer(id).unwrap().surface().unwrap().clone();
    let before = pixels(&s);
    assert_eq!(s.execute("path.toSelection", json!({"name": "Path 1"})).unwrap(), json!({"selected": false}));
    assert!(doc(&s).selection.is_none());
    s.execute("path.fill", json!({"name": "Path 1", "color": "#ff0000"})).unwrap();
    let r = s.execute("path.stroke", json!({"name": "Path 1", "tool": "pencil", "size": 4, "color": "#0000ff"})).unwrap();
    assert_eq!(r["damage"], json!([0, 0, 0, 0]));
    assert!(pixels(&s) == before, "an empty path paints nothing");
}

#[test]
fn an_empty_saved_path_survives_pcraft_and_psd_round_trips() {
    let s = emptied();
    let bytes = photocraft_format::save_to_bytes(doc(&s), &Default::default()).unwrap();
    let back = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(back.paths.iter().map(|p| (p.name.as_str(), p.path.subpaths.len())).collect::<Vec<_>>(), [("Path 1", 0)]);
    let psd = photocraft_io::export(doc(&s), "x.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("x.psd", &psd).unwrap().document;
    assert_eq!(back.paths.iter().map(|p| (p.name.as_str(), p.path.subpaths.len())).collect::<Vec<_>>(), [("Path 1", 0)]);
}

#[test]
fn an_empty_clipping_path_survives_a_psd_round_trip() {
    let mut s = emptied();
    s.execute("path.clippingPath.set", json!({"name": "Path 1"})).unwrap();
    let psd = photocraft_io::export(doc(&s), "x.psd", &Default::default()).unwrap().bytes;
    let back = photocraft_io::import("x.psd", &psd).unwrap().document;
    assert_eq!(back.paths.iter().map(|p| (p.name.as_str(), p.path.subpaths.len())).collect::<Vec<_>>(), [("Path 1", 0)]);
    assert_eq!(back.clipping_path.map(|c| c.name).as_deref(), Some("Path 1"));
}

/// A Type Path is not a saved path: its only anchor stays refused. No command makes a one-anchor
/// Type Path (attaching and editing need two points), so the document is trimmed directly.
#[test]
fn a_type_paths_last_anchor_is_still_refused() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 20, "y": 70, "text": "Hello", "size": 36, "color": "#000000"})).unwrap()["layer"].as_u64().unwrap();
    s.edit("path", |doc, _| {
        doc.work_path = Some(Path::new(vec![Subpath::polyline(&[(150.0, 10.0), (150.0, 110.0)])]));
        Ok(())
    })
    .unwrap();
    s.execute("type.attachToPath", json!({"layer": id})).unwrap();
    s.edit("trim", |doc, _| {
        if let Some(LayerContent::Text(t)) = doc.layer_mut(LayerId(id)).map(|l| &mut l.content)
            && let TextShape::Path { path, .. } = &mut t.shape
        {
            path.knots.truncate(1);
        }
        Ok(())
    })
    .unwrap();
    let type_path = |s: &Session| match &doc(s).layer(LayerId(id)).unwrap().content {
        LayerContent::Text(t) => crate::type_extra_cmds::type_path(t),
        _ => None,
    };
    let before = type_path(&s).unwrap();
    assert_eq!(before.subpaths.iter().map(|sp| sp.knots.len()).collect::<Vec<_>>(), [1]);
    let steps = s.active().unwrap().history.entries().len();
    let e = s.execute("path.deleteAnchor", json!({"name": "layer", "layer": id, "typePath": true, "subpath": 0, "knot": 0})).unwrap_err();
    assert!(e.to_string().contains("only anchor"), "{e}");
    assert_eq!(type_path(&s), Some(before));
    assert_eq!(s.active().unwrap().history.entries().len(), steps);
}

/// ⌫ / Delete on the Delete Anchor Point tool's last anchor of a subpath among others (#12761)
/// takes the subpath as one Delete Selection step; the tool's click keeps Delete Anchor Point.
#[test]
fn deleting_a_subpaths_last_anchor_with_its_segments_is_delete_selection() {
    let two = || json!({"subpaths": [{"closed": false, "knots": [[100, 300], [200, 300]]}, {"closed": false, "knots": [[400, 300]]}]});
    let s = &mut session();
    s.execute("path.set", json!({"name": "Path 1", "path": two()})).unwrap();
    let before = doc(s).paths[0].path.clone();
    let r = s.execute("path.deleteAnchor", json!({"name": "Path 1", "subpath": 1, "knot": 0, "segments": true})).unwrap();
    assert_eq!(r["subpathRemoved"], json!(true));
    assert_eq!(doc(s).paths[0].path.subpaths.len(), 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Delete Selection"));
    assert!(s.undo());
    assert_eq!(doc(s).paths[0].path, before);
    s.execute("path.deleteAnchor", json!({"name": "Path 1", "subpath": 1, "knot": 0})).unwrap();
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Delete Anchor Point"), "the click's label is unchanged");
}
