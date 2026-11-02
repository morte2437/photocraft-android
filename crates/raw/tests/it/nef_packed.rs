use photocraft_raw::testgen::{Val, nef_compressed};
use photocraft_raw::{Limits, RawError};

fn packed(values: &[u16], w: usize, bits: u32, padded: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for row in values.chunks_exact(w) {
        let start = out.len();
        let (mut acc, mut have) = (0u32, 0u32);
        for v in row {
            acc |= u32::from(*v) << have;
            have += bits;
            while have >= 8 {
                out.push(acc as u8);
                acc >>= 8;
                have -= 8;
            }
        }
        if have > 0 {
            out.push(acc as u8);
        }
        if padded {
            out.resize(start + (out.len() - start).div_ceil(16) * 16, 0xff);
        }
    }
    out
}

#[test]
fn packed_nef_uses_maker_mode_not_the_shared_tiff_compression() {
    let (w, h) = (22, 5);
    for (bits, mode) in [(12u32, 6u16), (14, 10)] {
        let values: Vec<u16> = (0..w * h).map(|i| ((i * 83 + 11) % (1 << bits)) as u16).collect();
        for padded in [false, true] {
            for big_note in [false, true] {
                let strip = packed(&values, w, bits, padded);
                for modern in [false, true] {
                    let tag = if modern {
                        let mut record = b"01000400\0\0".to_vec();
                        record.extend(if big_note { mode.to_be_bytes() } else { mode.to_le_bytes() });
                        (0x51, Val::Undefined(record))
                    } else {
                        (0x93, Val::Short(vec![mode]))
                    };
                    let bytes = nef_compressed(w, h, bits as u16, strip.clone(), [0, 1, 1, 2], big_note, vec![tag]);
                    let s = photocraft_raw::decode(&bytes, &Limits::default()).unwrap();
                    assert_eq!(s.data, values);
                }
            }
        }
    }
}

#[test]
fn packed_nef_rejects_truncation_wrong_depth_and_unknown_modes() {
    let strip = packed(&[1000; 18 * 5], 18, 14, true);
    for size in 0..strip.len() {
        let bytes = nef_compressed(18, 5, 14, strip[..size].to_vec(), [0, 1, 1, 2], false, vec![(0x93, Val::Short(vec![10]))]);
        // The unpadded size is another valid layout; all other truncations fail.
        if size != (18 * 14usize).div_ceil(8) * 5 {
            assert!(photocraft_raw::decode(&bytes, &Limits::default()).is_err());
        }
    }
    let bytes = nef_compressed(18, 5, 12, strip.clone(), [0, 1, 1, 2], false, vec![(0x93, Val::Short(vec![10]))]);
    assert!(matches!(photocraft_raw::decode(&bytes, &Limits::default()), Err(RawError::Unsupported(_))));
    for mode in [3, 4, 9, 13, 14, 65535] {
        let bytes = nef_compressed(18, 5, 14, strip.clone(), [0, 1, 1, 2], false, vec![(0x93, Val::Short(vec![mode]))]);
        assert!(matches!(photocraft_raw::decode(&bytes, &Limits::default()), Err(RawError::Unsupported(_))));
    }
}

#[test]
fn nikon_crop_area_uses_sensor_coordinates_and_ignores_invalid_rectangles() {
    let strip = packed(&[1000; 22 * 10], 22, 14, true);
    for (rect, expected) in [
        ([2, 2, 18, 6], photocraft_raw::Rect::new(2, 2, 18, 6)),
        ([2, 2, 22, 10], photocraft_raw::Rect::new(0, 0, 22, 10)),
        ([2, 2, 0, 6], photocraft_raw::Rect::new(0, 0, 22, 10)),
        ([u32::MAX, 2, 18, 6], photocraft_raw::Rect::new(0, 0, 22, 10)),
    ] {
        let bytes = nef_compressed(22, 10, 14, strip.clone(), [0, 1, 1, 2], false, vec![(0x93, Val::Short(vec![10])), (0x45, Val::Long(rect.to_vec()))]);
        let sensor = photocraft_raw::decode(&bytes, &Limits::default()).unwrap();
        assert_eq!(sensor.crop, expected);
        let developed = photocraft_raw::develop_sensor(&sensor, &Default::default()).unwrap();
        assert_eq!((developed.width as usize, developed.height as usize), (expected.width, expected.height));
    }
}
