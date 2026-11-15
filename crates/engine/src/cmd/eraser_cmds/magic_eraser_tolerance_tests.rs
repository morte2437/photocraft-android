//! #11995: on 8-bit RGB the Magic Eraser measures Tolerance by the spread of `{0, dR, dG, dB}`, so
//! opposite changes in two channels count double. The expected values are the reference
//! application's measurements from the issue; the stripes document is drawn at a tenth of the size.

use photocraft_algo::selection as sel;
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::Session;

const SEED: [u8; 3] = [200, 40, 40];
/// `d` of the patch grid's columns.
const GRID: [i32; 8] = [20, 50, 80, 110, 140, 170, 200, 230];

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn document(w: i32, h: i32, depth: u32, background: [u8; 3]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "mode": "rgb", "depth": depth, "background": hex(background)})).unwrap();
    s
}

fn fill(s: &mut Session, [x, y, w, h]: [i32; 4], color: [u8; 3]) {
    s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": h, "antiAlias": false})).unwrap();
    s.execute("edit.fill", json!({"color": hex(color)})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
}

fn erase(s: &mut Session, x: i32, y: i32, options: Value) {
    let mut p = json!({"x": x, "y": y, "antiAlias": false});
    p.as_object_mut().unwrap().extend(options.as_object().unwrap().clone());
    s.execute("paint.magicEraser", p).unwrap();
}

fn rgba(s: &Session, x: i32, y: i32) -> [f32; 4] {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

fn erased(s: &Session, x: i32, y: i32) -> bool {
    rgba(s, x, y)[3] == 0.0
}

/// Grey (100, 100, 100) with ten-pixel patches along the top: same-direction and opposite changes.
fn grey_patches(depth: u32) -> (Session, [(i32, [u8; 3], i32); 4]) {
    let patches = [(10, [130, 100, 70], 60), (30, [130, 130, 100], 30), (50, [70, 130, 100], 60), (70, [70, 70, 70], 30)];
    let mut s = document(90, 30, depth, [100, 100, 100]);
    for (x, color, _) in patches {
        fill(&mut s, [x, 10, 10, 10], color);
    }
    (s, patches)
}

/// Opposite changes count double and same-direction changes once (Tolerance 29, 30, 59, 60).
#[test]
fn opposite_channel_changes_count_double_around_a_grey_seed() {
    for tolerance in [29, 30, 59, 60] {
        let (mut s, patches) = grey_patches(8);
        erase(&mut s, 0, 0, json!({"tolerance": tolerance, "contiguous": false}));
        for (x, color, metric) in patches {
            assert_eq!(erased(&s, x + 5, 15), tolerance >= metric, "tolerance {tolerance}, patch {color:?}");
        }
    }
}

fn grid_colour(row: usize, d: i32) -> [u8; 3] {
    let [r, g, b] = SEED.map(i32::from);
    let [r, g, b] = match row {
        0 => [r - d, g, b],
        1 => [r + d, g, b],
        2 => [r, g + d, b],
        3 => [r, g + d, b + d],
        _ => [r - d, g + d, b + d],
    };
    [r, g, b].map(|v| v.clamp(0, 255) as u8)
}

/// The issue's patch grid around (200, 40, 40): `0` is erased and `x` kept, one row per Tolerance.
#[test]
fn the_patch_grid_matches_the_measured_table() {
    const TABLE: [(i32, [&str; 5]); 7] = [
        (30, ["0xxxxxxx", "0xxxxxxx", "0xxxxxxx", "0xxxxxxx", "xxxxxxxx"]),
        (40, ["0xxxxxxx", "0xxxxxxx", "0xxxxxxx", "0xxxxxxx", "0xxxxxxx"]),
        (60, ["00xxxxxx", "00000000", "00xxxxxx", "00xxxxxx", "0xxxxxxx"]),
        (100, ["000xxxxx", "00000000", "000xxxxx", "000xxxxx", "00xxxxxx"]),
        (150, ["00000xxx", "00000000", "00000xxx", "00000xxx", "00xxxxxx"]),
        (200, ["00000000", "00000000", "0000000x", "0000000x", "000xxxxx"]),
        (255, ["00000000", "00000000", "00000000", "00000000", "0000xxxx"]),
    ];
    for (tolerance, rows) in TABLE {
        let mut s = document(80, 60, 8, SEED);
        for row in 0..5 {
            for (col, d) in GRID.into_iter().enumerate() {
                fill(&mut s, [col as i32 * 10, 10 + row as i32 * 10, 10, 10], grid_colour(row, d));
            }
        }
        erase(&mut s, 0, 0, json!({"tolerance": tolerance, "contiguous": false}));
        for (row, want) in rows.into_iter().enumerate() {
            let got: String = (0..8).map(|col| if erased(&s, col * 10 + 5, 15 + row as i32 * 10) { '0' } else { 'x' }).collect();
            assert_eq!(got, want, "tolerance {tolerance}, row {row}");
        }
    }
}

const RED: [u8; 3] = [220, 30, 30];
const GREEN: [u8; 3] = [30, 180, 60];
const BLUE: [u8; 3] = [40, 60, 220];
const PINK: [u8; 3] = [240, 120, 120];
const UPPER_RED: (i32, i32) = (10, 5);
const LOWER_RED: (i32, i32) = (10, 22);
const WHITE_GAP: (i32, i32) = (10, 15);

/// The stripes over eight blocks (200, 40 + 20·i, 40), with a white gap cutting the red stripe.
fn stripes() -> Session {
    let mut s = document(80, 60, 8, [255, 255, 255]);
    for (i, color) in [RED, GREEN, BLUE, PINK].into_iter().enumerate() {
        fill(&mut s, [i as i32 * 20, 0, 20, 30], color);
    }
    fill(&mut s, [0, 14, 20, 2], [255, 255, 255]);
    for i in 0..8u8 {
        fill(&mut s, [i32::from(i) * 10, 30, 10, 30], [200, 40 + 20 * i, 40]);
    }
    s
}

fn erased_blocks(s: &Session) -> usize {
    (0..8).take_while(|i| erased(s, i * 10 + 5, 45)).count()
}

/// Contiguous from block 0: the blocks go one by one, and green and blue survive even at 255.
#[test]
fn stripes_contiguous_from_block_zero() {
    for (tolerance, blocks) in
        [(0, 1), (10, 1), (19, 1), (20, 2), (21, 2), (39, 2), (40, 3), (41, 3), (60, 4), (100, 6), (140, 8), (160, 8), (200, 8), (255, 8)]
    {
        let mut s = stripes();
        erase(&mut s, 5, 45, json!({"tolerance": tolerance}));
        assert_eq!(erased_blocks(&s), blocks, "tolerance {tolerance}");
        assert_eq!(8 - (0..8).filter(|i| !erased(&s, i * 10 + 5, 45)).count(), blocks, "tolerance {tolerance}: the erased blocks are consecutive");
        assert_eq!(erased(&s, 70, 15), tolerance >= 140, "tolerance {tolerance}: pink, reached through blocks 6 and 7");
        assert_eq!(erased(&s, UPPER_RED.0, UPPER_RED.1), tolerance == 255, "tolerance {tolerance}: upper red, reached through the white gap");
        if tolerance == 41 || tolerance >= 60 {
            assert!(erased(&s, LOWER_RED.0, LOWER_RED.1), "tolerance {tolerance}: lower red");
        }
        if tolerance < 30 {
            assert!(!erased(&s, LOWER_RED.0, LOWER_RED.1), "tolerance {tolerance}: lower red");
        }
        assert!(!erased(&s, 30, 15), "tolerance {tolerance}: green is kept");
        assert!(!erased(&s, 50, 15), "tolerance {tolerance}: blue is kept");
    }
}

/// Non-contiguous from block 0: white goes from Tolerance 220; green and blue never do.
#[test]
fn stripes_non_contiguous_from_block_zero() {
    let mut s = stripes();
    erase(&mut s, 5, 45, json!({"tolerance": 150, "contiguous": false}));
    assert_eq!(erased_blocks(&s), 8);
    assert!(erased(&s, UPPER_RED.0, UPPER_RED.1) && erased(&s, LOWER_RED.0, LOWER_RED.1), "both reds");
    assert!(erased(&s, 70, 15), "pink");
    assert!(!erased(&s, WHITE_GAP.0, WHITE_GAP.1), "the white gap (metric 215) is kept");
    assert!(!erased(&s, 30, 15) && !erased(&s, 50, 15), "green and blue");

    let mut s = stripes();
    erase(&mut s, 5, 45, json!({"tolerance": 220, "contiguous": false}));
    assert!(erased(&s, WHITE_GAP.0, WHITE_GAP.1), "white is within 220");
    assert!(!erased(&s, 30, 15) && !erased(&s, 50, 15), "green and blue");
}

/// The fringe uses the same distance: a neighbour the spread rejects is not fully erased, even
/// though every one of its channels is within the Tolerance.
#[test]
fn anti_alias_does_not_erase_a_neighbour_the_spread_rejects() {
    let (mut s, _) = grey_patches(8);
    erase(&mut s, 0, 0, json!({"tolerance": 32, "contiguous": false, "antiAlias": true}));
    assert!(rgba(&s, 15, 15)[3] == 1.0, "the patch's inside is kept");
    assert!(rgba(&s, 10, 15)[3] > 0.0, "(130, 100, 70) has spread 60, past Tolerance 32");
}

/// Depths the rule was not measured for keep comparing each channel on its own.
#[test]
fn sixteen_bit_keeps_the_per_channel_limit() {
    let (mut s, patches) = grey_patches(16);
    erase(&mut s, 0, 0, json!({"tolerance": 30, "contiguous": false}));
    for (x, color, _) in patches {
        assert!(erased(&s, x + 5, 15), "patch {color:?}");
    }
}

/// The Magic Wand and the Paint Bucket share the fill and keep the per-channel limit.
#[test]
fn the_magic_wand_and_paint_bucket_keep_the_per_channel_limit() {
    let (mut s, _) = grey_patches(8);
    s.execute("select.magicWand", json!({"x": 0, "y": 0, "tolerance": 30, "contiguous": false, "antiAlias": false})).unwrap();
    let selected = sel::mask_from_surface(s.active().unwrap().doc.selection.as_ref(), Rect::new(0, 0, 90, 30));
    assert_eq!(selected[15 * 90 + 15], 1.0, "the wand takes (130, 100, 70) at 30");

    let (mut s, _) = grey_patches(8);
    s.execute("paint.bucket", json!({"x": 0, "y": 0, "tolerance": 30, "contiguous": false, "antiAlias": false, "color": "#ff00ff"})).unwrap();
    let [r, g, b, _] = rgba(&s, 15, 15);
    assert_eq!([r, g, b], [1.0, 0.0, 1.0], "the bucket fills (130, 100, 70) at 30");
}

/// A transparent seed still matches every transparent pixel, and alpha still limits the match.
#[test]
fn the_spread_keeps_the_transparent_and_alpha_handling() {
    let area = Rect::new(0, 0, 3, 1);
    let run = |img: [[u8; 4]; 3], tolerance| sel::wand_region_by(&img, area, (0, 0), tolerance, false, false, None, sel::ColorDistance::Spread).unwrap();

    let region = run([[10, 20, 30, 0], [200, 0, 90, 0], [9, 9, 9, 255]], 0.0);
    assert_eq!((region.at(1, 0), region.at(2, 0)), (1.0, 0.0), "hidden colours don't matter; visible pixels don't match");

    let half = [[100, 100, 100, 200], [100, 100, 100, 100], [100, 100, 100, 170]];
    let region = run(half, 30.0);
    assert_eq!((region.at(1, 0), region.at(2, 0)), (0.0, 1.0), "alpha differences stay limited by the Tolerance");
}

/// The spread was measured on opaque colours only: a partly transparent seed or pixel keeps the
/// per-channel limit, in the match and in the anti-alias fringe.
#[test]
fn partly_transparent_colours_keep_the_per_channel_limit() {
    let area = Rect::new(0, 0, 3, 1);
    let run = |img: &[[u8; 4]; 3], distance, anti_alias| sel::wand_region_by(img, area, (0, 0), 30.0, false, anti_alias, None, distance).unwrap();

    let opaque = [[100, 100, 100, 255], [130, 100, 70, 255], [9, 9, 9, 255]];
    assert_eq!(run(&opaque, sel::ColorDistance::Spread, false).at(1, 0), 0.0, "opaque colours use the spread");

    for img in [
        [[100, 100, 100, 128], [130, 100, 70, 128], [160, 100, 40, 128]],
        [[100, 100, 100, 255], [130, 100, 70, 250], [160, 100, 40, 250]],
        [[100, 100, 100, 250], [130, 100, 70, 255], [160, 100, 40, 255]],
    ] {
        assert_eq!(run(&img, sel::ColorDistance::Spread, false).at(1, 0), 1.0, "{img:?}: every channel is within 30");
        assert!(run(&img, sel::ColorDistance::Spread, true).at(2, 0) > 0.0, "{img:?}: the third pixel gets a fringe");
        assert_eq!(run(&img, sel::ColorDistance::Spread, true), run(&img, sel::ColorDistance::LargestChannel, true), "{img:?}: same fringe");
    }
}
