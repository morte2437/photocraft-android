use super::ime::interrupt_incoming;
use super::ime_edit::note_ime_application_change;
use super::layout::{hit_layer, hit_offset, shows_own_layout};
use super::{
    Arc, FontFieldEdit, HashMap, LayerId, PhotocraftApp, TextEdit, TextLayer, accept_row, active_doc_id, apply, byte_range, current_text, families,
    font_labels_id, font_menu_rows, insertion_style, json, selected_range, session_key, text_layer,
};

/// Start editing an existing type layer. Like Photoshop, editing shows the text as the type
/// engine lays it out, so a PSD layer is re-rendered first (inside the edit session's history
/// step, so Cancel brings Photoshop's pixels back).
pub(super) fn begin_edit(app: &mut PhotocraftApp, id: LayerId, key: &str) -> Result<(), String> {
    let st = app.session.active().ok_or("no document open")?;
    let doc = st.doc.clone();
    if let Some(t) = text_layer(&doc, id)
        && !shows_own_layout(&doc, t)
    {
        app.run("type.edit", json!({"layer": id.0, "coalesce": key}))?;
    }
    Ok(())
}

/// Edit the active type layer from its thumbnail, selecting all text like Photoshop. Reuse the
/// current session when already editing it, so Cancel and undo still cover the whole edit.
pub fn edit_active(app: &mut PhotocraftApp) -> Result<(), String> {
    app.ui.type_insertion_style = None;
    let st = app.session.active().ok_or("no document open")?;
    let id = st.active_layer.ok_or("no active layer")?;
    let n = text_layer(&st.doc, id).ok_or("active layer is not a type layer")?.text.chars().count();
    if app.ui.text_edit.as_ref().is_none_or(|ed| ed.layer != id.0) {
        commit(app);
        let key = session_key(app);
        begin_edit(app, id, &key)?;
        app.ui.text_edit = Some(TextEdit {
            layer: id.0,
            document: active_doc_id(app),
            caret: n,
            anchor: 0,
            session: key,
            created: false,
            mask: false,
            dragging: false,
            resize: None,
            preedit: None,
        });
    } else {
        let mut cancelled = false;
        if let Some(ed) = app.ui.text_edit.as_mut() {
            ed.anchor = 0;
            ed.caret = n;
            // Re-entering the same session moves the caret away from the composition: cancel it
            // rather than reusing a range that no longer describes the text.
            cancelled = ed.preedit.take().is_some();
        }
        if cancelled {
            app.type_ime.clear_preedit();
            interrupt_incoming(app);
            app.type_ime.request_interrupt();
        }
    }
    let vertical = app.session.active().and_then(|st| text_layer(&st.doc, id)).is_some_and(|t| t.orientation == photocraft_doc::text::Orientation::Vertical);
    app.ui.text_edit_return_tool = None;
    app.ui.tool = if vertical { crate::state::Tool::VerticalType } else { crate::state::Tool::Type };
    app.ui.mask_target = false;
    app.ui.vector_mask_target = false;
    Ok(())
}

/// Double-click with the Move tool (Photoshop CC 2019+): edit the type layer under document
/// point (`x`, `y`) with the caret on the glyph there, switching to the Type tool. False when no
/// type is there.
pub fn edit_at(app: &mut PhotocraftApp, x: f64, y: f64) -> bool {
    let Some(id) = hit_layer(app, x, y) else { return false };
    let previous_tool = app.ui.tool;
    commit(app);
    if app.session.select_layer(id).is_err() {
        return false;
    }
    let key = session_key(app);
    if begin_edit(app, id, &key).is_err() {
        return false;
    }
    let off = hit_offset(app, id, x, y);
    app.ui.text_edit = Some(TextEdit {
        layer: id.0,
        document: active_doc_id(app),
        caret: off,
        anchor: off,
        session: key,
        created: false,
        mask: false,
        dragging: false,
        resize: None,
        preedit: None,
    });
    let vertical = app.session.active().and_then(|st| text_layer(&st.doc, id)).is_some_and(|t| t.orientation == photocraft_doc::text::Orientation::Vertical);
    if !previous_tool.is_type() {
        app.ui.text_edit_return_tool = Some(previous_tool);
    }
    app.ui.tool = if vertical { crate::state::Tool::VerticalType } else { crate::state::Tool::Type };
    app.ui.mask_target = false;
    app.ui.vector_mask_target = false;
    true
}

/// Copy the selected characters to the system clipboard. Returns the copied range. Shared by the
/// clipboard Copy event and Edit > Copy during a type edit.
pub(crate) fn copy_selection(app: &PhotocraftApp, ctx: &egui::Context) -> Option<[usize; 2]> {
    let [a, b] = selected_range(app)?;
    let ed = app.ui.text_edit.as_ref()?;
    let text = text_layer(&app.session.active()?.doc, LayerId(ed.layer))?;
    ctx.copy_text(text.text.chars().skip(a).take(b - a).collect());
    Some([a, b])
}

/// Edit > Paste during a type edit: ask the window for the clipboard text, which arrives as a
/// Paste event next frame and replaces the selection there (like ⌘V).
pub(crate) fn request_paste(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
}

/// Select > All during a type edit selects every character (like ⌘A).
pub(crate) fn select_all(app: &mut PhotocraftApp) {
    let Some(n) = app.ui.text_edit.as_ref().and_then(|ed| current_text(app, LayerId(ed.layer))).map(|t| t.chars().count()) else { return };
    if let Some(e) = app.ui.text_edit.as_mut() {
        (e.anchor, e.caret) = (0, n);
    }
}

/// Shared by the clipboard Cut event and Edit > Cut during a type edit.
pub(crate) fn cut_selection(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some([a, b]) = copy_selection(app, ctx) else { return };
    if let Some(ed) = app.ui.text_edit.as_mut() {
        ed.anchor = a;
        ed.caret = b;
    }
    insert(app, "");
}

/// Replace the selection with `s`.
pub(super) fn insert(app: &mut PhotocraftApp, s: &str) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    let (a, b) = (ed.caret.min(ed.anchor), ed.caret.max(ed.anchor));
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    if a == b && s.is_empty() {
        return;
    }
    crate::type_transform::finish(app);
    let applied = insertion_style::replace(app, a, b, &s);
    if applied {
        let mut cancelled = false;
        if let Some(e) = app.ui.text_edit.as_mut() {
            e.caret = a + s.chars().count();
            e.anchor = e.caret;
            // An application edit invalidates a live composition: its recorded range no longer
            // describes the text, and its pending native commit must not be applied here.
            cancelled = e.preedit.take().is_some();
        }
        if cancelled {
            app.type_ime.clear_preedit();
            interrupt_incoming(app);
            app.type_ime.request_interrupt();
        }
        // Ordinary typing, paste, cut and deletion are application-origin, never IME receipts.
        note_ime_application_change(app, &ed);
    }
}

/// Typed text (not a paste): with Preferences ▸ Type ▸ Use Smart Quotes, straight quotes become
/// typographic (Photoshop does not rewrite pasted text).
pub(super) fn insert_typed(app: &mut PhotocraftApp, s: &str) {
    if !app.session.prefs().type_.smart_quotes {
        return insert(app, s);
    }
    let prev = app.ui.text_edit.clone().and_then(|ed| {
        let at = ed.caret.min(ed.anchor);
        let text = current_text(app, LayerId(ed.layer))?;
        (at > 0).then(|| text.chars().nth(at - 1)).flatten()
    });
    insert(app, &smart_quotes(s, prev));
}

/// Straight quotes become “ ” and ‘ ’: opening after whitespace, a bracket or a dash (or at the
/// start of the text), closing next to a letter or another quote. `prev` is the character before
/// the caret.
pub(super) fn smart_quotes(s: &str, mut prev: Option<char>) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let c = match c {
            '"' => {
                if opens_quote(prev) {
                    '\u{201c}'
                } else {
                    '\u{201d}'
                }
            }
            '\'' => {
                if opens_quote(prev) {
                    '\u{2018}'
                } else {
                    '\u{2019}'
                }
            }
            _ => c,
        };
        out.push(c);
        prev = Some(c);
    }
    out
}

fn opens_quote(prev: Option<char>) -> bool {
    prev.is_none_or(|p| p.is_whitespace() || matches!(p, '(' | '[' | '{' | '-' | '\u{2013}' | '\u{2014}'))
}

/// Alt+←/→: kern the pair before the caret by `by` (1/1000 em). No pair (caret at a text or line
/// edge): nothing happens, like Photoshop. Not coalesced: one history step per press.
pub(super) fn kern_pair(app: &mut PhotocraftApp, id: LayerId, caret: usize, by: f32) {
    let Some(text) = current_text(app, id) else { return };
    let before = caret.checked_sub(1).and_then(|i| text.chars().nth(i));
    let after = text.chars().nth(caret);
    if before.is_none_or(|c| c == '\n') || after.is_none_or(|c| c == '\n') {
        return;
    }
    crate::type_transform::finish(app);
    let _ = app.run("type.edit", json!({"layer": id.0, "kernPair": {"at": caret, "by": by}}));
}

/// A character style the type editor's ⇧⌘ keys toggle (Photoshop: ⇧⌘K All Caps, ⇧⌘U
/// Underline, ⇧⌘B Faux Bold, ⇧⌘I Faux Italic, ⇧⌘/ Strikethrough).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CharToggle {
    AllCaps,
    Underline,
    FauxBold,
    FauxItalic,
    Strikethrough,
}

impl CharToggle {
    fn is_on(self, st: &photocraft_doc::text::CharStyle) -> bool {
        match self {
            Self::AllCaps => st.caps == photocraft_doc::text::Caps::AllCaps,
            Self::Underline => st.underline,
            Self::FauxBold => st.faux_bold,
            Self::FauxItalic => st.faux_italic,
            Self::Strikethrough => st.strikethrough,
        }
    }

    fn param(self, on: bool) -> (&'static str, serde_json::Value) {
        match self {
            Self::AllCaps => ("caps", json!(if on { "all" } else { "normal" })),
            Self::Underline => ("underline", json!(on)),
            Self::FauxBold => ("fauxBold", json!(on)),
            Self::FauxItalic => ("fauxItalic", json!(on)),
            Self::Strikethrough => ("strikethrough", json!(on)),
        }
    }
}

/// The character styles of the selection, or of the typing style when the text is empty. Empty
/// when nothing is selected in non-empty text.
pub(super) fn selected_styles(t: &TextLayer, a: usize, b: usize) -> Vec<photocraft_doc::text::CharStyle> {
    let runs = t.char_runs();
    if t.text.is_empty() {
        return runs.into_iter().take(1).map(|r| r.style).collect();
    }
    let (ba, bb) = byte_range(&t.text, a, b);
    let mut at = 0usize;
    let mut out = Vec::new();
    for r in runs {
        let (rs, re) = (at, at.saturating_add(r.len));
        at = re;
        if rs < bb && re > ba {
            out.push(r.style);
        }
    }
    out
}

/// The character range the type editor's formatting keys act on: the selection, or the empty
/// text's typing style. `None` for a collapsed caret in non-empty text: Photoshop would style the
/// next characters typed, which the document model can't hold apart from the text, so the keys
/// do nothing there (rather than restyle a word the user didn't select).
fn format_range(app: &PhotocraftApp, ed: &TextEdit) -> Option<(usize, usize)> {
    let t = text_layer(&app.session.active()?.doc, LayerId(ed.layer))?;
    let n = t.text.chars().count();
    let (a, b) = (ed.caret.min(ed.anchor).min(n), ed.caret.max(ed.anchor).min(n));
    (a < b || n == 0).then_some((a, b))
}

/// One `type.setStyle` step on the edited layer's `range`, kept apart from the typing session's
/// coalesced edit so Undo takes back one key press at a time (as with kerning).
fn set_style(app: &mut PhotocraftApp, ed: &TextEdit, range: (usize, usize), mut p: serde_json::Value) {
    crate::type_transform::finish(app);
    p["layer"] = json!(ed.layer);
    p["range"] = json!([range.0, range.1]);
    if let Err(e) = app.run("type.setStyle", p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// ⇧⌘K/U/B/I/: flips a character style over the selection: off when every selected character
/// already has it, else on.
pub(crate) fn toggle_char_style(app: &mut PhotocraftApp, which: CharToggle) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    let Some(range) = format_range(app, &ed) else { return };
    let Some(styles) = app.session.active().and_then(|s| text_layer(&s.doc, LayerId(ed.layer))).map(|t| selected_styles(t, range.0, range.1)) else {
        return;
    };
    let on = !styles.iter().all(|st| which.is_on(st));
    let (key, value) = which.param(on);
    set_style(app, &ed, range, json!({ key: value }));
}

/// ⌥↑/↓ leading, ⇧⌥↑/↓ baseline shift and ⌥←/→ tracking: steps every selected character from its
/// own value (`key` is `leadingBy`, `baselineShiftBy` or `trackingBy`).
pub(crate) fn step_char_metric(app: &mut PhotocraftApp, key: &str, by: f32) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    let Some(range) = format_range(app, &ed) else { return };
    // Leading steps in the points the Character panel shows for a scaled layer.
    set_style(app, &ed, range, json!({ key: by, "metricsAsShown": key == "leadingBy" }));
}

/// ⌥⇧⌘H: flips Hyphenate for the paragraphs the selection (or the caret) is in: off when all of
/// them hyphenate, else on.
pub(crate) fn toggle_hyphenate(app: &mut PhotocraftApp) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    let Some(t) = app.session.active().and_then(|s| text_layer(&s.doc, LayerId(ed.layer))) else { return };
    let n = t.text.chars().count();
    let (a, b) = (ed.caret.min(ed.anchor).min(n), ed.caret.max(ed.anchor).min(n));
    let (ba, bb) = byte_range(&t.text, a, b);
    let len = t.text.len();
    let mut at = 0usize;
    let mut all = true;
    let mut any = false;
    for r in t.paragraph_runs() {
        let (ps, pe) = (at, at.saturating_add(r.len));
        at = pe;
        let touches = if ba == bb { ps <= ba && (ba < pe || pe >= len) } else { ps < bb && pe > ba };
        if touches {
            any = true;
            all &= r.style.hyphenate;
        }
    }
    let on = !(any && all);
    set_style(app, &ed, (a, b), json!({ "hyphenate": on }));
}

/// End the editing session. A new layer left empty is deleted; a new layer is named after its text.
pub fn commit(app: &mut PhotocraftApp) {
    commit_with(app, egui::Modifiers::NONE);
}

/// The document's commit shortcut also works while a Type field owns keyboard input.
/// Resolve a pending font query before ending the edit; composition still owns its keys.
pub(crate) fn commit_shortcut(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    let Some(ed) = app.ui.text_edit.as_ref() else { return false };
    let composing = ed.preedit.is_some();
    let mods = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, .. } if modifiers.command => Some(*modifiers),
            _ => None,
        })
    });
    let Some(mods) = mods else { return false };
    let focused = ctx.memory(|m| m.focused());
    let font_edit = focused.and_then(|id| ctx.data(|d| d.get_temp::<FontFieldEdit>(id.with("edit"))));
    ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { key: egui::Key::Enter, modifiers, .. } if modifiers.command)));
    if composing || font_edit.as_ref().is_some_and(|e| e.composing) || ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Ime(_)))) {
        // Suppress the document/path shortcut, but leave IME events for their focused editor.
        return false;
    }
    end_if_left(app);
    if app.ui.text_edit.is_none() {
        return true;
    }
    if let Some(edit) = font_edit.filter(|e| !e.accepted && !e.text.trim().is_empty() && !e.query.is_empty()) {
        let families = families();
        let labels: Option<Arc<HashMap<String, String>>> = ctx.data(|d| d.get_temp(font_labels_id()));
        let (rows, _) = font_menu_rows(&families, labels.as_deref(), &edit.query, &edit.recents);
        let selected =
            edit.row.filter(|&i| rows.get(i).is_some_and(|(f, _)| **f == edit.current)).or_else(|| rows.iter().position(|(f, _)| **f == edit.current));
        let family = accept_row(&rows, &edit.text, selected).and_then(|i| rows.get(i)).map(|(f, _)| (*f).clone());
        if let Some(family) = family {
            photocraft_text::served::request(&family);
            app.note_recent_font(&family);
            app.ui.tool_options.type_font = family.clone();
            apply(app, ctx, json!({"font": family}));
        }
    }
    if let Some(field) = focused {
        ctx.memory_mut(|m| m.surrender_focus(field));
        ctx.data_mut(|d| d.remove::<FontFieldEdit>(field.with("edit")));
    }
    egui::Popup::close_all(ctx);
    ctx.data_mut(|d| {
        d.remove::<String>(egui::Id::unique("type-font-hover"));
        d.remove::<(String, f64)>(egui::Id::unique("type-font-hover-since"));
    });
    crate::type_font_preview::clear(app);
    commit_with(app, mods);
    true
}

/// [`commit`] with the modifiers held while committing: a Type Mask adds its glyphs to the
/// selection with ⇧, subtracts them with ⌥ and intersects with ⇧⌥ (Photoshop); with none it
/// replaces the selection.
pub fn commit_with(app: &mut PhotocraftApp, mods: egui::Modifiers) {
    app.ui.type_insertion_style = None;
    crate::type_font_preview::clear(app);
    let current = app.session.active_index();
    let Some(home) = app.ui.text_edit.as_ref().map(|ed| edit_home(app, ed)) else {
        return commit_here(app, mods);
    };
    match home {
        Some(home) if Some(home) != current => {
            // An edit left in a background document commits there, in its own history (#6483).
            app.session.set_active(home);
            commit_here(app, mods);
            if let Some(i) = current {
                app.session.set_active(i);
            }
        }
        Some(_) => commit_here(app, mods),
        // Its document closed: there is nothing to commit into, and the layer id may name a
        // layer of another document.
        None => {
            crate::type_transform::reset(app);
            app.ui.text_edit = None;
            app.ui.text_edit_return_tool = None;
            app.type_ime.clear_preedit();
        }
    }
}

/// Index of the document an edit belongs to; `None` once it is closed.
pub(super) fn edit_home(app: &PhotocraftApp, ed: &TextEdit) -> Option<usize> {
    match ed.document {
        Some(d) => app.session.documents().iter().position(|st| st.doc.id.0 == d),
        None => app.session.active_index(),
    }
}

/// [`commit_with`] in the active document, which is the edit's own.
fn commit_here(app: &mut PhotocraftApp, mods: egui::Modifiers) {
    if app.ui.tool.is_type() {
        crate::type_transform::finish(app);
    }
    crate::type_transform::reset(app);
    let Some(ed) = app.ui.text_edit.take() else { return };
    app.type_ime.clear_preedit();
    if let Some(tool) = app.ui.text_edit_return_tool.take()
        && app.ui.tool.is_type()
    {
        app.ui.tool = tool;
    }
    let Some(text) = current_text(app, LayerId(ed.layer)) else { return };
    if ed.mask {
        // Type Mask: the text's glyphs become (or combine with) the selection and the working
        // layer goes, all in the session's one history step. No text: nothing changes.
        let operation = crate::tool_feedback::selection_mode(crate::state::Tool::HorizontalTypeMask, 0, mods);
        if !text.trim().is_empty()
            && let Err(e) = app.run("type.createSelection", json!({"layer": ed.layer, "removeLayer": true, "operation": operation, "coalesce": ed.session}))
        {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        if current_text(app, LayerId(ed.layer)).is_some() {
            undo_session(app, &ed);
        }
        return;
    }
    if text.trim().is_empty() && ed.created {
        let _ = app.run("layer.delete", json!({"layer": ed.layer}));
    } else if ed.created {
        let name = photocraft_engine::type_cmds::layer_name(&text);
        let _ = app.run("type.edit", json!({"layer": ed.layer, "name": name, "coalesce": ed.session}));
    }
}

/// Commit the edit once its layer is no longer the one worked on (#6483): another layer became
/// active (a Layers panel menu, the canvas layer picker, automation) or another document did (a
/// tab, File ▸ New/Open). Photoshop commits then; left open, the session took single-key
/// shortcuts as typing and typed into whichever document was active.
pub fn end_if_left(app: &mut PhotocraftApp) {
    let Some(ed) = app.ui.text_edit.as_ref() else { return };
    let layer = LayerId(ed.layer);
    if edit_home(app, ed).is_none_or(|h| Some(h) != app.session.active_index()) {
        return commit(app);
    }
    let Some(st) = app.session.active() else { return };
    if st.active_layer == Some(layer) {
        return;
    }
    // The selection just made stands: ending an empty Type Mask undoes its session, and undo
    // restores the selection from before it.
    let chosen = (st.active_layer, st.selected_layers.clone(), st.layer_anchor);
    commit(app);
    let moved = app.session.active().is_some_and(|st| (st.active_layer, &st.selected_layers, st.layer_anchor) != (chosen.0, &chosen.1, chosen.2));
    if moved {
        let _ = app.session.restore_layer_selection(chosen.0, &chosen.1, chosen.2);
    }
}

/// Cancel the editing session: undo it back to where it started (removes a new layer).
pub fn cancel(app: &mut PhotocraftApp) {
    app.ui.type_insertion_style = None;
    crate::type_transform::reset(app);
    let Some(ed) = app.ui.text_edit.take() else { return };
    app.type_ime.clear_preedit();
    if let Some(tool) = app.ui.text_edit_return_tool.take()
        && app.ui.tool.is_type()
    {
        app.ui.tool = tool;
    }
    undo_session(app, &ed);
}

/// Undo the session's history step, if it is still the latest.
fn undo_session(app: &mut PhotocraftApp, ed: &TextEdit) {
    let coalesced = app.session.active().is_some_and(|s| s.coalesce.as_deref() == Some(ed.session.as_str()));
    if coalesced {
        let _ = app.run("edit.undo", json!({}));
    }
}
