//! #12656: the Blur tool refuses Indexed Color and Bitmap documents with the reference
//! application's messages, changing nothing and adding no history; the live stroke is refused the
//! same way. RGB, Grayscale, CMYK and Lab at every depth, Multichannel and Duotone keep working.

use photocraft_color::{ColorMode, SampleType};
use serde_json::json;

use super::dodge_refuses_indexed_tests::{pixels, session, steps};
use super::*;

const INDEXED: &str = "Could not use the blur tool because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";
const BITMAP: &str = "Could not use the blur tool because it does not work with Bitmap images (convert image to Grayscale mode to edit).";

/// The grey document with its left half black: an edge for the stroke to work on.
fn edged(mode: ColorMode, depth: SampleType) -> Session {
    let mut s = session(mode, depth);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 64})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn stroke() -> Value {
    json!({"points": [[20, 32], [44, 32]], "size": 20, "hardness": 100, "strength": 100})
}

#[test]
fn indexed_and_bitmap_documents_refuse_with_the_reference_words() {
    // Per mode: the command's error, the live stroke's error, pixels kept, history kept.
    let got = [ColorMode::Indexed, ColorMode::Bitmap].map(|mode| {
        let mut s = edged(mode, SampleType::U8);
        let (before, n) = (pixels(&s), steps(&s));
        let err = s.execute("paint.blur", stroke()).err().map(|e| e.to_string());
        let live = LiveDab::begin(&s, "paint.blur", &stroke()).err().map(|e| e.to_string());
        (err, live, pixels(&s) == before, steps(&s) == n)
    });
    let want = [INDEXED, BITMAP].map(|m| (Some(m.to_string()), Some(m.to_string()), true, true));
    assert_eq!(got, want);
}

/// Every other mode and depth keeps working: one History step and a live stroke.
#[test]
fn other_modes_keep_working_at_every_depth() {
    let depths = [SampleType::U8, SampleType::U16, SampleType::F32];
    let measured = [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab].into_iter().flat_map(|m| depths.map(|d| (m, d)));
    // Not measured in the reference: they keep today's behaviour.
    let unmeasured = [(ColorMode::Multichannel, SampleType::U8), (ColorMode::Duotone, SampleType::U8)];
    for (mode, depth) in measured.chain(unmeasured) {
        let mut s = edged(mode, depth);
        let n = steps(&s);
        assert!(LiveDab::begin(&s, "paint.blur", &stroke()).is_ok(), "{mode:?}/{depth:?}: live");
        let before = pixels(&s);
        s.execute("paint.blur", stroke()).unwrap_or_else(|e| panic!("{mode:?}/{depth:?}: {e}"));
        assert_ne!(pixels(&s), before, "{mode:?}/{depth:?}: the stroke changed pixels");
        assert_eq!(steps(&s), n + 1, "{mode:?}/{depth:?}: one history step");
    }
}
