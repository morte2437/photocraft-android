# Oil Paint performance

2026-10-22. One change removes repeated work from the existing Oil Paint line integral convolution.

The LIC takes 21 bilinear samples and 20 tangent lookups per output pixel, and each of those
computed a rounded sample coordinate with `f32::floor` or `f32::round`. On the baseline x86-64
target (SSE2, no SSE4.1 round instruction) both of those are library calls: `rustc -O --emit asm`
on `fn f(x: f32) -> f32 { x.floor() }` produces `jmp floorf`, and `f32::round` produces
`jmp roundf`. That made 42 `floorf` and 40 `roundf` calls per pixel, which is where most of the
filter's time went.

- **`sample()`:** `x.floor() as usize` became `x as usize`. The two arguments are clamped to
  `0 ..= w-1` on the lines above, so truncation is floor for every input the function can see
  (NaN truncates to 0 either way, which is the same sample the old code read).
- **Tangent and bristle lookups:** `round` became `round_i32(v) = (v as f64 + 0.5 * sign(v)) as i32`.
  `f32::round` is round-half-away-from-zero, which is truncation of `v ± 0.5` when that sum is
  exact. In `f32` it is not always: `0.49999997 + 0.5` (`0.5 - 2^-25`) rounds up to `1.0`, so an
  `f32` sum gives 1 where `f32::round` gives 0. In `f64` the sum is exact for every `f32` the
  `i32` cast does not saturate, so the integer is identical for every input; a unit test checks
  every `f32` bit pattern in [-4096, 4096] against `f32::round`. `round_index` adds the `clamp(0, n-1)` the tangent
  lookup already applied; NaN casts to 0 and ±∞ casts to an end of the clamp range in both
  versions, so the clamped results match for every input, including non-finite ones.

No public API, `FilterParams`, tile size, halo or `perf/budgets.toml` entry changed. The change is
pure arithmetic: no new dependency, no allocation, no threading, so the single-threaded wasm path
is unaffected.

## Output equivalence

Bit-identical. Six 600x400 fixtures were rendered through `kernel()` on the base and on the
changed tree and serialized as little-endian `f32` bits, then compared with `cmp` and per channel:

| Fixture | Params | Max abs delta | FNV-1a 64 of the sample bits |
|---|---|---:|---|
| default | stylization 4, cleanliness 5, scale 1, bristle 5, lighting on, RGBA8 | 0.0 | `6831efd11acaf51d` |
| short | stylization 1 | 0.0 | `010cb6391d88b6fb` |
| long | stylization 10, cleanliness 0, scale 3, bristle 10 | 0.0 | `c70940a922e81504` |
| unlit | lighting off | 0.0 | `c882b9a99f8785cd` |
| rgb3 | RGB8, no alpha | 0.0 | `31854edbc3f1db90` |
| cmyk | CMYKA8 | 0.0 | `3532e07cf752f624` |

`cmp` reported no differences over 21.6 MB of samples; every per-channel maximum was 0.0. The
fixture is `px = [(x % 256), ((3x + y) / 7 % 256), ((x ^ y) % 251), ((x + 2y) % 199), 128 or 255]`
on the 600x400 grid, read as RGBA8 / RGB8 / CMYKA8.

## Timings

Intel Core i5-10300H (4 cores / 8 threads), 16 GB, Windows 11, rustc 1.98.1
(`48a229cea 2026-09-01`), default release profile, 6000x4000 RGBA8, Oil Paint at stylization 4,
cleanliness 5, scale 1, bristle detail 5, lighting on. Before and after are the same
`bench_filters2` binary arguments, built from `origin/main` and from this change.

Nothing else was running: five runs per side, alternating (before, after, after, before, before,
after, after, before, before, after), both binaries already built, on an otherwise idle machine.

| Case | Before (s) | After (s) | Speedup |
|---|---:|---:|---:|
| Oil Paint, 24 MP RGBA8 | 8.36 | 6.82 | 1.23x |

Before: 8.36, 9.68, 9.54, 8.36, 9.36 (median 9.36). After: 6.82, 7.55, 7.84, 6.92, 7.85 (median
7.55, so 1.24x on the medians). An earlier pass took its numbers while other builds shared the
machine and read 1.88x off inflated absolute times; the table above is the quiet measurement.

`f32::floor` and `f32::round` are libm calls at this target's baseline ISA — `rustc --emit asm -O`
for the host target prints `callq floorf` and `callq roundf`, while `x as usize` inlines — so the
LIC paid 42 of those calls per pixel. The change replaces them with a truncation (the argument was
clamped two lines earlier, so truncation is floor) and a half-away-from-zero cast. What is left is
the gathers themselves.

The remaining time is the 21 bilinear gathers per pixel. Those are not removable without changing
the walk itself:

- **Streamline reuse is not faithful.** A neighbouring start does not share the walk. The tangent
  is read at the rounded position, the step `h = max(len / 10, 1)` is at least a pixel, and the
  bilinear taps land on different sub-pixel phases for every start, so no two starts produce the
  same samples. Reusing a walk would change the filter, not just its cost.
- **The bristle noise plane, hoisted step weights, whole-pixel slices and a precomputed relief
  height plane** were built and measured as one variant against the change in this note: three
  alternating pairs gave 28.94/29.13, 27.16/26.96 and 30.12/31.97, so the difference is inside
  the spread of the machine and in both directions. They are not in the diff.
- **The per-tile halo work** is small. A 256x256 tile reads a 308x308 source window, and the
  tensor stage plus both `gauss_blur_n` passes are about 11% of the filter, so bounding the halo
  is worth at most ~3%.

## Reproduce

```sh
# base tree
git worktree add --detach ../photocraft-oil-base origin/main

# same example, both trees
(cd ../photocraft-oil-base && cargo build --release -p photocraft-algo --example bench_filters2)
cargo build --release -p photocraft-algo --example bench_filters2

# 24 MP Oil Paint, no other builds or benchmarks running
../photocraft-oil-base/target/release/examples/bench_filters2.exe oil
target/release/examples/bench_filters2.exe oil
```

Repeat each measurement at least three times, alternating before/after and after/before, and
compare the minima. For the equivalence check, call
`kernel(&FilterParams::OilPaint { .. }, &Image::read(&surface, rect), rect, &ctx)` on both trees
for the fixtures in the table, write `v.to_bits().to_le_bytes()` in order and `cmp` the files.

## Checks

- `cargo fmt --all --check`, `cargo clippy -p photocraft-algo --all-targets -- -D warnings`
- `cargo check -p photocraft-algo --target wasm32-unknown-unknown` still builds.
- `cargo test --release -p photocraft-algo`: 605 passed, 1 failed. The failure is
  `pixelate::tests::facet_reuse_matches_scalar_in_every_model`, which reports
  `sample 56: NaN != NaN` (0x7FC00000 against 0xFFC00000) and fails in exactly the same way on
  the unmodified base commit on this machine, so this change did not cause it.
- The oil tests pass: `oil_paint_smooths_along_strokes` and the new
  `rounding_matches_the_standard_library`, which compares `round_i32` and `round_index` with
  `f32::round` and the clamped cast on a 0.25 grid from -40 to 2100 plus NaN and ±∞.
