//! Keyboard choice in an open dropdown list, as in Photoshop's pop-up menus (#1481): typing a
//! letter jumps to the next option that starts with it (the same letter again cycles through
//! them), letters typed in quick succession match a longer prefix, and ↩ closes the list on the
//! option it shows.

use egui::{Id, Key, Modifiers, Ui};

/// How long a pause starts a new prefix instead of adding to the letters typed before.
const PAUSE: f64 = 1.0;

/// Where the frame number of the last frame an open dropdown list was drawn is kept.
fn open_id() -> Id {
    Id::unique("pc-dropdown-list-open")
}

/// Is a dropdown list open? It then owns the keyboard, like an open menu: a letter picks an
/// option instead of a tool, ↩ closes the list instead of committing something on the canvas.
pub fn list_open(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(open_id())).is_some_and(|seen| seen.saturating_add(2) >= pass)
}

/// The keys of the open list of dropdown `id`, called while drawing the list, before its rows:
/// the index of the option this frame's typing picks (its text events are consumed), if any. ↩
/// closes the list. `current` is the option the list now shows; `labels` are as displayed.
pub fn keys(ui: &mut Ui, id: Id, current: Option<usize>, labels: &[&str]) -> Option<usize> {
    let pass = ui.ctx().cumulative_pass_nr();
    ui.data_mut(|d| d.insert_temp(open_id(), pass));
    let lang = crate::i18n::current();
    // One entry per list, tagged with the language its letters were typed in: a prefix typed in
    // another language never carries over, however quickly the language comes back.
    let key = id.with("type-ahead");
    // A list opened anew starts with no letters typed, and so does a frame in another language.
    let shown_id = key.with("pass");
    ui.data_mut(|d| {
        let other_language = d.get_temp::<(String, f64, String)>(key).is_some_and(|(_, _, typed_in)| typed_in != lang.code());
        if other_language || d.get_temp::<u64>(shown_id).is_none_or(|seen| seen.saturating_add(2) < pass) {
            d.remove::<(String, f64, String)>(key);
        }
        d.insert_temp(shown_id, pass);
    });
    if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        ui.close();
    }
    let (typed, now) = ui.input_mut(|i| {
        let mut typed = String::new();
        i.events.retain(|e| match e {
            egui::Event::Text(t) => {
                typed.push_str(t);
                false
            }
            _ => true,
        });
        (typed, i.time)
    });
    if typed.is_empty() {
        return None;
    }
    let (mut prefix, last, _) = ui.data(|d| d.get_temp::<(String, f64, String)>(key)).unwrap_or_default();
    // A pause (or a clock that went backwards) starts over.
    if !(now >= last && now - last <= PAUSE) {
        prefix.clear();
    }
    let lower: Vec<String> = labels.iter().map(|l| lang.lowercase(l)).collect();
    let mut pick = None;
    for c in typed.chars().filter(|c| !c.is_control()) {
        // A space only counts inside a prefix ("linear b"), never to start one.
        if c.is_whitespace() && prefix.is_empty() {
            continue;
        }
        prefix.push(c);
        let from = pick.or(current);
        let mut found = find(&lower, &lang.lowercase(&prefix), from);
        // A letter that leads nowhere after the others starts a new prefix ("m" then "d" is D).
        if found.is_none() {
            prefix.clear();
            if !c.is_whitespace() {
                prefix.push(c);
                found = find(&lower, &lang.lowercase(&prefix), from);
            }
        }
        pick = found.or(pick);
    }
    ui.data_mut(|d| d.insert_temp(key, (prefix, now, lang.code().to_owned())));
    pick
}

/// The option `prefix` (lower case) picks, searching on from `from` and wrapping. A prefix of one
/// repeated letter ("d", "dd") moves to the next option starting with that letter, so pressing it
/// again cycles through them; a longer prefix keeps the option shown while it still matches.
fn find(lower: &[String], prefix: &str, from: Option<usize>) -> Option<usize> {
    let n = lower.len();
    if n == 0 {
        return None;
    }
    let mut chars = prefix.chars();
    let first = chars.next()?;
    let (needle, skip) = if chars.all(|c| c == first) { (first.to_string(), 1) } else { (prefix.to_string(), 0) };
    // Without a shown option the search starts at the top, the first match included.
    let start = from.map_or(0, |f| f.saturating_add(skip)) % n;
    (0..n).map(|k| start.saturating_add(k) % n).find(|&i| lower.get(i).is_some_and(|l| l.starts_with(&needle)))
}

#[cfg(test)]
mod tests {
    use super::find;

    /// Actual dropdown text input keeps dotted/dotless prefixes and decomposed I+dot distinct.
    #[test]
    fn azerbaijani_dropdown_type_ahead_handles_both_i_pairs() {
        use egui::accesskit::Role;
        use egui_kittest::{Harness, kittest::Queryable};
        let az = crate::i18n::Lang::from_code("az").unwrap();
        crate::i18n::with_language(az, || {
            let mut h = Harness::builder().with_size(egui::vec2(300.0, 400.0)).build_ui_state(
                |ui, selected: &mut usize| {
                    crate::widgets::dropdown(ui, "az-i-pairs", "Mode", selected, &[(0, "Normal"), (1, "İşıq"), (2, "ıraq")], 120.0);
                },
                0,
            );
            h.get_by_role(Role::ComboBox).click();
            h.run();
            h.event(egui::Event::Text("İ".into()));
            h.run();
            assert_eq!(*h.state(), 1);
            h.event(egui::Event::Text("I".into()));
            h.run();
            assert_eq!(*h.state(), 2);
            h.event(egui::Event::Text("\u{307}".into()));
            h.run();
            assert_eq!(*h.state(), 1, "the combining dot completes the original I prefix");
        });
    }

    fn labels(l: &[&str]) -> Vec<String> {
        l.iter().map(|s| s.to_lowercase()).collect()
    }

    #[test]
    fn a_letter_cycles_and_a_prefix_narrows() {
        let l = labels(&["Normal", "Dissolve", "Darken", "Multiply", "Darker Color", "Lighten", "Linear Burn"]);
        assert_eq!(find(&l, "d", Some(0)), Some(1));
        assert_eq!(find(&l, "dd", Some(1)), Some(2));
        assert_eq!(find(&l, "ddd", Some(2)), Some(4));
        assert_eq!(find(&l, "dddd", Some(4)), Some(1), "wraps to the first match");
        assert_eq!(find(&l, "da", Some(1)), Some(2));
        assert_eq!(find(&l, "darke", Some(2)), Some(2), "keeps the shown option while it matches");
        assert_eq!(find(&l, "darker", Some(2)), Some(4));
        assert_eq!(find(&l, "linear b", Some(5)), Some(6));
        assert_eq!(find(&l, "x", Some(0)), None);
        assert_eq!(find(&l, "d", None), Some(1));
        assert_eq!(find(&[], "d", Some(0)), None);
        assert_eq!(find(&l, "", Some(0)), None);
        assert_eq!(find(&l, "n", Some(usize::MAX)), Some(0), "an index past the end doesn't panic");
    }

    /// Press `key` the way a keyboard does: a key event and the text it types.
    fn type_key<S>(h: &egui_kittest::Harness<'_, S>, key: egui::Key, text: &str) {
        h.key_press(key);
        h.event(egui::Event::Text(text.into()));
    }

    #[test]
    fn letters_pick_options_of_the_open_list_and_enter_closes_it() {
        use egui::accesskit::Role;
        use egui_kittest::{Harness, kittest::Queryable};

        let mut h = Harness::builder().with_size(egui::vec2(300.0, 400.0)).build_ui_state(
            |ui, selected: &mut usize| {
                let options = [(0, "Normal"), (1, "Dissolve"), (2, "Darken"), (3, "Multiply"), (4, "Darker Color")];
                crate::widgets::dropdown(ui, "blend-mode", tl!("Blend Mode"), selected, &options, 120.0);
            },
            0,
        );
        // Typing with the list closed does nothing.
        type_key(&h, egui::Key::D, "d");
        h.run();
        assert_eq!(*h.state(), 0);
        h.get_by_role(Role::ComboBox).click();
        h.run();
        type_key(&h, egui::Key::D, "D");
        h.run();
        assert_eq!(*h.state(), 1, "D picks the first option starting with it");
        type_key(&h, egui::Key::D, "d");
        h.run();
        assert_eq!(*h.state(), 2, "D again picks the next one");
        assert!(super::list_open(&h.ctx), "the list stays open while typing");
        h.key_press(egui::Key::Enter);
        h.run();
        h.run();
        assert_eq!(*h.state(), 2, "↩ keeps the option");
        assert!(!super::list_open(&h.ctx), "↩ closes the list");
        // Opened again, the letters typed before are forgotten: M picks Multiply.
        h.get_by_role(Role::ComboBox).click();
        h.run();
        type_key(&h, egui::Key::M, "m");
        h.run();
        assert_eq!(*h.state(), 3);
    }

    /// Letters typed before the language changed never extend a prefix typed after it, even when
    /// the language comes back within the pause.
    #[test]
    fn a_language_change_forgets_the_letters_typed_before() {
        use egui::accesskit::Role;
        use egui_kittest::{Harness, kittest::Queryable};
        let en = crate::i18n::Lang::EN;
        let de = crate::i18n::Lang::from_code("de").unwrap();
        // Short frames: the letters arrive well within the pause, so only the language change resets.
        let mut h = Harness::builder().with_size(egui::vec2(300.0, 400.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, (selected, lang): &mut (usize, crate::i18n::Lang)| {
                crate::i18n::with_language(*lang, || {
                    let options = [(0, "Normal"), (1, "Dissolve"), (2, "Darken"), (3, "Add")];
                    crate::widgets::dropdown(ui, "lang-switch", "Mode", selected, &options, 120.0);
                });
            },
            (0, en),
        );
        h.get_by_role(Role::ComboBox).click();
        h.run();
        type_key(&h, egui::Key::D, "d");
        h.run();
        assert_eq!(h.state().0, 1);
        h.state_mut().1 = de;
        h.step();
        h.state_mut().1 = en;
        h.step();
        assert!(super::list_open(&h.ctx), "the list stays open across the language change");
        type_key(&h, egui::Key::A, "a");
        h.run();
        assert_eq!(h.state().0, 3, "a starts a new prefix (Add); the d typed before the change would make it da (Darken)");
    }

    #[test]
    fn case_and_scripts_beyond_ascii_match() {
        let l = labels(&["Normal", "Éclaircir", "Ébène", "Нормальный", "Затемнение"]);
        assert_eq!(find(&l, "é", Some(0)), Some(1));
        assert_eq!(find(&l, "éé", Some(1)), Some(2));
        assert_eq!(find(&l, "з", Some(0)), Some(4));
        assert_eq!(find(&l, "но", Some(4)), Some(3));
    }
}
