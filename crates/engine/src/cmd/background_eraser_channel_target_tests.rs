//! The Background Eraser with one colour channel of an RGB document targeted (#12488): the
//! reference application refuses with an alert, erases nothing and adds no History state, with
//! Lock transparent pixels off and on. The composite erases as before; the partial sets that
//! weren't measured (two of three channels, CMYK inks and Lab components) keep erasing those
//! channels. 32-bit documents are left to #11912, where the reference refuses the tool outright.

use super::transparency_lock_tests::{session, target};
use super::*;

const CLICK: [[f64; 2]; 1] = [[32.0, 20.0]];
const DRAG: [[f64; 2]; 3] = [[8.0, 20.0], [32.0, 20.0], [56.0, 20.0]];

fn stroke(points: &[[f64; 2]], extra: Value) -> Value {
    let mut p = json!({"points": points, "size": 20, "hardness": 1, "tolerance": 50});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(extra.clone());
    }
    p
}

/// Runs `p` and the live preview of its first point, expecting both refused with the alert text
/// and the document (pixels, layers, History) left as it was.
fn assert_refused(s: &mut Session, p: Value, case: &str) {
    let (doc, steps) = (s.active().unwrap().doc.clone(), s.active().unwrap().history.past_len());
    let mut first = p.clone();
    first["points"] = json!([p["points"][0]]);
    let live = live_background_eraser(s, &first).err().map(|e| e.to_string());
    assert_eq!(live.as_deref(), Some(CHANNEL_TARGET_REFUSED), "{case}: the live preview shows nothing");
    let err = s.execute("paint.backgroundEraser", p).unwrap_err().to_string();
    assert_eq!(err, CHANNEL_TARGET_REFUSED, "{case}");
    let st = s.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&doc, &st.doc), "{case}: nothing erased");
    assert_eq!(st.history.past_len(), steps, "{case}: no History step");
}

/// Runs `p`, expecting it to erase as one History step.
fn assert_erases(s: &mut Session, p: Value, case: &str) {
    let id = s.active().unwrap().active_layer.unwrap();
    let (before, steps) = (s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().clone(), s.active().unwrap().history.past_len());
    s.execute("paint.backgroundEraser", p).unwrap_or_else(|e| panic!("{case}: {e}"));
    let st = s.active().unwrap();
    assert!(*st.doc.layer(id).unwrap().surface().unwrap() != before, "{case}: erased");
    assert_eq!(st.history.past_len(), steps + 1, "{case}: one History step");
}

#[test]
fn one_colour_channel_from_the_channels_panel_refuses_the_click_and_the_drag() {
    for depth in [8, 16] {
        for lock in [false, true] {
            for channel in ["red", "green", "blue"] {
                for points in [&CLICK[..], &DRAG[..]] {
                    let mut s = session(depth);
                    s.execute("layer.setProps", json!({"locks": {"transparency": lock}})).unwrap();
                    s.execute("channel.target", json!({"channel": channel})).unwrap();
                    assert_refused(&mut s, stroke(points, json!({})), &format!("{depth}-bit, lock {lock}, {channel}, {} points", points.len()));
                    assert_eq!(target(&s).locks.transparency, lock);
                }
            }
        }
    }
}

#[test]
fn one_colour_channel_as_the_target_param_refuses_too() {
    for k in 0..3 {
        let mut s = session(8);
        assert_refused(&mut s, stroke(&DRAG, json!({"target": {"colors": [k]}})), &format!("colors [{k}]"));
        let mut s = session(8);
        assert_refused(&mut s, stroke(&CLICK, json!({"target": {"color": k}})), &format!("color {k}"));
    }
}

#[test]
fn a_click_on_the_background_layer_is_refused_too() {
    for depth in [8, 16] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 40, "depth": depth})).unwrap();
        s.execute("channel.target", json!({"channel": "red"})).unwrap();
        assert_refused(&mut s, stroke(&CLICK, json!({})), &format!("{depth}-bit Background"));
    }
}

#[test]
fn the_composite_erases_as_before() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        s.execute("channel.target", json!({"channel": "red"})).unwrap();
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
        assert_erases(&mut s, stroke(&CLICK, json!({})), &format!("{depth}-bit composite"));
        assert_eq!(target(&s).surface().unwrap().rgba(32, 20)[3], 0.0, "{depth}-bit: erased to transparency");
        let mut s = session(depth);
        assert_erases(&mut s, stroke(&DRAG, json!({"target": {"colors": [0, 1, 2]}})), &format!("{depth}-bit, all three colour channels"));
    }
}

#[test]
fn partial_sets_that_were_not_measured_keep_erasing() {
    let mut s = session(8);
    assert_erases(&mut s, stroke(&DRAG, json!({"target": {"colors": [1, 2]}})), "RGB green and blue");
    for (mode, components) in [("cmyk", 4), ("lab", 3)] {
        for k in 0..components {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 64, "height": 40, "mode": mode})).unwrap();
            s.execute("select.rect", json!({"x": 0, "y": 10, "width": 64, "height": 20, "antiAlias": false})).unwrap();
            s.execute("edit.fill", json!({"color": "#1a3a6a"})).unwrap();
            s.execute("select.deselect", json!({})).unwrap();
            s.tools.background = [0.9, 0.2, 0.1, 1.0];
            assert_erases(&mut s, stroke(&DRAG, json!({"target": {"colors": [k]}})), &format!("{mode} component {k}"));
        }
    }
}
