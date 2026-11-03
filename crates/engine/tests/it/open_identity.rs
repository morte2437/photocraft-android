//! Background imports report the admitted document's identity, not just its temporary tab index.

use std::sync::Arc;

use photocraft_engine::Session;
use photocraft_engine::jobs::{OpenSource, Started};
use serde_json::{Value, json};

fn source() -> Session {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    session.execute("layer.new.layer", json!({})).unwrap();
    session.execute("edit.fill", json!({"color": "#336699"})).unwrap();
    session
}

fn open(session: &mut Session, name: &str, bytes: Vec<u8>) -> Value {
    match session.start_open(name, OpenSource::Bytes(Arc::new(bytes))).unwrap() {
        Started::Done(result) => result,
        Started::Job(job) => session.wait_job(job).unwrap(),
    }
}

#[test]
fn open_identity_reports_the_live_document_after_tabs_move() {
    let input = source();
    let bytes = photocraft_io::export(&input.active().unwrap().doc, "synthetic.png", &Default::default()).unwrap().bytes;
    let mut session = source();
    let result = open(&mut session, "synthetic.png", bytes);
    let index = result["document"].as_u64().unwrap() as usize;
    let imported = session.documents()[index].doc.id;
    assert_eq!(result["docId"], imported.0, "the engine result must carry the actual admitted identity: {result}");
    session.move_document(index, 0).unwrap();
    assert_eq!(session.documents()[0].doc.id.0, result["docId"]);
    assert_ne!(session.documents()[index].doc.id, imported, "the old index now identifies another document");
}

#[test]
fn open_identity_reports_the_remapped_id_when_a_native_document_is_opened_twice() {
    let mut session = source();
    let original = session.active().unwrap().doc.clone();
    // A wholly synthetic in-memory native file retains its DocId. PNG and DNG allocate fresh
    // IDs during decode, so they cannot deterministically exercise this collision.
    let bytes = photocraft_format::PcraftWriter::new().save_zip(&original, &Default::default()).unwrap().0;
    let result = open(&mut session, "synthetic.pcraft", bytes);
    let index = result["document"].as_u64().unwrap() as usize;
    let admitted = session.documents()[index].doc.id;
    assert_ne!(admitted, original.id, "the original document still owns its persisted ID");
    assert_eq!(result["docId"], admitted.0, "report the remapped identity, not the ID decoded from the file");
    assert!(Arc::ptr_eq(&session.documents()[0].doc, &original));
}
