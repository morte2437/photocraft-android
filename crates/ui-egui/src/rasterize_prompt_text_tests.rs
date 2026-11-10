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
    assert_eq!(Kind::Fill.message(), "This fill layer must be rasterized before proceeding. Its fill content will no longer be editable. Rasterize the layer?");
    assert!(!Kind::SmartObject.message().contains("  "), "single spaces between sentences");
}

#[test]
fn catalogs_that_had_a_translation_keep_one() {
    let new = Kind::SmartObject.message();
    let fr = Lang::from_code("fr").expect("French");
    assert_ne!(tr(fr, new), new, "French still translates the prompt");
    // A language without an entry falls back to English.
    let de = Lang::from_code("de").expect("German");
    assert_eq!(tr(de, new), new);
    // No catalog keeps the old key.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/i18n");
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "tsv") {
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(!text.contains(OLD), "{} still has the old key", path.display());
        }
    }
}
