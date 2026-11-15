use std::sync::Arc;

use photocraft_doc::{Layer, LayerContent, TextLayer};
use serde_json::json;

use super::import_data_sets_text;
use crate::Session;

const IMPORT: &str = "file.import.variableDataSets";

fn template() -> Session {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
    let (title, badge) = session
        .edit("Create template layers", |doc, active| {
            let badge = Layer::raster("Badge", doc.pixel_format());
            let title = Layer::new("Title", LayerContent::Text(TextLayer { text: "Template".into(), ..Default::default() }));
            let ids = (title.id, badge.id);
            *active = Some(title.id);
            doc.layers.extend([badge, title]);
            Ok(ids)
        })
        .unwrap();
    let history_len = session.active().unwrap().history.past_len();
    session
        .execute(
            "image.variables.define",
            json!({"defs": [
                {"name": "Headline", "layer": title.0, "type": "textReplacement"},
                {"name": "Badge", "layer": badge.0, "type": "visibility"}
            ]}),
        )
        .unwrap();
    session
        .execute(
            "image.variables.dataSets",
            json!({"dataSets": [
                {"name": "Previous", "values": [
                    {"variable": "Headline", "kind": "text", "value": "Previously applied"},
                    {"variable": "Badge", "kind": "visibility", "value": false}
                ]}
            ]}),
        )
        .unwrap();
    assert_eq!(session.active().unwrap().history.past_len(), history_len);
    session.execute("image.applyDataSet", json!({"name": "Previous"})).unwrap();
    assert_eq!(session.active().unwrap().doc.variables.active, Some(0));
    session
}

#[test]
fn import_records_one_step_and_undo_redo_restore_only_data_sets() {
    let mut session = template();
    let before = session.active().unwrap().doc.clone();
    let target = session.active().unwrap().layer_target();
    let history_len = session.active().unwrap().history.past_len();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sets.csv");
    std::fs::write(&path, "DataSet,Headline,Badge\nNew,Imported,True\nOther,Alternate,False\n").unwrap();
    let imported = session.execute(IMPORT, json!({"path": path})).unwrap();
    assert_eq!(imported, json!({"imported": 2}));

    let state = session.active().unwrap();
    assert_eq!(state.history.past_len(), history_len + 1);
    assert_eq!(state.history.entries().last().map(String::as_str), Some("Import Data Sets"));
    assert_eq!(state.layer_target(), target);
    let after = state.doc.clone();
    let listed = session.execute("variables.list", json!({})).unwrap();
    assert_eq!(listed["dataSets"][0]["name"], "New");
    assert_eq!(listed["dataSets"][0]["values"][0]["value"], "Imported");
    assert_eq!(listed["dataSets"][0]["values"][1]["value"], true);
    assert_eq!(listed["dataSets"][1]["name"], "Other");
    let mut expected = (*before).clone();
    expected.variables.data_sets = after.variables.data_sets.clone();
    assert_eq!(*after, expected, "import leaves the canvas, layers and variable definitions as they were");

    assert!(session.undo());
    let state = session.active().unwrap();
    assert_eq!(state.doc, before);
    assert_eq!(state.layer_target(), target);
    assert_eq!(state.history.past_len(), history_len);
    assert!(session.redo());
    assert_eq!(session.active().unwrap().doc, after);
}

#[test]
fn failed_import_preserves_existing_sets_and_redo() {
    let mut session = template();
    // The picker and automation also use the text entry point.
    import_data_sets_text(&mut session, "Headline\nNew text\n", &json!({})).unwrap();
    assert!(session.undo());
    let state = session.active().unwrap();
    let before = state.doc.clone();
    let revision = state.revision;
    let history = state.history.entries();
    let redo: Vec<_> = state.history.redo_labels().map(str::to_owned).collect();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sets.csv");
    let params = json!({"path": path});
    assert!(session.execute(IMPORT, params.clone()).is_err(), "missing file");
    std::fs::write(&path, [0xff]).unwrap();
    assert!(session.execute(IMPORT, params).is_err(), "invalid UTF-8");
    assert!(import_data_sets_text(&mut session, "\u{feff} \r\n\n", &json!({})).is_err(), "empty CSV");
    let state = session.active().unwrap();
    assert!(Arc::ptr_eq(&state.doc, &before));
    assert_eq!(state.revision, revision);
    assert_eq!(state.history.entries(), history);
    assert_eq!(state.history.redo_labels().collect::<Vec<_>>(), redo);
    assert!(session.redo());
    assert_eq!(session.active().unwrap().doc.variables.data_sets[0].name, "Data Set 1");
}
