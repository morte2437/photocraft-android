use super::*;
use photocraft_color::{ColorMode, SampleType};

fn reference(source: &Surface, area: Rect) -> Surface {
    let data: Vec<f32> = source.read_region(area).chunks_exact(source.channels()).map(|p| 1.0 - p[0]).collect();
    let mut out = Surface::new(PixelFormat::GRAY8);
    out.write_region(area, &data);
    out.prune();
    out
}

#[test]
fn inverse_matches_float_reference_and_clips_boundary_tiles() {
    // Negative, misaligned and empty bounds, including stored pixels beyond the canvas.
    for area in [Rect::new(-17, -11, 530, 270), Rect::new(0, 0, 512, 256), Rect::new(3, 5, 4, 6), Rect::EMPTY] {
        for sample in SampleType::ALL {
            for default in [0.0, 0.25, 1.0] {
                let fmt = PixelFormat::new(ColorMode::Grayscale, sample, false);
                let mut source = Surface::with_default(fmt, &[default]);
                source.fill_rect(Rect::new(-100, -100, 260, 120), &[1.0]);
                // Every encoded GRAY8 value, including feathered coverage at tile seams.
                let values: Vec<f32> = (0..256).map(|v| v as f32 / 255.0).collect();
                source.write_region(Rect::new(240, 125, 496, 126), &values);
                source.write_pixel(750, 280, &[0.7]);
                let got = inverse_surface(&source, area);
                let want = reference(&source, area);
                let window = Rect::new(-30, -20, 800, 290);
                assert_eq!(got.to_interleaved(window), want.to_interleaved(window), "{sample:?} {default} {area:?}");
                assert_eq!(got.default_bytes(), &[0]);
                assert_eq!(got.tile_count(), want.tile_count(), "zero tiles must be pruned");
            }
        }
    }
}

#[test]
fn inverse_preserves_sparse_storage_and_shared_payloads() {
    let area = Rect::new(0, 0, 2048, 1024);
    let empty = Surface::new(PixelFormat::GRAY8);
    let selected = inverse_surface(&empty, area);
    assert_eq!(selected.tile_count(), 32);
    let full = selected.tile(TileCoord::new(0, 0)).unwrap();
    assert!(selected.tiles().all(|(_, t)| Arc::ptr_eq(t, full)), "missing tiles share one solid payload");
    let cleared = inverse_surface(&selected, area);
    assert_eq!(cleared.tile_count(), 0, "fully selected tiles disappear");

    let mut mixed = Surface::new(PixelFormat::GRAY8);
    mixed.write_pixel(10, 11, &[0.25]);
    let payload = mixed.tile(TileCoord::new(0, 0)).unwrap().clone();
    mixed.put_tiles(area.tiles().map(|c| (c, payload.clone())));
    let snapshot = mixed.clone();
    let inverted = inverse_surface(&mixed, area);
    let result = inverted.tile(TileCoord::new(0, 0)).unwrap();
    assert!(inverted.tiles().all(|(_, t)| Arc::ptr_eq(t, result)), "shared mixed tiles are inverted once");
    assert!(!Arc::ptr_eq(result, &payload));
    assert_eq!(mixed.to_interleaved(area), snapshot.to_interleaved(area));
    assert_eq!(inverse_surface(&inverted, area).to_interleaved(area), mixed.to_interleaved(area));
}

#[test]
fn inverse_of_unpruned_zero_tiles_is_shared_full() {
    let mut source = Surface::new(PixelFormat::GRAY8);
    let area = Rect::new(0, 0, 768, 256);
    source.fill_rect(Rect::new(0, 0, 256, 256), &[0.0]);
    source.fill_rect(Rect::new(256, 0, 512, 256), &[1.0]);
    let out = inverse_surface(&source, area);
    assert_eq!(out.tile_count(), 2);
    assert!(out.tile(TileCoord::new(1, 0)).is_none());
    assert!(Arc::ptr_eq(out.tile(TileCoord::new(0, 0)).unwrap(), out.tile(TileCoord::new(2, 0)).unwrap()));
}

#[test]
fn inverse_clips_at_coordinate_limits_and_uses_first_channel() {
    for area in [Rect::new(i32::MAX - 255, 0, i32::MAX, 256), Rect::new(0, i32::MAX - 255, 256, i32::MAX), Rect::new(i32::MIN, -1, i32::MIN + 3, 2)] {
        let source = Surface::new(PixelFormat::GRAY8);
        let out = inverse_surface(&source, area);
        assert_eq!(out.to_interleaved(area), reference(&source, area).to_interleaved(area));
        assert_eq!(out.sample_channel(area.x1, area.y0, 0), 0.0);
        assert_eq!(out.sample_channel(area.x0, area.y1, 0), 0.0);
    }
    let area = Rect::new(0, 0, 513, 260);
    let mut source = Surface::with_default(PixelFormat::RGBA32F, &[0.25, 0.5, 0.75, 1.0]);
    source.fill_rect(Rect::new(10, 10, 400, 200), &[0.75, 1.0, 0.0, 0.5]);
    assert_eq!(inverse_surface(&source, area).to_interleaved(area), reference(&source, area).to_interleaved(area));
}
