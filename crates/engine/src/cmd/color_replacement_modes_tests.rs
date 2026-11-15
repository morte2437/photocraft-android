//! #11923: the Color Replacement tool refuses RGB/32, Grayscale, Bitmap and Indexed documents with
//! the reference application's messages, painting nothing and adding no history; the live stroke
//! is refused the same way. RGB/8, RGB/16, CMYK and Lab paint.

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Size};
use serde_json::json;

use super::*;

const DEPTH_32: &str =
    "Could not use the Color Replacement tool because it does not work with 32 bit per channel images (convert image to 8 or 16 bit per channel to edit).";
const NOT_FULL_COLOR: &str = "Could not use the Color Replacement tool because it only works in full color modes.";
const INDEXED: &str = "Could not use the Color Replacement tool because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";

/// A session with one 64 × 64 document of `mode` and `depth`, filled red.
fn session(mode: ColorMode, depth: SampleType) -> Session {
    let mut s = Session::new();
    s.add_document(Document::with_background("t", Size::new(64, 64), mode, depth, Color::rgb(220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0)), None);
    s.tools.foreground = [0.0, 0.0, 1.0, 1.0];
    s
}

fn stroke() -> Value {
    json!({"points": [[20, 32], [44, 32]], "size": 20, "hardness": 100})
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn pixels(s: &Session) -> Vec<f32> {
    let st = s.active().unwrap();
    let surf = st.doc.layers[0].surface().unwrap();
    (0..64).flat_map(|y| (0..64).flat_map(move |x| surf.pixel(x, y))).collect()
}

#[test]
fn refused_modes_say_why_and_paint_nothing() {
    for (mode, depth, want) in [
        (ColorMode::Rgb, SampleType::F32, DEPTH_32),
        (ColorMode::Grayscale, SampleType::U8, NOT_FULL_COLOR),
        (ColorMode::Grayscale, SampleType::U16, NOT_FULL_COLOR),
        (ColorMode::Bitmap, SampleType::U8, NOT_FULL_COLOR),
        (ColorMode::Indexed, SampleType::U8, INDEXED),
    ] {
        let mut s = session(mode, depth);
        let (before, n) = (pixels(&s), steps(&s));
        let err = s.execute("paint.colorReplacement", stroke()).unwrap_err().to_string();
        assert_eq!(err, want, "{mode:?}/{depth:?}");
        assert_eq!(pixels(&s), before, "{mode:?}/{depth:?}: nothing painted");
        assert_eq!(steps(&s), n, "{mode:?}/{depth:?}: no history");
        // The live stroke previews nothing either.
        let live = live_color_replacement(&s, &stroke()).err().map(|e| e.to_string());
        assert_eq!(live.as_deref(), Some(want), "{mode:?}/{depth:?}: live");
    }
}

#[test]
fn full_colour_documents_still_paint() {
    for (mode, depth) in
        [(ColorMode::Rgb, SampleType::U8), (ColorMode::Rgb, SampleType::U16), (ColorMode::Cmyk, SampleType::U8), (ColorMode::Lab, SampleType::U8)]
    {
        let mut s = session(mode, depth);
        let (before, n) = (pixels(&s), steps(&s));
        s.execute("paint.colorReplacement", stroke()).unwrap_or_else(|e| panic!("{mode:?}/{depth:?}: {e}"));
        assert_ne!(pixels(&s), before, "{mode:?}/{depth:?}: painted");
        assert_eq!(steps(&s), n + 1, "{mode:?}/{depth:?}");
        assert!(live_color_replacement(&s, &stroke()).is_ok(), "{mode:?}/{depth:?}: live");
    }
}
