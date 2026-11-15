//! #13724: Export As writes JPGs with a progressive scan layout (SOF2) at every quality.

use super::*;

/// A 32 x 24 document of solid colour blocks, so JPEG error stays small away from the edges.
fn blocks_doc() -> Document {
    use photocraft_doc::{Color, ColorMode, SampleType, Size};
    use photocraft_geom::Rect;
    let mut doc = Document::with_background("blocks", Size::new(32, 24), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let surf = doc.layers[0].surface_mut().unwrap();
    surf.fill_rect(Rect::new(0, 0, 16, 16), &[1.0, 0.0, 0.0, 1.0]);
    surf.fill_rect(Rect::new(16, 0, 32, 16), &[0.0, 1.0, 0.0, 1.0]);
    surf.fill_rect(Rect::new(0, 16, 16, 24), &[0.0, 0.0, 1.0, 1.0]);
    doc
}

fn export_jpg(doc: &Document, quality: u32) -> Vec<u8> {
    let f = json!({"format": "jpg", "quality": quality, "scale": 100, "embedProfile": false}).as_object().unwrap().clone();
    let out = super::export_document(doc, &f, None, &photocraft_engine::color_cmds::document_profile(doc)).unwrap();
    photocraft_io::export(&out, "x.jpg", &settings(&f).io_options()).unwrap().bytes
}

/// The main frame's marker and component sampling factors, skipping whole segments.
fn frame(bytes: &[u8]) -> (u8, [u8; 3]) {
    assert_eq!(bytes.get(..2), Some([0xff, 0xd8].as_slice()), "JPEG SOI");
    let mut offset = 2;
    loop {
        assert_eq!(bytes.get(offset), Some(&0xff), "marker prefix");
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *bytes.get(offset).expect("marker");
        offset += 1;
        assert!(!matches!(marker, 0xd9 | 0xda), "no frame before scan");
        let len = usize::from(u16::from_be_bytes([bytes[offset], bytes[offset + 1]]));
        let segment = &bytes[offset + 2..offset + len];
        if matches!(marker, 0xc0..=0xc3) {
            assert_eq!(segment[5], 3, "three components");
            return (marker, std::array::from_fn(|i| segment[6 + 3 * i + 1]));
        }
        offset += len;
    }
}

#[test]
fn export_as_jpg_is_progressive_at_every_quality() {
    let doc = blocks_doc();
    for (quality, sampling) in [(85, [0x11; 3]), (20, [0x22, 0x11, 0x11])] {
        let bytes = export_jpg(&doc, quality);
        let (marker, factors) = frame(&bytes);
        assert_eq!(marker, 0xc2, "quality {quality}: SOF2 (progressive), not SOF0");
        assert_eq!(factors, sampling, "quality {quality}: chroma sampling");
    }
}

#[test]
fn progressive_export_decodes_back_to_the_source() {
    let doc = blocks_doc();
    // Every pixel within 2 levels at the dialog default. Quality 20 quantises saturated flat
    // colours and their edges coarsely, so there only the block centres are held to the colour.
    for (quality, tolerance) in [(85, 2), (20, 64)] {
        let decoded =
            photocraft_codecs::decode(&export_jpg(&doc, quality)).unwrap().convert(photocraft_codecs::ChannelLayout::Rgb, photocraft_codecs::SampleType::U8);
        assert_eq!(decoded.dimensions(), (32, 24));
        for (i, px) in decoded.data().as_chunks::<3>().0.iter().enumerate() {
            let (x, y) = (i % 32, i / 32);
            if quality == 20 && !matches!((x, y), (7 | 23, 7) | (7, 20) | (23, 20)) {
                continue;
            }
            let want: [u8; 3] = match (x < 16, y < 16) {
                (true, true) => [255, 0, 0],
                (false, true) => [0, 255, 0],
                (true, false) => [0, 0, 255],
                (false, false) => [255, 255, 255],
            };
            // The block interiors; the colour edges fall on 16 x 16 MCU boundaries.
            for (a, b) in px.iter().zip(want) {
                assert!(a.abs_diff(b) <= tolerance, "quality {quality}, pixel ({x}, {y}): {px:?} vs {want:?}");
            }
        }
    }
}
