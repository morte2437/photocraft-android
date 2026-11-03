//! A scratch disk that fills up pauses spilling; History's byte limit applies again until it
//! resumes, so undo states cannot pile up in memory unbounded meanwhile.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer};
use photocraft_engine::Session;
use photocraft_geom::{Size, TileCoord};
use photocraft_raster::spill;
use serde_json::json;

#[test]
fn paused_scratch_disk_restores_the_history_limit() {
    let dir = std::env::temp_dir().join(format!("photocraft-engine-scratch-pause-{}", std::process::id()));
    let mut session = Session::new();
    session.edit_prefs(|p| {
        p.scratch_disks.disks = vec![photocraft_engine::prefs::ScratchDisk { path: dir.to_string_lossy().into(), enabled: true }];
        p.performance.memory_usage_mb = 64;
    });
    assert!(spill::enabled());
    assert_eq!(spill::stats().size_cap_bytes, None, "0 MB = no scratch size limit");
    // Preferences › Scratch Disks › Scratch Size Limit becomes the scratch file's cap.
    session.edit_prefs(|p| p.scratch_disks.size_limit_mb = 4096);
    assert_eq!(spill::stats().size_cap_bytes, Some(4096 << 20));
    session.edit_prefs(|p| p.scratch_disks.size_limit_mb = 0);
    assert_eq!(spill::stats().size_cap_bytes, None);
    let mut doc = Document::new("Pause fixture", Size::new(1024, 256), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("Pixels", PixelFormat::RGBA8);
    let id = layer.id;
    for i in 0..4 {
        for (j, b) in layer.surface_mut().unwrap().tile_mut(TileCoord::new(i, 0)).bytes_mut().iter_mut().enumerate() {
            *b = (j as u32).wrapping_mul(31).wrapping_add(i as u32 * 7) as u8;
        }
    }
    doc.layers.push(layer);
    session.add_document(doc, None);
    session.select_layer(id).unwrap();
    assert_eq!(session.active().unwrap().history.max_bytes, 0, "spilling: only History States bound History");

    // The disk "fills up": a size limit below what eviction needs pauses spilling.
    spill::set_size_cap(Some(1));
    spill::configure(Some(&dir), 1).unwrap_or_default();
    spill::settle();
    assert!(spill::paused() && !spill::enabled(), "{:?}", spill::stats());
    let limit = session.prefs().performance.history_budget_bytes();
    assert!(limit > 0);
    session.execute("edit.fill", json!({"color": "#204060"})).unwrap();
    assert_eq!(session.active().unwrap().history.max_bytes, limit, "a paused disk must restore History's byte limit");
    assert!(session.active().unwrap().memory_bytes() <= limit);

    // Room again: spilling resumes and the next command lifts the limit again.
    spill::set_size_cap(None);
    assert!(spill::enabled() && !spill::paused(), "{:?}", spill::stats());
    session.execute("edit.fill", json!({"color": "#406080"})).unwrap();
    assert_eq!(session.active().unwrap().history.max_bytes, 0);
    assert!(session.undo());
    assert!(session.undo());

    drop(session);
    spill::configure(None, 0).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
