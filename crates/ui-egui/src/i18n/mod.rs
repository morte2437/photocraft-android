//! UI localisation. Strings in code stay English and are the default lookup keys; a per-language
//! catalog (`*.tsv`, see `ja.tsv` for the format) maps them to display text at render time. Command
//! ids, menu paths used for logic, the control channel, the CLI and MCP never see translated text,
//! so agents and scripts are unaffected. A string without a translation is shown in English.
//!
//! # Adding a language
//! 1. Add `xx.tsv` next to `ja.tsv` (copy its header; translate from the *meaning* of the English
//!    text, clean-room, see `ja.tsv`).
//! 2. Add one row to [`LANGUAGES`] (code, native name, catalog, plural rule).
//!
//! That is all: the Preferences dropdown, the system-locale match and the catalog tests (parse,
//! placeholders, plural forms) pick it up from the registry. A right-to-left language (Arabic,
//! Hebrew, Persian, Urdu, see [`dir_for_code`]) also mirrors the window's chrome
//! ([`crate::direction`]).
//!
//! # Looking strings up
//! - [`tr`]: a plain string. [`tr_ctx`]: when one English word needs different translations.
//! - [`tr_id`]: a command-id keyed string with the English label as fallback (menu items), so a
//!   translation survives rewording of the English text and can differ per command.
//! - [`trn`]: plural-aware (`{n}` is filled in). [`fmt`]: fill `{name}` placeholders after [`tr`];
//!   translators may reorder placeholders freely.

mod catalog;
mod system;

pub use system::system_lang;

use std::cell::Cell;
use std::sync::OnceLock;

use catalog::Catalog;

/// One supported UI language.
pub struct LangInfo {
    /// BCP 47 code, lowercase (`ja`, `zh-hans`, `pt-br`). Also the `interface.language` value.
    pub code: &'static str,
    /// The language's name in itself, shown in the Preferences dropdown.
    pub name: &'static str,
    /// Catalog file contents, raw-deflated by build.rs (empty for the built-in English); see
    /// [`LangInfo::text`].
    pub source: &'static [u8],
    /// Plural form index for a count (English: 0 = one, 1 = other; Japanese, Chinese and Hungarian: always 0;
    /// Czech: 0 = one, 1 = few (2–4), 2 = other; French: 0 = one (0 and 1), 1 = other). A catalog's `@plural` entries list one form per
    /// index.
    pub plural: fn(u64) -> usize,
    /// Does the catalog cover every menu string? (`cargo xtask i18n-coverage` lists what it lacks)
    pub complete_menus: bool,
    catalog: OnceLock<Catalog>,
}

/// A script's reading direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Ltr,
    Rtl,
}

/// Languages written right to left, by primary subtag: Arabic, Sorani Kurdish, Dhivehi, Persian,
/// Hebrew, Pashto, Sindhi, Uyghur, Urdu, Yiddish.
const RTL_LANGUAGES: [&str; 10] = ["ar", "ckb", "dv", "fa", "he", "ps", "sd", "ug", "ur", "yi"];

/// The reading direction of a language code (`ar`, `fa-ir`, `he`; any case).
pub fn dir_for_code(code: &str) -> Dir {
    let primary = code.split(['-', '_']).next().unwrap_or("");
    if RTL_LANGUAGES.iter().any(|l| l.eq_ignore_ascii_case(primary)) { Dir::Rtl } else { Dir::Ltr }
}

fn plural_one_other(n: u64) -> usize {
    usize::from(n != 1)
}

fn plural_none(_: u64) -> usize {
    0
}

fn plural_russian(n: u64) -> usize {
    match (n % 10, n % 100) {
        (1, 11..=19) => 2,
        (1, _) => 0,
        (2..=4, 11..=19) => 2,
        (2..=4, _) => 1,
        _ => 2,
    }
}

/// Czech: 1 → one, 2–4 → few, everything else (0, 5+) → other.
fn plural_cs(n: u64) -> usize {
    match n {
        1 => 0,
        2..=4 => 1,
        _ => 2,
    }
}

/// French: 0 and 1 take the singular, everything else the plural.
fn plural_fr(n: u64) -> usize {
    usize::from(n > 1)
}

/// Portuguese: 0 and 1 take the singular, everything else the plural.
fn plural_pt(n: u64) -> usize {
    usize::from(n > 1)
}

/// Polish: 1 → one; 2–4, except 12–14 → few; everything else → many.
fn plural_polish(n: u64) -> usize {
    let last = n % 10;
    let last_two = n % 100;
    if n == 1 {
        0
    } else if (2..=4).contains(&last) && !(12..=14).contains(&last_two) {
        1
    } else {
        2
    }
}

/// A catalog file as packed by build.rs (`pack_catalogs`; the binary would otherwise carry ~2.2 MB
/// of translations, which the web build's size gate can't afford). See [`unpack`].
macro_rules! catalog {
    ($code:literal) => {
        include_bytes!(concat!(env!("OUT_DIR"), "/i18n/", $code, ".tsv.deflate"))
    };
}

/// Inflated catalogs are bounded, so corrupt data can't exhaust memory (the largest is ~0.2 MB).
const MAX_CATALOG_BYTES: u64 = 16 << 20;

/// Inflate a catalog. Bad data gives an error (and the language falls back to English), never a panic.
fn inflate(source: &[u8]) -> Result<String, String> {
    use std::io::Read as _;
    let mut text = String::new();
    if source.is_empty() {
        return Ok(text);
    }
    flate2::read::DeflateDecoder::new(source).take(MAX_CATALOG_BYTES + 1).read_to_string(&mut text).map_err(|e| e.to_string())?;
    if text.len() as u64 > MAX_CATALOG_BYTES {
        return Err(format!("over {MAX_CATALOG_BYTES} bytes"));
    }
    Ok(text)
}

/// Every catalog entry's `context<TAB>source`, stored once for all catalogs (build.rs).
static KEYS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/i18n/keys.deflate"));

/// Restore a catalog file from its packed form (build.rs `pack_catalogs`): entry lines refer to
/// [`KEYS`] instead of repeating the English source. Bad data gives an error (and the language
/// falls back to English), never a panic.
fn unpack(source: &[u8]) -> Result<String, String> {
    static KEY_TEXT: OnceLock<Result<String, String>> = OnceLock::new();
    if source.is_empty() {
        return Ok(String::new());
    }
    let keys: Vec<&str> = KEY_TEXT.get_or_init(|| inflate(KEYS)).as_ref().map_err(|e| format!("catalog keys: {e}"))?.split('\n').collect();
    expand(&inflate(source)?, &keys)
}

/// [`unpack`]'s second step: replace each packed entry line with its key and translation.
fn expand(packed: &str, keys: &[&str]) -> Result<String, String> {
    let mut text = String::with_capacity(packed.len().saturating_mul(2));
    let mut next = 0usize;
    for (n, line) in packed.split('\n').enumerate() {
        if n > 0 {
            text.push('\n');
        }
        let bad = || format!("packed line {}: bad key reference", n + 1);
        let (index, tr) = if let Some(tr) = line.strip_prefix('\x01') {
            (next, tr)
        } else if let Some(rest) = line.strip_prefix('\x02') {
            let (offset, tr) = rest.split_once('\t').ok_or_else(bad)?;
            let offset: i64 = offset.parse().map_err(|_| bad())?;
            let index = i64::try_from(next).ok().and_then(|n| n.checked_add(offset)).and_then(|i| usize::try_from(i).ok()).ok_or_else(bad)?;
            (index, tr)
        } else {
            text.push_str(line.strip_prefix('\x03').unwrap_or(line));
            continue;
        };
        text.push_str(keys.get(index).ok_or_else(bad)?);
        text.push('\t');
        text.push_str(tr);
        next = index.saturating_add(1);
        if text.len() as u64 > MAX_CATALOG_BYTES {
            return Err(format!("over {MAX_CATALOG_BYTES} bytes"));
        }
    }
    Ok(text)
}

/// The registry. English first: it is the fallback and the source language.
pub static LANGUAGES: [LangInfo; 22] = [
    LangInfo { code: "en", name: "English", source: b"", plural: plural_one_other, complete_menus: false, catalog: OnceLock::new() },
    LangInfo { code: "ja", name: "日本語", source: catalog!("ja"), plural: plural_none, complete_menus: true, catalog: OnceLock::new() },
    LangInfo { code: "zh-hans", name: "简体中文", source: catalog!("zh-hans"), plural: plural_none, complete_menus: true, catalog: OnceLock::new() },
    // Traditional Chinese in the vocabulary used in Taiwan; `zh-TW`, `zh-HK`, `zh-MO` and `zh-Hant-*`
    // locales all resolve here (see `candidates`).
    LangInfo { code: "zh-hant", name: "繁體中文", source: catalog!("zh-hant"), plural: plural_none, complete_menus: true, catalog: OnceLock::new() },
    LangInfo { code: "es", name: "Español", source: catalog!("es"), plural: plural_one_other, complete_menus: true, catalog: OnceLock::new() },
    LangInfo { code: "ru", name: "Русский", source: catalog!("ru"), plural: plural_russian, complete_menus: true, catalog: OnceLock::new() },
    // Ukrainian has the same one/few/many rule for integer counts.
    LangInfo { code: "uk", name: "Українська", source: catalog!("uk"), plural: plural_russian, complete_menus: true, catalog: OnceLock::new() },
    // Serbian (Latin script, Ekavian) has the same one/few/many rule for integer counts. `sr-RS`,
    // `sr-Latn-*` and `sr_RS@latin` resolve here; so do Cyrillic `sr` locales, which read Latin
    // rather than English.
    LangInfo { code: "sr", name: "Srpski", source: catalog!("sr"), plural: plural_russian, complete_menus: true, catalog: OnceLock::new() },
    LangInfo { code: "cs", name: "Čeština", source: catalog!("cs"), plural: plural_cs, complete_menus: true, catalog: OnceLock::new() },
    LangInfo { code: "fr", name: "Français", source: catalog!("fr"), plural: plural_fr, complete_menus: true, catalog: OnceLock::new() },
    LangInfo { code: "id", name: "Bahasa Indonesia", source: catalog!("id"), plural: plural_none, complete_menus: true, catalog: OnceLock::new() },
    LangInfo { code: "ko", name: "한국어", source: catalog!("ko"), plural: plural_none, complete_menus: true, catalog: OnceLock::new() },
    LangInfo { code: "pl", name: "Polski", source: catalog!("pl"), plural: plural_polish, complete_menus: true, catalog: OnceLock::new() },
    LangInfo { code: "de", name: "Deutsch", source: catalog!("de"), plural: plural_one_other, complete_menus: true, catalog: OnceLock::new() },
    // Brazilian Portuguese; `pt`, `pt-BR` and `pt-PT` locales all resolve here (see `candidates`).
    LangInfo { code: "pt-br", name: "Português (Brasil)", source: catalog!("pt-br"), plural: plural_pt, complete_menus: true, catalog: OnceLock::new() },
    LangInfo { code: "el", name: "Ελληνικά", source: catalog!("el"), plural: plural_one_other, complete_menus: true, catalog: OnceLock::new() },
    // Dutch; `nl-NL` and `nl-BE` locales both resolve here.
    LangInfo { code: "nl", name: "Nederlands", source: catalog!("nl"), plural: plural_one_other, complete_menus: true, catalog: OnceLock::new() },
    LangInfo { code: "it", name: "Italiano", source: catalog!("it"), plural: plural_one_other, complete_menus: true, catalog: OnceLock::new() },
    // Hungarian: a count takes the singular noun ("3 réteg"), so one plural form.
    LangInfo { code: "hu", name: "Magyar", source: catalog!("hu"), plural: plural_none, complete_menus: true, catalog: OnceLock::new() },
    // Turkish; `tr`, `tr-TR` and `tr_TR` locales resolve here.
    LangInfo { code: "tr", name: "Türkçe", source: catalog!("tr"), plural: plural_none, complete_menus: true, catalog: OnceLock::new() },
    // Vietnamese; `vi-VN` resolves here.
    LangInfo { code: "vi", name: "Tiếng Việt", source: catalog!("vi"), plural: plural_none, complete_menus: true, catalog: OnceLock::new() },
    // Hindi; `hi`, `hi-IN` locales resolve here.
    LangInfo { code: "hi", name: "हिन्दी", source: catalog!("hi"), plural: plural_one_other, complete_menus: false, catalog: OnceLock::new() },
];

impl LangInfo {
    /// The catalog file's text (empty for English, or if the bundled data can't be inflated).
    pub fn text(&self) -> String {
        unpack(self.source).unwrap_or_else(|e| {
            log::error!("translation catalog {}: {e}; showing English", self.code);
            String::new()
        })
    }

    fn catalog(&self) -> &Catalog {
        self.catalog.get_or_init(|| Catalog::parse(&self.text()))
    }
}

/// A language the UI can be shown in (a handle into [`LANGUAGES`]).
#[derive(Clone, Copy)]
pub struct Lang(&'static LangInfo);

impl std::fmt::Debug for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lang({})", self.0.code)
    }
}

impl PartialEq for Lang {
    fn eq(&self, other: &Self) -> bool {
        self.0.code == other.0.code
    }
}

impl Eq for Lang {}

impl Lang {
    pub const EN: Lang = Lang(&LANGUAGES[0]);

    pub fn code(self) -> &'static str {
        self.0.code
    }

    /// A language by its exact code.
    pub fn from_code(code: &str) -> Option<Lang> {
        LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code)).map(Lang)
    }

    /// Resolve the `interface.language` preference: a language code, or `auto` (and anything
    /// unknown, e.g. a code from a newer version) to follow the system locale.
    pub fn from_pref(pref: &str) -> Lang {
        if pref.eq_ignore_ascii_case("auto") {
            return system_lang();
        }
        Lang::from_code(pref).or_else(|| lang_from_tag(pref)).unwrap_or_else(system_lang)
    }

    /// Every registered language.
    pub fn all() -> impl Iterator<Item = Lang> {
        LANGUAGES.iter().map(Lang)
    }

    /// Every registered language in the order a picker lists them: English first (the source
    /// language and the fallback), then the rest by their own name with accents folded, so
    /// Čeština sits under C and Türkçe under T rather than after Z. Names in other scripts follow
    /// the Latin ones in code point order: Greek, Cyrillic, then the CJK languages.
    pub fn for_picker() -> Vec<Lang> {
        let mut rest: Vec<Lang> = Lang::all().filter(|l| l.code() != "en").collect();
        rest.sort_by_cached_key(|l| name_sort_key(l.name()));
        std::iter::once(Lang::EN).chain(rest).collect()
    }

    pub fn name(self) -> &'static str {
        self.0.name
    }

    /// Does this language's catalog claim to cover every menu string and `tl!` literal?
    pub fn complete_menus(self) -> bool {
        self.0.complete_menus
    }

    /// The language's reading direction.
    pub fn dir(self) -> Dir {
        dir_for_code(self.0.code)
    }

    fn catalog(self) -> &'static Catalog {
        self.0.catalog()
    }
}

/// `name` lowercased with the Latin accents that occur in language names folded away, so a
/// byte-wise sort reads alphabetically. Other scripts are left as they are.
fn name_sort_key(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'è'..='ë' | 'ē' | 'ě' | 'ę' | 'ế' | 'ệ' => 'e',
            'ì'..='ï' | 'ī' => 'i',
            'ñ' | 'ń' | 'ň' => 'n',
            'ò'..='ö' | 'ø' | 'ő' => 'o',
            'ś' | 'š' => 's',
            'ù'..='ü' | 'ū' | 'ů' | 'ű' => 'u',
            'ý' | 'ÿ' => 'y',
            'ź' | 'ż' | 'ž' => 'z',
            c => c,
        })
        .collect()
}

/// Candidate language codes for a locale tag, most specific first: `zh_TW.UTF-8` →
/// `zh-tw`, `zh-hant`, `zh`.
fn candidates(tag: &str) -> Vec<String> {
    let base = tag.split(['.', '@']).next().unwrap_or("").replace('_', "-").to_ascii_lowercase();
    let parts: Vec<&str> = base.split('-').filter(|p| !p.is_empty()).collect();
    let Some(&primary) = parts.first() else { return Vec::new() };
    let mut out = Vec::new();
    for n in (1..=parts.len()).rev() {
        out.push(parts[..n].join("-"));
    }
    if primary == "zh" && !parts.iter().any(|p| matches!(*p, "hans" | "hant")) {
        // Chinese by region when no script is given.
        let script = if parts.iter().any(|p| matches!(*p, "tw" | "hk" | "mo")) { "zh-hant" } else { "zh-hans" };
        out.insert(out.len() - 1, script.to_string());
    }
    if primary == "pt" && !out.iter().any(|c| c == "pt-br") {
        // The only Portuguese catalog is Brazilian; other regions use it rather than English.
        out.insert(out.len() - 1, "pt-br".to_string());
    }
    out
}

/// The registered language for a locale tag such as `ja_JP.UTF-8`, `ja-JP`, `zh-TW`; `None` if
/// the language isn't supported. `C`/`POSIX` mean English.
pub fn lang_from_tag(tag: &str) -> Option<Lang> {
    let cands = candidates(tag);
    if matches!(cands.first().map(String::as_str), Some("c" | "posix")) {
        return Some(Lang::EN);
    }
    cands.iter().find_map(|c| Lang::from_code(c))
}

thread_local! {
    // Independent app/test threads must not change each other's drawing language.
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::EN) };
}

/// Set the UI language for drawing (the shell calls this once per frame from the preference), so
/// widgets can translate without every call site carrying a language around.
pub fn set_current(lang: Lang) {
    CURRENT.set(lang);
}

/// The language the UI is drawn in.
pub fn current() -> Lang {
    CURRENT.get()
}

/// Temporarily draw in another language, restoring the previous one even on unwinding.
/// Preferences previews use this without changing the saved setting or other dialogs.
pub fn with_language<R>(lang: Lang, draw: impl FnOnce() -> R) -> R {
    let _restore = language_scope(lang);
    draw()
}

/// Keep the language active until this guard is dropped on the drawing thread.
#[must_use]
pub fn language_scope(lang: Lang) -> impl Drop {
    struct Restore {
        previous: Lang,
        _thread: std::marker::PhantomData<std::rc::Rc<()>>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            set_current(self.previous);
        }
    }
    let restore = Restore { previous: current(), _thread: std::marker::PhantomData };
    set_current(lang);
    restore
}

/// Apply a committed language to this context. Font caches are rebuilt only on a language
/// change, so Han glyphs follow the selected script even when the previous font covered them.
pub fn sync_context(ctx: &egui::Context, language: &str) {
    let lang = Lang::from_pref(language);
    set_current(lang);
    let id = egui::Id::new("photocraft-ui-language");
    let changed = ctx.data(|data| data.get_temp::<Lang>(id) != Some(lang));
    if changed {
        ctx.data_mut(|data| data.insert_temp(id, lang));
        crate::theme::install_fonts(ctx);
        ctx.request_repaint();
    }
}

/// Does `lang` have a catalog entry for this plain string? (English never does: it is the source.)
pub fn has(lang: Lang, s: &str) -> bool {
    lang.catalog().plain(s).is_some()
}

/// Translate an English UI string into the current language ([`tr`] with [`current`]).
pub fn t(s: &str) -> &str {
    tr(current(), s)
}

/// Translate an English UI string; unknown strings come back unchanged.
pub fn tr(lang: Lang, s: &str) -> &str {
    lang.catalog().plain(s).unwrap_or(s)
}

/// Like [`tr`], for an English string that needs a disambiguating `context`.
pub fn tr_ctx<'a>(lang: Lang, context: &str, s: &'a str) -> &'a str {
    lang.catalog().contextual(context, s).unwrap_or_else(|| tr(lang, s))
}

/// A string keyed by its command id, falling back to the translation of the English `label`.
pub fn tr_id<'a>(lang: Lang, id: &str, label: &'a str) -> &'a str {
    lang.catalog().id(id).unwrap_or_else(|| tr(lang, label))
}

/// Fill `{name}` placeholders. Unknown placeholders are left as written.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// A plural-aware message: `one`/`other` are the English forms (with `{n}` where the count goes).
pub fn trn(lang: Lang, n: u64, one: &str, other: &str) -> String {
    let idx = (lang.0.plural)(n);
    let text = lang.catalog().plural(one, other, idx).unwrap_or(if n == 1 { one } else { other });
    fmt(text, &[("n", &n.to_string())])
}

#[cfg(test)]
mod tests {
    use super::catalog::{parse_entries, placeholders};
    use super::*;

    const JA: fn() -> Lang = || Lang::from_code("ja").expect("ja registered");
    const ZH: fn() -> Lang = || Lang::from_code("zh-hant").expect("zh-hant registered");
    const CS: fn() -> Lang = || Lang::from_code("cs").expect("cs registered");
    const ID: fn() -> Lang = || Lang::from_code("id").expect("id registered");
    const VI: fn() -> Lang = || Lang::from_code("vi").expect("vi registered");
    const HI: fn() -> Lang = || Lang::from_code("hi").expect("hi registered");

    #[test]
    fn hindi_tags_resolve() {
        for tag in ["hi", "hi-IN", "hi_IN", "hi_IN.UTF-8"] {
            assert_eq!(lang_from_tag(tag), Some(HI()), "{tag}");
        }
        assert_eq!(HI().name(), "हिन्दी");
        assert_eq!(Lang::from_pref("hi"), HI());
        assert_eq!(Lang::from_pref("HI"), HI());
    }

    #[test]
    fn indonesian_tags_resolve() {
        for tag in ["id", "id-ID", "id_ID", "id_ID.UTF-8"] {
            assert_eq!(lang_from_tag(tag), Some(ID()), "{tag}");
        }
        assert_eq!(ID().name(), "Bahasa Indonesia");
    }

    #[test]
    fn vietnamese_tags_resolve() {
        for tag in ["vi", "vi-VN", "vi_VN", "vi_VN.UTF-8"] {
            assert_eq!(lang_from_tag(tag), Some(VI()), "{tag}");
        }
        assert_eq!(VI().name(), "Tiếng Việt");
        assert_eq!(Lang::from_pref("vi"), VI());
        assert_eq!(Lang::from_pref("VI"), VI());
    }

    #[test]
    fn picker_lists_english_first_then_names_alphabetically() {
        let order = Lang::for_picker();
        assert_eq!(order.first().map(|l| l.code()), Some("en"));
        assert_eq!(order.len(), Lang::all().count());
        let keys: Vec<String> = order[1..].iter().map(|l| name_sort_key(l.name())).collect();
        assert!(keys.windows(2).all(|w| w[0] <= w[1]), "{keys:?}");
        let pos = |code: &str| order.iter().position(|l| l.code() == code).unwrap_or_else(|| panic!("{code} registered"));
        assert!(pos("cs") < pos("de"), "Čeština sorts under C, not after Z");
        assert!(pos("it") < pos("hu"), "Magyar sorts under M, not under its code");
        assert!(pos("tr") < pos("el"), "Latin names come before Greek");
        assert!(pos("uk") < pos("ja"), "Cyrillic names come before CJK");
        assert_eq!(name_sort_key("Tiếng Việt"), "tieng viet");
        assert_eq!(name_sort_key("Türkçe"), "turkce");
    }

    #[test]
    fn tags_map_to_languages() {
        assert_eq!(lang_from_tag("ja_JP.UTF-8"), Some(JA()));
        assert_eq!(lang_from_tag("ja-JP"), Some(JA()));
        assert_eq!(lang_from_tag("en_US.UTF-8"), Some(Lang::EN));
        assert_eq!(lang_from_tag("C"), Some(Lang::EN));
        assert_eq!(lang_from_tag("POSIX"), Some(Lang::EN));
        assert_eq!(lang_from_tag("cs_CZ.UTF-8"), Some(CS()));
        assert_eq!(lang_from_tag("cs-CZ"), Some(CS()));
        assert_eq!(lang_from_tag("fr_FR"), Lang::from_code("fr"));
        assert_eq!(lang_from_tag("de_DE"), Lang::from_code("de"));
        assert_eq!(lang_from_tag("de-AT"), Lang::from_code("de"));
        assert_eq!(lang_from_tag("it_IT.UTF-8"), Lang::from_code("it"));
        assert_eq!(lang_from_tag("it-CH"), Lang::from_code("it"));
        assert_eq!(lang_from_tag("nl_NL.UTF-8"), Lang::from_code("nl"));
        assert_eq!(lang_from_tag("nl-BE"), Lang::from_code("nl"));
        assert_eq!(lang_from_tag("hu_HU.UTF-8"), Lang::from_code("hu"));
        assert_eq!(lang_from_tag("hu-HU"), Lang::from_code("hu"));
        assert_eq!(lang_from_tag("tr_TR.UTF-8"), Lang::from_code("tr"));
        assert_eq!(lang_from_tag("tr-TR"), Lang::from_code("tr"));
        assert_eq!(lang_from_tag("tr"), Lang::from_code("tr"));
        // Traditional Chinese: by region, by script, and with a region after the script.
        assert_eq!(lang_from_tag("zh_TW.UTF-8"), Some(ZH()));
        assert_eq!(lang_from_tag("zh-TW"), Some(ZH()));
        assert_eq!(lang_from_tag("zh-HK"), Some(ZH()));
        assert_eq!(lang_from_tag("zh_MO"), Some(ZH()));
        assert_eq!(lang_from_tag("zh-Hant"), Some(ZH()));
        assert_eq!(lang_from_tag("zh-Hant-TW"), Some(ZH()));
        assert_eq!(lang_from_tag("zh-Hant-HK"), Some(ZH()));
        // Simplified Chinese locales never pick up the Traditional catalog: they resolve to `zh-hans`.
        for tag in ["zh-CN", "zh_CN.UTF-8", "zh_SG", "zh-Hans", "zh-Hans-CN", "zh"] {
            assert_ne!(lang_from_tag(tag), Some(ZH()), "{tag}");
            assert_eq!(lang_from_tag(tag), Lang::from_code("zh-hans"), "{tag}");
        }
        assert_eq!(lang_from_tag(""), None);
        assert_eq!(lang_from_tag("_"), None);
    }

    #[test]
    fn candidates_walk_from_specific_to_general() {
        assert_eq!(candidates("pt_BR.UTF-8"), ["pt-br", "pt"]);
        assert_eq!(candidates("zh_TW"), ["zh-tw", "zh-hant", "zh"]);
        assert_eq!(candidates("zh-CN"), ["zh-cn", "zh-hans", "zh"]);
        assert_eq!(candidates("zh-Hant-HK"), ["zh-hant-hk", "zh-hant", "zh"]);
    }

    #[test]
    fn preferences_resolve_with_fallback() {
        assert_eq!(Lang::from_pref("ja"), JA());
        assert_eq!(Lang::from_pref("JA"), JA());
        assert_eq!(Lang::from_pref("zh-hant"), ZH());
        assert_eq!(Lang::from_pref("ZH-Hant"), ZH());
        assert_eq!(Lang::from_pref("en"), Lang::EN);
        // `auto` and unknown codes follow the system (English under test).
        assert_eq!(Lang::from_pref("auto"), Lang::EN);
        assert_eq!(Lang::from_pref("xx-unknown"), Lang::EN);
    }

    #[test]
    fn simplified_chinese_resolves_from_preferences() {
        let zh = Lang::from_code("zh-hans").expect("zh-hans registered");
        assert_eq!(Lang::from_pref("ZH-Hans"), zh);
        assert_eq!(tr(zh, "no such label"), "no such label");
    }

    #[test]
    fn spanish_resolves_and_pluralises() {
        let es = Lang::from_code("es").expect("es registered");
        for tag in ["es", "es_ES.UTF-8", "es-MX", "es-419"] {
            assert_eq!(lang_from_tag(tag), Some(es), "{tag}");
        }
        let forms: Vec<usize> = [0, 1, 3].into_iter().map(es.0.plural).collect();
        assert_eq!(forms, [1, 0, 1]);
    }

    #[test]
    fn hungarian_resolves_and_keeps_the_singular_after_counts() {
        let hu = Lang::from_code("hu").expect("hu registered");
        for tag in ["hu", "hu_HU.UTF-8", "hu-HU"] {
            assert_eq!(lang_from_tag(tag), Some(hu), "{tag}");
        }
        assert_eq!(hu.name(), "Magyar");
        let forms: Vec<usize> = [0, 1, 3, 21].into_iter().map(hu.0.plural).collect();
        assert_eq!(forms, [0, 0, 0, 0]);
    }

    #[test]
    fn brazilian_portuguese_resolves_and_pluralises() {
        let pt = Lang::from_code("pt-br").expect("pt-br registered");
        for tag in ["pt", "pt-BR", "pt_BR.UTF-8", "pt-PT", "pt_PT.UTF-8"] {
            assert_eq!(lang_from_tag(tag), Some(pt), "{tag}");
        }
        assert_eq!(candidates("pt_PT"), ["pt-pt", "pt-br", "pt"]);
        let forms: Vec<usize> = [0, 1, 2].into_iter().map(pt.0.plural).collect();
        assert_eq!(forms, [0, 0, 1]);
    }

    #[test]
    fn lookups_fall_back_to_english() {
        assert_eq!(tr(JA(), "no such label"), "no such label");
        assert_eq!(tr(Lang::EN, "Layer"), "Layer");
        assert_eq!(tr(ZH(), "no such label"), "no such label");
        assert_eq!(tr_id(ZH(), "no.such.id", "Layer"), tr(ZH(), "Layer"));
        assert_eq!(tr_id(JA(), "no.such.id", "Layer"), tr(JA(), "Layer"));
        assert_eq!(tr_ctx(JA(), "no such context", "Layer"), tr(JA(), "Layer"));
        assert_eq!(trn(JA(), 3, "{n} no such message", "{n} no such messages"), "3 no such messages");
    }

    #[test]
    fn russian_plural_rule() {
        let ru = Lang::from_code("ru").expect("ru registered");
        let forms: Vec<usize> = [0, 1, 2, 5, 11, 12, 14, 21, 22, 25, 101, 111, 112].into_iter().map(ru.0.plural).collect();
        assert_eq!(forms, [2, 0, 1, 2, 2, 2, 2, 0, 1, 2, 0, 2, 2]);
    }

    #[test]
    fn ukrainian_resolves_locales_and_preferences() {
        let uk = Lang::from_code("uk").expect("uk registered");
        assert_eq!(uk.name(), "Українська");
        for tag in ["uk", "UK", "uk-UA", "uk_UA", "uk_UA.UTF-8", "uk-UA@euro", "uk-Cyrl-UA"] {
            assert_eq!(lang_from_tag(tag), Some(uk), "{tag}");
            assert_eq!(Lang::from_pref(tag), uk, "{tag}");
        }
        assert_eq!(tr(uk, "unknown translation"), "unknown translation");
    }

    #[test]
    fn ukrainian_plural_rule_handles_teens_and_compound_counts() {
        let uk = Lang::from_code("uk").expect("uk registered");
        let counts = [0, 1, 2, 4, 5, 11, 12, 14, 19, 21, 22, 24, 25, 100, 101, 111, 112, 114, 121, 122, u64::MAX];
        let forms: Vec<usize> = counts.into_iter().map(uk.0.plural).collect();
        assert_eq!(forms, [2, 0, 1, 1, 2, 2, 2, 2, 2, 0, 1, 1, 2, 2, 0, 2, 2, 2, 0, 1, 2]);
    }

    #[test]
    fn serbian_resolves_locales_and_preferences() {
        let sr = Lang::from_code("sr").expect("sr registered");
        assert_eq!(sr.name(), "Srpski");
        for tag in ["sr", "SR", "sr-RS", "sr_RS", "sr_RS.UTF-8", "sr_RS@latin", "sr-Latn", "sr-Latn-RS", "sr-Cyrl-RS", "sr-ME"] {
            assert_eq!(lang_from_tag(tag), Some(sr), "{tag}");
            assert_eq!(Lang::from_pref(tag), sr, "{tag}");
        }
    }

    #[test]
    fn serbian_plural_rule_handles_teens_and_compound_counts() {
        let sr = Lang::from_code("sr").expect("sr registered");
        let counts = [0, 1, 2, 4, 5, 11, 12, 14, 21, 22, 25, 101, 111, 112, 122];
        let forms: Vec<usize> = counts.into_iter().map(sr.0.plural).collect();
        assert_eq!(forms, [2, 0, 1, 1, 2, 2, 2, 2, 0, 1, 2, 0, 2, 2, 1]);
    }

    #[test]
    fn catalog_kinds_are_parsed_and_looked_up() {
        let c = Catalog::parse("# c\n\tHello\tこんにちは\n@id\tfile.save\t保存する\nmenu\tWindows\tウィンドウ群\n@plural\t{n} file|{n} files\t{n} 個\n\n");
        assert_eq!(c.plain("Hello"), Some("こんにちは"));
        assert_eq!(c.id("file.save"), Some("保存する"));
        assert_eq!(c.contextual("menu", "Windows"), Some("ウィンドウ群"));
        assert_eq!(c.contextual("other", "Windows"), None);
        assert_eq!(c.plural("{n} file", "{n} files", 0), Some("{n} 個"));
        assert_eq!(c.plural("{n} file", "{n} files", 5), Some("{n} 個"), "an index past the forms clamps");
    }

    #[test]
    fn malformed_lines_are_reported_not_fatal() {
        let (entries, errors) = parse_entries("\tok\tはい\nno tabs here\n\tonly\n\ta\tb\tc\textra\n\t\tempty source\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(errors.len(), 4, "{errors:?}");
        assert_eq!(parse_entries("\ta\\tb\tx\\ny\\\\z\n").0[0], (String::new(), "a\tb".into(), "x\ny\\z".into()));
    }

    #[test]
    fn plurals_and_placeholders() {
        assert_eq!(trn(Lang::EN, 1, "{n} item", "{n} items"), "1 item");
        assert_eq!(trn(Lang::EN, 0, "{n} item", "{n} items"), "0 items");
        assert_eq!(trn(Lang::EN, 7, "{n} item", "{n} items"), "7 items");
        let cs = fixture_lang(plural_cs, "@plural\t{n} item|{n} items\t{n} one|{n} few|{n} other\n");
        for (n, expected) in [(0, "0 other"), (1, "1 one"), (3, "3 few"), (5, "5 other")] {
            assert_eq!(trn(cs, n, "{n} item", "{n} items"), expected);
        }
        let ja = fixture_lang(plural_none, "@plural\t{n} item|{n} items\t{n} only\n");
        assert_eq!(trn(ja, 1, "{n} item", "{n} items"), "1 only");
        assert_eq!(trn(ja, 7, "{n} item", "{n} items"), "7 only");
        assert_eq!(trn(ja, 7, "{n} file", "{n} files"), "7 files", "a missing row falls back to English");
        assert_eq!(fmt("{b} before {a}", &[("a", "x"), ("b", "y"), ("c", "z")]), "y before x");
        assert_eq!(fmt("{missing}", &[]), "{missing}");
        assert_eq!(placeholders("a {x} b {y} {"), ["x", "y"]);
    }

    #[test]
    fn french_resolves_and_pluralises() {
        let fr = Lang::from_code("fr").expect("fr registered");
        for tag in ["fr", "fr_FR.UTF-8", "fr-CA", "fr_BE", "fr-CH"] {
            assert_eq!(lang_from_tag(tag), Some(fr), "{tag}");
        }
        let forms: Vec<usize> = [0, 1, 2, 5, 100, u64::MAX].into_iter().map(fr.0.plural).collect();
        assert_eq!(forms, [0, 0, 1, 1, 1, 1]);
    }

    /// A language whose catalog is `text`, for testing lookups without depending on a bundled
    /// catalog's wording.
    fn fixture_lang(plural: fn(u64) -> usize, text: &str) -> Lang {
        let catalog = OnceLock::new();
        let _ = catalog.set(Catalog::parse(text));
        Lang(Box::leak(Box::new(LangInfo { code: "xx", name: "x", source: b"", plural, complete_menus: false, catalog })))
    }

    #[test]
    fn id_and_context_rows_win_over_the_plain_label() {
        let lang = fixture_lang(plural_one_other, "\tAll\tplain all\n@id\tselect.all\tid all\nnavigator\tZoom\tcontext zoom\n\tZoom\tplain zoom\n");
        assert_eq!(tr_id(lang, "select.all", "All"), "id all");
        assert_eq!(tr_id(lang, "edit.other", "All"), "plain all");
        assert_eq!(tr(lang, "All"), "plain all");
        assert_eq!(tr_ctx(lang, "navigator", "Zoom"), "context zoom");
        assert_eq!(tr_ctx(lang, "other", "Zoom"), "plain zoom");
        assert_eq!(tr_ctx(lang, "navigator", "Unlisted"), "Unlisted");
    }

    #[test]
    fn polish_plural_rule() {
        let forms: Vec<usize> = [0, 1, 2, 4, 5, 12, 14, 21, 22, 25, 112, 122].into_iter().map(plural_polish).collect();
        assert_eq!(forms, [2, 0, 1, 1, 2, 2, 2, 2, 1, 2, 2, 1]);
    }

    #[test]
    fn czech_plural_rule() {
        let forms: Vec<usize> = [0, 1, 2, 3, 4, 5, 11, 12, 21, 22, 100, u64::MAX].into_iter().map(plural_cs).collect();
        assert_eq!(forms, [2, 0, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2]);
    }

    /// The packed catalogs in the binary unpack to exactly the `.tsv` files.
    #[test]
    fn packed_catalogs_match_their_files() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/i18n");
        for l in LANGUAGES.iter().skip(1) {
            let file = std::fs::read_to_string(dir.join(format!("{}.tsv", l.code))).unwrap();
            assert_eq!(unpack(l.source).unwrap(), file, "{}", l.code);
            assert!(l.source.len() < file.len() / 2, "{}: stored deflated", l.code);
        }
        assert_eq!(LANGUAGES[0].text(), "", "English has no catalog");
    }

    /// Corrupt catalog data is an error (English is shown), not a panic.
    #[test]
    fn corrupt_catalog_data_is_an_error() {
        assert!(inflate(b"\xff\xff\xff\xff not deflate").is_err());
        let ja = LANGUAGES.iter().find(|l| l.code == "ja").unwrap();
        assert!(inflate(&ja.source[..ja.source.len() / 2]).is_err(), "truncated");
        let info: &'static LangInfo = Box::leak(Box::new(LangInfo {
            code: "xx",
            name: "x",
            source: b"\x00garbage",
            plural: plural_none,
            complete_menus: false,
            catalog: OnceLock::new(),
        }));
        assert_eq!(info.text(), "");
        assert_eq!(tr(Lang(info), "Layer"), "Layer", "falls back to English");
    }

    /// Packed lines unpack to their keys; references to keys that don't exist are an error, not a
    /// panic.
    #[test]
    fn packed_lines_unpack_and_bad_references_are_errors() {
        let keys = ["\tOpen", "@id\tfile.open", "\tClose"];
        let packed = "# c\n\x01Ouvrir\n\x02-1\tOuvrir\n\x02+1\tFermer\n\x03\x01x\n";
        assert_eq!(expand(packed, &keys).unwrap(), "# c\n\tOpen\tOuvrir\n\tOpen\tOuvrir\n\tClose\tFermer\n\x01x\n");
        for bad in ["\x02-1\tx", "\x02 3\tx", "\x023", "\x029223372036854775807\tx", "\x02-9223372036854775808\tx", "\x01a\n\x01b\n\x01c\n\x01d"] {
            assert!(expand(bad, &keys).is_err(), "{bad:?}");
        }
    }

    /// Every bundled catalog is well-formed and consistent with its sources.
    #[test]
    fn bundled_catalogs_are_consistent() {
        for l in &LANGUAGES {
            assert!(l.code == l.code.to_ascii_lowercase() && !l.name.is_empty(), "{}", l.code);
            let (entries, errors) = parse_entries(&l.text());
            assert!(errors.is_empty(), "{}: {errors:?}", l.code);
            let mut seen = std::collections::HashSet::new();
            for (ctx, src, tr) in &entries {
                assert!(seen.insert((ctx.clone(), src.clone())), "{}: duplicate {ctx:?} {src:?}", l.code);
                if ctx == "@plural" {
                    let one_other: Vec<&str> = src.split('|').collect();
                    assert_eq!(one_other.len(), 2, "{}: plural source must be `one|other`: {src:?}", l.code);
                    let forms = (0..=1000).map(l.plural).max().unwrap_or(0) + 1;
                    assert_eq!(tr.split('|').count(), forms, "{}: {forms} plural forms expected in {src:?}", l.code);
                    for form in tr.split('|') {
                        let mut want = placeholders(one_other[1]);
                        let mut got = placeholders(form);
                        want.sort_unstable();
                        got.sort_unstable();
                        assert_eq!(want, got, "{}: placeholders differ in {src:?}", l.code);
                    }
                    continue;
                }
                let mut want = placeholders(src);
                let mut got = placeholders(tr);
                want.sort_unstable();
                got.sort_unstable();
                assert_eq!(want, got, "{}: placeholders differ in {src:?}", l.code);
                if ctx.is_empty() {
                    assert_eq!(src.ends_with('…'), tr.ends_with('…'), "{}: ellipsis mismatch: {src:?}", l.code);
                }
                if ctx == "@id" {
                    assert!(crate::menus::is_live(src) || crate::menu_catalog::CATALOG.iter().any(|m| m.3 == src), "{}: unknown command id {src:?}", l.code);
                }
            }
        }
    }
}

#[cfg(test)]
mod live_tests;
