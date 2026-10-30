//! Photoshop keyboard shortcut sets (`.kys`, Edit › Keyboard Shortcuts › Save Set): parsing and
//! matching rows to our commands (#1163). `edit.keyboardShortcuts {"importKys": <xml>}` applies a
//! set here; the app's Keyboard Shortcuts dialog fills its overrides from the same plan
//! (`photocraft_ui_egui::kys_import`).
//!
//! A `.kys` is XML: `<command kind="static|dynamic" name="Gaussian Blur...">` rows, each with
//! one or more `<shortcut>Control+Opt+Cmd+G</shortcut>`, then the `<tool>` keys. Photoshop
//! names the command by its menu label alone, so rows are matched to our commands by label.
//! Where one label sits in several menus (Levels… under Image › Adjustments and Layer › New
//! Adjustment Layer; All under Select, Edit › Purge, View › Show), the command whose default is
//! one of the imported keys wins, then the one that has a default at all, then the first
//! candidate (the app lists them in Photoshop's menu order). A row's first shortcut becomes the
//! command's main key, unless one of its keys is already the command's default (Zoom In lists
//! `Cmd++` before `Cmd+=`); its other keys become the command's alternates (#3665: Copy is
//! `Cmd+C` and `F3`, and importing it must not leave `Cmd+C` unbound). Tool keys match our
//! tools by name ([`plan_tools`]; the app knows the tools). A set saved on Windows spells the
//! primary modifier Ctrl where a Mac set says Cmd ([`KysSet::windows`], [`to_ours`]).
//! [`write`] saves a set the same way, with our command id on each row (`pc-id`, which
//! Photoshop ignores), so PhotoCraft reads its own sets back exactly.

use std::collections::{BTreeMap, HashMap};

use crate::prefs::normalize_shortcut;

/// A set is a few tens of kilobytes; anything far larger is not one and is not parsed.
pub const MAX_BYTES: usize = 4 << 20;

/// One `<command>` row: Photoshop's label and its shortcuts, first one primary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KysCommand {
    pub name: String,
    pub shortcuts: Vec<String>,
    /// Our command id, in sets PhotoCraft wrote ([`write`]'s `pc-id`): matched before the label.
    pub id: Option<String>,
}

/// The parsed set: its name, its command rows and how many tools carry a key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KysSet {
    /// The set's name as Photoshop shows it (`texcuts`), from the root's `filename` attribute,
    /// which a saved set spells `$$$/FileName/Presets/KeyboardCustomization/texcuts=texcuts`.
    pub name: Option<String>,
    pub commands: Vec<KysCommand>,
    pub tool_keys: usize,
    /// The `<tool>` rows with a key: the tool's name, its key, our id in sets PhotoCraft wrote.
    pub tools: Vec<KysCommand>,
    /// Saved by Photoshop on Windows: no key spells ⌘ or Opt, and Ctrl is the primary modifier,
    /// so `Ctrl` here is our `Cmd` (a Mac set's `Control` stays the Control key). See [`to_ours`].
    pub windows: bool,
}

/// The set name in the root `filename` attribute (`$$$/.../{name}={name}`). The name may itself
/// contain `/` or `=`, so split on the key prefix and the doubled name, not on the last separator.
fn filename_to_name(f: &str) -> &str {
    let Some(rest) = f.strip_prefix("$$$/") else { return f.rsplit(['=', '/']).next().unwrap_or(f).trim() };
    let rest = rest.strip_prefix("FileName/Presets/KeyboardCustomization/").unwrap_or(rest);
    let mid = rest.len() / 2;
    if rest.len() % 2 == 1 && rest.is_char_boundary(mid) && rest.as_bytes()[mid] == b'=' && rest[..mid] == rest[mid + 1..] {
        return rest[..mid].trim();
    }
    rest.split_once('=').map_or(rest, |(_, n)| n).trim()
}

/// Parse a `.kys` file (or Photoshop's live `Keyboard Shortcuts.psp`, the same XML). Rows
/// without a name or a shortcut are skipped.
pub fn parse(text: &str) -> Result<KysSet, String> {
    if text.len() > MAX_BYTES {
        return Err("not a keyboard shortcut set (.kys): larger than 4 MiB".into());
    }
    let text = text.trim_start_matches('\u{feff}');
    let doc = photocraft_io::xml::parse(text, roxmltree::ParsingOptions::default()).map_err(|e| format!("not a keyboard shortcut set (.kys): {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "photoshop-keyboard-shortcuts" {
        return Err("not a keyboard shortcut set (.kys): no <photoshop-keyboard-shortcuts> root".into());
    }
    let name = root.attribute("filename").map(filename_to_name).filter(|n| !n.is_empty()).map(str::to_string);
    let mut set = KysSet { name, ..Default::default() };
    for node in root.children().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "command" => {
                let Some(name) = node.attribute("name").map(str::trim).filter(|n| !n.is_empty()) else { continue };
                let shortcuts: Vec<String> = node
                    .children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "shortcut")
                    .filter_map(|n| n.text().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string))
                    .collect();
                if !shortcuts.is_empty() {
                    let id = node.attribute(PC_ID).map(str::trim).filter(|i| !i.is_empty()).map(str::to_string);
                    set.commands.push(KysCommand { name: name.to_string(), shortcuts, id });
                }
            }
            "tool" => {
                let key = node.text().map(str::trim).filter(|t| !t.is_empty());
                let name = node.attribute("name").map(str::trim).filter(|n| !n.is_empty());
                if let Some(key) = key {
                    set.tool_keys += 1;
                    if let Some(name) = name {
                        let id = node.attribute(PC_ID).map(str::trim).filter(|i| !i.is_empty()).map(str::to_string);
                        set.tools.push(KysCommand { name: name.to_string(), shortcuts: vec![key.to_string()], id });
                    }
                }
            }
            _ => {}
        }
    }
    set.windows = windows_set(&set);
    Ok(set)
}

/// The modifier words of `sc` (`Ctrl+Alt+K` → Ctrl, Alt), whatever the key is.
fn modifiers(sc: &str) -> impl Iterator<Item = String> + '_ {
    let mods = sc.strip_suffix("++").unwrap_or_else(|| sc.rsplit_once('+').map_or("", |(m, _)| m));
    mods.split('+').filter(|m| !m.is_empty()).map(|m| m.trim().to_ascii_lowercase())
}

/// Photoshop on Windows writes Ctrl for the primary modifier, where a Mac set writes Cmd (and
/// Control for the Control key): a set with no ⌘ or Opt anywhere but some Ctrl is a Windows set.
fn windows_set(set: &KysSet) -> bool {
    let keys = || set.commands.iter().flat_map(|c| c.shortcuts.iter());
    let mac = keys().flat_map(|k| modifiers(k).collect::<Vec<_>>()).any(|m| matches!(m.as_str(), "cmd" | "command" | "opt" | "option"));
    let ctrl = keys().flat_map(|k| modifiers(k).collect::<Vec<_>>()).any(|m| matches!(m.as_str(), "ctrl" | "control"));
    !mac && ctrl
}

/// A key of `set` in our notation, normalised: on a Windows set, Ctrl (or Control) is our `Cmd`
/// (Ctrl on Windows and Linux, ⌘ on the Mac), so `Ctrl+L` is the `Cmd+L` default rather than a
/// literal Control key.
pub fn to_ours(set: &KysSet, sc: &str) -> Option<String> {
    if !set.windows {
        return normalize_shortcut(sc);
    }
    let (mods, key) = match sc.strip_suffix("++") {
        Some(m) => (m, "+"),
        None if sc == "+" => ("", "+"),
        None => sc.rsplit_once('+').unwrap_or(("", sc)),
    };
    let mods: Vec<&str> = mods
        .split('+')
        .filter(|m| !m.is_empty())
        .map(|m| if matches!(m.trim().to_ascii_lowercase().as_str(), "ctrl" | "control") { "Cmd" } else { m })
        .collect();
    normalize_shortcut(&mods.into_iter().chain(std::iter::once(key)).collect::<Vec<_>>().join("+"))
}

/// What an import would change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Command id → the normalised shortcut from the file, for every matched row.
    pub set: BTreeMap<String, String>,
    /// Photoshop names with no command here (scripts, Bridge, the macOS application menu…).
    pub unknown: Vec<String>,
    /// Rows whose shortcut could not be read (`unknown` does not list them).
    pub unreadable: Vec<String>,
    /// Second and later shortcuts of the matched rows (counted; kept in `alternate_keys`).
    pub alternates: usize,
    /// Command id → the matched row's other keys (normalised, without the main key in `set`),
    /// for every matched row: a set lists each command's whole binding, so an empty list
    /// means the command has no alternates in it.
    pub alternate_keys: BTreeMap<String, Vec<String>>,
}

/// One command a row may match: its id, its label and its default shortcut.
pub struct Candidate {
    pub id: String,
    pub label: String,
    pub default: Option<String>,
}

/// Photoshop's label and ours compared loosely: case, a trailing ellipsis (`...` or `…`) and
/// surrounding space do not count.
fn key(label: &str) -> String {
    let l = label.trim().replace("...", "…");
    l.trim_end_matches('…').trim().to_lowercase()
}

/// The engine's own candidates: every command in a menu (the engine's, then the shell's), in
/// command order.
pub fn command_candidates() -> Vec<Candidate> {
    let engine = crate::command_specs().iter().map(|c| (c.id, c.label, c.menu, c.shortcut));
    engine
        .chain(crate::shell_commands::SHELL_COMMANDS.iter().copied())
        .filter(|c| !c.2.is_empty())
        .map(|(id, label, _, default)| Candidate { id: id.to_string(), label: label.to_string(), default: default.map(str::to_string) })
        .collect()
}

/// Match the rows to `candidates` (see the module notes for the tie-breaks). One command may be
/// listed under several labels (the menu's Settings… is the command's General…); under each
/// label it counts once.
pub fn plan(candidates: impl IntoIterator<Item = Candidate>, set: &KysSet) -> Plan {
    let mut by_label: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();
    for c in candidates {
        let entry = by_label.entry(key(&c.label)).or_default();
        if !entry.iter().any(|(id, _)| *id == c.id) {
            entry.push((c.id, c.default));
        }
    }
    let by_id: HashMap<&str, &(String, Option<String>)> = by_label.values().flatten().map(|c| (c.0.as_str(), c)).collect();
    let mut plan = Plan::default();
    for row in &set.commands {
        // A set PhotoCraft wrote names the command itself, whatever its label says.
        let own = row.id.as_deref().and_then(|id| by_id.get(id)).map(|c| vec![(*c).clone()]);
        let Some(candidates) = own.as_ref().or_else(|| by_label.get(&key(&row.name))) else {
            plan.unknown.push(row.name.clone());
            continue;
        };
        let Some(sc) = row.shortcuts.first().and_then(|s| to_ours(set, s)) else {
            plan.unreadable.push(row.name.clone());
            continue;
        };
        plan.alternates += row.shortcuts.len().saturating_sub(1);
        let imported: Vec<String> = row.shortcuts.iter().filter_map(|s| to_ours(set, s)).collect();
        let pick = candidates
            .iter()
            .find(|(_, d)| d.as_deref().and_then(normalize_shortcut).is_some_and(|d| imported.contains(&d)))
            .or_else(|| candidates.iter().find(|(_, d)| d.is_some()))
            .or_else(|| candidates.first());
        if let Some((id, default)) = pick {
            // A row that lists the command's own default among its keys keeps the default
            // (Zoom In is "Cmd++" then "Cmd+=" in Photoshop; ours is Cmd+=).
            let default = default.as_deref().and_then(normalize_shortcut);
            let sc = default.filter(|d| imported.contains(d)).unwrap_or(sc);
            let mut rest: Vec<String> = Vec::new();
            for k in imported {
                if k != sc && !rest.contains(&k) && rest.len() < crate::prefs::MAX_ALTERNATES {
                    rest.push(k);
                }
            }
            plan.alternate_keys.insert(id.clone(), rest);
            plan.set.insert(id.clone(), sc);
        }
    }
    plan
}

/// The tool keys of `set` matched to `tools` (our tool id and Photoshop's name for it, which is
/// ours too: "Move Tool"): tool id → key (normalised: a letter, or an F-key), and the names with no
/// tool here. A row PhotoCraft wrote names the tool by id. A key with modifiers is skipped.
pub fn plan_tools<'a>(tools: impl IntoIterator<Item = (&'a str, &'a str)>, set: &KysSet) -> (BTreeMap<String, String>, Vec<String>) {
    let tools: Vec<(&str, &str)> = tools.into_iter().collect();
    let mut keys = BTreeMap::new();
    let mut unknown = Vec::new();
    for row in &set.tools {
        let found =
            row.id.as_deref().and_then(|id| tools.iter().find(|(t, _)| *t == id)).or_else(|| tools.iter().find(|(_, name)| key(name) == key(&row.name)));
        let Some((id, _)) = found else {
            unknown.push(row.name.clone());
            continue;
        };
        // One key without modifiers (Photoshop's letters; our tools may also take an F-key).
        let key = row.shortcuts.first().and_then(|k| normalize_shortcut(k.trim()));
        if let Some(k) = key.filter(|k| !k.contains('+') || k == "+") {
            keys.insert((*id).to_string(), k);
        }
    }
    (keys, unknown)
}

/// The attribute naming our command id on the rows PhotoCraft writes (Photoshop ignores it).
pub const PC_ID: &str = "pc-id";

/// One row to write: a command (or tool), its label and its keys, main key first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub label: String,
    pub tool: bool,
    pub keys: Vec<String>,
}

/// A key in the spelling Photoshop's macOS sets use (`Control+Opt+Shift+Cmd+G`): our `Ctrl` is
/// Control, `Alt` is Opt. `None` for a key that isn't one.
pub fn photoshop_key(sc: &str) -> Option<String> {
    let n = normalize_shortcut(sc)?;
    // The key is the last part; `Cmd++` ends in an empty part, so take it from the string.
    let (mods, key) = match n.strip_suffix("++") {
        Some(m) => (m, "+"),
        None if n == "+" => ("", "+"),
        None => n.rsplit_once('+').unwrap_or(("", n.as_str())),
    };
    let has = |m: &str| mods.split('+').any(|x| x == m);
    let mut out: Vec<&str> =
        [("Ctrl", "Control"), ("Alt", "Opt"), ("Shift", "Shift"), ("Cmd", "Cmd")].iter().filter(|(ours, _)| has(ours)).map(|(_, ps)| *ps).collect();
    out.push(key);
    Some(out.join("+"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// A `.kys` file for the set `name` (Edit › Keyboard Shortcuts › Save Set): every entry with a
/// key, commands then tools, labels with Photoshop's `...`, keys in [`photoshop_key`] spelling,
/// and our id on each row so PhotoCraft reads its own sets back exactly. Rows without a key are
/// left out, and a key that isn't one is skipped.
pub fn write(name: &str, entries: &[Entry]) -> String {
    let name = xml_escape(name.trim());
    let mut out = format!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<photoshop-keyboard-shortcuts version=\"4\" filename=\"$$$/FileName/Presets/KeyboardCustomization/{name}={name}\">\n"
    );
    let row = |e: &Entry| -> Option<String> {
        let keys: Vec<String> = e.keys.iter().filter_map(|k| photoshop_key(k)).collect();
        let first = keys.first()?;
        let label = xml_escape(&e.label.replace('…', "..."));
        let id = xml_escape(&e.id);
        Some(if e.tool {
            format!("\t<tool name=\"{label}\" {PC_ID}=\"{id}\">{}</tool>\n", xml_escape(first))
        } else {
            let shortcuts: String = keys.iter().map(|k| format!("<shortcut>{}</shortcut>", xml_escape(k))).collect();
            format!("\t<command kind=\"static\" name=\"{label}\" {PC_ID}=\"{id}\">{shortcuts}</command>\n")
        })
    };
    for e in entries.iter().filter(|e| !e.tool).chain(entries.iter().filter(|e| e.tool)) {
        if let Some(r) = row(e) {
            out.push_str(&r);
        }
    }
    out.push_str("</photoshop-keyboard-shortcuts>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_name_with_separators_round_trips() {
        for n in ["Retouch 1/2", "a=b", "x/y=z"] {
            assert_eq!(parse(&write(n, &[])).unwrap().name.as_deref(), Some(n));
        }
    }

    const SET: &str = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<photoshop-keyboard-shortcuts version=\"4\" filename=\"$$$/FileName/Presets/KeyboardCustomization/texcuts=texcuts\">
\t<command kind=\"static\" name=\"Levels...\" id=\"1801\"><shortcut>Cmd+L</shortcut></command>
\t<command kind=\"static\" name=\"Undo\" id=\"101\"><shortcut>Cmd+Z</shortcut><shortcut>F1</shortcut></command>
\t<command kind=\"static\" name=\"Hide Photoshop\" id=\"7001\"><shortcut>Cmd+H</shortcut></command>
\t<command kind=\"static\" name=\"Odd\" id=\"1\"><shortcut>Cmd+Shift+Ctrl</shortcut></command>
\t<tool name=\"Move Tool\" type=\"1\">V</tool>
</photoshop-keyboard-shortcuts>
";

    fn c(id: &str, label: &str, default: Option<&str>) -> Candidate {
        Candidate { id: id.into(), label: label.into(), default: default.map(str::to_string) }
    }

    #[test]
    fn a_bare_plus_key_is_kept_when_saving() {
        assert_eq!(photoshop_key("+").as_deref(), Some("+"));
        assert_eq!(photoshop_key("Cmd++").as_deref(), Some("Cmd++"));
        let e = Entry { id: "tools.zoom".into(), label: "Zoom Tool".into(), tool: true, keys: vec!["+".into()] };
        assert!(write("Mine", &[e]).contains(">+</tool>"));
    }

    #[test]
    fn a_bare_plus_command_survives_a_windows_set_with_a_ctrl_row() {
        let xml = "<photoshop-keyboard-shortcuts>\
            <command name=\"Copy\"><shortcut>Ctrl+C</shortcut></command>\
            <command name=\"Zoom In\"><shortcut>+</shortcut></command>\
            </photoshop-keyboard-shortcuts>";
        let set = parse(xml).unwrap();
        assert!(set.windows);
        assert_eq!(to_ours(&set, "+").as_deref(), Some("+"));
        let p = plan([c("edit.copy", "Copy", Some("Cmd+C")), c("view.zoomIn", "Zoom In", None)], &set);
        assert_eq!(p.set.get("view.zoomIn").map(String::as_str), Some("+"));
        assert!(p.unreadable.is_empty());
    }

    #[test]
    fn rows_match_by_label_and_the_default_holder_wins_a_shared_label() {
        let set = parse(SET).unwrap();
        assert_eq!(set.name.as_deref(), Some("texcuts"));
        assert_eq!(set.tool_keys, 1);
        let p = plan(
            [c("layer.newLevels", "Levels…", None), c("image.levels", "Levels…", Some("Cmd+L")), c("edit.undo", "Undo", Some("Cmd+Z")), c("odd", "Odd", None)],
            &set,
        );
        assert_eq!(p.set.get("image.levels").map(String::as_str), Some("Cmd+L"));
        assert!(!p.set.contains_key("layer.newLevels"));
        assert_eq!(p.unknown, ["Hide Photoshop"]);
        assert_eq!(p.unreadable, ["Odd"]);
        assert_eq!(p.alternates, 1);
        // #3665: the row's second key is kept as the command's alternate, not dropped.
        assert_eq!(p.alternate_keys.get("edit.undo"), Some(&vec!["F1".to_string()]));
        assert_eq!(p.alternate_keys.get("image.levels"), Some(&Vec::new()));
    }

    /// A set nested 20 000 deep overflowed the XML parser's recursion (an abort nothing can
    /// catch); it is an error now. The small stack makes a regression abort instead of passing.
    #[test]
    fn deeply_nested_sets_are_an_error_not_a_stack_overflow() {
        let xml = format!("<photoshop-keyboard-shortcuts>{}{}</photoshop-keyboard-shortcuts>", "<a>".repeat(20_000), "</a>".repeat(20_000));
        let e = std::thread::Builder::new().stack_size(512 * 1024).spawn(move || parse(&xml).err()).unwrap().join().unwrap();
        assert!(e.as_deref().is_some_and(|m| m.contains("nested")), "{e:?}");
    }

    #[test]
    fn a_row_with_the_default_and_a_function_key_keeps_both() {
        let xml =
            "<photoshop-keyboard-shortcuts><command name=\"Copy\"><shortcut>F3</shortcut><shortcut>Cmd+C</shortcut></command></photoshop-keyboard-shortcuts>";
        let p = plan([c("edit.copy", "Copy", Some("Cmd+C"))], &parse(xml).unwrap());
        assert_eq!(p.set.get("edit.copy").map(String::as_str), Some("Cmd+C"));
        assert_eq!(p.alternate_keys.get("edit.copy"), Some(&vec!["F3".to_string()]));
    }

    #[test]
    fn oversized_input_is_refused_before_parsing() {
        let big = format!("<photoshop-keyboard-shortcuts>{}</photoshop-keyboard-shortcuts>", " ".repeat(MAX_BYTES));
        assert!(parse(&big).unwrap_err().contains("4 MiB"));
        assert!(parse("<other/>").unwrap_err().contains("not a keyboard shortcut set"));
    }

    #[test]
    fn written_sets_read_back_exactly_and_keys_use_photoshops_spelling() {
        assert_eq!(photoshop_key("Cmd+Alt+Shift+Ctrl+g").as_deref(), Some("Control+Opt+Shift+Cmd+G"));
        assert_eq!(photoshop_key("Cmd++").as_deref(), Some("Cmd++"));
        assert_eq!(photoshop_key("F3").as_deref(), Some("F3"));
        assert_eq!(photoshop_key("Cmd+Shift+Ctrl"), None);
        let entries = vec![
            Entry { id: "edit.copy".into(), label: "Copy".into(), tool: false, keys: vec!["Cmd+C".into(), "F3".into()] },
            Entry { id: "x.odd".into(), label: "Levels…".into(), tool: false, keys: vec!["Cmd+Alt+L".into()] },
            Entry { id: "tools.select.move".into(), label: "Move Tool".into(), tool: true, keys: vec!["V".into()] },
            Entry { id: "none".into(), label: "Unbound".into(), tool: false, keys: vec![] },
            Entry { id: "bad".into(), label: "A <\"weird\"> & 'name'".into(), tool: false, keys: vec!["Cmd+Shift+Ctrl".into(), "F9".into()] },
        ];
        let xml = write(" My & Set ", &entries);
        assert!(xml.contains("KeyboardCustomization/My &amp; Set=My &amp; Set"), "{xml}");
        assert!(xml.contains("<command kind=\"static\" name=\"Levels...\" pc-id=\"x.odd\"><shortcut>Opt+Cmd+L</shortcut></command>"), "{xml}");
        assert!(!xml.contains("Unbound"), "{xml}");
        assert!(xml.find("Move Tool").unwrap() > xml.find("Levels").unwrap(), "tools after commands");
        let set = parse(&xml).unwrap();
        assert_eq!(set.name.as_deref(), Some("My & Set"));
        assert_eq!(set.tool_keys, 1);
        assert_eq!(set.commands.len(), 3);
        assert_eq!(set.commands[0], KysCommand { name: "Copy".into(), shortcuts: vec!["Cmd+C".into(), "F3".into()], id: Some("edit.copy".into()) });
        assert_eq!(set.commands[2].name, "A <\"weird\"> & 'name'");
        assert_eq!(set.commands[2].shortcuts, vec!["F9".to_string()]);
        // The id wins over the label: a renamed or translated label still finds its command.
        let cands = || {
            vec![
                Candidate { id: "x.odd".into(), label: "Something Else".into(), default: None },
                Candidate { id: "y.levels".into(), label: "Levels".into(), default: Some("Cmd+L".into()) },
            ]
        };
        let p = plan(cands(), &set);
        assert_eq!(p.set.get("x.odd").map(String::as_str), Some("Cmd+Alt+L"));
        assert!(!p.set.contains_key("y.levels"));
        // Without the id, the label matches as before.
        let mut plain = set.clone();
        plain.commands[1].id = None;
        assert!(plan(cands(), &plain).set.contains_key("y.levels"));
    }

    #[test]
    fn a_windows_set_means_cmd_by_ctrl_and_a_mac_set_keeps_control() {
        let win = "<photoshop-keyboard-shortcuts version=\"4\">
\t<command name=\"Levels...\"><shortcut>Ctrl+L</shortcut></command>
\t<command name=\"Gaussian Blur...\"><shortcut>Ctrl+Alt+Shift+G</shortcut></command>
\t<command name=\"Zoom In\"><shortcut>Ctrl++</shortcut><shortcut>Ctrl+=</shortcut></command>
</photoshop-keyboard-shortcuts>";
        let set = parse(win).unwrap();
        assert!(set.windows);
        assert_eq!(to_ours(&set, "Ctrl+L").as_deref(), Some("Cmd+L"));
        assert_eq!(to_ours(&set, "Ctrl+Alt+Shift+G").as_deref(), normalize_shortcut("Cmd+Alt+Shift+G").as_deref());
        assert_eq!(to_ours(&set, "Ctrl++").as_deref(), Some("Cmd++"));
        assert_eq!(to_ours(&set, "F3").as_deref(), Some("F3"));
        let cands = vec![Candidate { id: "image.levels".into(), label: "Levels…".into(), default: Some("Cmd+L".into()) }];
        assert_eq!(plan(cands, &set).set.get("image.levels").map(String::as_str), Some("Cmd+L"), "the default, not a Control override");
        // A Mac set spells ⌘ Cmd: its Control is the Control key.
        let mac = parse(
            "<photoshop-keyboard-shortcuts version=\"4\"><command name=\"Levels...\"><shortcut>Cmd+L</shortcut></command><command name=\"X\"><shortcut>Control+Opt+Cmd+G</shortcut></command><command name=\"Y\"><shortcut>Control+K</shortcut></command></photoshop-keyboard-shortcuts>",
        )
        .unwrap();
        assert!(!mac.windows);
        assert_eq!(to_ours(&mac, "Control+K"), normalize_shortcut("Ctrl+K"));
        // No modifiers at all: nothing to tell, nothing to change.
        let plain = parse("<photoshop-keyboard-shortcuts><command name=\"Copy\"><shortcut>F3</shortcut></command></photoshop-keyboard-shortcuts>").unwrap();
        assert!(!plain.windows);
    }

    #[test]
    fn tool_keys_match_by_name_or_id_and_take_one_letter() {
        let xml = "<photoshop-keyboard-shortcuts version=\"4\">
\t<tool name=\"Move Tool\" type=\"1\">V</tool>
\t<tool name=\"pencil tool\" type=\"1\">n</tool>
\t<tool name=\"Renamed\" pc-id=\"tools.select.brush\">B</tool>
\t<tool name=\"Imaginary Tool\">Q</tool>
\t<tool name=\"Lasso Tool\">Shift+L</tool>
\t<tool name=\"Default Colors\"></tool>
\t<tool>Z</tool>
</photoshop-keyboard-shortcuts>";
        let set = parse(xml).unwrap();
        assert_eq!((set.tool_keys, set.tools.len()), (6, 5), "a key without a name counts but can't be matched");
        let tools = [
            ("tools.select.move", "Move Tool"),
            ("tools.select.pencil", "Pencil Tool"),
            ("tools.select.brush", "Brush Tool"),
            ("tools.select.lasso", "Lasso Tool"),
        ];
        let (keys, unknown) = plan_tools(tools, &set);
        assert_eq!(keys.get("tools.select.move").map(String::as_str), Some("V"));
        assert_eq!(keys.get("tools.select.pencil").map(String::as_str), Some("N"), "case-insensitive, upper-cased");
        assert_eq!(keys.get("tools.select.brush").map(String::as_str), Some("B"), "by pc-id");
        assert!(!keys.contains_key("tools.select.lasso"), "a tool key has no modifiers");
        assert_eq!(unknown, vec!["Imaginary Tool".to_string()]);
        // Our own export reads back by id.
        let out = write("T", &[Entry { id: "tools.select.pencil".into(), label: "Pencil Tool".into(), tool: true, keys: vec!["F".into()] }]);
        let (keys, _) = plan_tools(tools, &parse(&out).unwrap());
        assert_eq!(keys.get("tools.select.pencil").map(String::as_str), Some("F"));
    }
}
