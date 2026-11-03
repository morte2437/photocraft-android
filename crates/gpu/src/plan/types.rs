use super::{BlendMode, Document, GlobalLight, Layer, LayerId, Pattern, Rect, Slot, Surface, Transfer};

/// Pipeline (fragment entry point) of a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kernel {
    /// Transparent fill (no draw).
    Clear,
    Content,
    Mask,
    Blend,
    /// `Content` then `Blend` in one pass: a plain layer's pixels blended straight onto A.
    LayerBlend,
    /// A run of consecutive `LayerBlend` passes drawn as one (never planned: the executor draws
    /// it in their place, see `batch_runs`).
    LayerBatch,
    /// `LayerBatch` of a run whose layers all blend Normal without text gamma, outside Lab
    /// documents (never planned: the executor picks it, see `batch_kernel`).
    LayerBatchNormal,
    Atop,
    Adjust,
    AdjMix,
    Lerp,
    /// Effect chain start: the layer at fill opacity, or an opaque copy of a clipping base.
    FxInit,
    /// Paint an effect through a coverage map (or the layer's alpha) into A.
    FxPaint,
    /// Layer + interior effects over the exterior result, then layer opacity against the backdrop.
    FxMerge,
    /// Outside strokes: accumulate one stroke's share (`kind` 0, premultiplied colour) or its
    /// coverage (`kind` 1).
    FxStroke,
    /// Outside strokes: resolve the accumulated strokes over the exterior result.
    FxStrokeEnd,
    /// Copy slot A into `dst` (an existing slot) over the pass clip (no draw).
    CopyRect,
    /// Copy all of slot A into the fresh slot `dst` (no draw).
    CopyFull,
    // Effect-map kernels (single-channel targets over an effect region, see `fx`).
    MShift,
    MDilate,
    MBlur,
    MGlow,
    MFinish,
    MBevelH,
    MBevelShade,
    MStroke,
    MBevelTex,
}

impl Kernel {
    /// Fragment entry point in `compose.wgsl` (`None` for copies and clears).
    pub fn entry(self) -> Option<&'static str> {
        Some(match self {
            Kernel::Clear | Kernel::CopyRect | Kernel::CopyFull => return None,
            Kernel::Content => "fs_content",
            Kernel::Mask => "fs_mask",
            Kernel::Blend => "fs_blend",
            Kernel::LayerBlend => "fs_layerblend",
            Kernel::LayerBatch => "fs_layerbatch",
            Kernel::LayerBatchNormal => "fs_layerbatch_normal",
            Kernel::Atop => "fs_atop",
            Kernel::Adjust => "fs_adjust",
            Kernel::AdjMix => "fs_adjmix",
            Kernel::Lerp => "fs_lerp",
            Kernel::FxInit => "fs_fxinit",
            Kernel::FxPaint => "fs_fxpaint",
            Kernel::FxMerge => "fs_fxmerge",
            Kernel::FxStroke => "fs_fxstroke",
            Kernel::FxStrokeEnd => "fs_fxstrokeend",
            Kernel::MShift => "fs_mshift",
            Kernel::MDilate => "fs_mdilate",
            Kernel::MBlur => "fs_mblur",
            Kernel::MGlow => "fs_mglow",
            Kernel::MFinish => "fs_mfinish",
            Kernel::MBevelH => "fs_mbevelh",
            Kernel::MBevelShade => "fs_mbevelshade",
            Kernel::MStroke => "fs_mstroke",
            Kernel::MBevelTex => "fs_mbeveltex",
        })
    }

    /// Kernels that write effect maps (R32F / R16F targets) rather than RGBA accumulators.
    pub fn is_map(self) -> bool {
        matches!(
            self,
            Kernel::MShift
                | Kernel::MDilate
                | Kernel::MBlur
                | Kernel::MGlow
                | Kernel::MFinish
                | Kernel::MBevelH
                | Kernel::MBevelShade
                | Kernel::MStroke
                | Kernel::MBevelTex
        )
    }

    /// Every kernel with a pipeline.
    pub const DRAWN: [Kernel; 24] = [
        Kernel::Content,
        Kernel::Mask,
        Kernel::Blend,
        Kernel::LayerBlend,
        Kernel::LayerBatch,
        Kernel::LayerBatchNormal,
        Kernel::Atop,
        Kernel::Adjust,
        Kernel::AdjMix,
        Kernel::Lerp,
        Kernel::FxInit,
        Kernel::FxPaint,
        Kernel::FxMerge,
        Kernel::FxStroke,
        Kernel::FxStrokeEnd,
        Kernel::MShift,
        Kernel::MDilate,
        Kernel::MBlur,
        Kernel::MGlow,
        Kernel::MFinish,
        Kernel::MBevelH,
        Kernel::MBevelShade,
        Kernel::MStroke,
        Kernel::MBevelTex,
    ];
}

/// Which resident texture a pass samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Content,
    Mask,
    /// A stroked shape's stroke alone (its fill uses `Content`).
    Stroke,
}

/// A surface the pass needs on the GPU.
#[derive(Clone, Debug)]
pub struct TexUse<'a> {
    pub layer: LayerId,
    pub role: Role,
    pub surface: SurfaceRef<'a>,
}

/// An effect map sampled by a pass: map `map` of enabled effect `item` of `plan.fx[fx]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapRef {
    pub fx: usize,
    pub item: usize,
    pub map: usize,
}

#[derive(Clone, Debug)]
pub struct Pass<'a> {
    pub kernel: Kernel,
    pub dst: Slot,
    pub a: Option<Slot>,
    pub b: Option<Slot>,
    pub c: Option<Slot>,
    /// A fourth slot, bound in place of `tex` (passes without layer pixels).
    pub d: Option<Slot>,
    pub mode: BlendMode,
    pub opacity: f32,
    /// The layer's Fill where it isn't folded into `opacity`: the final blend of a plain layer
    /// (`Kernel::Blend`), so the special eight can apply it inside the blend. 1 elsewhere.
    pub fill: f32,
    /// Layer pixels (raster / text / shape / smart cache / fill cache).
    pub tex: Option<TexUse<'a>>,
    /// Colour outside `tex` (the surface's default pixel), the solid fill colour, or the effect
    /// colour.
    pub color: [f32; 4],
    pub mask: Option<MaskUse<'a>>,
    /// Adjustment kind (see `adjust` in compose.wgsl) and parameters; the coverage kind of an
    /// effect paint.
    pub adjust_kind: i32,
    pub params: [[f32; 4]; 4],
    pub extra: [f32; 4],
    /// Extra shader flags (`F_KNOCKOUT`, …).
    pub flags: u32,
    /// 4096-entry LUT rows (Levels / Curves / Gradient map / gradient stops).
    pub lut: Option<Vec<[f32; 4096]>>,
    pub gradient: bool,
    /// Effect map sampled by `FxPaint`.
    pub map: Option<MapRef>,
    /// Pattern painted by `FxPaint`.
    pub pattern: Option<&'a Pattern>,
    /// Only pixels inside this rect (document coordinates) are computed; the pass is skipped for
    /// chunks it misses. Consumers of the slot must be clipped alike.
    pub clip: Option<Rect>,
}

/// Pixels a pass samples: a surface of the document, or one derived from it on the CPU (a
/// combined pixel × vector mask, a shape's fill or stroke alone).
#[derive(Clone, Debug)]
pub enum SurfaceRef<'a> {
    Doc(&'a Surface),
    Derived(std::sync::Arc<Surface>),
}

impl SurfaceRef<'_> {
    pub fn get(&self) -> &Surface {
        match self {
            SurfaceRef::Doc(s) => s,
            SurfaceRef::Derived(s) => s,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MaskUse<'a> {
    pub layer: LayerId,
    pub surface: SurfaceRef<'a>,
    pub density: f32,
    pub default: f32,
}

impl<'a> Pass<'a> {
    pub(crate) fn new(kernel: Kernel, dst: Slot) -> Self {
        Pass {
            kernel,
            dst,
            a: None,
            b: None,
            c: None,
            d: None,
            mode: BlendMode::Normal,
            opacity: 1.0,
            fill: 1.0,
            tex: None,
            color: [0.0; 4],
            mask: None,
            adjust_kind: 0,
            params: [[0.0; 4]; 4],
            extra: [0.0; 4],
            flags: 0,
            lut: None,
            gradient: false,
            map: None,
            pattern: None,
            clip: None,
        }
    }
}

/// Why a document can't be composited on the GPU (the caller falls back to the CPU compositor).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported(pub String);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GPU compositor: {}", self.0)
    }
}

impl std::error::Error for Unsupported {}

/// A layer whose effects the plan renders: its maps are built (or reused) before the chunks run.
#[derive(Clone, Copy, Debug)]
pub struct FxLayer<'a> {
    pub layer: &'a Layer,
    /// Region the maps cover (`compose::effect_maps`).
    pub region: Rect,
    /// Layer bounds (gradients and linked patterns are laid out in them).
    pub bounds: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// First root layer included after occlusion pruning.
    pub layer_start: usize,
    /// Root layers below this boundary (including the complete clipping unit).
    pub layer_end: usize,
    /// Resume at this pass with `slot` restored.
    pub pass_end: usize,
    pub slot: Slot,
}

/// Document-level inputs of a plan.
#[derive(Clone, Copy)]
pub struct DocCtx<'a> {
    pub canvas: Rect,
    pub transfer: Transfer,
    pub light: GlobalLight,
    pub patterns: &'a [Pattern],
    /// Colour mode (channel restrictions name its channels).
    pub mode: photocraft_color::ColorMode,
    /// Sample depth (adjustment results are rounded to it).
    pub depth: photocraft_color::SampleType,
    /// The document (effect geometry the CPU computes, e.g. gradient stroke frames).
    pub doc: &'a Document,
}

impl<'a> DocCtx<'a> {
    pub fn of(doc: &'a Document) -> Self {
        DocCtx {
            canvas: doc.bounds(),
            transfer: Transfer::for_document(doc.mode, doc.depth),
            light: doc.global_light,
            patterns: &doc.patterns,
            mode: doc.mode,
            depth: doc.depth,
            doc,
        }
    }
}
