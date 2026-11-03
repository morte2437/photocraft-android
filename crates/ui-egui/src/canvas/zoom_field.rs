//! Photoshop's rules for a zoom field (status bar, Navigator) while a number is typed into it.
//!
//! A typed zoom is held until Enter (or a click on something other than the pasteboard). Until
//! then the image keeps its zoom, and these keys never apply what was typed:
//!
//! * Esc drops it: the field shows the current zoom again.
//! * Tab and ⇧Tab keep the focus in the field, write the typed number as the field shows a zoom
//!   (`045` and `45,0` become `45`, `1/2` becomes `50`; the `%` is drawn beside the field), select
//!   it so typing replaces it, and play the system alert (Windows: the Asterisk sound).
//! * Alt (tapped) does only that alert: it neither puts the keyboard on the menu bar nor leaves
//!   the field.
//! * A click, with either button, on the pasteboard (the canvas around the image), anywhere on the
//!   title bar (menu titles, the empty drag area, the workspace switcher, search, the theme
//!   button, Minimise / Maximise / Close), or on an open menu or dropdown (Help's "Search menus"
//!   field included), keeps the typed number unapplied, rewritten and selected as Tab does it.
//! * Whatever the key or click, a typed number is never applied while the window is not focused.
//! * Only digits, `/` and the system's decimal separator ([`crate::i18n::decimal_separator`]: `,` on
//!   a German Windows, `.` on an English one, in ratios too: `100/5,0`) can be typed or pasted; any
//!   other character (the other separator too) is dropped and asks for the same alert, without
//!   touching the text. Where the system's separator is not known, both `.` and `,` are taken. The
//!   field shows its zoom with that separator (`33,33`).
//! * A second separator can be typed (`80,,2`), and pasted. Enter (or a click that applies) then
//!   shows Photoshop's "Invalid numeric entry" message with the field's range and applies the number
//!   before the second separator (`80`), as Photoshop does after its OK.
//!
//! The field itself is `widgets::number_edit` (with `hold_typed`), which calls [`before`] and
//! [`after`] around the egui `DragValue`. The rest is bookkeeping other parts of the app read:
//! [`editing`] (the shortcuts must not take Esc or Tab in the frame egui drops the focus),
//! [`is_focused`] (the menu bar must not take Alt) and [`tick`] (plays the alert, and tells the
//! field about a press on the pasteboard, which only the app can recognise).

use egui::{Context, FocusDirection, Id, Key, Modifiers, SurrenderFocusOn, Ui};

fn editing_id() -> Id {
    Id::unique("pc-zoom-field-editing")
}

fn pasteboard_id() -> Id {
    Id::unique("pc-zoom-field-pasteboard-press")
}

fn menu_bar_rect_id() -> Id {
    Id::unique("pc-zoom-field-menu-bar-rect")
}

fn alert_id() -> Id {
    Id::unique("pc-zoom-field-alert")
}

fn taken_key_id() -> Id {
    Id::unique("pc-zoom-field-taken-key")
}

fn keep_pass_id() -> Id {
    Id::unique("pc-zoom-field-keep-pass")
}

fn select_until_id() -> Id {
    Id::unique("pc-zoom-field-select-until")
}

fn title_bar_rect_id() -> Id {
    Id::unique("pc-zoom-field-title-bar-rect")
}

/// What [`before`] decided for this frame, for [`after`] and the field.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Guard {
    /// The setting to put back: egui's "surrender focus on" was switched off for this frame.
    restore: Option<SurrenderFocusOn>,
    /// Esc was pressed in the field: whatever was typed is dropped, the field leaves the keyboard.
    esc: bool,
    /// Whatever egui does with the typed text this frame, it must not reach the zoom: Esc, Tab, a
    /// click that keeps the typed number, a window that has lost the keyboard.
    veto: bool,
}

impl Guard {
    /// The field must not apply a typed number this frame (a scrub is not typed and is exempt).
    pub(crate) fn vetoes_typed(self) -> bool {
        self.veto
    }
}

/// Makes the `TextEdit` of the field being drawn select all of its text, the way Ctrl+A does:
/// the key is put in this frame's input, where the `TextEdit` reads it as typed.
fn select_all_by_key(ui: &Ui) {
    let ctrl_a = egui::Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
    ui.input_mut(|i| i.events.push(ctrl_a));
}

/// Can `c` be typed into a zoom field? Digits, `/` (for a ratio: `1/2` is 50 %, `5/2` is 250 %;
/// the `%` is drawn beside the field, never typed) and the decimal separator `sep` (both `.` and `,`
/// when it is not known).
fn accepted(c: char, sep: Option<char>) -> bool {
    c.is_ascii_digit() || c == '/' || sep.map_or(matches!(c, '.' | ','), |s| c == s)
}

/// Takes out of this frame's input whatever a zoom field does not take ([`accepted`]) before the
/// field sees it; true when something was. A second separator is not refused here (Photoshop types
/// it, and refuses the number on Enter: [`read`]).
///
/// egui keeps the text being edited in a private `EditState`, out of reach, so this module keeps
/// its own copy under [`mirror_id`], for Tab and a click that keeps the number to rewrite it
/// ([`canonical`]): what the field showed when it got the keyboard, with the keys applied that are
/// seen here (typed, pasted, cut, Backspace, Delete). A key that changes the text in a way not
/// followed (undo, a word deletion) makes the copy empty and marks it lost ([`lost_id`]): the text
/// is then only selected, never rewritten, until the field is left. The copy is kept with the `shown` it started from: when the value changes under the
/// focused field (ArrowUp, the wheel, another zoom), the field shows the new number, and the copy
/// starts again from it.
fn drop_unwanted_chars(ui: &Ui, id: Id, shown: &str) -> bool {
    let mirror = mirror_id(id);
    let stored = ui.data(|d| d.get_temp::<(String, String)>(mirror)).filter(|(from, _)| from == shown).map(|(_, text)| text);
    let mut text: Vec<char> = stored.clone().unwrap_or_else(|| shown.to_owned()).chars().collect();
    let len = text.len();
    let range = egui::TextEdit::load_state(ui.ctx(), id).and_then(|s| s.cursor.char_range());
    // Before the first key the whole number is selected, as the field is reached.
    let mut sel = match (&stored, range) {
        (Some(_), Some(r)) => {
            let (a, b) = (r.primary.index.0, r.secondary.index.0);
            (a.min(b).min(len), a.max(b).min(len))
        }
        (Some(_), None) => (len, len),
        (None, _) => (0, len),
    };
    let sep = crate::i18n::decimal_separator();
    let allowed = |c: char| accepted(c, sep);
    let mut stale = false;
    let dropped = ui.input_mut(|i| {
        let mut dropped = false;
        i.events.retain_mut(|e| match e {
            egui::Event::Text(t) | egui::Event::Paste(t) => {
                let kept: String = t.chars().filter(|&c| allowed(c)).collect();
                dropped |= kept.chars().count() != t.chars().count();
                // Nothing kept: the event goes, and the text stays as it is.
                if !kept.is_empty() {
                    text.drain(sel.0..sel.1);
                    for (n, c) in kept.chars().enumerate() {
                        text.insert(sel.0 + n, c);
                    }
                    let at = sel.0 + kept.chars().count();
                    sel = (at, at);
                }
                *t = kept;
                !t.is_empty()
            }
            egui::Event::Cut => {
                text.drain(sel.0..sel.1);
                sel = (sel.0, sel.0);
                true
            }
            egui::Event::Key { key: key @ (Key::Backspace | Key::Delete), pressed: true, modifiers, .. } => {
                if modifiers.is_none() || (*key == Key::Backspace && modifiers.shift_only()) {
                    if sel.0 != sel.1 {
                        text.drain(sel.0..sel.1);
                        sel = (sel.0, sel.0);
                    } else if *key == Key::Backspace && sel.0 > 0 {
                        text.remove(sel.0 - 1);
                        sel = (sel.0 - 1, sel.0 - 1);
                    } else if *key == Key::Delete && sel.0 < text.len() {
                        text.remove(sel.0);
                    }
                } else {
                    stale = true;
                }
                true
            }
            egui::Event::Key { key: Key::Z | Key::Y, pressed: true, modifiers, .. } => {
                stale |= modifiers.command;
                true
            }
            _ => true,
        });
        dropped
    });
    if stale {
        text.clear();
        // Lost for good, until the field is left or shows another value: what is typed from here on
        // is only part of the text.
        ui.data_mut(|d| d.insert_temp(lost_id(id), shown.to_owned()));
    }
    ui.data_mut(|d| d.insert_temp(mirror, (shown.to_owned(), text.iter().collect::<String>())));
    dropped
}

/// Where this module keeps its copy of the text being edited in the field with `id`.
fn mirror_id(id: Id) -> Id {
    id.with("pc-zoom-field-text")
}

/// Where this module notes that its copy of the text lost track of it (with the `shown` it was
/// kept for), so the copy is never written back over the real text ([`rewrite`]).
fn lost_id(id: Id) -> Id {
    id.with("pc-zoom-field-text-lost")
}

/// Drops the copy of the typed text, and with it the note that it was lost.
fn forget_text(ui: &Ui, id: Id) {
    ui.data_mut(|d| {
        d.remove_temp::<(String, String)>(mirror_id(id));
        d.remove_temp::<String>(lost_id(id));
    });
}

/// Call right before the field's `DragValue` is added; `id` is that `DragValue`'s id, `shown` the
/// text it shows when nothing was typed yet, and `canon` writes a typed text as the field would
/// show the zoom Enter makes of it (`None`: it makes none).
pub(crate) fn before(ui: &Ui, id: Id, shown: &str, canon: &dyn Fn(&str) -> Option<String>) -> Guard {
    let ctx = ui.ctx();
    let pass = ctx.cumulative_pass_nr();
    let focused = ui.memory(|m| m.has_focus(id));
    // Esc: take it (no shortcut, no job cancel, no canvas acts on it), and drop what was typed.
    // Whether egui dropped the focus already (the key's own frame) or not, the field applies
    // nothing this frame (`Guard::vetoes_typed`) and leaves the keyboard (`after`).
    let owner = focused || editing_field(ctx) == Some(id);
    // Esc is also looked for in the frame's raw input: something may have consumed it before the
    // field was drawn (a shortcut, a job), and egui has dropped the focus on it already.
    let raw_esc = |ui: &Ui| {
        ui.input(|i| i.raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Escape, pressed: true, modifiers, .. } if modifiers.is_none())))
    };
    // Only the field that owns the keyboard takes Esc: any other must leave it to whoever wants it.
    if owner && (ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) | raw_esc(ui)) {
        forget_text(ui, id);
        return Guard { esc: true, veto: true, ..Default::default() };
    }
    // The focus left before the field was drawn, and no click took it (egui drops the focus on
    // some keys and a shortcut may then eat them, so the `DragValue` cannot rely on seeing them):
    // the copy of the typed text is dropped. Nothing vetoes the `DragValue` here, so a click away
    // still applies what was typed.
    if !focused {
        forget_text(ui, id);
    }
    // A click that keeps what was typed (pasteboard, title bar, open menu), or a window that has
    // lost the keyboard: whatever egui does with the typed text, it does not reach the zoom. This
    // holds without the focus too (the click may have given it to another field earlier in this
    // frame, and egui hands a field its lost focus, with the text, when it is next drawn), and for
    // two frames after the click (the focus can go in one frame and be noticed in the next).
    let gesture = pasteboard_press(ctx);
    if gesture {
        ctx.data_mut(|d| d.insert_temp(keep_pass_id(), pass));
    }
    let recent = ctx.data(|d| d.get_temp::<u64>(keep_pass_id())).is_some_and(|p| pass <= p + 2);
    // Enter in the field is a commit of its own, however soon after the click: egui draws few frames
    // with the mouse still, so the Enter can land in that window.
    let enter = focused && ui.input(|i| i.key_pressed(Key::Enter));
    let veto = (recent && !enter) || ui.input(|i| i.viewport().focused) == Some(false);
    if !focused {
        return Guard { veto, ..Default::default() };
    }
    drop_taken_text(ctx);
    if drop_unwanted_chars(ui, id, shown) {
        request_alert(ctx);
    }
    // Tab: egui has already planned the move to the next widget; cancel it. The text is selected
    // now, and again in the next frames until something is typed: whatever moves the text cursor
    // meanwhile does not undo it.
    let tab = ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab) | i.consume_key(Modifiers::NONE, Key::Tab));
    if tab {
        ctx.memory_mut(|m| m.move_focus(FocusDirection::None));
        request_alert(ctx);
        rewrite(ui, id, shown, canon);
        select_all_by_key(ui);
        ctx.data_mut(|d| d.insert_temp(select_until_id(), pass + 3));
        ctx.request_repaint();
        return Guard { veto: true, ..Default::default() };
    }
    if let Some(until) = ctx.data(|d| d.get_temp::<u64>(select_until_id())) {
        let acted = ui.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Key { pressed: true, .. } | egui::Event::PointerButton { pressed: true, .. }
                )
            })
        });
        if acted || pass > until {
            ctx.data_mut(|d| d.remove_temp::<u64>(select_until_id()));
        } else {
            select_all_by_key(ui);
            ctx.request_repaint();
        }
    }
    if gesture {
        // egui drops the focus of the focused widget when a click lands elsewhere, and the
        // `DragValue` then applies the text; for this frame, neither.
        let prev = ctx.options(|o| o.input_options.surrender_focus_on);
        ctx.options_mut(|o| o.input_options.surrender_focus_on = SurrenderFocusOn::Never);
        // Selected when the button goes down and when it comes up, not on every frame it is held.
        if ui.input(|i| i.pointer.any_pressed() || i.pointer.any_released()) {
            rewrite(ui, id, shown, canon);
            select_all_by_key(ui);
        }
        return Guard { restore: Some(prev), veto: true, ..Default::default() };
    }
    Guard { veto, ..Default::default() }
}

/// Writes the typed text as the field shows its zoom ([`canonical`]), as Photoshop does on Tab and
/// on a click that keeps it: all of it is selected and replaced in this frame's input.
fn rewrite(ui: &Ui, id: Id, shown: &str, canon: &dyn Fn(&str) -> Option<String>) {
    // The copy lost track of the text (an undo, a word deletion): the text is only selected.
    if ui.data(|d| d.get_temp::<String>(lost_id(id))).is_some_and(|from| from == shown) {
        return;
    }
    let mirror = mirror_id(id);
    let text = ui.data(|d| d.get_temp::<(String, String)>(mirror)).filter(|(from, _)| from == shown).map_or_else(|| shown.to_owned(), |(_, t)| t);
    let Some(new) = canonical(&text, canon) else { return };
    select_all_by_key(ui);
    ui.input_mut(|i| i.events.push(egui::Event::Text(new.clone())));
    ui.data_mut(|d| d.insert_temp(mirror, (shown.to_owned(), new)));
}

/// What [`rewrite`] puts in place of `text`, when that changes it: `canon` of it, for text that
/// holds a number Enter takes as it is. Text Enter refuses ([`read`]) stays.
fn canonical(text: &str, canon: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let Entry::Number(_) = read(text) else { return None };
    canon(text).filter(|new| new != text && !new.is_empty())
}

/// Call right after the field's `DragValue` was added.
pub(crate) fn after(ui: &Ui, id: Id, guard: Guard) {
    let ctx = ui.ctx();
    if let Some(prev) = guard.restore {
        ctx.options_mut(|o| o.input_options.surrender_focus_on = prev);
    }
    if guard.esc {
        // Esc: the typed text goes, and so does the keyboard, so the field shows the zoom again.
        forget_text(ui, id);
        ui.memory_mut(|m| m.surrender_focus(id));
        ctx.request_repaint();
        return;
    }
    if !ui.memory(|m| m.has_focus(id)) {
        return;
    }
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(editing_id(), (id, pass)));
}

/// How Enter reads the text of a zoom field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    /// At most one decimal separator: this text (`,` made `.`) for the number parser.
    Number(String),
    /// More than one (`80,,2`, `1,5,2`): an invalid numeric entry. Photoshop says so and then takes
    /// the number before the second separator: this text for the parser (`80,` as `80.`), if any.
    Invalid(Option<String>),
}

/// Reads a zoom field's text for Enter: a decimal separator is `,` or `.` (only the system's one
/// can be typed, [`accepted`]), and the text holds at most one.
pub(crate) fn read(text: &str) -> Entry {
    let is_sep = |c: char| matches!(c, '.' | ',');
    let Some((second, _)) = text.char_indices().filter(|&(_, c)| is_sep(c)).nth(1) else {
        return Entry::Number(text.replace(',', "."));
    };
    let lead = text.get(..second).unwrap_or_default();
    Entry::Invalid((!lead.is_empty()).then(|| lead.replace(',', ".")))
}

/// `text` (a number formatted with `.`) with the system's decimal separator, as the field shows it.
pub(crate) fn localize(text: String) -> String {
    match crate::i18n::decimal_separator() {
        Some(',') => text.replace('.', ","),
        _ => text,
    }
}

fn invalid_entry_id() -> Id {
    Id::unique("pc-zoom-field-invalid-entry")
}

/// The field with `id` read an invalid entry ([`Entry::Invalid`]) this frame; see [`take_invalid`].
pub(crate) fn note_invalid(ctx: &Context, id: Id) {
    ctx.data_mut(|d| d.insert_temp(invalid_entry_id().with(id), true));
}

/// Did the field with `id` read an invalid entry this frame ([`note_invalid`])? Taken once.
pub(crate) fn take_invalid(ctx: &Context, id: Id) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(invalid_entry_id().with(id))).unwrap_or(false)
}

/// Photoshop's "Invalid numeric entry" message for a field whose range is `range` (in %): [`tick`]
/// opens it in the next frame.
pub(crate) fn request_invalid_entry(ctx: &Context, range: (f32, f32)) {
    ctx.data_mut(|d| d.insert_temp(invalid_entry_id(), range));
    ctx.request_repaint();
}

/// The message waiting to be shown, with its range (tests).
#[cfg(test)]
pub(crate) fn invalid_entry_pending(ctx: &Context) -> Option<(f32, f32)> {
    ctx.data(|d| d.get_temp::<(f32, f32)>(invalid_entry_id()))
}

/// The English text of the message; `{min}` and `{max}` are the range, with two decimals.
pub const INVALID_ENTRY: &str = "Invalid numeric entry. An integer between {min}% and {max}% is required. Previous value was inserted.";

/// The message for a range, in the UI language and with the system's decimal separator.
pub fn invalid_entry_message((min, max): (f32, f32)) -> String {
    let pct = |v: f32| localize(format!("{v:.2}"));
    crate::i18n::fmt(tl!(INVALID_ENTRY), &[("min", &pct(min)), ("max", &pct(max))])
}

fn editing_field(ctx: &Context) -> Option<Id> {
    let (id, pass) = ctx.data(|d| d.get_temp::<(Id, u64)>(editing_id()))?;
    // Drawn with the focus in the last frame (the one before this): in the frame egui drops
    // the focus for Esc or Tab, nothing is focused any more, but the field is still the owner.
    (pass + 1 >= ctx.cumulative_pass_nr()).then_some(id)
}

/// A zoom field had the keyboard in the last frame. Like a layer rename, it keeps its keys even in
/// the frame egui drops its focus (Esc, Tab), so the shortcuts must treat it as text.
pub fn editing(ctx: &Context) -> bool {
    editing_field(ctx).is_some()
}

/// A zoom field has the keyboard right now.
pub fn is_focused(ctx: &Context) -> bool {
    editing_field(ctx).is_some_and(|id| ctx.memory(|m| m.focused()) == Some(id))
}

/// A key the open menu took (a row's letter, Space) also arrives as typed text, in the same frame
/// or the next: while a zoom field has the keyboard, that text never reaches it, so the field
/// neither types it nor alerts.
pub(crate) fn drop_text(ctx: &Context, c: char) {
    if is_focused(ctx) {
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|d| d.insert_temp(taken_key_id(), (c, pass)));
        drop_taken_text(ctx);
    }
}

/// Takes the text of the key [`drop_text`] noted out of this frame's input, once, until the frame
/// after the key.
fn drop_taken_text(ctx: &Context) {
    let Some((c, pass)) = ctx.data(|d| d.get_temp::<(char, u64)>(taken_key_id())) else { return };
    if pass + 1 < ctx.cumulative_pass_nr() {
        ctx.data_mut(|d| d.remove_temp::<(char, u64)>(taken_key_id()));
        return;
    }
    let same = |t: &str| t.chars().eq(c.to_lowercase()) || t.chars().eq(c.to_uppercase());
    let found = ctx.input_mut(|i| {
        let at = i.events.iter().position(|e| matches!(e, egui::Event::Text(t) if same(t)));
        at.map(|at| i.events.remove(at)).is_some()
    });
    if found {
        ctx.data_mut(|d| d.remove_temp::<(char, u64)>(taken_key_id()));
    }
}

/// The system alert (Windows: Asterisk) is wanted; [`tick`] plays it in the next frame.
pub fn request_alert(ctx: &Context) {
    ctx.data_mut(|d| d.insert_temp(alert_id(), true));
    ctx.request_repaint();
}

/// Whether the alert is waiting to be played (tests).
#[cfg(test)]
pub(crate) fn alert_pending(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(alert_id())).unwrap_or(false)
}

/// Remembers the rectangle of the whole title bar (menus, the empty drag area, workspace switcher,
/// search and theme buttons, the caption buttons) for [`on_title_bar`].
pub(crate) fn set_title_bar_rect(ctx: &Context, rect: egui::Rect) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(title_bar_rect_id(), (rect, pass)));
}

/// Remembers the rectangle of the menu bar titles (`File` … `Help`) for [`on_menu_bar`].
pub(crate) fn set_menu_bar_rect(ctx: &Context, rect: egui::Rect) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(menu_bar_rect_id(), (rect, pass)));
}

/// Does the rectangle kept under `id` (with the pass it was drawn in) contain `pos`, and was it drawn
/// recently enough to still be there? Not drawn for a while (hidden by Tab or Full Screen Mode), its
/// rect is stale and must not catch a click. The one freshness rule of the title bar and the menu bar.
fn fresh_rect_contains(ctx: &Context, id: Id, pos: egui::Pos2) -> bool {
    ctx.data(|d| d.get_temp::<(egui::Rect, u64)>(id)).is_some_and(|(r, pass)| pass + 2 >= ctx.cumulative_pass_nr() && r.contains(pos))
}

/// A button went down on the pasteboard while a zoom field was being edited, and is still down or
/// came up in this very frame. (egui drops the focus of a text field on the click, which is the
/// release, so the press alone is not the whole gesture.)
fn pasteboard_press(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(pasteboard_id())).unwrap_or(false)
}

/// Once per frame ([`tick`], tests): `pressed` says a button went down on the pasteboard in this
/// frame. The gesture lasts until the frame after the button came up.
pub(crate) fn track_pasteboard(ctx: &Context, pressed: bool) {
    let (down, released) = ctx.input(|i| (i.pointer.any_down(), i.pointer.any_released()));
    ctx.data_mut(|d| {
        if pressed {
            d.insert_temp(pasteboard_id(), true);
        } else if !down && !released {
            d.remove::<bool>(pasteboard_id());
        }
    });
}

// ---- What needs the application: services and the document's geometry. ----

/// Once per frame, before the panels: shows a requested "Invalid numeric entry" message, plays a
/// requested alert and follows a press on the pasteboard.
pub fn tick(app: &mut crate::PhotocraftApp, ctx: &Context) {
    if let Some(range) = ctx.data_mut(|d| d.remove_temp::<(f32, f32)>(invalid_entry_id())) {
        let message = invalid_entry_message(range);
        // One commit can be read in more than one pass of a frame: one message, not a stack of them.
        let open =
            app.ui.dialogs.iter().any(|d| d.kind == crate::state::DialogKind::Error && d.fields.get("message").and_then(|m| m.as_str()) == Some(&message));
        if !open {
            app.ui.open_dialog(crate::state::DialogKind::Error, serde_json::json!({ "message": message }).as_object().cloned().unwrap_or_default());
        }
    }
    if ctx.data_mut(|d| d.remove_temp::<bool>(alert_id())).unwrap_or(false)
        // The platform's alert, else its plain beep: an app that only sets `beep` still answers.
        && let Some(play) = app.services.alert.or(app.services.beep)
    {
        play();
    }
    let press =
        if editing(ctx) { ctx.input(|i| if i.pointer.any_pressed() { i.pointer.press_origin().or(i.pointer.interact_pos()) } else { None }) } else { None };
    track_pasteboard(ctx, press.is_some_and(|pos| on_pasteboard(app, ctx, pos) || on_menu_bar(ctx, pos) || on_title_bar(ctx, pos) || on_open_popup(ctx, pos)));
}

/// Is screen point `pos` anywhere on the title bar: the menu titles and the gaps between them, the
/// workspace switcher (Essentials), search, the theme button, Minimise / Maximise / Close?
fn on_title_bar(ctx: &Context, pos: egui::Pos2) -> bool {
    fresh_rect_contains(ctx, title_bar_rect_id(), pos)
}

/// Is screen point `pos` on an open menu or dropdown (Help's "Search menus" field, a menu item,
/// the workspace list)? A menu or dropdown is open and `pos` is on a layer above the canvas.
fn on_open_popup(ctx: &Context, pos: egui::Pos2) -> bool {
    (egui::Popup::is_any_open(ctx) || crate::menu_nav::is_open(ctx)) && ctx.layer_id_at(pos).is_some_and(|l| l.order != egui::Order::Background)
}

/// Is screen point `pos` on one of the menu bar's top-level menu titles (`File`, `Edit`, …)?
fn on_menu_bar(ctx: &Context, pos: egui::Pos2) -> bool {
    fresh_rect_contains(ctx, menu_bar_rect_id(), pos)
}

/// Is screen point `pos` on the main canvas, outside the image, and not under a window or menu?
fn on_pasteboard(app: &crate::PhotocraftApp, ctx: &Context, pos: egui::Pos2) -> bool {
    let (Some(xf), Some(st)) = (crate::canvas::ViewXform::active(app), app.session.active()) else { return false };
    let doc = xf.to_doc(pos);
    let (w, h) = (f64::from(st.doc.size.width), f64::from(st.doc.size.height));
    xf.rect.contains(pos)
        && doc.iter().all(|v| v.is_finite())
        && (doc[0] < 0.0 || doc[1] < 0.0 || doc[0] >= w || doc[1] >= h)
        && ctx.layer_id_at(pos).is_some_and(|l| l.order == egui::Order::Background)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_reads_one_separator_and_the_number_before_a_second_one() {
        let number = |t: &str| Entry::Number(t.into());
        assert_eq!(read("100,90"), number("100.90"));
        assert_eq!(read("100.90"), number("100.90"));
        assert_eq!(read("100/5,0"), number("100/5.0"));
        assert_eq!(read("100/5.0"), number("100/5.0"));
        assert_eq!(read("45"), number("45"));
        assert_eq!(read(""), number(""));
        let invalid = |t: Option<&str>| Entry::Invalid(t.map(str::to_owned));
        assert_eq!(read("80,,2"), invalid(Some("80.")));
        assert_eq!(read("1,5,2"), invalid(Some("1.5")));
        assert_eq!(read("1,000.5"), invalid(Some("1.000")));
        assert_eq!(read("100/5,0,1"), invalid(Some("100/5.0")));
        assert_eq!(read(",,"), invalid(Some(".")));
        assert_eq!(read("..5"), invalid(Some(".")));
    }

    #[test]
    fn only_the_systems_decimal_separator_can_be_typed() {
        for (sep, point, comma) in [(Some(','), false, true), (Some('.'), true, false), (None, true, true)] {
            assert_eq!(accepted('.', sep), point, "{sep:?}");
            assert_eq!(accepted(',', sep), comma, "{sep:?}");
            assert!(accepted('7', sep) && accepted('/', sep) && !accepted('%', sep) && !accepted('a', sep));
        }
    }

    #[test]
    fn the_invalid_entry_message_names_the_range_with_the_systems_separator() {
        let want = |s: &str| format!("Invalid numeric entry. An integer between {s} is required. Previous value was inserted.");
        crate::i18n::with_decimal_separator(Some(','), || {
            assert_eq!(invalid_entry_message((0.25, 12800.0)), want("0,25% and 12800,00%"));
            assert_eq!(localize("33.33".into()), "33,33");
        });
        crate::i18n::with_decimal_separator(Some('.'), || assert_eq!(invalid_entry_message((0.25, 12800.0)), want("0.25% and 12800.00%")));
        assert_eq!(localize("33.33".into()), "33.33", "unknown: as formatted");
    }

    #[test]
    fn the_title_bar_keeps_a_click_wherever_it_is_drawn() {
        let ctx = Context::default();
        let bar = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 32.0));
        assert!(!on_title_bar(&ctx, egui::pos2(10.0, 10.0)), "nothing drawn yet");
        set_title_bar_rect(&ctx, bar);
        assert!(on_title_bar(&ctx, egui::pos2(790.0, 10.0)), "the far end: Essentials, Minimise, Close");
        assert!(on_title_bar(&ctx, egui::pos2(400.0, 10.0)), "the empty drag area between the menus and them");
        assert!(!on_title_bar(&ctx, egui::pos2(10.0, 100.0)), "below the bar is the app");
    }

    /// The menu bar rect goes stale like the title bar's: not drawn for a while, it catches nothing.
    #[test]
    fn a_menu_bar_that_is_no_longer_drawn_keeps_no_click() {
        let ctx = Context::default();
        let bar = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 32.0));
        let inside = egui::pos2(50.0, 10.0);
        assert!(!on_menu_bar(&ctx, inside), "nothing drawn yet");
        // Drawn in the pass the clock is at now.
        set_menu_bar_rect(&ctx, bar);
        assert!(on_menu_bar(&ctx, inside), "just drawn");
        assert!(!on_menu_bar(&ctx, egui::pos2(500.0, 10.0)), "outside the titles");
        // A few passes later, with the menu bar not drawn again (hidden): the rect is stale.
        for _ in 0..3 {
            // The output is dropped: its texture deltas must be cleared first, or egui panics.
            ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        }
        assert!(!on_menu_bar(&ctx, inside), "stale: the bar is no longer drawn");
        // Drawn again: fresh again.
        set_menu_bar_rect(&ctx, bar);
        assert!(on_menu_bar(&ctx, inside), "drawn again");
    }
}
