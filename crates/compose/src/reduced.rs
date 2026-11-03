use super::content::RENDER_TILE;
use super::ctx::Ctx;
use super::render::{band_rows_for, render, render_bands, render_tiled_with};
use super::{Buffer, Document, Rect, Rgba8Image, pattern, proxy, uniform};

/// Documents above this many pixels get thumbnails from a proxy (see [`proxy`]) when that is
/// faithful; smaller ones are reduced from the exact composite.
pub const PROXY_THUMBNAIL_PIXELS: u64 = 16 << 20;

/// Downscaled RGBA8 render of the document (area-averaged) for thumbnails, navigators and
/// histograms, at most `max_side` pixels on its longer side. Large documents are reduced from a
/// downsampled proxy (about twice the thumbnail's size), so a thumbnail of a 200 MP document
/// costs milliseconds and no full-size composite; the rest stream the exact composite in bands.
pub fn thumbnail(doc: &Document, max_side: u32) -> Rgba8Image {
    thumbnail_buffer(doc, max_side).to_rgba8()
}

/// Width and height of [`thumbnail`]`(doc, max_side)`.
pub fn thumbnail_size(doc: &Document, max_side: u32) -> (u32, u32) {
    let b = doc.bounds();
    let longest = b.width().max(b.height()).max(1);
    let scale = (max_side as f32 / longest as f32).min(1.0);
    (((b.width() as f32 * scale).round() as u32).max(1), ((b.height() as f32 * scale).round() as u32).max(1))
}

/// [`thumbnail`] as straight-alpha floats (for callers that colour-convert it first).
pub fn thumbnail_buffer(doc: &Document, max_side: u32) -> Buffer {
    let b = doc.bounds();
    let longest = b.width().max(b.height()).max(1);
    let (w, h) = thumbnail_size(doc, max_side);
    let k = longest / max_side.max(1).saturating_mul(2);
    if k >= 2 && doc.size.area() > PROXY_THUMBNAIL_PIXELS && proxy::proxy_faithful(doc) {
        render_reduced(&proxy::proxy_document(doc, k), w, h)
    } else {
        render_reduced(doc, w, h)
    }
}

/// The document's composite area-averaged (premultiplied) down to `w`×`h` (clamped to the
/// document size), rendered in bands so no full-size composite is held.
pub fn render_reduced(doc: &Document, w: u32, h: u32) -> Buffer {
    render_reduced_in_bands(doc, doc.bounds(), w, h, None, 0)
}

/// [`render_reduced`] of any area of the document plane, `rect`, which may lie past the canvas
/// (layer pixels there are kept until a crop deletes them). The buffer's rect is in output pixels
/// from (0, 0).
pub fn render_reduced_rect(doc: &Document, rect: Rect, w: u32, h: u32) -> Buffer {
    render_reduced_in_bands(doc, rect, w, h, None, 0)
}

/// The part of [`render_reduced`]`(doc, w, h)` that a change to `damage` (document pixels) can
/// affect: the output pixels whose source areas meet it, rendered from just those areas, with the
/// same values the whole reduction gives them. The buffer's rect is in output pixels (empty when
/// `damage` misses the document), so a reduced canvas texture can update only what a stroke touched.
pub fn render_reduced_damage(doc: &Document, w: u32, h: u32, damage: Rect) -> Buffer {
    render_reduced_in_bands(doc, doc.bounds(), w, h, Some(damage), 0)
}

/// [`render_reduced_rect`] (or, with `damage`, [`render_reduced_damage`]) of area `b` with an
/// explicit band height (see [`render_bands`]).
pub(crate) fn render_reduced_in_bands(doc: &Document, b: Rect, w: u32, h: u32, damage: Option<Rect>, band_rows: i32) -> Buffer {
    render_reduced_with(doc, b, w, h, damage, band_rows, true, &mut ReducedStats::default())
}

/// What a reduced render composited (tests check that one-colour areas are not composited).
#[derive(Debug, Default)]
pub(crate) struct ReducedStats {
    /// Whether one-colour areas were looked for ([`uniform::UniformAreas`]).
    pub(crate) uniform: bool,
    /// Document pixels composited one by one.
    pub(crate) composited: u64,
}

/// A tile of a reduced render: one colour (premultiplied), or its composited pixels.
enum Part {
    Solid([f32; 4]),
    Px(Buffer),
}

/// A column of document tiles in a reduced render (`b`-relative source columns).
struct TileColumn {
    /// The columns rendered: the tile's, within the source area.
    x0: usize,
    x1: usize,
    /// The whole tile's columns within `b`, whose values decide whether it is one colour.
    whole: (i64, i64),
    /// Runs of (output column, how many of the rendered columns it averages).
    runs: Vec<(usize, f32)>,
}

/// [`render_reduced_in_bands`]; `uniform` off composites every pixel (the reference in tests).
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_reduced_with(
    doc: &Document,
    b: Rect,
    w: u32,
    h: u32,
    damage: Option<Rect>,
    band_rows: i32,
    uniform: bool,
    stats: &mut ReducedStats,
) -> Buffer {
    let (fw, fh) = (b.width() as usize, b.height() as usize);
    let (w, h) = (w.clamp(1, b.width().max(1)) as usize, h.clamp(1, b.height().max(1)) as usize);
    let full = Rect::from_xywh(0, 0, w as u32, h as u32);
    if fw == 0 || fh == 0 {
        return Buffer::transparent(if damage.is_some() { Rect::EMPTY } else { full });
    }
    // Output column / row of each document column / row: output pixel t covers [t·f/n, (t+1)·f/n).
    let span = |t: usize, f: usize, n: usize| (t * f / n, ((t + 1) * f / n).max(t * f / n + 1).min(f));
    // The output pixel whose span holds document column / row `x` (spans are contiguous: n <= f).
    let cell = |x: usize, f: usize, n: usize| ((x + 1) * n).saturating_sub(1) / f;
    // Output pixels to produce and the document area they cover.
    let out = match damage {
        None => full,
        Some(d) => {
            let d = d.intersect(&b);
            if d.is_empty() {
                return Buffer::transparent(Rect::EMPTY);
            }
            let (x0, x1) = ((d.x0 - b.x0) as usize, (d.x1 - b.x0) as usize);
            let (y0, y1) = ((d.y0 - b.y0) as usize, (d.y1 - b.y0) as usize);
            Rect::new(cell(x0, fw, w) as i32, cell(y0, fh, h) as i32, cell(x1 - 1, fw, w) as i32 + 1, cell(y1 - 1, fh, h) as i32 + 1)
        }
    };
    let (ox0, oy0, ow, oh) = (out.x0 as usize, out.y0 as usize, out.width() as usize, out.height() as usize);
    let (sx0, sx1) = (span(ox0, fw, w).0, span(ox0 + ow - 1, fw, w).1);
    let (sy0, sy1) = (span(oy0, fh, h).0, span(oy0 + oh - 1, fh, h).1);
    let src = Rect::new(b.x0 + sx0 as i32, b.y0 + sy0 as i32, b.x0 + sx1 as i32, b.y0 + sy1 as i32);
    if (w, h) == (fw, fh) {
        let mut px = render(doc, src);
        px.rect = out;
        return px;
    }
    let map = |f: usize, n: usize| {
        let mut m = vec![0u32; f];
        for t in 0..n {
            let (a, z) = span(t, f, n);
            m[a..z].fill(t as u32);
        }
        m
    };
    let (cols, rows) = (map(fw, w), map(fh, h));
    let cols = &cols[sx0..sx1];
    let mut acc = vec![[0.0f32; 4]; ow * oh];
    // Each output row sums its source rows in a band (in order, top to bottom), each row's pixels
    // left to right: `row(y, out)` adds source row `y` (band-relative) to output row `out`.
    let sum_band = |acc: &mut [[f32; 4]], by0: usize, by1: usize, row: &(dyn Fn(usize, &mut [[f32; 4]]) + Sync)| {
        let (t0, t1) = (rows[by0] as usize, rows[by1 - 1] as usize + 1);
        let sum_row = |t: usize, out: &mut [[f32; 4]]| {
            let (a, z) = span(t, fh, h);
            for y in a.max(by0)..z.min(by1) {
                row(y - by0, out);
            }
        };
        let part = &mut acc[(t0 - oy0) * ow..(t1 - oy0) * ow];
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            part.par_chunks_mut(ow).enumerate().for_each(|(i, out)| sum_row(t0 + i, out));
        }
        #[cfg(target_arch = "wasm32")]
        for (i, out) in part.chunks_mut(ow).enumerate() {
            sum_row(t0 + i, out);
        }
    };
    let add = |o: &mut [f32; 4], p: &[f32; 4]| {
        for i in 0..3 {
            o[i] += p[i] * p[3];
        }
        o[3] += p[3];
    };
    if let Some(uniform) = uniform.then(|| uniform::UniformAreas::new(doc)).flatten() {
        stats.uniform = true;
        // One-colour tiles (a new document's background, the empty parts of a sparse one) are
        // composited once per colour and summed per run of source columns, not per pixel
        // (#4017). Tiles lie on the document's tile grid whatever `damage` is, so a damage
        // render sums exactly what the whole reduction does.
        let patterns = pattern::PreparedPatterns::new(&doc.patterns, pattern::PREPARED_PATTERN_BYTES).in_mode(doc.pixel_format().mode);
        let cx = Ctx::for_doc(doc, &patterns);
        let t = RENDER_TILE as usize;
        // The document tile grid (the surfaces' own), in `b`-relative coordinates: the start of
        // the tile holding `v` (negative when `b` starts inside a tile).
        let grid = |v: usize, o: i32| (i64::from(o) + v as i64).div_euclid(t as i64) * t as i64 - i64::from(o);
        let tile_cols: Vec<TileColumn> = (grid(sx0, b.x0)..sx1 as i64)
            .step_by(t)
            .map(|gx| {
                let (x0, x1) = ((gx.max(0) as usize).max(sx0), ((gx + t as i64) as usize).min(sx1));
                let mut runs: Vec<(usize, f32)> = Vec::new();
                for &c in &cols[x0 - sx0..x1 - sx0] {
                    match runs.last_mut() {
                        Some((last, n)) if *last == c as usize - ox0 => *n += 1.0,
                        _ => runs.push((c as usize - ox0, 1.0)),
                    }
                }
                TileColumn { x0, x1, whole: (gx.max(0), (gx + t as i64).min(fw as i64)), runs }
            })
            .collect();
        let band_h = band_rows_for(src.width(), band_rows).max(RENDER_TILE) as usize;
        let mut y = grid(sy0, b.y0);
        while y < sy1 as i64 {
            let y1 = (y + band_h as i64).min(sy1 as i64);
            let tiles: Vec<(Rect, Rect)> = (y..y1)
                .step_by(t)
                .flat_map(|gy| {
                    tile_cols.iter().map(move |&TileColumn { x0, x1, whole: (gx0, gx1), .. }| {
                        let at = |x0: i64, y0: i64, x1: i64, y1: i64| Rect::new(b.x0 + x0 as i32, b.y0 + y0 as i32, b.x0 + x1 as i32, b.y0 + y1 as i32);
                        let (gy0, gy1) = (gy.max(0), (gy + t as i64).min(fh as i64));
                        (at(gx0, gy0, gx1, gy1), at(x0 as i64, gy0.max(sy0 as i64), x1 as i64, gy1.min(sy1 as i64)))
                    })
                })
                .collect();
            let one = |(whole, part): &(Rect, Rect)| -> Part {
                let render = |r: Rect| render_tiled_with(doc, r, RENDER_TILE, &cx);
                match uniform.color(*whole, |r| render(r).px.first().copied()) {
                    Some(p) => Part::Solid([p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]]),
                    None => Part::Px(render(*part)),
                }
            };
            #[cfg(not(target_arch = "wasm32"))]
            let parts: Vec<Part> = {
                use rayon::prelude::*;
                tiles.par_iter().map(one).collect()
            };
            #[cfg(target_arch = "wasm32")]
            let parts: Vec<Part> = tiles.iter().map(one).collect();
            stats.composited += parts.iter().map(|p| if let Part::Px(b) = p { b.px.len() as u64 } else { 0 }).sum::<u64>();
            let (by0, by1) = (y.max(sy0 as i64) as usize, y1 as usize);
            let n = tile_cols.len();
            let band_y = y;
            sum_band(&mut acc, by0, by1, &|row: usize, out: &mut [[f32; 4]]| {
                let y = (by0 + row) as i64;
                let j = (y - band_y) as usize / t;
                let ty = (y - (band_y + (j * t) as i64).max(sy0 as i64)) as usize;
                for (TileColumn { x0, x1, runs, .. }, part) in tile_cols.iter().zip(parts.iter().skip(j * n).take(n)) {
                    match part {
                        Part::Solid(pa) => {
                            for &(c, k) in runs {
                                let o = &mut out[c];
                                for i in 0..4 {
                                    o[i] += pa[i] * k;
                                }
                            }
                        }
                        Part::Px(buf) => {
                            let tw = x1 - x0;
                            for (p, &c) in buf.px.iter().skip(ty * tw).take(tw).zip(&cols[x0 - sx0..x1 - sx0]) {
                                add(&mut out[c as usize - ox0], p);
                            }
                        }
                    }
                }
            });
            y = y1;
        }
    } else {
        let ok: Result<(), ()> = render_bands(doc, src, band_rows, |band| {
            stats.composited += band.px.len() as u64;
            let (by0, by1) = ((band.rect.y0 - b.y0) as usize, (band.rect.y1 - b.y0) as usize);
            let sw = sx1 - sx0;
            sum_band(&mut acc, by0, by1, &|y: usize, out: &mut [[f32; 4]]| {
                for (p, &c) in band.px[y * sw..(y + 1) * sw].iter().zip(cols) {
                    add(&mut out[c as usize - ox0], p);
                }
            });
            Ok(())
        });
        debug_assert!(ok.is_ok());
    }
    let col_w: Vec<usize> = (0..ow)
        .map(|x| {
            let (x0, x1) = span(ox0 + x, fw, w);
            (x1 - x0).max(1)
        })
        .collect();
    let norm_row = |row: usize, out: &mut [[f32; 4]]| {
        let (y0, y1) = span(oy0 + row, fh, h);
        let rh = (y1 - y0).max(1);
        for (p, &cw) in out.iter_mut().zip(&col_w) {
            let a = p[3];
            if a > 0.0 {
                let n = (rh * cw) as f32;
                *p = [p[0] / a, p[1] / a, p[2] / a, a / n];
            } else {
                *p = [0.0; 4];
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        acc.par_chunks_mut(ow).enumerate().for_each(|(row, out)| norm_row(row, out));
    }
    #[cfg(target_arch = "wasm32")]
    for (row, out) in acc.chunks_mut(ow).enumerate() {
        norm_row(row, out);
    }
    Buffer { rect: out, px: acc }
}
