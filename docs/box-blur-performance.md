# Box blur performance

2026-10-30. Two changes to `fxutil`, the running-sum box blur most smoothing filters are built from:
the row pass stopped clamping samples that cannot need it, and the column pass stopped paying a
cache miss per sample.

## Row pass: the clamp that cannot fire

The row pass read every sample through a clamping index, including the ~95 % of a line that is
nowhere near an end.

`fxutil::box_rows_n` walked each line with `at(x, c) = scratch[x.clamp(0, w - 1) * n + c]`, so every
sample paid a clamp, a signed→unsigned conversion and a bounds check. `box_line_n` now splits the
line into the head (x < r), the tail (x ≥ w - r - 1) and the middle: in the middle, `x - r` and
`x + r + 1` are already inside the line, so the taps are plain indices into the untouched copy. The
middle is also the only place the clamp provably cannot fire, so the samples, their order and the
`f64` accumulator are unchanged.

Nothing else changed: no public API, no `FilterParams`, no tile size, no halo, no
`perf/budgets.toml` entry, no dependency. The accumulator moved from one scalar per channel to a
`[f64; MAXC]` indexed by channel so the line is walked once, in pixel order, instead of once per
channel; the additions per channel still happen in the same order.

## Column pass: gather a strip before walking it

A column walk reads one sample per row at a stride of `w * n`, so every sample pulls its own cache
line and uses `n * 4` of its 64 bytes — 8x the image's traffic on a 1122-wide RGBA buffer, which is
the pass that costs 2x the row pass. `box_cols_n` now gathers up to 64 KiB of neighbouring columns
into a transposed scratch (one line read per row instead of one per sample), runs the same running
sum over contiguous runs, and writes back. `STRIP_BYTES` is the knob: the scratch has to stay
L2-resident or the gather costs more than it saves, and below two columns per strip the gather is
skipped for the old strided walk (which is why a 6000x4000 buffer still takes it).

This is the pattern `selection_blur.rs` already uses for its vertical pass; `fxutil` did not.

The taps and their order are unchanged: each column is still summed in `y` order with the same
`f64` accumulator, and the same head/middle/tail split applies per column. Output is bit-identical
(`fxutil::tests::cols_match_the_strided_walk` compares all `f32` bits against the strided walk,
including the 0×0, 5×0, `r = 0` and `r > h` edges).

## Output equivalence

Bit-identical. Every row of `bench_filters2` (a 6000x4000 RGBA8 layer, one `apply` per row) was
rendered from the base tree and from the changed tree and hashed with FNV-1a 64 over the raw `f32`
bytes (little-endian), in row order, read in 64-row bands so the harness holds a few MB instead of
the layer's 384 MB. All 16 hashes matched; the example prints them, so the claim is reproducible.

| Row | FNV-1a 64 |
|---|---|
| Oil Paint | `a24418295e253bed` |
| Reduce Noise | `cf615fc6e9753e31` |
| Lens Blur | `d37f714557820600` |
| Smart Blur r3 | `79a70345f731cc2f` |
| Smart Blur r12 | `fcb259263b91c1f3` |
| Smart Blur r100 | `fbf9288699e9bb52` |
| Shape Blur | `eb8a751d0eecba89` |
| Tilt-Shift | `6403ffccc9793893` |
| Iris Blur | `4eddeb227a11e508` |
| Field Blur | `2c73617cc35aeea8` |
| Spin Blur | `ab59231b1bcd0a8e` |
| Path Blur | `32cf5e5b6ad9a0fc` |
| Crystallize | `c4ef27a261a0d3ff` |
| Color Halftone | `b6b887f16c432c08` |
| Lighting Effects | `8534e2fec9cf4dd6` |
| Relight | `1d91fa30504342cb` |

The box blur is reached by Reduce Noise (guided filter), Relight, Oil Paint's pre-smoothing, the
Blur Gallery, Tilt-Shift, Shadows/Highlights and HDR ghosting, so this table covers the callers that
feed it.

## Timings

Intel Core i5-10300H (4 cores / 8 threads), 16 GB, Windows 11, rustc 1.98.1 (`48a229cea
2026-09-01`), default release profile (thin LTO, `codegen-units = 1`, no `target-cpu`), rayon at its
default 8 threads. Before: the base binary, five runs per row. Row pass only: the first commit. Both: a rebuild of
the second commit from a clean working tree, three runs per row (run-to-run spread on these rows is
a few percent; the deltas here are 4-48 %). Median reported, nothing else running.

| Case (6000x4000 RGBA8) | Before | Row pass only | Both | Speedup |
|---|---:|---:|---:|---:|
| Reduce Noise | 2430.8 | 2198.4 | 1638.3 | 1.48x |
| Tilt-Shift | 2161.0 | 1964.4 | 1532.9 | 1.41x |
| Relight | 3285.7 | 3155.6 | 2405.3 | 1.37x |
| Oil Paint | 8246.9 | 8102.7 | 7894.1 | 1.04x |
| Lighting Effects | 1273.0 | 1316.3 | 1333.9 | 0.95x |

The passes themselves, measured on the same machine and build settings (portable x86-64, so no
AVX2: the shipped profile does not set `target-cpu`):

| Buffer | Pass | Before | After | Speedup |
|---|---|---:|---:|---:|
| 324x324x2, r 9 (Reduce Noise window) | row | 0.627 ms | 0.425 ms | 1.48x |
| 388x388x2, r 5 | row | 1.126 ms | 0.602 ms | 1.87x |
| 786x1122x2, r 79 (Relight window) | row | 5.087 ms | 3.587 ms | 1.42x |
| 6000x4000x4, r 4 (single-shot paths) | row | 268.1 ms | 166.5 ms | 1.61x |
| 324x324x2, r 9 | column | 0.807 ms | 0.557 ms | 1.45x |
| 1122x1122x2, r 79 | column | 14.183 ms | 7.439 ms | 1.91x |
| 1122x786x2, r 79 | column | 9.253 ms | 5.203 ms | 1.78x |

Each pass above is timed on its own copy of the same input; timing both on one buffer measures the
second one on data the first already rewrote.

## What this does not fix

These four rows are the #211 "slow algorithms" entries measured by this example, and their
`perf/budgets.toml` targets (1 s on 24 MP) are not reachable from a kernel change. Measured on this
machine, single-threaded, one `apply`:

| Row | Total | Where it goes |
|---|---:|---|
| Reduce Noise | 6.78 s | box passes 4.90 s (chroma 3.52, luma 1.41), decompose 0.55 s |
| Relight | 10.06 s | shading blur 7.73 s (column passes 4.23, row passes 1.40, pack/unpack 2.10), luma fill 1.63 s |
| Smart Blur r 12 | 9.71 s | ~450 offsets per pixel over 24 M pixels; already tiled, already all cores |
| Oil Paint | 8.25 s | 21 bilinear samples and 41 `hash01` calls per pixel; already all cores |

The three levers that would move them, each measured or bounded:

1. **One transposed working buffer per window.** The column pass is no longer 8x wasteful, but it is
   still its own pass with its own gather and write-back: `gauss_blur_n` runs *all* its row passes
   and then *all* its column passes, so the gather happens once per column pass. Keeping the window
   transposed across every pass and elementwise step would remove the remaining tax. That is a
   restructuring of `denoise.rs`/`relight.rs`, not of the kernel.
2. **Halo redundancy.** Relight blurs 93.9 M pixels for a 24 M pixel image (3.9x), because each
   512-px tile carries a 305-px halo. Forcing a 2048-px tile cuts it to 2.6x and measures
   3211 -> 2113 ms parallel, but a 2048-px tile with that halo reads ~95 MB of source per tile,
   which is why `auto_tile` caps there; going further needs the banded path
   (`relight::relight_bands`) driven by the pipeline instead of a per-tile read.
3. **AVX2 for the line kernel.** The release profile is portable (no `target-cpu`), so the `f64`
   accumulate runs on SSE2. The same kernel built with `-C target-cpu=native` is another 2x faster;
   taking that would need runtime `is_x86_feature_detected` dispatch on a `#[target_feature]`
   variant.

## Reproduce

```
cargo build --release -p photocraft-algo --example bench_filters2
./target/release/examples/bench_filters2 <row-substring> --reps 5
```

The base tree is `origin/main` in a detached worktree; the example prints one `HASH <row> <fnv>`
line per rep on stderr and, with `--json <file>`, the report `cargo xtask perf` reads.

## Suites run

`cargo test -p photocraft-algo --release`: 764 passed, 2 failed, 27 ignored. Both failures
(`pixelate::tests::facet_reuse_matches_scalar_in_every_model`,
`pixelate::tests::cell_filters_match_scalar_models_edges_and_fractional_cells`) compare NaN bit
patterns (`NaN != NaN`, `2143289344` vs `4290772992`) and fail identically on the base tree with
`pixelate/` untouched, so they are pre-existing on this platform.
