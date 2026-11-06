//! An embedded, empty Type 3 font for the selection plane. Visible glyphs remain paths.
//! No source font programs are copied into this font; each code has a Unicode mapping.
use std::collections::BTreeSet;
use std::fmt::Write as _;

#[derive(Default)]
pub(super) struct SelectionFont {
    chars: BTreeSet<char>,
}

impl SelectionFont {
    /// Collect distinct characters until the one-byte code space is exceeded. Stopping there,
    /// rather than after a fixed number of characters scanned, keeps a long run of repeated
    /// text from hiding later distinct characters.
    pub(super) fn new<'a>(texts: impl Iterator<Item = &'a str>) -> Self {
        let mut chars = BTreeSet::new();
        'texts: for text in texts {
            for ch in text.chars() {
                chars.insert(ch);
                if chars.len() > 255 {
                    break 'texts;
                }
            }
        }
        Self { chars }
    }

    pub(super) fn encode(&self, text: &str) -> Option<String> {
        if self.chars.len() > 255 {
            return None;
        }
        let mut encoded = String::new();
        for ch in text.chars() {
            let code = self.chars.iter().position(|c| *c == ch)?.checked_add(1)?;
            write!(encoded, "{code:02X}").ok()?;
        }
        Some(encoded)
    }

    // Object numbers are assigned by the caller, after the optional ICC object.
    pub(super) fn objects(&self, base: usize) -> Vec<Vec<u8>> {
        let count = self.chars.len().min(255);
        let mut differences = String::new();
        let mut procs = String::new();
        let mut cmap = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /PhotoCraftUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<00> <FF>\nendcodespacerange\n",
        );
        // PDF permits at most 100 entries in each bfchar block.
        let chars: Vec<_> = self.chars.iter().take(count).collect();
        for (block, chunk) in chars.chunks(100).enumerate() {
            let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
            for (i, ch) in chunk.iter().enumerate() {
                let code = block * 100 + i + 1;
                let mut buf = [0; 2];
                let unicode = ch.encode_utf16(&mut buf).iter().map(|u| format!("{u:04X}")).collect::<String>();
                let _ = writeln!(cmap, "<{code:02X}> <{unicode}>");
            }
            cmap.push_str("endbfchar\n");
        }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        for code in 1..=count {
            let _ = write!(differences, " /c{code}");
            let _ = write!(procs, " /c{code} {} 0 R", base + 1);
        }
        let widths = "1000 ".repeat(count.max(1));
        let font = format!(
            "<< /Type /Font /Subtype /Type3 /Name /FSelect /FontBBox [0 -250 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs <<{procs} >> /Encoding << /Type /Encoding /Differences [1{differences}] >> /FirstChar 1 /LastChar {} /Widths [{widths}] /Resources << >> /ToUnicode {} 0 R >>",
            count.max(1),
            base + 2
        );
        let stream = |data: &str| format!("<< /Length {} >>\nstream\n{data}endstream", data.len()).into_bytes();
        vec![font.into_bytes(), stream("1000 0 0 -250 1000 1000 d1\n"), stream(&cmap)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_characters_after_a_long_repeated_run_still_encode() {
        let mut text = "a".repeat(70_000);
        text.push('Ω');
        let font = SelectionFont::new(std::iter::once(text.as_str()));
        assert!(font.encode("aΩ").is_some(), "the distinct tail character must survive the repetition");
    }

    #[test]
    fn the_one_byte_code_space_fits_exactly_255_distinct_characters() {
        let fits: String = (0x100u32..0x1FF).filter_map(char::from_u32).collect();
        assert_eq!(fits.chars().count(), 255);
        let font = SelectionFont::new(std::iter::once(fits.as_str()));
        assert!(font.encode(&fits).is_some(), "255 distinct characters fit one-byte codes");

        let too_many: String = (0x100u32..0x200).filter_map(char::from_u32).collect();
        assert_eq!(too_many.chars().count(), 256);
        let font = SelectionFont::new(std::iter::once(too_many.as_str()));
        assert!(font.encode("Ā").is_none(), "256 distinct characters exceed the code space");
    }

    #[test]
    fn to_unicode_maps_codes_beyond_the_first_bfchar_block() {
        let text: String = (0x100u32..0x196).filter_map(char::from_u32).collect();
        let font = SelectionFont::new(std::iter::once(text.as_str()));
        let objects = font.objects(0);
        let cmap = String::from_utf8_lossy(&objects[2]);
        assert!(cmap.contains("100 beginbfchar"));
        assert!(cmap.contains("50 beginbfchar"));
        assert!(cmap.contains("<65> <0164>"));
    }
}
