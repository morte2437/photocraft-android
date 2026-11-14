//! Canvas context menus for the selection tools, the Pen, the Zoom and Hand tools, an active Free
//! Transform box, text being edited with a Type tool, committed type under a Type tool, the
//! pasteboard (outside the image), a guide and a pending crop.

use egui::Context;
use photocraft_doc::{ColorMode, LayerContent};
use photocraft_engine::prefs::{self, CanvasColor};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::direct_select::PathRef;
use crate::{PhotocraftApp, state::Tool};
use photocraft_vector::edit::Hit;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasToolMenu {
    pub pos: [f32; 2],
    pub tool: Tool,
    pub has_selection: bool,
    #[serde(default)]
    pub has_path: bool,
    #[serde(default)]
    pub path_name: Option<String>,
    /// Opened over a Free Transform box: its modes instead of the tool's actions.
    #[serde(default)]
    pub transform: bool,
    /// Opened outside the image: the pasteboard colour instead of the tool's actions.
    #[serde(default)]
    pub pasteboard: bool,
    /// The Zoom and Hand tools' view menu: the document point right-clicked, which Zoom In and
    /// Zoom Out keep under the pointer.
    #[serde(default)]
    pub anchor: Option<[f64; 2]>,
    /// The selected tool when Space holds the Hand over it (`tool` is then the Hand): the menu
    /// closes when that tool changes (#11271).
    #[serde(default)]
    pub under: Option<Tool>,
    /// Opened while a Type tool edits text: the text and type rows instead of the tool's actions.
    #[serde(default)]
    pub text: bool,
    /// Opened with a Type tool over committed type (no edit running): the type layer rows.
    #[serde(default)]
    pub type_layer: bool,
    /// The shape of the committed text a type layer menu opened on, which picks its Convert row
    /// (#12043); `None` for every other menu.
    #[serde(default)]
    pub type_shape: Option<TypeShape>,
    /// Opened on a guide: the guide menu (`guide_select`) instead of the tool's actions.
    #[serde(default)]
    pub guide: bool,
    /// The misspelled word under the pointer when the type edit menu opened: its suggestions head
    /// the menu.
    #[serde(default)]
    pub spelling: Option<Spelling>,
    /// Opened on a pending crop: the crop menu, listing the saved crop presets by the names they
    /// had when it opened.
    #[serde(default)]
    pub crop: Option<Vec<String>>,
    /// The Color Sampler menu's sampler: the one under the pointer when it opened (its index),
    /// `None` off a sampler, where every row is greyed.
    #[serde(default)]
    pub sampler: Option<usize>,
    /// Where that sampler was when the menu opened: the active document's index and the sampler's
    /// position. Once a control command deletes, moves or renumbers samplers, or another document
    /// becomes active, the index may name a different sampler, so the menu closes and Delete greys.
    #[serde(default)]
    pub sampler_at: Option<(usize, [f64; 2])>,
    /// The Eyedropper's menu (#11747): the composite colour under the right-click, averaged with
    /// the Sample Size, which its Copy Color rows put on the clipboard. `None` over transparency.
    #[serde(default)]
    pub color: Option<[f32; 3]>,
    /// Opened with the Content-Aware Move tool inside the active selection (#12194).
    #[serde(default)]
    pub inside_selection: bool,
    /// Opened with the Delete Anchor Point or Convert Point tool over an anchor or a segment of the
    /// targeted path (#12771, #12744): the engine command that entry runs and its params. It heads
    /// the path menu, above a separator. The third part is the document id and revision it was read
    /// from: the knot index means something only there, so the entry expires once either moves on.
    #[serde(default)]
    pub point: Option<(String, Value, (u64, u64))>,
}

/// A misspelled word in the type layer being edited (`edit.checkSpelling`'s `list` entry).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spelling {
    pub layer: u64,
    /// The word's characters `start..end` in the layer's text.
    pub start: usize,
    pub end: usize,
    pub word: String,
    pub suggestions: Vec<String>,
}

/// Most suggestions the type edit menu lists.
const MAX_SUGGESTIONS: usize = 3;
/// The spelling rows' ids: `spelling.replace.<n>` picks the n-th suggestion.
const SPELLING_REPLACE: &str = "spelling.replace.";
const SPELLING_NONE: &str = "spelling.none";
const SPELLING_ADD: &str = "spelling.add";

/// One menu row: label and command id. `None` is a separator.
pub type Row<S = &'static str> = Option<(S, S)>;

/// Right-click while transforming, in Photoshop's order: what the box's handles do, then Rotate
/// and Flip, which turn the open box without committing it (`transform_tool::intercept`).
/// Content-Aware Scale and Puppet Warp are greyed while a box is open.
pub const TRANSFORM_MENU: &[Row] = &[
    Some(("Free Transform", "edit.freeTransform")),
    Some(("Scale", "edit.transform.scale")),
    Some(("Rotate", "edit.transform.rotate")),
    Some(("Skew", "edit.transform.skew")),
    Some(("Distort", "edit.transform.distort")),
    Some(("Perspective", "edit.transform.perspective")),
    Some(("Warp", "edit.transform.warp")),
    None,
    Some(("Content-Aware Scale", "edit.contentAwareScale")),
    Some(("Puppet Warp", "edit.puppetWarp")),
    None,
    Some(("Rotate 180°", "edit.transform.rotate180")),
    Some(("Rotate 90° Clockwise", "edit.transform.rotate90Cw")),
    Some(("Rotate 90° Counter Clockwise", "edit.transform.rotate90Ccw")),
    None,
    Some(("Flip Horizontal", "edit.transform.flipHorizontal")),
    Some(("Flip Vertical", "edit.transform.flipVertical")),
];

/// Photoshop's pasteboard menu (right-click outside the image): the pasteboard colour, the choice
/// Preferences › Interface also offers. The ids are this menu's own: [`choose`] turns them into
/// `prefs.set` (`pasteboard.<CanvasColor name>`) or opens the Color Picker.
pub const PASTEBOARD_MENU: &[Row] = &[
    Some(("Default", "pasteboard.default")),
    Some(("Black", "pasteboard.black")),
    Some(("Dark Gray", "pasteboard.darkGray")),
    Some(("Medium Gray", "pasteboard.mediumGray")),
    Some(("Light Gray", "pasteboard.lightGray")),
    Some(("Custom", "pasteboard.custom")),
    None,
    Some(("Select Custom Color…", PASTEBOARD_PICK)),
];

/// Photoshop's right-click menu inside text being edited: the edit rows act on the text (its
/// selection, the OS clipboard), the type rows on the layer, as the Type menu's do. Faux Bold and
/// Faux Italic are the Character panel's toggles (this menu's own ids, run as `type.setStyle`).
/// Convert to Paragraph Text and Convert to Point Text are both listed; the one that doesn't apply
/// greys out. The exact Photoshop placement is inferred (docs/context-menu-parity.md).
pub const TYPE_EDIT_MENU: &[Row] = &[
    Some(("Undo", "edit.undo")),
    None,
    Some(("Cut", "edit.cut")),
    Some(("Copy", "edit.copy")),
    Some(("Paste", "edit.paste")),
    Some(("Select All", "select.all")),
    None,
    Some(("Check Spelling…", "edit.checkSpelling")),
    Some(("Find and Replace Text…", "edit.findAndReplaceText")),
    None,
    Some(("Faux Bold", FAUX_BOLD)),
    Some(("Faux Italic", FAUX_ITALIC)),
    None,
    Some(("Rasterize Type", "type.rasterizeTypeLayer")),
    Some(("Create Work Path", "type.createWorkPath")),
    Some(("Convert to Shape", "type.convertToShape")),
    None,
    Some(("Horizontal", "type.orientation.horizontal")),
    Some(("Vertical", "type.orientation.vertical")),
    None,
    Some(("None", "type.antiAlias.none")),
    Some(("Sharp", "type.antiAlias.sharp")),
    Some(("Crisp", "type.antiAlias.crisp")),
    Some(("Strong", "type.antiAlias.strong")),
    Some(("Smooth", "type.antiAlias.smooth")),
    Some(("Windows LCD", "type.antiAlias.windowsLcd")),
    Some(("Windows", "type.antiAlias.windows")),
    None,
    Some(("Convert to Paragraph Text", "type.convertToParagraphText")),
    Some(("Convert to Point Text", "type.convertToPointText")),
    Some(("Warp Text…", "type.warpText")),
];

/// Right-click with a Type tool over type that isn't being edited (#7324): the type layer rows,
/// in the reference menu's order (#12043) — Edit Type… on top, Faux Bold and Faux Italic for the
/// whole layer, Layer Style… last, and only the Convert row the layer's text shape allows. Its
/// text-edit rows (Undo, Cut, Copy, Paste, Select All) stay in [`TYPE_EDIT_MENU`]: away from an
/// edit they would act on the document instead of the text.
macro_rules! type_layer_menu {
    ($($convert:expr),*) => {
        &[
            Some(("Edit Type…", "type.editText")),
            None,
            Some(("Check Spelling…", "edit.checkSpelling")),
            Some(("Find and Replace Text…", "edit.findAndReplaceText")),
            None,
            Some(("Rasterize Type", "type.rasterizeTypeLayer")),
            Some(("Create Work Path", "type.createWorkPath")),
            Some(("Convert to Shape", "type.convertToShape")),
            None,
            Some(("Horizontal", "type.orientation.horizontal")),
            Some(("Vertical", "type.orientation.vertical")),
            None,
            Some(("None", "type.antiAlias.none")),
            Some(("Sharp", "type.antiAlias.sharp")),
            Some(("Crisp", "type.antiAlias.crisp")),
            Some(("Strong", "type.antiAlias.strong")),
            Some(("Smooth", "type.antiAlias.smooth")),
            Some(("Windows LCD", "type.antiAlias.windowsLcd")),
            Some(("Windows", "type.antiAlias.windows")),
            None,
            Some(("Faux Bold", FAUX_BOLD)),
            Some(("Faux Italic", FAUX_ITALIC)),
            None,
            $($convert,)*
            None,
            Some(("Warp Text…", "type.warpText")),
            None,
            Some(("Layer Style…", LAYER_STYLE)),
        ]
    };
}

/// Point text: the only Convert row that applies turns it into paragraph text.
pub const TYPE_LAYER_MENU: &[Row] = type_layer_menu!(Some(("Convert to Paragraph Text", "type.convertToParagraphText")));
/// Paragraph (box) text: the mirror case, per the measured reference (#12043).
pub const TYPE_LAYER_MENU_PARAGRAPH: &[Row] = type_layer_menu!(Some(("Convert to Point Text", "type.convertToPointText")));
/// Path and area text: how the layer could be converted was not measured, so both rows stay as
/// before, each greyed unless the engine takes the layer.
pub const TYPE_LAYER_MENU_OTHER: &[Row] =
    type_layer_menu!(Some(("Convert to Paragraph Text", "type.convertToParagraphText")), Some(("Convert to Point Text", "type.convertToPointText")));

/// The shape of the committed text a type layer menu was opened on, which decides the Convert
/// rows it lists (#12043). Captured when the menu opens, as the crop menu captures its presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TypeShape {
    /// Point text: the menu lists Convert to Paragraph Text ([`TYPE_LAYER_MENU`]).
    Point,
    /// Paragraph (box) text: the menu lists Convert to Point Text ([`TYPE_LAYER_MENU_PARAGRAPH`]).
    Paragraph,
    /// Path or area text: both rows, greyed unless applicable ([`TYPE_LAYER_MENU_OTHER`]).
    Other,
}

/// The type layer menu's rows for the shape the menu captured; a menu without one (an older
/// snapshot) keeps today's table with both Convert rows.
fn type_layer_rows(menu: &CanvasToolMenu) -> &'static [Row] {
    match menu.type_shape {
        Some(TypeShape::Point) => TYPE_LAYER_MENU,
        Some(TypeShape::Paragraph) => TYPE_LAYER_MENU_PARAGRAPH,
        _ => TYPE_LAYER_MENU_OTHER,
    }
}

/// The type edit menu's Faux Bold and Faux Italic rows. In the type layer menu they toggle the
/// style of the whole layer instead of the characters being edited (#12043).
pub const FAUX_BOLD: &str = "type.fauxBold";
pub const FAUX_ITALIC: &str = "type.fauxItalic";
/// The type layer menu's Layer Style… row: the Layer Style dialog on the target layer, as the
/// Layer menu's Blending Options… (#12043).
pub const LAYER_STYLE: &str = "layer.layerStyle.blendingOptions";

/// Type edit rows that act on the whole layer (it may stop being type): the edit ends first.
const ENDS_TYPE_EDIT: [&str; 5] =
    ["type.rasterizeTypeLayer", "type.createWorkPath", "type.convertToShape", "type.convertToParagraphText", "type.convertToPointText"];

/// The pasteboard menu's Select Custom Color… row.
pub const PASTEBOARD_PICK: &str = "pasteboard.selectCustomColor";

/// The guide menu's Edit Selected Guides row: New Guide's dialog on the selected guide.
pub const GUIDE_EDIT: &str = "view.editSelectedGuides";
/// The guide menu's Clear Selected Guides row.
pub const GUIDE_CLEAR: &str = "view.clearSelectedGuides";

/// Photoshop's guide menu (right-click on a guide), in its order. Its two Selected Guides rows act
/// on the guide the right-click selected; the rest are the View menu's.
pub const GUIDE_MENU: &[Row] = &[
    Some(("Edit Selected Guides", GUIDE_EDIT)),
    None,
    Some(("Lock Guides", "view.lockGuides")),
    Some(("Clear Guides", "view.clearGuides")),
    Some(("Clear Canvas Guides", "view.clearCanvasGuides")),
    Some(("Clear Selected Guides", GUIDE_CLEAR)),
    Some(("Clear Selected Artboard Guides", "view.clearSelectedArtboardGuides")),
];

/// The crop menu's row ids. A ratio row is `crop.ratio.<key>`, the options bar's preset dropdown
/// key (`crop_size::dropdown_options`); a saved preset is `crop.preset.<n>`, the n-th name the menu
/// opened with; an overlay row is `crop.overlay.<id>` or `crop.overlayShow.<id>`.
const CROP_RATIO: &str = "crop.ratio.";
const CROP_PRESET: &str = "crop.preset.";
const CROP_OVERLAY: &str = "crop.overlay.";
const CROP_SHOW: &str = "crop.overlayShow.";
pub const CROP_CLEAR: &str = "crop.clear";
pub const CROP_NEW: &str = "crop.newPreset";
pub const CROP_DELETE: &str = "crop.deletePreset";
pub const CROP_CANCEL: &str = "crop.cancel";
pub const CROP_COMMIT: &str = "crop.commit";

/// The crop menu (right-click on a pending crop, #6561): the options bar's preset dropdown, in its
/// order, and its Clear; the overlay and when it shows; then Cancel and Crop, the bar's ⊘ and ✓.
/// Built from the bar's own lists, so the two cannot disagree. Empty for any other menu.
pub fn crop_rows(menu: &CanvasToolMenu) -> Vec<Row<String>> {
    use crate::crop_overlay::{CropOverlay, OverlayShow};
    let Some(users) = menu.crop.as_ref() else { return Vec::new() };
    let ratio = |(k, l): &(String, &str)| Some((l.to_string(), format!("{CROP_RATIO}{k}")));
    let (before, after): (Vec<_>, Vec<_>) = crate::crop_size::dropdown_options().into_iter().partition(|(k, _)| k != crate::crop_size::FRONT);
    let mut rows: Vec<Row<String>> = before.iter().map(ratio).collect();
    rows.extend(users.iter().enumerate().map(|(i, name)| Some((name.clone(), format!("{CROP_PRESET}{i}")))));
    rows.extend(after.iter().map(ratio));
    rows.push(Some(("Clear".into(), CROP_CLEAR.into())));
    rows.push(None);
    rows.push(Some(("New Crop Preset…".into(), CROP_NEW.into())));
    rows.push(Some(("Delete Crop Preset…".into(), CROP_DELETE.into())));
    rows.push(None);
    rows.extend(CropOverlay::ALL.iter().map(|k| Some((k.label().to_string(), format!("{CROP_OVERLAY}{}", k.id())))));
    rows.push(None);
    rows.extend(OverlayShow::ALL.iter().map(|k| Some((k.label().to_string(), format!("{CROP_SHOW}{}", k.id())))));
    rows.push(None);
    rows.push(Some(("Cancel".into(), CROP_CANCEL.into())));
    rows.push(Some(("Crop".into(), CROP_COMMIT.into())));
    rows
}

/// Is a crop row's label UI text (translated), rather than a saved preset's name?
pub fn crop_label_is_ui(id: &str) -> bool {
    !id.starts_with(CROP_PRESET)
}

/// Is `command` one of the crop menu's rows?
fn is_crop(menu: &CanvasToolMenu, command: &str) -> bool {
    crop_rows(menu).iter().flatten().any(|(_, id)| id == command)
}

/// The saved crop preset a `crop.preset.<n>` row picks.
fn crop_preset<'a>(menu: &'a CanvasToolMenu, command: &str) -> Option<&'a String> {
    let n = command.strip_prefix(CROP_PRESET)?.parse::<usize>().ok()?;
    menu.crop.as_ref()?.get(n)
}

/// The crop menu's checkmarks, as the options bar shows them: the dropdown's entry (a saved
/// preset the fields hold, else the mode or ratio, so a size preset checks W x H x Resolution),
/// the overlay and when it shows.
fn crop_checked(app: &PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> bool {
    let o = &app.ui.tool_options;
    let user = crate::crop_presets::current_user(app);
    if let Some(k) = command.strip_prefix(CROP_RATIO) {
        return user.is_none() && k == o.crop_ratio;
    }
    if command.starts_with(CROP_PRESET) {
        return user.is_some() && crop_preset(menu, command) == user.as_ref();
    }
    if let Some(k) = command.strip_prefix(CROP_OVERLAY) {
        return k == o.crop_overlay.id();
    }
    command.strip_prefix(CROP_SHOW).is_some_and(|k| k == o.crop_overlay_show.id())
}

fn crop_enabled(app: &PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> bool {
    match command {
        CROP_NEW => crate::crop_presets::savable(&app.ui.tool_options).is_some(),
        CROP_DELETE => menu.crop.as_ref().is_some_and(|u| !u.is_empty()),
        c => is_crop(menu, c),
    }
}

/// Right-click with `tool` on a pending crop: the crop menu at screen point `pos`. Only the Crop
/// tool itself, not one held over it (Space's Hand).
pub fn open_crop(app: &mut PhotocraftApp, ctx: &Context, tool: Tool, pos: [f32; 2]) -> bool {
    if tool != Tool::Crop || !crate::crop_ui::pending(app) || !pos.iter().all(|v| v.is_finite()) {
        return false;
    }
    app.ui.brush_picker = None;
    app.ui.layer_menu = None;
    let users = crate::crop_presets::user_presets(app).into_iter().map(|(name, _)| name).collect();
    app.ui.canvas_tool_menu = Some(CanvasToolMenu {
        pos,
        tool,
        has_selection: false,
        has_path: false,
        path_name: None,
        transform: false,
        pasteboard: false,
        anchor: None,
        under: None,
        text: false,
        type_layer: false,
        type_shape: None,
        guide: false,
        spelling: None,
        crop: Some(users),
        sampler: None,
        sampler_at: None,
        color: None,
        inside_selection: false,
        point: None,
    });
    crate::canvas_popup::mark_open(ctx, menu_id());
    true
}

/// A crop row: each runs what its options-bar twin runs.
fn choose_crop(app: &mut PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> Result<(), String> {
    use crate::crop_overlay::{CropOverlay, OverlayShow};
    use crate::crop_presets::{Choice, apply_choice};
    match command {
        CROP_CANCEL => crate::crop_ui::dismiss(app),
        CROP_COMMIT => crate::canvas::commit_crop(app),
        CROP_NEW => apply_choice(app, Choice::New),
        CROP_DELETE => apply_choice(app, Choice::Delete),
        _ => {
            let before = crate::crop_ui::preset_ratio(app);
            if let Some(k) = command.strip_prefix(CROP_RATIO) {
                apply_choice(app, Choice::Builtin(k.to_string()));
            } else if let Some(name) = crop_preset(menu, command) {
                apply_choice(app, Choice::User(name.clone()));
            } else if command == CROP_CLEAR {
                crate::crop_size::clear_ratio(&mut app.ui.tool_options);
            } else if let Some(k) = command.strip_prefix(CROP_OVERLAY).and_then(CropOverlay::from_id) {
                crate::crop_overlay::set_overlay(&mut app.ui.tool_options, k);
            } else if let Some(k) = command.strip_prefix(CROP_SHOW).and_then(OverlayShow::from_id) {
                app.ui.tool_options.crop_overlay_show = k;
            }
            // As the options bar does: a saved preset or Clear changes the ratio without refitting
            // the frame, so refit it when the ratio (or W x H) the frame follows changed.
            if crate::crop_ui::preset_ratio(app) != before {
                crate::crop_ui::apply_ratio(app);
            }
        }
    }
    Ok(())
}

/// The pasteboard colours in the menu's order, which Space+F steps through.
const CANVAS_COLORS: [CanvasColor; 6] =
    [CanvasColor::Default, CanvasColor::Black, CanvasColor::DarkGray, CanvasColor::MediumGray, CanvasColor::LightGray, CanvasColor::Custom];

/// Photoshop's Space+F (⇧Space+F backwards): the next pasteboard colour, round and round. Saved
/// like a choice in the pasteboard menu (`interface.canvasColor`).
pub fn cycle_canvas_color(app: &mut PhotocraftApp, backward: bool) -> Result<(), String> {
    let n = CANVAS_COLORS.len();
    let at = CANVAS_COLORS.iter().position(|c| *c == app.session.prefs().interface.canvas_color).unwrap_or(0);
    let next = if backward { (at + n - 1) % n } else { (at + 1) % n };
    let name = CANVAS_COLORS.get(next).map_or("default", |c| c.name());
    app.run("prefs.set", json!({"values": {"interface.canvasColor": name}})).map(|_| ())
}

/// The Zoom tool's context menu: the fixed zooms of the View menu (plus Fill Screen), then the
/// zoom steps last, in Photoshop's order (#6005).
pub const VIEW_MENU: &[Row] = &[
    Some(("Fit on Screen", "view.fitOnScreen")),
    Some(("Fill Screen", FILL_SCREEN)),
    Some(("100%", "view.actualPixels")),
    Some(("200%", "view.twoHundredPercent")),
    Some(("Print Size", "view.printSize")),
    None,
    Some(("Zoom In", "view.zoomIn")),
    Some(("Zoom Out", "view.zoomOut")),
];

/// The Hand tool's context menu: only the fixed zooms, as measured in the reference application (#12022).
pub const HAND_MENU: &[Row] = &[
    Some(("Fit on Screen", "view.fitOnScreen")),
    Some(("100%", "view.actualPixels")),
    Some(("200%", "view.twoHundredPercent")),
    Some(("Print Size", "view.printSize")),
];

/// The view menu's Fill Screen row. Photoshop has no View menu command for it, so the menu sets
/// the view itself.
pub const FILL_SCREEN: &str = "view.fillScreen";

/// A selection menu: the rows every selection menu shares, with `$fills` (the rows between
/// Transform Selection and Last Filter) in its own order.
macro_rules! selection_menu {
    ($($fills:expr),* $(,)?) => {
        &[
            Some(("Deselect", "select.deselect")),
            Some(("Select Inverse", "select.inverse")),
            Some(("Feather…", "select.modify.feather")),
            Some(("Select and Mask…", "select.selectAndMask")),
            None,
            Some(("Save Selection…", "select.saveSelection")),
            Some(("Make Work Path…", "select.toWorkPath")),
            None,
            Some(("Layer Via Copy", "layer.new.layerViaCopy")),
            Some(("Layer Via Cut", "layer.new.layerViaCut")),
            Some(("New Layer…", "layer.new.layer")),
            None,
            Some(("Free Transform", "edit.freeTransform")),
            Some(("Transform Selection", "select.transformSelection")),
            $($fills,)*
            None,
            Some(("Last Filter", "filter.lastFilter")),
            None,
            Some(("Fade…", "edit.fade")),
        ]
    };
}

/// Photoshop's selection-tool context menu with an active selection, in its order. Its Generative
/// Fill row has no PhotoCraft command and is left out. Rows stay visible and grey out exactly like
/// their menu-bar twins.
pub const SELECTION_MENU: &[Row] = selection_menu![
    Some(("Distort", "edit.transform.distort")),
    Some(("Perspective", "edit.transform.perspective")),
    None,
    Some(("Delete and Fill Selection", "edit.deleteAndFillSelection")),
    Some(("Fill…", "edit.fill")),
    Some(("Stroke…", "edit.stroke")),
    Some(("Content-Aware Fill…", "edit.contentAwareFill")),
];

/// The selection tools' context menu with nothing selected: making a selection.
pub const NO_SELECTION_MENU: &[Row] = &[
    Some(("Select All", "select.all")),
    Some(("Reselect", "select.reselect")),
    Some(("Color Range…", "select.colorRange")),
    None,
    Some(("Load Selection…", "select.loadSelection")),
];

/// Photoshop's generic layer menu, in its order for a pixel layer without a selection: the Count
/// tool's canvas menu on a marker or off one (#11772), and the Red Eye and Object Selection
/// (#12866) tools' without a selection. Each row is enabled as the Layers panel menu enables it
/// for the active layer (`menus::is_enabled`) and runs like the menu item.
pub const COUNT_MENU: &[Row] = &[
    Some(("Blending Options…", "layer.layerStyle.blendingOptions")),
    None,
    Some(("Duplicate Layer…", "layer.duplicate")),
    Some(("Delete Layer", "layer.delete")),
    Some(("Group from Layers…", "layer.new.groupFromLayers")),
    None,
    Some(("Convert to Smart Object", "layer.smartObjects.convertToSmartObject")),
    Some(("Convert to Layers", "layer.smartObjects.convertToLayers")),
    None,
    Some(("Rasterize Layer", "layer.rasterize.layer")),
    Some(("Rasterize Layer Style", "layer.rasterize.layerStyle")),
    None,
    Some(("Free Transform", "edit.freeTransform")),
    None,
    Some(("Color Range…", "select.colorRange")),
    Some(("Load Selection…", "select.loadSelection")),
    Some(("Reselect", "select.reselect")),
];

/// Right-click with the Content-Aware Move tool inside an active selection (#12194).
pub const CAM_INSIDE_SELECTION_MENU: &[Row] = &[Some(("Color Range…", "select.colorRange")), Some(("Load Selection…", "select.loadSelection"))];

/// The selection menu as measured in the reference application with the Red Eye tool (#12577) and
/// the Object Selection tool (#12866): no Distort or Perspective, and Delete and Fill Selection
/// after the fills.
pub const MEASURED_SELECTION_MENU: &[Row] = selection_menu![
    None,
    Some(("Fill…", "edit.fill")),
    Some(("Stroke…", "edit.stroke")),
    Some(("Content-Aware Fill…", "edit.contentAwareFill")),
    Some(("Delete and Fill Selection", "edit.deleteAndFillSelection")),
];

/// The Eyedropper's copy rows (#11747).
pub const EYEDROPPER_COPY_HTML: &str = "canvasMenu.eyedropper.copyHtml";
pub const EYEDROPPER_COPY_HEX: &str = "canvasMenu.eyedropper.copyHex";

/// The Eyedropper's canvas menu, in the reference application's order (#11747): the Sample Sizes
/// (the current one checked), then the two Copy Color rows for the colour under the pointer.
pub const EYEDROPPER_MENU: &[Row] = &[
    Some(("Point Sample", "canvasMenu.eyedropper.size.1")),
    Some(("3 by 3 Average", "canvasMenu.eyedropper.size.3")),
    Some(("5 by 5 Average", "canvasMenu.eyedropper.size.5")),
    Some(("11 by 11 Average", "canvasMenu.eyedropper.size.11")),
    Some(("31 by 31 Average", "canvasMenu.eyedropper.size.31")),
    Some(("51 by 51 Average", "canvasMenu.eyedropper.size.51")),
    Some(("101 by 101 Average", "canvasMenu.eyedropper.size.101")),
    None,
    Some(("Copy Color as HTML", EYEDROPPER_COPY_HTML)),
    Some(("Copy Color's Hex Code", EYEDROPPER_COPY_HEX)),
];

/// The Sample Size an Eyedropper menu row sets.
fn eyedropper_size(command: &str) -> Option<u32> {
    command.strip_prefix("canvasMenu.eyedropper.size.")?.parse().ok()
}

/// Is `menu` the Eyedropper's own menu (not the pasteboard, guide, transform or crop menu it can
/// also open)?
fn is_eyedropper(menu: &CanvasToolMenu) -> bool {
    menu.tool == Tool::Eyedropper
        && menu.crop.is_none()
        && !menu.guide
        && !menu.pasteboard
        && !menu.transform
        && menu.anchor.is_none()
        && !menu.text
        && !menu.type_layer
}

/// Photoshop's Pen context menu order. Rows whose operation is not applicable to the current
/// path or layer stay visible and disabled.
pub const PEN_MENU: &[Row] = &[
    Some(("Create Vector Mask", "layer.vectorMask.fromPath")),
    Some(("Delete Path", "path.delete")),
    None,
    Some(("Define Custom Shape…", "edit.defineCustomShape")),
    None,
    Some(("Make Selection…", "path.toSelection")),
    Some(("New Guides From Shape", "view.newGuidesFromShape")),
    Some(("Fill Path…", "path.fill")),
    Some(("Stroke Path…", "path.stroke")),
    None,
    Some(("Clipping Path…", "path.clippingPath.set")),
    None,
    Some(("Free Transform Path", "path.transform")),
    None,
    Some(("Unite Shapes", "layer.combineShapes.unite")),
    Some(("Subtract Front Shape", "layer.combineShapes.subtractFrontShape")),
    Some(("Unite Shapes at Overlap", "layer.combineShapes.intersectShapeAreas")),
    Some(("Subtract Shapes at Overlap", "layer.combineShapes.excludeOverlappingShapes")),
    None,
    Some(("Copy Fill", "path.style.copyFill")),
    Some(("Copy Complete Stroke", "path.style.copyStroke")),
    None,
    Some(("Paste Fill", "path.style.pasteFill")),
    Some(("Paste Complete Stroke", "path.style.pasteStroke")),
    None,
    Some(("Isolate Layers", "select.isolateLayers")),
    None,
    Some(("Make Symmetry Path", "paint.symmetryFromPath")),
    Some(("Disable Symmetry Path", "paint.symmetryDisable")),
];

/// The label a row shows, untranslated. The Pen's transform row names what it acts on, as in
/// Photoshop (#12913): the points of the path being drawn, or nothing when the document has no
/// path at all. A path that exists but isn't active, and the Path Selection and Direct Selection
/// menus, keep the table's label.
pub(crate) fn row_label(app: &PhotocraftApp, menu: &CanvasToolMenu, label: &'static str, command: &str) -> &'static str {
    if command != "path.transform" || menu.tool != Tool::Pen {
        return label;
    }
    if app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 2) {
        return "Free Transform Points";
    }
    let any_path = app.session.active().is_some_and(|st| {
        st.doc.work_path.is_some()
            || !st.doc.paths.is_empty()
            || st.doc.walk().iter().any(|(_, _, l)| l.vector_mask.is_some() || matches!(l.content, LayerContent::Shape(_)))
    });
    if any_path { label } else { "Free Transform" }
}

/// The Color Sampler menu's Delete row: removes the sampler the menu opened on.
pub const SAMPLER_DELETE: &str = "view.colorSamplers.delete";

/// The Color Sampler's canvas menu (#11733), in the reference application's order: Delete, then
/// the readout modes for the sampler under the pointer. The readout rows are this menu's own ids
/// and stay greyed until samplers have their own readout mode.
pub const SAMPLER_MENU: &[Row] = &[
    Some(("Delete", SAMPLER_DELETE)),
    None,
    Some(("Actual Color", "colorSampler.readout.actual")),
    Some(("Proof Color", "colorSampler.readout.proof")),
    None,
    Some(("Grayscale", "colorSampler.readout.gray")),
    Some(("RGB Color", "colorSampler.readout.rgb")),
    Some(("Web Color", "colorSampler.readout.web")),
    Some(("HSB Color", "colorSampler.readout.hsb")),
    Some(("HSL Color", "colorSampler.readout.hsl")),
    Some(("CMYK Color", "colorSampler.readout.cmyk")),
    Some(("Lab Color", "colorSampler.readout.lab")),
    None,
    Some(("Grayscale 16 bit", "colorSampler.readout.gray16")),
    Some(("RGB 16 bit", "colorSampler.readout.rgb16")),
    Some(("CMYK 16 bit", "colorSampler.readout.cmyk16")),
    Some(("Lab 16 bit", "colorSampler.readout.lab16")),
    None,
    Some(("Total Ink", "colorSampler.readout.totalInk")),
];

/// Does the Color Sampler menu's index still name the sampler it opened on, in the same document?
/// `true` off a sampler, where there is nothing to go stale.
fn sampler_current(app: &PhotocraftApp, menu: &CanvasToolMenu) -> bool {
    let Some(i) = menu.sampler else { return true };
    let now = app.session.active_index().zip(app.session.active().and_then(|d| d.color_samplers.get(i).copied()));
    now.is_some() && now == menu.sampler_at
}

/// Is `menu` the Color Sampler's own menu (not the pasteboard, guide or transform menu opened
/// with it)?
fn sampler_menu(menu: &CanvasToolMenu) -> bool {
    menu.tool == Tool::ColorSampler && !menu.transform && !menu.pasteboard && !menu.guide && menu.crop.is_none() && menu.anchor.is_none()
}

/// Is `menu` the Content-Aware Move tool's menu opened inside the active selection (#12194)?
pub fn inside_selection(menu: &CanvasToolMenu) -> bool {
    menu.tool == Tool::ContentAwareMove
        && menu.has_selection
        && menu.inside_selection
        && !menu.transform
        && !menu.pasteboard
        && !menu.guide
        && menu.crop.is_none()
        && menu.anchor.is_none()
}

/// Content-Aware Move's canvas context menu requires an open document and leaves Bitmap, Duotone
/// and Multichannel documents on their existing no-menu path (#12194).
fn cam_allowed(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|s| !matches!(s.doc.mode, ColorMode::Bitmap | ColorMode::Duotone | ColorMode::Multichannel))
}

/// Is document point `d` on the image and inside the active selection?
fn selected_at(app: &PhotocraftApp, d: [f64; 2]) -> bool {
    d.iter().all(|v| v.is_finite())
        && d[0] >= 0.0
        && d[1] >= 0.0
        && app.session.active().is_some_and(|s| {
            d[0] < f64::from(s.doc.size.width)
                && d[1] < f64::from(s.doc.size.height)
                && s.doc.selection.as_ref().is_some_and(|sel| sel.sample_channel(d[0].floor() as i32, d[1].floor() as i32, 0) >= 0.5)
        })
}

/// Does a Red Eye right-click at `doc` open a menu (#12577)? Only where the reference application
/// was measured: an RGB document whose one layer is a pixel layer, with no selection or the pointer
/// inside it. Elsewhere it keeps opening nothing.
fn red_eye_measured(app: &PhotocraftApp, doc: Option<[f64; 2]>) -> bool {
    app.session.active().is_some_and(|s| {
        s.doc.mode == ColorMode::Rgb
            && matches!(s.doc.layers.as_slice(), [l] if matches!(l.content, LayerContent::Raster(_)) && l.frame.is_none())
            && (s.doc.selection.is_none() || doc.is_some_and(|d| selected_at(app, d)))
    })
}

/// Tools whose plain canvas right-click opens a menu: the selection tools' actions, the
/// Content-Aware Move menu, the path menu, the Color Sampler's and the Red Eye tool's.
pub fn applies(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::RectMarquee
            | Tool::EllipseMarquee
            | Tool::SingleRowMarquee
            | Tool::SingleColumnMarquee
            | Tool::Lasso
            | Tool::PolygonLasso
            | Tool::MagneticLasso
            | Tool::MagicWand
            | Tool::ObjectSelection
            // Its brush is set from the options bar's chip; the canvas right-click is the
            // selection menu, as in Photoshop (#7268).
            | Tool::QuickSelection
            | Tool::ColorSampler
            // The layer menu (#11772).
            | Tool::Count
            // Sample Size and Copy Color (#11747).
            | Tool::Eyedropper
            | Tool::ContentAwareMove
            // The layer or selection menu, where measured (#12577).
            | Tool::RedEye
    ) || path_menu(tool)
}

/// A Magnetic Lasso border is being drawn on the document in front: a right-click opens no menu
/// over it, and the border stays live, as in Photoshop (#11689). A border left on another
/// document doesn't count, even before `magnetic_lasso_ui::frame` drops it: the control
/// channel's events run no frame. Nor does an ⌥-drag's freehand segment: that case wasn't
/// measured, so it keeps the menu it had.
pub fn magnetic_drawing(app: &PhotocraftApp) -> bool {
    let m = &app.ui.magnetic;
    m.active() && !m.freehand && app.ui.tool == Tool::MagneticLasso && app.session.active().map(|d| d.doc.id.0) == Some(m.doc)
}

/// Tools whose canvas right-click opens the path menu ([`PEN_MENU`]): the Pen, and the Path
/// Selection and Direct Selection tools, whose Photoshop menu offers the same whole-path actions
/// for the selected path (#1056).
pub fn path_menu(tool: Tool) -> bool {
    matches!(tool, Tool::Pen | Tool::PathSelection | Tool::DirectSelection) || point_menu(tool)
}

/// The anchor tools whose path menu also heads with the entry for the point under the right-click
/// (#12771, #12744). Add Anchor Point (#12710) joins here.
fn point_menu(tool: Tool) -> bool {
    matches!(tool, Tool::DeleteAnchorPoint | Tool::ConvertPoint)
}

/// The anchor or segment of the targeted path under document point `doc`, as the anchor tools hit
/// it (`direct_select::hit_active`): Delete Anchor Point on an anchor, Add Anchor Point on a
/// segment, at the clicked point. `None` on empty canvas, a handle, or the Pen draft, which the
/// engine doesn't edit.
fn point_at(app: &PhotocraftApp, doc: [f64; 2]) -> Option<(String, Value, (u64, u64))> {
    let (target, _, hit) = crate::direct_select::hit_active(app, doc)?;
    let (command, mut params) = match hit {
        Hit::Anchor([subpath, knot]) => (POINT_DELETE, json!({"subpath": subpath, "knot": knot})),
        Hit::Segment([subpath, knot], t) => (POINT_ADD, json!({"subpath": subpath, "knot": knot, "t": t})),
        Hit::Handle(..) => return None,
    };
    if target == PathRef::Pen || crate::anchor_tools::ignores(app, &target) {
        return None;
    }
    if let (Some(o), Some(t)) = (params.as_object_mut(), target.params(app).as_object()) {
        o.extend(t.clone());
    }
    let stamp = app.session.active().map(|st| (st.doc.id.0, st.revision))?;
    Some((command.to_string(), params, stamp))
}

const POINT_DELETE: &str = "path.deleteAnchor";
const POINT_ADD: &str = "path.addAnchor";

/// The label of a menu's top point entry, spelled out so `cargo xtask i18n-coverage` lists it.
fn point_label(command: &str) -> String {
    if command == POINT_DELETE { tl!("Delete Anchor Point").to_string() } else { tl!("Add Anchor Point").to_string() }
}

/// Is `command` the open menu's top point entry?
fn is_point(menu: &CanvasToolMenu, command: &str) -> bool {
    menu.point.as_ref().is_some_and(|(c, ..)| c == command)
}

/// Is the open menu's point entry still about the path as it is? The document and its revision are
/// the ones the entry was read from; an edit meanwhile (say a control-channel `path.addAnchor`)
/// shifts the knot indexes, so a stale entry runs nothing.
fn point_current(app: &PhotocraftApp, menu: &CanvasToolMenu) -> bool {
    menu.point.as_ref().is_some_and(|(.., stamp)| app.session.active().map(|st| (st.doc.id.0, st.revision)) == Some(*stamp))
}

/// Tools whose plain canvas right-click offers the view menu.
pub fn is_view_tool(tool: Tool) -> bool {
    matches!(tool, Tool::Zoom | Tool::Hand)
}

/// The selection tools' rows, with or without an active selection.
pub fn selection_rows(has_selection: bool) -> &'static [Row] {
    if has_selection { SELECTION_MENU } else { NO_SELECTION_MENU }
}

/// The rows of an open menu.
pub fn rows(menu: &CanvasToolMenu) -> &'static [Row] {
    if menu.crop.is_some() {
        // Built per menu: [`crop_rows`].
        &[]
    } else if menu.guide {
        GUIDE_MENU
    } else if menu.pasteboard {
        PASTEBOARD_MENU
    } else if menu.transform {
        TRANSFORM_MENU
    } else if menu.anchor.is_some() {
        if menu.tool == Tool::Hand { HAND_MENU } else { VIEW_MENU }
    } else if menu.text {
        TYPE_EDIT_MENU
    } else if menu.type_layer {
        type_layer_rows(menu)
    } else if path_menu(menu.tool) {
        PEN_MENU
    } else if sampler_menu(menu) {
        SAMPLER_MENU
    } else if menu.tool == Tool::Count {
        COUNT_MENU
    } else if matches!(menu.tool, Tool::RedEye | Tool::ObjectSelection) {
        if menu.has_selection { MEASURED_SELECTION_MENU } else { COUNT_MENU }
    } else if menu.tool == Tool::Eyedropper {
        EYEDROPPER_MENU
    } else if inside_selection(menu) {
        CAM_INSIDE_SELECTION_MENU
    } else {
        selection_rows(menu.has_selection)
    }
}

/// Is `command` one of the open menu's rows, and enabled?
pub fn available(app: &PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> bool {
    (is_point(menu, command) && point_current(app, menu))
        || ((rows(menu).iter().flatten().any(|(_, id)| *id == command) || is_spelling(menu, command) || is_crop(menu, command))
            && entry_enabled(app, menu, command))
}

/// Is `command` the transform box's current mode (checked in the transform menu)?
/// Scale stays Free Transform's mode (only Free Transform is checked). Rotate is its own mode.
/// Warp is not a box mode (it is its own session), and `for_command` would read it as Free.
pub(crate) fn mode_checked(app: &PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> bool {
    if menu.crop.is_some() {
        return crop_checked(app, menu, command);
    }
    if menu.guide {
        return command == "view.lockGuides" && app.ui.extras.lock_guides;
    }
    if menu.pasteboard {
        return pasteboard_choice(command) == Some(app.session.prefs().interface.canvas_color);
    }
    if menu.text || menu.type_layer {
        return type_checked(app, command);
    }
    if is_eyedropper(menu) {
        return eyedropper_size(command) == Some(app.ui.tool_options.eyedropper_size);
    }
    menu.transform
        && matches!(command, "edit.freeTransform" | "edit.transform.rotate" | "edit.transform.skew" | "edit.transform.distort" | "edit.transform.perspective")
        && app.ui.transform.as_ref().is_some_and(|t| t.mode == crate::state::TransformMode::for_command(command))
}

/// The type edit menu's checkmarks: the faux styles at the selection, the layer's orientation and
/// anti-aliasing.
fn type_checked(app: &PhotocraftApp, command: &str) -> bool {
    use photocraft_doc::text::Orientation;
    let Some(t) = crate::type_tool::target_text(app) else { return false };
    match command {
        FAUX_BOLD | FAUX_ITALIC => crate::type_tool::styles_at(app).is_some_and(|(c, _)| if command == FAUX_BOLD { c.faux_bold } else { c.faux_italic }),
        "type.orientation.horizontal" => t.orientation == Orientation::Horizontal,
        "type.orientation.vertical" => t.orientation == Orientation::Vertical,
        c => c.strip_prefix("type.antiAlias.") == Some(photocraft_engine::type_extra_cmds::aa_name(t.antialias)),
    }
}

/// Is the Type tool editing text, so that a right-click opens the type edit menu?
pub fn editing_text(app: &PhotocraftApp, tool: Tool) -> bool {
    tool.is_type() && app.ui.text_edit.is_some()
}

/// The committed type layer under document point `doc` when a right-click there with a Type tool
/// opens the type layer menu (#7324). The Type Mask tools are left out: to them existing type is
/// just pixels.
pub fn type_layer_at(app: &mut PhotocraftApp, tool: Tool, doc: [f64; 2]) -> Option<photocraft_doc::LayerId> {
    if !tool.is_type() || tool.is_type_mask() || app.ui.text_edit.is_some() || !doc.iter().all(|v| v.is_finite()) {
        return None;
    }
    crate::type_tool::hit_layer(app, doc[0], doc[1])
}

/// Make the right-clicked type layer `id` the active layer, which the structural rows (Rasterize
/// Type, Convert to Shape…) act on and the checks show. Inside a multiselection the selection stays,
/// so the formatting rows still act on every selected type layer, as the Type menu's do; outside it
/// the type replaces the selection, as a click with the Type tool does.
fn target_type_layer(app: &mut PhotocraftApp, id: photocraft_doc::LayerId) -> bool {
    let Some(st) = app.session.active() else { return false };
    let selected = st.selected_layers();
    if st.active_layer == Some(id) {
        true
    } else if selected.contains(&id) {
        photocraft_engine::layer_multi_cmds::set_selection(&mut app.session, selected, Some(id), Some(id)).is_ok()
    } else {
        app.session.select_layer(id).is_ok()
    }
}

/// Are the type layer menu's faux rows live? They run `type.setStyle` on the selected type
/// layers, so they are offered exactly when that command takes them (#12043 review): at least
/// one selected type layer, and none of them under Lock All - `check_text_unlocked`'s one
/// rejection. A position lock styles fine, as the command does.
fn faux_rows_enabled(app: &PhotocraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    let targets: Vec<photocraft_doc::LayerId> =
        st.selected_layers().into_iter().filter(|id| st.doc.layer(*id).is_some_and(|l| matches!(l.content, LayerContent::Text(_)))).collect();
    !targets.is_empty() && targets.iter().all(|id| !st.doc.effective_locks(*id).all)
}

/// The text shape of the menu's target layer, which is active after [`target_type_layer`]:
/// point or paragraph text get their single Convert row, anything else the pair (#12043).
fn active_type_shape(app: &PhotocraftApp) -> TypeShape {
    let Some(st) = app.session.active() else { return TypeShape::Other };
    match st.active_layer.and_then(|id| st.doc.layer(id)) {
        Some(l) => match &l.content {
            LayerContent::Text(t) => match t.shape {
                photocraft_doc::text::TextShape::Point => TypeShape::Point,
                photocraft_doc::text::TextShape::Box { .. } => TypeShape::Paragraph,
                _ => TypeShape::Other,
            },
            _ => TypeShape::Other,
        },
        None => TypeShape::Other,
    }
}

/// The misspelled word of the edited text under document point `doc`, with its suggestions.
fn spelling_at(app: &mut PhotocraftApp, doc: [f64; 2]) -> Option<Spelling> {
    let layer = app.ui.text_edit.as_ref()?.layer;
    if !doc.iter().all(|v| v.is_finite()) {
        return None;
    }
    let at = crate::type_tool::offset_at(app, photocraft_doc::LayerId(layer), doc[0], doc[1]);
    let found = app.session.execute("edit.checkSpelling", json!({"action": "list", "layer": layer, "suggestions": MAX_SUGGESTIONS})).ok()?;
    let hit = found["misspellings"].as_array()?.iter().find(|m| {
        let (a, b) = (m["start"].as_u64().unwrap_or(u64::MAX) as usize, m["end"].as_u64().unwrap_or(0) as usize);
        a <= at && at <= b
    })?;
    Some(Spelling {
        layer,
        start: hit["start"].as_u64()? as usize,
        end: hit["end"].as_u64()? as usize,
        word: hit["word"].as_str()?.to_string(),
        suggestions: hit["suggestions"].as_array()?.iter().filter_map(|s| s.as_str().map(str::to_string)).take(MAX_SUGGESTIONS).collect(),
    })
}

/// The spelling rows heading the type edit menu (label, id): the suggestions, or a greyed note
/// when there are none, then Add to Dictionary. Empty away from a misspelled word.
pub fn spelling_rows(menu: &CanvasToolMenu) -> Vec<(String, String)> {
    let Some(sp) = menu.spelling.as_ref().filter(|_| menu.text) else { return Vec::new() };
    let mut rows: Vec<(String, String)> = sp.suggestions.iter().enumerate().map(|(i, s)| (s.clone(), format!("{SPELLING_REPLACE}{i}"))).collect();
    if rows.is_empty() {
        rows.push(("No Spelling Suggestions".into(), SPELLING_NONE.into()));
    }
    rows.push(("Add to Dictionary".into(), SPELLING_ADD.into()));
    rows
}

/// Is `command` one of the menu's spelling rows?
fn is_spelling(menu: &CanvasToolMenu, command: &str) -> bool {
    spelling_rows(menu).iter().any(|(_, id)| id == command)
}

/// Does a spelling row's label come from the UI (translated) rather than from the dictionary?
pub fn spelling_label_is_ui(id: &str) -> bool {
    matches!(id, SPELLING_NONE | SPELLING_ADD)
}

/// Right-click over a Free Transform box: its modes.
pub fn open_transform(app: &mut PhotocraftApp, ctx: &Context, pos: [f32; 2]) -> bool {
    if !pos.iter().all(|v| v.is_finite()) {
        return false;
    }
    app.ui.brush_picker = None;
    app.ui.layer_menu = None;
    app.ui.canvas_tool_menu = Some(CanvasToolMenu {
        pos,
        tool: app.ui.tool,
        has_selection: false,
        has_path: false,
        path_name: None,
        transform: true,
        pasteboard: false,
        anchor: None,
        under: None,
        text: false,
        type_layer: false,
        type_shape: None,
        guide: false,
        spelling: None,
        crop: None,
        sampler: None,
        sampler_at: None,
        color: None,
        inside_selection: false,
        point: None,
    });
    crate::canvas_popup::mark_open(ctx, menu_id());
    true
}

/// The pasteboard colour a pasteboard menu row picks (`None` for Select Custom Color… or a row
/// of another menu).
fn pasteboard_choice(command: &str) -> Option<CanvasColor> {
    command.strip_prefix("pasteboard.").and_then(CanvasColor::parse)
}

/// The engine command (and params) a pasteboard row runs, for the automation policy: every row
/// changes a preference.
pub fn engine_call(app: &PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> (String, Value) {
    if menu.crop.is_some() && command == CROP_COMMIT {
        return ("image.crop".into(), json!({}));
    }
    if menu.guide {
        let selected = crate::guide_select::selected(app);
        return match command {
            GUIDE_EDIT => ("view.editGuide".into(), json!({})),
            GUIDE_CLEAR => ("view.deleteGuide".into(), selected.map_or(json!({}), |(v, i)| crate::guide_select::delete_params(v, i))),
            _ => (command.into(), json!({})),
        };
    }
    if sampler_menu(menu) {
        return (command.into(), menu.sampler.map_or(json!({}), |i| json!({"index": i})));
    }
    match pasteboard_choice(command) {
        Some(c) if menu.pasteboard => ("prefs.set".into(), json!({"values": {"interface.canvasColor": c.name()}})),
        _ if menu.pasteboard => ("prefs.set".into(), json!({})),
        _ if (menu.text || menu.type_layer) && matches!(command, FAUX_BOLD | FAUX_ITALIC) => ("type.setStyle".into(), json!({})),
        _ if is_spelling(menu, command) => spelling_call(menu, command),
        _ => (command.into(), json!({})),
    }
}

/// The `edit.checkSpelling` call a spelling row makes: replace the word with a suggestion, or add
/// it to the user dictionary.
fn spelling_call(menu: &CanvasToolMenu, command: &str) -> (String, Value) {
    let Some(sp) = menu.spelling.as_ref() else { return ("edit.checkSpelling".into(), json!({})) };
    let replace = command.strip_prefix(SPELLING_REPLACE).and_then(|n| n.parse::<usize>().ok()).and_then(|n| sp.suggestions.get(n));
    match replace {
        Some(to) => {
            ("edit.checkSpelling".into(), json!({"action": "change", "layer": sp.layer, "start": sp.start, "end": sp.end, "word": sp.word, "replace": to}))
        }
        None => ("edit.checkSpelling".into(), json!({"action": "addToDictionary", "word": sp.word})),
    }
}

/// Does a right-click with `tool` at document point `doc` open the pasteboard menu? Outside the
/// image, for the tools whose right-click has no job there: not the painting tools (their Brush
/// Preset picker or right-button erase), the Pen (its path menu) or text being edited.
pub fn pasteboard_at(app: &PhotocraftApp, tool: Tool, doc: [f64; 2]) -> bool {
    let Some(st) = app.session.active() else { return false };
    let (w, h) = (f64::from(st.doc.size.width), f64::from(st.doc.size.height));
    let outside = doc.iter().all(|v| v.is_finite()) && (doc[0] < 0.0 || doc[1] < 0.0 || doc[0] >= w || doc[1] >= h);
    outside && !path_menu(tool) && !crate::paint_mouse::has_brush_picker(tool) && !crate::paint_mouse::right_erases(app, tool) && !editing_text(app, tool)
}

/// Right-click on the pasteboard: its colour menu at screen point `pos`.
pub fn open_pasteboard(app: &mut PhotocraftApp, ctx: &Context, tool: Tool, pos: [f32; 2]) -> bool {
    if !pos.iter().all(|v| v.is_finite()) {
        return false;
    }
    app.ui.brush_picker = None;
    app.ui.layer_menu = None;
    app.ui.canvas_tool_menu = Some(CanvasToolMenu {
        pos,
        tool,
        has_selection: false,
        has_path: false,
        path_name: None,
        transform: false,
        pasteboard: true,
        anchor: None,
        // Space's Hand keeps the menu over the selected tool (#11271); a held zoom key's goes with
        // the key, as before.
        under: (tool == Tool::Hand && tool != app.ui.tool).then_some(app.ui.tool),
        text: false,
        type_layer: false,
        type_shape: None,
        guide: false,
        spelling: None,
        crop: None,
        sampler: None,
        sampler_at: None,
        color: None,
        inside_selection: false,
        point: None,
    });
    crate::canvas_popup::mark_open(ctx, menu_id());
    true
}

/// Right-click at document point `doc` on a guide that `tool` could drag (the Move tool, or ⌘ with
/// a tool on which ⌘ moves guides, `canvas::command_guide_at`): select the guide and open the guide
/// menu at screen point `pos`.
pub fn open_guide(app: &mut PhotocraftApp, ctx: &Context, tool: Tool, mods: egui::Modifiers, pos: [f32; 2], doc: [f64; 2]) -> bool {
    if !pos.iter().all(|v| v.is_finite()) || !doc.iter().all(|v| v.is_finite()) {
        return false;
    }
    let hit = if tool == Tool::Move { crate::rulers::guide_at(app, doc[0], doc[1]) } else { crate::canvas::command_guide_at(app, tool, doc, mods) };
    let Some((vertical, index)) = hit else { return false };
    crate::guide_select::select(app, vertical, index);
    app.ui.brush_picker = None;
    app.ui.layer_menu = None;
    app.ui.canvas_tool_menu = Some(CanvasToolMenu {
        pos,
        tool,
        has_selection: false,
        has_path: false,
        path_name: None,
        transform: false,
        pasteboard: false,
        anchor: None,
        under: None,
        text: false,
        type_layer: false,
        type_shape: None,
        guide: true,
        spelling: None,
        crop: None,
        sampler: None,
        sampler_at: None,
        color: None,
        inside_selection: false,
        point: None,
    });
    crate::canvas_popup::mark_open(ctx, menu_id());
    true
}

/// A guide row: every row is a View › Guides command, the two Selected Guides ones included.
fn choose_guide(app: &mut PhotocraftApp, ctx: &Context, command: &str) -> Result<(), String> {
    crate::menus::invoke(app, ctx, command, json!({})).map(|_| ())
}

/// A pasteboard row: set the colour, or open the Color Picker on the custom one.
fn choose_pasteboard(app: &mut PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> Result<(), String> {
    if command == PASTEBOARD_PICK {
        let rgb = prefs::parse_hex(&app.session.prefs().interface.canvas_custom_color).map_or([0.5; 3], |c| c.map(|v| f32::from(v) / 255.0));
        crate::color_picker_ui::open_for_target(app, "pasteboard", "Color Picker (Custom Canvas Color)", rgb);
        return Ok(());
    }
    let (command, params) = engine_call(app, menu, command);
    app.run(&command, params).map(|_| ())
}

pub fn entry_enabled(app: &PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> bool {
    if menu.crop.is_some() {
        return crop_enabled(app, menu, command);
    }
    if menu.guide {
        return match command {
            GUIDE_EDIT | GUIDE_CLEAR => crate::guide_select::selected(app).is_some(),
            _ => GUIDE_MENU.iter().flatten().any(|(_, id)| *id == command) && crate::menus::is_enabled(app, command),
        };
    }
    if menu.pasteboard {
        return PASTEBOARD_MENU.iter().flatten().any(|(_, id)| *id == command);
    }
    if is_eyedropper(menu) {
        return eyedropper_size(command).is_some() || (matches!(command, EYEDROPPER_COPY_HTML | EYEDROPPER_COPY_HEX) && menu.color.is_some());
    }
    // Warp needs a layer, selection or path box: a lone mask or channel keeps the plain box.
    if menu.transform && command == "edit.transform.warp" {
        return app.ui.transform.as_ref().is_some_and(|t| t.warp.is_none() && t.target.is_none());
    }
    if menu.anchor.is_some() && command == FILL_SCREEN {
        return app.session.active().is_some();
    }
    if menu.text {
        return match command {
            SPELLING_NONE => false,
            c if is_spelling(menu, c) => app.ui.text_edit.is_some(),
            FAUX_BOLD | FAUX_ITALIC => app.ui.text_edit.is_some(),
            c => TYPE_EDIT_MENU.iter().flatten().any(|(_, id)| *id == c) && crate::menus::is_enabled(app, c),
        };
    }
    if menu.type_layer {
        return match command {
            // #12043: over committed type the faux rows toggle the whole layer: they run as
            // `type.setStyle` on its type layers, so they are live exactly when that command
            // takes them.
            FAUX_BOLD | FAUX_ITALIC => faux_rows_enabled(app),
            c => type_layer_rows(menu).iter().flatten().any(|(_, id)| *id == c) && crate::menus::is_enabled(app, c),
        };
    }
    // Off a sampler every row is greyed. The readout rows wait for per-sampler readout modes.
    if sampler_menu(menu) {
        return command == SAMPLER_DELETE && menu.sampler.is_some() && sampler_current(app, menu);
    }
    // Inside the Content-Aware Move selection menu (#12194), Load Selection… stays greyed until a
    // selection channel (non-spot alpha channel) has been saved.
    if inside_selection(menu) && command == "select.loadSelection" {
        return app.session.active().is_some_and(|st| st.doc.channels.iter().any(|c| c.spot.is_none())) && crate::menus::is_enabled(app, command);
    }
    if menu.transform || !path_menu(menu.tool) {
        return crate::menus::is_enabled(app, command);
    }
    let Some(st) = app.session.active() else { return false };
    let layer = st.active_layer.and_then(|id| st.doc.layer(id));
    let locked = layer.is_some_and(|l| st.doc.effective_locks(l.id).all);
    let can_paint = layer.is_some_and(|l| matches!(l.content, LayerContent::Raster(_))) && !locked;
    let shape_layer = layer.and_then(|l| match &l.content {
        LayerContent::Shape(sh) => Some(sh),
        _ => None,
    });
    let shape = shape_layer.is_some();
    let can_mask = layer.is_some_and(|l| !matches!(l.content, LayerContent::Shape(_))) && !locked;
    let saved = menu.path_name.as_deref().is_some_and(|n| n != "work" && n != "layer");
    let has_path = menu.has_path;
    let pending = app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 2);
    let pending_layer = pending && (app.ui.tool_options.vector_mode == "shape" || app.ui.vector_mask_target);
    match command {
        "layer.vectorMask.fromPath" => has_path && can_mask && !pending_layer,
        "path.delete" => has_path && !pending_layer && (pending || menu.path_name.as_deref() != Some("layer")),
        "edit.defineCustomShape" | "path.toSelection" | "path.transform" | "paint.symmetryFromPath" => has_path,
        "view.newGuidesFromShape" => shape && !pending,
        "path.fill" | "path.stroke" => has_path && can_paint && !pending_layer,
        "path.clippingPath.set" => saved && !pending,
        "layer.combineShapes.unite"
        | "layer.combineShapes.subtractFrontShape"
        | "layer.combineShapes.intersectShapeAreas"
        | "layer.combineShapes.excludeOverlappingShapes" => !pending && shape_layer.is_some_and(|sh| sh.path.subpaths.len() > 1),
        "path.style.copyFill" => !pending && shape_layer.is_some_and(|sh| sh.fill.is_some()),
        "path.style.copyStroke" => !pending && shape_layer.is_some_and(|sh| sh.stroke.is_some()),
        "path.style.pasteFill" => !pending && shape && app.session.path_fill_clipboard.is_some(),
        "path.style.pasteStroke" => !pending && shape && app.session.path_stroke_clipboard.is_some(),
        "select.isolateLayers" => layer.is_some(),
        "paint.symmetryDisable" => st.symmetry.is_some(),
        _ => false,
    }
}

pub fn open(app: &mut PhotocraftApp, ctx: &Context, tool: Tool, pos: [f32; 2]) -> bool {
    open_at(app, ctx, tool, pos, None)
}

/// [`open`] for a right-click at document point `doc`: over a misspelled word of the text being
/// edited, the menu starts with its spelling suggestions; over committed type with a Type tool, the
/// type layer menu.
pub fn open_at(app: &mut PhotocraftApp, ctx: &Context, tool: Tool, pos: [f32; 2], doc: Option<[f64; 2]>) -> bool {
    let text = editing_text(app, tool);
    if !pos.iter().all(|v| v.is_finite()) {
        return false;
    }
    let mut type_shape = None;
    let type_layer = !text
        && doc.and_then(|d| type_layer_at(app, tool, d)).is_some_and(|id| {
            if !target_type_layer(app, id) {
                return false;
            }
            // The clicked layer is the menu's target; its shape picks the Convert row (#12043).
            type_shape = Some(active_type_shape(app));
            true
        });
    if !(applies(tool) || text || type_layer) {
        return false;
    }
    // Without an active document or in Bitmap/Duotone/Multichannel modes, Content-Aware Move opens
    // no menu (#12194); release any non-held control-channel HUD that would otherwise stay open.
    if tool == Tool::ContentAwareMove && !cam_allowed(app) {
        crate::color_hud::release(&mut app.ui);
        return false;
    }
    if tool == Tool::RedEye && !red_eye_measured(app, doc) {
        return false;
    }
    app.ui.brush_picker = None;
    app.ui.layer_menu = None;
    let has_selection = app.session.active().is_some_and(|s| s.doc.selection.is_some());
    let has_path = crate::vector_ui::active_path_name(app).is_some() || app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 2);
    let path_name = crate::vector_ui::active_path_name(app);
    let spelling = if text { doc.and_then(|d| spelling_at(app, d)) } else { None };
    let point = if point_menu(tool) && has_path { doc.filter(|d| d.iter().all(|v| v.is_finite())).and_then(|d| point_at(app, d)) } else { None };
    let inside_selection = tool == Tool::ContentAwareMove && doc.is_some_and(|d| selected_at(app, d));
    let sampler = if tool == Tool::ColorSampler { doc.and_then(|d| crate::analysis_ui::sampler_at(app, d)) } else { None };
    let sampler_at = sampler.and_then(|i| Some((app.session.active_index()?, *app.session.active()?.color_samplers.get(i)?)));
    // The Eyedropper reads the colour now, under the right-click: all layers, with the Sample
    // Size, whatever its Sample setting (#11747). Picking a row later copies this colour.
    let color = if tool == Tool::Eyedropper && !text && !type_layer { doc.and_then(|d| crate::canvas::composite_color(app, d[0], d[1])) } else { None };
    app.ui.canvas_tool_menu = Some(CanvasToolMenu {
        pos,
        tool,
        has_selection,
        has_path,
        path_name,
        transform: false,
        pasteboard: false,
        anchor: None,
        under: None,
        text,
        type_layer,
        type_shape,
        guide: false,
        spelling,
        crop: None,
        sampler,
        sampler_at,
        color,
        inside_selection,
        point,
    });
    crate::canvas_popup::mark_open(ctx, menu_id());
    true
}

/// Right-click with the Zoom or Hand tool at screen point `pos`, over document point `doc`: the
/// view menu. With the Hand held by Space over another tool, it closes when that tool changes
/// (#11271).
pub fn open_view(app: &mut PhotocraftApp, ctx: &Context, tool: Tool, pos: [f32; 2], doc: [f64; 2]) -> bool {
    if !is_view_tool(tool) || !pos.iter().all(|v| v.is_finite()) || !doc.iter().all(|v| v.is_finite()) {
        return false;
    }
    app.ui.brush_picker = None;
    app.ui.layer_menu = None;
    app.ui.canvas_tool_menu = Some(CanvasToolMenu {
        pos,
        tool,
        has_selection: false,
        has_path: false,
        path_name: None,
        transform: false,
        pasteboard: false,
        anchor: Some(doc),
        under: (tool != app.ui.tool).then_some(app.ui.tool),
        text: false,
        type_layer: false,
        type_shape: None,
        guide: false,
        spelling: None,
        crop: None,
        sampler: None,
        sampler_at: None,
        color: None,
        inside_selection: false,
        point: None,
    });
    crate::canvas_popup::mark_open(ctx, menu_id());
    true
}

/// A view menu row. The non-temporary Zoom tool's steps keep the view centre unless clicked-point
/// centring is enabled. Other view-tool cases retain their pressed-point anchoring.
fn choose_view(app: &mut PhotocraftApp, ctx: &Context, menu: &CanvasToolMenu, command: &str) -> Result<(), String> {
    let i = app.session.active_index().ok_or("no document")?;
    if command == FILL_SCREEN {
        let v = app.ui.views.get_mut(i).ok_or("no document view")?;
        v.fit_pending = false;
        v.fill_pending = true;
        v.request_zoom_animation(app.session.prefs().general.animated_zoom);
        v.zoom_anchor = None;
        return Ok(());
    }
    let before = app.ui.views.get(i).map(|v| (v.zoom, v.center));
    crate::menus::invoke(app, ctx, command, json!({}))?;
    if !matches!(command, "view.zoomIn" | "view.zoomOut") {
        return Ok(());
    }
    let (Some((zoom0, c)), Some(d)) = (before, menu.anchor) else { return Ok(()) };
    let center_on_point = app.session.prefs().tools.zoom_clicked_point_to_center;
    if menu.tool == Tool::Zoom && app.ui.tool == Tool::Zoom && app.ui.spring.is_none() && crate::hold_keys::held_tool(app, ctx).is_none() && !center_on_point {
        return Ok(());
    }
    let Some(v) = app.ui.views.get_mut(i) else { return Ok(()) };
    let d = [d[0] as f32, d[1] as f32];
    // The view scales about the centre, so the point keeps its screen position when its offset
    // from the centre shrinks (or grows) by the zoom ratio, whatever the rotation or flip.
    let ratio = zoom0 / v.zoom;
    v.center = if center_on_point || !ratio.is_finite() { d } else { [d[0] - (d[0] - c[0]) * ratio, d[1] - (d[1] - c[1]) * ratio] };
    v.zoom_anchor = (v.zoom_animating && !center_on_point).then_some(([d[0] as f64, d[1] as f64], menu.pos));
    Ok(())
}

pub fn choose(app: &mut PhotocraftApp, ctx: &Context, command: &str) {
    let Some(menu) = app.ui.canvas_tool_menu.take() else { return };
    if !available(app, &menu, command) {
        return;
    }
    let result = if is_point(&menu, command) {
        menu.point.as_ref().map_or(Ok(()), |(id, params, _)| app.run(id, params.clone()).map(|_| ()))
    } else if menu.crop.is_some() {
        choose_crop(app, &menu, command)
    } else if menu.guide {
        choose_guide(app, ctx, command)
    } else if menu.pasteboard {
        choose_pasteboard(app, &menu, command)
    } else if menu.anchor.is_some() {
        choose_view(app, ctx, &menu, command)
    } else if menu.text && is_spelling(&menu, command) {
        choose_spelling(app, &menu, command)
    } else if menu.text || menu.type_layer {
        choose_type(app, ctx, command)
    } else if path_menu(menu.tool) && !menu.transform {
        choose_pen(app, ctx, &menu, command)
    } else if sampler_menu(&menu) {
        let (id, params) = engine_call(app, &menu, command);
        app.run(&id, params).map(|_| ())
    } else if is_eyedropper(&menu) {
        choose_eyedropper(app, ctx, &menu, command)
    } else if command == "select.toWorkPath" {
        // Photoshop's Make Work Path… asks for the tolerance first.
        spec_dialog(app, command, "Make Work Path", r##"{"tolerance":0.5..10=2}"##, serde_json::Map::new());
        Ok(())
    } else {
        crate::menus::invoke(app, ctx, command, json!({})).map(|_| ())
    };
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// An Eyedropper row (#11747): a size sets the Sample Size (shared with the options bar); a Copy
/// Color row puts the colour read at the right-click on the clipboard, as the Color panel's menu
/// does. Neither touches the foreground colour or the history.
fn choose_eyedropper(app: &mut PhotocraftApp, ctx: &Context, menu: &CanvasToolMenu, command: &str) -> Result<(), String> {
    if let Some(size) = eyedropper_size(command) {
        app.ui.tool_options.eyedropper_size = size;
        return Ok(());
    }
    let rgb = menu.color.ok_or("No colour under the pointer")?;
    let text = if command == EYEDROPPER_COPY_HTML { crate::color_panel_ui::html_color(rgb) } else { crate::color_panel_ui::hex_code(rgb) };
    ctx.copy_text(text);
    Ok(())
}

/// A spelling row: replace the word under the pointer (the caret lands after the replacement), or
/// add it to the user dictionary.
fn choose_spelling(app: &mut PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> Result<(), String> {
    let (id, params) = engine_call(app, menu, command);
    let result = app.run(&id, params)?;
    if let Some(end) = result["end"].as_u64().map(|e| e as usize)
        && let Some(e) = app.ui.text_edit.as_mut()
    {
        (e.caret, e.anchor) = (end, end);
    }
    Ok(())
}

/// A type edit row. Undo, Cut, Copy, Paste and Select All run as their Edit and Select menu items,
/// which act on the text while it is being edited (Paste asks the window for the clipboard text).
fn choose_type(app: &mut PhotocraftApp, ctx: &Context, command: &str) -> Result<(), String> {
    match command {
        FAUX_BOLD | FAUX_ITALIC => {
            let bold = command == FAUX_BOLD;
            let on = !type_checked(app, command);
            // While editing, the selection's characters are styled; over a committed type layer
            // the same run takes the whole layer, which is what formatting_params scopes to with
            // no selection (#12043). Either way one `type.setStyle` run is one history state.
            let mut p = crate::type_tool::formatting_params(app).ok_or("No type to format")?;
            p[if bold { "fauxBold" } else { "fauxItalic" }] = json!(on);
            app.run("type.setStyle", p)?;
        }
        _ => {
            if ENDS_TYPE_EDIT.contains(&command) {
                crate::type_tool::commit(app);
            }
            crate::menus::invoke(app, ctx, command, json!({}))?;
        }
    }
    Ok(())
}

/// A parameter dialog for `command` following `spec` (registry notation), prefilled with `fields`.
fn spec_dialog(app: &mut PhotocraftApp, command: &str, label: &str, spec: &str, mut fields: serde_json::Map<String, serde_json::Value>) {
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(label));
    fields.insert("__filter".into(), json!(true));
    fields.insert("__spec".into(), json!(spec));
    app.ui.open_dialog(crate::state::DialogKind::Command, fields);
}

/// Photoshop's Stroke Path dialog: the brush-based tool to stroke with (painting with that tool's
/// current options) and Simulate Pressure (#2978).
const STROKE_PATH_SPEC: &str = r##"{"tool":"brush|pencil|eraser|backgroundEraser|cloneStamp|patternStamp|healingBrush|spotHealingBrush|historyBrush|blur|sharpen|smudge|dodge|burn|sponge|colorReplacement|mixerBrush"="brush","simulatePressure":bool=false}"##;

/// Where the Stroke Path dialog's Simulate Pressure choice is remembered: the UI-state preferences'
/// dialogs map, under the command id, as `edit.fill` and `view.newGuide` use their own (#10527).
const STROKE_PATH_REMEMBERED: &str = "path.stroke";

/// The choice the next Stroke Path dialog opens with: what the last OK applied, else off. Saved data
/// is never trusted — anything but an object holding a real bool (a hand-edited preferences file,
/// an agent that wrote something else) falls back to Photoshop's default, off.
fn remembered_simulate_pressure(app: &PhotocraftApp) -> bool {
    app.session
        .prefs()
        .dialogs
        .get(STROKE_PATH_REMEMBERED)
        .and_then(Value::as_object)
        .and_then(|saved| saved.get("simulatePressure"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// OK ran the stroke: remember the Simulate Pressure it applied so the dialog opens with it again
/// next time (#10527). Cancel never reaches here, so cancelling changes nothing, and a stroke run
/// without the dialog (the Paths panel's Stroke button) writes nothing either. A stroke whose params
/// said nothing about pressure applied the engine's default, off.
pub(crate) fn remember_simulate_pressure(app: &mut PhotocraftApp, params: &Value) {
    let on = params.get("simulatePressure").and_then(Value::as_bool).unwrap_or(false);
    app.session.prefs.edit(|p| p.dialogs.insert(STROKE_PATH_REMEMBERED.into(), json!({ "simulatePressure": on })));
}

/// The Stroke Path dialog for the path `name` (Paths panel menu, Alt-click on its Stroke button).
pub(crate) fn stroke_path_dialog(app: &mut PhotocraftApp, name: &str) {
    pen_dialog(app, "path.stroke", "Stroke Path…", name);
}

fn pen_dialog(app: &mut PhotocraftApp, command: &str, label: &str, name: &str) {
    let spec = match command {
        "edit.defineCustomShape" => r##"{"name":text}"##,
        "path.toSelection" => r##"{"feather":0..250=0,"antiAlias":bool=true,"mode":"replace|add|subtract|intersect"="replace"}"##,
        "path.fill" => r##"{"color":text,"opacity":0..100=100,"feather":0..250=0,"antiAlias":bool=true}"##,
        "path.stroke" => STROKE_PATH_SPEC,
        "path.clippingPath.set" => r##"{"flatness":0..100=0}"##,
        "path.transform" => r##"{"translateX":-30000..30000=0,"translateY":-30000..30000=0,"scaleX":0.01..100=1,"scaleY":0.01..100=1,"angle":-360..360=0}"##,
        _ => "{}",
    };
    let mut fields = serde_json::Map::new();
    if command == "edit.defineCustomShape" {
        fields.insert("path".into(), json!(name));
        fields.insert("name".into(), json!(""));
    } else {
        fields.insert("name".into(), json!(name));
    }
    if command == "path.toSelection" {
        // OK deselects and hides the path, as ⌘↩ does (#6079).
        fields.insert("__deselectPath".into(), json!(true));
        fields.insert(MAKE_SELECTION.into(), json!(true));
    }
    if command == "path.fill" {
        let fg = app.session.tools.foreground;
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        fields.insert("color".into(), json!(format!("#{:02x}{:02x}{:02x}", byte(fg[0]), byte(fg[1]), byte(fg[2]))));
    }
    if command == "path.transform" {
        for (key, value) in [("translateX", 0.0), ("translateY", 0.0), ("scaleX", 1.0), ("scaleY", 1.0), ("angle", 0.0)] {
            fields.insert(key.into(), json!(value));
        }
    }
    if command == "path.stroke" {
        // The spec-driven dialog reappears with fresh fields, so the checkbox needs its own seed:
        // a schema dialog carrying a `__spec` is outside `filter_dialog::remember`. Only Simulate
        // Pressure is remembered; the tool stays on the spec's default (#10527).
        fields.insert("simulatePressure".into(), json!(remembered_simulate_pressure(app)));
    }
    spec_dialog(app, command, label, spec, fields);
}

/// Marks the Make Selection dialog [`pen_dialog`] opens. It keeps the schema dialog's fields, so OK
/// and Alt+Reset work as before; only the body is Photoshop's layout (#12917).
const MAKE_SELECTION: &str = "__makeSelection";

/// The Make Selection operations, in Photoshop's order: (`path.toSelection` mode, label).
const MAKE_SELECTION_OPERATIONS: [(&str, &str); 4] =
    [("replace", "New Selection"), ("add", "Add to Selection"), ("subtract", "Subtract from Selection"), ("intersect", "Intersect with Selection")];

/// Is this dialog the Make Selection dialog?
pub(crate) fn owns_make_selection(fields: &serde_json::Map<String, Value>) -> bool {
    fields.contains_key(MAKE_SELECTION)
}

/// Photoshop's Make Selection dialog: Rendering (Feather Radius, Anti-aliased) and Operation.
/// Without a selection there is nothing to combine with, so only New Selection is available.
pub(crate) fn make_selection_body(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut serde_json::Map<String, Value>) {
    let has_selection = app.session.active().is_some_and(|st| st.doc.selection.is_some());
    ui.spacing_mut().item_spacing.y = 6.0;
    crate::widgets::section_label(ui, tl!("Rendering"));
    crate::widgets::hairline(ui);
    ui.horizontal(|ui| {
        ui.label(tl!("Feather Radius:"));
        let mut feather = f.get("feather").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        crate::widgets::value_field(ui, &mut feather, 0.0..=250.0, "", 64.0).accessible_name(tl!("Feather Radius:"));
        ui.label(tl!("pixels"));
        // The schema dialog's rounding, so OK sends what it sent.
        f.insert("feather".into(), json!((feather * 10.0).round() / 10.0));
    });
    let mut anti_alias = f.get("antiAlias").and_then(Value::as_bool).unwrap_or(true);
    crate::widgets::checkbox(ui, &mut anti_alias, tl!("Anti-aliased"));
    f.insert("antiAlias".into(), json!(anti_alias));
    ui.add_space(4.0);
    crate::widgets::section_label(ui, tl!("Operation"));
    crate::widgets::hairline(ui);
    let mut mode = f.get("mode").and_then(Value::as_str).unwrap_or("replace").to_string();
    // A combining mode set from outside, or a selection gone while the dialog was open.
    if !has_selection {
        mode = "replace".into();
    }
    for (key, label) in MAKE_SELECTION_OPERATIONS {
        ui.add_enabled_ui(key == "replace" || has_selection, |ui| ui.radio_value(&mut mode, key.to_string(), tl!(label)));
    }
    f.insert("mode".into(), json!(mode));
}

/// OK in the Make Selection dialog: a new selection is the History step Selection Change, as in
/// Photoshop (#12917). ⌘↩, the Paths panel and the combining operations keep the engine's Make
/// Selection (not measured). Only a step this run added is renamed: `revision` is from before it.
pub(crate) fn name_make_selection_step(app: &mut PhotocraftApp, fields: &serde_json::Map<String, Value>, params: &Value, revision: Option<u64>) {
    if !owns_make_selection(fields) || params.get("mode").and_then(Value::as_str).is_some_and(|m| m != "replace") {
        return;
    }
    if let Some(st) = app.session.active_mut().filter(|st| Some(st.revision) != revision) {
        st.history.set_current_label("Selection Change");
    }
}

/// The Pen options bar's Make Selection (Path mode, #5703): the canvas menu's Make Selection…
/// dialog for the path being drawn (committed first, as the menu does) or the active path.
pub(crate) fn make_selection(app: &mut PhotocraftApp) -> Result<(), String> {
    let pending = app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 2);
    // Where `pen_commit` puts the path being drawn: a shape layer or the targeted vector mask,
    // else the Paths panel path it adds to (a selected saved path, or the Work Path), which it
    // selects so it is the active path.
    let pending_layer = pending && (app.ui.tool_options.vector_mode == "shape" || crate::vector_ui::targeted_vector_mask(app).is_some());
    if pending && !crate::vector_ui::pen_commit(app, false) {
        return Err(app.ui.status.clone());
    }
    let name = if pending_layer { Some("layer".to_string()) } else { crate::vector_ui::active_path_name(app) }
        .ok_or_else(|| "No path is available for this action".to_string())?;
    pen_dialog(app, "path.toSelection", "Make Selection…", &name);
    Ok(())
}

fn choose_pen(app: &mut PhotocraftApp, ctx: &Context, menu: &CanvasToolMenu, command: &str) -> Result<(), String> {
    if command == "select.isolateLayers" {
        crate::menus::invoke(app, ctx, command, json!({}))?;
        return Ok(());
    }
    if command == "paint.symmetryDisable" {
        app.run(command, json!({}))?;
        return Ok(());
    }
    let pending = app.ui.pen.as_ref().is_some_and(|p| p.knots.len() >= 2);
    let pending_layer = pending && (app.ui.tool_options.vector_mode == "shape" || app.ui.vector_mask_target);
    if pending && !crate::vector_ui::pen_commit(app, false) {
        return Err(app.ui.status.clone());
    }
    let name = if pending { Some(if pending_layer { "layer" } else { "work" }.to_string()) } else { menu.path_name.clone() }
        .ok_or_else(|| "No path is available for this action".to_string())?;
    match command {
        "edit.defineCustomShape" | "path.toSelection" | "path.fill" | "path.stroke" | "path.clippingPath.set" | "path.transform" => {
            let label = PEN_MENU.iter().flatten().find(|(_, id)| *id == command).map_or(command, |(label, _)| *label);
            pen_dialog(app, command, label, &name);
        }
        _ => {
            app.run(command, json!({"name": name}))?;
        }
    }
    Ok(())
}

fn menu_id() -> egui::Id {
    egui::Id::unique("canvas-selection-menu")
}

pub fn show(app: &mut PhotocraftApp, ctx: &Context) {
    let Some(mut menu) = app.ui.canvas_tool_menu.clone() else { return };
    if ctx.input(|i| i.key_pressed(egui::Key::Escape))
        || app.ui.tool != menu.under.unwrap_or(menu.tool)
        || (menu.transform && app.ui.transform.is_none())
        || (menu.text && app.ui.text_edit.is_none())
        || (menu.type_layer && app.ui.text_edit.is_some())
        || (menu.crop.is_some() && !crate::crop_ui::pending(app))
        || (sampler_menu(&menu) && !sampler_current(app, &menu))
    {
        app.ui.canvas_tool_menu = None;
        return;
    }
    // The committed text's shape can change while its menu is open - a Convert, or an Undo of
    // one: the Convert row follows the shape frame by frame instead of freezing on the value
    // captured when the menu opened (#12043 review). The stored menu is refreshed too, so the
    // control channel's entries and a row pick see the same list the canvas draws.
    if menu.type_layer {
        let fresh = Some(active_type_shape(app));
        if menu.type_shape != fresh {
            if let Some(m) = app.ui.canvas_tool_menu.as_mut() {
                m.type_shape = fresh;
            }
            menu.type_shape = fresh;
        }
    }
    let id = menu_id();
    let screen = ctx.content_rect();
    let size = ctx.memory(|m| m.area_rect(id)).map_or(egui::vec2(210.0, 140.0), |r| r.size());
    let pos = egui::pos2(menu.pos[0].min(screen.right() - size.x).max(screen.left()), menu.pos[1].min(screen.bottom() - size.y).max(screen.top()));
    let mut selected = None;
    let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            let t = crate::theme::Tokens::get(ui.ctx());
            let v = &mut ui.style_mut().visuals;
            v.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
            v.widgets.hovered.weak_bg_fill = t.accent;
            v.widgets.hovered.bg_fill = t.accent;
            v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
            v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
            v.widgets.hovered.corner_radius = egui::CornerRadius::same(3);
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_width(200.0);
            crate::widgets::menu_scroll(ui, |ui| {
                let spelling = spelling_rows(&menu);
                for (label, command) in &spelling {
                    let text = if spelling_label_is_ui(command) { tl!(label).to_string() } else { label.clone() };
                    let item = egui::Button::new(text).min_size(egui::vec2(200.0, 20.0));
                    if ui.add_enabled(entry_enabled(app, &menu, command), item).clicked() {
                        selected = Some(command.clone());
                    }
                }
                if !spelling.is_empty() {
                    ui.separator();
                }
                for row in crop_rows(&menu) {
                    let Some((label, command)) = row else {
                        ui.separator();
                        continue;
                    };
                    let text = if crop_label_is_ui(&command) { tl!(&label).to_string() } else { label };
                    // The overlay rows carry their own glyph (#10889); the rest stay text-only.
                    let mut atoms = egui::Atoms::default();
                    if let Some(kind) = command.strip_prefix(CROP_OVERLAY).and_then(crate::crop_overlay::CropOverlay::from_id) {
                        atoms.push_right(crate::icons::image(kind.icon(), 14.0, t.icon));
                    }
                    atoms.push_right(text);
                    let item = egui::Button::selectable(mode_checked(app, &menu, &command), atoms).min_size(egui::vec2(200.0, 20.0));
                    if ui.add_enabled(entry_enabled(app, &menu, &command), item).clicked() {
                        selected = Some(command);
                    }
                }
                if let Some((command, ..)) = &menu.point {
                    let item = egui::Button::new(point_label(command)).min_size(egui::vec2(200.0, 20.0));
                    if ui.add_enabled(available(app, &menu, command), item).clicked() {
                        selected = Some(command.clone());
                    }
                    ui.separator();
                }
                for row in rows(&menu) {
                    let Some((label, command)) = *row else {
                        ui.separator();
                        continue;
                    };
                    let text = match row_label(app, &menu, label, command) {
                        // Spelled out so `cargo xtask i18n-coverage` lists the new label.
                        "Free Transform Points" => tl!("Free Transform Points"),
                        key => tl!(key),
                    };
                    let item = egui::Button::selectable(mode_checked(app, &menu, command), text).min_size(egui::vec2(200.0, 20.0));
                    if ui.add_enabled(entry_enabled(app, &menu, command), item).clicked() {
                        selected = Some((*command).to_string());
                    }
                }
            });
        });
    });
    if let Some(command) = selected {
        choose(app, ctx, &command);
    } else if crate::canvas_popup::outside_pressed(ctx, id, area.response.rect) {
        app.ui.canvas_tool_menu = None;
        // A fast click can press and release in one frame: the canvas input above already
        // suppressed its action through the open-menu hit test, and the deferred release clear
        // (in `canvas_view`, below the click handling) would never fire after this frame. Don't
        // arm the flag then, or it would eat the next ordinary click (CodeRabbit, #9841).
        app.ui.canvas_tool_menu_dismissed = !ctx.input(|i| i.pointer.any_released());
    }
}

/// Returns the menu rect if the canvas tool menu is currently open, for hit-testing
/// outside-click dismissal in the click handler.
pub fn menu_rect_if_open(app: &PhotocraftApp, ctx: &Context) -> Option<egui::Rect> {
    let menu = app.ui.canvas_tool_menu.as_ref()?;
    let id = menu_id();
    let screen = ctx.content_rect();
    let size = ctx.memory(|m| m.area_rect(id)).map_or(egui::vec2(210.0, 140.0), |r| r.size());
    let pos = egui::pos2(menu.pos[0].min(screen.right() - size.x).max(screen.left()), menu.pos[1].min(screen.bottom() - size.y).max(screen.top()));
    Some(egui::Rect::from_min_size(pos, size))
}

#[cfg(test)]
mod cam_menu_tests;
#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
pub(crate) mod count_tests;

#[cfg(test)]
mod hand_menu_entries_tests;

#[cfg(test)]
mod pen_free_transform_label_tests;

#[cfg(test)]
mod make_selection_dialog_tests;

#[cfg(test)]
mod type_layer_menu_tests;

#[cfg(test)]
mod delete_anchor_menu_tests;

#[cfg(test)]
mod convert_point_menu_tests;

#[cfg(test)]
mod object_selection_menu_tests;
