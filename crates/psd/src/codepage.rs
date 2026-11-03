//! Guessing the legacy code page of 8-bit names.
//!
//! Some formats store names as bytes in the writer's system ANSI code page: an `.atn` file's
//! event and dictionary names are CP936 (GBK) from Chinese Photoshop, CP932 (Shift_JIS) from
//! Japanese, Mac Roman from French Photoshop on a Mac, and so on. Nothing in the file says
//! which. When the file also stores some of those names as Unicode, [`match_known`] finds the
//! code page that turns the bytes into them. Otherwise [`guess`] infers one code page per file
//! from the names that are not valid UTF-8:
//!
//! 1. Candidates: Shift_JIS, GBK, Big5, EUC-KR, windows-1252, windows-1251, macintosh (Mac
//!    Roman). A candidate that cannot decode every name without errors is dropped.
//! 2. Each survivor is scored by the share of non-ASCII bytes that fall in its common-text
//!    ranges (GB2312 hanzi, JIS level-1 kanji and kana, Big5 level 1, KS X 1001 Hangul,
//!    letters, and each page's punctuation). The highest share wins.
//! 3. A tie is broken by the script of a hint (text the file stores as UTF-16, so it decodes
//!    fine): kana → Shift_JIS; Hangul → EUC-KR; Cyrillic → windows-1251; Han → GBK, then
//!    Big5, then Shift_JIS; Latin letters → windows-1252, then Mac Roman.
//! 4. Still tied: windows-1252, flagged [`Guess::uncertain`].
//! 5. Mac Roman must also beat windows-1252 when the Latin-1 symbols that are no letter in
//!    Mac Roman (© ° « ± …) count as common text (1252 "Größe »" is Mac Roman "Grˆﬂe ª").
//!    Otherwise the names are windows-1252, with step 3's hint and step 4's warning.
//!
//! Only the first [`GUESS_SAMPLE`] bytes of names are examined.

use encoding_rs::{
    BIG5, DecoderResult, EUC_KR, Encoding, GBK, ISO_2022_JP, MACINTOSH, REPLACEMENT, SHIFT_JIS, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1251, WINDOWS_1252,
    X_USER_DEFINED,
};

use crate::error::{PsdError, Result};

/// At most this many bytes of legacy names are examined by [`guess`].
pub(crate) const GUESS_SAMPLE: usize = 64 * 1024;
/// At most this many `(bytes, text)` pairs are examined by [`match_known`].
pub(crate) const KNOWN_PAIRS: usize = 256;

/// The code page [`guess`] settled on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Guess {
    /// The encoding to decode every legacy name with.
    pub(crate) encoding: &'static Encoding,
    /// Nothing told the candidates apart; `encoding` is the windows-1252 fallback.
    pub(crate) uncertain: bool,
}

/// Candidates in tie order (encoding_rs exposes encodings as statics, hence a function).
fn candidates() -> [&'static Encoding; 7] {
    [SHIFT_JIS, GBK, BIG5, EUC_KR, WINDOWS_1252, WINDOWS_1251, MACINTOSH]
}

/// The candidate that decodes the most of `pairs` (a name's bytes and the same name as Unicode)
/// exactly; ties go to the earlier candidate. `None` when it decodes none of them. Only the first
/// [`KNOWN_PAIRS`] pairs are examined.
pub(crate) fn match_known(pairs: &[(&[u8], &str)]) -> Option<&'static Encoding> {
    let pairs = pairs.get(..KNOWN_PAIRS).unwrap_or(pairs);
    let mut best = None;
    let mut best_count = 0usize;
    for enc in candidates() {
        let count = pairs.iter().filter(|(bytes, text)| enc.decode_without_bom_handling_and_without_replacement(bytes).is_some_and(|d| d == *text)).count();
        if count > best_count {
            best = Some(enc);
            best_count = count;
        }
    }
    best
}

/// Guesses the one code page that `legacy` (names that are not valid UTF-8, in file order) were
/// written in. `hint` is text from the same file whose script points at the writer's locale.
pub(crate) fn guess(legacy: &[&[u8]], hint: &str) -> Guess {
    let sample = sample(legacy);
    if sample.is_empty() {
        return Guess { encoding: UTF_8, uncertain: false };
    }
    let mut best: Vec<&'static Encoding> = Vec::new();
    let mut best_score = (0u64, 1u64);
    for enc in candidates() {
        if !sample.iter().all(|(name, truncated)| decodes_cleanly(enc, name, *truncated)) {
            continue;
        }
        let score = sample.iter().fold((0u64, 0u64), |(c, h), (name, _)| {
            let (nc, nh) = score(enc, name);
            (c.saturating_add(nc), h.saturating_add(nh))
        });
        let score = if score.1 == 0 { (0, 1) } else { score };
        // Compare common/high shares without division: a/b vs c/d ⇔ a·d vs c·b.
        let lhs = u128::from(score.0) * u128::from(best_score.1);
        let rhs = u128::from(best_score.0) * u128::from(score.1);
        if best.is_empty() || lhs > rhs {
            best = vec![enc];
            best_score = score;
        } else if lhs == rhs {
            best.push(enc);
        }
    }
    let picked = match best.as_slice() {
        [only] => Guess { encoding: only, uncertain: false },
        tied => match hinted(hint).iter().find(|enc| tied.contains(enc)) {
            Some(enc) => Guess { encoding: enc, uncertain: false },
            None => Guess { encoding: WINDOWS_1252, uncertain: true },
        },
    };
    if picked.encoding != MACINTOSH {
        return picked;
    }
    // windows-1252 maps every byte, so it is always a survivor here.
    let latin = |enc| {
        sample.iter().fold((0u64, 0u64), |(c, h), (name, _)| {
            let (nc, nh) = single_byte_score(enc, name, is_latin_common);
            (c.saturating_add(nc), h.saturating_add(nh))
        })
    };
    let (windows, mac) = (latin(WINDOWS_1252), latin(MACINTOSH));
    let lhs = u128::from(windows.0) * u128::from(mac.1.max(1));
    let rhs = u128::from(mac.0) * u128::from(windows.1.max(1));
    if lhs < rhs {
        picked
    } else if lhs > rhs || hinted(hint).contains(&WINDOWS_1252) {
        Guess { encoding: WINDOWS_1252, uncertain: false }
    } else {
        Guess { encoding: WINDOWS_1252, uncertain: true }
    }
}

/// Decodes `bytes` with `enc`; invalid sequences become U+FFFD.
pub(crate) fn decode(bytes: &[u8], enc: &'static Encoding) -> String {
    enc.decode_without_bom_handling(bytes).0.into_owned()
}

/// Whether `name` decodes cleanly in the double-byte page `enc` and every non-ASCII byte of it is
/// common text there. A name in such a page can be valid UTF-8 by accident (GBK 去色 is
/// `C8 A5 C9 AB`, "ȥɫ"); real UTF-8 rarely passes, as its continuation bytes fall outside the
/// common ranges. Single-byte pages are never asked: their letters cover UTF-8's lead and
/// continuation bytes ("Weiß" is "WeiÃŸ" in windows-1252), and a legacy name there is valid UTF-8
/// by accident only for unlikely pairs such as "Ã©".
pub(crate) fn reads_as_common(name: &[u8], enc: &'static Encoding) -> bool {
    if ![SHIFT_JIS, GBK, BIG5, EUC_KR].contains(&enc) {
        return false;
    }
    let (common, high) = score(enc, name);
    high > 0 && common == high && decodes_cleanly(enc, name, false)
}

/// Resolves an encoding label (`shift_jis`, `gbk`, `big5`, `euc-kr`, `windows-1252`, `utf-8`…;
/// case and surrounding whitespace are ignored). Labels for encodings that cannot hold an 8-bit
/// name are rejected, and so is ISO-2022-JP: its names are 7-bit escape sequences, which would
/// pass through as ASCII.
pub(crate) fn resolve_label(label: &str) -> Result<&'static Encoding> {
    match Encoding::for_label(label.as_bytes()) {
        Some(enc) if ![REPLACEMENT, UTF_16LE, UTF_16BE, X_USER_DEFINED, ISO_2022_JP].contains(&enc) => Ok(enc),
        _ => Err(PsdError::Unsupported(format!("text encoding `{label}`: use a label such as shift_jis, gbk, big5, euc-kr, windows-1252 or utf-8"))),
    }
}

/// The names [`guess`] examines, each with whether it was cut short: names in order while they
/// fit in [`GUESS_SAMPLE`] bytes, and at least the first (truncated if it alone is longer).
fn sample<'a>(legacy: &[&'a [u8]]) -> Vec<(&'a [u8], bool)> {
    let mut out = Vec::new();
    let mut total = 0usize;
    for name in legacy {
        let next = total.saturating_add(name.len());
        if next > GUESS_SAMPLE {
            if out.is_empty() {
                out.push((name.get(..GUESS_SAMPLE).unwrap_or(name), true));
            }
            break;
        }
        out.push((*name, false));
        total = next;
    }
    out
}

/// Whether `enc` decodes `bytes` without a single error. A `truncated` name may end inside a
/// character, which is not held against it.
fn decodes_cleanly(enc: &'static Encoding, bytes: &[u8], truncated: bool) -> bool {
    let mut decoder = enc.new_decoder_without_bom_handling();
    // Every candidate decodes one byte to at most three UTF-8 bytes, so this is linear in
    // `bytes` (a sampled name, or one name the file already holds).
    let mut out = String::with_capacity(bytes.len().saturating_mul(3).saturating_add(16));
    let mut rest = bytes;
    loop {
        let (result, read) = decoder.decode_to_string_without_replacement(rest, &mut out, !truncated);
        match result {
            DecoderResult::InputEmpty => return true,
            DecoderResult::Malformed(..) => return false,
            DecoderResult::OutputFull => {
                rest = rest.get(read..).unwrap_or_default();
                out.clear();
            }
        }
    }
}

/// `(common, high)`: how many of a name's non-ASCII bytes `enc` reads as common text, and how
/// many non-ASCII bytes it has.
fn score(enc: &'static Encoding, name: &[u8]) -> (u64, u64) {
    if enc == WINDOWS_1252 || enc == WINDOWS_1251 || enc == MACINTOSH {
        // Letters and typographic punctuation are common, as the double-byte pages' punctuation
        // rows are.
        return single_byte_score(enc, name, |ch| ch.is_alphabetic() || TYPOGRAPHIC.contains(&ch));
    }
    let (mut common, mut high) = (0u64, 0u64);
    let mut i = 0usize;
    while let Some(&lead) = name.get(i) {
        if lead < 0x80 {
            i += 1;
            continue;
        }
        // Shift_JIS reads 0x80 and 0xA0–0xDF (half-width katakana, rare in real text) as one
        // byte; elsewhere a lone non-ASCII byte is an error the decoder already rejected.
        let single = if enc == SHIFT_JIS { lead == 0x80 || (0xA0..=0xDF).contains(&lead) || lead >= 0xFD } else { lead == 0x80 || lead == 0xFF };
        if single {
            high += 1;
            i += 1;
            continue;
        }
        // A lead byte with nothing after it is where the sample was cut (the decoder rejects one
        // at the real end of a name), so it is not scored.
        let Some(&trail) = name.get(i + 1) else { break };
        // GB18030 four-byte sequences (second byte a digit) are never common text.
        let len = if enc == GBK && trail.is_ascii_digit() { 4 } else { 2 };
        let len = len.min(name.len().saturating_sub(i));
        high += len as u64;
        if len == 2 && is_common(enc, lead, trail) {
            common += 2;
        }
        i += len;
    }
    (common, high)
}

/// [`score`] for a single-byte page, where one byte is one character and a non-ASCII byte is a
/// non-ASCII character: `common` decides which of them count.
///
/// Known limitation: windows-1251 has Cyrillic letters (І і Ј д) where windows-1252 has ² ³ £ ä,
/// so a windows-1252 name like "Fläche m²" or "Preis £5" scores higher as windows-1251 ("Flдche
/// mІ", "Preis Ј5"). Discounting a Cyrillic letter glued to a Latin word would fix those but
/// break ordinary Russian names such as "CMYKканал" or Ukrainian "Копiя" (Latin i), which turn a
/// whole set into windows-1252.
fn single_byte_score(enc: &'static Encoding, name: &[u8], common: impl Fn(char) -> bool) -> (u64, u64) {
    decode(name, enc).chars().filter(|c| !c.is_ascii()).fold((0, 0), |(c, h), ch| (c.saturating_add(u64::from(common(ch))), h.saturating_add(1)))
}

/// Whether `ch` is common in Western names, for weighing windows-1252 against Mac Roman: letters
/// (but not those windows-1252 has only at 0x80–0x9F, where Mac Roman keeps its accented
/// lowercase: Mac "Größe" is 1252 "Grš§e"), typographic punctuation and [`LATIN_SYMBOLS`].
fn is_latin_common(ch: char) -> bool {
    (ch.is_alphabetic() && !['Š', 'š', 'Ž', 'ž', 'Œ', 'œ', 'Ÿ', 'ƒ', 'ˆ'].contains(&ch)) || TYPOGRAPHIC.contains(&ch) || LATIN_SYMBOLS.contains(&ch)
}

/// Latin-1 symbols people type in names (windows-1252 0xA1–0xB7) whose byte is not a letter in
/// Mac Roman. Left out because Mac Roman has a letter there: § ® ¹ » ¼ ½ ¾ ¿ (ß Æ π ª º Ω æ ø,
/// as in Mac "Weiß", "Størrelse", "Æble"), € and ™ (0x80, 0x99: Ä, ô), and ¥ (0xA5, Mac Roman's
/// bullet in [`TYPOGRAPHIC`]: Mac "Ebene • Kopie"). So a windows-1252 name whose only non-ASCII
/// character is one of those ("Logo ®", "¿Tamaño?", "Preis ¥100") still reads as Mac Roman
/// ("Logo Æ").
const LATIN_SYMBOLS: [char; 11] = ['¡', '¢', '£', '©', '«', '°', '±', '²', '³', '¶', '·'];

/// Quotes, dashes, ellipsis and bullet: windows-1252 and windows-1251 share them at 0x82–0x97;
/// Mac Roman has them too.
const TYPOGRAPHIC: [char; 10] = ['‘', '’', '‚', '“', '”', '„', '–', '—', '…', '•'];

/// Whether the two-byte character `lead trail` is in `enc`'s common-text ranges.
fn is_common(enc: &'static Encoding, lead: u8, trail: u8) -> bool {
    let both = u16::from_be_bytes([lead, trail]);
    if enc == GBK {
        // GB2312 hanzi, and the punctuation rows 0xA1 and 0xA3.
        (0xA1..=0xFE).contains(&trail) && ((0xB0..=0xF7).contains(&lead) || lead == 0xA1 || lead == 0xA3)
    } else if enc == BIG5 {
        // Level-1 hanzi and punctuation.
        (0xA4..=0xC6).contains(&lead) || lead == 0xA1
    } else if enc == SHIFT_JIS {
        // Kana, level-1 kanji and punctuation.
        (0x829F..=0x8396).contains(&both) || (0x889F..=0x9872).contains(&both) || (0x8140..=0x81FC).contains(&both)
    } else if enc == EUC_KR {
        // KS X 1001 Hangul and punctuation.
        trail >= 0xA1 && ((0xB0..=0xC8).contains(&lead) || lead == 0xA1)
    } else {
        false
    }
}

/// The candidates the script of `hint` points at, strongest first.
fn hinted(hint: &str) -> Vec<&'static Encoding> {
    let has = |lo: char, hi: char| hint.chars().any(|c| (lo..=hi).contains(&c) && c.is_alphabetic());
    if has('\u{3040}', '\u{30FF}') {
        vec![SHIFT_JIS]
    } else if has('\u{AC00}', '\u{D7A3}') {
        vec![EUC_KR]
    } else if has('\u{0400}', '\u{04FF}') {
        vec![WINDOWS_1251]
    } else if has('\u{4E00}', '\u{9FFF}') {
        vec![GBK, BIG5, SHIFT_JIS]
    } else if has('\u{00C0}', '\u{024F}') {
        vec![WINDOWS_1252, MACINTOSH]
    } else {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GBK_ISSUE: &[&[u8]] = &[b"\xbf\xc9\xd1\xa1\xd1\xd5\xc9\xab"]; // 可选颜色 (from #6743)
    const SJIS_COMMENT: &[&[u8]] = &[
        b"\x91\x49\x91\xf0\x20\x3a", // 選択 :
        b"\x90\xdd\x92\xe8\x20\x3a", // 設定 :
        b"\x83\x52\x83\x73\x81\x5b", // コピー
        b"\x95\xdb\x91\xb6",         // 保存
    ];

    fn encoded(enc: &'static Encoding, words: &[&str]) -> Vec<Vec<u8>> {
        words.iter().map(|w| enc.encode(w).0.into_owned()).collect()
    }

    fn guess_words(enc: &'static Encoding, words: &[&str], hint: &str) -> Guess {
        let bytes = encoded(enc, words);
        let refs: Vec<&[u8]> = bytes.iter().map(Vec::as_slice).collect();
        guess(&refs, hint)
    }

    #[test]
    fn issue_gbk_bytes_read_as_gbk() {
        let g = guess(GBK_ISSUE, "调白");
        assert_eq!(g.encoding, GBK);
        assert!(!g.uncertain);
        assert_eq!(decode(GBK_ISSUE[0], g.encoding), "可选颜色");
    }

    #[test]
    fn kanji_only_hint_still_reads_shift_jis() {
        let g = guess(SJIS_COMMENT, "保存");
        assert_eq!(g.encoding, SHIFT_JIS);
        assert_eq!(decode(SJIS_COMMENT[0], g.encoding), "選択 :");
    }

    #[test]
    fn big5_reads_as_big5() {
        assert_eq!(guess_words(BIG5, &["色彩範圍", "移動", "選取", "儲存"], "").encoding, BIG5);
    }

    #[test]
    fn hangul_hint_reads_euc_kr() {
        let g = guess_words(EUC_KR, &["선택", "이동", "저장", "색상 범위"], "동작 1");
        assert_eq!(g.encoding, EUC_KR);
        assert!(!g.uncertain);
    }

    #[test]
    fn cyrillic_hint_reads_1251() {
        let g = guess_words(WINDOWS_1251, &["Выделить", "Переместить", "Сохранить"], "Набор 1");
        assert_eq!(g.encoding, WINDOWS_1251);
        assert!(!g.uncertain);
    }

    #[test]
    fn latin_hint_reads_1252() {
        let g = guess_words(WINDOWS_1252, &["Größe"], "Größe ändern");
        assert_eq!(g.encoding, WINDOWS_1252);
        assert!(!g.uncertain);
    }

    #[test]
    fn tie_without_hint_is_1252_and_uncertain() {
        let g = guess_words(WINDOWS_1252, &["Größe"], "Set 1");
        assert_eq!(g.encoding, WINDOWS_1252);
        assert!(g.uncertain);
    }

    #[test]
    fn han_tie_prefers_gbk() {
        // GBK 保存: GBK, Big5 and EUC-KR all read both characters as common text.
        let names: &[&[u8]] = &[b"\xb1\xa3\xb4\xe6"];
        let g = guess(names, "动作");
        assert_eq!(g.encoding, GBK);
        assert!(!g.uncertain);
        assert_eq!(decode(names[0], g.encoding), "保存");
        assert_eq!(guess(names, "Set 1"), Guess { encoding: WINDOWS_1252, uncertain: true });
    }

    #[test]
    fn typographic_punctuation_reads_as_1252() {
        // ’ (0x92) then a letter is also a level-1 kanji pair in Shift_JIS.
        let names: &[&[u8]] = &[b"Taille de l\x92image"];
        let g = guess(names, "Actions par défaut");
        assert_eq!(g.encoding, WINDOWS_1252);
        assert!(!g.uncertain);
        assert_eq!(decode(names[0], g.encoding), "Taille de l’image");
        assert_eq!(guess(names, "Set 1"), Guess { encoding: WINDOWS_1252, uncertain: true });
    }

    #[test]
    fn utf8_lookalikes_read_as_common_text() {
        // GBK 去色 and EUC-KR 칠 are also valid UTF-8 ("ȥɫ", "ĥ").
        assert!(reads_as_common(b"\xc8\xa5\xc9\xab", GBK));
        assert!(reads_as_common(b"\xc4\xa5", EUC_KR));
        // Real UTF-8 text does not: its continuation bytes fall outside the common ranges.
        assert!(!reads_as_common("高斯模糊".as_bytes(), GBK));
        assert!(!reads_as_common("Gaussian Blur ✓".as_bytes(), GBK));
        assert!(!reads_as_common("Größe".as_bytes(), WINDOWS_1252));
        assert!(!reads_as_common(b"Blur", GBK), "ASCII has nothing to re-read");
        // Nor does UTF-8 in a single-byte page, though "Weiß" is Ã and Ÿ there and "Привет" is
        // all Р, С and letters: accidental UTF-8 is a double-byte page's problem.
        assert!(!reads_as_common("Weiß".as_bytes(), WINDOWS_1252));
        assert!(!reads_as_common("Привет".as_bytes(), WINDOWS_1251));
        assert!(!reads_as_common("Weiß".as_bytes(), MACINTOSH));
    }

    #[test]
    fn latin1_symbols_read_as_1252_not_mac_roman() {
        // Mac Roman has letters where windows-1252 has some symbols: 1252 "Größe »" is Mac Roman
        // "Grˆﬂe ª", "50 °" is "50 ∞". Latin-1 (what a code-page-blind reader shows) wins.
        for words in [&["Größe »"][..], &["50 °"], &["Contraste « auto »", "© Größe", "± 5"]] {
            let g = guess_words(WINDOWS_1252, words, "Größe ändern");
            assert_eq!(g, Guess { encoding: WINDOWS_1252, uncertain: false }, "{words:?}");
            assert_eq!(guess_words(WINDOWS_1252, words, "Set 1").encoding, WINDOWS_1252, "{words:?}");
        }
    }

    #[test]
    fn symbols_on_mac_roman_letters_stay_ambiguous() {
        // Known limitation: 1252 "Logo ®" is Mac Roman "Logo Æ", and ® is not counted for
        // windows-1252 because Mac "Æble" and "Weiß" use those bytes, so the name reads as Mac
        // Roman whatever the hint, as it did before the symbols counted at all.
        let logo: &[&[u8]] = &[b"Logo \xae"];
        assert_eq!(guess(logo, "Größe ändern"), Guess { encoding: MACINTOSH, uncertain: false });
        assert_eq!(guess(logo, "Set 1"), Guess { encoding: MACINTOSH, uncertain: false });
    }

    #[test]
    fn mac_roman_letters_still_read_as_mac_roman() {
        // Mac Roman "Größe" is 1252 "Grš§e", "Ändern" "€ndern", "Contrôle" "Contr™le": Mac Roman
        // still wins once § counts as common text in windows-1252.
        // Mac Roman ß ø æ Æ are 1252 § ¿ ¾ ®, which windows-1252 must not count as common.
        let words = [
            "Größe",
            "Ändern",
            "Contrôle",
            "Weiß",
            "Schwarzweiß",
            "Gaußscher Weichzeichner",
            "Schließen",
            "Størrelse",
            "Lærredstørrelse",
            "Fjern støy",
            "Æble",
        ];
        for word in words {
            let g = guess_words(MACINTOSH, &[word], "Définir l’arrière-plan");
            assert_eq!(g, Guess { encoding: MACINTOSH, uncertain: false }, "{word}");
        }
    }

    #[test]
    fn latin_hint_reads_mac_roman_when_1252_does_not_fit() {
        // Mac Roman "Arrière-plan": è is 0x8F, a control character in windows-1252.
        let names: &[&[u8]] = &[b"Arri\x8fre-plan"];
        let g = guess(names, "Définir l’arrière-plan");
        assert_eq!(g.encoding, MACINTOSH);
        assert!(!g.uncertain);
        assert_eq!(decode(names[0], g.encoding), "Arrière-plan");
    }

    #[test]
    fn known_names_pick_the_code_page() {
        // Every step of the corpus/atn sets that has a non-ASCII panel name also stores it as
        // Unicode (the descriptor's class name); here, Mac Roman "Définir".
        let pairs: &[(&[u8], &str)] = &[(b"D\x8efinir", "Définir")];
        assert_eq!(match_known(pairs), Some(MACINTOSH));
        // GBK 保存 is a three-way tie for the statistics; the known name settles it.
        assert_eq!(match_known(&[(b"\xb1\xa3\xb4\xe6", "保存")]), Some(GBK));
    }

    #[test]
    fn most_matching_names_win() {
        let pairs: &[(&[u8], &str)] = &[(b"\xb1\xa3\xb4\xe6", "保存"), (b"D\x8efinir", "Définir"), (b"Arri\x8fre-plan", "Arrière-plan")];
        assert_eq!(match_known(pairs), Some(MACINTOSH));
    }

    #[test]
    fn known_name_ties_take_candidate_order() {
        // Curly quotes are 0x93/0x94 in both windows-1252 and windows-1251.
        assert_eq!(match_known(&[(b"\x93x\x94", "“x”")]), Some(WINDOWS_1252));
    }

    #[test]
    fn known_names_that_match_nothing_decide_nothing() {
        assert_eq!(match_known(&[(b"D\x8efinir", "Set")]), None);
        assert_eq!(match_known(&[]), None);
        // Past the cap, pairs are not examined.
        let mut pairs = vec![(&b"D\x8efinir"[..], "x"); KNOWN_PAIRS];
        pairs.push((b"\xb1\xa3\xb4\xe6", "保存"));
        assert_eq!(match_known(&pairs), None);
    }

    #[test]
    fn bytes_no_cjk_page_accepts_still_decode() {
        // 0xFF is invalid in every double-byte candidate; windows-1251 (Ђ я Ђ) and Mac Roman
        // (Ä ˇ Ä) read all three bytes as letters, windows-1252 only one (€ ÿ €). With nothing to
        // break the tie, the fallback still decodes them.
        let names: &[&[u8]] = &[b"\x80\xff\x80"];
        let g = guess(names, "");
        assert_eq!(g, Guess { encoding: WINDOWS_1252, uncertain: true });
        assert_eq!(decode(names[0], g.encoding), "€ÿ€");
        assert_eq!(guess(names, "Набор").encoding, WINDOWS_1251);
    }

    #[test]
    fn empty_or_utf8_input_is_utf8() {
        let g = guess(&[], "");
        assert_eq!(g.encoding, UTF_8);
        assert!(!g.uncertain);
    }

    #[test]
    fn mixed_code_pages_pick_one_and_never_fail() {
        let names: &[&[u8]] = &[GBK_ISSUE[0], SJIS_COMMENT[0]];
        let g = guess(names, "");
        for name in names {
            assert!(!decode(name, g.encoding).is_empty());
        }
    }

    #[test]
    fn huge_input_is_sampled() {
        let names = vec![GBK_ISSUE[0]; 10_000];
        assert_eq!(guess(&names, "").encoding, GBK);
    }

    #[test]
    fn one_name_over_the_sample_cap_is_truncated_not_rejected() {
        // "a" then 可 repeated past the cap, so the cut lands inside a two-byte character.
        let mut name = b"a".to_vec();
        name.extend(b"\xbf\xc9".repeat(GUESS_SAMPLE / 2 + 1));
        assert_eq!(guess(&[&name], "可").encoding, GBK);
    }

    #[test]
    fn labels_resolve_and_reject() {
        for ok in ["shift_jis", "gbk", "big5", "euc-kr", "windows-1252", "utf-8"] {
            assert!(resolve_label(ok).is_ok(), "{ok}");
        }
        for bad in ["nope", "replacement", "utf-16le", "x-user-defined", "", "iso-2022-kr", "iso-2022-jp", "csISO2022JP"] {
            let Err(e) = resolve_label(bad) else { panic!("{bad} should be rejected") };
            assert!(e.to_string().contains("shift_jis"), "{e}");
        }
    }

    #[test]
    fn labels_match_loosely() {
        assert_eq!(resolve_label("Shift_JIS").ok(), Some(SHIFT_JIS));
        assert_eq!(resolve_label(" gbk ").ok(), Some(GBK));
        assert_eq!(resolve_label("gb2312").ok(), Some(GBK));
    }
}
