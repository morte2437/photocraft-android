//! Persistent preferences (Edit › Preferences), keyboard-shortcut, menu and toolbar
//! customisation, grouped like Photoshop's Preferences dialog sections.
//!
//! The whole set is plain serde data owned by the [`Session`] ([`Session::prefs`]), so every
//! frontend reads the same values and agents change them with `prefs.get` / `prefs.set` /
//! `prefs.reset`. Sections that only the GUI shell honours (cursors, checkerboard, guide colours,
//! interface theme…) live here too: keeping one document means the headless MCP server sees and
//! edits exactly what the desktop app persists, and the shell has no second store to keep in sync.
//!
//! The engine never touches the filesystem for preferences (it must build for wasm). Frontends
//! persist [`Session::prefs_to_json`] wherever the platform keeps settings (the desktop app: the
//! OS config directory; the web shell: browser storage) and restore it with
//! [`Session::load_prefs_json`].
//!
//! Engine-side values are applied by [`Session::apply_prefs`]: the history state limit and
//! History Options › Allow Non-Linear History (every open document), the effect-cache memory
//! budget, and units for commands that report lengths.
//! Colour settings (`colorSettings`) are stored with the colour state
//! ([`crate::color_cmds::ColorSettings`]) and persisted in the same document.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// A string-valued choice enum with its JSON names (used to validate `prefs.set` and to fill
/// dropdowns in the Preferences dialog).
macro_rules! choice {
    ($(#[$m:meta])* $name:ident { $($var:ident = $s:literal),+ $(,)? } default $def:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename = $s)] $var),+ }
        impl Default for $name {
            fn default() -> Self { Self::$def }
        }
        impl $name {
            pub const NAMES: &'static [&'static str] = &[$($s),+];
            pub fn name(self) -> &'static str {
                match self { $(Self::$var => $s),+ }
            }
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($s => Some(Self::$var),)+ _ => None }
            }
        }
    };
}

choice!(
    /// Length units for rulers, the Info panel and size dialogs.
    Unit { Pixels = "pixels", Inches = "inches", Centimeters = "cm", Millimeters = "mm", Points = "points", Picas = "picas", Percent = "percent" } default Pixels
);
choice!(TypeUnit { Points = "points", Pixels = "pixels", Millimeters = "mm" } default Points);
choice!(PointSize { PostScript = "postScript", Traditional = "traditional" } default PostScript);
choice!(Interpolation { BicubicAutomatic = "bicubicAutomatic", Nearest = "nearestNeighbor", Bilinear = "bilinear", Bicubic = "bicubic", BicubicSmoother = "bicubicSmoother", BicubicSharper = "bicubicSharper", PreserveDetails = "preserveDetails" } default BicubicAutomatic);
choice!(ColorPicker { Adobe = "adobe", System = "system" } default Adobe);
choice!(Theme { Pro = "pro", ProMedium = "proMedium", ProLightGray = "proLightGray", ProLight = "proLight", Studio = "studio", StudioLight = "studioLight", Classic = "classic", SolarizedDark = "solarizedDark", Adwaita = "adwaita", AdwaitaDark = "adwaitaDark", BreezeLight = "breezeLight", BreezeDark = "breezeDark", SoftDark = "softDark", SoftLight = "softLight" } default ProMedium);
choice!(AppearanceMode { Auto = "auto", Dark = "dark", Light = "light" } default Dark);
choice!(DarkTheme { Pro = "pro", ProMedium = "proMedium", Studio = "studio", SolarizedDark = "solarizedDark", AdwaitaDark = "adwaitaDark", BreezeDark = "breezeDark", SoftDark = "softDark" } default ProMedium);
choice!(LightTheme { ProLight = "proLight", ProLightGray = "proLightGray", StudioLight = "studioLight", Classic = "classic", Adwaita = "adwaita", BreezeLight = "breezeLight", SoftLight = "softLight" } default ProLight);
choice!(CanvasColor { Default = "default", Black = "black", DarkGray = "darkGray", MediumGray = "mediumGray", LightGray = "lightGray", Custom = "custom" } default Default);
choice!(CanvasBorder { DropShadow = "dropShadow", Line = "line", None = "none" } default DropShadow);
choice!(UiScale { Auto = "auto", P75 = "75", P80 = "80", P85 = "85", P90 = "90", P95 = "95", P100 = "100", P105 = "105", P110 = "110", P115 = "115", P120 = "120", P125 = "125", P150 = "150", P175 = "175", P200 = "200", P250 = "250", P300 = "300" } default Auto);
choice!(
    /// Graphics backend of the desktop app's window and GPU canvas (applies at next launch).
    /// `auto` lets PhotoCraft pick (DX12 for Intel adapters on Windows); `cpu` composites on the
    /// CPU and draws the window with a software adapter where the platform has one. A start that
    /// crashes inside the graphics driver moves this to the next safer choice.
    GpuBackend { Auto = "auto", Vulkan = "vulkan", Dx12 = "dx12", Metal = "metal", Gl = "gl", Cpu = "cpu" } default Auto
);
choice!(
    /// Rendering policy, independent of the advanced graphics backend selection.
    /// CPU disables image acceleration; the native window may still need hardware graphics.
    RenderingMode { Auto = "auto", Gpu = "gpu", Cpu = "cpu" } default Auto
);
choice!(
    /// Linux display server of the desktop app's window (applies at next launch). `x11` runs
    /// PhotoCraft through XWayland on a Wayland session, where native file drag and drop works
    /// (winit 0.30 has none on Wayland, issue #386); fractional scaling may then look softer.
    /// It needs an X server (`DISPLAY`); without one PhotoCraft starts as `auto`. Other
    /// platforms ignore it.
    LinuxDisplayServer { Auto = "auto", X11 = "x11" } default Auto
);
choice!(UiFontSize { Tiny = "tiny", Small = "small", Medium = "medium", Large = "large" } default Small);
choice!(LogDestination { Metadata = "metadata", TextFile = "textFile", Both = "both" } default Metadata);
choice!(LogDetail { SessionsOnly = "sessionsOnly", Concise = "concise", Detailed = "detailed" } default Concise);
choice!(Ask { Always = "always", Never = "never", Ask = "ask" } default Always);
choice!(QuickExportFormat { Png = "png", Jpg = "jpg", Gif = "gif", Webp = "webp", Exr = "exr" } default Png);
choice!(ExportLocation { Ask = "ask", SameFolder = "sameFolder" } default Ask);
choice!(ExportMetadata { None = "none", Copyright = "copyright", All = "all" } default Copyright);
choice!(
    /// Painting cursors: Standard (tool icon), Precise (crosshair), Normal Brush Tip (the 50%
    /// opacity contour), Full Size Brush Tip (the whole diameter).
    PaintingCursor { Standard = "standard", Precise = "precise", NormalTip = "normalTip", FullSizeTip = "fullSizeTip" } default NormalTip
);
choice!(
    /// Right mouse button on the canvas with a painting tool: open the Brush Preset picker at the
    /// pointer (Photoshop), or erase with the current brush while dragging (Krita, Paint).
    RightClickPaint { BrushPicker = "brushPicker", Erase = "erase" } default BrushPicker
);
choice!(OtherCursor { Standard = "standard", Precise = "precise" } default Standard);
choice!(CheckerSize { None = "none", Small = "small", Medium = "medium", Large = "large" } default Medium);
choice!(CheckerColors { Light = "light", Medium = "medium", Dark = "dark", Red = "red", Orange = "orange", Green = "green", Blue = "blue", Purple = "purple", Custom = "custom" } default Light);
choice!(LineStyle { Lines = "lines", Dashed = "dashedLines", Dots = "dots" } default Lines);
choice!(TextEngine { Modern = "modern", EastAsian = "eastAsian", WorldReady = "worldReady" } default WorldReady);
choice!(FontPreview { Off = "off", Small = "small", Medium = "medium", Large = "large", ExtraLarge = "extraLarge", Huge = "huge" } default Medium);
choice!(RawColorSpace { Srgb = "srgb", AdobeRgb = "adobeRgb", ProPhoto = "proPhoto", DisplayP3 = "displayP3" } default AdobeRgb);
choice!(RawDepth { Eight = "8", Sixteen = "16" } default Sixteen);
choice!(RawSharpen { None = "none", Screen = "screen", Glossy = "glossy", Matte = "matte" } default None);
choice!(
    /// How a raw file's colour filter array is filled in when it is developed: AHD
    /// (Hirakawa–Parks) keeps edges cleanest, Malvar–He–Cutler is sharp and fast, bilinear is the
    /// fastest and softest. Photoshop's Camera Raw has no such choice.
    RawDemosaic { Ahd = "ahd", Mhc = "mhc", Bilinear = "bilinear" } default Ahd
);

impl Unit {
    /// Document pixels → this unit. `dpi` is the document resolution, `extent` the 100% length
    /// (for percent), `ppi` points per inch (72 PostScript, 72.27 traditional).
    pub fn from_px(self, px: f64, dpi: f64, extent: f64, ppi: f64) -> f64 {
        let dpi = dpi.max(1e-6);
        match self {
            Unit::Pixels => px,
            Unit::Inches => px / dpi,
            Unit::Centimeters => px / dpi * 2.54,
            Unit::Millimeters => px / dpi * 25.4,
            Unit::Points => px / dpi * ppi,
            Unit::Picas => px / dpi * ppi / 12.0,
            Unit::Percent => px / extent.max(1e-6) * 100.0,
        }
    }
    /// This unit → document pixels (inverse of [`Unit::from_px`]).
    pub fn to_px(self, v: f64, dpi: f64, extent: f64, ppi: f64) -> f64 {
        let dpi = dpi.max(1e-6);
        match self {
            Unit::Pixels => v,
            Unit::Inches => v * dpi,
            Unit::Centimeters => v / 2.54 * dpi,
            Unit::Millimeters => v / 25.4 * dpi,
            Unit::Points => v / ppi.max(1e-6) * dpi,
            Unit::Picas => v * 12.0 / ppi.max(1e-6) * dpi,
            Unit::Percent => v * extent / 100.0,
        }
    }
    /// Short suffix for readouts ("px", "in", "cm", …).
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Pixels => "px",
            Unit::Inches => "in",
            Unit::Centimeters => "cm",
            Unit::Millimeters => "mm",
            Unit::Points => "pt",
            Unit::Picas => "pica",
            Unit::Percent => "%",
        }
    }
    /// Decimal places a readout in this unit needs, following Photoshop: whole pixels, one place
    /// for points and percent, two for millimetres, centimetres and picas, three for inches
    /// (#2434). [`fmt_decimals`] rounds to this many places and drops trailing zeros.
    pub fn decimals(self) -> usize {
        match self {
            Unit::Pixels => 0,
            Unit::Points | Unit::Percent => 1,
            Unit::Millimeters | Unit::Centimeters | Unit::Picas => 2,
            Unit::Inches => 3,
        }
    }
    /// A length as typed in Photoshop's numeric fields: a number with an optional unit suffix
    /// ("50%", "2.5 cm", "12pt", "3pc", "1,5 in", "100"). Returns the number and its unit (`None`
    /// when there is no suffix, so the caller picks the default), or `None` if the text is not a
    /// finite number followed by a known unit.
    pub fn parse_length(text: &str) -> Option<(f64, Option<Unit>)> {
        let t = text.trim();
        let end = t.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+'))).unwrap_or(t.len());
        let (number, suffix) = t.split_at(end);
        let v: f64 = number.trim().replace(',', ".").parse().ok()?;
        if !v.is_finite() {
            return None;
        }
        let suffix = suffix.trim();
        let unit = if suffix.is_empty() { None } else { Some(Unit::from_suffix(suffix)?) };
        Some((v, unit))
    }
    /// The unit a typed suffix names, in any case: `px`, `in` or `"`, `cm`, `mm`, `pt`, `pc` or
    /// `pica`, `%`, and their long forms (`pixels`, `inches`, `centimetres`…). The one table every
    /// length field reads.
    pub fn from_suffix(suffix: &str) -> Option<Unit> {
        Some(match suffix.to_ascii_lowercase().as_str() {
            "px" | "pixel" | "pixels" => Unit::Pixels,
            "in" | "inch" | "inches" | "\"" => Unit::Inches,
            "cm" | "centimeter" | "centimeters" | "centimetre" | "centimetres" => Unit::Centimeters,
            "mm" | "millimeter" | "millimeters" | "millimetre" | "millimetres" => Unit::Millimeters,
            "pt" | "point" | "points" => Unit::Points,
            "pc" | "pica" | "picas" => Unit::Picas,
            "%" | "percent" => Unit::Percent,
            _ => return None,
        })
    }
}

impl PointSize {
    /// Points per inch.
    pub fn per_inch(self) -> f64 {
        match self {
            PointSize::PostScript => 72.0,
            PointSize::Traditional => 72.27,
        }
    }
}

// ------------------------------------------------------------------ sections

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct General {
    pub color_picker: ColorPicker,
    /// Default resampling for Image Size and transforms.
    pub image_interpolation: Interpolation,
    pub beep_when_done: bool,
    pub export_clipboard: bool,
    pub resize_image_during_place: bool,
    pub always_create_smart_objects_when_placing: bool,
    /// Scroll wheel zooms instead of panning.
    pub zoom_with_scroll_wheel: bool,
    /// Wheel zoom eases over the frames of a notch; off, a notch is one instant ×1.1 step.
    pub animated_zoom: bool,
    pub zoom_resizes_windows: bool,
    pub use_legacy_free_transform: bool,
    pub auto_show_home_screen: bool,
    /// Desktop: check published stable releases once in the background at launch.
    pub automatically_check_updates: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            color_picker: ColorPicker::Adobe,
            image_interpolation: Interpolation::BicubicAutomatic,
            beep_when_done: false,
            export_clipboard: true,
            resize_image_during_place: true,
            always_create_smart_objects_when_placing: true,
            zoom_with_scroll_wheel: false,
            animated_zoom: true,
            zoom_resizes_windows: false,
            use_legacy_free_transform: false,
            auto_show_home_screen: true,
            automatically_check_updates: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Interface {
    /// Legacy single-theme preference, retained for old automation and saved settings.
    pub theme: Theme,
    pub appearance_mode: AppearanceMode,
    pub dark_theme: DarkTheme,
    pub light_theme: LightTheme,
    /// Theme shortcuts shown in the appearance button's context menu.
    pub favorite_themes: Vec<Theme>,
    /// Pasteboard colour in standard screen mode (`canvasCustomColor` when "custom").
    pub canvas_color: CanvasColor,
    pub canvas_custom_color: String,
    /// Show the repeating dot pattern on the canvas surround (including the empty workspace).
    /// Disable for a uniform background, for example on OLED displays (#2264).
    pub show_canvas_dots: bool,
    /// List recent files on the Home screen (the empty workspace). Off hides the list; the
    /// recent files themselves are kept and File > Open Recent still shows them.
    pub show_home_recent_files: bool,
    /// Document border in standard screen mode.
    pub canvas_border: CanvasBorder,
    pub ui_scale: UiScale,
    /// UI language: `auto` (follow the system) or a language code such as `en`, `ja`. The list of
    /// languages belongs to the shell (`ui-egui` i18n); an unknown code falls back to `auto`.
    /// Command ids and the control protocol stay English.
    pub language: String,
    pub ui_font_size: UiFontSize,
    pub show_channels_in_color: bool,
    pub dynamic_color_sliders: bool,
    /// Draw menu item colours set with Edit › Menus.
    pub show_menu_colors: bool,
    /// Layers panel thumbnails frame the layer's own pixels instead of the whole canvas, so small
    /// objects in a large document stay visible (Photoshop's Clip Thumbnails to Layer Bounds; #4281).
    pub clip_thumbnails_to_layer_bounds: bool,
    pub show_tooltips: bool,
    /// Menus, popups and collapsing sections fade and slide, and scroll areas glide to a new
    /// position. Off, everything changes at once (a reduced-motion mode; #1218, #3273).
    pub enable_animations: bool,
    /// Move tool drags show only the layer's outline and an arrow, leaving its pixels in place
    /// until release. Off (the default), the pixels follow the pointer live inside the outline.
    pub show_bounding_box_when_dragging_layer: bool,
    /// Windows and Linux: use the system's title bar and window buttons instead of PhotoCraft's
    /// own one-row title bar (tiling window managers, desktops that draw their own decorations;
    /// #1271, #1316). Read when the app starts. macOS always uses the system's.
    pub system_title_bar: bool,
    /// Show the active document's file name in the title bar and the OS window title. The tabs
    /// already name the file, so this can be turned off for a plain "PhotoCraft" title (#5037).
    /// The unsaved-changes mark stays either way.
    pub show_file_name_in_title: bool,
    /// Notices (the lower-right cards) hide themselves after
    /// [`Interface::notification_duration_seconds`] unless the pointer rests on them (#2022).
    pub notification_auto_hide: bool,
    /// Seconds a notice stays on screen before it hides itself (Auto Hide Notifications).
    pub notification_duration_seconds: u32,
}

impl Default for Interface {
    fn default() -> Self {
        Self {
            theme: Theme::ProMedium,
            appearance_mode: AppearanceMode::Dark,
            dark_theme: DarkTheme::ProMedium,
            light_theme: LightTheme::ProLight,
            favorite_themes: Vec::new(),
            canvas_color: CanvasColor::Default,
            canvas_custom_color: "#282828".into(),
            show_canvas_dots: true,
            show_home_recent_files: true,
            canvas_border: CanvasBorder::DropShadow,
            ui_scale: UiScale::Auto,
            language: "auto".into(),
            ui_font_size: UiFontSize::Small,
            show_channels_in_color: false,
            dynamic_color_sliders: true,
            show_menu_colors: true,
            clip_thumbnails_to_layer_bounds: false,
            show_tooltips: true,
            enable_animations: true,
            show_bounding_box_when_dragging_layer: false,
            system_title_bar: false,
            show_file_name_in_title: true,
            notification_auto_hide: true,
            notification_duration_seconds: 6,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Workspace {
    pub auto_collapse_icon_panels: bool,
    pub auto_show_hidden_panels: bool,
    pub open_documents_as_tabs: bool,
    pub enable_floating_document_window_docking: bool,
    pub large_tabs: bool,
    pub enable_narrow_options_bar: bool,
    pub remember_workspace_changes: bool,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            auto_collapse_icon_panels: false,
            auto_show_hidden_panels: true,
            open_documents_as_tabs: true,
            enable_floating_document_window_docking: true,
            large_tabs: false,
            enable_narrow_options_bar: false,
            remember_workspace_changes: true,
        }
    }
}

impl Workspace {
    /// Preferences ▸ Workspace ▸ Enable Narrow Options Bar: the options bar drops to its compact
    /// height, as in Photoshop, so the document gets more room (see `panels::options_bar`).
    pub fn narrow_options_bar(&self) -> bool {
        self.enable_narrow_options_bar
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Tools {
    pub show_tooltips: bool,
    /// Tool keys cycle a tool group only with ⇧ held (Photoshop's "Use Shift Key for Tool Switch", on by default there too).
    pub use_shift_key_for_tool_switch: bool,
    pub zoom_clicked_point_to_center: bool,
    pub enable_flick_panning: bool,
    pub vary_round_brush_hardness_on_hud: bool,
    /// Round snapped vector and transform coordinates to whole pixels.
    pub snap_vector_tools_and_transforms_to_pixel_grid: bool,
    /// The size and angle readouts beside the pointer while a marquee or crop frame is drawn,
    /// resized or turned (off: none).
    pub show_transformation_values: bool,
    /// Move tool with Auto-Select: outline the layer a click would pick under the pointer.
    pub show_highlight_on_rollover: bool,
    pub overscroll: bool,
    pub double_click_layer_mask_launches_select_and_mask: bool,
    /// What the right mouse button does on the canvas with the Brush and other painting tools.
    pub right_click_with_painting_tools: RightClickPaint,
    /// Pen tablets: pressure, tilt and rotation reach the brush (off: a pen paints like a mouse).
    pub use_tablet_pressure: bool,
    /// Pen pressure response: `[input, output]` control points in 0..1 that the pen's pressure
    /// passes through before it reaches any brush ([`PressureCurve`]). Linear by default.
    pub pressure_curve: Vec<[f32; 2]>,
    /// Arrow-key nudges of one thing (the selected layers, the selection outline, the Free
    /// Transform box) that follow each other within [`Tools::nudge_bundle_pause_ms`] are one
    /// History state (#2259). Off: every press is a state.
    pub bundle_nudges: bool,
    /// The longest pause, in milliseconds, between two nudges that still bundle into one state.
    /// Longer than a key's auto-repeat delay (about half a second), so a held key is one state.
    pub nudge_bundle_pause_ms: u32,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            show_tooltips: true,
            use_shift_key_for_tool_switch: true,
            zoom_clicked_point_to_center: false,
            enable_flick_panning: true,
            vary_round_brush_hardness_on_hud: true,
            snap_vector_tools_and_transforms_to_pixel_grid: true,
            show_transformation_values: true,
            show_highlight_on_rollover: true,
            overscroll: true,
            double_click_layer_mask_launches_select_and_mask: true,
            right_click_with_painting_tools: RightClickPaint::BrushPicker,
            use_tablet_pressure: true,
            pressure_curve: vec![[0.0, 0.0], [1.0, 1.0]],
            bundle_nudges: true,
            nudge_bundle_pause_ms: 1000,
        }
    }
}

/// Preferences › Tools › Pressure Curve, ready to evaluate: the same monotone cubic every brush
/// dynamic uses for its own control ([`photocraft_paint::ResponseCurve`]), sanitised the same way,
/// since a whole-section update or a hand-edited preferences file can store anything.
pub use photocraft_paint::ResponseCurve as PressureCurve;

mod cmds;
mod customize;
mod files;
mod shortcuts;
mod store;
mod validate;

pub use cmds::specs;
#[cfg(test)]
pub(crate) use files::write_files_atomically;
pub use shortcuts::{
    ALTERNATE_SHORTCUTS, BindKind, MAX_ALTERNATES, TEMPORARY_TOOLS, alternate_keys, bindable_items, conflicts, default_alternates, normalize_shortcut,
    reserved_shortcut,
};
use store::read_lenient;
pub use store::{InvalidPref, PREFS_VERSION, PrefsStore, read_section};
pub use validate::{HIDDEN_UNTIL_IMPLEMENTED, LINUX_ONLY, SECTIONS, choices, gpu_adapter_matches, is_color, is_hidden, parse_hex, range};
use validate::{check_value, get_path, keyed, set_path};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HistoryLog {
    pub enabled: bool,
    pub destination: LogDestination,
    /// Text file the log is appended to (destination textFile/both).
    pub file_path: String,
    pub detail: LogDetail,
}

impl Default for HistoryLog {
    fn default() -> Self {
        Self { enabled: false, destination: LogDestination::Metadata, file_path: String::new(), detail: LogDetail::Concise }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FileHandling {
    pub image_previews: Ask,
    pub lowercase_extension: bool,
    /// File › Save a Copy suggests the document's own name rather than "<name> copy" (#8080).
    pub do_not_append_copy: bool,
    /// Saves run in the background while you keep working (on), or in the foreground under a
    /// modal progress dialog that blocks the app until the file is written (off).
    pub save_in_background: bool,
    /// Save recovery information (crash recovery autosave).
    pub autosave: bool,
    /// Minutes between autosaves of documents with unsaved changes.
    pub autosave_minutes: u32,
    /// Offer to reopen recovered documents at launch.
    pub recover_on_launch: bool,
    /// Open a file with no ICC profile as untagged even when its EXIF ColorSpace tag says sRGB.
    pub ignore_exif_profile_tag: bool,
    /// Most renderers write ACEScg EXRs without stating the colour space. With this on, an EXR
    /// that would otherwise open as linear sRGB opens as ACEScg (AP1, linear) instead.
    pub exr_aces_cg: bool,
    pub ask_before_saving_layered_tiff: bool,
    pub maximize_psd_compatibility: Ask,
    /// SVG groups deeper than this are rasterised on import; parser and document safety caps remain fixed.
    pub rasterize_svg_groups_deeper_than: u32,
    pub recent_file_count: u32,
    /// Most recently opened files, newest first (File › Open Recent).
    pub recent_files: Vec<String>,
    /// A folder whose `.cube`, `.3dl` and `.look` files Color Lookup lists under its built-in
    /// looks (native only, #6558). Empty lists none.
    pub lut_folder: String,
}

impl FileHandling {
    /// What File › Save a Copy appends to the document's name: " copy", or nothing with Do not
    /// append "copy" to filename when saving a copy (#8080).
    pub fn copy_suffix(&self) -> &'static str {
        if self.do_not_append_copy { "" } else { " copy" }
    }
}

impl Default for FileHandling {
    fn default() -> Self {
        Self {
            image_previews: Ask::Always,
            lowercase_extension: true,
            do_not_append_copy: false,
            save_in_background: true,
            autosave: true,
            autosave_minutes: 10,
            recover_on_launch: true,
            ignore_exif_profile_tag: false,
            exr_aces_cg: false,
            ask_before_saving_layered_tiff: true,
            maximize_psd_compatibility: Ask::Always,
            rasterize_svg_groups_deeper_than: photocraft_doc::MAX_GROUP_DEPTH as u32,
            recent_file_count: 20,
            recent_files: Vec::new(),
            lut_folder: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Export {
    pub quick_export_format: QuickExportFormat,
    pub quick_export_location: ExportLocation,
    pub jpeg_quality: u32,
    /// Keep the existing lossless Quick Export default until the user opts into lossy WebP.
    pub webp_lossless: bool,
    pub webp_quality: u32,
    pub metadata: ExportMetadata,
    pub convert_to_srgb: bool,
}

impl Default for Export {
    fn default() -> Self {
        Self {
            quick_export_format: QuickExportFormat::Png,
            quick_export_location: ExportLocation::Ask,
            jpeg_quality: 85,
            webp_lossless: true,
            webp_quality: 85,
            metadata: ExportMetadata::Copyright,
            convert_to_srgb: true,
        }
    }
}

/// [`Performance::memory_usage_mb`]'s default.
pub const DEFAULT_MEMORY_USAGE_MB: u32 = 8192;

/// The most Memory Usage the web build honours, in MB: a wasm module's linear memory tops out at
/// 4 GiB, so a larger allowance (the desktop default is 8 GiB) would never trim anything.
const WASM_MEMORY_USAGE_CAP_MB: u32 = 1536;

/// The Memory Usage a build honours for a `stored` preference: capped on the web build (`web`).
/// Separate from the target check so native tests exercise both.
fn honoured_memory_usage_mb(stored: u32, web: bool) -> u32 {
    if web { stored.min(WASM_MEMORY_USAGE_CAP_MB) } else { stored }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Performance {
    /// Memory PhotoCraft may use, in MB: bounds each document's pixels plus its History (the
    /// oldest states are dropped beyond it), and the result of one transform.
    pub memory_usage_mb: u32,
    /// Undo steps kept per document (History panel states).
    pub history_states: u32,
    /// Cache Levels, 1–8: the highest cache level the Histogram panel counts a large document at
    /// (level n counts a proxy 2^(n-1) times smaller; 1 counts every pixel). Lower is more
    /// accurate, higher faster. The panel picks the level from the document's size up to this;
    /// the web build, which counts on the UI thread, reduces documents from a quarter of the size
    /// the desktop does. Uncached Refresh always counts at level 1.
    pub cache_levels: u32,
    /// Side of the GPU canvas texture tiles, in pixels (applies at next launch).
    pub cache_tile_size: u32,
    /// Draw the canvas with the GPU (applies at next launch).
    pub use_gpu: bool,
    /// Explicit rendering policy. None preserves older useGpu/gpuBackend preferences.
    pub rendering_mode: Option<RenderingMode>,
    /// Graphics backend (applies at next launch; see [`GpuBackend`]).
    pub gpu_backend: GpuBackend,
    /// Which graphics adapter the window and canvas use on a machine with several (applies at
    /// next launch): part of the adapter's name, matched case-insensitively ("nvidia", "arc"). Empty
    /// is automatic. `PHOTOCRAFT_GPU_ADAPTER` overrides it for one launch. If nothing matches or the
    /// match can't present to the window, startup falls back to automatic and says so.
    pub gpu_adapter: String,
    /// Live previews of large documents (adjustment and filter dialogs, an adjustment layer's
    /// sliders while they drag) render on a reduced copy: fast, but blocky when zoomed in. Off:
    /// they render at full resolution.
    pub low_resolution_previews: bool,
    /// Low-latency drawing (applies at next launch): frames reach the screen without waiting for
    /// vsync, so a stroke follows the pen about 50 ms sooner (the swap chain's wait, #1343). wgpu
    /// picks Immediate where the driver has it (most Vulkan and DX12 drivers on Windows and X11),
    /// so the canvas usually tears while it changes; Mailbox (no tearing) only where Immediate is
    /// missing. Frames are capped at 240 fps. Off (the default): vsync, smooth but late. Desktop
    /// only.
    pub low_latency_drawing: bool,
    /// Linux display server (applies at next launch; see [`LinuxDisplayServer`]).
    pub linux_display_server: LinuxDisplayServer,
    /// GPU memory the canvas compositor may hold for layer pages and effect maps, in MB; 0 is
    /// automatic (a graphics card's own memory isn't charged for the document's pixels, memory
    /// shared with the system is). Layers that don't fit redraw the whole document on the CPU.
    pub gpu_memory_mb: u32,
    /// Memory budget of the layer-effect cache, in MB.
    pub effect_cache_mb: u32,
    pub legacy_compositing: bool,
}

impl Performance {
    /// Resolve old preferences without allowing legacy flags to override an explicit mode.
    pub fn effective_rendering_mode(&self) -> RenderingMode {
        self.rendering_mode.unwrap_or_else(|| if !self.use_gpu || self.gpu_backend == GpuBackend::Cpu { RenderingMode::Cpu } else { RenderingMode::Auto })
    }

    /// [`Performance::memory_usage_mb`] as the build can honour it: capped on the web, where the
    /// stored preference (a saved 8192 still loads) is larger than the module's address space.
    pub fn effective_memory_usage_mb(&self) -> u32 {
        honoured_memory_usage_mb(self.memory_usage_mb, cfg!(target_arch = "wasm32"))
    }

    /// Pixel memory a document and its History may hold (Memory Usage), in bytes: beyond it
    /// the oldest history states are dropped.
    pub fn history_budget_bytes(&self) -> usize {
        (self.effective_memory_usage_mb() as usize).saturating_mul(1 << 20)
    }
}

impl Default for Performance {
    /// What a new install starts with, [`DEFAULT_MEMORY_USAGE_MB`] of Memory Usage among it.
    fn default() -> Self {
        Self {
            memory_usage_mb: DEFAULT_MEMORY_USAGE_MB,
            history_states: 50,
            cache_levels: 4,
            cache_tile_size: 8192,
            use_gpu: true,
            rendering_mode: None,
            gpu_backend: GpuBackend::Auto,
            gpu_adapter: String::new(),
            low_resolution_previews: true,
            low_latency_drawing: false,
            linux_display_server: LinuxDisplayServer::Auto,
            gpu_memory_mb: 0,
            effect_cache_mb: 768,
            legacy_compositing: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScratchDisk {
    pub path: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ScratchDisks {
    /// Disks for spilling tiles once the memory budget is reached (in priority order).
    pub disks: Vec<ScratchDisk>,
}

impl Default for ScratchDisks {
    fn default() -> Self {
        Self { disks: vec![ScratchDisk { path: "(system temp)".into(), enabled: true }] }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Cursors {
    pub painting: PaintingCursor,
    pub show_crosshair_in_brush_tip: bool,
    pub show_only_crosshair_while_painting: bool,
    pub other: OtherCursor,
    /// Brush preview (HUD) colour.
    pub brush_preview_color: String,
}

impl Default for Cursors {
    fn default() -> Self {
        Self {
            painting: PaintingCursor::NormalTip,
            show_crosshair_in_brush_tip: false,
            show_only_crosshair_while_painting: false,
            other: OtherCursor::Standard,
            brush_preview_color: "#ff0000".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TransparencyAndGamut {
    pub grid_size: CheckerSize,
    pub grid_colors: CheckerColors,
    /// Checkerboard colours used when `gridColors` is "custom".
    pub custom_light: String,
    pub custom_dark: String,
    pub gamut_warning_color: String,
    /// Gamut warning opacity, percent.
    pub gamut_warning_opacity: u32,
}

impl Default for TransparencyAndGamut {
    fn default() -> Self {
        Self {
            grid_size: CheckerSize::Medium,
            grid_colors: CheckerColors::Light,
            custom_light: "#ffffff".into(),
            custom_dark: "#cccccc".into(),
            gamut_warning_color: "#808080".into(),
            gamut_warning_opacity: 100,
        }
    }
}

impl TransparencyAndGamut {
    /// Checker square side in screen points (`None`: no checkerboard, transparency shows white).
    pub fn square(&self) -> Option<f32> {
        match self.grid_size {
            CheckerSize::None => None,
            CheckerSize::Small => Some(4.0),
            CheckerSize::Medium => Some(8.0),
            CheckerSize::Large => Some(16.0),
        }
    }
    /// The two checkerboard colours (light, dark) as sRGB bytes.
    pub fn colors(&self) -> [[u8; 3]; 2] {
        match self.grid_colors {
            CheckerColors::Light => [[255, 255, 255], [204, 204, 204]],
            CheckerColors::Medium => [[153, 153, 153], [102, 102, 102]],
            CheckerColors::Dark => [[102, 102, 102], [51, 51, 51]],
            CheckerColors::Red => [[255, 255, 255], [255, 204, 204]],
            CheckerColors::Orange => [[255, 255, 255], [255, 229, 204]],
            CheckerColors::Green => [[255, 255, 255], [204, 255, 204]],
            CheckerColors::Blue => [[255, 255, 255], [204, 229, 255]],
            CheckerColors::Purple => [[255, 255, 255], [229, 204, 255]],
            CheckerColors::Custom => [parse_hex(&self.custom_light).unwrap_or([255; 3]), parse_hex(&self.custom_dark).unwrap_or([204; 3])],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UnitsAndRulers {
    pub rulers: Unit,
    pub type_units: TypeUnit,
    pub column_width: f64,
    pub gutter: f64,
    pub print_resolution: f64,
    pub screen_resolution: f64,
    pub point_size: PointSize,
    /// View › Rulers as last set: written when the user toggles the rulers and applied at
    /// launch, so new and opened documents show them as Photoshop does. Set from the View
    /// menu (⌘R), not from the Preferences dialog.
    pub show_rulers: bool,
}

impl Default for UnitsAndRulers {
    fn default() -> Self {
        Self {
            rulers: Unit::Pixels,
            type_units: TypeUnit::Points,
            column_width: 180.0,
            gutter: 12.0,
            print_resolution: 300.0,
            screen_resolution: 72.0,
            point_size: PointSize::PostScript,
            show_rulers: false,
        }
    }
}

/// Rounds `v` to `decimals` places (half away from zero) and drops trailing zeros, so a readout
/// never shows more precision than it has: 0.7995 at three places is `0.8`, never `0.799` or
/// `0.800`, and a whole number keeps no `.0` (#2434).
pub fn fmt_decimals(v: f64, decimals: usize) -> String {
    let p = 10f64.powi(decimals as i32);
    let r = (v * p).round() / p;
    // -0.0 would format as "-0".
    let r = if r == 0.0 { 0.0 } else { r };
    let s = format!("{r:.*}", decimals);
    // Only a fractional part may lose zeros: "550" must not become "55".
    if !s.contains('.') {
        return s;
    }
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl UnitsAndRulers {
    /// Format a length in document pixels in the ruler unit, e.g. `"2.5 in"`.
    pub fn format(&self, px: f64, dpi: f64, extent: f64) -> String {
        let v = self.rulers.from_px(px, dpi, extent, self.point_size.per_inch());
        fmt_decimals(v, self.rulers.decimals())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GuidesGridAndSlices {
    pub guide_color: String,
    pub guide_style: LineStyle,
    pub smart_guide_color: String,
    pub grid_color: String,
    pub grid_style: LineStyle,
    /// Major gridline spacing in `gridUnit`.
    pub gridline_every: f64,
    pub grid_unit: Unit,
    pub subdivisions: u32,
    pub slice_color: String,
    pub show_slice_numbers: bool,
}

impl Default for GuidesGridAndSlices {
    fn default() -> Self {
        Self {
            guide_color: "#4affff".into(),
            guide_style: LineStyle::Lines,
            smart_guide_color: "#ff00ff".into(),
            grid_color: "#8c8c8c".into(),
            grid_style: LineStyle::Lines,
            gridline_every: 1.0,
            grid_unit: Unit::Inches,
            subdivisions: 4,
            slice_color: "#38b5ff".into(),
            show_slice_numbers: true,
        }
    }
}

impl GuidesGridAndSlices {
    /// Major gridline spacing in document pixels.
    pub fn major_px(&self, dpi: f64, extent: f64, ppi: f64) -> f64 {
        self.grid_unit.to_px(self.gridline_every.max(1e-3), dpi, extent, ppi).max(1e-3)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlugIns {
    /// Load the WebAssembly plug-ins (`*.wasm`) in `additional_plugins_folder` (native only).
    pub use_additional_plugins_folder: bool,
    pub additional_plugins_folder: String,
    pub show_extension_panels: bool,
    pub allow_scripts_to_connect: bool,
    pub generator_enabled: bool,
}

impl Default for PlugIns {
    fn default() -> Self {
        Self {
            use_additional_plugins_folder: false,
            additional_plugins_folder: String::new(),
            show_extension_panels: true,
            allow_scripts_to_connect: false,
            generator_enabled: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TypePrefs {
    pub smart_quotes: bool,
    pub missing_glyph_protection: bool,
    pub show_font_names_in_english: bool,
    pub use_esc_to_commit: bool,
    pub text_engine: TextEngine,
    pub font_preview: FontPreview,
    pub fill_new_type_layers_with_placeholder: bool,
    pub recent_fonts: u32,
    /// Favourite font families, starred in the font menu (`type.font.favorite`), in the order
    /// they were added.
    pub favorite_fonts: Vec<String>,
}

impl Default for TypePrefs {
    fn default() -> Self {
        Self {
            smart_quotes: true,
            missing_glyph_protection: true,
            show_font_names_in_english: true,
            use_esc_to_commit: true,
            text_engine: TextEngine::WorldReady,
            font_preview: FontPreview::Medium,
            fill_new_type_layers_with_placeholder: true,
            recent_fonts: 10,
            favorite_fonts: Vec::new(),
        }
    }
}

impl TypePrefs {
    /// Preferences ▸ Type ▸ Missing Glyph Protection: with it on (the default, as in Photoshop),
    /// glyphs the chosen face lacks fall back through the coverage fonts (#1909). The text engine
    /// applies this in `Session::apply_prefs`.
    pub fn missing_glyph_protection(&self) -> bool {
        self.missing_glyph_protection
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EnhancedControls {
    pub scrubby_slider_acceleration: bool,
    pub touch_gestures: bool,
    pub zoom_with_trackpad_pinch: bool,
    pub rotate_view_with_trackpad: bool,
}

impl Default for EnhancedControls {
    fn default() -> Self {
        Self { scrubby_slider_acceleration: true, touch_gestures: true, zoom_with_trackpad_pinch: true, rotate_view_with_trackpad: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RawDefaults {
    pub color_space: RawColorSpace,
    pub bit_depth: RawDepth,
    pub resolution: f64,
    pub sharpen_for: RawSharpen,
    pub open_as_smart_object: bool,
    pub apply_auto_tone: bool,
    /// Opening a raw file interactively shows the Camera Raw dialog first (Open / Cancel), as
    /// Photoshop does; off develops it with the defaults straight away. Automation opens never
    /// show the dialog.
    pub open_in_camera_raw: bool,
    /// The demosaic method raws are developed with; the open-time Camera Raw dialog starts from it.
    pub demosaic: RawDemosaic,
    /// Keep a raw file's develop settings in a sidecar (`IMG_0001.xmp` beside `IMG_0001.CR2`),
    /// as Camera Raw's "Save image settings in: Sidecar .xmp files"; the sidecar is read again
    /// when the file is next opened. Off reads sidecars but never writes one.
    pub write_xmp_sidecars: bool,
}

impl RawColorSpace {
    /// The built-in profile `edit.convertToProfile` knows this space as.
    pub fn profile_id(self) -> &'static str {
        match self {
            RawColorSpace::Srgb => "srgb",
            RawColorSpace::AdobeRgb => "adobe-rgb-compat",
            RawColorSpace::ProPhoto => "prophoto-compat",
            RawColorSpace::DisplayP3 => "display-p3",
        }
    }
}

impl RawSharpen {
    /// Camera Raw's output sharpening for this medium as the Detail panel's (amount, radius):
    /// none, Screen (light, fine), Matte paper (stronger, wider: ink spreads), Glossy paper.
    pub fn preset(self) -> Option<(f32, f32)> {
        match self {
            RawSharpen::None => None,
            RawSharpen::Screen => Some((25.0, 1.0)),
            RawSharpen::Matte => Some((35.0, 1.2)),
            RawSharpen::Glossy => Some((40.0, 1.0)),
        }
    }
}

impl RawDemosaic {
    /// The developer's method.
    pub fn method(self) -> photocraft_io::raw::Demosaic {
        match self {
            RawDemosaic::Ahd => photocraft_io::raw::Demosaic::Ahd,
            RawDemosaic::Mhc => photocraft_io::raw::Demosaic::Mhc,
            RawDemosaic::Bilinear => photocraft_io::raw::Demosaic::Bilinear,
        }
    }
}

impl Default for RawDefaults {
    fn default() -> Self {
        Self {
            color_space: RawColorSpace::AdobeRgb,
            bit_depth: RawDepth::Sixteen,
            resolution: 300.0,
            sharpen_for: RawSharpen::None,
            open_as_smart_object: false,
            apply_auto_tone: false,
            open_in_camera_raw: true,
            demosaic: RawDemosaic::Ahd,
            write_xmp_sidecars: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Integrations {
    /// Allow agents to drive the app over the loopback control channel / MCP bridge.
    pub allow_agent_control: bool,
    /// Default control port (0 = only when `--control` is given).
    pub control_port: u32,
}

impl Default for Integrations {
    fn default() -> Self {
        Self { allow_agent_control: true, control_port: 0 }
    }
}

/// The History panel's History Options… (not part of Edit › Preferences in Photoshop either).
/// Photoshop's snapshot options come with #3634.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HistoryOptions {
    /// Allow Non-Linear History: editing after selecting an earlier state keeps the later
    /// states (the new step is appended at the end), and deleting a state deletes only it.
    pub allow_non_linear_history: bool,
    /// Make Layer Visibility Changes Undoable: showing or hiding a layer is a history state.
    pub make_layer_visibility_changes_undoable: bool,
}

impl Default for HistoryOptions {
    fn default() -> Self {
        Self { allow_non_linear_history: false, make_layer_visibility_changes_undoable: true }
    }
}

/// Edit › Menus: hidden items and item colours, by command id.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MenuCustomization {
    pub hidden: Vec<String>,
    /// Command id → colour name (red, orange, yellow, green, blue, violet, gray).
    pub colors: BTreeMap<String, String>,
}

impl MenuCustomization {
    /// False for the items Edit › Menus can't hide (#5096): the way back into this dialog
    /// (Edit › Menus, Edit › Keyboard Shortcuts), Preferences, Save, Save As and Exit.
    pub fn hideable(id: &str) -> bool {
        !matches!(id, "edit.menus" | "edit.keyboardShortcuts" | "file.save" | "file.saveAs" | "file.exit") && !id.starts_with("edit.preferences")
    }

    /// Whether the menus leave `id` out. Items that can't be hidden stay shown even when an
    /// older or hand-edited preferences file lists them.
    pub fn is_hidden(&self, id: &str) -> bool {
        Self::hideable(id) && self.hidden.iter().any(|h| h == id)
    }
}

/// Edit › Toolbar: hidden tools and a custom order (tool names as in `ui.set {tool}`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ToolbarCustomization {
    pub hidden: Vec<String>,
    pub order: Vec<String>,
}

/// Layer Mask Display Options (double-click the layer mask's row in the Channels panel, or Mask
/// Options… on its Layers thumbnail): the colour and opacity of the layer mask rubylith (`\`,
/// ⇧⌥-click the thumbnail). As in Photoshop, one setting for every mask, kept across restarts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LayerMaskOverlay {
    /// `#rrggbb`.
    pub color: String,
    /// Percent, 0..=100.
    pub opacity: f32,
}

impl Default for LayerMaskOverlay {
    /// Photoshop's: red at 50 %.
    fn default() -> Self {
        Self { color: "#ff0000".into(), opacity: 50.0 }
    }
}

impl LayerMaskOverlay {
    /// The colour as 0..1 RGB; red if the stored value isn't a colour.
    pub fn rgb(&self) -> [f32; 3] {
        parse_hex(&self.color).unwrap_or([255, 0, 0]).map(|v| f32::from(v) / 255.0)
    }

    /// The opacity as a fraction (0..1); 50 % if the stored value isn't a number.
    pub fn alpha(&self) -> f32 {
        if self.opacity.is_finite() { self.opacity.clamp(0.0, 100.0) / 100.0 } else { 0.5 }
    }
}

/// All preferences. JSON keys are the `edit.preferences.<section>` ids.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub general: General,
    pub interface: Interface,
    pub workspace: Workspace,
    pub tools: Tools,
    pub history_log: HistoryLog,
    pub file_handling: FileHandling,
    pub export: Export,
    pub performance: Performance,
    pub scratch_disks: ScratchDisks,
    pub cursors: Cursors,
    pub transparency_and_gamut: TransparencyAndGamut,
    pub units_and_rulers: UnitsAndRulers,
    pub guides_grid_and_slices: GuidesGridAndSlices,
    pub plug_ins: PlugIns,
    #[serde(rename = "type")]
    pub type_: TypePrefs,
    pub enhanced_controls: EnhancedControls,
    pub raw_defaults: RawDefaults,
    pub integrations: Integrations,
    /// History panel › History Options… (`history.options`).
    pub history_options: HistoryOptions,
    /// Edit › Keyboard Shortcuts: command id → shortcut (`Cmd+Shift+N` notation); an empty
    /// string removes the default shortcut.
    pub shortcuts: BTreeMap<String, String>,
    /// Edit › Keyboard Shortcuts: a command's further keys besides its main one (#3665),
    /// command id → keys. A list replaces the command's default alternates
    /// ([`ALTERNATE_SHORTCUTS`]); an empty list removes them. Settings files from before this
    /// map load with nothing stored, so every command keeps its defaults.
    pub alternate_shortcuts: BTreeMap<String, Vec<String>>,
    /// Edit › Keyboard Shortcuts › Set: saved shortcut sets by name (`crate::shortcut_sets`).
    pub shortcut_sets: BTreeMap<String, crate::shortcut_sets::ShortcutSet>,
    /// The saved set in use ("" for the defaults).
    pub shortcut_set: String,
    pub menus: MenuCustomization,
    pub toolbar: ToolbarCustomization,
    /// Edit › Check Spelling: words added to the dictionary ("Add").
    pub user_dictionary: Vec<String>,
    /// Window › Workspace › New Workspace…: saved layouts by name (JSON owned by the shell).
    pub workspaces: BTreeMap<String, Value>,
    /// Window › Workspace › Lock Workspace: panels can't be moved.
    pub workspace_locked: bool,
    /// The live panel layout (dock group heights, order, collapsed groups, open panels), saved
    /// as the user changes it and restored at launch when Workspace › Remember Workspace
    /// Changes is on. JSON owned by the shell.
    pub panel_layout: Value,
    /// The Color panel's options (what it shows, its ramp, Web colors, which chip it edits),
    /// kept across restarts as Photoshop keeps them. JSON owned by the shell.
    pub color_panel: Value,
    /// The last choices of dialogs that remember them across restarts, by command id (Edit ›
    /// Fill…: `"edit.fill"` → its params). JSON owned by the shell.
    pub dialogs: BTreeMap<String, Value>,
    /// The Brush Preset picker's remembered view: which card parts show (name, stroke, tip) and
    /// the footer slider's card scale, saved as the user changes them and restored at launch.
    /// JSON owned by the shell.
    pub brush_picker: Value,
    /// The brush the tool was last using (settings and tip references, never the full bitmaps;
    /// shaped by `brush_cmds::remembered_brush`), restored at launch so the tool starts where it
    /// was left. Written only when `ToolState::brush_rev` moves.
    pub last_brush: Value,
    /// The tools' options-bar settings (Move › Auto-Select, marquee Feather, tolerance, …) that
    /// differ from their defaults, saved as the user changes them and restored at launch, as
    /// Photoshop remembers each tool's options between sessions. JSON owned by the shell.
    pub tool_options: Value,
    /// Each painting tool's own brush (the Brush, the Eraser, the retouching tools…; see the
    /// shell's `paint_mouse::sync_tool_brush`), saved as the user changes them and restored at
    /// launch, as Photoshop keeps every tool's brush between sessions (#6212): `{"owner": tool,
    /// "brushes": {tool: brush}}`, where `owner` is the tool whose brush `last_brush` holds and
    /// the brushes are shaped by `brush_cmds::remembered_settings`. JSON owned by the shell.
    pub tool_brushes: Value,
    /// Last selected tool, restored at launch. A tool-name string owned and validated by the
    /// shell; null in older files. Kept separate from tool options so neither overwrites the other.
    pub selected_tool: Value,
    /// View › Extras, the View › Show items (Pixel Grid, Grid, Guides, …) and the View › Snap To
    /// targets that differ from their defaults, saved as the user toggles them and restored at
    /// launch, as Photoshop keeps them between sessions (#3762). JSON owned by the shell.
    pub view_extras: Value,
    /// Recently Used Colors (`"#rrggbb"`, newest first), shown by the shape Fill and Stroke
    /// popovers and kept between sessions as in Photoshop. Owned by the shell.
    pub recent_colors: Vec<String>,
    /// File › Scripts › Script Events Manager: event → script bindings.
    pub script_events: crate::automate_cmds::ScriptEvents,
    /// Layer Mask Display Options: the layer mask rubylith's colour and opacity.
    pub layer_mask_overlay: LayerMaskOverlay,
}

impl Preferences {
    /// The preferences as JSON (camelCase keys).
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Read a value by dotted path (`""` = everything).
    pub fn get(&self, path: &str) -> Option<Value> {
        get_path(&self.to_json(), path).cloned()
    }

    /// Set a value by dotted path, validating it. Unknown paths and wrong types are errors.
    pub fn set(&mut self, path: &str, value: Value) -> std::result::Result<(), String> {
        if path == "alternateShortcuts" {
            let obj = value.as_object().ok_or("`alternateShortcuts` maps command ids to lists of shortcuts")?;
            let mut map = BTreeMap::new();
            for (id, v) in obj {
                map.insert(id.clone(), alternate_keys(v).map_err(|e| format!("`alternateShortcuts.{id}`: {e}"))?);
            }
            self.alternate_shortcuts = map;
            return Ok(());
        }
        if let Some(id) = path.strip_prefix("alternateShortcuts.") {
            // null = back to the command's default alternates.
            match value {
                Value::Null => {
                    self.alternate_shortcuts.remove(id);
                }
                v => {
                    let keys = alternate_keys(&v).map_err(|e| format!("`{path}`: {e}"))?;
                    self.alternate_shortcuts.insert(id.to_string(), keys);
                }
            }
            return Ok(());
        }
        let value = match (path.strip_prefix("shortcuts."), &value) {
            // Normalise shortcut spelling so conflicts compare equal.
            (Some(_), Value::String(s)) if !s.is_empty() => Value::String(normalize_shortcut(s).ok_or_else(|| format!("`{s}` is not a valid shortcut"))?),
            (Some(_), Value::Null) => {
                // null = back to the default.
                let id = &path["shortcuts.".len()..];
                self.shortcuts.remove(id);
                return Ok(());
            }
            _ => value,
        };
        check_value(path, &value)?;
        let mut root = self.to_json();
        set_path(&mut root, path, value)?;
        *self = serde_json::from_value(root).map_err(|e| format!("invalid value for `{path}`: {e}"))?;
        Ok(())
    }

    /// Reset one section (or one dotted path), or everything with `None`.
    pub fn reset(&mut self, path: Option<&str>) -> std::result::Result<(), String> {
        let Some(path) = path.filter(|p| !p.is_empty()) else {
            *self = Preferences::default();
            return Ok(());
        };
        // These maps have no stored defaults; removing an override restores the fallback.
        match keyed(path) {
            Some(("shortcuts", id)) => {
                self.shortcuts.remove(id);
                return Ok(());
            }
            Some(("alternateShortcuts", id)) => {
                self.alternate_shortcuts.remove(id);
                return Ok(());
            }
            Some(("menus.colors", id)) => {
                self.menus.colors.remove(id);
                return Ok(());
            }
            _ => {}
        }
        let def = Preferences::default().get(path).ok_or_else(|| format!("unknown preference `{path}`"))?;
        if path == "shortcuts" {
            self.shortcuts.clear();
            return Ok(());
        }
        // The favourite fonts are the user's library, not a Type setting: resetting the Type
        // section keeps them (only `type.favoriteFonts` itself, or Reset All, clears them).
        let favorites = (path == "type").then(|| self.type_.favorite_fonts.clone());
        let mut root = self.to_json();
        set_path(&mut root, path, def)?;
        *self = serde_json::from_value(root).map_err(|e| e.to_string())?;
        if let Some(favorites) = favorites {
            self.type_.favorite_fonts = favorites;
        }
        Ok(())
    }

    /// The shortcut in effect for a command: the user's override, else its default.
    pub fn shortcut<'a>(&'a self, id: &str, default: Option<&'a str>) -> Option<&'a str> {
        match self.shortcuts.get(id) {
            Some(s) if s.is_empty() => None,
            Some(s) => Some(s.as_str()),
            None => default,
        }
    }

    /// A command's further keys in effect (#3665): the user's list, else its defaults
    /// ([`ALTERNATE_SHORTCUTS`]), normalised, without duplicates and without `primary` (the
    /// command's main key in effect, which it already answers).
    pub fn alternates(&self, id: &str, primary: Option<&str>) -> Vec<String> {
        let main = primary.and_then(normalize_shortcut);
        let keys: Vec<String> = match self.alternate_shortcuts.get(id) {
            Some(list) => list.iter().filter_map(|s| normalize_shortcut(s)).collect(),
            None => default_alternates(id).filter_map(normalize_shortcut).collect(),
        };
        let mut out: Vec<String> = Vec::with_capacity(keys.len());
        for k in keys {
            if main.as_ref() != Some(&k) && !out.contains(&k) {
                out.push(k);
            }
        }
        out
    }
}

impl Session {
    /// The current preferences.
    pub fn prefs(&self) -> &Preferences {
        self.prefs.get()
    }

    /// Change preferences and apply the engine-side ones.
    pub fn edit_prefs<R>(&mut self, f: impl FnOnce(&mut Preferences) -> R) -> R {
        let r = self.prefs.edit(f);
        self.apply_prefs();
        r
    }

    /// Push engine-relevant preferences into live state: every document's history limit and
    /// linearity, the transform and layer-effect cache budgets and the text engine's
    /// missing-glyph protection.
    pub fn apply_prefs(&mut self) {
        let n = self.prefs().performance.history_states.max(1) as usize;
        let bytes = self.prefs().performance.history_budget_bytes();
        let budget = self.prefs().performance.effect_cache_mb as usize;
        let non_linear = self.prefs().history_options.allow_non_linear_history;
        for st in &mut self.docs {
            st.history.max_states = n;
            st.history.max_bytes = bytes;
            st.history.non_linear = non_linear;
            st.trim_history();
        }
        photocraft_compose::set_effect_cache_budget(budget << 20);
        crate::transform_cmds::set_budget(u64::from(self.prefs().performance.effective_memory_usage_mb()) << 20);
        // Without waiting for the system font scan: this runs on the UI thread at launch.
        photocraft_text::set_shared_missing_glyph_protection(self.prefs().type_.missing_glyph_protection());
        crate::plugin_cmds::sync_prefs(self);
    }

    /// Everything persisted as one JSON document: the preferences plus `colorSettings`.
    pub fn prefs_to_json(&self) -> String {
        serde_json::to_string_pretty(&self.prefs_value()).unwrap_or_default()
    }

    /// [`Session::prefs_to_json`] as a JSON tree (frontends merge it with what storage holds).
    pub fn prefs_value(&self) -> Value {
        let mut v = self.prefs().to_json();
        if let Value::Object(m) = &mut v {
            m.insert("colorSettings".into(), serde_json::to_value(&self.color.settings).unwrap_or(Value::Null));
            m.insert("version".into(), json!(PREFS_VERSION));
            m.insert("presets".into(), self.presets.to_json(self));
        }
        v
    }

    /// Restore preferences saved by [`Session::prefs_to_json`]. Missing keys keep their
    /// defaults and unknown keys are ignored, so files from older and newer versions load.
    /// A value of the wrong type (a hand-edited `"recentFileCount": "ten"`) keeps its default
    /// and everything else still loads (#6611); the returned list names those values. Only
    /// text that isn't a JSON object fails as a whole.
    pub fn load_prefs_json(&mut self, s: &str) -> std::result::Result<Vec<InvalidPref>, String> {
        let mut v: Value = serde_json::from_str(s).map_err(|e| format!("preferences: {e}"))?;
        if !v.is_object() {
            return Err("preferences: expected a JSON object".into());
        }
        // Files before version 2 hold Use Shift Key for Tool Switch off because that was the
        // default when they were written: every key is saved, chosen or not. Like Photoshop it is
        // on by default now (#3528), so those files take the new default (#5647: a tool letter
        // pressed again kept cycling its group). A choice made since is saved as version 2.
        if v.get("version").and_then(Value::as_u64).unwrap_or(0) < 2
            && let Some(tools) = v.get_mut("tools").and_then(Value::as_object_mut)
            && tools.get("useShiftKeyForToolSwitch") == Some(&json!(false))
        {
            tools.remove("useShiftKeyForToolSwitch");
        }
        // Saved preferences before appearance modes had one concrete theme. Preserve its
        // appearance instead of silently switching established users to Auto.
        if let Some(interface) = v.get_mut("interface").and_then(Value::as_object_mut)
            && !interface.contains_key("appearanceMode")
            && let Some(theme) = interface.get("theme").and_then(Value::as_str).and_then(Theme::parse)
        {
            let (mode, slot) = match theme {
                Theme::Pro | Theme::ProMedium | Theme::Studio | Theme::SolarizedDark | Theme::AdwaitaDark | Theme::BreezeDark | Theme::SoftDark => {
                    ("dark", "darkTheme")
                }
                Theme::StudioLight | Theme::ProLightGray | Theme::ProLight | Theme::Classic | Theme::Adwaita | Theme::BreezeLight | Theme::SoftLight => {
                    ("light", "lightTheme")
                }
            };
            interface.insert("appearanceMode".into(), json!(mode));
            // A partially migrated file may already contain a newer theme selection (#5139).
            interface.entry(slot).or_insert_with(|| json!(theme.name()));
        }
        let color = v.as_object_mut().and_then(|m| m.remove("colorSettings"));
        let presets = v.as_object_mut().and_then(|m| m.remove("presets"));
        let mut invalid = Vec::new();
        let prefs: Preferences = read_lenient(v, &[], &mut invalid);
        if let Some(c) = color {
            self.color.settings = read_lenient(c, &["colorSettings"], &mut invalid);
            photocraft_compose::psblend::set_text_gamma(self.color.settings.blend_text_gamma);
        }
        self.prefs.edit(|p| *p = prefs);
        if let Some(v) = presets {
            self.load_presets_json(v);
        }
        self.apply_prefs();
        Ok(invalid)
    }

    /// Full JSON view used by `prefs.get`: preferences plus `colorSettings`.
    fn prefs_view(&self) -> Value {
        let mut v = self.prefs().to_json();
        if let Value::Object(m) = &mut v {
            m.insert("colorSettings".into(), serde_json::to_value(&self.color.settings).unwrap_or(Value::Null));
        }
        v
    }

    /// Set one dotted path, routing `colorSettings.*` to the colour state.
    /// Batches defer colour validation until their complete candidate is staged. Returns
    /// whether colour settings were touched, including whole-section/root-object updates.
    fn set_pref(&mut self, path: &str, value: Value, validate_color: bool) -> std::result::Result<bool, String> {
        // The section itself or a path inside it: `colorSettingsworkingRgb`, `colorSettings.` and
        // `colorSettings..workingRgb` are none, and the authorization policy doesn't treat them as one.
        if let Some(rest) =
            path.strip_prefix("colorSettings").filter(|r| r.is_empty() || r.strip_prefix('.').is_some_and(|c| c.split('.').all(|part| !part.is_empty())))
        {
            let mut cur = serde_json::to_value(&self.color.settings).map_err(|e| e.to_string())?;
            let rest = rest.trim_start_matches('.');
            if rest.is_empty() {
                cur = value;
            } else {
                set_path(&mut cur, rest, value)?;
            }
            let next = serde_json::from_value(cur).map_err(|e| format!("invalid value for `{path}`: {e}"))?;
            if validate_color {
                crate::color_cmds::validate_settings(&next)?;
            }
            self.color.settings = next;
            self.prefs.edit(|_| ());
            return Ok(true);
        }
        let mut next = self.prefs().clone();
        let mut color_changed = false;
        if path.is_empty() || SECTIONS.iter().any(|(id, _)| *id == path) || matches!(path, "shortcuts" | "alternateShortcuts" | "menus" | "toolbar") {
            // Whole section (or everything): merge the object key by key so each value is checked.
            let obj = value.as_object().ok_or_else(|| format!("`{path}` needs an object"))?;
            for (k, v) in obj {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                if path.is_empty() && k == "colorSettings" {
                    color_changed |= self.set_pref("colorSettings", v.clone(), validate_color)?;
                    continue;
                }
                if v.is_object()
                    && !p.starts_with("shortcuts")
                    && !p.starts_with("alternateShortcuts")
                    && !p.ends_with(".colors")
                    && next.get(&p).is_some_and(|c| c.is_object())
                {
                    for (k2, v2) in v.as_object().into_iter().flatten() {
                        next.set(&format!("{p}.{k2}"), v2.clone())?;
                    }
                } else {
                    next.set(&p, v.clone())?;
                }
            }
        } else {
            next.set(path, value)?;
        }
        // Old `prefs.set interface.theme` clients (by path or inside a section object) still
        // select a visible theme.
        if path == "interface.theme" || next.interface.theme != self.prefs().interface.theme {
            match next.interface.theme {
                Theme::Pro => {
                    next.interface.dark_theme = DarkTheme::Pro;
                    next.interface.appearance_mode = AppearanceMode::Dark;
                }
                Theme::ProMedium => {
                    next.interface.dark_theme = DarkTheme::ProMedium;
                    next.interface.appearance_mode = AppearanceMode::Dark;
                }
                Theme::Studio => {
                    next.interface.dark_theme = DarkTheme::Studio;
                    next.interface.appearance_mode = AppearanceMode::Dark;
                }
                Theme::StudioLight => {
                    next.interface.light_theme = LightTheme::StudioLight;
                    next.interface.appearance_mode = AppearanceMode::Light;
                }
                Theme::ProLightGray => {
                    next.interface.light_theme = LightTheme::ProLightGray;
                    next.interface.appearance_mode = AppearanceMode::Light;
                }
                Theme::ProLight => {
                    next.interface.light_theme = LightTheme::ProLight;
                    next.interface.appearance_mode = AppearanceMode::Light;
                }
                Theme::Classic => {
                    next.interface.light_theme = LightTheme::Classic;
                    next.interface.appearance_mode = AppearanceMode::Light;
                }
                Theme::Adwaita => {
                    next.interface.light_theme = LightTheme::Adwaita;
                    next.interface.appearance_mode = AppearanceMode::Light;
                }
                Theme::SolarizedDark => {
                    next.interface.dark_theme = DarkTheme::SolarizedDark;
                    next.interface.appearance_mode = AppearanceMode::Dark;
                }
                Theme::AdwaitaDark => {
                    next.interface.dark_theme = DarkTheme::AdwaitaDark;
                    next.interface.appearance_mode = AppearanceMode::Dark;
                }
                Theme::BreezeDark => {
                    next.interface.dark_theme = DarkTheme::BreezeDark;
                    next.interface.appearance_mode = AppearanceMode::Dark;
                }
                Theme::BreezeLight => {
                    next.interface.light_theme = LightTheme::BreezeLight;
                    next.interface.appearance_mode = AppearanceMode::Light;
                }
                Theme::SoftDark => {
                    next.interface.dark_theme = DarkTheme::SoftDark;
                    next.interface.appearance_mode = AppearanceMode::Dark;
                }
                Theme::SoftLight => {
                    next.interface.light_theme = LightTheme::SoftLight;
                    next.interface.appearance_mode = AppearanceMode::Light;
                }
            }
        }
        self.prefs.edit(|p| *p = next);
        Ok(color_changed)
    }
}

#[cfg(test)]
#[path = "prefs/tests.rs"]
mod tests;

#[cfg(test)]
mod color_settings_path_tests {
    use crate::Session;
    use serde_json::json;

    #[test]
    fn a_path_that_only_starts_with_color_settings_is_not_in_that_section() {
        let mut s = Session::new();
        let before = s.color.settings.clone();
        for path in ["colorSettingsworkingRgb", "colorSettings.", "colorSettings..workingRgb"] {
            assert!(s.execute("prefs.set", json!({"path": path, "value": "/dev/zero"})).is_err(), "{path}");
        }
        assert_eq!(s.color.settings, before);
    }
}
