use photocraft_doc::adjust::{CurvePoint, HueRange, LevelsChannel, LutFormat, LutSource, ToneSpace, curve_map_outputs};
use photocraft_psd::descriptor::{Descriptor, Id, UnicodeString, Value, VersionedDescriptor};

use super::{BW_KEYS, BW_TINT, clamp_i16, cube_file_name, levels_write, lut_source_for, psd_lut_format, put16, q255, rgb_desc};

/// The values as big-endian 16-bit words.
pub(super) fn words(xs: &[u16]) -> Vec<u8> {
    let mut v = Vec::new();
    for &x in xs {
        put16(&mut v, x);
    }
    v
}

/// Appends `x` as a clamped big-endian 16-bit integer.
fn push_i16(v: &mut Vec<u8>, x: f32) {
    v.extend_from_slice(&clamp_i16(x).to_be_bytes());
}

/// The legacy Brightness/Contrast block and its descriptor block.
pub(super) fn brightness_contrast(brightness: f32, contrast: f32, legacy: bool) -> Vec<([u8; 4], Vec<u8>)> {
    let mut v = Vec::new();
    push_i16(&mut v, brightness);
    push_i16(&mut v, contrast);
    put16(&mut v, 127);
    v.push(0);
    let d = Descriptor::new("null")
        .with("Vrsn", Value::Integer(1))
        .with("Brgh", Value::Integer(brightness.round() as i32))
        .with("Cntr", Value::Integer(contrast.round() as i32))
        .with("means", Value::Integer(127))
        .with("Lab ", Value::Boolean(false))
        .with("useLegacy", Value::Boolean(legacy))
        .with("Auto", Value::Boolean(false));
    vec![(*b"brit", v), (*b"CgEd", VersionedDescriptor::new(d).to_bytes())]
}

/// Hue/Saturation payload: the master values, then the six hue ranges.
pub(super) fn hue_saturation(hsl: [f32; 3], colorize: bool, ranges: &[HueRange; 6]) -> Vec<u8> {
    let mut v = words(&[2]);
    v.push(u8::from(colorize));
    v.push(0);
    // The colorize triplet, then the master triplet. The inactive one gets Photoshop's
    // defaults; the colorize hue (0..360 in the model) is stored signed, -180..180.
    let [hue, saturation, lightness] = hsl;
    let (colorize_hsl, master) = if colorize {
        let h = hue.rem_euclid(360.0);
        ([if h > 180.0 { h - 360.0 } else { h }, saturation, lightness], [0.0; 3])
    } else {
        ([30.0, 25.0, 0.0], hsl)
    };
    for x in colorize_hsl.into_iter().chain(master) {
        push_i16(&mut v, x);
    }
    // Six hue ranges: four bounds in 0..360, then hue, saturation and lightness.
    for r in ranges {
        for x in r.bounds {
            put16(&mut v, x.rem_euclid(360.0).round() as u16 % 360);
        }
        for x in [r.hue, r.saturation, r.lightness] {
            push_i16(&mut v, x);
        }
    }
    v
}

/// Exposure payload.
pub(super) fn exposure(exposure: f32, offset: f32, gamma: f32) -> Vec<u8> {
    let mut v = words(&[1]);
    for x in [exposure, offset, gamma] {
        v.extend_from_slice(&x.to_be_bytes());
    }
    v.push(1); // color space flag (spec: "1 byte")
    v
}

/// Levels payload: master, the three colour channels, black (CMYK only), then identity records.
pub(super) fn levels(master: &LevelsChannel, per_channel: &[LevelsChannel; 3], space: ToneSpace, black: &LevelsChannel) -> Vec<u8> {
    let mut v = words(&[2]);
    levels_write(&mut v, master);
    for c in per_channel {
        levels_write(&mut v, c);
    }
    let ident = LevelsChannel::default();
    levels_write(&mut v, if space == ToneSpace::Cmyk { black } else { &ident });
    for _ in 5..29 {
        levels_write(&mut v, &LevelsChannel::default());
    }
    v
}

/// Curves payload: each channel's points, or its 256-entry map in pencil mode.
pub(super) fn curves(master: &[CurvePoint], per_channel: &[Vec<CurvePoint>; 3], space: ToneSpace, black: &[CurvePoint], pencil: bool) -> Vec<u8> {
    let mut v = vec![u8::from(pencil)];
    put16(&mut v, 1);
    let ink = space == ToneSpace::Cmyk && black.len() >= 2;
    v.extend_from_slice(&(if ink { 0b11111u32 } else { 0b1111 }).to_be_bytes());
    for c in std::iter::once(master).chain(per_channel.iter().map(Vec::as_slice)).chain(ink.then_some(black)) {
        if pencil {
            v.extend(curve_map_outputs(c).iter().map(|&o| u8::try_from(q255(o)).unwrap_or(u8::MAX)));
            continue;
        }
        put16(&mut v, c.len().min(19) as u16);
        for p in c.iter().take(19) {
            put16(&mut v, q255(p.output));
            put16(&mut v, q255(p.input));
        }
    }
    v
}

/// Selective Color payload, with each value clamped to ±100%.
pub(super) fn selective_color(relative: bool, adjustments: &[[f32; 4]; 9]) -> Vec<u8> {
    let mut v = words(&[1, u16::from(!relative)]);
    v.extend_from_slice(&[0; 8]);
    for rec in adjustments {
        for x in rec {
            push_i16(&mut v, x.clamp(-100.0, 100.0));
        }
    }
    v
}

/// Color Lookup payload, written as an identity cube when there is no usable table.
pub(super) fn color_lookup(name: &str, lut: Option<&[f32]>, size: u32, dither: bool, source: Option<&LutSource>) -> Vec<u8> {
    // A lookup without a usable table (none chosen yet, or a bad size) renders as the
    // identity, so it is written as a 2³ identity cube rather than dropped.
    let n = size as usize;
    let cube = lut.zip(photocraft_doc::adjust::lut3d_len(size)).and_then(|(table, len)| table.get(..len)).filter(|_| n <= 256);
    // Like Photoshop, embed the LUT file as loaded, in its own format; a table made or
    // changed in PhotoCraft is written as `.cube`, named to match.
    let (format, data, name, dither) = match cube {
        Some(cube) => match lut_source_for(source, cube, n) {
            Some(src) => (src.format, src.bytes.to_vec(), name.to_string(), dither),
            None => {
                let file = photocraft_cms::lutfile::LutFile { title: String::new(), size: n, data: cube.to_vec(), domain_min: [0.0; 3], domain_max: [1.0; 3] };
                (LutFormat::Cube, photocraft_cms::lutfile::write_cube(&file).into_bytes(), cube_file_name(name), dither)
            }
        },
        _ => {
            let file = photocraft_cms::lutfile::LutFile { title: String::new(), ..photocraft_cms::lutfile::LutFile::identity(2) };
            (LutFormat::Cube, photocraft_cms::lutfile::write_cube(&file).into_bytes(), cube_file_name(name), false)
        }
    };
    let en = |t: &str, val: &str| Value::Enumerated { type_id: Id::new(t), value: Id::new(val) };
    let text = |t: &str| Value::Text(UnicodeString::new_nul(t));
    let d = Descriptor::new("null")
        .with("lookupType", en("colorLookupType", "3DLUT"))
        .with("NM  ", text(&name))
        .with("Dthr", Value::Boolean(dither))
        .with("profile", Value::RawData(Vec::new()))
        .with("LUTFormat", en("LUTFormatType", psd_lut_format(format)))
        .with("dataOrder", en("colorLookupOrder", "rgbOrder"))
        .with("tableOrder", en("colorLookupOrder", "bgrOrder"))
        .with("LUT3DFileData", Value::RawData(data))
        .with("LUT3DFileName", text(&name));
    let mut v = words(&[1]);
    v.extend_from_slice(&VersionedDescriptor::new(d).to_bytes());
    v
}

/// At least the two stops Photoshop needs, matching how fewer stops render.
pub(super) fn gradient_stops(stops: &[(f32, [f32; 3])]) -> Vec<(f32, [f32; 3])> {
    // Photoshop needs two stops: none becomes black to white, one becomes a flat colour.
    match stops {
        [] => vec![(0.0, [0.0; 3]), (1.0, [1.0; 3])],
        [(_, c)] => vec![(0.0, *c), (1.0, *c)],
        s => s.to_vec(),
    }
}

/// A descriptor integer of `x` clamped to `lo..=hi`.
fn percent(x: f32, lo: f32, hi: f32) -> Value {
    Value::Integer(i32::from(clamp_i16(x.clamp(lo, hi))))
}

/// Vibrance payload.
pub(super) fn vibrance(vibrance: f32, saturation: f32) -> Vec<u8> {
    let d = Descriptor::new("null").with("vibrance", percent(vibrance, -100.0, 100.0)).with("Strt", percent(saturation, -100.0, 100.0));
    VersionedDescriptor::new(d).to_bytes()
}

/// Color Balance payload: shadows, midtones and highlights, then Preserve Luminosity.
pub(super) fn color_balance(tones: [&[f32; 3]; 3], preserve_luminosity: bool) -> Vec<u8> {
    let mut v = Vec::new();
    for x in tones.into_iter().flatten() {
        push_i16(&mut v, x.clamp(-100.0, 100.0));
    }
    v.push(u8::from(preserve_luminosity));
    v.push(0);
    v
}

/// Black & White payload.
pub(super) fn black_white(weights: &[f32; 6], tint: Option<[f32; 3]>) -> Vec<u8> {
    let mut d = Descriptor::new("null");
    for (w, k) in weights.iter().zip(BW_KEYS) {
        d = d.with(k, percent(*w, -200.0, 300.0));
    }
    let d = d
        .with("useTint", Value::Boolean(tint.is_some()))
        .with("tintColor", Value::Descriptor(rgb_desc(tint.unwrap_or(BW_TINT))))
        .with("bwPresetKind", Value::Integer(1))
        .with("blackAndWhitePresetFileName", Value::Text(UnicodeString::new_nul("")));
    VersionedDescriptor::new(d).to_bytes()
}

/// Photo Filter payload, version 2 with a 16-bit colour.
pub(super) fn photo_filter(color: [f32; 3], density: f32, preserve_luminosity: bool) -> Vec<u8> {
    // Version 2 keeps the colour at 16 bits.
    let mut v = words(&[2, 0]);
    for x in color {
        put16(&mut v, (x.clamp(0.0, 1.0) * 65535.0).round() as u16);
    }
    put16(&mut v, 0);
    v.extend_from_slice(&((density.clamp(0.0, 1.0) * 100.0).round() as u32).to_be_bytes());
    v.push(u8::from(preserve_luminosity));
    // Photoshop only opens the payload padded to 4 bytes inside the block (#1455).
    v.extend_from_slice(&[0, 0, 0]);
    v
}

/// Channel Mixer payload: the red, green and blue records, then the gray record.
pub(super) fn channel_mixer(matrix: &[[f32; 4]; 3], monochrome: bool) -> Vec<u8> {
    let mut v = words(&[1, u16::from(monochrome)]);
    // Red, green and blue rows, then the gray record that monochrome uses.
    for row in matrix.iter().chain(std::iter::once(&matrix[0])) {
        let pct = |x: f32| clamp_i16((x * 100.0).clamp(-200.0, 200.0)).to_be_bytes();
        for x in &row[..3] {
            v.extend_from_slice(&pct(*x));
        }
        v.extend_from_slice(&[0, 0]);
        v.extend_from_slice(&pct(row[3]));
    }
    v
}
