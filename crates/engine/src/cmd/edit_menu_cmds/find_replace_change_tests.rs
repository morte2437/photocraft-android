use super::*;
use photocraft_doc::LayerId;

fn text_of(s: &Session, id: LayerId) -> String {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.text.clone(),
        _ => String::new(),
    }
}

fn new_doc_with_type(text: &str) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "depth": 8})).unwrap();
    let id = LayerId(s.execute("type.create", json!({"text": text, "x": 2, "y": 12})).unwrap()["layer"].as_u64().unwrap());
    (s, id)
}

#[test]
fn change_without_match_changes_nothing_and_does_not_advance() {
    let (mut s, id) = new_doc_with_type("cat Cat");
    let steps = s.active().unwrap().history.past_len();
    let r = s.execute("edit.findAndReplaceText", json!({"find": "cat", "replace": "dog", "action": "change", "allLayers": false})).unwrap();
    assert!(r["changed"].is_null(), "{r}");
    assert!(r["found"].is_null(), "{r}");
    assert_eq!(text_of(&s, id), "cat Cat");
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    assert!(s.edit_state.find_cursor.is_none());
}

#[test]
fn change_all_with_zero_hits_drops_remembered_match() {
    let (mut s, id) = new_doc_with_type("cats");
    let f = s.execute("edit.findAndReplaceText", json!({"find": "cats", "action": "find"})).unwrap();
    assert_eq!(f["found"]["text"], "cats");

    let all = s.execute("edit.findAndReplaceText", json!({"find": "nonexistent", "replace": "dogs", "action": "changeAll"})).unwrap();
    assert_eq!(all["count"].as_u64(), Some(0));

    let c = s.execute("edit.findAndReplaceText", json!({"find": "cats", "replace": "dogs", "action": "change"})).unwrap();
    assert!(c["changed"].is_null(), "{c}");
    assert!(c["found"].is_null(), "{c}");
    assert_eq!(text_of(&s, id), "cats");
}

#[test]
fn new_find_or_option_change_drops_remembered_match() {
    let cases = [
        json!({"find": "dog", "replace": "bird", "action": "change"}),
        json!({"find": "cat", "replace": "bird", "action": "change", "caseSensitive": true}),
        json!({"find": "cat", "replace": "bird", "action": "change", "wholeWord": true}),
        json!({"find": "cat", "replace": "bird", "action": "change", "allLayers": false}),
        json!({"find": "cat", "replace": "bird", "action": "change", "forward": false}),
    ];
    for params in cases {
        let (mut s, id) = new_doc_with_type("cat");
        let f = s.execute("edit.findAndReplaceText", json!({"find": "cat", "action": "find"})).unwrap();
        assert_eq!(f["found"]["text"], "cat");

        let c = s.execute("edit.findAndReplaceText", params.clone()).unwrap();
        assert!(c["changed"].is_null(), "{params}: {c}");
        assert!(c["found"].is_null(), "{params}: {c}");
        assert_eq!(text_of(&s, id), "cat", "{params}");
    }
}

#[test]
fn reference_sequence_find_change_change_change_find_x4() {
    let (mut s, id) = new_doc_with_type("cat Cat CAT cats");
    let p = |action: &str| {
        json!({
            "find": "cat",
            "replace": "dog",
            "action": action,
            "allLayers": false,
            "caseSensitive": false,
            "wholeWord": false,
        })
    };

    let mut replacements = 0;

    let f = s.execute("edit.findAndReplaceText", p("find")).unwrap();
    assert_eq!((f["found"]["start"].as_u64(), f["found"]["end"].as_u64()), (Some(0), Some(3)), "{f}");

    let c1 = s.execute("edit.findAndReplaceText", p("change")).unwrap();
    assert!(c1["changed"].is_object(), "{c1}");
    replacements += 1;

    // Second Change in a row replaces nothing and does not move on.
    let c2 = s.execute("edit.findAndReplaceText", p("change")).unwrap();
    assert!(c2["changed"].is_null(), "{c2}");
    assert!(c2["found"].is_null(), "{c2}");
    assert_eq!(text_of(&s, id), "dog Cat CAT cats");

    let mut found_texts = Vec::new();
    for _ in 0..4 {
        let cf = s.execute("edit.findAndReplaceText", p("changeFind")).unwrap();
        if cf["changed"].is_object() {
            replacements += 1;
        }
        found_texts.push(cf["found"]["text"].as_str().map(str::to_string));
    }

    assert_eq!(found_texts, vec![Some("Cat".to_string()), Some("CAT".to_string()), Some("cat".to_string()), None]);
    assert_eq!(replacements, 4);
    assert_eq!(text_of(&s, id), "dog dog dog dogs");
}
