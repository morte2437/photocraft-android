//! "Did you mean …?" hints for unknown command ids and protocol method names (#3674), so an agent
//! that guesses a name gets something to correct from instead of re-listing the whole registry.
//! Only computed on the error path; the registry is small enough to scan.

use crate::commands;

/// Names that are documented elsewhere and can't be found by spelling alone: a protocol method
/// mistaken for the engine command that does the same thing.
const ALIASES: &[(&str, &str)] = &[("engine.commands", "command.list"), ("document.undo", "edit.undo"), ("document.redo", "edit.redo")];

/// At most this many suggestions per error.
const MAX_SUGGESTIONS: usize = 3;

/// Up to [`MAX_SUGGESTIONS`] `candidates` close to `name`, best first: a different case or a few
/// typos (edit distance), then names that `name` is the start of (`filter.blur.gaussian`), the
/// same last segment in another namespace (`document.undo` → `edit.undo`), and the same namespace
/// with a longer last segment (`image.size` → `image.imageSize`). Deterministic: within a tier
/// the closest spelling wins, then the name.
pub fn near_matches<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let lower = name.to_lowercase();
    let max = (lower.chars().count() / 4).clamp(1, 4);
    let dotted = lower.split_once('.');
    let tail = |s: &str| s.rsplit('.').next().map(str::to_owned).unwrap_or_default();
    let name_tail = tail(&lower);
    let long_tail = name_tail.chars().count() >= 3;
    let mut scored: Vec<(usize, usize, &'a str)> = candidates
        .into_iter()
        .filter(|c| *c != name)
        .filter_map(|c| {
            let cl = c.to_lowercase();
            if let Some(d) = photocraft_text::spell::edit_distance(&lower, &cl, max) {
                return Some((0, d, c));
            }
            let (namespace, _) = dotted?;
            let cl_tail = tail(&cl);
            let tier = if cl.starts_with(&lower) {
                1
            } else if long_tail && cl_tail == name_tail {
                2
            } else if long_tail && cl.split_once('.').is_some_and(|(ns, _)| ns == namespace) && cl_tail.contains(&name_tail) {
                3
            } else {
                return None;
            };
            Some((tier, photocraft_text::spell::edit_distance(&lower, &cl, 64).unwrap_or(usize::MAX), c))
        })
        .collect();
    scored.sort_unstable();
    scored.dedup();
    scored.into_iter().take(MAX_SUGGESTIONS).map(|(.., c)| c).collect()
}

/// `"; did you mean `a`?"` / `"; did you mean `a`, `b` or `c`?"`, or `""` for no suggestions.
pub fn did_you_mean(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
    match quoted.split_last() {
        None => String::new(),
        Some((last, [])) => format!("; did you mean {last}?"),
        Some((last, rest)) => format!("; did you mean {} or {last}?", rest.join(", ")),
    }
}

/// The hint appended to "unknown command `id`": registered ids close to `id`, or `""`.
pub fn unknown_command_hint(id: &str) -> String {
    let mut names: Vec<&str> = ALIASES.iter().filter(|(from, _)| *from == id).map(|(_, to)| *to).collect();
    for n in near_matches(id, commands::command_specs().iter().map(|s| s.id)) {
        if names.len() < MAX_SUGGESTIONS && !names.contains(&n) {
            names.push(n);
        }
    }
    did_you_mean(&names)
}

/// The error for `engine.execute` given `id`, a control-protocol method rather than an engine
/// command (shared by the headless server and the desktop control server).
pub fn protocol_method_as_command(id: &str) -> String {
    format!("{}; `{id}` is a protocol method: send it as the request's method, not through engine.execute", crate::EngineError::UnknownCommand(id.into()))
}

/// The error for an unknown protocol request `method`, given the server's `methods`: an engine
/// command id points at `engine.execute`, anything else gets the nearest method names.
pub fn unknown_method(method: &str, methods: &[&str]) -> String {
    if commands::find(method).is_some() {
        return format!("unknown method `{method}`; `{method}` is an engine command: run it with engine.execute {{\"command\": \"{method}\"}}");
    }
    format!("unknown method `{method}`{}", did_you_mean(&near_matches(method, methods.iter().copied())))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{EngineError, Session};

    fn unknown(id: &str) -> String {
        EngineError::UnknownCommand(id.into()).to_string()
    }

    #[test]
    fn protocol_listing_method_points_at_command_list() {
        assert_eq!(unknown("engine.commands"), "unknown command `engine.commands`; did you mean `command.list`?");
        let mut s = Session::new();
        let e = s.execute("engine.commands", json!({})).unwrap_err().to_string();
        assert!(e.contains("did you mean `command.list`"), "{e}");
        assert!(s.disabled_reason("engine.commands").unwrap().contains("`command.list`"));
    }

    #[test]
    fn near_misses_suggest_registered_ids() {
        // Typo, case, and the same verb in another namespace.
        assert!(unknown("edit.undoo").contains("did you mean `edit.undo`"), "{}", unknown("edit.undoo"));
        assert!(unknown("Edit.Undo").contains("did you mean `edit.undo`"), "{}", unknown("Edit.Undo"));
        let e = unknown("document.undo");
        assert!(e.contains("did you mean `edit.undo`"), "{e}");
        assert!(unknown("document.redo").contains("did you mean `edit.redo`"));
        // A namespace prefix, a truncated id, a shorter last segment.
        assert!(unknown("layer.new").contains("`layer.new.layer`"), "{}", unknown("layer.new"));
        assert!(unknown("filter.blur.gaussian").contains("did you mean `filter.blur.gaussianBlur`"));
        assert!(unknown("image.size").contains("did you mean `image.imageSize`"), "{}", unknown("image.size"));
        assert!(e.len() < 200, "a few suggestions, not a listing: {e}");
        // Nothing close: the plain error.
        assert_eq!(unknown("zzzz.qqqq"), "unknown command `zzzz.qqqq`");
        assert_eq!(unknown(""), "unknown command ``");
    }

    #[test]
    fn suggestions_are_capped_and_ordered() {
        let c = ["a.bc", "a.bd", "a.be", "a.bf", "x.y"];
        assert_eq!(near_matches("a.bb", c), ["a.bc", "a.bd", "a.be"]);
        assert_eq!(did_you_mean(&[]), "");
        assert_eq!(did_you_mean(&["a"]), "; did you mean `a`?");
        assert_eq!(did_you_mean(&["a", "b", "c"]), "; did you mean `a`, `b` or `c`?");
    }

    #[test]
    fn protocol_method_hints() {
        let methods = ["engine.execute", "doc.inspect", "doc.open", "methods"];
        assert_eq!(unknown_method("doc.inspct", &methods), "unknown method `doc.inspct`; did you mean `doc.inspect`?");
        assert!(unknown_method("edit.undo", &methods).contains("engine.execute {\"command\": \"edit.undo\"}"));
        assert_eq!(unknown_method("zzzz", &methods), "unknown method `zzzz`");
        assert!(protocol_method_as_command("doc.inspect").contains("`doc.inspect` is a protocol method"));
    }
}
