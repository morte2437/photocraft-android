//! UI state as plain data, so the control channel (and later MCP) can read and drive every
//! aspect of the interface: tools, zoom, panels, dialogs and windows.

use serde::{Deserialize, Serialize};

mod color_panel;
mod panel_state;
mod tool;
mod tool_options;
mod transform;

pub use color_panel::{ColorPanelMode, ColorPanelState, ColorRamp, DockTabs};
pub use panel_state::{
    CameraRawPreviewState, CameraRawScopeState, CurvesEditorState, LayerThumbnailContents, LayerThumbnailSize, LayersPanelDialog, LayersPanelOptions,
    PointCurveDrag, PointCurveState,
};
pub use tool::Tool;
pub use tool_options::{SampleAllLayers, ToolOptions};
use tool_options::{default_brush_tool, default_layer_filter_mode, yes};
pub use transform::{BoxResize, CamTransform, ContentAwareOptions, Extras, MadeLayer, TextEdit, TransformMode, TransformSession, TypeTransform};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Panels {
    pub layers: bool,
    pub history: bool,
    pub properties: bool,
    pub color: bool,
    pub navigator: bool,
    pub toolbar: bool,
    pub options_bar: bool,
    pub status_bar: bool,
    /// Window › Brush Settings (F5): floating, like Photoshop's.
    #[serde(default)]
    pub brush_settings: bool,
    /// Window › Brushes: dockable panel, like Photoshop's.
    #[serde(default)]
    pub brushes: bool,
    /// Window › Character / Paragraph: the Character | Paragraph dock group (#150).
    #[serde(default)]
    pub character: bool,
    /// The toolbar's header chevron: two columns even when one fits (#1197).
    #[serde(default)]
    pub toolbar_double: bool,
    /// The right-hand panel dock. ⇧Tab hides and shows it, as in Photoshop (#1313).
    #[serde(default = "yes")]
    pub dock: bool,
    /// What Tab hid (toolbar, options bar, dock), so a second Tab brings back just those.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden_by_tab: Option<[bool; 3]>,
    /// Tab in Full Screen Mode shows the menu bar, toolbar, options bar and dock over the canvas
    /// (#6760). Only a view of the moment: not saved, and the panels' own flags stay untouched.
    #[serde(skip)]
    pub full_screen_chrome: bool,
    /// The right dock's icon rail. An app that embeds PhotoCraft's UI can hide it for a simpler view.
    #[serde(default = "yes")]
    pub rail: bool,
    /// The title bar with the in-window menus. An app that embeds PhotoCraft's UI and draws its own
    /// bar can hide it; the caption buttons of a custom title bar go with it.
    #[serde(default = "yes")]
    pub menu_bar: bool,
    /// The Gradient tool's options-bar swatch opened the Gradient Editor window (`gradient_ui`).
    /// Photoshop opens its Gradient Editor from that swatch; this is the same idea, drawn as a
    /// floating window rather than a modal so the canvas stays usable while a gradient is edited.
    #[serde(default)]
    pub gradient_editor: bool,
}

impl Default for Panels {
    fn default() -> Self {
        Self {
            layers: true,
            history: false,
            properties: true,
            color: true,
            navigator: false,
            toolbar: true,
            options_bar: true,
            status_bar: true,
            brush_settings: false,
            brushes: false,
            character: false,
            toolbar_double: false,
            dock: true,
            hidden_by_tab: None,
            full_screen_chrome: false,
            rail: true,
            menu_bar: true,
            gradient_editor: false,
        }
    }
}

/// A modal or modeless dialog, identified by `id`, with editable fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dialog {
    pub id: u64,
    pub kind: DialogKind,
    /// Field values by name. Dialog widgets read and write these; so can automation.
    pub fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DialogKind {
    NewDocument,
    About,
    /// Parameter dialog for a command (generated from its params).
    Command,
    Error,
    /// Photoshop Layer Style dialog (see `layer_style.rs`).
    LayerStyle,
}

/// Per-document view (camera) state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    /// Screen (device) pixels per document pixel: the user-facing zoom (`100%` is `1.0`).
    /// Canvas geometry works in egui points and divides by `ctx.pixels_per_point`
    /// (`PhotocraftApp::point_zoom`), so neither the display scale nor Interface › UI Scale
    /// magnifies the image (#1943, #2121).
    pub zoom: f32,
    /// Document-space point shown at the canvas centre.
    pub center: [f32; 2],
    /// Recompute fit-to-screen on next frame.
    pub fit_pending: bool,
    /// Recompute fill-screen on next frame. Unlike fit, this may crop an edge of the document.
    #[serde(default)]
    pub fill_pending: bool,
    /// Document size this view last showed; a change (Image/Canvas Size, crop) re-centres it.
    #[serde(default)]
    pub doc_size: [u32; 2],
    /// Canvas camera rotation in degrees, wrapped to (−180, 180]. Turns the view, not the pixels.
    #[serde(default)]
    pub rotation: f32,
    /// Transient presentation state for a discrete zoom command; the target camera above remains
    /// authoritative and this flag is never persisted.
    #[serde(skip)]
    pub(crate) zoom_animating: bool,
    /// Optional document/screen anchor for a pointer-centred Zoom-tool transition.
    #[serde(skip)]
    pub(crate) zoom_anchor: Option<([f64; 2], [f32; 2])>,
    #[serde(skip)]
    pub(crate) display_camera: Option<(f32, [f32; 2])>,
    #[serde(skip)]
    pub(crate) zoom_transition: Option<ZoomTransition>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ZoomTransition {
    pub from: (f32, [f32; 2]),
    pub target: (f32, [f32; 2]),
    pub started: f64,
}

impl View {
    /// Start a discrete zoom only when Animated Zoom is enabled; otherwise discard the
    /// presentation camera so the requested camera takes effect immediately.
    pub(crate) fn request_zoom_animation(&mut self, enabled: bool) {
        self.zoom_animating = enabled;
        self.zoom_anchor = None;
        if !enabled {
            self.zoom_transition = None;
            self.display_camera = Some((self.zoom, self.center));
        }
    }
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            center: [0.0, 0.0],
            fit_pending: true,
            fill_pending: false,
            doc_size: [0, 0],
            rotation: 0.0,
            zoom_animating: false,
            zoom_anchor: None,
            display_camera: None,
            zoom_transition: None,
        }
    }
}

/// An extra OS window showing a document ("Window → Arrange → New Window for Document").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocWindow {
    pub id: u64,
    pub document: usize,
    pub view: View,
    pub open: bool,
    /// Where the window opens on screen (outer top-left, in points); `None` lets the OS place it.
    /// A tab dragged off the tab strip opens its window where it was dropped (#3733).
    #[serde(default)]
    pub position: Option<[f32; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiState {
    pub tool: Tool,
    /// The last tool used in each shortcut group (the tools sharing a key, [`Tool::key`]), at most
    /// one per group: the group's key brings it back, as in Photoshop (#2608).
    #[serde(default)]
    pub group_tools: Vec<Tool>,
    /// A tool letter still held (spring-loaded tool, #2672): the tool to give back on release.
    /// Exposed to automation, never restored from settings.
    #[serde(default, skip_deserializing)]
    pub spring: Option<crate::spring_tools::Spring>,
    /// Recently opened file paths, most-recent first (File › Open Recent). Capped; de-duplicated.
    #[serde(default)]
    pub recent_files: Vec<String>,
    /// Recently picked type families, most-recent first (the font menu's Recent group). Capped by
    /// Preferences ▸ Type ▸ Recent Fonts; de-duplicated.
    #[serde(default)]
    pub recent_fonts: Vec<String>,
    /// Inline type editing session, if any.
    #[serde(default)]
    pub text_edit: Option<TextEdit>,
    /// Clock time in seconds when type cursor blink was last reset (e.g. on keypress/caret move).
    #[serde(skip)]
    pub caret_blink_time: f64,
    /// Tool to restore when an inline type edit session commits or cancels (e.g. Move tool, #1132).
    #[serde(skip)]
    pub text_edit_return_tool: Option<Tool>,
    /// Ctrl/Cmd Type-tool gesture, exposed to automation but never restored as a held pointer.
    #[serde(default, skip_deserializing)]
    pub type_transform: Option<TypeTransform>,
    /// Reference point for the current type editing session (document coordinates).
    #[serde(skip)]
    pub type_transform_pivot: Option<[f64; 2]>,
    /// Content-Aware Move's Transform On Drop box, waiting for ↩ / Esc.
    #[serde(default, skip_deserializing)]
    pub cam_transform: Option<CamTransform>,
    /// Free Transform session, if any.
    #[serde(default)]
    pub transform: Option<TransformSession>,
    /// The options bar's Maintain Aspect Ratio link is off: W/H change on their own, and handle
    /// drags stretch freely (⇧ keeps the proportions), as in Photoshop (#4174).
    #[serde(default)]
    pub transform_unlinked: bool,
    /// Temporary symmetry-axis editing, exposed to automation but never restored from settings.
    #[serde(default, skip_deserializing)]
    pub symmetry_transform: Option<crate::symmetry_ui::Transform>,
    /// The Properties panel's armed Levels/Curves eyedropper on an adjustment layer (`layer_picker`),
    /// exposed to automation but never restored from settings.
    #[serde(default, skip_deserializing)]
    pub layer_picker: Option<crate::layer_picker::LayerPicker>,
    /// Clone Stamp / Healing source point (⌥-click) and the aligned offset once a stroke started.
    #[serde(default)]
    pub clone_source: Option<[f64; 2]>,
    #[serde(default)]
    pub clone_offset: Option<[f64; 2]>,
    /// Pattern Stamp aligned origin in document pixels, kept across strokes.
    #[serde(default)]
    pub pattern_stamp_phase: Option<[f32; 2]>,
    /// Painting targets the active layer's mask instead of its pixels.
    #[serde(default)]
    pub mask_target: bool,
    /// The active layer's vector mask is targeted (its Layers thumbnail is bracketed): the path
    /// tools edit it (#196). Never set together with `mask_target`.
    #[serde(default)]
    pub vector_mask_target: bool,
    /// The smart object (layer id) whose filter mask is targeted (its thumbnail on the Smart
    /// Filters row clicked): painting, fills, filters and adjustments edit that mask
    /// (FILE-215-3). Never set together with `mask_target` or `vector_mask_target`.
    #[serde(default)]
    pub filter_mask_target: Option<u64>,
    /// The adjustment or fill layer (id) whose own settings Properties shows although its mask
    /// is targeted: set when that layer is created, as in Photoshop (#3369). Targeting the mask
    /// thumbnail, the layer's pixels or another layer clears it.
    #[serde(default)]
    pub properties_layer: Option<u64>,
    /// Brush Preset picker opened by a right-click on the canvas or the options-bar brush chip:
    /// its screen position (points).
    #[serde(default)]
    pub brush_picker: Option<[f32; 2]>,
    /// The Brush Preset picker's preset list: search, collapsed groups, view, a rename in progress.
    #[serde(default = "crate::brush_picker::list_state")]
    pub brush_picker_list: crate::brush_panel::BrushesPanelState,
    /// The Brush Preset picker's content size, once its corner grip was dragged
    /// ([`crate::brush_picker::DEFAULT_SIZE`] before that).
    #[serde(default)]
    pub brush_picker_size: Option<[f32; 2]>,
    /// Layers under the pointer, listed by a right-click on the canvas with the Move tool or
    /// ⌘/Ctrl+right-click with any tool (`layer_pick_ui`, #307).
    #[serde(default)]
    pub layer_menu: Option<crate::layer_pick_ui::LayerMenu>,
    /// Selection-tool context menu opened by a plain canvas right-click.
    #[serde(default)]
    pub canvas_tool_menu: Option<crate::canvas_tool_menu::CanvasToolMenu>,
    /// The guide a Move-tool click selected (`guide_select`): drawn blue, Delete removes it.
    #[serde(default)]
    pub selected_guide: Option<crate::guide_select::SelectedGuide>,
    /// The brush is a per-tool option (the Brush, the Eraser and every retouching tool each keep
    /// their own size, hardness, tip and dynamics, as in Photoshop): the tool whose brush the
    /// session brush holds, and the other tools' saved brushes.
    #[serde(default = "default_brush_tool")]
    pub brush_tool: Tool,
    #[serde(default)]
    pub tool_brushes: Vec<(Tool, photocraft_engine::paint::BrushSettings)>,
    /// Pen path under construction.
    #[serde(default)]
    pub pen: Option<crate::vector_ui::PenPath>,
    /// The document the pen path is drawn on; switching away commits it to that Work Path.
    #[serde(default)]
    pub pen_doc: Option<photocraft_doc::DocId>,
    #[serde(default)]
    pub stroke_editor: Option<crate::shape_stroke_ui::StrokeEditor>,
    /// Direct Selection tool: selected anchors and the drag in progress (#790).
    #[serde(default)]
    pub direct_selection: crate::direct_select::DirectSelection,
    /// A live rectangle's corner radius widget being dragged (TOOL-214-12).
    #[serde(skip)]
    pub corner_radius: Option<crate::corner_radius::Drag>,
    /// Selected row in the Paths panel ("work" or a saved path name).
    #[serde(default)]
    pub selected_path: Option<String>,
    /// The document and Work Path deselected by making a selection from it with the Pen or Path
    /// Selection tool (#6079): the canvas hides that path until a path is selected again or the
    /// Work Path changes.
    #[serde(default)]
    pub hidden_work_path: Option<(u64, photocraft_doc::Path)>,
    /// Layers panel kind filter ("pixel", "adjustment", "type", "shape", "smart"); empty = all.
    #[serde(default)]
    pub layer_filter: Vec<String>,
    /// Thumbnail dimensions, content cropping and filter-strip visibility.
    #[serde(default)]
    pub layers_panel_options: LayersPanelOptions,
    /// Transient dialog; never restored with a workspace.
    #[serde(skip)]
    pub layers_panel_dialog: Option<LayersPanelDialog>,
    /// Layers panel filter type (#5048): kind (above), name or blend mode.
    #[serde(default)]
    pub layer_filter_by: crate::layer_filter_ui::FilterBy,
    /// Layers panel name filter: part of a layer name, ignoring case; empty = all.
    #[serde(default)]
    pub layer_filter_name: String,
    /// Layers panel blend-mode filter.
    #[serde(default = "default_layer_filter_mode")]
    pub layer_filter_mode: photocraft_color::BlendMode,
    /// Actions panel: which row is selected and which are expanded. The list lives on the session.
    #[serde(default)]
    pub actions: crate::actions::ActionsUi,
    /// Layer Comps panel: the selected comp (by comp id).
    #[serde(default)]
    pub layer_comp_selected: Option<u32>,
    /// Preset panels (Gradients, Patterns, Styles, Shapes, Tool Presets, Clone Source).
    #[serde(default)]
    pub presets_ui: crate::preset_panels::PresetUi,
    /// Save for Web: the preview tab last chosen (`original`, `optimized`, `2up`, `4up`), which the
    /// dialog reopens on (#9174).
    #[serde(default)]
    pub web_view: Option<String>,
    /// Character/Paragraph Styles, Glyphs and Check Spelling (see `type_panels_ui`).
    #[serde(default)]
    pub type_panels: crate::type_panels_ui::TypePanelsUi,
    /// Ruler/Count/Note tools, Measurement Log and Notes panels (see `analysis_ui`).
    #[serde(default)]
    pub analysis: crate::analysis_ui::AnalysisUi,
    pub timeline: crate::timeline_ui::TimelineUi,
    /// Slice and Slice Select tools (see `slice_ui`).
    #[serde(default)]
    pub slices: crate::slice_ui::SliceUi,
    /// Frame tool (see `frame_tool`).
    #[serde(default)]
    pub frame_tool: crate::frame_tool::FrameToolUi,
    /// Artboard tool (see `artboard_tool`).
    pub artboard_tool: crate::artboard_tool::ArtboardToolUi,
    /// Modifier Keys panel, custom pixel aspect ratios, workspace dialogs (see `workspace_ui`).
    #[serde(default)]
    pub shell: crate::workspace_ui::ShellUi,
    /// View extras: rulers (⌘R), grid (⌘'), guides (⌘;), snapping (⇧⌘;), locked guides (⌥⌘;).
    #[serde(default)]
    pub extras: Extras,
    /// View / Window / Type preferences: screen mode, Extras, Show and Snap To, flip, arrangement.
    #[serde(default)]
    pub view: crate::view_cmds::ViewOptions,
    pub panels: Panels,
    /// Views per open document (index-aligned with the session's documents).
    pub views: Vec<View>,
    pub dialogs: Vec<Dialog>,
    pub windows: Vec<DocWindow>,
    pub theme: crate::theme::ThemeKind,
    pub workspace: String,
    pub palette_open: bool,
    pub dock_tabs: DockTabs,
    /// Right-dock group order, heights and collapsed groups (see `dock`).
    #[serde(default)]
    pub dock: crate::dock::DockLayout,
    /// Authoritative custom panel membership and placement; legacy fields are compatibility projections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docking: Option<craft_ui::docking::Workspace<String>>,
    /// Where each floating panel docks back to.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub docking_returns: std::collections::BTreeMap<String, craft_ui::docking::Location<String>>,
    /// Invalidates in-flight gestures when a workspace is replaced.
    #[serde(skip)]
    pub docking_generation: u64,
    /// The saved `docking`, `dockingReturns` and `dockingCollapsed` of a workspace from a newer
    /// version, which shows the default layout: saved again unchanged until the layout changes.
    #[serde(skip)]
    pub docking_kept: Option<serde_json::Map<String, serde_json::Value>>,
    /// Panels belonging to explicitly collapsed custom tab groups.
    #[serde(default)]
    pub docking_collapsed: Vec<String>,
    /// One-frame focus/raise request from an explicit panel reveal.
    #[serde(skip)]
    pub docking_reveal: Option<String>,
    /// Which chip the Color panel edits, and how it shows the colour.
    #[serde(default)]
    pub color_panel: ColorPanelState,
    /// Histogram panel options: view, channel, statistics, colours, source.
    #[serde(default)]
    pub histogram_panel: crate::histogram_panel::HistogramPanelState,
    /// The tonal range dragged across the Histogram panel's graph (its statistics), `[from, to]`.
    #[serde(default, skip_deserializing)]
    pub histogram_range: Option<[u8; 2]>,
    /// Brush Settings: selected section (0 = Brush Tip Shape) and tab (0 settings, 1 Brushes).
    #[serde(default)]
    pub brush_section: usize,
    #[serde(default)]
    pub brush_tab: usize,
    /// Brushes panel: collapsed groups and the search filter.
    #[serde(default)]
    pub brushes_panel: crate::brush_panel::BrushesPanelState,
    /// Marquee options-bar mode: 0 new, 1 add, 2 subtract, 3 intersect (modifier keys override).
    #[serde(default)]
    pub selection_mode: u8,
    #[serde(default)]
    pub tool_options: ToolOptions,
    /// The shape Fill / Stroke swatches' recently used gradients, newest first (the Gradient
    /// tab's Recent row).
    #[serde(default)]
    pub recent_gradients: Vec<photocraft_doc::Fill>,
    /// In-progress polygonal lasso vertices (document coordinates).
    #[serde(default)]
    pub polygon: Vec<[f64; 2]>,
    /// The selection mode the polygonal lasso started in ("replace", "add", ...), set by the
    /// modifiers held at its first click.
    #[serde(default)]
    pub polygon_mode: String,
    /// The document the polygon is drawn on (set at its first click): another document becoming
    /// active drops it (`canvas::polygon_cancel_stale`).
    #[serde(default)]
    pub polygon_doc: Option<photocraft_doc::DocId>,
    /// Magnetic Lasso border in progress (`magnetic_lasso_ui`).
    #[serde(default)]
    pub magnetic: crate::magnetic_lasso_ui::MagneticLasso,
    /// Crop tool rectangle being edited [x0, y0, x1, y1] (document coordinates).
    #[serde(default)]
    pub crop_rect: Option<[f64; 4]>,
    /// The crop frame's turn in degrees, clockwise on screen about its centre (`crop_rect` is the
    /// frame before the turn); 0 when upright. Committing passes it as `image.crop`'s `angle`.
    #[serde(default)]
    pub crop_angle: f64,
    /// Perspective Crop quad being edited: TL, TR, BR, BL (document coordinates).
    #[serde(default)]
    pub perspective_crop: Option<[[f64; 2]; 4]>,
    pub next_id: u64,
    /// Last status message (errors from commands, hints).
    pub status: String,
    /// The status message is an error (shown in the warning colour).
    #[serde(default)]
    pub status_error: bool,
    /// Non-blocking notices (import/export warnings, files that couldn't open), newest last.
    #[serde(default)]
    pub notices: Vec<crate::notices::Notice>,
    /// Non-modal release-check window; readable and controllable through `ui.inspect`/`ui.set`.
    #[serde(default)]
    pub updates_open: bool,
    /// Pending GPU fallback warning, visible to automation.
    #[serde(default)]
    pub gpu_fallback_notice: Option<String>,
    /// A keyboard shortcut set found at first launch, waiting for Import / Don't Import.
    #[serde(default)]
    pub kys_offer: Option<crate::kys_import::Offer>,
    /// Documents (ids) whose slow full refresh on the CPU compositor has had its notice.
    #[serde(default)]
    pub slow_refresh_noticed: Vec<u64>,
    /// Status bar info field, Home screen (see `chrome_ui`).
    #[serde(default)]
    pub chrome: crate::chrome_ui::ChromeState,
    #[serde(default)]
    pub camera_raw_scope: CameraRawScopeState,
    #[serde(default)]
    pub camera_raw_preview: CameraRawPreviewState,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            tool: Tool::Brush,
            group_tools: Vec::new(),
            spring: None,
            recent_files: Vec::new(),
            recent_fonts: Vec::new(),
            text_edit: None,
            caret_blink_time: 0.0,
            text_edit_return_tool: None,
            type_transform: None,
            type_transform_pivot: None,
            cam_transform: None,
            transform: None,
            transform_unlinked: false,
            symmetry_transform: None,
            layer_picker: None,
            mask_target: false,
            vector_mask_target: false,
            filter_mask_target: None,
            properties_layer: None,
            brush_picker: None,
            brush_picker_list: crate::brush_picker::list_state(),
            brush_picker_size: None,
            layer_menu: None,
            canvas_tool_menu: None,
            selected_guide: None,
            brush_tool: Tool::Brush,
            tool_brushes: Vec::new(),
            clone_source: None,
            clone_offset: None,
            pattern_stamp_phase: None,
            extras: Extras::default(),
            view: Default::default(),
            actions: Default::default(),
            layer_comp_selected: None,
            presets_ui: Default::default(),
            web_view: None,
            type_panels: Default::default(),
            analysis: Default::default(),
            timeline: Default::default(),
            slices: Default::default(),
            frame_tool: Default::default(),
            artboard_tool: Default::default(),
            shell: Default::default(),
            layer_filter: Vec::new(),
            layers_panel_options: LayersPanelOptions::default(),
            histogram_panel: Default::default(),
            histogram_range: None,
            layers_panel_dialog: None,
            layer_filter_by: Default::default(),
            layer_filter_name: String::new(),
            layer_filter_mode: default_layer_filter_mode(),
            pen: None,
            pen_doc: None,
            stroke_editor: None,
            direct_selection: Default::default(),
            corner_radius: None,
            selected_path: None,
            hidden_work_path: None,
            panels: Panels::default(),
            views: Vec::new(),
            dialogs: Vec::new(),
            windows: Vec::new(),
            theme: crate::theme::ThemeKind::ProMedium,
            workspace: "Essentials".into(),
            palette_open: false,
            dock_tabs: DockTabs::default(),
            dock: Default::default(),
            docking: None,
            docking_returns: Default::default(),
            docking_kept: None,
            docking_generation: 0,
            docking_collapsed: Vec::new(),
            docking_reveal: None,
            color_panel: Default::default(),
            brush_section: 0,
            brush_tab: 0,
            brushes_panel: Default::default(),
            selection_mode: 0,
            tool_options: ToolOptions::default(),
            recent_gradients: Vec::new(),
            polygon: Vec::new(),
            polygon_mode: String::new(),
            polygon_doc: None,
            magnetic: Default::default(),
            crop_rect: None,
            crop_angle: 0.0,
            perspective_crop: None,
            next_id: 1,
            status: String::new(),
            status_error: false,
            notices: Vec::new(),
            updates_open: false,
            gpu_fallback_notice: None,
            kys_offer: None,
            slow_refresh_noticed: Vec::new(),
            chrome: Default::default(),
            camera_raw_scope: Default::default(),
            camera_raw_preview: Default::default(),
        }
    }
}

impl UiState {
    /// Records the current tool as the last one used in its shortcut group.
    pub fn remember_group_tool(&mut self) {
        let tool = self.tool;
        if !self.group_tools.contains(&tool) {
            self.group_tools.retain(|t| t.key() != tool.key());
            self.group_tools.push(tool);
        }
    }

    /// The last tool used in the shortcut group of `key`, if any.
    pub fn group_tool(&self, key: char) -> Option<Tool> {
        self.group_tools.iter().copied().find(|t| t.key() == key)
    }

    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn open_dialog(&mut self, kind: DialogKind, fields: serde_json::Map<String, serde_json::Value>) -> u64 {
        let id = self.alloc_id();
        self.dialogs.push(Dialog { id, kind, fields });
        id
    }

    pub fn dialog_mut(&mut self, id: u64) -> Option<&mut Dialog> {
        self.dialogs.iter_mut().find(|d| d.id == id)
    }

    pub fn close_dialog(&mut self, id: u64) -> Option<Dialog> {
        let i = self.dialogs.iter().position(|d| d.id == id)?;
        Some(self.dialogs.remove(i))
    }

    pub fn new_document_fields() -> serde_json::Map<String, serde_json::Value> {
        let v = serde_json::json!({"name": "Untitled-1", "width": 1920, "height": 1080, "mode": "rgb", "depth": 8, "background": "white"});
        v.as_object().cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_all_layers_is_per_tool_and_old_prefs_load() {
        // The single shared flag of older preferences is dropped: every tool starts off (#5524).
        let o: ToolOptions = serde_json::from_str(r#"{"sample_all_layers": true, "feather": 3.0}"#).unwrap();
        assert_eq!((o.sample_all, o.feather), (SampleAllLayers::default(), 3.0));
        let mut o = ToolOptions::default();
        o.sample_all.magic_wand = true;
        let back: ToolOptions = serde_json::from_value(serde_json::to_value(&o).unwrap()).unwrap();
        assert!(back.sample_all.magic_wand && !back.sample_all.get(Tool::Blur));
        assert!(!back.sample_all.get(Tool::Brush), "a tool without the option reads false");
        let mut s = SampleAllLayers::default();
        assert!(s.slot(Tool::Brush).is_none());
        for t in [Tool::MagicWand, Tool::PaintBucket, Tool::Blur, Tool::Sharpen, Tool::Smudge, Tool::Remove] {
            *s.slot(t).unwrap() = true;
            assert!(s.get(t), "{t:?}");
        }
        assert!(!s.get(Tool::QuickSelection));
    }

    #[test]
    fn tool_names_parse() {
        basic_and_retouch_tool_names_parse();
        vector_tool_names_parse();
    }

    fn basic_and_retouch_tool_names_parse() {
        assert_eq!(Tool::from_name("brush"), Some(Tool::Brush));
        assert_eq!(Tool::from_name("Rect"), Some(Tool::RectMarquee));
        assert_eq!(Tool::from_name("RectMarquee"), Some(Tool::RectMarquee));
        assert_eq!(Tool::from_name("Eraser Tool"), Some(Tool::Eraser));
        assert_eq!(Tool::from_name("mixerBrush"), Some(Tool::MixerBrush));
        assert_eq!(Tool::from_name("Mixer Brush Tool"), Some(Tool::MixerBrush));
        assert_eq!(Tool::from_name("colorReplacement"), Some(Tool::ColorReplacement));
        assert_eq!(Tool::from_name("Color Replacement Tool"), Some(Tool::ColorReplacement));
        assert_eq!(Tool::from_name("redEye"), Some(Tool::RedEye));
        assert_eq!(Tool::from_name("Red Eye Tool"), Some(Tool::RedEye));
        assert_eq!(Tool::from_name("artHistoryBrush"), Some(Tool::ArtHistoryBrush));
        assert_eq!(Tool::from_name("Art History Brush"), Some(Tool::ArtHistoryBrush));
        assert_eq!(Tool::from_name("Art History Brush Tool"), Some(Tool::ArtHistoryBrush));
        assert_eq!(Tool::RedEye.key(), 'J');
        assert!(!Tool::RedEye.is_brushlike());
        assert_eq!(Tool::from_name("patternStamp"), Some(Tool::PatternStamp));
        assert_eq!(Tool::from_name("Pattern Stamp Tool"), Some(Tool::PatternStamp));
        assert_eq!(Tool::ALL.len(), 68);
    }

    fn vector_tool_names_parse() {
        assert_eq!(Tool::from_name("Vertical Type Mask Tool"), Some(Tool::VerticalTypeMask));
        assert_eq!(Tool::from_name("addAnchorPoint"), Some(Tool::AddAnchorPoint));
        assert_eq!(Tool::from_name("Delete Anchor Point Tool"), Some(Tool::DeleteAnchorPoint));
        assert_eq!(Tool::DeleteAnchorPoint.key(), 'P');
        assert_eq!(Tool::from_name("convertPoint"), Some(Tool::ConvertPoint));
        assert_eq!(Tool::from_name("Convert Point Tool"), Some(Tool::ConvertPoint));
        assert_eq!(Tool::ConvertPoint.key(), 'P', "it shares the Pen's key");
        assert!(!Tool::ConvertPoint.is_brushlike());
        assert_eq!(Tool::from_name("Remove Tool"), Some(Tool::Remove));
        assert_eq!(Tool::Remove.key(), 'J');
        assert!(Tool::Remove.is_brushlike());
        assert_eq!(Tool::from_name("RotateView"), Some(Tool::RotateView));
        assert_eq!(Tool::from_name("Rotate View Tool"), Some(Tool::RotateView));
        assert_eq!(Tool::RotateView.key(), 'R');
        assert_eq!(Tool::ArtHistoryBrush.key(), 'Y');
        assert!(Tool::ArtHistoryBrush.is_brushlike());
        assert_eq!(Tool::from_name("nope"), None);
    }

    /// The README's "Everything in the box" tool heading and list name exactly the tools a user can
    /// pick, `Tool::ALL` (#996).
    #[test]
    fn readme_tool_list_matches_tool_all() {
        let readme = include_str!("../../../README.md");
        let (_, rest) = readme.split_once("<h4>🧰 ").unwrap();
        let (count, rest) = rest.split_once(" tools</h4>").unwrap();
        assert_eq!(count.parse::<usize>().unwrap(), Tool::ALL.len(), "README.md tool heading");
        let list = rest.trim_start().lines().next().unwrap();
        // "Rectangular and Elliptical Marquee" names two tools: "Rectangular Marquee", "Elliptical Marquee".
        let mut named: Vec<String> = Vec::new();
        for entry in list.split(" · ") {
            match entry.split_once(" and ").and_then(|(first, rest)| Some((first, rest.split_once(' ')?))) {
                Some((first, (second, noun))) => named.extend([format!("{first} {noun}"), format!("{second} {noun}")]),
                None => named.push(entry.to_owned()),
            }
        }
        named.sort();
        let mut labels: Vec<String> = Tool::ALL.iter().map(|tool| tool.label().trim_end_matches(" Tool").to_owned()).collect();
        labels.sort();
        assert_eq!(named, labels, "README.md tool list");
    }

    #[test]
    fn dialogs_open_and_close() {
        let mut s = UiState::default();
        let a = s.open_dialog(DialogKind::About, Default::default());
        let b = s.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
        assert_ne!(a, b);
        assert_eq!(s.dialog_mut(b).unwrap().fields["width"], 1920);
        assert!(s.close_dialog(a).is_some());
        assert_eq!(s.dialogs.len(), 1);
    }

    #[test]
    fn ui_state_serializes() {
        let s = UiState::default();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["tool"], "Brush");
        let back: UiState = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }
}
