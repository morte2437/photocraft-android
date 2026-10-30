//! Right-to-left text in labels that egui draws (#5159).
//!
//! egui lays text out with the Unicode Bidirectional Algorithm: each paragraph takes the direction
//! of its first strong letter, and a right-to-left run is placed right to left. Labels, names and
//! translations are therefore handed to egui in logical order, as typed. What egui can't know is
//! where a right-to-left name may be cut: [`elide`] keeps the *start* of the name.

use unicode_bidi::{BidiClass, bidi_class};

/// Whether `text` reads right to left as a paragraph: its first strong letter is Hebrew, Arabic…
pub fn is_rtl_paragraph(text: &str) -> bool {
    text.chars()
        .find_map(|c| match bidi_class(c) {
            BidiClass::L => Some(false),
            BidiClass::R | BidiClass::AL => Some(true),
            _ => None,
        })
        .unwrap_or(false)
}

/// A right-to-left `text` cut at its logical end with "…" where it doesn't fit (`fits` measures a
/// candidate). The start of the name stays, and egui puts the "…" at the left, the end of a
/// right-to-left line. `None`: `text` fits as it is (or a left-to-right one, which the caller
/// elides at its right end as usual).
pub fn elide(text: &str, fits: impl Fn(&str) -> bool) -> Option<String> {
    if !is_rtl_paragraph(text) || fits(text) {
        return None;
    }
    // The longest logical prefix (whole characters) that fits with "…" after it.
    let ends: Vec<usize> = text.char_indices().map(|(i, _)| i).skip(1).chain(std::iter::once(text.len())).collect();
    let cut = |n: usize| -> String {
        let end = if n == 0 { 0 } else { ends.get(n - 1).copied().unwrap_or(text.len()) };
        format!("{}…", text.get(..end).unwrap_or_default().trim_end())
    };
    let (mut lo, mut hi) = (0usize, ends.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(&cut(mid)) { lo = mid } else { hi = mid - 1 }
    }
    Some(cut(lo))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #5385 review: a right-to-left name that doesn't fit keeps its first words and ends with
    /// "…"; a left-to-right or fitting name is left to the caller.
    #[test]
    fn right_to_left_names_are_cut_at_their_logical_end() {
        let fits = |n: usize| move |s: &str| s.chars().count() <= n;
        // Cut anywhere, as egui elides: the first word and the start of the second.
        assert_eq!(elide("لاشي مكتوب هنا", fits(8)).as_deref(), Some("لاشي مك…"));
        assert_eq!(elide("לשלום עולם גדול", fits(11)).as_deref(), Some("לשלום עולם…"));
        assert_eq!(elide("لاشي مكتوب هنا", fits(40)), None, "fits");
        assert_eq!(elide("Layer copy 12", fits(4)), None, "left to right: elided by egui at its end");
        assert_eq!(elide("نص", fits(0)).as_deref(), Some("…"), "nothing fits: just the ellipsis");
        assert!(is_rtl_paragraph("12 نص") && !is_rtl_paragraph("Layer نص") && !is_rtl_paragraph("123"));
    }
}
