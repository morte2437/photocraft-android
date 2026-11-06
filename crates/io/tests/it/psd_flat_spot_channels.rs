//! Flat PSDs (no layer records) with spot and alpha channels. The reference editor writes one when a
//! document is flattened (psd-tools cmyk-alpha-spot.psd, its version 24.7). Its extra channels are
//! all alpha and spot channels: none is the merged transparency, which every extra channel used
//! to be read as first, so the first spot plane vanished and punched a hole in the image.

#[cfg(feature = "corpus")]
use std::path::PathBuf;

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document};
use photocraft_geom::{Rect, Size};
use photocraft_io::{ExportOptions, export, import};
use photocraft_psd::PsdFile;
use photocraft_psd::resources::ImageResource;
use photocraft_raster::Surface;

const W: u32 = 8;

/// Builds a grayscale channel plane at the requested sample depth.
fn plane(depth: SampleType, vals: &[f32]) -> Surface {
    let mut s = Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
    s.write_region(Rect::from_xywh(0, 0, W, 1), vals);
    s
}

/// An opaque document with a spot channel then an alpha channel.
fn flat_doc(mode: ColorMode, depth: SampleType) -> Document {
    let mut doc = Document::with_background("flat", Size::new(W, 1), mode, depth, Color::WHITE);
    let ink = [1.0, 1.0, 0.5, 0.0, 0.0, 0.25, 1.0, 0.0];
    let sel = [0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    doc.channels.push(AlphaChannel { spot: Some((Color::rgb(0.0, 0.6, 0.65), 0.0)), ..AlphaChannel::new("light teal", plane(depth, &ink)) });
    doc.channels.push(AlphaChannel::new("Alpha 1", plane(depth, &sel)));
    doc
}

/// `doc` saved, then rewritten without layer records, as the reference editor saves a flattened document.
fn without_layer_records(doc: &Document) -> PsdFile {
    let mut file = PsdFile::from_bytes(&export(doc, "x.psd", &ExportOptions::default()).unwrap().bytes).unwrap();
    file.layer_info = None;
    file
}

/// Gives the first DisplayInfo record (the spot's, after the 4-byte version) the reference editor's
/// undocumented colour space 0x2331.
fn unknown_spot_space(file: &mut PsdFile) {
    let info = file.resources.iter_mut().find(|r| r.id == 1077).unwrap();
    info.data[4..14].copy_from_slice(b"#15fc83e\0\0");
}

/// Reads the complete synthetic channel row.
fn values(s: &Surface) -> Vec<f32> {
    s.read_region(Rect::from_xywh(0, 0, W, 1))
}

/// Compares channel samples within eight-bit quantization tolerance.
fn assert_close(a: &[f32], b: &[f32], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}");
    for (x, y) in a.iter().zip(b) {
        assert!((x - y).abs() <= 1.0 / 255.0, "{what}: {a:?} vs {b:?}");
    }
}

/// Every extra channel of a flat file comes back, spot first with its ink, and the image stays
/// opaque, whether the names are in 1045 or only in 1006.
#[test]
fn flat_file_keeps_its_spot_and_alpha_channels() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Grayscale] {
        for depth in [SampleType::U8, SampleType::U16] {
            for unicode_names in [true, false] {
                let what = format!("{mode:?} {depth:?} 1045 {unicode_names}");
                let doc = flat_doc(mode, depth);
                let mut file = without_layer_records(&doc);
                if !unicode_names {
                    file.resources.retain(|r| r.id != 1045);
                }
                assert!(!file.merged_has_alpha(), "{what}");
                let opened = import("flat.psd", &file.to_bytes().unwrap()).unwrap().document;
                let names: Vec<_> = opened.channels.iter().map(|c| c.name.as_str()).collect();
                assert_eq!(names, ["light teal", "Alpha 1"], "{what}");
                assert!(opened.channels[0].spot.is_some() && opened.channels[1].spot.is_none(), "{what}");
                assert_close(&values(&opened.channels[0].surface), &values(&doc.channels[0].surface), &format!("{what}: spot ink"));
                assert_close(&values(&opened.channels[1].surface), &values(&doc.channels[1].surface), &format!("{what}: alpha"));
                let flat = photocraft_compose::flatten(&opened);
                assert!(flat.px.iter().all(|p| p[3] == 1.0), "{what}: the spot plane is not the image's transparency: {:?}", flat.px);
            }
        }
    }
}

#[test]
fn flat_extra_channel_names_decode_mac_roman_unless_unicode_is_present() {
    let doc = flat_doc(ColorMode::Rgb, SampleType::U8);
    let mut file = without_layer_records(&doc);
    file.resources.iter_mut().find(|r| r.id == 1006).unwrap().data = b"\x04caf\x8e\x07Alpha 1".to_vec();
    let opened = import("flat.psd", &file.to_bytes().unwrap()).unwrap().document;
    assert_eq!(opened.channels[0].name, "light teal", "Unicode resource takes precedence");
    file.resources.retain(|r| r.id != 1045);
    let opened = import("flat.psd", &file.to_bytes().unwrap()).unwrap().document;
    assert_eq!(opened.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["café", "Alpha 1"]);
    assert!(opened.channels[0].spot.is_some());
}

/// A flat file from a writer that names no channel (a plain RGBA file) still reads its extra
/// channel as the merged transparency, and so does one whose first name is "Transparency".
#[test]
fn flat_file_without_channel_names_keeps_its_transparency() {
    let mut doc = Document::new("rgba", Size::new(W, 1), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(photocraft_doc::Layer::raster("Layer 1", doc.pixel_format()));
    let mut file = without_layer_records(&doc);
    assert_eq!(file.header.channels, 4);
    file.resources.retain(|r| r.id != 1045 && r.id != 1006);
    assert!(file.merged_has_alpha());
    let mut named = file.clone();
    named.resources.push(ImageResource::new(1006, b"\x0cTransparency".to_vec()));
    assert!(named.merged_has_alpha());
    // One name that is not "Transparency" for the one extra channel: an alpha channel.
    let mut alpha = file.clone();
    alpha.resources.push(ImageResource::new(1006, b"\x07Alpha 1".to_vec()));
    assert!(!alpha.merged_has_alpha());
}

/// A spot whose DisplayInfo colour is in a space PhotoCraft doesn't read takes its ink from the
/// Alternate Spot Colors (1067), as in the reference editor's cmyk-alpha-spot.psd (Lab 58.47 /
/// −33.27 / −14.59 for channel id 3).
#[test]
fn unknown_display_space_takes_the_alternate_spot_colour() {
    let doc = flat_doc(ColorMode::Cmyk, SampleType::U8);
    let mut file = without_layer_records(&doc);
    unknown_spot_space(&mut file);
    let ids = file.resources.iter().find(|r| r.id == 1053).unwrap().data.clone();
    let spot_id = u32::from_be_bytes(ids[0..4].try_into().unwrap());
    let mut alt = vec![0, 1, 0, 1];
    alt.extend_from_slice(&spot_id.to_be_bytes());
    alt.extend_from_slice(&[0x00, 0x07, 0x16, 0xd7, 0xf3, 0x01, 0xfa, 0x4d, 0x00, 0x00]);
    let without_alt = import("a.psd", &file.to_bytes().unwrap()).unwrap().document;
    file.resources.push(ImageResource::new(1067, alt));
    let opened = import("b.psd", &file.to_bytes().unwrap()).unwrap().document;
    let (ink, solidity) = opened.channels[0].spot.unwrap();
    let rgb = ink.to_rgb().map(|v| (v * 255.0).round());
    // Lab (58.47, −33.27, −14.59) is about sRGB (0, 157, 165).
    assert!((rgb[0] - 0.0).abs() <= 3.0 && (rgb[1] - 157.0).abs() <= 3.0 && (rgb[2] - 165.0).abs() <= 3.0, "{rgb:?}");
    assert_eq!(solidity, 0.0);
    assert_ne!(without_alt.channels[0].spot.unwrap().0, ink, "without 1067 the record's components are read as RGB");
    // A known DisplayInfo space wins over 1067.
    let mut known = without_layer_records(&doc);
    known.resources.push(ImageResource::new(1067, file.resources.iter().find(|r| r.id == 1067).unwrap().data.clone()));
    assert_eq!(
        import("c.psd", &known.to_bytes().unwrap()).unwrap().document.channels[0].spot,
        import("d.psd", &without_layer_records(&doc).to_bytes().unwrap()).unwrap().document.channels[0].spot
    );
}

/// Malformed 1067 resources (truncated, a count past the end, a hostile count) are ignored.
#[test]
fn malformed_alternate_spot_colors_never_panic() {
    let doc = flat_doc(ColorMode::Rgb, SampleType::U8);
    for data in [vec![], vec![0], vec![0, 1, 0xff, 0xff], vec![0, 1, 0, 3, 0, 0, 0, 3, 0, 7], vec![0, 1, 0, 1, 0, 0, 0, 3, 0, 7, 0, 0, 0, 0, 0, 0, 0]] {
        let mut file = without_layer_records(&doc);
        unknown_spot_space(&mut file);
        file.resources.push(ImageResource::new(1067, data.clone()));
        let opened = import("m.psd", &file.to_bytes().unwrap()).unwrap().document;
        assert_eq!(opened.channels.len(), 2, "{data:?}");
    }
}

/// Duplicate identifiers select the first matching ink, and opaque resource bytes survive
/// both PSD serialization and a document import/export, including unknown colour spaces.
#[test]
fn alternate_resource_duplicates_and_unknown_spaces_round_trip() {
    for depth in [SampleType::U8, SampleType::U16] {
        let mut file = without_layer_records(&flat_doc(ColorMode::Rgb, depth));
        unknown_spot_space(&mut file);
        let ids = &file.resource(1053).unwrap().data;
        let id = &ids[..4];
        let mut data = vec![0, 1, 0, 4];
        // An unrelated green ink must be skipped before the matching red ink.
        let other_id = u32::from_be_bytes(id.try_into().unwrap()).wrapping_add(1);
        data.extend_from_slice(&other_id.to_be_bytes());
        data.extend_from_slice(&[0, 0, 0, 0, 0xff, 0xff, 0, 0, 0, 0]);
        for color in [[0, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0xff, 0xff, 0, 0, 0, 0], [0x23, 0x31, 1, 2, 3, 4, 5, 6, 7, 8]] {
            data.extend_from_slice(id);
            data.extend_from_slice(&color);
        }
        file.resources.push(ImageResource::new(1067, data.clone()));
        let bytes = file.to_bytes().unwrap();
        let parsed = PsdFile::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.resource(1067).unwrap().data, data);
        let opened = import("duplicate.psd", &bytes).unwrap().document;
        assert_eq!(opened.channels.len(), 2);
        assert_eq!(opened.channels[0].spot.unwrap().0.to_rgb(), [1.0, 0.0, 0.0]);
        let saved = export(&opened, "roundtrip.psd", &ExportOptions::default()).unwrap();
        let reparsed = PsdFile::from_bytes(&saved.bytes).unwrap();
        assert_eq!(reparsed.resource(1067).unwrap().data, data);
    }
}

#[cfg(feature = "corpus")]
fn psd_tools(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/psd-tools").join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}: run `cargo xtask corpus --all`", p.display()))
}

/// The reference editor's flat CMYK file (24.7): the "light teal" spot (solid at 1348,1492) and "Alpha 1"
/// both open, and the image stays opaque where the spot is inked.
#[cfg(feature = "corpus")]
#[test]
fn corpus_flat_photoshop_file_keeps_its_spot() {
    let doc = import("cmyk-alpha-spot.psd", &psd_tools("cmyk-alpha-spot.psd")).unwrap().document;
    let names: Vec<_> = doc.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["light teal", "Alpha 1"]);
    let (ink, solidity) = doc.channels[0].spot.unwrap();
    let rgb = ink.to_rgb().map(|v| (v * 255.0).round());
    assert!(rgb[0] <= 3.0 && (rgb[1] - 157.0).abs() <= 3.0 && (rgb[2] - 165.0).abs() <= 3.0, "{rgb:?}");
    assert_eq!(solidity, 0.0);
    assert!(doc.channels[1].spot.is_none());
    let at = Rect::from_xywh(1348, 1492, 1, 1);
    assert_eq!(doc.channels[0].surface.read_region(at), [1.0], "solid ink");
    let p = photocraft_compose::render(&doc, at).px[0];
    assert!(p[3] == 1.0 && p[..3].iter().all(|v| *v > 0.95), "no process ink, opaque: {p:?}");
}

/// psd-tools' flat CMYK file with three extra channels: all three open with their 1006 names.
#[cfg(feature = "corpus")]
#[test]
fn corpus_flat_file_with_pascal_names_keeps_every_channel() {
    let doc = import("cmyk-spot.psd", &psd_tools("cmyk-spot.psd")).unwrap().document;
    let names: Vec<_> = doc.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["blue", "fluorescent pink", "yellow"]);
    let flat = photocraft_compose::flatten(&doc);
    assert!(flat.px.iter().all(|p| p[3] == 1.0), "opaque");
}
