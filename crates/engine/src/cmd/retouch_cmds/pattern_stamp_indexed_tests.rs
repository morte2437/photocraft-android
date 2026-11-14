//! #12887: in an Indexed Color document the Pattern Stamp refuses every pattern (RGB, Grayscale,
//! CMYK, Lab, or one defined from an Indexed image) with the reference application's message,
//! painting nothing and adding no history; the live strokes are refused the same way. RGB,
//! Grayscale, CMYK and Lab documents at every depth keep painting with it.

use super::super::LiveRetouch;
use super::*;

/// A new `mode` document of `depth` bits, 64 × 64, white.
fn new_doc(s: &mut Session, mode: &str, depth: u64) {
    s.execute("file.new", json!({"width": 64, "height": 64, "depth": depth, "mode": mode})).unwrap();
}

/// The reference's test pattern: 20 × 20 with red, green, blue and white quadrants, defined from a
/// `mode` document. Returns its id.
fn quadrants(s: &mut Session, mode: &str) -> String {
    s.execute("file.new", json!({"width": 20, "height": 20, "depth": 8, "mode": mode})).unwrap();
    for (x, y, color) in [(0, 0, "#ff0000"), (10, 0, "#00ff00"), (0, 10, "#0000ff")] {
        s.execute("select.rect", json!({"x": x, "y": y, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": color})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    let r = s.execute("edit.definePattern", json!({"name": format!("Quadrants {mode}")})).unwrap();
    r["pattern"].as_str().unwrap().to_string()
}

fn to_indexed(s: &mut Session) {
    s.execute("image.mode.indexedColor", json!({"palette": "exact", "forced": "none", "dither": "none"})).unwrap();
}

fn stroke(pattern: &str, impressionist: bool) -> Value {
    json!({"points": [[12, 32], [52, 32]], "size": 30, "hardness": 100, "pattern": pattern, "impressionist": impressionist})
}

/// The active layer's pixels and the History length.
fn state(s: &Session) -> (Vec<[f32; 4]>, usize) {
    let st = s.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    ((0..64).flat_map(|y| (0..64).map(move |x| surf.rgba(x, y))).collect(), st.history.past_len())
}

#[test]
fn an_rgb_pattern_is_refused_in_an_indexed_document_with_the_reference_words() {
    let mut s = Session::new();
    let pat = quadrants(&mut s, "rgb");
    new_doc(&mut s, "rgb", 8);
    to_indexed(&mut s);
    let before = state(&s);
    for impressionist in [false, true] {
        let err = s.execute("paint.patternStamp", stroke(&pat, impressionist)).unwrap_err().to_string();
        assert_eq!(err, INDEXED_REFUSAL, "impressionist {impressionist}");
        assert_eq!(state(&s), before, "impressionist {impressionist}: nothing painted, no history");
    }
    // The press check the canvas alerts with, and the live strokes, which preview nothing.
    assert_eq!(press_refusal(&s, &stroke(&pat, false)).as_deref(), Some(INDEXED_REFUSAL));
    let live = LiveRetouch::begin(&s, "paint.patternStamp", &stroke(&pat, false)).err().map(|e| e.to_string());
    assert_eq!(live.as_deref(), Some(INDEXED_REFUSAL), "live");
    let dabs = live_impressionist(&s, &stroke(&pat, true)).err().map(|e| e.to_string());
    assert_eq!(dabs.as_deref(), Some(INDEXED_REFUSAL), "live Impressionist");
}

#[test]
fn other_documents_keep_painting_with_the_rgb_pattern() {
    let mut s = Session::new();
    let pat = quadrants(&mut s, "rgb");
    for mode in ["rgb", "grayscale", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            new_doc(&mut s, mode, depth);
            let (pixels, steps) = state(&s);
            assert_eq!(press_refusal(&s, &stroke(&pat, false)), None, "{mode}/{depth}");
            s.execute("paint.patternStamp", stroke(&pat, false)).unwrap_or_else(|e| panic!("{mode}/{depth}: {e}"));
            let after = state(&s);
            assert_ne!(after.0, pixels, "{mode}/{depth}: painted");
            assert_eq!(after.1, steps + 1, "{mode}/{depth}: one history step");
        }
    }
}

#[test]
fn grayscale_cmyk_lab_and_indexed_patterns_are_refused_in_an_indexed_document_too() {
    let mut s = Session::new();
    let mut patterns = ["grayscale", "cmyk", "lab"].map(|mode| quadrants(&mut s, mode)).to_vec();
    // A pattern defined from an Indexed Color image.
    s.execute("file.new", json!({"width": 20, "height": 20, "depth": 8, "mode": "rgb"})).unwrap();
    to_indexed(&mut s);
    patterns.push(s.execute("edit.definePattern", json!({"name": "Indexed"})).unwrap()["pattern"].as_str().unwrap().to_string());
    new_doc(&mut s, "rgb", 8);
    to_indexed(&mut s);
    let before = state(&s);
    for pat in patterns {
        let err = s.execute("paint.patternStamp", stroke(&pat, false)).unwrap_err().to_string();
        assert_eq!(err, INDEXED_REFUSAL, "{pat}");
        assert_eq!(press_refusal(&s, &stroke(&pat, false)).as_deref(), Some(INDEXED_REFUSAL), "{pat}");
        assert_eq!(state(&s), before, "{pat}: nothing painted, no history");
    }
}
