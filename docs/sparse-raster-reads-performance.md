# Sparse raster reads

Skip absent zero-default tiles and avoid temporary default-pixel allocations and copies. ZIP CRC acceleration is already on main.

A roughly 4% median dense 16-bit city-photo read slowdown repeated, with paired intervals crossing zero. The local fix shares the existing contiguous row loop between allocating and reusable-buffer reads, removes duplicated code, and preserves zero-filled allocation for allocating reads. Final measurements include all photo depths and authored designs; all outputs match bit for bit.

Measured on 2026-10-19 against main `d95fdf3181c01ad41e95456091a394b105a9afb2`, independently of the other splits. Apple M4 Max, 16 CPUs, 128 GiB, macOS arm64, rustc 1.99.1; release O3, LTO off, 16 codegen units, debug off. Builds used nice 10 and warm APFS-cloned caches; measurements used default priority without concurrent builds or profiling. Five alternating paired samples follow a warmup; confirmation and fix runs use nine pairs. Time change compares the two medians; intervals resample paired ratios. Negative time change means faster. Small differences are not automatically established improvements or regressions.

The full-resolution CC0 Pixls photographs are P1000475.RW2 (24 MP foliage centre crop, no resampling) and IMG_4059.CR2 (12 MP city), each converted to U8/U16/F32 with the ProPhoto RGB profile. F32 derives from 16-bit development, not HDR capture. MIT psd-tools masks.psd supplies a 640 x 1394 web layout with 22 imported layers and three masks; mask-index.psd supplies a 1000 x 867 illustrated card with a mask. The sources and hashes are pinned by the existing corpus. These are the available authored layered files, not a large layered-photo corpus. No generated noise, repeating ramps, enlarged charts or duplicated photo layers are used.

The shared harness times in-memory operations. Input loading, image conversion, thumbnail rendering, hashes and equivalence checks are outside timers. Every save uses a fresh PcraftWriter and a real rendered thumbnail. Common reads use main-encoded files; own-output reads are separate. PSD exports use ZIP merged images at U8/U16, Raw merged images at F32, and normal RLE layer channels at U8/U16 / ZIP prediction at F32. Results do not measure filesystem throughput or browser performance.

| Document | Operation | Main ms | Branch ms | Time change | Paired 95% interval | Main / branch bytes |
|---|---|---:|---:|---:|---:|---:|
| panasonic-16 | read (fixed) | 54.689 | 41.140 | -24.8% | -28.0..-23.8% | — |
| panasonic-16 | render (fixed) | 38.492 | 38.345 | -0.4% | -1.8..+2.0% | — |
| canon-16 | read (fixed) | 27.634 | 19.236 | -30.4% | -34.0..-23.3% | — |
| panasonic-8 | read (fixed) | 50.429 | 20.210 | -59.9% | -62.2..-56.7% | — |
| panasonic-8 | render (fixed) | 30.953 | 30.670 | -0.9% | -3.4..+1.5% | — |
| panasonic-32 | read (fixed) | 91.120 | 69.922 | -23.3% | -23.6..-20.7% | — |
| panasonic-32 | render (fixed) | 39.436 | 39.867 | +1.1% | -4.5..+3.7% | — |
| canon-8 | read (fixed) | 24.899 | 10.543 | -57.7% | -58.5..-54.9% | — |
| canon-8 | render (fixed) | 16.925 | 16.901 | -0.1% | -5.6..+5.5% | — |
| canon-16 | render (fixed) | 21.569 | 21.716 | +0.7% | -2.2..+2.4% | — |
| canon-32 | read (fixed) | 45.494 | 35.200 | -22.6% | -23.5..-21.5% | — |
| canon-32 | render (fixed) | 22.056 | 22.180 | +0.6% | -1.8..+6.9% | — |
| layout-masks | read (fixed) | 35.240 | 3.438 | -90.2% | -90.7..-89.9% | — |
| layout-masks | render (fixed) | 8.167 | 7.461 | -8.6% | -10.6..-8.3% | — |
| layout-mask-index | read (fixed) | 2.314 | 1.051 | -54.6% | -61.7..-50.0% | — |
| layout-mask-index | render (fixed) | 1.926 | 1.827 | -5.1% | -13.0..+0.6% | — |

All real read-region and render float bits match main. The committed previous-read-region oracle also covers sparse boundaries, negative coordinates, signed zero, defaults and eight pixel formats.

Current local checks: `cargo test -p photocraft-raster --locked --lib` (27 passed, one ignored manual benchmark); strict all-target Clippy; affected wasm32 check. All passed. The existing previous-read-path oracle covers the shared loop.

Local raw evidence and the shared harness: `.local/perf-real-261019/` in the repository checkout (ignored). This comparison supersedes the earlier synthetic performance table. Small synthetic unit fixtures remain correctness checks; ignored synthetic timing tests were not run.

## Earlier split validation

These checks passed when the split was prepared; they were not all rerun for this benchmark review. Current affected checks are listed above where production code changed.

- `cargo test -p photocraft-raster --locked`: passed (27 passed, 1 ignored).
- `cargo clippy -p photocraft-raster --locked --all-targets -- -D warnings`: passed.
- `cargo xtask layers`: passed.
- `cargo check -p photocraft-raster --locked --target wasm32-unknown-unknown`: passed.
