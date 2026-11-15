//! Guard the production display surfaces without banning compatibility identifiers or notices.
use std::path::{Path, PathBuf};

const BANNED: &[&str] = &[
    "Camera Raw",
    "Adobe Single-line Composer",
    "Adobe Every-line Composer",
    "Adobe PDF Presets",
    "Default Photoshop Size",
    "All shortcuts reset to Photoshop defaults.",
];
// Photoshop's own wording where it is stored data or a search alias, never a label: the .atn step
// name Save Actions writes back (the menu item and window read "RAW Develop"), and the names a
// command list is filtered by.
const ALLOWED: &[&str] = &[
    "Adobe Camera Raw Filter",
    "Camera Raw Filter",
    "Camera Raw Defaults",
    "Save these settings as a Camera Raw .xmp file",
    "Load settings from a Camera Raw .xmp file",
];
// The only catalog rows that may say "Camera Raw": their English text names Photoshop's .xmp files.
const ALLOWED_KEYS: &[&str] = &["Save these settings as a Camera Raw .xmp file", "Load settings from a Camera Raw .xmp file"];
const ROOTS: &[&str] = &["src", "../engine/src"];

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The bans as written, before any exemption: does `text` say one of them as a whole word?
fn mentions_banned(text: &str) -> bool {
    BANNED.iter().any(|banned| {
        text.match_indices(banned)
            .any(|(at, _)| !text[..at].chars().next_back().is_some_and(is_word) && !text[at + banned.len()..].chars().next().is_some_and(is_word))
    })
}

fn banned_display(text: &str) -> bool {
    !ALLOWED.contains(&text) && mentions_banned(text)
}

// Read ordinary and raw Rust string literals, ignoring comments. No compiler dependency is
// needed, so this guard can also run by itself when a full application build is unavailable.
#[allow(clippy::cognitive_complexity)] // This small test-only lexer keeps raw-string/comment state together.
fn literals(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;
    let mut test_module = false;
    let mut test_depth = 0_usize;
    while at < chars.len() {
        if chars.get(at..at + 12) == Some(&['#', '[', 'c', 'f', 'g', '(', 't', 'e', 's', 't', ')', ']']) {
            at += 12;
            let rest: String = chars[at..].iter().collect();
            if rest.trim_start().starts_with("mod ") {
                test_module = true;
            }
        } else if chars.get(at..at + 2) == Some(&['/', '/']) {
            while at < chars.len() && chars[at] != '\n' {
                at += 1;
            }
        } else if chars.get(at..at + 2) == Some(&['/', '*']) {
            at += 2;
            let mut depth = 1;
            while at < chars.len() && depth > 0 {
                if chars.get(at..at + 2) == Some(&['/', '*']) {
                    depth += 1;
                    at += 2;
                } else if chars.get(at..at + 2) == Some(&['*', '/']) {
                    depth -= 1;
                    at += 2;
                } else {
                    at += 1;
                }
            }
        } else if chars[at] == 'r' && matches!(chars.get(at + 1), Some('"' | '#')) {
            let start = at;
            at += 1;
            while chars.get(at) == Some(&'#') {
                at += 1;
            }
            let hashes = at - start - 1;
            if chars.get(at) != Some(&'"') {
                continue;
            }
            at += 1;
            let begin = at;
            while at < chars.len() {
                if chars[at] == '"' && (1..=hashes).all(|n| chars.get(at + n) == Some(&'#')) {
                    break;
                }
                at += 1;
            }
            if !test_module {
                out.push(chars[begin..at].iter().collect());
            }
            at += 1 + hashes;
        } else if chars[at] == '"' {
            at += 1;
            let mut text = String::new();
            while at < chars.len() {
                let c = chars[at];
                at += 1;
                if c == '"' {
                    break;
                }
                if c == '\\' && at < chars.len() {
                    text.push(chars[at]);
                    at += 1;
                } else {
                    text.push(c);
                }
            }
            if !test_module {
                out.push(text);
            }
        } else if chars[at] == '\'' && chars.get(at + 2) == Some(&'\'') {
            at += 3; // A character literal such as '"', not the start of a string.
        } else {
            if test_module {
                match chars[at] {
                    '{' => test_depth += 1,
                    '}' => {
                        test_depth = test_depth.saturating_sub(1);
                        if test_depth == 0 {
                            test_module = false;
                        }
                    }
                    ';' if test_depth == 0 => test_module = false,
                    _ => {}
                }
            }
            at += 1;
        }
    }
    out
}

fn check_tree(path: &Path, failures: &mut Vec<String>) {
    if path.is_dir() {
        for entry in std::fs::read_dir(path).expect("source directory") {
            let path = entry.expect("source entry").path();
            if path.file_name().is_some_and(|name| name == "tests") {
                continue;
            }
            check_tree(&path, failures);
        }
    } else if path.extension().is_some_and(|ext| ext == "rs") {
        let name = path.file_name().expect("source filename").to_string_lossy();
        if name == "tests.rs" || name.ends_with("_tests.rs") {
            return;
        }
        let source = std::fs::read_to_string(path).expect("UTF-8 Rust source");
        // Skip only inline test modules, not later production following test-only imports.
        for text in literals(&source) {
            if banned_display(&text) {
                failures.push(format!("{}: {text:?}", path.display()));
            }
        }
    }
}

#[test]
fn production_display_names_stay_neutral() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut failures = Vec::new();
    for root in ROOTS {
        let root = base.join(root);
        assert!(root.exists(), "missing display source: {}", root.display());
        check_tree(&root, &mut failures);
    }
    assert!(failures.is_empty(), "Banned display names:\n{}", failures.join("\n"));
}

#[test]
fn guard_distinguishes_labels_from_identifiers_and_comments() {
    for name in BANNED {
        assert!(banned_display(name), "{name}");
    }
    for name in ALLOWED {
        assert!(!banned_display(name), "{name}");
    }
    assert!(!banned_display("filter.cameraRaw"));
    assert!(!banned_display("tool.guidedUpright"));
    assert!(!banned_display("insert.smartArt"));
    assert_eq!(literals("// \"ignored\"\n/* \"ignored\" */ \"label\""), ["label"]);
    assert_eq!(literals("r#\"raw label\"#"), ["raw label"]);
}

#[test]
fn catalogs_do_not_restore_retired_display_names() {
    // The catalogs live in craft-i18n (`CRAFT_I18N_DIR`, at `craft-i18n.rev`); without it there is nothing to scan.
    let Some(root) = std::env::var_os("CRAFT_I18N_DIR").filter(|d| !d.is_empty()) else {
        return;
    };
    let mut scanned = 0;
    for entry in std::fs::read_dir(PathBuf::from(root).join("locales")).expect("craft-i18n locales directory") {
        let path = entry.expect("locale entry").path().join("photocraft.tsv");
        if !path.is_file() {
            continue;
        }
        scanned += 1;
        let source = std::fs::read_to_string(&path).expect("UTF-8 catalog");
        for line in source.lines().filter(|line| !line.starts_with('#')) {
            let columns: Vec<_> = line.split('\t').collect();
            if let [_, key, value] = columns.as_slice() {
                assert!(ALLOWED_KEYS.contains(key) || !mentions_banned(key), "{}: retired key {key}", path.display());
                if !ALLOWED_KEYS.contains(key) {
                    assert!(!mentions_banned(value), "{}: retired display text {value}", path.display());
                }
                assert_ne!(*key, "Open in camera raw", "{}: dormant key", path.display());
            }
        }
    }
    assert!(scanned > 30, "only {scanned} photocraft.tsv catalogs under CRAFT_I18N_DIR");
}

#[test]
fn guard_scans_production_after_test_only_items() {
    let source = r###"#[cfg(test)] mod tests { let _ = "ignored"; }
        #[cfg(test)] use fixtures::Helper;
        fn production() { let _ = r#"live label"#; }
    "###;
    assert_eq!(literals(source), ["live label"]);
    assert_eq!(literals("#[cfg(test)] mod tests; fn live() { let _ = \"live label\"; }"), ["live label"]);
}
