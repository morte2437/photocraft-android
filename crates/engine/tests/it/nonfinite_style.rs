//! Character style numbers that overflow `f32` are refused, so the document stays saveable.

use photocraft_engine::{EngineError, Session};
use serde_json::json;

fn fixture() -> (Session, u64) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "white"})).unwrap();
    let id = s.execute("type.create", json!({"text": "Hi", "size": 24})).unwrap()["layer"].as_u64().unwrap();
    (s, id)
}

#[test]
fn out_of_range_style_numbers_are_refused() {
    for key in ["leading", "baselineShift", "horizontalScale", "verticalScale"] {
        let (mut s, id) = fixture();
        let past = s.active().unwrap().history.past_len();
        for value in [1e39, -1e39, 1e12] {
            let r = s.execute("type.setStyle", json!({"layer": id, key: value}));
            assert!(matches!(r, Err(EngineError::BadParams { .. })), "{key}={value}: {r:?}");
        }
        assert_eq!(s.active().unwrap().history.past_len(), past, "{key}: no history step");
        let r = s.execute("type.create", json!({"text": "x", key: 1e39}));
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "create {key}: {r:?}");
        let dir = std::env::temp_dir().join(format!("pc_nonfinite_style_{key}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.pcraft");
        let saved = s.execute("file.saveACopy", json!({"path": path.to_string_lossy()}));
        std::fs::remove_dir_all(&dir).ok();
        saved.unwrap_or_else(|e| panic!("{key}: save failed: {e:?}"));
    }
}

#[test]
fn in_range_style_numbers_still_apply_and_save() {
    let (mut s, id) = fixture();
    s.execute("type.setStyle", json!({"layer": id, "leading": 30, "baselineShift": -4, "horizontalScale": 120, "verticalScale": 80})).unwrap();
    s.execute("type.setStyle", json!({"layer": id, "leading": "auto"})).unwrap();
    let dir = std::env::temp_dir().join(format!("pc_nonfinite_style_ok_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.pcraft");
    let saved = s.execute("file.saveACopy", json!({"path": path.to_string_lossy()}));
    std::fs::remove_dir_all(&dir).ok();
    saved.unwrap_or_else(|e| panic!("save failed: {e:?}"));
}

#[test]
fn non_numeric_leading_is_refused() {
    let (mut s, id) = fixture();
    let past = s.active().unwrap().history.past_len();
    for value in [json!("18pt"), json!("Auto "), json!(true), json!([12])] {
        let r = s.execute("type.setStyle", json!({"layer": id, "leading": value}));
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "leading={value}: {r:?}");
    }
    assert_eq!(s.active().unwrap().history.past_len(), past, "no history step");
}
