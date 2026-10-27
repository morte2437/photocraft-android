use super::panel::{chevron_icon, surface};
use super::scrub::{label_scrub_response, take_label_scrub};
use super::slider::slider;
use super::text_field::first_field_id;
use super::units::{FieldUnits, field_resolution_id, fmt_num, fmt_num_prec, fmt_num2, parse_num_in, plain, unit_of_suffix};
use super::*;

/// Monospace numeric field with a dimmed unit suffix, Photoshop style. Drag to scrub. Typed
/// arithmetic (`1920/2`) applies on Enter, Tab or click-away, and a unit after a number converts
/// into the field's unit (`1in` in a px field, at the resolution [`set_field_resolution`] gave).
pub fn value_field(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, width: f32) -> Response {
    value_field_in(ui, value, range, suffix, width, 0.0, FieldOpts::default(), None).0
}

/// A [`value_field`] whose visible caption was drawn apart from it (an options-bar caption in a
/// nested layout): `name` names the number when the caption's scrub link did not reach it.
pub fn value_field_named(ui: &mut Ui, name: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, width: f32) -> Response {
    value_field_in(ui, value, range, suffix, width, 0.0, FieldOpts::default(), Some(name)).0
}

/// A single-line label for a row that also holds fixed-width controls: laid out no wider than
/// `max_w`, cut with '…' when the text (often a longer translation) does not fit, with the full
/// string on hover. Fixed chrome must never let a label overrun its row (#3580), so an *essential*
/// label beside a fixed control should use this rather than a show/hide fit check; a *redundant*
/// label (one whose control already carries the meaning, e.g. an icon row's caption) may be dropped.
pub fn elided_label_sensed(ui: &mut Ui, text: &str, max_w: f32, sense: Sense) -> Response {
    let t = Tokens::get(ui.ctx());
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = crate::tab_strip::elided(ui, text, font, t.text_dim, max_w);
    let elided = galley.elided;
    let (rect, resp) = ui.allocate_exact_size(galley.size(), sense);
    ui.painter().galley(rect.min, galley, t.text_dim);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), text));
    if elided { resp.on_hover_text(text) } else { resp }
}

/// [`elided_label_sensed`] with hover only (a plain, non-interactive label).
pub fn elided_label(ui: &mut Ui, text: &str, max_w: f32) -> Response {
    elided_label_sensed(ui, text, max_w, Sense::hover())
}

/// Height of a [`value_field`]'s box.
pub const VALUE_FIELD_H: f32 = 24.0;

/// A [`value_field`] `height` points tall instead of [`VALUE_FIELD_H`], for slim bars such as
/// the status bar (#2823).
pub fn compact_value_field(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, width: f32, height: f32) -> Response {
    value_field_box(ui, value, range, suffix, vec2(width, height), 0.0, FieldOpts::default(), None).0
}

/// A compact field whose mouse scrub wraps at the native fullscreen viewport's horizontal edges.
pub(crate) fn compact_wrapping_value_field(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, size: Vec2) -> Response {
    value_field_box(ui, value, range, suffix, size, 0.0, FieldOpts { wrap_drag: true, ..Default::default() }, None).0
}

/// How a [`value_field_in`] shows and reads its number, beyond its suffix.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct FieldOpts {
    /// Decimal places shown (default: one, or two for a small range).
    pub(super) decimals: Option<u32>,
    /// Arrow-key and scrub step follow `decimals` (adjustment rows via [`slider_row_prec`], #5914).
    pub(super) step_from_decimals: bool,
    /// The unit typed units convert into (default: the suffix's, at [`set_field_resolution`]).
    pub(super) units: Option<FieldUnits>,
    /// The length (px) 100 % stands for, so lengths and percentages convert into each other.
    pub(super) extent: Option<f64>,
    pub(super) wrap_drag: bool,
}

/// Arrow-key and drag step for a value field: [`FieldOpts::step_from_decimals`] uses `decimals`;
/// otherwise the range width picks 0.01 (≤10) or 1.0.
fn field_step(range: &std::ops::RangeInclusive<f32>, decimals: Option<u32>, step_from_decimals: bool) -> f32 {
    if step_from_decimals && let Some(d) = decimals {
        return 10f32.powi(-(d.min(6) as i32));
    }
    if *range.end() - *range.start() <= 10.0 { 0.01 } else { 1.0 }
}

fn field_fine(range: &std::ops::RangeInclusive<f32>, decimals: Option<u32>, step_from_decimals: bool) -> bool {
    if step_from_decimals { decimals.is_some_and(|d| d > 0) || *range.end() - *range.start() <= 10.0 } else { *range.end() - *range.start() <= 10.0 }
}

/// A [`value_field`] that leaves `trailing` points free inside its box, right of the suffix, and
/// returns the box with the number's response.
// The explicit accessible name accompanies the existing numeric-field parameters.
#[expect(clippy::too_many_arguments)]
pub(super) fn value_field_in(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    width: f32,
    trailing: f32,
    opts: FieldOpts,
    name: Option<&str>,
) -> (Response, Rect) {
    value_field_box(ui, value, range, suffix, vec2(width, VALUE_FIELD_H), trailing, opts, name)
}

/// [`value_field_in`] with a box of any `size`.
// The explicit accessible name accompanies the existing numeric-field parameters.
#[expect(clippy::too_many_arguments)]
fn value_field_box(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    size: Vec2,
    trailing: f32,
    opts: FieldOpts,
    name: Option<&str>,
) -> (Response, Rect) {
    let FieldOpts { decimals, step_from_decimals, units, extent, wrap_drag } = opts;
    let t = Tokens::get(ui.ctx());
    let (rect, slot) = ui.allocate_exact_size(size, Sense::hover());
    let last_editor = ui.data(|d| d.get_temp::<egui::Id>(slot.id));
    let mut option_click = false;
    let scrub = take_label_scrub(ui, rect).map(|s| {
        if !s.option_area {
            return s;
        }
        // A direct click+drag target avoids egui snapping the padding/suffix to the nearby
        // numeric button. It is not focusable; the editor drawn next wins inside its bounds.
        // Preserve a popup field's trailing arrow, and never extend outside the UI clip.
        let hit = Rect::from_min_max(pos2(s.rect.left(), rect.top()), pos2(rect.right() - trailing, rect.bottom())).intersect(ui.clip_rect());
        let mut resp = ui.interact(hit, s.label, Sense::CLICK | Sense::DRAG);
        option_click = resp.clicked();
        if let Some(tip) = s.tip
            && resp.hover_pos().is_some_and(|p| s.rect.contains(p))
        {
            resp = resp.on_hover_text(tip);
        }
        label_scrub_response(ui, &resp, s.rect, s.max_gap, s.name)
    });
    let (dpi, ppi) = ui.data(|d| d.get_temp::<(f64, f64)>(field_resolution_id())).unwrap_or((72.0, 72.0));
    // Typed text loses its spaces before parsing, so a " pt" suffix reads as "pt".
    let typed_suffix = suffix.trim();
    let units = match units {
        Some(u) => crate::numeric_expression::Units { unit: Some(u.unit), suffix: typed_suffix, dpi: u.dpi, ppi: u.ppi, extent },
        None => crate::numeric_expression::Units { unit: unit_of_suffix(typed_suffix), suffix: typed_suffix, dpi, ppi, extent },
    };
    surface(ui, rect, t.field, false);
    if !t.bevel {
        ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    }
    // The number's slot ends 1 pt left of the suffix, by its measured width (the suffix is drawn
    // 6 pt from the right edge), so a one-character unit like `%` leaves the number room (#3040).
    let suffix_font = theme::mono(11.0);
    let suffix_w = if suffix.is_empty() { 0.0 } else { ui.painter().layout_no_wrap(suffix.to_owned(), suffix_font.clone(), t.text_faint).size().x + 3.0 };
    // Centred: the digits' ink then sits within 0.2 pt of the box's centre, where a 1 pt lift
    // (#1322) put it 0.9 pt high and the focus outline off-centre (#5150). The slot is at least
    // one text row tall, so a slim box's number isn't pushed out of it (#2823).
    let font = theme::mono(12.0);
    let row_h = ui.painter().layout_no_wrap("0".into(), font.clone(), t.text).size().y;
    let slot_h = (rect.height() - 4.0).max(row_h).min(rect.height() - 2.0);
    let cy = rect.center().y;
    let field = Rect::from_min_max(pos2(rect.left() + 4.0, cy - slot_h / 2.0), pos2(rect.right() - 4.0 - suffix_w - trailing, cy + slot_h / 2.0));
    // Small ranges (gamma 0.01–9.99, 0–1 centres) need two decimals and a finer drag, like Photoshop.
    let fine = field_fine(&range, decimals, step_from_decimals);
    let unit = field_step(&range, decimals, step_from_decimals);
    let (lo, hi) = (*range.start(), *range.end());
    // The field's id is only known once it's drawn: last frame's, saved under `slot` below.
    let focused = ui.data(|d| d.get_temp::<egui::Id>(slot.id)).filter(|id| ui.memory(|m| m.has_focus(*id)));
    let (step, grid) = arrow_step(ui, slot.id, unit);
    // The wheel steps it as the arrow keys do (#5473), focused or under the pointer.
    let notches = crate::field_wheel::notches(ui, slot.id, focused.is_some(), rect);
    if notches != 0.0
        && let Some(id) = focused
        && ui.data(|d| d.get_temp(id.with("arithmetic"))).unwrap_or(false)
    {
        // A typed sum works itself out first, as before an arrow step.
        ui.memory_mut(|m| m.surrender_focus(id));
    }
    let step = step + notches * unit;
    // new_child (not scope_builder): a scope would move the parent cursor back to the child rect.
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field));
    let mut resp = {
        let ui = &mut child;
        {
            ui.style_mut().visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
            ui.style_mut().visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            ui.style_mut().visuals.widgets.inactive.bg_stroke = Stroke::NONE;
            ui.style_mut().visuals.widgets.hovered.bg_stroke = Stroke::NONE;
            ui.style_mut().visuals.widgets.hovered.weak_bg_fill = Color32::TRANSPARENT;
            ui.style_mut().override_font_id = Some(font);
            // The theme's interact height (24) is taller than the number's slot: the DragValue
            // would grow past the slot's bottom and sit low, or spill out of a slim box (#2823).
            // Its horizontal padding would push a long number out of the box's left edge.
            ui.spacing_mut().interact_size.y = field.height();
            ui.spacing_mut().button_padding = Vec2::ZERO;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let layout = egui::Layout::centered_and_justified(ui.layout().main_dir());
                ui.allocate_ui_with_layout(field.size(), layout, |ui| number_edit(ui, value, range, fine, decimals, units, wrap_drag)).inner
            })
            .inner
        }
    };
    ui.data_mut(|d| d.insert_temp(slot.id, resp.id));
    // A click on the caption's expanded hit area (outside the editor) is not an edit: keep the
    // number's keyboard focus, but never take it for a field that did not have it.
    if option_click
        && let Some(id) = last_editor
        && ui.memory(|m| m.had_focus_last_frame(id))
    {
        ui.memory_mut(|m| m.request_focus(id));
    }
    // A scrubby caption is also the field's visible label, so it names the number for screen readers.
    if let Some(id) = scrub.as_ref().and_then(|s| s.name) {
        resp = resp.labelled_by(id);
    } else if let Some(name) = name {
        // The caption drew in another Ui (or row), so it could not name the number itself.
        resp = resp.accessible_name(name);
    }
    if let Some(s) = scrub {
        if s.started && ui.memory(|m| m.has_focus(resp.id)) {
            ui.memory_mut(|m| m.surrender_focus(resp.id));
        }
        if s.steps != 0.0 {
            // Onto the step grid first, so a whole-number field drops its decimals (55.4 + 1 = 56).
            let v = ((*value / unit).round() + s.steps) * unit;
            let v = (v * 1e4).round() / 1e4;
            // `f32::clamp` panics on a reversed or NaN range, which `DragValue::range` accepts.
            let v = if lo <= hi { v.clamp(lo, hi) } else { v };
            if v.is_finite() && v != *value {
                *value = v;
                resp.mark_changed();
            }
        }
        // Callers coalesce a drag into one history step; the label's drag is the field's.
        use egui::response::Flags;
        resp.flags.set(Flags::DRAG_STARTED, resp.flags.contains(Flags::DRAG_STARTED) || s.started);
        resp.flags.set(Flags::DRAGGED, resp.flags.contains(Flags::DRAGGED) || s.dragging);
        resp.flags.set(Flags::DRAG_STOPPED, resp.flags.contains(Flags::DRAG_STOPPED) || s.stopped);
    }
    if !suffix.is_empty() {
        ui.painter().text(pos2(rect.right() - 6.0 - trailing, rect.center().y), Align2::RIGHT_CENTER, suffix, suffix_font, t.text_faint);
    }
    if step != 0.0 {
        // Round to the step before adding it, so whole-number fields drop decimals (55.4 + 1 = 56).
        let v = (*value / grid).round() * grid + step;
        // Round to 4 decimal places, so repeated 0.1 steps don't leave float errors.
        let v = (v * 1e4).round() / 1e4;
        // `f32::clamp` panics on a reversed or NaN range, which `DragValue::range` accepts.
        let v = if lo <= hi { v.clamp(lo, hi) } else { v };
        // A wild wheel delta must not leave infinity in the field.
        if v.is_finite() {
            *value = v;
        }
        // A wheel step over a field without focus leaves the focus where it is.
        if focused.is_some() {
            // Only take focus back after a typed sum gave it up: re-requesting it while held resets
            // the field's arrow-key filter next frame, and a held arrow then moves focus away (#5474).
            if !ui.memory(|m| m.has_focus(resp.id)) {
                ui.memory_mut(|m| m.request_focus(resp.id));
            }
            // The text stays selected, as in Photoshop: typing after a step replaces the value.
            crate::field_tab::select_all(ui.ctx(), resp.id, &if fine { fmt_num2(f64::from(*value)) } else { fmt_num(f64::from(*value)) });
        }
        resp.mark_changed();
    }
    if ui.memory(|m| m.has_focus(resp.id)) {
        crate::field_wheel::set_focused(ui.ctx(), resp.id, slot.id);
    }
    (resp, rect)
}

/// Width of the ▾ that opens a [`popup_value_field`]'s slider, inside the field's box.
pub const POPUP_ARROW_W: f32 = 14.0;

/// Width of a [`popup_value_field`]'s pop-up slider.
const POPUP_SLIDER_W: f32 = 160.0;

/// What a [`popup_value_field`] did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PopupFieldResponse {
    /// The value changed (typed, scrubbed, stepped with the arrow keys, or set on the slider).
    pub changed: bool,
    /// While the value is dragged (on the slider, from the ▾, or scrubbing the number): a number
    /// for that drag, the same on every frame of it, for the caller to coalesce the drag's edits
    /// into one history step.
    pub drag: Option<u64>,
}

/// A [`value_field`] with Photoshop's pop-up slider (the Layers panel's Opacity and Fill): a click
/// on the ▾ inside the field's right edge opens a slider below it, a click outside closes it.
/// Pressing the ▾ and dragging moves the value straight away, as far as the slider would, and
/// closes the slider on release. The number can still be typed, scrubbed and stepped like any
/// value field. `name` (already translated) names the ▾ for its tooltip and for accessibility.
pub fn popup_value_field(ui: &mut Ui, name: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, width: f32) -> PopupFieldResponse {
    let t = Tokens::get(ui.ctx());
    let (lo, hi) = (*range.start(), *range.end());
    let (mut field, rect) = value_field_in(ui, value, range.clone(), suffix, width, POPUP_ARROW_W, FieldOpts::default(), None);
    let arrow = Rect::from_min_max(pos2(rect.right() - POPUP_ARROW_W - 2.0, rect.top() + 2.0), rect.max - vec2(2.0, 2.0));
    let resp = ui.interact(arrow, field.id.with("popup-slider"), Sense::click_and_drag());
    let name = name.trim_end_matches([':', '：']).to_string();
    // Both the number and its ▾ are named by the control's own semantic name.
    field = field.accessible_name(name.clone());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &name));
    let resp = resp.on_hover_text(&name);
    let drag_key = resp.id.with("drag");
    // Every drag gets a new number when it starts, even if its first frame changes nothing.
    let gesture = |ui: &Ui, started: bool| {
        if started {
            let n = ui.ctx().cumulative_pass_nr();
            ui.data_mut(|d| d.insert_temp(drag_key, n));
        }
        ui.data(|d| d.get_temp::<u64>(drag_key))
    };
    let mut out = PopupFieldResponse { changed: field.changed(), drag: None };
    let scrub = gesture(ui, field.drag_started());
    if field.changed() && field.dragged() {
        out.drag = scrub;
    }

    let popup_id = egui::Popup::default_response_id(&resp);
    let start_key = resp.id.with("start");
    if resp.drag_started() {
        egui::Popup::open_id(ui.ctx(), popup_id);
        ui.data_mut(|d| d.insert_temp(start_key, *value));
        gesture(ui, true);
    }
    if resp.dragged()
        && let (Some(start), Some(origin), Some(pos)) =
            (ui.data(|d| d.get_temp::<f32>(start_key)), ui.input(|i| i.pointer.press_origin()), resp.interact_pointer_pos())
    {
        // The pointer moves the value as much as it would move the slider's knob.
        let track = POPUP_SLIDER_W - 14.0;
        let v = start + (pos.x - origin.x) / track * (hi - lo);
        let v = if lo <= hi && v.is_finite() { v.clamp(lo, hi) } else { start };
        if v != *value {
            *value = v;
            out.changed = true;
        }
        out.drag = gesture(ui, false);
    }
    if resp.drag_stopped() {
        egui::Popup::close_id(ui.ctx(), popup_id);
    }

    let popup = egui::Popup::from_toggle_button_response(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .anchor(rect)
        .align(egui::RectAlign::BOTTOM_END)
        .align_alternatives(&[egui::RectAlign::TOP_END]);
    let open = popup.is_open();
    if resp.hovered() || resp.dragged() || open {
        ui.painter().rect_filled(arrow, t.radius_sm, t.field_border.gamma_multiply(0.6));
    }
    chevron_icon(ui, arrow, ui.style().interact(&resp), open);
    let parent_layer = resp.layer_id;
    popup.show(|ui| {
        crate::press_menu::above_parent_ui(parent_layer, ui);
        ui.set_width(POPUP_SLIDER_W);
        let s = slider(ui, value, range, None);
        s.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*value), &name));
        let key = gesture(ui, s.drag_started());
        if s.changed() {
            out.changed = true;
            // A click on the track is one edit of its own; a drag is one edit however long.
            if s.dragged() {
                out.drag = key;
            }
        }
    });
    out
}

pub(super) const NUMERIC_INPUT_LIMIT: usize = 128;

/// The number in a [`value_field`]. A typed number applies as it's typed; arithmetic waits for
/// Enter, Tab or click-away, because per keystroke `5/2` would land first and a caller that
/// rounds it would cut `5/2*2` short. `changed()` means a new value, not just a keystroke.
fn number_edit(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    fine: bool,
    decimals: Option<u32>,
    units: crate::numeric_expression::Units,
    wrap_drag: bool,
) -> Response {
    let (id, ctx) = (ui.next_auto_id(), ui.ctx().clone());
    // Focused before the DragValue is drawn, so it gains focus this frame and selects its text.
    if ui.data_mut(|d| d.remove_temp::<bool>(first_field_id())).unwrap_or(false) {
        ui.memory_mut(|m| m.request_focus(id));
    }
    // Tab in a dialog goes from one of these to the next (field_tab.rs).
    if ui.is_enabled() {
        crate::field_tab::register(&ctx, id);
    }
    // Use DragValue's focus check before it builds its TextEdit: oversized events must never
    // reach text layout. Registering interest also handles focus arriving by Tab this frame.
    let editing = ui.is_enabled()
        && ui.memory_mut(|m| {
            m.interested_in_focus(id, ui.layer_id());
            m.has_focus(id)
        });
    if editing {
        ui.input_mut(|input| {
            input.events.retain(|event| {
                let text = match event {
                    egui::Event::Text(text) | egui::Event::Paste(text) | egui::Event::Ime(egui::ImeEvent::Commit(text)) => text,
                    egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => text,
                    _ => return true,
                };
                text.chars().nth(NUMERIC_INPUT_LIMIT).is_none()
            });
        });
    }
    let held = id.with("arithmetic");
    let math = ui.memory(|m| m.has_focus(id)) && ui.data(|d| d.get_temp(held)).unwrap_or(false);
    ui.data_mut(|d| d.insert_temp(held, math));
    let before = *value;
    let speed = if fine { 0.01 } else { 0.5 };
    let wrapping = wrap_drag && crate::scrub_wrap::active(ui);
    let wrapped_drag = wrapping && ui.input(|i| i.pointer.primary_down());
    let mut resp = ui.add(
        egui::DragValue::new(value)
            .range(range.clone())
            // Only edits are clamped (below): showing a value past the range, such as a type layer
            // scaled to 2400 pt in a 1296 pt field, must not change it (#5717).
            .clamp_existing_to_range(false)
            // A warp is an absolute-position jump. The scoped scrub below removes that jump.
            .speed(if wrapped_drag { 0.0 } else { speed })
            .custom_formatter(move |v, _| match decimals {
                Some(d) => fmt_num_prec(v, d),
                None if fine => fmt_num2(v),
                None => fmt_num(v),
            })
            .update_while_editing(!math)
            // Focus is read when parsing, not above: Tab hands it on inside `ui.add`.
            .custom_parser(move |s| {
                // Short events can accumulate: reject before either parser normalizes the text.
                if s.chars().nth(NUMERIC_INPUT_LIMIT).is_some() {
                    return None;
                }
                let v = parse_num_in(s, units);
                if ctx.memory(|m| m.has_focus(id)) && plain(s).is_none() {
                    // Still typing arithmetic: hold it, and stop applying keystrokes once it parses.
                    ctx.data_mut(|d| d.insert_temp(held, v.is_some()));
                    return None;
                }
                v
            }),
    );
    if wrap_drag {
        crate::scrub_wrap::update(ui, &resp, value, before, range.clone(), speed, wrapping);
    }
    let (lo, hi) = (*range.start(), *range.end());
    // `f32::clamp` panics on a reversed or NaN range, which `DragValue::range` accepts.
    if *value != before && lo <= hi {
        *value = value.clamp(lo, hi);
    }
    resp.flags.set(egui::response::Flags::CHANGED, *value != before);
    resp
}

/// Increments a numerical field with the up/down arrow keys. Increments by 1 by default, 10 with shift, and 0.1 with ctrl/cmd.
/// Returns the amount to add, and what to round the value to first.
fn arrow_step(ui: &mut Ui, slot: egui::Id, step: f32) -> (f32, f32) {
    use egui::{Key, Modifiers};
    // The field's id is only known once it's drawn, so `value_field` saves it under `slot` for the next frame.
    let Some(id) = ui.data(|d| d.get_temp::<egui::Id>(slot)).filter(|id| ui.memory(|m| m.has_focus(*id))) else {
        return (0.0, step);
    };
    let (n, grid) = ui.input_mut(|i| {
        let (mut n, mut grid) = (0.0, step);
        // egui ignores an extra shift when matching, so shift is checked before the plain arrows to get its larger step.
        for (mods, k) in [(Modifiers::COMMAND, (step / 10.0).max(0.01)), (Modifiers::SHIFT, 10.0 * step), (Modifiers::NONE, step)] {
            let presses = i.count_and_consume_key(mods, Key::ArrowUp) as f32 - i.count_and_consume_key(mods, Key::ArrowDown) as f32;
            if presses != 0.0 && mods == Modifiers::COMMAND {
                grid = k;
            }
            n += k * presses;
        }
        (n, grid)
    });
    // If a sum like 5+5 has been typed, unfocus the field so it calculates it (as Enter would) before we step.
    if n != 0.0 && ui.data(|d| d.get_temp(id.with("arithmetic"))).unwrap_or(false) {
        ui.memory_mut(|m| m.surrender_focus(id));
    }
    (n, grid)
}
