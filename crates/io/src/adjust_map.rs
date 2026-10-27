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
use photocraft_doc::adjust::{CURVE_MAP_LEN, CurvePoint, HueRange, LevelsChannel, ToneSpace, curve_map, curve_map_outputs};
use photocraft_psd::descriptor::{Descriptor, Value, VersionedDescriptor};

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
    let (ext, format) = match desc_enum(&desc, "LUTFormat") {
        Some(b"LUTFormat3DL") => ("x.3dl", "3DL"),
        Some(b"LUTFormatLOOK") => ("x.look", "LOOK"),
        _ => ("x.cube", "CUBE"),
    };
    let lut = photocraft_cms::lutfile::parse(ext, bytes).map_err(|e| format!("its {format} LUT can't be read ({e})"))?;
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
    })
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
    let mut v = Vec::new();
    match adj {
        Adjustment::Invert => return vec![(*b"nvrt", Vec::new())],
        Adjustment::Threshold { level } => {
            put16(&mut v, q255(*level).max(1));
            put16(&mut v, 0);
            return vec![(*b"thrs", v)];
        }
        Adjustment::Posterize { levels } => {
            put16(&mut v, (*levels).clamp(2, 255) as u16);
            put16(&mut v, 0);
            return vec![(*b"post", v)];
        }
        Adjustment::BrightnessContrast { brightness, contrast, legacy } => {
            v.extend_from_slice(&clamp_i16(*brightness).to_be_bytes());
            v.extend_from_slice(&clamp_i16(*contrast).to_be_bytes());
            put16(&mut v, 127);
            v.push(0);
            let d = Descriptor::new("null")
                .with("Vrsn", Value::Integer(1))
                .with("Brgh", Value::Integer(brightness.round() as i32))
                .with("Cntr", Value::Integer(contrast.round() as i32))
                .with("means", Value::Integer(127))
                .with("Lab ", Value::Boolean(false))
                .with("useLegacy", Value::Boolean(*legacy))
                .with("Auto", Value::Boolean(false));
            return vec![(*b"brit", v), (*b"CgEd", VersionedDescriptor::new(d).to_bytes())];
        }
        Adjustment::HueSaturation { hue, saturation, lightness, colorize, ranges } => {
            put16(&mut v, 2);
            v.push(u8::from(*colorize));
            v.push(0);
            // The colorize triplet, then the master triplet. The inactive one gets Photoshop's
            // defaults; the colorize hue (0..360 in the model) is stored signed, -180..180.
            let (colorize_hsl, master) = if *colorize {
                let h = hue.rem_euclid(360.0);
                ([if h > 180.0 { h - 360.0 } else { h }, *saturation, *lightness], [0.0; 3])
            } else {
                ([30.0, 25.0, 0.0], [*hue, *saturation, *lightness])
            };
            for x in colorize_hsl.into_iter().chain(master) {
                v.extend_from_slice(&clamp_i16(x).to_be_bytes());
            }
            // Six hue ranges (reds, yellows, greens, cyans, blues, magentas): range values in
            // 0..360, then the range's hue, saturation and lightness.
            for r in ranges {
                for x in r.bounds {
                    put16(&mut v, x.rem_euclid(360.0).round() as u16 % 360);
                }
                for x in [r.hue, r.saturation, r.lightness] {
                    v.extend_from_slice(&clamp_i16(x).to_be_bytes());
                }
            }
            return vec![(*b"hue2", v)];
        }
        Adjustment::Exposure { exposure, offset, gamma } => {
            put16(&mut v, 1);
            for x in [exposure, offset, gamma] {
                v.extend_from_slice(&x.to_be_bytes());
            }
            v.push(1); // color space flag (spec: "1 byte")
            return vec![(*b"expA", v)];
        }
        Adjustment::Levels { master, per_channel, space, black } => {
            put16(&mut v, 2);
            levels_write(&mut v, master);
            for c in per_channel {
                levels_write(&mut v, c);
            }
            let ident = LevelsChannel::default();
            levels_write(&mut v, if *space == ToneSpace::Cmyk { black } else { &ident });
            for _ in 5..29 {
                levels_write(&mut v, &LevelsChannel::default());
            }
            return vec![(*b"levl", v)];
        }
        Adjustment::Curves { master, per_channel, space, black, pencil } => {
            v.push(u8::from(*pencil));
            put16(&mut v, 1);
            let ink = *space == ToneSpace::Cmyk && black.len() >= 2;
            v.extend_from_slice(&(if ink { 0b11111u32 } else { 0b1111 }).to_be_bytes());
            for c in std::iter::once(master).chain(per_channel.iter()).chain(ink.then_some(black)) {
                if *pencil {
                    v.extend(curve_map_outputs(c).iter().map(|&o| u8::try_from(q255(o)).unwrap_or(u8::MAX)));
                    continue;
                }
                put16(&mut v, c.len().min(19) as u16);
                for p in c.iter().take(19) {
                    put16(&mut v, q255(p.output));
                    put16(&mut v, q255(p.input));
                }
            }
            return vec![(*b"curv", v)];
        }
        Adjustment::SelectiveColor { relative, adjustments } => {
            put16(&mut v, 1);
            put16(&mut v, u16::from(!*relative));
            v.extend_from_slice(&[0; 8]);
            for rec in adjustments {
                for x in rec {
                    v.extend_from_slice(&clamp_i16(x.clamp(-100.0, 100.0)).to_be_bytes());
                }
            }
            return vec![(*b"selc", v)];
        }
        Adjustment::ColorLookup { name, lut, size, dither, .. } => {
            // A lookup without a usable table (none chosen yet, or a bad size) renders as the
            // identity, so it is written as a 2³ identity cube rather than dropped.
            let n = *size as usize;
            let cube = lut.as_deref().zip(photocraft_doc::adjust::lut3d_len(*size)).and_then(|(table, len)| table.get(..len)).filter(|_| n <= 256);
            let (file, dither) = match cube {
                Some(cube) => (
                    photocraft_cms::lutfile::LutFile { title: String::new(), size: n, data: cube.to_vec(), domain_min: [0.0; 3], domain_max: [1.0; 3] },
                    *dither,
                ),
                _ => (photocraft_cms::lutfile::LutFile { title: String::new(), ..photocraft_cms::lutfile::LutFile::identity(2) }, false),
            };
            let en =
                |t: &str, val: &str| Value::Enumerated { type_id: photocraft_psd::descriptor::Id::new(t), value: photocraft_psd::descriptor::Id::new(val) };
            let text = |t: &str| Value::Text(photocraft_psd::descriptor::UnicodeString::new_nul(t));
            let d = Descriptor::new("null")
                .with("lookupType", en("colorLookupType", "3DLUT"))
                .with("NM  ", text(name))
                .with("Dthr", Value::Boolean(dither))
                .with("profile", Value::RawData(Vec::new()))
                .with("LUTFormat", en("LUTFormatType", "LUTFormatCUBE"))
                .with("dataOrder", en("colorLookupOrder", "rgbOrder"))
                .with("tableOrder", en("colorLookupOrder", "bgrOrder"))
                .with("LUT3DFileData", Value::RawData(photocraft_cms::lutfile::write_cube(&file).into_bytes()))
                .with("LUT3DFileName", text(name));
            put16(&mut v, 1);
            v.extend_from_slice(&VersionedDescriptor::new(d).to_bytes());
            return vec![(*b"clrL", v)];
        }
        Adjustment::GradientMap { stops, reverse, dither } => {
            // Photoshop needs two stops; fewer are written as the gradient they render as
            // (none: black to white, i.e. the gray value itself; one: that colour throughout).
            let stops = match stops.as_slice() {
                [] => vec![(0.0, [0.0; 3]), (1.0, [1.0; 3])],
                [(_, c)] => vec![(0.0, *c), (1.0, *c)],
                s => s.to_vec(),
            };
            return vec![(*b"grdm", write_gradient_map(&stops, *reverse, *dither))];
        }
        Adjustment::Vibrance { vibrance, saturation } => {
            let d = Descriptor::new("null")
                .with("vibrance", Value::Integer(i32::from(clamp_i16(vibrance.clamp(-100.0, 100.0)))))
                .with("Strt", Value::Integer(i32::from(clamp_i16(saturation.clamp(-100.0, 100.0)))));
            return vec![(*b"vibA", VersionedDescriptor::new(d).to_bytes())];
        }
        Adjustment::ColorBalance { shadows, midtones, highlights, preserve_luminosity } => {
            for x in shadows.iter().chain(midtones).chain(highlights) {
                v.extend_from_slice(&clamp_i16(x.clamp(-100.0, 100.0)).to_be_bytes());
            }
            v.push(u8::from(*preserve_luminosity));
            v.push(0);
            return vec![(*b"blnc", v)];
        }
        Adjustment::BlackWhite { weights, tint } => {
            let mut d = Descriptor::new("null");
            for (w, k) in weights.iter().zip(BW_KEYS) {
                d = d.with(k, Value::Integer(i32::from(clamp_i16(w.clamp(-200.0, 300.0)))));
            }
            let d = d
                .with("useTint", Value::Boolean(tint.is_some()))
                .with("tintColor", Value::Descriptor(rgb_desc(tint.unwrap_or(BW_TINT))))
                .with("bwPresetKind", Value::Integer(1))
                .with("blackAndWhitePresetFileName", Value::Text(photocraft_psd::descriptor::UnicodeString::new_nul("")));
            return vec![(*b"blwh", VersionedDescriptor::new(d).to_bytes())];
        }
        Adjustment::PhotoFilter { color, density, preserve_luminosity } => {
            // Version 2: an RGB colour structure keeps the colour at 16 bits.
            put16(&mut v, 2);
            put16(&mut v, 0);
            for x in color {
                put16(&mut v, (x.clamp(0.0, 1.0) * 65535.0).round() as u16);
            }
            put16(&mut v, 0);
            v.extend_from_slice(&((density.clamp(0.0, 1.0) * 100.0).round() as u32).to_be_bytes());
            v.push(u8::from(*preserve_luminosity));
            // Photoshop requires the 17-byte Photo Filter payload after the version to be
            // padded to a 4-byte boundary INSIDE the tagged block's declared length.
            // A single pad byte made an 18-byte `phfl` block that Photoshop could not open
            // even though PhotoCraft and other tolerant readers accepted it (#1455).
            v.extend_from_slice(&[0, 0, 0]);
            return vec![(*b"phfl", v)];
        }
        Adjustment::ChannelMixer { matrix, monochrome } => {
            put16(&mut v, 1);
            put16(&mut v, u16::from(*monochrome));
            // Red, green, blue, then the gray record (the first row, which monochrome uses).
            for row in matrix.iter().chain(std::iter::once(&matrix[0])) {
                let pct = |x: f32| clamp_i16((x * 100.0).clamp(-200.0, 200.0)).to_be_bytes();
                for x in &row[..3] {
                    v.extend_from_slice(&pct(*x));
                }
                v.extend_from_slice(&[0, 0]);
                v.extend_from_slice(&pct(row[3]));
            }
            return vec![(*b"mixr", v)];
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
