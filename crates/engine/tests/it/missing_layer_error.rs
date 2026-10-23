//! A `"layer"` param that names no layer says which layers there are (#5058): ids are not
//! positions (a freshly opened PNG's Background is layer 2), and an agent should be able to
//! recover from the error alone. A name matches regardless of case as a fallback.

use photocraft_engine::{EngineError, Session};
use serde_json::json;

fn session() -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    let bg = s.active().unwrap().active_layer.unwrap().0;
    (s, bg)
}

fn unused_id(s: &Session) -> u64 {
    let doc = &s.active().unwrap().doc;
    (1..).find(|id| doc.layer(photocraft_doc::LayerId(*id)).is_none()).unwrap()
}

#[test]
fn an_unknown_layer_id_lists_the_documents_layers() {
    let (mut s, bg) = session();
    let layer = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    let missing = unused_id(&s);
    let err = s.execute("layer.setProps", json!({"layer": missing, "visible": false})).unwrap_err();
    assert!(matches!(err, EngineError::UnknownLayer { id, .. } if id.0 == missing), "{err}");
    let err = err.to_string();
    assert!(err.contains(&format!("no such layer {missing};")), "{err}");
    assert!(err.contains(&format!("{layer} \"Layer 1\", {bg} \"Background\"")), "top-down ids and names: {err}");
}

#[test]
fn an_unknown_layer_name_lists_the_documents_layers() {
    let (mut s, bg) = session();
    let err = s.execute("layer.setProps", json!({"layer": "Nope", "visible": false})).unwrap_err().to_string();
    assert!(err.contains("no layer named \"Nope\""), "{err}");
    assert!(err.contains(&format!("{bg} \"Background\"")), "{err}");
}

#[test]
fn a_layer_name_matches_regardless_of_case() {
    let (mut s, bg) = session();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.setProps", json!({"layer": "background", "name": "Base"})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer(photocraft_doc::LayerId(bg)).unwrap().name, "Base");
}

#[test]
fn an_exact_name_wins_over_a_case_insensitive_one() {
    let (mut s, _) = session();
    let lower = s.execute("layer.new.layer", json!({"name": "shadow"})).unwrap()["layer"].as_u64().unwrap();
    let upper = s.execute("layer.new.layer", json!({"name": "Shadow"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": "shadow", "visible": false})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert!(!doc.layer(photocraft_doc::LayerId(lower)).unwrap().visible);
    assert!(doc.layer(photocraft_doc::LayerId(upper)).unwrap().visible);
}

#[test]
fn a_long_layer_name_is_shortened_in_the_listing() {
    let (mut s, _) = session();
    let long = "é".repeat(500);
    let layer = s.execute("layer.new.layer", json!({"name": long})).unwrap()["layer"].as_u64().unwrap();
    let err = s.execute("layer.setProps", json!({"layer": "Nope", "visible": false})).unwrap_err().to_string();
    let shown = format!("{layer} \"{}…\"", "é".repeat(64));
    assert!(err.contains(&shown), "the name is cut at 64 characters: {err}");
    assert!(err.chars().count() < 300, "{err}");
}
