//! TIFFs open a group of rows at a time straight into the layer's tiles (#8169): the document
//! is the one decoding the whole image gives, for every layout and depth, and an image over the
//! old 2^30 px limit opens.

use photocraft_codecs::{self as codecs, ChannelLayout, EncodeOptions, Format, Image, SampleType, TiffCompression};
use photocraft_doc::LayerContent;

/// A smooth pattern with some detail, in `layout` and `sample`.
fn pattern(w: u32, h: u32, layout: ChannelLayout, sample: SampleType) -> Image {
    let ch = layout.channels();
    let px: Vec<f32> = (0..h).flat_map(|y| (0..w).flat_map(move |x| (0..ch).map(move |c| ((x * 7 + y * 13 + c as u32 * 29) % 256) as f32 / 255.0))).collect();
    Image::from_normalized(w, h, layout, sample, &px).unwrap()
}

fn tiff(img: &Image, compression: TiffCompression) -> Vec<u8> {
    codecs::encode(img, Format::Tiff, &EncodeOptions { tiff_compression: compression, ..Default::default() }).unwrap()
}

/// The layer's pixels as the codec's interleaved bytes.
fn layer_bytes(r: &photocraft_io::ImportResult) -> Vec<u8> {
    let d = &r.document;
    let [layer] = &d.layers[..] else { panic!("{} layers", d.layers.len()) };
    let LayerContent::Raster(s) = &layer.content else { panic!("not a raster layer") };
    s.to_interleaved(d.bounds())
}

#[test]
fn every_layout_opens_as_the_whole_image_decode_does() {
    let cases = [
        (ChannelLayout::Rgb, SampleType::U8),
        (ChannelLayout::Rgba, SampleType::U16),
        (ChannelLayout::Gray, SampleType::U8),
        (ChannelLayout::GrayA, SampleType::F32),
        (ChannelLayout::Cmyk, SampleType::U8),
        (ChannelLayout::CmykA, SampleType::U16),
        (ChannelLayout::Lab, SampleType::U16),
        (ChannelLayout::Rgb, SampleType::F16),
    ];
    for (layout, sample) in cases {
        for compression in [TiffCompression::None, TiffCompression::Lzw] {
            let what = format!("{layout:?} {sample:?} {compression:?}");
            let img = pattern(301, 257, layout, sample);
            let bytes = tiff(&img, compression);
            let r = photocraft_io::import("t.tif", &bytes).unwrap_or_else(|e| panic!("{what}: {e}"));
            let decoded = codecs::decode(&bytes).unwrap();
            let target = match layout {
                ChannelLayout::Gray | ChannelLayout::GrayA => ChannelLayout::GrayA,
                ChannelLayout::Cmyk | ChannelLayout::CmykA => ChannelLayout::CmykA,
                ChannelLayout::Lab | ChannelLayout::LabA => ChannelLayout::LabA,
                _ => ChannelLayout::Rgba,
            };
            let stored = decoded.sample_type();
            let depth = if stored == SampleType::F16 { SampleType::F32 } else { stored };
            assert!(layer_bytes(&r) == decoded.convert(target, depth).data(), "{what}: pixels");
            let d = &r.document;
            assert_eq!((d.size.width, d.size.height), (301, 257), "{what}");
            assert_eq!(d.layers[0].name, if layout.has_alpha() { "Layer 0" } else { "Background" }, "{what}");
            assert_eq!(d.icc_profile.as_deref().map(Vec::as_slice), decoded.icc.as_deref(), "{what}");
            assert_eq!(r.warnings.iter().any(|w| w.contains("16-bit float")), stored == SampleType::F16, "{what}: {:?}", r.warnings);
        }
    }
}

/// A file stored as one strip is one group, the whole image, converted to the layer in rounds of
/// rows: the document is the one the whole-image decode gives (#8536 review).
#[test]
fn a_single_strip_tiff_opens_as_the_whole_image_decode_does() {
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;
    let (w, h) = (1531, 2049);
    let img = pattern(w, h, ChannelLayout::Rgb, SampleType::U8);
    let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
    z.write_all(img.data()).unwrap();
    let strip = z.finish().unwrap();
    let mut f = b"II*\0\0\0\0\0".to_vec();
    f.extend_from_slice(&strip);
    let bytes = rgb_tiff_directory(f, w, h, h, &[8], &[strip.len() as u32]);
    let r = photocraft_io::import("one-strip.tif", &bytes).unwrap();
    let decoded = codecs::decode(&bytes).unwrap();
    assert_eq!(decoded.data(), img.data());
    assert!(layer_bytes(&r) == decoded.convert(ChannelLayout::Rgba, SampleType::U8).data(), "pixels");
}

/// Peak resident memory of this process so far (Linux), in bytes.
fn peak_rss() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: u64 = status.lines().find(|l| l.starts_with("VmHWM:"))?.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

/// An RGB8 TIFF of `w`×`h` with Deflate strips of 64 rows, written a strip at a time: a few
/// horizontal bands of colour with a ramp, which compresses well.
fn big_rgb_tiff(w: u32, h: u32) -> Vec<u8> {
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;
    let rows = 64u32;
    let mut f = b"II*\0\0\0\0\0".to_vec();
    let mut offsets = Vec::new();
    let mut counts = Vec::new();
    let mut row = vec![0u8; w as usize * 3];
    for y0 in (0..h).step_by(rows as usize) {
        let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
        for y in y0..(y0 + rows).min(h) {
            let band = (y / 997) as u8;
            for (x, px) in row.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                px.copy_from_slice(&[band.wrapping_mul(40), (x / 1024) as u8, 200]);
            }
            z.write_all(&row).unwrap();
        }
        let strip = z.finish().unwrap();
        offsets.push(f.len() as u32);
        counts.push(strip.len() as u32);
        f.extend_from_slice(&strip);
    }
    rgb_tiff_directory(f, w, h, rows, &offsets, &counts)
}

/// An all-black RGB8 TIFF of `w`×`h` whose Deflate strips of 64 rows all point at one
/// compressed strip, padded with zeros just past what the decompression-bomb guard asks for: a
/// valid file of tens of MB declaring hundreds of GB of pixels (#8536 review).
#[cfg(target_os = "linux")]
fn black_rgb_tiff(w: u32, h: u32) -> Vec<u8> {
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;
    let rows = 64u32;
    let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
    z.write_all(&vec![0u8; w as usize * 3 * rows as usize]).unwrap();
    let strip = z.finish().unwrap();
    let mut f = b"II*\0\0\0\0\0".to_vec();
    let at = f.len() as u32;
    f.extend_from_slice(&strip);
    let stored = u64::from(w) * u64::from(h) * 3;
    f.resize((stored / 4096 + (1 << 20)) as usize, 0);
    let n = h.div_ceil(rows) as usize;
    rgb_tiff_directory(f, w, h, rows, &vec![at; n], &vec![strip.len() as u32; n])
}

/// `f` (a little-endian header and the strip data) with the strip tables and the directory of
/// an RGB8 Deflate image appended.
fn rgb_tiff_directory(mut f: Vec<u8>, w: u32, h: u32, rows: u32, offsets: &[u32], counts: &[u32]) -> Vec<u8> {
    let arrays_at = f.len() as u32;
    for v in offsets.iter().chain(counts) {
        f.extend_from_slice(&v.to_le_bytes());
    }
    let bits_at = f.len() as u32;
    f.extend_from_slice(&[8, 0, 8, 0, 8, 0]);
    let ifd = f.len() as u32;
    f[4..8].copy_from_slice(&ifd.to_le_bytes());
    let n = offsets.len() as u32;
    let entries: [(u16, u16, u32, u32); 10] = [
        (256, 4, 1, w),
        (257, 4, 1, h),
        (258, 3, 3, bits_at),
        (259, 3, 1, 8),
        (262, 3, 1, 2),
        // A single strip's offset and count fit in the entry itself.
        (273, 4, n, if let [o] = offsets { *o } else { arrays_at }),
        (277, 3, 1, 3),
        (278, 4, 1, rows),
        (279, 4, n, if let [c] = counts { *c } else { arrays_at + 4 * n }),
        (284, 3, 1, 1),
    ];
    f.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, ty, count, v) in entries {
        f.extend_from_slice(&tag.to_le_bytes());
        f.extend_from_slice(&ty.to_le_bytes());
        f.extend_from_slice(&count.to_le_bytes());
        f.extend_from_slice(&v.to_le_bytes());
    }
    f.extend_from_slice(&0u32.to_le_bytes());
    f
}

/// Opt-in, release: `cargo test --release -p photocraft-io --test it -- tiff_banded_import:: --ignored
/// --nocapture`. Opens a 35,000 × 35,000 px TIFF (1.2 GP, the size from #8169's second report;
/// it failed with "1225000000 pixels exceed max 1073741824"). `BANNER=1` opens the 117,504 ×
/// 22,464 px banner from #8456 instead (2.64 GP; about 11 GB of RAM), `SIZE=<w>x<h>` any size.
#[test]
#[ignore = "needs about 5 GB of RAM (11 GB with BANNER=1); run in release"]
fn opens_a_tiff_over_a_gigapixel() {
    let size = std::env::var("SIZE").ok().and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))));
    let (w, h): (u32, u32) = size.unwrap_or(if std::env::var_os("BANNER").is_some() { (117_504, 22_464) } else { (35_000, 35_000) });
    let t = std::time::Instant::now();
    let bytes = big_rgb_tiff(w, h);
    eprintln!("built a {w}x{h} TIFF of {} MB in {:.1?}", bytes.len() / 1_000_000, t.elapsed());
    let before = peak_rss();
    let t = std::time::Instant::now();
    let r = photocraft_io::import("big.tif", &bytes).unwrap();
    let took = t.elapsed();
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (w, h));
    let LayerContent::Raster(s) = &d.layers[0].content else { panic!() };
    for (x, y) in [(0, 0), (w - 1, h - 1), (w / 2, h / 2), (5000.min(w - 1), 21_000.min(h - 1))] {
        let band = (y / 997) as u8;
        let want = [band.wrapping_mul(40), (x / 1024) as u8, 200, 255];
        assert_eq!(s.to_interleaved(photocraft_geom::Rect::new(x as i32, y as i32, x as i32 + 1, y as i32 + 1)), want, "pixel {x},{y}");
    }
    let layer_gb = u64::from(w) * u64::from(h) * 4;
    eprintln!(
        "opened in {took:.1?}; peak RSS {:.2} GB before, {:.2} GB after; the RGBA layer is {:.2} GB",
        before.unwrap_or(0) as f64 / 1e9,
        peak_rss().unwrap_or(0) as f64 / 1e9,
        layer_gb as f64 / 1e9
    );
}

/// A file that declares more pixels than the machine can hold is refused before the layer is
/// allocated (#8536 review). Linux overcommits, so allocating the tiles succeeded and writing them
/// had the process killed (with every open document) instead of returning an error.
#[test]
#[cfg(target_os = "linux")]
fn a_tiff_larger_than_free_memory_is_refused() {
    let (w, h) = (300_000, 300_000);
    let bytes = black_rgb_tiff(w, h);
    let layer = u64::from(w) * u64::from(h) * 4;
    let free = photocraft_io::memory::available().unwrap();
    assert!(free < layer, "this machine has {free} bytes free, enough for the {layer} byte layer");
    let e = photocraft_io::import("huge.tif", &bytes).map(|_| ()).unwrap_err().to_string();
    assert!(e.contains("not enough memory to open this 300000 × 300000 px image"), "{e}");
}
