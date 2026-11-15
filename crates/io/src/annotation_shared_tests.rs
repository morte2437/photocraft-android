//! Frozen valid-wire writer oracle from PhotoCraft5bdbcc118.
//! Original wire writer Brandon Thomas aa87012a; saturating icon fix wstock0d6e58e2.
use super::*;
fn pascal(out: &mut Vec<u8>, s: &str) {
    let bytes: Vec<u8> = s.chars().map(|c| if (c as u32) < 256 { c as u32 as u8 } else { b'?' }).take(255).collect();
    out.push(bytes.len() as u8);
    out.extend_from_slice(&bytes);
    if (bytes.len() + 1) % 2 == 1 {
        out.push(0);
    }
}

fn legacy_note(n: &Note) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"txtA");
    b.push(u8::from(n.open));
    b.push(0x1C);
    b.extend_from_slice(&1u16.to_be_bytes());
    // The position comes from the file or a command and can sit anywhere in i32 (issue #939):
    // the cast saturates, and the icon's far edge must saturate too rather than wrap (release) or
    // overflow (debug) when the note is re-saved.
    let (x, y) = (n.position[0].round() as i32, n.position[1].round() as i32);
    for v in [y, x, y.saturating_add(20), x.saturating_add(16)] {
        b.extend_from_slice(&v.to_be_bytes());
    }
    let pp = if n.popup == [0.0; 4] { [n.position[0] + 20.0, n.position[1] + 20.0, n.position[0] + 260.0, n.position[1] + 160.0] } else { n.popup };
    for v in [pp[1], pp[0], pp[3], pp[2]] {
        b.extend_from_slice(&(v.round() as i32).to_be_bytes());
    }
    b.extend_from_slice(&0u16.to_be_bytes());
    let rgb = n.color.to_rgb();
    for v in rgb {
        b.extend_from_slice(&((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes());
    }
    b.extend_from_slice(&0u16.to_be_bytes());
    pascal(&mut b, &n.author);
    pascal(&mut b, "");
    pascal(&mut b, &n.modified);
    let text = encode_text(&n.text);
    b.extend_from_slice(&((text.len() + 12) as u32).to_be_bytes());
    b.extend_from_slice(b"txtC");
    b.extend_from_slice(&(text.len() as u32).to_be_bytes());
    b.extend_from_slice(&text);
    let mut out = ((b.len() + 4) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(&b);
    out
}

/// An `Anno` block holding `notes` (plus raw sound entries carried over).
fn legacy_anno(notes: &[Note], raw_extra: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&2u16.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&((notes.len() + raw_extra.len()) as u32).to_be_bytes());
    for n in notes {
        out.extend_from_slice(&legacy_note(n));
    }
    for r in raw_extra {
        out.extend_from_slice(r);
    }
    out
}

#[test]
fn shared_writer_matches_valid_legacy_bytes() {
    for text in ["", "ascii", "héllo ✓", "one\r\ntwo\nthree", "𝄞"] {
        for x in [0.0, -5.5, f64::from(i32::MAX), f64::INFINITY] {
            let note = Note { text: text.into(), author: "ä✓".repeat(180), modified: "date".into(), position: [x, 2.5], ..Default::default() };
            let extra = vec![vec![0, 0, 0, 8, b's', b'n', b'd', b'A']];
            assert_eq!(write_anno(&[note.clone()], &extra).unwrap(), legacy_anno(&[note], &extra));
        }
    }
}
#[test]
fn every_export_path_refuses_excess_notes() {
    for mode in [photocraft_doc::ColorMode::Rgb, photocraft_doc::ColorMode::Multichannel, photocraft_doc::ColorMode::Bitmap] {
        let mut d = Document::new("notes", photocraft_doc::Size::new(1, 1), mode, photocraft_doc::SampleType::U8);
        d.notes = vec![Note::default(); 4097];
        assert!(export_blocks(&d, vec![]).is_err());
        assert!(crate::document_to_psd(&d).is_err());
        assert!(crate::document_to_psd_with(&d, &Default::default()).is_err());
        assert!(crate::encode_with(&d, "psd", &Default::default(), &photocraft_raster::Interrupt::NONE).is_err());
        assert!(crate::encode_with(&d, "tiff", &Default::default(), &photocraft_raster::Interrupt::NONE).is_err());
    }
}
#[test]
fn text_cannot_cross_declared_record_boundary() {
    let mut block = write_anno(&[Note { text: "payload".into(), ..Default::default() }], &[]).unwrap();
    let length = u32::from_be_bytes(block[8..12].try_into().unwrap());
    block[8..12].copy_from_slice(&(length - 2).to_be_bytes());
    assert!(parse_entries(&block).is_none());
}

#[test]
fn normalized_utf16_budget_counts_units_not_utf8_bytes() {
    assert_eq!(text_bytes("✓✓✓", 8), Some(8));
    assert_eq!(text_bytes("𝄞", 6), Some(6));
    assert_eq!(text_bytes("a\r\nb\nc\r", 14), Some(14));
    assert_eq!(text_bytes("✓✓✓", 7), None);
    assert_eq!(text_bytes("𝄞", 5), None);
    let note = Note { text: "✓".repeat(3 * 1024 * 1024), ..Default::default() };
    assert!(write_anno(&[note], &[]).is_ok());
}

#[test]
fn unchanged_legacy_text_keeps_raw_bytes_without_utf16_rewrite() {
    let mut block = legacy_anno(&[Note { text: "x".into(), ..Default::default() }], &[]);
    let key = block.windows(4).position(|w| w == b"txtC").unwrap();
    block.truncate(key + 8);
    block.extend(std::iter::repeat_n(b'x', 9 * 1024 * 1024));
    let size = u32::try_from(block.len() - key - 8).unwrap();
    block[key + 4..key + 8].copy_from_slice(&size.to_be_bytes());
    block[key - 4..key].copy_from_slice(&(size + 12).to_be_bytes());
    let record = u32::try_from(block.len() - 8).unwrap();
    block[8..12].copy_from_slice(&record.to_be_bytes());
    let mut doc = Document::new("notes", photocraft_doc::Size::new(1, 1), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    doc.notes = parse_anno(&block).unwrap();
    assert!(write_anno(&doc.notes, &[]).is_err());
    let blocks = vec![(*b"8BIM", ANNO, Arc::new(block))];
    assert_eq!(export_blocks(&doc, blocks.clone()).unwrap(), blocks);
    doc.notes[0].text.push('x');
    assert!(export_blocks(&doc, blocks).is_err());
}
