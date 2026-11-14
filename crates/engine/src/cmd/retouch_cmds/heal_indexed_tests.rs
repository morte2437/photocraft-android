//! #12112: the Healing Brush refuses an Indexed Color document with the reference application's
//! message, painting nothing and adding no history. RGB, Grayscale, CMYK and Lab at every depth
//! keep healing; Bitmap, Duotone and Multichannel (not measured) don't get the refusal.

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::Size;

use super::*;

const MESSAGE: &str = "Could not set the clone source because it does not work with Indexed Color images (convert image to RGB Color mode to edit).";

/// A 64 × 64 `mode` document of `depth`, mid grey with its left half black: the edge is the
/// texture the stroke heals onto the flat grey.
fn edged(mode: ColorMode, depth: SampleType) -> Session {
    let mut s = Session::new();
    s.add_document(Document::with_background("t", Size::new(64, 64), mode, depth, Color::rgb(0.5, 0.5, 0.5)), None);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 64})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn stroke() -> Value {
    json!({"points": [[48, 32]], "source": [32, 32], "size": 20, "hardness": 100})
}

/// The Background's native samples and the History length.
fn state(s: &Session) -> (Vec<f32>, usize) {
    let st = s.active().unwrap();
    let surf = st.doc.layers[0].surface().unwrap();
    ((0..64).flat_map(|y| (0..64).flat_map(move |x| surf.pixel(x, y))).collect(), st.history.past_len())
}

#[test]
fn an_indexed_document_refuses_with_the_reference_words() {
    let mut s = edged(ColorMode::Indexed, SampleType::U8);
    let before = state(&s);
    let err = s.execute("paint.healingBrush", stroke()).unwrap_err().to_string();
    assert_eq!(err, MESSAGE);
    assert!(state(&s) == before, "nothing painted, no history state");
}

#[test]
fn other_modes_keep_healing_at_every_depth() {
    let depths = [SampleType::U8, SampleType::U16, SampleType::F32];
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in depths {
            let mut s = edged(mode, depth);
            let (pixels, steps) = state(&s);
            s.execute("paint.healingBrush", stroke()).unwrap_or_else(|e| panic!("{mode:?}/{depth:?}: {e}"));
            let after = state(&s);
            assert!(after.0 != pixels, "{mode:?}/{depth:?}: healed");
            assert_eq!(after.1, steps + 1, "{mode:?}/{depth:?}: one history step");
        }
    }
}

#[test]
fn unmeasured_modes_never_get_the_indexed_refusal() {
    for mode in [ColorMode::Bitmap, ColorMode::Duotone, ColorMode::Multichannel] {
        let mut s = edged(mode, SampleType::U8);
        let err = s.execute("paint.healingBrush", stroke()).err().map(|e| e.to_string());
        assert_ne!(err.as_deref(), Some(MESSAGE), "{mode:?}");
    }
}
