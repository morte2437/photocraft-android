//! The Magic Wand's Anti-alias against the reference measurement (#11776): the region itself
//! stays fully selected, and a one-pixel soft fringe lies only outside it, next to colours just
//! past the tolerance. Document 800 x 600: a solid red rectangle 100,100–300,250 on a transparent
//! layer, and a grey ramp of ten 40 px blocks (grey 100, 105, …, 145) from x 100 at y 350–450,
//! on its own transparent layer or composited on white (Sample All Layers).

use photocraft_geom::Rect;

use super::{Region, magic_wand, wand_region};

const AREA: Rect = Rect { x0: 0, y0: 0, x1: 800, y1: 600 };
const W: usize = 800;
const H: usize = 600;

fn red_rectangle() -> Vec<[u8; 4]> {
    let mut img = vec![[0u8; 4]; W * H];
    for y in 100..250 {
        for x in 100..300 {
            img[y * W + x] = [220, 30, 30, 255];
        }
    }
    img
}

/// The grey ramp at y 350–450 on a transparent layer, or on white when `on_white`.
fn ramp(on_white: bool) -> Vec<[u8; 4]> {
    let mut img = vec![if on_white { [255u8; 4] } else { [0u8; 4] }; W * H];
    for y in 350..450 {
        for b in 0..10u8 {
            let g = 100 + 5 * b;
            for x in 100 + 40 * b as usize..140 + 40 * b as usize {
                img[y * W + x] = [g, g, g, 255];
            }
        }
    }
    img
}

/// (bounds of every touched pixel, fully selected count, partly selected count).
fn stats(r: &Region) -> (Rect, usize, usize) {
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    let (mut full, mut part) = (0, 0);
    for y in r.bbox.y0..r.bbox.y1 {
        for x in r.bbox.x0..r.bbox.x1 {
            let v = (r.at(x, y) * 255.0).round() as u8;
            if v == 0 {
                continue;
            }
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            if v == 255 { full += 1 } else { part += 1 }
        }
    }
    (Rect::new(x0, y0, x1, y1), full, part)
}

fn coverage(r: &Region, x: i32, y: i32) -> u8 {
    (r.at(x, y) * 255.0).round() as u8
}

#[test]
fn a_shape_next_to_a_clearly_different_colour_gets_no_fringe() {
    let img = red_rectangle();
    for aa in [true, false] {
        let r = wand_region(&img, AREA, (150, 150), 32.0, true, aa).unwrap();
        assert_eq!(stats(&r), (Rect::new(100, 100, 300, 250), 30000, 0), "anti-alias {aa}");
    }
}

#[test]
fn the_grey_ramp_gets_one_nearly_selected_column_past_the_tolerance() {
    let img = ramp(false);
    // Tolerance 32 from grey 100: blocks 100 to 130 (x 100–380) fully selected; the 135 block
    // is 3 levels past it, so its first column is nearly selected.
    let r = wand_region(&img, AREA, (110, 400), 32.0, true, true).unwrap();
    let (bounds, full, part) = stats(&r);
    assert_eq!((full, part), (280 * 100, 100), "{bounds:?}");
    assert_eq!(bounds, Rect::new(100, 350, 381, 450));
    for y in [350, 400, 449] {
        assert_eq!(coverage(&r, 379, y), 255, "the region's last column stays fully selected");
        assert_eq!(coverage(&r, 380, y), 251, "one fringe column of the next block, as measured");
        assert_eq!(coverage(&r, 381, y), 0, "only one column");
    }
    // Transparent pixels above and below the ramp have no colour: no fringe there.
    assert_eq!(coverage(&r, 200, 349), 0);
    assert_eq!(coverage(&r, 200, 450), 0);
    // Tolerance 10: x 100–220 plus one partly selected column of the 115 block.
    let r = wand_region(&img, AREA, (110, 400), 10.0, true, true).unwrap();
    let (bounds, full, part) = stats(&r);
    assert_eq!((bounds, full, part), (Rect::new(100, 350, 221, 450), 120 * 100, 100));
    assert_eq!(coverage(&r, 220, 400), 249, "5 levels past tolerance 10, as measured");
}

#[test]
fn anti_alias_off_has_no_partly_selected_pixels() {
    let img = ramp(false);
    let r = wand_region(&img, AREA, (110, 400), 32.0, true, false).unwrap();
    assert_eq!(stats(&r), (Rect::new(100, 350, 380, 450), 28000, 0));
}

#[test]
fn the_8_bit_and_f32_wands_draw_the_same_fringe() {
    let img = ramp(false);
    let f: Vec<[f32; 4]> = img.iter().map(|p| p.map(|v| f32::from(v) / 255.0)).collect();
    for (tol, contiguous) in [(32.0, true), (10.0, true), (32.0, false)] {
        let r = wand_region(&img, AREA, (110, 400), tol, contiguous, true).unwrap();
        let m = magic_wand(&f, AREA, (110, 400), tol, contiguous, true);
        for y in 340..460 {
            for x in 90..520 {
                let want = (m[y as usize * W + x as usize] * 255.0).round() as u8;
                assert_eq!(coverage(&r, x, y), want, "tolerance {tol}, contiguous {contiguous} at {x},{y}");
            }
        }
    }
}

#[test]
fn on_the_composite_the_white_around_the_ramp_gets_the_measured_fringe() {
    // Sample All Layers: the ramp on white. White is 155 levels from grey 100, 123 past
    // tolerance 32; the reference application's fringe there reads 110 (Photoshop 2025, the
    // same points; the issue reports 131–177 around the ramp).
    let img = ramp(true);
    let r = wand_region(&img, AREA, (110, 400), 32.0, true, true).unwrap();
    for (x, y) in [(200, 349), (200, 450), (99, 400), (99, 349)] {
        assert_eq!(coverage(&r, x, y), 110, "{x},{y}");
    }
    assert_eq!(coverage(&r, 380, 400), 251, "the near-miss block still gets the stronger fringe");
    assert_eq!(coverage(&r, 200, 400), 255, "inside stays fully selected");
}
