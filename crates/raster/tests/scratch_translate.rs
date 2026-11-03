//! Upstream's shared fills and pixel translations must also work with paged tile bytes.

use photocraft_color::{PixelFormat, SampleType};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Surface, spill};

#[test]
fn translations_and_shared_fills_preserve_spilled_pixels_at_every_depth() {
    let dir = std::env::temp_dir().join(format!("photocraft-translate-spill-{}", std::process::id()));
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let format = PixelFormat::RGBA8.with_sample(depth);
        let tile_bytes = (TILE_SIZE * TILE_SIZE) as usize * format.bytes_per_pixel();
        spill::configure(Some(&dir), tile_bytes * 2).unwrap();
        let bounds = Rect::new(-17, -21, 281, 271);
        let pixels: Vec<f32> = (0..bounds.size().area() as usize).flat_map(|i| [(i % 127) as f32 / 126.0, (i % 59) as f32 / 58.0, 0.75, 1.0]).collect();
        let mut original = Surface::new(format);
        original.write_region(bounds, &pixels);
        let expected = original.to_interleaved(bounds);
        spill::settle();
        assert!(spill::stats().resident_bytes <= tile_bytes * 2);
        let reloads = spill::stats().reloads;
        for (dx, dy) in [(19, -31), (256, -512)] {
            let mut moved = original.translated(dx, dy, original.content_bounds());
            assert_eq!(moved.to_interleaved(bounds.translate(dx, dy)), expected, "{depth:?} ({dx}, {dy})");
            moved.fill_rect(Rect::from_xywh(bounds.x0 + dx, bounds.y0 + dy, 4, 4), &[1.0, 0.0, 0.0, 1.0]);
            spill::settle();
            assert_eq!(original.to_interleaved(bounds), expected, "translation edits changed the source");
        }
        assert!(spill::stats().reloads > reloads, "translation must read paged pixels");
        let mut fill = Surface::new(format);
        fill.fill_rect(Rect::new(0, 0, 1024, 1024), &[0.5, 0.25, 0.75, 1.0]);
        assert!(std::sync::Arc::ptr_eq(fill.tile(TileCoord::new(0, 0)).unwrap(), fill.tile(TileCoord::new(3, 3)).unwrap()));
        let snapshot = fill.clone();
        let color = snapshot.pixel(10, 10);
        spill::settle();
        fill.fill_rect(Rect::from_xywh(0, 0, 4, 4), &[1.0, 0.0, 0.0, 1.0]);
        assert_eq!(snapshot.pixel(1, 1), color);
        assert_eq!(fill.pixel(10, 10), color);
        assert_eq!(fill.pixel(1, 1), vec![1.0, 0.0, 0.0, 1.0]);
        spill::check_integrity().unwrap();
    }
    spill::configure(None, 0).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
