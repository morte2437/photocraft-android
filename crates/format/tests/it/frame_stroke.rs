//! A Photoshop Frame's stroke (`Group::frame_stroke`) survives a `.pcraft` round trip; older
//! manifests, written before it existed, load without one (#4381).

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Fill, Layer, LayerContent, ShapeStroke, Size, StrokeAlign};
use photocraft_format::*;

fn stroke_of(d: &Document) -> Option<ShapeStroke> {
    match &d.layers.first()?.content {
        LayerContent::Group(g) => g.frame_stroke.clone(),
        _ => None,
    }
}

#[test]
fn frame_stroke_roundtrips_and_defaults_when_absent() {
    let mut doc = Document::new("a", Size::new(40, 30), ColorMode::Rgb, SampleType::U8);
    let stroke =
        ShapeStroke { width: 8.0, align: StrokeAlign::Inside, paint: Fill::Solid(Color::rgb(1.0, 0.0, 0.0)), dashes: vec![2.0, 1.0], ..ShapeStroke::default() };
    let mut g = Layer::group("Frame 1", vec![]);
    if let LayerContent::Group(gr) = &mut g.content {
        gr.frame_stroke = Some(stroke.clone());
    }
    doc.layers.push(g);
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(stroke_of(&load_from_bytes(&bytes).unwrap()), Some(stroke));

    // A manifest without the key (older files) loads with no stroke.
    let m = read_manifest(&bytes).unwrap();
    let mut v = serde_json::to_value(&m.document).unwrap();
    let content = &mut v["layers"][0]["content"];
    assert!(content.get("frame_stroke").is_some());
    content.as_object_mut().unwrap().remove("frame_stroke");
    let old: photocraft_format::manifest::DocM = serde_json::from_value(v).unwrap();
    let photocraft_format::manifest::ContentM::Group { frame_stroke, .. } = &old.layers[0].content else { panic!("not a group") };
    assert!(frame_stroke.is_none());
}
