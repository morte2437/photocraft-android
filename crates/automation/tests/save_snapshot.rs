//! History Options › Automatically Create New Snapshot When Saving (#2966) through the headless
//! server: a save of the document to its own layered file adds a snapshot when the option is on;
//! a flat export is a copy and adds none, and with the option off no save does.

use photocraft_automation::Headless;
use serde_json::json;

fn tmp() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pc-save-snapshot-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn snapshots(h: &Headless) -> usize {
    h.session.active().unwrap().snapshots.len()
}

#[test]
fn a_document_save_adds_a_snapshot_only_with_the_option() {
    let dir = tmp();
    let psd = dir.join("a.psd");
    let png = dir.join("a.png");
    let mut h = Headless::trusted_local();
    h.handle("doc.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    assert_eq!(snapshots(&h), 1, "the first snapshot");

    // Off (the default): saving adds nothing.
    h.handle("doc.save", json!({"path": psd.to_str().unwrap()})).unwrap();
    assert_eq!(snapshots(&h), 1);

    h.handle("engine.execute", json!({"command": "history.options", "params": {"automaticallyCreateNewSnapshotWhenSaving": true}})).unwrap();
    h.handle("engine.execute", json!({"command": "image.adjustments.invert", "params": {}})).unwrap();
    h.handle("doc.save", json!({})).unwrap();
    assert_eq!(snapshots(&h), 2, "the save added one");
    let st = h.session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&st.snapshots[1].doc, &st.doc), "of the document as saved");
    assert!(!st.is_dirty(), "and the document stays saved");

    // A flat export is a copy, not a save of the document.
    h.handle("doc.save", json!({"path": png.to_str().unwrap()})).unwrap();
    assert_eq!(snapshots(&h), 2);
    std::fs::remove_dir_all(dir).unwrap();
}
