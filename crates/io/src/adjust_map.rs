//! Mapping between PSD adjustment-layer blocks and [`Adjustment`].
//!
//! Binary layouts follow the Adobe spec ("Adjustment layer" section). The
//! mapped subset keeps the master/composite parameters; other channels of
//! Levels/Curves/Hue-Sat are written back with defaults.
//!
//! Every [`Adjustment`] kind is written (`vibA`, `blnc`, `blwh`, `phfl` and `mixr` included);
//! a Color Lookup without a table is written as an identity cube and a Gradient Map with fewer
//! than two stops as the equivalent two-stop gradient, so nothing is dropped on save. Blocks
//! that cannot be read (noise gradients, profile lookups, CMYK channel mixers, unknown versions)
//! stay [`Adjustment::Unsupported`] and are written back verbatim.

use photocraft_doc::Adjustment;
use photocraft_doc::adjust::{CURVE_MAP_LEN, CurvePoint, HueRange, LevelsChannel, LutFormat, LutSource, ToneSpace, curve_map};
use photocraft_psd::descriptor::{Descriptor, Value, VersionedDescriptor};

mod encode;

/// All PSD adjustment keys recognized as adjustment layers.
pub const ADJUSTMENT_KEYS: [&[u8; 4]; 16] =
    [b"levl", b"curv", b"hue2", b"brit", b"nvrt", b"thrs", b"post", b"expA", b"vibA", b"blnc", b"mixr", b"grdm", b"phfl", b"selc", b"blwh", b"clrL"];

fn be16(d: &[u8], at: usize) -> Option<u16> {
    d.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]))
}
fn bei16(d: &[u8], at: usize) -> Option<i16> {
    be16(d, at).map(|v| v as i16)
}
fn bef32(d: &[u8], at: usize) -> Option<f32> {
    d.get(at..at + 4).map(|b| f32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn unsupported(key: &[u8; 4], data: &[u8]) -> Adjustment {
    Adjustment::Unsupported { psd_key: String::from_utf8_lossy(key).into_owned(), raw: data.into() }
}

fn levels_rec(d: &[u8], at: usize) -> Option<LevelsChannel> {
    Some(LevelsChannel {
        in_black: f32::from(be16(d, at)?) / 255.0,
        in_white: f32::from(be16(d, at + 2)?) / 255.0,
        out_black: f32::from(be16(d, at + 4)?) / 255.0,
        out_white: f32::from(be16(d, at + 6)?) / 255.0,
        gamma: f32::from(be16(d, at + 8)?) / 100.0,
    })
}

fn parse_curves(d: &[u8]) -> Option<Adjustment> {
    // map flag(1) version(2) bitmap(4). A map (Photoshop's pencil-drawn curves) stores 256 output
    // bytes per channel instead of a point list (psd-tools reads the flag as `is_map`).
    let pencil = *d.first()? != 0;
    let version = be16(d, 1)?;
    if version != 1 && version != 4 {
        return None;
    }
    let bits = u32::from_be_bytes(d.get(3..7)?.try_into().ok()?);
    let mut at = 7;
    let line = || vec![CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 1.0, output: 1.0 }];
    let mut curves: Vec<Vec<CurvePoint>> = vec![line(), line(), line(), line(), line()];
    for bit in 0..32usize {
        if bits & (1 << bit) == 0 {
            continue;
        }
        if pencil {
            let map = d.get(at..at + CURVE_MAP_LEN)?;
            at += CURVE_MAP_LEN;
            if let Some(slot) = curves.get_mut(bit) {
                *slot = curve_map(&map.iter().map(|&b| f32::from(b) / 255.0).collect::<Vec<_>>());
            }
            continue;
        }
        let n = usize::from(be16(d, at)?);
        at += 2;
        let mut pts = Vec::with_capacity(n.min(64));
        for _ in 0..n {
            let out = be16(d, at)?;
            let inp = be16(d, at + 2)?;
            at += 4;
            pts.push(CurvePoint { input: f32::from(inp) / 255.0, output: f32::from(out) / 255.0 });
        }
        if let Some(slot) = curves.get_mut(bit) {
            *slot = pts;
        }
    }
    let mut it = curves.into_iter();
    let master = it.next()?;
    let (r, g, b, k) = (it.next()?, it.next()?, it.next()?, it.next()?);
    Some(Adjustment::Curves { master, per_channel: [r, g, b], space: ToneSpace::Rgb, black: k, pencil })
}

fn desc_num(d: &Descriptor, key: &str) -> Option<f32> {
    match d.get(key)? {
        Value::Integer(v) => Some(*v as f32),
        Value::Double(v) => Some(*v as f32),
        Value::UnitFloat { value, .. } => Some(*value as f32),
        _ => None,
    }
}

fn desc_bool(d: &Descriptor, key: &str) -> Option<bool> {
    match d.get(key)? {
        Value::Boolean(b) => Some(*b),
        _ => None,
    }
}

/// Channel interpretation of per-channel Levels/Curves records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channels {
    /// Records 1..=3 are R, G, B.
    Rgb,
    /// Record 1 is the gray channel (applied to all three display channels).
    Gray,
    /// Records 1..=4 are cyan, magenta, yellow, black ([`ToneSpace::Cmyk`]).
    Cmyk,
    /// Records 1..=3 are lightness, a, b ([`ToneSpace::Lab`]).
    Lab,
    /// Channel records do not map to display channels (multichannel, indexed…); ignored.
    Other,
}

/// The menu name of the adjustment a PSD key holds (for messages).
pub fn label(key: &[u8; 4]) -> &'static str {
    match key {
        b"levl" => "Levels",
        b"curv" => "Curves",
        b"hue2" => "Hue/Saturation",
        b"brit" => "Brightness/Contrast",
        b"nvrt" => "Invert",
        b"thrs" => "Threshold",
        b"post" => "Posterize",
        b"expA" => "Exposure",
        b"vibA" => "Vibrance",
        b"blnc" => "Color Balance",
        b"mixr" => "Channel Mixer",
        b"grdm" => "Gradient Map",
        b"phfl" => "Photo Filter",
        b"selc" => "Selective Color",
        b"blwh" => "Black & White",
        b"clrL" => "Color Lookup",
        _ => "adjustment",
    }
}

/// Keys of a Brightness/Contrast `CgEd` descriptor: the ones [`write`] emits plus Photoshop's
/// preset bookkeeping.
const BRIT_CGED_KEYS: [&str; 9] = ["Vrsn", "Brgh", "Cntr", "means", "Lab ", "useLegacy", "Auto", "presetKind", "presetFileName"];

/// Whether a `brit` layer's `CgEd` descriptor holds settings Brightness/Contrast has no place
/// for. Photoshop 2026's Light adjustment (Exposure, Contrast, Highlights, Shadows, Whites,
/// Blacks) is stored under the `brit` key; read as Brightness/Contrast it kept only Contrast and
/// lost the rest without a word (#6542), so such a layer stays [`Adjustment::Unsupported`].
pub fn is_extended_brit(cged: Option<&[u8]>) -> bool {
    cged.and_then(|c| VersionedDescriptor::parse_prefix(c).ok())
        .is_some_and(|(v, _)| v.descriptor.items.iter().any(|(k, _)| !BRIT_CGED_KEYS.iter().any(|n| k.is(n))))
}

/// Why [`parse`] keeps a block as [`Adjustment::Unsupported`], when there is more to say than
/// "it could not be read" (so far: Color Lookup).
pub fn unreadable_reason(key: &[u8; 4], data: &[u8]) -> Option<String> {
    match key {
        b"clrL" => read_lookup(data).err(),
        _ => None,
    }
}

/// Parses an adjustment block. `cged` is the optional `CgEd` block data
/// (modern brightness/contrast parameters). Levels/Curves store the
/// composite record first, then one per document channel (see [`Channels`]).
pub fn parse(key: &[u8; 4], data: &[u8], cged: Option<&[u8]>, channels: Channels) -> Adjustment {
    // A CMYK Channel Mixer (any record using the fourth, black, source) has no RGB meaning:
    // keep it raw. Mixers written by `write` leave that source at 0 and map in every mode.
    if key == b"mixr" && matches!(channels, Channels::Cmyk | Channels::Other) && (0..).map_while(|r| bei16(data, 4 + r * 10 + 6)).any(|k| k != 0) {
        return unsupported(key, data);
    }
    let mut a = parse_any(key, data, cged);
    match (&mut a, channels) {
        (Adjustment::Levels { space, black, .. }, Channels::Cmyk) | (Adjustment::Levels { space, black, .. }, Channels::Lab) => {
            *space = if channels == Channels::Cmyk { ToneSpace::Cmyk } else { ToneSpace::Lab };
            if channels == Channels::Lab {
                *black = LevelsChannel::default();
            }
        }
        (Adjustment::Curves { space, black, .. }, Channels::Cmyk) => {
            *space = ToneSpace::Cmyk;
            if photocraft_doc::adjust::is_identity_curve(black) {
                black.clear();
            }
        }
        (Adjustment::Curves { space, black, .. }, Channels::Lab) => {
            *space = ToneSpace::Lab;
            black.clear();
        }
        (Adjustment::Levels { black, .. }, _) => *black = LevelsChannel::default(),
        (Adjustment::Curves { black, .. }, _) => black.clear(),
        _ => {}
    }
    match (&mut a, channels) {
        (_, Channels::Rgb | Channels::Cmyk | Channels::Lab) => {}
        (Adjustment::Levels { per_channel, .. }, Channels::Gray) => {
            let g = per_channel[0].clone();
            *per_channel = [g.clone(), g.clone(), g];
        }
        (Adjustment::Curves { per_channel, .. }, Channels::Gray) => {
            let g = per_channel[0].clone();
            *per_channel = [g.clone(), g.clone(), g];
        }
        (Adjustment::Levels { per_channel, .. }, Channels::Other) => *per_channel = Default::default(),
        (Adjustment::Curves { per_channel, .. }, Channels::Other) => {
            let line = || vec![CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 1.0, output: 1.0 }];
            *per_channel = [line(), line(), line()];
        }
        _ => {}
    }
    a
}

fn parse_any(key: &[u8; 4], data: &[u8], cged: Option<&[u8]>) -> Adjustment {
    let parsed = match key {
        b"nvrt" => Some(Adjustment::Invert),
        b"thrs" => be16(data, 0).map(|v| Adjustment::Threshold { level: f32::from(v) / 255.0 }),
        b"post" => be16(data, 0).map(|v| Adjustment::Posterize { levels: u32::from(v) }),
        // Kept raw (with its `CgEd`, which the PSD writer keeps while the layer is unchanged).
        b"brit" if is_extended_brit(cged) => None,
        b"brit" => {
            let modern = cged.and_then(|c| VersionedDescriptor::parse_prefix(c).ok()).and_then(|(v, _)| {
                let d = v.descriptor;
                Some(Adjustment::BrightnessContrast {
                    brightness: desc_num(&d, "Brgh")?,
                    contrast: desc_num(&d, "Cntr")?,
                    legacy: desc_bool(&d, "useLegacy").unwrap_or(false),
                })
            });
            modern
                .or_else(|| Some(Adjustment::BrightnessContrast { brightness: f32::from(bei16(data, 0)?), contrast: f32::from(bei16(data, 2)?), legacy: true }))
        }
        b"hue2" => (|| {
            let colorize = *data.get(2)? != 0;
            let base = if colorize { 4 } else { 10 };
            // Six ranges from byte 16: four range values (degrees) then hue, saturation, lightness.
            let mut ranges = HueRange::defaults();
            for (i, r) in ranges.iter_mut().enumerate() {
                let at = 16 + i * 14;
                let v = |k: usize| bei16(data, at + 2 * k).map(f32::from);
                if let (Some(a), Some(b), Some(c), Some(d), Some(h), Some(s), Some(l)) = (v(0), v(1), v(2), v(3), v(4), v(5), v(6)) {
                    *r = HueRange { hue: h, saturation: s, lightness: l, bounds: HueRange::canonical_bounds([a, b, c, d]) };
                }
            }
            let hue = f32::from(bei16(data, base)?);
            Some(Adjustment::HueSaturation {
                // Photoshop stores the colorize hue signed (200 as -160); the model keeps 0..360.
                hue: if colorize { hue.rem_euclid(360.0) } else { hue },
                saturation: f32::from(bei16(data, base + 2)?),
                lightness: f32::from(bei16(data, base + 4)?),
                colorize,
                ranges,
            })
        })(),
        b"expA" => (|| Some(Adjustment::Exposure { exposure: bef32(data, 2)?, offset: bef32(data, 6)?, gamma: bef32(data, 10)? }))(),
        b"levl" => (|| {
            let m = levels_rec(data, 2)?;
            let r = levels_rec(data, 12)?;
            let g = levels_rec(data, 22)?;
            let b = levels_rec(data, 32)?;
            let k = levels_rec(data, 42).unwrap_or_default();
            Some(Adjustment::Levels { master: m, per_channel: [r, g, b], space: ToneSpace::Rgb, black: k })
        })(),
        b"curv" => parse_curves(data),
        b"selc" => parse_selective(data),
        b"clrL" => parse_lookup(data),
        b"grdm" => parse_gradient_map(data),
        b"vibA" => parse_vibrance(data),
        b"blnc" => parse_color_balance(data),
        b"blwh" => parse_black_white(data),
        b"phfl" => parse_photo_filter(data),
        b"mixr" => parse_channel_mixer(data),
        _ => None,
    };
    parsed.unwrap_or_else(|| unsupported(key, data))
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// `grdm` (Adobe spec, "Gradient settings"): version (1, or 3 with an interpolation method
/// key), reverse, dither, [method], name, colour stops (location /4096, midpoint %, colour
/// space + four u16 components, 2 bytes), transparency stops, then smoothness (/4096) among
/// noise-gradient fields. Smoothness, midpoints and the method are baked into dense stops like
/// gradient fills. Stops may be in any plain colour space Photoshop writes (RGB, HSB, CMYK, Lab,
/// Grayscale: a CMYK or Lab document stores them in its own space); they are converted to RGB
/// through the colour-managed conversions (the document's CMYK profile when one is in scope).
/// Noise gradients (no colour stops) and stops in other spaces (book colours) stay unsupported.
fn parse_gradient_map(d: &[u8]) -> Option<Adjustment> {
    let version = be16(d, 0)?;
    let reverse = *d.get(2)? != 0;
    let dither = d.get(3).is_some_and(|b| *b != 0);
    let (method, mut at) = match version {
        1 => (None, 4),
        3 => (Some(d.get(4..8)?), 8),
        _ => return None,
    };
    // The gradient's name (UTF-16 units); checked, as `len * 2` overflows a 32-bit `usize`.
    at = (be32(d, at)? as usize).checked_mul(2)?.checked_add(at + 4)?;
    let n = usize::from(be16(d, at)?);
    at += 2;
    let mut stops = Vec::with_capacity(n);
    let mut mids = Vec::with_capacity(n);
    for _ in 0..n {
        let loc = (be32(d, at)? as f32 / 4096.0).clamp(0.0, 1.0);
        let mid = be32(d, at + 4)? as f32 / 100.0;
        let space = be16(d, at + 8)?;
        let c = |k: usize| be16(d, at + 10 + 2 * k);
        stops.push((loc, crate::channel_map::decode_known_color(space, [c(0)?, c(1)?, c(2)?, c(3)?])?));
        mids.push(mid);
        at += 20;
    }
    if stops.len() < 2 {
        return None;
    }
    let nt = usize::from(be16(d, at)?);
    at += 2 + nt * 10 + 2; // transparency stops, expansion count
    let smooth = f32::from(be16(d, at)?) / 4096.0;
    // Midpoint k applies to the segment after stop k (in location order).
    let mut order: Vec<usize> = (0..stops.len()).collect();
    order.sort_by(|a, b| stops[*a].0.total_cmp(&stops[*b].0));
    let mids: Vec<f32> = order.iter().skip(1).map(|i| mids[*i]).collect();
    let baked = crate::gradient_bake::bake(stops, &mids, smooth, crate::gradient_bake::Method::from_code(method));
    let mut stops: Vec<(f32, [f32; 3])> = baked.iter().map(|(t, c)| (*t, c.to_rgb())).collect();
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(Adjustment::GradientMap { stops, reverse, dither })
}

/// Points an imported `grdm` block at `target` when only Reverse and/or Dither differ (the
/// stops parse unchanged): patches those two bytes and returns `true`, so the file's own stops
/// (in whatever colour space it stored them), smoothness and method survive the save. `false`
/// leaves `data` untouched.
pub(crate) fn retarget_gradient_map(data: &mut [u8], target: &Adjustment, channels: Channels) -> bool {
    let Adjustment::GradientMap { stops, reverse, dither } = target else { return false };
    let same_stops = matches!(parse(b"grdm", data, None, channels), Adjustment::GradientMap { stops: s, .. } if s == *stops);
    match (same_stops, data.get_mut(2..4)) {
        (true, Some(flags)) => {
            flags[0] = u8::from(*reverse);
            flags[1] = u8::from(*dither);
            true
        }
        _ => false,
    }
}

/// `true` for a `grdm` block exactly as older PhotoCraft builds wrote it: [`write_gradient_map`]'s
/// output without the two padding bytes, which makes Photoshop refuse the file (#8070). Blocks
/// from other writers, whatever their length, are not matched.
pub(crate) fn is_unpadded_photocraft_gradient_map(data: &[u8]) -> bool {
    if data.len().is_multiple_of(4) {
        return false;
    }
    match parse_gradient_map(data) {
        Some(Adjustment::GradientMap { stops, reverse, dither }) => {
            let padded = write_gradient_map(&stops, reverse, dither);
            padded.len() == data.len() + 2 && padded.starts_with(data)
        }
        _ => false,
    }
}

/// `grdm` version 1 for [`Adjustment::GradientMap`] (Classic interpolation, Smoothness 0, so
/// the stops are reproduced exactly).
fn write_gradient_map(stops: &[(f32, [f32; 3])], reverse: bool, dither: bool) -> Vec<u8> {
    let mut v = Vec::new();
    put16(&mut v, 1);
    v.push(u8::from(reverse));
    v.push(u8::from(dither));
    let name: Vec<u16> = "Custom".encode_utf16().chain(std::iter::once(0)).collect();
    v.extend_from_slice(&(name.len() as u32).to_be_bytes());
    for u in name {
        put16(&mut v, u);
    }
    put16(&mut v, stops.len().min(usize::from(u16::MAX)) as u16);
    for (t, c) in stops.iter().take(usize::from(u16::MAX)) {
        v.extend_from_slice(&((t.clamp(0.0, 1.0) * 4096.0).round() as u32).to_be_bytes());
        v.extend_from_slice(&50u32.to_be_bytes());
        put16(&mut v, 0);
        for x in c {
            put16(&mut v, (x.clamp(0.0, 1.0) * 65535.0).round() as u16);
        }
        put16(&mut v, 0); // fourth component
        put16(&mut v, 0);
    }
    put16(&mut v, 2);
    for t in [0u32, 4096] {
        v.extend_from_slice(&t.to_be_bytes());
        v.extend_from_slice(&50u32.to_be_bytes());
        put16(&mut v, 255);
    }
    put16(&mut v, 2); // expansion count
    put16(&mut v, 0); // smoothness
    put16(&mut v, 32); // length
    put16(&mut v, 0); // mode
    v.extend_from_slice(&0u32.to_be_bytes()); // random seed
    put16(&mut v, 0); // showing transparency
    put16(&mut v, 0); // using vector colour
    v.extend_from_slice(&2048u32.to_be_bytes()); // roughness
    put16(&mut v, 3); // colour model
    for x in [0u16, 0, 0, 0, 0x8000, 0x8000, 0x8000, 0x8000] {
        put16(&mut v, x);
    }
    put16(&mut v, 0);
    // Photoshop includes the two-byte pad in the `grdm` length when the block is 2 mod 4 (#8070).
    v.resize(v.len().next_multiple_of(4), 0);
    v
}

/// `selc`: version, method (0 relative / 1 absolute), then 10 CMYK records of i16 percentages;
/// record 0 is reserved, records 1..=9 are reds … blacks.
fn parse_selective(d: &[u8]) -> Option<Adjustment> {
    if be16(d, 0)? != 1 || d.len() < 84 {
        return None;
    }
    let relative = be16(d, 2)? == 0;
    let mut adjustments = [[0.0f32; 4]; 9];
    for (r, rec) in adjustments.iter_mut().enumerate() {
        for (k, v) in rec.iter_mut().enumerate() {
            *v = f32::from(bei16(d, 4 + (r + 1) * 8 + k * 2)?);
        }
    }
    Some(Adjustment::SelectiveColor { relative, adjustments })
}

fn desc_text(d: &Descriptor, key: &str) -> Option<String> {
    match d.get(key)? {
        Value::Text(t) => Some(t.to_string_lossy()),
        _ => None,
    }
}

fn desc_enum<'a>(d: &'a Descriptor, key: &str) -> Option<&'a [u8]> {
    match d.get(key)? {
        Value::Enumerated { value, .. } => Some(value.as_bytes()),
        _ => None,
    }
}

/// `clrL`: version 1 + a versioned descriptor carrying the LUT file itself (`LUT3DFileData`, in
/// the format named by `LUTFormat`). Profile-based lookups (abstract / device link) have no
/// table and stay [`Adjustment::Unsupported`].
fn parse_lookup(d: &[u8]) -> Option<Adjustment> {
    read_lookup(d).ok()
}

/// [`parse_lookup`] with the reason a block can't be read.
fn read_lookup(d: &[u8]) -> Result<Adjustment, String> {
    match be16(d, 0) {
        Some(1) => {}
        Some(v) => return Err(format!("unknown version {v}")),
        None => return Err("the block is truncated".into()),
    }
    let (v, _) = VersionedDescriptor::parse_prefix(d.get(2..).unwrap_or_default()).map_err(|e| format!("unreadable settings ({e})"))?;
    let desc = v.descriptor;
    let bytes = match desc.get("LUT3DFileData") {
        Some(Value::RawData(b)) if !b.is_empty() => b,
        Some(_) => return Err("its LUT data is empty".into()),
        None => return Err("it uses a colour profile (abstract or device link), not a LUT file, which isn't supported yet".into()),
    };
    let format = match desc_enum(&desc, "LUTFormat") {
        Some(b"LUTFormat3DL") => LutFormat::ThreeDl,
        Some(b"LUTFormatLOOK") => LutFormat::Look,
        _ => LutFormat::Cube,
    };
    let lut = parse_lut_source(format, bytes).map_err(|e| format!("its {} LUT can't be read ({e})", psd_lut_format(format).trim_start_matches("LUTFormat")))?;
    if !lut.domain_is_default() {
        return Err(format!(
            "its LUT has a non-default input domain (DOMAIN_MIN {:?}, DOMAIN_MAX {:?}), which is not supported yet",
            lut.domain_min, lut.domain_max
        ));
    }
    let name = desc_text(&desc, "LUT3DFileName").or_else(|| desc_text(&desc, "NM  ")).unwrap_or_default();
    Ok(Adjustment::ColorLookup {
        name,
        size: lut.size as u32,
        lut: Some(std::sync::Arc::new(lut.data)),
        tetrahedral: false,
        dither: desc_bool(&desc, "Dthr").unwrap_or(false),
        // Kept so a save embeds the file as loaded (Photoshop never converts it).
        source: Some(LutSource { format, bytes: std::sync::Arc::new(bytes.clone()) }),
    })
}

/// A LUT file's table, read the way a PSD read does (an XML file tagged `.cube` is sniffed as `.look`).
fn parse_lut_source(format: LutFormat, bytes: &[u8]) -> Result<photocraft_cms::lutfile::LutFile, photocraft_cms::lutfile::LutError> {
    let format = match format {
        LutFormat::Cube => photocraft_cms::lutfile::Format::Cube,
        LutFormat::ThreeDl => photocraft_cms::lutfile::Format::ThreeDl,
        LutFormat::Look => photocraft_cms::lutfile::Format::Look,
    };
    photocraft_cms::lutfile::parse(&format!("x.{}", format.extension()), bytes)
}

/// Photoshop's `LUTFormatType` value for a format.
fn psd_lut_format(format: LutFormat) -> &'static str {
    match format {
        LutFormat::Cube => "LUTFormatCUBE",
        LutFormat::ThreeDl => "LUTFormat3DL",
        LutFormat::Look => "LUTFormatLOOK",
    }
}

/// The source file of a Color Lookup to embed as is: only while it still parses to the
/// layer's table, so an edited table is never saved with stale bytes.
fn lut_source_for<'a>(source: Option<&'a LutSource>, table: &[f32], size: usize) -> Option<&'a LutSource> {
    let source = source?;
    let file = parse_lut_source(source.format, &source.bytes).ok()?;
    (file.size == size && file.data == table).then_some(source)
}

/// `name` with a `.3dl` / `.look` extension swapped for `.cube`, for a table saved as `.cube`.
fn cube_file_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    match [".3dl", ".look"].iter().find(|ext| lower.ends_with(*ext)) {
        Some(ext) => format!("{}.cube", name.get(..name.len() - ext.len()).unwrap_or(name)),
        None => name.to_string(),
    }
}

/// The descriptor of a `vibA` / `blwh` block: a 4-byte descriptor version (16), then the descriptor.
fn versioned(d: &[u8]) -> Option<Descriptor> {
    VersionedDescriptor::parse_prefix(d).ok().map(|(v, _)| v.descriptor)
}

/// `vibA` (Adobe spec, "Vibrance"): a versioned descriptor with `vibrance` and `Strt`
/// (saturation), both integers in -100..=100. Photoshop omits a key whose value is 0.
fn parse_vibrance(d: &[u8]) -> Option<Adjustment> {
    let desc = versioned(d)?;
    let get = |k: &str| desc_num(&desc, k).unwrap_or(0.0).clamp(-100.0, 100.0);
    Some(Adjustment::Vibrance { vibrance: get("vibrance"), saturation: get("Strt") })
}

/// `blnc` (Adobe spec, "Color Balance"): shadows, midtones and highlights as three i16 each
/// (cyan-red, magenta-green, yellow-blue, -100..=100), then preserve luminosity (1 byte).
fn parse_color_balance(d: &[u8]) -> Option<Adjustment> {
    let tone = |k: usize| -> Option<[f32; 3]> {
        let mut out = [0.0; 3];
        for (i, v) in out.iter_mut().enumerate() {
            *v = f32::from(bei16(d, (k * 3 + i) * 2)?).clamp(-100.0, 100.0);
        }
        Some(out)
    };
    Some(Adjustment::ColorBalance { shadows: tone(0)?, midtones: tone(1)?, highlights: tone(2)?, preserve_luminosity: *d.get(18)? != 0 })
}

/// Photoshop's default Black & White mix (reds, yellows, greens, cyans, blues, magentas) and tint.
const BW_DEFAULT: [f32; 6] = [40.0, 60.0, 40.0, 60.0, 20.0, 80.0];
const BW_KEYS: [&str; 6] = ["Rd  ", "Yllw", "Grn ", "Cyn ", "Bl  ", "Mgnt"];
const BW_TINT: [f32; 3] = [225.0 / 255.0, 211.0 / 255.0, 179.0 / 255.0];

/// An `RGBC` colour (0..255 doubles) read in f64 so `write`'s encoding comes back bit-exact;
/// other colour classes go through [`crate::blocks::color_from_desc`].
fn desc_rgb(d: &Descriptor) -> Option<[f32; 3]> {
    let g = |k: &str| match d.get(k)? {
        Value::Double(v) => Some(*v),
        Value::Integer(v) => Some(f64::from(*v)),
        Value::UnitFloat { value, .. } => Some(*value),
        _ => None,
    };
    if d.class_id.as_bytes() == b"RGBC" && d.get("redFloat").is_none() {
        let c = [g("Rd  ")?, g("Grn ")?, g("Bl  ")?];
        return Some(c.map(|v| (v / 255.0).clamp(0.0, 1.0) as f32));
    }
    crate::blocks::color_from_desc(d).map(|c| c.to_rgb().map(|v| v.clamp(0.0, 1.0)))
}

fn rgb_desc(c: [f32; 3]) -> Descriptor {
    let d = |v: f32| Value::Double(f64::from(v.clamp(0.0, 1.0)) * 255.0);
    Descriptor::new("RGBC").with("Rd  ", d(c[0])).with("Grn ", d(c[1])).with("Bl  ", d(c[2]))
}

/// `blwh` (Adobe spec, "Black and White"): a versioned descriptor with the six colour weights
/// (`Rd  `, `Yllw`, `Grn `, `Cyn `, `Bl  `, `Mgnt`, percent), `useTint` and `tintColor`.
fn parse_black_white(d: &[u8]) -> Option<Adjustment> {
    let desc = versioned(d)?;
    let mut weights = BW_DEFAULT;
    for (w, k) in weights.iter_mut().zip(BW_KEYS) {
        if let Some(v) = desc_num(&desc, k) {
            *w = v.clamp(-200.0, 300.0);
        }
    }
    let tint = desc_bool(&desc, "useTint").unwrap_or(false).then(|| crate::blocks::get_desc(&desc, "tintColor").and_then(desc_rgb).unwrap_or(BW_TINT));
    Some(Adjustment::BlackWhite { weights, tint })
}

/// `phfl` (Adobe spec, "Photo Filter"): version (2 or 3); version 2 has a colour structure
/// (colour space + four u16), version 3 "4 bytes each for XYZ color"; then density (u32 percent)
/// and preserve luminosity (1 byte).
///
/// The spec gives no scale for the XYZ values. They are read as XYZ relative to the D50 white
/// (X / 0.9642, Y, Z / 0.8249) with 1.0 = 32768, so white is stored as (32768, 32768, 32768).
/// That is inferred, not documented: decoded this way, every version 3 colour in the Photoshop
/// CS2 files of #3777 and #4940 is exactly a Photoshop preset (the Yellow filter holds
/// (24191, 25002, 3827) and decodes to #F9E31C, Warming Filter (85) holds (15548, 12029, 0)
/// and decodes to #EC8A00); reading them as absolute XYZ gives #FFE03A and #F18600.
const PHFL_XYZ_ONE: f32 = 32768.0;

fn parse_photo_filter(d: &[u8]) -> Option<Adjustment> {
    let (color, at) = match be16(d, 0)? {
        2 => {
            let space = be16(d, 2)?;
            if !matches!(space, 0 | 2 | 7 | 8) {
                return None;
            }
            let c = [be16(d, 4)?, be16(d, 6)?, be16(d, 8)?, be16(d, 10)?];
            (crate::channel_map::decode_color(space, c).to_rgb(), 12)
        }
        3 => {
            let rel = [be32(d, 2)?, be32(d, 6)?, be32(d, 10)?].map(|v| v as f32 / PHFL_XYZ_ONE);
            // Relative to the white, real colours stay below about 2.0 (Cooling Filter (82)
            // reaches 1.99 on Z); anything past that is not an XYZ colour.
            if rel.iter().any(|v| *v > 2.0) {
                return None;
            }
            // Relative to the D50 white: bring it back to absolute D50 XYZ before converting.
            let xyz = [rel[0] * photocraft_color::convert::D50[0], rel[1], rel[2] * photocraft_color::convert::D50[2]];
            let lin = photocraft_color::convert::mat3_mul(&photocraft_color::convert::XYZ_D50_TO_SRGB, xyz);
            (lin.map(|v| photocraft_color::convert::linear_to_srgb(v.clamp(0.0, 1.0))), 14)
        }
        _ => return None,
    };
    let density = (be32(d, at)? as f32 / 100.0).clamp(0.0, 1.0);
    let preserve_luminosity = *d.get(at + 4)? != 0;
    Some(Adjustment::PhotoFilter { color: color.map(|v| v.clamp(0.0, 1.0)), density, preserve_luminosity })
}

/// `mixr` (Adobe spec, "Channel Mixer"): version (1), monochrome (u16), then one 10-byte record
/// per output channel: four i16 source percentages (red, green, blue, and a fourth used only by
/// CMYK documents) and an i16 constant (percent). With monochrome on, the first record is the
/// gray mix (as ag-psd, MIT, documents).
fn parse_channel_mixer(d: &[u8]) -> Option<Adjustment> {
    if be16(d, 0)? != 1 {
        return None;
    }
    let monochrome = be16(d, 2)? != 0;
    let rec = |r: usize| -> Option<[f32; 4]> {
        let at = 4 + r * 10;
        let v = |k: usize| bei16(d, at + k * 2).map(|x| f32::from(x).clamp(-200.0, 200.0) / 100.0);
        Some([v(0)?, v(1)?, v(2)?, v(4)?])
    };
    let first = rec(0)?;
    // A monochrome block may carry only the gray record.
    let (g, b) = if monochrome { (rec(1).unwrap_or([0.0; 4]), rec(2).unwrap_or([0.0; 4])) } else { (rec(1)?, rec(2)?) };
    Some(Adjustment::ChannelMixer { matrix: [first, g, b], monochrome })
}

fn put16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn q255(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u16
}
fn clamp_i16(v: f32) -> i16 {
    v.round().clamp(-32768.0, 32767.0) as i16
}

fn levels_write(v: &mut Vec<u8>, c: &LevelsChannel) {
    put16(v, q255(c.in_black));
    put16(v, q255(c.in_white));
    put16(v, q255(c.out_black));
    put16(v, q255(c.out_white));
    put16(v, (c.gamma * 100.0).round().clamp(1.0, 999.0) as u16);
}

/// Serializes an adjustment into its PSD blocks: `(key, data)` pairs
/// (Brightness/Contrast also writes a `CgEd` descriptor). Every evaluated kind has an
/// encoding; the result is empty only for an [`Adjustment::Unsupported`] with a malformed key.
pub fn write(adj: &Adjustment) -> Vec<([u8; 4], Vec<u8>)> {
    let block = |key: &[u8; 4], data: Vec<u8>| vec![(*key, data)];
    match adj {
        Adjustment::Invert => return block(b"nvrt", Vec::new()),
        Adjustment::Threshold { level } => return block(b"thrs", encode::words(&[q255(*level).max(1), 0])),
        Adjustment::Posterize { levels } => return block(b"post", encode::words(&[(*levels).clamp(2, 255) as u16, 0])),
        Adjustment::BrightnessContrast { brightness, contrast, legacy } => return encode::brightness_contrast(*brightness, *contrast, *legacy),
        Adjustment::HueSaturation { hue, saturation, lightness, colorize, ranges } => {
            return block(b"hue2", encode::hue_saturation([*hue, *saturation, *lightness], *colorize, ranges));
        }
        Adjustment::Exposure { exposure, offset, gamma } => return block(b"expA", encode::exposure(*exposure, *offset, *gamma)),
        Adjustment::Levels { master, per_channel, space, black } => return block(b"levl", encode::levels(master, per_channel, *space, black)),
        Adjustment::Curves { master, per_channel, space, black, pencil } => {
            return block(b"curv", encode::curves(master, per_channel, *space, black, *pencil));
        }
        Adjustment::SelectiveColor { relative, adjustments } => return block(b"selc", encode::selective_color(*relative, adjustments)),
        Adjustment::ColorLookup { name, lut, size, dither, source, .. } => {
            return block(b"clrL", encode::color_lookup(name, lut.as_deref().map(Vec::as_slice), *size, *dither, source.as_ref()));
        }
        Adjustment::GradientMap { stops, reverse, dither } => {
            return block(b"grdm", write_gradient_map(&encode::gradient_stops(stops), *reverse, *dither));
        }
        Adjustment::Vibrance { vibrance, saturation } => return block(b"vibA", encode::vibrance(*vibrance, *saturation)),
        Adjustment::ColorBalance { shadows, midtones, highlights, preserve_luminosity } => {
            return block(b"blnc", encode::color_balance([shadows, midtones, highlights], *preserve_luminosity));
        }
        Adjustment::BlackWhite { weights, tint } => return block(b"blwh", encode::black_white(weights, *tint)),
        Adjustment::PhotoFilter { color, density, preserve_luminosity } => {
            return block(b"phfl", encode::photo_filter(*color, *density, *preserve_luminosity));
        }
        Adjustment::ChannelMixer { matrix, monochrome } => {
            return block(b"mixr", encode::channel_mixer(matrix, *monochrome));
        }
        Adjustment::Unsupported { psd_key, raw } => {
            let k = psd_key.as_bytes();
            if k.len() == 4 {
                return vec![([k[0], k[1], k[2], k[3]], raw.to_vec())];
            }
        }
    }
    // Only an `Unsupported` block whose key is not four bytes has no encoding.
    Vec::new()
}

#[cfg(test)]
mod tests;
