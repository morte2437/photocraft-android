//! #12573: every Red Eye click that runs adds one History state named "Red Eye Removal Tool",
//! also when it changes no pixel; clicks the tool refuses add none and stay errors.

use serde_json::{Value, json};

use super::*;

const STEP: &str = "Red Eye Removal Tool";

fn session(mode: &str) -> Session {
    session_at(mode, 8)
}

fn session_at(mode: &str, depth: u64) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "mode": mode, "depth": depth})).unwrap();
    s
}

fn last(s: &Session) -> (usize, Option<String>) {
    let h = &s.active().unwrap().history;
    (h.past_len(), h.entries().last().map(|e| e.to_string()))
}

/// The active layer's pixels, if it has any (a Multichannel document may not).
fn pixels(s: &Session) -> Option<Vec<f32>> {
    let d = s.active().unwrap();
    let surf = d.doc.layer(d.active_layer?)?.surface()?;
    Some(surf.read_region(d.doc.bounds()))
}

fn red_dot(s: &mut Session) {
    s.execute("paint.pencil", json!({"points": [[32, 32]], "size": 14, "hardness": 100, "color": "#f21818"})).unwrap();
}

/// Runs `params`, which must change no pixel, and checks it leaves one unchanged state that
/// Undo takes back and Redo puts back.
fn unchanged_state(s: &mut Session, params: Value, case: &str) {
    let (n, _) = last(s);
    let before = pixels(s);
    let r = s.execute("paint.redEye", params).unwrap_or_else(|e| panic!("{case}: {e}"));
    assert_eq!(r["pixels"], 0, "{case}: {r}");
    assert_eq!(last(s), (n + 1, Some(STEP.to_string())), "{case}");
    assert!(pixels(s) == before, "{case}: no pixel changes");
    assert!(s.undo(), "{case}");
    assert_eq!(last(s).0, n, "{case}: undo removes the state");
    assert!(pixels(s) == before, "{case}");
    assert!(s.redo(), "{case}");
    assert_eq!(last(s), (n + 1, Some(STEP.to_string())), "{case}: redo");
    assert!(pixels(s) == before, "{case}: redo");
}

fn refused(s: &mut Session, params: Value, case: &str) {
    let (n, _) = last(s);
    let before = pixels(s);
    assert!(s.execute("paint.redEye", params).is_err(), "{case}");
    assert_eq!(last(s).0, n, "{case}: no state");
    assert!(pixels(s) == before, "{case}");
}

#[test]
fn a_correcting_click_is_one_red_eye_removal_tool_state_that_undoes() {
    let mut s = session("rgb");
    s.execute("layer.new.layer", json!({})).unwrap();
    red_dot(&mut s);
    let (n, _) = last(&s);
    let before = pixels(&s);
    let r = s.execute("paint.redEye", json!({"x": 32, "y": 32})).unwrap();
    assert!(r["pixels"].as_u64().unwrap() > 0, "{r}");
    assert_eq!(last(&s), (n + 1, Some(STEP.to_string())));
    let after = pixels(&s);
    assert!(after != before);
    // A second click on the fixed eye is a second state.
    s.execute("paint.redEye", json!({"x": 32, "y": 32})).unwrap();
    assert_eq!(last(&s), (n + 2, Some(STEP.to_string())));
    assert!(s.undo() && s.undo());
    assert!(pixels(&s) == before, "undo restores the pixels");
    assert!(s.redo());
    assert!(pixels(&s) == after, "redo applies the click again");
}

#[test]
fn a_click_that_changes_no_pixel_still_adds_one_state() {
    for depth in [8, 16, 32] {
        let mut s = session_at("rgb", depth);
        // On the Background, which locks transparency.
        s.execute("edit.fill", json!({"color": "#283c78"})).unwrap();
        unchanged_state(&mut s, json!({"x": 32, "y": 32}), &format!("dark blue pupil, {depth} bit"));
        s.execute("layer.new.layer", json!({})).unwrap();
        unchanged_state(&mut s, json!({"x": 32, "y": 32}), &format!("empty transparent layer, {depth} bit"));
        red_dot(&mut s);
        unchanged_state(&mut s, json!({"x": 6, "y": 6, "pupilSize": 1}), &format!("transparent area of a layer, {depth} bit"));
        unchanged_state(&mut s, json!({"x": 32, "y": 32, "darken": 0}), &format!("Darken Amount 0, {depth} bit"));
    }
    let mut s = session("rgb");
    s.execute("layer.new.layer", json!({})).unwrap();
    red_dot(&mut s);
    unchanged_state(&mut s, json!({"points": [[6, 6], [58, 6], [6, 58]], "pupilSize": 1}), "a points batch");
    let mut lab = session("lab");
    lab.execute("edit.fill", json!({"color": "#283c78"})).unwrap();
    unchanged_state(&mut lab, json!({"x": 32, "y": 32}), "Lab document");
}

#[test]
fn a_points_batch_is_one_state() {
    let mut s = session("rgb");
    s.execute("layer.new.layer", json!({})).unwrap();
    red_dot(&mut s);
    let (n, _) = last(&s);
    let before = pixels(&s);
    s.execute("paint.redEye", json!({"points": [[6, 6], [32, 32]], "pupilSize": 15})).unwrap();
    assert_eq!(last(&s), (n + 1, Some(STEP.to_string())));
    assert!(pixels(&s) != before);
}

#[test]
fn refused_clicks_add_no_state() {
    let mut s = session("rgb");
    s.execute("layer.new.layer", json!({})).unwrap();
    for (params, case) in [
        (json!({}), "no click"),
        (json!({"x": "nope", "y": 1}), "bad coordinates"),
        (json!({"points": []}), "empty batch"),
        (json!({"x": 8, "y": 8, "pupilSize": 10_000.0}), "window too large"),
        (json!({"x": 80, "y": 8}), "off the canvas"),
    ] {
        refused(&mut s, params, case);
    }
    s.execute("select.rect", json!({"x": 40, "y": 40, "width": 10, "height": 10})).unwrap();
    refused(&mut s, json!({"x": 8, "y": 8}), "a click outside the selection");
    unchanged_state(&mut s, json!({"x": 44, "y": 44}), "a click inside the selection");
    s.execute("select.deselect", json!({})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.setProps", json!({"layer": id.0, "visible": false})).unwrap();
    refused(&mut s, json!({"x": 8, "y": 8}), "a hidden layer");
    s.execute("layer.setProps", json!({"layer": id.0, "visible": true})).unwrap();
    s.execute("layer.lockLayers", json!({"transparency": true})).unwrap();
    refused(&mut s, json!({"x": 8, "y": 8}), "a clear pixel under Lock transparent pixels");
    s.execute("layer.lockLayers", json!({"transparency": false})).unwrap();
    s.execute("layer.setProps", json!({"locks": {"pixels": true}})).unwrap();
    refused(&mut s, json!({"x": 8, "y": 8}), "a locked layer");
    for mode in ["gray", "cmyk"] {
        let mut s = session(mode);
        s.execute("edit.fill", json!({"color": "#283c78"})).unwrap();
        refused(&mut s, json!({"x": 32, "y": 32}), mode);
    }
    for (convert, params) in
        [("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})), ("image.mode.multichannel", json!({}))]
    {
        let mut s = session("rgb");
        s.execute("edit.fill", json!({"color": "#283c78"})).unwrap();
        s.execute(convert, params).unwrap();
        refused(&mut s, json!({"x": 32, "y": 32}), convert);
    }
}

#[test]
fn channel_and_quick_mask_targets_keep_the_miss() {
    let mut s = session("rgb");
    s.execute("channel.new", json!({})).unwrap();
    refused(&mut s, json!({"x": 32, "y": 32, "target": {"channel": 0}}), "an alpha channel");
    let mut s = session("rgb");
    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    refused(&mut s, json!({"x": 32, "y": 32, "target": "quickMask"}), "the Quick Mask");
}
