//! PhotoCraft's first UI shell, built on egui/eframe.
//!
//! This crate is deliberately thin. Every action goes through
//! [`photocraft_engine::Session::execute`], and all UI state lives in [`state::UiState`] (plain
//! data). The [`control`] module exposes both to automation, so agents can drive and inspect every
//! part of the interface.
#![forbid(unsafe_code)]
// `control::inspect` builds the whole UI snapshot in one `json!`, past the default limit of 128.
#![recursion_limit = "256"]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// PhotoCraft as craft-update knows it, with its 128 px icon and accent colour: the update
/// prompt shows both, and an AppImage's menu entry gets the icon.
#[cfg(not(target_arch = "wasm32"))]
pub const UPDATE_APP: craft_update::app::App = craft_update::app::App {
    icon_png: Some(include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.photocraft.png")),
    accent: Some([0x2f, 0x7b, 0xf5]),
    ..craft_update::suite::PHOTOCRAFT
};

/// Translate a string literal into the current UI language: `tl!("Image Size…")`.
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}
mod brush_mode_menu;
pub mod canvas;
pub mod control;
pub mod dialogs;
pub mod dock;
pub mod file_dialog;
pub mod file_open;
pub mod frame_stages;
pub mod gpu_status;
pub mod i18n;
mod icon_data;
pub mod icons;
pub mod jobs_ui;
pub(crate) mod last_brush;
pub mod links;
pub mod menu_catalog;
pub mod menus;
pub mod monitor_status;
pub mod notices;
pub mod os_window;
pub mod panels;
pub mod preset_files_ui;
pub mod proxy;
pub mod render_path;
pub mod screenshot;
pub mod state;
pub mod texel_guard;
pub mod theme;
pub use titlebar::{Caption, CaptionFamily, CaptionIcon, CaptionIcons, CaptionStyle, RenderIconFn};
pub(crate) mod tool_memory;
pub mod updates_ui;
pub mod widgets;
pub mod work_area;

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

use photocraft_doc::{DocId, Document};
use photocraft_engine::Session;
use serde_json::{Value, json};

pub use control::{ControlRequest, ControlResponse};
pub use file_dialog::{FileDialogAnswer, FileDialogFn, FileDialogReply, FileDialogRequest};
pub use file_open::OsEvent;
pub use state::{Tool, UiState};

mod app;
mod input;
mod prefs;
mod services;
#[cfg(not(target_arch = "wasm32"))]
mod smart_svg_ui;
#[cfg(target_arch = "wasm32")]
#[path = "smart_svg_ui/browser.rs"]
mod smart_svg_ui;
mod text;
mod tools;

#[cfg(test)]
use app::layer_thumb_frame;
use app::read_dropped;
pub use app::surface_fingerprint;
pub(crate) use app::{adjustment_layer_active, remember_new_document};
use canvas::ants;
pub use canvas::artboard_ui;
pub(crate) use canvas::background_redraw;
pub(crate) use canvas::cam_preview;
pub use canvas::canvas_autoscroll;
use canvas::canvas_popup;
pub use canvas::canvas_tool_menu;
pub use canvas::channel_view;
pub use canvas::gpu_canvas;
pub use canvas::guide_select;
pub use canvas::layer_pick_ui;
pub use canvas::move_ui;
pub use canvas::outline;
pub use canvas::paint_mouse;
pub use canvas::patch_preview;
pub use canvas::pattern_preview;
pub use canvas::pixel_grid;
pub use canvas::quick_pick;
pub use canvas::rotate_view;
pub use canvas::rulers;
pub use canvas::scrollbars;
pub use canvas::select_mask_view;
pub use canvas::snap_ui;
use canvas::tile_overlay;
use canvas::tool_cursor;
pub use canvas::tool_feedback;
pub use canvas::wheel_nav;
pub use canvas::zoom_field;
pub use canvas::zoom_hold;
pub use canvas::zoom_levels;
pub use canvas::zoom_window_fit;
pub use dialogs::adjust_dialog;
pub use dialogs::adjust_editors;
pub use dialogs::adjust_preview;
pub(crate) use dialogs::bitmap_dialog;
pub use dialogs::blur_pins;
pub use dialogs::brush_preset_dialog;
pub use dialogs::caf_ui;
use dialogs::camera_raw_detail_ui;
use dialogs::camera_raw_gpu;
use dialogs::camera_raw_preview_ui;
use dialogs::camera_raw_scope_ui;
pub use dialogs::camera_raw_ui;
pub use dialogs::channel_options_ui;
pub(crate) use dialogs::color_picker_preview;
pub use dialogs::color_picker_ui;
pub use dialogs::color_range_ui;
pub use dialogs::color_table_ui;
pub use dialogs::credits;
pub use dialogs::dds_options_ui;
pub use dialogs::delete_layer_prompt;
pub use dialogs::dialog_alt_reset;
pub use dialogs::dialog_blend_ui;
pub use dialogs::discard_ui;
pub use dialogs::disk_change_prompt;
pub use dialogs::distort_ui;
pub use dialogs::duplicate_layer_ui;
pub use dialogs::edit_history;
pub use dialogs::export_dialog;
pub use dialogs::file_ui;
pub use dialogs::fill_ui;
pub use dialogs::filter_dialog;
use dialogs::filter_preview_ui;
pub use dialogs::gallery_ui;
pub use dialogs::guide_layout_ui;
pub use dialogs::history_options_ui;
pub use dialogs::jpeg_options_ui;
pub use dialogs::layer_picker;
pub use dialogs::layer_style;
pub use dialogs::liquify_ui;
pub use dialogs::mask_trash_prompt;
pub use dialogs::new_doc_ui;
pub use dialogs::new_group_ui;
pub use dialogs::new_guide_ui;
pub use dialogs::new_layer_ui;
pub use dialogs::open_prompt;
pub use dialogs::palette;
pub(crate) use dialogs::pattern_fill_ui;
pub use dialogs::preset_group_prompt;
pub use dialogs::rasterize_prompt;
pub use dialogs::replace_prompt;
use dialogs::rgb_histogram;
pub use dialogs::screen_picker;
pub use dialogs::select_mask_ui;
pub use dialogs::shape_dialog;
pub(crate) use dialogs::shape_fill_ui;
use dialogs::sizing;
pub use dialogs::smart_filter_dialog;
pub use dialogs::snapshot_dialog;
pub(crate) use dialogs::solid_fill_ui;
pub use dialogs::stack_dialog;
pub use dialogs::stroke_ui;
pub use dialogs::targa_options_ui;
pub use dialogs::tiff_options_ui;
pub use dialogs::vp_ui;
pub use dialogs::wide_angle_ui;
use input::alt_grab;
use input::blend_keys;
use input::control_click;
pub use input::hold_keys;
use input::layout_keys;
pub use input::nudge_bundle;
use input::opacity_keys;
use input::page_keys;
use input::scrub_wrap;
pub use input::shortcut_dispatch;
pub use input::shortcuts;
use input::single_line_drag;
pub use input::spring_tools;
pub use input::stylus;
use input::tool_shortcuts;
pub use input::touch_twins;
pub use menus::enable_rules;
pub use menus::menu_keys;
pub use menus::menu_nav;
pub use menus::native_menu;
pub use menus::parity;
pub use menus::plugin_ui;
pub use menus::view_cmds;
pub use menus::workspace_ui;
pub use panels::actions;
pub use panels::adjust_ui;
pub use panels::apply_color;
pub(crate) use panels::blend_preview;
use panels::brand;
pub use panels::brush_panel;
pub use panels::brush_preview;
pub use panels::brush_sections;
pub use panels::brushes_tab;
pub use panels::channels_panel;
pub use panels::chrome_ui;
pub use panels::clip_line_ui;
pub use panels::color_hud;
pub use panels::color_panel_ui;
pub use panels::comps_ui;
pub use panels::doc_props_ui;
pub use panels::histogram_panel;
pub use panels::history_menu_ui;
pub use panels::layer_filter_ui;
pub use panels::layer_menu_ui;
pub use panels::layer_props_ui;
use panels::layer_reveal;
pub use panels::layer_row_ui;
use panels::layer_transfer;
pub use panels::layer_tree_ui;
pub use panels::layers_panel_ui;
use panels::mask_props_ui;
pub use panels::mask_thumbs_ui;
use panels::panel_docking;
pub use panels::preset_panels;
pub use panels::props_layout;
pub use panels::smart_ui;
pub use panels::swatches_ui;
use panels::timeline_ui;
use panels::titlebar;
pub use panels::type_panels_ui;
use panels::variables_ui;
pub use prefs::flick_pan;
pub use prefs::kys_import;
pub use prefs::prefs_ui;
use prefs::pressure_curve_ui;
pub(crate) use services::SAVE_OPTIONS_OPEN;
#[cfg(test)]
pub(crate) use services::saved_revision_after_write;
pub use services::{
    AdoptAutosaveFn, AppendTextFn, AutomationCommandFn, AutomationOpenPcraftFn, AutomationReadFn, AutomationWriteFn, AutosaveFn, AutosaveResultsFn, CapsLockFn,
    CaptionStyleFn, ClipboardGetFn, ClipboardSetFn, ClipboardUnreadableFn, CursorPosFn, DiscardAutosaveFn, EncodePngFn, ExportFn, ExportSettings,
    ExternalSvgEditor, ExternalSvgSession, ExternalVectorSave, ExternalVectorSource, FileStamp, FileStampFn, FormatChoices, ImportFn, Inbox, LoadTextFn,
    OpenExternalSvgFn, OpenUrlFn, OsEventsFn, PhotoshopShortcutsFn, QuitFn, RecoverFn, Recoverable, RightButtonDownFn, SameFileFn, SaveFileFn, SaveTextFn,
    Services, SystemThemeFn, WindowFrameFn, WriteFn,
};
pub use text::bidi_text;
pub use text::cjk_fonts;
pub use text::direction;
pub use text::egui_fonts;
use text::font_filters;
use text::font_preview;
pub use text::served_fonts;
use tools::align_bar;
pub use tools::analysis_ui;
pub use tools::anchor_tools;
pub use tools::artboard_tool;
pub(crate) use tools::brush_cursor;
pub use tools::brush_picker;
pub use tools::brush_resize;
use tools::cam_transform;
pub use tools::cas_transform;
pub use tools::corner_radius;
pub use tools::crop_mode;
pub use tools::crop_overlay;
pub use tools::crop_presets;
pub use tools::crop_shield;
pub use tools::crop_size;
pub use tools::crop_straighten;
pub use tools::crop_ui;
pub use tools::direct_select;
pub use tools::eraser_ui;
pub use tools::eyedropper_ui;
pub use tools::frame_tool;
pub use tools::gradient_ui;
pub use tools::lasso_ui;
pub use tools::magnetic_lasso_ui;
pub use tools::move_box;
pub use tools::move_distance;
pub use tools::move_hover;
pub use tools::move_lock;
pub use tools::move_mods;
pub use tools::object_finder;
use tools::paint_picker;
use tools::path_components;
pub use tools::perspective_crop_ui;
pub use tools::perspective_ui;
pub use tools::puppet_ui;
pub use tools::retouch_ui;
use tools::shape_gradient_ui;
use tools::shape_options;
use tools::shape_paint_ui;
pub use tools::shape_stroke_ui;
pub use tools::slice_ui;
pub use tools::stroke_constraint;
pub use tools::stroke_trace;
pub use tools::stroke_trail;
pub use tools::symmetry_ui;
use tools::tool_reset;
pub use tools::transform_fx;
pub use tools::transform_tex;
pub use tools::transform_tool;
use tools::type_font_preview;
pub use tools::type_tool;
use tools::type_transform;
pub use tools::vector_ui;
pub use tools::warp_preview;
pub use tools::zoom_tool;
pub use widgets::field_tab;
pub use widgets::field_wheel;
use widgets::numeric_expression;
pub use widgets::point_curve;
pub use widgets::press_menu;
pub use widgets::swap_arrow;
use widgets::tab_strip;
pub use widgets::text_field_menu;
pub use widgets::tone;

/// The Info panel's cached sample: pixel x, y, document revision and Eyedropper Sample Size.
type InfoSampleKey = (i32, i32, u64, u32);

pub struct PhotocraftApp {
    pub session: Session,
    pub ui: UiState,
    pub services: Services,
    /// Canvas caches per (document, display): CPU textures hold monitor values; the GPU
    /// canvas state is shared (`canvas::GPU_OUTPUT`).
    canvases: HashMap<(DocId, u32), canvas::CanvasCache>,
    /// Navigator textures belong to open documents, not the lifetime of the egui context.
    navigator_textures: HashMap<DocId, canvas::NavigatorCache>,
    /// `canvas::display_doc`'s live-adjust and Duotone documents per document: (snapshot,
    /// preview key, the document shown).
    display_memo: HashMap<DocId, (std::sync::Weak<Document>, u64, std::sync::Arc<Document>)>,
    /// The main canvas's last GPU view of each document while it shows the plain document on
    /// the GPU canvas: the Navigator draws the same texture at its own size (#4518).
    navigator_gpu: HashMap<DocId, crate::gpu_canvas::ViewParams>,
    /// Documents whose GPU canvas texture holds the plain document (`canvas::ensure_gpu`).
    navigator_plain: std::collections::HashSet<DocId>,
    /// The Info panel's Color Sampler readouts for (document, revision) (`panels::sampler_readouts`).
    info_samplers: Option<panels::SamplerReadouts>,
    /// Display profile readings (#569).
    monitors: monitor_status::State,
    checker: Option<egui::TextureHandle>,
    drag: Option<canvas::Drag>,
    /// An owned vector drag was cancelled (`canvas::cancel_stale_vector_drag`): the rest of its
    /// pointer events go nowhere.
    pub(crate) cancelled_vector_drag: bool,
    /// A Move-tool press landed on a locked layer: the first pointer move shows Photoshop's
    /// message (`move_lock`), a plain click shows nothing.
    pub(crate) move_blocked: bool,
    /// The tool pointer events go to this frame when it isn't the selected one: the Move tool
    /// while ⌘ is held (`hold_keys::cmd_moves`). Set by the canvas for its gestures, never saved.
    pub(crate) tool_override: Option<state::Tool>,
    /// Brush/Eraser stroke being drawn, rendered by the engine (see `canvas::LiveStroke`).
    live_stroke: Option<canvas::LiveStroke>,
    /// A live stroke just committed with its own pixels: the canvas caches that showed it
    /// refresh only what it drew after the step they showed (`canvas::damage_since`).
    committed_stroke: Option<canvas::CommittedStroke>,
    /// Whether the last retouching stroke's commit took the pixels its live stroke drew
    /// (`"fromLive"`) instead of rendering the stroke again (`retouch_ui::finish_stroke`).
    pub(crate) retouch_from_live: bool,
    /// Footprint trail of a retouching drag (see `stroke_trail`).
    trail: Option<stroke_trail::Trail>,
    /// Remove tool strokes waiting to be applied together (Remove after each stroke off), the
    /// document, layer and channel they belong to, and their tint on the canvas (`retouch_ui`).
    /// Each as a `paint.remove` `strokes` entry: its points and the size, hardness and spacing it
    /// was drawn with, which its tint shows and its removal uses.
    pub(crate) remove_pending: Vec<serde_json::Value>,
    pub(crate) remove_pending_target: Option<retouch_ui::RemoveTarget>,
    pub(crate) remove_trail: Option<stroke_trail::Trail>,
    /// The open menus' items, reused between frames (`menus::ItemCache`).
    pub(crate) menu_cache: Option<menus::ItemCache>,
    /// Move tool drag shown live (`move_ui`).
    pub(crate) move_preview: Option<move_ui::MovePreview>,
    /// Move tool › Show Highlight on Rollover: the last hover answer (`move_hover`).
    pub(crate) move_hover: Option<move_hover::Hover>,
    /// Smart Guides distances shown with ⌘ held over the canvas: the last answer (`move_distance`).
    pub(crate) move_distance: Option<move_distance::Cache>,
    /// Object Selection › Object Finder: the hover preview (`object_finder`).
    pub(crate) object_finder: Option<object_finder::Finder>,
    /// A blend mode hovered in the Layers panel, shown live (`blend_preview`).
    pub(crate) blend_preview: Option<blend_preview::BlendPreview>,
    /// Layers panel drag: temporary reordered document displayed until mouse release.
    pub(crate) reorder_preview: Option<layers_panel_ui::ReorderPreview>,
    /// Whether the currently dragged layer is over a valid destination this frame.
    pub(crate) reorder_preview_hit: bool,
    /// Patch Tool drag: the healed document at the pointer (`patch_preview`).
    pub(crate) patch_preview: Option<patch_preview::PatchPreview>,
    /// Content-Aware Move drag or box shown with its result (`cam_preview`).
    pub(crate) cam_preview: Option<cam_preview::CamPreview>,
    /// The pixels a Magnetic Lasso border follows (`magnetic_lasso_ui`).
    pub(crate) magnetic: magnetic_lasso_ui::Runtime,
    /// The next tool `Down` is a right-button drag that erases (see `paint_mouse`).
    secondary_erase: bool,
    /// The Brush or Pencil erase key is held this frame (`hold_keys::erase_held`, #3439).
    held_erase: bool,
    /// While a batch of recovered pointer samples is replayed, defer the live-stroke update to one
    /// call for the whole frame (see `canvas::canvas_view`).
    defer_live_stroke: bool,
    /// A live painting stroke started on the press (`canvas_view`): the drag egui recognises later,
    /// or the click, continues or ends it rather than starting another.
    press_stroke: bool,
    /// Position and pen pressure at the last stroke's endpoint: ⇧-click interpolates from them (#178, #2827).
    last_stroke_end: Option<(DocId, [f64; 3])>,
    /// Control+Alt-drag brush resize in progress (`brush_resize`, #231).
    pub(crate) brush_resize: Option<brush_resize::Resize>,
    /// A ⌘⌥⌃-click layer pick is in progress; its drag and release are swallowed (`quick_pick`).
    pub(crate) quick_pick: bool,
    /// The next tool `Down` is an Alt+right-drag that resizes the brush (#297). `tool_event`
    /// takes it on every event, so a press another handler consumes can't leave it set.
    pub(crate) brush_resize_armed: bool,
    /// This press began with ⌥ (Alt) held on a painting tool, so it samples colours instead of
    /// painting until it is released (`canvas::alt_eyedropper`, #417).
    pub(crate) alt_sampling: bool,
    /// This press began with ⌥ (Alt) held on the Mixer Brush, so it loaded paint from the canvas
    /// and paints nothing until it is released, even with ⌥ let go (`canvas::tool_event`).
    pub(crate) mixer_sampling: bool,
    /// Caps Lock toggled on, read from the OS each frame (`services.caps_lock`): painting tools
    /// show the precise crosshair whatever the cursor preference (#1758).
    pub caps_lock: bool,
    /// The first digit of a two-digit opacity typed on the number keys (`opacity_keys`, #352).
    pub(crate) opacity_keys: opacity_keys::Pending,
    control_rx: Option<Receiver<ControlRequest>>,
    /// `ui.screenshot` requests: bounded admission, a weak-slot callback per issuance, monotonic
    /// deadlines and a capture-time geometry epoch (see `screenshot.rs`).
    captures: screenshot::Captures,
    /// The custom Type tool's IME owner/generation/receipt tracker (`type_tool::sync_ime`).
    pub(crate) type_ime: type_tool::ImeState,
    /// Control replies waiting for queued synthetic input to be processed.
    input_waiters: Vec<Sender<ControlResponse>>,
    /// Live (uncommitted) adjustment edit shown on canvas while a slider is dragged.
    pub live_adjust: Option<(photocraft_doc::LayerId, Value)>,
    /// Frames rendered (for tests and the status bar).
    pub frame: u64,
    /// The pointer rested on the notice stack last frame. Used to give a fresh auto-hide delay in
    /// the frame the pointer leaves, so a long stationary hover never counts as elapsed time
    /// (#2022); set by `notices::show`.
    pub(crate) notices_hovered: bool,
    /// Apply theme on first frame.
    styled: bool,
    /// Whether the window uses an integrated (transparent) macOS title bar.
    pub integrated_titlebar: bool,
    /// Windows and Linux: the window has no OS decorations and the app's top bar is the title bar
    /// (caption buttons, window dragging and edge resizing, `titlebar`).
    pub custom_titlebar: bool,
    /// Last window title sent to the OS (`ViewportCommand::Title`, see `panels::sync_window_title`):
    /// sent again only when it changes, so idle frames don't spam the backend.
    last_window_title: String,
    fonts_ready: bool,
    /// Screen rect of the main canvas last frame (for overlays and the navigator).
    pub last_canvas_rect: egui::Rect,
    /// Window › Arrange: the first document of the shown run of tiles, when there are more
    /// documents than tiles (`canvas::arranged_cells`).
    pub(crate) arrange_start: usize,
    /// Physical pixels per egui point of the canvas last frame (`ctx.pixels_per_point`, which
    /// folds in both the display scale and Interface › UI Scale). The canvas maps document pixels
    /// to physical pixels, so point-space geometry divides the view zoom by this (see
    /// [`Self::point_zoom`]).
    pub ppp: f32,
    /// The document area showing the active document's canvas last frame (not the tabs, the
    /// start screen or an opening file's card): files dropped here are placed as layers.
    pub(crate) drop_canvas_rect: Option<egui::Rect>,
    /// The document tab strip last frame: a drop there opens the file at the slot under it.
    pub(crate) tab_strip: Option<canvas::TabStrip>,
    /// Files dropped on the canvas still to place, one Free Transform at a time.
    pub(crate) drop_places: std::collections::VecDeque<file_open::DropPlace>,
    /// The document each of `ui.views` belongs to, as of the last [`Self::sync_views`].
    view_docs: Vec<DocId>,
    /// The document `ui.selected_path` belongs to, as of the last [`Self::sync_views`]: a Paths
    /// panel selection never carries over to a same-named path in another tab.
    path_document: Option<(DocId, u64, std::sync::Weak<Document>)>,
    pub fps: f32,
    pub(crate) last_frame_time: f64,
    thumbs: HashMap<(photocraft_doc::LayerId, u8), (u64, egui::TextureHandle)>,
    /// The document area each layer thumbnail in `thumbs` frames (the canvas, or the layer's
    /// bounds with Clip Thumbnails to Layer Bounds; #4281).
    thumb_frames: HashMap<photocraft_doc::LayerId, photocraft_geom::Rect>,
    /// History snapshot thumbnails by (document, snapshot id): a snapshot never changes, so each
    /// is made once (`snapshot_thumb`, #2966).
    snapshot_thumbs: HashMap<(DocId, u64), egui::TextureHandle>,
    /// Snapshots whose live layer/mask keys were last used to prune thumbnail handles.
    thumb_documents: Vec<(DocId, std::sync::Weak<Document>)>,
    /// Content bounds cached per (key, revision): scanning a 36 MP layer every frame cost ~77 ms.
    bounds_cache: HashMap<u64, (u64, photocraft_geom::Rect)>,
    /// Downsampled proxy of the active document for live previews: (doc, revision, k, proxy).
    pub(crate) proxy: Option<(DocId, u64, u32, std::sync::Arc<Document>)>,
    /// Key of the preview currently uploaded to the GPU (doc, params hash).
    pub(crate) proxy_uploaded: Option<(DocId, u64)>,
    /// Selection outline cache: (doc, revision, segments).
    /// Live filter preview (proxy document with the filter applied).
    pub(crate) filter_preview: Option<filter_dialog::FilterPreview>,
    pub(crate) smart_filter_targets: smart_filter_dialog::Targets,
    #[cfg(not(target_arch = "wasm32"))]
    filter_preview_worker: crate::dialogs::filter_preview_worker::Worker,
    /// Full CPU canvas redraws running in the background, by document (#4024).
    pub(crate) redraws: std::collections::HashMap<photocraft_doc::DocId, background_redraw::Redraw>,
    /// The last finished background redraws, for going back to a state drawn a moment ago (#6528).
    pub(crate) kept: background_redraw::KeptComposites,
    /// Select › Color Range dialog preview (proxy document + mask / image textures).
    pub(crate) color_range: Option<color_range_ui::Preview>,
    /// Image › Adjustments dialog preview through a temporary clipped adjustment layer.
    pub(crate) adjust_preview: Option<adjust_preview::AdjustPreview>,
    /// The Properties panel's live adjustment-layer drag: its damage region and coverage
    /// (`adjust_preview::live_key`).
    pub(crate) live_adjust_preview: Option<adjust_preview::LiveAdjust>,
    /// Synthetic input events queued by automation (`ui.click`, `ui.key`, …), injected next frame.
    pub(crate) synthetic: Vec<egui::Event>,
    /// The one synthetic batch currently being delivered.
    ///
    /// eframe preserves input appended by `raw_input_hook` across hidden (`update_logic_only`)
    /// ticks, so draining another step every hidden tick would deliver several host iterations
    /// as one burst. At most one batch is ever in flight; the next step waits for a pass.
    synthetic_batch: Option<Vec<egui::Event>>,
    /// A pass consumed the batch but returned before handling input (fonts not ready): put the
    /// same events back on the next hook so the step is not lost. Cleared as soon as they are
    /// re-appended, so two hidden ticks cannot deliver the same step twice.
    synthetic_reinject: bool,
    /// Document/focus identity the batch was created for. A batch queued against one document is
    /// cancelled, not replayed, after the active document (including a switch away and back) or
    /// the focus changes; an edit in the same document does not cancel it.
    synthetic_snapshot: Option<SyntheticTarget>,
    /// True only while a frame is processing synthetic automation input. It
    /// makes shortcut- and dialog-triggered commands pass the same policy as
    /// direct control calls.
    pub(crate) automation_input: bool,
    /// How many `run_command` calls are on the stack. Only the outermost one writes the frame's
    /// command stage: a nested run (`file.save` → `layer.smartObjects.saveContents`,
    /// `actions.play` → its steps) is already inside the outer call's time.
    run_depth: u32,
    /// Action playbacks paused at a step dialog, by the token in that dialog's fields
    /// ([`actions::RESUME`]): what to play next and the automation gate it paused under.
    pub(crate) paused_playbacks: actions::PausedPlaybacks,
    /// Levels/Curves histogram cache: (document, adjustment layer, revision it is valid for).
    pub(crate) tone_hist: Option<(DocId, photocraft_doc::LayerId, u64, std::sync::Arc<tone::Histograms>)>,
    /// Histogram panel: the document's histogram (`histogram_panel`).
    pub(crate) doc_hist: Option<histogram_panel::PanelHist>,
    /// Histogram panel: the histogram of the adjustment previewing on the canvas.
    pub(crate) doc_hist_preview: Option<histogram_panel::PanelHist>,
    /// The panel histogram being computed on a worker thread.
    pub(crate) hist_job: Option<histogram_panel::HistJob>,
    /// The (document, revision) an Uncached Refresh asked to count at cache level 1.
    pub(crate) hist_uncached: Option<(DocId, u64)>,
    /// Free Transform preview (document without the moving pixels + their texture).
    pub(crate) transform_preview: Option<transform_tool::TransformPreview>,
    /// Move-tool ⇧/⌥ drag state (move_mods).
    pub(crate) move_mods: move_mods::MoveDrag,
    /// Cached document for a modal text Style Options color preview.
    pub(crate) text_style_preview: Option<type_panels_ui::color_picker::Preview>,
    /// The run of arrow-key nudges that share one History state (nudge_bundle).
    pub(crate) nudge_bundle: nudge_bundle::NudgeBundle,
    /// Live Layer Style dialog preview: (key over revision + style fields, preview or validation error).
    pub(crate) style_preview: Option<(u64, Result<std::sync::Arc<Document>, String>)>,
    pub(crate) solid_fill_preview: Option<solid_fill_ui::Preview>,
    /// Live Pattern Fill dialog preview (pattern_fill_ui).
    pub(crate) pattern_fill_preview: Option<pattern_fill_ui::Preview>,
    /// Shape fill Color Picker preview (shape_fill_ui).
    pub(crate) shape_fill_preview: Option<shape_fill_ui::Preview>,
    /// Colours a colour swatch widget's Color Picker returned with OK, waiting for the widget to
    /// take them next frame (`color_picker_ui::widget_tick`): (widget id, sRGB).
    pub(crate) widget_colors: Vec<(u64, [f32; 3])>,
    /// Liquify dialog, Puppet Warp and Perspective Warp sessions (distort_ui).
    pub(crate) distort: distort_ui::Distort,
    /// Gradient tool live-mode drags and previews (gradient_ui).
    pub(crate) gradient: gradient_ui::LiveGradient,
    /// Filter › Camera Raw Filter dialog (camera_raw_ui).
    pub(crate) camera_raw: Option<camera_raw_ui::CameraRawDialog>,
    /// A raw file just opened interactively, waiting for the open-time Camera Raw dialog (shown
    /// on the next frame, which has the egui context).
    pub(crate) pending_raw_open: Option<camera_raw_ui::RawOpen>,
    /// The open-time re-develops (or their Camera Raw steps) running in the background, one per
    /// raw being opened.
    pub(crate) raw_redevelop: Vec<camera_raw_ui::Redevelop>,
    /// Filter › Adaptive Wide Angle dialog (wide_angle_ui).
    pub(crate) wide_angle: Option<wide_angle_ui::WideAngleDialog>,
    /// ⌘Z history of each open parameter dialog (edit_history).
    pub(crate) dialog_edits: edit_history::DialogEdits,
    /// The filter a "Rasterize?" prompt answered for, run after the dialog closes, where the
    /// egui context is available again (rasterize_prompt).
    pub(crate) pending_after_prompt: Option<(String, serde_json::Value)>,
    /// Last mirrored or imported OS image; this can predate the current internal copy.
    os_clip_sig: Option<u64>,
    /// The session copy, if any, need not track the OS clipboard. An older OS signature is kept
    /// to recognize unchanged system contents when exporting a newer copy failed or was disabled.
    clip_unmirrored: bool,
    /// The session clipboard currently holds an image imported from the OS clipboard: it has no
    /// original position, so Paste centres it (see `menus` "edit.paste").
    pub(crate) clip_external: bool,
    /// The OS clipboard was already read for the paste in flight (Edit › Paste positions it first),
    /// so `run` doesn't read it a second time.
    pub(crate) clip_read_for_paste: bool,
    pub(crate) smart_svg: smart_svg_ui::State,
    /// Pointer position over the canvas (document px), for the Info panel and status bar.
    pub(crate) hover_doc: Option<[f64; 2]>,
    /// Layer hovered in the Layers panel, outlined on the canvas.
    pub(crate) hover_layer: Option<photocraft_doc::LayerId>,
    /// Layer hovered on the canvas, highlighted in the Layers panel.
    pub(crate) canvas_hover_layer: Option<photocraft_doc::LayerId>,
    pub(crate) clone_preview: Option<crate::canvas::ClonePreviewCache>,
    /// Info panel sample cache: ((x, y, revision, Sample Size), composite RGBA).
    info_sample: Option<(InfoSampleKey, [f32; 4])>,
    /// Guide being dragged (from a ruler or with the Move tool).
    pub(crate) guide_drag: Option<rulers::GuideDrag>,
    /// Move tool marquee selecting layers (`move_box`).
    pub(crate) layer_box: Option<move_box::LayerBox>,
    /// Crop tool gesture in progress (see `crop_ui`).
    pub(crate) crop: crop_ui::CropState,
    /// Perspective Crop tool gesture and clicked corners (see `perspective_crop_ui`).
    pub(crate) pcrop: perspective_crop_ui::PerspectiveCrop,
    /// Type tool layout cache: ((doc, revision, layer), layout).
    pub(crate) type_layout: Option<((u64, u64, u64), std::sync::Arc<photocraft_text::TextLayout>)>,
    /// Type tool: (layer, clicks in the current run) for word / line / paragraph / all selection.
    pub(crate) type_clicks: Option<(u64, u32)>,
    pub(crate) type_transform_preview: Option<type_transform::Preview>,
    /// A font hovered in the Type tool's font picker, shown live (`type_font_preview`).
    pub(crate) type_font_preview: Option<type_font_preview::TypeFontPreview>,
    /// The colour under a text or gradient stop Color Picker, shown live before OK (`color_picker_preview`).
    pub(crate) color_picker_preview: Option<color_picker_preview::Preview>,
    /// Channel thumbnails for one document snapshot; view-only revisions reuse their pixels.
    channel_thumbs: Option<(DocId, std::sync::Weak<Document>, bool, Vec<egui::TextureHandle>)>,
    /// Channels panel overlays / channel views drawn over the canvas, per document id.
    pub(crate) channel_views: HashMap<u64, channel_view::Cache>,
    /// Selection outline keyed by (document, mask identity × step × visible region).
    pub(crate) outline_cache: Option<(DocId, u64, std::sync::Arc<Vec<outline::Segment>>)>,
    /// GPU canvas renderer, when running on the wgpu backend (see [`Self::set_wgpu`]). Dropped
    /// for the rest of the session when the device is lost (see `gpu_status`).
    gpu: Option<gpu_canvas::GpuCanvas>,
    /// The device the window itself draws with, and its health flag: the GPU canvas's, or one
    /// watched for the CPU canvas (see `gpu_status::check_window`).
    window_device: Option<(eframe::wgpu::Device, photocraft_gpu::DeviceHealth)>,
    /// That device was lost: frames draw nothing and the app closes once its work is saved (#3438).
    window_lost: bool,
    /// When (in `egui::InputState::time`) to try saving unsaved work for recovery again after a
    /// failed attempt; the app closes only once it succeeded.
    window_recovery_retry: Option<f64>,
    /// The render state the GPU canvas was built on, kept after it is dropped so Retry GPU can
    /// build a new one on the same device (see `gpu_status::choose_recovery`, #6577).
    gpu_render_state: Option<eframe::egui_wgpu::RenderState>,
    /// Why the GPU canvas was dropped (see `gpu_status::fall_back`).
    gpu_fault: Option<photocraft_gpu::Fault>,
    /// Out-of-memory errors this session after which the GPU canvas was rebuilt by itself (#6564).
    gpu_oom_retries: u32,
    /// Run once the first frames have rendered (see [`Self::on_started`]).
    started: Option<gpu_status::StartedHook>,
    /// Frame and canvas-upload timings (exposed via `ui.inspect`).
    pub perf: gpu_canvas::Perf,
    /// Per-frame stroke timings for latency reports (`PHOTOCRAFT_STROKE_TRACE`, see `stroke_trace`).
    pub stroke_trace: stroke_trace::StrokeTrace,
    /// Why each document's view is drawn on the CPU, and the history of changes (`render_path`).
    pub render_path: render_path::RenderPath,
    /// The last slow frames and their stages (`perf.slowFrames`, see `frame_stages`).
    pub slow_frames: frame_stages::SlowFrames,
    /// Preferences, autosave and snapping runtime state (see `prefs_ui`, `snap_ui`).
    pub(crate) prefs_rt: prefs_ui::Runtime,
    pub(crate) updates_rt: updates_ui::Runtime,
    /// Close, Revert or Exit parked behind the unsaved-changes prompt (see `discard_ui`).
    pub(crate) discard: Option<discard_ui::Prompt>,
    /// The Open or Save dialog in progress, and the action waiting on it (see `file_dialog`).
    pub(crate) file_dialog: Option<file_dialog::Pending>,
    /// A Save As to a layered TIFF parked behind the TIFF Options prompt (see `tiff_options_ui`).
    pub(crate) tiff_options: Option<tiff_options_ui::Prompt>,
    /// Where each document was last saved or exported to through a dialog (#1826): its next Save
    /// As or export dialog starts there rather than beside the document. Session-only.
    pub(crate) save_dirs: HashMap<photocraft_doc::DocId, std::path::PathBuf>,
    /// Each document's own file as it was when the document was opened from it or saved to it
    /// (see [`FileStamp`]).
    pub(crate) disk_stamps: HashMap<photocraft_doc::DocId, (String, FileStamp)>,
    /// A save to an RGB Targa parked behind the Targa Options prompt (see `targa_options_ui`).
    pub(crate) targa_options: Option<targa_options_ui::Prompt>,
    /// A save to a JPEG parked behind the JPEG Options prompt (see `jpeg_options_ui`).
    pub(crate) jpeg_options: Option<jpeg_options_ui::Prompt>,
    /// A save to a DDS texture parked behind the DDS Options prompt (see `dds_options_ui`).
    pub(crate) dds_options: Option<dds_options_ui::Prompt>,
    /// Files read but not opened yet: the first waits on its profile prompt, the rest behind it
    /// (see `open_prompt`).
    pub(crate) held_opens: std::collections::VecDeque<open_prompt::Held>,
    /// A brush or gradient file waiting on the group-name prompt (see `preset_group_prompt`).
    pub(crate) held_preset_import: Option<preset_group_prompt::Held>,
    /// Set once the user has agreed to quit, so the resulting close request goes through.
    pub(crate) allow_close: bool,
    /// Pen pressure/tilt from the platform (see `stylus`).
    pub stylus: stylus::Stylus,
    control_click: control_click::ControlClick,
    /// Run long commands and file opens as background jobs with progress and Cancel (#210; see
    /// `jobs_ui`). The desktop app turns it on; off (the default), everything runs inline as
    /// before, which tests and scripts rely on.
    pub background_jobs: bool,
    /// Shortest time between frames (ms), or `None` for no cap. A present mode that doesn't wait
    /// for the display (Low-Latency Drawing) would otherwise redraw as fast as it can while
    /// anything keeps requesting repaints (a drag hovering, autoscroll, Liquify held still).
    /// After #4472 (astigmata).
    pub min_frame_ms: Option<f64>,
    last_frame_start_ms: f64,
    /// Background job bookkeeping: opening tabs, control replies waiting on a job.
    pub jobs: jobs_ui::JobsUi,
    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
    live_tokens: theme::live::LiveTokens,
}

/// The state a synthetic input batch was queued for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SyntheticTarget {
    document: Option<screenshot::DocumentId>,
    focus: Option<egui::Id>,
}

impl SyntheticTarget {
    /// The document must be the same one in the same activation (a switch away and back is a
    /// change); its revision is attribution only, so an edit does not cancel a queued step.
    fn matches(self, other: Self) -> bool {
        self.focus == other.focus
            && match (self.document, other.document) {
                (Some(a), Some(b)) => a.same_activation(b),
                (None, None) => true,
                _ => false,
            }
    }
}

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod screenshot_tests;

#[cfg(test)]
mod navigator_gpu_tests;

#[cfg(test)]
mod color_swatch_tests;

#[cfg(test)]
mod profile_open_tests;

#[cfg(test)]
mod profile_prompt_tests;

#[cfg(test)]
mod input_tests;

#[cfg(test)]
mod empty_workspace_tests;

#[cfg(test)]
mod pen_variants_tests;

#[cfg(test)]
mod color_replacement_tests;
#[cfg(test)]
mod layer_shortcut_tests;
#[cfg(test)]
mod live_retouch_tests;
#[cfg(test)]
mod pencil_tests;
#[cfg(test)]
mod tool_options_theme_tests;
#[cfg(test)]
mod tool_shortcut_tests;

#[cfg(test)]
mod pixel_aspect_tests;

#[cfg(test)]
mod transform_fx_tests;

#[cfg(test)]
mod layer_props_ui_tests;
#[cfg(test)]
mod notice_text_tests;
#[cfg(test)]
mod transform_switch_tests;
#[cfg(test)]
mod transform_undo_tests;

#[cfg(test)]
mod transform_type_tests;

#[cfg(test)]
mod transform_several_tests;

#[cfg(test)]
mod psd_open_tests;
#[cfg(test)]
mod save_identity_tests;

#[cfg(test)]
mod move_auto_select_tests;

#[cfg(test)]
mod new_group_button_tests;

#[cfg(test)]
mod color_panel_tests;
#[cfg(test)]
mod color_tab_theme_tests;

#[cfg(test)]
mod new_layer_button_tests;

#[cfg(test)]
mod smart_filter_rows_tests;

#[cfg(test)]
mod blur_pins_tests;

#[cfg(test)]
mod move_outline_tests;

#[cfg(test)]
mod layer_thumb_bounds_tests;
#[cfg(test)]
mod mask_thumb_refresh_tests;

#[cfg(test)]
mod new_doc_remember_tests;

#[cfg(test)]
mod hidden_layer_tests;

#[cfg(test)]
mod blend_dropdown_keys_tests;

#[cfg(test)]
mod warp_text_dialog_tests;

#[cfg(test)]
mod blend_dropdown_wheel_tests;
#[cfg(test)]
mod paths_scroll_tests;

#[cfg(test)]
mod options_bar_mode_wheel_tests;

#[cfg(test)]
mod field_wheel_app_tests;

#[cfg(test)]
mod dropdown_press_tests;

#[cfg(test)]
mod marquee_tests;

#[cfg(test)]
mod guide_cursor_tests;

#[cfg(test)]
mod canvas_menu_tests;

#[cfg(test)]
mod nudge_bundle_tests;

#[cfg(test)]
mod caps_lock_tests;

#[cfg(test)]
mod view_sync_tests;

#[cfg(test)]
mod stamp_tests;

#[cfg(test)]
mod alt_click_tests;
#[cfg(test)]
mod status_bar_tests;
#[cfg(test)]
mod stroke_timing_tests;

#[cfg(test)]
mod polygon_lasso_tests;

#[cfg(test)]
mod window_frame_tests;

#[cfg(test)]
mod browse_field_tests;
#[cfg(test)]
mod options_hit_area_tests;
#[cfg(test)]
mod select_and_mask_button_tests;

#[cfg(test)]
mod saved_path_edit_tests;

#[cfg(test)]
mod pen_path_ownership_tests;
