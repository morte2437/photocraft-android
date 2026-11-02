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
use super::{LayerId, PhotocraftApp, byte_of, current_text, json, text_char_count, text_layer};

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
                let edit = KeyEditContext { id, vertical_flow, edit: &ed, selection: (a, b), text: &text, char_count: n };
                let mut flow = EventFlow { handled: &mut handled, index: k, custom_mutations: &mut custom_mutations, deferred_custom: &mut deferred_custom };
                handle_pressed_key(app, ctx, key, m, &edit, &mut flow);
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

struct KeyEditContext<'a> {
    id: LayerId,
    vertical_flow: bool,
    edit: &'a crate::state::TextEdit,
    selection: (usize, usize),
    text: &'a str,
    char_count: usize,
}

struct EventFlow<'a> {
    handled: &'a mut [bool],
    index: usize,
    custom_mutations: &'a mut bool,
    deferred_custom: &'a mut bool,
}

fn handle_pressed_key(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    raw_key: &egui::Key,
    modifiers: &egui::Modifiers,
    edit: &KeyEditContext<'_>,
    flow: &mut EventFlow<'_>,
) {
    let key = flow_key(*raw_key, edit.vertical_flow);
    if handle_style_key(app, key, modifiers, edit) || handle_delete_key(app, key, modifiers, edit) || handle_move_key(app, key, modifiers, edit) {
        return;
    }

    use egui::Key;
    match key {
        Key::Enter if modifiers.command && edit.edit.preedit.is_none() => commit_with(app, *modifiers),
        Key::Enter if modifiers.command => {} // IME owns confirmation until preedit ends.
        Key::Enter => insert(app, "\n"),
        Key::Tab if !modifiers.command && !modifiers.alt && !modifiers.shift => {
            // egui queues focus navigation before this handler runs; consuming the
            // key alone would let the menu bar take focus after the first tab (#8889).
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            insert(app, "\t");
        }
        Key::Escape if crate::type_transform::active(app) => crate::type_transform::cancel_drag(app),
        Key::Escape if edit.edit.mask => cancel(app),
        Key::Escape if app.session.prefs().type_.use_esc_to_commit => commit(app),
        Key::Escape => cancel(app),
        Key::A if modifiers.command => {
            app.ui.type_insertion_style = None;
            if let Some(state) = app.ui.text_edit.as_mut() {
                state.anchor = 0;
                state.caret = edit.char_count;
            }
        }
        Key::T if modifiers.command && !modifiers.shift && !modifiers.alt => {
            let _ = crate::menus::invoke(app, ctx, "window.panel.character", json!({}));
        }
        _ if modifiers.command => {
            if key == Key::Z {
                crate::type_transform::cancel_drag(app);
            }
            if reset_guarded(app, ctx.viewport_id()) || app.type_ime.incoming.is_some() {
                interrupt_action(app, ctx.viewport_id());
                *flow.custom_mutations = false;
                *flow.deferred_custom = true;
                suspend_reset(app, ctx);
            }
            flow.handled[flow.index] = false;
        }
        _ => {}
    }
}

fn handle_style_key(app: &mut PhotocraftApp, key: egui::Key, modifiers: &egui::Modifiers, edit: &KeyEditContext<'_>) -> bool {
    use egui::Key;
    match key {
        Key::K | Key::U | Key::B | Key::I | Key::Slash | Key::Questionmark if modifiers.command && modifiers.shift && !modifiers.alt => {
            let style = match key {
                Key::K => CharToggle::AllCaps,
                Key::U => CharToggle::Underline,
                Key::B => CharToggle::FauxBold,
                Key::I => CharToggle::FauxItalic,
                _ => CharToggle::Strikethrough,
            };
            toggle_char_style(app, style);
            true
        }
        Key::H if modifiers.command && modifiers.shift && modifiers.alt => {
            toggle_hyphenate(app);
            true
        }
        Key::ArrowUp | Key::ArrowDown if modifiers.alt => {
            let amount = if modifiers.command { 10.0 } else { 2.0 };
            let up = key == Key::ArrowUp;
            let (property, delta) =
                if modifiers.shift { ("baselineShiftBy", if up { amount } else { -amount }) } else { ("leadingBy", if up { -amount } else { amount }) };
            step_char_metric(app, property, delta);
            true
        }
        Key::ArrowLeft | Key::ArrowRight if modifiers.alt && !modifiers.shift && edit.selection.0 != edit.selection.1 => {
            let amount = if modifiers.command { 100.0 } else { 20.0 };
            let delta = if key == Key::ArrowRight { amount } else { -amount };
            step_char_metric(app, "trackingBy", delta);
            true
        }
        Key::ArrowLeft | Key::ArrowRight if modifiers.alt && !modifiers.shift && edit.selection.0 == edit.selection.1 => {
            let amount = if modifiers.command { 100.0 } else { 20.0 };
            let delta = if key == Key::ArrowRight { amount } else { -amount };
            kern_pair(app, edit.id, edit.edit.caret, delta);
            true
        }
        _ => false,
    }
}

fn handle_delete_key(app: &mut PhotocraftApp, key: egui::Key, modifiers: &egui::Modifiers, edit: &KeyEditContext<'_>) -> bool {
    use egui::Key;
    if !matches!(key, Key::Backspace | Key::Delete) {
        return false;
    }
    let (start, end) = edit.selection;
    if start == end {
        let forward = key == Key::Delete;
        let to = match (forward, modifiers.alt, modifiers.command) {
            (false, _, true) => line_edge(app, edit.id, start, false),
            (false, true, _) => word_boundary(edit.text, start, false),
            (false, _, _) => start.saturating_sub(1),
            (true, true, _) => word_boundary(edit.text, start, true),
            (true, _, _) => (start + 1).min(edit.char_count),
        };
        if to != start {
            if let Some(state) = app.ui.text_edit.as_mut() {
                state.anchor = to;
            }
            insert(app, "");
        }
    } else {
        insert(app, "");
    }
    true
}

fn handle_move_key(app: &mut PhotocraftApp, key: egui::Key, modifiers: &egui::Modifiers, edit: &KeyEditContext<'_>) -> bool {
    use egui::Key;
    let caret = edit.edit.caret;
    let (start, end) = edit.selection;
    let destination = match key {
        Key::ArrowLeft | Key::ArrowRight => {
            let forward = key == Key::ArrowRight;
            Some(if modifiers.command || modifiers.alt {
                word_boundary(edit.text, caret, forward)
            } else if start != end && !modifiers.shift {
                if forward { end } else { start }
            } else if forward {
                caret + 1
            } else {
                caret.saturating_sub(1)
            })
        }
        Key::ArrowUp | Key::ArrowDown => Some(if modifiers.command {
            if key == Key::ArrowUp { 0 } else { edit.char_count }
        } else {
            line_step(app, edit.id, caret, if key == Key::ArrowUp { -1 } else { 1 })
        }),
        Key::Home | Key::End => Some(line_edge(app, edit.id, caret, key == Key::End)),
        _ => None,
    };
    if let Some(destination) = destination {
        set_caret(app, edit.char_count, destination, modifiers.shift);
        true
    } else {
        false
    }
}

fn set_caret(app: &mut PhotocraftApp, char_count: usize, caret: usize, extend: bool) {
    if let Some(state) = app.ui.text_edit.as_mut() {
        if state.caret != caret.min(char_count) || (!extend && state.anchor != caret.min(char_count)) {
            app.ui.type_insertion_style = None;
        }
        state.caret = caret.min(char_count);
        if !extend {
            state.anchor = state.caret;
        }
    }
}
