use super::editing::{
    CharToggle, cancel, commit, commit_with, copy_selection, cut_selection, end_if_left, insert, insert_typed, kern_pair, step_char_metric, toggle_char_style,
    toggle_hyphenate,
};
use super::ime::{
    bind_reset_context, claim_raw_pass, complete_reset, controls_only, ime_target, interrupt_action, interrupt_incoming, prepare_reset, reset_guarded,
    reset_route_eligible, suspend_reset, sync_incoming,
};
use super::ime_edit::{ime_delete_bytes, ime_delete_chars, ime_update};
use super::layout::layout;
use super::pointer::word_boundary;
use super::*;

/// Caret on the neighbouring line (±1; a column in vertical type), keeping the position
/// along the line. Works in line space, so it serves both orientations.
fn line_step(app: &mut PhotocraftApp, id: LayerId, caret: usize, dir: i32) -> usize {
    let Some((l, _, text)) = layout(app, id) else { return caret };
    let x = l.caret(byte_of(&text, caret)).0;
    photocraft_text::line_step(&l, &text, caret, x, dir)
}

pub(super) fn is_vertical(app: &PhotocraftApp, id: LayerId) -> bool {
    app.session.active().and_then(|s| text_layer(&s.doc, id)).is_some_and(|t| t.orientation == photocraft_doc::text::Orientation::Vertical)
}

/// Arrow keys follow the text flow: in vertical type ↑/↓ move along the column (previous/next
/// character) and ←/→ move to the next/previous column (columns advance right to left). Returns
/// the horizontal-type key with the same meaning.
pub(crate) fn flow_key(key: egui::Key, vertical: bool) -> egui::Key {
    use egui::Key;
    if !vertical {
        return key;
    }
    match key {
        Key::ArrowUp => Key::ArrowLeft,
        Key::ArrowDown => Key::ArrowRight,
        Key::ArrowLeft => Key::ArrowDown,
        Key::ArrowRight => Key::ArrowUp,
        k => k,
    }
}

/// Line start / end for the caret's line.
fn line_edge(app: &mut PhotocraftApp, id: LayerId, caret: usize, end: bool) -> usize {
    let Some((l, _, text)) = layout(app, id) else { return caret };
    photocraft_text::line_edge(&l, &text, caret, end)
}

/// Keyboard input while editing. Returns true when a type edit session is active (single-key
/// tool shortcuts must then be skipped). Handled events are removed from the frame's input.
pub fn handle_keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    bind_reset_context(app, ctx);
    // A receipt is only valid for the frame that produced it; never reuse last frame's.
    app.type_ime.frame = None;
    end_if_left(app);
    sync_incoming(app, ctx.viewport_id());
    if !ctx.input(|i| i.focused) || ctx.text_edit_focused() || ctx.egui_wants_keyboard_input() {
        interrupt_incoming(app);
    }
    crate::type_transform::cancel_stale(app);
    if !ctx.input(|i| i.focused) {
        crate::type_transform::cancel_drag(app);
    }
    let Some(ed) = app.ui.text_edit.clone() else {
        controls_only(app, ctx);
        return false;
    };
    // The font picker (and other focused fields) owns keyboard input while it is open.
    if !reset_route_eligible(app, ctx) {
        controls_only(app, ctx);
        return true;
    }
    let id = LayerId(ed.layer);
    let Some(n) = text_char_count(app, id) else {
        app.ui.text_edit = None;
        app.ui.text_edit_return_tool = None;
        app.type_ime.clear_preedit();
        controls_only(app, ctx);
        return false;
    };
    // Undo/redo can shorten the text under us.
    if let Some(e) = app.ui.text_edit.as_mut() {
        e.caret = e.caret.min(n);
        e.anchor = e.anchor.min(n);
    }
    if !claim_raw_pass(app, ctx) {
        return true;
    }
    app.type_ime.begin_frame(ctx);
    prepare_reset(app, ctx);
    // A target swap (session replacement, Type Mask, another layer) is known from the last
    // descriptor the platform received. Cancel the native composition before any event in this
    // pass can be applied to the new target.
    if ime_target(app, &ed, ctx.viewport_id()).is_some_and(|target| app.type_ime.emitted.is_some_and(|last| last.target != target)) {
        app.type_ime.interrupt = true;
    }
    let time = ctx.input(|i| i.time);
    let elapsed = (time - app.ui.caret_blink_time).max(0.0);
    let period = 0.530;
    let remaining = period - (elapsed % period);
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(remaining.clamp(0.010, period)));
    let events = ctx.input(|i| i.events.clone());
    let vertical_flow = is_vertical(app, id);
    let mut handled = vec![false; events.len()];
    let mut primary_down = events
        .iter()
        .find_map(|event| match event {
            egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed, .. } => Some(!pressed),
            _ => None,
        })
        .unwrap_or_else(|| ctx.input(|input| input.pointer.primary_down()));
    let mut custom_mutations = true;
    let mut deferred_custom = false;
    for (k, ev) in events.iter().enumerate() {
        if custom_mutations && !reset_route_eligible(app, ctx) {
            custom_mutations = false;
            suspend_reset(app, ctx);
        }
        if !custom_mutations {
            if let egui::Event::ImeResetCompleted { request_id } = ev {
                complete_reset(app, ctx, *request_id);
                if let Some(slot) = handled.get_mut(k) {
                    *slot = true;
                }
            } else if deferred_custom
                && matches!(ev, egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Cut | egui::Event::Ime(_))
                && let Some(slot) = handled.get_mut(k)
            {
                *slot = true;
            }
            continue;
        }
        prepare_reset(app, ctx);
        let Some(ed) = app.ui.text_edit.clone() else { continue };
        let (a, b) = (ed.caret.min(ed.anchor), ed.caret.max(ed.anchor));
        // Only key handling needs the text (word/line edges); pointer, clipboard and IME events
        // do not clone the whole layer.
        let is_key = matches!(ev, egui::Event::Key { .. });
        let text = if is_key { current_text(app, id).unwrap_or_default() } else { String::new() };
        let n = if is_key { text.chars().count() } else { 0 };
        let set = |app: &mut PhotocraftApp, caret: usize, extend: bool| {
            if let Some(e) = app.ui.text_edit.as_mut() {
                e.caret = caret.min(n);
                if !extend {
                    e.anchor = e.caret;
                }
            }
        };
        handled[k] = true;
        match ev {
            egui::Event::Text(s) => insert_typed(app, s),
            egui::Event::Paste(s) => insert(app, s),
            egui::Event::ImeResetCompleted { request_id } => complete_reset(app, ctx, *request_id),
            egui::Event::Ime(ime_event) => {
                // Untagged terminal payload cannot acknowledge a host reset. Only the exact
                // current completion arm above may open admission for the following element.
                if reset_guarded(app, ctx.viewport_id()) {
                    continue;
                }
                match ime_event {
                    // A bare line break from the IME is the Enter key, which the key arm handles.
                    egui::ImeEvent::Preedit { text: s, .. } | egui::ImeEvent::Commit(s) if s == "\n" || s == "\r" => handled[k] = false,
                    egui::ImeEvent::Preedit { text, .. } => ime_update(app, text, false),
                    egui::ImeEvent::Commit(s) => {
                        ime_update(app, s, true);
                        app.type_ime.incoming = None;
                    }
                    egui::ImeEvent::DeleteSurrounding { before_chars, after_chars } => {
                        ime_delete_chars(app, *before_chars, *after_chars);
                    }
                    egui::ImeEvent::DeleteSurroundingBytes { before_bytes, after_bytes } => {
                        ime_delete_bytes(app, *before_bytes, *after_bytes);
                    }
                    _ => {}
                }
            }
            egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed, .. } => {
                primary_down = *pressed;
                if *pressed {
                    // The canvas destination handler runs after this pass; never apply a
                    // same-batch Text/Paste/Cut/IME suffix at the old caret just because no
                    // composition or reset guard existed yet.
                    interrupt_action(app, ctx.viewport_id());
                    custom_mutations = false;
                    deferred_custom = true;
                    suspend_reset(app, ctx);
                }
                handled[k] = false;
            }
            egui::Event::PointerMoved(_) if primary_down => {
                interrupt_action(app, ctx.viewport_id());
                custom_mutations = false;
                deferred_custom = true;
                suspend_reset(app, ctx);
                handled[k] = false;
            }
            egui::Event::Copy => {
                copy_selection(app, ctx);
            }
            egui::Event::Cut => cut_selection(app, ctx),
            egui::Event::Key { key, pressed: true, modifiers: m, .. } => {
                use egui::Key;
                let key = &flow_key(*key, vertical_flow);
                match key {
                    // Photoshop's type formatting keys act on the selected characters and win
                    // over the global ⇧⌘K/U/B/I (Color Settings, Desaturate, Auto Color, Inverse)
                    // while typing. Each press is one history step.
                    Key::K | Key::U | Key::B | Key::I | Key::Slash | Key::Questionmark if m.command && m.shift && !m.alt => {
                        let which = match key {
                            Key::K => CharToggle::AllCaps,
                            Key::U => CharToggle::Underline,
                            Key::B => CharToggle::FauxBold,
                            Key::I => CharToggle::FauxItalic,
                            _ => CharToggle::Strikethrough,
                        };
                        toggle_char_style(app, which);
                    }
                    Key::H if m.command && m.shift && m.alt => toggle_hyphenate(app),
                    // ⌥↑/↓ leading -/+2 pt, ⇧⌥↑/↓ baseline shift +/-2 pt (10 with ⌘/Ctrl).
                    Key::ArrowUp | Key::ArrowDown if m.alt => {
                        let step = if m.command { 10.0 } else { 2.0 };
                        let up = *key == Key::ArrowUp;
                        if m.shift {
                            step_char_metric(app, "baselineShiftBy", if up { step } else { -step });
                        } else {
                            step_char_metric(app, "leadingBy", if up { -step } else { step });
                        }
                    }
                    // ⌥←/→ over a selection tracks it by -/+20 (100 with ⌘/Ctrl).
                    Key::ArrowLeft | Key::ArrowRight if m.alt && !m.shift && a != b => {
                        let step = if m.command { 100.0 } else { 20.0 };
                        step_char_metric(app, "trackingBy", if *key == Key::ArrowRight { step } else { -step });
                    }
                    Key::Backspace | Key::Delete => {
                        if a == b {
                            let fwd = *key == Key::Delete;
                            let to = match (fwd, m.alt, m.command) {
                                (false, _, true) => line_edge(app, id, a, false),
                                (false, true, _) => word_boundary(&text, a, false),
                                (false, _, _) => a.saturating_sub(1),
                                (true, true, _) => word_boundary(&text, a, true),
                                (true, _, _) => (a + 1).min(n),
                            };
                            if to != a {
                                if let Some(e) = app.ui.text_edit.as_mut() {
                                    e.anchor = to;
                                }
                                insert(app, "");
                            }
                        } else {
                            insert(app, "");
                        }
                    }
                    // Photoshop: Alt/Option+←/→ at a collapsed caret kerns the pair before it by
                    // 20/1000 em (100 with ⌘/Ctrl), one history step per press.
                    Key::ArrowLeft | Key::ArrowRight if m.alt && !m.shift && a == b => {
                        let step = if m.command { 100.0 } else { 20.0 };
                        kern_pair(app, id, ed.caret, if *key == Key::ArrowRight { step } else { -step });
                    }
                    Key::ArrowLeft | Key::ArrowRight => {
                        let fwd = *key == Key::ArrowRight;
                        // Word movement: ⌘/Ctrl (Photoshop's; Alt is kerning), and Alt+Shift
                        // extends the selection by words; Home/End go to the line edges.
                        let to = if m.command || m.alt {
                            word_boundary(&text, ed.caret, fwd)
                        } else if a != b && !m.shift {
                            if fwd { b } else { a }
                        } else if fwd {
                            ed.caret + 1
                        } else {
                            ed.caret.saturating_sub(1)
                        };
                        set(app, to, m.shift);
                    }
                    Key::ArrowUp | Key::ArrowDown => {
                        let to = if m.command {
                            if *key == Key::ArrowUp { 0 } else { n }
                        } else {
                            line_step(app, id, ed.caret, if *key == Key::ArrowUp { -1 } else { 1 })
                        };
                        set(app, to, m.shift);
                    }
                    Key::Home | Key::End => {
                        let to = line_edge(app, id, ed.caret, *key == Key::End);
                        set(app, to, m.shift);
                    }
                    Key::Enter if m.command => commit_with(app, *m),
                    Key::Enter => insert(app, "\n"),
                    // Photoshop types a tab character; layout advances it to the next tab stop.
                    Key::Tab if !m.command && !m.alt && !m.shift => {
                        // egui queues focus navigation before this handler runs; consuming the
                        // key alone would let the menu bar take focus after the first tab (#8889).
                        ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                        insert(app, "\t");
                    }
                    Key::Escape if crate::type_transform::active(app) => crate::type_transform::cancel_drag(app),
                    // The Type Mask tools: Escape always cancels, leaving the selection as it was.
                    Key::Escape if ed.mask => cancel(app),
                    // Preferences ▸ Type: with "Use Escape to Commit" off, Escape cancels the
                    // session (undoes it, removing a just-created layer) like the Cancel button.
                    Key::Escape if app.session.prefs().type_.use_esc_to_commit => commit(app),
                    Key::Escape => cancel(app),
                    Key::A if m.command => {
                        if let Some(e) = app.ui.text_edit.as_mut() {
                            e.anchor = 0;
                            e.caret = n;
                        }
                    }
                    // Photoshop: while editing type, ⌘/Ctrl+T shows or hides the Character panel
                    // (Free Transform of the layer being typed into is not what it means here).
                    Key::T if m.command && !m.shift && !m.alt => {
                        let _ = crate::menus::invoke(app, ctx, "window.panel.character", json!({}));
                    }
                    // Other command shortcuts (⌘Z, ⌘S, …) pass through to the menus.
                    _ if m.command => {
                        if *key == Key::Z {
                            crate::type_transform::cancel_drag(app);
                        }
                        if reset_guarded(app, ctx.viewport_id()) || app.type_ime.incoming.is_some() {
                            interrupt_action(app, ctx.viewport_id());
                            custom_mutations = false;
                            deferred_custom = true;
                            suspend_reset(app, ctx);
                        }
                        handled[k] = false;
                    }
                    _ => {}
                }
            }
            egui::Event::Key { pressed: false, modifiers: m, .. } => handled[k] = !m.command,
            _ => handled[k] = false,
        }
    }
    prepare_reset(app, ctx);
    if handled.iter().any(|h| *h) {
        app.ui.caret_blink_time = ctx.input(|i| i.time);
        let mut k = 0;
        ctx.input_mut(|i| {
            i.events.retain(|_| {
                let keep = !handled.get(k).copied().unwrap_or(false);
                k += 1;
                keep
            })
        });
    }
    app.ui.text_edit.is_some()
}
