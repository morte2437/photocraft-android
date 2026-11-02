use super::bar::released_on;
use super::items::MenuItem;
use super::state::translated_menu_label;

/// Most results the Help menu search lists.
pub(super) const HELP_SEARCH_MAX: usize = 15;

/// How well `query` (lowercase) matches a menu item: its label starting with it, then a word of
/// the label starting with it, then anywhere in the label, then anywhere in its menu path.
/// `None` when it doesn't match at all.
pub fn search_rank(query: &str, label: &str, path: &[String]) -> Option<u8> {
    let l = label.to_lowercase();
    if l.starts_with(query) {
        Some(0)
    } else if l.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(query)) {
        Some(1)
    } else if l.contains(query) {
        Some(2)
    } else if path.iter().any(|p| p.to_lowercase().contains(query)) {
        Some(3)
    } else {
        None
    }
}

/// Menu items matching `query`, best first (Photoshop order within a rank). Both the English
/// labels and their `lang` translations match, so a search works in the UI language and in
/// English; untranslated strings are simply their English text.
pub fn search_items<'a>(items: &'a [MenuItem], query: &str, lang: crate::i18n::Lang) -> Vec<&'a MenuItem> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let rank = |it: &MenuItem| {
        let english = search_rank(&q, &it.label, &it.path);
        let path: Vec<String> = it.path.iter().map(|p| crate::i18n::tr(lang, p).to_string()).collect();
        let local = search_rank(&q, &translated_menu_label(lang, it), &path);
        english.into_iter().chain(local).min()
    };
    let mut hits: Vec<(u8, usize, &MenuItem)> =
        items.iter().enumerate().filter(|(_, it)| it.label != "---").filter_map(|(i, it)| rank(it).map(|r| (r, i, it))).collect();
    hits.sort_by_key(|(r, i, _)| (*r, *i));
    let mut seen = std::collections::HashSet::new();
    hits.into_iter().filter(|(_, _, it)| seen.insert(it.id.as_str())).take(HELP_SEARCH_MAX).map(|(_, _, it)| it).collect()
}

/// Help › Search was drawn in this frame (or the last): the open menu has a text field of its own.
pub(super) fn shown(ctx: &egui::Context) -> bool {
    let pass_id = egui::Id::unique("help-menu-search").with("pass");
    ctx.data(|d| d.get_temp::<u64>(pass_id)).is_some_and(|p| p + 1 >= ctx.cumulative_pass_nr())
}

/// Help › Search : a field at the top of the Help menu that finds any menu command
/// by name. Results show their menu path; click, ↓ then ↩, or ↩ in the field runs one.
/// `opening`: this frame's release ends the click on the Help title that opened the menu.
pub(super) fn help_search(ui: &mut egui::Ui, items: &[MenuItem], opening: bool, clicked: &mut Option<String>, nav: &mut crate::menu_nav::Nav) {
    let ctx = ui.ctx().clone();
    let lang = crate::i18n::current();
    let text_id = egui::Id::unique("help-menu-search");
    let pass_id = text_id.with("pass");
    // A fresh opening of the Help menu starts empty, with the field focused.
    let pass = ctx.cumulative_pass_nr();
    let reopened = ctx.data(|d| d.get_temp::<u64>(pass_id)).is_none_or(|p| p + 1 < pass);
    ctx.data_mut(|d| d.insert_temp(pass_id, pass));
    let mut query: String = if reopened { String::new() } else { ctx.data(|d| d.get_temp(text_id)).unwrap_or_default() };
    let field = ui.add(egui::TextEdit::singleline(&mut query).id(text_id.with("field")).hint_text(crate::i18n::tr(lang, "Search menus")).desired_width(220.0));
    // The menu opens on the press; the release a frame later completes a click on the title,
    // outside the field, and egui takes the focus away from it again (#1447, #1438).
    if (reopened || opening) && !crate::zoom_field::editing(&ctx) {
        field.request_focus();
    }
    let results = search_items(items, &query, lang);
    if field.lost_focus()
        && ui.input(|i| i.key_pressed(egui::Key::Enter))
        && let Some(it) = results.iter().find(|it| it.enabled)
    {
        *clicked = Some(it.id.clone());
        ui.close();
    }
    ctx.data_mut(|d| d.insert_temp(text_id, query.clone()));
    if !query.trim().is_empty() {
        if results.is_empty() {
            ui.weak(crate::i18n::tr(lang, "No matching commands"));
        }
        for it in results {
            let mut trail: Vec<String> = it.path.iter().map(|p| crate::i18n::tr(lang, p).to_string()).collect();
            trail.push(translated_menu_label(lang, it));
            let mut b = egui::Button::new(trail.join(" › "));
            if let Some(sc) = &it.shortcut {
                b = b.shortcut_text(crate::shortcuts::pretty(sc));
            }
            let hit = nav.row(ui, 0, it.enabled, Some(&it.id), None, |ui, _| {
                let r = ui.add_enabled(it.enabled, b);
                let hit = r.clicked() || released_on(ui, &r);
                (r, hit)
            });
            if hit {
                *clicked = Some(it.id.clone());
                ui.close();
            }
        }
    }
    ui.separator();
}
