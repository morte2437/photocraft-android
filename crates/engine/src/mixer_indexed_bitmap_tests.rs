//! The Mixer Brush refuses Indexed Color and Bitmap documents with the reference's alert texts
//! (#11958): `paint.mixerBrush` returns the error and leaves the document and History unchanged.
//! RGB, CMYK, Lab and Grayscale documents still paint.

use crate::Session;
use crate::brush_cmds::mixer_live::{BITMAP_REFUSAL, INDEXED_REFUSAL};
use serde_json::json;

fn stroke(s: &mut Session) -> crate::Result<serde_json::Value> {
    s.execute("paint.mixerBrush", json!({"points": [[10, 10], [40, 20]], "size": 12, "wet": 0, "mix": 0}))
}

fn assert_refused(s: &mut Session, text: &str) {
    let (doc, steps) = (s.active().unwrap().doc.clone(), s.active().unwrap().history.past_len());
    let err = stroke(s).unwrap_err().to_string();
    assert_eq!(err, text);
    assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc), "nothing painted");
    assert_eq!(s.active().unwrap().history.past_len(), steps, "no history step");
}

#[test]
fn indexed_color_documents_are_refused_with_the_alert() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 32, "background": "white"})).unwrap();
    s.execute("image.mode.indexedColor", json!({})).unwrap();
    assert_refused(&mut s, INDEXED_REFUSAL);
    assert_eq!(
        INDEXED_REFUSAL,
        "Could not use the Mixer Brush tool because it does not work with Indexed Color images (convert image to RGB Color mode to edit)."
    );
}

#[test]
fn bitmap_documents_are_refused_with_the_alert() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 32, "background": "white"})).unwrap();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    s.execute("image.mode.bitmap", json!({})).unwrap();
    assert_refused(&mut s, BITMAP_REFUSAL);
    assert_eq!(BITMAP_REFUSAL, "Could not use the Mixer Brush tool because it does not work with Bitmap images (convert image to Grayscale mode to edit).");
}

#[test]
fn rgb_cmyk_lab_and_grayscale_documents_still_paint() {
    let mut painted = 0;
    for mode in ["rgb", "cmyk", "lab", "grayscale"] {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            // Not every mode has every depth (as in the reference, CMYK and Lab stop at 16 bits).
            if s.execute("file.new", json!({"width": 64, "height": 32, "mode": mode, "depth": depth, "background": "white"})).is_err() {
                continue;
            }
            let steps = s.active().unwrap().history.past_len();
            stroke(&mut s).unwrap_or_else(|e| panic!("{mode} {depth}: {e}"));
            assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{mode} {depth}");
            painted += 1;
        }
    }
    assert!(painted >= 8, "only {painted} mode/depth combinations painted");
}
