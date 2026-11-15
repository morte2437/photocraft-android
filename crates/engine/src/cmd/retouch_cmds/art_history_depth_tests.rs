//! The Art History Brush works in 8-bit documents only and refuses a source of another colour
//! mode, with the reference application's alert texts: no state, no pixel change (#12637). The
//! History Brush keeps painting at every depth.

use super::art_history_noop_state_tests::{pixels, steps, stroke};
use super::*;

const BITS16: &str =
    "Could not use the art history brush because it does not work with 16 bit per channel images (convert image to 8 bit per channel to edit).";
const BITS32: &str =
    "Could not use the art history brush because it does not work with 32 bit per channel images (convert image to 8 or 16 bit per channel to edit).";
const NOT_EDITABLE: &str = "Could not use the art history brush because the pixel data for this layer is not directly editable.";
const MODE: &str = "Could not use the art history brush because the current color mode does not match that of the history state.";

/// Rows Open (white), Fill (red): row 1 is the red state.
fn filled(depth: u64, mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 48, "height": 32, "depth": depth, "mode": mode, "background": "white"})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s
}

/// The newest History panel row, the state that holds every layer there is now.
fn last_row(s: &Session) -> usize {
    s.active().unwrap().history.entries().len() - 1
}

/// The stroke's error, after checking it added no state and changed no pixel.
fn refused(s: &mut Session, extra: Value, case: &str) -> String {
    let (before, n) = (s.active().unwrap().doc.clone(), steps(s));
    let e = stroke(s, extra).expect_err(case).to_string();
    assert_eq!(steps(s), n, "{case}: no state");
    assert!(s.active().unwrap().doc == before, "{case}: no pixel changed");
    e
}

/// A tolerance-0 stroke from `state` that paints and adds one state.
fn paints(s: &mut Session, state: Value, case: &str) {
    let (before, n) = (pixels(s), steps(s));
    let r = stroke(s, json!({"tolerance": 0, "state": state})).unwrap_or_else(|e| panic!("{case}: {e}"));
    assert!(!r["damage"].is_null(), "{case}: {r}");
    assert_eq!(steps(s), n + 1, "{case}");
    assert_ne!(pixels(s), before, "{case}: the source painted");
}

#[test]
fn a_single_layer_16_bit_document_is_refused() {
    for mode in ["rgb", "gray"] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 48, "height": 32, "depth": 16, "mode": mode})).unwrap();
        // The source is the open state, the History panel's default.
        assert_eq!(refused(&mut s, json!({"state": null, "tolerance": 0}), mode), BITS16, "{mode}");
        let mut s = filled(16, mode);
        assert_eq!(refused(&mut s, json!({"tolerance": 0}), mode), BITS16, "{mode} from row 1");
    }
}

#[test]
fn a_layered_16_bit_document_is_not_directly_editable() {
    let mut s = filled(16, "rgb");
    s.execute("layer.new.layer", json!({})).unwrap();
    let row = last_row(&s);
    assert_eq!(refused(&mut s, json!({"tolerance": 0, "state": row}), "two layers"), NOT_EDITABLE);
}

#[test]
fn a_depth_change_to_16_bits_after_the_source_is_not_directly_editable() {
    for layered in [false, true] {
        let mut s = filled(8, "rgb");
        if layered {
            s.execute("layer.new.layer", json!({})).unwrap();
        }
        s.execute("image.mode.bits16", json!({})).unwrap();
        for tolerance in [0, 90] {
            let case = format!("layered {layered} tolerance {tolerance}");
            let row = last_row(&s) - 1;
            let e = refused(&mut s, json!({"tolerance": tolerance, "state": row}), &case);
            assert_eq!(e, NOT_EDITABLE, "{case}");
        }
    }
}

#[test]
fn a_32_bit_document_is_refused() {
    let mut s = filled(32, "rgb");
    assert_eq!(refused(&mut s, json!({"tolerance": 0}), "new 32-bit"), BITS32);
    s.execute("layer.new.layer", json!({})).unwrap();
    let row = last_row(&s);
    assert_eq!(refused(&mut s, json!({"tolerance": 0, "state": row}), "two layers"), BITS32);
    let mut s = filled(8, "rgb");
    s.execute("image.mode.bits32", json!({})).unwrap();
    assert_eq!(refused(&mut s, json!({"tolerance": 0}), "8 to 32 bits after the source"), BITS32);
}

#[test]
fn a_source_of_another_color_mode_gives_the_color_mode_text() {
    let mut s = filled(8, "rgb");
    s.execute("image.mode.grayscale", json!({})).unwrap();
    for tolerance in [0, 90] {
        assert_eq!(refused(&mut s, json!({"tolerance": tolerance}), "RGB to Grayscale"), MODE, "tolerance {tolerance}");
    }
}

#[test]
fn eight_bit_documents_paint_as_before() {
    for mode in ["rgb", "gray", "cmyk", "lab"] {
        let mut s = filled(8, mode);
        paints(&mut s, json!(0), mode);
    }
    // Grayscale with the source taken after the conversion.
    let mut s = filled(8, "rgb");
    s.execute("image.mode.grayscale", json!({})).unwrap();
    let row = last_row(&s);
    s.execute("edit.fill", json!({"color": "#ffffff"})).unwrap();
    paints(&mut s, json!(row), "Grayscale source after the conversion");
}

/// 16 -> 8 bits after the source wasn't measured: the deeper source is converted and paints, as
/// before.
#[test]
fn an_8_bit_document_with_a_deeper_source_still_paints() {
    let mut s = filled(16, "rgb");
    s.execute("image.mode.bits8", json!({})).unwrap();
    paints(&mut s, json!(0), "16 to 8 bits after the source");
}

#[test]
fn the_history_brush_still_paints_at_every_depth() {
    for depth in [8u64, 16, 32] {
        let mut s = filled(depth, "rgb");
        let (before, n) = (pixels(&s), steps(&s));
        // Row 0, the white open state, over the red fill.
        let p = json!({"points": [[8, 16], [40, 16]], "size": 12, "hardness": 100, "state": 0});
        s.execute("paint.historyBrush", p).unwrap_or_else(|e| panic!("depth {depth}: {e}"));
        assert_eq!(steps(&s), n + 1, "depth {depth}");
        assert_ne!(pixels(&s), before, "depth {depth}: the source painted");
    }
}
