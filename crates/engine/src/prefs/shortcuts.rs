use super::{BTreeMap, Value};

/// Most further keys one command may hold; Photoshop's own defaults use one.
pub const MAX_ALTERNATES: usize = 8;

/// Validate and normalise a command's alternate keys: a list of shortcut strings (a single
/// string is a one-key list, `""` none).
pub fn alternate_keys(v: &Value) -> std::result::Result<Vec<String>, String> {
    let items: Vec<&Value> = match v {
        Value::String(s) if s.is_empty() => Vec::new(),
        Value::String(_) => vec![v],
        Value::Array(a) => a.iter().collect(),
        _ => return Err("alternate shortcuts are a list of shortcuts (e.g. [\"F3\"])".into()),
    };
    if items.len() > MAX_ALTERNATES {
        return Err(format!("at most {MAX_ALTERNATES} alternate shortcuts (got {})", items.len()));
    }
    let mut out: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        let s = item.as_str().ok_or_else(|| format!("alternate shortcut `{item}` must be a string"))?;
        let n = normalize_shortcut(s).ok_or_else(|| format!("`{s}` is not a valid shortcut (e.g. Cmd+Shift+N, F7)"))?;
        if !out.contains(&n) {
            out.push(n);
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ shortcuts

/// Canonical spelling of a shortcut: modifiers in `Cmd+Ctrl+Alt+Shift` order, key last
/// (single characters upper-cased). `None` if it has no key or an unknown modifier.
pub fn normalize_shortcut(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (mut cmd, mut ctrl, mut alt, mut shift) = (false, false, false, false);
    let mut key: Option<String> = None;
    // A trailing "+" key is spelled "Cmd++".
    let (body, plus_key) = if s.ends_with("++") || s == "+" { (s.trim_end_matches('+'), true) } else { (s, false) };
    for part in body.split('+').filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "⌘" | "meta" | "super" => cmd = true,
            "ctrl" | "control" | "⌃" => ctrl = true,
            "alt" | "option" | "opt" | "⌥" => alt = true,
            "shift" | "⇧" => shift = true,
            _ => {
                if key.is_some() {
                    return None;
                }
                let k = if part.chars().count() == 1 { part.to_uppercase() } else { capitalize(part) };
                key = Some(k);
            }
        }
    }
    if plus_key {
        if key.is_some() {
            return None;
        }
        key = Some("+".into());
    }
    let key = key?;
    let mut out = Vec::new();
    if cmd {
        out.push("Cmd");
    }
    if ctrl {
        out.push("Ctrl");
    }
    if alt {
        out.push("Alt");
    }
    if shift {
        out.push("Shift");
    }
    let mut s = out.join("+");
    if !s.is_empty() {
        s.push('+');
    }
    s.push_str(&key);
    Some(s)
}

/// Key names spelled with more than one capital (egui's `Key::name`): Title case would turn
/// `PageUp` into `Pageup`, which no key matches, so a binding saved that way never fired (#6827).
const MULTI_WORD_KEYS: [&str; 8] =
    ["PageUp", "PageDown", "OpenBracket", "CloseBracket", "OpenCurlyBracket", "CloseCurlyBracket", "BrowserBack", "IntlBackslash"];

fn capitalize(s: &str) -> String {
    if let Some(k) = MULTI_WORD_KEYS.iter().find(|k| k.eq_ignore_ascii_case(s)) {
        return (*k).to_string();
    }
    let mut c = s.chars();
    match c.next() {
        Some(f) if f.is_ascii_alphabetic() => {
            let rest: String = c.collect();
            // Function keys stay upper-case (F1…F24); named keys are Title case.
            if f.eq_ignore_ascii_case(&'f') && rest.chars().all(|c| c.is_ascii_digit()) && !rest.is_empty() {
                format!("F{rest}")
            } else {
                format!("{}{}", f.to_ascii_uppercase(), rest.to_ascii_lowercase())
            }
        }
        _ => s.to_string(),
    }
}

/// Hold-and-release gestures (#249): temporary tools that last while their key is held and give
/// the previous tool back on release. They are bound like shortcuts (Edit › Keyboard Shortcuts ›
/// Tools › Temporary, `edit.keyboardShortcuts`) but held rather than pressed, so they are not
/// commands; the UI reads their keys through [`Preferences::shortcut`]. While a selection or
/// shape is being dragged, the Hand key repositions it instead of panning (Photoshop's Space).
/// The erase key keeps the Brush or Pencil and makes its strokes erase (Photoshop's `~`).
pub const TEMPORARY_TOOLS: &[(&str, &str, &str)] = &[
    ("tools.temporary.hand", "Hand Tool (hold)", "Space"),
    ("tools.temporary.zoomIn", "Zoom In (hold, drag to scrub)", "Cmd+Space"),
    ("tools.temporary.zoomOut", "Zoom Out (hold)", "Alt+Space"),
    ("tools.temporary.erase", "Erase with Brush or Pencil (hold)", "`"),
];

/// Photoshop's second default keys (#3665): (command, key). Each command keeps its main key
/// (the one its menu shows) and answers these as well, as in Photoshop's default set: the
/// legacy function keys F2 Cut, F3 Copy, F4 Paste and Shift+F7 Inverse; Shift+Backspace
/// (Shift+Delete on a Mac) for Fill; ⌘+ beside ⌘= for Zoom In; and off the Mac, Ctrl+F4 beside
/// Ctrl+W for Close (Windows' close-document key; macOS keeps Ctrl+F4 for window focus).
pub const ALTERNATE_SHORTCUTS: &[(&str, &str)] = &[
    ("edit.cut", "F2"),
    ("edit.copy", "F3"),
    ("edit.paste", "F4"),
    ("edit.fill", "Shift+Backspace"),
    ("select.inverse", "Shift+F7"),
    ("view.zoomIn", "Cmd++"),
    #[cfg(not(target_os = "macos"))]
    ("file.close", "Cmd+F4"),
];

/// A command's default alternate keys ([`ALTERNATE_SHORTCUTS`]).
pub fn default_alternates(id: &str) -> impl Iterator<Item = &'static str> + '_ {
    ALTERNATE_SHORTCUTS.iter().filter(move |(c, _)| *c == id).map(|(_, sc)| *sc)
}

/// Keys the system takes before the app sees them (switching apps, closing the window, force
/// quit): `edit.keyboardShortcuts` won't bind them, as Photoshop won't. `sc` is in our notation,
/// where `Cmd` is Ctrl off the Mac.
pub fn reserved_shortcut(sc: &str) -> bool {
    let Some(n) = normalize_shortcut(sc) else { return false };
    let reserved: &[&str] = if cfg!(target_os = "macos") {
        &["Cmd+Tab", "Cmd+Shift+Tab", "Cmd+Alt+Escape", "Cmd+`"]
    } else if cfg!(target_os = "windows") {
        &["Alt+Tab", "Alt+Shift+Tab", "Alt+F4", "Cmd+Alt+Delete", "Cmd+Escape", "Cmd+Shift+Escape"]
    } else {
        // Linux and the BSDs: window switching and closing, and the virtual consoles.
        &[
            "Alt+Tab",
            "Alt+Shift+Tab",
            "Alt+F4",
            "Cmd+Alt+Delete",
            "Cmd+Alt+F1",
            "Cmd+Alt+F2",
            "Cmd+Alt+F3",
            "Cmd+Alt+F4",
            "Cmd+Alt+F5",
            "Cmd+Alt+F6",
            "Cmd+Alt+F7",
            "Cmd+Alt+F8",
            "Cmd+Alt+F9",
            "Cmd+Alt+F10",
            "Cmd+Alt+F11",
            "Cmd+Alt+F12",
        ]
    };
    reserved.iter().filter_map(|r| normalize_shortcut(r)).any(|r| r == n)
}

/// What a bindable id is: a command (the engine's or the shell's), a tool, or a held tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindKind {
    Command,
    Tool,
    Hold,
}

/// Every bindable id: (id, label, menu path, default key, kind). The engine's commands, the
/// shell's ([`crate::shell_commands::SHELL_COMMANDS`]), the tools and the held temporary tools,
/// so Edit › Keyboard Shortcuts works the same with or without the app's UI.
pub fn bindable_items() -> impl Iterator<Item = (&'static str, &'static str, &'static [&'static str], Option<&'static str>, BindKind)> {
    let engine = crate::command_specs().iter().map(|c| (c.id, c.label, c.menu, c.shortcut, BindKind::Command));
    let shell = crate::shell_commands::SHELL_COMMANDS.iter().map(|c| (c.0, c.1, c.2, c.3, BindKind::Command));
    let tools = crate::shell_commands::TOOLS.iter().map(|t| (t.0, t.1, &["Tools"][..], t.2, BindKind::Tool));
    let hold = TEMPORARY_TOOLS.iter().map(|t| (t.0, t.1, &["Tools", "Temporary"][..], Some(t.2), BindKind::Hold));
    engine.chain(shell).chain(tools).chain(hold)
}

/// Every bindable id with its default.
pub(super) fn bindable() -> impl Iterator<Item = (&'static str, Option<&'static str>)> {
    bindable_items().map(|i| (i.0, i.3))
}

pub(super) fn is_tool(id: &str) -> bool {
    crate::shell_commands::tool_default(id).is_some()
}

/// Shortcuts bound to more than one command: (shortcut, ids). `bindings` are (id, shortcut).
pub fn conflicts<'a>(bindings: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<(String, Vec<String>)> {
    let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, sc) in bindings {
        if let Some(n) = normalize_shortcut(sc) {
            by.entry(n).or_default().push(id.to_string());
        }
    }
    by.into_iter().filter(|(_, ids)| ids.len() > 1).collect()
}
