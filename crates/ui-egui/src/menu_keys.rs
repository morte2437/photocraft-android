//! The menu bar from the keyboard (#4988), as Photoshop's on Windows: a tap on Alt (or F10)
//! puts the keyboard on the bar (File lit; ← / → move along it, ↓ / ↩ / Space open the lit menu,
//! Esc or a click leaves), Alt+letter opens the menu with that mnemonic straight away, and in an
//! open menu a letter runs the row with that mnemonic (or opens its submenu).
//!
//! Mnemonics are underlined while Alt is held, the bar is focused or a menu was opened from the
//! keyboard; a menu opened with the mouse shows none until Alt is pressed, as Windows does.
//! They are assigned per menu level from the labels as shown (so translated), unique within
//! the level: Photoshop's letters for the top menus where the label has them (F E I L Y S T V
//! W H), else the first unused letter, word starts first. Only ASCII letters and digits are keys
//! the keyboard reports as such, so a label without one (Japanese, Chinese…) shows its English
//! label's letter after it, as Windows menus do in those languages: "文件(F)" ([`localized`]).
//!
//! The state is view state in egui memory, like [`crate::menu_nav::Nav`]. On macOS the native
//! menu bar replaces the in-window one and this never runs.

use egui::{Color32, Event, Id, Key, Modifiers, Stroke, Ui};

use crate::menu_nav::Nav;

/// Photoshop's mnemonics for the top menus (Windows), used where the shown label has the letter.
pub const TOP_PREFERRED: [(&str, char); 10] = [
    ("File", 'F'),
    ("Edit", 'E'),
    ("Image", 'I'),
    ("Layer", 'L'),
    ("Type", 'Y'),
    ("Select", 'S'),
    ("Filter", 'T'),
    ("View", 'V'),
    ("Window", 'W'),
    ("Help", 'H'),
];

/// Keyboard state of the menu bar between frames.
#[derive(Clone, Debug, Default)]
pub struct Keys {
    /// The title the keyboard is on with no menu open, in screen order (Alt tap, F10).
    pub focus: Option<usize>,
    /// Underline the mnemonics: the bar is focused or a menu was opened from the keyboard.
    pub underline: bool,
    /// Alt is down (from its key events, whatever the platform reports as modifiers).
    alt_down: bool,
    /// Alt went down and nothing else was pressed since: releasing it is a tap.
    alt_tap: bool,
    /// The mouse buttons that are down, one bit each, from the button events in order (the
    /// frame's pointer state is its end state, which a whole click within one frame doesn't show).
    buttons_held: u8,
    /// A menu was just opened from the keyboard: light its first row once it is drawn.
    first_row: bool,
}

fn keys_id() -> Id {
    Id::new("pc-menu-bar-keys")
}

fn focus_id() -> Id {
    Id::new("pc-menu-bar-focus")
}

/// Is the keyboard on the menu bar (no menu open yet)? The shortcuts leave the keys to it then,
/// as they do for an open menu: ← / → / ↓ / ↩ / Esc and the letters drive the bar.
pub fn is_focused(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(focus_id())).is_some_and(|seen| seen + 2 >= pass)
}

/// A command shortcut pressed while the keyboard is on the bar with no menu open (⌘D, ⌘⇧I, F7…)
/// takes it off the bar, so the shortcut runs in the same press, as a native menu bar does
/// (#6573). Keys the bar uses (arrows, ↩, Esc, letters, Alt+letter, F10) keep driving it, and
/// an open menu still swallows shortcuts. True when the bar was left.
pub fn leave_for_shortcut(ctx: &egui::Context) -> bool {
    if !is_focused(ctx) || crate::menu_nav::is_open(ctx) {
        return false;
    }
    let shortcut = ctx.input(|i| i.events.iter().any(|e| matches!(e, Event::Key { key, pressed: true, modifiers, .. } if is_command_key(*key, *modifiers))));
    if !shortcut {
        return false;
    }
    let mut keys = Keys::load(ctx);
    keys.focus = None;
    keys.underline = false;
    keys.store(ctx);
    ctx.request_repaint();
    true
}

/// A key press that is a command shortcut rather than a key of the bar: ⌘ / Ctrl with a key
/// (not the modifier keys themselves), or a function key other than F10 (which toggles the bar).
fn is_command_key(key: Key, mods: Modifiers) -> bool {
    let modifier_key = matches!(
        key,
        Key::ShiftLeft | Key::ShiftRight | Key::ControlLeft | Key::ControlRight | Key::AltLeft | Key::AltRight | Key::SuperLeft | Key::SuperRight
    );
    if modifier_key {
        return false;
    }
    let function_key = key != Key::F10 && key.name().strip_prefix('F').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    mods.command || mods.ctrl || mods.mac_cmd || function_key
}

/// The mnemonic a key press stands for: the letter or digit of a plain key.
fn key_char(key: Key) -> Option<char> {
    let name = key.name();
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}

/// Mnemonics for `labels`, unique within the group: the preferred letter when the label has it,
/// else the first unused ASCII letter or digit, trying the first character of each word before
/// the others. Compared case-insensitively; the returned letter is upper case.
pub fn assign(labels: &[&str], preferred: &[Option<char>]) -> Vec<Option<char>> {
    let mut used: Vec<char> = Vec::new();
    let mut out = Vec::with_capacity(labels.len());
    for (i, label) in labels.iter().enumerate() {
        let upper: Vec<char> = label.chars().map(|c| c.to_ascii_uppercase()).collect();
        let has = |c: char| upper.contains(&c.to_ascii_uppercase());
        let mut pick = preferred.get(i).copied().flatten().map(|c| c.to_ascii_uppercase()).filter(|&c| has(c) && !used.contains(&c));
        if pick.is_none() {
            let starts = label.split(|c: char| !c.is_alphanumeric()).filter_map(|w| w.chars().next());
            let rest = label.chars();
            pick = starts.chain(rest).filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).find(|c| !used.contains(c));
        }
        if let Some(c) = pick {
            used.push(c);
        }
        out.push(pick);
    }
    out
}

/// The labels to show and their mnemonics for translated `labels` whose English originals are
/// `english` (same order). A label with an ASCII letter or digit keeps its text and is assigned
/// as [`assign`] does. One without (Chinese, Japanese, Korean…) can't be typed on its own, so it
/// shows its English label's letter after it in parentheses and takes that letter, as Windows menus
/// do in those languages ("文件(F)", #5863). Mnemonics stay unique within the group, and a shown
/// "(F)" is always the row's key.
pub fn localized(labels: &[String], english: &[&str], preferred: &[Option<char>]) -> (Vec<String>, Vec<Option<char>>) {
    let typed = |l: &str| l.chars().any(|c| c.is_ascii_alphanumeric());
    // The key goes before a trailing ellipsis, as Windows writes it: "新建(N)…".
    let with_key = |l: &str, k: char| match l.strip_suffix('…').map(|b| (b, "…")).or_else(|| l.strip_suffix("...").map(|b| (b, "..."))) {
        Some((base, tail)) => format!("{base}({k}){tail}"),
        None => format!("{l}({k})"),
    };
    let english_keys = assign(english, preferred);
    let mut shown = Vec::with_capacity(labels.len());
    let mut pref = Vec::with_capacity(labels.len());
    for (i, label) in labels.iter().enumerate() {
        match english_keys.get(i).copied().flatten().filter(|_| !typed(label)) {
            Some(k) => {
                shown.push(with_key(label, k));
                pref.push(Some(k));
            }
            None => {
                shown.push(label.clone());
                pref.push(preferred.get(i).copied().flatten());
            }
        }
    }
    // The suffixed labels take their keys first: a label with letters of its own must not take
    // the key a suffix shows ("Camera Raw 滤镜…" before "转换为智能滤镜(C)" leaves C to the latter).
    let suffixed = |i: &usize| shown.get(*i) != labels.get(*i);
    let order: Vec<usize> = (0..shown.len()).filter(suffixed).chain((0..shown.len()).filter(|i| !suffixed(i))).collect();
    let ordered: Vec<&str> = order.iter().map(|&i| shown.get(i).map_or("", String::as_str)).collect();
    let ordered_pref: Vec<Option<char>> = order.iter().map(|&i| pref.get(i).copied().flatten()).collect();
    let mut keys = vec![None; shown.len()];
    for (&i, key) in order.iter().zip(assign(&ordered, &ordered_pref)) {
        if let Some(slot) = keys.get_mut(i) {
            *slot = key;
        }
    }
    (shown, keys)
}

/// `prefix + label` as menu text with the mnemonic's first occurrence in `label` underlined
/// (none when `mnemonic` is None). `color` is the text colour; None takes the widget's.
pub fn mnemonic_text(ui: &Ui, prefix: &str, label: &str, mnemonic: Option<char>, color: Option<Color32>) -> egui::WidgetText {
    let color = color.unwrap_or(Color32::PLACEHOLDER);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let plain = egui::TextFormat { font_id: font.clone(), color, ..Default::default() };
    let at = mnemonic.and_then(|m| label.char_indices().find(|(_, c)| c.to_ascii_uppercase() == m).map(|(i, c)| (i, c.len_utf8())));
    let Some((i, len)) = at else {
        return egui::RichText::new(format!("{prefix}{label}")).color(color).into();
    };
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let underlined = egui::TextFormat { underline: Stroke::new(1.0, color), ..plain.clone() };
    let before = format!("{prefix}{}", &label[..i]);
    if !before.is_empty() {
        job.append(&before, 0.0, plain.clone());
    }
    job.append(&label[i..i + len], 0.0, underlined);
    if i + len < label.len() {
        job.append(&label[i + len..], 0.0, plain);
    }
    job.into()
}

/// The checkmark menu rows draw in their gutter.
pub const CHECK_MARK: &str = "✔ ";

/// Width of the checkmark column every menu row reserves (#5705): the width of [`CHECK_MARK`] in
/// the button font, so checked, unchecked and plain rows (and submenu headers) all start their
/// label at the same x, whatever the script of the label.
pub fn check_gutter(ui: &Ui) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    ui.painter().layout_no_wrap(CHECK_MARK.to_owned(), font, Color32::PLACEHOLDER).size().x
}

/// A menu row's text (#5705): the label behind a fixed-width checkmark column ([`check_gutter`]),
/// with ✔ in the column when `checked`, and the mnemonic underlined when given (as
/// [`mnemonic_text`]).
pub fn menu_row_text(ui: &Ui, checked: bool, label: &str, mnemonic: Option<char>) -> egui::WidgetText {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let plain = egui::TextFormat { font_id: font, color: Color32::PLACEHOLDER, ..Default::default() };
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    // The label starts after the column: a checked row draws the mark in it, the others leave it
    // empty (a leading space of the same width, not padding characters whose width varies).
    let mut lead = check_gutter(ui);
    if checked {
        job.append(CHECK_MARK, 0.0, plain.clone());
        lead = 0.0;
    }
    let at = mnemonic.and_then(|m| label.char_indices().find(|(_, c)| c.to_ascii_uppercase() == m).map(|(i, c)| (i, c.len_utf8())));
    let parts = match at {
        Some((i, len)) => [label.get(..i), label.get(i..i + len), label.get(i + len..)],
        None => [Some(label), None, None],
    };
    for (k, part) in parts.into_iter().enumerate() {
        let Some(part) = part.filter(|p| !p.is_empty()) else { continue };
        let format =
            if k == 1 && at.is_some() { egui::TextFormat { underline: Stroke::new(1.0, Color32::PLACEHOLDER), ..plain.clone() } } else { plain.clone() };
        job.append(part, lead, format);
        lead = 0.0;
    }
    if job.sections.is_empty() {
        // An empty label still keeps the row as wide as its column.
        job.append("", lead, plain);
    }
    job.into()
}

impl Keys {
    /// Last frame's state.
    pub fn load(ctx: &egui::Context) -> Keys {
        ctx.data(|d| d.get_temp(keys_id())).unwrap_or_default()
    }

    /// The state as the last frame left it (tests, inspection).
    pub fn current(ctx: &egui::Context) -> Keys {
        Self::load(ctx)
    }

    pub fn store(self, ctx: &egui::Context) {
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|d| {
            if self.focus.is_some() {
                d.insert_temp(focus_id(), pass);
            } else {
                d.remove::<u64>(focus_id());
            }
            d.insert_temp(keys_id(), self);
        });
    }

    /// Underline the mnemonics this frame?
    pub fn show_underlines(&self, ctx: &egui::Context) -> bool {
        self.underline || self.focus.is_some() || self.alt_down || ctx.input(|i| i.modifiers.alt)
    }

    /// A menu just opened from the keyboard: light its first row (once, when it is drawn).
    pub fn take_first_row(&mut self, nav: &mut Nav) {
        if self.first_row && nav.top.is_some() {
            nav.highlight_first(0);
            self.first_row = false;
        }
    }

    /// Handle this frame's keys, after the menus were drawn. `tops` are the titles' popup ids
    /// and `mnemonics` their letters, both in screen order; `nav` is the open menu's state. A
    /// command a mnemonic ran lands in `clicked`.
    pub fn keys(&mut self, ctx: &egui::Context, tops: &[Id], mnemonics: &[Option<char>], nav: &mut Nav, clicked: &mut Option<String>) {
        if tops.is_empty() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        let mut tap = false;
        // The tap is judged on every event of the frame, also those another handler consumed
        // before the bar's turn: the shortcuts run first, so Ctrl+Alt+I (Image Size) left no
        // trace in `events`, and releasing Alt counted as a tap that put the keyboard on the bar;
        // Enter then opened File instead of confirming the dialog (#7481).
        let raw = ctx.input(|i| i.raw.events.clone());
        for e in &raw {
            match e {
                Event::Key { key: Key::AltLeft | Key::AltRight, pressed: true, .. } => {
                    self.alt_down = true;
                    // Alt pressed while a mouse button is down is a modifier for that drag
                    // (Free Transform's scale-from-centre), never a tap (#8113).
                    self.alt_tap = self.buttons_held == 0;
                }
                Event::Key { key: Key::AltLeft | Key::AltRight, pressed: false, .. } => {
                    self.alt_down = false;
                    tap |= std::mem::take(&mut self.alt_tap);
                }
                // A click or the end of a drag while Alt is held is no tap. It also cancels a tap
                // taken earlier in this frame: a whole drag can arrive in one frame (press, Alt
                // down, Alt up, release).
                Event::PointerButton { button, pressed, .. } => {
                    let bit = 1u8 << (*button as u8);
                    if *pressed {
                        self.buttons_held |= bit;
                    } else {
                        self.buttons_held &= !bit;
                    }
                    self.alt_tap = false;
                    tap = false;
                }
                // Alt going to the desktop is no tap, and a button let go out there is up.
                Event::WindowFocused(false) => {
                    self.alt_tap = false;
                    self.buttons_held = 0;
                }
                // Alt+wheel (zoom, brush size…) is no tap either.
                Event::Key { pressed: true, .. } | Event::Text(_) | Event::MouseWheel { .. } => self.alt_tap = false,
                _ => {}
            }
        }
        if events.iter().any(|e| matches!(e, Event::PointerButton { pressed: true, .. })) {
            self.focus = None;
            self.underline = false;
        }
        let open = nav.top.is_some();
        let n = tops.len();
        let open_top = |i: usize, keys: &mut Keys| {
            if let Some(&id) = tops.get(i) {
                egui::Popup::open_id(ctx, id);
                keys.focus = None;
                keys.underline = true;
                keys.first_row = true;
                ctx.request_repaint();
            }
        };
        let consume = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        // Alt+letter opens that menu, open or not (an open one switches), as on Windows.
        let alt_letter = events.iter().find_map(|e| match e {
            Event::Key { key, pressed: true, modifiers, .. } if modifiers.alt && !modifiers.command && !modifiers.ctrl => key_char(*key).map(|c| (*key, c)),
            _ => None,
        });
        if let Some((key, c)) = alt_letter
            && let Some(i) = mnemonics.iter().position(|m| *m == Some(c))
        {
            consume(Modifiers::ALT, key);
            self.alt_tap = false;
            open_top(i, self);
            return;
        }
        if open {
            // A letter runs the row with that mnemonic in the level the arrows are in, unless it is
            // typed into a field in the menu: Help › Search took "S" for System Info (#5996).
            let typing = ctx.text_edit_focused();
            let letter = events.iter().filter(|_| !typing).find_map(|e| match e {
                Event::Key { key, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.ctrl && !modifiers.alt => {
                    key_char(*key).map(|c| (*key, c))
                }
                _ => None,
            });
            if let Some((key, c)) = letter
                && let Some(row) = nav.row_with_mnemonic(c)
            {
                consume(Modifiers::NONE, key);
                self.underline = true;
                if let Some(cmd) = row.command.clone() {
                    *clicked = Some(cmd);
                    egui::Popup::close_all(ctx);
                } else {
                    nav.open_sub_row(ctx, row.widget);
                }
                ctx.request_repaint();
            }
            if tap {
                egui::Popup::close_all(ctx);
                self.underline = false;
                ctx.request_repaint();
            }
            return;
        }
        // No menu open: Alt (tapped) or F10 puts the keyboard on the bar or takes it off again.
        let f10 = consume(Modifiers::NONE, Key::F10);
        if tap || f10 {
            self.focus = if self.focus.is_some() { None } else { Some(0) };
            self.underline = false;
            ctx.request_repaint();
            return;
        }
        let Some(focus) = self.focus else { return };
        let press = |k: Key| consume(Modifiers::NONE, k);
        if press(Key::Escape) {
            self.focus = None;
        } else if press(Key::ArrowRight) {
            self.focus = Some((focus + 1) % n);
        } else if press(Key::ArrowLeft) {
            self.focus = Some((focus + n - 1) % n);
        } else if press(Key::ArrowDown) || press(Key::Enter) || press(Key::Space) {
            open_top(focus, self);
        } else if let Some((key, c)) = events.iter().find_map(|e| match e {
            Event::Key { key, pressed: true, modifiers, .. } if modifiers.is_none() => key_char(*key).map(|c| (*key, c)),
            _ => None,
        }) {
            if let Some(i) = mnemonics.iter().position(|m| *m == Some(c)) {
                consume(Modifiers::NONE, key);
                open_top(i, self);
            }
        } else {
            return;
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_are_unique_and_prefer_photoshop_s() {
        let labels = ["File", "Edit", "Image", "Layer", "Type", "Select", "Filter", "View", "Window", "Help"];
        let preferred: Vec<Option<char>> = TOP_PREFERRED.iter().map(|(_, c)| Some(*c)).collect();
        assert_eq!(assign(&labels, &preferred), "FEILYSTVWH".chars().map(Some).collect::<Vec<_>>());
        // Items: first letters first, then the next free letter of the word starts, then any.
        assert_eq!(
            assign(&["New…", "Open…", "Open Recent", "Close", "Close All", "Save", "Save As…"], &[]),
            [Some('N'), Some('O'), Some('R'), Some('C'), Some('A'), Some('S'), Some('V'),]
        );
        // A preferred letter the label lacks (a translation) falls back; a taken one too.
        assert_eq!(assign(&["Datei", "Bearbeiten"], &[Some('F'), Some('E')]), [Some('D'), Some('E')]);
        assert_eq!(assign(&["Aa", "Ab", "Ba"], &[Some('A'), Some('A'), Some('A')]), [Some('A'), Some('B'), None]);
        // Only ASCII letters and digits are keys: no mnemonic without one.
        assert_eq!(assign(&["ファイル", "Zoom 100%", "✔ Rulers"], &[]), [None, Some('Z'), Some('R')]);
    }

    #[test]
    fn key_chars_are_plain_letters_and_digits() {
        assert_eq!(key_char(Key::A), Some('A'));
        assert_eq!(key_char(Key::Num7), Some('7'));
        assert_eq!(key_char(Key::F10), None);
        assert_eq!(key_char(Key::ArrowDown), None);
        assert_eq!(key_char(Key::Space), None);
    }

    #[test]
    fn command_keys_are_command_combos_and_function_keys_but_f10() {
        assert!(is_command_key(Key::D, Modifiers::COMMAND));
        assert!(is_command_key(Key::I, Modifiers::COMMAND | Modifiers::SHIFT));
        assert!(is_command_key(Key::F7, Modifiers::NONE));
        assert!(is_command_key(Key::F12, Modifiers::SHIFT));
        assert!(!is_command_key(Key::F10, Modifiers::NONE), "F10 toggles the bar");
        assert!(!is_command_key(Key::ControlLeft, Modifiers::CTRL), "Ctrl going down is no shortcut yet");
        assert!(!is_command_key(Key::E, Modifiers::NONE), "a letter is a mnemonic");
        assert!(!is_command_key(Key::E, Modifiers::ALT), "Alt+letter opens a menu");
        assert!(!is_command_key(Key::ArrowRight, Modifiers::NONE));
        assert!(!is_command_key(Key::Enter, Modifiers::NONE));
        assert!(!is_command_key(Key::Escape, Modifiers::NONE));
    }
}

#[cfg(test)]
#[path = "menu_keys_tests.rs"]
mod harness_tests;
