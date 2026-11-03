//! A Photoshop Frame (Frame tool, K) is a layer group whose record carries the frame outline as a
//! vector mask (`vsms`) and its Properties › Stroke as a shape-style `vstk`. The stroke draws
//! along the outline even when nothing is placed in the frame (#4381).

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Fill, Group, Layer, LayerContent, Path, ShapeStroke, Size, StrokeAlign, Subpath, VectorMask};
use photocraft_io::vector_map::{parse_vstk, vstk_bytes};
use photocraft_io::*;
use photocraft_psd::{PsdFile, SectionType, TaggedBlock};

fn red_inside(width: f32) -> ShapeStroke {
    ShapeStroke { width, align: StrokeAlign::Inside, paint: Fill::Solid(Color::rgb(1.0, 0.0, 0.0)), ..ShapeStroke::default() }
}

/// An 80 × 60 white document with an empty rectangular frame over (20, 10)–(60, 50).
fn frame_doc(depth: SampleType, stroke: Option<ShapeStroke>) -> Document {
    let mut d = Document::with_background("f", Size::new(80, 60), ColorMode::Rgb, depth, Color::WHITE);
    let mut g = Layer::group("Frame 1", vec![]);
    if let LayerContent::Group(Group { frame_stroke, .. }) = &mut g.content {
        *frame_stroke = stroke;
    }
    g.vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&[(20.0, 10.0), (60.0, 10.0), (60.0, 50.0), (20.0, 50.0)])])));
    d.layers.push(g);
    d
}

fn frame_stroke(d: &Document) -> Option<ShapeStroke> {
    d.layers.iter().find_map(|l| match &l.content {
        LayerContent::Group(g) => Some(g.frame_stroke.clone()),
        _ => None,
    })?
}

fn px(d: &Document, x: i32, y: i32) -> [f32; 4] {
    photocraft_compose::flatten_to_surface(d, PixelFormat::RGBA8.with_sample(SampleType::F32), None).rgba(x, y)
}

/// The group's own record (its top, `lsct` 1 or 2) in a PSD file.
fn frame_record(f: &mut PsdFile) -> &mut photocraft_psd::LayerRecord {
    f.layers_mut()
        .iter_mut()
        .find(|r| r.section_divider().is_some_and(|s| matches!(s.kind, SectionType::OpenFolder | SectionType::ClosedFolder)))
        .expect("the frame's group record")
}

#[test]
fn a_photoshop_frames_vstk_draws_its_stroke_along_the_outline() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        // A file shaped like Photoshop's: the group record carries `vsms` and a `vstk`.
        let mut f = document_to_psd(&frame_doc(depth, None));
        frame_record(&mut f).blocks.push(TaggedBlock::new(*b"vstk", vstk_bytes(Some(&red_inside(4.0)), true, 72.0)));
        let (d, _) = psd_to_document(&f);
        assert_eq!(frame_stroke(&d), Some(red_inside(4.0)), "{depth:?}");
        // Inside the outline, 4 px wide: red on the frame's edge band, white in the middle and
        // outside it.
        let close = |a: [f32; 4], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 0.02);
        for (x, y) in [(21, 30), (58, 30), (40, 11), (40, 48)] {
            assert!(close(px(&d, x, y), [1.0, 0.0, 0.0]), "{depth:?} ({x}, {y}): {:?}", px(&d, x, y));
        }
        for (x, y) in [(40, 30), (18, 30), (62, 30), (40, 8)] {
            assert!(close(px(&d, x, y), [1.0, 1.0, 1.0]), "{depth:?} ({x}, {y}): {:?}", px(&d, x, y));
        }
    }
}

#[test]
fn a_frame_stroke_round_trips_through_psd() {
    let stroke = ShapeStroke { dashes: vec![2.0, 1.0], opacity: 0.5, align: StrokeAlign::Outside, ..red_inside(3.0) };
    let d = frame_doc(SampleType::U8, Some(stroke.clone()));
    let bytes = export(&d, "f.psd", &ExportOptions::default()).unwrap().bytes;
    let back = import("f.psd", &bytes).unwrap().document;
    assert_eq!(frame_stroke(&back), Some(stroke.clone()));
    // Outside the outline at half opacity: pink just outside the edge.
    let p = px(&back, 18, 30);
    assert!((p[1] - 0.5).abs() < 0.05 && p[0] > 0.98, "{p:?}");

    // Saved again unchanged, the imported `vstk` stays byte for byte.
    let mut f = PsdFile::from_bytes(&bytes).unwrap();
    let first = frame_record(&mut f).block(b"vstk").unwrap().data.clone();
    let again = export(&back, "f.psd", &ExportOptions::default()).unwrap().bytes;
    let mut f2 = PsdFile::from_bytes(&again).unwrap();
    assert_eq!(frame_record(&mut f2).block(b"vstk").unwrap().data, first);

    // Removing the stroke writes it disabled; no stroke at all writes no `vstk`.
    let mut cleared = back.clone();
    if let Some(LayerContent::Group(g)) = cleared.layers.iter_mut().map(|l| &mut l.content).find(|c| matches!(c, LayerContent::Group(_))) {
        g.frame_stroke = None;
    }
    let out = export(&cleared, "f.psd", &ExportOptions::default()).unwrap().bytes;
    let mut f3 = PsdFile::from_bytes(&out).unwrap();
    assert_eq!(parse_vstk(&frame_record(&mut f3).block(b"vstk").unwrap().data, 72.0).unwrap().stroke, None);
    let plain = export(&frame_doc(SampleType::U8, None), "f.psd", &ExportOptions::default()).unwrap().bytes;
    let mut f4 = PsdFile::from_bytes(&plain).unwrap();
    assert!(frame_record(&mut f4).block(b"vstk").is_none());
}

#[test]
fn a_frame_stroke_without_a_vector_mask_or_width_draws_nothing() {
    let mut d = frame_doc(SampleType::U8, Some(red_inside(4.0)));
    d.layers[1].vector_mask = None;
    assert_eq!(px(&d, 21, 30), [1.0, 1.0, 1.0, 1.0]);
    let d = frame_doc(SampleType::U8, Some(ShapeStroke { width: 0.0, ..red_inside(4.0) }));
    assert_eq!(px(&d, 21, 30), [1.0, 1.0, 1.0, 1.0]);
    // Hostile widths from a file render without panicking.
    for width in [f32::NAN, f32::INFINITY, -5.0, 1e9] {
        let _ = px(&frame_doc(SampleType::U8, Some(ShapeStroke { width, ..red_inside(4.0) })), 21, 30);
    }
}

#[test]
fn an_isolated_frame_draws_its_outside_stroke_past_the_outline() {
    let mut d = frame_doc(SampleType::U8, Some(ShapeStroke { align: StrokeAlign::Outside, ..red_inside(3.0) }));
    d.layers[1].blend = photocraft_color::BlendMode::Normal;
    let p = px(&d, 18, 30);
    assert!(p[0] > 0.98 && p[1] < 0.02, "{p:?}");
    assert!(px(&d, 22, 30)[1] > 0.98, "inside stays white");
}
