#![cfg(not(target_arch = "wasm32"))]

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer};
use photocraft_geom::{Size, TileCoord};
use photocraft_raster::spill;
use std::io::{Read, Seek, SeekFrom, Write};

#[test]
fn retry_after_failed_hashing_never_reuses_placeholder_hashes() {
    let dir = std::env::temp_dir().join(format!("photocraft-format-scratch-retry-{}", std::process::id()));
    spill::configure(Some(&dir), 1).unwrap();
    let mut doc = Document::new("Retry", Size::new(512, 256), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("Pixels", PixelFormat::RGBA8);
    for i in 0..2 {
        let mut bytes = layer.surface_mut().unwrap().tile_mut(TileCoord::new(i, 0)).bytes_mut();
        for (j, byte) in bytes.iter_mut().enumerate() {
            *byte = (j as u8).wrapping_mul(17).wrapping_add((i * 43) as u8);
        }
    }
    doc.layers.push(layer);
    spill::settle();
    assert_eq!(spill::stats().resident_bytes, 0);
    let mut file = spill::scratch_handle_for_tests().unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut good = Vec::new();
    file.read_to_end(&mut good).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&vec![0; good.len()]).unwrap();
    file.sync_all().unwrap();

    let mut writer = photocraft_format::PcraftWriter::new();
    // This save discovers the failure itself, after entering the hash collection stage.
    assert!(writer.save_zip(&doc, &Default::default()).is_err());
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&good).unwrap();
    file.sync_all().unwrap();
    for (_, tile) in doc.layers[0].surface().unwrap().tiles() {
        tile.try_bytes().unwrap();
    }
    assert!(spill::check_integrity().is_ok());
    let (saved, _) = writer.save_zip(&doc, &Default::default()).unwrap();
    let loaded = photocraft_format::load_from_bytes(&saved).unwrap();
    assert_eq!(loaded.layers, doc.layers);
    drop((doc, loaded, writer, file));
    spill::configure(None, 0).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
