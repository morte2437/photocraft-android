//! Trim and Duplicate › Merged Layers Only flatten the document; on a canvas over the compositor's
//! render cap the flatten comes back empty, and both must report it instead of panicking (#7223).

use photocraft_engine::Session;
use serde_json::json;

/// 1.2e9 px: within File › New's per-side limit, over `MAX_RENDER_PIXELS` (2^30).
fn huge() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40000, "height": 30000, "background": "transparent"})).unwrap();
    s
}

#[test]
fn trim_on_a_document_too_large_to_flatten_is_an_error() {
    for based in ["topLeft", "bottomRight", "transparent"] {
        let mut s = huge();
        let err = s.execute("image.trim", json!({"basedOn": based})).unwrap_err().to_string();
        assert!(err.contains("too large"), "{based}: {err}");
        assert_eq!(s.active().unwrap().doc.size.width, 40000, "{based}: the document is untouched");
    }
}

#[test]
fn merged_duplicate_of_a_document_too_large_to_flatten_is_an_error() {
    let mut s = huge();
    let err = s.execute("image.duplicate", json!({"mergedOnly": true})).unwrap_err().to_string();
    assert!(err.contains("too large"), "{err}");
    assert_eq!(s.documents().len(), 1, "no half-built copy is added");
}
