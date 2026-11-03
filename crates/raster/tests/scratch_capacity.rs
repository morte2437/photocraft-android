//! [`spill::scratch_capacity`]: the room imports may count on for tile pixels beyond RAM.
//!
//! One test function: the spill manager is process-wide, so its steps must not interleave.

use photocraft_raster::spill;

#[test]
fn scratch_capacity_follows_spilling_the_size_limit_and_pauses() {
    let root = std::env::temp_dir().join(format!("photocraft-scratch-capacity-{}", std::process::id()));
    let dir = root.join("scratch");

    // Spilling off: nothing to promise.
    spill::configure(None, 0).unwrap();
    assert_eq!(spill::scratch_capacity(), None);

    spill::configure(Some(&dir), 1 << 20).unwrap();
    assert!(spill::enabled() && !spill::paused());
    // The free-space reserve applies; an unknown volume without a size limit promises nothing.
    if let Some(room) = spill::scratch_capacity() {
        assert!(room < u64::MAX / 2, "the reserve and margin are applied: {room}");
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
    let _ = std::fs::remove_dir_all(&root);
}
