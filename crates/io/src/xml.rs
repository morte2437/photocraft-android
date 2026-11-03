//! XML parsing with a nesting cap. roxmltree recurses once per element level, so a few thousand
//! nested tags (a 30 KB file or command param) overflow a 2 MB thread stack, an abort no guard
//! can catch. Every roxmltree caller that reads a file or a param goes through [`parse`].

use roxmltree::{Document, ParsingOptions};

/// Deepest element nesting accepted, the same as for SVG drawings: real files stay far below.
pub const MAX_DEPTH: usize = crate::svg::MAX_XML_DEPTH;

/// Parses `text`, refusing nesting deeper than [`MAX_DEPTH`] before roxmltree sees it.
///
/// With `opts.allow_dtd` an internal DTD subset is refused too: its entities can expand into
/// markup the depth check never sees. An external DOCTYPE (a public or system id) still parses.
pub fn parse<'i>(text: &'i str, opts: ParsingOptions<'i>) -> Result<Document<'i>, String> {
    if opts.allow_dtd && internal_subset(text.as_bytes()) {
        return Err("internal DTD subsets aren't supported".into());
    }
    if crate::svg::nesting_depth(text.as_bytes(), MAX_DEPTH) > MAX_DEPTH {
        return Err(format!("elements are nested more than {MAX_DEPTH} levels deep"));
    }
    Document::parse_with_options(text, opts).map_err(|e| e.to_string())
}

/// Does a DOCTYPE have an internal subset (`<!DOCTYPE x [ … ]>`)? Every `<!DOCTYPE` in the text
/// is checked, not only the one a walk over the prolog reaches: a quoted `?>` in the XML
/// declaration must not hide it. One inside a comment may be refused needlessly; that fails safe.
///
/// Each `<!DOCTYPE` starts a scan over what follows: the name and the quoted public/system ids
/// come first, a `[` outside quotes opens the subset, a `>` ends a DOCTYPE without one. Scans in
/// the same quote state go on alike, so one pass keeps just three flags (outside quotes, in `"`,
/// in `'`) and stays linear however many `<!DOCTYPE`s a comment holds.
fn internal_subset(xml: &[u8]) -> bool {
    const DOCTYPE: &[u8] = b"<!DOCTYPE";
    let (mut bare, mut double, mut single) = (false, false, false);
    for (at, &b) in xml.iter().enumerate() {
        if at.checked_sub(DOCTYPE.len()).and_then(|start| xml.get(start..at)) == Some(DOCTYPE) {
            bare = true;
        }
        if bare && b == b'[' {
            return true;
        }
        (bare, double, single) = (
            (bare && !matches!(b, b'"' | b'\'' | b'>')) || (double && b == b'"') || (single && b == b'\''),
            (double && b != b'"') || (bare && b == b'"'),
            (single && b != b'\'') || (bare && b == b'\''),
        );
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested(levels: usize) -> String {
        format!("<a>{}{}</a>", "<g>".repeat(levels), "</g>".repeat(levels))
    }

    fn dtd<'a>() -> ParsingOptions<'a> {
        ParsingOptions { allow_dtd: true, ..Default::default() }
    }

    #[test]
    fn nesting_past_the_cap_is_an_error_not_a_stack_overflow() {
        let deep = nested(20_000);
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(move || {
                for opts in [ParsingOptions::default(), dtd()] {
                    let e = parse(&deep, opts).err().unwrap_or_else(|| panic!("parsed"));
                    assert!(e.contains("nested") && e.contains(&MAX_DEPTH.to_string()), "{e}");
                }
            })
            .unwrap()
            .join()
            .unwrap();
        // The root plus MAX_DEPTH - 1 levels is exactly MAX_DEPTH deep.
        assert!(parse(&nested(MAX_DEPTH - 1), ParsingOptions::default()).is_ok());
        assert!(parse(&nested(MAX_DEPTH), ParsingOptions::default()).is_err());
    }

    #[test]
    fn markup_in_comments_cdata_pis_and_attributes_adds_no_depth() {
        let g = "<g>".repeat(2 * MAX_DEPTH);
        let xml = format!("<?xml version=\"1.0\"?><!-- {g} --><a t=\"&lt;&gt; /&gt; >\"><![CDATA[ {g} ]]><?pi {g} ?><b/></a>");
        assert!(parse(&xml, ParsingOptions::default()).is_ok());
    }

    /// The scan per `<!DOCTYPE` that [`internal_subset`] folds into one pass.
    fn internal_subset_per_doctype(xml: &[u8]) -> bool {
        const DOCTYPE: &[u8] = b"<!DOCTYPE";
        xml.windows(DOCTYPE.len()).enumerate().filter(|(_, w)| *w == DOCTYPE).any(|(at, _)| {
            let mut quote = None;
            for &b in &xml[at + DOCTYPE.len()..] {
                match quote {
                    Some(q) if b == q => quote = None,
                    Some(_) => {}
                    None if b == b'"' || b == b'\'' => quote = Some(b),
                    None if b == b'[' => return true,
                    None if b == b'>' => return false,
                    None => {}
                }
            }
            false
        })
    }

    #[test]
    fn the_one_pass_doctype_scan_agrees_with_a_scan_per_doctype() {
        let parts: [&[u8]; 7] = [b"<!DOCTYPE", b"\"", b"'", b"[", b">", b"a", b"<!-- "];
        // Every sequence of up to six parts, so the scans overlap in each quote state.
        let mut checked = 0;
        for len in 0..=6u32 {
            for mut code in 0..parts.len().pow(len) {
                let mut xml = Vec::new();
                for _ in 0..len {
                    xml.extend_from_slice(parts[code % parts.len()]);
                    code /= parts.len();
                }
                assert_eq!(internal_subset(&xml), internal_subset_per_doctype(&xml), "{}", String::from_utf8_lossy(&xml));
                checked += 1;
            }
        }
        assert!(checked > 100_000);
    }

    #[test]
    fn many_doctypes_in_a_comment_scan_in_linear_time() {
        // A scan per `<!DOCTYPE` reads on to the end from each of the ~1.9 million here, some
        // 10^13 steps; one pass reads the 16 MiB once.
        let xml = format!("<!-- {} --><a/>", "<!DOCTYPE".repeat((16 << 20) / 9));
        let started = std::time::Instant::now();
        assert!(parse(&xml, dtd()).is_ok());
        assert!(started.elapsed() < std::time::Duration::from_secs(20), "{:?}", started.elapsed());
    }

    #[test]
    fn an_internal_subset_is_refused_an_external_doctype_parses() {
        let krita = "<?xml version=\"1.0\"?>\n<!-- c --><!DOCTYPE DOC PUBLIC '-//KDE//DTD krita 2.0//EN' 'http://www.calligra.org/DTD/[krita].dtd'>\n<DOC/>";
        assert!(parse(krita, dtd()).is_ok());
        let g = ("<g>".repeat(20_000), "</g>".repeat(20_000));
        for xml in [
            "<!DOCTYPE a [<!ENTITY e \"x\">]><a>&e;</a>".to_string(),
            "\u{feff}<?xml version=\"1.0\"?><!-- ' --> <!DOCTYPE a SYSTEM \"a.dtd\" [<!-- ' -->]><a/>".to_string(),
            format!("<!DOCTYPE a [<!ENTITY e \"{}{}\">]><a>&e;</a>", g.0, g.1),
            // roxmltree reads the declaration's values as quoted attributes, so they may hold `?>`.
            format!("<?xml version=\"?>\"?><!DOCTYPE a [<!ENTITY e \"{}{}\">]><a>&e;</a>", g.0, g.1),
            format!("<?xml version=\"?>\"?><!DOCTYPE a [<!-- ' -->]><a>{}{}</a><!-- ' -->", g.0, g.1),
        ] {
            let e = parse(&xml, dtd()).err().unwrap_or_else(|| panic!("parsed {xml:.60}"));
            assert!(e.contains("internal DTD"), "{e}");
        }
        // Without allow_dtd roxmltree refuses any DOCTYPE itself.
        assert!(parse(krita, ParsingOptions::default()).is_err());
    }
}
