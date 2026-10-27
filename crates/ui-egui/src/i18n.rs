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
    let id = egui::Id::unique("photocraft-ui-language");
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
mod tests;

#[cfg(test)]
mod live_tests;
