//! Indexed Color and Duotone documents in flat exports.

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{ColorTable, Document, Duotone, DuotoneInk, Size};

fn doc(mode: ColorMode) -> Document {
    let mut d = Document::with_background("m", Size::new(4, 2), ColorMode::Rgb, SampleType::U8, Color::rgba(1.0, 0.0, 0.0, 1.0));
    d.mode = mode;
    d
}

#[test]
fn indexed_png_is_palette_png() {
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: vec![[0, 0, 0], [255, 0, 0]], transparent: None });
    let r = photocraft_io::export(&d, "x.png", &Default::default()).unwrap();
    // IHDR colour type 3 = indexed; a PLTE chunk follows.
    assert_eq!(r.bytes[25], 3);
    assert!(r.bytes.windows(4).any(|w| w == b"PLTE"));
    let back = photocraft_io::import("x.png", &r.bytes).unwrap().document;
    let px = back.layers[0].surface().unwrap().rgba(1, 1);
    assert_eq!(&px[..3], &[1.0, 0.0, 0.0]);
    // Other formats keep the expanded pixels.
    let j = photocraft_io::export(&d, "x.tif", &Default::default()).unwrap();
    assert!(!j.bytes.is_empty());
}

#[test]
fn indexed_png_keeps_duplicate_palette_colors_opaque() {
    // PLTE entries may have identical RGB but different tRNS alpha. RGB-only lookup used to
    // choose the first red entry for both pixels, making the opaque one disappear on save.
    let palette = [[255, 0, 0], [0, 255, 0], [255, 0, 0], [0, 0, 255]];
    for transparent in [0, 2] {
        let png = photocraft_codecs::encode_png_indexed(4, 1, &[0, 1, 2, 3], &palette, Some(transparent), None, None).unwrap();
        let d = photocraft_io::import("in.png", &png).unwrap().document;
        assert_eq!(d.mode, ColorMode::Indexed);
        let before = d.color_table.clone();
        let r = photocraft_io::export(&d, "out.png", &Default::default()).unwrap();
        let table = photocraft_codecs::png_palette(&r.bytes).unwrap().unwrap();
        assert_eq!(table.colors, palette, "palette order must not change");
        assert_eq!(table.transparent, Some(transparent));
        assert_eq!(photocraft_codecs::decode(&r.bytes).unwrap().data(), photocraft_codecs::decode(&png).unwrap().data());
        let back = photocraft_io::import("out.png", &r.bytes).unwrap().document;
        assert_eq!(back.color_table, before);
        assert_eq!(d.color_table, before, "export must not edit the document's table");
        for x in 0..4 {
            assert_eq!(back.layers[0].surface().unwrap().rgba(x, 0)[3], if x == i32::from(transparent) { 0.0 } else { 1.0 });
        }
    }
}

#[test]
fn indexed_png_uses_the_nearest_opaque_color() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for transparent in [0, 1, 2] {
            let mut palette = vec![[0, 0, 255], [255, 128, 0]];
            palette.insert(usize::from(transparent), [255, 0, 0]);
            let mut d = Document::with_background("m", Size::new(1, 1), ColorMode::Rgb, depth, Color::rgba(1.0, 0.0, 0.0, 1.0));
            d.mode = ColorMode::Indexed;
            d.color_table = Some(ColorTable { colors: palette.clone(), transparent: Some(transparent) });
            let r = photocraft_io::export(&d, "out.png", &Default::default()).unwrap();
            let image = photocraft_codecs::decode(&r.bytes).unwrap();
            assert_eq!(image.data(), &[255, 128, 0, 255], "{depth:?}, transparent = {transparent}");
            let table = photocraft_codecs::png_palette(&r.bytes).unwrap().unwrap();
            assert_eq!(table.colors, palette);
            assert_eq!(table.transparent, Some(transparent));
        }
    }
}

#[test]
fn indexed_png_breaks_opaque_color_ties_in_palette_order() {
    // Both opaque entries are equally far from magenta; the transparent entry is an exact
    // match. Filtering transparency must still choose the earlier original opaque entry.
    for opaque in [[[255, 0, 0], [0, 0, 255]], [[0, 0, 255], [255, 0, 0]]] {
        let mut d = Document::with_background("m", Size::new(1, 1), ColorMode::Rgb, SampleType::U8, Color::rgba(1.0, 0.0, 1.0, 1.0));
        d.mode = ColorMode::Indexed;
        d.color_table = Some(ColorTable { colors: vec![[255, 0, 255], opaque[0], opaque[1]], transparent: Some(0) });
        let r = photocraft_io::export(&d, "out.png", &Default::default()).unwrap();
        let image = photocraft_codecs::decode(&r.bytes).unwrap();
        assert_eq!(image.data(), &[opaque[0][0], opaque[0][1], opaque[0][2], 255]);
    }
}

#[test]
fn indexed_png_keeps_the_alpha_threshold() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut d = Document::with_background("m", Size::new(3, 1), ColorMode::Rgb, depth, Color::rgba(1.0, 0.0, 0.0, 1.0));
        d.mode = ColorMode::Indexed;
        d.color_table = Some(ColorTable { colors: vec![[255, 0, 0], [255, 0, 0]], transparent: Some(0) });
        let surface = d.layers[0].surface_mut().unwrap();
        surface.write_pixel(0, 0, &[1.0, 0.0, 0.0, 0.49]);
        surface.write_pixel(1, 0, &[1.0, 0.0, 0.0, 0.5]);
        let r = photocraft_io::export(&d, "out.png", &Default::default()).unwrap();
        assert_eq!(photocraft_codecs::decode(&r.bytes).unwrap().data(), &[255, 0, 0, 0, 255, 0, 0, 255, 255, 0, 0, 255], "{depth:?}");
    }
}

#[test]
fn indexed_png_keeps_the_last_palette_index() {
    let mut palette = vec![[0, 0, 255]; 256];
    palette[0] = [255, 0, 0];
    palette[255] = [255, 0, 0];
    for transparent in [0, 255] {
        let png = photocraft_codecs::encode_png_indexed(2, 1, &[0, 255], &palette, Some(transparent), None, None).unwrap();
        let d = photocraft_io::import("in.png", &png).unwrap().document;
        let r = photocraft_io::export(&d, "out.png", &Default::default()).unwrap();
        assert_eq!(photocraft_codecs::png_palette(&r.bytes).unwrap(), photocraft_codecs::png_palette(&png).unwrap());
        assert_eq!(photocraft_codecs::decode(&r.bytes).unwrap().data(), photocraft_codecs::decode(&png).unwrap().data());
    }
}

#[test]
fn indexed_png_keeps_a_single_transparent_entry() {
    let png = photocraft_codecs::encode_png_indexed(2, 1, &[0, 0], &[[255, 0, 0]], Some(0), None, None).unwrap();
    let d = photocraft_io::import("in.png", &png).unwrap().document;
    let r = photocraft_io::export(&d, "out.png", &Default::default()).unwrap();
    assert_eq!(photocraft_codecs::png_palette(&r.bytes).unwrap(), photocraft_codecs::png_palette(&png).unwrap());
    assert_eq!(photocraft_codecs::decode(&r.bytes).unwrap().data(), photocraft_codecs::decode(&png).unwrap().data());
}

#[test]
fn indexed_png_rejects_opaque_pixels_without_an_opaque_entry() {
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: vec![[255, 0, 0]], transparent: Some(0) });
    let err = photocraft_io::export(&d, "out.png", &Default::default()).unwrap_err();
    assert!(matches!(err, photocraft_io::IoError::Unsupported(_)), "{err}");
    assert!(err.to_string().contains("no opaque entry"), "{err}");
}

#[test]
fn duotone_exports_the_inks_as_rgb() {
    // Mid gray background.
    let mut d = Document::with_background("m", Size::new(4, 2), ColorMode::Grayscale, SampleType::U8, Color::rgba(0.5, 0.5, 0.5, 1.0));
    d.mode = ColorMode::Duotone;
    d.duotone = Some(Duotone { inks: vec![DuotoneInk::new("Black", [0.0; 3]), DuotoneInk::new("Orange", [1.0, 0.5, 0.0])], psd_raw: None });
    let r = photocraft_io::export(&d, "x.png", &Default::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("Duotone")), "{:?}", r.warnings);
    let back = photocraft_io::import("x.png", &r.bytes).unwrap().document;
    let px = back.layers[0].surface().unwrap().rgba(0, 0);
    assert!(px[0] > px[2] + 0.05, "warm: {px:?}");
}

#[test]
fn indexed_png_keeps_the_document_resolution() {
    // #3047: the palette PNG lost its pHYs chunk and reopened at 72 ppi.
    let mut d = doc(ColorMode::Indexed);
    d.resolution_dpi = 300.0;
    d.color_table = Some(ColorTable { colors: vec![[0, 0, 0], [255, 0, 0]], transparent: None });
    let r = photocraft_io::export(&d, "x.png", &Default::default()).unwrap();
    assert_eq!(r.bytes[25], 3, "still a palette PNG");
    let back = photocraft_io::import("x.png", &r.bytes).unwrap().document;
    assert!((back.resolution_dpi - 300.0).abs() < 0.01, "{}", back.resolution_dpi);
}

#[test]
fn indexed_bmp_is_palette_bmp() {
    // The BMP export wrote an Indexed Color document as 24-bit RGB, so reopening it lost the mode (#3869 follow-up).
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: vec![[0, 0, 0], [255, 0, 0], [0, 255, 0]], transparent: None });
    let r = photocraft_io::export(&d, "x.bmp", &Default::default()).unwrap();
    assert_eq!(u16::from_le_bytes([r.bytes[28], r.bytes[29]]), 8, "8 bits per pixel");
    assert!(r.warnings.iter().any(|w| w.contains("palette BMP")), "{:?}", r.warnings);
    let back = photocraft_io::import("x.bmp", &r.bytes).unwrap().document;
    assert_eq!(back.mode, ColorMode::Indexed);
    assert_eq!(back.color_table.as_ref().map(|t| t.colors.clone()), Some(vec![[0, 0, 0], [255, 0, 0], [0, 255, 0]]));
    let px = back.layers[0].surface().unwrap().rgba(1, 1);
    assert_eq!(&px[..3], &[1.0, 0.0, 0.0]);
}

#[test]
fn indexed_bmp_warns_that_it_has_no_transparency() {
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: vec![[0, 0, 0], [255, 0, 0]], transparent: Some(0) });
    let r = photocraft_io::export(&d, "x.bmp", &Default::default()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("no transparency")), "{:?}", r.warnings);
}

#[test]
fn indexed_bmp_exports_when_its_only_palette_entry_is_marked_transparent() {
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: vec![[255, 0, 0]], transparent: Some(0) });

    let r = photocraft_io::export(&d, "x.bmp", &Default::default()).unwrap();

    assert_eq!(u16::from_le_bytes([r.bytes[28], r.bytes[29]]), 8, "8 bits per pixel");
    let back = photocraft_io::import("x.bmp", &r.bytes).unwrap().document;
    assert_eq!(&back.layers[0].surface().unwrap().rgba(1, 1)[..3], &[1.0, 0.0, 0.0]);
}

#[test]
fn indexed_bmp_matches_opaque_pixels_against_the_transparent_palette_entry() {
    let mut d = doc(ColorMode::Indexed);
    d.color_table = Some(ColorTable { colors: vec![[255, 0, 0], [0, 0, 255]], transparent: Some(0) });

    let r = photocraft_io::export(&d, "x.bmp", &Default::default()).unwrap();

    let back = photocraft_io::import("x.bmp", &r.bytes).unwrap().document;
    assert_eq!(&back.layers[0].surface().unwrap().rgba(1, 1)[..3], &[1.0, 0.0, 0.0]);
}

#[test]
fn indexed_bmp_without_a_usable_table_falls_back_to_rgb() {
    // No table (or one past 256 entries) keeps the old 24-bit path rather than failing the save.
    for table in [None, Some(ColorTable { colors: vec![[0; 3]; 257], transparent: None })] {
        let mut d = doc(ColorMode::Indexed);
        d.color_table = table;
        let r = photocraft_io::export(&d, "x.bmp", &Default::default()).unwrap();
        assert_eq!(u16::from_le_bytes([r.bytes[28], r.bytes[29]]), 24);
    }
}
