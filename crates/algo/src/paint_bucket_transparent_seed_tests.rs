//! Paint Bucket match rule for transparent and semi-transparent seeds (#12529), Anti-alias off.
//!
//! The issue's document is 800 x 600 with a red square, a grey ramp and an anti-aliased black
//! disc on a transparent layer. These tests shrink the geometry (see `NOTES.md`) but keep every
//! measured relationship: the ramp is its own 256 x 1 strip so seeds 127 / 128 and the tolerances
//! 0 / 1 / 32 / 100 give the issue's exact grey ranges.

use photocraft_color::{PixelFormat, blend::BlendMode};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

use crate::paint::{FillMode, bucket_fill_inks};
use crate::selection::{rgba8_image, wand_region};

const W: i32 = 100;
const H: i32 = 80;
const AREA: Rect = Rect { x0: 0, y0: 0, x1: W, y1: H };
const GREEN: [u8; 4] = [0, 255, 0, 255];
/// Row of the black disc's edge; x 10..14 hold alpha 40, 112, 197, 253 like (690..700, 400).
const EDGE_Y: i32 = 70;
/// Alphas of the edge row from x = 10: the 40 / 112 / 197 / 253 ladder, then solid 253 inside.
const LADDER: [u8; 4] = [40, 112, 197, 253];

fn idx(x: i32, y: i32) -> usize {
    (y * W + x) as usize
}

/// Red square (40..60 x 10..30), grey ramp strip (value = x, x 0..64, y 40..43), and a black
/// edge row at `EDGE_Y` (x 10..14 the alpha ladder, x 14..30 alpha 253); the rest transparent.
fn document() -> Vec<[u8; 4]> {
    let mut img = vec![[0u8; 4]; (W * H) as usize];
    for y in 10..30 {
        for x in 40..60 {
            img[idx(x, y)] = [220, 30, 30, 255];
        }
    }
    for y in 40..43 {
        for x in 0..64 {
            img[idx(x, y)] = [x as u8, x as u8, x as u8, 255];
        }
    }
    for (i, a) in LADDER.iter().enumerate() {
        img[idx(10 + i as i32, EDGE_Y)] = [0, 0, 0, *a];
    }
    for x in 14..30 {
        img[idx(x, EDGE_Y)] = [0, 0, 0, 253];
    }
    img
}

fn surface(img: &[[u8; 4]], area: Rect) -> Surface {
    let bytes: Vec<u8> = img.iter().flatten().copied().collect();
    Surface::from_interleaved(PixelFormat::RGBA8, area, &bytes)
}

/// Bucket-fills green at `seed` and returns the layer afterwards.
fn fill(img: &[[u8; 4]], area: Rect, seed: (i32, i32), tol: f32, contiguous: bool) -> Vec<[u8; 4]> {
    let mut s = surface(img, area);
    let green = |_, _| [0.0, 1.0, 0.0, 1.0];
    assert!(bucket_fill_inks(&mut s, area, seed, tol, contiguous, false, 1.0, None, false, green, None, FillMode::Blend(BlendMode::Normal)));
    rgba8_image(&s, area)
}

/// Pixels the fill changed, as indices.
fn changed(before: &[[u8; 4]], after: &[[u8; 4]]) -> Vec<usize> {
    (0..before.len()).filter(|&i| before[i] != after[i]).collect()
}

fn assert_only_alpha_zero_filled(before: &[[u8; 4]], after: &[[u8; 4]], label: &str) {
    for (i, (b, a)) in before.iter().zip(after).enumerate() {
        if b[3] == 0 {
            assert_eq!(*a, GREEN, "{label}: transparent pixel {i} should fill");
        } else {
            assert_eq!(a, b, "{label}: pixel {i} with alpha {} must not change", b[3]);
        }
    }
}

#[test]
fn transparent_seed_fills_only_alpha_zero_at_any_tolerance() {
    let img = document();
    for tol in [0.0, 32.0, 255.0] {
        let after = fill(&img, AREA, (49, 49), tol, true);
        // The transparent background is one connected region here, so contiguous reaches all of it.
        assert_only_alpha_zero_filled(&img, &after, &format!("tolerance {tol}"));
        for (i, a) in LADDER.iter().enumerate() {
            assert_eq!(after[idx(10 + i as i32, EDGE_Y)], [0, 0, 0, *a], "edge alpha {a} survives at tolerance {tol}");
        }
        assert_eq!(after[idx(9, EDGE_Y)], GREEN, "transparent pixel next to the edge fills");
        assert_eq!(after[idx(50, 20)], [220, 30, 30, 255], "red square untouched at tolerance {tol}");
        assert_eq!(after[idx(30, 41)], [30, 30, 30, 255], "ramp untouched at tolerance {tol}");
    }
}

#[test]
fn transparent_seed_not_contiguous_fills_every_alpha_zero_pixel_only() {
    let mut img = document();
    // An enclosed transparent hole that contiguous fill cannot reach.
    for y in 12..16 {
        for x in 44..48 {
            img[idx(x, y)] = [0, 0, 0, 0];
        }
    }
    for tol in [0.0, 255.0] {
        let after = fill(&img, AREA, (49, 49), tol, false);
        assert_only_alpha_zero_filled(&img, &after, &format!("global, tolerance {tol}"));
        assert_eq!(after[idx(45, 13)], GREEN, "the enclosed transparent hole fills");
        assert_eq!(after[idx(13, EDGE_Y)], [0, 0, 0, 253]);
    }
}

#[test]
fn semi_transparent_seed_takes_same_colour_pixels_of_any_nonzero_alpha() {
    let img = document();
    let after = fill(&img, AREA, (10, EDGE_Y), 32.0, true);
    // The 40 / 112 / 197 / 253 ladder and the solid 253 run all fill: no alpha band applies.
    for x in 10..30 {
        assert_eq!(after[idx(x, EDGE_Y)], GREEN, "black alpha pixel at x {x} fills");
    }
    assert_eq!(after[idx(9, EDGE_Y)], [0, 0, 0, 0], "the transparent neighbour is never taken");
    let n = changed(&img, &after).len();
    assert_eq!(n, 20, "only the black edge row changes");
}

#[test]
fn semi_transparent_seed_respects_colour_tolerance() {
    let mut img = document();
    img[idx(12, EDGE_Y)] = [100, 100, 100, 112];
    let after = fill(&img, AREA, (10, EDGE_Y), 32.0, true);
    assert_eq!(after[idx(11, EDGE_Y)], GREEN);
    assert_eq!(after[idx(12, EDGE_Y)], [100, 100, 100, 112], "a colour outside Tolerance stays");
    assert_eq!(after[idx(13, EDGE_Y)], [0, 0, 0, 253], "and it cuts the contiguous run");
}

fn ramp_strip() -> Vec<[u8; 4]> {
    (0..=255u8).map(|v| [v, v, v, 255]).collect()
}

fn filled_range(after: &[[u8; 4]]) -> Option<(usize, usize)> {
    let hits: Vec<usize> = (0..after.len()).filter(|&i| after[i] == GREEN).collect();
    Some((*hits.first()?, *hits.last()?))
}

#[test]
fn opaque_seed_keeps_todays_ranges() {
    let img = ramp_strip();
    let area = Rect::new(0, 0, 256, 1);
    for (seed, tol, contiguous, lo, hi) in
        [(127, 0.0, true, 127, 127), (127, 1.0, true, 126, 128), (127, 32.0, true, 95, 159), (127, 100.0, true, 27, 227), (128, 32.0, false, 96, 160)]
    {
        let after = fill(&img, area, (seed, 0), tol, contiguous);
        assert_eq!(filled_range(&after), Some((lo, hi)), "seed {seed} tolerance {tol} contiguous {contiguous}");
        let count = after.iter().filter(|p| **p == GREEN).count();
        assert_eq!(count, hi - lo + 1, "the filled range is solid");
    }
}

/// The wand rule did not move: a transparent seed at Tolerance 255 still matches every pixel
/// (alpha within 255 of 0), which is what the Magic Wand and Magic Eraser keep.
#[test]
fn wand_region_still_uses_the_alpha_band() {
    let img = document();
    let r = wand_region(&img, AREA, (49, 49), 255.0, true, false).expect("seed is inside");
    assert_eq!(r.at(50, 20), 1.0, "red square is inside the wand region");
    assert_eq!(r.at(30, 41), 1.0, "ramp is inside the wand region");
    assert_eq!(r.at(13, EDGE_Y), 1.0, "disc edge is inside the wand region");
    let r32 = wand_region(&img, AREA, (49, 49), 32.0, true, false).expect("seed is inside");
    assert_eq!(r32.at(10, EDGE_Y), 0.0, "alpha 40 is outside a Tolerance 32 alpha band");
    assert_eq!(r32.at(9, EDGE_Y), 1.0, "alpha 0 is inside it");
}

/// The measured semi-transparent case is one colour at several alphas; a pixel of another
/// colour keeps the alpha band the shared rule always had (the issue leaves unmeasured
/// cases at today's behaviour). Seed [0, 0, 0, 40] at x = 1: x = 0 is the same-colour
/// control [0, 0, 0, 253]; x = 2 is [20, 20, 20, 197] — its RGB distance of 20 passes
/// Tolerance 32, but its alpha sits 157 from the seed's, so it is not filled.
#[test]
fn semi_transparent_seed_keeps_the_alpha_band_for_other_colours() {
    let img: Vec<[u8; 4]> = vec![[0, 0, 0, 253], [0, 0, 0, 40], [20, 20, 20, 197]];
    let area = Rect::new(0, 0, 3, 1);
    let after = fill(&img, area, (1, 0), 32.0, true);
    let ch = changed(&img, &after);
    assert!(ch.contains(&0), "same-colour pixel of alpha 253 fills");
    assert!(!ch.contains(&2), "different-colour pixel of alpha 197 keeps the alpha band");
}
