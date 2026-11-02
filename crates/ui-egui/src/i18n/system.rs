//! Native UI-language preferences, queried once without subprocesses or registry parsing.

use super::{Lang, lang_from_tag};

#[cfg(not(target_arch = "wasm32"))]
const MAX_SYSTEM_TAGS: usize = 64;
#[cfg(not(target_arch = "wasm32"))]
const MAX_SYSTEM_TAG_BYTES: usize = 128;

/// Resolve Auto against the current registry. Cache the OS's tags, rather than a language,
/// so matching remains separate from platform detection.
/// Missing or unsupported system preferences always resolve to English.
pub fn system_lang() -> Lang {
    #[cfg(test)]
    {
        TEST_SYSTEM_TAGS.with(|tags| resolve(&tags.borrow()))
    }
    #[cfg(not(test))]
    {
        static SYSTEM: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
        resolve(SYSTEM.get_or_init(detect_system_tags))
    }
}

fn resolve(tags: &[String]) -> Lang {
    tags.iter().find_map(|tag| lang_from_tag(tag)).unwrap_or(Lang::EN)
}

#[cfg(not(target_arch = "wasm32"))]
fn bounded_tags(tags: impl IntoIterator<Item = String>) -> Vec<String> {
    tags.into_iter()
        .take(MAX_SYSTEM_TAGS)
        .filter(|tag| {
            !tag.trim().is_empty()
                && tag.len() <= MAX_SYSTEM_TAG_BYTES
                && tag.trim().bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'@'))
        })
        .map(|tag| tag.trim().to_owned())
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn detect_system_tags() -> Vec<String> {
    if let Ok(tag) = std::env::var("PHOTOCRAFT_LOCALE")
        && !tag.trim().is_empty()
    {
        return bounded_tags([tag]);
    }
    // sys-locale exposes safe wrappers for GetUserPreferredUILanguages (Windows) and
    // CFLocaleCopyPreferredLanguages (macOS); Unix uses its standard locale environment.
    // A failure in platform interop must also leave the app usable in English.
    std::panic::catch_unwind(|| bounded_tags(sys_locale::get_locales())).unwrap_or_default()
}

#[cfg(all(not(test), target_arch = "wasm32"))]
fn detect_system_tags() -> Vec<String> {
    Vec::new()
}

/// The decimal separator of the system's number format (Windows: Region › Decimal symbol), when it
/// is `.` or `,` and can be read: Photoshop's numeric fields take only that one (a German Windows
/// refuses a typed `.`). `None` where it can't be read (macOS, Linux and the web for now, or another
/// symbol): the fields then take both.
pub fn decimal_separator() -> Option<char> {
    #[cfg(test)]
    {
        TEST_DECIMAL.get()
    }
    #[cfg(not(test))]
    {
        static SYSTEM: std::sync::OnceLock<Option<char>> = std::sync::OnceLock::new();
        *SYSTEM.get_or_init(detect_decimal_separator)
    }
}

/// `.` or `,` as a decimal separator; anything else (empty, `٫`, two characters) is not one.
#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
fn separator_from(text: &str) -> Option<char> {
    match text.trim() {
        "." => Some('.'),
        "," => Some(','),
        _ => None,
    }
}

/// The user's own number format (`HKCU\Control Panel\International\sDecimal`, where Windows keeps
/// the decimal symbol of the format chosen in Region, with any change made to it), read through a
/// safe binding.
#[cfg(target_os = "windows")]
fn detect_decimal_separator() -> Option<char> {
    use winsafe::{self as w, co};
    match w::HKEY::CURRENT_USER.RegGetValue(Some("Control Panel\\International"), Some("sDecimal"), co::RRF::RT_REG_SZ) {
        Ok(w::RegistryValue::Sz(text)) => separator_from(&text),
        _ => None,
    }
}

#[cfg(all(not(test), not(target_os = "windows")))]
fn detect_decimal_separator() -> Option<char> {
    None
}

#[cfg(test)]
thread_local! {
    // Tests take both separators unless they choose one (`with_decimal_separator`), whatever the
    // developer's own number format is.
    static TEST_DECIMAL: std::cell::Cell<Option<char>> = const { std::cell::Cell::new(None) };
}

/// Runs `run` as on a system whose decimal separator is `sep` (`None`: unknown, both taken).
#[cfg(test)]
pub(crate) fn with_decimal_separator<R>(sep: Option<char>, run: impl FnOnce() -> R) -> R {
    struct Restore(Option<char>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_DECIMAL.set(self.0);
        }
    }
    let _restore = Restore(TEST_DECIMAL.replace(sep));
    run()
}

#[cfg(test)]
thread_local! {
    // Keep ordinary tests independent of the developer's locale, without mutating OS or
    // process-global preferences. Startup tests supply the OS result on their own thread.
    static TEST_SYSTEM_TAGS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(super) fn with_system_tags<R>(tags: &[&str], run: impl FnOnce() -> R) -> R {
    struct Restore(Vec<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_SYSTEM_TAGS.set(std::mem::take(&mut self.0));
        }
    }
    let _restore = Restore(TEST_SYSTEM_TAGS.replace(tags.iter().map(|tag| (*tag).to_owned()).collect()));
    run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_preferences_are_negotiated_in_order() {
        for (tags, expected) in [
            (vec!["ja-JP", "en-US"], "ja"),
            (vec!["fr-CA", "en-US"], "fr"),
            (vec!["uk-UA", "en-US"], "uk"),
            (vec!["sw-KE", "uk_UA.UTF-8", "ru-RU"], "uk"),
            (vec!["sw-KE", "ko-KR", "fr-FR"], "ko"),
            (vec!["zh-Hant-HK", "zh-CN"], "zh-hant"),
            (vec!["zh-Hans-CN", "zh-TW"], "zh-hans"),
            (vec!["en-US", "ru-RU"], "en"),
            (vec!["is-IS", "ar-SA"], "ar"),
            (vec!["sw-KE", "is-IS"], "en"),
            (vec!["de-AT", "en-US"], "de"),
            (vec!["pt-PT"], "pt-br"),
            (vec!["it-IT", "en-US"], "it"),
            (vec!["nl-BE", "fr-BE"], "nl"),
            (vec!["hu-HU", "en-US"], "hu"),
            (vec!["tr-TR", "en-US"], "tr"),
            (vec!["sr-Latn-RS", "en-US"], "sr"),
            (vec!["sw-KE", "sr_RS@latin", "ru-RU"], "sr"),
            (vec!["hi-IN", "en-US"], "hi"),
            (vec!["sv-SE", "en-US"], "sv"),
            (vec!["sv_FI.UTF-8", "fi-FI", "en-US"], "sv"),
            (vec!["fi-FI", "sv-FI", "en-US"], "fi"),
            (vec!["da-DK", "en-US"], "da"),
            (vec!["nb-NO", "en-US"], "nb"),
            (vec!["nn_NO.UTF-8", "en-US"], "nb"),
            (vec!["is-IS", "no-NO"], "nb"),
            (vec![], "en"),
        ] {
            with_system_tags(&tags, || assert_eq!(system_lang().code(), expected));
        }
    }

    #[test]
    fn test_system_preferences_restore_and_remain_thread_local() {
        with_system_tags(&["ko-KR"], || {
            with_system_tags(&["fr-FR"], || assert_eq!(system_lang().code(), "fr"));
            assert_eq!(system_lang().code(), "ko");
            std::thread::spawn(|| assert_eq!(system_lang(), Lang::EN)).join().expect("locale test thread");
            assert!(std::panic::catch_unwind(|| with_system_tags(&["ja-JP"], || panic!("test unwind"))).is_err());
            assert_eq!(system_lang().code(), "ko");
        });
        assert_eq!(system_lang(), Lang::EN);
    }

    #[test]
    fn only_a_point_or_a_comma_is_a_decimal_separator() {
        assert_eq!(separator_from(","), Some(','));
        assert_eq!(separator_from(" . "), Some('.'));
        for other in ["", "\u{66b}", ",,", "'", "x"] {
            assert_eq!(separator_from(other), None, "{other:?}");
        }
        assert_eq!(decimal_separator(), None, "tests take both unless they choose");
        with_decimal_separator(Some(','), || assert_eq!(decimal_separator(), Some(',')));
        assert_eq!(decimal_separator(), None);
    }

    /// The real Windows number format is read (the developer's or CI's: `,` or `.`).
    #[cfg(target_os = "windows")]
    #[test]
    fn the_windows_number_format_has_a_decimal_separator() {
        let sep = detect_decimal_separator();
        eprintln!("Windows decimal separator: {sep:?}");
        assert!(matches!(sep, Some('.' | ',')), "{sep:?}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_locale_results_are_bounded_and_trimmed() {
        assert_eq!(bounded_tags(["  fr-CA  ".into(), "".into(), " ".into(), "fr-\0".into(), "x".repeat(129)]), ["fr-CA"]);
        assert_eq!(bounded_tags(std::iter::repeat_n("en-US".into(), 100)).len(), MAX_SYSTEM_TAGS);
        let detected = detect_system_tags();
        assert!(detected.len() <= MAX_SYSTEM_TAGS);
        assert!(detected.iter().all(|tag| !tag.is_empty() && tag.len() <= MAX_SYSTEM_TAG_BYTES));
    }
}
