//! Edit › Fill… (`edit.fill`): Photoshop's Fill dialog as one command.
//!
//! Contents: Foreground / Background / Color / Content-Aware / Pattern / History / Black /
//! 50% Gray / White, blended with `mode` at `opacity`, optionally preserving the target's
//! transparency. The selection (or the whole canvas) is filled, as one history step ("Fill").
//! Every parameter is validated: a bad one is an error, never a panic.

use photocraft_color::BlendMode;
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Surface, from_rgba_into, to_rgba};
use serde_json::{Value, json};

use crate::commands::{blend_from_str, color_param, layer_param};
use crate::{EngineError, Result, Session};

const CMD: &str = "edit.fill";

/// Every `contents` value, in the dialog's order.
pub const CONTENTS: [&str; 9] = ["foreground", "background", "color", "contentAware", "pattern", "history", "black", "gray", "white"];

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// What a pixel of the filled area gets before blending.
enum Source {
    Color([f32; 4]),
    /// An opaque colour that was picked as CMYK inks: `rgba` is its sRGB stand-in (what other
    /// colour models and blend modes use), `cmyk` the inks a CMYK target gets as they are.
    Cmyk {
        rgba: [f32; 4],
        cmyk: [f32; 4],
    },
    /// Straight RGBA per pixel of `rect` (row-major).
    Pixels(Rect, Vec<[f32; 4]>),
}

impl Source {
    fn at(&self, x: i32, y: i32) -> [f32; 4] {
        match self {
            Source::Color(c) => *c,
            Source::Cmyk { rgba, .. } => *rgba,
            Source::Pixels(r, px) => {
                if !r.contains(x, y) {
                    return [0.0; 4];
                }
                let i = (y - r.y0) as usize * r.width() as usize + (x - r.x0) as usize;
                px.get(i).copied().unwrap_or([0.0; 4])
            }
        }
    }
}

/// The `contents` param; without one, `color` (the old `{"color": …}` form, foreground by default).
fn contents(p: &Value) -> Result<&str> {
    match p.get("contents") {
        None | Some(Value::Null) => Ok("color"),
        Some(Value::String(c)) => {
            CONTENTS.iter().find(|k| k.eq_ignore_ascii_case(c)).copied().ok_or_else(|| bad(format!("unknown contents `{c}` ({})", CONTENTS.join("|"))))
        }
        Some(v) => Err(bad(format!("`contents` must be a string, not {v}"))),
    }
}

fn mode(p: &Value) -> Result<BlendMode> {
    match p.get("mode") {
        None | Some(Value::Null) => Ok(BlendMode::Normal),
        Some(Value::String(m)) => blend_from_str(m).filter(|m| *m != BlendMode::PassThrough).ok_or_else(|| bad(format!("unknown blend mode `{m}`"))),
        Some(v) => Err(bad(format!("`mode` must be a blend mode name, not {v}"))),
    }
}

/// Opacity in percent (0–100) → 0..1.
fn opacity(p: &Value) -> Result<f32> {
    match p.get("opacity") {
        None | Some(Value::Null) => Ok(1.0),
        Some(v) => {
            let o = v.as_f64().filter(|o| o.is_finite()).ok_or_else(|| bad(format!("`opacity` must be a number 0..100, not {v}")))?;
            Ok((o.clamp(0.0, 100.0) / 100.0) as f32)
        }
    }
}

fn flag(p: &Value, k: &str, d: bool) -> Result<bool> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(d),
        Some(Value::Bool(b)) => Ok(*b),
        Some(v) => Err(bad(format!("`{k}` must be true or false, not {v}"))),
    }
}

/// Edit › Fill.
pub fn fill(s: &mut Session, p: &Value) -> Result<Value> {
    let contents = contents(p)?;
    let mode = mode(p)?;
    let opacity = opacity(p)?;
    let preserve = flag(p, "preserveTransparency", false)?;
    let channel = crate::channel_cmds::is_channel_target(p);
    let id = if channel { None } else { Some(layer_param(s, p)?) };
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc_bounds = st.doc.bounds();
    let sel = st.doc.selection.clone();
    let area = sel.as_ref().map_or(doc_bounds, |m| m.content_bounds().intersect(&doc_bounds));
    let mut out = json!({ "contents": contents });
    let restore = contents == "history";
    let source = match contents {
        "foreground" => color_source(s, s.tools.foreground),
        "background" => color_source(s, s.tools.background),
        "color" => color_source(s, color_param(CMD, p, "color", s.tools.foreground)?),
        "black" => Source::Color([0.0, 0.0, 0.0, 1.0]),
        "gray" => Source::Color([128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]),
        "white" => Source::Color([1.0; 4]),
        "pattern" => {
            let pat = crate::pattern_cmds::resolve_param(s, CMD, p)?;
            let (scale, angle, _, phase) = crate::pattern_cmds::placement(CMD, p)?;
            let tile = photocraft_compose::pattern::Tile::new(&pat).ok_or_else(|| bad("the pattern is empty or too large to paint"))?;
            // Photoshop's Fill uses the canvas origin.
            let place = photocraft_compose::pattern::Placement::new(Rect::EMPTY, false, phase, scale, angle);
            out["pattern"] = json!(pat.id);
            Source::Pixels(area, photocraft_compose::pattern::render(&tile, &place, area))
        }
        "history" => {
            // A History panel row `state` or a `snapshot`, else the History Brush's source.
            let source_doc = crate::history_source_cmds::paint_source(st, p, CMD)?;
            let src = crate::channel_cmds::read_target_surface(source_doc.as_ref(), id, p)
                .map_err(|_| EngineError::Other("the target did not exist (as pixels, a mask or a channel) in that history state".into()))?;
            Source::Pixels(area, read_rgba(src, area, "reading Fill's history source")?)
        }
        "contentAware" => {
            let sel = sel.as_ref().ok_or_else(|| EngineError::Other("Content-Aware fill needs a selection".into()))?;
            if channel || crate::commands::is_mask_target(p) {
                return Err(EngineError::Other("Content-Aware fill works on layer pixels only".into()));
            }
            let adapt = flag(p, "colorAdaptation", true)?;
            let lid = id.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            let surf = st.doc.layer(lid).and_then(|l| l.surface()).ok_or_else(|| EngineError::Other("Content-Aware fill needs a pixel layer".into()))?;
            let seed = p.get("seed").and_then(Value::as_u64).unwrap_or(1);
            content_aware(surf, sel, doc_bounds, adapt, seed)?
        }
        _ => return Err(bad(format!("unknown contents `{contents}`"))),
    };
    if area.is_empty() {
        return Ok(out);
    }
    // The document's own CMYK profile converts the fill colour, like the compositor and brush.
    let cmyk = crate::brush_cmds::doc_cmyk_space(s);
    photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
        s.edit("Fill", |doc, _| {
            let sel = doc.selection.clone();
            let (surf, lock) = crate::channel_cmds::target_surface(doc, id, p)?;
            blend_into(surf, area, &source, sel.as_ref(), Blend { mode, opacity, keep_alpha: preserve || lock, restore })?;
            Ok(())
        })
    })?;
    Ok(out)
}

/// A fill colour: the inks it was picked as when it was picked as CMYK (#4196), else plain sRGB.
fn color_source(s: &Session, c: [f32; 4]) -> Source {
    match s.tools.picked_cmyk([c[0], c[1], c[2]]) {
        Some(cmyk) if c[3] >= 1.0 => Source::Cmyk { rgba: c, cmyk },
        _ => Source::Color(c),
    }
}

/// `source`'s opaque colour as samples of `fmt` (alpha last, as in [`from_rgba_into`]), or `None`
/// for a source that is not one colour. A CMYK pick keeps its inks in a CMYK target.
fn encode_color(fmt: &photocraft_color::PixelFormat, source: &Source, enc: &mut [f32; 8]) -> Option<usize> {
    let (rgba, cmyk) = match source {
        Source::Color(c) if c[3] >= 1.0 => (*c, None),
        Source::Cmyk { rgba, cmyk } => (*rgba, Some(*cmyk)),
        _ => return None,
    };
    let n = from_rgba_into(fmt, rgba, enc);
    if let Some(k) = cmyk
        && fmt.mode == photocraft_color::ColorMode::Cmyk
    {
        enc[..4].copy_from_slice(&k);
    }
    Some(n)
}

fn read_rgba(src: &Surface, area: Rect, purpose: &str) -> Result<Vec<[f32; 4]>> {
    let len = (area.width() as usize)
        .checked_mul(area.height() as usize)
        .ok_or_else(|| EngineError::Other(format!("not enough memory for {purpose} (requested size overflowed); the document was not changed")))?;
    let mut px = crate::allocation::filled(len, [0.0f32; 4], purpose)?;
    src.read_rgba_into(area, &mut px);
    Ok(px)
}

/// `src` over `surf` across `area` (Normal, 100 %), each pixel weighted by `limit` when given:
/// a paste into a layer mask or channel, which keeps the pasted pixels' luminosity (#1035).
pub(crate) fn composite_over(surf: &mut Surface, src: &Surface, area: Rect, limit: Option<&Surface>) -> Result<()> {
    let source = Source::Pixels(area, read_rgba(src, area, "pasting pixels")?);
    blend_into(surf, area, &source, limit, Blend { mode: BlendMode::Normal, opacity: 1.0, keep_alpha: false, restore: false })
}

/// `area` of `surf` filled with an opaque `color` (Normal, 100 %), weighted by `sel` when given:
/// Clear on a targeted alpha channel or the Quick Mask (#2734).
pub(crate) fn fill_color(surf: &mut Surface, area: Rect, color: [f32; 4], sel: Option<&Surface>) -> Result<()> {
    blend_into(surf, area, &Source::Color(color), sel, Blend { mode: BlendMode::Normal, opacity: 1.0, keep_alpha: false, restore: false })
}

/// Reuse Edit › Fill's compositing for Edit › Stroke. The stroke's rasterized band
/// is already a grayscale coverage mask; opacity, blend modes and alpha locking
/// must behave identically for both commands at 8/16/32-bit depth.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blend_color_mask(
    surf: &mut Surface,
    area: Rect,
    color: [f32; 4],
    picked: Option<[f32; 4]>,
    coverage: &Surface,
    mode: BlendMode,
    opacity: f32,
    preserve_transparency: bool,
) -> Result<()> {
    let source = match picked {
        Some(cmyk) if color[3] >= 1.0 => Source::Cmyk { rgba: color, cmyk },
        _ => Source::Color(color),
    };
    blend_into(surf, area, &source, Some(coverage), Blend { mode, opacity, keep_alpha: preserve_transparency, restore: false })
}

/// The selection filled from its surroundings (`photocraft_algo::content_aware`), as straight
/// RGBA over the sampling window. Only the selected pixels are used.
fn content_aware(surf: &Surface, sel: &Surface, canvas: Rect, color_adaptation: bool, seed: u64) -> Result<Source> {
    let hole_bounds = sel.content_bounds().intersect(&canvas);
    if hole_bounds.is_empty() {
        return Err(EngineError::Other("the selection is outside the canvas".into()));
    }
    let is_hole = |x: i32, y: i32| sel.sample_channel(x, y, 0) > 0.0;
    let (window, filled) = content_aware_native(surf, hole_bounds, canvas, color_adaptation, seed, is_hole, |_, _| true)?;
    let fmt = surf.format();
    let px = filled.chunks_exact(fmt.channels().max(1)).map(|c| to_rgba(&fmt, c)).collect();
    Ok(Source::Pixels(window, px))
}

/// The pixels `is_hole` picks inside `hole_bounds` filled from their surroundings
/// (`photocraft_algo::content_aware`): Photoshop's sampling window around `hole_bounds`, sampling
/// only the non-hole pixels `usable` allows. Returns the window and its pixels in the surface's
/// own channels (the hole filled, the rest as read). Shared by Edit › Fill › Content-Aware and the
/// Crop tool's Content-Aware fill (#3823).
pub(crate) fn content_aware_native(
    surf: &Surface,
    hole_bounds: Rect,
    canvas: Rect,
    color_adaptation: bool,
    seed: u64,
    is_hole: impl Fn(i32, i32) -> bool,
    usable: impl Fn(i32, i32) -> bool,
) -> Result<(Rect, Vec<f32>)> {
    use photocraft_algo::content_aware::{FillOptions, color_level, fill, rotation_level};
    let window = sampling_window(hole_bounds, canvas);
    let n = surf.format().channels();
    let (w, h) = window_size(window)?;
    let img = surf.read_region(window);
    let len = w.checked_mul(h).ok_or_else(|| EngineError::Other("the area is too large for Content-Aware fill".into()))?;
    let mut hole = crate::allocation::filled(len, false, "Content-Aware fill")?;
    let mut source = crate::allocation::filled(len, false, "Content-Aware fill")?;
    for (i, (hv, sv)) in hole.iter_mut().zip(source.iter_mut()).enumerate() {
        let (x, y) = (window.x0 + (i % w) as i32, window.y0 + (i / w) as i32);
        *hv = is_hole(x, y);
        *sv = !*hv && usable(x, y);
    }
    if !source.iter().any(|s| *s) {
        return Err(EngineError::Other("Content-Aware fill needs pixels outside the area to fill to sample from".into()));
    }
    let opts = FillOptions {
        color_adaptation: color_level(if color_adaptation { "default" } else { "none" }),
        rotations: rotation_level("none"),
        scale: false,
        mirror: false,
        seed,
    };
    if img.len() != len * n {
        return Err(EngineError::Other("could not read the layer pixels".into()));
    }
    Ok((window, fill(w, h, n, &img, &hole, &source, &opts)))
}

/// Photoshop's Content-Aware sampling window around the selection bounds `hole`
/// ([`photocraft_algo::content_aware::sampling_window`]): Edit › Fill, the workspace's Rectangular
/// and Auto areas and the retouching tools all copy from within it. Worked out in i64, so a loaded
/// document declaring an absurd selection can't overflow it (#963).
pub(crate) fn sampling_window(hole: Rect, canvas: Rect) -> Rect {
    photocraft_algo::content_aware::sampling_window(hole.intersect(&canvas), canvas)
}

/// The size of a Content-Aware sampling window, or an error for a window too large to read and
/// fill (#963). Same cap as other whole-image operations: refuse absurd windows instead of
/// allocating.
pub(crate) fn window_size(window: Rect) -> Result<(usize, usize)> {
    let (w, h) = (window.width(), window.height());
    if u64::from(w) * u64::from(h) > 400_000_000 {
        return Err(EngineError::Other("the selection is too large for Content-Aware fill".into()));
    }
    Ok((w as usize, h as usize))
}

/// [`Surface::read_region`] of `r`, read in parallel bands and asking `cancelled` before each
/// (`None` once it says so): a Content-Aware window can be tens of megapixels.
pub(crate) fn read_window(surf: &Surface, r: Rect, cancelled: &(dyn Fn() -> bool + Sync)) -> Option<Vec<f32>> {
    use rayon::prelude::*;
    const BAND: usize = 64;
    let n = surf.channels();
    let (w, h) = (r.width() as usize, r.height() as usize);
    let mut out = vec![0.0f32; w * h * n];
    if out.is_empty() {
        return Some(out);
    }
    // Each worker reads its bands through one buffer of its own.
    out.par_chunks_mut(BAND * w * n).enumerate().for_each_init(Vec::new, |buf, (i, chunk)| {
        if cancelled() {
            return;
        }
        let y0 = r.y0 + (i * BAND) as i32;
        let band = Rect::new(r.x0, y0, r.x1, y0 + (chunk.len() / (w * n)) as i32);
        surf.read_region_into(band, buf);
        if buf.len() == chunk.len() {
            chunk.copy_from_slice(buf);
        }
    });
    if cancelled() { None } else { Some(out) }
}

/// Channel 0 of [`read_window`] (a selection's or an alpha channel's coverage).
pub(crate) fn read_window_mask(surf: &Surface, r: Rect, cancelled: &(dyn Fn() -> bool + Sync)) -> Option<Vec<f32>> {
    use rayon::prelude::*;
    let n = surf.channels().max(1);
    let all = read_window(surf, r, cancelled)?;
    if n == 1 {
        return Some(all);
    }
    Some(all.par_chunks_exact(n).map(|px| px[0]).collect())
}

/// How the source goes onto the target.
#[derive(Clone, Copy)]
struct Blend {
    mode: BlendMode,
    opacity: f32,
    /// Leave every pixel's alpha as it was (Preserve Transparency, or locked transparency).
    keep_alpha: bool,
    /// Normal mode replaces the pixels, transparency included (History: back to that state).
    restore: bool,
}

/// `d` moved towards `c` by `k`, in premultiplied space (straight RGBA in and out).
fn lerp(d: [f32; 4], c: [f32; 4], k: f32) -> [f32; 4] {
    let a = d[3] * (1.0 - k) + c[3] * k;
    if a <= 0.0 {
        return [0.0; 4];
    }
    let ch = |i: usize| (d[i] * d[3] * (1.0 - k) + c[i] * c[3] * k) / a;
    [ch(0), ch(1), ch(2), a]
}

/// Blend `source` into `surf` over `area`: per pixel, the selection coverage times the opacity,
/// in the blend mode.
/// Works tile by tile in parallel on the encoded pixels (no full-surface f32 round trip).
fn blend_into(surf: &mut Surface, area: Rect, source: &Source, sel: Option<&Surface>, b: Blend) -> Result<()> {
    use rayon::prelude::*;
    let fmt = surf.format();
    // An opaque colour in Normal mode at 100 % that may replace alpha: fully covered tiles become
    // one shared solid tile (copy-on-write), which costs nothing per pixel.
    let mut enc = [0.0f32; 8];
    let solid = match encode_color(&fmt, source, &mut enc) {
        Some(n) if b.mode == BlendMode::Normal && b.opacity >= 1.0 && (!b.keep_alpha || !fmt.alpha) => Some(surf.solid_tile(enc.get(..n).unwrap_or(&[]))),
        _ => None,
    };
    let count = area.tiles().count();
    let done = crate::allocation::capacity(count, "blending pixels")?;
    let tiles = surf
        .try_take_tiles(area)
        .map_err(|e| EngineError::Other(format!("not enough memory while preparing pixel tiles ({e}); the document was not changed")))?;
    let done = std::sync::Mutex::new(done);
    tiles.into_par_iter().try_for_each(|(tc, tile)| -> Result<()> {
        let tr = tc.rect().intersect(&area);
        let mask = sel.map(|m| Mask::of(m, tc));
        if let Some(s) = &solid
            && tr == tc.rect()
            && mask.as_ref().is_none_or(|m| m.full(tr))
        {
            done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((tc, s.clone()));
            return Ok(());
        }
        // Nothing selected in this tile (or no opacity): `blend_tile` would leave every pixel as
        // it is, so keep the tile itself rather than a copy the history would hold again (#4517).
        if b.opacity <= 0.0 || mask.as_ref().is_some_and(|m| m.empty(tr)) {
            done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((tc, tile));
            return Ok(());
        }
        let mut tile = match std::sync::Arc::try_unwrap(tile) {
            Ok(tile) => tile,
            Err(tile) => tile
                .try_clone()
                .map_err(|e| EngineError::Other(format!("not enough memory while blending a pixel tile ({e}); the document was not changed")))?,
        };
        blend_tile(tile.bytes_mut(), &fmt, tc.rect(), tr, source, mask.as_ref(), b);
        done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((tc, std::sync::Arc::new(tile)));
        Ok(())
    })?;
    let done = done.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
    surf.put_tiles(done);
    Ok(())
}

/// The selection's coverage over one tile (its channel 0).
pub(crate) struct Mask<'a> {
    bytes: &'a [u8],
    fmt: photocraft_color::PixelFormat,
    origin: Rect,
}

impl<'a> Mask<'a> {
    /// `sel` over the tile at `tc` (its default pixel where it has no tile there).
    pub(crate) fn of(sel: &'a Surface, tc: photocraft_geom::TileCoord) -> Self {
        Mask { bytes: sel.tile(tc).map_or(sel.default_bytes(), |t| t.bytes()), fmt: sel.format(), origin: tc.rect() }
    }
    pub(crate) fn at(&self, x: i32, y: i32) -> f32 {
        let bpp = self.fmt.bytes_per_pixel();
        // A default-pixel mask is one pixel long: every position reads it.
        let i = if self.bytes.len() == bpp { 0 } else { ((y - self.origin.y0) as usize * TILE_SIZE as usize + (x - self.origin.x0) as usize) * bpp };
        self.bytes.get(i..i + bpp).map_or(0.0, |px| photocraft_color::read_sample(px, self.fmt.sample, 0))
    }
    /// Fully selected over `r`?
    pub(crate) fn full(&self, r: Rect) -> bool {
        (r.y0..r.y1).all(|y| (r.x0..r.x1).all(|x| self.at(x, y) >= 1.0))
    }
    /// Whether nothing in `r` is selected.
    pub(crate) fn empty(&self, r: Rect) -> bool {
        (r.y0..r.y1).all(|y| (r.x0..r.x1).all(|x| self.at(x, y) <= 0.0))
    }
}

/// Blend into one tile's encoded pixels (`bytes`, the tile at `origin`) over `tr`.
fn blend_tile(bytes: &mut [u8], fmt: &photocraft_color::PixelFormat, origin: Rect, tr: Rect, source: &Source, mask: Option<&Mask>, b: Blend) {
    let Blend { mode, opacity, keep_alpha, restore } = b;
    let n = fmt.channels();
    let bpp = fmt.bytes_per_pixel();
    let blend = if mode == BlendMode::Dissolve { BlendMode::Normal } else { mode };
    let mut px = [0.0f32; 8];
    let mut enc = [0.0f32; 8];
    // An opaque colour at full coverage in Normal mode just replaces the colour (and the alpha
    // unless it is kept): copy its encoded bytes.
    let opaque = match encode_color(fmt, source, &mut enc) {
        Some(m) if mode == BlendMode::Normal => {
            let mut bytes = vec![0u8; bpp];
            photocraft_raster::encode_pixel(fmt, enc.get(..m.min(n)).unwrap_or(&[]), &mut bytes);
            // Keeping alpha: copy the colour samples only (alpha is the last sample).
            let keep = if keep_alpha && fmt.alpha { bpp - bpp / n.max(1) } else { bpp };
            Some((bytes, keep))
        }
        _ => None,
    };
    for y in tr.y0..tr.y1 {
        let row = (y - origin.y0) as usize * TILE_SIZE as usize;
        for x in tr.x0..tr.x1 {
            let mut k = mask.map_or(1.0, |m| m.at(x, y)) * opacity;
            if k <= 0.0 {
                continue;
            }
            let i = (row + (x - origin.x0) as usize) * bpp;
            let Some(pb) = bytes.get_mut(i..i + bpp) else { continue };
            if k >= 1.0
                && let Some((enc_px, keep)) = &opaque
            {
                if let (Some(dst), Some(src)) = (pb.get_mut(..*keep), enc_px.get(..*keep)) {
                    dst.copy_from_slice(src);
                }
                continue;
            }
            for (c, v) in px.iter_mut().enumerate().take(n) {
                *v = photocraft_color::read_sample(pb, fmt.sample, c);
            }
            let d = to_rgba(fmt, &px[..n.min(8)]);
            let mut c = source.at(x, y);
            if mode == BlendMode::Dissolve {
                // Dissolve: each pixel is either fully painted or untouched, with the coverage as odds.
                k = if photocraft_color::dither_noise(x, y) < c[3] * k { 1.0 } else { 0.0 };
                c[3] = 1.0;
                if k <= 0.0 {
                    continue;
                }
            }
            let keep = keep_alpha || !fmt.alpha;
            let mut o = if restore && blend == BlendMode::Normal {
                lerp(d, c, k)
            } else if keep {
                // Alpha kept: source atop, so a half-transparent pixel takes the new colour by the
                // source alpha alone (#5783).
                photocraft_color::blend::composite_atop(blend, d, c, k)
            } else {
                photocraft_color::blend::composite(blend, d, c, k)
            };
            if keep {
                o[3] = d[3];
            }
            let m = from_rgba_into(fmt, o, &mut enc);
            photocraft_raster::encode_pixel(fmt, enc.get(..m.min(n)).unwrap_or(&[]), pb);
        }
    }
}

#[cfg(test)]
mod tests;
