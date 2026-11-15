use super::{Contour, Map, satin_map};
use photocraft_color::{BlendMode, Color};
use photocraft_doc::{FxCommon, Satin};

/// #14983: black Satin over an orange rectangle, measured down its centre column.
fn green_column(size: f32) -> Vec<u8> {
    let mut shape = Map::new(400, 300, 0.0);
    for row in shape.v.as_chunks_mut::<400>().0.iter_mut().skip(75).take(150) {
        row[100..300].fill(1.0);
    }
    let satin = Satin {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        color: Color::BLACK,
        angle: 90.0,
        distance: 20.0,
        size,
        contour: Contour::Linear,
        anti_alias: true,
        invert: false,
    };
    let coverage = satin_map(&shape, &satin);
    (75..225).map(|y| (128.0 * (1.0 - coverage.get(200, y))).round() as u8).collect()
}

fn assert_profile(size: f32, dark_rows: usize, ramp: &[u8], tolerance: u8) {
    let green = green_column(size);
    let bright_rows = 150 - 2 * (dark_rows + ramp.len());
    let expected = std::iter::repeat_n(0, dark_rows)
        .chain(ramp.iter().copied())
        .chain(std::iter::repeat_n(128, bright_rows))
        .chain(ramp.iter().rev().copied())
        .chain(std::iter::repeat_n(0, dark_rows));
    for (i, (actual, expected)) in green.into_iter().zip(expected).enumerate() {
        assert!(actual.abs_diff(expected) <= tolerance, "Satin size {size}, y {}: expected G {expected}, got {actual}", i + 75);
    }
}

#[test]
fn satin_size_one_softens_each_band_edge_by_one_pixel() {
    assert_profile(1.0, 19, &[32, 96], 2);
}

#[test]
fn satin_other_sizes_keep_their_profiles() {
    assert_profile(0.0, 20, &[], 0);
    assert_profile(3.0, 18, &[15, 43, 85, 114], 2);
    assert_profile(10.0, 11, &[2, 4, 8, 13, 20, 27, 36, 46, 58, 71, 82, 92, 101, 109, 115, 120, 124, 127], 2);
    let wide = green_column(30.0);
    assert_eq!(&wide[19..21], &[62, 66]);
    assert_eq!(&wide[129..131], &[66, 62]);
}
