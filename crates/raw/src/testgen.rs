//! Synthetic raw-file generators for tests and fuzz seeds: a lossless-JPEG
//! (T.81 process 14) encoder, a small TIFF writer, and DNG / CR2 / TIFF-EP /
//! RW2 / ORF / RAF builders. The files are written from the public specifications only and
//! are meant to exercise the decoder, not to be camera-accurate.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

// ---------------------------------------------------------------- lossless JPEG

mod canon;
mod dng;
mod lj92;
mod orf;
mod raf;
mod rw2;
mod tiff;

pub use canon::{Cr2Spec, craw_block, craw_encode, nef_compressed, sony_craw, tiff_ep};
pub use dng::{DngSpec, DngStorage, baseline_jpeg, gain_map_opcode_list, map_polynomial_opcode_list};
pub use lj92::{lj92_encode, lj92_encode_tables};
pub use orf::orf;
pub use raf::{RafPacking, RafSpec, XTRANS, mosaic, mosaic_cfa, scene};
pub use rw2::{rw2, rw2_format4, rw2_pack, rw2_v4_pack};
pub use tiff::{TiffBuilder, Val};

/// A CR3 container holding one JPEG track and CMT1 orientation. It deliberately
/// contains no CRX data: used to test explicit preview fallback and BMFF bounds.
pub fn cr3_preview(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    fn bx(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut b = ((data.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(data);
        b
    }
    fn moov(jpeg_len: usize, offset: u64, orientation: u16) -> Vec<u8> {
        let mut tiff = TiffBuilder::default();
        let ifd = tiff.ifd(vec![(274, Val::Short(vec![orientation]))]);
        tiff.chain = vec![ifd];
        let mut meta = vec![0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48];
        meta.extend(bx(b"CMT1", &tiff.build()));
        let mut entry = vec![0; 82];
        entry.extend(bx(b"JPEG", &[0; 4]));
        let mut description = vec![0, 0, 0, 0, 0, 0, 0, 1];
        description.extend(bx(b"CRAW", &entry));
        let mut sizes = vec![0; 4];
        sizes.extend_from_slice(&(jpeg_len as u32).to_be_bytes());
        sizes.extend_from_slice(&1u32.to_be_bytes());
        let mut offsets = vec![0, 0, 0, 0, 0, 0, 0, 1];
        offsets.extend_from_slice(&offset.to_be_bytes());
        let mapping: Vec<u8> = [0u32, 1, 1, 1, 1].iter().flat_map(|n| n.to_be_bytes()).collect();
        let table = [bx(b"stsd", &description), bx(b"stsz", &sizes), bx(b"co64", &offsets), bx(b"stsc", &mapping)].concat();
        let track = bx(b"trak", &bx(b"mdia", &bx(b"minf", &bx(b"stbl", &table))));
        bx(b"moov", &[bx(b"uuid", &meta), track].concat())
    }
    let ftyp = bx(b"ftyp", b"crx \0\0\0\x01crx isom");
    let header = moov(jpeg.len(), 0, orientation);
    let offset = ftyp.len() + header.len() + 8;
    [ftyp, moov(jpeg.len(), offset as u64, orientation), bx(b"mdat", jpeg)].concat()
}
