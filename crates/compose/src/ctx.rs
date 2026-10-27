use super::content::empty_in;
use super::fx_cache::{effect_maps, fx_over_budget, layer_identity};
use super::*;

type VectorMaskSlot = std::sync::Arc<std::sync::OnceLock<photocraft_vector::CompiledVectorMask>>;

#[derive(Default)]
pub(crate) struct RenderVectorMasks {
    /// Compiled masks by content. Callers pass temporary clones (see `advanced.rs`), so an
    /// address cannot identify a mask: a dropped clone's address is reused by the next one.
    masks: std::sync::Mutex<Vec<(photocraft_doc::VectorMask, VectorMaskSlot)>>,
}

impl RenderVectorMasks {
    pub(crate) fn values(&self, mask: &photocraft_doc::VectorMask, rect: Rect) -> Vec<f32> {
        let slot = {
            let mut masks = self.masks.lock().unwrap_or_else(|e| e.into_inner());
            match masks.iter().find(|(m, _)| m == mask) {
                Some((_, slot)) => slot.clone(),
                None => {
                    let slot = VectorMaskSlot::default();
                    masks.push((mask.clone(), slot.clone()));
                    slot
                }
            }
        };
        // Compilation is sequential. Rendering can enter Rayon and must happen after
        // initialization, so waiting tiles cannot re-enter a slot still being built.
        let compiled = slot.get_or_init(|| photocraft_vector::CompiledVectorMask::new(mask));
        compiled.render(rect)
    }
}

/// Rendering context shared down the tree.
pub(crate) struct Ctx<'a> {
    /// Document canvas: fill layers and gradients are laid out relative to it, never to the render rect.
    pub(crate) canvas: Rect,
    /// Tone transfer used by adjustments that work in linear light.
    pub(crate) transfer: adjust::Transfer,
    /// Global light for layer effects.
    pub(crate) light: photocraft_doc::GlobalLight,
    /// Prepared document patterns, shared across all tiles and bands of this render call.
    pub(crate) patterns: &'a pattern::PreparedPatterns<'a>,
    /// The document's colour mode (channel restrictions name its channels).
    pub(crate) mode: photocraft_color::ColorMode,
    pub(crate) depth: photocraft_color::SampleType,
    /// At most 1 MiB of tone tables, initialized once and shared by this render's tiles and bands.
    pub(crate) tone_luts: Vec<(&'a photocraft_doc::Adjustment, std::sync::OnceLock<[Vec<f32>; 4]>)>,
    pub(crate) vector_masks: RenderVectorMasks,
    /// Effect maps used by this render, by cache key: built before the parallel tiles (see
    /// [`prepare_effects`]) and held for the whole render (or band, for maps clipped to it: the
    /// `bool`), so eviction can't force a rebuild.
    pub(crate) fx_maps: std::sync::Mutex<std::collections::HashMap<u64, (std::sync::Arc<effects::FxMaps>, bool)>>,
    /// The rectangle being rendered (a band of [`render_bands`]); see [`effect_region`].
    pub(crate) fx_clip: std::sync::Mutex<Option<Rect>>,
    /// The whole-region effect maps of this render's layers wouldn't fit in the effect cache
    /// together ([`fx_over_budget`]), so local ones are clipped to the render rect (#8330).
    pub(crate) fx_over_budget: bool,
    /// The document being rendered (deep knockouts reveal its Background); `None` for a lone layer.
    pub(crate) doc: Option<&'a Document>,
    /// Effect-map identities of this render's document layers (see [`LayerKeys`]).
    pub(crate) layer_keys: LayerKeys,
}

/// Memo of [`layer_identity`] for the layers of the document being rendered. The identity depends
/// on the layer alone, not on the tile, so it is computed once per layer per render instead of
/// once per tile. Only layers that sit in the document tree are memoised by address: the document
/// is borrowed for the whole render, so no other object can live at such an address. Temporary
/// copies (`advanced::effects_source`) are never in the tree and always take the full path.
#[derive(Default)]
pub(crate) struct LayerKeys {
    /// Addresses of every layer in the document tree, collected on first use.
    in_doc: std::sync::OnceLock<std::collections::HashSet<usize>>,
    ids: std::sync::Mutex<std::collections::HashMap<usize, u64>>,
    /// Identities computed (tests count them).
    #[cfg(test)]
    pub(crate) built: std::sync::atomic::AtomicUsize,
}

impl LayerKeys {
    pub(crate) fn identity(&self, layer: &Layer, doc: Option<&Document>) -> u64 {
        let addr = std::ptr::from_ref(layer) as usize;
        let in_doc = doc.is_some_and(|d| self.in_doc.get_or_init(|| doc_layer_addresses(d)).contains(&addr));
        if in_doc {
            let mut ids = self.ids.lock().unwrap_or_else(|e| e.into_inner());
            // Computing an identity only hashes immutable layer data; it does not run Rayon
            // work or re-enter this memo. Keep the miss and insertion atomic across callers.
            *ids.entry(addr).or_insert_with(|| self.compute(layer))
        } else {
            self.compute(layer)
        }
    }

    fn compute(&self, layer: &Layer) -> u64 {
        use std::hash::Hasher;
        #[cfg(test)]
        self.built.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        layer_identity(layer, &mut h, &mut |_| {});
        h.finish()
    }
}

/// Addresses of all layers of `doc` (groups' children included).
fn doc_layer_addresses(doc: &Document) -> std::collections::HashSet<usize> {
    let mut out = std::collections::HashSet::new();
    let mut todo: Vec<&Layer> = doc.layers.iter().collect();
    while let Some(l) = todo.pop() {
        out.insert(std::ptr::from_ref(l) as usize);
        if let LayerContent::Group(g) = &l.content {
            todo.extend(g.children.iter());
        }
    }
    out
}

impl<'a> Ctx<'a> {
    pub(crate) fn for_doc(doc: &'a Document, patterns: &'a pattern::PreparedPatterns<'a>) -> Self {
        Self {
            doc: Some(doc),
            canvas: doc.bounds(),
            transfer: adjust::Transfer::for_document(doc.mode, doc.depth),
            light: doc.global_light,
            patterns,
            mode: doc.mode,
            depth: doc.depth,
            vector_masks: RenderVectorMasks::default(),
            fx_maps: Default::default(),
            fx_clip: Default::default(),
            fx_over_budget: fx_over_budget(&doc.layers, doc.bounds()),
            layer_keys: Default::default(),
            tone_luts: prepare_tones(&doc.layers, 0),
        }
    }

    /// Start rendering `rect`: effect maps clipped to the previous rectangle are let go.
    pub(crate) fn set_fx_clip(&self, rect: Rect) {
        let mut clip = self.fx_clip.lock().unwrap_or_else(|e| e.into_inner());
        if *clip != Some(rect) {
            *clip = Some(rect);
            self.fx_maps.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, (_, clipped)| !*clipped);
        }
    }

    pub(crate) fn apply_adjustment(&self, adj: &photocraft_doc::Adjustment, buf: &mut Buffer) {
        if let photocraft_doc::Adjustment::Levels { space, .. } | photocraft_doc::Adjustment::Curves { space, .. } = adj
            && !matches!(adj, photocraft_doc::Adjustment::Levels { space: photocraft_doc::adjust::ToneSpace::Rgb, .. } if self.depth == photocraft_color::SampleType::F32)
            && let Some((_, luts)) = self.tone_luts.iter().find(|(a, _)| std::ptr::eq(*a, adj))
        {
            let identity = adjust::float_tone_identity(adj, Some(self.depth));
            if identity == [true; 3] {
                return;
            }
            let extrapolate = adjust::float_tone_extrapolates(adj, Some(self.depth), self.mode);
            adjust::apply_tone(buf, *space, luts.get_or_init(|| adjust::tone_luts_depth(adj, Some(self.depth))), identity, extrapolate);
        } else {
            adjust::apply_in(adj, buf, self.transfer, Some(self.depth), self.mode);
        }
    }
}

pub(crate) fn prepare_tones(layers: &[Layer], depth: u32) -> Vec<(&photocraft_doc::Adjustment, std::sync::OnceLock<[Vec<f32>; 4]>)> {
    let mut out = Vec::new();
    for layer in layers.iter().filter(|l| l.visible) {
        if out.len() == 16 {
            break;
        }
        match &layer.content {
            LayerContent::Adjustment(a @ (photocraft_doc::Adjustment::Levels { .. } | photocraft_doc::Adjustment::Curves { .. })) => {
                out.push((a, Default::default()))
            }
            LayerContent::Group(g) if depth < PREPARE_DEPTH => out.extend(prepare_tones(&g.children, depth + 1).into_iter().take(16 - out.len())),
            _ => {}
        }
    }
    out
}

/// Deepest group nesting preparation walks (deeper layers prepare on demand).
pub(crate) const PREPARE_DEPTH: u32 = 64;

/// Build the effect maps of every visible layer in `layers` (groups included) that can reach
/// `rect` before tiles render in parallel, layers concurrently on native targets, each build
/// inside `setting` (the tiles' thread-local colour setting). Tiles then only read finished maps:
/// none builds one while others need it (#276), and none duplicates another's build. Builds never
/// wait on each other (`cached_effect_maps`), so building them in parallel can't deadlock.
pub(crate) fn prepare_effects(layers: &[Layer], rect: Rect, cx: &Ctx, setting: impl Fn(&mut dyn FnMut()) + Sync) {
    let mut todo = Vec::new();
    effect_layers(layers, rect, 0, &mut todo);
    let build = |l: &std::borrow::Cow<Layer>| {
        setting(&mut || {
            let _ = effect_maps(l, rect, cx);
        });
    };
    #[cfg(not(target_arch = "wasm32"))]
    if todo.len() > 1 {
        use rayon::prelude::*;
        todo.par_iter().for_each(build);
        return;
    }
    todo.iter().for_each(build);
}

/// The visible layers of `layers` (groups included) with effects that can reach `rect`.
pub(crate) fn effect_layers<'l>(layers: &'l [Layer], rect: Rect, depth: u32, out: &mut Vec<std::borrow::Cow<'l, Layer>>) {
    let mut base_visible = true;
    for l in layers {
        if !l.clipped {
            base_visible = l.visible;
        }
        // A clipping group is drawn only when its base is.
        if !l.visible || !base_visible {
            continue;
        }
        if effects::has_effects(l) && !empty_in(l, rect) {
            // A mask that hides the effects is left out of the shape they are built from.
            out.push(advanced::effects_source(l));
        }
        if let LayerContent::Group(g) = &l.content
            && depth < PREPARE_DEPTH
        {
            effect_layers(&g.children, rect, depth + 1, out);
        }
    }
}
