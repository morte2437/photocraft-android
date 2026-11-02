use super::composite::texture_ctx;
use super::ctx::{Ctx, PREPARE_DEPTH};
use super::extents::{effect_shape, effect_source_bounds};
use super::{Layer, LayerContent, Rect, Surface, advanced, effects};

// ---------------------------------------------------------------------------------------------
// Layer-effect map cache

/// Order-independent identity of a layer's pixels, masks, effects, Advanced Blending and Blend If
/// and (for groups) children.
/// Pixels are identified by their copy-on-write tile pointers. `pin` sees every tile hashed: a
/// cache entry keeps a [`std::sync::Weak`] to each, which reserves the tile's address (so no
/// other tile can reuse it and match the key) without keeping its pixels alive (#4530).
pub(crate) fn layer_identity(layer: &Layer, h: &mut std::collections::hash_map::DefaultHasher, pin: &mut dyn FnMut(&std::sync::Arc<photocraft_raster::Tile>)) {
    use std::hash::{Hash, Hasher};
    fn surface_fp(s: &Surface, pin: &mut dyn FnMut(&std::sync::Arc<photocraft_raster::Tile>)) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        s.format().hash(&mut h);
        // Missing tiles read the default pixel, so tile identity alone is not pixel identity.
        for sample in s.default_pixel() {
            sample.to_bits().hash(&mut h);
        }
        let tiles = s.tiles().fold(s.tile_count() as u64, |acc, (c, t)| {
            pin(t);
            let mut x = (std::sync::Arc::as_ptr(t) as usize as u64) ^ ((c.tx as u64) << 40) ^ ((c.ty as u32 as u64) << 8);
            x = x.wrapping_mul(0x9e37_79b9_7f4a_7c15);
            acc.wrapping_add(x ^ (x >> 29))
        });
        tiles.hash(&mut h);
        h.finish()
    }
    layer.id.0.hash(h);
    // What the maps depend on: `fill_opacity` shades them; layer `opacity` is applied when the
    // maps are composited and `visible` only skips the lookup entirely, so neither belongs in
    // the key (an opacity drag must not rebuild shadows over the whole layer; the GPU's
    // `fx::shape_key` keeps them out for the same reason).
    layer.fill_opacity.to_bits().hash(h);
    match &layer.content {
        LayerContent::Group(g) => {
            for c in &g.children {
                // A styled group's maps come from its children's composite, so how each child
                // composites (unlike the group's own visibility and opacity) shapes them.
                (c.visible, c.opacity.to_bits(), c.clipped).hash(h);
                format!("{:?}", c.blend).hash(h);
                layer_identity(c, h, pin);
            }
            format!("{:?}{:?}", g.artboard, g.frame_stroke).hash(h);
        }
        LayerContent::Fill(f) => {
            format!("{f:?}").hash(h);
            // The cached pixels are what shapes the maps while they match the fill: a
            // pixel-only rewrite of the cache (Image › Transform, crop) leaves `f` alone, and
            // the stale maps would keep shadowing the pre-rewrite shape.
            if let Some(c) = &layer.fill_cache
                && c.fill == *f
            {
                surface_fp(&c.surface, pin).hash(h);
            }
        }
        LayerContent::Adjustment(a) => format!("{a:?}").hash(h),
        _ => layer.surface().map_or(0, |s| surface_fp(s, pin)).hash(h),
    }
    if let Some(m) = &layer.mask {
        (surface_fp(&m.surface, pin), m.enabled, m.density.to_bits(), m.feather.to_bits()).hash(h);
    }
    if let Some(vm) = &layer.vector_mask {
        format!("{vm:?}").hash(h);
    }
    format!("{:?}", layer.effects).hash(h);
    // Transparency Shapes Layer picks the shape (content or the whole layer); a group child's
    // Blend If and knockout change the composite a styled group's shape comes from.
    layer.advanced.hash(h);
    layer.blend_if.hash(h);
    h.write_u8(0xfe);
}

struct FxEntry {
    maps: std::sync::Arc<effects::FxMaps>,
    /// The tiles the key identifies (see [`layer_identity`]): their addresses, not their pixels.
    _pin: Vec<std::sync::Weak<photocraft_raster::Tile>>,
    bytes: usize,
}

/// Global cache of effect maps (bounded by bytes), one slot per layer state. Slots are filled
/// with `set`, never `get_or_init`: nothing may block on a build (see `cached_effect_maps`).
type FxSlot = std::sync::Arc<std::sync::OnceLock<FxEntry>>;

struct FxCache {
    map: std::collections::HashMap<u64, FxSlot>,
    order: std::collections::VecDeque<u64>,
    bytes: usize,
}

/// Default effect-cache budget (Preferences › Performance can change it).
pub const DEFAULT_FX_CACHE_BUDGET: usize = 768 << 20;

static FX_CACHE_BUDGET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(DEFAULT_FX_CACHE_BUDGET);

/// Set the memory budget (bytes) of the layer-effect map cache; entries over it are evicted
/// oldest-first on the next build.
pub fn set_effect_cache_budget(bytes: usize) {
    FX_CACHE_BUDGET.store(bytes.max(1 << 20), std::sync::atomic::Ordering::Relaxed);
}

/// The current effect-cache budget in bytes.
pub fn effect_cache_budget() -> usize {
    FX_CACHE_BUDGET.load(std::sync::atomic::Ordering::Relaxed)
}

/// Bytes currently held by the effect-map cache.
pub fn effect_cache_bytes() -> usize {
    fx_cache().lock().unwrap_or_else(|e| e.into_inner()).bytes
}

/// Drop every cached effect map (Edit › Purge › All). Returns the bytes released.
pub fn purge_effect_cache() -> usize {
    let mut c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
    let freed = c.bytes;
    c.map.clear();
    c.order.clear();
    c.bytes = 0;
    freed
}

fn fx_cache() -> &'static std::sync::Mutex<FxCache> {
    static C: std::sync::OnceLock<std::sync::Mutex<FxCache>> = std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(FxCache { map: Default::default(), order: Default::default(), bytes: 0 }))
}

/// Where `layer`'s effect maps are built (see [`effect_maps`]) and whether that is clipped to the
/// rectangle being rendered rather than the whole region.
///
/// Transparency Shapes Layer off makes the region the whole canvas whatever the layer holds: on a
/// 1920 × 103675 webtoon page each such layer's maps took about 2.4 GB, and a render held them for
/// every layer at once (60 GB with 25 stroked type layers, #1909). When its effects are local
/// ([`effects::maps_are_local`]), such a layer's maps are built over the render rectangle padded
/// by twice the effect reach instead, which gives the same pixels there. So are any layer's local
/// maps when the render's whole-region maps wouldn't fit in the effect cache ([`fx_over_budget`]).
/// Only when `rect`, what
/// the caller composites the layer over, lies within the render rectangle: a group's effect shape
/// (or anything else rendering children over a wider area) gets the whole region.
pub(crate) fn effect_region(layer: &Layer, rect: Rect, cx: &Ctx) -> (Rect, bool) {
    let m = effects::margin(layer);
    let region = effect_source_bounds(layer, cx.canvas).inflate(m).intersect(&cx.canvas.inflate(m));
    let clip = *cx.fx_clip.lock().unwrap_or_else(|e| e.into_inner());
    match clip {
        Some(c) if c.contains_rect(&rect) && (advanced::shapeless(layer) || cx.fx_over_budget) && effects::maps_are_local(layer) => {
            let part = region.intersect(&c.inflate(m.saturating_mul(2)));
            (part, part != region)
        }
        _ => (region, false),
    }
}

/// Whether the effect maps of the visible layers of `layers` (groups included), each over its
/// whole region, would take more than the effect cache budget together. A render needs all of
/// them, so when they don't fit, building one evicts the next one needed and every render builds
/// them all again, seconds even for a small rect (#8330). Counted as [`effects::FxMaps::bytes`]
/// counts the built maps ([`effects::map_count`] arrays over each region).
pub(crate) fn fx_over_budget(layers: &[Layer], canvas: Rect) -> bool {
    fx_region_bytes(layers, canvas) > effect_cache_budget() as u64
}

/// The bytes [`fx_over_budget`] compares with the budget.
pub(crate) fn fx_region_bytes(layers: &[Layer], canvas: Rect) -> u64 {
    fn bytes(layers: &[Layer], canvas: Rect, depth: u32) -> u64 {
        let mut total = 0u64;
        for l in layers.iter().filter(|l| l.visible) {
            if effects::has_effects(l) {
                let m = effects::margin(l);
                let region = effect_source_bounds(l, canvas).inflate(m).intersect(&canvas.inflate(m));
                let maps = effects::map_count(l) as u64;
                let px = u64::from(region.width()) * u64::from(region.height());
                total = total.saturating_add(maps.saturating_mul(px).saturating_mul(4));
            }
            if let LayerContent::Group(g) = &l.content
                && depth < PREPARE_DEPTH
            {
                total = total.saturating_add(bytes(&g.children, canvas, depth + 1));
            }
        }
        total
    }
    bytes(layers, canvas, 0)
}

/// Most bytes one layer's effect maps and source may take to build, whatever the cache budget.
const MAX_FX_BUILD_BYTES: u64 = if cfg!(target_pointer_width = "64") { 4 << 30 } else { 512 << 20 };

/// The layer's effect maps over its region (layer bounds grown by the effect reach, within the
/// canvas grown likewise; see [`effect_region`]), built once per layer state and shared by every
/// tile. `rect` is what the caller composites the layer over (see [`effect_region`]).
pub(crate) fn effect_maps(layer: &Layer, rect: Rect, cx: &Ctx) -> std::sync::Arc<effects::FxMaps> {
    use std::hash::{Hash, Hasher};
    let (mut region, clipped) = effect_region(layer, rect, cx);
    // Non-local effects keep their whole source region, which a few distant pixels stretch over
    // the canvas: past the cap the effects are left out instead of allocating it.
    let region_px = u64::from(region.width()) * u64::from(region.height());
    if (effects::map_count(layer) as u64 + 1).saturating_mul(region_px).saturating_mul(4) > MAX_FX_BUILD_BYTES {
        region = Rect::EMPTY;
    }
    static NO_CACHE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *NO_CACHE.get_or_init(|| std::env::var_os("PHOTOCRAFT_FX_NOCACHE").is_some()) {
        let shape = if region.is_empty() { Vec::new() } else { effect_shape(layer, region, cx) };
        return std::sync::Arc::new(effects::build_maps_prepared(layer, shape, region, &cx.light, &texture_ctx(layer, region, cx), cx.patterns));
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    cx.layer_keys.identity(layer, cx.doc).hash(&mut h);
    (region.x0, region.y0, region.x1, region.y1).hash(&mut h);
    (cx.light.angle.to_bits(), cx.light.altitude.to_bits()).hash(&mut h);
    let key = h.finish();
    // Prepared for this render (see `prepare_effects`): no global lookup, no build.
    if let Some((m, _)) = cx.fx_maps.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return m.clone();
    }
    let maps = cached_effect_maps(layer, region, key, cx);
    cx.fx_maps.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_insert((maps, clipped)).0.clone()
}

fn build_fx_slot_maps(layer: &Layer, region: Rect, key: u64, cx: &Ctx, slot: &FxSlot) -> std::sync::Arc<effects::FxMaps> {
    let shape: Vec<f32> = if region.is_empty() { Vec::new() } else { effect_shape(layer, region, cx) };
    let maps = effects::build_maps_prepared(layer, shape, region, &cx.light, &texture_ctx(layer, region, cx), cx.patterns);
    let mut pin = Vec::new();
    layer_identity(layer, &mut std::collections::hash_map::DefaultHasher::new(), &mut |t| pin.push(std::sync::Arc::downgrade(t)));
    // The maps, plus what each reserved tile address keeps allocated.
    let bytes = maps.bytes().saturating_add(pin.len().saturating_mul(std::mem::size_of::<photocraft_raster::Tile>() + 2 * std::mem::size_of::<usize>()));
    let maps = std::sync::Arc::new(maps);
    if slot.set(FxEntry { maps: maps.clone(), _pin: pin, bytes }).is_ok() {
        // Counted exactly once, by the build that filled the slot, and only while the
        // slot is still cached (eviction may have dropped it meanwhile).
        let mut c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
        if c.map.get(&key).is_some_and(|s| std::sync::Arc::ptr_eq(s, slot)) {
            c.bytes += bytes;
        }
    }
    // A concurrent build that finished first wins, so every tile shares one map.
    slot.get().map_or(maps, |e| e.maps.clone())
}

fn evict_fx_cache_over_budget(key: u64) {
    let mut c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
    let budget = effect_cache_budget();
    while c.bytes > budget && c.order.len() > 1 {
        let Some(old) = c.order.pop_front() else { break };
        if old == key {
            c.order.push_back(old);
            continue;
        }
        if let Some(s) = c.map.remove(&old) {
            c.bytes = c.bytes.saturating_sub(s.get().map_or(0, |e| e.bytes));
        }
    }
}

/// The layer's effect maps from the global cache, built on a miss.
///
/// Never waits for another thread's build (#276): this can run on a Rayon worker inside a tile,
/// and the build itself runs Rayon work. A worker waiting on that work may steal another tile of
/// the same layer; had it blocked on a once-init held further up its own stack, it would deadlock.
/// Instead a miss builds the maps here and the first finished build is kept. Renders build every
/// layer's maps before their parallel tiles (`prepare_effects`), so tiles rarely miss.
fn cached_effect_maps(layer: &Layer, region: Rect, key: u64, cx: &Ctx) -> std::sync::Arc<effects::FxMaps> {
    let slot = {
        let mut c = fx_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = c.map.get(&key) {
            s.clone()
        } else {
            let s: FxSlot = Default::default();
            c.map.insert(key, s.clone());
            c.order.push_back(key);
            s
        }
    };
    let maps = match slot.get() {
        Some(e) => e.maps.clone(),
        None => build_fx_slot_maps(layer, region, key, cx, &slot),
    };
    // Evict the oldest entries over budget (never the one just used).
    evict_fx_cache_over_budget(key);
    maps
}
