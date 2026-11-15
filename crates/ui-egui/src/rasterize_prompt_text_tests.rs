//! #11899: the rasterize prompt for a smart object uses the reference application's wording;
//! the type, shape and fill prompts are unchanged, and the catalogs that translated the old text
//! translate the new one.

use super::*;
use crate::i18n::{Lang, tr};

const OLD: &str = "This Smart Object must be rasterized before proceeding. Its contents will no longer be editable. Rasterize the Smart Object?";

#[test]
fn the_four_prompts_read_as_measured() {
    assert_eq!(
        Kind::SmartObject.message(),
        "This smart object must be rasterized before proceeding. Edit Contents will no longer be available. Rasterize the smart object?"
    );
    assert_eq!(Kind::Type.message(), "This type layer must be rasterized before proceeding. Its text will no longer be editable. Rasterize the type?");
    assert_eq!(Kind::Shape.message(), "This shape layer must be rasterized before proceeding. It will no longer be editable as a shape. Rasterize the shape?");
    assert_eq!(
        Kind::Fill.message(),
        "The fill for this layer must be rasterized before proceeding. It will no longer be editable as a fill. Rasterize the fill?"
    );
    assert!(!Kind::SmartObject.message().contains("  "), "single spaces between sentences");
}

#[test]
fn catalogs_that_had_a_translation_keep_one() {
    if !crate::i18n::needs_catalogs() {
        return;
    }
    let new = Kind::SmartObject.message();
    let fr = Lang::from_code("fr").expect("French");
    assert_ne!(tr(fr, new), new, "French still translates the prompt");
    // A language without an entry falls back to English.
    let de = Lang::from_code("de").expect("German");
    assert_eq!(tr(de, new), new);
    // No catalog keeps the old key.
    for (code, _, catalog) in crate::i18n::CRAFT_I18N {
        // Rows are `context <TAB> English <TAB> translation`; `#` starts a comment.
        let mut keys = catalog.lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.split('\t').nth(1));
        assert!(keys.all(|k| k != OLD), "{code} still has the old key");
    }
}
