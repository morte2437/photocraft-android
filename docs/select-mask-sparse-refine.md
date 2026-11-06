# Sparse Select and Mask refinement (#8822)

Measured 2026-11-01. Refinement writes a zero-default GRAY8 Surface, skips empty tiles,
shares solid interiors, and filters mixed blocks in bounded native parallel batches.
The queue holds at most 24 blocks and its conservative combined scratch estimate is
limited to 512 MiB. Wasm processes the same bounded work sequentially.

## Release measurements

[Benchmark run](https://github.com/wanikhawar/photocraft/actions/runs/41329979528),
Ubuntu 24.04 hosted runner, `RAYON_NUM_THREADS=4`, five repetitions per configuration,
alternating queue-size order. All rows below come from that same run. Values are medians.
The benchmark compares the previous dense tiled implementation (retained under `cfg(test)`)
with the sparse implementation and varies only its queue size. Production uses a fixed
queue limit; the benchmark's environment-variable tuning exists only on the scratch branch.

| Fixture | Dense time (ms) | Sparse queue 4 (ms) | Sparse queue 24 (ms) | Dense peak RSS (KiB) | Sparse queue 24 peak RSS (KiB) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Small selection on 6000 × 4000 canvas | 6.819 | 6.792 | 6.968 | 9,352 | 9,332 |
| Select All, 6000 × 4000 | 54.782 | 12.863 | 12.309 | 56,220 | 12,892 |
| Thin diagonal, 6000 × 4000 | 434.603 | 483.662 | 348.148 | 70,868 | 54,208 |
| Select All, 100,000 × 100,000 | Not run | 501.262 | 455.635 | Not run | 113,988 |
| Thin diagonal, 100,000 × 100,000 | Not run | 12,305.541 | 8,475.229 | Not run | 198,912 |

Queue 24 reduces the 24 MP diagonal time by 28% against queue 4 and by 20% against
the dense reference. The 100k² diagonal improves by 31% against queue 4. The small
selection is essentially unchanged; the measurements do not establish a universal speedup.
Queue 24 uses more memory than queue 4 on diagonals (54,208 vs 43,016 KiB for 24 MP;
198,912 vs 186,944 KiB for 100k²), while keeping work bounded.

Each measurement runs in a fresh process. Time covers refinement and, for the dense
reference, conversion to the output Surface. RSS is the whole process high-water mark,
including fixture construction and output checksum calculation; it is not the scratch
estimate or a measurement of the desktop app's total memory. Image sampling uses a
constant synthetic RGBA8 source, radius 4 and feather 1; the diagonal is a 3 px brush
stroke. These fixtures do not measure a textured guide, every radius, or every colour mode.
The dense reference is never executed on 100k² fixtures.

Output checksums cover tile coordinates and tile bytes, reusing hashes for shared buffers.
Every implementation and queue size produced the same checksum for each fixture.
Small-image regression tests separately compare dense and sparse bytes directly,
including non-aligned origins, holes, soft coverage, smart radius, and add/restore strokes.

Reproduce using the measurement script and fixture at
[scratch commit 3f41ac0b](https://github.com/wanikhawar/photocraft/tree/3f41ac0b853d7eeb50815665fc0dc2b626709529/evidence-8822).
The scratch workflow and runtime queue selector are excluded from the feature PR.

## Preview checks

[Preview evidence run](https://github.com/wanikhawar/photocraft/actions/runs/41322345403)
compares the previous dense Coverage implementation with sparse float-tile Coverage.
All six decoded RGBA pairs match exactly: Overlay, Black & White, and On Layers, each
with Show Edge off/on, using overlapping add/restore brush strokes. The overlay and
edge-band captures were also inspected visually. These are composed preview images,
not full-window screenshots; unchanged workspace controls are not assessed by them.

## Remaining limits

Candidate work is capped at 1,048,576 blocks; scratch estimates, checked offsets and
fallible output writes reject oversized requests before changing the document.
Remove Background passes a cooperative interrupt; cancellation is checked while building
candidates, between blocks, before batch execution and on workers before filtering.
A cancelled refinement discards its partial output.
The sparse representation does not make an inherently dense, detailed output small.
Refine Hair, Object Aware and Presets remain open, so TOOL-214-1 stays partial and
readiness percentages and performance-budget enforcement are unchanged.
