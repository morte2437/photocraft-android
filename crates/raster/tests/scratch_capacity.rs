//! [`spill::scratch_capacity`]: the room imports may count on for tile pixels beyond RAM.
//!
//! One test function: the spill manager is process-wide, so its steps must not interleave.

use photocraft_raster::spill;

/// The room, once no query is outstanding. A query that outlives its 2 s wait on a slow machine
/// counts as no answer, and its late answer would change the status between two reads, so ask again
/// until the room and the status describe the same answer.
fn answered_capacity() -> Option<u64> {
    let mut room = None;
    let started = std::time::Instant::now();
    for _ in 0..10 {
        room = spill::scratch_capacity();
        while spill::query_threads_for_tests() != 0 {
            assert!(started.elapsed() < std::time::Duration::from_secs(30), "scratch capacity query did not finish within 30 seconds: {:?}", spill::stats());
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if room.is_none() == spill::free_space_unmeasured() {
            break;
        }
    }
    room
}

#[test]
fn scratch_capacity_follows_spilling_the_size_limit_and_pauses() {
    let root = std::env::temp_dir().join(format!("photocraft-scratch-capacity-{}", std::process::id()));
    let dir = root.join("scratch");

    // Spilling off: nothing to promise.
    spill::configure(None, 0).unwrap();
    assert_eq!(spill::scratch_capacity(), None);

    spill::configure(Some(&dir), 1 << 20).unwrap();
    assert!(spill::enabled() && !spill::paused());
    // With no size limit, the flag says exactly when the capacity is unknown.
    let unknown = answered_capacity().is_none();
    assert_eq!(spill::free_space_unmeasured(), unknown);
    // The free-space reserve applies; an unknown volume without a size limit promises nothing.
    if let Some(room) = spill::scratch_capacity() {
        assert!(room < u64::MAX / 2, "the reserve and margin are applied: {room}");
    }

    let measurable = answered_capacity().is_some();

    // A folder that stops answering is flagged and clears once it answers again. Windows keeps the
    // open scratch file's folder, so this part needs a folder that can be removed.
    if std::fs::remove_dir_all(&dir).is_ok() {
        assert_eq!(spill::scratch_capacity(), None);
        assert!(spill::free_space_unmeasured());
        assert!(spill::stats().free_space_unmeasured);
        std::fs::create_dir_all(&dir).unwrap();
        let unknown = answered_capacity().is_none();
        assert_eq!(spill::free_space_unmeasured(), unknown);
        assert_eq!(spill::stats().free_space_unmeasured, unknown);
        if measurable {
            assert!(!unknown, "the folder answers again: its room must be known again");
        }
    }

    // The Scratch Size Limit bounds it, less the 1/64 margin, whatever the volume has free.
    let cap = 64u64 << 20;
    spill::set_size_cap(Some(cap));
    let room = spill::scratch_capacity().expect("a size limit is known room");
    assert!(room <= cap - cap / 64, "{room}");

    // Paused (an unusable directory): no credit while tiles cannot leave memory.
    let blocked = root.join("not-a-dir");
    std::fs::write(&blocked, b"x").unwrap();
    assert!(spill::configure(Some(&blocked), 1 << 20).is_err());
    assert!(spill::paused());
    assert_eq!(spill::scratch_capacity(), None);

    spill::set_size_cap(None);
    spill::configure(None, 0).unwrap();
    assert_eq!(spill::scratch_capacity(), None);
    assert!(!spill::free_space_unmeasured());
    let _ = std::fs::remove_dir_all(&root);
}
