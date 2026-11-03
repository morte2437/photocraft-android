//! Scratch disk: tiles over the budget go to disk and come back unchanged.
//!
//! One test function: the spill manager is process-wide, so its steps must not interleave.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Surface, spill};

const TILE_BYTES: usize = (TILE_SIZE * TILE_SIZE) as usize * 4;

fn rgba8() -> PixelFormat {
    PixelFormat { mode: ColorMode::Rgb, sample: SampleType::U8, alpha: true }
}

/// Incompressible bytes, different per tile and per seed.
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

/// A surface of `n × 1` tiles of noise; tile `i` holds `noise(seed + i)`.
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
fn tiles_spill_and_come_back() {
    let dir = std::env::temp_dir().join(format!("photocraft-spill-test-{}", std::process::id()));
    let budget = 8 * TILE_BYTES;
    spill::configure(Some(&dir), budget).unwrap();
    assert!(spill::enabled());

    // 1. Over budget: the oldest tiles go to disk, resident memory drops to the budget.
    let a = surface(1, 32);
    let b = surface(1000, 32);
    spill::settle();
    let st = spill::stats();
    assert!(st.resident_bytes <= budget, "resident {} > budget {budget}", st.resident_bytes);
    assert!(st.evictions >= 48, "only {} evictions", st.evictions);
    assert!(st.scratch_bytes > 0 && st.last_error.is_none(), "{st:?}");

    // 2. Every pixel reads back exactly (CRC-checked, LZ4 round trip), through every reader.
    check(&a, 1, 32);
    check(&b, 1000, 32);
    assert!(spill::stats().reloads > 0);
    let mut row = vec![[0u8; 4]; TILE_SIZE as usize];
    a.read_rgba8_into(Rect::from_xywh(5 * TILE_SIZE, 7, TILE_SIZE as u32, 1), &mut row);
    let want = noise(6, TILE_BYTES);
    let off = 7 * TILE_SIZE as usize * 4;
    assert!(row.iter().flatten().eq(want[off..off + TILE_SIZE as usize * 4].iter()));

    // Upstream's fallible readers and clones must reload evicted storage too.
    spill::settle();
    let rect = Rect::from_xywh(0, 0, TILE_SIZE as u32, TILE_SIZE as u32);
    assert_eq!(a.try_to_interleaved(rect).unwrap(), noise(1, TILE_BYTES));
    assert_eq!(a.try_read_region(rect).unwrap(), a.read_region(rect));
    let copied = a.tile(TileCoord::new(0, 0)).unwrap().try_clone().unwrap();
    assert_eq!(&*copied.try_bytes().unwrap(), &noise(1, TILE_BYTES));
    drop(copied);

    // 3. Copy-on-write still works on spilled tiles: the clone changes, the original doesn't.
    spill::settle();
    let mut c = a.clone();
    c.fill_rect(Rect::from_xywh(0, 0, 10, 10), &[1.0, 0.0, 0.0, 1.0]);
    assert_eq!(c.pixel(3, 3), vec![1.0, 0.0, 0.0, 1.0]);
    check(&a, 1, 32);
    spill::settle();
    // The edit survives its own eviction (the stale disk copy was released, a new one written).
    assert_eq!(c.pixel(3, 3), vec![1.0, 0.0, 0.0, 1.0]);
    check(&a, 1, 32);

    // 4. Parallel readers of spilled tiles all see the right bytes.
    spill::settle();
    std::thread::scope(|scope| {
        for k in 0..8 {
            let (a, b) = (&a, &b);
            scope.spawn(move || {
                for round in 0..4 {
                    let (s, seed) = if (k + round) % 2 == 0 { (a, 1) } else { (b, 1000) };
                    check(s, seed, 32);
                }
            });
        }
    });

    // 5. Dropping everything frees memory and scratch space.
    drop((a, b, c));
    let st = spill::stats();
    assert_eq!(st.scratch_bytes, 0, "scratch extents leaked: {st:?}");

    // 6. Off: nothing more is evicted, and the budget no longer applies.
    spill::configure(None, 0).unwrap();
    assert!(!spill::enabled());
    let before = spill::stats().evictions;
    let d = surface(5000, 32);
    assert_eq!(spill::stats().evictions, before);
    check(&d, 5000, 32);
    drop(d);

    // 7. Encoded bytes and COW edits survive eviction at every supported depth.
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let fmt = PixelFormat { mode: ColorMode::Rgb, sample, alpha: true };
        let len = (TILE_SIZE * TILE_SIZE) as usize * fmt.bytes_per_pixel();
        spill::configure(Some(&dir), len * 2).unwrap();
        let mut s = Surface::new(fmt);
        for i in 0..12 {
            s.tile_mut(TileCoord::new(i, 0)).bytes_mut().copy_from_slice(&noise(8000 + i as u32, len));
        }
        let original = s.clone();
        spill::settle();
        assert!(spill::stats().resident_bytes <= len * 2);
        s.fill_rect(Rect::from_xywh(0, 0, 4, 4), &[1.0, 0.0, 0.0, 1.0]);
        spill::settle();
        assert_eq!(s.pixel(1, 1), vec![1.0, 0.0, 0.0, 1.0]);
        for i in 0..12 {
            assert_eq!(&*original.tile(TileCoord::new(i, 0)).unwrap().bytes(), &noise(8000 + i as u32, len));
        }
        let mut converted = [[0; 4]; 4];
        s.read_rgba8_into(Rect::from_xywh(0, 0, 4, 1), &mut converted);
        assert_eq!(converted, [[255, 0, 0, 255]; 4]);
    }
    // A tiny budget must still allow a pinned reader to finish instead of evicting itself.
    spill::configure(Some(&dir), 1).unwrap();
    let tiny = surface(9000, 2);
    spill::settle();
    check(&tiny, 9000, 2);
    drop(tiny);
    spill::configure(None, 0).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
