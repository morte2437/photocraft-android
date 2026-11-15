//! Protect Detail in [`local_sharpen`] sharpens gently instead of changing nothing (#12938).
//!
//! The issue's test document, in RGBA like the Sharpen tool's dabs: a grey 96 | grey 160 step and
//! a lone grey-160 pixel in grey 96. A hard 40 px tip covers each test image, so one click is one
//! pass that moves every pixel by `strength · (sharpened − original)`, rounded to 8 bits. The
//! expected values are the reference application's, with the issue's tolerances.

use super::*;

const W: usize = 60;
const H: usize = 21;
/// x of the first grey-160 column of the step: x 400 in the issue's document.
const EDGE: usize = 30;

fn grey(levels: impl Fn(usize, usize) -> f32, w: usize, h: usize) -> Vec<f32> {
    (0..w * h)
        .flat_map(|i| {
            let g = levels(i % w, i / w) / 255.0;
            [g, g, g, 1.0]
        })
        .collect()
}

fn step() -> Vec<f32> {
    grey(|x, _| if x < EDGE { 96.0 } else { 160.0 }, W, H)
}

/// One pass of the tool at `strength` (0–1) over the whole image.
fn pass(data: &[f32], w: usize, h: usize, strength: f32, protect: bool) -> Vec<f32> {
    let out = local_sharpen(data, w, h, 4, Some(3), (0, 0, w, h), 1.0, protect);
    data.iter().zip(&out).map(|(v, s)| v + (s - v) * strength).collect()
}

/// Red channel of each pixel in 8-bit levels.
fn levels(data: &[f32]) -> Vec<i32> {
    data.as_chunks::<4>().0.iter().map(|p| (p[0].clamp(0.0, 1.0) * 255.0).round() as i32).collect()
}

/// Row `y` of the step, as x 400 + `x - EDGE` in the issue's document.
fn at(row: &[i32], x_doc: usize) -> i32 {
    row[x_doc + EDGE - 400]
}

#[test]
fn one_click_on_the_step_gives_a_low_halo_that_follows_strength() {
    let row = |strength: f32| levels(&pass(&step(), W, H, strength, true))[H / 2 * W..(H / 2 + 1) * W].to_vec();
    let full = row(1.0);
    for (x, want) in [(396, 94), (397, 92), (398, 91), (399, 90), (400, 164), (401, 163), (402, 161)] {
        assert!((at(&full, x) - want).abs() <= 3, "100 %: x {x} is {}, want {want} ± 3 ({full:?})", at(&full, x));
    }
    let half = row(0.5);
    assert!((at(&half, 399) - 93).abs() <= 2 && (at(&half, 400) - 162).abs() <= 2, "50 %: x 399/400 = {}/{}", at(&half, 399), at(&half, 400));
    let quarter = row(0.25);
    assert!(at(&quarter, 399) < 96 && at(&quarter, 400) > 160, "25 % still sharpens: {}/{}", at(&quarter, 399), at(&quarter, 400));
    assert_eq!(row(0.01), levels(&step())[..W].to_vec(), "1 % changes nothing");
}

#[test]
fn one_click_on_a_lone_pixel_lifts_it_gently() {
    let (w, h, c) = (41, 41, 20);
    let data = grey(|x, y| if (x, y) == (c, c) { 160.0 } else { 96.0 }, w, h);
    let on = levels(&pass(&data, w, h, 1.0, true));
    let off = levels(&pass(&data, w, h, 1.0, false));
    let centre = on[c * w + c];
    assert!((centre - 168).abs() <= 4, "centre {centre}, want 168 ± 4");
    for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
        let n = on[(c as i32 + dy) as usize * w + (c as i32 + dx) as usize];
        assert!((93..=95).contains(&n), "neighbour ({dx}, {dy}) is {n}, want 93–95");
    }
    let (lift_on, lift_off) = (centre - 160, off[c * w + c] - 160);
    assert!(lift_on > 0 && 2 * lift_on < lift_off, "Protect Detail lifts {lift_on}, off lifts {lift_off}");
}

#[test]
fn uniform_areas_and_alpha_stay_exactly_as_they_were() {
    for g in [0.0, 96.0 / 255.0, 0.5, 1.0, 2.5] {
        let data: Vec<f32> = (0..12 * 9).flat_map(|i| [g, g * 0.5, 0.25, [0.0, 0.3, 1.0][i % 3]]).collect();
        let out = local_sharpen(&data, 12, 9, 4, Some(3), (0, 0, 12, 9), 1.0, true);
        for (o, d) in out.as_chunks::<4>().0.iter().zip(data.as_chunks::<4>().0) {
            assert_eq!(o[3].to_bits(), d[3].to_bits(), "alpha unchanged");
        }
        let opaque: Vec<f32> = (0..12 * 9).flat_map(|_| [g, g * 0.5, 0.25, 1.0]).collect();
        let out = local_sharpen(&opaque, 12, 9, 4, Some(3), (2, 2, 10, 7), 1.0, true);
        // Exact up to the Gaussian weights' rounding, far below one 16-bit level.
        let same = |a: f32, b: f32| (a - b).abs() < 1e-6;
        assert!(out.as_chunks::<4>().0.iter().all(|p| same(p[0], g) && same(p[1], g * 0.5) && same(p[2], 0.25)), "grey {g} unchanged: {out:?}");
    }
}

#[test]
fn three_drags_widen_the_halo_without_ringing_between_black_and_white() {
    // A drag of a 40 px tip at 25 % spacing passes about four dabs over each point; each drag is a
    // stroke of its own, rounded to 8 bits when it ends.
    let (w, h, edge) = (80, 5, 40);
    let mut data = grey(|x, _| if x < edge { 96.0 } else { 160.0 }, w, h);
    let mut halos = Vec::new();
    for _ in 0..3 {
        for _ in 0..4 {
            data = pass(&data, w, h, 1.0, true);
        }
        data = data.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0).collect();
        let row = levels(&data)[h / 2 * w..(h / 2 + 1) * w].to_vec();
        halos.push(row.iter().enumerate().filter(|&(x, &v)| (v - if x < edge { 96 } else { 160 }).abs() >= 2).count());
        // Darker towards the edge on the dark side, lighter towards it on the light side.
        for x in edge - 5..edge - 1 {
            assert!(row[x] >= row[x + 1], "dark side rises at x {x}: {row:?}");
        }
        for x in edge..edge + 4 {
            assert!(row[x] >= row[x + 1], "light side rises at x {x}: {row:?}");
        }
        assert!(row[..edge].iter().all(|&v| v < 255) && row[edge..].iter().all(|&v| v > 0), "no black–white ringing: {row:?}");
    }
    assert!(halos[0] < halos[2], "the halo widens with every drag: {halos:?}");
}

#[test]
fn protect_detail_off_keeps_the_sigma_1_unsharp_mask() {
    let data = step();
    let blur = local_blur(&data, W, H, 4, Some(3), (0, 0, W, H), 1.0);
    let out = local_sharpen(&data, W, H, 4, Some(3), (0, 0, W, H), 1.0, false);
    for (i, ((v, b), o)) in data.iter().zip(&blur).zip(&out).enumerate() {
        let want = if i % 4 == 3 { *v } else { v + (v - b) };
        assert_eq!(o.to_bits(), want.to_bits(), "sample {i}");
    }
}
