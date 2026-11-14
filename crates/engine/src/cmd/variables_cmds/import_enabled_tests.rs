//! #13920: File > Import > Variable Data Sets is greyed until the active document has a variable.

use super::*;

const IMPORT: &str = "file.import.variableDataSets";

fn new_doc(s: &mut Session) {
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
}

fn define_variable(s: &mut Session) {
    let layer = s.active().unwrap().doc.layers[0].id;
    s.execute("image.variables.define", json!({"defs": [{"name": "show", "layer": layer.0, "type": "visibility"}]})).unwrap();
}

fn refusal(s: &mut Session) -> String {
    let path = std::env::temp_dir().join(format!("pc-vars-{}-import-enabled.csv", std::process::id()));
    match s.execute(IMPORT, json!({"path": path.to_string_lossy()})) {
        Err(EngineError::Disabled(id, why)) => {
            assert_eq!(id, IMPORT);
            why
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// The other File > Import entries, with whether each is enabled now.
fn other_imports(s: &Session) -> Vec<(&'static str, bool)> {
    crate::cmd::command_specs().iter().filter(|c| c.menu == ["File", "Import"] && c.id != IMPORT).map(|c| (c.id, s.is_enabled(c.id))).collect()
}

#[test]
fn import_is_disabled_without_a_document() {
    let mut s = Session::new();
    assert!(!s.is_enabled(IMPORT));
    assert_eq!(refusal(&mut s), "no document");
}

#[test]
fn import_is_disabled_without_variables_and_leaves_the_document_alone() {
    let mut s = Session::new();
    new_doc(&mut s);
    assert!(!s.is_enabled(IMPORT));
    assert_eq!(refusal(&mut s), "no variables in document");
    assert!(s.active().unwrap().doc.variables.data_sets.is_empty());
}

#[test]
fn import_is_enabled_with_a_variable_and_no_data_sets() {
    let mut s = Session::new();
    new_doc(&mut s);
    define_variable(&mut s);
    assert!(s.active().unwrap().doc.variables.data_sets.is_empty());
    assert!(s.is_enabled(IMPORT));

    let path = std::env::temp_dir().join(format!("pc-vars-{}-import-enabled-ok.csv", std::process::id()));
    std::fs::write(&path, "show\ntrue\nfalse\n").unwrap();
    let r = s.execute(IMPORT, json!({"path": path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(r["imported"], 2);
}

#[test]
fn import_follows_the_active_document() {
    let mut s = Session::new();
    new_doc(&mut s);
    define_variable(&mut s);
    new_doc(&mut s);
    // The new, active document has no variables; the one behind it does.
    assert!(!s.is_enabled(IMPORT));
    assert_eq!(refusal(&mut s), "no variables in document");

    assert!(s.set_active(0));
    assert!(s.is_enabled(IMPORT));
    assert!(s.set_active(1));
    assert!(!s.is_enabled(IMPORT));
}

#[test]
fn other_import_entries_do_not_depend_on_variables() {
    let mut s = Session::new();
    new_doc(&mut s);
    let without = other_imports(&s);
    assert!(!without.is_empty(), "File > Import has other entries");
    define_variable(&mut s);
    assert_eq!(other_imports(&s), without);
}
