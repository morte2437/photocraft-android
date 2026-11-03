//! #10030: decoding a smart object's source (what its first re-render does) composites it in
//! tile-aligned blocks, so the peak memory growth stays a small multiple of the source's pixels, not the 32
//! bytes a pixel the full-size RGBA buffer and its f32 copy took on top of the decoded document.
//!
//! The one test here runs in its own process: it reads the process's peak RSS (`VmHWM`, Linux
//! only) and nothing else may allocate meanwhile.

#![cfg(target_os = "linux")]

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
use photocraft_engine::{Session, smart_cmds};
use serde_json::json;

fn peak_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

const SIDE: u32 = 4096;

fn png() -> Vec<u8> {
    let px = (0..SIDE * SIDE).flat_map(|i| {
        let (x, y) = (i % SIDE, i / SIDE);
        [(x * 5 + y) as u8, ((y * 7) ^ x) as u8, ((x * y) % 251) as u8, 255 - ((x / 64 + y / 64) % 4 * 40) as u8]
    });
    let img = Image::from_raw(SIDE, SIDE, ChannelLayout::Rgba, SampleType::U8, px.collect()).unwrap();
    photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap()
}

#[test]
fn decoding_a_smart_object_source_stays_within_16_bytes_a_source_pixel() {
    let bytes = png();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let fmt = s.active().unwrap().doc.pixel_format();
    // Writing 5 resets the peak RSS to the current RSS; skip where the kernel doesn't allow it.
    if std::fs::write("/proc/self/clear_refs", "5").is_err() {
        return;
    }
    let Some(before) = peak_rss_bytes() else { return };
    let img = smart_cmds::source_image("big.png", &bytes, fmt).unwrap();
    let grown = peak_rss_bytes().unwrap().saturating_sub(before);
    assert_eq!(img.bounds.width(), SIDE);

    let limit = 16 * u64::from(SIDE) * u64::from(SIDE);
    eprintln!("peak RSS growth {} MiB for {} px ({:.1} B/px)", grown >> 20, SIDE * SIDE, grown as f64 / f64::from(SIDE * SIDE));
    assert!(grown < limit, "peak RSS grew by {grown} bytes, over {limit} (16 B/px)");
}
