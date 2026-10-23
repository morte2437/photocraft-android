//! One-colour areas of a composite, known without compositing them pixel by pixel.
//!
//! A new document is a canvas of one shared white tile (or no tiles at all), and a sparse one is
//! mostly that. When every layer composites pixel by pixel from its own pixels (no effects,
//! feathered masks, dissolve, dithering or layer kinds laid out on the canvas), an area over which
//! every surface holds one value composites to one colour: one pixel of it is composited and
//! stands for the rest. The reduced canvas of a gigapixel document (#4017) then costs what its
//! painted tiles cost, not a composite of every document pixel.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use photocraft_color::blend::BlendMode;
use photocraft_doc::{Adjustment, Document, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

/// The surfaces a document composites from, when its composite at each pixel depends only on
/// their values at that pixel.
pub(crate) struct UniformAreas<'a> {
    surfaces: Vec<&'a Surface>,
    /// One value per tile (by address), or `None` when its pixels differ: each tile is scanned once.
    tile_values: Mutex<HashMap<usize, Option<Box<[u8]>>>>,
    /// The composite colour of each combination of surface values met so far.
    colors: Mutex<HashMap<Vec<u8>, [f32; 4]>>,
}

impl<'a> UniformAreas<'a> {
    /// `None` when some layer's composite depends on more than its pixels at the same place.
    pub(crate) fn new(doc: &'a Document) -> Option<Self> {
        if doc.mode == photocraft_color::ColorMode::Multichannel {
            return None;
        }
        let mut surfaces = Vec::new();
        // Hidden layers are not composited: one with an effect must not rule the shortcut out.
        for l in crate::visible_layers(&doc.layers) {
            let effects = l.effects.enabled && !l.effects.items.is_empty();
            if effects || l.vector_mask.is_some() || l.video.is_some() || l.blend == BlendMode::Dissolve {
                return None;
            }
            if let Some(m) = &l.mask {
                if m.feather != 0.0 {
                    return None;
                }
                surfaces.push(&m.surface);
            }
            match &l.content {
                LayerContent::Raster(s) => surfaces.push(s),
                LayerContent::Group(g) if g.artboard.is_none() => {}
                LayerContent::Adjustment(a) if per_pixel(a) => {}
                _ => return None,
            }
        }
        Some(Self { surfaces, tile_values: Mutex::default(), colors: Mutex::default() })
    }

    /// The colour every pixel of `r` composites to, when every surface holds one value over it;
    /// `composite` renders one pixel the first time a combination of values is met.
    pub(crate) fn color(&self, r: Rect, composite: impl FnOnce(Rect) -> Option<[f32; 4]>) -> Option<[f32; 4]> {
        if r.is_empty() {
            return None;
        }
        let mut key = Vec::new();
        for s in &self.surfaces {
            key.extend_from_slice(&self.value_over(s, r)?);
        }
        if let Some(c) = self.colors.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
            return Some(*c);
        }
        let c = composite(Rect::from_xywh(r.x0, r.y0, 1, 1))?;
        self.colors.lock().unwrap_or_else(PoisonError::into_inner).insert(key, c);
        Some(c)
    }

    /// The one encoded value `s` holds over `r`, if it holds one.
    fn value_over(&self, s: &Surface, r: Rect) -> Option<Box<[u8]>> {
        let mut found: Option<Box<[u8]>> = None;
        for c in r.tiles() {
            let v = match s.tile(c) {
                None => Some(Box::from(s.default_bytes())),
                Some(t) => {
                    let id = std::sync::Arc::as_ptr(t) as usize;
                    let cached = self.tile_values.lock().unwrap_or_else(PoisonError::into_inner).get(&id).cloned();
                    match cached {
                        Some(v) => v,
                        None => {
                            let bpp = s.default_bytes().len().max(1);
                            let mut px = t.bytes().chunks_exact(bpp);
                            let v = px.next().filter(|p0| px.all(|p| p == *p0)).map(Box::from);
                            self.tile_values.lock().unwrap_or_else(PoisonError::into_inner).insert(id, v.clone());
                            v
                        }
                    }
                }
            }?;
            if found.as_ref().is_some_and(|f| *f != v) {
                return None;
            }
            found = Some(v);
        }
        found
    }
}

/// Whether adjustment `a` maps each pixel on its own (dithered ones vary with the position).
fn per_pixel(a: &Adjustment) -> bool {
    !matches!(a, Adjustment::GradientMap { dither: true, .. } | Adjustment::ColorLookup { dither: true, .. })
}
