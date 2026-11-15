//! The right-aligned label column in front of a dialog's fields ("Width:", "Contents:"…).
//!
//! The column is as wide as its widest label in the current language, between the English width a
//! dialog was laid out for and a cap that still leaves room for its fields; a label wider than the
//! cap elides with '…' and shows the full text on hover. Nothing is ever painted outside the
//! column, so a longer translation cannot run out of the dialog (`docs/ui-design.md`: "A label
//! never overflows its row", #3580).

use super::{Align2, Color32, Response, Sense, Tokens, Ui, pos2, vec2};

/// Gap between a column label and its field.
pub const LABEL_COLUMN_GAP: f32 = 6.0;
/// Height of a label column row (a field's).
const ROW_H: f32 = 22.0;

/// The font a column label is drawn in, and measured with by [`label_column_width`].
fn label_font() -> egui::FontId {
    egui::FontId::proportional(12.0)
}

/// Width of a label column that holds every one of `labels` (already translated) with its gap,
/// at least `min_w` (the width the dialog was laid out for in English) and at most `max_w`.
/// Measure once per dialog body and pass the result to every [`column_label`] so the fields stay
/// aligned. Never panics: a bad bound (NaN, `max_w < min_w`) falls back to `min_w`.
pub fn label_column_width(ui: &Ui, labels: &[&str], min_w: f32, max_w: f32) -> f32 {
    let widest = labels.iter().map(|s| ui.painter().layout_no_wrap((*s).to_owned(), label_font(), Color32::PLACEHOLDER).size().x).fold(0.0, f32::max);
    let min_w = if min_w.is_finite() { min_w.max(0.0) } else { 0.0 };
    let max_w = if max_w.is_finite() { max_w.max(min_w) } else { min_w };
    // `max`/`min` rather than `clamp`, which panics on NaN bounds.
    (widest + LABEL_COLUMN_GAP).ceil().max(min_w).min(max_w)
}

/// A field label right-aligned in a `width`-wide column, ending [`LABEL_COLUMN_GAP`] before its
/// field. A label too wide for the column elides with '…' and shows its full text on hover.
pub fn column_label(ui: &mut Ui, text: &str, width: f32) -> Response {
    column_label_padded(ui, text, width, LABEL_COLUMN_GAP)
}

/// [`column_label`] ending `gap` before its field (a panel's tighter column).
pub fn column_label_padded(ui: &mut Ui, text: &str, width: f32, gap: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    // `f32::max` drops a NaN, so a bad width or gap allocates nothing rather than misbehaving.
    let width = width.max(0.0);
    let gap = gap.max(0.0).min(width);
    let (r, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
    if text.is_empty() {
        return resp;
    }
    let galley = crate::tab_strip::elided(ui, text, label_font(), t.text_dim, width - gap);
    let elided = galley.elided;
    let at = Align2::RIGHT_CENTER.anchor_size(pos2(r.right() - gap, r.center().y), galley.size());
    ui.painter().galley(at.min, galley, t.text_dim);
    if elided { resp.on_hover_text(text) } else { resp }
}
