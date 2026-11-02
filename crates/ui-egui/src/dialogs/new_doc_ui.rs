//! File › New: Photoshop's New Document dialog. Category tabs with blank-document presets on the
//! left, Preset Details on the right. Values live in the dialog fields (`width`/`height` in pixels,
//! `resolution` in ppi, `mode`, `depth`, `background`, `name`), so `ui.dialog.set` drives it and
//! Create runs `file.new` with them.

use egui::{Align2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Map, Value, json};

use photocraft_engine::prefs::UnitsAndRulers;

use crate::theme::Tokens;
use crate::{icons, widgets};
use photocraft_engine::document_preset_cmds::DocumentSettings;

/// A blank-document preset: (name, width px, height px, ppi).
pub type Preset = (&'static str, u32, u32, f32);

/// Photoshop's New Document categories and their blank-document presets.
pub const CATEGORIES: &[(&str, &[Preset])] = &[
    ("Recent", &[("Default PhotoCraft Size", 2100, 1500, 300.0), ("HDTV 1080p", 1920, 1080, 72.0)]),
    (
        "Photo",
        &[
            ("Landscape, 6 x 4", 1800, 1200, 300.0),
            ("Landscape, 7 x 5", 2100, 1500, 300.0),
            ("Landscape, 10 x 8", 3000, 2400, 300.0),
            ("Portrait, 4 x 6", 1200, 1800, 300.0),
            ("Portrait, 5 x 7", 1500, 2100, 300.0),
            ("Square, 5 x 5", 1500, 1500, 300.0),
        ],
    ),
    (
        "Print",
        &[
            ("Letter", 2550, 3300, 300.0),
            ("Legal", 2550, 4200, 300.0),
            ("Tabloid", 3300, 5100, 300.0),
            ("A4", 2480, 3508, 300.0),
            ("A3", 3508, 4961, 300.0),
            ("A5", 1748, 2480, 300.0),
            ("B3", 4169, 5906, 300.0),
            ("B4", 2953, 4169, 300.0),
        ],
    ),
    (
        "Art & Illustration",
        &[("Poster", 5400, 7200, 300.0), ("Postcard", 1800, 1200, 300.0), ("Comic Book", 1988, 3075, 300.0), ("Square, 12 x 12", 3600, 3600, 300.0)],
    ),
    (
        "Web",
        &[
            ("Web Most Common", 1366, 768, 72.0),
            ("Web Minimum", 1024, 768, 72.0),
            ("Web Large", 1920, 1080, 72.0),
            ("MacBook Pro 16\"", 3456, 2234, 72.0),
            ("iMac 24\"", 4480, 2520, 72.0),
        ],
    ),
    (
        "Mobile",
        &[
            ("iPhone 16", 1179, 2556, 72.0),
            ("iPhone 16 Pro Max", 1320, 2868, 72.0),
            ("iPad Pro 13\"", 2064, 2752, 72.0),
            ("Android 1080p", 1080, 1920, 72.0),
            ("Apple Watch 45mm", 396, 484, 72.0),
        ],
    ),
    (
        "Film & Video",
        &[
            ("HDTV 1080p", 1920, 1080, 72.0),
            ("HDTV 720p", 1280, 720, 72.0),
            ("UHD 4K", 3840, 2160, 72.0),
            ("DCI 4K", 4096, 2160, 72.0),
            ("UHD 8K", 7680, 4320, 72.0),
        ],
    ),
];

const DEPTH_OPTIONS: &[(u64, &str, &str)] = &[(8, "8 bit", "Integer"), (16, "16 bit", "Integer"), (32, "32 bit (float)", "Floating point")];

/// Width/Height units: (key, label, units per inch; 0 = pixels).
pub const UNITS: &[(&str, &str, f32)] =
    &[("px", "Pixels", 0.0), ("in", "Inches", 1.0), ("cm", "Centimeters", 2.54), ("mm", "Millimeters", 25.4), ("pt", "Points", 72.0), ("pica", "Picas", 6.0)];

/// Pixels to the display unit at `ppi`.
pub fn to_unit(px: f32, unit: &str, ppi: f32) -> f32 {
    match UNITS.iter().find(|u| u.0 == unit) {
        Some((_, _, per_in)) if *per_in > 0.0 => px / ppi.max(1.0) * per_in,
        _ => px,
    }
}

/// Display unit back to pixels at `ppi`.
pub fn from_unit(v: f32, unit: &str, ppi: f32) -> f32 {
    match UNITS.iter().find(|u| u.0 == unit) {
        Some((_, _, per_in)) if *per_in > 0.0 => (v / per_in * ppi.max(1.0)).round(),
        _ => v.round(),
    }
}

/// A typed size as the whole pixel count `file.new` takes (#254: a float like `512.0` isn't one, so
/// the command fell back to its 1920 x 1080 default). Clamped to the command's 1–300000 range.
pub fn px_value(px: f32) -> Value {
    json!(if px.is_finite() { px.round().clamp(1.0, 300_000.0) as u32 } else { 1 })
}

/// Name of the preset that takes the clipboard image's size.
pub const CLIPBOARD: &str = "Clipboard";

/// Offer the Clipboard preset (`w` × `h` px, at 72 ppi) first under the Recent presets, and
/// select it.
pub fn set_clipboard(f: &mut Map<String, Value>, w: u32, h: u32) {
    if w == 0 || h == 0 {
        return;
    }
    f.insert("__clipboard".into(), json!([w, h]));
    apply_preset(f, &(CLIPBOARD, w, h, 72.0), &UnitsAndRulers::default());
}

/// The clipboard image's size, when the dialog offers the Clipboard preset.
fn clipboard_preset(f: &Map<String, Value>) -> Option<Preset> {
    let size = f.get("__clipboard")?.as_array()?;
    let dim = |i: usize| size.get(i)?.as_u64().and_then(|v| u32::try_from(v).ok()).filter(|v| *v > 0);
    Some((CLIPBOARD, dim(0)?, dim(1)?, 72.0))
}

/// Set Width or Height (`key`) from a value typed in `unit`: whole pixels for `file.new`, plus the
/// typed value so the field keeps showing it (see [`shown_size`]).
pub fn set_size(f: &mut Map<String, Value>, key: &str, v: f32, unit: &str, ppi: f32) {
    f.insert(key.into(), px_value(from_unit(v, unit, ppi)));
    f.insert(typed_key(key), json!({"value": v, "unit": unit}));
    f.remove("__preset");
    f.remove("__savedPreset");
}

/// What Width or Height (`key`, `d` pixels when unset) shows in `unit`: the value the user typed
/// while it still gives the field's pixel count, else the pixels converted. Converting the rounded
/// pixels back on every keystroke turned a typed `5` mm into 14 px and the text into `4.9` (#1147).
/// A size over the 300000 px limit shows the limit, not the typed value the document won't have.
pub fn shown_size(f: &Map<String, Value>, key: &str, d: f32, unit: &str, ppi: f32) -> f32 {
    let px = get_f(f, key, d);
    let typed = f.get(&typed_key(key)).and_then(Value::as_object).filter(|t| unit != "px" && t.get("unit").and_then(Value::as_str) == Some(unit));
    match typed.and_then(|t| t.get("value")).and_then(Value::as_f64) {
        Some(v) if from_unit(v as f32, unit, ppi).max(1.0) == px => v as f32,
        _ => to_unit(px, unit, ppi),
    }
}

fn typed_key(key: &str) -> String {
    format!("__{key}Typed")
}

/// Swap Width and Height (the Orientation buttons), with the values typed for them, so a typed
/// 841 x 1189 mm turns into 1189 x 841 mm, not 1188.9 x 841.
pub fn swap_size(f: &mut Map<String, Value>) {
    f.remove("__savedPreset");
    let (w, h) = (get_f(f, "width", 1920.0), get_f(f, "height", 1080.0));
    f.insert("width".into(), px_value(h));
    f.insert("height".into(), px_value(w));
    let (typed_w, typed_h) = (f.remove(&typed_key("width")), f.remove(&typed_key("height")));
    if let Some(t) = typed_h {
        f.insert(typed_key("width"), t);
    }
    if let Some(t) = typed_w {
        f.insert(typed_key("height"), t);
    }
}

/// Print and photo presets (listed at 300 ppi) are specified in inches, screen presets in pixels.
fn is_print(p: &Preset) -> bool {
    p.3 >= 300.0
}

/// A built-in preset turned to the orientation picked with the Orientation buttons (#4668), so the
/// cards and what they apply follow it. Square presets and the Clipboard preset stay as they are.
fn oriented(p: &Preset, f: &Map<String, Value>) -> Preset {
    let portrait = match f.get("__orientation").and_then(Value::as_str) {
        Some("portrait") => true,
        Some("landscape") => false,
        _ => return *p,
    };
    if p.0 == CLIPBOARD || p.1 == p.2 || (p.2 > p.1) == portrait {
        return *p;
    }
    (p.0, p.2, p.1, p.3)
}

/// The unit a built-in preset shows and applies: a print preset in the physical unit picked for
/// Width/Height (#4668), else in inches; a screen preset in pixels.
fn preset_unit(p: &Preset, f: &Map<String, Value>) -> String {
    let unit = get_s(f, "__unit", "px");
    if !is_print(p) {
        "px".into()
    } else if unit != "px" && UNITS.iter().any(|u| u.0 == unit) {
        unit
    } else {
        "in".into()
    }
}

/// A built-in preset at Preferences › Units & Rulers › New Document Preset Resolutions, as in
/// Photoshop: a print preset keeps its size in inches at the Print Resolution (Letter at 200 ppi is
/// 1700 x 2200 px), a screen preset keeps its pixels at the Screen Resolution. The Clipboard preset
/// is the copied pixels and keeps its own resolution. A resolution that isn't a positive finite
/// number leaves the preset as listed.
pub fn at_pref_resolutions(p: &Preset, units: &UnitsAndRulers) -> Preset {
    let ppi = (if is_print(p) { units.print_resolution } else { units.screen_resolution }) as f32;
    if p.0 == CLIPBOARD || !(ppi.is_finite() && ppi > 0.0) {
        return *p;
    }
    if !is_print(p) {
        return (p.0, p.1, p.2, ppi);
    }
    let scale = ppi / p.3;
    let px = |v: u32| (v as f32 * scale).round().clamp(1.0, 300_000.0) as u32;
    (p.0, px(p.1), px(p.2), ppi)
}

/// Apply a built-in preset (at the preference resolutions, see [`at_pref_resolutions`]) to the
/// dialog fields.
pub fn apply_preset(f: &mut Map<String, Value>, p: &Preset, units: &UnitsAndRulers) {
    let (name, w, h, ppi) = at_pref_resolutions(p, units);
    let unit = preset_unit(p, f);
    f.remove("__savedPreset");
    f.insert("width".into(), json!(w));
    f.insert("height".into(), json!(h));
    f.insert("resolution".into(), json!(ppi));
    f.insert("__preset".into(), json!(name));
    f.insert("__unit".into(), json!(unit));
}

/// The colour mode of the dialog's `mode` value.
fn color_mode_of(mode: &str) -> Option<photocraft_color::ColorMode> {
    use photocraft_color::ColorMode;
    match mode {
        "rgb" => Some(ColorMode::Rgb),
        "gray" | "grayscale" => Some(ColorMode::Grayscale),
        "cmyk" => Some(ColorMode::Cmyk),
        "lab" => Some(ColorMode::Lab),
        _ => None,
    }
}

/// The Color Profile menu for a document of colour `space` (#321): the working space (shown with
/// its profile's description), then the built-in and installed profiles of that space, as
/// `file.new`'s `profile` values (an id or a path) with their labels.
fn profile_options(
    space: Option<photocraft_cms::ColorSpace>,
    working: &str,
    installed: &[photocraft_engine::installed_profiles::InstalledProfile],
) -> Vec<(String, String)> {
    let mut v = vec![("working".to_string(), crate::i18n::fmt(tl!("Working: {working}"), &[("working", working)]))];
    let Some(space) = space else { return v };
    v.extend(photocraft_cms::Builtin::ALL.iter().filter(|b| b.profile().color_space == space).map(|b| (b.id().to_string(), b.description().to_string())));
    v.extend(installed.iter().filter(|i| i.is_destination() && i.color_space == space).map(|i| (i.path.clone(), i.description.clone())));
    v
}

/// Fields `file.new` takes (drops the dialog's `__` UI keys).
pub fn command_params(f: &Map<String, Value>) -> Value {
    Value::Object(f.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// Set the resolution (pixels/inch) the way Photoshop's New Document does (#758): with Width/Height
/// in a physical unit the physical size is kept and the pixel count changes; in pixels the pixels
/// are kept.
pub fn set_resolution(f: &mut Map<String, Value>, new_ppi: f32) {
    let old_ppi = get_f(f, "resolution", 72.0);
    f.insert("resolution".into(), json!(new_ppi));
    if old_ppi != new_ppi {
        f.remove("__savedPreset");
    }
    if get_s(f, "__unit", "px") == "px" || !(old_ppi > 0.0 && new_ppi > 0.0) || old_ppi == new_ppi {
        return;
    }
    let scale = new_ppi / old_ppi;
    let (w, h) = (get_f(f, "width", 1920.0), get_f(f, "height", 1080.0));
    f.insert("width".into(), px_value(w * scale));
    f.insert("height".into(), px_value(h * scale));
    f.remove("__preset");
    f.remove("__savedPreset");
}

fn get_f(f: &Map<String, Value>, k: &str, d: f32) -> f32 {
    f.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}

fn get_s(f: &Map<String, Value>, k: &str, d: &str) -> String {
    f.get(k).and_then(Value::as_str).unwrap_or(d).to_string()
}

fn small_label(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(s).size(11.5).color(t.text_dim));
}

/// Lay out a preset card's title centred in `width`: wrapped onto at most two lines, the rest
/// elided, so long translations stay inside the card.
fn card_title(painter: &egui::Painter, title: &str, width: f32, color: egui::Color32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(title.to_owned(), egui::FontId::proportional(12.0), color, width);
    job.wrap.max_rows = 2;
    job.halign = egui::Align::Center;
    painter.layout_job(job)
}

/// Paint a page thumbnail with the preset's aspect ratio.
fn page_icon(ui: &egui::Ui, r: Rect, w: u32, h: u32, t: &Tokens) {
    let s = 30.0 / (w.max(h) as f32);
    let page = Rect::from_center_size(r.center(), vec2(w as f32 * s, h as f32 * s));
    ui.painter().rect_stroke(page, 1.0, Stroke::new(1.2, t.text_dim), StrokeKind::Inside);
}

/// The Background Contents menu's entry for a colour of the user's own ([`body`]).
const CUSTOM: &str = "custom";
/// Side of the Background Contents colour swatch.
const SWATCH: f32 = 22.0;

/// The colour the new document's background starts with: white, black, the captured or current
/// toolbox background colour, or a custom `"#rrggbb"`; `None` for Transparent.
fn background_rgb(app: &crate::PhotocraftApp, f: &Map<String, Value>) -> Option<[f32; 3]> {
    let bg = get_s(f, "background", "white");
    match bg.as_str() {
        "transparent" => None,
        "black" => Some([0.0; 3]),
        "backgroundColor" => Some(
            f.get("backgroundColor")
                .and_then(Value::as_array)
                .and_then(|a| Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32]))
                .unwrap_or_else(|| {
                    let b = app.session.tools.background;
                    [b[0], b[1], b[2]]
                }),
        ),
        hex => Some(crate::color_picker_ui::parse_hex(hex).unwrap_or([1.0; 3])),
    }
}

/// Restore a snapshot without replacing the new document's name or its clipboard offer.
pub fn apply_saved_preset(f: &mut Map<String, Value>, preset: &photocraft_engine::document_preset_cmds::DocumentPreset) {
    apply_settings(f, &preset.settings);
    f.insert("__savedPreset".into(), json!(preset.name));
}

/// Restore every setting of a saved or recent document (all but its name).
pub fn apply_settings(f: &mut Map<String, Value>, settings: &DocumentSettings) {
    f.remove("backgroundColor");
    // A preset without a Color Profile is on the working space, whatever was picked before.
    f.remove("profile");
    if let Some(params) = settings.command_params().as_object() {
        f.extend(params.clone());
    }
    f.insert("__unit".into(), json!(settings.unit));
    f.insert("__resUnit".into(), json!(settings.resolution_unit));
    f.remove("__savedPreset");
    f.remove("__preset");
    // Typed Width/Height belong to the previous values (`shown_size`).
    f.remove(&typed_key("width"));
    f.remove(&typed_key("height"));
}

/// The dialog's settings as `document.presets.save` / `document.recent.add` take them: the
/// `file.new` params plus the display units.
pub fn document_settings(f: &Map<String, Value>) -> Value {
    let mut settings = command_params(f);
    let unit = get_s(f, "__unit", "px");
    settings["unit"] = json!(if UNITS.iter().any(|u| u.0 == unit) { unit.as_str() } else { "px" });
    settings["resolutionUnit"] = json!(if get_s(f, "__resUnit", "in") == "cm" { "cm" } else { "in" });
    settings
}

/// Whether the dialog currently holds `settings` (so its Recent card shows selected).
fn holds(f: &Map<String, Value>, settings: &DocumentSettings) -> bool {
    // Unset fields hold the dialog's defaults, as the details pane shows them.
    let (w, h) = (get_f(f, "width", 1920.0), get_f(f, "height", 1080.0));
    (w, h, get_f(f, "resolution", 72.0)) == (settings.width as f32, settings.height as f32, settings.resolution)
        && f.get("depth").and_then(Value::as_u64).unwrap_or(8) == u64::from(settings.depth)
        && get_s(f, "mode", "rgb") == settings.mode
        && get_s(f, "background", "white") == settings.background
}

/// A card in the preset grid: a built-in preset or a recently created document.
enum Card {
    Builtin(Preset),
    Recent(DocumentSettings),
}

impl Card {
    /// Pixel size and resolution; a built-in preset at the Units & Rulers preset resolutions.
    fn size(&self, units: &UnitsAndRulers) -> (u32, u32, f32) {
        match self {
            Card::Builtin(listed) => {
                let p = at_pref_resolutions(listed, units);
                (p.1, p.2, p.3)
            }
            Card::Recent(s) => (s.width, s.height, s.resolution),
        }
    }

    /// The preset's name; a recent document takes the name of the built-in preset it matches.
    fn title(&self) -> &'static str {
        match self {
            Card::Builtin(p) => p.0,
            Card::Recent(s) => {
                CATEGORIES.iter().flat_map(|c| c.1.iter()).find(|p| (p.1, p.2, p.3) == (s.width, s.height, s.resolution)).map_or("Custom", |p| p.0)
            }
        }
    }

    /// The size line, in the unit the preset applies (a recent one: the unit it was made in).
    fn size_line(&self, units: &UnitsAndRulers, f: &Map<String, Value>) -> String {
        let (w, h, ppi) = self.size(units);
        let unit = match self {
            Card::Builtin(p) => preset_unit(p, f),
            Card::Recent(s) => s.unit.clone(),
        };
        if unit == "px" {
            format!("{w} x {h} px @ {} ppi", widgets::fmt_num(f64::from(ppi)))
        } else {
            let v = |n: u32| widgets::fmt_num(f64::from(to_unit(n as f32, &unit, ppi)));
            format!("{} x {} {unit} @ {} ppi", v(w), v(h), widgets::fmt_num(f64::from(ppi)))
        }
    }
}

/// The Recent category: the Clipboard preset, the documents recently created here (newest
/// first), then the built-in defaults they don't already cover.
fn recent_cards(f: &Map<String, Value>, recent: &[DocumentSettings], units: &UnitsAndRulers) -> Vec<Card> {
    let mut cards: Vec<Card> = clipboard_preset(f).map(Card::Builtin).into_iter().collect();
    cards.extend(recent.iter().cloned().map(Card::Recent));
    let defaults = CATEGORIES[0].1.iter().filter(|p| {
        let (_, w, h, ppi) = at_pref_resolutions(p, units);
        !recent.iter().any(|s| (s.width, s.height, s.resolution) == (w, h, ppi))
    });
    cards.extend(defaults.copied().map(Card::Builtin));
    cards
}

fn saved_presets(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> bool {
    let presets = app.session.presets.documents.clone();
    if presets.is_empty() {
        ui.label(tl!("No saved presets yet."));
        return false;
    }
    let mut create = false;
    egui::ScrollArea::vertical().id_salt("saved-document-presets").max_height(320.0).show(ui, |ui| {
        for preset in &presets {
            ui.push_id(&preset.name, |ui| {
                ui.horizontal(|ui| {
                    let selected = get_s(f, "__savedPreset", "") == preset.name;
                    let label = egui::Button::new(&preset.name).selected(selected).truncate();
                    let response = ui.add_sized(vec2(422.0, 32.0), label).on_hover_text(&preset.name);
                    if response.clicked() {
                        apply_saved_preset(f, preset);
                        // Touch-tolerant double-tap (finger jitter > egui's 6 px); see `widgets::second_click`.
                        create |= widgets::second_click(ui, &response);
                    }
                    if widgets::secondary_button(ui, tl!("Delete"), 76.0).clicked() {
                        match app.run("document.presets.delete", json!({"name":preset.name})) {
                            Ok(_) => {
                                f.remove("__presetError");
                                if selected {
                                    f.remove("__savedPreset");
                                }
                            }
                            Err(e) => {
                                f.insert("__presetError".into(), json!(e.to_string()));
                            }
                        }
                    }
                });
                let s = &preset.settings;
                small_label(ui, &format!("{} × {} px @ {} ppi · {}/{}", s.width, s.height, s.resolution, s.mode.to_uppercase(), s.depth));
                ui.add_space(8.0);
            });
        }
    });
    create
}

fn save_preset(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.add_space(12.0);
    if f.get("__savingPreset").and_then(Value::as_bool) != Some(true) {
        if widgets::secondary_button(ui, tl!("Save Preset…"), 240.0).clicked() {
            f.insert("__savingPreset".into(), json!(true));
            f.remove("__presetError");
        }
    } else {
        // Consume before TextEdit sees Enter, so naming can never confirm the outer dialog.
        let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let label = ui.label(tl!("Preset Name"));
        let mut name = get_s(f, "__presetName", "");
        ui.add(egui::TextEdit::singleline(&mut name).id_salt("document-preset-name").desired_width(240.0).char_limit(255)).labelled_by(label.id);
        f.insert("__presetName".into(), json!(name));
        ui.horizontal(|ui| {
            if widgets::secondary_button(ui, tl!("Save"), 110.0).clicked() || enter {
                match app.run("document.presets.save", json!({"name":name,"settings":document_settings(f)})) {
                    Ok(_) => {
                        if let Some(preset) = app.session.presets.documents.last() {
                            apply_saved_preset(f, preset);
                        }
                        f.insert("__category".into(), json!("Saved"));
                        f.remove("__savingPreset");
                        f.remove("__presetName");
                        f.remove("__presetError");
                        ui.ctx().request_repaint();
                    }
                    Err(e) => {
                        f.insert("__presetError".into(), json!(e.to_string()));
                    }
                }
            }
            if widgets::secondary_button(ui, tl!("Cancel"), 110.0).clicked() {
                f.remove("__savingPreset");
                f.remove("__presetError");
            }
        });
    }
    if let Some(error) = f.get("__presetError").and_then(Value::as_str) {
        // The engine's message stays copyable, unlike the static labels around it.
        ui.add(egui::Label::new(error).wrap().selectable(true));
    }
}

/// Returns whether a preset double-click requested creation, after applying its settings.
/// A recent category click can make egui count the preset double-click as a triple-click.
pub fn body(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) -> bool {
    let t = Tokens::get(ui.ctx());
    // Headings and field labels are static, as in Photoshop: no text cursor over them, no
    // drag-selecting them (#8331). Text fields keep theirs.
    ui.style_mut().interaction.selectable_labels = false;
    let cat = get_s(f, "__category", "Recent");
    let mut create = false;
    // Category tabs.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        for name in CATEGORIES.iter().map(|c| c.0).chain(["Saved"]) {
            let on = cat == name;
            let label = if name == "Saved" { tl!("Saved") } else { tl!(name) };
            let r = ui
                .add(egui::Label::new(RichText::new(label).size(13.0).color(if on { t.text } else { t.text_dim })).selectable(false).sense(Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if on {
                ui.painter().line_segment([r.rect.left_bottom() + vec2(0.0, 3.0), r.rect.right_bottom() + vec2(0.0, 3.0)], Stroke::new(2.0, t.text));
            }
            if r.clicked() {
                f.insert("__category".into(), json!(name));
            }
        }
    });
    ui.add_space(6.0);
    widgets::hairline(ui);
    ui.add_space(8.0);
    let units = app.session.prefs().units_and_rulers.clone();
    let presets: Vec<Card> = match CATEGORIES.iter().find(|c| c.0 == cat) {
        Some(c) if c.0 != CATEGORIES[0].0 => c.1.iter().copied().map(Card::Builtin).collect(),
        _ => recent_cards(f, &app.session.presets.recent_documents, &units),
    };
    let presets: Vec<Card> = presets
        .into_iter()
        .map(|c| match c {
            Card::Builtin(p) => Card::Builtin(oriented(&p, f)),
            recent => recent,
        })
        .collect();
    let chosen = get_s(f, "__preset", "");
    ui.horizontal_top(|ui| {
        // Left: preset grid.
        ui.vertical(|ui| {
            ui.set_width(520.0);
            if cat == "Saved" {
                create |= saved_presets(app, ui, f);
                return;
            }
            ui.label(RichText::new(crate::i18n::fmt(tl!("BLANK DOCUMENT PRESETS ({n})"), &[("n", &presets.len().to_string())])).size(11.0).color(t.text_faint));
            ui.add_space(6.0);
            let card = vec2(164.0, 112.0);
            // Up to 20 recent documents: scroll past three rows rather than grow the dialog.
            egui::ScrollArea::vertical().id_salt("new-document-presets").max_height(3.0 * (card.y + 8.0)).show(ui, |ui| {
                for row in presets.chunks(3) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        for c in row {
                            let (r, resp) = ui.allocate_exact_size(card, Sense::click());
                            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                            let (w, h, _) = c.size(&units);
                            let on = match c {
                                Card::Builtin(p) => chosen == p.0,
                                Card::Recent(settings) => holds(f, settings),
                            };
                            ui.painter().rect_filled(
                                r,
                                t.radius,
                                if on {
                                    t.row_selected
                                } else if resp.hovered() {
                                    t.hover
                                } else {
                                    t.field
                                },
                            );
                            if on {
                                ui.painter().rect_stroke(r, t.radius, Stroke::new(1.5, t.accent), StrokeKind::Inside);
                            }
                            page_icon(ui, Rect::from_center_size(pos2(r.center().x, r.top() + 34.0), vec2(40.0, 40.0)), w, h, &t);
                            let title = card_title(ui.painter(), tl!(c.title()), card.x - 12.0, t.text);
                            let elided = title.elided;
                            ui.painter().galley(pos2(r.center().x, r.top() + 70.0 - title.size().y / 2.0), title, t.text);
                            let size = c.size_line(&units, f);
                            ui.painter().text(pos2(r.center().x, r.top() + 97.0), Align2::CENTER_CENTER, &size, egui::FontId::proportional(10.5), t.text_faint);
                            let resp = match c {
                                // A recent document's tooltip names everything it restores.
                                Card::Recent(s) => {
                                    let background = match s.background.as_str() {
                                        "white" => tl!("White"),
                                        "black" => tl!("Black"),
                                        "transparent" => tl!("Transparent"),
                                        "backgroundColor" => tl!("Background Color"),
                                        hex => hex,
                                    };
                                    resp.on_hover_text(format!("{} · {size} · {}/{} · {background}", tl!(c.title()), s.mode.to_uppercase(), s.depth))
                                }
                                Card::Builtin(_) if elided => resp.on_hover_text(tl!(c.title())),
                                Card::Builtin(_) => resp,
                            };
                            if resp.clicked() {
                                match c {
                                    Card::Builtin(p) => apply_preset(f, p, &units),
                                    Card::Recent(settings) => apply_settings(f, settings),
                                }
                                // Touch-tolerant double-tap (finger jitter > egui's 6 px); see `widgets::second_click`.
                                create |= widgets::second_click(ui, &resp);
                            }
                        }
                    });
                    ui.add_space(8.0);
                }
            });
        });
        ui.add_space(10.0);
        // Right: Preset Details.
        ui.vertical(|ui| {
            ui.set_width(260.0);
            ui.label(RichText::new(tl!("PRESET DETAILS")).size(11.0).color(t.text_faint));
            ui.add_space(4.0);
            // Keep the canonical English default in state; only the TextEdit shows a translation.
            let raw = get_s(f, "name", "Untitled-1");
            let mut name = if raw == "Untitled-1" { tl!("Untitled-1").to_string() } else { raw };
            let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(250.0).font(egui::FontId::proportional(15.0))).accessible_name(tl!("Name"));
            if r.changed() {
                f.insert("name".into(), json!(name));
            }
            crate::field_tab::register(ui.ctx(), r.id);
            // Photoshop opens the dialog with the name selected, so typing replaces it and Tab goes
            // straight on to Width. (`__focused` marks that this instance has had its turn.)
            if !f.contains_key("__focused") {
                f.insert("__focused".into(), json!(true));
                crate::field_tab::focus(ui.ctx(), r.id);
            }
            if r.gained_focus() {
                crate::field_tab::select_all(ui.ctx(), r.id, &name);
            }
            ui.add_space(8.0);
            let ppi = get_f(f, "resolution", 72.0);
            let mut unit = get_s(f, "__unit", "px");
            // A typed unit (`210mm` while in pixels) converts at the new document's resolution.
            let units = widgets::FieldUnits { unit: widgets::unit_of_key(&unit), dpi: f64::from(ppi), ppi: 72.0 };
            small_label(ui, tl!("Width"));
            ui.horizontal(|ui| {
                let mut w = shown_size(f, "width", 1920.0, &unit, ppi);
                if widgets::value_field_units(ui, &mut w, 0.0..=300_000.0, 110.0, widgets::unit_decimals(&unit), units).accessible_name(tl!("Width")).changed()
                {
                    set_size(f, "width", w, &unit, ppi);
                }
                let opts: Vec<(String, &str)> = UNITS.iter().map(|u| (u.0.to_string(), u.1)).collect();
                if widgets::dropdown(ui, "nd-unit", tl!("Units"), &mut unit, &opts, 120.0) {
                    f.insert("__unit".into(), json!(unit));
                    f.remove("__savedPreset");
                }
            });
            small_label(ui, tl!("Height"));
            ui.horizontal(|ui| {
                let mut h = shown_size(f, "height", 1080.0, &unit, ppi);
                if widgets::value_field_units(ui, &mut h, 0.0..=300_000.0, 110.0, widgets::unit_decimals(&unit), units).accessible_name(tl!("Height")).changed()
                {
                    set_size(f, "height", h, &unit, ppi);
                }
                ui.add_space(6.0);
                // Page orientation (≠ Type › Orientation → Hướng chữ).
                small_label(ui, crate::i18n::tr_ctx(crate::i18n::current(), "document", "Orientation"));
                let (w, h) = (get_f(f, "width", 1920.0), get_f(f, "height", 1080.0));
                for (icon, portrait) in [("rectangle-vertical", true), ("rectangle-horizontal", false)] {
                    // Tooltip strings are looked up via `icons::button` → `tl!`; enum/state is aspect swap only.
                    if icons::button(ui, icon, 24.0, (h > w) == portrait, if portrait { "Portrait" } else { "Landscape" }).clicked() {
                        // The pick also turns the presets (`oriented`), even when the size already has it.
                        f.insert("__orientation".into(), json!(if portrait { "portrait" } else { "landscape" }));
                        if (h > w) != portrait {
                            swap_size(f);
                        }
                    }
                }
            });
            // Under Orientation, as in Photoshop: the document starts as one artboard (#9086).
            let mut artboard = f.get("artboard").and_then(Value::as_bool).unwrap_or(false);
            if widgets::checkbox(ui, &mut artboard, tl!("Artboards")).changed() {
                f.insert("artboard".into(), json!(artboard));
            }
            ui.add_space(4.0);
            small_label(ui, tl!("Resolution"));
            ui.horizontal(|ui| {
                let per_cm = get_s(f, "__resUnit", "in") == "cm";
                let mut r = if per_cm { ppi / 2.54 } else { ppi };
                if widgets::value_field(ui, &mut r, 1.0..=30_000.0, "", 110.0).accessible_name(tl!("Resolution")).changed() {
                    set_resolution(f, if per_cm { r * 2.54 } else { r });
                }
                let mut ru = get_s(f, "__resUnit", "in");
                if widgets::dropdown(
                    ui,
                    "nd-resunit",
                    tl!("Resolution"),
                    &mut ru,
                    &[("in".to_string(), tl!("Pixels/Inch")), ("cm".to_string(), tl!("Pixels/Centimeter"))],
                    120.0,
                ) {
                    f.insert("__resUnit".into(), json!(ru));
                    f.remove("__savedPreset");
                }
            });
            ui.add_space(4.0);
            small_label(ui, tl!("Color Mode"));
            ui.horizontal(|ui| {
                let mut mode = get_s(f, "mode", "rgb");
                if widgets::dropdown(
                    ui,
                    "nd-mode",
                    tl!("Color Mode"),
                    &mut mode,
                    &[
                        ("gray".to_string(), tl!("Grayscale")),
                        ("rgb".to_string(), tl!("RGB Color")),
                        ("cmyk".to_string(), tl!("CMYK Color")),
                        ("lab".to_string(), tl!("Lab Color")),
                    ],
                    110.0,
                ) {
                    f.insert("mode".into(), json!(mode));
                    // A profile of the old mode's colour space doesn't suit the new one.
                    f.remove("profile");
                    f.remove("__savedPreset");
                }
                let mut depth = f.get("depth").and_then(Value::as_u64).unwrap_or(8);
                let depth_options: Vec<(u64, &str, &str)> = DEPTH_OPTIONS.iter().map(|(bits, label, tooltip)| (*bits, *label, *tooltip)).collect();
                if widgets::dropdown_with_tooltips(ui, "nd-depth", tl!("Depth"), &mut depth, &depth_options, 120.0) {
                    f.insert("depth".into(), json!(depth));
                    f.remove("__savedPreset");
                }
            });
            ui.add_space(4.0);
            // Photoshop's Advanced Options › Color Profile (#321): the working space by default,
            // or another profile of the mode's colour space, built in or installed in the system.
            small_label(ui, tl!("Color Profile"));
            {
                let mode = get_s(f, "mode", "rgb");
                let space = color_mode_of(&mode).map(photocraft_engine::color_cmds::mode_space);
                let working = color_mode_of(&mode).map(|m| app.session.color.working(m).description.clone()).unwrap_or_default();
                let installed = photocraft_engine::installed_profiles::installed();
                let opts = profile_options(space, &working, &installed);
                let mut profile = get_s(f, "profile", "working");
                if !opts.iter().any(|(v, _)| *v == profile) {
                    // Not a profile of this mode (or no longer installed): Create must not send it.
                    profile = "working".into();
                    f.remove("profile");
                }
                let shown: Vec<(String, &str)> = opts.iter().map(|(v, l)| (v.clone(), l.as_str())).collect();
                if widgets::dropdown(ui, "nd-profile", tl!("Color Profile"), &mut profile, &shown, 240.0) {
                    if profile == "working" {
                        f.remove("profile");
                    } else {
                        f.insert("profile".into(), json!(profile));
                    }
                    f.remove("__savedPreset");
                }
            }
            ui.add_space(4.0);
            small_label(ui, tl!("Background Contents"));
            // A custom colour is kept as its "#rrggbb", which `file.new` takes as is (#4789).
            let current = get_s(f, "background", "white");
            let mut bg = if crate::color_picker_ui::parse_hex(&current).is_some() { CUSTOM.to_string() } else { current.clone() };
            let opts = [
                ("white".to_string(), tl!("White")),
                ("black".to_string(), tl!("Black")),
                ("backgroundColor".to_string(), tl!("Background Color")),
                ("transparent".to_string(), tl!("Transparent")),
                (CUSTOM.to_string(), tl!("Custom…")),
            ];
            let shown = background_rgb(app, f);
            let mut pick = false;
            ui.horizontal(|ui| {
                if widgets::dropdown(ui, "nd-bg", tl!("Background"), &mut bg, &opts, 240.0 - SWATCH - 6.0) {
                    if bg == CUSTOM {
                        // Custom… opens the Color Picker on the colour shown so far.
                        pick = true;
                    } else {
                        f.insert("background".into(), json!(bg));
                    }
                    f.remove("__savedPreset");
                    // A fresh choice of Background Color uses today's toolbox colour. Selecting a
                    // saved preset instead restores the colour captured when it was saved.
                    f.remove("backgroundColor");
                }
                // Photoshop's swatch beside the menu: the colour the canvas starts with; a click
                // opens the Color Picker, as Custom… does.
                let (r, resp) = ui.allocate_exact_size(vec2(SWATCH, SWATCH), Sense::click());
                match shown {
                    Some(c) => {
                        ui.painter().rect_filled(
                            r,
                            2.0,
                            egui::Color32::from_rgb((c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8),
                        );
                    }
                    None => widgets::checker(ui.painter(), r, 4.0),
                }
                ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, Tokens::get(ui.ctx()).field_border), StrokeKind::Inside);
                let resp = resp.on_hover_text(tl!("Custom background color")).on_hover_cursor(egui::CursorIcon::PointingHand);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Custom background color")));
                if resp.clicked() {
                    pick = true;
                    f.remove("__savedPreset");
                }
            });
            if pick {
                let start = shown.unwrap_or([1.0; 3]);
                f.insert("background".into(), json!(crate::color_picker_ui::hex(start)));
                f.remove("backgroundColor");
                crate::color_picker_ui::request_field(f, "background", &crate::color_picker_ui::title_for(tl!("Background Contents")));
            }
            save_preset(app, ui, f);
        });
    });
    create
}

#[cfg(test)]
mod tests;
