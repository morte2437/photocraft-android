//! The Layers panel's filter row (#5048): a filter-type dropdown, as in Photoshop, then the
//! controls for that type. Kind toggles layer kinds, Name matches part of the layer name, Effect
//! picks one effect kind (#10880), Mode picks one blend mode, Attribute one layer attribute or
//! its opposite (#10881), Color one layer colour label (#5898), Smart Object toggles linked,
//! missing and embedded smart objects, Artboard lists only the artboards that hold the selected
//! layers. The criteria live in `UiState` (`layer_filter*`), so the control channel can read and
//! set them.

use std::collections::HashSet;

use photocraft_color::BlendMode;
use photocraft_doc::{Document, Fill, LabelColor, Layer, LayerContent, LayerId, SmartSource};
use serde::{Deserialize, Serialize};

use crate::state::UiState;
use crate::{PhotocraftApp, icons, widgets};

/// What the Layers panel filters by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterBy {
    #[default]
    Kind,
    Name,
    Effect,
    Mode,
    Attribute,
    Color,
    SmartObject,
    Artboard,
}

/// The filter criteria, copied out of `UiState` for one frame of the Layers panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    pub by: FilterBy,
    pub kinds: Vec<String>,
    /// As typed; matched trimmed and ignoring case.
    pub name: String,
    pub mode: BlendMode,
    pub effect: effect::EffectKind,
    pub attribute: attribute::Attribute,
    pub color: LabelColor,
    /// The Smart Object toggles that are on, from [`SMART`].
    pub smart: Vec<String>,
    /// The linked files the Smart Object filter's Missing toggle lists
    /// ([`photocraft_engine::Session::missing_links`]).
    pub missing: HashSet<String>,
    /// The rows the Artboard filter lists ([`board_rows`]); empty lists every row.
    pub board_rows: HashSet<LayerId>,
    /// The empty layers the Attribute filter's Empty and Not Empty use
    /// ([`attribute::empty_layers`]); `None` reads each layer's pixels.
    pub empty: Option<std::sync::Arc<HashSet<LayerId>>>,
}

impl Filter {
    pub fn of(ui: &UiState) -> Self {
        Self {
            by: ui.layer_filter_by,
            kinds: ui.layer_filter.clone(),
            name: ui.layer_filter_name.clone(),
            mode: ui.layer_filter_mode,
            effect: ui.layer_filter_effect,
            attribute: ui.layer_filter_attribute,
            color: ui.layer_filter_color,
            smart: ui.layer_filter_smart.clone(),
            missing: HashSet::new(),
            board_rows: HashSet::new(),
            empty: None,
        }
    }

    /// Returns [`Filter::of`] for `app`'s settings, with the active document's missing linked
    /// files while the panel filters by Smart Object, and `board_rows` set to the rows of the
    /// artboards in `doc` that hold `selection` while it filters by Artboard, and `empty` set
    /// while it filters by Empty or Not Empty. The Layers panel passes the document and the
    /// selection it already read for the frame.
    pub fn with_selection(app: &PhotocraftApp, doc: &Document, selection: &[LayerId]) -> Self {
        let mut f = Self::of(&app.ui);
        if f.by == FilterBy::SmartObject {
            f.missing = app.session.missing_links();
        }
        if f.by == FilterBy::Artboard {
            f.board_rows = board_rows(doc, selection);
        }
        if f.by == FilterBy::Attribute {
            f.empty = attribute::empty_layers(app, f.attribute);
        }
        f
    }

    /// Returns [`Filter::with_selection`] for the active document and its selected layers, or
    /// [`Filter::of`] with no artboard rows when no document is open.
    pub fn of_app(app: &PhotocraftApp) -> Self {
        match app.session.active() {
            Some(d) => Self::with_selection(app, &d.doc, &d.selected_layers()),
            None => Self::of(&app.ui),
        }
    }

    /// Whether the panel is filtered. Like Photoshop, Effect, Mode, Attribute and Color always
    /// filter (by Bevel & Emboss, by Normal, by Visible and by no colour at first); Kind, Name and
    /// Smart Object only once a toggle is on or some text is typed; Artboard only while some layers
    /// are selected and each of them is in an artboard.
    pub fn active(&self) -> bool {
        match self.by {
            FilterBy::Kind => !self.kinds.is_empty(),
            FilterBy::Name => !self.name.trim().is_empty(),
            FilterBy::Effect | FilterBy::Mode | FilterBy::Attribute | FilterBy::Color => true,
            FilterBy::SmartObject => !self.smart.is_empty(),
            FilterBy::Artboard => !self.board_rows.is_empty(),
        }
    }

    /// Whether `l` is listed under this filter.
    pub fn shows(&self, l: &Layer) -> bool {
        if !self.active() {
            return true;
        }
        match self.by {
            FilterBy::Kind => self.kinds.iter().any(|k| k == kind(l)),
            FilterBy::Name => l.name.to_lowercase().contains(&self.name.trim().to_lowercase()),
            FilterBy::Effect => self.effect.shows(l),
            FilterBy::Mode => l.blend == self.mode,
            FilterBy::Attribute => self.attribute.shows(l, self.empty.as_deref()),
            FilterBy::Color => l.label == self.color,
            FilterBy::SmartObject => {
                let LayerContent::Smart(sm) = &l.content else { return false };
                self.smart.iter().any(|k| match (k.as_str(), &sm.source) {
                    ("linked", SmartSource::Linked { .. }) | ("embedded", SmartSource::Embedded { .. }) => true,
                    ("missing", SmartSource::Linked { path }) => self.missing.contains(path),
                    _ => false,
                })
            }
            FilterBy::Artboard => self.board_rows.contains(&l.id),
        }
    }
}

/// The Artboard filter's rows for `selection`: the artboards that hold the selected layers and
/// every layer inside them. The set is empty when nothing is selected or a selected layer is in
/// no artboard, and the panel is then not filtered. The panel lists these rows as its tree does: the contents of a closed
/// group stay hidden until the group is opened (`layer_tree_ui::panel_rows` opens every group only
/// for the types whose matches can sit in a closed group).
pub fn board_rows(doc: &Document, selection: &[LayerId]) -> HashSet<LayerId> {
    fn add(l: &Layer, rows: &mut HashSet<LayerId>) {
        let mut pending = vec![l];
        while let Some(layer) = pending.pop() {
            rows.insert(layer.id);
            pending.extend(layer.children().unwrap_or_default());
        }
    }
    let selected: HashSet<LayerId> = selection.iter().copied().collect();
    if selected.is_empty() {
        return HashSet::new();
    }
    // Artboards are top-level groups ([`Document::artboard_of`]): each one's layers are walked once.
    let (mut rows, mut held) = (HashSet::new(), 0);
    for board in doc.layers.iter().filter(|l| l.artboard().is_some()) {
        let mut inside = HashSet::new();
        add(board, &mut inside);
        let n = selected.iter().filter(|id| inside.contains(id)).count();
        if n > 0 {
            held += n;
            rows.extend(inside);
        }
    }
    if held < selected.len() {
        return HashSet::new();
    }
    rows
}

/// The kinds the Kind filter toggles.
pub const KINDS: [&str; 5] = ["pixel", "adjustment", "type", "shape", "smart"];

/// The Smart Object filter's toggles: smart objects linked to a file, linked to a file that is
/// not there, and embedded in the document.
pub const SMART: [&str; 3] = ["linked", "missing", "embedded"];

/// `ui.set {layerFilter: {by?, kinds?, name?, effect?, mode?, attribute?, color?, smart?}}`:
/// `current` with the given fields replaced, or an error naming the bad field (nothing changes
/// then). `by` is `kind`, `name`, `effect`, `mode`, `attribute`, `color`, `smartObject` or
/// `artboard`; `kinds` a list of [`KINDS`]; `effect` an [`effect::EffectKind`] id; `mode` one of
/// the layer blend modes such as `Multiply` (not Pass Through, which the Mode list doesn't offer,
/// #10878); `attribute` an [`attribute::Attribute`] id; `color` a label colour id (`none`, `red`,
/// … `gray`, [`LabelColor::id`]); `smart` a list of [`SMART`]. Artboard has no criteria of its
/// own: the selected layers pick the artboards.
pub fn patched(current: &UiState, v: &serde_json::Value) -> Result<Filter, String> {
    let Some(o) = v.as_object() else { return Err("layerFilter must be an object".into()) };
    if let Some(k) = o.keys().find(|k| !["by", "kinds", "name", "effect", "mode", "attribute", "color", "smart"].contains(&k.as_str())) {
        return Err(format!("unknown layerFilter field `{k}` (fields: by, kinds, name, effect, mode, attribute, color, smart)"));
    }
    let mut f = Filter::of(current);
    if let Some(by) = o.get("by") {
        f.by = serde_json::from_value(by.clone())
            .map_err(|_| "layerFilter.by must be kind, name, effect, mode, attribute, color, smartObject or artboard".to_string())?;
    }
    if let Some(kinds) = o.get("kinds") {
        let list = kinds.as_array().ok_or("layerFilter.kinds must be a list")?;
        f.kinds = Vec::new();
        for k in list {
            let Some(k) = k.as_str().filter(|k| KINDS.contains(k)) else {
                return Err(format!("layerFilter.kinds: unknown kind {k} (kinds: {})", KINDS.join(", ")));
            };
            if !f.kinds.iter().any(|have| have == k) {
                f.kinds.push(k.to_string());
            }
        }
    }
    if let Some(name) = o.get("name") {
        f.name = name.as_str().ok_or("layerFilter.name must be a string")?.to_string();
    }
    if let Some(mode) = o.get("mode") {
        let name = mode.as_str().ok_or("layerFilter.mode must be a blend mode name")?;
        f.mode = photocraft_engine::commands::blend_from_str(name).filter(|m| BlendMode::LAYER_MODES.contains(m)).ok_or_else(|| {
            let modes: Vec<&str> = BlendMode::LAYER_MODES.iter().map(|m| m.label()).collect();
            format!("layerFilter.mode: `{name}` is not a layer blend mode the filter lists (modes: {})", modes.join(", "))
        })?;
    }
    if let Some(e) = o.get("effect") {
        f.effect = effect::parse(e)?;
    }
    if let Some(a) = o.get("attribute") {
        f.attribute = attribute::parse(a)?;
    }
    if let Some(color) = o.get("color") {
        let id = color.as_str().ok_or("layerFilter.color must be a label colour id")?;
        f.color = LabelColor::ALL.into_iter().find(|c| c.id() == id).ok_or_else(|| {
            let ids: Vec<&str> = LabelColor::ALL.iter().map(|c| c.id()).collect();
            format!("layerFilter.color: unknown colour `{id}` (colours: {})", ids.join(", "))
        })?;
    }
    if let Some(smart) = o.get("smart") {
        let list = smart.as_array().ok_or("layerFilter.smart must be a list")?;
        f.smart = Vec::new();
        for k in list {
            let Some(k) = k.as_str().filter(|k| SMART.contains(k)) else {
                return Err(format!("layerFilter.smart: unknown toggle {k} (toggles: {})", SMART.join(", ")));
            };
            if !f.smart.iter().any(|have| have == k) {
                f.smart.push(k.to_string());
            }
        }
    }
    Ok(f)
}

/// Store `f` as the Layers panel's filter.
pub fn apply(ui: &mut UiState, f: Filter) {
    ui.layer_filter_by = f.by;
    ui.layer_filter = f.kinds;
    ui.layer_filter_name = f.name;
    ui.layer_filter_mode = f.mode;
    ui.layer_filter_effect = f.effect;
    ui.layer_filter_attribute = f.attribute;
    ui.layer_filter_color = f.color;
    ui.layer_filter_smart = f.smart;
}

/// The kind filter's name for `l`'s content ("" for groups, which no kind matches).
fn kind(l: &Layer) -> &'static str {
    match &l.content {
        LayerContent::Raster(_) => "pixel",
        LayerContent::Adjustment(_) => "adjustment",
        // A fill layer (Layer › New Fill Layer) matches no Kind button, as in the reference
        // application, except a solid colour with a vector mask, which lists as a shape (#10878).
        LayerContent::Fill(Fill::Solid(_)) if l.vector_mask.is_some() => "shape",
        LayerContent::Fill(_) => "",
        LayerContent::Text(_) => "type",
        LayerContent::Shape(_) => "shape",
        LayerContent::Smart(_) => "smart",
        LayerContent::Group(_) => "",
    }
}

/// Draw the filter row: the type dropdown, then that type's controls.
pub fn bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let by = [
            (FilterBy::Kind, tl!("Kind")),
            (FilterBy::Name, tl!("Name")),
            (FilterBy::Effect, tl!("Effect")),
            (FilterBy::Mode, tl!("Mode")),
            (FilterBy::Attribute, tl!("Attribute")),
            (FilterBy::Color, tl!("Color")),
            (FilterBy::SmartObject, tl!("Smart Object")),
            (FilterBy::Artboard, tl!("Artboard")),
        ];
        widgets::dropdown(ui, "layer-filter-by", tl!("Kind"), &mut app.ui.layer_filter_by, &by, 64.0);
        ui.add_space(4.0);
        match app.ui.layer_filter_by {
            FilterBy::Kind => {
                for (kind, icon, tip) in [
                    ("pixel", "image", tl!("Filter for pixel layers")),
                    ("adjustment", "adjustment-layer", tl!("Filter for adjustment layers")),
                    ("type", "type", tl!("Filter for type layers")),
                    ("shape", "square", tl!("Filter for shape layers")),
                    ("smart", "app-window", tl!("Filter for smart objects")),
                ] {
                    let on = app.ui.layer_filter.iter().any(|k| k == kind);
                    if icons::button(ui, icon, 22.0, on, tip).clicked() {
                        if on {
                            app.ui.layer_filter.retain(|k| k != kind);
                        } else {
                            app.ui.layer_filter.push(kind.to_string());
                        }
                    }
                }
            }
            FilterBy::Name => {
                let width = ui.available_width().max(48.0);
                widgets::text_field(ui, ui.scope_id().with("layer-filter-name"), &mut app.ui.layer_filter_name, tl!("Search"), width, false);
            }
            FilterBy::Effect => effect::controls(app, ui),
            FilterBy::Mode => {
                // The 27 layer modes, without Pass Through: a group set to Pass Through is never
                // listed by Mode (#10878).
                let modes: Vec<(BlendMode, &str)> = BlendMode::LAYER_MODES.into_iter().map(|m| (m, m.label())).collect();
                let width = ui.available_width().max(96.0);
                widgets::dropdown(ui, "layer-filter-mode", tl!("Mode"), &mut app.ui.layer_filter_mode, &modes, width);
            }
            FilterBy::Attribute => attribute::controls(app, ui),
            FilterBy::Color => {
                // Photoshop's list: None, then the label colours in the Layers panel menu's order.
                let colors: Vec<(LabelColor, &str)> =
                    LabelColor::ALL.into_iter().map(|c| (c, if c == LabelColor::None { tl!("None") } else { tl!(c.label()) })).collect();
                let width = ui.available_width().max(96.0);
                widgets::dropdown(ui, "layer-filter-color", tl!("Color"), &mut app.ui.layer_filter_color, &colors, width);
            }
            FilterBy::SmartObject => {
                for (kind, icon, tip) in [
                    ("linked", "link", tl!("Filter for smart objects linked locally")),
                    ("missing", "unlink", tl!("Filter for linked smart objects that are missing")),
                    ("embedded", "file", tl!("Filter for embedded smart objects")),
                ] {
                    let on = app.ui.layer_filter_smart.iter().any(|k| k == kind);
                    if icons::button(ui, icon, 22.0, on, tip).clicked() {
                        if on {
                            app.ui.layer_filter_smart.retain(|k| k != kind);
                        } else {
                            app.ui.layer_filter_smart.push(kind.to_string());
                        }
                    }
                }
            }
            // Artboard has no controls: the selected layers pick the artboards ([`board_rows`]).
            FilterBy::Artboard => {}
        }
    });
    selection::follow(app, ui.ctx());
}

pub mod effect;
pub mod selection;

#[cfg(test)]
mod effect_tests;

pub mod attribute;

#[cfg(test)]
mod attribute_tests;
#[cfg(test)]
mod selection_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod kind_mode_tests;
