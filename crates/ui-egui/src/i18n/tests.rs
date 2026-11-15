use super::*;

/// A registered language whose catalogs are embedded. Callers check [`needs_catalogs`] first.
fn lang(code: &str) -> Lang {
    Lang::from_code(code).unwrap_or_else(|| panic!("{code} registered and embedded"))
}

#[test]
fn hindi_has_the_reveal_and_camera_raw_rows() {
    if !needs_catalogs() {
        return;
    }
    let hi = lang("hi");
    assert_eq!(tr(hi, "Reveal in Explorer"), "Explorer में दिखाएँ");
    assert_eq!(tr(hi, "Reveal in Finder"), "Finder में दिखाएँ");
    assert_eq!(tr(hi, "RAW Defaults…"), "RAW डिफ़ॉल्ट…");
    assert!(has(hi, "RAW Defaults…"), "RAW Defaults… is a catalog row, not a fallback");
}

#[test]
fn hindi_tags_resolve() {
    if !needs_catalogs() {
        return;
    }
    let hi = lang("hi");
    for tag in ["hi", "hi-IN", "hi_IN", "hi_IN.UTF-8"] {
        assert_eq!(lang_from_tag(tag), Some(hi), "{tag}");
    }
    assert_eq!(hi.name(), "हिन्दी");
    assert_eq!(Lang::from_pref("hi"), hi);
    assert_eq!(Lang::from_pref("HI"), hi);
}

#[test]
fn georgian_tags_resolve() {
    if !needs_catalogs() {
        return;
    }
    let ka = lang("ka");
    for tag in ["ka", "ka-GE", "ka_GE", "ka_GE.UTF-8"] {
        assert_eq!(lang_from_tag(tag), Some(ka), "{tag}");
    }
    assert_eq!(ka.name(), "ქართული");
    assert_eq!(Lang::from_pref("ka"), ka);
    assert_eq!(Lang::from_pref("KA"), ka);
    assert_eq!(tr(ka, "Layer"), "შრე");
    assert_eq!(tr(ka, "File"), "ფაილი");
    assert_eq!(trn(ka, 1, "{n} layer", "{n} layers"), "შრე: 1");
    assert_eq!(trn(ka, 3, "{n} layer", "{n} layers"), "შრე: 3");
}

#[test]
fn indonesian_vietnamese_and_thai_tags_resolve() {
    if !needs_catalogs() {
        return;
    }
    for tag in ["id", "id-ID", "id_ID", "id_ID.UTF-8"] {
        assert_eq!(lang_from_tag(tag), Some(lang("id")), "{tag}");
    }
    assert_eq!(lang("id").name(), "Bahasa Indonesia");
    for tag in ["vi", "vi-VN", "vi_VN", "vi_VN.UTF-8"] {
        assert_eq!(lang_from_tag(tag), Some(lang("vi")), "{tag}");
    }
    assert_eq!(lang("vi").name(), "Tiếng Việt");
    assert_eq!(Lang::from_pref("VI"), lang("vi"));
    let th = lang("th");
    for tag in ["th", "TH", "th-TH", "th_TH", "th_TH.UTF-8"] {
        assert_eq!(lang_from_tag(tag), Some(th), "{tag}");
        assert_eq!(Lang::from_pref(tag), th, "{tag}");
    }
    assert_eq!(th.name(), "ไทย");
    assert_eq!(tr(th, "File"), "ไฟล์");
    assert_eq!(tr(th, "Layer"), "เลเยอร์");
}

/// Tamil: regional tags resolve, counts take one|other, and placeholders stay untranslated.
#[test]
fn tamil_tags_plurals_and_placeholders() {
    if !needs_catalogs() {
        return;
    }
    let ta = lang("ta");
    for tag in ["ta", "ta-IN", "ta_LK", "ta_IN.UTF-8"] {
        assert_eq!(Lang::from_pref(tag), ta, "{tag}");
    }
    assert_eq!(trn(ta, 1, "{n} layer", "{n} layers"), "1 அடுக்கு");
    assert_eq!(trn(ta, 3, "{n} layer", "{n} layers"), "3 அடுக்குகள்");
    assert!(tr(ta, "Scratch: {size}").contains("{size}"));
    assert_ne!(tr(ta, "Zoom In (hold, drag to scrub)"), "Zoom In (hold, drag to scrub)");
}

#[test]
fn picker_lists_english_first_then_names_alphabetically() {
    let order = Lang::for_picker();
    assert_eq!(order.first().map(|l| l.code()), Some("en"));
    assert_eq!(order.len(), Lang::all().count());
    let keys: Vec<String> = order[1..].iter().map(|l| name_sort_key(l.name())).collect();
    assert!(keys.windows(2).all(|w| w[0] <= w[1]), "{keys:?}");
    assert_eq!(name_sort_key("Tiếng Việt"), "tieng viet");
    assert_eq!(name_sort_key("Türkçe"), "turkce");
    if !needs_catalogs() {
        return;
    }
    assert_eq!(order.len(), LANGUAGES.len(), "every registered language is embedded");
    let pos = |code: &str| order.iter().position(|l| l.code() == code).unwrap_or_else(|| panic!("{code} registered"));
    assert!(pos("cs") < pos("de"), "Čeština sorts under C, not after Z");
    assert!(pos("it") < pos("hu"), "Magyar sorts under M, not under its code");
    assert!(pos("tr") < pos("el"), "Latin names come before Greek");
    assert!(pos("uk") < pos("ja"), "Cyrillic names come before CJK");
}

#[test]
fn western_and_legacy_tags_resolve() {
    assert_eq!(lang_from_tag("en_US.UTF-8"), Some(Lang::EN));
    assert_eq!(lang_from_tag("C"), Some(Lang::EN));
    assert_eq!(lang_from_tag("POSIX"), Some(Lang::EN));
    assert_eq!(lang_from_tag(""), None);
    assert_eq!(lang_from_tag("_"), None);
    if !needs_catalogs() {
        return;
    }
    assert_eq!(lang_from_tag("ja_JP.UTF-8"), Some(lang("ja")));
    assert_eq!(lang_from_tag("ja-JP"), Some(lang("ja")));
    assert_eq!(lang_from_tag("cs_CZ.UTF-8"), Some(lang("cs")));
    assert_eq!(lang_from_tag("cs-CZ"), Some(lang("cs")));
    for (tag, code) in [
        ("fr_FR", "fr"),
        ("de_DE", "de"),
        ("de-AT", "de"),
        ("it_IT.UTF-8", "it"),
        ("it-CH", "it"),
        ("nl_NL.UTF-8", "nl"),
        ("nl-BE", "nl"),
        ("hu_HU.UTF-8", "hu"),
        ("hu-HU", "hu"),
        ("tr_TR.UTF-8", "tr"),
        ("tr-TR", "tr"),
        ("tr", "tr"),
    ] {
        assert_eq!(lang_from_tag(tag), Some(lang(code)), "{tag}");
    }
}

#[test]
fn chinese_tags_resolve_by_region_and_script() {
    if !needs_catalogs() {
        return;
    }
    let zh = lang("zh-hant");
    // Traditional Chinese: by region, by script, and with a region after the script.
    for tag in ["zh_TW.UTF-8", "zh-TW", "zh-HK", "zh_MO", "zh-Hant", "zh-Hant-TW", "zh-Hant-HK"] {
        assert_eq!(lang_from_tag(tag), Some(zh), "{tag}");
    }
    // Simplified Chinese locales never pick up the Traditional catalog: they resolve to `zh-hans`.
    for tag in ["zh-CN", "zh_CN.UTF-8", "zh_SG", "zh-Hans", "zh-Hans-CN", "zh"] {
        assert_eq!(lang_from_tag(tag), Some(lang("zh-hans")), "{tag}");
    }
}

#[test]
fn candidates_walk_from_specific_to_general() {
    assert_eq!(candidates("pt_BR.UTF-8"), ["pt-br", "pt"]);
    assert_eq!(candidates("zh_TW"), ["zh-tw", "zh-hant", "zh"]);
    assert_eq!(candidates("zh-CN"), ["zh-cn", "zh-hans", "zh"]);
    assert_eq!(candidates("zh-Hant-HK"), ["zh-hant-hk", "zh-hant", "zh"]);
    assert_eq!(candidates("nn_NO.UTF-8"), ["nn-no", "nb", "nn"]);
    assert_eq!(candidates("no"), ["nb", "no"]);
    assert_eq!(candidates("pt_PT"), ["pt-pt", "pt-br", "pt"]);
}

/// PhotoCraft's Serbian catalog was `sr` and is craft-i18n's `sr-latn`: plain, regional and
/// Cyrillic Serbian tags still reach it, ahead of the bare `sr`, and the Latin tags keep their own
/// order. Holds without embedded catalogs (the candidates are PhotoCraft's own).
#[test]
fn serbian_tags_reach_the_latin_catalog() {
    assert_eq!(candidates("sr"), ["sr-latn", "sr"]);
    assert_eq!(candidates("sr-RS"), ["sr-rs", "sr-latn", "sr"]);
    assert_eq!(candidates("sr_RS@latin"), ["sr-rs", "sr-latn", "sr"]);
    assert_eq!(candidates("sr-Cyrl-RS"), ["sr-cyrl-rs", "sr-cyrl", "sr-latn", "sr"]);
    assert_eq!(candidates("sr-ME"), ["sr-me", "sr-latn", "sr"]);
    assert_eq!(candidates("sr-Latn-RS"), ["sr-latn-rs", "sr-latn", "sr"]);
}

#[test]
fn preferences_resolve_with_fallback() {
    assert_eq!(Lang::from_pref("en"), Lang::EN);
    // `auto` and unknown codes follow the system (English under test).
    assert_eq!(Lang::from_pref("auto"), Lang::EN);
    assert_eq!(Lang::from_pref("xx-unknown"), Lang::EN);
    if !needs_catalogs() {
        return;
    }
    assert_eq!(Lang::from_pref("ja"), lang("ja"));
    assert_eq!(Lang::from_pref("JA"), lang("ja"));
    assert_eq!(Lang::from_pref("zh-hant"), lang("zh-hant"));
    assert_eq!(Lang::from_pref("ZH-Hant"), lang("zh-hant"));
    assert_eq!(Lang::from_pref("ZH-Hans"), lang("zh-hans"));
}

/// Without embedded catalogs the interface is English: only English is offered, every lookup is
/// the English text, and every preference resolves to it.
#[test]
fn a_build_without_catalogs_is_english_only() {
    if catalogs_embedded() {
        return;
    }
    assert_eq!(Lang::all().collect::<Vec<_>>(), [Lang::EN]);
    assert_eq!(Lang::from_code("ja"), None);
    assert_eq!(Lang::from_pref("ja"), Lang::EN);
    assert_eq!(tr(Lang::EN, "Layer"), "Layer");
    assert_eq!(Lang::EN.name(), "English");
}

#[test]
fn spanish_swedish_finnish_danish_and_norwegian_resolve() {
    if !needs_catalogs() {
        return;
    }
    let es = lang("es");
    for tag in ["es", "es_ES.UTF-8", "es-MX", "es-419"] {
        assert_eq!(lang_from_tag(tag), Some(es), "{tag}");
    }
    let sv = lang("sv");
    assert_eq!(sv.name(), "Svenska");
    for tag in ["sv", "SV", "sv-SE", "sv_SE.UTF-8", "sv-FI", "sv_FI.UTF-8", "sv-AX"] {
        assert_eq!(Lang::from_pref(tag), sv, "{tag}");
    }
    assert_eq!(tr(sv, "File"), "Arkiv");
    assert_eq!(trn(sv, 1, "{n} layer", "{n} layers"), "1 lager");
    assert_eq!(trn(sv, 3, "{n} layer", "{n} layers"), "3 lager");
    let fi = lang("fi");
    assert_eq!(fi.name(), "Suomi");
    for tag in ["fi", "FI", "fi-FI", "fi_FI.UTF-8"] {
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
    let da = lang("da");
    assert_eq!(da.name(), "Dansk");
    for tag in ["da", "DA", "da-DK", "da_DK.UTF-8", "da-GL"] {
        assert_eq!(Lang::from_pref(tag), da, "{tag}");
    }
    assert_eq!(trn(da, 1, "{n} document", "{n} documents"), "1 dokument");
    assert_eq!(trn(da, 3, "{n} document", "{n} documents"), "3 dokumenter");
    let nb = lang("nb");
    assert_eq!(nb.name(), "Norsk bokmål");
    for tag in ["nb", "NB", "nb-NO", "nb_NO.UTF-8", "no", "no-NO", "no_NO.UTF-8", "nn", "nn-NO", "nn_NO.UTF-8"] {
        assert_eq!(Lang::from_pref(tag), nb, "{tag}");
    }
    assert_eq!(trn(nb, 3, "{n} item", "{n} items"), "3 elementer");
}

#[test]
fn hungarian_and_portuguese_resolve() {
    if !needs_catalogs() {
        return;
    }
    let hu = lang("hu");
    for tag in ["hu", "hu_HU.UTF-8", "hu-HU"] {
        assert_eq!(lang_from_tag(tag), Some(hu), "{tag}");
    }
    assert_eq!(hu.name(), "Magyar");
    let pt = lang("pt-br");
    for tag in ["pt", "pt-BR", "pt_BR.UTF-8", "pt-PT", "pt_PT.UTF-8"] {
        assert_eq!(lang_from_tag(tag), Some(pt), "{tag}");
    }
}

#[test]
fn lookups_fall_back_to_english() {
    assert_eq!(tr(Lang::EN, "Layer"), "Layer");
    if !needs_catalogs() {
        return;
    }
    let (ja, zh) = (lang("ja"), lang("zh-hant"));
    assert_eq!(tr(ja, "no such label"), "no such label");
    assert_eq!(tr(zh, "no such label"), "no such label");
    assert_eq!(tr_id(zh, "no.such.id", "Layer"), tr(zh, "Layer"));
    assert_eq!(tr_id(ja, "no.such.id", "Layer"), tr(ja, "Layer"));
    assert_eq!(tr_ctx(ja, "no such context", "Layer"), tr(ja, "Layer"));
    assert_eq!(trn(ja, 3, "{n} no such message", "{n} no such messages"), "3 no such messages");
}

#[test]
fn ukrainian_and_arabic_resolve() {
    if !needs_catalogs() {
        return;
    }
    let uk = lang("uk");
    assert_eq!(uk.name(), "Українська");
    for tag in ["uk", "UK", "uk-UA", "uk_UA", "uk_UA.UTF-8", "uk-UA@euro", "uk-Cyrl-UA"] {
        assert_eq!(Lang::from_pref(tag), uk, "{tag}");
    }
    let ar = lang("ar");
    assert_eq!(ar.name(), "العربية");
    for tag in ["ar", "AR", "ar-SA", "ar_EG", "ar_EG.UTF-8", "ar-AE", "ar-MA"] {
        assert_eq!(lang_from_tag(tag), Some(ar), "{tag}");
        assert_eq!(Lang::from_pref(tag), ar, "{tag}");
    }
    assert_eq!(dir_for_code("ar"), Dir::Rtl);
    assert_eq!(ar.dir(), Dir::Rtl);
}

/// Every Serbian tag PhotoCraft matched to its `sr` catalog reaches `sr-latn`, as a system tag and
/// as a preference (a saved `sr` included).
#[test]
fn serbian_resolves_locales_and_preferences() {
    if !needs_catalogs() {
        return;
    }
    let sr = lang("sr-latn");
    for tag in ["sr", "SR", "sr-RS", "sr_RS", "sr_RS.UTF-8", "sr_RS@latin", "sr-Latn", "sr-Latn-RS", "sr-Cyrl-RS", "sr-ME"] {
        assert_eq!(lang_from_tag(tag), Some(sr), "{tag}");
        assert_eq!(Lang::from_pref(tag), sr, "{tag}");
    }
}

/// The catalog resolves regional tags, exposes its native name and falls back to English.
#[test]
fn azerbaijani_resolves_locales_and_preferences() {
    if !needs_catalogs() {
        return;
    }
    let az = lang("az");
    assert_eq!(az.name(), "Azərbaycanca");
    assert_eq!(az.dir(), Dir::Ltr);
    assert!(az.complete_menus());
    for tag in ["az", "AZ", "az-AZ", "az_AZ.UTF-8", "az-Latn", "AZ-LATN-AZ"] {
        assert_eq!(lang_from_tag(tag), Some(az), "{tag}");
        assert_eq!(Lang::from_pref(tag), az, "{tag}");
    }
    assert_eq!(tr(az, "unknown translation"), "unknown translation");
    for n in [0, 1, 2, 11, 19, 20, 21, 99, 100, 101, 102, 119, 120, 1000, 1001, u64::MAX] {
        assert_eq!(trn(az, n, "{n} layer", "{n} layers"), format!("{n} qat"));
    }
}

/// Exact lookups retain contextual meanings, dotted/dotless letters and product placeholders.
#[test]
fn azerbaijani_keeps_context_placeholders_and_command_names() {
    if !needs_catalogs() {
        return;
    }
    let az = lang("az");
    for (source, translated) in [("File", "Fayl"), ("Layer", "Qat"), ("Kind", "Növ"), ("Type Layer", "Mətn qatı"), ("Light Theme", "Açıq mövzu")] {
        assert_eq!(tr(az, source), translated);
    }
    assert_eq!(tr_ctx(az, "cameraRaw", "Light"), "İşıq");
    assert_eq!(tr_ctx(az, "fontWeight", "Light"), "İncə");
    assert_ne!(tr_id(az, "edit.clear", "Clear"), "Clear");
    assert!(fmt(tr(az, "About {app}"), &[("app", "PhotoCraft")]).contains("PhotoCraft"));
}

/// Bosnian locales, with or without region, script or encoding, select the Latin catalog.
#[test]
fn bosnian_resolves_locales_and_preferences() {
    if !needs_catalogs() {
        return;
    }
    let bs = lang("bs");
    assert_eq!(bs.name(), "Bosanski");
    for tag in ["bs", "BS", "bs-BA", "bs_BA", "bs_BA.UTF-8", "bs-Latn", "bs-Latn-BA", "bs-Cyrl-BA", "bs_BA@cyrillic"] {
        assert_eq!(lang_from_tag(tag), Some(bs), "{tag}");
        assert_eq!(Lang::from_pref(tag), bs, "{tag}");
    }
    assert_ne!(lang_from_tag("hr-HR"), Some(bs), "Croatian is not Bosnian");
    assert_eq!(trn(bs, 3, "{n} pixel", "{n} pixels"), "3 piksela");
    assert_eq!(trn(bs, 5, "{n} document", "{n} documents"), "5 dokumenata");
}

#[test]
fn french_resolves() {
    if !needs_catalogs() {
        return;
    }
    let fr = lang("fr");
    for tag in ["fr", "fr_FR.UTF-8", "fr-CA", "fr_BE", "fr-CH"] {
        assert_eq!(lang_from_tag(tag), Some(fr), "{tag}");
    }
}

/// Counts pick the form PhotoCraft's own plural rules picked before its catalogs moved to
/// craft-i18n, now from CLDR (the crate): per language, the old rule's form index per count.
/// Exceptions, from the spec's parity list or CLDR: Hungarian, Turkish and Georgian have two forms
/// (their catalogs repeat the one form), Hebrew's `two` form repeats `other`
/// ([`hebrew_counts_read_one_or_other`]), and Hindi's 0 takes the `one` form.
#[test]
fn counts_pick_the_same_plural_form_as_before() {
    const MAX: u64 = u64::MAX;
    let cases: &[(&'static str, usize, &[u64], &[usize])] = &[
        ("ar", 6, &[0, 1, 2, 3, 10, 11, 99, 100, 101, 102, 103, 111, 199, 1000, MAX], &[0, 1, 2, 3, 3, 4, 4, 5, 5, 5, 3, 4, 4, 5, 4]),
        ("ro", 3, &[0, 1, 2, 19, 20, 21, 100, 101, 119, 120, 121, 1001], &[1, 0, 1, 1, 2, 2, 2, 1, 1, 2, 2, 1]),
        ("ru", 3, &[0, 1, 2, 5, 11, 12, 14, 21, 22, 25, 101, 111, 112], &[2, 0, 1, 2, 2, 2, 2, 0, 1, 2, 0, 2, 2]),
        (
            "uk",
            3,
            &[0, 1, 2, 4, 5, 11, 12, 14, 19, 21, 22, 24, 25, 100, 101, 111, 112, 114, 121, 122, MAX],
            &[2, 0, 1, 1, 2, 2, 2, 2, 2, 0, 1, 1, 2, 2, 0, 2, 2, 2, 0, 1, 2],
        ),
        ("sr-latn", 3, &[0, 1, 2, 4, 5, 11, 12, 14, 21, 22, 25, 101, 111, 112, 122], &[2, 0, 1, 1, 2, 2, 2, 2, 0, 1, 2, 0, 2, 2, 1]),
        ("bs", 3, &[0, 1, 2, 4, 5, 11, 12, 14, 21, 22, 25, 101, 111, 112, 122], &[2, 0, 1, 1, 2, 2, 2, 2, 0, 1, 2, 0, 2, 2, 1]),
        ("pl", 3, &[0, 1, 2, 4, 5, 12, 14, 21, 22, 25, 112, 122], &[2, 0, 1, 1, 2, 2, 2, 2, 1, 2, 2, 1]),
        ("cs", 3, &[0, 1, 2, 3, 4, 5, 11, 12, 21, 22, 100, MAX], &[2, 0, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2]),
        ("fr", 2, &[0, 1, 2, 5, 100, 1_000_000, MAX], &[0, 0, 1, 1, 1, 1, 1]),
        ("pt-br", 2, &[0, 1, 2, 1_000_000], &[0, 0, 1, 1]),
        ("es", 2, &[0, 1, 3, 1_000_000], &[1, 0, 1, 1]),
        ("it", 2, &[0, 1, 3, 1_000_000], &[1, 0, 1, 1]),
        ("hi", 2, &[0, 1, 2, 5, 21], &[0, 0, 1, 1, 1]),
        ("ja", 1, &[0, 1, 5], &[0, 0, 0]),
        ("th", 1, &[0, 1, 5], &[0, 0, 0]),
    ];
    let one_other = ["sv", "fi", "da", "nb", "de", "nl", "el", "az", "ta", "ur"];
    let one_other_cases = one_other.map(|code| (code, 2, &[0, 1, 2, 21, 1_000_000][..], &[1, 0, 1, 1, 1][..]));
    for &(code, forms, counts, expected) in cases.iter().chain(one_other_cases.iter()) {
        let row: String = (0..forms).map(|i| format!("{{n}} f{i}")).collect::<Vec<_>>().join("|");
        let lang = fixture_lang(code, Box::leak(format!("@plural\t{{n}} x|{{n}} xs\t{row}\n").into_boxed_str()));
        for (&n, &index) in counts.iter().zip(expected) {
            assert_eq!(trn(lang, n, "{n} x", "{n} xs"), format!("{n} f{index}"), "{code}, {n}");
        }
    }
}

/// Hebrew counts read as PhotoCraft's one|other rule did: craft-i18n's Hebrew rows list
/// one|two|other with `two` repeating `other`, so 2 reads like 3.
#[test]
fn hebrew_counts_read_one_or_other() {
    let he = fixture_lang("he", "@plural\t{n} layer|{n} layers\tשכבה {n}|{n} שכבות|{n} שכבות\n");
    for n in [0, 2, 3, 10, 11, 20, 100, 1000, u64::MAX] {
        assert_eq!(trn(he, n, "{n} layer", "{n} layers"), format!("{n} שכבות"), "{n}");
    }
    assert_eq!(trn(he, 1, "{n} layer", "{n} layers"), "שכבה 1");
}

#[test]
fn plurals_and_placeholders() {
    assert_eq!(trn(Lang::EN, 1, "{n} item", "{n} items"), "1 item");
    assert_eq!(trn(Lang::EN, 0, "{n} item", "{n} items"), "0 items");
    assert_eq!(trn(Lang::EN, 7, "{n} item", "{n} items"), "7 items");
    assert_eq!(trn(Lang::EN, 1234, "{n} item", "{n} items"), "1234 items", "no grouping");
    let ja = fixture_lang("ja", "@plural\t{n} item|{n} items\t{n} only\n");
    assert_eq!(trn(ja, 1, "{n} item", "{n} items"), "1 only");
    assert_eq!(trn(ja, 7, "{n} item", "{n} items"), "7 only");
    assert_eq!(trn(ja, 7, "{n} file", "{n} files"), "7 files", "a missing row falls back to English");
    assert_eq!(fmt("{b} before {a}", &[("a", "x"), ("b", "y"), ("c", "z")]), "y before x");
    assert_eq!(fmt("{missing}", &[]), "{missing}");
}

#[test]
fn id_and_context_rows_win_over_the_plain_label() {
    let lang = fixture_lang("de", "\tAll\tplain all\n@id\tselect.all\tid all\nnavigator\tZoom\tcontext zoom\n\tZoom\tplain zoom\n");
    assert_eq!(tr_id(lang, "select.all", "All"), "id all");
    assert_eq!(tr_id(lang, "edit.other", "All"), "plain all");
    assert_eq!(tr(lang, "All"), "plain all");
    assert_eq!(tr_ctx(lang, "navigator", "Zoom"), "context zoom");
    assert_eq!(tr_ctx(lang, "other", "Zoom"), "plain zoom");
    assert_eq!(tr_ctx(lang, "navigator", "Unlisted"), "Unlisted");
}

/// An `@id` row that drops its label's `{placeholders}` or trailing `…` is skipped: the label's
/// own translation shows, so a menu item never loses the "opens a dialog" mark or a value.
#[test]
fn id_rows_keep_the_shape_of_their_label() {
    let lang = fixture_lang("de", "\tStroke…\tKontur…\n@id\tedit.stroke\tKontur\n@id\tlayer.rename\t{name} umbenennen\n");
    assert_eq!(tr_id(lang, "edit.stroke", "Stroke…"), "Kontur…", "the row without … is skipped");
    assert_eq!(tr_id(lang, "layer.rename", "Rename {name}"), "{name} umbenennen");
    assert_eq!(tr_id(lang, "layer.rename", "Rename {layer}"), "Rename {layer}", "a different placeholder");
    assert!(same_shape("{b} und {a}", "{a} and {b}"), "placeholders may be reordered");
    assert!(!same_shape("Öffnen {name…", "Open…"), "a placeholder left open");
    assert!(!same_shape("Öffnen name}…", "Open…"), "a closing brace without its opening one");
    assert!(!same_shape("{name}} öffnen", "Open {name}"), "a second closing brace");
}

/// The language list is well-formed, and with the catalogs embedded each registered language has
/// one. Problems in the embedded catalogs' rows are not failed: craft-i18n
/// checks its rows, a row PhotoCraft can't use shows in English, and `cargo xtask i18n-report`
/// lists what each language lacks.
#[test]
fn bundled_catalogs_are_consistent() {
    let mut codes = std::collections::HashSet::new();
    for l in &LANGUAGES {
        assert!(codes.insert(l.code), "duplicate code {}", l.code);
        assert!(l.code == l.code.to_ascii_lowercase() && !l.code.is_empty(), "{}", l.code);
    }
    assert_eq!(LANGUAGES.first().map(|l| l.code), Some("en"), "English first: the source and the fallback");
    if !needs_catalogs() {
        return;
    }
    // A build that requires the catalogs (`CRAFT_I18N_REQUIRED=1`) must offer every registered
    // language, not only whichever locales the checkout happens to hold.
    for l in LANGUAGES.iter().filter(|l| l.code != "en") {
        assert!(!app_catalog(l.code).is_empty(), "{}: no photocraft.tsv at craft-i18n.rev", l.code);
        assert!(Lang::from_code(l.code).is_some(), "{}: registered but not offered", l.code);
    }
    let problems = I18N.check_all();
    if !problems.is_empty() {
        println!("{} catalog rows are skipped (they show in English):\n{problems:#?}", problems.len());
    }
    for (code, _, app) in CRAFT_I18N {
        for line in app.lines().filter(|l| l.starts_with("@id\t")) {
            let id = line.split('\t').nth(1).unwrap_or_default();
            if id != "language.name" && !(crate::menus::is_live(id) || crate::menu_catalog::CATALOG.iter().any(|m| m.3 == id)) {
                println!("{code}: @id row for a command PhotoCraft doesn't have: {id:?}");
            }
        }
    }
}

#[test]
fn raw_develop_labels_are_translated_in_every_shipped_catalog() {
    if !needs_catalogs() {
        return;
    }
    for language in LANGUAGES.iter().skip(1) {
        let l = lang(language.code);
        for source in ["RAW Develop", "RAW Develop Filter", "RAW Develop Filter…", "RAW Defaults", "RAW Defaults…"] {
            assert!(has(l, source) && !tr(l, source).is_empty(), "{}: {source}", language.code);
        }
    }
}
