//! Compare the former full-canvas formulas with cropped morphology on 36 MP.
//! Run in release on CI; includes output allocation and checks exact equality.
use photocraft_algo::selection::{border, contract, edt, expand};
use std::{hint::black_box, time::Instant};

type Op = fn(&[f32], usize, usize, f32) -> Vec<f32>;

fn old_expand(m: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
    let seeds: Vec<bool> = m.iter().map(|v| *v >= 0.5).collect();
    m.iter().zip(edt(&seeds, w, h)).map(|(v, d)| v.max((r + 1.0 - d).clamp(0.0, 1.0))).collect()
}
fn old_contract(m: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
    let inv: Vec<f32> = m.iter().map(|v| 1.0 - v).collect();
    old_expand(&inv, w, h, r).into_iter().map(|v| 1.0 - v).collect()
}
fn old_border(m: &[f32], w: usize, h: usize, r: f32) -> Vec<f32> {
    let inside: Vec<bool> = m.iter().map(|v| *v >= 0.5).collect();
    let outside: Vec<bool> = inside.iter().map(|v| !v).collect();
    let (di, dout) = (edt(&inside, w, h), edt(&outside, w, h));
    inside.iter().zip(di.iter().zip(&dout)).map(|(s, (a, b))| (1.0 - ((if *s { b } else { a }) - 1.0) / r).clamp(0.0, 1.0)).collect()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (w, h) = (6000, 6000);
    println!("36 MP; median of three runs including allocation; RAYON_NUM_THREADS={}", std::env::var("RAYON_NUM_THREADS").unwrap_or_default());
    for (case, size) in [("sparse", 50), ("dense", 4000), ("full", 6000)] {
        let mut m = vec![0.0; w * h];
        let start = (w - size) / 2;
        for row in m.chunks_exact_mut(w).skip(start).take(size) {
            if let Some(span) = row.get_mut(start..start + size) {
                span.fill(1.0)
            }
        }
        for r in [3.0, 50.0] {
            for (name, old, new) in
                [("expand", old_expand as Op, expand as Op), ("contract", old_contract as Op, contract as Op), ("border", old_border as Op, border as Op)]
            {
                let mut timings = [0.0; 2];
                let expected = old(&m, w, h, r);
                if new(&m, w, h, r) != expected {
                    return Err(format!("output mismatch: {case} {name} {r}").into());
                }
                drop(expected);
                for (slot, op) in timings.iter_mut().zip([old, new]) {
                    let mut runs = [0.0; 3];
                    for time in &mut runs {
                        let now = Instant::now();
                        let result = black_box(op(black_box(&m), w, h, r));
                        *time = now.elapsed().as_secs_f64() * 1000.0;
                        drop(result);
                    }
                    runs.sort_by(f64::total_cmp);
                    *slot = runs.get(1).copied().unwrap_or_default();
                }
                println!(
                    "{case} {name} r={r}: old={:.2}ms new={:.2}ms speedup={:.2}x exact=true",
                    timings.first().copied().unwrap_or_default(),
                    timings.get(1).copied().unwrap_or_default(),
                    timings.first().copied().unwrap_or_default() / timings.get(1).copied().unwrap_or(1.0)
                );
            }
        }
    }
    Ok(())
}
