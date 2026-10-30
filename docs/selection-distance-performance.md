# Selection distance transforms

The distance transform used by Select > Modify > Expand, Contract and Border now runs
as two native parallel passes. It also serves matting and mask feathering callers in the
engine. The compositor has a separate implementation and is outside this change.

The first pass finds the nearest selected pixel above and below each pixel. Since its
input is binary, two sweeps replace the general parabola envelope for that axis. Vertical
distances are stored as integers in column order. The second pass copies 32-row bands
into contiguous rows and evaluates the lower envelope of squared-distance parabolas.
Both passes are linear in the number of pixels. WebAssembly uses the same scalar kernels.

The row envelope follows [Felzenszwalb and Huttenlocher, 2012](https://toc.cs.uchicago.edu/articles/v008a019/).
Its intersections use `(fq - fp) / (2 * (q - p)) + (q + p) / 2` in f64. This avoids the old
subtraction of f32 coordinate squares, which lost half-pixel boundary precision on large
canvases. Distances are rounded to f32 only at the output.

## Correctness

The previous transform can return a positive distance at a selected pixel past coordinate
4096. A 6000 x 1 fully selected mask reproduces it. The regression tests also compare sparse,
competing sites in 65537 x 2 and 2 x 65537 masks against direct Euclidean distances.

Tests enumerate every mask up to 4 x 3 against a brute-force oracle, check rectangular and
partial row bands against the previous implementation, preserve fractional Expand/Contract/
Border coverage on smaller inputs, and compare native worker counts 1, 2 and 4. Empty or
inconsistent dimensions return an empty buffer. No-seed masks retain the finite 1e10 result.
Empty and fully selected masks skip the transform passes.

Large benchmark inputs check every selected pixel has zero distance and check independent
nearest-site distances at corners and the centre. The sparse-grid cases match the old
output exactly. Ellipse and dense cases intentionally correct old distance errors, so this
is not a claim of identical output everywhere or of measured Photoshop parity.

## Measurements

Recorded 2026-10-10 on an AWS c7i.4xlarge in ap-southeast-2: 16 vCPUs, 32 GiB RAM,
Rust 1.99.0, the workspace's normal release profile, 16 Rayon workers. Synthetic masks are
an ellipse, a sparse regular grid, and a deterministic dense pattern. Input generation,
reference validation and destruction of the output happen outside the timed interval.
Each implementation gets a warmup, then three measured runs with alternating order.

| Size | Mask | Before p50 | After p50 | Speedup | Changed distances | Largest correction |
|---|---|---:|---:|---:|---:|---:|
| 24 MP | ellipse | 630.74 ms | 83.11 ms | 7.59x | 827,500 | 1 px |
| 24 MP | sparse | 621.81 ms | 71.03 ms | 8.75x | 0 | 0 px |
| 24 MP | dense | 1048.21 ms | 99.73 ms | 10.51x | 2,034,343 | 1.4142135 px |
| 36 MP | ellipse | 994.14 ms | 124.48 ms | 7.99x | 2,431,817 | 1 px |
| 36 MP | sparse | 984.52 ms | 109.94 ms | 8.96x | 0 | 0 px |
| 36 MP | dense | 1588.23 ms | 150.28 ms | 10.57x | 4,771,985 | 1.4142135 px |

These are kernel timings. Mask conversion, selection history, document compositing and
canvas upload are excluded. They do not establish an end-to-end interaction budget.

## Memory and limits

The old transform used one f32 plane. The new transform needs one u32 plane plus the f32
result, or eight bytes per pixel, excluding the caller's input mask. It also uses up to
`32 * width * 4 + width * (size_of::<usize>() + 8)` scratch bytes per active row task.
At 24 MP the two planes total 192 MB; at 36 MP they total 288 MB. Fully empty and fully
selected masks need only the result plane. Small mixed masks below 65536 pixels run serially.

The extra plane buys contiguous writes and independent parallel bands. This remains a dense
transform, so it does not solve streaming selections on very large PSB documents. Dimension
multiplication, element-size limits and input length are checked before allocation.

## Reproduce

```sh
cargo test --release -p photocraft-algo distance_transform_release_comparison -- --ignored --nocapture
EDT_WORKERS=1 cargo test --release -p photocraft-algo distance_transform_release_comparison -- --ignored --nocapture
```

The previous scalar implementation is retained only in the test module as the reference
and benchmark baseline.

## Cropped selection morphology (2026-10-26)

Expand, Contract and Border now keep distance work inside the nonzero mask bounds,
expanded by `ceil(radius) + 1` and clipped to the supplied canvas. Nonzero bounds include
fractional coverage below the binary 0.5 threshold. Real pixels in the halo provide
outside seeds; cropping does not introduce an artificial selection edge. The engine's
existing canvas-bound padding remains responsible for Apply effect at canvas bounds.

For radii up to 5 px, two vertical sweeps store capped distances in one byte per pixel.
A short horizontal stencil computes exact Euclidean distances only within `radius + 1`;
farther samples cannot contribute to the existing coverage formulas. Larger radii use
the general EDT above. Contract classifies complement seeds directly and writes the
complement coverage formula without allocating inverted float masks. Border runs its
two cropped transforms sequentially, reusing one seed mask and retaining one distance
plane at a time.

A 50×50 selection at radius 5 needs a 62×62 distance window (3,844 samples), even on a
36 MP canvas. Seed, vertical and output-distance buffers total about 23 KB on the bounded
path, excluding allocator overhead. The flat API still scans its supplied input for bounds and
returns an output of the same size (144 MB for a direct 36 MP call). The three Modify
commands now supply only the cropped selection window, including for mask conversion
and tiled replacement. Undo and compositing retain their existing behavior; this does
not establish a GUI latency budget or solve arbitrary sparse document operations.

Regression tests compare against the previous full-canvas formulas for empty/full masks,
one-row/column canvases, holes, edge-touching shapes, fractional radii, and fractional
coverage including complement rounding at 0.5. Every 3×3 binary mask is also checked
inside a larger canvas with a faint sample outside its binary bounds. The small-radius
distances are compared with the general EDT. Invalid dimensions and nonfinite/negative
radii return empty buffers.

Reproduce on the fork through the CI workflow's optional `selection_morphology` input.
It runs `cargo run --release -p photocraft-algo --example bench_selection_morphology`
on Linux, and `cargo xtask perf --quick` on the project’s macOS performance runner class. The comparison uses a 6000×6000 mask with either
a centred 50×50 square or a centred 4000×4000 square, radii 3 and 50, and four Rayon
workers. It checks every output sample against the previous formulas before timing;
reported medians include output allocation and exclude output destruction, with three
runs per implementation. These measurements are separate from the historical EDT-only
measurements above and do not change the scorecard baseline.

Measurements from [scratch CI job 121583341033](https://github.com/wanikhawar/photocraft/actions/runs/40326176434/job/121583341033),
2026-10-26, commit `5afd231138368f9be856e8c21c2946f2e386797c`, GitHub-hosted Ubuntu 26.04,
stable Rust, normal release profile. The benchmark step completed successfully; the
superseded run was cancelled during the later quick performance build. The full validation
run is recorded in the PR. Times below are milliseconds; this table reports Expand and Border (eight cases).
All twelve output comparisons, including Contract, matched exactly. The current benchmark
also times Contract using its original owning iterator so allocation reuse is preserved.

| Selection | Operation | Radius | Before p50 | After p50 | Speedup |
|---|---|---:|---:|---:|---:|
| 50×50 | Expand | 3 | 359.55 | 94.51 | 3.80× |
| 50×50 | Border | 3 | 738.30 | 19.52 | 37.82× |
| 50×50 | Expand | 50 | 357.84 | 99.76 | 3.59× |
| 50×50 | Border | 50 | 738.50 | 20.08 | 36.79× |
| 4000×4000 | Expand | 3 | 416.83 | 171.26 | 2.43× |
| 4000×4000 | Border | 3 | 802.48 | 377.78 | 2.12× |
| 4000×4000 | Expand | 50 | 414.50 | 268.96 | 1.54× |
| 4000×4000 | Border | 50 | 784.89 | 467.06 | 1.68× |


### Cropped command reads and writeback

The engine gets the selection surface's nonzero content bounds, inflates by
`ceil(radius) + 1`, clips to the canvas and reads only that rectangle for Expand,
Contract and Border. Internal rectangle edges have a zero halo, so existing padding
only replicates nonzero values at real canvas edges. A nonzero surface default selects
implicit pixels throughout the canvas and therefore uses the full canvas. Empty
selections clear normally. The existing oversized-canvas safety limit remains.

Replacement already accepts a rectangle and builds a fresh sparse selection surface,
so it writes only affected tiles and retains the existing undoable transaction. The
shared `current_mask` helper and Grow/Similar are unchanged. Smooth and Feather also
use command windows as described below.

Command tests compare against full-canvas reads, padding and replacement for empty/full
selections, both canvas-bounds settings, edge-touching masks, holes, fractional coverage,
nonzero implicit defaults and entirely off-canvas pixels. Undo and redo preserve the
selection. A synthetic 36 MP test verifies that a 50×50 selection at radius 5 supplies
only 62×62 samples and executes all three commands without full-canvas mask allocation.

The optional CI benchmark also runs:

```sh
cargo test --release -p photocraft-engine --lib sparse_modify_release_comparison -- --ignored --nocapture
```

This isolates command-window savings: both paths use the optimized algorithm kernels,
but the reference keeps full-canvas mask conversion and replacement. Both include the
undoable edit; session creation and full-canvas equality checks are outside timing.
Three rounds alternate execution order, with radii 3 and 50 on a 50×50 selection in a
6000×6000 canvas. The cropped path dispatches the actual command; the reference calls
the prior conversion/edit sequence directly. GUI rendering remains excluded.

Uniform bounded-distance masks retain the general EDT's no-sweep shortcuts: no seeds
return the finite empty-distance sentinel; all seeds return zero. The current algorithm
benchmark also includes a full-canvas selected mask to check Select All alongside the
sparse and dense partial selections. The historical table above covers its recorded
Expand/Border sparse and dense cases.
Border also skips distance planes and coverage passes when its binary mask is uniform:
there is no edge inside the supplied rectangle. Extreme library radii retain the finite
no-seed sentinel behavior; the command's canvas-edge padding still creates a real edge
for Select All.


## Cropped Smooth and Feather commands (2026-10-27)

All five Select › Modify commands now use the same bounded mask read and replacement.
Smooth grows the nonzero selection bounds by `ceil(radius) + 1`. Feather takes the
truncated Gaussian reach from `selection::feather_reach(radius)` and adds the same
one-pixel zero margin: `ceil(3 * radius) + 1` for the accepted command radii. Using the
blur's shared reach keeps command windows and canvas-edge padding in sync with the
kernel. Both windows are clipped to the canvas before any mask allocation.

For a 50×50 selection at radius 5 on a 6000×6000 canvas, the command supplies 62×62
samples to Smooth and 82×82 to Feather, instead of 36 million samples. Nonzero implicit
defaults and malformed tile coordinates retain the full-canvas fallback. The existing
canvas-size safety limit still applies.

The command comparisons above now include Smooth and Feather, fractional radii down
to Feather's 0.1 px minimum, both canvas-bound settings, soft coverage at 8/16/32-bit
sample depths, one-pixel/thin canvases, and undo/redo. They compare stored selection
coverage with the previous full-canvas path.

Dispatch the fork's CI with `selection_morphology=true` to run the release comparison
and the general quick performance suite. The existing
`sparse_modify_release_comparison` test now measures both blurs on sparse 50×50,
dense 4000×4000 and fully selected masks in a 36 MP canvas, at radii 3 and 50. It
alternates execution order for three rounds, reports median command times and verifies
every output sample outside timing. The full-canvas reference uses the same optimized
blur kernels and includes the undoable edit. GUI rendering and live-preview scheduling
are excluded; these measurements do not update the P26/P27 scorecard budgets.
