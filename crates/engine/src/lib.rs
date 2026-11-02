//! The PhotoCraft engine façade: open documents, history, and the command registry.
//!
//! Every user-visible action is a command with a stable id (Photoshop-style, such as
//! `layer.newAdjustmentLayer.invert`) and JSON parameters. Every frontend goes through the same
//! [`Session::execute`] entry point: the egui UI, the CLI, the remote-control channel and the MCP
//! server. This is what makes the UI swappable and the app fully scriptable.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod adjust_params;
mod allocation;
pub mod build_info;
mod canvas_geom;
mod channel_clip;
mod clip_group;
pub mod color_channels;
mod crop_fill;
pub mod display_color;
pub mod doc_path;
pub mod filters;
pub mod filters_ext;
mod group_options;
pub mod guide_layout;
pub mod hidden_target;
pub mod inspect;
pub mod installed_profiles;
pub mod jobs;
pub mod kys;
pub mod layer_place;
pub mod layer_style;
pub mod link_watch;
pub mod new_layer;
mod param_check;
mod pixels;
pub mod prefs;
pub mod preset_store;
pub mod presets;
pub mod proof_sim;
pub mod select_mask_edits;
pub mod shell_commands;
pub mod shortcut_sets;
pub mod snap;
pub use variables_cmds::{export_data_sets_with, import_data_sets_text};
pub use video_cmds::render_video_with;

use std::sync::Arc;

use photocraft_doc::{Document, LayerId};
use photocraft_ops::{History, LayerTarget};
use serde_json::Value;

pub use commands::{CommandSpec, command_specs};
pub use photocraft_doc as doc;
pub use photocraft_paint as paint;
pub use photocraft_paint::BrushSettings;

pub mod cmd;
mod doc_state;
mod error;
mod session;

pub use cmd as commands;
pub use cmd::actions_cmds;
pub use cmd::actions_edit_cmds;
pub use cmd::adjust_cmds;
pub use cmd::align_cmds;
pub use cmd::analysis_cmds;
pub use cmd::artboard_cmds;
pub use cmd::atn_cmds;
pub use cmd::automate_cmds;
pub use cmd::background_cmds;
pub use cmd::brush_cmds;
pub use cmd::brush_key_cmds;
pub use cmd::brush_preset_cmds;
pub use cmd::channel_cmds;
pub use cmd::color_cmds;
pub use cmd::color_to_alpha_cmds;
pub use cmd::comps_cmds;
pub use cmd::contour_lib_cmds;
pub use cmd::cutout_cmds;
pub use cmd::distort_cmds;
pub use cmd::document_preset_cmds;
pub use cmd::edit_cmds;
pub use cmd::edit_menu_cmds;
pub use cmd::eraser_cmds;
pub use cmd::export_preset_cmds;
pub use cmd::exr_cmds;
pub use cmd::extra_cmds;
pub use cmd::face_cmds;
pub use cmd::file_cmds;
pub use cmd::fill_cmds;
pub use cmd::fill_key_cmds;
pub use cmd::float_cmds;
pub use cmd::font_library_cmds;
use cmd::frame_cmds;
pub use cmd::fx_view_cmds;
pub use cmd::fx_visibility_cmds;
pub use cmd::gallery_cmds;
pub use cmd::gradient_fill_cmds;
pub use cmd::group_view_cmds;
pub use cmd::history_cmds;
pub use cmd::history_menu_cmds;
pub use cmd::history_options_cmds;
pub use cmd::history_select_cmds;
pub use cmd::history_source_cmds;
pub use cmd::image_cmds;
pub use cmd::layer_copy_cmds;
pub use cmd::layer_label_cmds;
pub use cmd::layer_list_cmds;
pub use cmd::layer_mask_props_cmds;
pub use cmd::layer_menu_cmds;
pub use cmd::layer_multi_cmds;
pub use cmd::layer_nav_cmds;
pub use cmd::layer_xmp_cmds;
pub use cmd::lens_cmds;
pub use cmd::magnetic_cmds;
pub use cmd::mask_view_cmds;
use cmd::migrate_cmds;
pub use cmd::mode_cmds;
pub use cmd::multichannel_cmds;
pub use cmd::notes_cmds;
pub use cmd::paint_cmds;
use cmd::path_edit_cmds;
pub use cmd::pattern_cmds;
pub use cmd::pattern_fill_cmds;
pub use cmd::perspective_crop_cmds;
pub use cmd::photo_cmds;
pub use cmd::pick_cmds;
pub use cmd::plugin_cmds;
pub use cmd::preset_import_cmds;
pub use cmd::print_cmds;
pub use cmd::rasterize_style_cmds;
pub use cmd::redeye_cmds;
pub use cmd::render_cmds;
pub use cmd::retouch_cmds;
pub use cmd::sample_cmds;
pub use cmd::sampler_cmds;
pub use cmd::select_extra_cmds;
pub use cmd::selection_cmds;
pub use cmd::slice_cmds;
pub use cmd::smart_cmds;
pub use cmd::smart_svg_cmds;
pub use cmd::smartselect_cmds;
pub use cmd::solid_fill_cmds;
pub use cmd::stamp_cmds;
pub use cmd::style_asl_cmds;
pub use cmd::swatch_cmds;
pub use cmd::symmetry_cmds;
use cmd::timeline_cmds;
pub use cmd::transform_cmds;
use cmd::trap_cmds;
pub use cmd::type_area_cmds;
pub use cmd::type_caret_cmds;
pub use cmd::type_cmds;
pub use cmd::type_extra_cmds;
pub use cmd::type_mask_cmds;
pub use cmd::type_spell_cmds;
pub use cmd::type_styles_cmds;
pub use cmd::updates_cmds;
use cmd::variables_cmds;
pub use cmd::vector_cmds;
use cmd::video_cmds;
pub use cmd::vp_cmds;
pub use cmd::warp_cmds;
pub use cmd::web_cmds;
use cmd::wia_cmds;
pub use doc_state::{CopiedStyle, DocState, PRECHECKED, ToolState};
pub use error::{EngineError, Result};
pub use session::move_item;
use session::restore_target;
pub(crate) use session::{carry_visibility, fix_selection, visibility_changes};

/// The pixels of a raster layer (typically one a command just created), as an error instead
/// of a panic if the layer has none.
pub(crate) fn pixels_mut(l: &mut photocraft_doc::Layer) -> Result<&mut photocraft_raster::Surface> {
    let id = l.id;
    l.surface_mut().ok_or_else(|| EngineError::Other(format!("layer {id:?} has no pixels")))
}

/// The active document's active layer, for `enabled` predicates.
pub(crate) fn active_layer_of(s: &Session) -> std::result::Result<&photocraft_doc::Layer, String> {
    let d = s.active().ok_or("no document open")?;
    d.active_layer.and_then(|id| d.doc.layer(id)).ok_or_else(|| "no active layer".into())
}

#[derive(Default)]
pub struct Session {
    docs: Vec<DocState>,
    /// Admitted document identities in opening order, independent of visual-tab reordering.
    open_document_order: Vec<photocraft_doc::DocId>,
    active: Option<usize>,
    /// Monotonic counter, bumped whenever a different document becomes active (including a
    /// switch away and back inside one pass). UI work that must not cross documents (control
    /// screenshots, automation input) binds to it.
    active_generation: u64,
    /// The document id [`Self::active_generation`] was last bumped for.
    active_doc_for_generation: Option<photocraft_doc::DocId>,
    pub tools: ToolState,
    /// Log of executed commands (for action recording and debugging).
    pub journal: Vec<(String, Value)>,
    /// Coalescing key of the command being executed.
    coalesce_request: Option<String>,
    /// Pixels copied with Edit › Copy / Cut (shared by all documents, like Photoshop).
    pub clipboard: Option<edit_cmds::Clip>,
    /// Layer › Layer Style › Copy Layer Style: the effects and the Blending Options
    /// ([`CopiedStyle`]: blend mode, fill opacity, Advanced Blending, Blend If, channels).
    pub style_clipboard: Option<CopiedStyle>,
    /// Shape path context menu: copied vector fill and stroke styles.
    pub path_fill_clipboard: Option<photocraft_doc::Fill>,
    pub path_stroke_clipboard: Option<photocraft_doc::ShapeStroke>,
    /// Colour management: proofing state, monitor profile, display transforms.
    pub color: color_cmds::ColorState,
    /// Open Edit Contents documents and the smart objects they update.
    pub smart_links: Vec<smart_cmds::SmartLink>,
    /// Type › Save Default Type Styles: character and paragraph style new type layers start from.
    pub type_defaults: Option<(photocraft_doc::text::CharStyle, photocraft_doc::text::ParagraphStyle)>,
    /// Quick Mask Options (colour, opacity, colour indicates) used when entering Quick Mask.
    pub quick_mask_options: channel_cmds::QuickMaskOptions,
    /// Colour channel the running command may change (a single colour channel is targeted).
    color_restrict: Option<color_channels::Restriction>,
    /// Smart object whose filter mask the running command edits (`"target":"filterMask"`): each
    /// edit re-renders it, in the same history step.
    filter_mask_layer: Option<LayerId>,
    /// Edit › Preferences, keyboard shortcuts, menu and toolbar customisation (see `prefs`).
    pub prefs: prefs::PrefsStore,
    /// Release checks use an injected platform transport; no network I/O in the engine.
    pub updates: updates_cmds::Updates,
    /// Edit menu state: Fade source, custom shape library (see `edit_menu_cmds`).
    pub edit_state: edit_menu_cmds::EditState,
    /// The pattern library (Window › Patterns; see `pattern.*`).
    pub patterns: pattern_cmds::PatternLibrary,
    /// Image › Analysis: Measurement Log and Select Data Points (see `analysis_cmds`).
    pub analysis: analysis_cmds::AnalysisState,
    /// Window › Gradients, Patterns (groups), Styles, Shapes, Tool Presets and Clone Source.
    pub presets: presets::PresetState,
    /// File menu state: Lock Slices, Image Assets, last Print / Save for Web settings, script
    /// event log (see `automate_cmds`).
    pub file_menu: automate_cmds::FileMenuState,
    /// Persistent brush preset store (desktop only; `None` keeps presets session-only, as in
    /// headless and test sessions). See `preset_store`. The same store holds the Actions list.
    pub preset_store: Option<preset_store::PresetStore>,
    /// Window › Actions. The list persists with the preset store when one is attached.
    pub actions: actions_cmds::ActionState,
    /// Gate for every command the session runs, including the ones a command runs on its own
    /// behalf and `actions.play` steps. Untrusted sessions (MCP, the control channel) install
    /// the same check a top-level request sees. `None` runs everything, which is what a local
    /// UI and `photocraft-cli run` do.
    pub authorize: Option<fn(&str, &serde_json::Value) -> Result<()>>,
    /// A further route for File › Print when neither `lp` nor a CUPS server over IPP is
    /// reachable (the XDG Print portal in a Flatpak); see `print_cmds::Spooler`.
    pub print_spooler: Option<print_cmds::Spooler>,
    /// The printers `file.printers`, `file.print.printers` and the Print dialog list in place of
    /// the system's (tests); see `print_cmds::PrinterSource`. `None` asks the spooler.
    pub printer_source: Option<print_cmds::PrinterSource>,
    /// Background jobs (see [`jobs`]).
    jobs: jobs::Jobs,
    /// Color Sampler points dropped by the last canvas-size change (crop, Canvas Size, Image Size,
    /// rotation), for the status message. Read with [`Session::take_samplers_removed`].
    samplers_removed: usize,
    /// The canvas-geometry map the next [`Session::edit`] applies to the Color Sampler points (crop,
    /// Canvas Size, Image Size, rotation set it just before their edit). Consumed by the edit.
    sampler_map: Option<photocraft_geom::Affine>,
    /// The same edit's map in edge coordinates, for the painting symmetry (Image Rotation maps the
    /// samplers' pixel centres instead). Consumed by the edit.
    symmetry_map: Option<photocraft_geom::Affine>,
    /// Each open document's linked smart objects' files: size and modification time when last
    /// seen ([`link_watch`]).
    pub link_stamps: std::collections::HashMap<(u64, String), link_watch::Stamp>,
    /// The stamps being taken on a worker thread ([`Session::start_link_scan`]).
    pub link_scan: Option<link_watch::LinkScan>,
    /// Select Subject's last result, reused while the pixels are unchanged (#2865).
    subject_cache: smartselect_cmds::SubjectCache,
    /// Object Selection's Object Finder results (#4087), shared by the hover preview and the
    /// click that commits it; valid while the pixels are unchanged.
    pub object_finder: smartselect_cmds::ObjectFinderCache,
    /// A finished live stroke offered to the `paint.stroke` / `paint.pencil` that commits it
    /// ([`Session::offer_live_stroke`]).
    pub(crate) live_commit: Option<brush_cmds::LiveCommit>,
    /// Documents a background open decoded but held back until the user answers the profile
    /// prompt ([`ColorState::open_question`](color_cmds::ColorState::open_question)): `(id,
    /// document, source read-only)`. The shell takes them with [`Session::take_held_open`].
    held_opens: Vec<(u64, Document, bool)>,
    next_held_open: u64,
    /// The document before the latest [`Session::edit`] of the running command, which
    /// [`Session::hint_damage`] checks a hint against. Set by `edit`, taken by `hint_damage`,
    /// dropped when the command ends. Debug and test builds only.
    #[cfg(any(test, debug_assertions))]
    hint_before: Option<Arc<Document>>,
    /// Damage hints that missed a tile their edit changed ([`Session::hint_damage`]): each one a
    /// command reporting too narrow a redraw area (#4053). Counted in debug and test builds only;
    /// tests read it.
    pub damage_misses: usize,
}

#[cfg(test)]
mod fill_layer_mode_tests;
#[cfg(test)]
mod pattern_mode_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod fx_mode_tests;

pub mod command_hints;
