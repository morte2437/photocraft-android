//! A PNG imported in steps gives the document `import_with_options_and_interrupt` gives.

use std::sync::Arc;

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType, encode};
use photocraft_doc::Document;
use photocraft_io::{ImportOptions, ImportResult, PngImport, import_with_options_and_interrupt};
use photocraft_raster::Interrupt;

/// A pseudo-random `w`×`h` image.
fn noise(w: u32, h: u32, layout: ChannelLayout, sample: SampleType) -> Image {
    let len = w as usize * h as usize * layout.channels() * sample.bytes();
    Image::from_raw(w, h, layout, sample, (0..len).map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8).collect()).unwrap()
}

fn whole(name: &str, bytes: &[u8]) -> ImportResult {
    import_with_options_and_interrupt(name, bytes, &ImportOptions::default(), &Interrupt::NONE).unwrap()
}

/// Imports `bytes` in steps, `per_call` rows at a time (0: unlimited); returns the result and
/// how many calls it took.
fn stepped(name: &str, bytes: &[u8], per_call: usize) -> (ImportResult, usize) {
    let bytes = Arc::new(bytes.to_vec());
    let mut import = PngImport::new(name, &bytes).unwrap().expect("a plain PNG");
    let mut calls = 0;
    loop {
        calls += 1;
        let mut left = per_call;
        let mut more = || {
            left = left.wrapping_sub(1);
            per_call == 0 || left > 0
        };
        if let Some(r) = import.step(&mut more).unwrap() {
            return (r, calls);
        }
        assert!(calls < 100_000);
    }
}

/// The documents are the same but for the ids minted for them.
fn assert_same(a: &Document, b: &Document, what: &str) {
    let (mut a, mut b) = (a.clone(), b.clone());
    b.id = a.id;
    assert_eq!(a.layers.len(), b.layers.len(), "{what}");
    for (x, y) in a.layers.iter_mut().zip(&mut b.layers) {
        y.id = x.id;
    }
    assert!(a == b, "{what}: the documents differ");
}

fn assert_stepped_equals_import(name: &str, bytes: &[u8], what: &str) {
    let expected = whole(name, bytes);
    for per_call in [1, 7, 0] {
        let (got, _) = stepped(name, bytes, per_call);
        assert_same(&got.document, &expected.document, &format!("{what}, {per_call} rows per call"));
        assert_eq!(got.warnings, expected.warnings, "{what}");
        assert_eq!((got.source_read_only, got.preview_only), (expected.source_read_only, expected.preview_only), "{what}");
    }
}

#[test]
fn stepped_png_import_equals_import() {
    for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
        for sample in [SampleType::U8, SampleType::U16] {
            let mut img = noise(45, 38, layout, sample);
            img.meta.dpi = Some((144.0, 144.0));
            img.meta.text.push(("Comment".into(), "stepped".into()));
            for interlaced in [false, true] {
                let bytes = encode(&img, Format::Png, &EncodeOptions { png_interlaced: interlaced, ..Default::default() }).unwrap();
                assert_stepped_equals_import("a.png", &bytes, &format!("{layout:?} {sample:?} interlaced={interlaced}"));
            }
        }
    }
}

#[test]
fn stepped_png_import_keeps_a_palette_as_indexed_colour() {
    let pal = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [9, 9, 9]];
    let idx: Vec<u8> = (0..40 * 30).map(|i| (i % 4) as u8).collect();
    let bytes = photocraft_codecs::encode_png_indexed(40, 30, &idx, &pal, Some(3), None, None).unwrap();
    let (got, _) = stepped("p.png", &bytes, 0);
    assert!(got.document.color_table.is_some(), "an Indexed Color document");
    assert_stepped_equals_import("p.png", &bytes, "palette");
}

#[test]
fn stepped_png_import_takes_a_step_per_row_and_bounded_steps_to_build_the_document() {
    let bytes = encode(&noise(8, 20, ChannelLayout::Rgba, SampleType::U8), Format::Png, &EncodeOptions::default()).unwrap();
    let (_, calls) = stepped("a.png", &bytes, 1);
    // 19 calls that each decode a row (its rows become tiles as they come), one that decodes the
    // last row and reads to the end of the file, then the prune scan of the tiles and the document.
    assert_eq!(calls, 21);
    let arc = Arc::new(bytes);
    let mut import = PngImport::new("a.png", &arc).unwrap().unwrap();
    assert_eq!(import.progress(), 0.0);
    while import.step(&mut || false).unwrap().is_none() {
        assert!(import.progress() <= 1.0);
    }
    assert!(import.step(&mut || true).is_err(), "a finished import is spent");
}

/// Imports `bytes` with an empty budget (one bounded piece per call); returns the result and the
/// number of calls.
fn stepped_zero_budget(name: &str, bytes: &[u8]) -> (ImportResult, usize) {
    let bytes = Arc::new(bytes.to_vec());
    let mut import = PngImport::new(name, &bytes).unwrap().expect("a plain PNG");
    let mut calls = 0;
    loop {
        calls += 1;
        if let Some(r) = import.step(&mut || false).unwrap() {
            return (r, calls);
        }
        assert!(calls < 100_000, "the import never finished");
    }
}

/// A CRC-valid PNG chunk.
fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    let mut out = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc.sum().to_be_bytes());
    out
}

/// A compressed `zTXt` chunk with `text` as its value.
fn ztxt_chunk(keyword: &[u8], text: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(text).unwrap();
    let packed = encoder.finish().unwrap();
    let mut data = keyword.to_vec();
    data.extend_from_slice(&[0, 0]);
    data.extend_from_slice(&packed);
    png_chunk(b"zTXt", &data)
}

/// `png` with `chunks` between the image data and `IEND`.
fn with_chunks_before_iend(png: &[u8], chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut at = 8;
    loop {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        if &png[at + 4..at + 8] == b"IEND" {
            let mut out = png[..at].to_vec();
            for c in chunks {
                out.extend_from_slice(c);
            }
            out.extend_from_slice(&png[at..]);
            return out;
        }
        at += len + 12;
    }
}

#[test]
fn stepped_png_import_copies_a_multi_tile_image_in_bounded_steps() {
    // Four tiles (300 × 260 at 256 px), with a compressed text chunk after the image data.
    let mut img = noise(300, 260, ChannelLayout::Rgba, SampleType::U16);
    img.meta.dpi = Some((240.0, 240.0));
    let base = encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
    let text = "A".repeat(120_000);
    let bytes = with_chunks_before_iend(&base, &[ztxt_chunk(b"Comment", text.as_bytes())]);
    assert_stepped_equals_import("tiles.png", &bytes, "multi-tile with compressed text");

    let expected = whole("tiles.png", &bytes);
    let (got, calls) = stepped_zero_budget("tiles.png", &bytes);
    assert_same(&got.document, &expected.document, "zero budget");
    assert_eq!(got.warnings, expected.warnings);
    assert_eq!(got.document.metadata.text, vec![("Comment".into(), text)], "the compressed text survives the steps");
    // One call per row, then its copy bands, its prune batch and the document step, each a call
    // of its own with an empty budget.
    assert!(calls >= 260 + 4, "{} calls: an empty budget must not make one step do it all", calls);
}

#[test]
fn only_a_plain_png_is_stepped() {
    let png = Arc::new(encode(&noise(4, 4, ChannelLayout::Rgb, SampleType::U8), Format::Png, &EncodeOptions::default()).unwrap());
    assert!(PngImport::new("a.png", &png).unwrap().is_some());
    // The same bytes under a name that sends them to another importer, or another format.
    assert!(PngImport::new("a.pcraft", &png).unwrap().is_none());
    let jpeg = Arc::new(encode(&noise(4, 4, ChannelLayout::Rgb, SampleType::U8), Format::Jpeg, &EncodeOptions::default()).unwrap());
    assert!(PngImport::new("a.jpg", &jpeg).unwrap().is_none());
    assert!(PngImport::new("a.svg", &Arc::new(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec())).unwrap().is_none());
}

#[test]
fn a_damaged_png_fails_in_steps_as_it_does_whole() {
    let bytes = encode(&noise(64, 64, ChannelLayout::Rgb, SampleType::U8), Format::Png, &EncodeOptions::default()).unwrap();
    for cut in [30, bytes.len() / 2] {
        let cut = Arc::new(bytes.get(..cut).unwrap().to_vec());
        let whole = import_with_options_and_interrupt("a.png", &cut, &ImportOptions::default(), &Interrupt::NONE);
        let stepped = PngImport::new("a.png", &cut).and_then(|import| {
            let mut import = import.expect("a PNG by its signature");
            loop {
                if let Some(r) = import.step(&mut || true)? {
                    return Ok(r);
                }
            }
        });
        assert!(whole.is_err() && stepped.is_err(), "cut at {}", cut.len());
    }
}

/// An `eXIf` chunk (little-endian) holding only the orientation `o`.
fn exif_chunk(o: u16) -> Vec<u8> {
    let mut t = b"II*\0".to_vec();
    t.extend_from_slice(&8u32.to_le_bytes());
    t.extend_from_slice(&1u16.to_le_bytes());
    for v in [0x0112u16, 3] {
        t.extend_from_slice(&v.to_le_bytes());
    }
    t.extend_from_slice(&1u32.to_le_bytes());
    t.extend_from_slice(&o.to_le_bytes());
    t.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    png_chunk(b"eXIf", &t)
}

/// `png` with `chunk` just before its image data.
fn with_chunk_before_idat(png: &[u8], chunk: &[u8]) -> Vec<u8> {
    let at = png.windows(4).position(|w| w == b"IDAT").unwrap() - 4;
    let mut out = png[..at].to_vec();
    out.extend_from_slice(chunk);
    out.extend_from_slice(&png[at..]);
    out
}

/// Drives `bytes` to the end a row at a time; true if the import streamed at the start.
fn streams_at_start(bytes: &[u8]) -> bool {
    let bytes = Arc::new(bytes.to_vec());
    PngImport::new("a.png", &bytes).unwrap().unwrap().streams()
}

/// A plain PNG streams its rows into the layer; a file that can't (interlaced, or stored in
/// another orientation) is decoded whole.
#[test]
fn a_plain_png_streams_and_other_files_are_decoded_whole() {
    let img = noise(40, 30, ChannelLayout::Rgb, SampleType::U8);
    let plain = encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
    assert!(streams_at_start(&plain));
    let interlaced = encode(&img, Format::Png, &EncodeOptions { png_interlaced: true, ..Default::default() }).unwrap();
    assert!(!streams_at_start(&interlaced));
    assert!(!streams_at_start(&with_chunk_before_idat(&plain, &exif_chunk(6))));
    // An upright eXIf streams; one after the image data is only seen at the end.
    assert!(streams_at_start(&with_chunk_before_idat(&plain, &exif_chunk(1))));
    assert!(streams_at_start(&with_chunks_before_iend(&plain, &[exif_chunk(6)])));
}

/// A file with an orientation turns upright in bands and gives the document the one-shot import
/// gives, whether the `eXIf` comes before the image data (decoded whole from the start) or after
/// it (streamed, then decoded whole), for every orientation, and a tall file takes many calls.
#[test]
fn an_oriented_png_is_turned_upright_in_steps_like_import() {
    for (layout, sample) in [(ChannelLayout::Gray, SampleType::U8), (ChannelLayout::Rgba, SampleType::U16)] {
        let mut img = noise(70, 150, layout, sample);
        img.meta.dpi = Some((120.0, 120.0));
        img.icc = Some(vec![7; 40]);
        let base = encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
        for o in 2..=8u16 {
            for late in [false, true] {
                let bytes = if late { with_chunks_before_iend(&base, &[exif_chunk(o)]) } else { with_chunk_before_idat(&base, &exif_chunk(o)) };
                let what = format!("{layout:?} {sample:?} orientation {o}, eXIf {}", if late { "after" } else { "before" });
                let expected = whole("o.png", &bytes);
                assert_eq!(expected.document.size.width, if o >= 5 { 150 } else { 70 }, "{what}: upright");
                let (got, calls) = stepped_zero_budget("o.png", &bytes);
                assert_same(&got.document, &expected.document, &what);
                assert_eq!(got.warnings, expected.warnings, "{what}");
                // 150 rows decoded one call each, the turn in bands of 64 output rows, the copy.
                assert!(calls > 150 + 2, "{what}: {calls} calls");
                assert_stepped_equals_import("o.png", &bytes, &what);
            }
        }
    }
}

/// A late `eXIf` sends a streamed import back to a whole decode, and the import says so.
#[test]
fn a_late_exif_ends_the_streaming() {
    let base = encode(&noise(20, 20, ChannelLayout::Rgb, SampleType::U8), Format::Png, &EncodeOptions::default()).unwrap();
    let bytes = Arc::new(with_chunks_before_iend(&base, &[exif_chunk(3)]));
    let mut import = PngImport::new("a.png", &bytes).unwrap().unwrap();
    assert!(import.streams());
    while import.streams() {
        assert!(import.step(&mut || false).unwrap().is_none());
    }
    // From here the whole image is decoded.
    assert!(import.step(&mut || false).unwrap().is_none());
    assert!(!import.streams());
}

/// A streamed import with an ICC profile, text and a resolution is the one-shot import, at every
/// depth and layout, 16-bit included.
#[test]
fn a_streamed_png_with_a_profile_and_text_equals_import() {
    for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
        for sample in [SampleType::U8, SampleType::U16] {
            let mut img = noise(300, 270, layout, sample);
            img.icc = Some(vec![9; 60]);
            img.meta.dpi = Some((200.0, 200.0));
            img.meta.text.push(("Title".into(), "streamed".into()));
            let bytes = encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
            assert!(streams_at_start(&bytes));
            assert_stepped_equals_import("s.png", &bytes, &format!("{layout:?} {sample:?} with a profile"));
            let (got, _) = stepped("s.png", &bytes, 0);
            assert_eq!(got.document.icc_profile.as_deref().map(Vec::as_slice), Some(&[9u8; 60][..]));
        }
    }
}
