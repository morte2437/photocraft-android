//! Case conversion of translated UI text; command IDs and English aliases use `Lang::EN`.

use icu_casemap::CaseMapper;
use icu_locale_core::{LanguageIdentifier, langid};

use super::Lang;

impl Lang {
    /// Locales that tailor Unicode's dotted/dotless I mappings in this UI.
    fn case_locale(self) -> Option<LanguageIdentifier> {
        match self.code() {
            "az" => Some(langid!("az")),
            "tr" => Some(langid!("tr")),
            _ => None,
        }
    }

    /// Lowercase UI text in its language, including Turkic I and combining-dot contexts.
    pub fn lowercase(self, text: &str) -> String {
        match self.case_locale() {
            Some(locale) => CaseMapper::new().lowercase_to_string(text, &locale).into_owned(),
            None => text.to_lowercase(),
        }
    }

    /// Uppercase UI text in its language (`i` → `İ`, `ı` → `I` in Azerbaijani and Turkish).
    pub fn uppercase(self, text: &str) -> String {
        match self.case_locale() {
            Some(locale) => CaseMapper::new().uppercase_to_string(text, &locale).into_owned(),
            None => text.to_uppercase(),
        }
    }

    /// Match visible text without case distinctions in this language.
    pub fn contains_ignore_case(self, text: &str, query: &str) -> bool {
        self.lowercase(text).contains(&self.lowercase(query))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unicode Turkic mappings keep the two I pairs distinct, including decomposed input.
    #[test]
    fn azerbaijani_and_turkish_case_mapping_handles_combining_dots() {
        if !crate::i18n::needs_catalogs() {
            return;
        }
        for code in ["az", "tr"] {
            let lang = Lang::from_code(code).unwrap();
            assert_eq!(lang.lowercase("Iİıi"), "ıiıi");
            assert_eq!(lang.uppercase("Iİıi"), "IİIİ");
            assert_eq!(lang.lowercase("I\u{307} I\u{323}\u{307} I\u{301}"), "i i\u{323} ı\u{301}");
            assert!(lang.contains_ignore_case("İşıq", "İŞIQ"));
            assert!(!lang.contains_ignore_case("İşıq", "IŞIQ"));
            assert!(!lang.contains_ignore_case("ı", "i"));
        }
        assert_eq!(Lang::EN.lowercase("Iİıi"), "ii\u{307}ıi");
        assert_eq!(Lang::EN.uppercase("iı"), "II");
        assert_eq!(Lang::EN.lowercase("ΟΣ"), "ος", "keep contextual final sigma");
    }
}
