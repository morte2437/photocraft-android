//! Guides ⇄ PSD beyond image resource 1032.
//!
//! Photoshop writes two undocumented resources next to 1032 (Grid and Guides), layout
//! inferred from the psd-tools, ag-psd and Photoshop-authored corpora (#5965):
//!
//! * 1092, Grid and Guides version 2: `u32` version 2, `u32` horizontal and vertical grid
//!   cycle (576 = 18 px in 1/32 px, as in 1032), `u32` count, then per guide 9 bytes: `i32`
//!   position in 1/32 px, `u8` direction (0 vertical, 1 horizontal), one `u8` that is always 0
//!   in the corpora, and the guide's colour as `u8` R, G, B. In every corpus file its guides
//!   equal 1032's, in the same order.
//! * 1097: `u32` count (1092's), then per guide a length-prefixed ASCII GUID, a `u32` 0 and a
//!   second length-prefixed GUID, always all zeros.
//!
//! Import takes the guides' own colours from 1092 (a guide in Photoshop's default colour keeps
//! following the preference colour). Export keeps both resources only while 1092 still
//! describes the document's guides and colours; otherwise it leaves both out, so the file
//! carries 1032 alone, as Photoshop files before version 2 did (#5966).

use photocraft_doc::{Document, Guides};

/// Grid and Guides, version 2 (positions, directions and colours).
pub const GUIDES_V2: u16 = 1092;
/// One GUID per guide, in 1092's order.
pub const GUIDE_IDS: u16 = 1097;
/// Photoshop's default guide colour (Cyan, `#4affff`), which 1092 also stores for guides
/// that use the preference colour.
pub const DEFAULT_GUIDE_RGB: [u8; 3] = [0x4a, 0xff, 0xff];

/// One guide of a 1092 resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GuideV2 {
    /// Position in 1/32 px.
    pub location: i32,
    pub vertical: bool,
    pub rgb: [u8; 3],
}

fn be_u32(data: &[u8], at: usize) -> Option<u32> {
    let b = data.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// The guides of a 1092 resource, or `None` when it is not version 2 or is cut short.
pub fn parse_v2(data: &[u8]) -> Option<Vec<GuideV2>> {
    if be_u32(data, 0)? != 2 {
        return None;
    }
    let count = usize::try_from(be_u32(data, 12)?).ok()?;
    // Each guide takes 9 bytes: a count the data can't hold is not a 1092 we understand.
    if count > data.len().saturating_sub(16) / 9 {
        return None;
    }
    (0..count)
        .map(|i| {
            let at = 16 + i * 9;
            let g = data.get(at..at + 9)?;
            Some(GuideV2 { location: i32::from_be_bytes([g[0], g[1], g[2], g[3]]), vertical: g[4] != 1, rgb: [g[6], g[7], g[8]] })
        })
        .collect()
}

/// The number of entries a 1097 resource declares.
pub fn ids_count(data: &[u8]) -> Option<u32> {
    be_u32(data, 0)
}

/// The document's guides as 1092 would store them, sorted.
fn expected(guides: &Guides) -> Vec<GuideV2> {
    let mut v: Vec<GuideV2> = [(&guides.vertical, true), (&guides.horizontal, false)]
        .into_iter()
        .flat_map(|(list, vertical)| {
            list.iter().map(move |&p| GuideV2 { location: (p * 32.0).round() as i32, vertical, rgb: guides.color(vertical, p).unwrap_or(DEFAULT_GUIDE_RGB) })
        })
        .collect();
    v.sort();
    v
}

/// Whether an imported 1092 still describes the document's guides: the same positions,
/// directions and colours, in any order.
pub fn describes(doc: &Document, data: &[u8]) -> bool {
    let Some(mut stored) = parse_v2(data) else { return false };
    stored.sort();
    stored == expected(&doc.guides)
}

/// Whether to write back an imported 1092 or 1097: only while 1092 describes the guides
/// (and, for 1097, holds as many entries as 1092).
pub fn keep_resource(doc: &Document, id: u16, data: &[u8]) -> bool {
    match id {
        GUIDES_V2 => describes(doc, data),
        GUIDE_IDS => {
            let v2 = doc.metadata.psd_resources.iter().find(|(rid, _, _)| *rid == GUIDES_V2);
            v2.is_some_and(|(_, _, d)| describes(doc, d) && parse_v2(d).map(|g| g.len()).and_then(|n| u32::try_from(n).ok()) == ids_count(data))
        }
        _ => true,
    }
}

/// Gives the document's guides the colours an imported 1092 stores. Only guides that 1032
/// also has are coloured, and Photoshop's default colour is left to the preference.
pub fn apply_colors(doc: &mut Document) {
    let Some(stored) = doc.metadata.psd_resources.iter().find(|(id, _, _)| *id == GUIDES_V2).and_then(|(_, _, d)| parse_v2(d)) else {
        return;
    };
    for g in stored {
        if g.rgb == DEFAULT_GUIDE_RGB {
            continue;
        }
        let pos = f64::from(g.location) / 32.0;
        let list = if g.vertical { &doc.guides.vertical } else { &doc.guides.horizontal };
        if list.contains(&pos) {
            doc.guides.set_color(g.vertical, pos, Some(g.rgb));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v2(guides: &[(i32, u8, [u8; 3])]) -> Vec<u8> {
        let mut v = Vec::new();
        for n in [2u32, 576, 576, guides.len() as u32] {
            v.extend_from_slice(&n.to_be_bytes());
        }
        for (loc, dir, rgb) in guides {
            v.extend_from_slice(&loc.to_be_bytes());
            v.extend_from_slice(&[*dir, 0]);
            v.extend_from_slice(rgb);
        }
        v
    }

    #[test]
    fn parses_corpus_layout() {
        // clipping-mask4.psd (psd-tools): one vertical and one horizontal guide at 16 px.
        let d = v2(&[(512, 0, DEFAULT_GUIDE_RGB), (512, 1, DEFAULT_GUIDE_RGB)]);
        let g = parse_v2(&d).unwrap();
        assert_eq!(g.len(), 2);
        assert!(g[0].vertical && !g[1].vertical);
        assert_eq!(g[1].location, 512);
    }

    #[test]
    fn rejects_other_versions_and_short_or_hostile_data() {
        let mut d = v2(&[(512, 0, DEFAULT_GUIDE_RGB)]);
        assert!(parse_v2(&d[..d.len() - 1]).is_none());
        d[3] = 1;
        assert!(parse_v2(&d).is_none());
        let mut huge = v2(&[]);
        huge[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse_v2(&huge).is_none());
        assert!(parse_v2(&[]).is_none());
        assert!(ids_count(&[0, 0]).is_none());
    }
}
