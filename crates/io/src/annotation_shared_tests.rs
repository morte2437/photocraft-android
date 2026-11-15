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

