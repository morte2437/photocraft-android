# Crystallize and Pointillize performance

Measured on 2026-10-15, against `4e7a17cd5d5723313f459753663d0d155f047e19`.
The code revision is `3a527eb64e0d0108e44c70dfd200afc0ffd5fe67`.

## Whole command

The command bench calls `Session::execute` on a populated 6000 x 4000 RGB layer.
The table below uses 16 workers and 8-bit storage.
The clock includes filtering and the history commit. It excludes document setup,
GUI repaint and presentation. The last run also checks that Undo restores the
source and Redo restores the filtered result. Both checks happen outside the clock.

| Layer kind | Filter | Cell size | Before | After | Speedup |
|---|---|---:|---:|---:|---:|
| Layer | Crystallize | 10 | 560.8 ms | 137.5 ms | 4.08x |
| Layer | Pointillize | 5 | 430.5 ms | 146.4 ms | 2.94x |
| Layer | Crystallize | 100 | 1387.5 ms | 202.1 ms | 6.87x |
| Layer | Pointillize | 100 | 543.4 ms | 131.9 ms | 4.12x |
| Background | Crystallize | 10 | 891.5 ms | 441.1 ms | 2.02x |
| Background | Pointillize | 5 | 738.5 ms | 608.2 ms | 1.21x |
| Background | Crystallize | 100 | 1708.4 ms | 500.1 ms | 3.42x |
| Background | Pointillize | 100 | 851.4 ms | 439.0 ms | 1.94x |

Ordinary layers are unlocked and have varying alpha, including zero alpha.
Background layers are opaque and transparency-locked. The existing transparency
restoration pass limits their speedup, especially Pointillize at small cell sizes.
These are command timings, not input-to-display timings.

## Whole filter

The filter bench times `apply_in`: reading the input, filtering, converting the
result back to the document depth, writing the tiles and pruning. The table below uses 16 workers.
There is no selection in these timing cases. Correctness tests also cover soft selections.
The fixture has varying alpha, fully transparent pixels and a negative origin.
Every run filters the original surface again; no result cache survives a run.

| Format | Filter | Cell size | Before | After | Speedup |
|---|---|---:|---:|---:|---:|
| RGB u8 | Crystallize | 10 | 601.6 ms | 137.8 ms | 4.36x |
| RGB u8 | Pointillize | 5 | 449.9 ms | 147.3 ms | 3.05x |
| RGB u8 | Crystallize | 100 | 1340.5 ms | 175.1 ms | 7.65x |
| RGB u8 | Pointillize | 100 | 526.2 ms | 128.1 ms | 4.11x |
| RGB u16 | Crystallize | 10 | 647.3 ms | 214.0 ms | 3.02x |
| RGB u16 | Pointillize | 5 | 502.7 ms | 219.8 ms | 2.29x |
| RGB u16 | Crystallize | 100 | 1413.8 ms | 262.2 ms | 5.39x |
| RGB u16 | Pointillize | 100 | 580.5 ms | 209.3 ms | 2.77x |
| RGB f32 | Crystallize | 10 | 688.4 ms | 362.7 ms | 1.90x |
| RGB f32 | Pointillize | 5 | 551.5 ms | 360.9 ms | 1.53x |
| RGB f32 | Crystallize | 100 | 1495.2 ms | 412.1 ms | 3.63x |
| RGB f32 | Pointillize | 100 | 643.7 ms | 361.6 ms | 1.78x |
| CMYK u16 | Crystallize | 10 | 729.5 ms | 260.2 ms | 2.80x |
| CMYK u16 | Pointillize | 5 | 574.6 ms | 262.2 ms | 2.19x |
| CMYK u16 | Crystallize | 100 | 1560.3 ms | 329.1 ms | 4.74x |
| CMYK u16 | Pointillize | 100 | 675.0 ms | 255.6 ms | 2.64x |
| LAB f32 | Crystallize | 10 | 706.2 ms | 367.6 ms | 1.92x |
| LAB f32 | Pointillize | 5 | 550.4 ms | 361.9 ms | 1.52x |
| LAB f32 | Crystallize | 100 | 1469.7 ms | 404.9 ms | 3.63x |
| LAB f32 | Pointillize | 100 | 647.8 ms | 362.2 ms | 1.79x |
| GRAY u8 | Crystallize | 10 | 396.0 ms | 93.4 ms | 4.24x |
| GRAY u8 | Pointillize | 5 | 332.9 ms | 109.2 ms | 3.05x |
| GRAY u8 | Crystallize | 100 | 1020.5 ms | 105.1 ms | 9.71x |
| GRAY u8 | Pointillize | 100 | 352.9 ms | 76.7 ms | 4.60x |

All trials and output hashes are in [pixelate-cell-timings.json](pixelate-cell-timings.json).
Filter hashes are SHA-256 of the complete tile-ordered normalized f32 output.
Command hashes are BLAKE3 of the complete tile-ordered native storage bytes.
Fixture generation, output dumps and hashing are outside the measured interval.

## Where the time went

Both filters were rebuilding the seeded cell geometry for every pixel and every
neighbour. Pointillize also repeated the dot radius, source lookup and colour
jitter. Those values belong to the cell. The new grid prepares them once per cell
in a tile, reducing that setup work from roughly nine times the pixel count to
the number of cells.

Crystallize accumulated a premultiplied average for every cell, then usually threw
it away and used the pixel at the cell centre. Only cells whose centres are outside
the image need that average. The new path skips interior averages. Boundary cells
still add their samples in the original row order, with the original f32 rounding.

Larger cells also allow whole horizontal spans to share a winner. For each site,
the absolute horizontal distance has its maximum at an endpoint. Clamping the site
centre to the span gives a lower bound. The rounded coordinate map is monotone;
squaring nonnegative distances, adding the fixed vertical term and taking a square
root preserve these bounds through the same f32 operations. Subtracting distance
from a fixed dot radius reverses the endpoints for Pointillize. If one site's
worst score beats every other site's best score, it owns the span. Otherwise the span splits. Single pixels retain the original strict
comparison and tie order. A fully covered Pointillize span also has exactly unit
coverage, so it needs no per-pixel square roots.

The tile path reads sparse cell-centre samples directly from the surface.
Crystallize's boundary tiles retain the dense input for their averages. Extent
clamping and selection mixing follow the original path.

After these changes, profiling showed a serial tail: converting samples and
writing them back took about 160-180 ms of a roughly 260 ms operation. The two
filters now convert on their existing tile workers. Committing a group copies
encoded bytes. Integer rounding and native byte order are the same as
`Surface::write_region`; float bits are kept. The optional site grid is capped at
16 MiB. Failed grid/average reservations use the scalar kernel; a failed encode
reservation keeps the existing floating-point writeback.

## Reproduce

Machine: EC2 c7i.4xlarge, 16 vCPUs (8 cores with SMT), 32 GiB RAM, Intel Xeon
Platinum 8488C, Ubuntu 24.04, rustc 1.99.0. Release profile, thin LTO, one codegen
unit. No `target-cpu` override. Five trials per row; tables use the median.
One-worker rows are in the raw results too. Before and after use separate target
directories so Cargo cannot reuse a library from the other worktree. No builds
ran during the timed trials.

```sh
RAYON_NUM_THREADS=16 cargo run --release -p photocraft-algo --example bench_pixelate -- crystallize 6000 4000 u8 5 /tmp/crystallize.bin rgb
RAYON_NUM_THREADS=16 cargo run --release -p photocraft-algo --example bench_pixelate -- pointillize-large 6000 4000 u16 5 /tmp/pointillize.bin cmyk
RAYON_NUM_THREADS=16 cargo run --release -p photocraft-engine --example bench_cell_filters -- crystallize 10 5 8 layer
RAYON_NUM_THREADS=16 cargo run --release -p photocraft-engine --example bench_cell_filters -- pointillize 100 5 8 layer
RAYON_NUM_THREADS=16 cargo run --release -p photocraft-engine --example bench_cell_filters -- pointillize 5 5 8 background
```

For the before run, create a worktree at the base above and copy the two benchmark
examples into it. Build with a separate `CARGO_TARGET_DIR` and run there with the same arguments
and thread count.
The `-large` filter cases use cell size 100; the other cases use 10 for Crystallize
and 5 for Pointillize. The engine example takes the size explicitly.

## Validation

- 2,447 tests passed across algo and engine; 43 ignored.
- Clippy passed for both crates with all targets and warnings denied.
- Layer checks, wasm checks, scorecard freshness and the quick performance suite passed.
- Scalar comparisons cover RGB, Gray, CMYK, Lab and Multichannel, alpha on/off,
  negative origins, fractional cells, boundary averages, HDR and nonfinite samples.
- Storage comparisons cover U8/U16/F32, soft selections, clamped extents and
  several tile sizes. The span tests compare discrete winners and coverage,
  including coordinates near f32 precision boundaries.
- All 44 measured before/after configurations have identical output hashes.
  The 16 command configurations also pass Undo/Redo checks.
