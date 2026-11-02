# Text rasterization: what a type render costs

Measured 2026-10-17 on Windows 11, `i5-10300H` (4 cores / 8 threads, 16 GB), release build,
`rustc 1.98.1`, with a probe that renders one paragraph layer through
`photocraft_text::render::rasterize_warped` and times each stage of it.

Document: 300 dpi, a 2400 × 1600 px paragraph box holding 4000 characters at 14 pt — the worst
case a type edit has (a whole page of text at print resolution; 1851 laid-out glyphs, 2329 × 1610 px
of ink = 3.75 MP).

| Stage | ms | What it does |
|---|---:|---|
| ink rectangle (`ink_rect_warped`) | 4.1 | its own pass over every glyph, to size the surface |
| draw (`draw_pass` into `Coverage`) | 18.6–22.9 | glyph outlines, curve flattening (0.03 px tolerance), winding scanlines |
| prefix sum (`Coverage::finish`) | 7.0–8.1 | resolves the winding accumulator to coverage 0…1, one running sum over 3.75 M pixels |
| composite | 22.6–33.4 | each colour's coverage over the premultiplied accumulator |
| unpremultiply | 11.2–14.0 | divides the accumulator back to straight colour |
| `Surface::write_region` | 23.8 | converts to the document format and fills the tiles |
| `Surface::prune` | 3.1 | drops tiles that came back the layer's default |
| **total** | **86–104** | |

So the glyph geometry — what one would expect to dominate — is under a quarter of it. The rest is
scalar work over the pixel buffers, and every one of those passes is per pixel: a pixel's maths
reads and writes only its own slots.

## What runs in parallel now

`composite`, `unpremultiply` and the tile conversion run on all cores (rayon, native only; the web
build keeps the serial loops, since wasm has no threads). The rasterizer writes its tiles with
`Surface::write_region_par`; `Surface::write_region` stays serial, because mask builds call it inside
a `OnceLock` initializer, which must not enter rayon. Nothing about them
is order-dependent, so each pixel still sees the same operations on the same inputs — the pixels
are identical, not merely close.

| | before | after |
|---|---:|---:|
| the 3.75 MP case above | 98.3–103.9 ms | 60.3–61.8 ms |

`crates/text/src/tests.rs::parallel_raster_matches_a_single_thread` renders the shapes the pipeline
can take (plain, faux bold italic, no antialias, wide gamut, a wrapping paragraph box; 8/16/32-bit)
on a one-thread pool and on the default pool and compares the surfaces; `crates/raster`'s existing
`region_writes_match_scalar_bytes_for_every_format` compares `write_region` and `write_region_par`
against a scalar reference implementation, tile for tile.

## What stays on one thread, and why

- **draw** (19–23 ms): the winding accumulator carries across scanlines and rows — a pixel's value
  depends on the rows above it, and one scanline can spill into the next row's first pixel. A band
  split can be exact (bands keep absolute coordinates, carry the spill in band order, and are
  assembled before the prefix sum), but each band re-traverses every glyph — the traversal is not
  free, so the win is bounded by the band count. Worth doing when the draw matters more than it does
  now.
- **prefix sum** (7–8 ms): a running f32 sum over the whole image. Splitting it per row needs the
  row's starting sum, which is that same sequential chain — the only exactly-known starting sums are
  the ones the chain computes.
- **ink rectangle** (4.1 ms): a second full traversal into a bounds sink. Per-glyph bounds reduce by
  min/max, which is associative and exact, so this is a candidate for the same treatment as the
  stages above (a glyph-range split, not a band split).

## Related

Byte-identical output matters here beyond taste: text pixels reach a PSD's saved cache and the
round-trip corpora. `cargo xtask test-corpus` and the Photoshop oracle tests are the gate.
