# PSD pixel buffer allocation

Write PSD interleaved pixels directly into the final buffer. Direct RLE output is already on main. The unrelated export-side sample-encoder inlining hint is removed locally.

Whole-photo imports improve by about 4–7%, and the authored designs by about 20–26%. An added export-side inlining hint accompanied a separately confirmed 2.3% float PSD export slowdown (2440.168 → 2497.423 ms). Removing that one line leaves export source equal to main; follow-up float export is +0.4% and 16-bit export +0.9%, with both paired intervals crossing zero. These results supersede the much larger previous interleave-only percentages.

Measured on 2026-10-19 against main `d95fdf3181c01ad41e95456091a394b105a9afb2`, independently of the other splits. Apple M4 Max, 16 CPUs, 128 GiB, macOS arm64, rustc 1.99.1; release O3, LTO off, 16 codegen units, debug off. Builds used nice 10 and warm APFS-cloned caches; measurements used default priority without concurrent builds or profiling. Five alternating paired samples follow a warmup; confirmation and fix runs use nine pairs. Time change compares the two medians; intervals resample paired ratios. Negative time change means faster. Small differences are not automatically established improvements or regressions.

The full-resolution CC0 Pixls photographs are P1000475.RW2 (24 MP foliage centre crop, no resampling) and IMG_4059.CR2 (12 MP city), each converted to U8/U16/F32 with the ProPhoto RGB profile. F32 derives from 16-bit development, not HDR capture. MIT psd-tools masks.psd supplies a 640 x 1394 web layout with 22 imported layers and three masks; mask-index.psd supplies a 1000 x 867 illustrated card with a mask. The sources and hashes are pinned by the existing corpus. These are the available authored layered files, not a large layered-photo corpus. No generated noise, repeating ramps, enlarged charts or duplicated photo layers are used.

The shared harness times in-memory operations. Input loading, image conversion, thumbnail rendering, hashes and equivalence checks are outside timers. Every save uses a fresh PcraftWriter and a real rendered thumbnail. Common reads use main-encoded files; own-output reads are separate. PSD exports use ZIP merged images at U8/U16, Raw merged images at F32, and normal RLE layer channels at U8/U16 / ZIP prediction at F32. Results do not measure filesystem throughput or browser performance.

| Document | Operation | Main ms | Branch ms | Time change | Paired 95% interval | Main / branch bytes |
|---|---|---:|---:|---:|---:|---:|
| panasonic-32 | psd-export (fixed) | 2471.456 | 2481.877 | +0.4% | -0.1..+1.7% | 461,042,978 / 461,042,978 |
| panasonic-16 | psd-export (fixed) | 2649.246 | 2673.071 | +0.9% | -5.2..+1.6% | 274,409,683 / 274,409,683 |
| panasonic-8 | psd-import | 275.828 | 262.237 | -4.9% | -5.9..-4.1% | 98,823,859 / 98,823,859 |
| panasonic-8 | psd-export | 2937.854 | 2915.929 | -0.7% | -11.5..+5.6% | 98,823,859 / 98,823,859 |
| panasonic-16 | psd-import | 365.901 | 349.827 | -4.4% | -4.4..-2.6% | 274,409,683 / 274,409,683 |
| panasonic-32 | psd-import | 360.706 | 334.838 | -7.2% | -12.6..-4.4% | 461,042,978 / 461,042,978 |
| canon-8 | psd-import | 141.331 | 135.878 | -3.9% | -5.1..-3.6% | 56,209,597 / 56,209,597 |
| canon-8 | psd-export | 1052.421 | 1054.472 | +0.2% | -0.6..+1.3% | 56,209,597 / 56,209,597 |
| canon-16 | psd-import | 177.327 | 169.952 | -4.2% | -5.5..-1.7% | 140,673,004 / 140,673,004 |
| canon-16 | psd-export | 1248.449 | 1239.159 | -0.7% | -1.6..+0.6% | 140,673,004 / 140,673,004 |
| canon-32 | psd-import | 175.721 | 163.271 | -7.1% | -9.3..-4.9% | 228,234,238 / 228,234,238 |
| canon-32 | psd-export | 1008.772 | 1005.356 | -0.3% | -1.8..+0.7% | 228,234,238 / 228,234,238 |
| layout-masks | psd-import | 12.509 | 9.981 | -20.2% | -20.8..-14.5% | 493,006 / 493,006 |
| layout-masks | psd-export | 55.113 | 54.714 | -0.7% | -2.3..+1.2% | 493,006 / 493,006 |
| layout-mask-index | psd-import | 3.299 | 2.443 | -25.9% | -30.8..-23.4% | 411,435 / 411,435 |
| layout-mask-index | psd-export | 18.200 | 18.397 | +1.1% | -0.1..+1.7% | 411,435 / 411,435 |

Real PSD exports remain byte-identical and imported layer/mask float samples match main. The committed literal previous-interleave oracle covers depths, bands, fills, truncation and inversion.

Current local checks: affected `pixels::` unit tests (five passed, one ignored manual benchmark); strict all-target IO Clippy; affected IO wasm32 check. All passed. Follow-up exports at float and 16-bit depth remain byte-identical to main; no synthetic timing test was run.

Local raw evidence and the shared harness: `.local/perf-real-261019/` in the repository checkout (ignored). This comparison supersedes the earlier synthetic performance table. Small synthetic unit fixtures remain correctness checks; ignored synthetic timing tests were not run.

## Earlier split validation

These checks passed when the split was prepared; they were not all rerun for this benchmark review. Current affected checks are listed above where production code changed.

- `cargo test -p photocraft-io --locked`: passed (617 passed, 6 ignored).
- `cargo clippy -p photocraft-io --locked --all-targets -- -D warnings`: passed.
- `cargo xtask layers`: passed.
- `cargo check -p photocraft-io --locked --target wasm32-unknown-unknown`: passed.
- `cargo xtask test-corpus -p io`: passed (646 passed, 6 ignored).
