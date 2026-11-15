//! The History Brush refuses a source state from before Image › Mode changed RGB to Grayscale, or
//! changed only the bit depth, with Photoshop's alert texts, adding no state and changing no pixel.
//! The pairs not measured convert the source as before (#12842).

use serde_json::{Value, json};

use crate::Session;
use crate::retouch_cmds::LiveRetouch;

const MODE: &str = "Could not use the history brush because the current color mode does not match that of the history state.";
const DEPTH: &str = "Could not use the history brush because the current bit depth does not match that of the history state.";

/// Rows Open (white), Fill (red): row 1 is the red state.
fn session(mode: &str, depth: u64) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 24, "height": 16, "mode": mode, "depth": depth, "background": "white"})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

/// A History Brush stroke from row 1 with `extra` merged in (`"state": null` for the default source).
fn params(extra: Value) -> Value {
    let mut p = json!({"points": [[4, 8], [20, 8]], "size": 6, "hardness": 100, "state": 1});
    if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    p
}

/// The History Brush's error, after checking it added no state and changed no pixel.
fn refused(s: &mut Session, extra: Value, case: &str) -> String {
    let (before, n) = (s.active().unwrap().doc.clone(), steps(s));
    let e = s.execute("paint.historyBrush", params(extra)).expect_err(case).to_string();
    assert_eq!(steps(s), n, "{case}: no state");
    assert!(s.active().unwrap().doc == before, "{case}: no pixel changed");
    e
}

/// The History Brush stroke paints and adds one state.
fn paints(s: &mut Session, extra: Value, case: &str) {
    let n = steps(s);
    let r = s.execute("paint.historyBrush", params(extra)).unwrap_or_else(|e| panic!("{case}: {e}"));
    assert!(!r["damage"].is_null(), "{case}: {r}");
    assert_eq!(steps(s), n + 1, "{case}");
}

#[test]
fn rgb_to_grayscale_is_refused() {
    for depth in [8u64, 16] {
        for state in [json!(1), Value::Null] {
            let mut s = session("rgb", depth);
            s.execute("image.mode.grayscale", json!({})).unwrap();
            let case = format!("depth {depth} state {state}");
            assert_eq!(refused(&mut s, json!({"state": state}), &case), MODE, "{case}");
        }
    }
    let mut s = session("rgb", 8);
    let snap = s.execute("history.newSnapshot", json!({})).unwrap()["snapshot"].clone();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    assert_eq!(refused(&mut s, json!({"state": null, "snapshot": snap}), "snapshot"), MODE);
}

#[test]
fn a_source_of_another_bit_depth_is_refused() {
    let cases = [("rgb", 8u64, "image.mode.bits16"), ("rgb", 16, "image.mode.bits32"), ("rgb", 16, "image.mode.bits8"), ("rgb", 32, "image.mode.bits16")];
    for (mode, depth, bits) in cases.into_iter().chain([("gray", 8, "image.mode.bits16")]) {
        for state in [json!(1), Value::Null] {
            let mut s = session(mode, depth);
            s.execute(bits, json!({})).unwrap();
            let case = format!("{mode} {depth} {bits} state {state}");
            assert_eq!(refused(&mut s, json!({"state": state}), &case), DEPTH, "{case}");
        }
    }
}

/// Not measured, so these keep today's behaviour: RGB to CMYK or Lab, and a change of both the
/// colour mode and the bit depth, convert the source; Indexed Color flattens the document, so the
/// source lacks the new layer, as before.
#[test]
fn unmeasured_mode_pairs_keep_converting() {
    for mode in ["image.mode.cmyk", "image.mode.lab"] {
        let mut s = session("rgb", 8);
        s.execute(mode, json!({})).unwrap();
        paints(&mut s, json!({}), mode);
    }
    let mut s = session("rgb", 8);
    s.execute("image.mode.indexedColor", json!({})).unwrap();
    assert!(refused(&mut s, json!({}), "indexed").starts_with("the target did not exist"));
    let mut s = session("rgb", 8);
    s.execute("image.mode.bits16", json!({})).unwrap();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    paints(&mut s, json!({}), "mode and depth");
}

#[test]
fn the_canvas_size_refusal_comes_first() {
    let mut s = session("rgb", 8);
    s.execute("image.mode.grayscale", json!({})).unwrap();
    s.execute("image.canvasSize", json!({"width": 30, "height": 16, "anchor": "topLeft"})).unwrap();
    assert!(refused(&mut s, json!({}), "size and mode").contains("canvas size"));
}

/// As in Photoshop, a document converted before the source state was taken paints as before:
/// the source and the document are in the same mode and depth.
#[test]
fn a_source_of_the_same_mode_and_depth_still_paints() {
    for depth in [8u64, 16, 32] {
        for mode in ["rgb", "gray", "cmyk", "lab"] {
            let mut s = session(mode, depth);
            s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
            let before = s.active().unwrap().doc.clone();
            paints(&mut s, json!({}), &format!("depth {depth} {mode}"));
            assert!(s.active().unwrap().doc != before, "depth {depth} {mode}: the red state painted");
        }
    }
    let mut s = session("rgb", 8);
    s.execute("image.mode.bits16", json!({})).unwrap();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    let row = s.active().unwrap().history.entries().len() - 1;
    s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
    paints(&mut s, json!({"state": row}), "converted before the source state");
}

/// The live stroke refuses the same way, so its preview never shows a stroke the commit refuses.
#[test]
fn the_live_preview_is_refused_too() {
    for (change, text) in [("image.mode.grayscale", MODE), ("image.mode.bits16", DEPTH)] {
        let mut s = session("rgb", 8);
        s.execute(change, json!({})).unwrap();
        let e = LiveRetouch::begin(&s, "paint.historyBrush", &params(json!({}))).err().map(|e| e.to_string());
        assert_eq!(e.as_deref(), Some(text), "{change}");
        assert!(LiveRetouch::begin(&s, "paint.historyBrush", &params(json!({"eraseToHistory": true}))).is_ok(), "{change}: Erase to History");
    }
}

/// Only the History Brush refuses: the Eraser's Erase to History and Fill › History were not
/// measured and keep converting the source.
#[test]
fn other_history_painting_keeps_todays_behaviour() {
    for change in ["image.mode.grayscale", "image.mode.bits16"] {
        let mut s = session("rgb", 8);
        s.execute(change, json!({})).unwrap();
        let n = steps(&s);
        s.execute("paint.historyBrush", params(json!({"eraseToHistory": true}))).unwrap();
        s.execute("edit.fill", json!({"contents": "history", "state": 1})).unwrap();
        assert_eq!(steps(&s), n + 2, "{change}");
        // Only the absence of this tool's texts: the Art History Brush gets its own colour mode
        // refusal in #12826.
        if let Err(e) = s.execute("paint.artHistoryBrush", params(json!({}))) {
            let e = e.to_string();
            assert!(e != MODE && e != DEPTH, "{change}: {e}");
        }
    }
}
