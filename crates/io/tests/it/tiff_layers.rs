//! Layered TIFF (Photoshop layer data in tags 37724 / 34377): Document → TIFF → Document round
//! trips in every mode and depth, equivalence with the PSD path, the flat fall-backs, files
//! written by an independent implementation (psdtags, both byte orders), and malformed input.

use crate::common;

use common::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LayerContent};
use photocraft_io::*;
use photocraft_psd::tiff::{ByteOrder, ImageSourceData};

fn tiff(doc: &Document, layers: bool) -> ExportResult {
    export(doc, "x.tif", &ExportOptions { tiff_layers: layers, ..Default::default() }).expect("export")
}

/// The two Photoshop tags of a TIFF: (resources, layers).
fn tags(bytes: &[u8]) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let (r, l) = photocraft_codecs::tiff_photoshop_tags(bytes);
    (r.map(<[u8]>::to_vec), l.map(<[u8]>::to_vec))
}

fn roundtrip(doc: &Document) -> Document {
    let r = tiff(doc, true);
    let (resources, layers) = tags(&r.bytes);
    let layers = layers.expect("layer data");
    assert!(resources.is_some_and(|r| !r.is_empty()), "image resources");
    // The layer data matches the container's byte order and is byte-stable through the psd crate.
    let order = if photocraft_codecs::tiff_writes_little_endian() { ByteOrder::Little } else { ByteOrder::Big };
    let (isd, w) = ImageSourceData::from_bytes(&layers).expect("parse layer data");
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(isd.byte_order, order);
    assert_eq!(isd.to_bytes(order).unwrap().0, layers);
    // The TIFF is a plain image to any other reader: the composite at the document's size.
    let flat = photocraft_codecs::decode(&r.bytes).expect("flat decode");
    assert_eq!(flat.dimensions(), (doc.size.width, doc.size.height));
    // Extra alpha channels and a Quick Mask travel as TIFF extra samples (at most MAX_EXTRA_SAMPLES).
    let extra = doc.channels.len() + usize::from(doc.quick_mask.is_some());
    assert!(!r.warnings.iter().any(|w| w.contains("not written")), "{:?}", r.warnings);
    if extra > 0 && extra <= photocraft_codecs::MAX_EXTRA_SAMPLES {
        assert_eq!(flat.meta.extra_samples.len(), extra, "{:?}", r.warnings);
    }
    import("x.tif", &r.bytes).expect("import").document
}

/// `doc` as a layered TIFF round trip returns it, including extra channels.
fn expected(doc: &Document) -> Document {
    doc.clone()
}

macro_rules! rt_case {
    ($name:ident, $mode:expr, $depth:expr, $f:expr) => {
        #[test]
        fn $name() {
            let d = gen_doc($mode, $depth, $f);
            let back = roundtrip(&d);
            assert_docs_eq(&expected(&d), &back);
        }
    };
}

rt_case!(rt_rgb8_all, ColorMode::Rgb, SampleType::U8, Features::ALL);
rt_case!(rt_rgb16_all, ColorMode::Rgb, SampleType::U16, Features::ALL);
rt_case!(rt_rgb32_all, ColorMode::Rgb, SampleType::F32, Features::ALL);
rt_case!(rt_gray8_all, ColorMode::Grayscale, SampleType::U8, Features::ALL);
rt_case!(rt_gray16_all, ColorMode::Grayscale, SampleType::U16, Features::ALL);
rt_case!(rt_gray32_all, ColorMode::Grayscale, SampleType::F32, Features::ALL);
rt_case!(rt_cmyk8_all, ColorMode::Cmyk, SampleType::U8, Features::ALL);
rt_case!(rt_cmyk16_all, ColorMode::Cmyk, SampleType::U16, Features::ALL);
rt_case!(rt_rgb8_pixels, ColorMode::Rgb, SampleType::U8, Features::PIXELS);
rt_case!(rt_gray16_pixels, ColorMode::Grayscale, SampleType::U16, Features::PIXELS);
rt_case!(rt_cmyk8_pixels, ColorMode::Cmyk, SampleType::U8, Features::PIXELS);

#[test]
fn tiff_and_psd_open_as_the_same_document() {
    for (mode, depth) in [(ColorMode::Rgb, SampleType::U8), (ColorMode::Cmyk, SampleType::U16), (ColorMode::Grayscale, SampleType::F32)] {
        let d = gen_doc(mode, depth, Features::ALL);
        let psd = export(&d, "x.psd", &ExportOptions::default()).unwrap();
        let from_psd = import("x.psd", &psd.bytes).unwrap().document;
        let from_tif = roundtrip(&d);
        assert_docs_eq(&expected(&from_psd), &from_tif);
        // The composite the TIFF carries is what the document looks like.
        let tif = tiff(&d, true);
        let flat = import(
            "flat.tif",
            &photocraft_codecs::encode(&photocraft_codecs::decode(&tif.bytes).unwrap(), photocraft_codecs::Format::Tiff, &Default::default()).unwrap(),
        )
        .unwrap()
        .document;
        let a = photocraft_compose::flatten(&d).px;
        let b = photocraft_compose::flatten(&flat).px;
        assert!(max_diff(&a, &b) <= 2.0 / 255.0, "{mode:?} {depth:?}: composite differs by {}", max_diff(&a, &b));
    }
}

#[test]
fn every_compression_keeps_the_layers() {
    use photocraft_codecs::TiffCompression;
    let d = gen_doc(ColorMode::Rgb, SampleType::U16, Features::PIXELS);
    for c in [TiffCompression::None, TiffCompression::Lzw, TiffCompression::Deflate, TiffCompression::PackBits] {
        let mut opts = ExportOptions { tiff_layers: true, ..Default::default() };
        opts.encode.tiff_compression = c;
        let r = export(&d, "x.tiff", &opts).unwrap();
        let back = import("x.tiff", &r.bytes).unwrap().document;
        assert_docs_eq(&d, &back);
    }
}

/// The Artist tag's text, and the bytes every EXIF assertion looks for.
const ARTIST: &[u8] = b"fixture-artist";

/// A minimal little-endian EXIF (TIFF) block with a single Artist tag and no Orientation, so
/// `upright_exif` passes it through untouched.
fn exif() -> Vec<u8> {
    let mut artist = ARTIST.to_vec();
    artist.push(0); // ASCII NUL terminator
    let mut v = b"II*\0\x08\0\0\0".to_vec(); // TIFF header, first IFD at 8
    v.extend_from_slice(&1u16.to_le_bytes()); // one entry
    v.extend_from_slice(&0x013bu16.to_le_bytes()); // Artist
    v.extend_from_slice(&2u16.to_le_bytes()); // ASCII
    v.extend_from_slice(&(artist.len() as u32).to_le_bytes());
    v.extend_from_slice(&26u32.to_le_bytes()); // value offset: 8 + 2 + 12 + 4
    v.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
    v.extend_from_slice(&artist);
    v
}

/// #1545: a layered TIFF carries EXIF as Photoshop resource 1058 and reads it back, so it must
/// not report a loss it never makes. A flat TIFF writes it as its own tags and keeps it too.
#[test]
fn layered_tiff_keeps_exif_without_claiming_a_loss() {
    let mut d = gen_doc(ColorMode::Rgb, SampleType::U8, Features::PIXELS);
    d.metadata.exif = Some(std::sync::Arc::new(exif()));

    let r = tiff(&d, true);
    assert!(!r.warnings.iter().any(|w| w.contains("EXIF")), "a layered TIFF keeps EXIF, so it must not claim to drop it: {:?}", r.warnings);
    // ...and the tags really are in the file, so the missing warning is a true absence of loss.
    let back = import("x.tif", &r.bytes).unwrap().document;
    let kept = back.metadata.exif.as_deref().expect("EXIF survives the layered export");
    assert!(kept.windows(ARTIST.len()).any(|w| w == ARTIST), "the Artist tag did not survive: {kept:?}");

    // The flat path writes the EXIF block's tags as the TIFF's own (here IFD0's Artist, which
    // reads back as the document's Artist text), so it reports no loss either.
    let flat = tiff(&d, false);
    assert!(!flat.warnings.iter().any(|w| w.contains("EXIF")), "a flat TIFF keeps EXIF: {:?}", flat.warnings);
    let back = import("x.tif", &flat.bytes).unwrap().document;
    let artist = std::str::from_utf8(ARTIST).unwrap().trim_end_matches('\0');
    assert!(back.metadata.text.iter().any(|(k, v)| k == "Artist" && v == artist), "{:?}", back.metadata.text);
}

#[test]
fn discard_layers_writes_a_flat_file() {
    let d = gen_doc(ColorMode::Rgb, SampleType::U8, Features::PIXELS);
    let r = tiff(&d, false);
    assert_eq!(tags(&r.bytes), (None, None));
    assert!(r.warnings.iter().any(|w| w.contains("flattened")), "{:?}", r.warnings);
    let back = import("x.tif", &r.bytes).unwrap().document;
    assert_eq!(back.layers.len(), 1);
    assert!(!tiff_layers::would_write_layers(&back));
}

#[test]
fn a_plain_background_needs_no_layer_data() {
    let d = import("x.png", &export(&gen_doc(ColorMode::Rgb, SampleType::U8, Features::PIXELS), "x.png", &Default::default()).unwrap().bytes).unwrap().document;
    assert_eq!(d.layers.len(), 1);
    assert!(!tiff_layers::would_write_layers(&d));
    let r = tiff(&d, true);
    assert_eq!(tags(&r.bytes), (None, None));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let back = import("x.tif", &r.bytes).unwrap().document;
    assert_docs_eq(&d, &back);
}

#[test]
fn lab_documents_are_saved_flat_with_a_warning() {
    let d = gen_doc(ColorMode::Lab, SampleType::U8, Features::PIXELS);
    let r = tiff(&d, true);
    assert_eq!(tags(&r.bytes), (None, None));
    assert!(r.warnings.iter().any(|w| w.contains("Lab") && w.contains("layers are not kept")), "{:?}", r.warnings);
    assert!(import("x.tif", &r.bytes).is_ok());
}

#[test]
fn transparency_is_straight_in_the_composite() {
    // A translucent layer over nothing: the TIFF's composite keeps its colour under the alpha
    // instead of being matted against white like a PSD's merged image.
    let mut d = Document::new("t", photocraft_geom::Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    let fmt = d.pixel_format();
    let mut l = raster("Red", fmt, photocraft_geom::Rect::new(0, 0, 4, 4), 0, false);
    if let LayerContent::Raster(s) = &mut l.content {
        let px: Vec<f32> = (0..16).flat_map(|_| [1.0, 0.0, 0.0, 0.5]).collect();
        s.write_region(photocraft_geom::Rect::new(0, 0, 4, 4), &px);
    }
    d.layers.push(l);
    let r = tiff(&d, true);
    let flat = photocraft_codecs::decode(&r.bytes).unwrap();
    assert!(flat.layout().has_alpha());
    let px = flat.data();
    assert_eq!(&px[..4], &[255, 0, 0, 128]);
    let back = import("x.tif", &r.bytes).unwrap().document;
    assert_docs_eq(&d, &back);
}

/// Photoshop (27.11) saves a layered TIFF with Save Transparency as a composite with a
/// transparency sample, a positive layer count and an `MTrn` block first in the layer data
/// (the specification's key is `Mtrn`; `nrTM` in an `II` file). That sample is the merged
/// transparency: it opens as no alpha channel, and a PSD save doesn't write it twice.
#[test]
fn photoshop_mtrn_marks_the_merged_transparency_of_a_layered_tiff() {
    use photocraft_psd::{PsdFile, TaggedBlock};
    for depth in [SampleType::U8, SampleType::U16] {
        let mut d = Document::new("t", photocraft_geom::Size::new(8, 6), ColorMode::Rgb, depth);
        let fmt = d.pixel_format();
        d.layers.push(raster("shape", fmt, photocraft_geom::Rect::new(1, 1, 5, 4), 3, true));
        let r = tiff(&d, true);
        let mut img = photocraft_codecs::decode(&r.bytes).expect("decode");
        assert!(img.layout().has_alpha(), "{depth:?}: the composite has a transparency sample");
        let (isd, _) = ImageSourceData::from_bytes(&tags(&r.bytes).1.expect("layer data")).expect("parse");
        for order in [ByteOrder::Little, ByteOrder::Big] {
            let case = format!("{depth:?} {order:?}");
            // Photoshop's form of the same layer data.
            let mut ps = isd.clone();
            ps.layer_info.as_mut().expect("layers").merged_alpha = false;
            ps.global_blocks.insert(0, TaggedBlock::new(*b"MTrn", Vec::new()));
            let layers = ps.to_bytes(order).expect("layer data").0;
            if order == ByteOrder::Little {
                assert!(layers.windows(12).any(|w| w == b"MIB8nrTM\0\0\0\0"), "{case}: stored as Photoshop stores it");
            }
            img.meta.photoshop_layers = Some(layers);
            let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Tiff, &photocraft_codecs::EncodeOptions::default()).expect("encode");

            let opened = import("x.tif", &bytes).expect("import").document;
            let names: Vec<&str> = opened.channels.iter().map(|c| c.name.as_str()).collect();
            assert!(names.is_empty(), "{case}: no alpha channel from the merged transparency: {names:?}");
            // The marker is kept for the next save.
            let mut want = d.clone();
            want.metadata.psd_global_blocks = vec![(*b"8BIM", *b"MTrn", std::sync::Arc::new(Vec::new()))];
            assert_docs_eq(&want, &opened);
            let psd = PsdFile::from_bytes(&export(&opened, "x.psd", &ExportOptions::default()).expect("PSD save").bytes).expect("parse");
            assert_eq!(psd.header.channels, 4, "{case}: colour + merged transparency");
        }
    }
}

#[test]
fn big_documents_use_the_psb_signature() {
    // Forcing PSB stands in for a > 30000 px document.
    let d = gen_doc(ColorMode::Rgb, SampleType::U8, Features::PIXELS);
    let r = export(&d, "x.tif", &ExportOptions { force_psb: true, tiff_layers: true, ..Default::default() }).unwrap();
    let (_, layers) = tags(&r.bytes);
    assert!(layers.unwrap().starts_with(photocraft_psd::tiff::SIGNATURE_PSB));
    let back = import("x.tif", &r.bytes).unwrap().document;
    assert_docs_eq(&d, &back);
}

/// Files written by psdtags (`scripts/layered_tiff_fixtures.py`): two RGB layers, "Background"
/// (RLE) and an offset, translucent "Upper é" (ZIP, Multiply, opacity 200), in both byte orders
/// at 8 and 16 bits. Binary files stay out of this repository: run the script to write them
/// to `corpus/layered-tiff/` (gitignored); without them these checks are skipped.
fn fixture(name: &str) -> Option<Vec<u8>> {
    let bytes = std::fs::read(format!("{}/../../corpus/layered-tiff/{name}", env!("CARGO_MANIFEST_DIR"))).ok();
    if bytes.is_none() {
        eprintln!("skipped {name}: run `python scripts/layered_tiff_fixtures.py` to write corpus/layered-tiff/");
    }
    bytes
}

#[test]
fn psdtags_fixtures_open_with_their_layers() {
    for (name, depth) in [
        ("layered-le-8bit.tif", SampleType::U8),
        ("layered-be-8bit.tif", SampleType::U8),
        ("layered-le-16bit.tif", SampleType::U16),
        ("layered-be-16bit.tif", SampleType::U16),
    ] {
        let Some(bytes) = fixture(name) else { continue };
        let r = import(name, &bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let d = &r.document;
        assert_eq!(d.size, photocraft_geom::Size::new(6, 4), "{name}");
        assert_eq!(d.mode, ColorMode::Rgb);
        assert_eq!(d.depth, depth, "{name}");
        let names: Vec<&str> = d.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Background", "Upper \u{e9}"], "{name}");
        let upper = &d.layers[1];
        assert_eq!(upper.blend, photocraft_color::BlendMode::Multiply);
        assert!((upper.opacity - 200.0 / 255.0).abs() < 1e-6);
        let LayerContent::Raster(s) = &upper.content else { panic!("raster") };
        let bounds = s.content_bounds();
        assert_eq!((bounds.x0, bounds.y0, bounds.width(), bounds.height()), (1, 1, 4, 3), "{name}");
        // Pixel (x=2, y=0) of the upper layer: the generator's ramp, alpha 160/255.
        let px = s.pixel(3, 1);
        let expect = |v: u32| (v % 256) as f32 / 255.0;
        assert!((px[0] - expect(2 * 40 + 7)).abs() < 1e-3, "{name}: {px:?}");
        assert!((px[1] - expect(7 * 3)).abs() < 1e-3, "{name}: {px:?}");
        assert!((px[3] - 160.0 / 255.0).abs() < 1e-3, "{name}: {px:?}");
        let LayerContent::Raster(bg) = &d.layers[0].content else { panic!("raster") };
        let b = bg.pixel(5, 3);
        assert!((b[0] - expect(5 * 40 + 1)).abs() < 1e-3, "{name}: {b:?}");
        assert!((b[3] - 1.0).abs() < 1e-6);
        assert_eq!(d.resolution_dpi, 300.0);
        // tifffile writes a Software tag; nothing else is worth a warning.
        assert!(r.warnings.iter().all(|w| w.contains("text metadata")), "{name}: {:?}", r.warnings);
        // psdtags wrote the composite's alpha sample without the transparency flag, so by the
        // PSD rules it is an alpha channel. A re-save keeps it as a TIFF extra sample.
        assert_eq!(d.channels.len(), 1, "{name}");
        let out = tiff(d, true);
        let back = import(name, &out.bytes).unwrap().document;
        assert_docs_eq(d, &back);
    }
}

/// Advanced Blending (`knko`, `infx`, `clbl`, `tsly`, `lmgm`, `vmgm`), fill opacity (`iOpa`),
/// channel restrictions (`brst`) and Blend If (the records' blending ranges) survive a layered
/// TIFF with its layer data in either byte order: the encoder's own, and the other one made by
/// transcoding the tag (the same length, spliced in place).
#[test]
fn advanced_blending_survives_layered_tiff_in_both_byte_orders() {
    use photocraft_doc::{AdvancedBlending, BlendRange, Knockout};
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut d = gen_doc(ColorMode::Rgb, depth, Features::PIXELS);
        let adv = AdvancedBlending {
            knockout: Knockout::Deep,
            blend_interior: true,
            blend_clipped: false,
            transparency_shapes: false,
            layer_mask_hides_effects: true,
            vector_mask_hides_effects: true,
        };
        d.layers[1].advanced = adv;
        d.layers[1].fill_opacity = 64.0 / 255.0;
        d.layers[1].excluded_channels = 0b010;
        d.layers[1].blend_if.set(0, [BlendRange { black: [10, 40], white: [200, 250] }, BlendRange { black: [0, 0], white: [128, 128] }]);
        d.layers[2].advanced.knockout = Knockout::Shallow;
        let r = tiff(&d, true);
        let (_, layers) = tags(&r.bytes);
        let layers = layers.expect("layer data");
        let (isd, w) = ImageSourceData::from_bytes(&layers).expect("parse");
        assert!(w.is_empty(), "{w:?}");
        let native = isd.byte_order;
        let other = if native == ByteOrder::Little { ByteOrder::Big } else { ByteOrder::Little };
        let (swapped, w) = isd.to_bytes(other).expect("transcode");
        assert!(w.is_empty(), "{depth:?}: blocks dropped while transcoding: {w:?}");
        assert_eq!(swapped.len(), layers.len());
        let at = r.bytes.windows(layers.len()).position(|x| x == &layers[..]).expect("tag data in the file");
        let mut flipped = r.bytes.clone();
        flipped[at..at + layers.len()].copy_from_slice(&swapped);
        for (order, bytes) in [(native, &r.bytes), (other, &flipped)] {
            let back = import("x.tif", bytes).unwrap_or_else(|e| panic!("{depth:?} {order:?}: {e}")).document;
            let l = &back.layers[1];
            assert_eq!(l.advanced, adv, "{depth:?} {order:?}");
            assert_eq!(l.fill_opacity, 64.0 / 255.0, "{depth:?} {order:?}");
            assert_eq!(l.excluded_channels, 0b010, "{depth:?} {order:?}");
            assert_eq!(l.blend_if, d.layers[1].blend_if, "{depth:?} {order:?}");
            assert_eq!(back.layers[2].advanced.knockout, Knockout::Shallow, "{depth:?} {order:?}");
            assert_docs_eq(&expected(&d), &back);
        }
    }
}

#[test]
fn damaged_layer_data_opens_flattened() {
    let d = gen_doc(ColorMode::Rgb, SampleType::U8, Features::PIXELS);
    let r = tiff(&d, true);
    let (_, layers) = tags(&r.bytes);
    let layers = layers.unwrap();
    let at = r.bytes.windows(layers.len()).position(|w| w == &layers[..]).unwrap();
    // Garbage after the signature: no layers, a warning, the composite as one layer.
    let mut bad = r.bytes.clone();
    for b in &mut bad[at + 36..at + 36 + 24] {
        *b ^= 0xa5;
    }
    let r2 = import("x.tif", &bad).unwrap();
    assert_eq!(r2.document.layers.len(), 1);
    assert!(r2.warnings.iter().any(|w| w.contains("could not be read")), "{:?}", r2.warnings);
    // Truncations and bit flips through the whole file: never a panic. A run takes a quarter of
    // them, a different quarter each day.
    let sweep = photocraft_testkit::Sweep::new(4);
    let mut case = 0;
    let mut runs = || {
        case += 1;
        sweep.includes(case)
    };
    for cut in (0..r.bytes.len()).step_by(11).filter(|_| runs()) {
        let _ = import("x.tif", &r.bytes[..cut]);
    }
    for i in (0..r.bytes.len()).step_by(29).filter(|_| runs()) {
        let mut b = r.bytes.clone();
        b[i] ^= 0x5a;
        let _ = import("x.tif", &b);
    }
    for name in ["layered-le-8bit.tif", "layered-be-16bit.tif"] {
        let Some(bytes) = fixture(name) else { continue };
        for cut in (0..bytes.len()).step_by(7).filter(|_| runs()) {
            let _ = import(name, &bytes[..cut]);
        }
        for i in (0..bytes.len()).step_by(5).filter(|_| runs()) {
            let mut b = bytes.clone();
            b[i] ^= 0xff;
            let _ = import(name, &b);
        }
    }
}

/// Every sample of a TIFF, read by the `tiff` crate rather than PhotoCraft's decoder.
///
/// That decoder drops unspecified extra samples when the photometric interpretation is RGB, so
/// the file is retagged MinIsBlack first. The result is then a multiband image of every sample,
/// which is what libtiff's `TIFFReadScanline` returns and what a RIP walks.
fn independent_samples(bytes: &[u8]) -> (u16, Vec<u16>, Vec<u16>, u32, Vec<u8>) {
    let mut tags = tiff::decoder::Decoder::new(std::io::Cursor::new(bytes)).expect("tiff tags");
    let spp: u16 = tags.get_tag_unsigned(tiff::tags::Tag::SamplesPerPixel).expect("spp");
    let bps = tags.get_tag_u16_vec(tiff::tags::Tag::BitsPerSample).expect("bps");
    let extra = tags.get_tag_u16_vec(tiff::tags::Tag::ExtraSamples).unwrap_or_default();
    let rows = tags.find_tag_unsigned::<u32>(tiff::tags::Tag::RowsPerStrip).ok().flatten().unwrap_or(0);
    let patched = photometric_min_is_black(bytes);
    let mut dec = tiff::decoder::Decoder::new(std::io::Cursor::new(patched.as_slice())).expect("tiff decode");
    let tiff::decoder::DecodingResult::U8(samples) = dec.read_image().expect("read image") else {
        panic!("expected 8-bit samples");
    };
    (spp, bps, extra, rows, samples)
}

/// Classic-TIFF photometric tag 262 → 1 (MinIsBlack), value stored inline in the IFD entry.
fn photometric_min_is_black(src: &[u8]) -> Vec<u8> {
    let mut bytes = src.to_vec();
    let le = bytes.first().copied() == Some(b'I');
    let ru16 = |b: &[u8], i: usize| -> Option<u16> {
        let s = b.get(i..i + 2)?;
        Some(if le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
    };
    let ru32 = |b: &[u8], i: usize| -> Option<u32> {
        let s = b.get(i..i + 4)?;
        Some(if le { u32::from_le_bytes(s.try_into().ok()?) } else { u32::from_be_bytes(s.try_into().ok()?) })
    };
    let Some(ifd) = ru32(&bytes, 4) else { return bytes };
    let Some(n) = ru16(&bytes, ifd as usize) else { return bytes };
    for e in 0..n as usize {
        let at = ifd as usize + 2 + e * 12;
        if ru16(&bytes, at) == Some(262) {
            let value = if le { 1u16.to_le_bytes() } else { 1u16.to_be_bytes() };
            if let Some(slot) = bytes.get_mut(at + 8..at + 10) {
                slot.copy_from_slice(&value);
            }
            break;
        }
    }
    bytes
}

fn assert_plane(samples: &[u8], spp: usize, plane: usize, expected: &[u8]) {
    assert_eq!(samples.len(), expected.len().saturating_mul(spp));
    let mut bad = 0usize;
    let mut first = None;
    for (i, want) in expected.iter().enumerate() {
        let got = samples.get(i * spp + plane).copied();
        if got != Some(*want) {
            bad += 1;
            first.get_or_insert((i, got));
        }
    }
    assert_eq!(bad, 0, "{bad} mismatches, first {first:?}");
}

/// A multi-layer transparent document, large enough for several TIFF strips and several
/// raster tiles, with a spot whose every pixel is a known value.
#[test]
fn dtf_like_spot_is_written_in_full() {
    use photocraft_color::{Color, PixelFormat};
    use photocraft_doc::{AlphaChannel, Layer};
    use photocraft_geom::{Rect, Size};
    use photocraft_psd::tiff::resources_from_bytes;
    use photocraft_raster::Surface;

    let (w, h) = (900u32, 360u32);
    let mut doc = Document::new("dtf", Size::new(w, h), ColorMode::Rgb, SampleType::U8);
    let fmt = doc.pixel_format();
    let mut art = Surface::new(fmt);
    let px = photocraft_raster::from_rgba(&fmt, [0.2, 0.5, 0.9, 1.0]);
    art.fill_rect(Rect::new(30, 20, 870, 340), &px);
    doc.layers.push(Layer::new("Art", LayerContent::Raster(art)));
    let mut ink = Surface::new(fmt);
    let px = photocraft_raster::from_rgba(&fmt, [0.9, 0.1, 0.2, 0.5]);
    ink.fill_rect(Rect::new(80, 40, 700, 300), &px);
    let mut layer = Layer::new("Ink", LayerContent::Raster(ink));
    layer.opacity = 0.8;
    doc.layers.push(layer);

    let n = (w as usize) * (h as usize);
    let mut spot_bytes = vec![0u8; n];
    for y in 0..h as usize {
        for x in 0..w as usize {
            spot_bytes[y * w as usize + x] = ((x * 3 + y * 17) % 251) as u8;
        }
    }
    // Outside the artwork the composite is transparent, but the spot is not zero: a reader that
    // binds "Spot Color 1" to the alpha plane fails here.
    assert_eq!(spot_bytes[10 * w as usize + 10], 200);
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let spot_surface = Surface::from_interleaved(gray, Rect::new(0, 0, w as i32, h as i32), &spot_bytes);
    let mut channel = AlphaChannel::new("Spot Color 1", spot_surface);
    channel.spot = Some((Color::rgb(1.0, 0.0, 0.0), 0.6));
    doc.channels.push(channel);

    let mut seen = Vec::new();
    for (name, layers) in [("flat", false), ("layered", true)] {
        let opts = ExportOptions { tiff_layers: layers, ..ExportOptions::default() };
        let r = export(&doc, "dtf.tif", &opts).unwrap();
        assert!(!r.warnings.iter().any(|w| w.contains("not written")), "{name}: {:?}", r.warnings);
        let img = photocraft_codecs::decode(&r.bytes).unwrap();
        assert!(!img.layout().has_alpha(), "{name}: {:?}", img.layout());
        assert_eq!(img.layout().color_channels(), 3, "{name}");
        assert_eq!(img.meta.extra_samples.len(), 1, "{name}");
        assert_eq!(img.meta.extra_samples[0].kind, 0, "{name}");
        // Photoshop stores spot ink inverted: document 255 (solid) is file 0.
        let stored_spot: Vec<u8> = spot_bytes.iter().map(|b| 255 - b).collect();
        assert_eq!(img.meta.extra_samples[0].data, stored_spot, "{name} own decoder");

        let (spp, bps, extra, rows, samples) = independent_samples(&r.bytes);
        assert_eq!(spp, 4, "{name}");
        assert_eq!(bps, vec![8, 8, 8, 8], "{name}");
        assert_eq!(extra, vec![0], "{name} ExtraSamples");
        assert!(rows > 0 && rows < h, "{name} rows_per_strip {rows}");
        assert_plane(&samples, 4, 3, &stored_spot);
        seen.push(samples.clone());

        // Outside the artwork the composite is matted to white (Save Transparency off). The
        // spot at that pixel is not zero, so a reader that binds the name to the alpha fails.
        let at = (10 * w as usize + 10) * 4;
        assert_eq!(&samples[at..at + 3], &[255, 255, 255], "{name} matted rgb");
        let (resources, layer_tag) = tags(&r.bytes);
        let resources = resources.expect("photoshop resources");
        let (blocks, warnings) = resources_from_bytes(&resources).unwrap();
        assert!(warnings.is_empty(), "{name} {warnings:?}");
        let block = |id: u16| blocks.iter().find(|b| b.id == id).unwrap_or_else(|| panic!("{name} missing {id}"));
        // "Spot Color 1" is 12 characters: Photoshop stores 13 bytes and pads the 8BIM block, not the string.
        assert_eq!(block(1006).data, b"\x0cSpot Color 1", "{name}");
        assert_eq!(block(1045).data.len(), 30, "{name}");
        let info = &block(1077).data;
        assert_eq!(info.len(), 17, "{name}");
        assert_eq!(u16::from_be_bytes([info[14], info[15]]), 60, "{name} solidity");
        assert_eq!(info[16], 2, "{name} spot kind");
        // Photoshop numbers a new document's channels from 3.
        assert_eq!(block(1053).data, 3u32.to_be_bytes(), "{name} channel id");

        let back = import("dtf.tif", &r.bytes).unwrap().document;
        assert_eq!(back.channels.len(), 1, "{name}");
        assert_eq!(back.channels[0].name, "Spot Color 1");
        let (ink, sol) = back.channels[0].spot.expect("spot");
        assert_eq!(ink.to_rgb(), [1.0, 0.0, 0.0], "{name}");
        assert!((sol - 0.6).abs() < 1e-4, "{name}");
        let area = Rect::new(0, 0, w as i32, h as i32);
        assert_eq!(back.channels[0].surface.to_interleaved(area), spot_bytes, "{name} reimport");
        if layers {
            let layer_tag = layer_tag.expect("layer tag");
            let (isd, w) = ImageSourceData::from_bytes(&layer_tag).unwrap();
            assert!(w.is_empty(), "{w:?}");
            let info = isd.layer_info.expect("layers");
            assert!(!info.merged_alpha, "{name}");
            assert!(info.layers.iter().all(|l| l.flags.0 & 0x08 != 0), "{name} photoshop flag");
            assert!(info.layers.iter().all(|l| l.blocks.iter().any(|b| &b.key == b"lnsr")), "{name} lnsr");
            let mask = isd.global_layer_mask.expect("LMsk");
            assert_eq!(mask.data.len(), 14, "{name}");
            assert_eq!(mask.opacity(), Some(50));
            assert_eq!(mask.kind(), Some(128));
            assert_photoshop_layered_tiff(&r.bytes, true);
            assert_eq!(back.layers.len(), 2, "{name}");
            let ink = back.layers.iter().find(|l| l.name == "Ink").expect("ink layer");
            let px = ink.surface().expect("ink pixels").pixel(100, 100);
            assert!((px[3] - 0.5).abs() < 0.02, "{name} layer alpha {px:?}");
            let outside = ink.surface().expect("ink pixels").pixel(0, 0);
            assert!(outside[3] < 0.02, "{name} transparent pixel {outside:?}");
        } else {
            assert!(layer_tag.is_none(), "{name}");
        }
    }
    assert_eq!(seen[0], seen[1], "flat and layered IFD samples differ");
}

/// A transparent document with no user channels still stores image alpha.
#[test]
fn transparent_tiff_without_channels_keeps_image_alpha() {
    use photocraft_doc::Layer;
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::Surface;

    let mut doc = Document::new("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    let fmt = doc.pixel_format();
    let mut art = Surface::new(fmt);
    art.fill_rect(Rect::new(1, 1, 6, 6), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 0.5]));
    doc.layers.push(Layer::new("Art", LayerContent::Raster(art)));
    for layers in [false, true] {
        let r = export(&doc, "t.tif", &ExportOptions { tiff_layers: layers, ..ExportOptions::default() }).unwrap();
        let img = photocraft_codecs::decode(&r.bytes).unwrap();
        assert!(img.layout().has_alpha(), "{layers}");
        assert!(img.meta.extra_samples.is_empty(), "{layers}");
        let mut dec = tiff::decoder::Decoder::new(std::io::Cursor::new(r.bytes.as_slice())).unwrap();
        let extra = dec.get_tag_u16_vec(tiff::tags::Tag::ExtraSamples).unwrap();
        assert_eq!(extra, vec![2], "{layers}");
        assert!(!r.warnings.iter().any(|w| w.contains("Save Transparency")), "{layers}: {:?}", r.warnings);
        if layers {
            assert_photoshop_layered_tiff(&r.bytes, false);
        }
    }
}

/// A flat TIFF that also stores an alpha or spot channel mattes the composite on white
/// and drops image transparency. The export says so.
#[test]
fn flat_tiff_warns_when_extra_channels_matte_transparency() {
    use photocraft_color::{Color, PixelFormat};
    use photocraft_doc::{AlphaChannel, Layer};
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::Surface;

    let mut doc = Document::new("t", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    let fmt = doc.pixel_format();
    let mut art = Surface::new(fmt);
    // (0, 0) stays transparent. (1, 1) is half-red, so the matte is halfway to white.
    art.fill_rect(Rect::new(1, 1, 2, 2), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 0.5]));
    doc.layers.push(Layer::new("Art", LayerContent::Raster(art)));
    let mut spot = AlphaChannel::new("Spot Color 1", Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false)));
    spot.spot = Some((Color::rgb(0.0, 1.0, 0.0), 1.0));
    doc.channels.push(spot);

    let r = export(&doc, "t.tif", &ExportOptions { tiff_layers: false, ..ExportOptions::default() }).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("composited over white") && w.contains("Save Transparency")), "{:?}", r.warnings);
    let img = photocraft_codecs::decode(&r.bytes).unwrap();
    assert!(!img.layout().has_alpha(), "{:?}", img.layout());
    assert_eq!(img.meta.extra_samples.len(), 1);
    let (_spp, _bps, extra, _rows, samples) = independent_samples(&r.bytes);
    assert_eq!(extra, vec![0]);
    // 4 samples per pixel: RGB + spot. (0, 0) was transparent and is now white.
    assert_eq!(&samples[..3], &[255, 255, 255]);
    // Half-red over white is about (255, 128, 128). Pixel (1, 1), 4 samples per pixel.
    let (x, y, width, spp) = (1usize, 1usize, 4usize, 4usize);
    let at = (y * width + x) * spp;
    let rgb = &samples[at..at + 3];
    assert!(rgb[0] > 250 && (rgb[1] as i16 - 128).abs() <= 2 && (rgb[2] as i16 - 128).abs() <= 2, "{rgb:?}");
}

/// A 4×4 RGB document whose one layer is transparent outside its middle, plus a "White" spot
/// channel with solid ink in its left half: the shape of a UV print's underbase.
fn transparent_art_with_white_spot() -> Document {
    use photocraft_color::{Color, PixelFormat};
    use photocraft_doc::{AlphaChannel, Layer};
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::Surface;

    let mut doc = Document::new("t", Size::new(4, 4), ColorMode::Rgb, SampleType::U8);
    let fmt = doc.pixel_format();
    let mut art = Surface::new(fmt);
    art.fill_rect(Rect::new(1, 1, 3, 3), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
    doc.layers.push(Layer::new("Layer 1", LayerContent::Raster(art)));
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let mut plate = Surface::new(gray);
    plate.fill_rect(Rect::new(0, 0, 2, 4), &photocraft_raster::from_rgba(&gray, [1.0, 1.0, 1.0, 1.0]));
    let mut white = AlphaChannel::new("White", plate);
    white.spot = Some((Color::rgb(0.8, 0.2, 0.2), 1.0));
    doc.channels.push(white);
    doc
}

/// The first channel's ink at (0, 0), inked, and at (3, 0), bare.
fn ink(d: &Document) -> (f32, f32) {
    (d.channels[0].surface.pixel(0, 0)[0], d.channels[0].surface.pixel(3, 0)[0])
}

/// `doc` laid out as Photoshop 25.12 saves a TIFF with Save Transparency on: R, G, B,
/// transparency and spot samples, with "Transparency" first in the channel names and display
/// records. With `layers`, the layer data has a positive layer count and an `MTrn` block (the
/// specification's key is `Mtrn`); without, the file is flat. Built from the PSD model of `doc`.
fn photoshop_transparency_and_spot_tiff(doc: &Document, layers: bool) -> Vec<u8> {
    use photocraft_psd::{Compression, TaggedBlock};

    let opts = PsdExportOptions { merged_matte: false, merged_compression: Compression::Raw, ..Default::default() };
    let (file, _) = document_to_psd_with(doc, &opts);
    let planes = file.decode_merged().unwrap();
    let n = 16;
    assert_eq!(planes.len(), 5 * n);
    let rgba: Vec<u8> = (0..n).flat_map(|i| (0..4).map(move |c| (i, c))).map(|(i, c)| planes[c * n + i]).collect();
    let mut img = photocraft_codecs::Image::from_raw(4, 4, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, rgba).unwrap();
    img.meta.extra_samples = vec![photocraft_codecs::ExtraSample { kind: 0, data: planes[4 * n..].to_vec() }];
    img.meta.photoshop_resources = Some(photocraft_psd::tiff::resources_to_bytes(&file.resources).unwrap());
    if layers {
        let mut isd = ImageSourceData::from_psd(&file);
        if let Some(info) = &mut isd.layer_info {
            info.merged_alpha = false;
        }
        isd.global_blocks.insert(0, TaggedBlock::new(*b"MTrn", Vec::new()));
        let order = if photocraft_codecs::tiff_writes_little_endian() { ByteOrder::Little } else { ByteOrder::Big };
        img.meta.photoshop_layers = Some(isd.to_bytes(order).unwrap().0);
    }
    photocraft_codecs::encode(&img, photocraft_codecs::Format::Tiff, &photocraft_codecs::EncodeOptions::default()).unwrap()
}

/// Photoshop's layered TIFF with transparency next to a spot channel opens with the spot as the
/// only channel. A layered re-save leaves the transparency sample out next to the spot, so it
/// must also drop the `MTrn` marker, or the spot would read back as transparency.
#[test]
fn photoshop_mtrn_tiff_opens_without_a_transparency_channel() {
    use photocraft_psd::TaggedBlock;

    let bytes = photoshop_transparency_and_spot_tiff(&transparent_art_with_white_spot(), true);
    let opened = import("t.tif", &bytes).unwrap().document;
    let names: Vec<&str> = opened.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["White"]);
    assert!(opened.channels[0].spot.is_some());
    assert_eq!(ink(&opened), (1.0, 0.0));

    let r = tiff(&opened, true);
    let (isd, _) = ImageSourceData::from_bytes(&tags(&r.bytes).1.unwrap()).unwrap();
    assert!(!isd.global_blocks.iter().any(TaggedBlock::is_merged_transparency_marker));
    let back = import("t.tif", &r.bytes).unwrap().document;
    let names: Vec<&str> = back.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["White"]);
    assert_eq!(ink(&back), (1.0, 0.0));
}

/// The same Photoshop layout in a flat TIFF (no layer data): the image keeps its transparency,
/// and the spot gets its own name and display record, not the leading "Transparency" ones, so
/// it opens as a spot with its ink the right way round.
#[test]
fn photoshop_flat_tiff_with_transparency_opens_its_spot_as_a_spot() {
    let bytes = photoshop_transparency_and_spot_tiff(&transparent_art_with_white_spot(), false);
    assert!(tags(&bytes).1.is_none(), "flat: no layer data");
    let opened = import("t.tif", &bytes).unwrap().document;
    let names: Vec<&str> = opened.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["White"]);
    assert!(opened.channels[0].spot.is_some(), "a spot, not an alpha channel");
    assert_eq!(ink(&opened), (1.0, 0.0));
    let LayerContent::Raster(s) = &opened.layers[0].content else { panic!("raster") };
    assert_eq!(s.pixel(0, 0)[3], 0.0, "the image keeps its transparency");
}

/// A Lab document with a spot channel keeps the spot as a CIELab extra sample, and its
/// transparency is matted on Lab white (L* 100, a* and b* 0), not on the maximum of every sample.
#[test]
fn lab_tiff_keeps_spot_and_mattes_on_lab_white() {
    use photocraft_color::{Color, PixelFormat};
    use photocraft_doc::{AlphaChannel, Layer};
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::Surface;

    for sample in [SampleType::U8, SampleType::U16] {
        let mut doc = Document::new("t", Size::new(4, 4), ColorMode::Lab, sample);
        let fmt = doc.pixel_format();
        let mut art = Surface::new(fmt);
        // (0, 0) stays transparent.
        art.fill_rect(Rect::new(1, 1, 2, 2), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
        doc.layers.push(Layer::new("Art", LayerContent::Raster(art)));
        let mut plate = Surface::new(PixelFormat::new(ColorMode::Grayscale, sample, false));
        plate.fill_rect(Rect::new(0, 0, 2, 4), &photocraft_raster::from_rgba(&plate.format(), [1.0, 1.0, 1.0, 1.0]));
        let mut spot = AlphaChannel::new("Spot Color 1", plate);
        spot.spot = Some((Color::rgb(0.0, 1.0, 0.0), 1.0));
        doc.channels.push(spot);

        // Lab is always saved flat, whether layers were asked for or not.
        for layers in [false, true] {
            let r = export(&doc, "t.tif", &ExportOptions { tiff_layers: layers, ..ExportOptions::default() }).unwrap();
            let img = photocraft_codecs::decode(&r.bytes).unwrap();
            assert_eq!(img.layout(), photocraft_codecs::ChannelLayout::Lab, "{sample:?} {layers}");
            assert_eq!(img.meta.extra_samples.len(), 1, "{sample:?} {layers}");
            let white = &img.to_normalized()[..3];
            assert!((white[0] - 1.0).abs() < 1e-3 && (white[1] - 128.0 / 255.0).abs() < 1e-3 && (white[2] - 128.0 / 255.0).abs() < 1e-3, "{white:?}");
            let back = import("t.tif", &r.bytes).unwrap().document;
            assert_eq!(back.pixel_format().mode, ColorMode::Lab, "{sample:?} {layers}");
            assert_eq!(back.channels.len(), 1, "{sample:?} {layers}");
            assert_eq!(back.channels[0].name, "Spot Color 1");
            assert!(back.channels[0].spot.is_some(), "{sample:?} {layers}");
            let plate = |c: &AlphaChannel| c.surface.to_interleaved(Rect::new(0, 0, 4, 4));
            assert_eq!(plate(&back.channels[0]), plate(&doc.channels[0]), "{sample:?} {layers}");
        }
    }
}

/// Tag 37724 walked as raw bytes (not through [`ImageSourceData`]).
///
/// Matches a Photoshop 27 layered TIFF: classic 4-byte block lengths, `Layr` then a 14-byte
/// `LMsk`, no `MTrn` (that block is only for a composite that keeps both transparency and a
/// spot). A 4320×4320 RGB spot plate is 74_649_600 bytes and still fits those u32 lengths;
/// this check uses the same layout on a smaller image.
fn assert_photoshop_layered_tiff(bytes: &[u8], with_spot: bool) {
    assert!(bytes.starts_with(b"II"), "little-endian TIFF");
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    let ifd = u32_at(4) as usize;
    let n = u16_at(ifd) as usize;
    let mut layers = None;
    let mut spp = 0u16;
    let mut extra = Vec::new();
    for e in 0..n {
        let at = ifd + 2 + e * 12;
        let tag = u16_at(at);
        let typ = u16_at(at + 2);
        let count = u32_at(at + 4) as usize;
        let val = u32_at(at + 8) as usize;
        let unit: usize = match typ {
            3 => 2,
            4 => 4,
            _ => 1,
        };
        let size = unit.saturating_mul(count);
        let inline = at + 8;
        let raw = if size <= 4 { &bytes[inline..inline + size] } else { &bytes[val..val + size] };
        match tag {
            277 => spp = u16::from_le_bytes([raw[0], raw[1]]),
            338 => extra = raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect(),
            37724 => layers = Some(raw),
            _ => {}
        }
    }
    let isd = layers.expect("37724");
    let sig = b"Adobe Photoshop Document Data Block\0";
    assert!(isd.starts_with(sig), "PSB-sized lengths");
    let mut pos = sig.len();
    let mut keys = Vec::new();
    let mut layr: Option<Vec<u8>> = None;
    while pos + 12 <= isd.len() {
        assert_eq!(&isd[pos..pos + 4], b"MIB8", "block at {pos}");
        let key = [isd[pos + 7], isd[pos + 6], isd[pos + 5], isd[pos + 4]];
        let len = u32::from_le_bytes(isd[pos + 8..pos + 12].try_into().unwrap()) as usize;
        let body = &isd[pos + 12..pos + 12 + len];
        if &key == b"Layr" {
            layr = Some(body.to_vec());
        }
        if &key == b"LMsk" {
            assert_eq!(body, &[0, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0x32, 0, 0x80, 0]);
        }
        keys.push(key);
        pos += 12 + len;
        let rest = &isd[pos..];
        let pad = (0..=3).find(|&k| k == rest.len() || rest.get(k..k + 4) == Some(b"MIB8")).unwrap_or(0);
        pos += pad;
    }
    assert_eq!(pos, isd.len(), "trailing bytes after the last block");
    assert_eq!(keys.first(), Some(b"Layr"));
    assert_eq!(keys.get(1), Some(b"LMsk"));
    assert!(!keys.iter().any(|k| k == b"MTrn" || k == b"Mtrn"));
    let layr = layr.expect("Layr");
    let count = i16::from_le_bytes([layr[0], layr[1]]);
    if with_spot {
        assert!(count > 0, "layer count {count}");
        assert_eq!((spp, extra.as_slice()), (4, [0u16].as_slice()));
    } else {
        assert!(count < 0, "layer count {count}");
        assert_eq!(extra, vec![2]);
    }
    let nch = u16::from_le_bytes([layr[18], layr[19]]) as usize;
    let mut ids = Vec::new();
    let mut p = 20;
    for _ in 0..nch {
        ids.push(i16::from_le_bytes([layr[p], layr[p + 1]]));
        p += 6;
    }
    assert!(ids.contains(&-1), "channel ids {ids:?}");
}

/// Photoshop 27's Kato-Karate TIFF stores spot ink inverted: every non-white artwork pixel
/// has spot 0, and the white background is spot 255. An alpha channel is the other way
/// around (255 = selected). Photoshop's PSDs store both the same way, and so does a PSD saved
/// here. Everything round-trips, PSD → TIFF → PSD included.
#[test]
fn spot_ink_is_stored_as_zero_and_alpha_is_not() {
    use photocraft_color::{Color, PixelFormat};
    use photocraft_doc::{AlphaChannel, Layer};
    use photocraft_geom::{Rect, Size};
    use photocraft_psd::PsdFile;
    use photocraft_raster::Surface;

    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let (w, h) = (8u32, 8u32);
        let mut doc = Document::new("pol", Size::new(w, h), ColorMode::Rgb, depth);
        let fmt = doc.pixel_format();
        let mut art = Surface::new(fmt);
        art.fill_rect(Rect::new(2, 2, 6, 6), &photocraft_raster::from_rgba(&fmt, [1.0, 0.2, 0.1, 1.0]));
        doc.layers.push(Layer::new("Art", LayerContent::Raster(art)));
        let gray = PixelFormat::new(ColorMode::Grayscale, depth, false);
        let n = (w as usize) * (h as usize);
        let mut ink = vec![0.0f32; n];
        for y in 2..6 {
            for x in 2..6 {
                ink[y * w as usize + x] = 1.0;
            }
        }
        let mut spot = AlphaChannel::new("Spot Color 1", Surface::new(gray));
        spot.surface.write_region(Rect::new(0, 0, w as i32, h as i32), &ink);
        spot.spot = Some((Color::rgb(1.0, 0.0, 0.0), 0.6));
        doc.channels.push(spot);
        let mut alpha = AlphaChannel::new("Alpha 1", Surface::new(gray));
        alpha.surface.write_region(Rect::new(0, 0, w as i32, h as i32), &ink);
        doc.channels.push(alpha);

        let psd = export(&doc, "pol.psd", &ExportOptions::default()).unwrap();
        let file = PsdFile::from_bytes(&psd.bytes).unwrap();
        let merged = file.decode_merged().unwrap();
        let bps = depth.bytes();
        let plane = n * bps;
        let first = 3 + usize::from(file.merged_has_alpha());
        let spot_plane = &merged[first * plane..(first + 1) * plane];
        let alpha_plane = &merged[(first + 1) * plane..(first + 2) * plane];
        // Background (0,0) and artwork (3,3). Spot: 0 = solid ink, max = paper.
        // Alpha: max = selected, 0 = empty.
        assert_stored(spot_plane, 0, depth, false, "psd spot background");
        assert_stored(spot_plane, (3 * w as usize + 3) * bps, depth, true, "psd spot art");
        assert_stored(alpha_plane, 0, depth, true, "psd alpha background");
        assert_stored(alpha_plane, (3 * w as usize + 3) * bps, depth, false, "psd alpha art");
        // The merged transparency (0), then the spot and the alpha channel (3, 4).
        assert_eq!(file.resource(1053).map(|r| identifiers(&r.data)), Some(vec![0, 3, 4]), "{depth:?}");
        let back = import("pol.psd", &psd.bytes).unwrap().document;
        let area = Rect::new(0, 0, w as i32, h as i32);
        assert_eq!(back.channels[0].surface.to_interleaved(area), doc.channels[0].surface.to_interleaved(area), "{depth:?} psd spot");
        assert_eq!(back.channels[1].surface.to_interleaved(area), doc.channels[1].surface.to_interleaved(area), "{depth:?} psd alpha");
        assert!(back.channels[0].spot.is_some(), "{depth:?}");
        assert!(back.channels[1].spot.is_none(), "{depth:?}");

        if depth == SampleType::U8 {
            for layers in [false, true] {
                // The document as opened from the PSD, so the PSD's own resources come along.
                let r = export(&back, "pol.tif", &ExportOptions { tiff_layers: layers, ..ExportOptions::default() }).unwrap();
                let (spp, _bps, extra, _rows, samples) = independent_samples(&r.bytes);
                assert_eq!((spp, extra.as_slice()), (5, [0u16, 0].as_slice()), "{layers}");
                // Reference TIFF: artwork spot sample is 0, background spot sample is 255.
                assert_eq!(samples[3], 255, "{layers} background spot");
                assert_eq!(samples[(3 * w as usize + 3) * 5 + 3], 0, "{layers} art spot");
                assert_eq!(samples[4], 0, "{layers} background alpha");
                assert_eq!(samples[(3 * w as usize + 3) * 5 + 4], 255, "{layers} art alpha");
                // One identifier per extra sample, numbered from 3 (no transparency sample).
                assert_eq!(tiff_alpha_identifiers(&r.bytes), [3, 4], "{layers}");
                let from_tiff = import("pol.tif", &r.bytes).unwrap().document;
                assert_eq!(from_tiff.channels[0].surface.to_interleaved(area), doc.channels[0].surface.to_interleaved(area), "{layers}");
                assert_eq!(from_tiff.channels[1].surface.to_interleaved(area), doc.channels[1].surface.to_interleaved(area), "{layers}");
                assert!(from_tiff.channels[0].spot.is_some() && from_tiff.channels[1].spot.is_none(), "{layers}");
                // Saved as PSD again: the first PSD's planes, and one 1053 of its own. The flat TIFF's
                // composite was matted on white, so only the layered one brings transparency back.
                let psd = PsdFile::from_bytes(&export(&from_tiff, "pol.psd", &ExportOptions::default()).unwrap().bytes).unwrap();
                let again = psd.decode_merged().unwrap();
                assert_eq!(again[again.len() - 2 * plane..], merged[(first * plane)..(first + 2) * plane], "{layers} tiff to psd");
                assert_eq!(psd.merged_has_alpha(), layers, "{layers} tiff to psd");
                let want: Vec<u32> = if layers { vec![0, 3, 4] } else { vec![3, 4] };
                assert_eq!(psd.resources.iter().filter(|r| r.id == 1053).map(|r| identifiers(&r.data)).collect::<Vec<_>>(), [want], "{layers} tiff to psd");
                let reopened = import("pol.psd", &psd.to_bytes().unwrap()).unwrap().document;
                assert_eq!(reopened.channels[0].surface.to_interleaved(area), doc.channels[0].surface.to_interleaved(area), "{layers}");
                assert_eq!(reopened.channels[1].surface.to_interleaved(area), doc.channels[1].surface.to_interleaved(area), "{layers}");
            }
        }
    }
}

/// The big-endian u32s of a resource 1053.
fn identifiers(data: &[u8]) -> Vec<u32> {
    data.as_chunks::<4>().0.iter().map(|c| u32::from_be_bytes(*c)).collect()
}

/// The identifiers in a TIFF's resource 1053 (none without one).
fn tiff_alpha_identifiers(bytes: &[u8]) -> Vec<u32> {
    let resources = tags(bytes).0.expect("photoshop resources");
    let (blocks, _) = photocraft_psd::tiff::resources_from_bytes(&resources).unwrap();
    blocks.iter().find(|b| b.id == 1053).map(|b| identifiers(&b.data)).unwrap_or_default()
}

/// `at` addresses one sample of a big-endian PSD plane. `zero` expects 0; otherwise the
/// maximum sample (255 / 65535 / 1.0).
fn assert_stored(plane: &[u8], at: usize, depth: SampleType, zero: bool, what: &str) {
    match depth {
        SampleType::U8 => {
            let v = plane[at];
            assert_eq!(v, if zero { 0 } else { 255 }, "{what} {v}");
        }
        SampleType::U16 => {
            let v = u16::from_be_bytes([plane[at], plane[at + 1]]);
            assert_eq!(v, if zero { 0 } else { 65535 }, "{what} {v}");
        }
        SampleType::F32 => {
            let v = f32::from_be_bytes([plane[at], plane[at + 1], plane[at + 2], plane[at + 3]]);
            assert_eq!(v, if zero { 0.0 } else { 1.0 }, "{what} {v}");
        }
    }
}

/// Quick Mask stays the last channel after the merged transparency plane is omitted.
#[test]
fn spot_tiff_keeps_quick_mask_when_composite_alpha_is_omitted() {
    use photocraft_color::{Color, PixelFormat};
    use photocraft_doc::{AlphaChannel, Layer};
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::Surface;

    let mut doc = Document::new("q", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    let fmt = doc.pixel_format();
    let mut art = Surface::new(fmt);
    art.fill_rect(Rect::new(1, 1, 6, 6), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 0.5]));
    doc.layers.push(Layer::new("Art", LayerContent::Raster(art)));
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let area = Rect::new(0, 0, 8, 8);
    let mut spot_bytes = vec![9u8; 64];
    spot_bytes[0] = 3;
    let mut channel = AlphaChannel::new("Spot Color 1", Surface::from_interleaved(gray, area, &spot_bytes));
    channel.spot = Some((Color::rgb(1.0, 0.0, 0.0), 0.6));
    doc.channels.push(channel);
    let mut qbytes = vec![7u8; 64];
    qbytes[1] = 4;
    doc.quick_mask = Some(AlphaChannel::new("Quick Mask", Surface::from_interleaved(gray, area, &qbytes)));

    for layers in [false, true] {
        let r = export(&doc, "q.tif", &ExportOptions { tiff_layers: layers, ..ExportOptions::default() }).unwrap();
        let (spp, _bps, extra, _rows, samples) = independent_samples(&r.bytes);
        assert_eq!((spp, extra.as_slice()), (5, [0, 0].as_slice()), "{layers}");
        let stored_spot: Vec<u8> = spot_bytes.iter().map(|b| 255 - b).collect();
        assert_plane(&samples, 5, 3, &stored_spot);
        // A Quick Mask is an alpha channel: stored straight, not as spot ink.
        assert_plane(&samples, 5, 4, &qbytes);
        let back = import("q.tif", &r.bytes).unwrap().document;
        assert_eq!(back.channels.len(), 1, "{layers}");
        assert_eq!(back.channels[0].surface.to_interleaved(area), spot_bytes, "{layers}");
        let qm = back.quick_mask.as_ref().unwrap_or_else(|| panic!("{layers}: no quick mask"));
        assert_eq!(qm.surface.to_interleaved(area), qbytes, "{layers}");
    }
}

#[test]
fn default_export_options_write_a_flat_tiff() {
    // Scripted and agent saves (CLI, batch, MCP) build on the defaults: layers only on request.
    let d = gen_doc(ColorMode::Rgb, SampleType::U8, Features::PIXELS);
    assert!(tiff_layers::would_write_layers(&d));
    let r = export(&d, "x.tif", &ExportOptions::default()).unwrap();
    assert_eq!(tags(&r.bytes), (None, None));
    assert_eq!(import("x.tif", &r.bytes).unwrap().document.layers.len(), 1);
}

/// #2940: layered TIFF data whose groups nest past the document's cap is rejected with the same
/// limit error as a PSD, instead of being converted.
#[test]
fn layered_tiff_nested_past_the_cap_is_rejected() {
    use photocraft_psd::{GroupSpec, LayerSpec, PixelData, PsdBuilder, PsdFile};
    let tiff_nested = |levels: usize| {
        let mut b = PsdBuilder::new(4, 4);
        for _ in 0..levels {
            b.begin_group(GroupSpec::new("g"));
        }
        b.push_layer(LayerSpec::new("deep", 0, 0, 1, 1, PixelData::Rgba8(vec![1, 2, 3, 4])));
        for _ in 0..levels {
            b.end_group().unwrap();
        }
        let file = PsdFile::from_bytes(&b.to_bytes().unwrap()).unwrap();
        let order = if photocraft_codecs::tiff_writes_little_endian() { ByteOrder::Little } else { ByteOrder::Big };
        let layers = ImageSourceData::from_psd(&file).to_bytes(order).unwrap().0;
        let d = gen_doc(ColorMode::Rgb, SampleType::U8, Features::PIXELS);
        let mut img = photocraft_codecs::decode(&tiff(&d, false).bytes).unwrap();
        img.meta.photoshop_layers = Some(layers);
        photocraft_codecs::encode(&img, photocraft_codecs::Format::Tiff, &photocraft_codecs::EncodeOptions::default()).unwrap()
    };
    let err = import("deep.tif", &tiff_nested(101)).unwrap_err();
    assert!(err.to_string().contains("nested deeper than 100"), "{err}");
    let r = import("hundred.tif", &tiff_nested(100)).unwrap();
    assert_eq!(r.document.max_group_depth(), 100);
}

#[test]
fn layered_tiff_writes_photoshops_global_layer_mask() {
    // #9095: Photoshop reads only the composite of a layered TIFF whose `LMsk` block is empty
    // ("There was a problem reading the layer data"); it writes 14 bytes there itself: overlay
    // colour space 0, colour (65535, 0, 0, 0), opacity 50, kind 128, one filler byte.
    for depth in [SampleType::U8, SampleType::U16] {
        let d = gen_doc(ColorMode::Rgb, depth, Features::PIXELS);
        let (_, layers) = tags(&tiff(&d, true).bytes);
        let (isd, _) = ImageSourceData::from_bytes(&layers.expect("layer data")).expect("parse layer data");
        let mask = isd.global_layer_mask.expect("LMsk block");
        assert_eq!(mask.data, [0, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 50, 128, 0], "{depth:?}");
    }
}

/// Past MAX_EXTRA_SAMPLES (16), a layered TIFF stores the first 16 extra channels. Its channel
/// resources must describe those 16 and no more: 16 names in 1006 and 1045, 16 DisplayInfo
/// records and 16 identifiers, and no Quick Mask record for a Quick Mask that wasn't stored.
#[test]
fn channels_past_sixteen_leave_no_resources_behind() {
    use photocraft_color::PixelFormat;
    use photocraft_doc::{AlphaChannel, Layer};
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::Surface;

    let mut doc = Document::new("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    let fmt = doc.pixel_format();
    let mut art = Surface::new(fmt);
    art.fill_rect(Rect::new(0, 0, 8, 8), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
    doc.layers.push(Layer::new("Art", LayerContent::Raster(art)));
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    for i in 1..=18 {
        doc.channels.push(AlphaChannel::new(format!("Alpha {i}"), Surface::new(gray)));
    }
    doc.quick_mask = Some(AlphaChannel::new("Quick Mask", Surface::new(gray)));

    let r = tiff(&doc, true);
    assert!(r.warnings.iter().any(|w| w.contains("19 extra channels") && w.contains("first 16")), "{:?}", r.warnings);
    let (resources, _) = tags(&r.bytes);
    let (resources, _) = photocraft_psd::tiff::resources_from_bytes(&resources.expect("resources")).expect("parse resources");
    let data = |id: u16| resources.iter().find(|r| r.id == id).map(|r| r.data.clone()).unwrap_or_default();
    let pascal = data(1006);
    let (mut at, mut names) = (0usize, 0usize);
    while let Some(&len) = pascal.get(at) {
        at += 1 + usize::from(len);
        names += 1;
    }
    assert_eq!(names, 16, "1006");
    let unicode = data(1045);
    let (mut at, mut names) = (0usize, 0usize);
    while let Some(b) = unicode.get(at..at + 4) {
        at += 4 + 2 * u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
        names += 1;
    }
    assert_eq!(names, 16, "1045");
    assert_eq!(data(1077).len(), 4 + 16 * 13, "1077");
    assert_eq!(data(1053).len(), 16 * 4, "1053");
    assert!(resources.iter().all(|r| r.id != 1022), "the Quick Mask was not stored, so its record goes");

    let flat = photocraft_codecs::decode(&r.bytes).expect("decode");
    assert_eq!(flat.meta.extra_samples.len(), 16);
    let back = import("x.tif", &r.bytes).expect("import").document;
    let names: Vec<&str> = back.channels.iter().map(|c| c.name.as_str()).collect();
    let want: Vec<String> = (1..=16).map(|i| format!("Alpha {i}")).collect();
    assert_eq!(names, want.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(back.quick_mask.is_none());
}
