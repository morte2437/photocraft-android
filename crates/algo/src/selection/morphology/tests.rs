use super::*;
use crate::selection::{border, contract, expand};

fn oracle(m: &[f32], w: usize, h: usize, r: f32, kind: Kind) -> Vec<f32> {
    let inside: Vec<bool> = m.iter().map(|v| *v >= 0.5).collect();
    match kind {
        Kind::Expand => m.iter().zip(edt(&inside, w, h)).map(|(v, d)| v.max((r + 1.0 - d).clamp(0.0, 1.0))).collect(),
        Kind::Contract => {
            let inv: Vec<f32> = m.iter().map(|v| 1.0 - v).collect();
            oracle(&inv, w, h, r, Kind::Expand).iter().map(|v| 1.0 - v).collect()
        }
        Kind::Border => {
            if r == 0.0 {
                return vec![0.0; m.len()];
            }
            let outside: Vec<bool> = inside.iter().map(|v| !v).collect();
            let di = edt(&inside, w, h);
            let dout = edt(&outside, w, h);
            inside.iter().zip(di.iter().zip(&dout)).map(|(sel, (di, dout))| (1.0 - ((if *sel { dout } else { di }) - 1.0) / r).clamp(0.0, 1.0)).collect()
        }
    }
}

#[test]
fn matches_full_canvas_formulas() {
    for (w, h) in [(1usize, 1usize), (1, 31), (37, 1), (39, 27)] {
        for case in 0..9 {
            let m: Vec<f32> = (0..w * h)
                .map(|i| {
                    let (x, y) = (i % w, i / w);
                    match case {
                        0 => 0.0,
                        1 => 1.0,
                        2 => {
                            if x == w / 2 && y == h / 2 {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        3 => {
                            if x < w / 2 && y < h / 2 {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        4 => {
                            if x.abs_diff(w / 2) < 3 && y.abs_diff(h / 2) < 3 {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        5 => {
                            if x.abs_diff(w / 2) < 5 && y.abs_diff(h / 2) < 5 && (x.abs_diff(w / 2) > 2 || y.abs_diff(h / 2) > 2) {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        6 => [0.0, 0.1, 0.499_999_97, 0.5, 0.6, 1.0][(i * 31 + i / 7) % 6],
                        7 => 0.25,
                        _ => {
                            if x == 0 || x + 1 == w || y == 0 || y + 1 == h {
                                1.0
                            } else {
                                0.0
                            }
                        }
                    }
                })
                .collect();
            for r in [0.0, 0.25, 1.0, 1.5, 2.0, 4.25, 5.0, 5.01, 12.0, 100.0] {
                for kind in [Kind::Expand, Kind::Contract, Kind::Border] {
                    assert_eq!(modify(&m, w, h, r, kind), oracle(&m, w, h, r, kind), "{w}x{h} case {case} radius {r}");
                }
            }
        }
    }
}

#[test]
fn tiny_selection_uses_tiny_distance_window() {
    let (w, h) = (1024, 768);
    let mut m = vec![0.0; w * h];
    for y in 300..350 {
        m[y * w + 400..y * w + 450].fill(1.0);
    }
    let b = window(&m, w, h, 5.0).unwrap();
    assert_eq!(b, Window { x0: 394, x1: 456, y0: 294, y1: 356 });
    assert_eq!((b.x1 - b.x0) * (b.y1 - b.y0), 62 * 62);
}

#[test]
fn invalid_dimensions_and_radii_are_safe() {
    for op in [expand, contract, border] {
        for (w, h) in [(0, 1), (1, 0), (2, 2), (usize::MAX, 2)] {
            assert!(op(&[1.0], w, h, 2.0).is_empty());
        }
        for r in [f32::NAN, f32::INFINITY, -1.0] {
            assert!(op(&[1.0], 1, 1, r).is_empty());
        }
    }
}

#[test]
fn bounded_distances_match_edt_within_reach() {
    for (w, h) in [(1, 17), (23, 1), (31, 29)] {
        for pattern in 0..4 {
            let seeds: Vec<bool> = (0..w * h)
                .map(|i| match pattern {
                    0 => false,
                    1 => true,
                    2 => i == w * h / 2,
                    _ => (i * 73 + i / w * 11) % 19 == 0,
                })
                .collect();
            let exact = edt(&seeds, w, h);
            for reach in [1.0, 1.25, 2.0, 4.5, 6.0] {
                let got = edt_bounded(&seeds, w, h, reach);
                for (g, e) in got.iter().zip(&exact) {
                    if *e <= reach { assert_eq!(g, e) } else { assert!(*g > reach) }
                }
            }
        }
    }
}

#[test]
fn every_three_by_three_mask_matches_with_a_cropped_halo() {
    let (w, h) = (17, 19);
    for bits in 0..512 {
        let mut m = vec![0.0; w * h];
        for y in 0..3 {
            for x in 0..3 {
                if bits & (1 << (y * 3 + x)) != 0 {
                    m[(y + 7) * w + x + 6] = 1.0;
                }
            }
        }
        // Retain faint pixels beyond the binary selection's bounds too.
        m[3 * w + 2] = 0.125;
        for r in [0.0, 0.414_213_54, 0.414_213_66, 1.5, 3.0, 5.0, 5.5] {
            for kind in [Kind::Expand, Kind::Contract, Kind::Border] {
                assert_eq!(modify(&m, w, h, r, kind), oracle(&m, w, h, r, kind), "mask {bits} radius {r}");
            }
        }
    }
}

#[test]
fn enormous_finite_radii_keep_the_no_seed_sentinel_behavior() {
    for m in [vec![0.0; 9], vec![1.0; 9], vec![0.25; 9]] {
        for r in [1e10, 2e10, f32::MAX] {
            for kind in [Kind::Expand, Kind::Contract, Kind::Border] {
                assert_eq!(modify(&m, 3, 3, r, kind), oracle(&m, 3, 3, r, kind));
            }
        }
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn bounded_and_general_paths_are_worker_count_independent() {
    let (w, h) = (400, 300);
    let m: Vec<f32> = (0..w * h).map(|i| if (i * 73 + i / w * 11) % 19 == 0 { 1.0 } else { 0.0 }).collect();
    for r in [3.0, 9.0] {
        for kind in [Kind::Expand, Kind::Contract, Kind::Border] {
            let expected = oracle(&m, w, h, r, kind);
            for workers in [1, 2, 4] {
                let pool = rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap();
                assert_eq!(pool.install(|| modify(&m, w, h, r, kind)), expected);
            }
        }
    }
}
