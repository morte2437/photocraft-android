//! `cargo xtask panel-commands`: every panel button routes through a command id (AGENTS.md rule 1).
//!
//! The UI, CLI, control channel and MCP stay equivalent only while panels dispatch engine commands
//! by id (`app.run`, `menus::invoke`, `session.execute`, the shell's `UI_COMMANDS`). This check
//! scans the egui shell's source for click handlers that change the document or its history
//! directly instead.
//!
//! It is a lexical scanner, not a type checker (std only, no `syn`):
//!
//! * a **click handler** is the block of an `if` whose condition calls `.clicked()` (or
//!   `double_clicked`, `secondary_clicked`, `triple_clicked`, `middle_clicked`, `changed`,
//!   `dragged`, `drag_stopped`), reads a local bound by `let x = … .clicked() …;` in the same file,
//!   or calls a widget function (one that takes a `Ui` and returns `bool`, like `color_readout`);
//! * inside it, a **direct mutation** is `active_mut()` (the mutable document state),
//!   `Arc::make_mut` on a `doc`, `session.undo/redo/select_layer/add_document/close(…)`, or an
//!   assignment to a session field (`app.session.tools.foreground = …`);
//! * test code is skipped: `*_tests.rs`, `tests.rs`, files under `tests/` and `#[cfg(test)]` items.
//!
//! Out of scope: preferences (`edit_prefs`), shell view state (`app.ui`, the `UI_COMMANDS`) and
//! switching the active document. Calls into helper functions are not followed, so a handler
//! that calls a helper which mutates the document passes; the check catches the inline pattern,
//! which is how such shortcuts start. Only `if` blocks (also a `match` arm guard with a block)
//! and `let` locals count as handlers: a `match` on a response without one, a response kept in a
//! struct field and tested later, or a closure callback is not scanned.
//!
//! Existing violations are listed in [`ALLOWLIST`] by file and enclosing function, each with the
//! reason it is still there. An entry that no longer matches anything is an error, so the list only
//! shrinks: fix the handler (dispatch a command id), then delete its entry.

use std::path::{Path, PathBuf};

/// The shell source the check scans, relative to the workspace root.
pub const SCAN_ROOT: &str = "crates/ui-egui/src";

/// A known violation that is tolerated until its handler routes through a command.
#[derive(Debug, Clone, Copy)]
pub struct Allowed {
    /// Path relative to the workspace root, with `/` separators.
    pub file: &'static str,
    /// Enclosing function of the click handler (`<top level>` outside any function).
    pub func: &'static str,
    /// Why it still mutates directly (and what the fix is).
    pub reason: &'static str,
}

/// Existing click handlers that change the document directly. Shrink only: each entry is a
/// follow-up (route it through an engine command), never a place to park new code.
pub const ALLOWLIST: &[Allowed] = &[
    // Window › Color: the colour field and hue strip (and their readout) assign the foreground or
    // background colour on every drag frame, so the change never reaches the journal, action
    // recording or the control channel's event log. Fix: dispatch `tools.setColors` (it already exists).
    Allowed {
        file: "crates/ui-egui/src/panels/color.rs",
        func: "color_field",
        reason: "Color panel field, hue strip and readout assign session.tools.foreground/background; dispatch tools.setColors",
    },
];

/// A click handler that changes the document or its history without a command id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub file: String,
    pub line: usize,
    pub func: String,
    pub what: String,
}

const TRIGGERS: &[&str] = &["clicked", "double_clicked", "secondary_clicked", "triple_clicked", "middle_clicked", "changed", "dragged", "drag_stopped"];

/// Session methods that change documents or history behind the command registry's back.
const SESSION_MUTATORS: &[&str] = &["undo", "redo", "select_layer", "add_document", "close"];

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Ident(String),
    Punct(char),
    /// Literals and lifetimes: never part of a pattern.
    Other,
}

#[derive(Debug, Clone)]
struct Token {
    kind: Kind,
    line: usize,
}

impl Token {
    fn is_ident(&self, s: &str) -> bool {
        matches!(&self.kind, Kind::Ident(i) if i == s)
    }
    fn is_punct(&self, c: char) -> bool {
        self.kind == Kind::Punct(c)
    }
    fn ident(&self) -> Option<&str> {
        match &self.kind {
            Kind::Ident(i) => Some(i),
            _ => None,
        }
    }
}

/// Split Rust source into identifiers and punctuation, dropping comments and literals.
fn lex(src: &str) -> Vec<Token> {
    let c: Vec<char> = src.chars().collect();
    let at = |i: usize| c.get(i).copied().unwrap_or('\0');
    let mut out = Vec::new();
    let mut line = 1;
    let mut i = 0;
    while i < c.len() {
        let ch = at(i);
        if ch == '\n' {
            line += 1;
            i += 1;
        } else if ch.is_whitespace() {
            i += 1;
        } else if ch == '/' && at(i + 1) == '/' {
            while i < c.len() && at(i) != '\n' {
                i += 1;
            }
        } else if ch == '/' && at(i + 1) == '*' {
            (i, line) = skip_block_comment(&c, i, line);
        } else if let Some(hashes) = raw_string_start(&c, i) {
            // r"…", r#"…"#, br#"…"#: skip to the quote followed by the same number of hashes.
            let start_line = line;
            i += hashes.prefix;
            while i < c.len() {
                if at(i) == '"' && (1..=hashes.count).all(|k| at(i + k) == '#') {
                    i += 1 + hashes.count;
                    break;
                }
                if at(i) == '\n' {
                    line += 1;
                }
                i += 1;
            }
            out.push(Token { kind: Kind::Other, line: start_line });
        } else if ch == '"' || (ch == 'b' && at(i + 1) == '"') || (ch == 'c' && at(i + 1) == '"') {
            let start_line = line;
            (i, line) = skip_string(&c, i + if ch == '"' { 1 } else { 2 }, line);
            out.push(Token { kind: Kind::Other, line: start_line });
        } else if ch == '\'' || (ch == 'b' && at(i + 1) == '\'') {
            // Char literal ('a', '\n', b'x') or lifetime/label ('a, 'static).
            let q = if ch == 'b' { i + 1 } else { i };
            if at(q + 1) == '\\' {
                i = q + 2;
                while i < c.len() && at(i) != '\'' && at(i) != '\n' {
                    i += 1;
                }
                i += 1;
            } else if at(q + 2) == '\'' {
                i = q + 3;
            } else {
                i = q + 1;
                while i < c.len() && (at(i).is_alphanumeric() || at(i) == '_') {
                    i += 1;
                }
            }
            out.push(Token { kind: Kind::Other, line });
        } else if ch.is_alphabetic() || ch == '_' {
            let s = i;
            while i < c.len() && (at(i).is_alphanumeric() || at(i) == '_') {
                i += 1;
            }
            out.push(Token { kind: Kind::Ident(c.get(s..i).unwrap_or_default().iter().collect()), line });
        } else if ch.is_ascii_digit() {
            while i < c.len() && (at(i).is_alphanumeric() || at(i) == '_' || (at(i) == '.' && at(i + 1).is_ascii_digit())) {
                i += 1;
            }
            out.push(Token { kind: Kind::Other, line });
        } else {
            out.push(Token { kind: Kind::Punct(ch), line });
            i += 1;
        }
    }
    out
}

/// Skip a (nested) block comment starting at `i`; returns the index after it and the new line.
fn skip_block_comment(c: &[char], mut i: usize, mut line: usize) -> (usize, usize) {
    let at = |k: usize| c.get(k).copied().unwrap_or('\0');
    let mut depth = 0usize;
    while i < c.len() {
        if at(i) == '/' && at(i + 1) == '*' {
            depth += 1;
            i += 2;
        } else if at(i) == '*' && at(i + 1) == '/' {
            depth = depth.saturating_sub(1);
            i += 2;
            if depth == 0 {
                break;
            }
        } else {
            if at(i) == '\n' {
                line += 1;
            }
            i += 1;
        }
    }
    (i, line)
}

/// Skip a quoted string body starting just after its opening quote; returns the index after the
/// closing quote and the new line.
fn skip_string(c: &[char], mut i: usize, mut line: usize) -> (usize, usize) {
    let at = |k: usize| c.get(k).copied().unwrap_or('\0');
    while i < c.len() {
        match at(i) {
            '\\' => {
                // A line continuation (`\` before a newline) still ends a source line.
                if at(i + 1) == '\n' {
                    line += 1;
                }
                i += 2;
            }
            '"' => {
                i += 1;
                break;
            }
            '\n' => {
                line += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    (i, line)
}

struct RawStart {
    /// Characters before the body (`r#"` is 3).
    prefix: usize,
    count: usize,
}

fn raw_string_start(c: &[char], i: usize) -> Option<RawStart> {
    let at = |k: usize| c.get(k).copied().unwrap_or('\0');
    // Not the tail of an identifier such as `for` or `bar`.
    if i > 0 && (at(i - 1).is_alphanumeric() || at(i - 1) == '_') {
        return None;
    }
    let mut j = i;
    if at(j) == 'b' || at(j) == 'c' {
        j += 1;
    }
    if at(j) != 'r' {
        return None;
    }
    j += 1;
    let mut count = 0;
    while at(j) == '#' {
        count += 1;
        j += 1;
    }
    (at(j) == '"').then_some(RawStart { prefix: j + 1 - i, count })
}

/// Index of the bracket closing the one at `open` (`{`, `(` or `[`), or the last token.
fn matching(toks: &[Token], open: usize) -> usize {
    let mut depth = 0usize;
    for (k, t) in toks.iter().enumerate().skip(open) {
        match t.kind {
            Kind::Punct('{' | '(' | '[') => depth += 1,
            Kind::Punct('}' | ')' | ']') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return k;
                }
            }
            _ => {}
        }
    }
    toks.len().saturating_sub(1)
}

/// First `{` (or `;`) after `from` outside parentheses and brackets.
fn next_block_or_semi(toks: &[Token], from: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (k, t) in toks.iter().enumerate().skip(from) {
        match t.kind {
            Kind::Punct('(' | '[') => depth += 1,
            Kind::Punct(')' | ']') => depth = depth.saturating_sub(1),
            Kind::Punct('{' | ';') if depth == 0 => return Some(k),
            _ => {}
        }
    }
    None
}

/// Drop every item under `#[cfg(test)]` (or a `cfg(all(test, …))`), keeping `cfg(not(test))`.
fn strip_cfg_test(toks: Vec<Token>) -> Vec<Token> {
    let mut out = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        if is_test_attr(&toks, i) {
            // Skip this attribute, any further attributes, then the item itself.
            let mut k = i;
            while toks.get(k).is_some_and(|t| t.is_punct('#')) && toks.get(k + 1).is_some_and(|t| t.is_punct('[')) {
                k = matching(&toks, k + 1) + 1;
            }
            i = match next_block_or_semi(&toks, k) {
                Some(e) if toks.get(e).is_some_and(|t| t.is_punct('{')) => matching(&toks, e) + 1,
                Some(e) => e + 1,
                None => toks.len(),
            };
            continue;
        }
        if let Some(t) = toks.get(i) {
            out.push(t.clone());
        }
        i += 1;
    }
    out
}

fn is_test_attr(toks: &[Token], i: usize) -> bool {
    let (Some(hash), Some(open), Some(cfg)) = (toks.get(i), toks.get(i + 1), toks.get(i + 2)) else { return false };
    if !hash.is_punct('#') || !open.is_punct('[') || !cfg.is_ident("cfg") {
        return false;
    }
    let close = matching(toks, i + 1);
    let inner = toks.get(i + 3..close).unwrap_or_default();
    inner.iter().any(|t| t.is_ident("test")) && !inner.iter().any(|t| t.is_ident("not"))
}

/// `(body open, body close, name)` of every function with a body.
fn functions(toks: &[Token]) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if !t.is_ident("fn") {
            continue;
        }
        let Some(name) = toks.get(i + 1).and_then(Token::ident) else { continue };
        if let Some(open) = next_block_or_semi(toks, i + 2).filter(|&o| toks.get(o).is_some_and(|t| t.is_punct('{'))) {
            out.push((open, matching(toks, open), name.to_string()));
        }
    }
    out
}

/// Does `toks` call `.trigger(` for one of the click triggers?
fn calls_trigger(toks: &[Token]) -> bool {
    toks.windows(3).any(|w| {
        w.first().is_some_and(|t| t.is_punct('.'))
            && w.get(1).and_then(Token::ident).is_some_and(|n| TRIGGERS.contains(&n))
            && w.get(2).is_some_and(|t| t.is_punct('('))
    })
}

/// Locals bound by `let name = … .clicked() …;`.
fn click_locals(toks: &[Token]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if !t.is_ident("let") {
            continue;
        }
        let mut n = i + 1;
        if toks.get(n).is_some_and(|t| t.is_ident("mut")) {
            n += 1;
        }
        let (Some(name), Some(eq)) = (toks.get(n).and_then(Token::ident), toks.get(n + 1)) else { continue };
        if !eq.is_punct('=') {
            continue;
        }
        let Some(end) = next_block_or_semi(toks, n + 2) else { continue };
        // A `{` before the `;` (a block or closure value) is out of scope for this heuristic.
        if toks.get(end).is_some_and(|t| t.is_punct(';')) && calls_trigger(toks.get(n + 2..end).unwrap_or_default()) {
            out.push(name.to_string());
        }
    }
    out
}

/// Direct document/history mutations in `toks`: `(token index, description)`.
fn mutations(toks: &[Token]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        let next = |k: usize| toks.get(i + k);
        let Some(name) = t.ident() else { continue };
        let call = next(1).is_some_and(|t| t.is_punct('('));
        if name == "active_mut" && call && i > 0 && toks.get(i - 1).is_some_and(|p| p.is_punct('.')) {
            out.push((i, "`active_mut()` changes the document directly".to_string()));
        } else if name == "make_mut" && call {
            let close = matching(toks, i + 1);
            let args = toks.get(i + 2..close).unwrap_or_default();
            if args.iter().any(|a| a.ident().is_some_and(|n| n == "doc" || n.ends_with("_doc") || n == "document")) {
                out.push((i, "`Arc::make_mut` on a document changes it directly".to_string()));
            }
        } else if name == "session"
            && next(1).is_some_and(|t| t.is_punct('.'))
            && let Some(m) = next(2).and_then(Token::ident).filter(|m| SESSION_MUTATORS.contains(m))
            && next(3).is_some_and(|t| t.is_punct('('))
        {
            out.push((i, format!("`session.{m}()` bypasses the command registry")));
        } else if name == "session"
            && let Some(field) = assigned_field(toks, i)
        {
            out.push((i, format!("`session.{field} = …` changes session state directly")));
        }
    }
    out
}

/// `a.b` when `toks[i]` (`session`) starts `session.a.b = …` or a compound assignment (`+=`, …).
fn assigned_field(toks: &[Token], i: usize) -> Option<String> {
    let mut path = Vec::new();
    let mut k = i + 1;
    while toks.get(k).is_some_and(|t| t.is_punct('.')) {
        path.push(toks.get(k + 1)?.ident()?.to_string());
        k += 2;
    }
    let first = toks.get(k)?;
    let second = toks.get(k + 1);
    let plain = first.is_punct('=') && !second.is_some_and(|t| t.is_punct('=') || t.is_punct('>'));
    let compound = matches!(first.kind, Kind::Punct('+' | '-' | '*' | '/' | '|' | '&' | '^')) && second.is_some_and(|t| t.is_punct('='));
    (!path.is_empty() && (plain || compound)).then(|| path.join("."))
}

/// Names of functions that draw a widget and report an edit: they take a `Ui` and return `bool`
/// (`fn color_readout(ui: &mut Ui, …) -> bool`). `if widget(…) { … }` is a click handler too.
pub fn widget_fns(src: &str) -> Vec<String> {
    let toks = strip_cfg_test(lex(src));
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if !t.is_ident("fn") {
            continue;
        }
        let Some(name) = toks.get(i + 1).and_then(Token::ident) else { continue };
        let Some(open) = toks.iter().enumerate().skip(i + 2).find(|(_, t)| t.is_punct('(')).map(|(k, _)| k) else { continue };
        let close = matching(&toks, open);
        let takes_ui = toks.get(open..close).unwrap_or_default().iter().any(|t| t.is_ident("Ui"));
        let returns_bool = toks.get(close + 1).is_some_and(|t| t.is_punct('-'))
            && toks.get(close + 2).is_some_and(|t| t.is_punct('>'))
            && toks.get(close + 3).is_some_and(|t| t.is_ident("bool"));
        if takes_ui && returns_bool && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// Every click handler in `src` that changes the document directly. `file` labels the results.
/// `widgets` lists the widget functions of the whole tree (see [`widget_fns`]).
pub fn scan(file: &str, src: &str, widgets: &[String]) -> Vec<Violation> {
    let toks = strip_cfg_test(lex(src));
    let fns = functions(&toks);
    let locals = click_locals(&toks);
    let mut out: Vec<Violation> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if !t.is_ident("if") {
            continue;
        }
        let Some(open) = next_block_or_semi(&toks, i + 1) else { continue };
        if !toks.get(open).is_some_and(|t| t.is_punct('{')) {
            continue;
        }
        let cond = toks.get(i + 1..open).unwrap_or_default();
        let is_click = calls_trigger(cond)
            || cond.iter().any(|c| c.ident().is_some_and(|n| locals.iter().any(|l| l == n)))
            || cond
                .windows(2)
                .any(|w| w.first().and_then(Token::ident).is_some_and(|n| widgets.iter().any(|f| f == n)) && w.get(1).is_some_and(|t| t.is_punct('(')));
        if !is_click {
            continue;
        }
        let close = matching(&toks, open);
        let body = toks.get(open..=close).unwrap_or_default();
        for (k, what) in mutations(body) {
            let at = open + k;
            let line = toks.get(at).map_or(0, |t| t.line);
            let func = fns.iter().filter(|(o, c, _)| *o < at && at < *c).min_by_key(|(o, c, _)| c - o).map_or("<top level>", |f| f.2.as_str());
            let v = Violation { file: file.to_string(), line, func: func.to_string(), what };
            if !out.contains(&v) {
                out.push(v);
            }
        }
    }
    out.sort_by_key(|v| v.line);
    out
}

/// Is `path` (relative, `/`-separated) test code?
fn is_test_file(path: &str) -> bool {
    path.ends_with("_tests.rs") || path.ends_with("/tests.rs") || path == "tests.rs" || path.contains("/tests/") || path.starts_with("tests/")
}

/// Violations not covered by `allow`, and allowlist entries that match nothing (stale).
pub fn check<'a>(violations: &[Violation], allow: &'a [Allowed]) -> (Vec<Violation>, Vec<&'a Allowed>) {
    let covered = |v: &Violation, a: &Allowed| v.file == a.file && v.func == a.func;
    let new = violations.iter().filter(|v| !allow.iter().any(|a| covered(v, a))).cloned().collect();
    let stale = allow.iter().filter(|a| !violations.iter().any(|v| covered(v, a))).collect();
    (new, stale)
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for e in entries {
        let p = e.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if p.is_dir() {
            rust_files(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
    Ok(())
}

/// Scan the shell source under `root` (the workspace root).
pub fn scan_tree(root: &Path) -> Result<Vec<Violation>, String> {
    let base = root.join(SCAN_ROOT);
    let mut files = Vec::new();
    rust_files(&base, &mut files)?;
    files.sort();
    let mut sources = Vec::new();
    for f in files {
        let rel = f.strip_prefix(&base).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
        if is_test_file(&rel) {
            continue;
        }
        let src = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        sources.push((format!("{SCAN_ROOT}/{rel}"), src));
    }
    let mut widgets = Vec::new();
    for (_, src) in &sources {
        for name in widget_fns(src) {
            if !widgets.contains(&name) {
                widgets.push(name);
            }
        }
    }
    Ok(sources.iter().flat_map(|(file, src)| scan(file, src, &widgets)).collect())
}

pub fn run(root: &Path) -> Result<(), String> {
    let violations = scan_tree(root)?;
    let (new, stale) = check(&violations, ALLOWLIST);
    println!("Panel command routing (AGENTS.md rule 1): click handlers in {SCAN_ROOT} must dispatch a command id\n");
    println!("{} direct mutation(s) in click handlers, {} allowlisted function(s)", violations.len(), ALLOWLIST.len());
    for v in &new {
        println!("  {}:{}: in `{}`: {}; dispatch an engine command by id instead", v.file, v.line, v.func, v.what);
    }
    for a in ALLOWLIST.iter().filter(|a| !stale.iter().any(|s| s.file == a.file && s.func == a.func)) {
        println!("  allowlisted {} `{}`: {}", a.file, a.func, a.reason);
    }
    for a in &stale {
        println!("  stale allowlist entry {} `{}`: no direct mutation left there; delete it from xtask/src/panel_commands.rs", a.file, a.func);
    }
    if new.is_empty() && stale.is_empty() {
        println!("OK: every other click handler routes through a command id.");
        Ok(())
    } else {
        Err(format!("{} unrouted click handler(s), {} stale allowlist entr(y/ies)", new.len(), stale.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(src: &str) -> Vec<(usize, String)> {
        scan("f.rs", src, &[]).into_iter().map(|v| (v.line, v.func)).collect()
    }

    #[test]
    fn routed_button_passes() {
        let src = r#"
fn panel(app: &mut App, ui: &mut Ui) {
    if ui.button("Flatten").clicked() {
        let _ = app.run("layer.flatten", json!({}));
    }
    if ui.button("Undo").clicked() {
        menus::invoke(app, ctx, "edit.undo", Value::Null);
    }
}
"#;
        assert!(scan("f.rs", src, &[]).is_empty());
    }

    #[test]
    fn direct_mutation_fails_with_file_line_and_function() {
        let src = r#"
fn panel(app: &mut App, ui: &mut Ui) {
    if ui.button("Hide").clicked() {
        if let Some(st) = app.session.active_mut() {
            st.active_layer = None;
        }
    }
}
"#;
        let v = scan("crates/ui-egui/src/x.rs", src, &[]);
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].file.as_str(), v[0].line, v[0].func.as_str()), ("crates/ui-egui/src/x.rs", 4, "panel"));
        assert!(v[0].what.contains("active_mut"));
    }

    #[test]
    fn line_continuations_in_strings_keep_line_numbers() {
        let src = "
fn panel(app: &mut App, ui: &mut Ui) {
    ui.label(\"first \\
        second \\
        third\");
    if ui.button(\"Hide\").clicked() {
        if let Some(st) = app.session.active_mut() {
            st.active_layer = None;
        }
    }
}
";
        let v = scan("f.rs", src, &[]);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].line, 7);
    }

    #[test]
    fn a_match_arm_guard_with_a_block_is_a_click_handler() {
        let src = "
fn panel(app: &mut App, ui: &mut Ui) {
    match ui.button(\"Hide\") {
        r if r.clicked() => {
            app.session.active_mut();
        }
        _ => {}
    }
}
";
        assert_eq!(lines(src), [(5, "panel".to_string())]);
    }

    #[test]
    fn every_mutation_kind_and_trigger_is_caught() {
        let src = r#"
fn a(app: &mut App, ui: &mut Ui) {
    if ui.button("x").secondary_clicked() { app.session.undo(); }
    if resp.double_clicked() && ok { Arc::make_mut(&mut st.doc).layers.clear(); }
    let pressed = ui.button("y").clicked();
    if pressed { app.session.close(0); }
    if slider.changed() { app.session.select_layer(id); }
}
"#;
        assert_eq!(lines(src), vec![(3, "a".into()), (4, "a".into()), (6, "a".into()), (7, "a".into())]);
    }

    #[test]
    fn mutation_outside_click_handlers_test_code_comments_and_strings_pass() {
        let src = r##"
fn sync(app: &mut App) {
    if let Some(st) = app.session.active_mut() { st.dirty = true; }
}
fn panel(app: &mut App, ui: &mut Ui) {
    if ui.button("x").clicked() {
        // app.session.active_mut()
        let s = "app.session.undo()";
        let r = r#"session.close(0)"#;
        let c = '{';
        let _ = app.run("edit.undo", Value::Null);
    }
    if ui.button("y").clicked() { theme(Arc::make_mut(&mut fonts)); }
}
#[cfg(test)]
mod tests {
    fn t(app: &mut App, ui: &mut Ui) {
        if ui.button("x").clicked() { app.session.undo(); }
    }
}
"##;
        assert!(scan("f.rs", src, &[]).is_empty(), "{:?}", scan("f.rs", src, &[]));
    }

    #[test]
    fn widget_functions_and_session_assignments_count() {
        let widgets = widget_fns("fn color_readout(ui: &mut egui::Ui, c: &mut [f32; 4]) -> bool { false }\nfn label(ui: &mut Ui) {}");
        assert_eq!(widgets, vec!["color_readout".to_string()]);
        let src = r#"
fn field(app: &mut App, ui: &mut Ui) {
    if color_readout(ui, &mut c) { app.session.tools.foreground = c; }
    if r.dragged() { app.session.tools.brush.size += 1.0; }
    if r.clicked() { let same = app.session.tools.foreground == c; app.ui.show = true; }
    if r.clicked() { let _ = app.run("tools.setColors", json!({"foreground": "red"})); }
}
"#;
        let v = scan("f.rs", src, &widgets);
        assert_eq!(v.iter().map(|v| v.line).collect::<Vec<_>>(), vec![3, 4]);
        assert!(v[0].what.contains("session.tools.foreground"), "{}", v[0].what);
        assert!(v[1].what.contains("session.tools.brush.size"), "{}", v[1].what);
    }

    #[test]
    fn cfg_not_test_is_scanned_and_lifetimes_do_not_confuse_the_lexer() {
        let src = r#"
#[cfg(not(test))]
fn panel<'a>(app: &'a mut App, ui: &mut Ui) {
    if ui.button("x").clicked() { app.session.redo(); }
}
"#;
        assert_eq!(lines(src), vec![(4, "panel".into())]);
    }

    #[test]
    fn allowlisted_passes_and_stale_entry_is_reported() {
        let v = vec![Violation { file: "a.rs".into(), line: 3, func: "panel".into(), what: "x".into() }];
        let allow = [Allowed { file: "a.rs", func: "panel", reason: "known" }, Allowed { file: "a.rs", func: "gone", reason: "fixed already" }];
        let (new, stale) = check(&v, &allow);
        assert!(new.is_empty());
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].func, "gone");
        let (new, stale) = check(&v, &[]);
        assert_eq!(new, v);
        assert!(stale.is_empty());
    }

    #[test]
    fn test_files_are_recognised() {
        assert!(is_test_file("panels_tests.rs"));
        assert!(is_test_file("file_dialog/tests.rs"));
        assert!(is_test_file("discard_ui/tests/tiff_tests.rs"));
        assert!(!is_test_file("panels.rs"));
        assert!(!is_test_file("tests_ui.rs"));
    }

    #[test]
    fn the_allowlist_has_reasons_and_no_duplicates() {
        for (i, a) in ALLOWLIST.iter().enumerate() {
            assert!(!a.reason.trim().is_empty(), "{} `{}` needs a reason", a.file, a.func);
            assert!(a.file.starts_with(SCAN_ROOT), "{} is outside {SCAN_ROOT}", a.file);
            assert!(!ALLOWLIST.iter().skip(i + 1).any(|b| b.file == a.file && b.func == a.func), "duplicate {} `{}`", a.file, a.func);
        }
    }
}
