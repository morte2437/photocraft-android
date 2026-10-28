# Deflate backend comparison

Replace the workspace flate2 Rust backend with zlib-rs and runtime CPU detection. Tiles in .pcraft remain Zstandard; deflate changes its PNG previews.

Size, import-speed and dependency tradeoffs remain maintainer decisions.
Some PSD ZIP files grow by up to 0.71%. The macOS 16-bit city PSD import is about 6% slower in confirmation.
The small web-layout PNG grows by 598 bytes (+1.40%). One .pcraft preview adds 25 bytes.
Real photo PNGs become smaller and encode faster. The default and best levels are unchanged; callers that
used level 1 (PDF export and print, brush tips, PNG Fast) now use level 3, because zlib-rs level 1 writes
static Huffman blocks only (13-32% bigger on photos). Level 3 is never bigger than miniz's level 1 on the
docs/images photos (0-16% smaller) and takes 1.2-3.4 times as long.
There is no native-format tile speedup from deflate.

The timings below measure only the backend switch. They exclude the later ZIP allocation, validation and slice decoder changes.

Measured on 2026-10-19 against main `d95fdf31`, independently of the other performance changes.
Host: Apple M4 Max, 16 CPUs, 128 GiB RAM, macOS arm64, rustc 1.99.1.
Release settings: optimization level 3, no LTO, 16 codegen units and no debug information.
Builds finish before measurement. Measurements use default priority without concurrent builds or profiling.
Five alternating pairs follow a warmup. Confirmation uses nine pairs.
Time changes compare medians, and intervals resample paired ratios. Negative changes mean less elapsed time.
Small differences are not automatically established improvements or regressions.

The full-resolution CC0 Pixls photographs are P1000475.RW2 (24 MP foliage centre crop, no resampling) and IMG_4059.CR2 (12 MP city), each converted to U8/U16/F32 with the ProPhoto RGB profile. F32 derives from 16-bit development, not HDR capture. MIT psd-tools masks.psd supplies a 640 x 1394 web layout with 22 imported layers and three masks; mask-index.psd supplies a 1000 x 867 illustrated card with a mask. The sources and hashes are pinned by the existing corpus. These are the available authored layered files, not a large layered-photo corpus. No generated noise, repeating ramps, enlarged charts or duplicated photo layers are used.

The shared harness times in-memory operations. Input loading, image conversion, thumbnail rendering, hashes and equivalence checks are outside timers. Every save uses a fresh PcraftWriter and a real rendered thumbnail. Common reads use main-encoded files; own-output reads are separate. PSD exports use ZIP merged images at U8/U16, Raw merged images at F32, and normal RLE layer channels at U8/U16 / ZIP prediction at F32. Results do not measure filesystem throughput or browser performance.

| Document | Operation | Main ms | Branch ms | Time change | Paired 95% interval | Main / branch bytes |
|---|---|---:|---:|---:|---:|---:|
| canon-16 | psd-import (confirm) | 168.891 | 178.614 | +5.8% | +3.5..+5.9% | 140,673,004 / 140,673,004 |
| panasonic-8 | save | 166.659 | 162.322 | -2.6% | -3.9..-1.0% | 49,643,217 / 49,643,242 |
| panasonic-8 | load-common | 410.794 | 409.622 | -0.3% | -0.5..+0.3% | 49,643,217 / 49,643,217 |
| panasonic-8 | png-encode | 373.880 | 165.381 | -55.8% | -56.3..-47.4% | 24,569,965 / 24,238,416 |
| panasonic-8 | png-decode | 168.141 | 167.247 | -0.5% | -2.0..+1.0% | 24,569,965 / 24,569,965 |
| panasonic-8 | psd-export | 2938.276 | 1115.740 | -62.0% | -62.3..-61.6% | 98,823,859 / 98,837,585 |
| panasonic-8 | psd-import | 275.713 | 174.046 | -36.9% | -38.8..-34.7% | 98,823,859 / 98,823,859 |
| panasonic-16 | save | 357.998 | 355.900 | -0.6% | -1.8..+0.5% | 175,681,852 / 175,681,844 |
| panasonic-16 | load-common | 387.986 | 391.411 | +0.9% | -1.3..+1.1% | 175,681,852 / 175,681,852 |
| panasonic-16 | png-encode | 354.924 | 288.664 | -18.7% | -27.8..-14.2% | 111,583,441 / 109,241,690 |
| panasonic-16 | png-decode | 434.083 | 433.645 | -0.1% | -1.0..+1.7% | 111,583,441 / 111,583,441 |
| panasonic-16 | psd-export | 2645.838 | 2249.281 | -15.0% | -15.6..-13.8% | 274,409,683 / 275,654,405 |
| panasonic-16 | psd-import | 371.291 | 374.777 | +0.9% | -1.1..+1.8% | 274,409,683 / 274,409,683 |
| panasonic-32 | save | 639.677 | 631.556 | -1.3% | -5.2..+1.8% | 252,231,671 / 252,231,663 |
| panasonic-32 | load-common | 1338.160 | 1326.936 | -0.8% | -1.1..-0.2% | 252,231,671 / 252,231,671 |
| panasonic-32 | psd-export | 2522.700 | 1352.144 | -46.4% | -49.6..-44.6% | 461,042,978 / 459,545,262 |
| panasonic-32 | psd-import | 356.077 | 310.425 | -12.8% | -20.2..-4.7% | 461,042,978 / 461,042,978 |
| canon-8 | save | 89.139 | 85.965 | -3.6% | -8.9..+2.7% | 26,151,631 / 26,151,203 |
| canon-8 | load-common | 219.711 | 219.088 | -0.3% | -0.3..+1.0% | 26,151,631 / 26,151,631 |
| canon-8 | png-encode | 179.944 | 80.123 | -55.5% | -60.8..-40.0% | 13,720,268 / 13,551,023 |
| canon-8 | png-decode | 69.704 | 70.040 | +0.5% | -0.4..+0.6% | 13,720,268 / 13,720,268 |
| canon-8 | psd-export | 1074.094 | 475.802 | -55.7% | -56.1..-55.4% | 56,209,597 / 56,608,401 |
| canon-8 | psd-import | 142.523 | 96.323 | -32.4% | -34.4..-26.9% | 56,209,597 / 56,209,597 |
| canon-16 | save | 185.898 | 183.472 | -1.3% | -2.8..-0.2% | 88,750,003 / 88,749,555 |
| canon-16 | load-common | 193.409 | 193.704 | +0.2% | -0.2..+0.8% | 88,750,003 / 88,750,003 |
| canon-16 | png-encode | 169.432 | 135.364 | -20.1% | -26.2..-15.4% | 58,279,529 / 57,568,207 |
| canon-16 | png-decode | 211.947 | 211.829 | -0.1% | -0.4..+0.5% | 58,279,529 / 58,279,529 |
| canon-16 | psd-export | 1295.076 | 1130.697 | -12.7% | -13.5..-9.9% | 140,673,004 / 141,105,829 |
| canon-32 | save | 340.781 | 334.426 | -1.9% | -5.3..+0.7% | 120,957,984 / 120,957,536 |
| canon-32 | load-common | 774.663 | 778.401 | +0.5% | -1.3..+8.5% | 120,957,984 / 120,957,984 |
| canon-32 | psd-export | 1028.651 | 647.483 | -37.1% | -40.5..-36.1% | 228,234,238 / 227,497,106 |
| canon-32 | psd-import | 177.500 | 146.373 | -17.5% | -18.5..-17.1% | 228,234,238 / 228,234,238 |
| layout-masks | save | 18.588 | 18.268 | -1.7% | -2.7..+0.6% | 433,949 / 433,924 |
| layout-masks | load-common | 16.099 | 16.080 | -0.1% | -1.5..+0.6% | 433,949 / 433,949 |
| layout-masks | png-encode | 3.911 | 2.557 | -34.6% | -36.0..-32.3% | 42,842 / 43,440 |
| layout-masks | png-decode | 0.781 | 0.767 | -1.8% | -2.5..+2.2% | 42,842 / 42,842 |
| layout-masks | psd-export | 56.043 | 46.949 | -16.2% | -18.5..-15.9% | 493,006 / 493,145 |
| layout-masks | psd-import | 12.324 | 12.334 | +0.1% | -4.8..+2.3% | 493,006 / 493,006 |
| layout-mask-index | save | 7.367 | 7.202 | -2.2% | -5.5..-0.7% | 344,892 / 344,746 |
| layout-mask-index | load-common | 6.601 | 6.602 | +0.0% | -2.8..+4.7% | 344,892 / 344,892 |
| layout-mask-index | png-encode | 11.520 | 5.971 | -48.2% | -49.5..-46.0% | 115,229 / 114,892 |
| layout-mask-index | png-decode | 1.451 | 1.444 | -0.5% | -4.2..+0.5% | 115,229 / 115,229 |
| layout-mask-index | psd-export | 19.135 | 12.721 | -33.5% | -34.0..-31.5% | 411,435 / 411,594 |
| layout-mask-index | psd-import | 3.362 | 2.721 | -19.1% | -21.3..-11.2% | 411,435 / 411,435 |

Every PNG encode decodes to identical source pixels, each PSD operation produces the same decoded layer/mask samples as main, and complete .pcraft documents compare equal at every depth. Compressed bytes may differ. The committed compatibility test independently inflates PNG IDAT and PSD ZIP/ZIP-prediction data using old miniz_oxide and checks .pcraft tiles plus thumbnails.

## Allocation and validation changes

ZIP lookup keys borrow valid UTF-8 entry paths, avoiding a second string allocation per entry. Invalid names keep their previous lossy spelling and duplicate-name lookup still selects the first entry. The bundle writer reserves the complete archive after compression, including its central directory. It rejects entry paths that exceed ZIP's 65,535-byte field before writing and checks that the central-directory size fits its ZIP field. These archive paths are independent of filesystem path limits.

Affinity zlib entries, deep EXR ZIP chunks and compressed SVG input now decode directly from their byte slices. This removes flate2's 32 KiB input-buffer allocation and copy for each decoder. Output bounds, checksums and SVG concatenated gzip members remain covered by tests. These are structural allocation reductions, not measured timing gains.

## Linux and Windows x86-64 validation

[GitHub Actions run](https://github.com/reneleonhardt/photocraft/actions/runs/39444647224) passed the deflate compatibility test,
the full IO/codecs/format package tests and `cargo xtask wasm` on Ubuntu 24.04 and Windows 2022 x86-64.
The temporary `ci/deflate-zlib-rs-x86` branch at `46232997` contains the workflow and benchmark setup.
The run's `deflate-x86-results` artifact contains all raw samples, the full table, input and binary hashes, backend features and host details.

The timing host used an Intel Xeon 6973P-C with four vCPUs and rustc 1.99.1.
Main at `d95fdf31` and the backend change at `a37198d4` used matching release settings.
One warmup preceded nine alternating pairs on the same 12/24 MP photos at U8/U16.
Both importers received the same baseline-encoded PSD files. Loading, conversion, hashes and checks stayed outside the elapsed-time measurements.
PNG encode time fell 23.8–56.8%, PSD export time 18.1–58.2%, and PSD import time 7.8–21.6%.
The 16-bit city import fell from 440.733 to 402.057 ms (-8.78%); the macOS slowdown did not recur on this runner.
Photo PNGs became 1.2–2.1% smaller. Exported PSDs grew by 0.014–0.709%.
All decoded pixels, dimensions, depths and profiles matched. These results cover this Linux host; Windows and F32 were not timed.
