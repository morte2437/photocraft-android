//! The Layers panel's Attribute filter (#10881): lists the layers with one attribute (visible,
//! locked, empty, linked, clipped, masks, effects, advanced blending) or without it.

use photocraft_doc::{AdvancedBlending, Layer, LayerContent};
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
    /// (and groups) its positive entry doesn't.
    pub fn shows(self, l: &Layer) -> bool {
        match self {
            Attribute::Visible => l.visible,
            // Any lock counts, the Background's too.
            Attribute::Locked => l.locks.any(),
            // A pixel layer with no visible pixels; an empty group isn't listed.
            Attribute::Empty => matches!(&l.content, LayerContent::Raster(s) if !has_visible_pixels(s)),
            Attribute::Linked => l.link_group.is_some(),
            // The clipped layer, not its base.
            Attribute::Clipped => l.clipped,
            Attribute::LayerMask => l.mask.is_some(),
            // A shape layer's outline is its vector mask.
            Attribute::VectorMask => l.vector_mask.is_some() || matches!(l.content, LayerContent::Shape(_)),
            // Also a layer whose only effect is turned off, unlike the Effect filter.
            Attribute::LayerEffects => !l.effects.items.is_empty(),
            Attribute::AdvancedBlending => advanced_blending(l),
            Attribute::NotVisible => !Attribute::Visible.shows(l),
            Attribute::NotLocked => !Attribute::Locked.shows(l),
            Attribute::NotEmpty => !Attribute::Empty.shows(l),
            Attribute::NotLinked => !Attribute::Linked.shows(l),
            Attribute::NotClipped => !Attribute::Clipped.shows(l),
            Attribute::NoLayerMask => !Attribute::LayerMask.shows(l),
            Attribute::NoVectorMask => !Attribute::VectorMask.shows(l),
            Attribute::NoLayerEffects => !Attribute::LayerEffects.shows(l),
            Attribute::NoAdvancedBlending => !Attribute::AdvancedBlending.shows(l),
        }
    }
}

/// Whether `s` has a pixel with alpha > 0. The Eraser only clears alpha, so an erased pixel keeps
/// its colour bytes and `content_bounds` still counts it (as Reveal All's `visible_bounds` notes,
/// #9379). Without an alpha channel, or over an opaque default, any content counts.
fn has_visible_pixels(s: &Surface) -> bool {
    let fmt = s.format();
    if !fmt.alpha || s.default_pixel().last().is_none_or(|a| *a > 0.0) {
        return !s.content_bounds().is_empty();
    }
    let (n, bpp) = (fmt.channels(), fmt.bytes_per_pixel());
    let Some(alpha) = n.checked_sub(1).filter(|_| bpp > 0) else { return false };
    s.tiles().any(|(_, t)| t.bytes().chunks_exact(bpp).any(|px| photocraft_color::read_sample(px, fmt.sample, alpha) > 0.0))
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
