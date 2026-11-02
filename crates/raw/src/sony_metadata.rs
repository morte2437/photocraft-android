//! Sony's encrypted SR2SubIFD: black levels and the white balance actually applied by the camera.
//!
//! Directory/tag meanings are from ExifTool's public Sony tag documentation. The keystream was
//! measured independently by XOR-ing file bytes with ExifTool 13.59's black-box `-v5` metadata
//! dumps of three ILCE-6000 ARWs (56958-byte blocks, key bytes 11 22 33 44). All 14239 whole
//! words in each block obey k[n] = k[n-63] XOR k[n-127]; the first 127 words agree across the
//! files. This is metadata cipher data, not camera colour calibration or image data. LightCraft's
//! documented investigation suggested the experiment; no sibling or third-party decoder code
//! is used. Other keys remain unreadable rather than being guessed.

use crate::Limits;
use crate::tiff::{Ifd, Tiff};

const PRIVATE_DATA: u16 = 50740;
const OFFSET: u16 = 0x7200;
const LENGTH: u16 = 0x7201;
const KEY: u16 = 0x7221;
const MAX_METADATA: usize = 1 << 20;

// Measured from metadata only. The key's first 127 words, in byte-stream order (big endian).
const INITIAL: [u32; 127] = [
    0x54c2c4d6, 0x49c478bc, 0x34ae2f8e, 0x25d80504, 0xc0d9d6b0, 0xd838fb71, 0xe8eff27d, 0xfbc1fcea, 0x506c499a, 0x47f20f37, 0x710777cf, 0x7867e7ba, 0x42d67caa,
    0x7f2bd11a, 0x67a216ca, 0x0e986d40, 0x4ae8d4c0, 0xe36778b4, 0x5a958415, 0xdbfe2be8, 0x20faa1aa, 0x7132a6b8, 0xf4de4b7f, 0x55991aa1, 0xa849d5aa, 0x49577832,
    0xb92f3daa, 0x399cc526, 0x22cdd000, 0xe1977a29, 0x37c5db55, 0xb0177e1e, 0x2a1016aa, 0xa300086e, 0x3bab9bfe, 0x262eece0, 0x23771aa9, 0x0a5dc91c, 0x31b902ae,
    0x58e64bf8, 0x259c300e, 0xa57705c8, 0x284a6541, 0xfb229c60, 0x1bacaa9e, 0xbcab3350, 0x67cd9fbe, 0x8f135e60, 0xf8c26a40, 0x6770da61, 0x3e1febfd, 0xd0c70803,
    0x8dbb037b, 0x6f6fa4c5, 0x6749d10d, 0x7f51598d, 0xd5e5a4ec, 0x207dfa91, 0x6558ebc2, 0xbe594639, 0x617a9e5d, 0x3c497950, 0x0844eb3f, 0x04207ed2, 0xd27ceac4,
    0x70d20f05, 0xb47003f6, 0xe9e4e3ae, 0xcc19d265, 0x326dd956, 0xf0d3a327, 0xb71275f0, 0x7994e285, 0x0aff594d, 0x128e8345, 0x7bda597a, 0xd634c380, 0xe24a006f,
    0x8974818b, 0x3320b22a, 0xbe808417, 0xa2d5648a, 0x6fe80b39, 0x23ebad41, 0xa2d11e5d, 0x027d9397, 0x9a722ac8, 0x432c7dac, 0x7146692a, 0x82a3dc77, 0xd66887c5,
    0x831f43b7, 0x4e5dddde, 0x03793f81, 0x306ab437, 0x00ccf86c, 0xfc6ed3d2, 0x076b8fdb, 0x9808cfca, 0x0f4eef6e, 0xc8cc3830, 0x104ac16a, 0xa189eff4, 0x3e085c08,
    0xd28baf88, 0x5c853ac4, 0xe60480f8, 0xc51acd98, 0x691e5ee1, 0x333feeb9, 0x1e35bc33, 0xec4a4642, 0xee57c5a5, 0xbeeb51f7, 0xe0c4f32c, 0xa5422f6a, 0x1d266d12,
    0x3752fd3b, 0xfbc53c7d, 0x2421a4a3, 0xcdc6a2de, 0x26e6b330, 0x6c073d46, 0x058e2f27, 0x43833f30, 0x46d1382e, 0x5f0804ec,
];

fn decipher(data: &mut [u8]) {
    let mut history = INITIAL;
    for (n, chunk) in data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let slot = n % 127;
        let word = if n < 127 {
            INITIAL.get(n).copied().unwrap_or(0)
        } else {
            history.get(slot).copied().unwrap_or(0) ^ history.get((n - 63) % 127).copied().unwrap_or(0)
        };
        if let Some(previous) = history.get_mut(slot) {
            *previous = word;
        }
        for (byte, key) in chunk.iter_mut().zip(word.to_be_bytes()) {
            *byte ^= key;
        }
    }
    // Sony leaves an incomplete final word in the clear.
}

#[derive(Default)]
pub(crate) struct Levels {
    pub black: Option<[f64; 4]>,
    pub wb: Option<[f64; 4]>,
}

/// Reads only four SHORT/SSHORT levels from a complete directory. Overflow value offsets are
/// absolute file offsets, but must point inside this decrypted block, never into other file data.
fn four(t: &Tiff, ifd: &Ifd, start: usize, tag: u16) -> Option<[f64; 4]> {
    let mut e = *ifd.get(tag)?;
    if !matches!(e.typ, 3 | 8) || e.count != 4 {
        return None;
    }
    e.at = e.at.checked_sub(start)?;
    t.raw(&e)?;
    let v = [t.float(&e, 0)?, t.float(&e, 1)?, t.float(&e, 2)?, t.float(&e, 3)?];
    v.iter().all(|x| (0.0..=16383.0).contains(x)).then_some(v)
}

fn parse(plain: &[u8], start: usize, le: bool) -> Option<Levels> {
    let t = Tiff { data: plain, le, first_ifd: 0 };
    let count = usize::from(t.u16_at(0)?);
    let end = count.checked_mul(12)?.checked_add(6)?;
    plain.get(..end)?;
    let ifd = t.ifd_at(0, 0)?;
    Some(Levels { black: four(&t, &ifd, start, 0x7310), wb: four(&t, &ifd, start, 0x7313) })
}

pub(crate) fn read(t: &Tiff, limits: &Limits) -> Option<Levels> {
    let ifd0 = t.ifd_at(t.first_ifd, 0)?;
    let pointer = ifd0.get(PRIVATE_DATA)?;
    // BYTE[4]/UNDEFINED[4] containing a file-order offset (not a DNG private payload).
    if !matches!(pointer.typ, 1 | 7) || pointer.count != 4 {
        return None;
    }
    let private = t.ifd_at(t.u32_at(pointer.at)? as usize, 0)?;
    if t.raw(private.get(KEY)?)? != [0x11, 0x22, 0x33, 0x44] {
        return None;
    }
    let start = t.tag_uint(&private, OFFSET)? as usize;
    let len = t.tag_uint(&private, LENGTH)? as usize;
    if !(6..=MAX_METADATA).contains(&len) || len as u64 > limits.max_alloc {
        return None;
    }
    let mut plain = t.bytes(start, len)?.to_vec();
    decipher(&mut plain);
    parse(&plain, start, t.le)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgen::{TiffBuilder, Val};
    use crate::{DevelopOptions, RawFormat, WhiteBalance, decode, develop_sensor};

    /// Procedural CFA TIFF with plain and encrypted levels that deliberately disagree.
    /// No camera file, scene pixels, metadata dump or private fixture is included.
    fn fixture(be: bool, plain: bool, black: [u16; 4], wb: [u16; 4]) -> Vec<u8> {
        let u16_bytes = |v: u16| if be { v.to_be_bytes() } else { v.to_le_bytes() };
        let u32_bytes = |v: u32| if be { v.to_be_bytes() } else { v.to_le_bytes() };
        let mut block = Vec::new();
        block.extend_from_slice(&u16_bytes(2));
        for (tag, typ, offset) in [(0x7310, 3, 2048), (0x7313, 8, 2056)] {
            block.extend_from_slice(&u16_bytes(tag));
            block.extend_from_slice(&u16_bytes(typ));
            block.extend_from_slice(&u32_bytes(4));
            block.extend_from_slice(&u32_bytes(offset)); // replaced with absolute offsets below
        }
        block.extend_from_slice(&[0; 4]);
        block.resize(2048, 0); // crosses multiple 127-word recurrence periods
        for v in black.into_iter().chain(wb) {
            block.extend_from_slice(&u16_bytes(v));
        }
        block.extend_from_slice(&[0xAA, 0xBB]); // tail stored in the clear
        let mut b = TiffBuilder { big_endian: be, ..Default::default() };
        let len = block.len();
        let data = b.blob(block);
        let pixels = b.blob(
            (0..32 * 16)
                .flat_map(|i| {
                    u16_bytes(match (i % 32 % 2, i / 32 % 2) {
                        (0, 0) => 1512,
                        (1, 1) => 2512,
                        _ => 3512,
                    })
                })
                .collect(),
        );
        let mut entries = vec![
            (256, Val::Long(vec![32])),
            (257, Val::Long(vec![16])),
            (258, Val::Short(vec![16])),
            (259, Val::Short(vec![1])),
            (262, Val::Short(vec![32803])),
            (273, Val::Blobs(vec![pixels])),
            (277, Val::Short(vec![1])),
            (278, Val::Long(vec![16])),
            (279, Val::Long(vec![32 * 16 * 2])),
            (33421, Val::Short(vec![2, 2])),
            (33422, Val::Byte(vec![0, 1, 1, 2])),
            (50717, Val::Short(vec![16383])),
        ];
        if plain {
            entries.extend([(0x7310, Val::Short(vec![600; 4])), (0x7313, Val::Short(vec![2048, 1024, 1024, 1536]))]);
        }
        let raw = b.ifd(entries);
        let private = b.ifd(vec![(OFFSET, Val::Blobs(vec![data])), (LENGTH, Val::Long(vec![len as u32])), (KEY, Val::Byte(vec![0x11, 0x22, 0x33, 0x44]))]);
        let root = b.ifd(vec![
            (271, Val::Ascii("SONY".into())),
            (272, Val::Ascii("ILCE-Synthetic".into())),
            (330, Val::Ifds(vec![raw])),
            (PRIVATE_DATA, Val::Ifds(vec![private])),
        ]);
        b.chain = vec![root];
        let mut bytes = b.build();
        let t = Tiff::new(&bytes).unwrap();
        let root = t.ifd_at(t.first_ifd, 0).unwrap();
        let pointer = *root.get(PRIVATE_DATA).unwrap();
        let private = t.ifd_at(t.u32_at(pointer.at).unwrap() as usize, 0).unwrap();
        let start = t.tag_uint(&private, OFFSET).unwrap() as usize;
        // TIFF BYTE[4] offset rather than the generator's LONG[1].
        bytes[pointer.at - 6..pointer.at - 4].copy_from_slice(&u16_bytes(1));
        bytes[pointer.at - 4..pointer.at].copy_from_slice(&u32_bytes(4));
        for (entry, offset) in [(10, 2048), (22, 2056)] {
            bytes[start + entry..start + entry + 4].copy_from_slice(&u32_bytes((start + offset) as u32));
        }
        decipher(&mut bytes[start..start + len]);
        bytes
    }

    #[test]
    fn encrypted_applied_white_balance_and_black_reach_the_developer() {
        for be in [false, true] {
            let bytes = fixture(be, false, [512, 513, 514, 515], [3072, 1024, 1024, 2048]);
            let s = decode(&bytes, &Limits::default()).unwrap();
            assert_eq!(s.format, RawFormat::Arw);
            assert_eq!(s.camera_wb, Some([3.0, 1.0, 2.0]));
            assert_eq!(s.black.values, vec![512.0, 513.0, 514.0, 515.0]);
            let dev = develop_sensor(&s, &DevelopOptions::default()).unwrap();
            assert_eq!(dev.info.wb_multipliers, [3.0, 1.0, 2.0]);
            assert!(!dev.warnings.iter().any(|w| w.contains("grey world") || w.contains("assumed 0")));
            let explicit = develop_sensor(&s, &DevelopOptions { white_balance: WhiteBalance::Multipliers([3.0, 1.0, 2.0]), ..Default::default() }).unwrap();
            assert_eq!(dev.rgb, explicit.rgb);
        }
    }

    #[test]
    fn plain_levels_win_and_invalid_encrypted_levels_are_ignored() {
        let s = decode(&fixture(false, true, [800; 4], [3072, 1024, 1024, 2048]), &Limits::default()).unwrap();
        assert_eq!(s.camera_wb, Some([2.0, 1.0, 1.5]));
        assert_eq!(s.black.values, vec![600.0; 4]);
        let mut broken_plain = fixture(false, true, [800; 4], [3072, 1024, 1024, 2048]);
        let t = Tiff::new(&broken_plain).unwrap();
        let raw = t.all_ifds().into_iter().find(|i| i.has(0x7313)).unwrap();
        let at = raw.get(0x7313).unwrap().at;
        broken_plain[at..at + 8].fill(0);
        let s = decode(&broken_plain, &Limits::default()).unwrap();
        assert_eq!(s.camera_wb, Some([3.0, 1.0, 2.0]));
        assert_eq!(s.black.values, vec![600.0; 4]);
        for wb in [[0, 0, 0, 0], [65535, 1024, 1024, 2048], [30000, 1024, 1024, 2048], [3072, 0, 0, 2048]] {
            let s = decode(&fixture(false, false, [65535; 4], wb), &Limits::default()).unwrap();
            assert_eq!(s.camera_wb, None);
            assert_eq!(s.black.values, vec![0.0]);
        }
    }

    #[test]
    fn hostile_private_metadata_is_bounded_and_optional() {
        let bytes = fixture(false, false, [512; 4], [3072, 1024, 1024, 2048]);
        for n in 0..bytes.len() {
            if let Some(t) = Tiff::new(&bytes[..n]) {
                let _ = read(&t, &Limits::default());
            }
        }
        let t = Tiff::new(&bytes).unwrap();
        assert!(read(&t, &Limits { max_alloc: 2000, ..Default::default() }).is_none());
        let root = t.ifd_at(t.first_ifd, 0).unwrap();
        let private = t.ifd_at(t.u32_at(root.get(PRIVATE_DATA).unwrap().at).unwrap() as usize, 0).unwrap();
        for (at, value) in [(private.get(KEY).unwrap().at, 0), (private.get(LENGTH).unwrap().at, u32::MAX), (private.get(OFFSET).unwrap().at, u32::MAX)] {
            let mut corrupt = bytes.clone();
            corrupt[at..at + 4].copy_from_slice(&value.to_le_bytes());
            assert!(read(&Tiff::new(&corrupt).unwrap(), &Limits::default()).is_none());
            let s = decode(&corrupt, &Limits::default()).unwrap();
            assert_eq!(s.camera_wb, None);
        }
        let start = t.tag_uint(&private, OFFSET).unwrap() as usize;
        let len = t.tag_uint(&private, LENGTH).unwrap() as usize;
        let mut corrupt = bytes.clone();
        decipher(&mut corrupt[start..start + len]);
        // Values before the encrypted block cannot be interpreted as metadata.
        corrupt[start + 22..start + 26].copy_from_slice(&8u32.to_le_bytes());
        decipher(&mut corrupt[start..start + len]);
        assert!(read(&Tiff::new(&corrupt).unwrap(), &Limits::default()).unwrap().wb.is_none());
    }

    #[test]
    fn measured_keystream_recurrence_and_unencrypted_tail() {
        let mut zero = vec![0; 14239 * 4 + 2];
        decipher(&mut zero);
        assert_eq!(&zero[..4], &[0x54, 0xc2, 0xc4, 0xd6]);
        // Words recovered independently from all three oracle dumps, including ring wrap
        // and the final complete word of a real metadata block.
        assert_eq!(&zero[127 * 4..128 * 4], &[0x86, 0xbe, 0x2e, 0x12]);
        assert_eq!(&zero[255 * 4..256 * 4], &[0xc9, 0x1a, 0x54, 0xc4]);
        assert_eq!(&zero[14238 * 4..14239 * 4], &[0xb9, 0xd3, 0x6e, 0xab]);
        assert_eq!(&zero[14239 * 4..], &[0, 0]);
        decipher(&mut zero);
        assert!(zero.iter().all(|&b| b == 0));
    }
}
