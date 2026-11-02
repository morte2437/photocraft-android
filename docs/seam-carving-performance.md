# Content-Aware Scale performance

## Seam carving keeps its search across seams (2026-10-13, #5236)

Perf scenario P28 ([#211](https://github.com/storytold/photocraft/issues/211)) scales a 24 MP
photo to 90 % width with Edit › Content-Aware Scale. Its budget is 3 s.

- The committed baseline (2026-10-05) records 954 s. That predates the per-seam energy updates
  and the reduced-height seam search already in `crates/algo/src/seam.rs`.
- Before this change, the row measured 3.3 s on the machine below.
- After it, the row measures 1.6 s and the carve itself is 3.6 times faster. The output is the
  same.

### What changed

All changes are in `crates/algo/src/seam.rs`. The command, its parameters and its output are
unchanged.

- **The seam search keeps its path-cost map across seams.**
  - The search is the dynamic programme of Avidan and Shamir (2007). Before, it refilled the whole
    map for every seam: for P28, 600 serial passes over about 3 M cells.
  - After a removal, `Dp::update` recomputes row by row only two kinds of pixel: those next to the
    seam, whose energies or parents changed, and those with a parent whose cost actually changed
    (compared bit for bit). Nothing else can change, so the map stays identical to a full pass.
  - On the bench images this recomputes 1.3 % of the map per seam on the reduced-height search,
    and 6–10 % on the full-resolution one. 98–99 % of the recomputed pixels do change, so the work
    is the removed seam's basin, not slack in the bounds.
  - Directions are no longer stored. The seam is traced back from the costs with the same tie
    rules (`cell`). Rows are computed by one shared, vectorised kernel (`row_costs`).
- **The carver moves no pixels.**
  - It borrows the image, keeps each remaining pixel's source column (`u32`), and reads energy
    neighbours and the result through it.
  - A removal shifts whichever side of the seam is shorter, in three 4-byte per-pixel buffers
    (index, energy, cost). Before, it shifted everything right of the seam in up to four buffers,
    including the 16-byte RGBA pixels and, on the tracked paths, an 8-byte index.
  - The image is no longer copied.
- **The full-resolution stages run in parallel over rows.** These are the tall-image proxy build,
  its full-resolution rebuild (runs copied between removed columns), the height pass's transposes
  (blocked) and the plain resample used by Amount.

### Exactness

**Inputs whose energy stays a number** (every finite image without overflow) produce output that
is bit-identical to the previous code. These tests pin it:

- `output_bits_are_pinned`, for images under 2048 rows.
- `tall_proxy_output_bits_are_pinned`, for the reduced-height search. It was pinned from the
  previous code before any change.
- `carver_matches_reference`, which runs 400 random cases against the original straightforward
  implementation.
- `incremental_dp_matches_full_dp`, which after every removal checks the kept map, the seam, the
  energy and the pixels against a recomputation from scratch.
- The hashes of the four `bench_seam_cas` outputs.

**An adversarial review** compared the previous and new code side by side over more than 37 000
generated cases. Inputs included huge values, denormals, -0.0, out-of-range protection, enlarging
past 2× and proxy-size images. No difference appeared on any input whose energy stays a number.

**NaN and infinities** are now deterministic on every platform:

- An energy that isn't a number is the highest energy (`+inf`). Such energies come from NaN
  samples, from infinite samples at the border or next to each other, and from overflowing
  differences.
- NaN protection protects nothing.

Before, these inputs fed NaN to the seam search. The chosen seam then depended on NaN payloads,
which arithmetic does not keep consistently across builds or on wasm. `nan_payloads_do_not_steer_seams`
and `infinite_samples_carve_deterministically` pin the new behaviour.

**The height pass now keeps infinite protection.** It resamples the protection map; where that
gives NaN, the map clamped to 0..=1 is resampled instead. Before, a row protected with `+inf`
was carved away while one protected with `1.0` was kept.

**Degenerate inputs now return early instead of panicking.** These are no channels, an empty
width with a height change, and images or protection maps shorter than their size.

### Measurements

**Machine and method:**

- 4-vCPU cloud VM (Intel Xeon 2.1 GHz, Linux, no GPU), release builds.
- The VM's speed drifts by about 20 % over time, so before and after binaries ran interleaved in
  the same window. Reported values are p50 with the [min–max] range.

**The carver alone:**

- Command: `cargo run --release -p photocraft-algo --example bench_seam_cas`.
- Input: a synthetic photo-like RGBA image.
- Five interleaved rounds; the output hashes were identical in every run.

| Case | Before | After | Speed-up |
|---|---|---|---|
| 24 MP to 90 % width (P28's carve, reduced-height search) | 2013 ms [1958–2149] | 556 ms [542–623] | 3.6× |
| 24 MP to 90 % height (transposed) | 2639 ms [2617–2809] | 993 ms [967–1017] | 2.7× |
| 5.4 MP to 90 % width (full-resolution search) | 1548 ms [1514–1648] | 677 ms [664–719] | 2.3× |
| 1.5 MP to 70 % width (full-resolution search) | 671 ms [658–766] | 406 ms [376–421] | 1.65× |

**The P28 row:**

- Command: `perf_scenarios --cpu --only "Content-Aware Scale"`.
- It also times reading the layer, the job and writing the result back.
- Five interleaved rounds, n = 1 per run:

| P28 row | Before | After |
|---|---|---|
| Time | 3281 ms [3178–3364] | 1639 ms [1503–1728] |
| Peak RSS | 1368–1386 MB | 1305–1367 MB |

`cargo run --release -p photocraft-engine --example bench_content_aware_scale` times the same
command without building the UI crate. After the change, one run split as follows:

| Stage | Time |
|---|---|
| Reading the layer region | about 0.3 s |
| The carve | about 0.5 s |
| Writing the result back, with the undo step | about 0.2 s |

**Memory inside the carver** is now 12 bytes per pixel. Before, it was the image copy (4 bytes per
channel), the energy and the stored directions, plus protection and an 8-byte index where used.

- The only case that uses more is a one-channel shrink without protection: 9 → 12 bytes per pixel.
  The review measured its peak 1.17× higher.
- Every other case uses less.

### Limits and what is still open

- **The scorecard and enforcement.** The scorecard keeps showing the 2026-10-05 baseline until
  `cargo xtask perf --update-baseline` runs on the reference machine. `enforce` stays false for
  P28 until then.
- **The rest of the P28 row** is now mostly plumbing outside the carver. `Surface::read_region`
  converts the 24 MP layer pixel by pixel on one thread, and `write_region` also runs on one
  thread.
- **Full-resolution searches.** Enlarging still runs them, since only shrinking tall images uses
  the reduced-height search. The review measured a 24 MP 1 % enlarge going from 7.0 s to 1.8 s on a
  loaded machine. What remains per seam is the removed seam's basin in the cost map (serial, row by
  row) and the row shifts.
- **A worst case.** When every seam hugs the right edge of a one-sided smooth ramp, the old code
  shifted nothing, but the new code's cost changes cover almost the whole map. The review measured
  that case 1.3–1.5× slower than before. No other input it tried was slower.
- **wasm** builds run the same code on one thread.

## P28's recorded number is stale (2026-10-22, #8526)

`perf/baseline.json` records P28 (Content-Aware Scale on 24 MP to 90 % width) as 954 192 ms from the
2026-10-05 baseline, which is why `perf/budgets.toml` carries `enforce = false` for that row. The
seam-search proxies and the per-seam energy update landed after that capture:

| Tree | 6000 × 4000 → 5400, 4 ch |
|---|---:|
| `f5d2bcb` (the 2026-10-05 baseline commit) | 266 039 ms |
| `1104b6f` (main with #8526, before #5236) | 2 559 ms |

`f5d2bcb` has no `PROXY_SHRINK_MIN_HEIGHT`: it carves at full height with a full energy recompute per
seam. The 954 s in the baseline was that code on a machine whose recorded load average was 226–290
on 14 cores. The two `bench_seam` runs above are 104× apart (266 039 ms against 2 559 ms); that is the
seam-search core alone, and it is not the same measurement as the row's 954 s, which also includes a
document read and writeback the timer above excludes.

The row itself is not flipped to `enforce = true` here: the budget is set for the macOS/aarch64
machine class and only a run on that class can vouch for it. The carving core was 2.56 s at
24 MP on a 4-core laptop CPU before #5236; the budgeted machine is faster, but the row also includes the document
read and writeback this measurement excludes. Flip it in the PR whose nightly run shows the row
under 3 s, and delete this paragraph with it.

## The shrink proxy's column map (2026-10-22, #8526)

Main had dropped the proxy search's per-pixel original-column map in favour of counting the columns
already taken from each row (`record_removed`, −15 % at 24 MP). The carver above keeps a 4-byte
source column per remaining pixel and returns original columns itself, so that counting step has
nothing left to do and was removed when #5236 joined main; the tall-image output is unchanged
(`tall_proxy_output_bits_are_pinned`).

Its example times the deep shrinks it was measured on:

```sh
cargo build --release -p photocraft-algo --example bench_seam
./target/release/examples/bench_seam 6000 4000 600 4
./target/release/examples/bench_seam 6000 4000 3000 4
```

On main with #8526 these printed fnv `0f7eaa6af3cf1557` and `1780ee9bc9077ee9`. Both changes keep
the output unchanged, so the same hashes are expected; #8526's timings were taken on a different
base than the ones above and are not comparable with them.
