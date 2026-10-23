//! Amiga IFF/ILBM reader (EA "Interchange File Format", "InterLeaved BitMap" and the chunky
//! "PBM " variant). Written from the published IFF/ILBM layout: a `FORM` chunk of four-byte
//! chunks (`BMHD` bitmap header, `CMAP` palette, `CAMG` Amiga viewport mode, `BODY` pixel data).
//! Read-only: modern saves should use PNG.
//!
//! Supported: 1–8 bitplanes with a palette (or a grey ramp when there is none), masking 0/1/2,
//! and both stored (0) and ByteRun1/PackBits (1) compression. Anything else is a clear
//! `Unsupported`/`Malformed` error, never a panic.

use crate::error::CodecError;
use crate::format::Format;
use crate::image::{ChannelLayout, Image, SampleType};
use crate::options::Limits;

const F: Format = Format::Iff;

fn bad(msg: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, msg)
}

fn unsupported(msg: impl Into<String>) -> CodecError {
    CodecError::unsupported(F, msg)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_be_bytes(s))
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_be_bytes(s))
}

/// `FORM` + a bitmap type (`ILBM` interleaved, `PBM ` chunky) at offset 8.
pub(crate) fn looks_like_iff(b: &[u8]) -> bool {
    b.get(0..4) == Some(b"FORM") && matches!(b.get(8..12), Some(b"ILBM" | b"PBM "))
}

/// The bitmap header fields decoding needs.
struct Header {
    width: u32,
    height: u32,
    planes: u8,
    masking: u8,
    compression: u8,
    transparent: u16,
    /// `PBM ` stores one palette index per byte, padded to even row lengths.
    chunky: bool,
}

/// One four-byte chunk's id and byte range.
struct Chunk<'a> {
    id: [u8; 4],
    data: &'a [u8],
}

/// Walk the `FORM`'s chunks from offset 12 to `end` (the form length), stopping at the first
/// malformed length so a hostile file can't run past the buffer.
fn chunks(b: &[u8]) -> impl Iterator<Item = Chunk<'_>> {
    let end = be32(b, 4).map_or(b.len(), |n| (n as usize).saturating_add(8).min(b.len()));
    let mut pos = 12usize;
    std::iter::from_fn(move || {
        if pos + 8 > end {
            return None;
        }
        let id = b.get(pos..pos + 4)?.try_into().ok()?;
        let len = be32(b, pos + 4)? as usize;
        let start = pos + 8;
        let data = b.get(start..start.checked_add(len)?)?;
        // Chunks are padded to an even length.
        pos = start.checked_add(len)?.checked_add(len & 1)?;
        Some(Chunk { id, data })
    })
}

fn parse_header(b: &[u8], chunky: bool) -> Result<Header, CodecError> {
    let bmhd = chunks(b).find(|c| &c.id == b"BMHD").ok_or_else(|| bad("no BMHD chunk"))?;
    let d = bmhd.data;
    if d.len() < 20 {
        return Err(bad("BMHD chunk is too short"));
    }
    let width = u32::from(be16(d, 0).ok_or_else(|| bad("BMHD width"))?);
    let height = u32::from(be16(d, 2).ok_or_else(|| bad("BMHD height"))?);
    let planes = *d.get(8).ok_or_else(|| bad("BMHD planes"))?;
    let masking = *d.get(9).ok_or_else(|| bad("BMHD masking"))?;
    let compression = *d.get(10).ok_or_else(|| bad("BMHD compression"))?;
    if width == 0 || height == 0 {
        return Err(bad("zero-sized bitmap"));
    }
    if !(1..=8).contains(&planes) {
        return Err(unsupported(format!("{planes} bitplanes (1–8, with a palette)")));
    }
    if masking > 2 {
        return Err(unsupported(format!("masking {masking}")));
    }
    if compression > 1 {
        return Err(unsupported(format!("compression {compression} (only stored and ByteRun1)")));
    }
    if chunky && (planes != 8 || masking == 1) {
        return Err(unsupported("PBM requires 8-bit indices without a mask plane"));
    }
    let transparent = be16(d, 12).ok_or_else(|| bad("BMHD transparent colour"))?;
    Ok(Header { width, height, planes, masking, compression, transparent, chunky })
}

/// ByteRun1 / PackBits decode of one plane row into `out`; returns the bytes consumed. A `0x80`
/// control byte is a no-op, as the layout specifies.
fn unpack_byte_run1(src: &[u8], out: &mut [u8]) -> Result<usize, CodecError> {
    let (mut i, mut o) = (0usize, 0usize);
    while o < out.len() {
        let n = *src.get(i).ok_or_else(|| bad("truncated ByteRun1 data"))? as i8;
        i += 1;
        if n >= 0 {
            let count = n as usize + 1;
            let lit = src.get(i..i.checked_add(count).ok_or_else(|| bad("ByteRun1 overflow"))?).ok_or_else(|| bad("truncated ByteRun1 run"))?;
            if count > out.len() - o {
                return Err(bad("ByteRun1 run crosses the plane row"));
            }
            out[o..o + count].copy_from_slice(lit);
            i += count;
            o += count;
        } else if n != -128 {
            let count = (1 - n as i32) as usize;
            let value = *src.get(i).ok_or_else(|| bad("truncated ByteRun1 repeat"))?;
            i += 1;
            if count > out.len() - o {
                return Err(bad("ByteRun1 repeat crosses the plane row"));
            }
            out[o..o + count].fill(value);
            o += count;
        }
    }
    Ok(i)
}

pub(crate) fn decode(b: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    if !looks_like_iff(b) {
        return Err(bad("not an IFF/ILBM file (no FORM ... ILBM)"));
    }
    let h = parse_header(b, b.get(8..12) == Some(b"PBM "))?;
    if let Some(camg) = chunks(b).find(|c| &c.id == b"CAMG") {
        let mode = be32(camg.data, 0).ok_or_else(|| bad("short CAMG chunk"))?;
        if mode & (0x80 | 0x800) != 0 {
            return Err(unsupported("EHB and HAM viewport modes"));
        }
    }
    let layout = if h.masking == 0 { ChannelLayout::Rgb } else { ChannelLayout::Rgba };
    limits.check(h.width, h.height, layout, SampleType::U8)?;

    // Palette: three bytes per colour; missing entries read as black.
    let cmap = chunks(b).find(|c| &c.id == b"CMAP").map(|c| c.data).filter(|p| !p.is_empty());
    let colors = 1usize << h.planes;
    let palette = |idx: usize| -> [u8; 3] {
        if let Some(p) = cmap {
            p.get(idx * 3..idx * 3 + 3).map(|c| [c[0], c[1], c[2]]).unwrap_or([0; 3])
        } else {
            let v = ((idx as u32 * 255) / (colors as u32 - 1).max(1)) as u8;
            [v, v, v]
        }
    };

    let body = chunks(b).find(|c| &c.id == b"BODY").map(|c| c.data).ok_or_else(|| bad("no BODY chunk"))?;
    let (w, hh) = (h.width as usize, h.height as usize);
    // Each plane row is stored as a whole number of 16-bit words.
    let plane_row = if h.chunky { w.div_ceil(2) * 2 } else { w.div_ceil(16) * 2 };
    // A masking type 1 bitmap carries one extra (mask) plane per row.
    let rows_planes: usize = if h.chunky { 1 } else { usize::from(h.planes) + usize::from(h.masking == 1) };

    // Decode every stored plane row up front, then recombine.
    let total_rows = hh.checked_mul(rows_planes).ok_or_else(|| CodecError::LimitExceeded("bitmap too large".into()))?;
    let total_bytes = total_rows.checked_mul(plane_row).ok_or_else(|| CodecError::LimitExceeded("bitmap too large".into()))?;
    if total_bytes as u64 > limits.max_alloc {
        return Err(CodecError::LimitExceeded("bitmap storage exceeds max_alloc".into()));
    }
    Limits::check_plausible(total_bytes as u64, body.len(), if h.compression == 0 { 1 } else { 128 })?;
    let mut stored = crate::options::alloc_zeroed::<u8>(total_bytes)?;
    let mut cursor = 0usize;
    for row in stored.chunks_mut(plane_row.max(1)).take(total_rows) {
        match h.compression {
            0 => {
                let src = body.get(cursor..cursor.checked_add(plane_row).ok_or_else(|| bad("BODY overflow"))?).ok_or_else(|| bad("truncated BODY data"))?;
                let n = row.len().min(src.len());
                row[..n].copy_from_slice(&src[..n]);
                cursor += plane_row;
            }
            _ => {
                let tail = body.get(cursor..).ok_or_else(|| bad("truncated BODY data"))?;
                cursor += unpack_byte_run1(tail, row)?;
            }
        }
    }

    let mut out = Vec::new();
    out.try_reserve_exact(w.saturating_mul(hh).saturating_mul(layout.channels())).map_err(|_| CodecError::LimitExceeded("cannot allocate the image".into()))?;
    let planes = usize::from(h.planes);
    for y in 0..hh {
        for x in 0..w {
            let index = if h.chunky {
                usize::from(*stored.get(y * plane_row + x).ok_or_else(|| bad("PBM pixel outside BODY"))?)
            } else {
                (0..planes).filter(|&p| bit_at(&stored, (y * rows_planes + p) * plane_row, x)).fold(0usize, |acc, p| acc | (1 << p))
            };
            let c = palette(index);
            out.extend_from_slice(&c);
            if h.masking != 0 {
                let opaque = if h.masking == 1 { bit_at(&stored, (y * rows_planes + planes) * plane_row, x) } else { index != usize::from(h.transparent) };
                out.push(if opaque { 255 } else { 0 });
            }
        }
    }
    Image::from_u8(h.width, h.height, layout, out)
}

/// The bit for pixel `x` of a stored plane row at `base` (bits run MSB-first).
fn bit_at(stored: &[u8], base: usize, x: usize) -> bool {
    stored.get(base + x / 8).is_some_and(|byte| byte & (0x80 >> (x % 8)) != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `FORM ILBM` with a `BMHD`, `CMAP` and `BODY`. `planes` plane rows per row,
    /// `compression` 0 (stored) or 1 (ByteRun1), MSB-first.
    fn ilbm(w: u16, h: u16, planes: u8, palette: &[[u8; 3]], compression: u8, body: &[u8]) -> Vec<u8> {
        let mut b = b"FORM".to_vec();
        b.extend_from_slice(&0u32.to_be_bytes()); // patched below
        b.extend_from_slice(b"ILBM");
        // BMHD (20 bytes)
        b.extend_from_slice(b"BMHD");
        b.extend_from_slice(&20u32.to_be_bytes());
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[0, 0, 0, 0]); // x/y origin
        b.push(planes);
        b.push(0); // masking
        b.push(compression);
        b.push(0); // pad
        b.extend_from_slice(&[0, 0]); // transparent colour
        b.extend_from_slice(&[1, 1]); // aspect
        b.extend_from_slice(&[0, 0, 0, 0]); // page size
        // CMAP
        b.extend_from_slice(b"CMAP");
        b.extend_from_slice(&((palette.len() * 3) as u32).to_be_bytes());
        for c in palette {
            b.extend_from_slice(c);
        }
        if palette.len() % 2 == 1 {
            b.push(0);
        }
        // BODY
        b.extend_from_slice(b"BODY");
        b.extend_from_slice(&(body.len() as u32).to_be_bytes());
        b.extend_from_slice(body);
        if body.len() % 2 == 1 {
            b.push(0);
        }
        let size = (b.len() - 8) as u32;
        b[4..8].copy_from_slice(&size.to_be_bytes());
        b
    }

    #[test]
    fn reads_a_stored_two_plane_bitmap() {
        // 2×1, 2 planes: pixel 0 index 0 (plane bits 0,0), pixel 1 index 3 (bits 1,1).
        // Plane 0 = 0b0100_0000, plane 1 = 0b0100_0000, each padded to 2 bytes.
        let body = [0b0100_0000, 0, 0b0100_0000, 0];
        let pal = [[10, 20, 30], [40, 50, 60], [70, 80, 90], [100, 110, 120]];
        let b = ilbm(2, 1, 2, &pal, 0, &body);
        assert!(looks_like_iff(&b));
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!((img.width(), img.height(), img.layout()), (2, 1, ChannelLayout::Rgb));
        assert_eq!(img.data(), &[10, 20, 30, 100, 110, 120]);
    }

    #[test]
    fn reads_byte_run1_compressed_rows() {
        // Same image, ByteRun1: literal run of two bytes (0x01) then the two plane bytes each row.
        let body = [0x01, 0b0100_0000, 0, 0x01, 0b0100_0000, 0];
        let pal = [[1, 2, 3], [4, 5, 6], [7, 8, 9], [250, 251, 252]];
        let b = ilbm(2, 1, 2, &pal, 1, &body);
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!(img.data(), &[1, 2, 3, 250, 251, 252]);
    }

    #[test]
    fn no_palette_falls_back_to_a_grey_ramp() {
        // 2×1, 1 plane, no CMAP: indices 0 and 1 → black and white.
        let body = [0b0100_0000, 0];
        let b = ilbm(2, 1, 1, &[], 0, &body);
        let img = decode(&b, &Limits::default()).unwrap();
        assert_eq!(img.data(), &[0, 0, 0, 255, 255, 255]);
    }

    #[test]
    fn pbm_reads_chunky_indices_with_odd_width_padding() {
        let pal = [[10, 20, 30], [40, 50, 60], [70, 80, 90]];
        for (compression, body) in [(0, vec![2, 0, 1, 255, 1, 2, 0, 255]), (1, vec![3, 2, 0, 1, 255, 3, 1, 2, 0, 255])] {
            let mut b = ilbm(3, 2, 8, &pal, compression, &body);
            b[8..12].copy_from_slice(b"PBM ");
            let img = decode(&b, &Limits::default()).unwrap();
            assert_eq!(img.data(), &[70, 80, 90, 10, 20, 30, 40, 50, 60, 40, 50, 60, 70, 80, 90, 10, 20, 30]);
        }
    }

    #[test]
    fn transparency_uses_the_mask_plane_or_transparent_index() {
        for masking in [1, 2] {
            let mut b = ilbm(2, 1, 1, &[[10, 20, 30], [40, 50, 60]], 0, if masking == 1 { &[0x40, 0, 0x40, 0] } else { &[0x40, 0] });
            b[29] = masking;
            let img = decode(&b, &Limits::default()).unwrap();
            assert_eq!(img.layout(), ChannelLayout::Rgba);
            assert_eq!(img.data(), &[10, 20, 30, 0, 40, 50, 60, 255]);
        }
        let mut b = ilbm(2, 1, 8, &[[10, 20, 30], [40, 50, 60]], 0, &[0, 1]);
        b[8..12].copy_from_slice(b"PBM ");
        b[29] = 2;
        b[32..34].copy_from_slice(&1u16.to_be_bytes());
        assert_eq!(decode(&b, &Limits::default()).unwrap().data(), &[10, 20, 30, 255, 40, 50, 60, 0]);
    }

    #[test]
    fn rejects_ham_and_ehb_instead_of_returning_wrong_colours() {
        for mode in [0x80u32, 0x800] {
            let mut b = ilbm(1, 1, 1, &[], 0, &[0, 0]);
            b.extend_from_slice(b"CAMG");
            b.extend_from_slice(&4u32.to_be_bytes());
            b.extend_from_slice(&mode.to_be_bytes());
            let len = (b.len() - 8) as u32;
            b[4..8].copy_from_slice(&len.to_be_bytes());
            assert!(matches!(decode(&b, &Limits::default()), Err(CodecError::Unsupported { .. })));
        }
    }

    #[test]
    fn missing_palette_entries_are_black() {
        let b = ilbm(1, 1, 2, &[[10, 20, 30]], 0, &[0x80, 0, 0x80, 0]);
        assert_eq!(decode(&b, &Limits::default()).unwrap().data(), &[0, 0, 0]);
    }

    #[test]
    fn hostile_input_errors_not_panics() {
        // Truncations of a valid file never panic.
        let body = [0b0100_0000, 0, 0b0100_0000, 0];
        let full = ilbm(2, 1, 2, &[[1, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12]], 0, &body);
        for cut in [0, 4, 8, 12, 40, full.len() - 1] {
            assert!(decode(&full[..cut.min(full.len())], &Limits::default()).is_err(), "cut {cut}");
        }
        // Too many planes is unsupported.
        let bad = ilbm(1, 1, 9, &[], 0, &[0, 0]);
        assert!(matches!(decode(&bad, &Limits::default()), Err(CodecError::Unsupported { .. })));
    }
}
