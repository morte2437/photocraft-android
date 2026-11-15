//! #12536: the Paint Bucket refuses a Bitmap document with the reference's alert text, before
//! anything else, and leaves the document and History unchanged. Grayscale, RGB, CMYK, Lab and
//! Indexed Color documents still fill; Duotone and Multichannel were not measured and keep
//! today's behaviour (no refusal).

use super::*;

fn bitmap() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    s.execute("image.mode.grayscale", json!({})).unwrap();
    s.execute("image.mode.bitmap", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.mode, photocraft_color::ColorMode::Bitmap);
    s.execute("tools.setColors", json!({"foreground": "#00ff00"})).unwrap();
    s
}

fn assert_refused(s: &mut Session, params: Value, case: &str) {
    let (doc, steps) = (s.active().unwrap().doc.clone(), s.active().unwrap().history.past_len());
    let err = s.execute("paint.bucket", params).unwrap_err().to_string();
    assert_eq!(err, BITMAP_REFUSAL, "{case}");
    assert!(std::sync::Arc::ptr_eq(&doc, &s.active().unwrap().doc), "{case}: document unchanged");
    assert_eq!(s.active().unwrap().history.past_len(), steps, "{case}: no history state");
}

#[test]
fn the_refusal_is_the_reference_alert_text() {
    assert_eq!(BITMAP_REFUSAL, "Could not use the paint bucket because it does not work with Bitmap images (convert image to Grayscale mode to edit).");
}

#[test]
fn a_bitmap_document_is_refused() {
    let mut s = bitmap();
    assert_refused(&mut s, json!({"x": 32, "y": 32}), "foreground");
    assert_refused(&mut s, json!({"x": 32, "y": 32, "color": "#000000", "mode": "clear"}), "black, Clear");
    assert_refused(&mut s, json!({"x": 32, "y": 32, "contents": "pattern"}), "a pattern");
    assert_refused(&mut s, json!({"x": 32, "y": 32, "sampleAllLayers": true}), "all layers");
}

/// The mode comes first: a click that would fill nothing, or a target that would be refused,
/// gets the Bitmap alert as well.
#[test]
fn the_refusal_comes_before_the_click_and_the_target() {
    let mut s = bitmap();
    assert_refused(&mut s, json!({"x": -5, "y": 32}), "off the canvas");
    assert_refused(&mut s, json!({"x": 32, "y": 32, "layer": 999_999}), "a missing layer");
    assert_refused(&mut s, json!({"x": 32, "y": 32, "target": "mask"}), "a missing mask");
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8, "antiAlias": false})).unwrap();
    assert_refused(&mut s, json!({"x": 32, "y": 32}), "outside the selection");
}

#[test]
fn other_modes_still_fill() {
    for (mode, depth) in [
        ("grayscale", 8),
        ("grayscale", 16),
        ("grayscale", 32),
        ("rgb", 8),
        ("rgb", 16),
        ("rgb", 32),
        ("cmyk", 8),
        ("cmyk", 16),
        ("lab", 8),
        ("lab", 16),
        ("indexed", 8),
    ] {
        let mut s = Session::new();
        let new_mode = if mode == "indexed" { "rgb" } else { mode };
        s.execute("file.new", json!({"width": 64, "height": 64, "mode": new_mode, "depth": depth, "background": "white"})).unwrap();
        if mode == "indexed" {
            s.execute("image.mode.indexedColor", json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc.mode, photocraft_color::ColorMode::Indexed);
        }
        s.execute("tools.setColors", json!({"foreground": "#000000"})).unwrap();
        let steps = s.active().unwrap().history.past_len();
        let r = s.execute("paint.bucket", json!({"x": 32, "y": 32})).unwrap_or_else(|e| panic!("{mode} {depth}: {e}"));
        assert_eq!(r["filled"], true, "{mode} {depth}: {r}");
        assert_eq!(s.active().unwrap().history.past_len(), steps + 1, "{mode} {depth}");
    }
}

#[test]
fn unmeasured_modes_are_not_refused() {
    let mut multi = Session::new();
    multi.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    multi.execute("image.mode.multichannel", json!({})).unwrap();
    let mut duotone = Session::new();
    duotone.execute("file.new", json!({"width": 64, "height": 64, "mode": "grayscale", "background": "white"})).unwrap();
    duotone.execute("image.mode.duotone", json!({"type": "monotone"})).unwrap();
    for (mut s, mode) in [(multi, photocraft_color::ColorMode::Multichannel), (duotone, photocraft_color::ColorMode::Duotone)] {
        assert_eq!(s.active().unwrap().doc.mode, mode);
        let r = s.execute("paint.bucket", json!({"x": 32, "y": 32}));
        assert!(r.as_ref().err().is_none_or(|e| e.to_string() != BITMAP_REFUSAL), "{mode:?}: {r:?}");
    }
}
