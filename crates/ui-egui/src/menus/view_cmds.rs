//! Shell-side View, Window and Type menu items: screen modes, Extras and the Show / Snap To
//! toggles, zoom presets, view flipping, document arrangement (tiles, n-up, floating windows,
//! Match Zoom/Location), panel shortcuts that pick the right dock tab, type UI preferences, and
//! the picker/dialog front ends of the engine's File commands.
//!
//! Everything here is view or window state ([`ViewOptions`], serde, part of `UiState`, so the
//! control channel reads and drives it). Anything that changes a document is an engine command;
//! this module only opens its dialog or file picker first.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

mod arrange;
mod dialogs;
mod dispatch;
mod options;

pub use arrange::cells;
pub(crate) use arrange::{document_is_floating, float_window};
pub(crate) use dialogs::write_copy;
#[cfg(test)]
use dialogs::{form, front, label_of};
pub use dialogs::{form_body, form_keys, reset_form};
#[cfg(test)]
use dispatch::is_enabled_with_engine;
pub use dispatch::{checked, handles, invoke, is_enabled, leave_full_screen_without_document};
#[cfg(test)]
use options::screen_mode_commands;
pub use options::{persist, pixel_aspect_ratio, restore};

/// View › Show flags (Photoshop defaults).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Show {
    pub layer_edges: bool,
    pub selection_edges: bool,
    pub target_path: bool,
    pub pixel_grid: bool,
    pub smart_guides: bool,
    pub brush_preview: bool,
    pub slices: bool,
    pub notes: bool,
    pub mesh: bool,
    pub edit_pins: bool,
    pub count: bool,
    pub canvas_guides: bool,
    pub artboard_guides: bool,
}

impl Default for Show {
    fn default() -> Self {
        Self {
            layer_edges: false,
            selection_edges: true,
            target_path: true,
            pixel_grid: true,
            smart_guides: true,
            brush_preview: true,
            slices: true,
            notes: true,
            mesh: false,
            edit_pins: true,
            count: true,
            canvas_guides: true,
            artboard_guides: true,
        }
    }
}

/// View › Snap To targets (snapping itself is the `view.snap` master switch).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnapTo {
    pub guides: bool,
    pub grid: bool,
    pub layers: bool,
    pub slices: bool,
    pub document_bounds: bool,
}

impl Default for SnapTo {
    fn default() -> Self {
        Self { guides: true, grid: true, layers: true, slices: true, document_bounds: true }
    }
}

/// View-, window- and type-preference state of the shell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewOptions {
    /// "standard" | "fullScreenWithMenuBar" | "fullScreen" (View › Screen Mode, F cycles).
    pub screen_mode: String,
    /// View › Extras (⌘H): master switch for guides, grid, edges and other overlays.
    pub extras: bool,
    pub show: Show,
    pub snap_to: SnapTo,
    /// View › Flip Horizontal: mirror the view only (the document is untouched).
    pub flip_horizontal: bool,
    /// View › Pixel Aspect Ratio preset id ("square", "d1DvNtsc", …) and its correction toggle.
    pub pixel_aspect: String,
    pub pixel_aspect_correction: bool,
    pub pattern_preview: bool,
    pub pixel_art_preview: bool,
    /// Window › Arrange layout of the document area: "tabs" or a tile/n-up layout id.
    pub arrange: String,
    /// Type › Font Preview Size: "small" | "medium" | "large" | "extraLarge" | "huge".
    pub font_preview_size: String,
    /// Type › Language Options: "defaultFeatures" | "eastAsianFeatures" | "middleEasternFeatures",
    /// and the Middle Eastern & South Asian composer.
    pub language_features: String,
    pub middle_eastern_composer: bool,
    /// Parameters from the last successfully applied New Guide Layout dialog.
    pub guide_layout: Value,
    /// Developer overlay of the active layer's storage tiles (tile_overlay.rs), set with the
    /// control channel's `ui.set {"tileGrid": true}`.
    pub tile_grid: bool,
}

/// Factory New Guide Layout values (Alt+Reset and a fresh ViewOptions).
pub(crate) fn default_guide_layout() -> Value {
    json!({"columns": 8, "gutter": 20, "rows": 0, "rowGutter": 0, "margin": 0, "centerColumns": false, "clearExisting": false})
}

impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            screen_mode: "standard".into(),
            extras: true,
            show: Show::default(),
            snap_to: SnapTo::default(),
            flip_horizontal: false,
            pixel_aspect: "square".into(),
            // Photoshop shows non-square pixels corrected until the user turns it off.
            pixel_aspect_correction: true,
            pattern_preview: false,
            pixel_art_preview: false,
            arrange: "tabs".into(),
            font_preview_size: "medium".into(),
            language_features: "defaultFeatures".into(),
            middle_eastern_composer: false,
            guide_layout: default_guide_layout(),
            tile_grid: false,
        }
    }
}

impl ViewOptions {
    /// Is an overlay with this Show flag visible (Extras on and the flag on)?
    pub fn shows(&self, flag: bool) -> bool {
        self.extras && flag
    }
    pub fn hides_chrome(&self) -> bool {
        self.screen_mode == "fullScreen"
    }
    pub fn hides_tabs(&self) -> bool {
        self.screen_mode != "standard"
    }
    /// Horizontal display scale of a document pixel: the pixel aspect ratio while View › Pixel
    /// Aspect Ratio Correction is on, else 1 (square). The document's pixels never change.
    pub fn display_aspect(&self) -> f32 {
        let r = pixel_aspect_ratio(&self.pixel_aspect);
        // `ui.set` can store any string; only the dialog's 0.1–10 range is drawn.
        if self.pixel_aspect_correction && r.is_finite() { r.clamp(0.1, 10.0) } else { 1.0 }
    }
    /// Photoshop's full screen modes show no scroll bars.
    pub fn shows_scrollbars(&self) -> bool {
        self.screen_mode == "standard"
    }
}

const SCREEN_MODES: [&str; 3] = ["standard", "fullScreenWithMenuBar", "fullScreen"];
const PIXEL_ASPECTS: [&str; 8] = ["square", "d1DvNtsc", "d1DvPal", "d1DvNtscWidescreen", "hdv1080", "d1DvPalWidescreen", "dvcproHd1080", "anamorphic2To1"];
const FONT_PREVIEW: [&str; 5] = ["small", "medium", "large", "extraLarge", "huge"];
const LANGUAGE: [&str; 3] = ["defaultFeatures", "eastAsianFeatures", "middleEasternFeatures"];
/// Window › Arrange layouts drawn by [`cells`].
const LAYOUTS: [&str; 10] = [
    "tileAllVertically",
    "tileAllHorizontally",
    "twoUpVertical",
    "twoUpHorizontal",
    "threeUpVertical",
    "threeUpHorizontal",
    "threeUpStacked",
    "fourUp",
    "sixUp",
    "tile",
];

#[cfg(test)]
mod label_tests {
    use super::label_of;

    #[test]
    fn untranslated_form_labels_stay_sentence_case() {
        crate::i18n::with_language(crate::i18n::Lang::EN, || {
            assert_eq!(label_of("useAntialias"), "Use antialias");
            assert_eq!(label_of("radius"), "Radius");
            assert_eq!(label_of("iPhone 14 Pro Max"), "iPhone 14 Pro Max");
        });
    }
}

#[cfg(test)]
mod form_reset_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn alt_reset_restores_form_orig() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let id = form(&mut app, "test.form", "Test", json!({"radius": 5.0, "on": true}), json!({}));
        let f = &mut app.ui.dialog_mut(id).unwrap().fields;
        f.insert("radius".into(), json!(99.0));
        f.insert("on".into(), json!(false));
        reset_form(f);
        assert_eq!(f["radius"], json!(5.0));
        assert_eq!(f["on"], json!(true));
        assert!(f.contains_key("__formOrig"));
    }
}

#[cfg(test)]
#[path = "view_cmds/tests.rs"]
mod tests;
