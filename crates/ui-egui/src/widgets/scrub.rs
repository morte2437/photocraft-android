use super::*;

/// A drag on a value field's label this frame, for the value field drawn next on the label's row
/// (Photoshop's scrubby labels). Left by [`scrub_label`], taken by that field.
#[derive(Clone, Copy, Debug)]
pub(super) struct LabelScrub {
    pass: u64,
    /// The id whose drags move the field (the label itself, or an options caption's hit area).
    pub(super) label: egui::Id,
    /// The visible label's own id, which names the field it scrubs.
    pub(super) name: Option<egui::Id>,
    pub(super) rect: Rect,
    /// How far right of the label the field may start (the whole row for a slider row).
    pub(super) max_gap: f32,
    /// Options captions defer interaction until the adjacent field bounds are known.
    pub(super) option_area: bool,
    /// An options icon's tooltip, shown by the drag area laid over it (which takes its hover).
    pub(super) tip: Option<&'static str>,
    /// Whole steps to move the value (left negative; Shift ×10, Alt a tenth as fast).
    pub(super) steps: f32,
    pub(super) started: bool,
    pub(super) dragging: bool,
    pub(super) stopped: bool,
}

fn label_scrub_id() -> egui::Id {
    egui::Id::unique("pc-label-scrub")
}

/// Makes `label` scrubby: dragging it left or right moves the value field drawn right after it on
/// the same row (one step a point, ×10 with Shift, a tenth as fast with Alt), with Photoshop's
/// scrubby cursor. A label with no value field right after it stays a plain label.
pub fn scrub_label(ui: &Ui, label: &Response) -> Response {
    scrub_label_within(ui, label, 24.0)
}

/// A tool-options caption whose scrub target includes the adjacent field's outer box.
/// Keep the original label bounds for linking; register the hit area before the numeric editor.
pub fn scrub_option_label(ui: &Ui, label: &Response) {
    scrub_option_area(ui, label, None);
}

/// [`scrub_option_label`] for an icon with tooltip `tip`. The field's drag area is laid over the
/// icon after it, so egui hovers the area and not the icon: `on_hover_text` on the icon never shows.
pub fn scrub_option_icon(ui: &Ui, icon: &Response, tip: &'static str) {
    scrub_option_area(ui, icon, Some(tip));
}

fn scrub_option_area(ui: &Ui, label: &Response, tip: Option<&'static str>) {
    let scrub = LabelScrub {
        pass: ui.ctx().cumulative_pass_nr(),
        label: label.id.with("pc-option-scrub"),
        name: Some(label.id),
        rect: label.rect,
        max_gap: 24.0,
        option_area: true,
        tip,
        steps: 0.0,
        started: false,
        dragging: false,
        stopped: false,
    };
    ui.data_mut(|d| d.insert_temp(label_scrub_id(), scrub));
}

pub(super) fn scrub_label_within(ui: &Ui, label: &Response, max_gap: f32) -> Response {
    // Drag only, not focusable: Tab goes to the value field, not to its label as well.
    let resp = label.interact(Sense::DRAG);
    let scrub = label_scrub_response(ui, &resp, label.rect, max_gap, Some(label.id));
    ui.data_mut(|d| {
        d.insert_temp(label_scrub_id(), scrub);
        d.insert_temp(resp.id.with("pc-scrub-linked"), false);
    });
    resp
}

pub(super) fn label_scrub_response(ui: &Ui, resp: &Response, original_rect: Rect, max_gap: f32, name: Option<egui::Id>) -> LabelScrub {
    // Whether a field took this label's drags last frame: only then is it scrubby.
    let linked = ui.data(|d| d.get_temp::<bool>(resp.id.with("pc-scrub-linked"))).unwrap_or(false);
    if linked && resp.enabled() && (resp.hovered() || resp.dragged()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let acc_id = resp.id.with("pc-scrub-acc");
    let mut steps = 0.0;
    if resp.dragged() || resp.drag_stopped() {
        let mut acc: f32 = if resp.drag_started() { 0.0 } else { ui.data(|d| d.get_temp(acc_id)).unwrap_or(0.0) };
        let mods = ui.input(|i| i.modifiers);
        let rate = if mods.shift {
            10.0
        } else if mods.alt {
            0.1
        } else {
            1.0
        };
        // The drag starts once the pointer has moved a few points: count those too.
        let dx = match (resp.drag_started(), ui.input(|i| i.pointer.press_origin()), resp.interact_pointer_pos()) {
            (true, Some(origin), Some(pos)) => pos.x - origin.x,
            _ => resp.drag_delta().x,
        };
        acc += dx * rate;
        if !acc.is_finite() {
            acc = 0.0;
        }
        steps = acc.trunc();
        ui.data_mut(|d| d.insert_temp(acc_id, acc - steps));
    }
    LabelScrub {
        pass: ui.ctx().cumulative_pass_nr(),
        label: resp.id,
        name,
        rect: original_rect,
        max_gap,
        option_area: false,
        tip: None,
        steps,
        started: resp.drag_started(),
        dragging: resp.dragged(),
        stopped: resp.drag_stopped(),
    }
}

/// The label drag left for the field at `rect` this frame, if its label is just left of it on
/// its row. Taken either way: a label only scrubs the next field drawn.
pub(super) fn take_label_scrub(ui: &Ui, rect: Rect) -> Option<LabelScrub> {
    let s = ui.data_mut(|d| {
        let s = d.get_temp::<LabelScrub>(label_scrub_id());
        d.remove::<LabelScrub>(label_scrub_id());
        s
    })?;
    let same_row = rect.top() < s.rect.bottom() && rect.bottom() > s.rect.top();
    let gap = rect.left() - s.rect.right();
    let linked = s.pass == ui.ctx().cumulative_pass_nr() && same_row && gap >= -1.0 && gap <= s.max_gap;
    if linked {
        ui.data_mut(|d| d.insert_temp(s.label.with("pc-scrub-linked"), true));
    }
    linked.then_some(s)
}

/// The visible caption left for the control drawn next this frame, if any, consumed so that a
/// non-scrubbing control (a dropdown) can still take its name and the caption cannot leak on.
pub(super) fn take_label_scrub_name(ui: &Ui, rect: Rect) -> Option<egui::Id> {
    let s = ui.data_mut(|d| {
        let s = d.get_temp::<LabelScrub>(label_scrub_id());
        d.remove::<LabelScrub>(label_scrub_id());
        s
    })?;
    if s.pass != ui.ctx().cumulative_pass_nr() {
        return None;
    }
    // Only a caption on the control's own row names it; one left over from an earlier row does not.
    let same_row = rect.top() < s.rect.bottom() && rect.bottom() > s.rect.top();
    same_row.then_some(s.name).flatten()
}
