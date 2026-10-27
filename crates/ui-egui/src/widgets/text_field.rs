use super::panel::surface;
use super::*;

/// The most characters a [`text_field`] holds unless its caller sets a limit: far beyond any
/// search, name or hex value, yet a pasted multi-gigabyte string is cut at the field instead of
/// being laid out every frame (#2150).
pub const TEXT_FIELD_LIMIT: usize = 1024;

/// The most characters one typed, pasted or IME-committed text event carries into the app
/// ([`cap_text_events`]): a megabyte of text, more than any field or text layer is pasted.
pub const INPUT_EVENT_LIMIT: usize = 1 << 20;

/// Cuts every text, paste and IME event to [`INPUT_EVENT_LIMIT`] characters (at a character
/// boundary) before any widget sees it, so no text field -- whatever its own limit -- has to
/// lay out an 8 GB paste (#2150).
pub fn cap_text_events(events: &mut [egui::Event]) {
    fn cut(text: &mut String) -> bool {
        let Some((end, _)) = text.char_indices().nth(INPUT_EVENT_LIMIT) else { return false };
        text.truncate(end);
        true
    }
    for event in events {
        match event {
            egui::Event::Text(text) | egui::Event::Paste(text) | egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                cut(text);
            }
            // The highlighted range indexes the old text; past the cut it would point nowhere.
            egui::Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars }) => {
                if cut(text) && active_range_chars.as_ref().is_some_and(|r| r.end > INPUT_EVENT_LIMIT) {
                    *active_range_chars = None;
                }
            }
            _ => {}
        }
    }
}

/// Height of a [`text_field`]'s box, matching a [`value_field`].
pub const TEXT_FIELD_H: f32 = 24.0;

/// Inner padding of a [`text_field`]'s box: 6 pt either side, 2 pt above and below.
const TEXT_FIELD_PAD: Vec2 = vec2(6.0, 2.0);

/// A single-line text field in PhotoCraft's shared field style (#3789): the same box as a
/// [`value_field`] -- the theme's field fill and `field_border`, rounded by `radius_sm` -- with a
/// consistent inner padding and no focus outline. egui's default white focus ring (its
/// `selection.stroke`) is what made the Preferences search field look different from the Color
/// Picker's hex field, so every plain, non-numeric input (search, hex, name, filter) is drawn
/// through here to look and behave the same. `id` carries the field's text and focus across
/// frames; `hint` shows while it is empty; `mono` picks the monospaced face for hex and other
/// code-like values.
pub fn text_field(ui: &mut Ui, id: egui::Id, text: &mut String, hint: &str, width: f32, mono: bool) -> Response {
    text_field_ex(ui, id, text, hint, width, mono, None)
}

/// [`text_field`] limited to `char_limit` characters (a hex field's six digits, say), or to
/// [`TEXT_FIELD_LIMIT`] without one.
pub fn text_field_ex(ui: &mut Ui, id: egui::Id, text: &mut String, hint: &str, width: f32, mono: bool, char_limit: Option<usize>) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(16.0), TEXT_FIELD_H), Sense::hover());
    surface(ui, rect, t.field, false);
    if !t.bevel {
        ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    }
    let font = if mono { theme::mono(12.0) } else { egui::FontId::proportional(12.5) };
    let inner = rect.shrink2(TEXT_FIELD_PAD);
    let limit = char_limit.unwrap_or(TEXT_FIELD_LIMIT);
    // A value already past the limit (restored from a state file, set by automation) is shown and
    // edited as its first `limit` characters, so it is never laid out whole; the caller's string
    // changes only when that copy is edited.
    let mut bounded = text.char_indices().nth(limit).map(|(end, _)| text[..end].to_string());
    let shown = match bounded.as_mut() {
        Some(b) => b,
        None => &mut *text,
    };
    // TextEdit draws its text at the top of its rect; fill the box and centre the line in it, so
    // the text sits on the field's middle like a dropdown's or value field's.
    let edit = egui::TextEdit::singleline(shown)
        .id(id)
        .hint_text(hint.to_owned())
        .frame(egui::Frame::NONE)
        .font(font)
        .min_size(inner.size())
        .align(egui::Align2::LEFT_CENTER)
        .char_limit(limit);
    // A child Ui keeps the field's selection colour local: `Visuals::selection` also fills selected
    // list rows and slider trails, which must keep the theme's.
    let layout = egui::Layout::centered_and_justified(egui::Direction::TopDown);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(layout));
    // The theme's 24 pt interact height would make the edit taller than the box's inside.
    child.spacing_mut().interact_size.y = inner.height();
    child.style_mut().visuals.selection.bg_fill = t.field_selection();
    let response = child.add(edit);
    if let Some(b) = bounded
        && response.changed()
    {
        *text = b;
    }
    response
}

pub(super) fn first_field_id() -> egui::Id {
    egui::Id::unique("pc-focus-first-field")
}

/// While `on`, the next number field drawn takes the keyboard focus with its text selected (a
/// dialog opening on its first value, as in Photoshop). Turn it off once the body is laid out.
pub fn focus_first_field(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| {
        if on {
            d.insert_temp(first_field_id(), true);
        } else {
            d.remove::<bool>(first_field_id());
        }
    });
}
