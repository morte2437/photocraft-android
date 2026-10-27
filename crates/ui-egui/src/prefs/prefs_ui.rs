//! Edit › Preferences, Keyboard Shortcuts and Menus, Color Settings and the rest of the Edit
//! menu's dialogs, plus the shell side of preferences: loading and saving them through
//! [`crate::Services`], applying the interface theme, autosave and crash recovery, and the
//! history log.
//!
//! The values live in the engine ([`photocraft_engine::prefs::Preferences`], so agents read and
//! change them with `prefs.get` / `prefs.set`); this module only edits a working copy in a dialog
//! and commits it with those commands on Apply or OK.

use std::collections::{BTreeMap, HashMap, VecDeque};

use egui::{Color32, RichText, Sense, vec2};
use photocraft_doc::DocId;
use photocraft_engine::jobs::{JobEvent, JobId, JobOutcome, Started};
#[cfg(test)]
use photocraft_engine::prefs::Theme;
use photocraft_engine::prefs::{self, AppearanceMode, DarkTheme, LightTheme, SECTIONS, Theme as SavedTheme};
use photocraft_engine::snap::{SnapLine, SnapOptions, SnapTargets};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;
use crate::theme::{ThemeKind, Tokens};

/// Shell runtime state for preferences, autosave and snapping (not serialised).
#[derive(Default)]
pub struct Runtime {
    loaded: bool,
    /// Preferences revision last written to storage (or loaded from it).
    saved_rev: u64,
    /// This instance's preferences as of `saved_rev`: the base of the three-way merge on save.
    saved_value: Option<Value>,
    save_retry: SaveRetry,
    next_autosave_ms: f64,
    /// Last revision whose recovery file was actually written successfully.
    autosaved: HashMap<DocId, u64>,
    /// Queued writes must not masquerade as durable snapshots.
    autosave_pending: HashMap<DocId, u64>,
    recovery_started: bool,
    recovery_queue: VecDeque<crate::Recoverable>,
    recovery_pending: Option<(JobId, String)>,
    log_len: usize,
    /// Snapping state of the drag in progress (see `snap_ui`).
    pub(crate) snap: Option<crate::snap_ui::ActiveSnap>,
    pub(crate) snap_lines: Vec<SnapLine>,
    pub(crate) guide_targets: Option<((DocId, u64, SnapOptions), SnapTargets)>,
    /// Checkerboard colours the cached CPU checker texture was built with.
    pub(crate) checker_key: Option<[[u8; 3]; 2]>,
    /// Style last sent to the GPU canvas.
    pub(crate) gpu_style: Option<crate::gpu_canvas::CanvasStyle>,
    /// `unitsAndRulers.showRulers` as last applied to View › Rulers.
    show_rulers: Option<bool>,
    /// Tool options as last remembered in the preferences (see `tool_memory`).
    pub(crate) tool_options: Option<crate::state::ToolOptions>,
    pub(crate) selected_tool: Option<crate::state::Tool>,
    /// The tool owning the session brush and the other tools' brushes, as last remembered.
    pub(crate) tool_brushes: Option<crate::tool_memory::ToolBrushes>,
}

// ------------------------------------------------------------------ shortcuts

pub use crate::shortcuts::{default_shortcut, effective_shortcut};

mod appearance;
mod dialogs;
mod fields;
mod graphics;
mod save;
mod shortcuts;
mod startup;

#[cfg(test)]
use appearance::{DARK_THEME_OPTIONS, LIGHT_THEME_OPTIONS, display_scale, favorite_theme_groups, selected_theme};
pub(crate) use appearance::{appearance_context_menu, cycle_appearance, step_brightness, sync_appearance};
pub use appearance::{canvas_style, pasteboard_color};
pub use dialogs::{
    PREFERENCES_SIZE, apply, confirm, invoke, is_preferences, open_mismatch, open_preferences, open_shortcuts, owns, preferences_changed, width,
};
#[cfg(test)]
use dialogs::{preference_values, presets_io_body, settings_height};
pub use fields::body;
#[cfg(test)]
use fields::{
    choice_label, export_field_visible, field_visible, has_visible_fields, hex_field, humanize, preference_sections, section_fields, tools_field_visible,
};
#[cfg(test)]
use graphics::{gpu_status_rows, rendering_mode_value};
use save::SaveRetry;
pub(crate) use save::save_preferences;
#[cfg(test)]
use save::{SAVE_RETRY_MAX_S, SAVE_RETRY_S, merge_changes, persist};
pub use save::{autosave_now, tick};
#[cfg(test)]
pub(crate) use shortcuts::capture_key_press;
pub use shortcuts::{is_modifier_key, reserved_shortcut, shortcut_items, shortcut_text};
pub use startup::load;
pub(crate) use startup::{on_recovery_event, remember_rulers};

// Test modules stay after the production code: the prefs_usage test reads each file up to
// its first `#[cfg(test)] mod`.
#[cfg(test)]
mod recovery_tests;

#[cfg(test)]
mod shortcut_capture_tests;
#[cfg(test)]
mod shortcut_set_tests;

#[cfg(test)]
mod tests;
