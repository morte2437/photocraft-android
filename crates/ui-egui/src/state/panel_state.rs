use super::*;

/// Photoshop-style Layers panel display preferences; part of serialized UI state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum LayerThumbnailSize {
    None,
    Small,
    #[default]
    Medium,
    Large,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum LayerThumbnailContents {
    LayerBounds,
    #[default]
    EntireDocument,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LayersPanelOptions {
    pub thumbnail_size: LayerThumbnailSize,
    /// Thumbnail Contents in the Panel Options dialog. Not saved with these options: the dialog
    /// shows and sets `interface.clipThumbnailsToLayerBounds` (Layer Bounds = clipped, #4281),
    /// which the panel menu's Clip Thumbnails to Layer Bounds toggles too.
    #[serde(skip)]
    pub thumbnail_contents: LayerThumbnailContents,
    pub show_filters: bool,
    pub use_default_masks_on_fill_layers: bool,
    pub expand_new_effects: bool,
    pub add_copy_to_copied_layers_and_groups: bool,
    pub preview_on_canvas_when_reordering_layers: bool,
    pub show_layer_mask_badges: bool,
}

impl LayersPanelOptions {
    /// Every option at the Photoshop default: Medium thumbnails, filters shown, new effects expanded, no mask badges.
    pub fn defaults() -> Self {
        Self {
            thumbnail_size: LayerThumbnailSize::Medium,
            thumbnail_contents: LayerThumbnailContents::EntireDocument,
            show_filters: true,
            use_default_masks_on_fill_layers: true,
            expand_new_effects: true,
            add_copy_to_copied_layers_and_groups: true,
            preview_on_canvas_when_reordering_layers: true,
            show_layer_mask_badges: false,
        }
    }
}

impl Default for LayersPanelOptions {
    /// Same as [`LayersPanelOptions::defaults`], so serde fills missing fields with the Photoshop defaults.
    fn default() -> Self {
        Self::defaults()
    }
}

/// Pending dialog data is transient: Cancel never changes persisted options.
#[derive(Clone, Debug, PartialEq)]
pub enum LayersPanelDialog {
    PanelOptions(LayersPanelOptions),
    NewLayer(String),
    NewGroup(String),
    LockLayers { transparency: bool, pixels: bool, position: bool, artboard: bool, all: bool },
}

/// Shared selection and capture for a point curve; document parameters remain with the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PointCurveState {
    pub selected: Option<usize>,
    /// Captured point (None while removed outside), grab offset, and re-entry eligibility.
    pub drag: Option<PointCurveDrag>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PointCurveDrag {
    pub index: Option<usize>,
    pub offset: [f32; 2],
    pub removed: bool,
}

/// Curves editor memory (egui temp data): the edited channel and the point gesture.
#[derive(Clone, Copy, Debug, Default)]
pub struct CurvesEditorState {
    pub channel: usize,
    pub gesture: PointCurveState,
}

/// Camera Raw scope preferences are view state, never filter parameters or document history.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CameraRawScopeState {
    pub shadows: bool,
    pub highlights: bool,
    pub lab: bool,
    pub sampler_tool: bool,
    pub samplers: Vec<[f32; 2]>,
    pub vectorscope: bool,
    pub selected_region: bool,
    pub red_right: bool,
    pub hide_skin_line: bool,
    pub floating: bool,
    pub floating_rect: Option<[f32; 4]>,
}

/// Camera Raw navigation, separate from filter settings. Reset for each opened image.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CameraRawPreviewState {
    /// None fits the image; Some is physical display pixels per source pixel.
    pub zoom: Option<f32>,
    pub center: [f32; 2],
    pub hand: bool,
}

impl Default for CameraRawPreviewState {
    fn default() -> Self {
        Self { zoom: None, center: [0.5, 0.5], hand: false }
    }
}
