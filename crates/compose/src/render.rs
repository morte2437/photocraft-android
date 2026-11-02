use super::composite::composite_stack_at;
use super::content::RENDER_TILE;
use super::ctx::{Ctx, RenderVectorMasks, prepare_effects, prepare_tones};
use super::{Buffer, Document, Layer, Rect, Surface, adjust, advanced, cmyk, multichannel, native, native_lab, pattern, psblend};

/// Every layer the compositor draws, groups before their children: hidden layers and everything
/// inside hidden groups are left out.
pub(crate) fn visible_layers(layers: &[Layer]) -> Vec<&Layer> {
    fn rec<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
        for l in layers.iter().filter(|l| l.visible) {
            out.push(l);
            if let Some(children) = l.children() {
                rec(children, out);
            }
        }
    }
    let mut out = Vec::new();
    rec(layers, &mut out);
    out
}

/// Composite the whole document over `rect`, in parallel 256² tiles on
/// native targets (single-threaded on wasm).
pub fn render(doc: &Document, rect: Rect) -> Buffer {
    render_tiled(doc, rect, RENDER_TILE)
}

/// Most pixels one [`render`] call will produce (2^30, a 16 GiB buffer; 32768² px). Larger rects
/// (a hostile canvas size is up to 300000 px a side) return an empty buffer instead of aborting
/// on allocation; callers wanting more render in pieces.
pub const MAX_RENDER_PIXELS: u64 = 1 << 30;

/// [`render`] with an explicit tile size (tests check tile independence).
pub fn render_tiled(doc: &Document, rect: Rect, tile: i32) -> Buffer {
    let patterns = pattern::PreparedPatterns::new(&doc.patterns, pattern::PREPARED_PATTERN_BYTES).in_mode(doc.pixel_format().mode);
    let cx = Ctx::for_doc(doc, &patterns);
    render_tiled_with(doc, rect, tile, &cx)
}

pub(crate) fn render_tiled_with(doc: &Document, rect: Rect, tile: i32, cx: &Ctx) -> Buffer {
    let tile = tile.max(1);
    // Lab documents mix Normal blending in CIELAB, as Photoshop does (psblend::LAB_MIX); 32-bit
    // documents don't clip Add / Divide or Hue / Saturation / Color / Luminosity at 1 (psblend::HDR).
    let lab = doc.mode == photocraft_color::ColorMode::Lab;
    let hdr = doc.depth == photocraft_color::SampleType::F32;
    // CMYK layers are read through the document's own CMYK profile (thread-local scope).
    let cmyk = cmyk_space(doc);
    let cmyk = cmyk.as_ref();
    if rect.is_empty() {
        return Buffer::transparent(rect);
    }
    // Refuse rects too large to hold: an empty buffer, not an allocation failure.
    if u64::from(rect.width()) * u64::from(rect.height()) > MAX_RENDER_PIXELS {
        return Buffer::transparent(Rect::EMPTY);
    }
    // A CMYK stack with adjustment layers the inks carry exactly composites on its inks.
    if let Some(buf) = native::render_cmyk(doc, rect, tile) {
        return buf;
    }
    // A Lab stack with adjustment layers that work on L, a and b composites on its stored values.
    if let Some(buf) = native_lab::render_lab(doc, rect, tile) {
        return buf;
    }
    cx.set_fx_clip(rect);
    if i64::from(rect.width()) <= i64::from(tile) && i64::from(rect.height()) <= i64::from(tile) {
        return photocraft_color::convert::with_cmyk_space(cmyk, || {
            let mut buf = multichannel::backdrop(doc, rect);
            in_blend_space(lab, hdr, || composite_stack_at(&doc.layers, &mut buf, cx, advanced::Scope::ROOT));
            buf
        });
    }
    // Effect maps are built once, here, before any tile needs them (#276).
    prepare_effects(&doc.layers, rect, cx, |f| {
        photocraft_color::convert::with_cmyk_space(cmyk, || in_blend_space(lab, hdr, f));
    });
    let run = |t: Rect| {
        photocraft_color::convert::with_cmyk_space(cmyk, || {
            let mut b = multichannel::backdrop(doc, t);
            in_blend_space(lab, hdr, || composite_stack_at(&doc.layers, &mut b, cx, advanced::Scope::ROOT));
            b
        })
    };
    // Tiles are written straight into the output, one row of tiles (a band) at a time, so the
    // peak is the output plus the tiles in flight, not a second full-size copy.
    let w = rect.width() as usize;
    let mut out = Buffer::transparent(rect);
    let band_tiles = |y0: i32| {
        let y1 = y0.saturating_add(tile).min(rect.y1);
        let mut v = Vec::new();
        let mut x = rect.x0;
        while x < rect.x1 {
            let x1 = x.saturating_add(tile).min(rect.x1);
            v.push(Rect::new(x, y0, x1, y1));
            x = x1;
        }
        v
    };
    let put = |band: &mut [[f32; 4]], y0: i32, part: &Buffer| {
        let pw = part.rect.width() as usize;
        for (row, src) in part.px.chunks_exact(pw).enumerate() {
            let o = ((part.rect.y0 - y0) as usize + row) * w + (part.rect.x0 - rect.x0) as usize;
            band[o..o + pw].copy_from_slice(src);
        }
    };
    let band_len = w * tile as usize;
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        let bands = rect.height().div_ceil(tile as u32) as usize;
        if bands >= 2 * rayon::current_num_threads() {
            // Enough bands to keep every core busy: one band per task, its tiles in turn.
            out.px.par_chunks_mut(band_len).enumerate().for_each(|(i, band)| {
                let y0 = rect.y0 + i as i32 * tile;
                for t in band_tiles(y0) {
                    put(band, y0, &run(t));
                }
            });
        } else {
            // Few, wide bands: the tiles of each band in parallel, and the bands too, so a band's
            // slowest tile doesn't leave the other cores idle until the next band starts. Each
            // tile is written as soon as it is done, so only the tiles in flight are held.
            out.px.par_chunks_mut(band_len).enumerate().for_each(|(i, band)| {
                let y0 = rect.y0 + i as i32 * tile;
                let band = std::sync::Mutex::new(band);
                band_tiles(y0).into_par_iter().for_each(|t| {
                    let part = run(t);
                    let mut band = band.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    put(&mut band, y0, &part);
                });
            });
        }
    }
    #[cfg(target_arch = "wasm32")]
    for (i, band) in out.px.chunks_mut(band_len).enumerate() {
        let y0 = rect.y0 + i as i32 * tile;
        for t in band_tiles(y0) {
            put(band, y0, &run(t));
        }
    }
    out
}

/// Runs `f` with this thread's document blend flags set ([`psblend::LAB_MIX`], [`psblend::HDR`]),
/// then restores the values they had (also on unwind): a nested rayon job can run another tile
/// on this thread mid-composite, and clearing the flags there broke the outer tile (#1112).
pub(crate) fn in_blend_space<R>(lab: bool, hdr: bool, f: impl FnOnce() -> R) -> R {
    psblend::with_lab_mix(lab, || psblend::with_hdr(hdr, f))
}

/// The document's own CMYK profile for reading its CMYK pixels (`None`: not a CMYK document,
/// untagged, or the built-in coated CMYK). Enter it with `photocraft_color::convert::with_cmyk_space`
/// around code that converts the document's CMYK pixels or colours to RGB.
pub fn cmyk_space(doc: &Document) -> Option<std::sync::Arc<photocraft_color::convert::CmykSpace>> {
    if doc.mode != photocraft_color::ColorMode::Cmyk {
        return None;
    }
    photocraft_color::convert::CmykSpace::for_profile(doc.icc_profile.as_ref())
}

/// Pixels per band of [`render_bands`] (a 14000 px wide band is ~600 rows, ~130 MB of f32).
pub const BAND_PIXELS: u64 = 8 << 20;

/// Composite `rect` in horizontal bands, top to bottom, handing each band to `sink` (tiles inside
/// a band render in parallel). Peak memory is one band instead of the whole composite, so huge
/// documents can be exported or displayed without a full-size float copy. `band_rows` of 0 picks
/// about [`BAND_PIXELS`] per band; bands are whole multiples of [`RENDER_TILE`] rows.
pub fn render_bands<E>(doc: &Document, rect: Rect, band_rows: i32, mut sink: impl FnMut(Buffer) -> Result<(), E>) -> Result<(), E> {
    if rect.is_empty() {
        return Ok(());
    }
    // Keep prepared pixels and compiled vector masks across bands without retaining any rendered band.
    let patterns = pattern::PreparedPatterns::new(&doc.patterns, pattern::PREPARED_PATTERN_BYTES).in_mode(doc.pixel_format().mode);
    let cx = Ctx::for_doc(doc, &patterns);
    let rows = band_rows_for(rect.width(), band_rows);
    let mut y = rect.y0;
    while y < rect.y1 {
        let y1 = y.saturating_add(rows).min(rect.y1);
        sink(render_tiled_with(doc, Rect::new(rect.x0, y, rect.x1, y1), RENDER_TILE, &cx))?;
        y = y1;
    }
    Ok(())
}

/// Band height for [`render_bands`]: `requested` rounded up to whole tiles, or ~[`BAND_PIXELS`].
pub fn band_rows_for(width: u32, requested: i32) -> i32 {
    let want = if requested > 0 { i64::from(requested) } else { (BAND_PIXELS / u64::from(width.max(1))) as i64 };
    let tiles = (want + i64::from(RENDER_TILE) - 1) / i64::from(RENDER_TILE);
    (tiles.clamp(1, 1 << 16) * i64::from(RENDER_TILE)) as i32
}

/// The document's composite as a pixel surface in `fmt` (straight RGBA converted to its model
/// and depth), optionally flattened over an opaque `background`; rendered and written in bands,
/// so no full-size float composite is held. Tiles equal to the default pixel are pruned.
///
/// Fully transparent pixels are written as the default pixel. Converting transparent black
/// leaves colour behind in CMYK (K = 100%) and Lab (neutral a/b = 0.5), and those pixels would
/// otherwise count as content: the result would span the whole canvas.
///
/// A CMYK document flattened to CMYK is composited in its ink values instead, so flattening
/// doesn't round-trip its pixels through RGB: plain Normal layers by [`native::flatten_cmyk`]
/// (#3778), and masks, groups, clipping, other blend modes and the other layer kinds by [`cmyk`]
/// where its layers mix on their inks (#2220). Otherwise the composite goes back to CMYK through
/// the document's own profile, the one its layers were read through. A Lab document of plain
/// layers is composited in its L, a and b ([`native_lab::flatten_lab`]).
pub fn flatten_to_surface(doc: &Document, fmt: photocraft_color::PixelFormat, background: Option<[f32; 3]>) -> Surface {
    composite_to_surface(doc, doc.bounds(), fmt, background)
}

/// [`flatten_to_surface`] over `rect` only (the rest of the surface stays empty).
pub fn composite_to_surface(doc: &Document, rect: Rect, fmt: photocraft_color::PixelFormat, background: Option<[f32; 3]>) -> Surface {
    let paper = background.is_some_and(|bg| bg == [1.0; 3]);
    if (background.is_none() || paper)
        && let Some(s) = native::flatten_cmyk(doc, rect, fmt, paper)
    {
        return s;
    }
    if (background.is_none() || paper)
        && let Some(s) = native_lab::flatten_lab(doc, rect, fmt, paper)
    {
        return s;
    }
    let space = cmyk_space(doc);
    let to_cmyk = fmt.mode == photocraft_color::ColorMode::Cmyk && doc.mode == photocraft_color::ColorMode::Cmyk;
    if to_cmyk && cmyk::renders_inks(doc) {
        return composite_inks_to_surface(doc, rect, fmt, background, space.as_ref());
    }
    // The composite separates back to CMYK through the document's own profile, the one its
    // pixels were read through.
    let mut s = Surface::new(fmt);
    let n = fmt.channels();
    let empty = s.default_pixel();
    let mut data = Vec::new();
    let _ = render_bands(doc, rect, 0, |band| -> Result<(), ()> {
        let band = match background {
            Some(bg) => band.over_background(bg),
            None => band,
        };
        data.clear();
        data.resize(band.px.len() * n, 0.0);
        photocraft_color::convert::with_cmyk_space(space.as_ref(), || {
            for (p, out) in band.px.iter().zip(data.chunks_exact_mut(n)) {
                if fmt.alpha && (p[3] <= 0.0 || p[3].is_nan()) && empty.len() == n {
                    out.copy_from_slice(&empty);
                } else {
                    photocraft_raster::from_rgba_into(&fmt, *p, out);
                }
            }
        });
        s.write_region(band.rect, &data);
        Ok(())
    });
    s.prune();
    s
}

/// One CMYK pixel's output channels into `data`: its inks (over `paper` when given) and alpha.
fn push_ink_pixel(data: &mut Vec<f32>, p: &[f32; 5], paper: Option<[f32; 4]>, fmt: photocraft_color::PixelFormat, empty: &[f32]) {
    if paper.is_none() && fmt.alpha && (p[4] <= 0.0 || p[4].is_nan()) && empty.len() == fmt.channels() {
        data.extend_from_slice(empty);
        return;
    }
    // Over the paper (fully covered pixels keep their inks exactly), or straight.
    let ink = |c: usize| match paper {
        Some(bg) if p[4] < 1.0 => p[c] * p[4] + bg[c] * (1.0 - p[4]),
        _ => p[c],
    };
    data.extend((0..4).map(ink));
    if fmt.alpha {
        data.push(if paper.is_some() { 1.0 } else { p[4] });
    }
}

/// [`composite_to_surface`] for a CMYK document flattened to CMYK, composited in its inks.
fn composite_inks_to_surface(
    doc: &Document,
    rect: Rect,
    fmt: photocraft_color::PixelFormat,
    background: Option<[f32; 3]>,
    space: Option<&std::sync::Arc<photocraft_color::convert::CmykSpace>>,
) -> Surface {
    let mut s = Surface::new(fmt);
    let n = fmt.channels();
    let empty = s.default_pixel();
    let mut data = Vec::new();
    // White paper is no ink; another background colour goes through the profile.
    let paper = background
        .map(|bg| if bg == [1.0; 3] { [0.0; 4] } else { photocraft_color::convert::with_cmyk_space(space, || photocraft_color::convert::rgb_to_cmyk(bg)) });
    let _ = cmyk::render_cmyk_bands(doc, rect, 0, |band| -> Result<(), ()> {
        data.clear();
        data.reserve(band.px.len() * n);
        for p in &band.px {
            push_ink_pixel(&mut data, p, paper, fmt, &empty);
        }
        s.write_region(band.rect, &data);
        Ok(())
    });
    s.prune();
    s
}

/// Composite the full canvas.
pub fn flatten(doc: &Document) -> Buffer {
    render(doc, doc.bounds())
}

/// Render an arbitrary subset: a single layer (e.g. for thumbnails), isolated.
/// Rects past [`MAX_RENDER_PIXELS`] give an empty buffer.
pub fn render_layer(layer: &Layer, rect: Rect) -> Buffer {
    if u64::from(rect.width()) * u64::from(rect.height()) > MAX_RENDER_PIXELS {
        return Buffer::transparent(Rect::EMPTY);
    }
    let mut buf = Buffer::transparent(rect);
    let patterns = pattern::PreparedPatterns::new(&[], pattern::PREPARED_PATTERN_BYTES);
    composite_stack_at(
        std::slice::from_ref(layer),
        &mut buf,
        &Ctx {
            canvas: rect,
            transfer: adjust::Transfer::Srgb,
            light: photocraft_doc::GlobalLight::default(),
            patterns: &patterns,
            mode: photocraft_color::ColorMode::Rgb,
            depth: photocraft_color::SampleType::F32,
            vector_masks: RenderVectorMasks::default(),
            fx_maps: Default::default(),
            fx_clip: Default::default(),
            fx_over_budget: false,
            tone_luts: prepare_tones(std::slice::from_ref(layer), 0),
            doc: None,
            layer_keys: Default::default(),
        },
        advanced::Scope::ROOT,
    );
    buf
}

/// [`render_layer`] in `doc`'s context: the layer alone over `rect`, with the document's patterns,
/// Global Light, colour mode, depth and canvas (fill layers and gradients are laid out on it), as
/// the canvas composites it. Bakes (Scripts › Flatten All Layer Effects) render through this so
/// the pixels match what was on screen (#5719). Rects past [`MAX_RENDER_PIXELS`] give an empty
/// buffer.
pub fn render_layer_in(doc: &Document, layer: &Layer, rect: Rect) -> Buffer {
    if rect.is_empty() {
        return Buffer::transparent(rect);
    }
    if u64::from(rect.width()) * u64::from(rect.height()) > MAX_RENDER_PIXELS {
        return Buffer::transparent(Rect::EMPTY);
    }
    let patterns = pattern::PreparedPatterns::new(&doc.patterns, pattern::PREPARED_PATTERN_BYTES).in_mode(doc.pixel_format().mode);
    // A lone layer: a deep knockout reveals transparency, not the document's Background.
    let cx = Ctx { doc: None, ..Ctx::for_doc(doc, &patterns) };
    let lab = doc.mode == photocraft_color::ColorMode::Lab;
    let hdr = doc.depth == photocraft_color::SampleType::F32;
    let cmyk = cmyk_space(doc);
    photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
        let mut buf = Buffer::transparent(rect);
        in_blend_space(lab, hdr, || composite_stack_at(std::slice::from_ref(layer), &mut buf, &cx, advanced::Scope::ROOT));
        buf
    })
}
