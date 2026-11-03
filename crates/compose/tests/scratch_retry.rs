#![cfg(not(target_arch = "wasm32"))]

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Effect, Layer};
use photocraft_geom::{Rect, Size};
use photocraft_raster::spill;
use std::io::{Read, Seek, SeekFrom, Write};

#[test]
fn failed_effect_build_is_not_reused_after_original_pixels_recover() {
    let dir = std::env::temp_dir().join(format!("photocraft-compose-scratch-retry-{}", std::process::id()));
    spill::configure(Some(&dir), 1).unwrap();
    let mut doc = Document::new("Shadow", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
    let mut background = Layer::raster("Background", PixelFormat::RGBA8);
    background.surface_mut().unwrap().fill_rect(Rect::from_xywh(0, 0, 64, 64), &[1.0; 4]);
    let mut foreground = Layer::raster("Shadow", PixelFormat::RGBA8);
    foreground.surface_mut().unwrap().fill_rect(Rect::new(16, 16, 48, 48), &[1.0, 0.0, 0.0, 1.0]);
    foreground.effects.items.push(Effect::default_drop_shadow());
    doc.layers.extend([background, foreground]);
    let expected = photocraft_compose::flatten(&doc).px.clone();
    // Keep the known-good bounds, but force a new effect-map build from the damaged source.
    photocraft_compose::purge_effect_cache();
    spill::settle();
    assert_eq!(spill::stats().resident_bytes, 0);
    let mut file = spill::scratch_handle_for_tests().unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut good = Vec::new();
    file.read_to_end(&mut good).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&vec![0; good.len()]).unwrap();
    file.sync_all().unwrap();
    drop(photocraft_compose::flatten(&doc));
    assert!(spill::check_integrity().is_err());

    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&good).unwrap();
    file.sync_all().unwrap();
    for layer in &doc.layers {
        for (_, tile) in layer.surface().unwrap().tiles() {
            tile.try_bytes().unwrap();
        }
    }
    assert!(spill::check_integrity().is_ok());
    assert_eq!(photocraft_compose::flatten(&doc).px, expected);
    photocraft_compose::purge_effect_cache();
    drop((doc, file));
    spill::configure(None, 0).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
