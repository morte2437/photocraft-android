//! A stroked shape layer split into its fill and its vector stroke (rasterised over the canvas),
//! for shapes with clipped layers: Photoshop draws the stroke above the clipped layers. Cached
//! per shape state, so tiles (and the GPU's textures) reuse one rasterisation.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::vector::ShapeLayer;
use photocraft_doc::{Layer, LayerContent};
use photocraft_geom::{Rect, TileCoord};
use photocraft_raster::Surface;

/// (fill, stroke), last use, and the shape's pixels the stroke was fitted to (kept alive so
/// their tile addresses, part of the key, can't be reused).
type Entry = ((Surface, Surface), u64, Option<Surface>);
type Cache = (HashMap<u64, Entry>, u64);

fn cache() -> &'static Mutex<Cache> {
    static C: OnceLock<Mutex<Cache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new((HashMap::new(), 0)))
}

pub(crate) fn purge() {
    cache().lock().unwrap_or_else(|e| e.into_inner()).0.clear();
}

/// Pixel-byte budget for split parts and their pinned source surfaces (#4530).
#[cfg(not(test))]
const BUDGET: usize = 256 << 20;
/// Small under test, so a few 512² shapes exercise the eviction.
#[cfg(test)]
const BUDGET: usize = 8 << 20;

/// Pixel bytes retained by the parts and their source; overflow makes the entry oversized.
fn entry_bytes(parts: &(Surface, Surface), source: Option<&Surface>) -> usize {
    [&parts.0, &parts.1].into_iter().chain(source).flat_map(|s| s.tiles().map(|(_, t)| t.len())).fold(0, usize::saturating_add)
}

/// Look up a split, refreshing recency on a hit.
fn get(c: &Mutex<Cache>, key: u64) -> Option<(Surface, Surface)> {
    let mut c = c.lock().unwrap_or_else(|e| e.into_inner());
    c.1 += 1;
    let tick = c.1;
    let entry = c.0.get_mut(&key)?;
    entry.1 = tick;
    Some(entry.0.clone())
}

/// Cache a split only when its retained pixel bytes fit the budget and are nonzero.
fn insert(c: &Mutex<Cache>, key: u64, parts: &(Surface, Surface), source: Option<&Surface>) {
    let new_bytes = entry_bytes(parts, source);
    // Don't cache entries that hold no pixels or exceed the byte budget.
    if new_bytes == 0 || new_bytes > BUDGET {
        return;
    }
    let mut c = c.lock().unwrap_or_else(|e| e.into_inner());
    // Another caller may have filled this key while we rendered. Charge its replacement once.
    c.0.remove(&key);
    // Count the retained pixels once, then subtract each eviction's charge.
    let mut held: usize = c.0.values().map(|e| entry_bytes(&e.0, e.2.as_ref())).sum();
    // Least recently used first, until the new entry fits the byte budget.
    loop {
        if held.saturating_add(new_bytes) <= BUDGET {
            break;
        }
        let Some(old) = c.0.iter().min_by_key(|e| (e.1).1).map(|e| *e.0) else { break };
        let Some(removed) = c.0.remove(&old) else { break };
        held = held.saturating_sub(entry_bytes(&removed.0, removed.2.as_ref()));
    }
    c.1 += 1;
    let tick = c.1;
    c.0.insert(key, (parts.clone(), tick, source.cloned()));
}

/// Drop every cached split (Edit › Purge › All), returning its retained pixel bytes. Callers may
/// still hold returned surfaces, so this is not a measure of freed process memory.
pub fn purge_cache() -> usize {
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    let bytes = c.0.values().map(|e| entry_bytes(&e.0, e.2.as_ref())).sum();
    c.0.clear();
    bytes
}

/// (fill only, stroke only) of a stroked shape, rendered over `canvas` in RGBA at `depth`.
/// `None` without a stroke. Where the shape's pixels
/// (Photoshop's rendering, `sh.cache`) reach past our fill, the part the fill doesn't explain is
/// stroke: the stroke follows Photoshop's coverage (psd-tools double-stroke-effects).
pub fn split(sh: &ShapeLayer, canvas: Rect, depth: SampleType) -> Option<(Surface, Surface)> {
    let generation = photocraft_raster::spill::read_error_generation();
    sh.stroke.as_ref()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (canvas.x0, canvas.y0, canvas.x1, canvas.y1).hash(&mut h);
    depth.hash(&mut h);
    format!("{:?}{:?}{:?}{:?}", sh.path, sh.fill, sh.stroke, sh.live).hash(&mut h);
    (sh.density.to_bits(), sh.feather.to_bits()).hash(&mut h);
    if let Some(c) = &sh.cache {
        format!("{:?}{:?}", c.format(), c.default_pixel()).hash(&mut h);
        for (k, t) in c.tiles() {
            (k.tx, k.ty, std::sync::Arc::as_ptr(t) as usize).hash(&mut h);
        }
    }
    let key = h.finish();
    if let Some(parts) = get(cache(), key) {
        return Some(parts);
    }
    let fmt = PixelFormat::RGBA8.with_sample(depth);
    let fill_only = ShapeLayer { stroke: None, cache: None, ..sh.clone() };
    let stroke_only = ShapeLayer { fill: None, cache: None, ..sh.clone() };
    let mut parts = (photocraft_vector::render_shape(&fill_only, fmt, canvas), photocraft_vector::render_shape(&stroke_only, fmt, canvas));
    if let Some(cache) = &sh.cache {
        fit_stroke(&parts.0, &mut parts.1, cache, canvas);
    }
    if photocraft_raster::spill::check_since(generation).is_err() || sh.cache.as_ref().is_some_and(|c| c.scratch_health().is_err()) {
        return Some(parts);
    }
    insert(cache(), key, &parts, sh.cache.as_ref());
    Some(parts)
}

/// The stroke-only shape a Frame's stroke draws: the Frame tool's own border (`Layer::frame`),
/// else a Photoshop Frame's `Group::frame_stroke` along the group's enabled vector mask (the
/// frame outline). `None` for other layers and for a frame without a visible stroke (or, for a
/// Photoshop Frame, a vector mask).
pub fn frame_shape(layer: &Layer) -> Option<ShapeLayer> {
    let LayerContent::Group(g) = &layer.content else { return None };
    if let Some(shape) = layer.frame.as_ref().and_then(|f| f.stroke_shape()) {
        return Some(shape);
    }
    let stroke = g.frame_stroke.as_ref().filter(|s| s.width.is_finite() && s.width > 0.0 && s.opacity > 0.0)?;
    let vm = layer.vector_mask.as_ref().filter(|m| m.enabled && !m.path.is_empty())?;
    Some(ShapeLayer { path: vm.path.clone(), fill: None, stroke: Some(stroke.clone()), ..ShapeLayer::default() })
}

/// A Photoshop Frame's stroke ([`frame_shape`]) rendered over `canvas` in RGBA at `depth`, from
/// the same cache as the shape strokes. It goes above the frame's (masked) content.
pub fn frame_stroke(layer: &Layer, canvas: Rect, depth: SampleType) -> Option<Surface> {
    split(&frame_shape(layer)?, canvas, depth).map(|(_, stroke)| stroke)
}

/// Fits the stroke part to the shape's own pixels: where they reach past the fill, the stroke
/// covers what the fill doesn't (`(pixels - fill) / (1 - fill)`), in the pixels' colour where we
/// drew no stroke, and elsewhere in the colour that, drawn over the fill, gives back the pixels'
/// colour. Our stroke's own colour would be wrong where the file's fill covers more of the pixel
/// than ours (shapes snapped to whole pixels with a stroke thinner than a pixel): the fitted
/// stroke is then mostly the file's fill, and an edge drawn in the stroke's full colour is far
/// too dark.
fn fit_stroke(fill: &Surface, stroke: &mut Surface, cache: &Surface, canvas: Rect) {
    // Where either has pixels (the stroke may reach past the shape's pixels, and vice versa).
    let (cb, sb) = (crate::bounds::content_bounds(cache), crate::bounds::content_bounds(stroke));
    let r = if cb.is_empty() {
        sb
    } else if sb.is_empty() {
        cb
    } else {
        cb.union(&sb)
    }
    .intersect(&canvas);
    if r.is_empty() {
        return;
    }
    let float = stroke.format().sample == SampleType::F32;
    // Tiles none of the three reaches read as the defaults; with transparent defaults the fit
    // leaves the stroke's default pixel there, so those tiles are exact skips and we can scan
    // just the union of the three surfaces' allocated tiles instead of every canvas tile (two
    // sparse regions on a 300k×300k document would otherwise walk ~1.4M empty tiles). A
    // non-transparent default is everywhere, so the scan falls back to the full rectangle.
    let whole = r;
    let bare_transparent = [fill, &*stroke, cache].iter().all(|s| s.default_pixel().iter().all(|v| *v == 0.0));
    if bare_transparent {
        let mut coords: Vec<TileCoord> = fill.tiles().map(|(c, _)| *c).chain(stroke.tiles().map(|(c, _)| *c)).chain(cache.tiles().map(|(c, _)| *c)).collect();
        coords.sort_unstable();
        coords.dedup();
        for c in coords {
            let r = c.rect().intersect(&whole);
            if !r.is_empty() {
                fit_stroke_rect(fill, stroke, cache, r, float);
            }
        }
    } else {
        for t in whole.tiles() {
            let r = t.rect().intersect(&whole);
            if !r.is_empty() {
                fit_stroke_rect(fill, stroke, cache, r, float);
            }
        }
    }
}

/// [`fit_stroke`] over one rectangle.
fn fit_stroke_rect(fill: &Surface, stroke: &mut Surface, cache: &Surface, r: Rect, float: bool) {
    let n = r.width() as usize * r.height() as usize;
    let (mut f, mut s, mut c) = (vec![[0.0f32; 4]; n], vec![[0.0f32; 4]; n], vec![[0.0f32; 4]; n]);
    fill.read_rgba_into(r, &mut f);
    stroke.read_rgba_into(r, &mut s);
    cache.read_rgba_into(r, &mut c);
    let mut out = Vec::with_capacity(n * 4);
    for ((fp, sp), cp) in f.iter().zip(&s).zip(&c) {
        let mut p = *sp;
        if fp[3] < 1.0 - 1e-3 {
            let a = ((cp[3] - fp[3]) / (1.0 - fp[3])).clamp(0.0, 1.0);
            p = if sp[3] <= 0.0 {
                [cp[0], cp[1], cp[2], a]
            } else if a > 1e-3 {
                // Normal over the fill: a·p + f·(1 - a)·fill = pixels (premultiplied).
                let k = fp[3] * (1.0 - a);
                let un = |i: usize| {
                    let v = (cp[3] * cp[i] - k * fp[i]) / a;
                    // Float RGB retains HDR values; integer samples use the normalized range.
                    if float { v } else { v.clamp(0.0, 1.0) }
                };
                [un(0), un(1), un(2), a]
            } else {
                [sp[0], sp[1], sp[2], a]
            };
        }
        out.extend_from_slice(&p);
    }
    stroke.write_region(r, &out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::Color;
    use photocraft_doc::Fill;
    use photocraft_doc::vector::{Path, ShapeStroke, Subpath};

    fn shape(x: f64) -> ShapeLayer {
        ShapeLayer {
            path: Path::new(vec![Subpath::polygon(&[(x, 10.0), (x + 400.0, 10.0), (x + 400.0, 500.0), (x, 500.0)])]),
            fill: Some(Fill::Solid(Color::rgb(0.2, 0.4, 0.6))),
            stroke: Some(ShapeStroke { width: 6.0, ..ShapeStroke::default() }),
            live: None,
            cache: None,
            psd_raw: None,
            ..ShapeLayer::default()
        }
    }

    fn held() -> usize {
        cache().lock().unwrap_or_else(|e| e.into_inner()).0.values().map(|e| entry_bytes(&e.0, e.2.as_ref())).sum()
    }

    fn tile(sample: SampleType, value: f32) -> Surface {
        let mut surface = Surface::new(PixelFormat::RGBA8.with_sample(sample));
        surface.fill_rect(Rect::new(0, 0, 1, 1), &[value, value, value, 1.0]);
        surface
    }

    /// A hit protects that entry while one or several older entries make room for the new split.
    #[test]
    fn split_cache_evicts_the_exact_least_recently_used_entry() {
        for sample in [SampleType::F32, SampleType::U8] {
            let c = Mutex::new(Cache::default());
            let parts = (tile(sample, 0.25), tile(sample, 0.5));
            let source = tile(SampleType::U8, 0.75);
            let bytes = entry_bytes(&parts, Some(&source));
            let slots = (BUDGET / bytes) as u64;
            let incoming = (tile(SampleType::F32, 0.5), tile(SampleType::F32, 0.75));
            let incoming_bytes = entry_bytes(&incoming, Some(&source));
            let victims = (incoming_bytes / bytes) as u64;
            assert_eq!(incoming_bytes % bytes, 0);
            assert!(slots > victims);
            for key in 0..slots {
                assert!(get(&c, key).is_none());
                insert(&c, key, &parts, Some(&source));
            }
            assert!(get(&c, 0).is_some());
            assert!(get(&c, slots).is_none());
            insert(&c, slots, &incoming, Some(&source));
            let c = c.lock().unwrap();
            assert_eq!(c.0.len(), (slots - victims + 1) as usize);
            for key in 0..=slots {
                assert_eq!(c.0.contains_key(&key), key == 0 || key > victims, "{sample:?}: entry {key}");
            }
            assert_eq!(c.0.values().map(|e| entry_bytes(&e.0, e.2.as_ref())).sum::<usize>(), bytes * (slots - victims) as usize + incoming_bytes);
        }
    }

    /// Replacement accounts only the new parts/source and releases the previous tile owners.
    #[test]
    fn split_cache_replacement_updates_bytes_and_releases_old_surfaces() {
        let c = Mutex::new(Cache::default());
        let old_parts = (tile(SampleType::U8, 0.25), tile(SampleType::U8, 0.5));
        let source = tile(SampleType::U8, 0.75);
        let old_part = std::sync::Arc::downgrade(old_parts.0.tiles().next().unwrap().1);
        let old_source = std::sync::Arc::downgrade(source.tiles().next().unwrap().1);
        assert!(get(&c, 7).is_none());
        insert(&c, 7, &old_parts, Some(&source));
        drop(old_parts);
        drop(source);
        assert!(old_part.upgrade().is_some() && old_source.upgrade().is_some());
        let new_parts = (tile(SampleType::F32, 0.5), tile(SampleType::F32, 0.75));
        let source = tile(SampleType::U16, 0.25);
        let expected = entry_bytes(&new_parts, Some(&source));
        insert(&c, 7, &new_parts, Some(&source));
        assert!(old_part.upgrade().is_none() && old_source.upgrade().is_none());
        assert_eq!(get(&c, 7).unwrap().0.pixel(0, 0), new_parts.0.pixel(0, 0));
        let c = c.lock().unwrap();
        assert_eq!(c.0.len(), 1);
        assert_eq!(c.0.values().map(|e| entry_bytes(&e.0, e.2.as_ref())).sum::<usize>(), expected);
    }

    /// Two callers may miss before either renders; the second completion replaces the first.
    #[test]
    fn split_cache_duplicate_misses_do_not_evict_unrelated_entries() {
        let c = Mutex::new(Cache::default());
        let parts = (tile(SampleType::F32, 0.25), tile(SampleType::F32, 0.5));
        let bytes = entry_bytes(&parts, None);
        let slots = (BUDGET / bytes) as u64;
        assert!(slots >= 2);
        for key in 0..slots - 1 {
            assert!(get(&c, key).is_none());
            insert(&c, key, &parts, None);
        }
        // These are the lookup/insert lock acquisitions of two overlapping cold splits.
        assert!(get(&c, slots - 1).is_none());
        assert!(get(&c, slots - 1).is_none());
        insert(&c, slots - 1, &parts, None);
        insert(&c, slots - 1, &parts, None);
        let c = c.lock().unwrap();
        assert_eq!(c.0.len(), slots as usize, "replacing the first result must not evict a neighbor");
        for key in 0..slots {
            assert!(c.0.contains_key(&key), "entry {key} must survive");
        }
        assert_eq!(c.0.values().map(|e| entry_bytes(&e.0, e.2.as_ref())).sum::<usize>(), bytes * slots as usize);
    }

    /// Two cold splits can both miss before either completes; completion order sets recency.
    #[test]
    fn split_cache_orders_overlapping_misses_by_completion() {
        let c = Mutex::new(Cache::default());
        let parts = (tile(SampleType::F32, 0.25), tile(SampleType::F32, 0.5));
        let slots = (BUDGET / entry_bytes(&parts, None)) as u64;
        assert!(slots >= 2);
        assert!(get(&c, 0).is_none());
        assert!(get(&c, 1).is_none());
        insert(&c, 0, &parts, None);
        insert(&c, 1, &parts, None);
        {
            let c = c.lock().unwrap();
            assert!(c.0[&0].1 < c.0[&1].1, "the later completion must get a newer tick");
        }
        for key in 2..=slots {
            insert(&c, key, &parts, None);
        }
        let c = c.lock().unwrap();
        assert_eq!(c.0.len(), slots as usize);
        assert!(!c.0.contains_key(&0), "evict the earlier completion");
        assert!(c.0.contains_key(&1), "keep the later completion");
    }

    /// The split cache stays within its byte budget, evicting the least recently used, and
    /// Edit › Purge › All empties it (#4530).
    #[test]
    fn split_cache_keeps_to_its_budget_and_purges() {
        let canvas = Rect::new(0, 0, 1024, 512);
        for i in 0..6 {
            let parts = split(&shape(f64::from(i) * 10.0), canvas, SampleType::U8).expect("stroked");
            assert!(entry_bytes(&parts, None) > 0);
            assert!(held() <= BUDGET, "shape {i}: {} bytes held", held());
        }
        // Other tests share the cache, so only what this test can rely on: a purge frees bytes.
        assert!(purge_cache() > 0, "something to purge");
    }
}
