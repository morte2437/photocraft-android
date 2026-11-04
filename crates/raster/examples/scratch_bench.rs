//! 24 MP release benchmark: identical pixel access with RAM-only and disk-backed tiles.
use photocraft_color::PixelFormat;
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Surface, spill};
use std::time::Instant;

fn run(dir: Option<&std::path::Path>) -> Result<(), String> {
    spill::configure(dir, 16 << 20)?;
    let start = Instant::now();
    let area = Rect::from_xywh(0, 0, 6000, 4000);
    let mut surface = Surface::new(PixelFormat::RGBA8);
    let mut random = 0x1234abcd_u32;
    for tc in area.tiles() {
        let mut bytes = surface.tile_mut(tc).bytes_mut();
        for byte in bytes.iter_mut() {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            *byte = random as u8;
        }
    }
    spill::settle();
    let build_ms = start.elapsed().as_secs_f64() * 1000.0;
    let resident = spill::stats().resident_bytes;
    let disk = spill::stats().scratch_bytes;
    let read = Instant::now();
    let mut checksum = 0u64;
    // Rows exercise the same locks and conversions the CPU renderer uses.
    let mut row = vec![[0; 4]; 6000];
    for y in 0..4000 {
        surface.read_rgba8_into(Rect::from_xywh(0, y, 6000, 1), &mut row);
        for pixel in &row {
            checksum = checksum.wrapping_add(u32::from_le_bytes(*pixel) as u64);
        }
    }
    spill::check_integrity()?;
    println!(
        "mode={} build_ms={build_ms:.2} read_ms={:.2} resident_bytes={resident} scratch_bytes={disk} checksum={checksum} tile_side={TILE_SIZE}",
        if dir.is_some() { "scratch" } else { "ram" },
        read.elapsed().as_secs_f64() * 1000.0
    );
    drop(surface);
    spill::configure(None, 0)?;
    Ok(())
}

fn main() -> Result<(), String> {
    run(None)?;
    let dir = std::env::temp_dir().join(format!("photocraft-24mp-{}", std::process::id()));
    run(Some(&dir))?;
    std::fs::remove_dir(&dir).map_err(|e| e.to_string())
}
