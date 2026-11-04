//! The Layers panel's Attribute filter (#10881): lists the layers with one attribute (visible,
//! locked, empty, linked, clipped, masks, effects, advanced blending) or without it.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use photocraft_color::SampleType;
use photocraft_doc::{AdvancedBlending, Document, Layer, LayerContent, LayerId};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::Surface;
use serde::{Deserialize, Serialize};

use crate::{PhotocraftApp, widgets};

/// The attributes the Attribute filter picks from, in Photoshop's order: the nine attributes,
/// then their opposites.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Attribute {
    #[default]
    Visible,
    Locked,
    Empty,
    Linked,
    Clipped,
    LayerMask,
    VectorMask,
    LayerEffects,
    AdvancedBlending,
    NotVisible,
    NotLocked,
    NotEmpty,
    NotLinked,
    NotClipped,
    NoLayerMask,
    NoVectorMask,
    NoLayerEffects,
    NoAdvancedBlending,
}

impl Attribute {
    pub const ALL: [Attribute; 18] = [
        Attribute::Visible,
        Attribute::Locked,
        Attribute::Empty,
        Attribute::Linked,
        Attribute::Clipped,
        Attribute::LayerMask,
        Attribute::VectorMask,
        Attribute::LayerEffects,
        Attribute::AdvancedBlending,
        Attribute::NotVisible,
        Attribute::NotLocked,
        Attribute::NotEmpty,
        Attribute::NotLinked,
        Attribute::NotClipped,
        Attribute::NoLayerMask,
        Attribute::NoVectorMask,
        Attribute::NoLayerEffects,
        Attribute::NoAdvancedBlending,
    ];

    /// The id `ui.set {layerFilter: {attribute}}` takes.
    pub fn id(self) -> &'static str {
        match self {
            Attribute::Visible => "visible",
            Attribute::Locked => "locked",
            Attribute::Empty => "empty",
            Attribute::Linked => "linked",
            Attribute::Clipped => "clipped",
            Attribute::LayerMask => "layerMask",
            Attribute::VectorMask => "vectorMask",
            Attribute::LayerEffects => "layerEffects",
            Attribute::AdvancedBlending => "advancedBlending",
            Attribute::NotVisible => "notVisible",
            Attribute::NotLocked => "notLocked",
            Attribute::NotEmpty => "notEmpty",
            Attribute::NotLinked => "notLinked",
            Attribute::NotClipped => "notClipped",
            Attribute::NoLayerMask => "noLayerMask",
            Attribute::NoVectorMask => "noVectorMask",
            Attribute::NoLayerEffects => "noLayerEffects",
            Attribute::NoAdvancedBlending => "noAdvancedBlending",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Attribute::Visible => tl!("Visible"),
            Attribute::Locked => tl!("Locked"),
            Attribute::Empty => tl!("Empty"),
            Attribute::Linked => tl!("Linked"),
            Attribute::Clipped => tl!("Clipped"),
            Attribute::LayerMask => tl!("Layer Mask"),
            Attribute::VectorMask => tl!("Vector Mask"),
            Attribute::LayerEffects => tl!("Layer Effects"),
            Attribute::AdvancedBlending => tl!("Advanced Blending"),
            Attribute::NotVisible => tl!("Not Visible"),
            Attribute::NotLocked => tl!("Not Locked"),
            Attribute::NotEmpty => tl!("Not Empty"),
            Attribute::NotLinked => tl!("Not Linked"),
            Attribute::NotClipped => tl!("Not Clipped"),
            Attribute::NoLayerMask => tl!("No Layer Mask"),
            Attribute::NoVectorMask => tl!("No Vector Mask"),
            Attribute::NoLayerEffects => tl!("No Layer Effects"),
            Attribute::NoAdvancedBlending => tl!("No Adv. Blending"),
        }
    }

    /// Whether `l` is listed under this attribute. Each "Not …"/"No …" entry lists the layers
    /// (and groups) its positive entry doesn't. `empty` is the document's empty layers
    /// ([`empty_layers_of`]); without it, Empty reads `l`'s pixels without knowing the canvas, so a
    /// layer with an opaque default pixel is never empty.
    pub fn shows(self, l: &Layer, empty: Option<&HashSet<LayerId>>) -> bool {
        match self {
            Attribute::Visible => l.visible,
            // Any lock counts, the Background's too.
            Attribute::Locked => l.locks.any(),
            // A pixel layer with no visible pixels; an empty group isn't listed.
            Attribute::Empty => empty.map_or_else(|| is_empty(l, None), |set| set.contains(&l.id)),
            Attribute::Linked => l.link_group.is_some(),
            // The clipped layer, not its base.
            Attribute::Clipped => l.clipped,
            Attribute::LayerMask => l.mask.is_some(),
            // A shape layer's outline is its vector mask.
            Attribute::VectorMask => l.vector_mask.is_some() || matches!(l.content, LayerContent::Shape(_)),
            // Also a layer whose only effect is turned off, unlike the Effect filter.
            Attribute::LayerEffects => !l.effects.items.is_empty(),
            Attribute::AdvancedBlending => advanced_blending(l),
            Attribute::NotVisible => !Attribute::Visible.shows(l, empty),
            Attribute::NotLocked => !Attribute::Locked.shows(l, empty),
            Attribute::NotEmpty => !Attribute::Empty.shows(l, empty),
            Attribute::NotLinked => !Attribute::Linked.shows(l, empty),
            Attribute::NotClipped => !Attribute::Clipped.shows(l, empty),
            Attribute::NoLayerMask => !Attribute::LayerMask.shows(l, empty),
            Attribute::NoVectorMask => !Attribute::VectorMask.shows(l, empty),
            Attribute::NoLayerEffects => !Attribute::LayerEffects.shows(l, empty),
            Attribute::NoAdvancedBlending => !Attribute::AdvancedBlending.shows(l, empty),
        }
    }
}

/// Whether `l` is a pixel layer with no visible pixels; an empty group isn't listed. Stored pixels
/// past the canvas count (they are the layer's, Reveal All brings them back); `canvas` only bounds
/// the check for a layer whose default pixel is opaque (see [`has_visible_pixels`]).
fn is_empty(l: &Layer, canvas: Option<Rect>) -> bool {
    matches!(&l.content, LayerContent::Raster(s) if !has_visible_pixels(s, canvas))
}

/// Whether `s` has a pixel with alpha > 0. The Eraser only clears alpha, so an erased pixel keeps
/// its colour bytes and `content_bounds` still counts it (as Reveal All's `visible_bounds` notes,
/// #9379). A surface without an alpha channel covers everything. With an opaque default pixel,
/// every part of `canvas` without a stored tile shows it, so only stored tiles that cover the
/// canvas and are clear on it make the layer empty (pixels outside the canvas are left out there,
/// as the default reaches beyond it anyway); without a `canvas` such a layer always counts.
fn has_visible_pixels(s: &Surface, canvas: Option<Rect>) -> bool {
    let fmt = s.format();
    if !fmt.alpha {
        return true;
    }
    let clip = match (s.default_pixel().last().is_none_or(|a| *a > 0.0), canvas) {
        (false, _) => None,
        (true, Some(canvas)) if covers(s, canvas) => Some(canvas),
        (true, _) => return true,
    };
    let (n, bpp, size) = (fmt.channels(), fmt.bytes_per_pixel(), fmt.sample.bytes());
    let Some(at) = n.checked_sub(1).and_then(|i| i.checked_mul(size)).filter(|_| bpp > 0) else { return false };
    // Integer alpha is above 0 where any of its bytes is, so a block of pixels is OR-ed together
    // (which vectorises) before looking for one; only float alpha needs decoding.
    let visible = |block: &[u8]| match fmt.sample {
        SampleType::F32 => block.chunks_exact(bpp).any(|px| px.get(at..at + size).is_some_and(|a| a.try_into().is_ok_and(|b| f32::from_ne_bytes(b) > 0.0))),
        SampleType::U8 | SampleType::U16 => {
            block.chunks_exact(bpp).fold(0u8, |acc, px| px.get(at..at + size).map_or(acc, |a| a.iter().fold(acc, |acc, b| acc | b))) != 0
        }
    };
    let tile_visible = |c: &TileCoord, t: &Arc<photocraft_raster::Tile>| {
        let bytes = t.bytes();
        let r = c.rect();
        let Some(clip) = clip.filter(|clip| !clip.contains_rect(&r)) else { return bytes.chunks(bpp.saturating_mul(1024)).any(visible) };
        // Only the part of the tile on the canvas, row by row.
        let on = r.intersect(&clip);
        let row = (TILE_SIZE as usize).saturating_mul(bpp);
        let (x0, x1) = ((on.x0 - r.x0) as usize * bpp, (on.x1 - r.x0) as usize * bpp);
        (on.y0 - r.y0..on.y1 - r.y0).any(|y| bytes.get(y as usize * row + x0..y as usize * row + x1).is_some_and(visible))
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        let tiles: Vec<_> = s.tiles().filter(|(c, _)| clip.is_none_or(|clip| !c.rect().intersect(&clip).is_empty())).collect();
        tiles.par_iter().any(|(c, t)| tile_visible(c, t))
    }
    #[cfg(target_arch = "wasm32")]
    s.tiles().filter(|(c, _)| clip.is_none_or(|clip| !c.rect().intersect(&clip).is_empty())).any(|(c, t)| tile_visible(c, t))
}

/// Whether `s` stores a tile for every part of `canvas`.
fn covers(s: &Surface, canvas: Rect) -> bool {
    if canvas.is_empty() {
        return true;
    }
    let (first, last) = (TileCoord::containing(canvas.x0, canvas.y0), TileCoord::containing(canvas.x1 - 1, canvas.y1 - 1));
    let needed = (i64::from(last.tx) - i64::from(first.tx) + 1) * (i64::from(last.ty) - i64::from(first.ty) + 1);
    let stored = s.tiles().filter(|(c, _)| (first.tx..=last.tx).contains(&c.tx) && (first.ty..=last.ty).contains(&c.ty)).count();
    i64::try_from(stored).is_ok_and(|stored| stored == needed)
}

/// The empty pixel layers of `doc`.
pub fn empty_layers_of(doc: &Document) -> HashSet<LayerId> {
    let canvas = Some(doc.bounds());
    doc.walk().into_iter().filter(|(_, _, l)| is_empty(l, canvas)).map(|(_, _, l)| l.id).collect()
}

/// The document [`empty_layers`] last read and what it found. A `Weak` keeps the document's
/// allocation from being reused while it is held, and makes `Arc::make_mut` copy the document on
/// its next edit even when nothing else holds it, so the same allocation always means the same
/// layers and pixels.
static EMPTY: Mutex<Option<EmptyCache>> = Mutex::new(None);

type EmptyCache = (Weak<Document>, Arc<HashSet<LayerId>>);

/// The empty layers of `app`'s active document while Empty or Not Empty is picked, found once per
/// document: a layer erased to nothing has every pixel read, too much for every repaint of the
/// Layers panel. Keyed by the document itself rather than its revision, which also moves when only
/// the selection does.
pub fn empty_layers(app: &PhotocraftApp, attribute: Attribute) -> Option<Arc<HashSet<LayerId>>> {
    if !matches!(attribute, Attribute::Empty | Attribute::NotEmpty) {
        return None;
    }
    let d = app.session.active()?;
    let mut cache = EMPTY.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((doc, set)) = cache.as_ref()
        && std::ptr::eq(doc.as_ptr(), Arc::as_ptr(&d.doc))
    {
        return Some(set.clone());
    }
    let set = Arc::new(empty_layers_of(&d.doc));
    *cache = Some((Arc::downgrade(&d.doc), set.clone()));
    Some(set)
}

/// Whether any Advanced Blending option is away from its default: a channel left out, Knockout,
/// the four "as Group" / "Transparency Shapes" / "Hides Effects" switches, or a Blend If range.
/// Fill opacity alone doesn't count.
fn advanced_blending(l: &Layer) -> bool {
    l.excluded_channels != 0 || l.advanced != AdvancedBlending::default() || !l.blend_if.is_default()
}

/// `layerFilter.attribute`: the attribute named by `v`, or an error listing the ids.
pub fn parse(v: &serde_json::Value) -> Result<Attribute, String> {
    let ids = || Attribute::ALL.map(Attribute::id).join(", ");
    let name = v.as_str().ok_or_else(|| format!("layerFilter.attribute must be one of {}", ids()))?;
    Attribute::ALL.into_iter().find(|a| a.id() == name).ok_or_else(|| format!("layerFilter.attribute: unknown attribute `{name}` (attributes: {})", ids()))
}

/// The Attribute filter's controls: the attribute dropdown.
pub fn controls(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let attributes: Vec<(Attribute, &str)> = Attribute::ALL.into_iter().map(|a| (a, a.label())).collect();
    let width = ui.available_width().max(96.0);
    widgets::dropdown(ui, "layer-filter-attribute", tl!("Attribute"), &mut app.ui.layer_filter_attribute, &attributes, width);
}
