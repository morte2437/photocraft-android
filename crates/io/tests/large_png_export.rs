//! PNG export of a multi-gigapixel print (#8456): a 136 ft × 26 ft banner at 72 ppi is
//! 117504 × 22464 px (2.64 GP, 7.9 GB of 8-bit RGB), past the 2^30 px a whole-image export could
//! hold. It is written a band of rows at a time, its zlib stream split over IDAT chunks.

use std::io::Read;

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_io::{ExportOptions, export};
use photocraft_raster::Surface;

/// The IHDR fields and the IDAT payloads of `png`.
fn chunks(png: &[u8]) -> ((u32, u32, u8, u8), Vec<&[u8]>) {
    let (mut at, mut ihdr, mut idat) = (8, None, Vec::new());
    while at + 12 <= png.len() {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let data = &png[at + 8..at + 8 + len];
        match &png[at + 4..at + 8] {
            b"IHDR" => ihdr = Some((u32::from_be_bytes(data[0..4].try_into().unwrap()), u32::from_be_bytes(data[4..8].try_into().unwrap()), data[8], data[9])),
            b"IDAT" => idat.push(data),
            _ => {}
        }
        at += 12 + len;
    }
    (ihdr.unwrap(), idat)
}

/// Run with `cargo test --release -p photocraft-io --test large_png_export -- --ignored`; with
/// `COMPOSITE=1` a translucent layer on top makes the export composite the banner.
/// Measured 2026-10-22 (20 threads): 12.3 s and 0.49 GB peak RSS, composited 29 s and 1.2 GB; the
/// whole-image export took 12.9 s and 8.0 GB, composited 43 s and 10.3 GB.
#[test]
#[ignore = "encodes 7.9 GB of pixels; run in release"]
fn a_banner_over_two_gigapixels_exports_as_png() {
    const W: u32 = 117_504;
    const H: u32 = 22_464;
    let mut doc = Document::new("Tent banner", Size::new(W, H), ColorMode::Rgb, SampleType::U8);
    let mut s = Surface::new(doc.pixel_format());
    s.fill_rect(doc.bounds(), &[1.0, 1.0, 1.0, 1.0]);
    s.fill_rect(Rect::new(0, 15_000, W as i32, H as i32), &[0.1, 0.45, 0.8, 1.0]);
    s.fill_rect(Rect::new(40_000, 2_000, 77_504, 14_000), &[0.55, 0.05, 0.08, 1.0]);
    doc.layers.push(Layer::new("Background", LayerContent::Raster(s)));
    if std::env::var_os("COMPOSITE").is_some() {
        let mut t = Surface::new(doc.pixel_format());
        t.fill_rect(Rect::new(10_000, 5_000, 100_000, 20_000), &[0.9, 0.7, 0.1, 0.6]);
        doc.layers.push(Layer::new("Text", LayerContent::Raster(t)));
    }

    let started = std::time::Instant::now();
    let r = export(&doc, "banner.png", &ExportOptions::default()).unwrap();
    eprintln!("exported {} MB in {:.1} s; warnings {:?}", r.bytes.len() >> 20, started.elapsed().as_secs_f64(), r.warnings);

    let ((w, h, depth, color), idat) = chunks(&r.bytes);
    assert_eq!((w, h, depth, color), (W, H, 8, 2), "8-bit RGB");
    // The zlib stream inflates (Adler-32 checked) to a filter byte and W RGB pixels per row.
    let joined = idat.iter().flat_map(|c| c.iter().copied()).collect::<Vec<u8>>();
    let mut z = flate2::read::ZlibDecoder::new(joined.as_slice());
    let (mut buf, mut total) = (vec![0u8; 1 << 20], 0u64);
    loop {
        let n = z.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    assert_eq!(total, u64::from(H) * (1 + u64::from(W) * 3));
}
