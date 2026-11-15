//! The Art History Brush refuses a source state from before Image › Mode changed the document's
//! colour mode, as Photoshop does, for a normal stroke and for a click the tolerance holds back:
//! no state, no pixel change (#12826).

use super::art_history_noop_state_tests::{held_back_session, pixels, steps, stroke};
use super::*;

const MODES: [&str; 3] = ["image.mode.grayscale", "image.mode.cmyk", "image.mode.lab"];

/// Indexed Color flattens the document, so the source no longer holds the active layer: the
/// refusal still names the colour mode, which its RGB pixel format would hide. (Multichannel leaves
/// no layer, so the command is unavailable there before any source is read.)
#[test]
fn a_source_of_another_color_mode_is_refused() {
    let cases = [8u64, 16].into_iter().flat_map(|depth| MODES.map(move |mode| (depth, mode)));
    for (depth, mode) in cases.chain([(8, "image.mode.indexedColor")]) {
        // Tolerance 0 paints; 90 holds this small change back, which #12636 records as a state.
        for tolerance in [0, 90] {
            // Row 1 (RGB) passed, and the History panel's default source, the RGB Open state.
            for state in [json!(1), Value::Null] {
                let mut s = held_back_session(depth);
                s.execute(mode, json!({})).unwrap();
                let (before, n) = (s.active().unwrap().doc.clone(), steps(&s));
                let case = format!("depth {depth} {mode} tolerance {tolerance} state {state}");
                let e = stroke(&mut s, json!({"tolerance": tolerance, "state": state})).unwrap_err().to_string();
                assert!(e.contains("color mode does not match"), "{case}: {e}");
                assert_eq!(steps(&s), n, "{case}: no state");
                assert!(s.active().unwrap().doc == before, "{case}: no pixel changed");
            }
        }
    }
}

#[test]
fn a_source_in_the_current_color_mode_still_paints() {
    for mode in MODES {
        let mut s = held_back_session(8);
        s.execute(mode, json!({})).unwrap();
        let row = s.active().unwrap().history.entries().len() - 1;
        s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
        let (before, n) = (pixels(&s), steps(&s));
        let r = stroke(&mut s, json!({"tolerance": 0, "state": row})).unwrap();
        assert!(!r["damage"].is_null(), "{mode}: {r}");
        assert_eq!(steps(&s), n + 1, "{mode}");
        assert_ne!(pixels(&s), before, "{mode}: the source state painted");
    }
}
