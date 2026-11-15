//! Bracket-key size steps from the measured sequence in #12936 and its specified extensions.

use super::*;

/// The commands walk the measured sizes in both directions without changing hardness or spacing.
#[test]
fn commands_follow_the_measured_sequence_in_both_directions() {
    let sizes = [
        1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 15.0, 20.0, 25.0, 30.0, 35.0, 40.0, 45.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0, 125.0, 150.0, 175.0,
        200.0, 250.0, 300.0, 400.0, 500.0, 600.0, 700.0, 800.0, 900.0, 1000.0, 1100.0, 1200.0, 1300.0, 1400.0, 1500.0,
    ];
    let mut s = Session::new();
    s.execute("tools.setBrush", json!({"brush": {"size": 1, "hardness": 0.37, "spacing": 0.42}})).unwrap();
    for (command, expected) in [
        ("tools.increaseBrushSize", sizes.iter().skip(1).copied().collect::<Vec<_>>()),
        ("tools.decreaseBrushSize", sizes.iter().rev().skip(1).copied().collect()),
    ] {
        for size in expected {
            s.execute(command, json!({})).unwrap();
            assert_eq!(s.tools.brush.size, size, "{command}");
            assert_eq!(s.tools.brush.hardness, 0.37);
            assert_eq!(s.tools.brush.spacing, 0.42);
            assert_eq!(s.journal.last().map(|(id, _)| id.as_str()), Some(command));
        }
    }
    assert!(s.documents().is_empty(), "brush keys still work without a document");
}

/// Fractional and integer sizes between entries snap to the next entry in the requested direction.
#[test]
fn intermediate_sizes_snap_in_the_keys_direction() {
    for (size, smaller, larger) in [
        (1.25, 1.0, 2.0),
        (9.5, 9.0, 10.0),
        (10.5, 10.0, 15.0),
        (12.0, 10.0, 15.0),
        (27.0, 25.0, 30.0),
        (32.0, 30.0, 35.0),
        (49.5, 45.0, 50.0),
        (55.0, 50.0, 60.0),
        (99.0, 90.0, 100.0),
        (100.5, 100.0, 125.0),
        (149.0, 125.0, 150.0),
        (176.0, 175.0, 200.0),
        (225.0, 200.0, 250.0),
        (299.0, 250.0, 300.0),
        (301.0, 300.0, 400.0),
        (1499.0, 1400.0, 1500.0),
        (1501.0, 1500.0, 1600.0),
        (4999.0, 4900.0, 5000.0),
    ] {
        assert_eq!(size_step(size, false), smaller, "[ from {size}");
        assert_eq!(size_step(size, true), larger, "] from {size}");
    }
}

/// The specified extension above 1500 continues in hundreds and saturates at the brush maximum.
#[test]
fn commands_continue_in_hundreds_to_the_maximum_and_back() {
    let mut s = Session::new();
    s.execute("tools.setBrush", json!({"brush": {"size": 1500}})).unwrap();
    for size in (1600..=5000).step_by(100) {
        s.execute("tools.increaseBrushSize", Value::Null).unwrap();
        assert_eq!(s.tools.brush.size, size as f32);
    }
    assert_eq!(s.tools.brush.size, MAX_SIZE);
    for _ in 0..3 {
        s.execute("tools.increaseBrushSize", Value::Null).unwrap();
        assert_eq!(s.tools.brush.size, MAX_SIZE);
    }
    for size in (1500_u16..=4900).step_by(100).rev() {
        s.execute("tools.decreaseBrushSize", Value::Null).unwrap();
        assert_eq!(s.tools.brush.size, size as f32);
    }
}

/// Small sizes still reach 1 px, stay there on decrease and grow again on increase (#3229).
#[test]
fn commands_reach_and_hold_the_one_pixel_minimum() {
    let mut s = Session::new();
    s.execute("tools.setBrush", json!({"brush": {"size": 3}})).unwrap();
    for size in [2.0, 1.0, 1.0, 1.0] {
        s.execute("tools.decreaseBrushSize", Value::Null).unwrap();
        assert_eq!(s.tools.brush.size, size);
    }
    s.execute("tools.increaseBrushSize", Value::Null).unwrap();
    assert_eq!(s.tools.brush.size, 2.0);
    assert_eq!(size_step(0.5, true), 1.0, "a subpixel brush grows to the first entry");
    assert_eq!(size_step(0.5, false), 1.0, "the bracket-key minimum remains 1 px");
}
