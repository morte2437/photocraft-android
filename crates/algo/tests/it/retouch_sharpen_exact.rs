//! Frozen scalar sharpen from main, used as an exact-output oracle.
use photocraft_algo::retouch::{local_blur, local_sharpen};

#[test]
fn matches_scalar_sharpen_bit_for_bit_across_layouts_edges_and_hdr() {
    for (w, h) in [(1, 1), (3, 7), (37, 29)] {
        for ch in 1..=5 {
            for alpha in std::iter::once(None).chain((0..ch).map(Some)) {
                let data: Vec<f32> = (0..w * h * ch)
                    .map(|i| if Some(i % ch) == alpha { [0.0, 1e-8, 0.25, 0.75, 1.0][(i / ch) % 5] } else { ((i * 73 + 19) % 113) as f32 / 17.0 - 2.0 })
                    .collect();
                for win in [(0, 0, w, h), (w / 2, h / 2, w, h), (0, 0, 1, 1), (0, 0, 0, h)] {
                    for amount in [0.0, 0.5, 1.0, 2.5] {
                        for protect_detail in [false, true] {
                            let expected = scalar_local_sharpen(&data, w, h, ch, alpha, win, amount, protect_detail);
                            let actual = local_sharpen(&data, w, h, ch, alpha, win, amount, protect_detail);
                            assert_eq!(actual.len(), expected.len());
                            for (i, (a, b)) in actual.iter().zip(&expected).enumerate() {
                                assert_eq!(
                                    a.to_bits(),
                                    b.to_bits(),
                                    "{w}x{h} ch={ch} alpha={alpha:?} win={win:?} amount={amount} protect={protect_detail} sample={i}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn empty_channel_layout_remains_empty() {
    assert!(local_sharpen(&[], 5, 5, 0, None, (0, 0, 5, 5), 1.0, true).is_empty());
}

#[allow(clippy::too_many_arguments)]
fn scalar_local_sharpen(
    data: &[f32],
    w: usize,
    h: usize,
    ch: usize,
    alpha: Option<usize>,
    win: (usize, usize, usize, usize),
    amount: f32,
    protect_detail: bool,
) -> Vec<f32> {
    let (x0, y0, x1, y1) = win;
    let (ow, oh) = (x1 - x0, y1 - y0);
    if ow == 0 || oh == 0 || ch == 0 {
        return Vec::new();
    }
    let blur = local_blur(data, w, h, ch, alpha, win, 1.0);
    let mut out = vec![0.0f32; blur.len()];
    for y in y0..y1 {
        for x in x0..x1 {
            let o = ((y - y0) * ow + (x - x0)) * ch;
            let i = (y * w + x) * ch;
            for c in 0..ch {
                let v = data[i + c];
                if Some(c) == alpha {
                    out[o + c] = v;
                    continue;
                }
                let mut s = v + amount * (v - blur[o + c]);
                if protect_detail {
                    let (mut lo, mut hi) = (v, v);
                    for (dx, dy) in [(-1i32, -1i32), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                        let (nx, ny) = ((x as i32 + dx).clamp(0, w as i32 - 1) as usize, (y as i32 + dy).clamp(0, h as i32 - 1) as usize);
                        let nv = data[(ny * w + nx) * ch + c];
                        lo = lo.min(nv);
                        hi = hi.max(nv);
                    }
                    s = s.clamp(lo, hi);
                }
                out[o + c] = s;
            }
        }
    }
    out
}
