//! `Session::precheck` accepts what `execute` accepts for the commands it covers (all but the
//! group nesting limit, which only the edit checks)
//! (`PRECHECKED`): Free Transform applies its box before them only when they will run (#5859).

use photocraft_engine::{PRECHECKED, Session};
use serde_json::{Value, json};

/// Documents in different states: a Background only, a pixel layer active, a group active, a
/// selection, no document.
const STATES: [&str; 5] = ["no document", "background only", "pixel layer", "group, no background", "selection"];

/// A fresh session in `state`.
fn state(state: &str) -> Session {
    let mut s = Session::new();
    if state == "no document" {
        return s;
    }
    let background = if state == "group, no background" { "transparent" } else { "white" };
    s.execute("file.new", json!({"width": 40, "height": 30, "background": background})).unwrap();
    match state {
        "pixel layer" => {
            s.execute("layer.new.layer", json!({})).unwrap();
            s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        }
        "group, no background" => {
            s.execute("layer.new.group", json!({})).unwrap();
        }
        "selection" => {
            s.execute("layer.new.layer", json!({})).unwrap();
            s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
            s.execute("select.rect", json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
        }
        _ => {}
    }
    s
}

fn params() -> Vec<Value> {
    vec![
        Value::Null,
        json!({}),
        json!([1]),
        json!("x"),
        json!({"mode": "Bogus"}),
        json!({"mode": "Multiply"}),
        json!({"fillNeutral": true}),
        json!({"mode": "Overlay", "fillNeutral": true}),
        json!({"position": "x"}),
        json!({"position": "below"}),
        json!({"opacity": "half"}),
        json!({"name": 5}),
        json!({"name": "ok"}),
        json!({"color": "chartreuse"}),
        json!({"layer": 999_999}),
        json!({"layer": 1}),
        json!({"layer": 2}),
        json!({"layer": null}),
        json!({"layer": "Layer 1"}),
        json!({"layer": "no such layer"}),
        json!({"layer": true}),
        json!({"ifPresent": true}),
    ]
}

#[test]
fn precheck_agrees_with_execute() {
    let mut disagreements = Vec::new();
    for id in PRECHECKED {
        for p in params() {
            for name in STATES {
                // A fresh session for each call: execute changes it.
                let mut s = state(name);
                let pre = s.precheck(id, &p).is_ok();
                let ran = s.execute(id, p.clone()).is_ok();
                if pre != ran {
                    disagreements.push(format!("{id} {p} on {name}: precheck {pre}, execute {ran}"));
                }
            }
        }
    }
    assert!(disagreements.is_empty(), "{disagreements:#?}");
}
