//! #12867: `select.object` refuses Indexed Color and Bitmap documents with the reference
//! application's alert, selecting nothing and adding no history, for a rectangle and a point.
//! RGB, Grayscale, CMYK and Lab at every depth, Multichannel and Duotone keep selecting.

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::Size;

use super::*;

const INDEXED: &str = "Could not complete your request because the tool does not work with Indexed Color images. Please convert the image to RGB or another supported mode before using..";
const BITMAP: &str = "Could not complete your request because the tool does not work with Bitmap images. Please convert the image to RGB or another supported mode before using..";

/// A session with one 64 × 64 dark document of `mode` and `depth` with a light square at
/// 20..44, an object both the rectangle and the point find.
fn session(mode: ColorMode, depth: SampleType) -> Session {
    let mut doc = Document::with_background("t", Size::new(64, 64), mode, depth, Color::rgb(0.1, 0.1, 0.1));
    let bg = doc.layers[0].surface_mut().unwrap();
    let light = photocraft_raster::from_rgba(&bg.format(), [0.9, 0.9, 0.9, 1.0]);
    bg.fill_rect(Rect::new(20, 20, 44, 44), &light);
    let mut s = Session::new();
    s.add_document(doc, None);
    s
}

fn rect() -> Value {
    json!({"rect": [14, 14, 36, 36]})
}

fn point() -> Value {
    json!({"point": [32, 32]})
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

/// The selection's coverage over the document, to compare two selections exactly.
fn selection(s: &Session) -> Option<Vec<f32>> {
    s.active().unwrap().doc.selection.as_ref().map(|m| m.read_region(Rect::new(0, 0, 64, 64)))
}

#[test]
fn indexed_and_bitmap_documents_refuse_with_the_reference_words() {
    for (mode, want) in [(ColorMode::Indexed, INDEXED), (ColorMode::Bitmap, BITMAP)] {
        for params in [rect(), point()] {
            let mut s = session(mode, SampleType::U8);
            s.execute("select.all", json!({})).unwrap();
            let (before, n) = (selection(&s), steps(&s));
            let err = s.execute("select.object", params.clone()).unwrap_err().to_string();
            assert_eq!(err, want, "{mode:?} {params}");
            assert_eq!(selection(&s), before, "{mode:?} {params}: selection unchanged");
            assert_eq!(steps(&s), n, "{mode:?} {params}: no history");
        }
    }
}

#[test]
fn other_modes_keep_selecting() {
    let measured = [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab]
        .into_iter()
        .flat_map(|m| [SampleType::U8, SampleType::U16, SampleType::F32].map(|d| (m, d)));
    // Not measured in the reference: they keep today's behaviour.
    let unmeasured = [(ColorMode::Multichannel, SampleType::U8), (ColorMode::Duotone, SampleType::U8)];
    for (mode, depth) in measured.chain(unmeasured) {
        for params in [rect(), point()] {
            let mut s = session(mode, depth);
            let n = steps(&s);
            s.execute("select.object", params.clone()).unwrap_or_else(|e| panic!("{mode:?}/{depth:?} {params}: {e}"));
            assert!(selection(&s).is_some(), "{mode:?}/{depth:?} {params}: selected");
            assert_eq!(steps(&s), n + 1, "{mode:?}/{depth:?} {params}: one history step");
        }
    }
}
