//! #11933: a Color Replacement click, a drag and a Shift+click segment each add one History state
//! named "Color Replacement Tool", on a layer and on the Background with Lock transparent pixels;
//! the command's label, which recorded actions show, is the same name.

use serde_json::json;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    s.execute("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
    s
}

fn last(s: &Session) -> (usize, Option<String>) {
    let h = &s.active().unwrap().history;
    (h.past_len(), h.entries().last().map(|e| e.to_string()))
}

#[test]
fn every_click_stroke_and_segment_is_one_color_replacement_tool_state() {
    for on_background in [false, true] {
        let mut s = session();
        if !on_background {
            s.execute("layer.new.layer", json!({})).unwrap();
            s.execute("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
        } else {
            s.execute("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
            s.execute("layer.lockLayers", json!({"transparency": true})).unwrap();
        }
        for points in [json!([[20, 20]]), json!([[20, 50], [80, 50]]), json!([[80, 50], [80, 80]])] {
            let (n, _) = last(&s);
            s.execute("paint.colorReplacement", json!({"points": points, "size": 15})).unwrap();
            assert_eq!(last(&s), (n + 1, Some(COLOR_REPLACEMENT_STEP.to_string())), "{points} (background {on_background})");
        }
    }
    assert_eq!(COLOR_REPLACEMENT_STEP, "Color Replacement Tool");
}

#[test]
fn recorded_steps_use_the_same_name() {
    let spec = crate::commands::find("paint.colorReplacement").expect("the command");
    assert_eq!(spec.label, COLOR_REPLACEMENT_STEP);
}
