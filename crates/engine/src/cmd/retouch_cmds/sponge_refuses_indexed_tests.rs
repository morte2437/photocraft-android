//! #13033: the Sponge tool refuses Indexed Color and Bitmap documents with the reference
//! application's messages, changing nothing and adding no history; the live stroke is refused the
//! same way. RGB, Grayscale, CMYK and Lab at every depth, Multichannel and Duotone keep working.

use photocraft_color::{ColorMode, SampleType};
use serde_json::json;

use super::dodge_refuses_indexed_tests::{pixels, session, steps};
use super::*;

const INDEXED: &str = "Could not use the sponge tool because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";
const BITMAP: &str = "Could not use the sponge tool because it does not work with Bitmap images (convert image to Grayscale mode to edit).";

fn stroke() -> Value {
    json!({"points": [[20, 32], [44, 32]], "size": 20, "hardness": 100, "mode": "desaturate"})
}

#[test]
fn indexed_and_bitmap_documents_refuse_with_the_reference_words() {
    // Per mode: the command's error, the live stroke's error, pixels kept, history kept.
    let got = [ColorMode::Indexed, ColorMode::Bitmap].map(|mode| {
        let mut s = session(mode, SampleType::U8);
        let (before, n) = (pixels(&s), steps(&s));
        let err = s.execute("paint.sponge", stroke()).err().map(|e| e.to_string());
        let live = LiveDab::begin(&s, "paint.sponge", &stroke()).err().map(|e| e.to_string());
        (err, live, pixels(&s) == before, steps(&s) == n)
    });
    let want = [INDEXED, BITMAP].map(|m| (Some(m.to_string()), Some(m.to_string()), true, true));
    assert_eq!(got, want);
}

/// Every other mode and depth keeps working: one History step and a live stroke.
///
/// The grey document has no saturation to take away, so this checks that the stroke runs and
/// records its History step, not that pixels change.
#[test]
fn other_modes_keep_working_at_every_depth() {
    let depths = [SampleType::U8, SampleType::U16, SampleType::F32];
    let measured = [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab].into_iter().flat_map(|m| depths.map(|d| (m, d)));
    // Not measured in the reference: they keep today's behaviour.
    let unmeasured = [(ColorMode::Multichannel, SampleType::U8), (ColorMode::Duotone, SampleType::U8)];
    for (mode, depth) in measured.chain(unmeasured) {
        let mut s = session(mode, depth);
        let n = steps(&s);
        assert!(LiveDab::begin(&s, "paint.sponge", &stroke()).is_ok(), "{mode:?}/{depth:?}: live");
        s.execute("paint.sponge", stroke()).unwrap_or_else(|e| panic!("{mode:?}/{depth:?}: {e}"));
        assert_eq!(steps(&s), n + 1, "{mode:?}/{depth:?}: one history step");
    }
}
