//! Layer > Smart Objects > Embed Linked and Embed All Linked (#11401).

use photocraft_doc::{LayerContent, SmartSource};
use serde_json::json;

use crate::Session;
use crate::cmd::command_specs;

fn label(id: &str) -> &'static str {
    command_specs().iter().find(|c| c.id == id).map(|c| c.label).unwrap_or_else(|| panic!("{id} is not registered"))
}

/// Adds a layer with a few painted pixels (an empty layer can't become a smart object).
fn painted_layer(s: &mut Session, red: f32) {
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("setup", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surf.fill_rect(photocraft_geom::Rect::new(2, 2, 10, 10), &[red, 1.0 - red, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
}

/// Three smart objects: the first two linked to files in `dir`, the third embedded.
fn three(dir: &std::path::Path) -> (Session, [u64; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
    let mut ids = [0u64; 3];
    for (i, slot) in ids.iter_mut().enumerate() {
        painted_layer(&mut s, 0.25 * (i + 1) as f32);
        *slot = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
        if i < 2 {
            let path = dir.join(format!("linked{i}.pcraft"));
            s.execute("layer.smartObjects.convertToLinked", json!({"path": path.to_str().unwrap()})).unwrap();
        }
    }
    (s, ids)
}

fn is_linked(s: &Session, id: u64) -> bool {
    let doc = &s.active().unwrap().doc;
    let layer = doc.walk().into_iter().map(|(_, _, l)| l).find(|l| l.id.0 == id).unwrap();
    match &layer.content {
        LayerContent::Smart(sm) => matches!(sm.source, SmartSource::Linked { .. }),
        other => panic!("not smart: {}", other.kind_name()),
    }
}

/// The file name and bytes an embedded smart object carries.
fn embedded(s: &Session, id: u64) -> (String, Vec<u8>) {
    let doc = &s.active().unwrap().doc;
    let layer = doc.walk().into_iter().map(|(_, _, l)| l).find(|l| l.id.0 == id).unwrap();
    match &layer.content {
        LayerContent::Smart(sm) => match &sm.source {
            SmartSource::Embedded { file_name, bytes } => (file_name.clone(), bytes.to_vec()),
            SmartSource::Linked { .. } => panic!("still linked"),
        },
        other => panic!("not smart: {}", other.kind_name()),
    }
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pc-embed-all-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn embed_linked_and_embed_all_linked_are_named_in_the_engine() {
    assert_eq!(label("layer.smartObjects.convertToEmbedded"), "Embed Linked");
    assert_eq!(label("layer.smartObjects.embedAllLinked"), "Embed All Linked");
    // The warp commands stay registered for scripts and the Edit menu routes.
    for id in ["layer.smartObjects.warp", "layer.smartObjects.perspectiveWarp", "layer.smartObjects.puppetWarp"] {
        label(id);
    }
}

#[test]
fn embed_all_linked_embeds_both_linked_objects_in_one_undoable_step() {
    let dir = temp_dir("both");
    let (mut s, ids) = three(&dir);
    assert!(is_linked(&s, ids[0]) && is_linked(&s, ids[1]) && !is_linked(&s, ids[2]));
    assert!(s.is_enabled("layer.smartObjects.embedAllLinked"));
    let before = s.active().unwrap().history.entries().len();
    let files: Vec<Vec<u8>> = (0..2).map(|i| photocraft_format::read_file(&dir.join(format!("linked{i}.pcraft"))).unwrap()).collect();
    assert_ne!(files[0], files[1], "the fixtures differ");
    let third = embedded(&s, ids[2]);

    let r = s.execute("layer.smartObjects.embedAllLinked", json!({})).unwrap();
    assert_eq!(r["embedded"], json!([ids[0], ids[1]]));
    assert!(ids.iter().all(|&id| !is_linked(&s, id)));
    for (i, file) in files.iter().enumerate() {
        let (name, bytes) = embedded(&s, ids[i]);
        assert_eq!(name, format!("linked{i}.pcraft"), "object {i} keeps its own file name");
        assert_eq!(&bytes, file, "object {i} carries its own linked file's bytes");
    }
    assert_eq!(embedded(&s, ids[2]), third, "the already embedded object is untouched");
    let entries = s.active().unwrap().history.entries();
    assert_eq!(entries.len(), before + 1, "one history step");
    assert_eq!(entries.last().map(String::as_str), Some("Embed All Linked"));
    assert!(!s.is_enabled("layer.smartObjects.embedAllLinked"), "nothing left to embed");

    s.execute("edit.undo", json!({})).unwrap();
    assert!(is_linked(&s, ids[0]) && is_linked(&s, ids[1]) && !is_linked(&s, ids[2]));
    assert_eq!(s.active().unwrap().history.entries().len(), before);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn embed_all_linked_with_no_linked_object_is_an_error_not_a_panic() {
    let mut s = Session::new();
    assert!(!s.is_enabled("layer.smartObjects.embedAllLinked"), "no document");
    assert!(s.execute("layer.smartObjects.embedAllLinked", json!({})).is_err());
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    painted_layer(&mut s, 1.0);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert!(!s.is_enabled("layer.smartObjects.embedAllLinked"), "only embedded objects");
    let before = s.active().unwrap().history.entries().len();
    assert!(s.execute("layer.smartObjects.embedAllLinked", json!({})).is_err());
    assert_eq!(s.active().unwrap().history.entries().len(), before);
}

#[test]
fn embed_all_linked_refuses_the_whole_step_when_a_linked_file_is_missing() {
    let dir = temp_dir("missing");
    let (mut s, ids) = three(&dir);
    std::fs::remove_file(dir.join("linked1.pcraft")).unwrap();
    let before = s.active().unwrap().history.entries().len();
    let err = s.execute("layer.smartObjects.embedAllLinked", json!({})).unwrap_err().to_string();
    assert!(err.contains("linked1.pcraft"), "the error names the missing file: {err}");
    assert!(is_linked(&s, ids[0]) && is_linked(&s, ids[1]), "nothing was embedded");
    assert_eq!(s.active().unwrap().history.entries().len(), before);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn embed_linked_still_embeds_one_object_under_its_old_id() {
    let dir = temp_dir("one");
    let (mut s, ids) = three(&dir);
    s.execute("layer.smartObjects.convertToEmbedded", json!({"layer": ids[0]})).unwrap();
    assert!(!is_linked(&s, ids[0]) && is_linked(&s, ids[1]));
    std::fs::remove_dir_all(&dir).ok();
}
