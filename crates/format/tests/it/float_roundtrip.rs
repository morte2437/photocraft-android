//! `.pcraft` is lossless for f64: every coordinate reloads bit-for-bit.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, Size};

#[test]
fn f64_coordinates_reload_bit_exact() {
    let x = 11.183221215612905_f64;
    let y = 0.1 + 0.2;
    let mut doc = Document::new("Floats", Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::new(
        "Shape",
        LayerContent::Shape(photocraft_doc::ShapeLayer {
            path: photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&[(x, y), (40.0, 10.0), (25.0, 40.0)])]),
            ..Default::default()
        }),
    ));
    doc.guides.horizontal.push(x);
    doc.guides.vertical.push(y);
    doc.ruler_origin = [x, y];
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let back = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(back.guides.horizontal[0].to_bits(), x.to_bits());
    assert_eq!(back.guides.vertical[0].to_bits(), y.to_bits());
    assert_eq!(back.ruler_origin.map(f64::to_bits), [x, y].map(f64::to_bits));
    assert_eq!(back, doc);
}

#[test]
fn a_perspective_ruler_saves_and_a_damaged_one_is_dropped() {
    let mut doc = Document::new("Ruler", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    doc.perspective_ruler = Some(vec![[-1.5, 2.0], [7.25, 2.0]]);
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap().perspective_ruler, doc.perspective_ruler);
    // A point absurdly far from the canvas (the engine would refuse to edit it): dropped.
    doc.perspective_ruler = Some(vec![[0.0, 0.0], [1.0e300, 2.0]]);
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap().perspective_ruler, None);
    // Four points isn't a ruler: the file still opens, without one.
    doc.perspective_ruler = Some(vec![[0.0, 0.0]; 4]);
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap().perspective_ruler, None);
    // Files without one open without one.
    doc.perspective_ruler = None;
    let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap().perspective_ruler, None);
}
