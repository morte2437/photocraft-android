//! What the page says when the module stops, and the `?free-mb` diagnostic flag (#10024).
//!
//! A failed allocation on `wasm32-unknown-unknown` aborts the module: the page keeps the last
//! frame and nothing says why. `web.rs` listens for the trap in the page and shows the static
//! `#photocraft_stopped` element; the decisions that need no browser live here, so native tests
//! cover them.

use std::sync::{Mutex, PoisonError};

/// The message of the last panic, kept for the trap that follows it (a panic aborts the module).
static LAST_PANIC: Mutex<String> = Mutex::new(String::new());

/// The most characters of a panic message the page shows.
const MAX_PANIC_CHARS: usize = 300;

/// Truncates `message` to [`MAX_PANIC_CHARS`] characters, on a character boundary.
pub fn truncate(message: &str) -> String {
    match message.char_indices().nth(MAX_PANIC_CHARS) {
        Some((end, _)) => format!("{}\u{2026}", message.get(..end).unwrap_or_default()),
        None => message.to_owned(),
    }
}

/// Remembers a panic's message (and where it happened) for [`take_panic`].
pub fn record_panic(message: &str) {
    *LAST_PANIC.lock().unwrap_or_else(PoisonError::into_inner) = truncate(message);
}

/// The message [`record_panic`] kept, if any.
pub fn take_panic() -> Option<String> {
    let mut last = LAST_PANIC.lock().unwrap_or_else(PoisonError::into_inner);
    (!last.is_empty()).then(|| std::mem::take(&mut *last))
}

/// `true` when an uncaught error, by its `name` and `message`, is the module trapping or the
/// browser refusing it memory, rather than some other script error or failed request. Browsers
/// word and capitalize these differently (Safari: `Unreachable code should not be executed`), so
/// the message is matched ignoring case.
pub fn is_trap(name: &str, message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    name == "RuntimeError"
        || ["unreachable", "memory access out of bounds", "maximum call stack", "could not allocate memory", "out of memory"]
            .iter()
            .any(|m| message.contains(m))
}

/// Trap messages (lowercase, matched ignoring case) that mean an allocation failed:
/// `unreachable` with no panic before it is the allocation-failure abort (the only one that runs
/// no Rust), the others are the browser refusing to grow the memory. `memory access out of
/// bounds` is not among them: it is a bug.
const OUT_OF_MEMORY: [&str; 3] = ["unreachable", "could not allocate memory", "out of memory"];

/// The line under "PhotoCraft stopped": a captured panic means an internal error; a trap with no
/// panic before it is out of memory when its message says so (or says nothing), and otherwise
/// unexpected.
pub fn detail(panic: Option<&str>, trap_message: &str) -> String {
    if let Some(p) = panic {
        return format!("It hit an internal error: {p}");
    }
    let lower = trap_message.to_ascii_lowercase();
    if lower.is_empty() || OUT_OF_MEMORY.iter().any(|m| lower.contains(m)) {
        "It ran out of memory.".to_owned()
    } else {
        format!("It stopped unexpectedly: {}", truncate(trap_message))
    }
}

/// `?free-mb=N` from a URL query (`?cpu&free-mb=300`): pretend only N MiB can be allocated, so
/// the memory gates and the low-memory warning can be seen without gigabytes of images.
pub fn free_mb(query: &str) -> Option<u64> {
    query.trim_start_matches('?').split('&').find_map(|pair| pair.strip_prefix("free-mb=")?.parse().ok())
}

/// The bytes `free-mb` asks for, without overflowing.
pub fn free_bytes(mb: u64) -> u64 {
    mb.saturating_mul(1 << 20)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_message_is_shown_as_an_internal_error() {
        assert_eq!(detail(Some("index out of bounds"), "unreachable"), "It hit an internal error: index out of bounds");
    }

    #[test]
    fn a_trap_with_no_panic_is_an_out_of_memory_abort() {
        assert_eq!(detail(None, "unreachable"), "It ran out of memory.");
        assert_eq!(detail(None, "RuntimeError: unreachable executed"), "It ran out of memory.");
        assert_eq!(detail(None, ""), "It ran out of memory.");
        assert_eq!(detail(None, "WebAssembly.Memory(): could not allocate memory"), "It ran out of memory.");
        // Safari and Firefox word and capitalize theirs differently.
        assert_eq!(detail(None, "Unreachable code should not be executed"), "It ran out of memory.");
        assert_eq!(detail(None, "out of memory"), "It ran out of memory.");
    }

    #[test]
    fn another_kind_of_trap_is_not_blamed_on_memory() {
        let text = detail(None, "divide by zero");
        assert!(text.starts_with("It stopped unexpectedly: divide by zero"), "{text}");
        // A bad memory access is a bug, not memory running out.
        let text = detail(None, "memory access out of bounds");
        assert!(text.starts_with("It stopped unexpectedly: memory access out of bounds"), "{text}");
    }

    #[test]
    fn only_traps_count() {
        assert!(is_trap("RuntimeError", "unreachable"));
        assert!(is_trap("RuntimeError", ""));
        assert!(is_trap("RangeError", "Maximum call stack size exceeded"));
        assert!(is_trap("RangeError", "WebAssembly.Memory(): could not allocate memory"));
        // Matched ignoring case: Safari's wording, reported without the RuntimeError name.
        assert!(is_trap("Error", "Unreachable code should not be executed"));
        assert!(is_trap("Error", "Out of memory"));
        assert!(!is_trap("TypeError", "Failed to fetch"));
        assert!(!is_trap("Error", "ResizeObserver loop completed"));
    }

    #[test]
    fn long_messages_are_cut_on_a_character_boundary() {
        let long = "\u{e9}".repeat(1000);
        let cut = truncate(&long);
        assert_eq!(cut.chars().count(), MAX_PANIC_CHARS + 1);
        assert!(cut.ends_with('\u{2026}'));
        assert_eq!(truncate("short"), "short");
        assert_eq!(truncate(&"x".repeat(MAX_PANIC_CHARS)).chars().count(), MAX_PANIC_CHARS);
    }

    #[test]
    fn the_recorded_panic_is_taken_once() {
        record_panic("boom at a.rs:1");
        assert_eq!(take_panic().as_deref(), Some("boom at a.rs:1"));
        assert_eq!(take_panic(), None);
    }

    #[test]
    fn free_mb_is_read_from_the_query() {
        assert_eq!(free_mb(""), None);
        assert_eq!(free_mb("?cpu"), None);
        assert_eq!(free_mb("?free-mb=300"), Some(300));
        assert_eq!(free_mb("?cpu&free-mb=400&webgl"), Some(400));
        assert_eq!(free_mb("?free-mb=lots"), None);
        assert_eq!(free_bytes(300), 300 << 20);
        assert_eq!(free_bytes(u64::MAX), u64::MAX);
    }
}
