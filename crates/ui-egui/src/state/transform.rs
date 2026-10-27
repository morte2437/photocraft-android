use super::*;

/// Edit › Transform's mode: what a handle drag does with no modifier keys held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransformMode {
    /// Free Transform and Scale: handles scale, drag outside rotates.
    #[default]
    Free,
    /// Edit › Transform › Rotate: handle drags rotate about the reference point (#3941).
    Rotate,
    /// Edge and corner handles skew: each moves along one axis.
    Skew,
    /// Corner handles move freely, one at a time; nothing else moves them (no rotating, no
    /// edges) and nothing snaps.
    Distort,
    /// Corner handles move in pairs, mirrored (one-point perspective).
    Perspective,
}

impl TransformMode {
    /// The mode an `edit.transform.*` / `edit.freeTransform` menu id starts.
    pub fn for_command(id: &str) -> Self {
        match id {
            "edit.transform.rotate" => Self::Rotate,
            "edit.transform.skew" => Self::Skew,
            "edit.transform.distort" => Self::Distort,
            "edit.transform.perspective" => Self::Perspective,
            _ => Self::Free,
        }
    }
}

/// Free Transform in progress: the source frame `rect` and where its corners currently are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransformSession {
    pub session: u64,
    pub layer: u64,
    pub rect: [f64; 4],
    /// Corners clockwise from top-left (document px).
    pub quad: [[f64; 2]; 4],
    /// Reference point (rotation / ⌥-scale centre).
    pub pivot: [f64; 2],
    pub interpolation: String,
    /// Warp mode (Edit › Transform › Warp): the warp being edited over `rect`, in document px.
    #[serde(default)]
    pub warp: Option<photocraft_geom::warp::Warp>,
    /// Select › Transform Selection: the box transforms the selection outline, not pixels.
    #[serde(default)]
    pub selection: bool,
    /// `edit.transform`'s `"target"` when the box moves an unlinked layer mask, an alpha channel or
    /// the Quick Mask by itself (`None`: the layer, with its linked masks).
    #[serde(default)]
    pub target: Option<serde_json::Value>,
    /// Free Transform Path: `path.transform`'s `name` (and `layer`) when the box moves a
    /// path's anchors and handles instead of pixels.
    #[serde(default)]
    pub path: Option<serde_json::Value>,
    /// The layer was made for this session (⌥⌘T's copy, #352; a file dropped on the canvas), so
    /// Cancel takes it back and OK folds it into one history step with the transform.
    #[serde(default)]
    pub made: Option<MadeLayer>,
    /// Edit › Transform › Rotate / Skew / Distort / Perspective (`Free` for Free Transform and Scale).
    #[serde(default)]
    pub mode: TransformMode,
    /// Edit › Content-Aware Scale (#3096): the box only scales and moves, and OK runs
    /// `edit.contentAwareScale` with these options instead of `edit.transform`.
    #[serde(default)]
    pub content_aware: Option<ContentAwareOptions>,
    /// The options bar's V (vertical skew, degrees) last typed. Any box can be read as scale,
    /// skew and rotation with any vertical skew, so the bar keeps showing the typed one.
    #[serde(default)]
    pub skew_v: f64,
}

/// Content-Aware Scale's options bar: Amount, Protect and Protect Skin Tones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentAwareOptions {
    /// How much is content-aware, 0..100 % (the rest is a plain resample).
    pub amount: f32,
    /// The alpha channel whose white areas keep their shape (`None`: no protection).
    #[serde(default)]
    pub protect: Option<String>,
    #[serde(default)]
    pub protect_skin_tones: bool,
}

impl Default for ContentAwareOptions {
    fn default() -> Self {
        Self { amount: 100.0, protect: None, protect_skin_tones: false }
    }
}

/// Why a Free Transform session's layer was made for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MadeLayer {
    /// Free Transform on a copy (⌥⌘T): OK makes the copy and the transform one Free Transform step.
    Copy,
    /// A file dropped on the canvas: OK makes the place and the transform one Place Embedded step.
    Place,
}

/// A paragraph-box handle drag: the handle, and the box (text-space) and layer transform as
/// they were at the press. Every move recomputes from this snapshot, so ⌥ pressed or released
/// mid-drag mirrors about the centre the box started with — as if it had been held from the
/// start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxResize {
    pub handle: u8,
    pub shape: [f32; 4],
    pub aff: photocraft_geom::Affine,
}

/// In-progress inline type editing (Type tool). Offsets are character indices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextEdit {
    pub layer: u64,
    /// Id of the document being edited (layer ids can repeat across documents, e.g. one .pcraft
    /// opened twice). `None`: the active document.
    #[serde(default)]
    pub document: Option<u64>,
    pub caret: usize,
    pub anchor: usize,
    /// History coalescing key: the whole editing session is one "Edit Type Layer" step.
    pub session: String,
    /// The layer was created by this session (its name follows the text; empty on commit = delete).
    pub created: bool,
    /// A Type Mask tool session: the layer is temporary and commits as a selection.
    #[serde(default)]
    pub mask: bool,
    #[serde(skip)]
    pub dragging: bool,
    /// Paragraph-box handle being dragged (0-3 corners from top-left clockwise, 4-7 top/right/bottom/left edges).
    #[serde(skip)]
    pub resize: Option<BoxResize>,
    /// IME composition in progress: (start, length) in characters. The preedit text lives in the
    /// layer so it lays out like typed text; each IME update replaces it.
    #[serde(skip)]
    pub preedit: Option<(usize, usize)>,
}

/// Temporary Type-tool drag. The document is unchanged until release; this frame is view state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TypeTransform {
    pub document: u64,
    pub revision: u64,
    pub original: photocraft_geom::Affine,
    pub frame: TransformSession,
    pub(crate) gesture: crate::transform_tool::Gesture,
}

/// Content-Aware Move with Transform On Drop: the box on the dropped patch. The document is
/// unchanged until it is committed; this is view state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CamTransform {
    pub document: u64,
    pub revision: u64,
    /// How far the selection was dragged; the box's `rect` is the selection's bounds there.
    pub offset: [i32; 2],
    pub frame: TransformSession,
    #[serde(skip)]
    pub(crate) gesture: Option<crate::transform_tool::Gesture>,
}

/// View-menu overlays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Extras {
    pub rulers: bool,
    pub grid: bool,
    pub guides: bool,
    pub snap: bool,
    pub lock_guides: bool,
}

impl Default for Extras {
    fn default() -> Self {
        Self { rulers: false, grid: false, guides: true, snap: true, lock_guides: false }
    }
}
