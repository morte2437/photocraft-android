//! #12032: the Magic Eraser refuses Indexed Color and Bitmap documents with the reference's
//! alerts, and refuses them *before* it samples or edits anything: no History step, the Background
//! stays a Background, the pixels and the selection stay as they were. Duotone and Multichannel
//! are not measured, so they keep today's behaviour; the modes and depths the issue lists as
//! working (RGB, Grayscale, CMYK and Lab at 8 bits, RGB at 16 and 32 bits) still erase.

use super::*;
use photocraft_doc::{Color, SampleType, Size};

/// The reference's alert for an Indexed Color document, measured (#12032); the alert has one OK
/// button. Spelled out here rather than taken from the command, so the test fails if the text the
/// app shows ever drifts from what was measured.
const INDEXED: &str = "Could not use the magic eraser because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";
/// Its alert for a Bitmap document, measured (#12032).
const BITMAP: &str = "Could not use the magic eraser because it does not work with Bitmap images (convert image to Grayscale mode to edit).";

/// A 64x48 document in `mode` at `depth`: a white Background - one flat colour, so a Magic Eraser
/// click on it would wipe the canvas out - with a live selection. Both must survive a refusal.
fn session(mode: photocraft_color::ColorMode, depth: SampleType) -> Session {
    let d = Document::with_background("palette", Size::new(64, 48), mode, depth, Color::WHITE);
    let mut s = Session::new();
    s.add_document(d, None);
    s.execute("select.rect", json!({"x": 4, "y": 4, "width": 20, "height": 20, "antiAlias": false})).unwrap();
    s
}

/// What a refused click must leave exactly as it was: the document itself, the History depth, one
/// pixel, and the selection.
fn snapshot(s: &Session) -> (std::sync::Arc<Document>, usize, Vec<f32>, Option<photocraft_raster::Surface>) {
    let st = s.active().unwrap();
    let pixels = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(2, 2).to_vec();
    (st.doc.clone(), st.history.past_len(), pixels, st.doc.selection.clone())
}

/// The text `paint.magicEraser` answers this click with, panicking with `case` in its message when
/// it erases or answers something else.
fn alert(s: &mut Session, case: &str, want: &str, p: Value) -> String {
    match s.execute("paint.magicEraser", p.clone()) {
        Err(e) if e.to_string() == want => e.to_string(),
        Err(e) => panic!("{case}: refused with the wrong text: {e}"),
        Ok(r) => panic!("{case}: the click erased instead of being refused: {r}"),
    }
}

#[test]
fn magic_eraser_refuses_indexed_and_bitmap_with_the_reference_alerts() {
    for (case, mode, want) in [("Indexed", photocraft_color::ColorMode::Indexed, INDEXED), ("Bitmap", photocraft_color::ColorMode::Bitmap, BITMAP)] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            // The refusal is the same whatever else the click asks for: sampling every layer or
            // only this one, the whole canvas or one colour, inside the selection or not.
            for p in [
                json!({"x": 2, "y": 2, "antiAlias": false}),
                json!({"x": 12, "y": 12, "antiAlias": false}),
                json!({"x": 2, "y": 2, "antiAlias": false, "sampleAllLayers": true}),
                json!({"x": 2, "y": 2, "antiAlias": false, "contiguous": false, "tolerance": 255, "opacity": 1}),
            ] {
                let name = format!("{case} {depth:?} {p}");
                let mut s = session(mode, depth);
                let (doc, steps, pixels, selection) = snapshot(&s);
                let id = s.active().unwrap().active_layer.unwrap();
                assert_eq!(alert(&mut s, &name, want, p), want.to_string());
                let st = s.active().unwrap();
                // Nothing at all happened: not a History step, not a pixel, not the selection, and
                // the Background is still a Background rather than the "Layer 0" the erase makes.
                assert!(std::sync::Arc::ptr_eq(&doc, &st.doc), "{name}: the document is untouched");
                assert_eq!(st.history.past_len(), steps, "{name}: no History step");
                assert_eq!(st.doc.layer(id).unwrap().name, "Background", "{name}: the layer name");
                assert!(st.doc.is_background(id), "{name}: still a Background");
                assert_eq!(st.doc.layer(id).unwrap().surface().unwrap().rgba(2, 2).to_vec(), pixels, "{name}: pixels unchanged");
                assert_eq!(st.doc.selection, selection, "{name}: the selection is unchanged");
            }
        }
    }
}

#[test]
fn magic_eraser_still_erases_the_measured_modes_and_depths() {
    for (mode, depth) in [("rgb", 8), ("gray", 8), ("cmyk", 8), ("lab", 8), ("rgb", 16), ("rgb", 32)] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48, "mode": mode, "depth": depth})).unwrap();
        let h = s.active().unwrap().history.past_len();
        let id = s.active().unwrap().active_layer.unwrap();
        let r = s.execute("paint.magicEraser", json!({"x": 2, "y": 2, "antiAlias": false})).unwrap();
        assert_eq!(r["erased"], true, "{mode} {depth}: still erases");
        assert_eq!(s.active().unwrap().history.past_len(), h + 1, "{mode} {depth}: one undo step");
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(id).unwrap().name, "Layer 0", "{mode} {depth}: the Background became a layer");
        assert_eq!(st.doc.layer(id).unwrap().surface().unwrap().rgba(2, 2)[3], 0.0, "{mode} {depth}: erased to transparency");
    }
}

/// Duotone and Multichannel are *not measured* (#12032), so they keep today's behaviour: the click
/// erases, and neither of the two alerts appears.
#[test]
fn duotone_and_multichannel_keep_todays_behaviour() {
    for (case, mode) in [("Duotone", photocraft_color::ColorMode::Duotone), ("Multichannel", photocraft_color::ColorMode::Multichannel)] {
        let mut s = session(mode, SampleType::U8);
        let id = s.active().unwrap().active_layer.unwrap();
        let r = s.execute("paint.magicEraser", json!({"x": 2, "y": 2, "antiAlias": false}));
        assert!(r.is_ok(), "{case}: keeps today's behaviour, answers {r:?}");
        assert_eq!(r.unwrap()["erased"], true, "{case}");
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().name, "Layer 0", "{case}");
    }
}
