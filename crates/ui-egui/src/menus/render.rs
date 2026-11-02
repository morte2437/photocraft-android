use super::bar::released_on;
use super::items::{MenuItem, menu_tint};
use super::state::translated_menu_label;

pub(super) fn render_level(ui: &mut egui::Ui, items: &[&MenuItem], depth: usize, clicked: &mut Option<String>, nav: &mut crate::menu_nav::Nav) {
    // Menu popups can be taller than the window: each level stays on screen and scrolls (wheel,
    // scroll arrows, keyboard), like a native menu on a small display.
    crate::menu_nav::level(ui, depth, nav, |ui, nav| render_level_rows(ui, items, depth, clicked, nav));
}

/// Height of a menu separator: a line with room above and below, as in native menus.
const MENU_SEPARATOR: f32 = 9.0;

pub(super) fn render_level_rows(ui: &mut egui::Ui, items: &[&MenuItem], depth: usize, clicked: &mut Option<String>, nav: &mut crate::menu_nav::Nav) {
    let lang = crate::i18n::current();
    // Items never wrap: the menu widens to its longest label plus shortcut (translations can be
    // longer than the English).
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    // Rows touch, as in native menus: the dialog spacing between them made long menus a fifth
    // taller than they need to be (#402).
    ui.spacing_mut().item_spacing.y = 0.0;
    // Secondary menus use the exact same Spectrum hover contrast and spacing.
    crate::widgets::style_spectrum_popup_menu(ui);
    // Walk in Photoshop order: leaves and separators at this depth; a submenu appears at the position
    // of its first child.
    // Each row's mnemonic (#4988), unique within this level, in display order: the leaves' labels
    // and the submenus' names as shown.
    // A label with no Latin letter shows its English label's key after it: "新建(N)…" (#5863).
    let (translated, english): (Vec<String>, Vec<&str>) = {
        let mut subs: Vec<&str> = Vec::new();
        items
            .iter()
            .filter_map(|it| {
                if it.path.len() == depth {
                    (it.label != "---").then(|| (translated_menu_label(lang, it), it.label.as_str()))
                } else {
                    let name = it.path.get(depth).map(String::as_str)?;
                    (!subs.contains(&name)).then(|| {
                        subs.push(name);
                        (crate::i18n::tr(lang, name).to_string(), name)
                    })
                }
            })
            .unzip()
    };
    let (shown_labels, mnemonics) = crate::menu_keys::localized(&translated, &english, &[]);
    let mut next_row = 0usize;
    let mut shown_subs: Vec<&str> = Vec::new();
    let mut last_was_sep = true;
    for (i, it) in items.iter().enumerate() {
        if it.path.len() == depth {
            if it.label == "---" {
                if !last_was_sep && i + 1 < items.len() {
                    ui.add(egui::Separator::default().spacing(MENU_SEPARATOR));
                    last_was_sep = true;
                }
                continue;
            }
            let label = shown_labels.get(next_row).cloned().unwrap_or_else(|| translated_menu_label(lang, it));
            let mnemonic = mnemonics.get(next_row).copied().flatten();
            next_row += 1;
            // Checked, unchecked and plain rows share one checkmark column (#5705).
            let text = crate::menu_keys::menu_row_text(ui, it.checked == Some(true), &label, mnemonic.filter(|_| nav.underline));
            let mut b = egui::Button::new(text);
            if let Some(tint) = it.color.as_deref().and_then(menu_tint) {
                b = b.fill(tint.gamma_multiply(0.55));
            }
            if let Some(sc) = &it.shortcut {
                b = b.shortcut_text(crate::shortcuts::pretty(sc));
            }
            let hit = nav.row(ui, depth - 1, it.enabled, Some(&it.id), mnemonic, |ui, _| {
                let r = ui.add_enabled(it.enabled, b);
                let r = match it.id.as_str() {
                    "image.mode.bits8" | "image.mode.bits16" => r.on_hover_text(crate::i18n::tr(lang, "Integer")),
                    "image.mode.bits32" => r.on_hover_text(crate::i18n::tr(lang, "Floating point")),
                    _ => r,
                };
                let hit = r.clicked() || released_on(ui, &r);
                (r, hit)
            });
            if hit {
                *clicked = Some(it.id.clone());
                ui.close();
            }
            last_was_sep = false;
        } else if it.path.len() > depth {
            let name = it.path[depth].as_str();
            if shown_subs.contains(&name) {
                continue;
            }
            shown_subs.push(name);
            let child: Vec<&MenuItem> = items.iter().copied().filter(|c| c.path.len() > depth && c.path[depth] == name).collect();
            let enabled = submenu_enabled(&child);
            let mnemonic = mnemonics.get(next_row).copied().flatten();
            let shown = shown_labels.get(next_row).cloned().unwrap_or_else(|| crate::i18n::tr(lang, name).to_string());
            next_row += 1;
            let title = crate::menu_keys::menu_row_text(ui, false, &shown, mnemonic.filter(|_| nav.underline));
            ui.add_enabled_ui(enabled, |ui| {
                nav.row(ui, depth - 1, enabled, None, mnemonic, |ui, nav| {
                    let (response, inner) = submenu_row(ui, title, |ui| render_level(ui, &child, depth + 1, clicked, nav));
                    if inner.is_some() {
                        // Where the submenu hangs from, to keep it below the menu bar (#319).
                        nav.set_anchor(depth, response.rect);
                    }
                    (response, ())
                });
            });
            last_was_sep = false;
        }
    }
}

/// A submenu row, drawn as `ui.menu_button` draws it, except that a row scrolled out of view
/// doesn't open its submenu. egui opens a submenu whenever the pointer is over its row, clipped
/// or not: a menu reopened scrolled down (after Edit › Preferences or Window › Theme) has rows
/// hidden above its view, over the menu bar, and the hidden one under the title opened on the
/// press; the release on the title then counted as a click outside that submenu and closed the
/// whole menu (#5896, #6225). An open submenu (e.g. opened from the keyboard) is drawn as usual.
fn submenu_row<R>(ui: &mut egui::Ui, title: egui::WidgetText, content: impl FnOnce(&mut egui::Ui) -> R) -> (egui::Response, Option<R>) {
    use egui::containers::menu::{MenuState, SubMenu, SubMenuButton};
    let sub = SubMenu::id_from_widget_id(ui.next_auto_id());
    let open = MenuState::from_ui(ui, |state, _| state.open_item == Some(sub));
    let inactive = ui.style().visuals.widgets.inactive;
    if open {
        ui.style_mut().visuals.widgets.inactive = ui.style().visuals.widgets.open;
    }
    let response = ui.add(egui::Button::new(title).right_text(SubMenuButton::arrow_atom(None)));
    ui.style_mut().visuals.widgets.inactive = inactive;
    // The rect egui tests the pointer against (`SubMenu::show`), against the part that can be seen.
    let reach = response.rect.expand2(ui.spacing().item_spacing / 2.0);
    let hidden = ui.ctx().pointer_hover_pos().is_some_and(|p| reach.contains(p) && !ui.clip_rect().contains(p));
    if hidden && !open {
        return (response, None);
    }
    let inner = SubMenu::new().show(ui, &response, content);
    (response, inner.map(|r| r.inner))
}

/// A submenu header is enabled only while some item under it (at any depth) can run, so a
/// submenu of greyed items is greyed too instead of opening onto them (#1976).
pub(super) fn submenu_enabled(children: &[&MenuItem]) -> bool {
    children.iter().any(|c| c.enabled && c.label != "---")
}

/// The command a menu click runs: ⌥ + Merge Down / Merge Layers / Merge Visible keep the
/// originals, running Stamp Down / Stamp Visible instead (#217).
pub fn alt_click(id: String, alt: bool) -> String {
    match photocraft_engine::stamp_cmds::alt_variant(&id) {
        Some(stamp) if alt => stamp.to_string(),
        _ => id,
    }
}
