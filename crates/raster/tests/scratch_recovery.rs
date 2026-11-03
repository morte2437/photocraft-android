//! Scratch disk failures pause spilling instead of turning it off for the session: tiles stay
//! in memory, the pause is retried with backoff and spilling resumes once the disk has room.
//!
//! One test function: the spill manager is process-wide, so its steps must not interleave.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Surface, spill};

const TILE_BYTES: usize = (TILE_SIZE * TILE_SIZE) as usize * 4;

fn rgba8() -> PixelFormat {
    PixelFormat { mode: ColorMode::Rgb, sample: SampleType::U8, alpha: true }
}

/// Incompressible bytes, different per seed.
fn noise(seed: u32, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

fn surface(seed: u32, n: i32) -> Surface {
    let mut s = Surface::new(rgba8());
    for i in 0..n {
        let r = Rect::from_xywh(i * TILE_SIZE, 0, TILE_SIZE as u32, TILE_SIZE as u32);
        s.write_interleaved(r, &noise(seed + i as u32, TILE_BYTES));
    }
    s
}

fn check(s: &Surface, seed: u32, n: i32) {
    for i in 0..n {
        let t = s.tile(TileCoord { tx: i, ty: 0 }).unwrap();
        assert!(*t.bytes() == *noise(seed + i as u32, TILE_BYTES), "tile {i} of seed {seed} changed");
    }
}

#[test]
fn full_or_missing_scratch_disk_pauses_and_recovers() {
    let root = std::env::temp_dir().join(format!("photocraft-scratch-recovery-{}", std::process::id()));
    let dir = root.join("scratch");
    let budget = 8 * TILE_BYTES;
    spill::configure(Some(&dir), budget).unwrap();
    assert!(spill::enabled() && !spill::paused());

    // 1. "Disk full": the scratch file reaches its size limit. Spilling pauses, every tile stays
    //    in memory and readable, and owners of memory caps see the change.
    let cap = 16 * TILE_BYTES as u64;
    spill::set_size_cap(Some(cap));
    let epoch = spill::epoch();
    let a = surface(1, 64);
    spill::settle();
    let st = spill::stats();
    assert!(st.paused && !spill::enabled(), "{st:?}");
    assert_ne!(spill::epoch(), epoch, "pausing must tell owners to re-apply their caps");
    assert!(st.resident_bytes > budget, "tiles must stay in memory while paused: {st:?}");
    assert!(st.scratch_bytes < cap + 2 * TILE_BYTES as u64, "the file grew past its limit: {st:?}");
    assert!(st.last_error.as_deref().is_some_and(|e| e.contains("paused") && e.contains("retrying in 1 s")), "{st:?}");
    check(&a, 1, 64);

    // 2. A retry while the disk is still full pauses again, with a longer backoff; a budget
    //    change (the memory limit moved) does not end the pause.
    spill::retry_now_for_tests();
    spill::maintain_budget();
    let st = spill::stats();
    assert!(st.paused && !spill::enabled(), "{st:?}");
    assert!(st.last_error.as_deref().is_some_and(|e| e.contains("retrying in 2 s")), "{st:?}");
    spill::set_budget(budget);
    assert!(!spill::enabled(), "set_budget must not bypass the pause");

    // 3. Room again: the next retry resumes spilling with the latest budget and clears the error.
    let epoch = spill::epoch();
    spill::set_size_cap(None);
    spill::maintain_budget();
    let st = spill::stats();
    assert!(!st.paused && spill::enabled() && st.budget_bytes == budget && st.last_error.is_none(), "{st:?}");
    assert_ne!(spill::epoch(), epoch);
    spill::settle();
    assert!(spill::stats().resident_bytes <= budget, "{:?}", spill::stats());
    check(&a, 1, 64);
    drop(a);

    // 4. "Disk gone": the directory can't be created (a file is in the way). Configuring reports
    //    it and pauses; the directory is kept and retried, and spilling resumes once it works.
    let blocked = root.join("blocked");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&blocked, b"not a directory").unwrap();
    assert!(spill::configure(Some(&blocked), budget).is_err());
    let st = spill::stats();
    assert!(st.paused && !spill::enabled() && st.dir.as_deref() == Some(blocked.as_path()), "{st:?}");
    let b = surface(500, 32);
    spill::settle();
    check(&b, 500, 32);
    std::fs::remove_file(&blocked).unwrap();
    spill::retry_now_for_tests();
    spill::maintain_budget();
    assert!(spill::enabled() && !spill::paused(), "{:?}", spill::stats());
    spill::settle();
    assert!(spill::stats().resident_bytes <= budget);
    check(&b, 500, 32);

    // 5. Cleanup: once spilling is off and the tiles are freed, no scratch file is left behind.
    drop(b);
    spill::configure(None, 0).unwrap();
    for d in [&dir, &blocked] {
        let left: Vec<_> = std::fs::read_dir(d).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        assert!(left.is_empty(), "scratch files left in {}: {left:?}", d.display());
    }
    let _ = std::fs::remove_dir_all(&root);
}
