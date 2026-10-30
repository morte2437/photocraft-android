//! Pattern paint: Pattern Overlay, pattern strokes and pattern fill layers.
//!
//! A pattern tiles the plane from an origin (the layer's frame when linked, else the canvas
//! origin, plus the phase), scaled and rotated about that origin. Samples are bilinear with
//! wrap-around on premultiplied colour, so integer placements at 100 % reproduce the tile
//! exactly and scaled ones stay seamless. Below 100 % each document pixel averages an
//! `n × n` grid of bilinear taps over its footprint in the tile (see [`pixel_taps`]), so a
//! minified tile is box-filtered instead of aliased; at 100 % and above it is a single tap.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use photocraft_doc::{ColorMode, Pattern, PixelFormat, Rect, SampleType};

/// What one rendering call may keep of its patterns' working copies: every pattern it paints with
/// stays converted for the call (four of the largest 8-bit ones on desktop), so none is converted
/// once per output tile. A ceiling, not an allocation: only the patterns a call paints with are
/// converted. Past it a pattern is converted under one lock and shared while in use (see
/// [`PreparedPatterns::get`]). The web build keeps a quarter of that.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const PREPARED_PATTERN_BYTES: usize = 1 << 30;
/// See the desktop value above.
#[cfg(target_arch = "wasm32")]
pub(crate) const PREPARED_PATTERN_BYTES: usize = 256 << 20;
const MAX_PREPARED_PATTERNS: usize = 64;
/// Absolute cap on a single pattern's working copy on the uncached path: the document's own cap
/// on what paints (`Pattern::is_paintable`). A pattern above it is skipped, not converted.
const MAX_UNCACHED_PATTERN_BYTES: usize = photocraft_doc::pattern::MAX_WORKING_BYTES as usize;
// A call keeps at least two of the largest, or a second large fill is converted for every tile.
const _: () = assert!(2 * photocraft_doc::pattern::MAX_WORKING_BYTES <= PREPARED_PATTERN_BYTES as u64);
type PreparedSlot = Arc<OnceLock<Option<Arc<Tile>>>>;

struct PreparedState {
    slots: HashMap<usize, PreparedSlot>,
    reserved_bytes: usize,
}

/// Patterns past the call's budget: converted one at a time under this lock, and shared by the
/// workers using one at the same moment (a `Weak` per pattern), so the extra copies in memory are
/// the patterns in use, not one per worker.
#[derive(Default)]
struct OverBudget {
    live: Mutex<HashMap<usize, Weak<Tile>>>,
    warned: AtomicBool,
}

/// Demand-prepared pixels owned by one rendering call, never by the document.
pub(crate) struct PreparedPatterns<'a> {
    patterns: &'a [Pattern],
    budget: usize,
    /// The colour mode the patterns are painted into (see [`pattern_rgba`]).
    mode: ColorMode,
    state: Mutex<PreparedState>,
    over_budget: OverBudget,
}

impl<'a> PreparedPatterns<'a> {
    pub(crate) fn new(patterns: &'a [Pattern], budget: usize) -> Self {
        Self {
            patterns,
            budget,
            mode: ColorMode::Rgb,
            state: Mutex::new(PreparedState { slots: HashMap::new(), reserved_bytes: 0 }),
            over_budget: OverBudget::default(),
        }
    }

    /// Patterns painted into a document of colour mode `mode`.
    pub(crate) fn in_mode(mut self, mode: ColorMode) -> Self {
        self.mode = mode;
        self
    }

    pub(crate) fn source(&self) -> &'a [Pattern] {
        self.patterns
    }

    pub(crate) fn get(&self, id: &str, name: &str) -> Option<Arc<Tile>> {
        let p = photocraft_doc::pattern::find(self.patterns, id, name)?;
        if !p.is_paintable() {
            return None;
        }
        let bytes = usize::try_from(p.working_bytes()).ok();
        // The borrowed immutable slice pins these addresses for the complete call.
        let key = std::ptr::from_ref(p) as usize;
        let slot = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(slot) = state.slots.get(&key) {
                Some(Arc::clone(slot))
            } else if let Some(bytes) = bytes
                && bytes <= self.budget.saturating_sub(state.reserved_bytes)
                && state.slots.len() < MAX_PREPARED_PATTERNS
            {
                let slot: PreparedSlot = Arc::new(OnceLock::new());
                // Reserve before initialization: concurrent pending buffers consume the budget too.
                state.reserved_bytes += bytes;
                state.slots.insert(key, Arc::clone(&slot));
                Some(slot)
            } else {
                None
            }
        };
        match slot {
            Some(slot) => {
                // Conversion must stay serial: Rayon work stealing can re-enter this slot
                // while its initializer is running inside an effect-map OnceLock.
                // Run on the requesting worker to preserve its active CMYK profile.
                slot.get_or_init(|| Tile::new_in(p, self.mode).map(Arc::new)).clone()
            }
            // Uncached path: a pattern whose working copy would exceed an absolute cap is not
            // materialized at all. One past the call's budget is converted under one lock, and
            // shared while some worker still holds it, so workers painting tiles at the same
            // time do not each convert their own copy (CPU and memory), and it says so once.
            // The cap is independent of the (possibly zero) cache budget, so small patterns
            // still convert on demand when nothing is cached.
            None if bytes.is_none_or(|b| b > MAX_UNCACHED_PATTERN_BYTES) => None,
            None => {
                // The lock is held through the conversion, so conversions of different
                // over-budget patterns run one at a time. That is the point (one extra copy
                // being built at once, not one per worker) and fine for a fallback: patterns
                // that fit the budget never come here.
                let mut live = self.over_budget.live.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(shared) = live.get(&key).and_then(Weak::upgrade) {
                    return Some(shared);
                }
                if !self.over_budget.warned.swap(true, Ordering::Relaxed) {
                    // Written, not printed: a warning must never panic, even with no stderr.
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        use std::io::Write as _;
                        let _ = writeln!(
                            std::io::stderr(),
                            "photocraft: pattern \"{}\" is past this render's pattern memory ({} MiB) and is converted again as tiles need it",
                            p.display_name(),
                            self.budget >> 20
                        );
                    }
                }
                let tile = Tile::new_in(p, self.mode).map(Arc::new)?;
                live.retain(|_, w| w.strong_count() > 0);
                live.insert(key, Arc::downgrade(&tile));
                Some(tile)
            }
        }
    }
}

/// A pattern's pixels (straight-alpha RGBA, row-major) as they look painted into a document of
/// colour mode `mode`. A pattern keeps its own colours (an RGB pattern stays RGB, in the
/// library and in the document), but in a Grayscale or CMYK document it paints that mode's
/// colours: what Edit › Fill writes into the document's pixels, and what Photoshop shows (the
/// pattern in colour in its picker, filled in grey). Empty for a pattern that isn't
/// [paintable](Pattern::is_paintable).
pub fn pattern_rgba(p: &Pattern, mode: ColorMode) -> Vec<[f32; 4]> {
    let mut px = Vec::new();
    if p.is_paintable() && pattern_rgba_rows(p, mode, 0, p.height, &mut px).is_none() {
        px = Vec::new();
    }
    px
}

/// Rows `top..bottom` of [`pattern_rgba`] (of a paintable pattern), replacing `out`: a large
/// pattern is read a band at a time instead of as one copy at 16 bytes per pixel. `None` when the
/// memory for the rows cannot be had (on the web a failed allocation aborts the tab, so it is
/// asked for, not assumed).
pub fn pattern_rgba_rows(p: &Pattern, mode: ColorMode, top: u32, bottom: u32, out: &mut Vec<[f32; 4]>) -> Option<()> {
    let bottom = bottom.min(p.height);
    let top = top.min(bottom);
    out.clear();
    let pixels = (p.width as usize).checked_mul((bottom - top) as usize)?;
    out.try_reserve_exact(pixels).ok()?;
    out.resize(pixels, [0.0f32; 4]);
    p.surface.read_rgba_into(Rect::new(0, top as i32, p.width as i32, bottom as i32), out);
    let paints_in = matches!(mode, ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone | ColorMode::Cmyk);
    if paints_in && p.surface.format().mode != mode {
        let fmt = PixelFormat::new(mode, SampleType::F32, true);
        let mut v = [0.0f32; 5];
        for q in out.iter_mut() {
            let n = photocraft_raster::from_rgba_into(&fmt, *q, &mut v);
            *q = photocraft_raster::to_rgba(&fmt, v.get(..n).unwrap_or_default());
        }
    }
    Some(())
}

/// The pixels of a paintable pattern as they paint in a document of colour mode `mode`, rounded
/// to the pattern's own sample grid by `quantize`, read a band of rows at a time: no full-size
/// `f32` copy. A pattern with its own colours comes through exactly (its samples are on that
/// grid); one converted to a grey or CMYK document is rounded to the grid it was drawn on. `None`
/// when the memory cannot be had.
fn quantized_rgba<T>(p: &Pattern, mode: ColorMode, quantize: impl Fn(f32) -> T) -> Option<Vec<[T; 4]>> {
    let mut out = Vec::new();
    out.try_reserve_exact((p.width as usize).checked_mul(p.height as usize)?).ok()?;
    let mut band = Vec::new();
    for top in (0..p.height).step_by(photocraft_doc::pattern::BAND_ROWS as usize) {
        pattern_rgba_rows(p, mode, top, top + photocraft_doc::pattern::BAND_ROWS, &mut band)?;
        out.extend(band.iter().map(|c| [quantize(c[0]), quantize(c[1]), quantize(c[2]), quantize(c[3])]));
    }
    Some(out)
}

/// `u8 / 255` as `f32`, for every `u8`: the value an 8-bit sample reads as.
const UNIT8: [f32; 256] = {
    let mut table = [0.0f32; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = i as f32 / 255.0;
        i += 1;
    }
    table
};

/// A pattern's pixels as the sampler holds them.
enum Texels {
    /// Premultiplied `f32` RGBA, 16 bytes per pixel: any pattern that fits.
    Premultiplied(Vec<[f32; 4]>),
    /// Straight RGBA8, 4 bytes per pixel, premultiplied as each tap reads it (see [`UNIT8`]): a
    /// large 8-bit pattern, with the values the expanded copy would hold.
    Rgba8(Vec<[u8; 4]>),
    /// Straight RGBA16, 8 bytes per pixel, premultiplied as each tap reads it: a large 16-bit one.
    Rgba16(Vec<[u16; 4]>),
}

/// A pattern converted once for sampling.
pub struct Tile {
    w: usize,
    h: usize,
    texels: Texels,
}

impl Tile {
    pub fn new(p: &Pattern) -> Option<Tile> {
        Tile::new_in(p, ColorMode::Rgb)
    }

    /// The tile of `p` painted into a document of colour mode `mode` (see [`pattern_rgba`]).
    /// `None` when the pattern isn't [paintable](Pattern::is_paintable).
    pub fn new_in(p: &Pattern, mode: ColorMode) -> Option<Tile> {
        if !p.is_paintable() {
            return None;
        }
        Tile::build(p, mode, p.pixel_count().saturating_mul(16) > photocraft_doc::pattern::MAX_WORKING_BYTES)
    }

    /// [`Tile::new_in`], held at the pattern's own depth when `compact` and it has a smaller form.
    fn build(p: &Pattern, mode: ColorMode, compact: bool) -> Option<Tile> {
        let (w, h) = (p.width as usize, p.height as usize);
        let texels = match p.surface.format().sample {
            // A large pattern is held at its own depth (`Pattern::working_bytes`).
            SampleType::U8 if compact => Texels::Rgba8(quantized_rgba(p, mode, |v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)?),
            SampleType::U16 if compact => Texels::Rgba16(quantized_rgba(p, mode, |v| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16)?),
            _ => {
                let mut px = pattern_rgba(p, mode);
                if px.len() as u64 != p.pixel_count() {
                    return None;
                }
                for q in &mut px {
                    for c in 0..3 {
                        q[c] *= q[3];
                    }
                }
                Texels::Premultiplied(px)
            }
        };
        Some(Tile { w, h, texels })
    }

    #[inline]
    fn at(&self, x: i64, y: i64) -> [f32; 4] {
        let xi = x.rem_euclid(self.w as i64) as usize;
        let yi = y.rem_euclid(self.h as i64) as usize;
        let i = yi * self.w + xi;
        match &self.texels {
            Texels::Premultiplied(px) => px.get(i).copied().unwrap_or_default(),
            Texels::Rgba8(px) => {
                let [r, g, b, a] = px.get(i).copied().unwrap_or_default();
                let a = UNIT8[usize::from(a)];
                [UNIT8[usize::from(r)] * a, UNIT8[usize::from(g)] * a, UNIT8[usize::from(b)] * a, a]
            }
            Texels::Rgba16(px) => {
                let [r, g, b, a] = px.get(i).copied().unwrap_or_default();
                let unit = |v: u16| f32::from(v) / 65535.0;
                let a = unit(a);
                [unit(r) * a, unit(g) * a, unit(b) * a, a]
            }
        }
    }

    /// Bilinear, wrapping sample at continuous tile coordinates (pixel centres at `i + 0.5`).
    /// Returns straight alpha.
    pub fn sample(&self, u: f64, v: f64) -> [f32; 4] {
        unpremultiply(self.sample_premul(u, v))
    }

    /// The pattern at the centre `(x, y)` of a document pixel, filtered over the pixel's
    /// footprint in the tile when the placement shrinks it. Returns straight alpha.
    pub fn sample_area(&self, place: &Placement, x: f64, y: f64) -> [f32; 4] {
        let n = pixel_taps(place.inv_scale);
        if n == 1 {
            let (u, v) = place.map(x, y);
            return self.sample(u, v);
        }
        let mut acc = [0.0f32; 4];
        for j in 0..n {
            for i in 0..n {
                let (u, v) = place.map(x + tap_offset(i, n), y + tap_offset(j, n));
                let q = self.sample_premul(u, v);
                for c in 0..4 {
                    acc[c] += q[c];
                }
            }
        }
        let k = 1.0 / (n * n) as f32;
        unpremultiply([acc[0] * k, acc[1] * k, acc[2] * k, acc[3] * k])
    }

    /// Bilinear, wrapping sample, premultiplied.
    fn sample_premul(&self, u: f64, v: f64) -> [f32; 4] {
        let (u, v) = (u - 0.5, v - 0.5);
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = ((u - x0) as f32, (v - y0) as f32);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let mut acc = [0.0f32; 4];
        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
            if wy == 0.0 {
                continue;
            }
            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                if wx == 0.0 {
                    continue;
                }
                let p = self.at(x0.saturating_add(dx), y0.saturating_add(dy));
                let k = wx * wy;
                for c in 0..4 {
                    acc[c] += p[c] * k;
                }
            }
        }
        acc
    }
}

/// Most taps per axis of the minification filter (the WGSL mirror uses the same cap).
const MAX_TAPS: usize = 8;

/// Taps per axis for a placement whose inverse scale is `inv_scale`: one pixel covers
/// `inv_scale` tile pixels, so that many taps (capped) box-filter it. 1 at 100 % and above.
fn pixel_taps(inv_scale: f64) -> usize {
    let s = inv_scale as f32;
    if s.is_finite() && s > 1.0 { (s.ceil() as usize).min(MAX_TAPS) } else { 1 }
}

/// Offset in document pixels of tap `i` of `n`, centred on the pixel.
fn tap_offset(i: usize, n: usize) -> f64 {
    (i as f64 + 0.5) / n as f64 - 0.5
}

fn unpremultiply(acc: [f32; 4]) -> [f32; 4] {
    if acc[3] <= 0.0 {
        return [0.0; 4];
    }
    [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3].min(1.0)]
}

/// Where a pattern sits: document point → tile coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    origin: (f64, f64),
    /// Inverse rotation (cos, sin) and inverse scale.
    cs: (f64, f64),
    inv_scale: f64,
}

impl Placement {
    /// `frame`: the layer's frame (used when `link`); `phase` in pixels; `scale` as a fraction;
    /// `angle` in degrees counter-clockwise.
    pub fn new(frame: Rect, link: bool, phase: (f32, f32), scale: f32, angle: f32) -> Placement {
        let anchor = if frame.is_empty() { (0.0, 0.0) } else { (f64::from(frame.x0), f64::from(frame.y0)) };
        Self::anchored(anchor, link, phase, scale, angle)
    }

    /// Like [`Placement::new`], with the linked origin given as a point. Layer effects anchor
    /// linked patterns at the layer's effects reference point (PSD `fxrp`): Photoshop's
    /// pattern overlays and pattern strokes match pixel-for-pixel tiled from there.
    pub fn anchored(anchor: (f64, f64), link: bool, phase: (f32, f32), scale: f32, angle: f32) -> Placement {
        let base = if link { anchor } else { (0.0, 0.0) };
        let origin = (base.0 + f64::from(phase.0), base.1 + f64::from(phase.1));
        let s = f64::from(scale);
        let inv_scale = if s.is_finite() && s > 1e-3 { 1.0 / s } else { 1.0 };
        let a = f64::from(angle).to_radians();
        Placement { origin, cs: (a.cos(), a.sin()), inv_scale }
    }

    /// (origin, inverse rotation (cos, sin), inverse scale), for GPU shaders.
    pub fn parts(&self) -> ((f64, f64), (f64, f64), f64) {
        (self.origin, self.cs, self.inv_scale)
    }

    /// Tile coordinates of the document point `(x, y)`.
    #[inline]
    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        let (dx, dy) = (x - self.origin.0, y - self.origin.1);
        // Undo a counter-clockwise rotation (y points down on screen).
        let (c, s) = self.cs;
        let (rx, ry) = (dx * c - dy * s, dx * s + dy * c);
        (rx * self.inv_scale, ry * self.inv_scale)
    }
}

/// Renders `tile` over `rect` (row-major straight RGBA).
pub fn render(tile: &Tile, place: &Placement, rect: Rect) -> Vec<[f32; 4]> {
    let mut out = Vec::with_capacity(rect.width() as usize * rect.height() as usize);
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            out.push(tile.sample_area(place, f64::from(x) + 0.5, f64::from(y) + 0.5));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{PixelFormat, Surface};

    fn checker() -> Pattern {
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 4, 4), &[1.0, 0.0, 0.0, 1.0]);
        s.fill_rect(Rect::new(0, 0, 2, 2), &[0.0, 0.0, 1.0, 1.0]);
        s.fill_rect(Rect::new(2, 2, 4, 4), &[0.0, 0.0, 1.0, 1.0]);
        Pattern::new("checker", s, 4, 4)
    }

    #[test]
    fn identity_placement_tiles_exactly() {
        let patterns = [checker()];
        let prepared = PreparedPatterns::new(&patterns, PREPARED_PATTERN_BYTES);
        let t = prepared.get(&patterns[0].id, "").unwrap();
        let again = prepared.get(&patterns[0].id, "").unwrap();
        assert!(Arc::ptr_eq(&t, &again));
        let uncached = PreparedPatterns::new(&patterns, 0);
        let fallback = uncached.get(&patterns[0].id, "").unwrap();
        let p = Placement::new(Rect::new(0, 0, 10, 10), false, (0.0, 0.0), 1.0, 0.0);
        let px = render(&t, &p, Rect::new(-4, 0, 8, 1));
        assert_eq!(px, render(&fallback, &p, Rect::new(-4, 0, 8, 1)));
        assert_eq!(px[0], [0.0, 0.0, 1.0, 1.0]); // x = -4 wraps to 0
        assert_eq!(px[6], [1.0, 0.0, 0.0, 1.0]); // x = 2
        assert_eq!(px[8], [0.0, 0.0, 1.0, 1.0]); // x = 4
    }

    #[test]
    fn patterns_too_large_to_paint_are_skipped_without_allocating() {
        // Only the size is declared (no tiles): a hostile file can claim any size.
        let p = Pattern { id: "big".into(), name: "Big".into(), width: 9000, height: 8000, surface: Surface::new(PixelFormat::RGBA8) };
        assert!(!p.is_paintable());
        assert!(Tile::new(&p).is_none());
        assert!(pattern_rgba(&p, ColorMode::Rgb).is_empty());
        let list = [p];
        assert!(PreparedPatterns::new(&list, 0).get("big", "").is_none());
    }

    /// Review of #8902: what `Pattern::is_paintable` admits, the compositor expands. A pattern that
    /// fits is expanded as before; a large one is held at its own depth, so Define Pattern's
    /// 8000 x 8000 fits at 8 bits. (Only the size is declared: the patterns themselves are empty.)
    #[test]
    fn the_most_paintable_pixels_still_expand() {
        let sparse = |name: &str, (width, height): (u32, u32), format: PixelFormat| Pattern {
            id: name.into(),
            name: name.into(),
            width,
            height,
            surface: Surface::new(format),
        };
        let float = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
        let list = [
            sparse("max8", (8000, 8000), PixelFormat::RGBA8),
            sparse("max16", (5792, 5792), PixelFormat::RGBA16),
            sparse("max32", (4096, 4096), float),
            sparse("over8", (9000, 8000), PixelFormat::RGBA8),
            sparse("over16", (8000, 8000), PixelFormat::RGBA16),
            sparse("over32", (4097, 4096), float),
        ];
        assert_eq!(list.each_ref().map(|p| p.is_paintable()), [true, true, true, false, false, false]);
        let prepared = PreparedPatterns::new(&list, PREPARED_PATTERN_BYTES);
        let held = |name: &str| prepared.get(name, "").map(|t| (matches!(t.texels, Texels::Rgba8(_)), matches!(t.texels, Texels::Rgba16(_))));
        assert_eq!(held("max8"), Some((true, false)), "an 8-bit 8000 x 8000 pattern expands to 4 bytes a pixel");
        assert_eq!(held("max16"), Some((false, true)), "a 16-bit one to 8");
        for name in ["over8", "over16", "over32"] {
            assert!(prepared.get(name, "").is_none(), "{name} is skipped, not allocated");
        }
        assert!(PreparedPatterns::new(&list, 0).get("max8", "").is_some(), "uncached too, at the cap");
    }

    /// A render that paints with two large patterns (a pattern fill and a pattern overlay, say)
    /// converts each once, not once per output tile: both stay in the call's budget. A third that
    /// does not fit is converted when asked for (correct, just not kept).
    #[test]
    fn two_large_patterns_are_each_converted_once_per_call() {
        let sparse = |name: &str| Pattern { id: name.into(), name: name.into(), width: 5000, height: 5000, surface: Surface::new(PixelFormat::RGBA8) };
        let list = [sparse("a"), sparse("b"), sparse("c")];
        assert!(list.iter().all(|p| p.working_bytes() == 100_000_000));
        let prepared = PreparedPatterns::new(&list, 250_000_000);
        let same = |name: &str| {
            let (first, second) = (prepared.get(name, "").unwrap(), prepared.get(name, "").unwrap());
            Arc::ptr_eq(&first, &second)
        };
        assert!(same("a"), "the first is kept");
        assert!(same("b"), "and the second");
        // A third past the budget is converted under one lock and shared while a worker holds it,
        // so concurrent tiles use one copy; it is converted again once nobody holds it.
        assert!(same("c"), "shared while in use");
        let held = prepared.get("c", "").unwrap();
        let other = prepared.get("c", "").unwrap();
        assert!(Arc::ptr_eq(&held, &other), "two workers at once get one copy");
        drop((held, other));
        assert!(prepared.get("c", "").is_some(), "and it converts again after");
        // The real budget holds two of the largest paintable ones.
        assert!(2 * photocraft_doc::pattern::MAX_WORKING_BYTES <= PREPARED_PATTERN_BYTES as u64);
    }

    /// A large pattern's compact copy samples exactly as the expanded one: every 8-bit and 16-bit
    /// value survives the round trip through `f32`, and a tap reads the same premultiplied numbers.
    #[test]
    fn the_compact_copies_read_the_same_as_the_expanded_one() {
        for v in 0..=255u8 {
            assert_eq!(((f32::from(v) / 255.0) * 255.0 + 0.5) as u8, v);
            assert_eq!(UNIT8[usize::from(v)], f32::from(v) / 255.0);
        }
        for v in 0..=u16::MAX {
            assert_eq!(((f32::from(v) / 65535.0) * 65535.0 + 0.5) as u16, v);
        }
        for (format, grid) in [(PixelFormat::RGBA8, 255.0f32), (PixelFormat::RGBA16, 65535.0)] {
            let mut s = Surface::new(format);
            for y in 0..16i32 {
                for x in 0..16i32 {
                    let level = |v: i32| (v as f32 / 255.0 * grid).round() / grid;
                    s.fill_rect(Rect::new(x, y, x + 1, y + 1), &[level(x * 17), level(y * 17), level((x + y) * 8), level(255 - x * 13 - y * 3)]);
                }
            }
            let p = Pattern::new("ramp", s, 16, 16);
            let (expanded, compact) = (Tile::build(&p, ColorMode::Rgb, false).unwrap(), Tile::build(&p, ColorMode::Rgb, true).unwrap());
            assert!(matches!(expanded.texels, Texels::Premultiplied(_)));
            assert!(matches!((&compact.texels, grid as u32), (Texels::Rgba8(_), 255) | (Texels::Rgba16(_), 65535)));
            for y in 0..16 {
                for x in 0..16 {
                    assert_eq!(compact.at(x, y), expanded.at(x, y), "{grid} ({x}, {y})");
                }
            }
            // And rendered, scaled, rotated and shifted across the wrap.
            let place = Placement::new(Rect::new(3, 5, 40, 40), true, (2.0, 1.0), 0.6, 20.0);
            assert_eq!(render(&compact, &place, Rect::new(-9, -7, 30, 25)), render(&expanded, &place, Rect::new(-9, -7, 30, 25)));
        }
    }

    /// Memory for the rows is asked for, not assumed: a pattern whose rows cannot be had (here a
    /// declared size past what any allocation could be) gives `None`, never an aborted allocation.
    #[test]
    fn rows_that_cannot_be_allocated_give_none() {
        let huge = Pattern { id: "huge".into(), name: "huge".into(), width: u32::MAX, height: u32::MAX, surface: Surface::new(PixelFormat::RGBA8) };
        let mut out = Vec::new();
        assert!(pattern_rgba_rows(&huge, ColorMode::Rgb, 0, u32::MAX, &mut out).is_none());
        assert!(quantized_rgba(&huge, ColorMode::Rgb, |v| v as u8).is_none());
        assert!(pattern_rgba(&huge, ColorMode::Rgb).is_empty());
        assert!(Tile::new(&huge).is_none());
        let small = Pattern { id: "small".into(), name: "small".into(), width: 3, height: 2, surface: Surface::new(PixelFormat::RGBA8) };
        assert_eq!(pattern_rgba_rows(&small, ColorMode::Rgb, 0, 2, &mut out), Some(()));
        assert_eq!(out.len(), 6);
    }

    #[test]
    fn link_phase_scale_and_angle_move_the_origin() {
        let t = Tile::new(&checker()).unwrap();
        let linked = Placement::new(Rect::new(2, 0, 10, 10), true, (0.0, 0.0), 1.0, 0.0);
        assert_eq!(render(&t, &linked, Rect::new(2, 0, 3, 1))[0], [0.0, 0.0, 1.0, 1.0]);
        let phased = Placement::new(Rect::EMPTY, false, (2.0, 0.0), 1.0, 0.0);
        assert_eq!(render(&t, &phased, Rect::new(2, 0, 3, 1))[0], [0.0, 0.0, 1.0, 1.0]);
        let scaled = Placement::new(Rect::EMPTY, false, (0.0, 0.0), 2.0, 0.0);
        let row = render(&t, &scaled, Rect::new(0, 1, 8, 2));
        assert_eq!(row[2], [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(row[6], [1.0, 0.0, 0.0, 1.0]);
        // 90°: x and y swap roles.
        let rot = Placement::new(Rect::EMPTY, false, (0.0, 0.0), 1.0, 90.0);
        let (u, v) = rot.map(1.0, 3.0);
        assert!((u - -3.0).abs() < 1e-9 && (v - 1.0).abs() < 1e-9, "{u},{v}");
    }

    #[test]
    fn minified_pattern_is_filtered_not_aliased() {
        // 1-px dark / light stripes (mean 0.5), shown at 25 %: every pixel covers 4 tile pixels.
        // The 0.125 px phase puts each pixel centre on a light tile pixel's centre, so a single
        // bilinear tap would return pure white.
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 8, 8), &[1.0, 1.0, 1.0, 1.0]);
        for x in (0..8).step_by(2) {
            s.fill_rect(Rect::new(x, 0, x + 1, 8), &[0.0, 0.0, 0.0, 1.0]);
        }
        let t = Tile::new(&Pattern::new("stripes", s, 8, 8)).unwrap();
        let place = Placement::new(Rect::EMPTY, false, (0.125, 0.0), 0.25, 0.0);
        let (u, _) = place.map(0.5, 0.5);
        assert_eq!(t.sample(u, 0.5)[0], 1.0, "the single tap lands on a light stripe");
        for p in render(&t, &place, Rect::new(0, 0, 8, 1)) {
            assert!((p[0] - 0.5).abs() < 0.05, "{p:?}");
        }
        // 100 % stays exact.
        let one = Placement::new(Rect::EMPTY, false, (0.0, 0.0), 1.0, 0.0);
        let row = render(&t, &one, Rect::new(0, 0, 2, 1));
        assert_eq!((row[0][0], row[1][0]), (0.0, 1.0));
    }

    #[test]
    fn sampling_at_hostile_phases_does_not_overflow() {
        let patterns = [checker()];
        let prepared = PreparedPatterns::new(&patterns, PREPARED_PATTERN_BYTES);
        let t = prepared.get(&patterns[0].id, "").unwrap();
        for u in [1.0e30, -1.0e30, f64::INFINITY, f64::NAN] {
            let _ = t.sample(u, u);
        }
    }
}
