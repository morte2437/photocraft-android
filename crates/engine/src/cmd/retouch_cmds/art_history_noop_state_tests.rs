//! An Art History Brush click the tolerance holds back still adds one unchanged "Art History
//! Brush" state, as in Photoshop; refusals and strokes that could write nothing add none (#12636).

use super::*;

fn session(depth: u64) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 48, "height": 32, "depth": depth, "mode": "rgb"})).unwrap();
    s
}

fn fill_gray(s: &mut Session, v: f32) {
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let px = photocraft_raster::from_rgba(&fmt, [v, v, v, 1.0]);
        let data: Vec<f32> = (0..b.width() * b.height()).flat_map(|_| px.iter().copied()).collect();
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
}

/// History row 1 is grey 0.5, the layer now grey 0.6: a small change every high tolerance holds back.
pub(super) fn held_back_session(depth: u64) -> Session {
    let mut s = session(depth);
    fill_gray(&mut s, 0.5);
    fill_gray(&mut s, 0.6);
    s
}

pub(super) fn pixels(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().read_region(d.doc.bounds())
}

pub(super) fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

pub(super) fn stroke(s: &mut Session, extra: Value) -> Result<Value> {
    let mut p = json!({"points": [[8, 16], [40, 16]], "size": 12, "hardness": 100, "area": 50, "style": "tightMedium", "state": 1, "tolerance": 90});
    if let Some(map) = extra.as_object() {
        for (k, v) in map {
            p[k] = v.clone();
        }
    }
    s.execute(CMD, p)
}

#[test]
fn a_click_held_back_by_the_tolerance_adds_one_unchanged_state() {
    for tolerance in [90, 95, 99, 100] {
        for points in [json!([[24, 16]]), json!([[8, 16], [40, 16]])] {
            let mut s = held_back_session(8);
            let (before, n) = (pixels(&s), steps(&s));
            let r = stroke(&mut s, json!({"tolerance": tolerance, "points": points})).unwrap();
            let case = format!("tolerance {tolerance} points {points}");
            assert!(r["damage"].is_null(), "{case}: {r}");
            assert_eq!(steps(&s), n + 1, "{case}: one state");
            assert_eq!(s.active().unwrap().history.undo_label(), Some("Art History Brush"), "{case}");
            assert_eq!(pixels(&s), before, "{case}: no pixel changed");
            assert!(s.undo(), "{case}");
            assert_eq!(steps(&s), n, "{case}: undo removes the state");
            assert_eq!(pixels(&s), before, "{case}: undo leaves the layer as it was");
        }
    }
}

#[test]
fn refusals_under_the_tolerance_add_no_state() {
    let refused = |name: &str, s: &mut Session, extra: Value| {
        let n = steps(s);
        assert!(stroke(s, extra).is_err(), "{name} is refused");
        assert_eq!(steps(s), n, "{name} adds no state");
    };

    let mut s = held_back_session(8);
    s.execute("layer.setProps", json!({"visible": false})).unwrap();
    refused("a hidden layer", &mut s, json!({}));

    let mut s = held_back_session(8);
    s.execute("layer.setProps", json!({"locks": {"pixels": true}})).unwrap();
    refused("a locked layer", &mut s, json!({}));

    let mut s = held_back_session(8);
    s.execute("layer.new.group", json!({})).unwrap();
    refused("a group", &mut s, json!({}));

    let mut s = held_back_session(8);
    refused("a missing state", &mut s, json!({"state": 99}));

    let mut s = held_back_session(8);
    refused("a missing snapshot", &mut s, json!({"state": null, "snapshot": 99}));

    let mut s = held_back_session(8);
    let snapshot = s.execute("history.newSnapshot", json!({})).unwrap()["snapshot"].clone();
    s.execute("history.setSource", json!({"snapshot": snapshot})).unwrap();
    s.execute("history.deleteSnapshot", json!({"snapshot": snapshot})).unwrap();
    refused("a deleted History panel source", &mut s, json!({"state": null}));

    let mut s = held_back_session(8);
    s.execute("image.canvasSize", json!({"width": 60, "height": 40, "anchor": "center"})).unwrap();
    refused("a source of another canvas size", &mut s, json!({}));

    let mut s = held_back_session(8);
    refused("another blend mode", &mut s, json!({"mode": "multiply"}));
}

/// Not measured in Photoshop, so these keep the behaviour they had: no state, no error.
#[test]
fn strokes_that_can_write_nothing_stay_quiet() {
    let mut s = held_back_session(8);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    let (before, n) = (pixels(&s), steps(&s));
    let r = stroke(&mut s, json!({})).unwrap();
    assert!(r["damage"].is_null(), "{r}");
    assert_eq!(steps(&s), n, "wholly outside the selection");
    assert_eq!(pixels(&s), before);

    let mut s = session(8);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
    let n = steps(&s);
    // Row 1 is the new transparent layer: at tolerance 0 the stroke would write here, but Lock Transparency skips every pixel.
    let r = stroke(&mut s, json!({})).unwrap();
    assert!(r["damage"].is_null(), "{r}");
    assert_eq!(steps(&s), n, "locked transparent pixels");
}

/// A selection across part of the stroke: at tolerance 0 the stroke would paint inside it, so the
/// click the tolerance holds back adds its unchanged state.
#[test]
fn a_held_back_click_across_part_of_the_selection_adds_one_unchanged_state() {
    let mut s = held_back_session(8);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 24, "height": 32})).unwrap();
    let (before, n) = (pixels(&s), steps(&s));
    let r = stroke(&mut s, json!({})).unwrap();
    assert!(r["damage"].is_null(), "{r}");
    assert_eq!(steps(&s), n + 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Art History Brush"));
    assert_eq!(pixels(&s), before);
}
