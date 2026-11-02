//! The font menu's filters, as in Photoshop's font menu (except its Adobe Fonts parts): a class
//! list (Serif, Sans Serif, Script…), Show Variable Fonts, Show Favorite Fonts and Show Similar
//! Fonts, and Clear All. Each family's class, format and shape come from its own tables
//! ([`photocraft_text::font_class`]), read once per font list and cached here.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError};

use photocraft_text::{FamilyTraits, FontClass, FontFormat};

/// How many fonts Show Similar Fonts lists (the current one first).
pub(crate) const SIMILAR_COUNT: usize = 30;

/// The filter bar's state, per font menu (kept for the session).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FontFilters {
    /// Only fonts of this class (`None`: All Classes).
    pub class: Option<FontClass>,
    pub favorites: bool,
    pub variable: bool,
    /// Fonts like the current one, most alike first.
    pub similar: bool,
    /// The class list is collapsed (the « button).
    pub hide_classes: bool,
}

impl FontFilters {
    /// Whether any filter narrows the list (Clear All is enabled).
    pub fn any(&self) -> bool {
        self.class.is_some() || self.favorites || self.variable || self.similar
    }

    /// Clear All: every filter off; the class list stays as it was.
    pub fn clear(&mut self) {
        *self = FontFilters { hide_classes: self.hide_classes, ..Default::default() };
    }
}

/// Family traits by family name; `None` for a family that can't be read (or a served font not
/// fetched yet).
pub(crate) type TraitsMap = HashMap<String, Option<FamilyTraits>>;

/// The traits of `family` from the shared font database, cached until fonts are registered.
pub(crate) fn traits(family: &str) -> Option<FamilyTraits> {
    type Cached = Option<(u64, TraitsMap)>;
    static CACHE: std::sync::Mutex<Cached> = std::sync::Mutex::new(None);
    let generation = photocraft_text::fonts::shared_generation();
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if cache.as_ref().is_none_or(|(g, _)| *g != generation) {
        *cache = Some((generation, HashMap::new()));
    }
    let (_, map) = cache.as_mut()?;
    if let Some(t) = map.get(family) {
        return t.clone();
    }
    // Only once the background font scan is done: never block the UI on it.
    let t = photocraft_text::try_shared()?.lock().unwrap_or_else(PoisonError::into_inner).fonts.family_traits(family);
    map.insert(family.to_string(), t.clone());
    t
}

/// Narrow and order the menu's `rows` by `filters`. `current` is the font Show Similar Fonts
/// compares with; `lookup` gives a family's traits. Returns false when Show Similar Fonts is on
/// but the current font has nothing to compare (its rows are then left as they were).
pub(crate) fn apply<'a>(
    rows: &mut Vec<(&'a String, &'a str)>,
    filters: &FontFilters,
    favorites: &[String],
    current: &str,
    lookup: &dyn Fn(&str) -> Option<FamilyTraits>,
) -> bool {
    if filters.favorites {
        rows.retain(|(f, _)| favorites.contains(f));
    }
    if filters.variable {
        rows.retain(|(f, _)| lookup(f).is_some_and(|t| t.variable()));
    }
    if let Some(class) = filters.class {
        rows.retain(|(f, _)| lookup(f).is_some_and(|t| t.class == Some(class)));
    }
    if !filters.similar {
        return true;
    }
    let Some(reference) = lookup(current).filter(|t| t.class.is_some() || t.panose.iter().any(|d| *d > 1)) else {
        return false;
    };
    let mut scored: Vec<(f32, (&String, &str))> = rows
        .drain(..)
        .filter_map(|row| lookup(row.0).map(|t| (if row.0 == current { -1.0 } else { photocraft_text::font_class::difference(&reference, &t) }, row)))
        .collect();
    scored.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.extend(scored.into_iter().take(SIMILAR_COUNT).map(|(_, row)| row));
    true
}

/// The translated class name.
pub(crate) fn class_label(class: FontClass) -> &'static str {
    match class {
        FontClass::Serif => tl!("Serif"),
        FontClass::SlabSerif => tl!("Slab Serif"),
        FontClass::SansSerif => tl!("Sans Serif"),
        FontClass::Script => tl!("Script"),
        FontClass::Monospace => tl!("Monospace"),
        FontClass::Handwritten => tl!("Handwritten"),
        FontClass::Blackletter => tl!("Blackletter"),
        FontClass::Decorative => tl!("Decorative"),
    }
}

/// The format badge's tooltip.
pub(crate) fn format_tip(format: FontFormat) -> &'static str {
    match format {
        FontFormat::OpenType => tl!("OpenType font"),
        FontFormat::TrueType => tl!("TrueType font"),
        FontFormat::Variable => tl!("Variable font"),
    }
}

/// The class list's width; the filter buttons sit above it.
pub(crate) const CLASS_COLUMN_WIDTH: f32 = 118.0;

/// The filter buttons: the class-list toggle and the three filters. Returns true when a filter
/// changed.
pub(crate) fn filter_buttons(ui: &mut egui::Ui, filters: &mut FontFilters) -> bool {
    let before = filters.clone();
    let (icon, tip) = if filters.hide_classes { ("chevrons-right", tl!("Show Font Classes")) } else { ("chevrons-left", tl!("Hide Font Classes")) };
    if crate::icons::button(ui, icon, 22.0, false, tip).clicked() {
        filters.hide_classes = !filters.hide_classes;
    }
    if crate::icons::button(ui, "font-variable", 22.0, filters.variable, tl!("Show Variable Fonts")).clicked() {
        filters.variable = !filters.variable;
    }
    let tip = if filters.favorites { tl!("Show All Fonts") } else { tl!("Show Favorite Fonts") };
    if crate::icons::button(ui, if filters.favorites { "star-filled" } else { "star" }, 22.0, filters.favorites, tip).clicked() {
        filters.favorites = !filters.favorites;
    }
    if crate::icons::button(ui, "font-similar", 22.0, filters.similar, tl!("Show Similar Fonts")).clicked() {
        filters.similar = !filters.similar;
    }
    *filters != before
}

/// Clear All: turns every filter off. Returns true when it did.
pub(crate) fn clear_all(ui: &mut egui::Ui, filters: &mut FontFilters) -> bool {
    let clicked = ui.add_enabled(filters.any(), egui::Button::new(tl!("Clear All")).small()).clicked();
    if clicked {
        filters.clear();
    }
    clicked
}

/// The preview banner's height.
pub(crate) const PREVIEW_HEIGHT: f32 = 64.0;

/// The preview at the top of the font menu: `family` (the font under the pointer, else the
/// current one) named and set large, `width` wide. It sets `text`, the user's own type, when
/// there is some, else a sample.
pub(crate) fn preview_banner(ui: &mut egui::Ui, family: &str, label: &str, text: Option<&str>, width: f32) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, PREVIEW_HEIGHT), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{} {label}", tl!("Font preview:"))));
    if !ui.is_rect_visible(rect) {
        return;
    }
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.painter().rect_filled(rect, t.radius_sm, t.canvas);
    ui.painter().text(rect.left_top() + egui::vec2(10.0, 8.0), egui::Align2::LEFT_TOP, label, egui::FontId::proportional(11.0), t.text_dim);
    if let Some(text) = text {
        let line = egui::Rect::from_min_max(rect.left_top() + egui::vec2(10.0, 20.0), rect.right_bottom() - egui::vec2(10.0, 2.0));
        crate::font_preview::paint_text(ui, family, text, line, 30.0);
        return;
    }
    let Some((size, _)) = crate::font_preview::sample(photocraft_engine::prefs::FontPreview::Huge) else { return };
    crate::font_preview::paint(ui, family, rect.translate(egui::vec2(0.0, 6.0)), size);
}

/// The class list beside the fonts. Returns true when the class changed.
pub(crate) fn class_list(ui: &mut egui::Ui, filters: &mut FontFilters) -> bool {
    let before = filters.class;
    ui.vertical(|ui| {
        ui.set_width(CLASS_COLUMN_WIDTH);
        if ui.add(egui::Button::selectable(filters.class.is_none(), tl!("All Classes")).truncate()).clicked() {
            filters.class = None;
        }
        for class in FontClass::ALL {
            if ui.add(egui::Button::selectable(filters.class == Some(class), class_label(class)).truncate()).clicked() {
                filters.class = Some(class);
            }
        }
    });
    filters.class != before
}

/// Traits a test hands the menu instead of reading installed fonts.
pub(crate) fn lookup_in(map: Option<Arc<TraitsMap>>) -> impl Fn(&str) -> Option<FamilyTraits> {
    move |family| match &map {
        Some(map) => map.get(family).cloned().flatten(),
        None => traits(family),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(class: FontClass, format: FontFormat, panose: [u8; 10], weight: u16) -> Option<FamilyTraits> {
        Some(FamilyTraits { class: Some(class), format, panose, weight })
    }

    fn fixture() -> (Vec<String>, TraitsMap) {
        let sans = [2, 11, 6, 4, 2, 2, 2, 2, 2, 4];
        let serif = [2, 2, 6, 3, 5, 4, 5, 2, 3, 4];
        let map: TraitsMap = [
            ("Arial", t(FontClass::SansSerif, FontFormat::TrueType, sans, 400)),
            ("Arial Black", t(FontClass::SansSerif, FontFormat::TrueType, [2, 11, 10, 4, 2, 1, 2, 2, 2, 4], 900)),
            ("Bahnschrift", t(FontClass::SansSerif, FontFormat::Variable, sans, 400)),
            ("Consolas", t(FontClass::Monospace, FontFormat::TrueType, [2, 11, 6, 9, 2, 2, 4, 3, 2, 4], 400)),
            ("Georgia", t(FontClass::Serif, FontFormat::TrueType, serif, 400)),
            ("Minion", t(FontClass::Serif, FontFormat::OpenType, serif, 400)),
            ("Mystery", None),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let mut families: Vec<String> = map.keys().cloned().collect();
        families.sort();
        (families, map)
    }

    fn run(filters: &FontFilters, favorites: &[&str], current: &str) -> (Vec<String>, bool) {
        let (families, map) = fixture();
        let mut rows: Vec<(&String, &str)> = families.iter().map(|f| (f, f.as_str())).collect();
        let favorites: Vec<String> = favorites.iter().map(|s| s.to_string()).collect();
        let lookup = lookup_in(Some(Arc::new(map)));
        let ok = apply(&mut rows, filters, &favorites, current, &lookup);
        (rows.into_iter().map(|(f, _)| f.clone()).collect(), ok)
    }

    #[test]
    fn class_variable_and_favorite_filters_narrow_and_combine() {
        let class = |c| FontFilters { class: Some(c), ..Default::default() };
        assert_eq!(run(&class(FontClass::Serif), &[], "Arial").0, ["Georgia", "Minion"]);
        assert_eq!(run(&class(FontClass::Monospace), &[], "Arial").0, ["Consolas"]);
        assert!(run(&class(FontClass::Blackletter), &[], "Arial").0.is_empty());
        let variable = FontFilters { variable: true, ..Default::default() };
        assert_eq!(run(&variable, &[], "Arial").0, ["Bahnschrift"]);
        let both = FontFilters { favorites: true, class: Some(FontClass::SansSerif), ..Default::default() };
        assert_eq!(run(&both, &["Georgia", "Arial Black"], "Arial").0, ["Arial Black"]);
        // Nothing on: every font, unknown ones too.
        assert_eq!(run(&FontFilters::default(), &[], "Arial").0.len(), 7);
    }

    #[test]
    fn similar_fonts_start_with_the_current_one_most_alike_first() {
        let similar = FontFilters { similar: true, ..Default::default() };
        let (rows, ok) = run(&similar, &[], "Arial");
        assert!(ok);
        assert_eq!(rows[0], "Arial", "the current font first");
        assert_eq!(rows[1], "Bahnschrift", "same class and shape");
        let georgia = rows.iter().position(|f| f == "Georgia").unwrap();
        let black = rows.iter().position(|f| f == "Arial Black").unwrap();
        assert!(black < georgia, "another weight of the class is nearer than another class");
        assert!(!rows.contains(&"Mystery".to_string()), "unreadable fonts can't be compared");
        // A current font with nothing to compare leaves the list alone and says so.
        let (rows, ok) = run(&similar, &[], "Mystery");
        assert!(!ok);
        assert_eq!(rows.len(), 7);
    }

    #[test]
    fn clear_all_keeps_the_class_list_state() {
        let mut f = FontFilters { class: Some(FontClass::Script), favorites: true, variable: true, similar: true, hide_classes: true };
        assert!(f.any());
        f.clear();
        assert_eq!(f, FontFilters { hide_classes: true, ..Default::default() });
        assert!(!f.any());
    }
}
