//! The Crop tool's straight crop of a document whose only artboard covers the whole canvas fits
//! that board to the frame, as the reference application's Crop tool resizes the artboard (#8111).
//! Partial and several boards keep their layout; Image > Crop with a selection and Image > Trim
//! leave boards alone (the reference application's Image > Crop does nothing in an artboard
//! document and its Trim keeps the board's size).

use photocraft_doc::{Artboard, ArtboardBackground, ColorMode, Document, Group, Layer, LayerContent, Rect, SampleType, Size};
use photocraft_engine::Session;
use serde_json::json;

fn board_layer(board: Artboard, children: Vec<Layer>) -> Layer {
    Layer::new("Page", LayerContent::Group(Group { children, expanded: true, artboard: Some(board), frame_stroke: None }))
}

/// A 100 x 80 document with one artboard per rect, each carrying a size preset.
fn fixture(rects: &[Rect]) -> Session {
    let mut doc = Document::new("Synthetic canvas", Size::new(100, 80), ColorMode::Rgb, SampleType::U8);
    for rect in rects {
        let mut board = Artboard::new(*rect);
        board.preset = "original preset".into();
        doc.layers.push(board_layer(board, vec![]));
    }
    let mut session = Session::new();
    session.add_document(doc, None);
    session
}

/// A sole canvas board follows the frame in both crop modes, with exact undo and redo.
#[test]
fn canvas_artboard_follows_the_crop_frame_with_undo_and_redo() {
    for delete in [false, true] {
        let mut s = fixture(&[Rect::new(0, 0, 100, 80)]);
        let before = s.active().unwrap().doc.clone();
        s.execute("image.crop", json!({"x":20,"y":10,"width":60,"height":40,"deleteCroppedPixels":delete})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.size, Size::new(60, 40));
        assert_eq!(doc.artboards()[0].2.rect, doc.bounds(), "delete {delete}");
        assert!(doc.artboards()[0].2.preset.is_empty());
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc.artboards()[0].2.rect, Rect::new(0, 0, 60, 40));
    }
}

/// A crop frame reaching past the canvas grows the canvas board with it.
#[test]
fn extending_crop_grows_the_canvas_artboard() {
    for delete in [false, true] {
        let mut s = fixture(&[Rect::new(0, 0, 100, 80)]);
        s.execute("image.crop", json!({"x":-10,"y":-5,"width":130,"height":95,"deleteCroppedPixels":delete})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.bounds(), Rect::new(0, 0, 130, 95));
        assert_eq!(doc.artboards()[0].2.rect, doc.bounds(), "delete {delete}");
    }
}

/// Independent boards keep their layout and preset when the canvas origin moves.
#[test]
fn crop_preserves_partial_and_multiple_artboard_layouts_and_presets() {
    for rects in [vec![Rect::new(10, 10, 50, 50)], vec![Rect::new(0, 0, 100, 80), Rect::new(60, 10, 90, 50)]] {
        let mut s = fixture(&rects);
        s.execute("image.crop", json!({"x":20,"y":10,"width":60,"height":40,"deleteCroppedPixels":false})).unwrap();
        for ((_, _, board), original) in s.active().unwrap().doc.artboards().iter().zip(rects) {
            assert_eq!(board.rect, original.translate(-20, -10));
            assert_eq!(board.preset, "original preset");
        }
    }
}

/// Image > Crop with a selection and Image > Trim keep the board's size: only the canvas moves.
#[test]
fn selection_crop_and_trim_leave_the_canvas_artboard_alone() {
    let mut s = fixture(&[Rect::new(0, 0, 100, 80)]);
    s.execute("select.rect", json!({"x":20,"y":10,"width":60,"height":40})).unwrap();
    s.execute("image.crop", json!({})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.bounds(), Rect::new(0, 0, 60, 40));
    assert_eq!(doc.artboards()[0].2.rect, Rect::new(-20, -10, 80, 70));
    assert_eq!(doc.artboards()[0].2.preset, "original preset");

    let mut doc = Document::new("Trim", Size::new(100, 80), ColorMode::Rgb, SampleType::U8);
    let mut board = Artboard::new(doc.bounds());
    board.background = ArtboardBackground::Transparent;
    let mut surface = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    surface.write_region(Rect::new(20, 10, 80, 50), &[1.0, 0.0, 0.0, 1.0].repeat(60 * 40));
    doc.layers.push(board_layer(board, vec![Layer::new("Paint", LayerContent::Raster(surface))]));
    let mut s = Session::new();
    s.add_document(doc, None);
    s.execute("image.trim", json!({"basedOn":"transparent"})).unwrap();
    assert_eq!(s.active().unwrap().doc.bounds(), Rect::new(0, 0, 60, 40));
    assert_eq!(s.active().unwrap().doc.artboards()[0].2.rect, Rect::new(-20, -10, 80, 70));
}

/// Resampling to the crop's target size keeps the canvas board fitted to the final canvas.
#[test]
fn crop_target_resampling_keeps_the_canvas_board_fitted() {
    let mut s = fixture(&[Rect::new(0, 0, 100, 80)]);
    s.execute("image.crop", json!({"x":20,"y":10,"width":60,"height":40,"targetWidth":30,"targetHeight":20,"resolution":144})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert_eq!(doc.bounds(), Rect::new(0, 0, 30, 20));
    assert_eq!(doc.artboards()[0].2.rect, doc.bounds());
    assert_eq!(doc.resolution_dpi, 144.0);
}

/// A frame of the board's own size (the whole canvas, or a same-size frame moved past an edge)
/// keeps the board's size preset; a new size drops it.
#[test]
fn same_size_crop_keeps_the_canvas_artboard_preset() {
    for (x, y, width, height, kept) in [(0, 0, 100, 80, true), (-10, 5, 100, 80, true), (0, 0, 100, 79, false), (0, 0, 99, 80, false)] {
        let mut s = fixture(&[Rect::new(0, 0, 100, 80)]);
        s.execute("image.crop", json!({"x":x,"y":y,"width":width,"height":height,"deleteCroppedPixels":false})).unwrap();
        let doc = &s.active().unwrap().doc;
        let board = &doc.artboards()[0].2;
        assert_eq!(board.rect, Rect::new(0, 0, width, height));
        assert_eq!(board.preset == "original preset", kept, "{x},{y} {width}x{height}: {:?}", board.preset);
        assert_eq!(board.preset.is_empty(), !kept);
    }
}

/// A turned crop fits a canvas board to its frame like a straight one, with exact undo.
#[test]
fn turned_crop_fits_the_canvas_artboard() {
    for delete in [false, true] {
        let mut s = fixture(&[Rect::new(0, 0, 100, 80)]);
        let before = s.active().unwrap().doc.clone();
        s.execute("image.crop", json!({"x":20,"y":10,"width":60,"height":40,"angle":10,"deleteCroppedPixels":delete})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.bounds(), Rect::new(0, 0, 60, 40));
        assert_eq!(doc.artboards()[0].2.rect, doc.bounds(), "delete {delete}");
        assert!(doc.artboards()[0].2.preset.is_empty());
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, before);
    }
    // Several boards keep their own layout under a turned crop too.
    let mut s = fixture(&[Rect::new(0, 0, 100, 80), Rect::new(60, 10, 90, 50)]);
    s.execute("image.crop", json!({"x":20,"y":10,"width":60,"height":40,"angle":10,"deleteCroppedPixels":false})).unwrap();
    assert!(s.active().unwrap().doc.artboards().iter().all(|(_, _, b)| b.preset == "original preset" && b.rect.width() != 60));
}
