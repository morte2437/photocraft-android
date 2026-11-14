//! UI localisation. Strings in code stay English and are the default lookup keys; the craft-i18n
//! catalogs map them to display text at render time. Command ids, menu paths used for logic, the
//! control channel, the CLI and MCP never see translated text, so agents and scripts are
//! unaffected. A string without a translation is shown in English, and a missing translation never
//! fails a build or a release. A few tests check named rows of the pinned catalogs, so moving
//! `craft-i18n.rev` to a commit that drops one fails that test until it is updated.
//!
//! The catalogs live in [storytold/craft-i18n](https://github.com/storytold/craft-i18n)
//! (`locales/<code>/common.tsv`, shared by the Crafting Apps, and `photocraft.tsv`), not in this
//! repository. `build.rs` embeds them from `CRAFT_I18N_DIR` (CI and releases check out the commit
//! in `craft-i18n.rev`); without it the build is English only. Lookup, fallback (PhotoCraft's rows
//! over shared ones), plural forms (CLDR) and language names are the `craft-i18n` crate's
//! (craft-libs). Which languages are offered, how the system locale picks one, reading direction
//! and `{name}` placeholders stay here.
//!
//! # Adding or changing a translation
//! In craft-i18n: add rows to `locales/<code>/photocraft.tsv` (its `docs/integration.md`). They
//! show here once `craft-i18n.rev` moves to a commit that has them. A feature PR here adds the
//! English only; `cargo xtask i18n-report` lists what each language lacks.
//!
//! # Adding a language
//! 1. craft-i18n gets `locales/<code>/photocraft.tsv` with its `@id language.name` row.
//! 2. Add one row to [`LANGUAGES`].
//!
//! That is all: the Preferences dropdown and the system-locale match pick it up from the registry.
//! A right-to-left language (Arabic, Hebrew, Persian, Urdu, see [`dir_for_code`]) also mirrors the
//! window's chrome ([`crate::direction`]).
//!
//! # Looking strings up
//! - [`tr`]: a plain string. [`tr_ctx`]: when one English word needs different translations.
//! - [`tr_id`]: a command-id keyed string with the English label as fallback (menu items), so a
//!   translation survives rewording of the English text and can differ per command.
//! - [`trn`]: plural-aware (`{n}` is filled in). [`fmt`]: fill `{name}` placeholders after [`tr`];
//!   translators may reorder placeholders freely.

mod casing;
mod system;

#[cfg(test)]
pub(crate) use system::with_decimal_separator;
pub use system::{decimal_separator, system_lang};

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{LazyLock, OnceLock};

use craft_i18n::I18n;

include!(concat!(env!("OUT_DIR"), "/craft_i18n.rs"));

/// The embedded catalogs, offering exactly the languages in [`LANGUAGES`]. Built on first use; a
/// locale is parsed on its first lookup.
static I18N: LazyLock<I18n> = LazyLock::new(|| {
    let offered: Vec<&str> = LANGUAGES.iter().map(|l| l.code).collect();
    let (i18n, problems) = I18n::new(CRAFT_I18N, &offered);
    for problem in problems {
        log::warn!("interface catalogs: {problem}");
    }
    i18n
});

/// One UI language PhotoCraft offers, when its catalogs are embedded: what the app needs beyond
/// them. Its name and translations come from craft-i18n.
pub struct LangInfo {
    /// BCP 47 code, lowercase (`ja`, `zh-hans`, `pt-br`, `sr-latn`). Also the `interface.language`
    /// value and the craft-i18n locale.
    pub code: &'static str,
    /// Does the catalog cover every menu string? (`cargo xtask i18n-report` lists what it lacks)
    pub complete_menus: bool,
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

/// How many languages [`LANGUAGES`] lists.
const LANGUAGE_COUNT: usize = 35;

/// The registry. English first: it is the fallback and the source language. Plural forms follow
/// each language's CLDR rules (the craft-i18n crate).
pub static LANGUAGES: [LangInfo; LANGUAGE_COUNT] = [
    LangInfo { code: "en", complete_menus: false },
    LangInfo { code: "ja", complete_menus: true },
    LangInfo { code: "zh-hans", complete_menus: true },
    // Traditional Chinese in the vocabulary used in Taiwan; `zh-TW`, `zh-HK`, `zh-MO` and `zh-Hant-*`
    // locales all resolve here (see `candidates`).
    LangInfo { code: "zh-hant", complete_menus: true },
    LangInfo { code: "es", complete_menus: true },
    LangInfo { code: "ru", complete_menus: true },
    LangInfo { code: "uk", complete_menus: true },
    // Serbian (Latin script, Ekavian). `sr-RS`, `sr-Latn-*` and `sr_RS@latin` resolve here; so do
    // plain `sr` and Cyrillic `sr` locales, which read Latin rather than English (see `candidates`).
    // Saved as `sr` before the catalogs moved to craft-i18n (the engine migrates it).
    LangInfo { code: "sr-latn", complete_menus: true },
    // Bosnian (Latin script, Ijekavian). `bs-BA`, `bs-Latn-*` and `bs_BA.UTF-8` resolve here; so do
    // Cyrillic `bs` locales, which read Latin rather than English.
    LangInfo { code: "bs", complete_menus: true },
    LangInfo { code: "cs", complete_menus: true },
    LangInfo { code: "fr", complete_menus: true },
    LangInfo { code: "id", complete_menus: true },
    LangInfo { code: "ko", complete_menus: true },
    LangInfo { code: "pl", complete_menus: true },
    LangInfo { code: "de", complete_menus: true },
    // Brazilian Portuguese; `pt`, `pt-BR` and `pt-PT` locales all resolve here (see `candidates`).
    LangInfo { code: "pt-br", complete_menus: true },
    LangInfo { code: "el", complete_menus: true },
    // Dutch; `nl-NL` and `nl-BE` locales both resolve here.
    LangInfo { code: "nl", complete_menus: true },
    LangInfo { code: "it", complete_menus: true },
    // Hungarian: a count takes the singular noun ("3 réteg"), so both plural forms read the same.
    LangInfo { code: "hu", complete_menus: true },
    // Turkish; `tr`, `tr-TR` and `tr_TR` locales resolve here.
    LangInfo { code: "tr", complete_menus: true },
    // Azerbaijani (Latin script); regional and explicit Latin-script tags resolve here.
    LangInfo { code: "az", complete_menus: true },
    // Vietnamese; `vi-VN` resolves here.
    LangInfo { code: "vi", complete_menus: true },
    // Thai; `th`, `th-TH` and `th_TH.UTF-8` locales resolve here. Thai has no plural inflection.
    LangInfo { code: "th", complete_menus: true },
    // Hindi; `hi`, `hi-IN` locales resolve here.
    LangInfo { code: "hi", complete_menus: false },
    // Arabic (Modern Standard); `ar-SA`, `ar-EG` and the other regional locales resolve here. The
    // chrome mirrors right to left (`dir_for_code`), and egui lays each label out right to left.
    LangInfo { code: "ar", complete_menus: true },
    // Swedish; `sv-SE` and `sv-FI` locales resolve here.
    LangInfo { code: "sv", complete_menus: true },
    // Finnish; `fi-FI` resolves here (Swedish in Finland, `sv-FI`, stays Swedish). The partitive
    // singular follows any number but 1 ("1 taso", "2 tasoa").
    LangInfo { code: "fi", complete_menus: true },
    // Danish; `da-DK` and the other `da-*` locales resolve here.
    LangInfo { code: "da", complete_menus: true },
    // Norwegian Bokmål; `nb-NO`, and the macrolanguage `no` and Nynorsk `nn` from POSIX locales,
    // resolve here (see `candidates`).
    LangInfo { code: "nb", complete_menus: true },
    // Hebrew; `he-il` and `he_IL` resolve here. The chrome mirrors right to left (`dir_for_code`),
    // like Arabic. Its `two` plural form repeats `other`, as modern Hebrew software UI does.
    LangInfo { code: "he", complete_menus: true },
    // Urdu; written right to left like Arabic (`dir_for_code`).
    LangInfo { code: "ur", complete_menus: true },
    // Romanian; `ro-RO` and `ro-MD` resolve here.
    LangInfo { code: "ro", complete_menus: true },
    // Tamil; `ta-IN`, `ta-LK` and `ta_IN.UTF-8` resolve here.
    LangInfo { code: "ta", complete_menus: true },
    // Georgian: nouns stay singular after a numeral (`შრე: 5`, `5 შრე`), so both plural forms read
    // the same.
    LangInfo { code: "ka", complete_menus: true },
];

/// Per language of [`LANGUAGES`], PhotoCraft's own catalog rows as written (escaped): (context,
/// source) to translation, read on first use ([`Lang::owns`]).
static OWN_ROWS: [OnceLock<HashMap<(&'static str, &'static str), &'static str>>; LANGUAGE_COUNT] = [const { OnceLock::new() }; LANGUAGE_COUNT];

/// A language the UI can be shown in: English, or one of [`LANGUAGES`] whose catalogs are
/// embedded.
#[derive(Clone, Copy)]
pub struct Lang {
    lang: craft_i18n::Lang,
    /// The catalogs it is looked up in: the embedded ones ([`I18N`]), or a test fixture's.
    catalogs: Option<&'static I18n>,
}

impl std::fmt::Debug for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lang({})", self.code())
    }
}

impl PartialEq for Lang {
    fn eq(&self, other: &Self) -> bool {
        self.code() == other.code()
    }
}

impl Eq for Lang {}

impl Lang {
    pub const EN: Lang = Lang { lang: craft_i18n::Lang::EN, catalogs: None };

    fn i18n(self) -> &'static I18n {
        self.catalogs.unwrap_or_else(|| LazyLock::force(&I18N))
    }

    fn info(self) -> Option<&'static LangInfo> {
        let code = self.code();
        LANGUAGES.iter().find(|l| l.code == code)
    }

    pub fn code(self) -> &'static str {
        self.i18n().code(self.lang)
    }

    /// A language by its exact code (any case): one of [`LANGUAGES`] whose catalogs are embedded.
    pub fn from_code(code: &str) -> Option<Lang> {
        let info = LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code))?;
        I18N.from_code(info.code).map(|lang| Lang { lang, catalogs: None })
    }

    /// Resolve the `interface.language` preference: a language code, or `auto` (and anything
    /// unknown, e.g. a code from a newer version) to follow the system locale.
    pub fn from_pref(pref: &str) -> Lang {
        if pref.eq_ignore_ascii_case("auto") {
            return system_lang();
        }
        Lang::from_code(pref).or_else(|| lang_from_tag(pref)).unwrap_or_else(system_lang)
    }

    /// Every language the interface can be shown in: [`LANGUAGES`] in order, those whose catalogs
    /// are embedded (English alone in a build without craft-i18n).
    pub fn all() -> impl Iterator<Item = Lang> {
        LANGUAGES.iter().filter_map(|l| Lang::from_code(l.code))
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

    /// The language's name in itself (its `@id language.name` row), shown in the Preferences
    /// dropdown.
    pub fn name(self) -> &'static str {
        self.i18n().name(self.lang)
    }

    /// Does this language's catalog claim to cover every menu string and `tl!` literal?
    pub fn complete_menus(self) -> bool {
        self.info().is_some_and(|l| l.complete_menus)
    }

    /// The language's reading direction.
    pub fn dir(self) -> Dir {
        dir_for_code(self.code())
    }

    /// Whether `translated`, the lookup's text for (`context`, `source`), comes from PhotoCraft's
    /// own catalog (`photocraft.tsv`, not the shared `common.tsv`). The shared catalog's `@id` rows
    /// are keyed by the other apps' command ids, which can name another command, so command-id
    /// lookups ask this; every `@id` row PhotoCraft had is in its own catalog. Comparing the text,
    /// not only the key, keeps a shared row out when craft-i18n skipped a malformed own row. A test
    /// fixture has no shared catalog: its rows are its own.
    fn owns(self, context: &str, source: &str, translated: &str) -> bool {
        if let Some(fixture) = self.catalogs {
            return fixture.get(self.lang, context, source) == Some(translated);
        }
        let code = self.code();
        let Some(slot) = LANGUAGES.iter().position(|l| l.code == code).and_then(|i| OWN_ROWS.get(i)) else { return false };
        let rows: &HashMap<(&str, &str), &str> = slot.get_or_init(|| {
            app_catalog(code)
                .lines()
                .filter(|l| !l.starts_with('#'))
                .filter_map(|l| {
                    let mut columns = l.split('\t');
                    Some(((columns.next()?, columns.next()?), columns.next()?))
                })
                .collect()
        });
        let escape = |s: &str| s.replace('\\', "\\\\").replace('\n', "\\n").replace('\t', "\\t");
        rows.get(&(escape(context).as_str(), escape(source).as_str())).is_some_and(|own| *own == escape(translated))
    }
}

impl Lang {
    /// The embedded catalog texts this language reads, the shared one then PhotoCraft's; none for
    /// English. For checks that the interface fonts draw every translation.
    #[cfg(test)]
    pub(crate) fn catalog_texts(self) -> impl Iterator<Item = &'static str> {
        let code = self.code();
        CRAFT_I18N.iter().filter(move |(c, ..)| c.eq_ignore_ascii_case(code)).flat_map(|(_, shared, app)| [*shared, *app])
    }
}

/// PhotoCraft's own embedded catalog text (`photocraft.tsv`) in the locale `code`; empty when
/// there is none.
fn app_catalog(code: &str) -> &'static str {
    CRAFT_I18N.iter().find(|(c, ..)| c.eq_ignore_ascii_case(code)).map_or("", |(_, _, app)| *app)
}

/// Whether this build embeds the craft-i18n catalogs (`CRAFT_I18N_DIR` was set). Without them the
/// interface is English only; tests that need a translation skip with a note.
pub fn catalogs_embedded() -> bool {
    !CRAFT_I18N.is_empty()
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
    if primary == "sr" && !out.iter().any(|c| c == "sr-latn") {
        // The only Serbian catalog is Latin (`sr-latn`). Plain, regional (`sr-RS`, `sr-ME`) and
        // Cyrillic (`sr-Cyrl-RS`) tags read it rather than English, as when it was PhotoCraft's
        // `sr` catalog; craft-i18n's own matching never takes `sr` to a script it doesn't name.
        out.insert(out.len() - 1, "sr-latn".to_string());
    }
    if matches!(primary, "no" | "nn") && !out.iter().any(|c| c == "nb") {
        // The only Norwegian catalog is Bokmål; the macrolanguage code and Nynorsk use it rather
        // than English.
        out.insert(out.len() - 1, "nb".to_string());
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
    lang.i18n().get(lang.lang, "", s).is_some()
}

/// Translate an English UI string into the current language ([`tr`] with [`current`]).
pub fn t(s: &str) -> &str {
    tr(current(), s)
}

/// Translate an English UI string; unknown strings come back unchanged.
pub fn tr(lang: Lang, s: &str) -> &str {
    lang.i18n().t(lang.lang, s)
}

/// Like [`tr`], for an English string that needs a disambiguating `context`.
pub fn tr_ctx<'a>(lang: Lang, context: &str, s: &'a str) -> &'a str {
    lang.i18n().tc(lang.lang, context, s)
}

/// A string keyed by its command id, falling back to the translation of the English `label`. Only
/// PhotoCraft's own `@id` rows count ([`own_id`]).
pub fn tr_id<'a>(lang: Lang, id: &str, label: &'a str) -> &'a str {
    if own_id(lang, id, label) { lang.i18n().tid(lang.lang, id, label) } else { tr(lang, label) }
}

/// Whether `lang` has PhotoCraft's own `@id` row for `id` ([`Lang::owns`]: command ids are each
/// app's own) and it keeps the `{placeholders}` and trailing `…` of the English `label`. craft-i18n
/// can't check that, because an `@id` row's source is the id, so a row that doesn't is skipped
/// here and the label's translation shows instead.
fn own_id(lang: Lang, id: &str, label: &str) -> bool {
    lang.i18n().get(lang.lang, "@id", id).is_some_and(|translated| lang.owns("@id", id, translated) && same_shape(translated, label))
}

/// Whether `translated` has the `{placeholders}` (in any order) and the trailing `…` of `english`,
/// the check craft-i18n makes on every row whose source is English text, and no `{` left open or
/// `}` without its `{`.
fn same_shape(translated: &str, english: &str) -> bool {
    let placeholders = |text: &str| -> Option<Vec<String>> {
        let mut names = text.split('{').skip(1).map(|p| p.split_once('}').map(|(name, _)| name.to_owned())).collect::<Option<Vec<_>>>()?;
        if text.matches('}').count() != names.len() {
            return None;
        }
        names.sort_unstable();
        Some(names)
    };
    translated.ends_with('…') == english.ends_with('…') && placeholders(translated).is_some_and(|names| Some(names) == placeholders(english))
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
/// The form is the language's CLDR category; `{n}` is written in `0`–`9` without grouping or
/// isolate marks, as PhotoCraft always has.
pub fn trn(lang: Lang, n: u64, one: &str, other: &str) -> String {
    lang.i18n().trn_plain(lang.lang, n, one, other)
}

/// A test language whose PhotoCraft catalog is `source` (catalog format), so tests of lookup code
/// don't depend on the embedded catalogs' wording. `code` picks the CLDR plural rules.
#[cfg(test)]
pub(crate) fn fixture_lang(code: &'static str, source: &'static str) -> Lang {
    let embedded: &'static craft_i18n::Embedded = Box::leak(Box::new([(code, "", source)]));
    let (i18n, _) = I18n::new(embedded, &[code]);
    let i18n: &'static I18n = Box::leak(Box::new(i18n));
    let lang = i18n.from_code(code).unwrap_or(craft_i18n::Lang::EN);
    Lang { lang, catalogs: Some(i18n) }
}

/// For a test that needs the embedded catalogs: `false`, with a note, when none were embedded
/// (`CRAFT_I18N_DIR` unset), so the test can skip; the interface is then English only.
#[cfg(test)]
pub(crate) fn needs_catalogs() -> bool {
    if !catalogs_embedded() {
        eprintln!("skipped: built without craft-i18n (set CRAFT_I18N_DIR to a craft-i18n checkout); the interface is English only");
    }
    catalogs_embedded()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod live_tests;
