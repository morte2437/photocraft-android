//! Disk-backed current pixels and history: lossless undo, and failed reads never commit.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer};
use photocraft_engine::Session;
use photocraft_geom::{Rect, Size, TileCoord};
use photocraft_raster::spill;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Arc;

#[test]
fn scratch_pixels_history_and_failed_save_are_transactional() {
    let dir = std::env::temp_dir().join(format!("photocraft-engine-scratch-{}", std::process::id()));
    let mut session = Session::new();
    session.edit_prefs(|p| {
        p.scratch_disks.disks = vec![photocraft_engine::prefs::ScratchDisk { path: dir.to_string_lossy().into(), enabled: true }];
        p.performance.memory_usage_mb = 1;
    });
    assert!(spill::enabled());
    assert_eq!(session.prefs().performance.history_cap_bytes(), 0);
    let mut doc = Document::new("Scratch fixture", Size::new(2048, 256), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("Pixels", PixelFormat::RGBA8);
    let id = layer.id;
    for i in 0..8 {
        let mut bytes = layer.surface_mut().unwrap().tile_mut(TileCoord::new(i, 0)).bytes_mut();
        for (j, b) in bytes.iter_mut().enumerate() {
            *b = (j as u32).wrapping_mul(17).wrapping_add(i as u32 * 43) as u8;
        }
    }
    doc.layers.push(layer);
    session.add_document(doc, None);
    session.select_layer(id).unwrap();
    let before = session.active().unwrap().doc.clone();
    session
        .edit("Paint", |d, _| {
            d.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(Rect::from_xywh(0, 0, 4, 4), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    spill::settle();
    assert!(session.undo());
    assert_eq!(*session.active().unwrap().doc, *before);
    assert!(session.redo());
    assert_eq!(session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().pixel(1, 1), vec![1.0, 0.0, 0.0, 1.0]);
    spill::settle();

    // Force all tiles to disk, including the tile the failed edit will touch.
    spill::configure(Some(&dir), 1).unwrap();
    spill::settle();
    assert_eq!(spill::stats().resident_bytes, 0);
    // Damage only this test's scratch file; retain its exact contents to restore the backing.
    let mut file = spill::scratch_handle_for_tests().unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut good = Vec::new();
    file.read_to_end(&mut good).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&vec![0; good.len()]).unwrap();
    file.sync_all().unwrap();
    let retained = session.active().unwrap().doc.clone();
    let revision = session.active().unwrap().revision;
    let failed = session.edit("Failed paint", |d, active| {
        let discarded = Layer::raster("Discarded", PixelFormat::RGBA8);
        *active = Some(discarded.id);
        d.layers.push(discarded);
        d.layer_mut(id).unwrap().surface_mut().unwrap().fill_rect(Rect::from_xywh(1800, 0, 4, 4), &[0.0, 1.0, 0.0, 1.0]);
        Ok(())
    });
    assert!(failed.is_err());
    assert!(Arc::ptr_eq(&session.active().unwrap().doc, &retained));
    assert_eq!(session.active().unwrap().revision, revision);
    assert_eq!(session.active().unwrap().active_layer, Some(id));
    let mut writer = photocraft_format::PcraftWriter::new();
    assert!(writer.save_zip(&retained, &Default::default()).is_err());
    assert!(photocraft_io::export(&retained, "png", &Default::default()).is_err());
    // No failure or recovery happens during a retry, so only the document's own health
    // refuses it.
    let generation = spill::read_error_generation();
    assert!(session.edit("Retry paint", |_, _| Ok(())).is_err());
    assert_eq!(spill::read_error_generation(), generation, "an already-failed tile does not change the generation");
    assert!(Arc::ptr_eq(&session.active().unwrap().doc, &retained));

    // Another document is unaffected: it edits, saves and exports while this one is unreadable.
    assert!(spill::check_integrity().is_err(), "the process-wide status still reports the failure");
    let mut other = Document::new("Healthy", Size::new(512, 256), ColorMode::Rgb, SampleType::U8);
    let mut pixels = Layer::raster("Other", PixelFormat::RGBA8);
    let other_id = pixels.id;
    pixels.surface_mut().unwrap().fill_rect(Rect::from_xywh(0, 0, 512, 256), &[0.0, 0.0, 1.0, 1.0]);
    other.layers.push(pixels);
    session.add_document(other, None);
    session.select_layer(other_id).unwrap();
    session
        .edit("Paint other", |d, _| {
            d.layer_mut(other_id).unwrap().surface_mut().unwrap().fill_rect(Rect::from_xywh(0, 0, 8, 8), &[1.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    spill::settle();
    let healthy = session.active().unwrap().doc.clone();
    assert!(healthy.scratch_health().is_ok());
    let (saved, _) = photocraft_format::PcraftWriter::new().save_zip(&healthy, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap().layers, healthy.layers);
    assert!(photocraft_io::export(&healthy, "png", &Default::default()).is_ok());
    assert!(retained.scratch_health().is_err());

    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&good).unwrap();
    file.sync_all().unwrap();
    for d in [&before, &retained] {
        for (_, tile) in d.layer(id).unwrap().surface().unwrap().tiles() {
            tile.try_bytes().unwrap();
        }
    }
    assert!(spill::check_integrity().is_ok());
    let (bytes, _) = writer.save_zip(&retained, &Default::default()).unwrap();
    let loaded = photocraft_format::load_from_bytes(&bytes).unwrap();
    assert_eq!(loaded.layers, retained.layers);
    drop((session, before, retained, healthy, loaded, writer, file));
    spill::configure(None, 0).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
