use super::panel::chevron_icon;
use super::scrub::take_label_scrub_name;
use super::slider::wheel_notches_of;
use super::*;

/// A compact labelled dropdown in the studio style.
pub fn dropdown<T: PartialEq + Clone>(ui: &mut Ui, id: &str, name: &str, current: &mut T, options: &[(T, &str)], width: f32) -> bool {
    dropdown_hovered(ui, id, name, current, options, width).0
}

/// [`dropdown`] whose option labels are shown exactly as given: names the user typed (a channel
/// or layer name) must not be translated, unlike the built-in labels [`dropdown`] translates (#5335).
pub fn dropdown_literal<T: PartialEq + Clone>(ui: &mut Ui, id: &str, name: &str, current: &mut T, options: &[(T, &str)], width: f32) -> bool {
    let (chosen, _) = dropdown_with(ui, id, name, current, options, width, false, false);
    !chosen.is_empty()
}

/// [`dropdown`], also returning the option under the pointer in its open list (live previews).
pub fn dropdown_hovered<T: PartialEq + Clone>(ui: &mut Ui, id: &str, name: &str, current: &mut T, options: &[(T, &str)], width: f32) -> (bool, Option<T>) {
    let (chosen, hovered) = dropdown_with(ui, id, name, current, options, width, false, true);
    (!chosen.is_empty(), hovered)
}

/// [`dropdown_hovered`] whose value also steps with the wheel over its button or open list: down
/// picks the next option, up the previous one, one per notch (Shift too), stopping at the ends
/// (Photoshop's Layers panel blend modes, #1747). The wheel then scrolls neither the list, which
/// follows the value instead, nor what the dropdown is in. Returns every option chosen this frame
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
    dropdown_with(ui, id, name, current, options, width, true, true)
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
    let response =
        egui::ComboBox::from_id_salt(id).selected_text(label).width(width).height(420.0).icon(chevron_icon).close_behavior(close).show_ui(ui, |ui| {
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
            // Over the open list the wheel steps the value too, instead of scrolling the list.
            over_list = wheel && ui.rect_contains_pointer(ui.clip_rect());
            if over_list {
                let stepped = wheel_steps(ui, wheel_id, true, current, options);
                wheeled = !stepped.is_empty();
                reveal |= wheeled;
                chosen.extend(stepped);
            }
            // The row nearest the pointer, by its distance to each row.
            let pointer = ui.ctx().pointer_hover_pos().filter(|_| ui.rect_contains_pointer(ui.clip_rect()));
            let mut nearest: Option<(f32, &T)> = None;
            for ((v, _), text) in options.iter().zip(texts.iter()) {
                let item = ui.selectable_label(v == current, text.as_str());
                if reveal && v == current {
                    item.scroll_to_me(None);
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
        let stepped = wheel_steps(ui, wheel_id, response.hovered(), current, options);
        wheeled = !stepped.is_empty();
        if wheeled && egui::ComboBox::is_open(ui.ctx(), response.id) {
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

/// The options the wheel over dropdown `id` (while `hovered`) steps `current` through this frame,
/// one per notch, in order; the last is the new `current`. While the pointer is over it the wheel
/// scrolls nothing else.
fn wheel_steps<T: PartialEq + Clone>(ui: &mut Ui, id: egui::Id, hovered: bool, current: &mut T, options: &[(T, &str)]) -> Vec<T> {
    if hovered {
        ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
    }
    let notches = wheel_notches_of(ui, id, hovered, 1.0);
    let Some(mut index) = options.iter().position(|(v, _)| v == current) else { return Vec::new() };
    let mut chosen = Vec::new();
    // Down (negative) is the next option. No more steps than there are options.
    for _ in 0..(notches.abs().min(options.len() as f32) as usize) {
        let next = if notches < 0.0 { index.checked_add(1) } else { index.checked_sub(1) };
        let Some((next, (v, _))) = next.and_then(|i| options.get(i).map(|o| (i, o))) else { break };
        index = next;
        chosen.push(v.clone());
    }
    if let Some(v) = chosen.last() {
        *current = v.clone();
    }
    chosen
}

/// Give a dropdown keyboard focus when it opens, then use the arrow keys to move through its
/// choices. `egui::ComboBox` opens a popup but leaves focus on the canvas by default, which makes
/// controls such as the Layers panel's Blend Mode dropdown unreachable from the keyboard.
fn combo_box_arrow_keys<T: PartialEq + Clone>(ui: &mut Ui, response: &Response, current: &mut T, options: &[(T, &str)]) -> bool {
    if response.clicked() {
        response.request_focus();
    }
    // The popup, rather than its button, becomes the focused egui layer after it opens. While it
    // is open it owns its navigation keys, even though `response.has_focus()` is then false.
    if !egui::ComboBox::is_open(ui.ctx(), response.id) || options.is_empty() {
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
/// screen first shifts up and only scrolls when it is taller than the window.
pub fn menu_scroll<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    // The popup frame's margin and stroke, and a small gap to the window's edges.
    let frame = ui.spacing().menu_margin.sum().y + 2.0 + 2.0 * MENU_EDGE;
    let room = (crate::work_area::visible_rect(ui.ctx()).height() - frame).max(MENU_MIN_HEIGHT);
    // A popup's Ui is only as tall as the popup was last frame (400 pt on the first), so ask for
    // the whole room: the area still shrinks to its rows when they need less.
    egui::ScrollArea::vertical().max_height(room).min_scrolled_height(room).show(ui, add_contents).inner
}

/// Gap kept between a context menu and the window's edges, and the shortest it gets.
const MENU_EDGE: f32 = 4.0;

const MENU_MIN_HEIGHT: f32 = 120.0;

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
    let response =
        egui::ComboBox::from_id_salt(id).selected_text(label).width(width).height(420.0).icon(chevron_icon).close_behavior(close).show_ui(ui, |ui| {
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
