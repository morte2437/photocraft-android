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
    western_and_legacy_tags_resolve();
    chinese_tags_resolve_by_region_and_script();
}

fn western_and_legacy_tags_resolve() {
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
}

fn chinese_tags_resolve_by_region_and_script() {
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
    assert_eq!(candidates("nn_NO.UTF-8"), ["nn-no", "nb", "nn"]);
    assert_eq!(candidates("no"), ["nb", "no"]);
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
fn swedish_resolves_and_pluralises() {
    let sv = Lang::from_code("sv").expect("sv registered");
    assert_eq!(sv.name(), "Svenska");
    for tag in ["sv", "SV", "sv-SE", "sv_SE.UTF-8", "sv-FI", "sv_FI.UTF-8", "sv-AX"] {
        assert_eq!(lang_from_tag(tag), Some(sv), "{tag}");
        assert_eq!(Lang::from_pref(tag), sv, "{tag}");
    }
    let forms: Vec<usize> = [0, 1, 2, 21].into_iter().map(sv.0.plural).collect();
    assert_eq!(forms, [1, 0, 1, 1]);
    assert_eq!(tr(sv, "File"), "Arkiv");
    assert_eq!(trn(sv, 1, "{n} layer", "{n} layers"), "1 lager");
    assert_eq!(trn(sv, 3, "{n} layer", "{n} layers"), "3 lager");
}

#[test]
fn finnish_resolves_and_pluralises() {
    let fi = Lang::from_code("fi").expect("fi registered");
    assert_eq!(fi.name(), "Suomi");
    for tag in ["fi", "FI", "fi-FI", "fi_FI.UTF-8"] {
        assert_eq!(lang_from_tag(tag), Some(fi), "{tag}");
        assert_eq!(Lang::from_pref(tag), fi, "{tag}");
    }
    // Filipino and Swedish in Finland don't pick Finnish.
    assert_ne!(lang_from_tag("fil"), Some(fi));
    assert_ne!(lang_from_tag("sv-FI"), Some(fi));
    assert_eq!(tr(fi, "File"), "Tiedosto");
    // The partitive singular follows any number but 1.
    for (n, layer) in [(0, "tasoa"), (1, "taso"), (2, "tasoa"), (21, "tasoa")] {
        assert_eq!(trn(fi, n, "{n} layer", "{n} layers"), format!("{n} {layer}"));
    }
}

#[test]
fn danish_resolves_and_pluralises() {
    let da = Lang::from_code("da").expect("da registered");
    assert_eq!(da.name(), "Dansk");
    for tag in ["da", "DA", "da-DK", "da_DK.UTF-8", "da-GL"] {
        assert_eq!(lang_from_tag(tag), Some(da), "{tag}");
        assert_eq!(Lang::from_pref(tag), da, "{tag}");
    }
    assert_eq!(tr(da, "File"), "Filer");
    assert_eq!(trn(da, 1, "{n} document", "{n} documents"), "1 dokument");
    assert_eq!(trn(da, 3, "{n} document", "{n} documents"), "3 dokumenter");
}

#[test]
fn norwegian_resolves_and_pluralises() {
    let nb = Lang::from_code("nb").expect("nb registered");
    assert_eq!(nb.name(), "Norsk bokmål");
    for tag in ["nb", "NB", "nb-NO", "nb_NO.UTF-8", "no", "no-NO", "no_NO.UTF-8", "nn", "nn-NO", "nn_NO.UTF-8"] {
        assert_eq!(lang_from_tag(tag), Some(nb), "{tag}");
        assert_eq!(Lang::from_pref(tag), nb, "{tag}");
    }
    assert_eq!(tr(nb, "File"), "Fil");
    assert_eq!(trn(nb, 1, "{n} item", "{n} items"), "1 element");
    assert_eq!(trn(nb, 3, "{n} item", "{n} items"), "3 elementer");
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
fn arabic_resolves_locales_and_preferences() {
    let ar = Lang::from_code("ar").expect("ar registered");
    assert_eq!(ar.name(), "العربية");
    for tag in ["ar", "AR", "ar-SA", "ar_EG", "ar_EG.UTF-8", "ar-AE", "ar-MA"] {
        assert_eq!(lang_from_tag(tag), Some(ar), "{tag}");
        assert_eq!(Lang::from_pref(tag), ar, "{tag}");
    }
    assert_eq!(dir_for_code("ar"), Dir::Rtl);
}

#[test]
fn arabic_plural_rule_has_six_forms() {
    let ar = Lang::from_code("ar").expect("ar registered");
    let counts = [0, 1, 2, 3, 10, 11, 99, 100, 101, 102, 103, 111, 199, 1000, u64::MAX];
    let forms: Vec<usize> = counts.into_iter().map(ar.0.plural).collect();
    assert_eq!(forms, [0, 1, 2, 3, 3, 4, 4, 5, 5, 5, 3, 4, 4, 5, 4]);
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
    let info: &'static LangInfo =
        Box::leak(Box::new(LangInfo { code: "xx", name: "x", source: b"\x00garbage", plural: plural_none, complete_menus: false, catalog: OnceLock::new() }));
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
