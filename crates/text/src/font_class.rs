//! What a font menu filters and sorts by: a family's classification (Serif, Sans Serif, Script…),
//! its outline format and whether it is variable, read from the font's own tables (OS/2
//! `sFamilyClass` and PANOSE, `post.isFixedPitch`, `fvar`, `CFF `/`glyf`) and, where those say
//! nothing, from its name. Photoshop's font menu offers the same classes.

use skrifa::raw::{FontRef, TableProvider};

/// A family's classification, as the font menu's class filter lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FontClass {
    Serif,
    SlabSerif,
    SansSerif,
    Script,
    Monospace,
    Handwritten,
    Blackletter,
    Decorative,
}

impl FontClass {
    /// Every class, in menu order.
    pub const ALL: [FontClass; 8] = [
        FontClass::Serif,
        FontClass::SlabSerif,
        FontClass::SansSerif,
        FontClass::Script,
        FontClass::Monospace,
        FontClass::Handwritten,
        FontClass::Blackletter,
        FontClass::Decorative,
    ];

    /// The English menu label (translated by the UI).
    pub fn label(self) -> &'static str {
        match self {
            FontClass::Serif => "Serif",
            FontClass::SlabSerif => "Slab Serif",
            FontClass::SansSerif => "Sans Serif",
            FontClass::Script => "Script",
            FontClass::Monospace => "Monospace",
            FontClass::Handwritten => "Handwritten",
            FontClass::Blackletter => "Blackletter",
            FontClass::Decorative => "Decorative",
        }
    }

    /// The stable key for commands and saved filters (`"slabSerif"`).
    pub fn key(self) -> &'static str {
        match self {
            FontClass::Serif => "serif",
            FontClass::SlabSerif => "slabSerif",
            FontClass::SansSerif => "sansSerif",
            FontClass::Script => "script",
            FontClass::Monospace => "monospace",
            FontClass::Handwritten => "handwritten",
            FontClass::Blackletter => "blackletter",
            FontClass::Decorative => "decorative",
        }
    }

    /// The class with this [`Self::key`].
    pub fn from_key(key: &str) -> Option<FontClass> {
        Self::ALL.into_iter().find(|c| c.key() == key)
    }
}

/// How a family's outlines are stored, as the font menu's format badge shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontFormat {
    /// OpenType with PostScript (CFF) outlines.
    OpenType,
    /// TrueType outlines (`glyf`), including OpenType fonts that use them.
    TrueType,
    /// A variable font (`fvar`), whatever its outlines.
    Variable,
}

impl FontFormat {
    /// The badge text: Photoshop's "O", "TT" and "VAR".
    pub fn badge(self) -> &'static str {
        match self {
            FontFormat::OpenType => "O",
            FontFormat::TrueType => "TT",
            FontFormat::Variable => "VAR",
        }
    }
}

/// What the font menu knows about a family, from its representative (regular) face.
#[derive(Clone, Debug, PartialEq)]
pub struct FamilyTraits {
    pub class: Option<FontClass>,
    pub format: FontFormat,
    /// The 10 PANOSE digits (all 0 when the font doesn't say).
    pub panose: [u8; 10],
    /// `usWeightClass` (400 when absent).
    pub weight: u16,
}

impl FamilyTraits {
    pub fn variable(&self) -> bool {
        self.format == FontFormat::Variable
    }
}

/// Read a face's traits. Missing or malformed tables fall back to "unknown", never to an error.
pub fn traits_of(family: &str, font: &FontRef) -> FamilyTraits {
    let os2 = font.os2().ok();
    let family_class = os2.as_ref().map_or(0, |t| t.s_family_class());
    let mut panose = [0u8; 10];
    if let Some(bytes) = os2.as_ref().map(|t| t.panose_10()) {
        for (slot, b) in panose.iter_mut().zip(bytes.iter()) {
            *slot = *b;
        }
    }
    let weight = os2.as_ref().map_or(400, |t| t.us_weight_class()).clamp(1, 1000);
    let fixed_pitch = font.post().ok().is_some_and(|p| p.is_fixed_pitch() != 0);
    let format = if font.fvar().is_ok() {
        FontFormat::Variable
    } else if font.cff().is_ok() || font.cff2().is_ok() {
        FontFormat::OpenType
    } else {
        FontFormat::TrueType
    };
    FamilyTraits { class: classify(family, family_class, panose, fixed_pitch), format, panose, weight }
}

/// Name fragments (lowercase) that mark a blackletter face; the tables have no class for it.
const BLACKLETTER: [&str; 7] = ["fraktur", "blackletter", "black letter", "textura", "old english", "schwabacher", "rotunda"];

/// Classify from the OS/2 `sFamilyClass` (high byte: the IBM class), the PANOSE digits, the
/// `post` fixed-pitch flag and, last, the family name.
pub fn classify(family: &str, family_class: i16, panose: [u8; 10], fixed_pitch: bool) -> Option<FontClass> {
    let name = family.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    // Whole words of the name, so "Encode Sans" isn't read as a "code" font.
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    // PANOSE Latin Text proportion 9 is monospaced.
    if fixed_pitch || (panose[0] == 2 && panose[3] == 9) {
        return Some(FontClass::Monospace);
    }
    if has(&BLACKLETTER) {
        return Some(FontClass::Blackletter);
    }
    // PANOSE family kind: 3 Latin Hand Written, 4 Latin Decoratives, 5 Latin Symbol.
    match panose[0] {
        3 => return Some(if family_class >> 8 == 10 { FontClass::Script } else { FontClass::Handwritten }),
        4 | 5 => return Some(FontClass::Decorative),
        _ => {}
    }
    match family_class >> 8 {
        // 4 is Clarendon: bracketed serifs (Georgia), not slabs.
        1..=4 | 7 => return Some(FontClass::Serif),
        5 => return Some(FontClass::SlabSerif),
        8 => return Some(FontClass::SansSerif),
        9 | 12 => return Some(FontClass::Decorative),
        10 => return Some(if has(&["hand", "marker", "brush", "pen", "chalk"]) { FontClass::Handwritten } else { FontClass::Script }),
        _ => {}
    }
    // PANOSE Latin Text serif style: 11-15 are sans (normal, obtuse, perpendicular, flared,
    // rounded); square (6) is a slab; the rest, the square coves (bracketed serifs, as Georgia's)
    // included, are serifs.
    if panose[0] == 2 {
        match panose[1] {
            11..=15 => return Some(FontClass::SansSerif),
            6 => return Some(FontClass::SlabSerif),
            2..=10 => return Some(FontClass::Serif),
            _ => {}
        }
    }
    // Last, the name.
    if words.iter().any(|w| matches!(*w, "mono" | "monospace" | "monospaced" | "code" | "courier") || w.starts_with("consol")) {
        Some(FontClass::Monospace)
    } else if has(&["slab"]) {
        Some(FontClass::SlabSerif)
    } else if has(&["sans", "grotesk", "grotesque", "gothic"]) {
        Some(FontClass::SansSerif)
    } else if has(&["serif", "roman", "garamond", "times", "bodoni", "baskerville", "caslon"]) {
        Some(FontClass::Serif)
    } else if has(&["hand", "marker", "brush", "chalk"]) {
        Some(FontClass::Handwritten)
    } else if has(&["script"]) {
        Some(FontClass::Script)
    } else {
        None
    }
}

/// How unlike `b` is to `a`: 0 for the same traits, larger the more they differ. Used to list
/// similar fonts (Photoshop's "Show similar fonts"): the class matters most, then the PANOSE
/// digits both fonts state (serif style, weight, proportion, contrast…), then the weight.
pub fn difference(a: &FamilyTraits, b: &FamilyTraits) -> f32 {
    let mut d = 0.0;
    if a.class != b.class {
        d += 20.0;
    }
    // Digits 1..=9; 0 and 1 mean "any"/"no fit", so only stated digits count.
    for (x, y) in a.panose.iter().zip(b.panose.iter()).skip(1) {
        if *x > 1 && *y > 1 {
            d += f32::from(x.abs_diff(*y)).min(6.0);
        }
    }
    d + f32::from(a.weight.abs_diff(b.weight)) / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: [u8; 10] = [0; 10];

    #[test]
    fn the_os2_class_wins_over_the_name() {
        assert_eq!(classify("Georgia", 0x0105, NONE, false), Some(FontClass::Serif));
        assert_eq!(classify("Georgia", 0x0405, NONE, false), Some(FontClass::Serif), "Clarendon is a serif");
        assert_eq!(classify("Rockwell", 0x0501, NONE, false), Some(FontClass::SlabSerif));
        assert_eq!(classify("Arial", 0x0805, NONE, false), Some(FontClass::SansSerif));
        assert_eq!(classify("Edwardian Script ITC", 0x0a05, NONE, false), Some(FontClass::Script));
        assert_eq!(classify("Wingdings", 0x0c00, NONE, false), Some(FontClass::Decorative));
    }

    #[test]
    fn panose_classifies_fonts_without_an_os2_class() {
        let sans = [2, 11, 6, 4, 2, 2, 2, 2, 2, 4];
        let slab = [2, 6, 6, 3, 2, 2, 2, 2, 2, 4];
        let serif = [2, 2, 6, 3, 5, 4, 5, 2, 3, 4];
        let hand = [3, 1, 1, 1, 1, 1, 1, 1, 1, 1];
        let deco = [4, 1, 1, 1, 1, 1, 1, 1, 1, 1];
        let mono = [2, 11, 6, 9, 2, 2, 2, 2, 2, 4];
        assert_eq!(classify("A", 0, sans, false), Some(FontClass::SansSerif));
        assert_eq!(classify("B", 0, slab, false), Some(FontClass::SlabSerif));
        assert_eq!(classify("C", 0, serif, false), Some(FontClass::Serif));
        assert_eq!(classify("D", 0, hand, false), Some(FontClass::Handwritten));
        assert_eq!(classify("E", 0, deco, false), Some(FontClass::Decorative));
        assert_eq!(classify("F", 0, mono, false), Some(FontClass::Monospace));
        // Georgia: square cove serifs are bracketed serifs, not slabs.
        assert_eq!(classify("Georgia", 0, [2, 4, 5, 2, 5, 4, 5, 2, 3, 3], false), Some(FontClass::Serif));
    }

    #[test]
    fn fixed_pitch_and_names_decide_the_rest() {
        assert_eq!(classify("Anything", 0x0805, NONE, true), Some(FontClass::Monospace), "fixed pitch beats the class");
        assert_eq!(classify("UnifrakturMaguntia", 0, NONE, false), Some(FontClass::Blackletter));
        assert_eq!(classify("Old English Text MT", 0x0805, NONE, false), Some(FontClass::Blackletter));
        assert_eq!(classify("Roboto Slab", 0, NONE, false), Some(FontClass::SlabSerif));
        assert_eq!(classify("Noto Serif", 0, NONE, false), Some(FontClass::Serif));
        assert_eq!(classify("Noto Sans", 0, NONE, false), Some(FontClass::SansSerif));
        assert_eq!(classify("Fira Code", 0, NONE, false), Some(FontClass::Monospace));
        assert_eq!(classify("Space Mono", 0, NONE, false), Some(FontClass::Monospace));
        // "code" inside a word, or a name the tables contradict, doesn't make a monospace.
        assert_eq!(classify("Encode Sans", 0, NONE, false), Some(FontClass::SansSerif));
        assert_eq!(classify("Encode Sans", 0x0805, NONE, false), Some(FontClass::SansSerif));
        assert_eq!(classify("Code Pro Sans", 0x0805, NONE, false), Some(FontClass::SansSerif), "the class beats the name");
        assert_eq!(classify("Permanent Marker", 0, NONE, false), Some(FontClass::Handwritten));
        assert_eq!(classify("Unknowable", 0, NONE, false), None);
        // Out-of-range or garbage table values don't panic or misfile.
        assert_eq!(classify("X", i16::MIN, [255; 10], false), None);
        assert_eq!(classify("X", -1, [2, 200, 0, 0, 0, 0, 0, 0, 0, 0], false), None);
    }

    #[test]
    fn similar_fonts_share_class_shape_and_weight() {
        let t = |class, panose, weight| FamilyTraits { class: Some(class), format: FontFormat::TrueType, panose, weight };
        let arial = t(FontClass::SansSerif, [2, 11, 6, 4, 2, 2, 2, 2, 2, 4], 400);
        let helvetica = t(FontClass::SansSerif, [2, 11, 6, 4, 2, 2, 2, 2, 2, 4], 400);
        let arial_black = t(FontClass::SansSerif, [2, 11, 10, 4, 2, 1, 2, 2, 2, 4], 900);
        let times = t(FontClass::Serif, [2, 2, 6, 3, 5, 4, 5, 2, 3, 4], 400);
        assert_eq!(difference(&arial, &helvetica), 0.0);
        assert!(difference(&arial, &arial_black) < difference(&arial, &times));
        assert!(difference(&arial, &times) >= 20.0, "another class is far away");
    }

    #[test]
    fn bundled_families_are_read_from_their_tables() {
        let mut db = crate::FontDb::new();
        let mono = db.family_traits(crate::fonts::MONO_FAMILY).expect("the bundled mono font");
        assert_eq!(mono.class, Some(FontClass::Monospace));
        // Inter states no class or PANOSE: it is read, but left unclassified rather than guessed.
        let sans = db.family_traits(crate::fonts::DEFAULT_FAMILY).expect("the bundled UI font");
        assert!(sans.class.is_none() || sans.class == Some(FontClass::SansSerif), "{:?}", sans.class);
        assert!((1..=1000).contains(&sans.weight));
        assert_eq!(db.family_traits("No Such Family"), None);
        assert_eq!(db.family_traits(crate::fonts::DEFAULT_FAMILY), Some(sans), "cached");
    }

    #[test]
    fn class_keys_round_trip() {
        for c in FontClass::ALL {
            assert_eq!(FontClass::from_key(c.key()), Some(c));
        }
        assert_eq!(FontClass::from_key("slabSerif"), Some(FontClass::SlabSerif));
        assert_eq!(FontClass::from_key("nope"), None);
    }
}
