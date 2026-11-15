use super::panel::chevron_icon;
use super::scrub::take_label_scrub_name;
use super::{Align2, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, Vec2, pos2, type_ahead, vec2};

/// A compact labelled dropdown in the studio style.
pub fn dropdown<T: PartialEq + Clone>(ui: &mut Ui, id: &str, name: &str, current: &mut T, options: &[(T, &str)], width: f32) -> bool {
    dropdown_hovered(ui, id, name, current, options, width).0
}

/// [`dropdown`] whose option labels are shown exactly as given: names the user typed (a channel
/// or layer name) must not be translated, unlike the built-in labels [`dropdown`] translates (#5335).
pub fn dropdown_literal<T: PartialEq + Clone>(ui: &mut Ui, id: &str, name: &str, current: &mut T, options: &[(T, &str)], width: f32) -> bool {
    let (chosen, _) = dropdown_with(ui, id, name, current, options, width, false, false, &|_| false, &|_| true);
    !chosen.is_empty()
}

/// [`dropdown`] whose open list greys the options `enabled` refuses: drawn disabled and never
/// picked, by the pointer or the keys (#12906, the Pen's Pixels mode).
pub fn dropdown_enabled<T: PartialEq + Clone>(
    ui: &mut Ui,
    id: &str,
    name: &str,
    current: &mut T,
    options: &[(T, &str)],
    width: f32,
    enabled: impl Fn(&T) -> bool,
) -> bool {
    let before = current.clone();
    let (chosen, _) = dropdown_with(ui, id, name, current, options, width, false, true, &|_| false, &enabled);
    // The arrow keys and type-ahead step onto greyed rows too; landing there keeps the value.
    if !enabled(current) {
        *current = before;
        ui.ctx().request_repaint();
        return false;
    }
    !chosen.is_empty()
}

/// [`dropdown`], also returning the option under the pointer in its open list (live previews).
pub fn dropdown_hovered<T: PartialEq + Clone>(ui: &mut Ui, id: &str, name: &str, current: &mut T, options: &[(T, &str)], width: f32) -> (bool, Option<T>) {
    let (chosen, hovered) = dropdown_with(ui, id, name, current, options, width, false, true, &|_| false, &|_| true);
    (!chosen.is_empty(), hovered)
}

/// [`dropdown_hovered`] whose value also steps with the wheel over its button or open list: down
/// picks the next option, up the previous one, one per notch (Shift too), stopping at the ends
/// (Photoshop's Layers panel blend modes, #1747), for a wheel gesture that starts over either
/// (#11353). The wheel then scrolls neither the list, which follows the value instead, nor what
/// the dropdown is in. Returns every option chosen this frame
/// in order (one per notch; the last is `current`) and the option under the pointer in the open
/// list, none while the pointer rests where the wheel last stepped.
pub fn dropdown_wheel_hovered<T: PartialEq + Clone>(
    ui: &mut Ui,
    id: &str,
    name: &str,
    current: &mut T,
    options: &[(T, &str)],
    width: f32,
) -> (Vec<T>, Option<T>) {
    dropdown_with(ui, id, name, current, options, width, true, true, &|_| false, &|_| true)
}

/// [`dropdown_wheel_hovered`] whose open list draws a line above each option (but the first) that
/// `starts_group`: Photoshop's blend mode lists group their modes (#5532). The lines are only
/// drawn: the options, their order and how the wheel, the keys and the pointer pick them stay.
pub fn dropdown_wheel_grouped<T: PartialEq + Clone>(
    ui: &mut Ui,
    id: &str,
    name: &str,
    current: &mut T,
    options: &[(T, &str)],
    width: f32,
    starts_group: impl Fn(&T) -> bool,
) -> (Vec<T>, Option<T>) {
    dropdown_with(ui, id, name, current, options, width, true, true, &starts_group, &|_| true)
}

/// [`dropdown_wheel_grouped`] whose open list greys the options `enabled` refuses: drawn disabled,
/// not picked by the pointer and not previewed, and passed over by every wheel notch. The keys
/// still land on them; the caller steps past (#12879, the Pattern Stamp's Behind on the
/// Background).
#[allow(clippy::too_many_arguments)]
pub fn dropdown_wheel_grouped_enabled<T: PartialEq + Clone>(
    ui: &mut Ui,
    id: &str,
    name: &str,
    current: &mut T,
    options: &[(T, &str)],
    width: f32,
    starts_group: impl Fn(&T) -> bool,
    enabled: impl Fn(&T) -> bool,
) -> (Vec<T>, Option<T>) {
    dropdown_with(ui, id, name, current, options, width, true, true, &starts_group, &enabled)
}

#[allow(clippy::too_many_arguments)]
fn dropdown_with<T: PartialEq + Clone>(
    ui: &mut Ui,
    id: &str,
    name: &str,
    current: &mut T,
    options: &[(T, &str)],
    width: f32,
    wheel: bool,
    translate: bool,
    starts_group: &dyn Fn(&T) -> bool,
    enabled: &dyn Fn(&T) -> bool,
) -> (Vec<T>, Option<T>) {
    let shown = |l: &str| if translate { tl!(l).to_string() } else { l.to_string() };
    let label = options.iter().find(|(v, _)| v == current).map(|(_, l)| shown(l)).unwrap_or_else(|| "—".into());
    let (mut chosen, mut hovered) = (Vec::new(), None);
    // The wheel's notches add up over the button and the open list alike.
    let wheel_id = ui.make_persistent_id(id).with("wheel");
    let wheel = wheel && ui.is_enabled();
    let (mut over_list, mut wheeled, mut keyed) = (false, false, false);
    // A wheel step scrolls the open list to the new value: at once over the list, the next frame
    // after a step over the button.
    let reveal_id = wheel_id.with("reveal");
    let mut reveal = wheel && ui.data_mut(|d| d.remove_temp::<bool>(reveal_id)).unwrap_or(false);
    // The list opens on the press: press, drag onto an option and release to choose it (#2735).
    let (press, close) = crate::press_menu::PressCombo::before(ui, id);
    let texts: Vec<String> = options.iter().map(|(_, l)| shown(l)).collect();
    let response = combo_list(ui, id, label, width, close, |ui| {
        fit_widest_option(ui, texts.iter().map(|l| l.as_str()));
        // A typed letter jumps to the next option starting with it, ↩ closes the list (#1481).
        let labels: Vec<&str> = texts.iter().map(|l| l.as_str()).collect();
        let shown = options.iter().position(|(v, _)| v == current);
        if let Some((v, _)) = type_ahead::keys(ui, ui.make_persistent_id(id), shown, &labels).and_then(|i| options.get(i)) {
            *current = v.clone();
            chosen.push(v.clone());
            reveal = true;
            keyed = true;
        }
        // Over the open list the wheel steps the value too, instead of scrolling the list: a
        // gesture that started over the list or the button, not one scrolling the panel that
        // carried the list under the pointer (#11353).
        over_list = wheel && ui.rect_contains_pointer(ui.clip_rect());
        if over_list {
            let stepped = wheel_steps(crate::field_wheel::hover_notches(ui, wheel_id, ui.clip_rect(), 1.0), current, options, enabled);
            wheeled = !stepped.is_empty();
            reveal |= wheeled;
            chosen.extend(stepped);
        }
        // The row nearest the pointer, by its distance to each row.
        let pointer = ui.ctx().pointer_hover_pos().filter(|_| ui.rect_contains_pointer(ui.clip_rect()));
        let mut nearest: Option<(f32, &T)> = None;
        for (i, ((v, _), text)) in options.iter().zip(texts.iter()).enumerate() {
            if i > 0 && starts_group(v) {
                list_separator(ui);
            }
            let on = enabled(v);
            let item = if on {
                ui.selectable_label(v == current, text.as_str())
            } else {
                ui.add_enabled(false, egui::Button::selectable(v == current, text.as_str()))
            };
            if reveal && v == current {
                item.scroll_to_me(None);
            }
            // A greyed option is neither previewed nor picked.
            if !on {
                continue;
            }
            if item.hovered() {
                hovered = Some(v.clone());
            }
            if let Some(p) = pointer {
                let d = (item.rect.top() - p.y).max(p.y - item.rect.bottom()).max(0.0);
                if nearest.is_none_or(|(n, _)| d < n) {
                    nearest = Some((d, v));
                }
            }
            if press.chosen(ui, &item) {
                *current = v.clone();
                chosen.push(v.clone());
            }
        }
        // Between two rows (item spacing) the pointer hovers neither; it is still over the list,
        // so keep previewing the nearest row instead of flashing the current value back (#2553).
        if hovered.is_none() {
            hovered = nearest.map(|(_, v)| v.clone());
        }
    });
    // A scrubby caption drawn just before the dropdown names it; otherwise its own name does.
    let response = match take_label_scrub_name(ui, response.response.rect) {
        Some(label) => response.response.labelled_by(label),
        None => response.response.accessible_name(name),
    };
    press.after(&response);
    if combo_box_arrow_keys(ui, &response, current, options) {
        chosen.push(current.clone());
    }
    if wheel && !over_list {
        // A gesture that started over the button or the open list, not one scrolling the panel
        // past it (#11353). Asked off them too, so a gesture that leaves them is the panel's.
        let notches = crate::field_wheel::hover_notches(ui, wheel_id, response.rect, 1.0);
        let stepped = wheel_steps(notches, current, options, enabled);
        wheeled = !stepped.is_empty();
        if wheeled && craft_ui::ComboBox::is_open(ui.ctx(), response.id) {
            ui.data_mut(|d| d.insert_temp(reveal_id, true));
        }
        chosen.extend(stepped);
    }
    // Nor is the option under a pointer resting where the keyboard picked another.
    if wheel_rests(ui, wheel_id, wheeled || keyed) {
        hovered = None;
    }
    (chosen, hovered)
}

/// A line between two groups of an open dropdown list's options, as wide as the list. Like egui's
/// separator it takes no width while egui sizes the list, so it never widens it.
fn list_separator(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let width = if ui.is_sizing_pass() { 0.0 } else { ui.available_width() };
    let (rect, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, t.separator));
}

/// Makes an open dropdown list as wide as its widest option is when hovered, from the start. egui
/// takes a plain row's frame stroke off its padding but draws a hovered row with the stroke, so a
/// row grows by two stroke widths under the pointer, and the list never shrinks back: scrolling
/// the widest option under the pointer widened the list (#2607).
fn fit_widest_option<'a>(ui: &mut Ui, labels: impl Iterator<Item = &'a str>) {
    let extend = Some(egui::TextWrapMode::Extend);
    let widest = labels.map(|l| egui::WidgetText::from(l).into_galley(ui, extend, f32::INFINITY, egui::FontSelection::Default).size().x).fold(0.0, f32::max);
    ui.set_min_width(widest + 2.0 * ui.spacing().button_padding.x);
}

/// Whether the pointer still rests where the wheel last stepped dropdown `id` (`wheeled`: it did
/// this frame). The option under it then isn't the one to preview: the value moved on without it.
fn wheel_rests(ui: &Ui, id: egui::Id, wheeled: bool) -> bool {
    let key = id.with("at");
    let pointer = ui.input(|i| i.pointer.hover_pos());
    if wheeled && let Some(p) = pointer {
        ui.data_mut(|d| d.insert_temp(key, p));
    }
    let rests = pointer.is_some() && ui.data(|d| d.get_temp::<Pos2>(key)) == pointer;
    if !rests {
        ui.data_mut(|d| d.remove::<Pos2>(key));
    }
    rests
}

/// The options `notches` wheel notches step `current` through, one per notch, in order; the last
/// is the new `current`. A notch goes to the next option `enabled` accepts, so a greyed one costs
/// no notch.
fn wheel_steps<T: PartialEq + Clone>(notches: f32, current: &mut T, options: &[(T, &str)], enabled: &dyn Fn(&T) -> bool) -> Vec<T> {
    let Some(mut index) = options.iter().position(|(v, _)| v == current) else { return Vec::new() };
    let mut chosen = Vec::new();
    // Down (negative) is the next option. No more steps than there are options.
    for _ in 0..(notches.abs().min(options.len() as f32) as usize) {
        let mut ahead = options.iter().enumerate().skip(index.saturating_add(1));
        let mut behind = options.iter().enumerate().take(index).rev();
        let next = if notches < 0.0 { ahead.find(|(_, (v, _))| enabled(v)) } else { behind.find(|(_, (v, _))| enabled(v)) };
        let Some((next, (v, _))) = next else { break };
        index = next;
        chosen.push(v.clone());
    }
    if let Some(v) = chosen.last() {
        *current = v.clone();
    }
    chosen
}

/// Give a dropdown keyboard focus when it opens, then use the arrow keys to move through its
/// choices. `craft_ui::ComboBox` opens a popup but leaves focus on the canvas by default, which makes
/// controls such as the Layers panel's Blend Mode dropdown unreachable from the keyboard.
fn combo_box_arrow_keys<T: PartialEq + Clone>(ui: &mut Ui, response: &Response, current: &mut T, options: &[(T, &str)]) -> bool {
    if response.clicked() {
        response.request_focus();
    }
    // The popup, rather than its button, becomes the focused egui layer after it opens. While it
    // is open it owns its navigation keys, even though `response.has_focus()` is then false.
    if !craft_ui::ComboBox::is_open(ui.ctx(), response.id) || options.is_empty() {
        return false;
    }
    let step = if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
        1
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
        -1
    } else {
        return false;
    };
    let index = options.iter().position(|(value, _)| value == current).unwrap_or(0);
    let next = if step > 0 { (index + 1).min(options.len() - 1) } else { index.saturating_sub(1) };
    match options.get(next) {
        Some((value, _)) if next != index => {
            *current = value.clone();
            true
        }
        _ => false,
    }
}

/// The body of a right-click menu: as tall as its items up to the part of the window that can be
/// seen (clear of a taskbar a too-tall window runs under, #315), scrolling past that, so long
/// context menus (a layer's, the canvas tools') stay reachable on small windows instead of running
/// off the bottom. egui moves a popup up to keep it in the window, so a menu opened low on the
/// screen first shifts up and only scrolls when it is taller than the window. An overflowing
/// menu's ▲/▼ scroll arrows (#2091) count toward that height, so both stay on screen.
pub fn menu_scroll<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    // The popup frame's margin and stroke, and a small gap to the window's edges.
    let frame = ui.spacing().menu_margin.sum().y + 2.0 + 2.0 * MENU_EDGE;
    let room = (crate::work_area::visible_rect(ui.ctx()).height() - frame).max(MENU_MIN_HEIGHT);
    // An overflowing menu gets Photoshop's scroll arrows above and below its rows (#2091): the
    // thin floating scroll bar alone gave no hint that more items were hidden. Whether it
    // overflows is known only after layout, so it is remembered from the last frame.
    let id = ui.scope_id().with("menu_scroll");
    let overflow = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let offset = ui.data(|d| d.get_temp::<f32>(id.with("offset"))).unwrap_or(0.0);
    // The arrows are as wide as the rows were last frame, not the popup's whole available width:
    // that widened an overflowing menu on every later open (a layer menu went from ~230 to ~600
    // pt). The output's `inner_rect` is the width asked for, before auto-shrink fits the rows.
    let width = ui.data(|d| d.get_temp::<f32>(id.with("width"))).unwrap_or(0.0);
    // The arrows come out of the room, so both stay on screen: the minimum is the whole menu's
    // height, arrows included. `room` is at least `MENU_MIN_HEIGHT`, which is taller than the two
    // arrows, so the rows always keep a positive height; a window shorter than that minimum still
    // gets a menu of that height, as one without arrows does.
    let arrows = if overflow { 2.0 * MENU_ARROW_HEIGHT } else { 0.0 };
    let room = (room - arrows).max(MENU_MIN_HEIGHT - arrows);
    // The arrows sit flush against the rows, so no item spacing is added around them that `room`
    // doesn't count; the rows keep the menu's own spacing.
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut step = 0.0;
    if overflow {
        step += menu_scroll_arrow(ui, true, offset > 0.5, width);
    }
    // A popup's Ui is only as tall as the popup was last frame (400 pt on the first), so ask for
    // the whole room: the area still shrinks to its rows when they need less.
    let mut area = egui::ScrollArea::vertical().id_salt(id).max_height(room).min_scrolled_height(room);
    // A step is taken on the pass after the arrow was hovered, and only then: a menu closed while
    // an arrow was hovered doesn't jump when it next opens.
    let now = ui.ctx().cumulative_pass_nr();
    let pending = ui.data(|d| d.get_temp::<(f32, u64)>(id.with("step"))).filter(|&(_, pass)| pass.saturating_add(1) == now).map_or(0.0, |(step, _)| step);
    if pending != 0.0 {
        area = area.vertical_scroll_offset((offset + pending).max(0.0));
    }
    let out = area.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = spacing;
        add_contents(ui)
    });
    let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
    let offset = out.state.clamped_offset().y.clamp(0.0, max);
    if overflow {
        step += menu_scroll_arrow(ui, false, offset < max - 0.5, width);
    }
    ui.spacing_mut().item_spacing.y = spacing;
    ui.data_mut(|d| {
        d.insert_temp(id, max > 0.5);
        d.insert_temp(id.with("offset"), offset);
        d.insert_temp(id.with("width"), out.content_size.x);
        d.insert_temp(id.with("step"), (step, now));
    });
    out.inner
}

/// A dropdown: a button showing `selected_text` (at least `width` wide, wider for a longer text)
/// whose list (`add_contents`) opens below it on a click. The list is as tall as its options up to
/// the room the window has on that side, scrolling only past that (#5495): Photoshop's dropdowns
/// open at full height. egui's `ComboBox` capped every list at the size its popup takes on the
/// frame it opens (`default_area_size`, 400 pt), so the Layers panel's 27 blend modes scrolled.
///
/// The list never covers its button, as in Photoshop: it opens below the button, or above it when
/// the whole list doesn't fit below and there is more room above, and scrolls when it doesn't fit
/// on that side either. Over the button, the release of the click that opened it would choose the
/// row under the pointer. Its left edge is the button's and its side only depends on its rows'
/// height, so it stays put while its value (and with it the button's width) changes, as the wheel
/// steps it over the open list (#1747). The ids are `craft_ui::ComboBox`'s (egui's), so
/// `ComboBox::is_open` and [`crate::press_menu::PressCombo`] apply.
pub fn combo_list<R>(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    selected_text: impl Into<egui::WidgetText>,
    width: f32,
    close: egui::PopupCloseBehavior,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let button_id = ui.make_persistent_id(egui::IdSalt::new(id_salt));
    let popup_id = button_id.with("popup");
    let selected_text = selected_text.into();
    ui.horizontal(|ui| {
        let ctx = ui.ctx().clone();
        let button = combo_button(ui, button_id, &selected_text, width, egui::Popup::is_id_open(&ctx, popup_id));
        let enabled = ui.is_enabled();
        button.widget_info(|| {
            let mut info = egui::WidgetInfo::new(egui::Role::ComboBox);
            info.enabled = enabled;
            info.current_text_value = Some(selected_text.text().to_owned());
            info
        });
        // The room below and above the button, less the popup frame's margin and stroke. The
        // rows' full height is measured on the (invisible) pass that sizes the list as it opens.
        let visible = crate::work_area::visible_rect(&ctx).shrink(MENU_EDGE);
        let frame = ui.spacing().menu_margin.sum().y + 2.0;
        let below = visible.bottom() - button.rect.bottom() - frame;
        let above = button.rect.top() - visible.top() - frame;
        let rows_id = popup_id.with("rows");
        let rows = ctx.data(|d| d.get_temp::<f32>(rows_id));
        let up = rows.is_some_and(|rows| rows > below) && above > below;
        // Only the room that side has, even when it is small: a taller list would be pushed back
        // into the window, over its button (#5495 review).
        let room = (if up { above } else { below }).max(0.0);
        // Above the button the list's top is placed from its known height: egui would place its
        // bottom by the size the list had last frame, so it would move as it settles, under a
        // click aimed at a row. `chrome` is the popup frame's margin and stroke.
        let anchor = match rows.filter(|_| up) {
            Some(rows) => {
                let chrome = egui::Frame::popup(&ctx.global_style()).total_margin().sum().y;
                let top = button.rect.top() - rows.min(room) - chrome;
                egui::Rect::from_x_y_ranges(button.rect.x_range(), top - button.rect.height()..=top)
            }
            None => button.rect,
        };
        let parent_layer = ui.layer_id();
        let inner = egui::Popup::menu(&button)
            .id(popup_id)
            .anchor(anchor)
            .align(egui::RectAlign::BOTTOM_START)
            .align_alternatives(&[])
            .width(button.rect.width())
            .close_behavior(close)
            .style(egui::style::StyleModifier::default())
            .show(|ui| {
                crate::press_menu::above_parent_ui(parent_layer, ui);
                ui.set_min_width(ui.available_width());
                // A popup's Ui is only as tall as the popup was last frame (400 pt on the first),
                // so ask for the whole room: the area still shrinks to its rows when they need less.
                let scroll = egui::ScrollArea::vertical().max_height(room).min_scrolled_height(room).show(ui, |ui| {
                    // Options don't wrap in a list as narrow as its button: they widen it.
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    add_contents(ui)
                });
                let rows = scroll.content_size.y;
                ui.ctx().data_mut(|d| d.insert_temp(rows_id, rows));
                scroll.inner
            })
            .map(|r| r.inner);
        egui::InnerResponse::new(inner, button)
    })
    .inner
}

/// The button of a [`combo_list`], drawn as egui's `ComboBox` draws its own.
fn combo_button(ui: &mut Ui, id: egui::Id, text: &egui::WidgetText, width: f32, open: bool) -> Response {
    let background = ui.painter().add(egui::Shape::Noop);
    let margin = ui.spacing().button_padding;
    let min_height = ui.spacing().interact_size.y;
    let mut outer = ui.available_rect_before_wrap();
    outer.set_height(outer.height().max(min_height));
    let mut content = ui.new_child(egui::UiBuilder::new().max_rect(outer.shrink2(margin)));
    let icon_spacing = content.spacing().icon_spacing;
    let icon_size = Vec2::splat(content.spacing().icon_width);
    let wrap_mode = content.wrap_mode();
    let wrap_width = if wrap_mode == egui::TextWrapMode::Extend { f32::INFINITY } else { content.available_width() - icon_spacing - icon_size.x };
    let galley = text.clone().into_galley(&content, Some(wrap_mode), wrap_width, egui::TextStyle::Button);
    let size = vec2((galley.size().x + icon_spacing + icon_size.x).max(width - 2.0 * margin.x), galley.size().y.max(icon_size.y));
    let (_, rect) = content.allocate_space(size);
    let mut outer = content.min_rect().expand2(margin);
    outer.set_height(outer.height().max(min_height));
    let response = ui.interact(outer, id, Sense::click());
    if ui.is_rect_visible(outer) {
        let visuals = if open { &ui.visuals().widgets.open } else { ui.style().interact(&response) };
        let shape =
            egui::epaint::RectShape::new(outer.expand(visuals.expansion), visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, StrokeKind::Inside);
        ui.painter().set(background, shape);
        let icon_rect = Align2::RIGHT_CENTER.align_size_within_rect(icon_size, rect);
        chevron_icon(ui, icon_rect.expand(visuals.expansion), visuals, open);
        let text_rect = Align2::LEFT_CENTER.align_size_within_rect(galley.size(), rect);
        ui.painter().galley(text_rect.min, galley, visuals.text_color());
    }
    ui.advance_cursor_after_rect(outer);
    response
}

/// One scroll arrow of an overflowing menu: hovering scrolls that way, as in Photoshop's and the
/// system menus. It doesn't page on a click: egui closes a context menu on any click inside it,
/// so the page would never be seen. `width` is the rows' width. Returns the scroll step for the
/// next frame.
fn menu_scroll_arrow(ui: &mut Ui, up: bool, enabled: bool, width: f32) -> f32 {
    let width = if width.is_finite() { width.max(MENU_ARROW_HEIGHT) } else { MENU_ARROW_HEIGHT };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, MENU_ARROW_HEIGHT), Sense::hover());
    let t = Tokens::get(ui.ctx());
    let color = if enabled { t.text } else { t.text_dim.gamma_multiply(0.5) };
    let (c, h) = (rect.center(), 3.5);
    let pts = if up {
        vec![pos2(c.x - h * 1.4, c.y + h * 0.6), pos2(c.x + h * 1.4, c.y + h * 0.6), pos2(c.x, c.y - h * 0.8)]
    } else {
        vec![pos2(c.x - h * 1.4, c.y - h * 0.6), pos2(c.x + h * 1.4, c.y - h * 0.6), pos2(c.x, c.y + h * 0.8)]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    if !enabled {
        return 0.0;
    }
    if !resp.hovered() {
        return 0.0;
    }
    ui.ctx().request_repaint();
    let sign = if up { -1.0 } else { 1.0 };
    sign * MENU_ARROW_SPEED * ui.input(|i| i.stable_dt).clamp(0.0, 0.1)
}

/// Gap kept between a context menu and the window's edges.
const MENU_EDGE: f32 = 4.0;
/// The shortest a context menu gets, scroll arrows included.
pub(super) const MENU_MIN_HEIGHT: f32 = 120.0;
/// Height of an overflowing menu's scroll arrows, and how fast hovering one scrolls (points per
/// second).
pub(super) const MENU_ARROW_HEIGHT: f32 = 14.0;
const MENU_ARROW_SPEED: f32 = 400.0;

/// The colour picker popup of a colour swatch: a click on `swatch` toggles it, a click outside
/// closes it. While open, its left edge stays where it first showed (at the swatch, or further
/// left when the window edge needs it), below the swatch or above it as room allows. The picker's
/// width follows its value readouts, and placing it anew every frame moved it under the pointer,
/// flipping it from side to side near the right edge of the window (#534).
pub fn swatch_popup(swatch: &Response) -> egui::Popup<'static> {
    // Room for the readouts to widen after the picker opened.
    const SLACK: f32 = 32.0;
    let popup = egui::Popup::from_toggle_button_response(swatch).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    let ctx = &swatch.ctx;
    let key = popup.get_id().with("left");
    if !popup.is_open() {
        ctx.data_mut(|d| d.remove::<f32>(key));
        return popup;
    }
    crate::press_menu::above_parent(ctx, swatch.layer_id, egui::LayerId::new(egui::Order::Foreground, popup.get_id()));
    // Its width is known from the frame after it opened (egui sizes it unseen first).
    let left = ctx.data(|d| d.get_temp::<f32>(key)).or_else(|| {
        let width = popup.get_expected_size()?.x;
        let screen = ctx.content_rect();
        let left = swatch.rect.left().min(screen.right() - width - SLACK).max(screen.left());
        ctx.data_mut(|d| d.insert_temp(key, left));
        Some(left)
    });
    let Some(left) = left else { return popup };
    popup
        .anchor(Rect::from_x_y_ranges(left..=left, swatch.rect.y_range()))
        .align(egui::RectAlign::BOTTOM_START)
        .align_alternatives(&[egui::RectAlign::TOP_START])
}

pub fn dropdown_with_tooltips<T: PartialEq + Clone>(ui: &mut Ui, id: &str, name: &str, current: &mut T, options: &[(T, &str, &str)], width: f32) -> bool {
    let label = options.iter().find(|(v, _, _)| v == current).map(|(_, l, _)| tl!(l)).unwrap_or("—");
    let mut changed = false;
    let (press, close) = crate::press_menu::PressCombo::before(ui, id);
    let response = combo_list(ui, id, label, width, close, |ui| {
        fit_widest_option(ui, options.iter().map(|(_, l, _)| tl!(l)));
        let labels: Vec<&str> = options.iter().map(|(_, l, _)| tl!(l)).collect();
        let shown = options.iter().position(|(v, _, _)| v == current);
        if let Some((v, _, _)) = type_ahead::keys(ui, ui.make_persistent_id(id), shown, &labels).and_then(|i| options.get(i)) {
            *current = v.clone();
            changed = true;
        }
        for (v, l, tip) in options {
            let item = ui.selectable_label(v == current, tl!(l)).on_hover_text(tl!(tip));
            if press.chosen(ui, &item) {
                *current = v.clone();
                changed = true;
            }
        }
    });
    let response = match take_label_scrub_name(ui, response.response.rect) {
        Some(label) => response.response.labelled_by(label),
        None => response.response.accessible_name(name),
    };
    press.after(&response);
    if let Some((_, _, tip)) = options.iter().find(|(v, _, _)| v == current) {
        response.on_hover_text(tl!(tip));
    }
    changed
}
