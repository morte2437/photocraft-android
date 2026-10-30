//! Backend compatibility, including native-format tile data rather than ZIP assembly alone.
use crate::common;

use std::io::Read;

use photocraft_codecs::{ChannelLayout, Format, Image, SampleType as CodecSample};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_format::{SaveOptions, load_from_bytes, read_thumbnail, save_to_bytes};
use photocraft_geom::Size;
use photocraft_io::{PsdExportOptions, document_to_psd_with, import};
use photocraft_psd::Compression;
use photocraft_raster::Rgba8Image;

fn document(depth: SampleType, size: Size) -> Document {
    let mut doc = Document::new("compatibility", size, ColorMode::Rgb, depth);
    doc.layers.push(Layer::new("pixels", LayerContent::Raster(common::pattern(doc.pixel_format(), doc.bounds(), 3, false))));
    doc
}

/// Compare both decoders and generate a legacy stream without flate2 compression.
fn old_and_new_inflate(bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let old = miniz_oxide::inflate::decompress_to_vec_zlib(bytes)
        .unwrap_or_else(|e| panic!("old decoder: {:?}, header {:?}", e.status, &bytes[..bytes.len().min(8)]));
    let legacy = miniz_oxide::deflate::compress_to_vec_zlib(&old, 6);
    for stream in [bytes, legacy.as_slice()] {
        let mut new = Vec::new();
        flate2::read::ZlibDecoder::new(stream).read_to_end(&mut new).unwrap();
        assert_eq!(new, old);
    }
    (old, legacy)
}

/// Read new PNGs and PNGs with legacy-miniz IDAT data through the codec.
fn check_png(bytes: &[u8], image: &Image) {
    assert_eq!(photocraft_codecs::decode(bytes).unwrap().data(), image.data());
    let mut idat = Vec::new();
    let mut legacy_png = bytes[..8].to_vec();
    let mut idat_offset = None;
    let mut offset = 8;
    while offset < bytes.len() {
        let len = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let end = offset + 12 + len;
        if &bytes[offset + 4..offset + 8] == b"IDAT" {
            idat_offset.get_or_insert(legacy_png.len());
            idat.extend_from_slice(&bytes[offset + 8..offset + 8 + len]);
        } else {
            legacy_png.extend_from_slice(&bytes[offset..end]);
        }
        offset = end;
    }
    let (_, legacy) = old_and_new_inflate(&idat);
    let mut chunk = (legacy.len() as u32).to_be_bytes().to_vec();
    chunk.extend_from_slice(b"IDAT");
    chunk.extend_from_slice(&legacy);
    chunk.extend_from_slice(&crc32fast::hash(&chunk[4..]).to_be_bytes());
    let at = idat_offset.unwrap();
    legacy_png.splice(at..at, chunk);
    assert_eq!(photocraft_codecs::decode(&legacy_png).unwrap().data(), image.data());
}

/// New zlib-rs streams and legacy-miniz streams preserve the same document samples.
#[test]
fn png_psd_and_pcraft_decode_identically() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let doc = document(depth, Size::new(257, 131));
        let pixels = doc.layers[0].surface().unwrap().read_region(doc.bounds());
        if depth != SampleType::F32 {
            let sample = if depth == SampleType::U8 { CodecSample::U8 } else { CodecSample::U16 };
            let image = Image::from_normalized(257, 131, ChannelLayout::Rgba, sample, &pixels).unwrap();
            check_png(&photocraft_codecs::encode(&image, Format::Png, &Default::default()).unwrap(), &image);
        }
        let raw_opts = PsdExportOptions { merged_compression: Compression::Raw, merged_matte: false, ..Default::default() };
        let merged = document_to_psd_with(&doc, &raw_opts).0.image_data.data;
        for compression in [Compression::Zip, Compression::ZipPrediction] {
            let opts = PsdExportOptions { merged_compression: compression, merged_matte: false, ..Default::default() };
            let mut file = document_to_psd_with(&doc, &opts).0;
            // The IO exporter normally chooses Raw for float composites; also cover ZIP here.
            if depth == SampleType::F32 {
                file.image_data = photocraft_psd::ImageData::encode(compression, &merged, &file.header).unwrap();
            }
            let bytes = file.to_bytes().unwrap();
            let parsed = photocraft_psd::PsdFile::from_bytes(&bytes).unwrap();
            assert_eq!(parsed.image_data.compression, compression);
            let (inflated, legacy) = old_and_new_inflate(&parsed.image_data.data);
            // The PSD streaming helper uses a different flate2 reader from plane decoding.
            assert_eq!(photocraft_psd::compression::zip_decompress(&legacy, inflated.len()).unwrap(), inflated);
            file.image_data.data = legacy;
            for fixture in [bytes, file.to_bytes().unwrap()] {
                let parsed = photocraft_psd::PsdFile::from_bytes(&fixture).unwrap();
                assert_eq!(parsed.image_data.decode(&parsed.header).unwrap(), merged);
                let back = import("compatibility.psd", &fixture).unwrap().document;
                assert_eq!(back.layers[0].surface().unwrap().read_region(doc.bounds()), pixels);
            }
        }
        let mut thumbnail = Rgba8Image::new(7, 3);
        thumbnail.pixels = (0..84).map(|i| (i * 37) as u8).collect();
        let opts = SaveOptions { thumbnail: Some(thumbnail.clone()), ..Default::default() };
        let bytes = save_to_bytes(&doc, &opts).unwrap();
        assert!(load_from_bytes(&bytes).unwrap() == doc, "all tile bytes, {depth:?}");
        let image = Image::from_u8(7, 3, ChannelLayout::Rgba, thumbnail.pixels).unwrap();
        check_png(&read_thumbnail(&bytes).unwrap().unwrap(), &image);
    }
}

/// PDF export writes its image at the old backend's size or smaller: zlib-rs level 1
/// (`Compression::fast()`) writes static Huffman blocks only, which made PDFs of photos
/// 5-30% bigger than miniz's level 1 did.
#[test]
fn pdf_image_stream_is_no_bigger_than_miniz_fast() {
    let (w, h) = (512u32, 384u32);
    let mut doc = Document::new("photo", Size::new(w, h), ColorMode::Rgb, SampleType::U8);
    // A noisy gradient: smooth like a photo, with sensor-like grain.
    let mut x32 = 0x9E37_79B9u32;
    let mut samples = Vec::with_capacity((w * h * 3) as usize);
    let mut vals = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                x32 ^= x32 << 13;
                x32 ^= x32 >> 17;
                x32 ^= x32 << 5;
                let base = (x * 160 / w + y * 80 / h + c * 20) as i32;
                let v = (base + (x32 % 9) as i32 - 4).clamp(0, 255) as u8;
                samples.push(v);
                vals.push(common::g(v));
            }
            vals.push(1.0);
        }
    }
    let mut surface = photocraft_raster::Surface::new(doc.pixel_format());
    surface.write_region(doc.bounds(), &vals);
    doc.layers.push(Layer::new("photo", LayerContent::Raster(surface)));
    let pdf = photocraft_io::export(&doc, "photo.pdf", &Default::default()).unwrap().bytes;
    let at = pdf.windows(15).position(|s| s == b"/Subtype /Image").unwrap();
    let dict = &pdf[at..];
    let len_at = dict.windows(8).position(|s| s == b"/Length ").unwrap() + 8;
    let digits = dict[len_at..].iter().take_while(|b| b.is_ascii_digit()).count();
    let len: usize = std::str::from_utf8(&dict[len_at..len_at + digits]).unwrap().parse().unwrap();
    let start = dict.windows(7).position(|s| s == b"stream\n").unwrap() + 7;
    let stream = &dict[start..start + len];
    assert_eq!(miniz_oxide::inflate::decompress_to_vec_zlib(stream).unwrap(), samples);
    let old = miniz_oxide::deflate::compress_to_vec_zlib(&samples, 1).len();
    assert!(len <= old, "PDF image stream {len} bytes, miniz level 1 wrote {old}");
}
