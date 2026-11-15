//! The Elliptical Marquee's anti-aliased edge against the reference measurement (#11391):
//! document 400 × 350, ellipse 100,100–300,250, the patch x 127–132, y 119–124 on the 45° part
//! of its top-left edge. Grids hold what a black fill of the selection reads there (255 outside,
//! 0 fully selected), as the issue's tables do.

use photocraft_geom::Rect;
use photocraft_raster::Surface;

use super::ellipse_surface;

const AREA: Rect = Rect { x0: 0, y0: 0, x1: 400, y1: 350 };
const ELLIPSE: Rect = Rect { x0: 100, y0: 100, x1: 300, y1: 250 };
const PATCH: (i32, i32) = (127, 119);

/// The reference with Anti-alias on.
const SOFT: [[u8; 6]; 6] = [
    [255, 255, 255, 255, 255, 152],
    [255, 255, 255, 212, 106, 0],
    [255, 255, 172, 64, 0, 0],
    [247, 137, 26, 0, 0, 0],
    [105, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0],
];

/// Anti-alias off: the reference and PhotoCraft before #11391 agree.
const HARD: [[u8; 6]; 6] =
    [[255, 255, 255, 255, 255, 255], [255, 255, 255, 255, 0, 0], [255, 255, 255, 0, 0, 0], [255, 255, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0]];

/// The grey a black fill of selection `s` gives at `(x, y)` on white.
fn grey(s: &Surface, x: i32, y: i32) -> u8 {
    let v = s.pixel(x, y).first().copied().unwrap_or(0.0);
    (255.0 - v * 255.0).round() as u8
}

fn patch(s: &Surface) -> [[u8; 6]; 6] {
    std::array::from_fn(|j| std::array::from_fn(|i| grey(s, PATCH.0 + i as i32, PATCH.1 + j as i32)))
}

#[test]
fn the_anti_aliased_patch_matches_the_reference_within_three_levels() {
    let got = patch(&ellipse_surface(ELLIPSE, AREA, true));
    for (j, (row, want)) in got.iter().zip(&SOFT).enumerate() {
        for (i, (g, w)) in row.iter().zip(want).enumerate() {
            assert!(g.abs_diff(*w) <= 3, "{},{}: {g} vs reference {w}\n{got:?}", PATCH.0 + i as i32, PATCH.1 + j as i32);
        }
    }
}

#[test]
fn the_other_three_quadrants_mirror_the_measured_one() {
    let s = ellipse_surface(ELLIPSE, AREA, true);
    // Symmetric about x = 200 and y = 175: pixel x mirrors to 399 - x, y to 349 - y.
    for y in PATCH.1..PATCH.1 + 6 {
        for x in PATCH.0..PATCH.0 + 6 {
            let here = s.pixel(x, y);
            assert_eq!(s.pixel(399 - x, y), here, "{x},{y} mirrored across x");
            assert_eq!(s.pixel(x, 349 - y), here, "{x},{y} mirrored across y");
            assert_eq!(s.pixel(399 - x, 349 - y), here, "{x},{y} mirrored through the centre");
        }
    }
    // A few partial pixels, so the mirror is not of 0s and 1s alone.
    assert_eq!(grey(&s, 399 - 129, 122), grey(&s, 129, 122));
    assert!((1..255).contains(&grey(&s, 399 - 129, 349 - 121)));
}

#[test]
fn the_hard_edge_is_unchanged() {
    assert_eq!(patch(&ellipse_surface(ELLIPSE, AREA, false)), HARD);
}

/// Fully inside reads exactly 1 and outside exactly 0, so Add / Subtract / Intersect combine as
/// before; the edge is soft only in a band at most √2 px wide.
#[test]
fn inside_is_exactly_selected_and_outside_exactly_empty() {
    let s = ellipse_surface(ELLIPSE, AREA, true);
    // Every pixel whose square lies inside, here all of the axes but the end pixels (whose
    // corners poke out by a sliver).
    for x in 101..299 {
        assert_eq!(s.pixel(x, 175), vec![1.0], "{x},175 on the horizontal axis");
    }
    for y in 101..249 {
        assert_eq!(s.pixel(200, y), vec![1.0], "200,{y} on the vertical axis");
    }
    for (x, y) in [(100, 175), (299, 175), (200, 100), (200, 249)] {
        assert!(s.pixel(x, y)[0] > 0.99, "{x},{y}: the axis ends are all but full");
    }
    for (x, y) in [(99, 175), (300, 175), (200, 99), (200, 250), (100, 100), (299, 249), (0, 0)] {
        assert_eq!(s.pixel(x, y), vec![0.0], "{x},{y} outside");
    }
    let partial = (0..350).flat_map(|y| (0..400).map(move |x| (x, y))).filter(|&(x, y)| (1..255).contains(&grey(&s, x, y))).count();
    let (hard_in, soft_in) = {
        let h = ellipse_surface(ELLIPSE, AREA, false);
        let sum = |s: &Surface| s.read_region(AREA).iter().map(|v| f64::from(*v)).sum::<f64>();
        (sum(&h), sum(&s))
    };
    // The perimeter (≈ 556 px) times a band 1 to √2 px across, and the hard ellipse's area.
    assert!((556..787).contains(&partial), "{partial} partial pixels");
    let exact = std::f64::consts::PI * 100.0 * 75.0;
    assert!((soft_in - exact).abs() < 0.002 * exact && (hard_in - exact).abs() < 0.002 * exact, "{soft_in} {hard_in} vs {exact}");
}
