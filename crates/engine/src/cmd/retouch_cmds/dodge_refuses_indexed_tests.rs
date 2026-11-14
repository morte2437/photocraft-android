//! #12686: the Dodge tool refuses Indexed Color and Bitmap documents with the reference
//! application's messages, toning nothing and adding no history; the live stroke is refused the
//! same way. RGB, Grayscale, CMYK and Lab at every depth, Multichannel and Duotone keep dodging.

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Size};
use serde_json::json;

use super::*;

const INDEXED: &str = "Could not use the dodge tool because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";
const BITMAP: &str = "Could not use the dodge tool because it does not work with Bitmap images (convert image to Grayscale mode to edit).";

/// A session with one 64 × 64 document of `mode` and `depth`, filled mid grey.
pub(super) fn session(mode: ColorMode, depth: SampleType) -> Session {
    let mut s = Session::new();
    s.add_document(Document::with_background("t", Size::new(64, 64), mode, depth, Color::rgb(0.5, 0.5, 0.5)), None);
    s
}

fn stroke() -> Value {
    json!({"points": [[20, 32], [44, 32]], "size": 20, "hardness": 100, "range": "midtones", "exposure": 100})
}

pub(super) fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

pub(super) fn pixels(s: &Session) -> Vec<f32> {
    let st = s.active().unwrap();
    let surf = st.doc.layers[0].surface().unwrap();
    (0..64).flat_map(|y| (0..64).flat_map(move |x| surf.pixel(x, y))).collect()
}

#[test]
fn indexed_and_bitmap_documents_refuse_with_the_reference_words() {
    for (mode, want) in [(ColorMode::Indexed, INDEXED), (ColorMode::Bitmap, BITMAP)] {
        let mut s = session(mode, SampleType::U8);
        let (before, n) = (pixels(&s), steps(&s));
        let err = s.execute("paint.dodge", stroke()).unwrap_err().to_string();
        assert_eq!(err, want, "{mode:?}");
        assert_eq!(pixels(&s), before, "{mode:?}: nothing toned");
        assert_eq!(steps(&s), n, "{mode:?}: no history");
        // The live stroke previews nothing either.
        let live = LiveDab::begin(&s, "paint.dodge", &stroke()).err().map(|e| e.to_string());
        assert_eq!(live.as_deref(), Some(want), "{mode:?}: live");
    }
}

#[test]
fn other_modes_keep_dodging_at_every_depth() {
    let mut cases = Vec::new();
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            cases.push((mode, depth));
        }
    }
    // Not measured in the reference: they keep today's behaviour.
    cases.extend([(ColorMode::Multichannel, SampleType::U8), (ColorMode::Duotone, SampleType::U8)]);
    for (mode, depth) in cases {
        let mut s = session(mode, depth);
        let (before, n) = (pixels(&s), steps(&s));
        s.execute("paint.dodge", stroke()).unwrap_or_else(|e| panic!("{mode:?}/{depth:?}: {e}"));
        assert_ne!(pixels(&s), before, "{mode:?}/{depth:?}: dodged");
        assert_eq!(steps(&s), n + 1, "{mode:?}/{depth:?}: one history step");
        assert!(LiveDab::begin(&s, "paint.dodge", &stroke()).is_ok(), "{mode:?}/{depth:?}: live");
    }
}
