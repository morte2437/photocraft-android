//! #11771: deleting the current count group makes the first remaining group current. Deleting
//! another group by index keeps the current one (#5654), and the last group goes without error.

use serde_json::{Value, json};

use crate::Session;

/// Three groups, each with one marker, and the second one current.
fn three_groups() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    for (i, name) in ["One", "Two", "Three"].iter().enumerate() {
        s.execute("count.newGroup", json!({"name": name})).unwrap();
        s.execute("count.add", json!({"x": 10 + 10 * i, "y": 10})).unwrap();
    }
    s.execute("count.setGroup", json!({"group": 1})).unwrap();
    assert_eq!(current(&mut s), (1, "Two".to_string()));
    s
}

/// The current group's index and name.
fn current(s: &mut Session) -> (u64, String) {
    let c = s.execute("image.analysis.countTool", json!({})).unwrap();
    let i = c["activeGroup"].as_u64().unwrap();
    let name = c["groups"].get(i as usize).and_then(|g| g["name"].as_str()).unwrap_or_default().to_string();
    (i, name)
}

fn groups(s: &mut Session) -> usize {
    s.execute("image.analysis.countTool", json!({})).unwrap()["groups"].as_array().map_or(0, Vec::len)
}

#[test]
fn deleting_the_current_group_makes_the_first_one_current() {
    for params in [json!({}), json!({"group": 1})] {
        let mut s = three_groups();
        s.execute("count.deleteGroup", params.clone()).unwrap();
        assert_eq!(groups(&mut s), 2, "{params}");
        assert_eq!(current(&mut s), (0, "One".to_string()), "{params}");
    }
}

#[test]
fn deleting_another_group_keeps_the_current_one() {
    let mut s = three_groups();
    s.execute("count.deleteGroup", json!({"group": 2})).unwrap();
    assert_eq!(current(&mut s), (1, "Two".to_string()));
    let mut s = three_groups();
    s.execute("count.deleteGroup", json!({"group": 0})).unwrap();
    assert_eq!(current(&mut s), (0, "Two".to_string()));
}

#[test]
fn deleting_the_last_group_leaves_none() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    s.execute("count.newGroup", json!({"name": "Only"})).unwrap();
    s.execute("count.deleteGroup", json!({})).unwrap();
    let c: Value = s.execute("image.analysis.countTool", json!({})).unwrap();
    assert_eq!(c["groups"].as_array().map(Vec::len), Some(0));
}
