//! Planner: walks the layer tree into a linear list of GPU passes over abstract buffer slots.
//!
//! The structure mirrors `photocraft_compose::composite_stack` / `composite_layer` /
//! `composite_atop` one to one, so both backends share Photoshop semantics (pass-through vs
//! isolated groups, clipping, masks, opacity × fill, adjustments, layer effects). Every pass
//! reads slots and writes a fresh slot; slots are recycled as soon as nothing refers to them.
//!
//! Layer effects follow `photocraft_compose::effects::composite_with_effects`: the layer's
//! effect maps (shadow / glow / satin / bevel coverage, stroke bands) are built once per layer
//! state by [`crate::fx`] and sampled here; exterior effects paint into a copy of the backdrop,
//! the layer at fill opacity takes the interior effects, and the two merge with the layer's mode
//! (or the layer merges first and the effects paint over it: `effects::premerges_layer`) and
//! opacity. Effect passes are clipped to the layer's effect region and the result is copied
//! back into the backdrop in place, so a small text layer costs only its own pixels.

use photocraft_color::BlendMode;
use photocraft_compose::adjust::{self, Transfer};
use photocraft_compose::effects::has_effects;
use photocraft_doc::adjust::{ToneSpace, lut3d_len};
use photocraft_doc::{Adjustment, Document, Effect, Fill, FxPaint, GlobalLight, Glow, Gradient, Layer, LayerContent, LayerId, Pattern, StrokePosition};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

use crate::bounds;

mod lower;
mod planner;
mod types;

#[cfg(test)]
use lower::check_gradient_stops;
pub use lower::{adjustment_on_gpu, adjustment_program, mode_index, stroke_widths};
use planner::Planner;
#[cfg(test)]
use planner::blend_if_params;
pub use planner::{
    F_ADD_DIFF, F_ATOP, F_BLEND_IF, F_CHANNELS, F_FIRST, F_GATE, F_HDR, F_KNOCKOUT, F_LAB, F_LAYER_SHAPE, F_NO_LAYER, F_OUTLINE, F_QUANT, F_REL, F_STROKE_OUT,
    F_TEXT_GAMMA, F_VECTOR, INIT_ALPHA, INIT_COVER, INIT_INSIDE, INIT_OPAQUE, INIT_OUTLINE, INIT_RELATIVE, INIT_SCALE, INIT_VSTROKE, SHAPE_MAP,
};
pub use types::{Checkpoint, DocCtx, FxLayer, Kernel, MapRef, MaskUse, Pass, Role, SurfaceRef, TexUse, Unsupported};

/// Index of a chunk-sized accumulator texture.
pub type Slot = u32;

#[derive(Debug)]
pub struct Plan<'a> {
    pub passes: Vec<Pass<'a>>,
    pub slots: u32,
    pub root: Slot,
    /// Effect layers in dependency order (a group's inner effect layers come first).
    pub fx: Vec<FxLayer<'a>>,
    /// Complete root stack units, where only the accumulated backdrop remains live.
    pub checkpoints: Vec<Checkpoint>,
}

/// Build the pass list for `doc`.
pub fn plan(doc: &Document) -> Result<Plan<'_>, Unsupported> {
    if doc.mode == photocraft_color::ColorMode::Multichannel {
        return Err(Unsupported("Multichannel inks (printed on the CPU)".into()));
    }
    let mut p = Planner::new(DocCtx::of(doc));
    let root = p.clear();
    // Start at the topmost layer that hides everything beneath it (an opaque fill layer): the
    // layers below can't change the result, so they cost no passes or uploads.
    let start = doc.layers.iter().rposition(|l| photocraft_compose::occludes_below(l, doc.mode)).unwrap_or(0);
    let root = p.stack_inner(doc.layers.get(start..).unwrap_or(&doc.layers), root, Some(start))?;
    Ok(p.finish(root))
}

#[cfg(test)]
mod tests;
