//! Duotone inks ⇄ a PSD's Color Mode Data in Duotone mode (colour mode 8).
//!
//! The PSD specification leaves this block undocumented. Photoshop writes 524 bytes (observed on
//! files it made with 1–4 inks):
//!
//! | offset | size | content |
//! |---|---|---|
//! | 0 | 2 | version, 1 |
//! | 2 | 2 | number of inks, 1–4 |
//! | 4 | 4 × 10 | ink colours: colour space + 4 × u16, as in swatch files; space 0xFFFF = no ink |
//! | 44 | 4 × 64 | ink names, Pascal strings |
//! | 300 | 4 × 28 | ink curves: 13 × i16 then an i16 (0), the PSD's transfer-function layout |
//! | 412 | 2 | dot gain, 20 |
//! | 414 | 11 × 10 | overprint colours, space 0xFFFF = computed |
//!
//! A curve's 13 values are the ink's density in tenths of a percent at gray densities of 0, 5,
//! 10, 20, …, 90, 95 and 100 %; −1 leaves a point out (Photoshop's empty dialog fields), except
//! at 0 and 100 %: Photoshop refuses to open a file with −1 there.
//!
//! A colour-book ink (PANTONE, …) is stored by its book key, not a colour. Photoshop also writes
//! every ink's Lab equivalent in image resource 1066 (Alternate Duotone Colors): version 1, a
//! count, then one 10-byte colour per ink in the same order.

use photocraft_doc::adjust::CurvePoint;
use photocraft_doc::{Duotone, DuotoneInk};
use photocraft_psd::aco::AcoColor;

/// Length of the block Photoshop writes.
pub(crate) const BLOCK_LEN: usize = 524;
/// Gray densities (%) of a curve's 13 points.
const CURVE_AT: [u16; 13] = [0, 5, 10, 20, 30, 40, 50, 60, 70, 80, 90, 95, 100];
const NO_COLOR: [u8; 10] = [0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
const NAMES_AT: usize = 44;
const CURVES_AT: usize = 300;
const DOT_GAIN_AT: usize = 412;
/// Image resource: the inks' Lab equivalents.
pub(crate) const ALTERNATE_DUOTONE_COLORS: u16 = 1066;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|s| u16::from_be_bytes([s[0], s[1]]))
}

/// The sRGB display colour of the 10-byte colour structure at `at`; `None` for a colour book or
/// another space we can't show.
fn display(b: &[u8], at: usize) -> Option<[f32; 3]> {
    let space = u16_at(b, at)?;
    let values = [u16_at(b, at + 2)?, u16_at(b, at + 4)?, u16_at(b, at + 6)?, u16_at(b, at + 8)?];
    let c = crate::channel_map::decode_known_color(space, values)?;
    Some(c.to_rgb().map(|v| v.clamp(0.0, 1.0)))
}

/// The inks of a Duotone block, or `None` when it is not one this reads (the caller keeps it raw).
/// `alternate` is resource 1066, which gives a colour-book ink its colour. `unknown` is set when
/// an ink's colour can't be shown even so; it is then black here, and the caller shows the
/// grey plate instead.
pub(crate) fn parse(b: &[u8], alternate: Option<&[u8]>, unknown: &mut bool) -> Option<Vec<DuotoneInk>> {
    if b.len() < DOT_GAIN_AT || u16_at(b, 0)? != 1 {
        return None;
    }
    let n = usize::from(u16_at(b, 2)?);
    if !(1..=4).contains(&n) {
        return None;
    }
    (0..n)
        .map(|i| {
            let alt = alternate.filter(|a| u16_at(a, 0) == Some(1) && u16_at(a, 2).is_some_and(|k| usize::from(k) > i));
            let color = display(b, 4 + 10 * i).or_else(|| display(alt?, 4 + 10 * i)).unwrap_or_else(|| {
                *unknown = true;
                [0.0; 3]
            });
            let name_at = NAMES_AT + 64 * i;
            let len = usize::from(*b.get(name_at)?).min(63);
            let name = String::from_utf8_lossy(b.get(name_at + 1..name_at + 1 + len)?).into_owned();
            let curve_at = CURVES_AT + 28 * i;
            let mut curve: Vec<CurvePoint> = Vec::new();
            for (k, x) in CURVE_AT.iter().enumerate() {
                let v = u16_at(b, curve_at + 2 * k)? as i16;
                if v >= 0 {
                    curve.push(CurvePoint { input: f32::from(*x) / 100.0, output: (f32::from(v) / 1000.0).clamp(0.0, 1.0) });
                }
            }
            if curve.len() < 2 {
                curve = DuotoneInk::new("", [0.0; 3]).curve;
            }
            Some(DuotoneInk { name, color, curve })
        })
        .collect()
}

/// The Duotone block for `d`'s first four inks: RGB ink colours, their names and their curves
/// sampled at the 13 gray densities (the points on that grid as they are, the others left out),
/// Photoshop's default dot gain and computed overprint colours.
pub(crate) fn write(d: &Duotone) -> Vec<u8> {
    let mut b = Vec::with_capacity(BLOCK_LEN);
    let inks: Vec<&DuotoneInk> = d.inks.iter().take(4).collect();
    b.extend_from_slice(&1u16.to_be_bytes());
    b.extend_from_slice(&(inks.len().max(1) as u16).to_be_bytes());
    for i in 0..4 {
        match inks.get(i) {
            Some(ink) => {
                let q = |v: f32| (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
                let (space, v) = AcoColor::Rgb(ink.color.map(q)).to_record();
                b.extend_from_slice(&space.to_be_bytes());
                v.iter().for_each(|x| b.extend_from_slice(&x.to_be_bytes()));
            }
            None => b.extend_from_slice(&NO_COLOR),
        }
    }
    for i in 0..4 {
        let mut name = [0u8; 64];
        if let Some(ink) = inks.get(i) {
            let bytes: Vec<u8> = ink.name.chars().map(|c| if c.is_ascii() { c as u8 } else { b'?' }).take(63).collect();
            name[0] = bytes.len() as u8;
            name[1..=bytes.len()].copy_from_slice(&bytes);
        }
        b.extend_from_slice(&name);
    }
    for i in 0..4 {
        let values = curve_values(inks.get(i).map(|ink| ink.curve.as_slice()));
        values.iter().for_each(|v| b.extend_from_slice(&v.to_be_bytes()));
        b.extend_from_slice(&0i16.to_be_bytes());
    }
    b.extend_from_slice(&20u16.to_be_bytes());
    for _ in 0..11 {
        b.extend_from_slice(&NO_COLOR);
    }
    b
}

/// A curve's 13 values: an identity curve (or none) is 0 and 100 % only, as Photoshop writes it;
/// a curve whose points all lie on the 13 gray densities keeps exactly those points; any other
/// curve is sampled at all 13.
fn curve_values(curve: Option<&[CurvePoint]>) -> [i16; 13] {
    let mut out = [-1i16; 13];
    let pts = curve.filter(|c| c.len() >= 2);
    let identity = pts.is_none_or(|c| c.iter().all(|p| (p.input - p.output).abs() < 1e-4));
    let per_mille = |v: f32| (v.clamp(0.0, 1.0) * 1000.0).round() as i16;
    if identity {
        out[0] = 0;
        out[12] = 1000;
        return out;
    }
    let pts = pts.unwrap_or_default();
    let slot = |x: f32| CURVE_AT.iter().position(|a| (f32::from(*a) / 100.0 - x).abs() < 1e-4);
    let ink = DuotoneInk { name: String::new(), color: [0.0; 3], curve: pts.to_vec() };
    if pts.iter().all(|p| slot(p.input).is_some()) {
        for p in pts {
            if let Some(k) = slot(p.input) {
                out[k] = per_mille(p.output);
            }
        }
        // Photoshop needs both ends.
        for k in [0, 12] {
            if out[k] < 0 {
                out[k] = per_mille(ink.density(f32::from(CURVE_AT[k]) / 100.0));
            }
        }
        return out;
    }
    for (k, x) in CURVE_AT.iter().enumerate() {
        out[k] = per_mille(ink.density(f32::from(*x) / 100.0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ink(name: &str, color: [f32; 3], curve: &[(f32, f32)]) -> DuotoneInk {
        let curve =
            if curve.is_empty() { DuotoneInk::new("", [0.0; 3]).curve } else { curve.iter().map(|&(input, output)| CurvePoint { input, output }).collect() };
        DuotoneInk { name: name.into(), color, curve }
    }

    #[test]
    fn writes_the_block_as_photoshop_does() {
        // Black and orange (255, 128, 0) with plain curves: the block Photoshop 2025 wrote for the
        // same inks, field by field.
        let d = Duotone { inks: vec![ink("Black", [0.0; 3], &[]), ink("Orange", [1.0, 32895.0 / 65535.0, 0.0], &[])], psd_raw: None };
        let b = write(&d);
        assert_eq!(b.len(), BLOCK_LEN);
        assert_eq!(&b[..4], &[0, 1, 0, 2]);
        assert_eq!(&b[4..14], &[0; 10], "black, RGB");
        assert_eq!(&b[14..24], &[0, 0, 0xFF, 0xFF, 0x80, 0x7F, 0, 0, 0, 0], "orange, RGB");
        assert_eq!(&b[24..44], &[NO_COLOR, NO_COLOR].concat(), "no third and fourth ink");
        assert_eq!(&b[44..50], b"\x05Black");
        assert_eq!(&b[108..115], b"\x06Orange");
        let identity: Vec<u8> = [0i16, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 1000, 0].iter().flat_map(|v| v.to_be_bytes()).collect();
        for i in 0..4 {
            assert_eq!(&b[300 + 28 * i..328 + 28 * i], &identity[..], "curve {i}");
        }
        assert_eq!(&b[412..414], &[0, 20], "dot gain");
        assert!(b[414..].chunks(10).all(|c| c == NO_COLOR), "overprint colours computed");
    }

    #[test]
    fn inks_round_trip() {
        let d = Duotone {
            inks: vec![
                ink("Black", [0.0; 3], &[(0.0, 0.0), (0.6, 0.0), (1.0, 1.0)]),
                ink("Teal", [0.0, 0.5, 0.5], &[(0.0, 0.1), (0.05, 0.2), (0.95, 0.9), (1.0, 0.95)]),
                ink("Tan", [0.8, 0.7, 0.5], &[]),
            ],
            psd_raw: None,
        };
        let mut unknown = false;
        let back = parse(&write(&d), None, &mut unknown).unwrap();
        assert!(!unknown);
        assert_eq!(back.len(), 3);
        for (a, b) in d.inks.iter().zip(&back) {
            assert_eq!(a.name, b.name);
            assert!(a.color.iter().zip(b.color).all(|(x, y)| (x - y).abs() < 1e-4), "{a:?} {b:?}");
            assert_eq!(a.curve.len(), b.curve.len(), "{}", a.name);
            for (p, q) in a.curve.iter().zip(&b.curve) {
                assert!((p.input - q.input).abs() < 1e-6 && (p.output - q.output).abs() < 1e-3, "{}: {p:?} {q:?}", a.name);
            }
        }
    }

    #[test]
    fn a_curve_off_the_grid_is_sampled_at_all_13_points() {
        let curve = [CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 0.33, output: 0.66 }, CurvePoint { input: 1.0, output: 1.0 }];
        let v = curve_values(Some(&curve));
        assert!(v.iter().all(|&x| x >= 0), "{v:?}");
        assert_eq!((v[0], v[6], v[12]), (0, 746, 1000), "{v:?}");
    }

    /// Black (CMYK) and PANTONE 327 CVC as Photoshop writes them (psd-tools'
    /// `colormodes/4x4_8bit_duotone.psd`): the book ink is stored by its key, "  327C".
    fn black_and_pantone() -> Vec<u8> {
        let mut b = write(&Duotone { inks: vec![ink("Black", [0.0; 3], &[]), ink("PANTONE 327 CVC", [0.0; 3], &[])], psd_raw: None });
        b[4..14].copy_from_slice(&[0, 2, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0]);
        b[14..24].copy_from_slice(&[0, 3, 0x20, 0x20, 0x33, 0x32, 0x37, 0x43, 0, 0]);
        b
    }

    #[test]
    fn a_colour_book_ink_without_its_lab_equivalent_is_unknown() {
        let mut unknown = false;
        let inks = parse(&black_and_pantone(), None, &mut unknown).unwrap();
        assert!(unknown);
        assert_eq!((inks[1].name.as_str(), inks[1].color), ("PANTONE 327 CVC", [0.0; 3]));
        assert!(inks[0].color.iter().all(|&c| c < 0.2), "CMYK black: {:?}", inks[0].color);
    }

    #[test]
    fn a_colour_book_ink_takes_its_colour_from_resource_1066() {
        // Photoshop's resource 1066 of the same file: Lab (9.53, 4.98, 4.25) and (45.49, -66, -5).
        let alternate = [0, 1, 0, 2, 0, 7, 0x03, 0xB9, 0x01, 0xF2, 0x01, 0xA9, 0, 0, 0, 7, 0x11, 0xC5, 0xE6, 0x38, 0xFE, 0x0C, 0, 0, 1, 0];
        let mut unknown = false;
        let inks = parse(&black_and_pantone(), Some(&alternate), &mut unknown).unwrap();
        assert!(!unknown);
        let [r, g, b] = inks[1].color;
        assert!(r < 0.1 && g > 0.4 && b > 0.35 && g > b, "PANTONE 327 is teal: {:?}", inks[1].color);
        // A short or foreign resource gives nothing.
        let mut unknown = false;
        parse(&black_and_pantone(), Some(&alternate[..14]), &mut unknown).unwrap();
        assert!(unknown);
    }

    #[test]
    fn an_hsb_ink_is_shown() {
        let mut b = write(&Duotone { inks: vec![ink("Green", [0.0; 3], &[])], psd_raw: None });
        // Hue 120°, full saturation and brightness.
        b[4..14].copy_from_slice(&[0, 1, 0x55, 0x55, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0]);
        let mut unknown = false;
        let inks = parse(&b, None, &mut unknown).unwrap();
        assert!(!unknown);
        assert!(inks[0].color[1] > 0.99 && inks[0].color[0] < 0.01 && inks[0].color[2] < 0.01, "{:?}", inks[0].color);
    }

    #[test]
    fn a_curve_always_has_both_ends() {
        // On the grid, but without a 0 % or a 100 % point: Photoshop won't open −1 there.
        let curve = [CurvePoint { input: 0.2, output: 0.1 }, CurvePoint { input: 0.5, output: 0.7 }];
        let v = curve_values(Some(&curve));
        assert_eq!(v, [100, -1, -1, 100, -1, -1, 700, -1, -1, -1, -1, -1, 700]);
    }

    #[test]
    fn malformed_blocks_are_not_read() {
        let mut unknown = false;
        let good = write(&Duotone { inks: vec![ink("Black", [0.0; 3], &[])], psd_raw: None });
        assert!(parse(&good[..100], None, &mut unknown).is_none());
        let mut v = good.clone();
        v[1] = 2;
        assert!(parse(&v, None, &mut unknown).is_none(), "version");
        let mut n = good;
        n[3] = 5;
        assert!(parse(&n, None, &mut unknown).is_none(), "ink count");
    }
}
