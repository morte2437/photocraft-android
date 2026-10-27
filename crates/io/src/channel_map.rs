//! Alpha / spot channel options <-> PSD `DisplayInfo` image resources (1077, and the older 1007).
//!
//! Layout (Adobe PSD spec, "DisplayInfo"): resource 1077 starts with a 4-byte version (1), then
//! one 13-byte record per extra channel: colour space (u16), four u16 colour components, opacity
//! (u16, 0..100) and kind (u8: 0 = colour indicates selected areas, 1 = masked ("protected")
//! areas, 2 = spot channel). Resource 1007 has the same records without the version and the spot
//! kind, each followed by a padding byte (14 bytes). Photoshop 27's TIFFs use the same 13-byte
//! 1077 records.

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{AlphaChannel, ColorIndicates};

pub(crate) const DISPLAY_INFO: u16 = 1077;
pub(crate) const DISPLAY_INFO_OLD: u16 = 1007;
/// Quick Mask information: channel id (u16) and an "initially empty" flag (u8).
pub(crate) const QUICK_MASK_INFO: u16 = 1022;
/// Alpha channel identifiers (Photoshop 6): one big-endian u32 per extra channel, no count prefix.
pub(crate) const ALPHA_IDENTIFIERS: u16 = 1053;

const SPACE_RGB: u16 = 0;
const SPACE_HSB: u16 = 1;
const SPACE_CMYK: u16 = 2;
const SPACE_LAB: u16 = 7;
const SPACE_GRAY: u16 = 8;

/// Converts one spot channel's plane (native-endian samples) between the document's ink density
/// (1 = solid ink) and the way Photoshop stores a spot channel (0 = solid ink, the maximum =
/// bare paper; every PSD and TIFF Photoshop 27.11 saves stores it so). The conversion is its own
/// inverse, so it serves both open and save. Every reader and writer of Photoshop-polarity spot
/// planes goes through here: a change to the in-memory convention changes only this function.
/// Alpha channels are stored straight (the maximum = selected) and never come here.
pub(crate) fn photoshop_spot_plane(plane: &mut [u8], sample: SampleType) {
    crate::pixels::invert_plane(plane, sample);
}

/// Resource 1053 for `count` channels: one big-endian u32 identifier each, with no length prefix
/// (Photoshop, ag-psd and psd-tools read u32s until the resource ends). Photoshop 27.11 numbers a
/// new document's channels 3, 4, 5, … in every colour mode (RGB, CMYK, Gray and Lab, in its PSDs
/// and TIFFs alike), so these do too. `transparency`: the merged image's transparency channel
/// comes first, with identifier 0 (Photoshop's transparent PSD lists `0, 3, 4, 5`).
pub(crate) fn alpha_identifiers(transparency: bool, count: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(count.saturating_add(1).saturating_mul(4));
    if transparency {
        out.extend_from_slice(&0u32.to_be_bytes());
    }
    for i in 0..count {
        let id = 3u32.saturating_add(u32::try_from(i).unwrap_or(u32::MAX));
        out.extend_from_slice(&id.to_be_bytes());
    }
    out
}

/// Resource 1077 for `channels` (in file order): a version, then 13 bytes per channel (colour
/// space, four components, opacity, kind). Every corpus file, Photoshop's included, uses 13;
/// 14-byte records (the older 1007 layout) shift every channel after the first.
/// `transparency`: the merged image's transparency channel comes first and gets a record too.
pub(crate) fn display_info(channels: &[&AlphaChannel], transparency: bool) -> Vec<u8> {
    let mut out = 1u32.to_be_bytes().to_vec();
    if transparency {
        // As Photoshop writes it: red, 100 %, masked areas.
        out.extend_from_slice(&[0, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 100, 1]);
    }
    for c in channels {
        let (color, opacity, kind) = match c.spot {
            Some((ink, solidity)) => (ink, solidity, 2u8),
            None => (c.color, c.opacity, if c.indicates == ColorIndicates::SelectedAreas { 0 } else { 1 }),
        };
        let rgb = color.to_rgb();
        out.extend_from_slice(&SPACE_RGB.to_be_bytes());
        for v in [rgb[0], rgb[1], rgb[2], 0.0] {
            out.extend_from_slice(&((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes());
        }
        out.extend_from_slice(&((opacity.clamp(0.0, 1.0) * 100.0).round() as u16).to_be_bytes());
        out.push(kind);
    }
    out
}

/// The records of a DisplayInfo resource (`versioned` = 1077: 13-byte records after a version;
/// else the older 1007 with 14-byte records). A trailing partial record is left out.
fn display_records(data: &[u8], versioned: bool) -> std::slice::ChunksExact<'_, u8> {
    let (body, size) = if versioned { (data.get(4..).unwrap_or(&[]), 13) } else { (data, 14) };
    body.chunks_exact(size)
}

/// Which of the extra channels are spot channels (kind 2), after skipping `skip` records (the
/// merged transparency channel's), in the same records [`apply_display_info`] reads. A channel
/// without a record is not a spot.
pub(crate) fn spot_flags(data: &[u8], versioned: bool, skip: usize) -> Vec<bool> {
    display_records(data, versioned).skip(skip).map(|rec| rec.get(12) == Some(&2)).collect()
}

/// Whether a DisplayInfo resource 1077 marks any channel as a spot (kind 2), in 13-byte records
/// (Photoshop's, and PhotoCraft's since 0.8.2) or in the 14-byte padded records PhotoCraft wrote
/// up to 0.7.0.
pub(crate) fn display_info_marks_spot(data: &[u8]) -> bool {
    let body = data.get(4..).unwrap_or_default();
    [13, 14].into_iter().any(|size| body.chunks_exact(size).any(|rec| rec.get(12) == Some(&2)))
}

/// Applies a DisplayInfo resource (`versioned` = 1077: 13-byte records after a version; else the
/// older 1007 with 14-byte records) to the imported channels in order, after skipping `skip`
/// records (the merged transparency channel's).
pub(crate) fn apply_display_info(data: &[u8], versioned: bool, skip: usize, channels: &mut [AlphaChannel]) {
    for (c, rec) in channels.iter_mut().zip(display_records(data, versioned).skip(skip)) {
        let u = |i: usize| u16::from_be_bytes([rec[i], rec[i + 1]]);
        let comps = [u(2), u(4), u(6), u(8)];
        let color = decode_color(u(0), comps);
        let opacity = f32::from(u(10).min(100)) / 100.0;
        match rec[12] {
            2 => c.spot = Some((color, opacity)),
            k => {
                // An alpha channel, not an ink (a Multichannel import starts every channel as one).
                c.spot = None;
                c.color = color;
                c.opacity = opacity;
                c.indicates = if k == 0 { ColorIndicates::SelectedAreas } else { ColorIndicates::MaskedAreas };
            }
        }
    }
}

/// A PSD colour structure (space id + four u16 components) as an RGB [`Color`]; unknown spaces
/// (book colours…) read the first three components as RGB. See [`decode_known_color`].
pub(crate) fn decode_color(space: u16, c: [u16; 4]) -> Color {
    let n = |v: u16| f32::from(v) / 65535.0;
    decode_known_color(space, c).unwrap_or_else(|| Color::rgb(n(c[0]), n(c[1]), n(c[2])))
}

/// A PSD colour structure in one of the spaces Photoshop writes for plain colours (0 RGB,
/// 1 HSB, 2 CMYK, 7 Lab, 8 Grayscale) as an RGB [`Color`], or `None` for any other space id.
/// CMYK goes through the colour-managed conversion (the active document CMYK profile, see
/// `photocraft_color::convert::with_cmyk_space`), Lab through the D50 Lab → sRGB conversion.
pub(crate) fn decode_known_color(space: u16, c: [u16; 4]) -> Option<Color> {
    let n = |v: u16| f32::from(v) / 65535.0;
    Some(match space {
        SPACE_RGB => Color::rgb(n(c[0]), n(c[1]), n(c[2])),
        SPACE_HSB => {
            let rgb = hsb_to_rgb(n(c[0]) * 360.0, n(c[1]), n(c[2]));
            Color::rgb(rgb[0], rgb[1], rgb[2])
        }
        // Photoshop colour structures store CMYK inverted (65535 = no ink).
        SPACE_CMYK => {
            let cmyk = Color { mode: ColorMode::Cmyk, c: [1.0 - n(c[0]), 1.0 - n(c[1]), 1.0 - n(c[2]), 1.0 - n(c[3])], alpha: 1.0 };
            let rgb = cmyk.to_rgb();
            Color::rgb(rgb[0], rgb[1], rgb[2])
        }
        SPACE_LAB => {
            let l = f32::from(c[0].min(10000)) / 10000.0;
            let a = f32::from(c[1] as i16) / 100.0;
            let b = f32::from(c[2] as i16) / 100.0;
            let lab = Color { mode: ColorMode::Lab, c: [l, (a + 128.0) / 255.0, (b + 128.0) / 255.0, 0.0], alpha: 1.0 };
            let rgb = lab.to_rgb();
            Color::rgb(rgb[0], rgb[1], rgb[2])
        }
        SPACE_GRAY => {
            let v = 1.0 - f32::from(c[0].min(10000)) / 10000.0;
            Color::rgb(v, v, v)
        }
        _ => return None,
    })
}

/// HSB (hue in degrees, saturation and brightness 0..1) → RGB.
pub(crate) fn hsb_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = if h.is_finite() { h.rem_euclid(360.0) / 60.0 } else { 0.0 };
    let (s, v) = (s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    let f = h - h.floor();
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match h.floor() as u8 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;
    use photocraft_raster::Surface;

    #[test]
    fn display_info_roundtrip() {
        let mut a = AlphaChannel::new("A", Surface::new(PixelFormat::GRAY8));
        a.color = Color::rgb(0.0, 1.0, 0.0);
        a.opacity = 0.3;
        a.indicates = ColorIndicates::SelectedAreas;
        let mut s = AlphaChannel::new("S", Surface::new(PixelFormat::GRAY8));
        s.spot = Some((Color::rgb(1.0, 0.0, 1.0), 0.8));
        let bytes = display_info(&[&a, &s], false);
        // 13 bytes per channel, as every corpus file (Photoshop's included) lays out 1077.
        assert_eq!(bytes.len(), 4 + 2 * 13);
        let mut back = vec![AlphaChannel::new("A", Surface::new(PixelFormat::GRAY8)), AlphaChannel::new("S", Surface::new(PixelFormat::GRAY8))];
        apply_display_info(&bytes, true, 0, &mut back);
        assert_eq!(back[0].color, a.color);
        assert!((back[0].opacity - 0.3).abs() < 1e-6);
        assert_eq!(back[0].indicates, ColorIndicates::SelectedAreas);
        assert_eq!(back[1].spot, s.spot);
        assert_eq!(spot_flags(&bytes, true, 0), [false, true]);
        assert!(display_info_marks_spot(&bytes));
        assert!(!display_info_marks_spot(&display_info(&[&a], true)), "alpha and transparency records only");
        assert!(!display_info_marks_spot(&bytes[..10]));
        // Photoshop 27 writes a 13-byte record (no pad): red spot, solidity 60.
        let ps = [0, 0, 0, 1, 0, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0x3c, 2];
        let mut spot = vec![AlphaChannel::new("S", Surface::new(PixelFormat::GRAY8))];
        apply_display_info(&ps, true, 0, &mut spot);
        assert_eq!(spot[0].spot.map(|(c, s)| (c.to_rgb(), (s * 100.0).round() as u16)), Some(([1.0, 0.0, 0.0], 60)));
        // With the merged transparency's record first, the channels still get their own.
        let with_t = display_info(&[&a, &s], true);
        assert_eq!(with_t.len(), 4 + 3 * 13);
        let mut back = vec![AlphaChannel::new("A", Surface::new(PixelFormat::GRAY8)), AlphaChannel::new("S", Surface::new(PixelFormat::GRAY8))];
        apply_display_info(&with_t, true, 1, &mut back);
        assert_eq!(back[0].indicates, ColorIndicates::SelectedAreas);
        assert_eq!(back[1].spot, s.spot);
        assert_eq!(spot_flags(&with_t, true, 1), [false, true]);
        // Truncated data is ignored rather than misread.
        let mut c = vec![AlphaChannel::new("x", Surface::new(PixelFormat::GRAY8))];
        apply_display_info(&bytes[..10], true, 0, &mut c);
        assert_eq!(c[0].opacity, 0.5);
        assert!(spot_flags(&bytes[..10], true, 0).is_empty());
        assert!(spot_flags(&bytes, true, 5).is_empty());

        // 14 unpadded records are a multiple of 14 as well (14×13 = 13×14). Resource 1077
        // still reads them as 13-byte records; the last spot's solidity must survive.
        let mut many = Vec::new();
        for i in 0..14 {
            let mut ch = AlphaChannel::new(format!("{i}"), Surface::new(PixelFormat::GRAY8));
            ch.spot = Some((Color::rgb(1.0, 0.0, 0.0), (i as f32) / 14.0));
            many.push(ch);
        }
        let refs: Vec<&AlphaChannel> = many.iter().collect();
        let packed = display_info(&refs, false);
        assert_eq!(packed.len(), 4 + 14 * 13);
        let mut got: Vec<AlphaChannel> = (0..14).map(|i| AlphaChannel::new(format!("{i}"), Surface::new(PixelFormat::GRAY8))).collect();
        apply_display_info(&packed, true, 0, &mut got);
        assert_eq!(got[13].spot.map(|(_, s)| (s * 100.0).round() as u16), many[13].spot.map(|(_, s)| (s * 100.0).round() as u16));
    }

    /// Resource 1053 as Photoshop 27.11 numbers a new document's channels: 3, 4, 5 in every colour
    /// mode, with the merged transparency first as 0 (its transparent PSD lists `0, 3, 4, 5`).
    #[test]
    fn alpha_identifiers_are_numbered_as_photoshop_does() {
        let ids = |transparency, count| alpha_identifiers(transparency, count).as_chunks::<4>().0.iter().map(|c| u32::from_be_bytes(*c)).collect::<Vec<_>>();
        assert_eq!(ids(false, 3), [3, 4, 5]);
        assert_eq!(ids(true, 3), [0, 3, 4, 5]);
        assert_eq!(ids(false, 1), [3]);
        assert!(ids(false, 0).is_empty());
        assert_eq!(ids(true, 0), [0]);
        assert_eq!(alpha_identifiers(false, 2), [0, 0, 0, 3, 0, 0, 0, 4]);
    }

    /// PhotoCraft 0.1.0 to 0.7.0 padded each 1077 record to 14 bytes, so a spot after the first
    /// record sits where a 13-byte reader finds no kind 2; it is still recognised.
    #[test]
    fn a_spot_is_found_in_padded_records() {
        let alpha = |kind: u8| [0, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 50, kind, 0];
        let mut old = 1u32.to_be_bytes().to_vec();
        old.extend_from_slice(&alpha(1));
        old.extend_from_slice(&alpha(0));
        old.extend_from_slice(&alpha(2));
        assert_eq!(spot_flags(&old, true, 0), [false, false, false], "13-byte records misread the padded ones");
        assert!(display_info_marks_spot(&old));
        old.truncate(old.len() - 14);
        assert!(!display_info_marks_spot(&old), "two alpha records");
    }

    /// Photoshop's spot ink (0 = solid) and the document's density (1 = solid) at every depth,
    /// both ways: the 25 % / 75 % columns of the Photoshop oracles store 191 / 64.
    #[test]
    fn photoshop_spot_plane_converts_both_ways() {
        let mut p8 = vec![255u8, 64, 191, 0];
        photoshop_spot_plane(&mut p8, SampleType::U8);
        assert_eq!(p8, [0, 191, 64, 255]);
        photoshop_spot_plane(&mut p8, SampleType::U8);
        assert_eq!(p8, [255, 64, 191, 0]);
        let mut p16: Vec<u8> = [65535u16, 16448, 0].iter().flat_map(|v| v.to_ne_bytes()).collect();
        photoshop_spot_plane(&mut p16, SampleType::U16);
        let back: Vec<u16> = p16.as_chunks::<2>().0.iter().map(|c| u16::from_ne_bytes(*c)).collect();
        assert_eq!(back, [0, 49087, 65535]);
        let mut p32: Vec<u8> = [1.0f32, 0.25].iter().flat_map(|v| v.to_ne_bytes()).collect();
        photoshop_spot_plane(&mut p32, SampleType::F32);
        let back: Vec<f32> = p32.as_chunks::<4>().0.iter().map(|c| f32::from_ne_bytes(*c)).collect();
        assert_eq!(back, [0.0, 0.75]);
        // A trailing partial sample is left alone rather than misread.
        let mut odd = vec![0u8, 0, 7];
        photoshop_spot_plane(&mut odd, SampleType::U16);
        assert_eq!(odd, [0xff, 0xff, 7]);
    }

    #[test]
    fn decodes_other_spaces() {
        let g = decode_color(SPACE_GRAY, [10000, 0, 0, 0]);
        assert_eq!(g.to_rgb(), [0.0; 3]);
        let lab = decode_color(SPACE_LAB, [10000, 0, 0, 0]).to_rgb();
        assert!(lab.iter().all(|v| *v > 0.95), "{lab:?}");
        // HSB: hue 120 degrees (65535 / 3), full saturation and brightness is pure green.
        let g = decode_color(SPACE_HSB, [21845, 65535, 65535, 0]).to_rgb();
        assert!((g[0]).abs() < 1e-3 && (g[1] - 1.0).abs() < 1e-3 && g[2].abs() < 1e-3, "{g:?}");
        let half = decode_color(SPACE_HSB, [0, 0, 32768, 0]).to_rgb();
        assert!(half.iter().all(|v| (v - 0.5).abs() < 1e-3), "{half:?}");
        // Unknown spaces (book colours) are not guessed.
        assert_eq!(decode_known_color(3, [1, 2, 3, 4]), None);
        assert_eq!(decode_known_color(SPACE_RGB, [65535, 0, 0, 0]), Some(Color::rgb(1.0, 0.0, 0.0)));
    }
}
