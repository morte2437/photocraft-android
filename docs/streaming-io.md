# EXR streaming

Native path opens (`photocraft_io::import_file_for_open_with`, used by the background open job
and by automation file opens) decode single-part flat scanline EXR files one compression block
at a time straight into guarded raster tiles. The document keeps the source dimensions and
samples; HALF samples are stored as F32, with a warning. Tiles count against the RAM tile
budget, and scratch residency can move cold tiles to disk; without a scratch disk an image whose
tiles cannot fit the budget fails with a clear out-of-memory error before any pixels are
decoded. Layered PSD/PSB files use the existing streaming PSD reader on the same route.

The block decoder (`photocraft_codecs::exr_stream`) treats the file as untrusted. Header metadata
is bounded (8 MiB, 4096 attributes); dimensions, channel count, chunk count, offsets, block
lengths and sample counts are checked against the file length and the header before anything is
allocated, and one decoded block is capped at 64 MiB. Duplicate, misaligned or truncated blocks
fail the open. It picks the same colour channels as the whole-file decoder.

Multi-part, deep, tiled, subsampled and custom-chromaticity files, and headers the block decoder
refuses, fall back to the whole-file decoder, under its own limits and working-memory admission.
Web and byte opens always use the whole-file decoder.

Cancellation is checked between reads and between scanlines, and progress reports completed
lines. A cancelled or failed open publishes no partial document. The open also fails if the
file's size or modification time changed while it was being read.

Tests compare the streamed document with the whole-file decoder across Gray/GrayA/RGB/RGBA, HALF
and FLOAT, and None/RLE/ZIP/ZIPS/PIZ compression, and cover cancellation, truncation, forged
offsets and lengths, hostile metadata and oversized headers.
