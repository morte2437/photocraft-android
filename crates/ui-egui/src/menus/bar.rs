use super::invoke::invoke;
use super::items::{ItemCache, MenuItem, menu_items};
use super::render::{alt_click, render_level, render_level_rows};
use super::search::help_search;
use super::{PhotocraftApp, TOP_MENUS, Value, json};

/// Draws the menu bar; returns the inner edge of the menu titles, facing the rest of the title bar
/// (the right edge of the last title, or its left edge in a right-to-left UI, where File is at the
/// right). The bar itself fills the row.
pub fn menu_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> f32 {
    // Built only while a menu is open: every item's enabled/checked state scales with the
    // document (layer lookups), which cost milliseconds per frame on large layouts (#125). While
    // one stays open, the items are reused until what they depend on changes.
    let kept = std::cell::Cell::new(app.menu_cache.take());
    let built: std::cell::OnceCell<ItemCache> = std::cell::OnceCell::new();
    let app_ref: &PhotocraftApp = app;
    let rtl = crate::direction::is_rtl();
    let mut edge = if rtl { ui.cursor().right() } else { ui.cursor().left() };
    let lang = crate::i18n::current();
    let mut clicked: Option<String> = None;
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut nav = crate::menu_nav::Nav::load(ui.ctx());
    // The keyboard on the bar (#4988): Alt / F10 focus, Alt+letter, mnemonics.
    let mut keys = crate::menu_keys::Keys::load(ui.ctx());
    keys.take_first_row(&mut nav);
    nav.underline = keys.show_underlines(ui.ctx());
    let translated: Vec<String> = TOP_MENUS.iter().map(|top| crate::i18n::tr(lang, top).to_string()).collect();
    let preferred: Vec<Option<char>> = TOP_MENUS.iter().map(|top| crate::menu_keys::TOP_PREFERRED.iter().find(|(t, _)| t == top).map(|(_, c)| *c)).collect();
    // A title with no Latin letter shows its English key: "文件(F)" (#5863).
    let (top_labels, top_mnemonics) = crate::menu_keys::localized(&translated, &TOP_MENUS, &preferred);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let v = &mut ui.style_mut().visuals;
        v.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
        v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, t.text_dim);
        v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
        egui::MenuBar::new().ui(ui, |ui| {
            // egui lays the bar out left to right; a right-to-left UI starts it at the right.
            let mut mirrored = rtl.then(|| ui.new_child(egui::UiBuilder::new().layout(crate::direction::row())));
            let ui = mirrored.as_mut().unwrap_or(ui);
            ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
            nav.bar_bottom = Some(ui.max_rect().bottom());
            let mut buttons = Vec::with_capacity(TOP_MENUS.len());
            for (k, top) in TOP_MENUS.iter().enumerate() {
                // egui's menu_button toggles on release; these titles open on the press (one
                // gesture can press, drag to an item and release), so each drives its popup.
                // The title the keyboard is on (Alt / F10) is lit like a hovered one.
                let focused = keys.focus == Some(if rtl { TOP_MENUS.len() - 1 - k } else { k });
                let label = if nav.underline {
                    crate::menu_keys::mnemonic_text(ui, "", &top_labels[k], top_mnemonics[k], Some(t.text_dim))
                } else {
                    egui::RichText::new(top_labels[k].as_str()).color(t.text_dim).into()
                };
                let mut button = egui::Button::new(label);
                if focused {
                    button = button.fill(ui.visuals().widgets.hovered.weak_bg_fill);
                }
                let title = ui.add(button);
                // The bar's close behaviour and style, as a menu (not bar) config so submenus inside
                // render as submenus.
                let bar = egui::containers::menu::MenuConfig::find(ui);
                let config = egui::containers::menu::MenuConfig::new().close_behavior(bar.close_behavior).style(bar.style.clone());
                let open = title_press(ui.ctx(), &title);
                // The release ending the press that opened this menu is a click "outside" the
                // popup: it must not close it again.
                let opening = title.clicked() && ui.ctx().data(|d| d.get_temp::<bool>(press_gesture_id())).unwrap_or(false);
                let close = if opening {
                    egui::PopupCloseBehavior::IgnoreClicks
                } else if *top == "Help" {
                    // Menus close on any click, inside them too; a click in Help's search field
                    // must keep it open. Its items close it themselves when chosen.
                    egui::PopupCloseBehavior::CloseOnClickOutside
                } else {
                    config.close_behavior
                };
                egui::Popup::menu(&title)
                    .align(crate::direction::below())
                    .open_memory(open)
                    .close_behavior(close)
                    .style(config.style.clone())
                    .info(egui::UiStackInfo::new(egui::UiKind::Menu).with_tag_value(egui::containers::menu::MenuConfig::MENU_CONFIG_TAG, config))
                    .show(|ui| {
                        let items = &built
                            .get_or_init(|| {
                                let state = crate::native_menu::state_hash(app_ref, None);
                                let input = crate::native_menu::had_input(ui.ctx());
                                let items = kept.take().and_then(|c| c.reuse(state, input)).unwrap_or_else(|| menu_items(app_ref));
                                ItemCache { state, items }
                            })
                            .items;
                        let mine: Vec<&MenuItem> = items.iter().filter(|i| i.path.first().map(String::as_str) == Some(*top)).collect();
                        ui.set_min_width(220.0);
                        if mine.is_empty() {
                            ui.weak(crate::i18n::tr(lang, "(coming soon)"));
                        }
                        if *top == "Help" {
                            // The search field scrolls with the rows, as part of the menu's content.
                            crate::menu_nav::level(ui, 1, &mut nav, |ui, nav| {
                                help_search(ui, items, opening, &mut clicked, nav);
                                render_level_rows(ui, &mine, 1, &mut clicked, nav);
                            });
                        } else {
                            render_level(ui, &mine, 1, &mut clicked, &mut nav);
                        }
                    });
                buttons.push(title);
            }
            edge = if rtl { buttons.iter().map(|b| b.rect.left()).fold(edge, f32::min) } else { buttons.iter().map(|b| b.rect.right()).fold(edge, f32::max) };
            switch_on_hover(ui.ctx(), &buttons);
            // ← and → step to the menu on that side, so in screen order.
            let mut tops: Vec<egui::Id> = buttons.iter().map(egui::Popup::default_response_id).collect();
            if rtl {
                tops.reverse();
            }
            nav.keys(ui.ctx(), &tops, &mut clicked);
            let mut mnemonics = top_mnemonics.clone();
            if rtl {
                mnemonics.reverse();
            }
            keys.keys(ui.ctx(), &tops, &mnemonics, &mut nav, &mut clicked);
        });
    });
    keys.store(ui.ctx());
    nav.store(ui.ctx());
    // No menu open this frame: nothing is kept.
    app.menu_cache = built.into_inner();
    // The press-drag gesture ends with the button (its release was handled by the rows above).
    if ui.input(|i| i.pointer.primary_released() || !i.pointer.primary_down()) {
        ui.ctx().data_mut(|d| d.remove::<bool>(press_gesture_id()));
    }
    if let Some(id) = clicked {
        let ctx = ui.ctx().clone();
        let alt = ctx.input(|i| i.modifiers.alt);
        // ⌥ while choosing an Image › Adjustments item opens it at its last settings (#8900).
        let r = if alt && crate::adjust_dialog::has_dialog(&id) {
            let opened = crate::adjust_dialog::open_last(app, &id).map(|_| Value::Null).ok_or_else(|| "no document".to_string());
            // Edit › Search's recents, as `invoke` notes a plain click.
            if opened.is_ok() {
                crate::palette::note_recent(&ctx, &id);
            }
            opened
        } else {
            invoke(app, &ctx, &alt_click(id, alt), json!({}))
        };
        if let Err(e) = r {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    edge
}

fn press_gesture_id() -> egui::Id {
    egui::Id::unique("menu-press-gesture")
}

/// A menu title's open/close command this frame, the press opens a closed menu (and
/// starts a press-drag gesture: releasing on an item runs it) or closes an open one; the release
/// never toggles, so the menu doesn't blink.
fn title_press(ctx: &egui::Context, title: &egui::Response) -> Option<egui::SetOpenCommand> {
    // A press this frame on the title (still down, or a whole click within one frame).
    let pressed = ctx.input(|i| i.pointer.primary_pressed()) && (title.is_pointer_button_down_on() || title.clicked());
    if !pressed {
        return None;
    }
    let open = egui::Popup::is_id_open(ctx, egui::Popup::default_response_id(title));
    if !open {
        ctx.data_mut(|d| d.insert_temp(press_gesture_id(), true));
    }
    Some(egui::SetOpenCommand::Bool(!open))
}

/// Did the press-drag gesture that opened the menus end over `item` (released on it)?
pub(super) fn released_on(ui: &egui::Ui, item: &egui::Response) -> bool {
    item.enabled()
        && item.contains_pointer()
        && ui.input(|i| i.pointer.primary_released())
        && ui.ctx().data(|d| d.get_temp::<bool>(press_gesture_id())).unwrap_or(false)
}

/// True only when the pointer can actually reach a menu title. A tall submenu can be
/// repositioned upward by egui and overlap the menu bar; in that case the popup's layer is
/// top-most and hovering it must not switch the open top-level menu.
///
/// The title's column extends up to the top of the window: egui lights a widget up to
/// `interact_radius` away, so with the window maximized a pointer pushed against the screen edge
/// (above the title's own rect) highlights it and must switch to it too (#7702).
pub(super) fn pointer_reaches_title(ctx: &egui::Context, response: &egui::Response, p: egui::Pos2) -> bool {
    let r = response.interact_rect;
    let column = egui::Rect::from_min_max(egui::pos2(r.left(), f32::NEG_INFINITY), r.max);
    column.contains(p) && ctx.layer_id_at(p) == Some(response.layer_id)
}

/// Like a native menu bar (Windows, macOS): while one top-level menu is open, hovering another
/// top-level title opens that menu instead.
fn switch_on_hover(ctx: &egui::Context, buttons: &[egui::Response]) {
    let ids: Vec<egui::Id> = buttons.iter().map(egui::Popup::default_response_id).collect();
    let Some(open) = ids.iter().position(|id| egui::Popup::is_id_open(ctx, *id)) else { return };
    // `Response::hovered` is false while the menu's popup layer is open. Hit-test the title
    // rect manually, but only when its own layer is top-most at the pointer.
    let Some(p) = ctx.pointer_hover_pos() else { return };
    // Only a moving pointer switches: a pointer resting on a title must not undo ← / → .
    if ctx.input(|i| i.pointer.delta() == egui::Vec2::ZERO) {
        return;
    }
    if let Some(i) = buttons.iter().position(|b| pointer_reaches_title(ctx, b, p))
        && i != open
    {
        egui::Popup::open_id(ctx, ids[i]);
        ctx.request_repaint();
    }
}
