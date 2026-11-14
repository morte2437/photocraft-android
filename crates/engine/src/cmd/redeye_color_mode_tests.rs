//! #12575: the Red Eye tool refuses CMYK, Grayscale and Indexed Color documents with the
//! reference's alert text and leaves the document and History unchanged. RGB (8, 16, 32 bit) and
//! Lab still correct the pupil; Bitmap, Duotone and Multichannel were not measured and keep
//! today's behaviour (no refusal).

use serde_json::{Value, json};

use super::*;

fn new_doc(mode: &str, depth: u64) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "mode": mode, "depth": depth})).unwrap();
    s
}

/// A red pupil on its own layer.
fn red_pupil(s: &mut Session) {
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("paint.pencil", json!({"points": [[32, 32]], "size": 14, "hardness": 100, "color": "#f21818"})).unwrap();
}

fn assert_refused(s: &mut Session, params: Value, case: &str) {
    let (doc, steps) = (s.active().unwrap().doc.clone(), s.active().unwrap().history.past_len());
    let err = s.execute("paint.redEye", params).unwrap_err().to_string();
    assert_eq!(err, MODE_REFUSAL, "{case}");
    assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc), "{case}: document unchanged");
    assert_eq!(s.active().unwrap().history.past_len(), steps, "{case}: no history state");
}

#[test]
fn the_refusal_is_the_reference_alert_text() {
    assert_eq!(MODE_REFUSAL, "Could not use the red eye removal tool because it only works in full color modes.");
}

#[test]
fn cmyk_and_grayscale_documents_are_refused() {
    for (mode, depths) in [("cmyk", &[8u64, 16][..]), ("gray", &[8, 16, 32][..])] {
        for &depth in depths {
            let mut s = new_doc(mode, depth);
            red_pupil(&mut s);
            assert_refused(&mut s, json!({"x": 32, "y": 32}), &format!("{mode} {depth} bit"));
        }
    }
}

#[test]
fn indexed_color_documents_are_refused() {
    let mut s = new_doc("rgb", 8);
    red_pupil(&mut s);
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).unwrap();
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Indexed);
    assert_refused(&mut s, json!({"x": 32, "y": 32}), "indexed");
    assert_refused(&mut s, json!({"points": [[32, 32], [10, 10]]}), "indexed, a points batch");
}

/// Converting to Indexed Color flattens the document, so a script's layer id is gone: the mode
/// is refused before the layer is looked up.
#[test]
fn the_refusal_comes_before_the_layer_lookup() {
    let mut s = new_doc("rgb", 8);
    red_pupil(&mut s);
    let id = s.active().unwrap().active_layer.unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "dither": "none", "forced": "none"})).unwrap();
    assert_refused(&mut s, json!({"x": 32, "y": 32, "layer": id.0}), "a flattened layer id");
}

#[test]
fn rgb_and_lab_documents_still_correct_the_pupil() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("lab", 8), ("lab", 16)] {
        let mut s = new_doc(mode, depth);
        red_pupil(&mut s);
        let steps = s.active().unwrap().history.past_len();
        let r = s.execute("paint.redEye", json!({"x": 32, "y": 32})).unwrap_or_else(|e| panic!("{mode} {depth}: {e}"));
        assert!(r["pixels"].as_u64().unwrap() > 0, "{mode} {depth}: {r}");
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{mode} {depth}");
    }
}

#[test]
fn unmeasured_modes_are_not_refused() {
    let mut multi = new_doc("rgb", 8);
    red_pupil(&mut multi);
    multi.execute("image.mode.multichannel", json!({})).unwrap();
    let mut bitmap = new_doc("gray", 8);
    bitmap.execute("image.mode.bitmap", json!({})).unwrap();
    let mut duotone = new_doc("gray", 8);
    duotone.execute("image.mode.duotone", json!({"type": "monotone"})).unwrap();
    for (mut s, mode) in [(multi, ColorMode::Multichannel), (bitmap, ColorMode::Bitmap), (duotone, ColorMode::Duotone)] {
        assert_eq!(s.active().unwrap().doc.mode, mode);
        let r = s.execute("paint.redEye", json!({"x": 32, "y": 32}));
        assert!(r.as_ref().err().is_none_or(|e| e.to_string() != MODE_REFUSAL), "{mode:?}: {r:?}");
    }
}
