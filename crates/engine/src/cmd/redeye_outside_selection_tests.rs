//! #12576: a Red Eye press outside the selection does nothing and adds no History state; a press
//! inside it is clipped to the selection as before, and with no selection nothing changes.

use serde_json::{Value, json};

use super::*;

/// 800 × 600 RGB/8 with eye A at (200, 200) and eye B at (500, 200): a white disc of radius 40
/// with a red pupil (210, 40, 40) of radius 30, on a skin tone.
fn eyes() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "mode": "rgb", "depth": 8})).unwrap();
    s.execute("edit.fill", json!({"color": "#c08a68"})).unwrap();
    for cx in [200, 500] {
        for (r, color) in [(40, "#ffffff"), (30, "#d22828")] {
            s.execute("select.rect", json!({"x": cx - r, "y": 200 - r, "width": 2 * r, "height": 2 * r, "ellipse": true})).unwrap();
            s.execute("edit.fill", json!({"color": color})).unwrap();
        }
    }
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// The left half of eye B: x 450–499.
fn select_left_half_of_eye_b(s: &mut Session) {
    s.execute("select.rect", json!({"x": 450, "y": 0, "width": 50, "height": 600})).unwrap();
}

/// History length, and the Background's pixels as one value per channel.
fn snapshot(s: &Session) -> (usize, Vec<f32>) {
    let d = s.active().unwrap();
    let surf = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap();
    (d.history.past_len(), surf.read_region(d.doc.bounds()))
}

/// Runs a press and returns the states it added, the first and last columns whose pixels
/// changed (`None` if none did) and the command's result.
fn press(s: &mut Session, params: Value) -> (usize, Option<(usize, usize)>, std::result::Result<Value, String>) {
    let (n, before) = snapshot(s);
    let r = s.execute("paint.redEye", params).map_err(|e| e.to_string());
    let (m, after) = snapshot(s);
    let channels = before.len() / (800 * 600);
    let cols: Vec<usize> = before.chunks(channels).zip(after.chunks(channels)).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i % 800).collect();
    let span = cols.iter().min().zip(cols.iter().max()).map(|(a, b)| (*a, *b));
    (m - n, span, r)
}

#[test]
fn a_press_outside_the_selection_changes_nothing_and_adds_no_state() {
    let mut s = eyes();
    select_left_half_of_eye_b(&mut s);
    for params in [json!({"x": 500, "y": 200}), json!({"points": [[500, 200], [490, 200]]})] {
        let (states, cols, r) = press(&mut s, params.clone());
        assert_eq!(cols, None, "{params}: pixels changed");
        assert_eq!(states, 0, "{params}: a history state was added");
        assert_eq!(r, Err(MISS.to_string()), "{params}");
    }
}

#[test]
fn a_press_inside_the_selection_is_clipped_and_adds_one_state() {
    // A batch whose first point is inside runs, and x 499.5 is still on the last selected column.
    for params in [json!({"x": 490, "y": 200}), json!({"points": [[490, 200], [500, 200]]}), json!({"x": 499.5, "y": 200})] {
        let mut s = eyes();
        select_left_half_of_eye_b(&mut s);
        let (states, cols, r) = press(&mut s, params.clone());
        assert!(r.unwrap()["pixels"].as_u64().unwrap() > 0, "{params}");
        assert_eq!(states, 1, "{params}");
        let (first, last) = cols.expect("the pupil changed");
        assert!(first >= 450 && last <= 499, "{params}: clipped to the selection: columns {first}..={last}");
    }
}

#[test]
fn with_no_selection_a_press_on_eye_b_corrects_both_halves() {
    let mut s = eyes();
    let (states, cols, r) = press(&mut s, json!({"x": 500, "y": 200}));
    assert!(r.unwrap()["pixels"].as_u64().unwrap() > 0);
    assert_eq!(states, 1);
    let (first, last) = cols.expect("the pupil changed");
    assert!(first < 500 && last > 500, "columns {first}..={last}");
}

#[test]
fn a_selection_covering_eye_a_gives_the_same_result_as_none() {
    let mut plain = eyes();
    plain.execute("paint.redEye", json!({"x": 200, "y": 200})).unwrap();
    let mut selected = eyes();
    selected.execute("select.rect", json!({"x": 140, "y": 140, "width": 120, "height": 120})).unwrap();
    selected.execute("paint.redEye", json!({"x": 200, "y": 200})).unwrap();
    assert!(snapshot(&plain).1 != snapshot(&eyes()).1, "the press corrects eye A");
    assert!(snapshot(&selected).1 == snapshot(&plain).1, "the same pixels");
}
