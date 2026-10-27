use super::tool::Tool;
use super::*;

/// Options-bar state for tools (Photoshop keeps these per tool).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolOptions {
    pub tolerance: f32,
    pub contiguous: bool,
    pub anti_alias: bool,
    /// Sample All Layers, kept per tool as in Photoshop (#5524). Stored under a new key, so the
    /// single shared `sample_all_layers` of older preferences is ignored on load.
    pub sample_all: SampleAllLayers,
    /// Object Selection › Object Finder: hovering highlights the object a click selects (#4087).
    pub object_finder: bool,
    /// Remove tool: the fill goes to a new layer above the target (Photoshop's Create a new layer).
    pub remove_new_layer: bool,
    /// Remove tool: each stroke is filled when released (Photoshop's default). Off, the strokes
    /// collect as a tint on the canvas and ↩ applies them together (Esc drops them).
    pub remove_after_each_stroke: bool,
    pub feather: f32,
    /// linear | radial | angle | reflected | diamond
    pub gradient_style: String,
    pub gradient_reverse: bool,
    /// Gradient tool: dither to reduce 8-bit banding (Photoshop default on).
    pub gradient_dither: bool,
    /// Gradient tool mode: false = "Gradient" (live: a Gradient Fill layer, editable on canvas),
    /// true = "Classic gradient" (paints the pixels).
    pub gradient_classic: bool,
    /// Blend mode used by both live and classic gradient drags.
    pub gradient_blend_mode: photocraft_color::BlendMode,
    pub fill_opacity: f32,
    /// Paint Bucket fill source: false = Foreground colour, true = Pattern (Patterns panel selection).
    pub bucket_fill_pattern: bool,
    /// Paint Bucket › Mode (any layer mode; Clear is `bucket_clear`).
    pub bucket_blend_mode: photocraft_color::BlendMode,
    /// Paint Bucket › Mode: Clear, which erases the filled area (see `brush_clear`).
    pub bucket_clear: bool,
    /// Type tool: family, style name, size (pt), anti-aliasing and paragraph alignment.
    pub type_font: String,
    pub type_style: String,
    pub type_size: f32,
    pub type_aa: String,
    pub type_align: String,
    /// Character/Paragraph panel defaults beyond the options bar, in `type.setStyle`'s
    /// parameter format. Kept as overrides so saved engine defaults still supply unset fields.
    /// OpenType features and variable-font axes are merged per tag/axis.
    #[serde(default)]
    pub type_style_defaults: serde_json::Map<String, serde_json::Value>,
    /// Type tool: paragraph direction of new type, "auto" (from the first strong character),
    /// "ltr" or "rtl" (TYPE-218-2).
    pub type_direction: String,
    /// Clone Stamp / Healing Brush.
    pub clone_aligned: bool,
    /// Pattern Stamp: lock the tile origin in document space across strokes.
    pub pattern_stamp_aligned: bool,
    /// Pattern Stamp: jitter each dab's phase (seeded, replayable).
    pub pattern_stamp_impressionist: bool,
    /// Pattern Stamp scale %, 1..1000 (100 = the tile's native size).
    pub pattern_stamp_scale: f32,
    /// Pattern Stamp rotation in degrees (counter-clockwise).
    pub pattern_stamp_angle: f32,
    /// current | currentAndBelow | all
    pub clone_sample: String,
    /// Spot Healing: contentAware | createTexture | proximityMatch
    pub spot_type: String,
    /// Patch: source (repair the selection) | destination (repair where it is dragged).
    pub patch_mode: String,
    /// Patch: Content-Aware instead of Normal, with its Structure 1..7 and Color 0..10.
    pub patch_content_aware: bool,
    pub patch_structure: f32,
    pub patch_color: f32,
    /// Content-Aware Move: move | extend, Structure 1..7, Color 0..10.
    pub cam_mode: String,
    pub cam_structure: f32,
    pub cam_color: f32,
    /// Content-Aware Move: Transform On Drop (a transform box on the dropped patch).
    pub cam_transform_on_drop: bool,
    /// Dodge/Burn: shadows | midtones | highlights, exposure %, protect tones.
    pub tone_range: String,
    pub exposure: f32,
    pub protect_tones: bool,
    /// Sponge: desaturate | saturate, vibrance.
    pub sponge_mode: String,
    pub vibrance: bool,
    /// Blur/Sharpen/Smudge strength %, Sharpen protect detail, Smudge finger painting.
    pub strength: f32,
    pub protect_detail: bool,
    pub finger_painting: bool,
    /// Quick Selection enhance edge.
    pub enhance_edge: bool,
    /// Pen: "path" (work path) or "shape" (shape layer).
    pub vector_mode: String,
    /// Shape tools: fill with the foreground colour, stroke width (0 = none), rectangle corner
    /// radius, polygon sides, line weight.
    pub shape_fill: bool,
    pub stroke_width: f32,
    #[serde(default)]
    pub shape_stroke: crate::shape_stroke_ui::StrokeOptions,
    /// Shape tools: the gradient the next shape's fill / stroke takes (the swatches' Gradient
    /// tab) instead of the foreground / background colour; `None` = the colour.
    pub shape_fill_gradient: Option<photocraft_doc::Fill>,
    pub shape_stroke_gradient: Option<photocraft_doc::Fill>,
    pub corner_radius: f32,
    pub polygon_sides: u32,
    pub line_weight: f32,
    /// Freeform Pen › Curve Fit: how far (px) the fitted path may stray from the pointer trace.
    #[serde(default = "default_curve_fit")]
    pub curve_fit: f32,
    /// Pen › Auto Add/Delete: a Pen click on a segment of the active path adds an anchor there,
    /// on one of its anchors deletes it (#4144).
    #[serde(default = "yes")]
    pub pen_auto_add_delete: bool,
    /// Marquee options bar: "normal" | "fixedRatio" | "fixedSize", with the ratio or size.
    #[serde(default = "default_marquee_style")]
    pub marquee_style: String,
    #[serde(default = "one")]
    pub marquee_width: f32,
    #[serde(default = "one")]
    pub marquee_height: f32,
    /// The unit Fixed Size's Width and Height show (a [`photocraft_engine::prefs::Unit`] name,
    /// picked by right-clicking them); the size itself stays in pixels (#7910).
    #[serde(default = "default_marquee_unit")]
    pub marquee_unit: String,
    /// Slice tool options bar: the same Styles as the marquee's, with the ratio or the size in px
    /// (#8360).
    #[serde(default = "default_marquee_style")]
    pub slice_style: String,
    #[serde(default = "one")]
    pub slice_width: f32,
    #[serde(default = "one")]
    pub slice_height: f32,
    /// Move tool: Auto-Select (with "layer" or "group" target) and Show Transform Controls.
    /// Auto-Select is on with the "group" target by default, as in current Photoshop;
    /// ⌘/Ctrl-click inverts it.
    #[serde(default = "yes")]
    pub move_auto_select: bool,
    #[serde(default = "default_move_target")]
    pub move_target: String,
    #[serde(default)]
    pub move_show_transform: bool,
    /// Crop options bar: aspect ratio ("" = Ratio/unconstrained, "w:h", or "original") and
    /// Delete Cropped Pixels.
    #[serde(default)]
    pub crop_ratio: String,
    /// In-progress custom crop ratio fields. Kept separately because entering one side before the
    /// other must survive a frame while the ratio itself remains unconstrained.
    #[serde(default)]
    pub crop_custom_ratio_width: f32,
    #[serde(default)]
    pub crop_custom_ratio_height: f32,
    #[serde(default = "yes")]
    pub crop_delete: bool,
    /// Crop options bar › Fill: false = Background (Default), true = Content-Aware Fill (the area a
    /// rotated or extending crop uncovers is filled from its surroundings, #3823).
    #[serde(default)]
    pub crop_content_aware: bool,
    /// Crop W x H x Resolution (`crop_ratio` = "whr", #2443): width and height as typed lengths
    /// with their unit ("4 in", "1024 px"; empty = unset), the resolution ("" = the document's)
    /// and its unit ("px/in" or "px/cm"). See `crop_size`.
    #[serde(default)]
    pub crop_width: String,
    #[serde(default)]
    pub crop_height: String,
    #[serde(default)]
    pub crop_resolution: String,
    #[serde(default = "default_crop_resolution_unit")]
    pub crop_resolution_unit: String,
    /// Perspective Crop's Show Grid (`perspective_crop_ui`); it shares W, H and the resolution above.
    #[serde(default = "yes")]
    pub perspective_crop_grid: bool,
    /// Crop overlay (#1919): the guide in the crop box, when it shows and its orientation
    /// (Photoshop's defaults: Rule of Thirds, Auto Show Overlay). See `crop_overlay`.
    #[serde(default)]
    pub crop_overlay: crate::crop_overlay::CropOverlay,
    #[serde(default)]
    pub crop_overlay_show: crate::crop_overlay::OverlayShow,
    #[serde(default)]
    pub crop_overlay_orientation: u8,
    /// Crop gear menu (#1919): Show Cropped Area and the crop shield. See `crop_shield`.
    #[serde(default)]
    pub crop_shield: crate::crop_shield::CropShield,
    /// Magic Eraser opacity % (tolerance, anti-alias and contiguous are shared with the Magic Wand
    /// and Paint Bucket).
    pub magic_eraser_opacity: f32,
    /// Background Eraser: sampling (continuous | once | backgroundSwatch), limits (discontiguous |
    /// contiguous | findEdges), tolerance % and Protect Foreground Color.
    pub bg_sampling: String,
    pub bg_limits: String,
    pub bg_tolerance: f32,
    pub bg_protect_fg: bool,
    /// Color Replacement: mode (hue | saturation | color | luminosity), sampling (continuous |
    /// once | backgroundSwatch), limits (discontiguous | contiguous | findEdges), tolerance % and
    /// anti-alias, as `paint.colorReplacement` takes them.
    pub cr_mode: String,
    pub cr_sampling: String,
    pub cr_limits: String,
    pub cr_tolerance: f32,
    pub cr_anti_alias: bool,
    /// Zoom tool › Scrubby Zoom: dragging left/right zooms continuously (else a zoom rectangle).
    #[serde(default = "yes")]
    pub zoom_scrubby: bool,
    /// Pencil › Auto Erase: a stroke that starts on the foreground colour paints the background colour.
    #[serde(default)]
    pub pencil_auto_erase: bool,
    /// Brush › Mode: Clear, which erases with the Brush (Photoshop's painting-only mode, #3439).
    /// Not a blend mode, so it is kept beside the brush's `mode` rather than in it.
    #[serde(default)]
    pub brush_clear: bool,
    /// Pencil › Mode: Clear (see `brush_clear`).
    #[serde(default)]
    pub pencil_clear: bool,
    /// Eraser › Mode: `brush` (soft, anti-aliased) or `pencil` (hard, aliased pixels, through
    /// `paint.pencil`), as in Photoshop's options bar (#2662).
    #[serde(default = "default_eraser_mode")]
    pub eraser_mode: String,
    /// Magnetic Lasso: detection width (px, 1..256), edge contrast (%, 1..100), how often it
    /// fastens points by itself (0..100), and whether pen pressure narrows the width.
    pub magnetic_width: f32,
    pub magnetic_contrast: f32,
    pub magnetic_frequency: f32,
    pub magnetic_pressure: bool,
    /// Red Eye: pupil search size (1–100) and how far corrected pixels darken (0–100).
    #[serde(default = "fifty")]
    pub red_eye_pupil_size: f32,
    #[serde(default = "fifty")]
    pub red_eye_darken: f32,
    /// Eyedropper › Sample Size: 1 = Point Sample, else the side of the averaged square (3, 5,
    /// 11, 31, 51, 101; `photocraft_engine::sample_cmds::SAMPLE_SIZES`). Shared, as in Photoshop,
    /// with the ⌥-click eyedropper of the painting tools, the dialog eyedroppers and the Info panel.
    #[serde(default = "one_u32")]
    pub eyedropper_size: u32,
    /// Eyedropper › Sample: current | currentAndBelow | all | allNoAdjustments |
    /// currentAndBelowNoAdjustments (`document.sampleColor`'s `sampleLayer`).
    #[serde(default = "default_eyedropper_sample")]
    pub eyedropper_sample: String,
    /// Eyedropper › Show Sampling Ring.
    #[serde(default = "yes")]
    pub eyedropper_ring: bool,
    /// Art History Brush: dab layout, Area (0–100), Tolerance (0–100).
    #[serde(default = "default_art_history_style")]
    pub art_history_style: String,
    #[serde(default = "fifty")]
    pub art_history_area: f32,
    #[serde(default)]
    pub art_history_tolerance: f32,
}

fn default_art_history_style() -> String {
    "tightMedium".into()
}

pub(super) fn yes() -> bool {
    true
}

fn default_crop_resolution_unit() -> String {
    crate::crop_size::PX_PER_IN.into()
}

fn default_move_target() -> String {
    "group".into()
}

fn default_curve_fit() -> f32 {
    2.0
}

fn default_eraser_mode() -> String {
    "brush".into()
}

fn default_marquee_style() -> String {
    "normal".into()
}

fn default_marquee_unit() -> String {
    "pixels".into()
}

fn one() -> f32 {
    1.0
}

fn fifty() -> f32 {
    50.0
}

/// The session brush starts out as the Brush's (the tool the app opens with).
pub(super) fn default_brush_tool() -> Tool {
    Tool::Brush
}

pub(super) fn default_layer_filter_mode() -> photocraft_color::BlendMode {
    photocraft_color::BlendMode::Normal
}

fn one_u32() -> u32 {
    1
}

fn default_eyedropper_sample() -> String {
    "all".into()
}

/// Each tool's Sample All Layers (the Paint Bucket calls it All Layers). Photoshop keeps one per
/// tool, all off by default: turning it on for the Magic Wand must not leave Blur or Smudge
/// copying the composite onto the active layer (#5524). The Mixer Brush keeps its own with the
/// brush (`MixerSettings`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SampleAllLayers {
    pub magic_wand: bool,
    pub quick_selection: bool,
    pub object_selection: bool,
    pub paint_bucket: bool,
    pub magic_eraser: bool,
    pub spot_healing: bool,
    pub patch: bool,
    pub content_aware_move: bool,
    pub remove: bool,
    pub blur: bool,
    pub sharpen: bool,
    pub smudge: bool,
}

impl SampleAllLayers {
    /// `tool`'s setting; false for a tool without the option.
    pub fn get(mut self, tool: Tool) -> bool {
        self.slot(tool).is_some_and(|v| *v)
    }

    /// `tool`'s setting, or `None` for a tool without the option.
    pub fn slot(&mut self, tool: Tool) -> Option<&mut bool> {
        Some(match tool {
            Tool::MagicWand => &mut self.magic_wand,
            Tool::QuickSelection => &mut self.quick_selection,
            Tool::ObjectSelection => &mut self.object_selection,
            Tool::PaintBucket => &mut self.paint_bucket,
            Tool::MagicEraser => &mut self.magic_eraser,
            Tool::SpotHealing => &mut self.spot_healing,
            Tool::Patch => &mut self.patch,
            Tool::ContentAwareMove => &mut self.content_aware_move,
            Tool::Remove => &mut self.remove,
            Tool::Blur => &mut self.blur,
            Tool::Sharpen => &mut self.sharpen,
            Tool::Smudge => &mut self.smudge,
            _ => return None,
        })
    }
}

impl Default for ToolOptions {
    fn default() -> Self {
        Self {
            tolerance: 32.0,
            contiguous: true,
            anti_alias: true,
            sample_all: SampleAllLayers::default(),
            object_finder: true,
            remove_new_layer: false,
            remove_after_each_stroke: true,
            feather: 0.0,
            gradient_style: "linear".into(),
            gradient_reverse: false,
            gradient_dither: true,
            gradient_classic: false,
            gradient_blend_mode: photocraft_color::BlendMode::Normal,
            fill_opacity: 100.0,
            bucket_fill_pattern: false,
            bucket_blend_mode: photocraft_color::BlendMode::Normal,
            bucket_clear: false,
            type_font: "Inter".into(),
            type_style: "Regular".into(),
            type_size: 48.0,
            type_aa: "sharp".into(),
            type_align: "left".into(),
            type_style_defaults: serde_json::Map::new(),
            type_direction: "auto".into(),
            clone_aligned: true,
            pattern_stamp_aligned: true,
            pattern_stamp_impressionist: false,
            pattern_stamp_scale: 100.0,
            pattern_stamp_angle: 0.0,
            clone_sample: "current".into(),
            spot_type: "contentAware".into(),
            patch_mode: "source".into(),
            patch_content_aware: false,
            patch_structure: 4.0,
            patch_color: 0.0,
            cam_mode: "move".into(),
            cam_structure: 4.0,
            cam_color: 0.0,
            cam_transform_on_drop: false,
            tone_range: "midtones".into(),
            exposure: 50.0,
            protect_tones: true,
            sponge_mode: "desaturate".into(),
            vibrance: true,
            strength: 50.0,
            protect_detail: true,
            finger_painting: false,
            enhance_edge: false,
            vector_mode: "path".into(),
            shape_fill: true,
            stroke_width: 0.0,
            shape_stroke: Default::default(),
            shape_fill_gradient: None,
            shape_stroke_gradient: None,
            corner_radius: 0.0,
            polygon_sides: 5,
            line_weight: 3.0,
            curve_fit: default_curve_fit(),
            pen_auto_add_delete: true,
            marquee_style: default_marquee_style(),
            marquee_width: 1.0,
            marquee_height: 1.0,
            marquee_unit: default_marquee_unit(),
            slice_style: default_marquee_style(),
            slice_width: 1.0,
            slice_height: 1.0,
            move_auto_select: true,
            move_target: default_move_target(),
            move_show_transform: false,
            crop_ratio: String::new(),
            crop_custom_ratio_width: 0.0,
            crop_custom_ratio_height: 0.0,
            crop_delete: true,
            crop_content_aware: false,
            crop_width: String::new(),
            crop_height: String::new(),
            crop_resolution: String::new(),
            crop_resolution_unit: default_crop_resolution_unit(),
            perspective_crop_grid: true,
            crop_overlay: Default::default(),
            crop_overlay_show: Default::default(),
            crop_overlay_orientation: 0,
            crop_shield: Default::default(),
            magic_eraser_opacity: 100.0,
            bg_sampling: "continuous".into(),
            bg_limits: "contiguous".into(),
            bg_tolerance: 50.0,
            bg_protect_fg: false,
            cr_mode: "color".into(),
            cr_sampling: "continuous".into(),
            cr_limits: "contiguous".into(),
            cr_tolerance: 30.0,
            cr_anti_alias: true,
            zoom_scrubby: true,
            pencil_auto_erase: false,
            brush_clear: false,
            pencil_clear: false,
            eraser_mode: default_eraser_mode(),
            magnetic_width: 10.0,
            magnetic_contrast: 10.0,
            magnetic_frequency: 57.0,
            magnetic_pressure: false,
            red_eye_pupil_size: 50.0,
            red_eye_darken: 50.0,
            eyedropper_size: 1,
            eyedropper_sample: default_eyedropper_sample(),
            eyedropper_ring: true,
            art_history_style: default_art_history_style(),
            art_history_area: 50.0,
            art_history_tolerance: 0.0,
        }
    }
}
