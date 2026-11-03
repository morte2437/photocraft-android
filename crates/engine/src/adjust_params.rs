//! Full parameter sets for the per-pixel adjustment kinds, shared by the destructive
//! `image.adjustments.*` commands, `layer.newAdjustmentLayer.*` and `layer.setAdjustment`.
//!
//! [`from_params`] reads params over a base adjustment (missing keys keep the base's value, so a
//! partial update only changes what it names) and [`to_params`] writes the complete set back, so
//! `from_params(kind, &to_params(a), None, mode) == a`. Values out of range are clamped like
//! Photoshop's fields; a key of the wrong type is an error.
//!
//! Units: Levels and Curves in 0–255 levels; percentages for the rest (as in Photoshop's dialogs);
//! colours as `"#rrggbb"` or `[r, g, b]` in 0–1.

use photocraft_color::ColorMode;
use photocraft_doc::Adjustment;
use photocraft_doc::adjust::{CURVE_MAP_LEN, CurvePoint, HueRange, LevelsChannel, ToneSpace, curve_map, curve_map_outputs};
use serde_json::{Map, Value, json};

use crate::{EngineError, Result};

/// Hue/Saturation and Black & White colour-range keys (storage order).
pub const HUE_RANGES: [&str; 6] = ["reds", "yellows", "greens", "cyans", "blues", "magentas"];

/// Black & White default weights (percent) for [`HUE_RANGES`].
pub const BW_DEFAULTS: [f32; 6] = [40.0, 60.0, 40.0, 60.0, 20.0, 80.0];

/// Photo Filter presets `(id, label, colour)`: common photographic filter colours.
pub const PHOTO_FILTERS: &[(&str, &str, [u8; 3])] = &[
    ("warming85", "Warming Filter (85)", [236, 138, 0]),
    ("warmingLBA", "Warming Filter (LBA)", [250, 150, 0]),
    ("warming81", "Warming Filter (81)", [235, 177, 19]),
    ("cooling80", "Cooling Filter (80)", [0, 109, 255]),
    ("coolingLBB", "Cooling Filter (LBB)", [0, 93, 255]),
    ("cooling82", "Cooling Filter (82)", [0, 181, 255]),
    ("red", "Red", [234, 26, 26]),
    ("orange", "Orange", [243, 132, 23]),
    ("yellow", "Yellow", [249, 227, 28]),
    ("green", "Green", [25, 201, 25]),
    ("cyan", "Cyan", [29, 203, 234]),
    ("blue", "Blue", [29, 53, 234]),
    ("violet", "Violet", [155, 29, 234]),
    ("magenta", "Magenta", [227, 24, 227]),
    ("sepia", "Sepia", [172, 122, 51]),
    ("deepRed", "Deep Red", [255, 0, 0]),
    ("deepBlue", "Deep Blue", [0, 34, 205]),
    ("deepEmerald", "Deep Emerald", [0, 140, 0]),
    ("deepYellow", "Deep Yellow", [255, 213, 0]),
    ("underwater", "Underwater", [0, 194, 177]),
];

/// Per-channel keys of Levels/Curves in each tone space (after the composite).
pub fn channel_keys(space: ToneSpace) -> &'static [&'static str] {
    match space {
        ToneSpace::Rgb => &["red", "green", "blue"],
        ToneSpace::Cmyk => &["cyan", "magenta", "yellow", "black"],
        ToneSpace::Lab => &["lightness", "a", "b"],
    }
}

/// The tone space new Levels/Curves get in a document of `mode`.
pub fn default_space(mode: ColorMode) -> ToneSpace {
    match mode {
        ColorMode::Cmyk => ToneSpace::Cmyk,
        ColorMode::Lab => ToneSpace::Lab,
        _ => ToneSpace::Rgb,
    }
}

/// The neutral adjustment of `kind` (what "Reset to defaults" restores).
pub fn default_for(kind: &str, mode: ColorMode) -> Result<Adjustment> {
    from_params(kind, &json!({}), None, mode)
}

/// Reads the adjustment of `kind` from `p` over `base` (or the kind's defaults when `base` is
/// None or of another kind). `mode` picks the Levels/Curves channel space when neither the params
/// nor the base name one.
pub fn from_params(kind: &str, p: &Value, base: Option<&Adjustment>, mode: ColorMode) -> Result<Adjustment> {
    let cmd = format!("{kind} adjustment");
    if !p.is_object() && !p.is_null() {
        return Err(bad(&cmd, "params must be an object"));
    }
    let r = Reader { cmd: &cmd, p };
    Ok(match kind {
        "invert" => Adjustment::Invert,
        "brightnessContrast" => {
            let (b0, c0, l0) = match base {
                Some(Adjustment::BrightnessContrast { brightness, contrast, legacy }) => (*brightness, *contrast, *legacy),
                _ => (0.0, 0.0, false),
            };
            let legacy = r.boolean("legacy", l0)?;
            let (lo, hi) = if legacy { (-100.0, 100.0) } else { (-50.0, 100.0) };
            Adjustment::BrightnessContrast { brightness: r.num("brightness", b0, -150.0, 150.0)?, contrast: r.num("contrast", c0, lo, hi)?, legacy }
        }
        "threshold" => {
            let l0 = match base {
                Some(Adjustment::Threshold { level }) => level * 255.0,
                _ => 128.0,
            };
            Adjustment::Threshold { level: r.num("level", l0, 1.0, 255.0)?.round() / 255.0 }
        }
        "posterize" => {
            let l0 = match base {
                Some(Adjustment::Posterize { levels }) => *levels as f32,
                _ => 4.0,
            };
            Adjustment::Posterize { levels: r.num("levels", l0, 2.0, 255.0)?.round() as u32 }
        }
        "exposure" => {
            let (e, o, g) = match base {
                Some(Adjustment::Exposure { exposure, offset, gamma }) => (*exposure, *offset, *gamma),
                _ => (0.0, 0.0, 1.0),
            };
            Adjustment::Exposure { exposure: r.num("exposure", e, -20.0, 20.0)?, offset: r.num("offset", o, -0.5, 0.5)?, gamma: r.num("gamma", g, 0.01, 9.99)? }
        }
        "vibrance" => {
            let (v, s) = match base {
                Some(Adjustment::Vibrance { vibrance, saturation }) => (*vibrance, *saturation),
                _ => (0.0, 0.0),
            };
            Adjustment::Vibrance { vibrance: r.num("vibrance", v, -100.0, 100.0)?, saturation: r.num("saturation", s, -100.0, 100.0)? }
        }
        "hueSaturation" => hue_saturation(&r, base)?,
        "levels" => levels(&r, base, mode)?,
        "curves" => curves(&r, base, mode)?,
        "colorBalance" => {
            let (mut tones, preserve) = match base {
                Some(Adjustment::ColorBalance { shadows, midtones, highlights, preserve_luminosity }) => {
                    ([*shadows, *midtones, *highlights], *preserve_luminosity)
                }
                _ => ([[0.0; 3]; 3], true),
            };
            for (i, key) in ["shadows", "midtones", "highlights"].iter().enumerate() {
                if let Some(v) = r.numbers(key, 3)? {
                    tones[i] = [v[0], v[1], v[2]].map(|x| x.clamp(-100.0, 100.0));
                }
            }
            Adjustment::ColorBalance {
                shadows: tones[0],
                midtones: tones[1],
                highlights: tones[2],
                preserve_luminosity: r.boolean("preserveLuminosity", preserve)?,
            }
        }
        "blackWhite" => {
            let (mut weights, tint0) = match base {
                Some(Adjustment::BlackWhite { weights, tint }) => (*weights, *tint),
                _ => (BW_DEFAULTS, None),
            };
            for (i, key) in HUE_RANGES.iter().enumerate() {
                weights[i] = r.num(key, weights[i], -200.0, 300.0)?;
            }
            // Photoshop's default tint colour is a light sepia.
            let color = r.color("tintColor", tint0.unwrap_or([0.882, 0.827, 0.702]))?;
            let tint = r.boolean("tint", tint0.is_some())?.then_some(color);
            Adjustment::BlackWhite { weights, tint }
        }
        "photoFilter" => {
            let (c0, d0, p0) = match base {
                Some(Adjustment::PhotoFilter { color, density, preserve_luminosity }) => (*color, *density, *preserve_luminosity),
                _ => ([236.0 / 255.0, 138.0 / 255.0, 0.0], 0.25, true),
            };
            let preset = match p.get("filter") {
                None | Some(Value::Null) => None,
                Some(Value::String(id)) => {
                    let f = PHOTO_FILTERS.iter().find(|f| f.0 == id).ok_or_else(|| bad(&cmd, format!("unknown filter `{id}`")))?;
                    Some(f.2.map(|v| f32::from(v) / 255.0))
                }
                Some(_) => return Err(bad(&cmd, "`filter` must be a preset id")),
            };
            let color = r.color("color", preset.unwrap_or(c0))?;
            Adjustment::PhotoFilter {
                color,
                density: r.num("density", d0 * 100.0, 0.0, 100.0)? / 100.0,
                preserve_luminosity: r.boolean("preserveLuminosity", p0)?,
            }
        }
        "channelMixer" => {
            let (mut matrix, mono0) = match base {
                Some(Adjustment::ChannelMixer { matrix, monochrome }) => (*matrix, *monochrome),
                _ => ([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]], false),
            };
            let monochrome = r.boolean("monochrome", mono0)?;
            if monochrome && !mono0 && p.get("gray").is_none() {
                // Turning Monochrome on starts from Photoshop's grey mix.
                matrix[0] = [0.4, 0.4, 0.2, 0.0];
            }
            for (i, key) in ["red", "green", "blue"].iter().enumerate() {
                if let Some(v) = r.numbers(key, 4)? {
                    matrix[i] = mixer_row(&v);
                }
            }
            if let Some(v) = r.numbers("gray", 4)? {
                matrix[0] = mixer_row(&v);
            }
            Adjustment::ChannelMixer { matrix, monochrome }
        }
        "gradientMap" => {
            let (stops0, rev0, dither0) = match base {
                Some(Adjustment::GradientMap { stops, reverse, dither }) => (stops.clone(), *reverse, *dither),
                _ => (vec![(0.0, [0.0; 3]), (1.0, [1.0; 3])], false, false),
            };
            let stops = match p.get("stops") {
                None | Some(Value::Null) => stops0,
                Some(v) => gradient_stops(&cmd, v)?,
            };
            Adjustment::GradientMap { stops, reverse: r.boolean("reverse", rev0)?, dither: r.boolean("dither", dither0)? }
        }
        "selectiveColor" => crate::adjust_cmds::selective_from_params(p, base.filter(|b| matches!(b, Adjustment::SelectiveColor { .. })))?,
        "colorLookup" => crate::adjust_cmds::lookup_from_params(p, base.filter(|b| matches!(b, Adjustment::ColorLookup { .. })))?,
        other => return Err(bad("adjustment", format!("unknown adjustment kind `{other}`"))),
    })
}

/// The complete parameter set of `adj` (the inverse of [`from_params`]).
pub fn to_params(adj: &Adjustment) -> Value {
    match adj {
        Adjustment::Invert | Adjustment::Unsupported { .. } => json!({}),
        Adjustment::BrightnessContrast { brightness, contrast, legacy } => json!({"brightness": brightness, "contrast": contrast, "legacy": legacy}),
        Adjustment::Threshold { level } => json!({"level": (level * 255.0).round()}),
        Adjustment::Posterize { levels } => json!({"levels": levels}),
        Adjustment::Exposure { exposure, offset, gamma } => json!({"exposure": exposure, "offset": offset, "gamma": gamma}),
        Adjustment::Vibrance { vibrance, saturation } => json!({"vibrance": vibrance, "saturation": saturation}),
        Adjustment::HueSaturation { hue, saturation, lightness, colorize, ranges } => {
            let mut v = json!({"hue": hue, "saturation": saturation, "lightness": lightness, "colorize": colorize});
            for (key, r) in HUE_RANGES.iter().zip(ranges.iter()) {
                v[*key] = json!({"hue": r.hue, "saturation": r.saturation, "lightness": r.lightness, "range": r.bounds});
            }
            v
        }
        Adjustment::Levels { master, per_channel, space, black } => {
            let mut v = levels_obj(master);
            for (key, c) in channel_keys(*space).iter().zip([&per_channel[0], &per_channel[1], &per_channel[2], black]) {
                v[*key] = levels_obj(c);
            }
            if *space == ToneSpace::Lab {
                // No composite in Lab: the top-level keys are the lightness channel.
                v = merge(levels_obj(&per_channel[0]), v);
            }
            v
        }
        Adjustment::Curves { master, per_channel, space, black, pencil } => {
            let arr = |c: &[CurvePoint]| if *pencil { map_arr(c) } else { curve_arr(c) };
            let mut v = json!({"points": arr(master)});
            let ink = if black.len() >= 2 { black.clone() } else { identity_curve() };
            let chans = per_channel.iter().cloned().chain(std::iter::once(ink));
            for (key, c) in channel_keys(*space).iter().zip(chans) {
                v[*key] = arr(&c);
            }
            if *space == ToneSpace::Lab {
                v["points"] = arr(&per_channel[0]);
            }
            if *pencil {
                v["pencil"] = json!(true);
            }
            v
        }
        Adjustment::ColorBalance { shadows, midtones, highlights, preserve_luminosity } => {
            json!({"shadows": shadows, "midtones": midtones, "highlights": highlights, "preserveLuminosity": preserve_luminosity})
        }
        Adjustment::BlackWhite { weights, tint } => {
            let mut v = json!({"tint": tint.is_some(), "tintColor": hex(tint.unwrap_or([0.882, 0.827, 0.702]))});
            for (key, w) in HUE_RANGES.iter().zip(weights.iter()) {
                v[*key] = json!(w);
            }
            v
        }
        Adjustment::PhotoFilter { color, density, preserve_luminosity } => {
            json!({"color": hex(*color), "density": (density * 1000.0).round() / 10.0, "preserveLuminosity": preserve_luminosity})
        }
        Adjustment::ChannelMixer { matrix, monochrome } => {
            let row = |r: &[f32; 4]| json!(r.map(|x| (x * 1000.0).round() / 10.0));
            let mut v = json!({"red": row(&matrix[0]), "green": row(&matrix[1]), "blue": row(&matrix[2]), "monochrome": monochrome});
            if *monochrome {
                v["gray"] = row(&matrix[0]);
            }
            v
        }
        Adjustment::GradientMap { stops, reverse, dither } => {
            json!({"stops": stops.iter().map(|(t, c)| json!([t, hex(*c)])).collect::<Vec<_>>(), "reverse": reverse, "dither": dither})
        }
        Adjustment::SelectiveColor { relative, adjustments } => {
            let mut v = json!({"method": if *relative { "relative" } else { "absolute" }});
            for (i, key) in crate::adjust_cmds::RANGES.iter().enumerate() {
                v[*key] = json!(adjustments[i]);
            }
            v
        }
        Adjustment::ColorLookup { tetrahedral, dither, .. } => {
            json!({"interpolation": if *tetrahedral { "tetrahedral" } else { "trilinear" }, "dither": dither})
        }
    }
}

// ---------------------------------------------------------------------------------------------

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// Typed, clamped access to optional params.
struct Reader<'a> {
    cmd: &'a str,
    p: &'a Value,
}

impl Reader<'_> {
    fn get(&self, key: &str) -> Option<&Value> {
        self.p.get(key).filter(|v| !v.is_null())
    }

    fn num(&self, key: &str, default: f32, lo: f32, hi: f32) -> Result<f32> {
        match self.get(key) {
            None => Ok(default.clamp(lo, hi)),
            Some(v) => num_in(self.cmd, key, v, lo, hi),
        }
    }

    fn boolean(&self, key: &str, default: bool) -> Result<bool> {
        match self.get(key) {
            None => Ok(default),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err(bad(self.cmd, format!("`{key}` must be true or false"))),
        }
    }

    /// An array of exactly `n` numbers.
    fn numbers(&self, key: &str, n: usize) -> Result<Option<Vec<f32>>> {
        let Some(v) = self.get(key) else { return Ok(None) };
        let a = v.as_array().filter(|a| a.len() == n).ok_or_else(|| bad(self.cmd, format!("`{key}` must be an array of {n} numbers")))?;
        a.iter().map(|x| num_in(self.cmd, key, x, -1e6, 1e6)).collect::<Result<Vec<_>>>().map(Some)
    }

    fn color(&self, key: &str, default: [f32; 3]) -> Result<[f32; 3]> {
        match self.get(key) {
            None => Ok(default),
            Some(v) => color_of(self.cmd, key, v),
        }
    }

    /// A channel object (`{"inBlack": …}`) or None.
    fn object(&self, key: &str) -> Result<Option<Reader<'_>>> {
        match self.get(key) {
            None => Ok(None),
            Some(v) if v.is_object() => Ok(Some(Reader { cmd: self.cmd, p: v })),
            Some(_) => Err(bad(self.cmd, format!("`{key}` must be an object"))),
        }
    }
}

fn num_in(cmd: &str, key: &str, v: &Value, lo: f32, hi: f32) -> Result<f32> {
    let x = v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| bad(cmd, format!("`{key}` must be a number")))?;
    Ok((x as f32).clamp(lo, hi))
}

fn color_of(cmd: &str, key: &str, v: &Value) -> Result<[f32; 3]> {
    let err = || bad(cmd, format!("`{key}` must be \"#rrggbb\" or [r, g, b] in 0..1"));
    match v {
        Value::String(s) => {
            let h = s.trim_start_matches('#');
            let b = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|x| f32::from(x) / 255.0);
            match (h.len(), b(0), b(2), b(4)) {
                (6 | 8, Some(r), Some(g), Some(bb)) => Ok([r, g, bb]),
                _ => Err(err()),
            }
        }
        Value::Array(a) if a.len() >= 3 => {
            let c = |i: usize| a.get(i).and_then(Value::as_f64).filter(|x| x.is_finite()).map(|x| (x as f32).clamp(0.0, 1.0)).ok_or_else(err);
            Ok([c(0)?, c(1)?, c(2)?])
        }
        _ => Err(err()),
    }
}

pub fn hex(c: [f32; 3]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

fn merge(mut a: Value, b: Value) -> Value {
    if let (Some(ao), Value::Object(bo)) = (a.as_object_mut(), b) {
        for (k, v) in bo {
            ao.entry(k).or_insert(v);
        }
    }
    a
}

fn mixer_row(v: &[f32]) -> [f32; 4] {
    std::array::from_fn(|i| v.get(i).copied().unwrap_or(0.0).clamp(-200.0, 200.0) / 100.0)
}

fn hue_saturation(r: &Reader<'_>, base: Option<&Adjustment>) -> Result<Adjustment> {
    let (h, s, l, c, mut ranges) = match base {
        Some(Adjustment::HueSaturation { hue, saturation, lightness, colorize, ranges }) => (*hue, *saturation, *lightness, *colorize, ranges.clone()),
        _ => (0.0, 0.0, 0.0, false, HueRange::defaults()),
    };
    let colorize = r.boolean("colorize", c)?;
    // Colorize hue is 0..360, a shift is -180..180.
    let hue = if colorize { r.num("hue", h.rem_euclid(360.0), 0.0, 360.0)? } else { r.num("hue", h, -180.0, 180.0)? };
    // Turning Colorize on starts at 25% saturation unless one is given, like the dialog's
    // checkbox and Photoshop; an explicit 0 stays 0, which colorizes to grey (#2326).
    let s = if colorize && !c { s.max(25.0) } else { s };
    let (saturation, lightness) = (r.num("saturation", s, if colorize { 0.0 } else { -100.0 }, 100.0)?, r.num("lightness", l, -100.0, 100.0)?);
    for (i, key) in HUE_RANGES.iter().enumerate() {
        let Some(o) = r.object(key)? else { continue };
        let Some(range) = ranges.get_mut(i) else { continue };
        range.hue = o.num("hue", range.hue, -180.0, 180.0)?;
        range.saturation = o.num("saturation", range.saturation, -100.0, 100.0)?;
        range.lightness = o.num("lightness", range.lightness, -100.0, 100.0)?;
        if let Some(b) = o.numbers("range", 4)? {
            // Keep the four sliders ordered and within one turn of the first.
            range.bounds = HueRange::canonical_bounds([b[0], b[1], b[2], b[3]]);
        }
    }
    Ok(Adjustment::HueSaturation { hue, saturation, lightness, colorize, ranges })
}

/// Which tone space the params address: any channel key of a space picks it.
fn space_of(r: &Reader<'_>, base: Option<ToneSpace>, mode: ColorMode) -> ToneSpace {
    let has = |keys: &[&str]| keys.iter().any(|k| r.get(k).is_some());
    if has(&["cyan", "magenta", "yellow", "black"]) {
        ToneSpace::Cmyk
    } else if has(&["a", "b"]) || (has(&["lightness"]) && !has(&["red", "green", "blue"])) {
        ToneSpace::Lab
    } else if has(&["red", "green", "blue"]) {
        ToneSpace::Rgb
    } else {
        base.unwrap_or_else(|| default_space(mode))
    }
}

fn levels(r: &Reader<'_>, base: Option<&Adjustment>, mode: ColorMode) -> Result<Adjustment> {
    let (mut master, mut chans, base_space) = match base {
        Some(Adjustment::Levels { master, per_channel, space, black }) => {
            (master.clone(), [per_channel[0].clone(), per_channel[1].clone(), per_channel[2].clone(), black.clone()], Some(*space))
        }
        _ => (LevelsChannel::default(), Default::default(), None),
    };
    let space = space_of(r, base_space, mode);
    if base_space.is_some_and(|b| b != space) {
        chans = Default::default();
    }
    let read = |o: &Reader<'_>, c: &LevelsChannel| -> Result<LevelsChannel> {
        let in_black = o.num("inBlack", c.in_black * 255.0, 0.0, 253.0)?;
        let in_white = o.num("inWhite", c.in_white * 255.0, 2.0, 255.0)?.max(in_black + 2.0);
        Ok(LevelsChannel {
            in_black: in_black / 255.0,
            in_white: in_white / 255.0,
            gamma: o.num("gamma", c.gamma, 0.01, 9.99)?,
            out_black: o.num("outBlack", c.out_black * 255.0, 0.0, 255.0)? / 255.0,
            out_white: o.num("outWhite", c.out_white * 255.0, 0.0, 255.0)? / 255.0,
        })
    };
    if space == ToneSpace::Lab {
        // Top-level keys (and "gray") address lightness: Lab has no composite.
        chans[0] = read(r, &chans[0])?;
    } else {
        master = read(r, &master)?;
        if let Some(o) = r.object("gray")? {
            master = read(&o, &master)?;
        }
    }
    for (i, key) in channel_keys(space).iter().enumerate() {
        if let (Some(o), Some(c)) = (r.object(key)?, chans.get(i).cloned()) {
            chans[i] = read(&o, &c)?;
        }
    }
    let [c0, c1, c2, black] = chans;
    Ok(Adjustment::Levels { master, per_channel: [c0, c1, c2], space, black: if space == ToneSpace::Cmyk { black } else { LevelsChannel::default() } })
}

fn identity_curve() -> Vec<CurvePoint> {
    vec![CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 1.0, output: 1.0 }]
}

const CURVE_COORDINATE_PRECISION: f32 = 100.0;
const CURVE_POINT_DUPLICATE_DISTANCE: f32 = 0.5 / 255.0;

fn curve_arr(c: &[CurvePoint]) -> Value {
    let c = if c.len() >= 2 { c.to_vec() } else { identity_curve() };
    json!(
        c.iter()
            .map(|p| {
                [
                    (p.input * 255.0 * CURVE_COORDINATE_PRECISION).round() / CURVE_COORDINATE_PRECISION,
                    (p.output * 255.0 * CURVE_COORDINATE_PRECISION).round() / CURVE_COORDINATE_PRECISION,
                ]
            })
            .collect::<Vec<_>>()
    )
}

/// Most points a curve may have (Photoshop allows 16; the PSD record holds 19).
pub const MAX_CURVE_POINTS: usize = 19;

/// Points Photoshop's "Edit points to modify the curve" places on a pencil-drawn curve at most.
const MAX_FITTED_POINTS: usize = 16;

/// Most Smooth passes one `smooth` param applies.
pub const MAX_SMOOTH_PASSES: f32 = 10.0;

/// A pencil-mode curve as params: its [`CURVE_MAP_LEN`] outputs in 0..255.
fn map_arr(c: &[CurvePoint]) -> Value {
    json!(curve_map_outputs(c).iter().map(|o| (o * 255.0 * CURVE_COORDINATE_PRECISION).round() / CURVE_COORDINATE_PRECISION).collect::<Vec<_>>())
}

/// `lut` (evenly spaced over 0..=1) at `x`, linearly interpolated.
fn lut_at(lut: &[f32], x: f32) -> f32 {
    let Some(last) = lut.len().checked_sub(1) else { return x };
    let f = x.clamp(0.0, 1.0) * last as f32;
    let i = (f.floor() as usize).min(last);
    let (a, b) = (lut.get(i).copied().unwrap_or(x), lut.get((i + 1).min(last)).copied().unwrap_or(x));
    a + (b - a) * (f - i as f32)
}

/// A point curve drawn into a pencil-mode map (Photoshop switching to "Draw to modify the curve").
pub fn points_to_map(points: &[CurvePoint]) -> Vec<CurvePoint> {
    let lut = photocraft_compose::adjust::curve_lut(points);
    let last = (CURVE_MAP_LEN - 1) as f32;
    curve_map(&(0..CURVE_MAP_LEN).map(|k| lut_at(&lut, k as f32 / last)).collect::<Vec<_>>())
}

/// A pencil-mode map turned back into a point curve (Photoshop's "Edit points to modify the
/// curve"): the endpoints, then the worst-fitting input until the spline is within one level of
/// the map or [`MAX_FITTED_POINTS`] are placed.
pub fn map_to_points(map: &[CurvePoint]) -> Vec<CurvePoint> {
    let target = curve_map_outputs(map);
    let last = CURVE_MAP_LEN - 1;
    let at = |k: usize| CurvePoint { input: k as f32 / last as f32, output: target.get(k).copied().unwrap_or(0.0) };
    let mut pts = vec![at(0), at(last)];
    while pts.len() < MAX_FITTED_POINTS {
        let lut = photocraft_compose::adjust::curve_lut(&pts);
        let worst = (0..CURVE_MAP_LEN)
            .map(|k| (k, (lut_at(&lut, k as f32 / last as f32) - target.get(k).copied().unwrap_or(0.0)).abs()))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        let Some((k, _)) = worst.filter(|w| w.1 > 1.0 / 255.0) else { break };
        let p = at(k);
        if pts.iter().any(|q| (q.input - p.input).abs() < CURVE_POINT_DUPLICATE_DISTANCE) {
            break;
        }
        pts.push(p);
        pts.sort_by(|a, b| a.input.total_cmp(&b.input));
    }
    pts
}

/// One pass of Curves' Smooth button over pencil-mode outputs (any scale): each value becomes
/// the mean of a symmetric window of up to three neighbours a side, so straight runs and the two
/// endpoints stay put while jagged strokes even out.
pub fn smooth_curve_map(outputs: &mut [f32]) {
    const RADIUS: usize = 3;
    let src = outputs.to_vec();
    let n = src.len();
    for (k, out) in outputs.iter_mut().enumerate() {
        let r = RADIUS.min(k).min(n.saturating_sub(k + 1));
        if r == 0 {
            continue;
        }
        let window = src.get(k - r..=k + r).unwrap_or_default();
        *out = window.iter().sum::<f32>() / window.len().max(1) as f32;
    }
}

/// Whether a pencil-mode curve maps every level to itself.
fn is_identity_map(c: &[CurvePoint]) -> bool {
    let last = (CURVE_MAP_LEN - 1) as f32;
    curve_map_outputs(c).iter().enumerate().all(|(k, o)| (o - k as f32 / last).abs() < 0.5 / 255.0)
}

/// Fixed-target Curves eyedroppers. The UI keeps the sampled operation provisional; this helper
/// only resolves a sample into an ordinary Curves adjustment that the existing command can apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurvesEyedropper {
    Black,
    NeutralGray,
    White,
}

/// Verify that serializing and parsing `points` keeps every point and the requested anchor.
fn curve_round_trip_keeps_anchor(points: &[CurvePoint], input: f32, output: f32) -> bool {
    let Ok(parsed) = parse_curve("curves eyedropper", "curve", &curve_arr(points)) else { return false };
    if parsed.len() != points.len() {
        return false;
    }
    let serialized = |value: f32| ((value * 255.0 * CURVE_COORDINATE_PRECISION).round() / CURVE_COORDINATE_PRECISION) / 255.0;
    let (input, output) = (serialized(input), serialized(output));
    parsed.iter().any(|point| (point.input - input).abs() < 1e-6 && (point.output - output).abs() < 1e-6)
}

fn move_curve_endpoint(points: &mut [CurvePoint], sample: f32, black: bool) {
    let sample = sample.clamp(0.0, 1.0);
    let mut candidate = points.to_vec();
    if black {
        for p in &mut candidate {
            p.input = sample + p.input * (1.0 - sample);
        }
        if let Some(first) = candidate.first_mut() {
            first.input = sample;
            first.output = 0.0;
        }
    } else {
        for p in &mut candidate {
            p.input *= sample;
        }
        if let Some(last) = candidate.last_mut() {
            last.input = sample;
            last.output = 1.0;
        }
    }

    // `curve_arr` stores hundredths of a 0..255 level, while `parse_curve` drops points
    // separated by less than half a level. Scaling an endpoint toward the sample can collapse
    // several edited points together even when the target endpoint itself survives. Check that
    // the full curve survives the actual serializer and parser.
    let target = if black { 0.0 } else { 1.0 };
    if curve_round_trip_keeps_anchor(&candidate, sample, target) {
        points.clone_from_slice(&candidate);
        return;
    }

    // If scaling collapses any points, keep the user's point positions and edits, and make the
    // two points nearest the selected endpoint a target plateau. Near an endpoint sample, those
    // two points bracket the sample; the plateau is representable and leaves the rest of the
    // curve intact.
    if black {
        for point in points.iter_mut().rev().take(2) {
            point.output = 0.0;
        }
    } else {
        for point in points.iter_mut().take(2) {
            point.output = 1.0;
        }
    }
}

fn anchor_curve(points: &mut Vec<CurvePoint>, input: f32, output: f32) {
    let (input, output) = (input.clamp(0.0, 1.0), output.clamp(0.0, 1.0));
    let mut candidate = points.clone();
    if let Some(point) = candidate.iter_mut().find(|p| (p.input - input).abs() < CURVE_POINT_DUPLICATE_DISTANCE) {
        point.input = input;
        point.output = output;
    } else if candidate.len() < MAX_CURVE_POINTS {
        candidate.push(CurvePoint { input, output });
    } else if let Some((index, _)) = candidate
        .iter()
        .enumerate()
        .filter(|(index, _)| *index > 0 && *index + 1 < candidate.len())
        .min_by(|(_, a), (_, b)| (a.input - input).abs().total_cmp(&(b.input - input).abs()))
        && let Some(point) = candidate.get_mut(index)
    {
        *point = CurvePoint { input, output };
    }
    candidate.sort_by(|a, b| a.input.total_cmp(&b.input));
    if curve_round_trip_keeps_anchor(&candidate, input, output) {
        *points = candidate;
        return;
    }

    // A channel sample can land between two points only half a level apart. Moving one point to
    // the sample would then make the parser collapse the pair, so retain their positions and make
    // the bracketing pair a deterministic neutral plateau instead.
    let Some(last) = points.len().checked_sub(1) else { return };
    let split = points.partition_point(|point| point.input < input);
    let mut lower = split.saturating_sub(1).min(last);
    let mut upper = split.min(last);
    if lower == upper {
        if upper < last {
            upper += 1;
        } else {
            lower = lower.saturating_sub(1);
        }
    }
    if let Some(point) = points.get_mut(lower) {
        point.output = output;
    }
    if let Some(point) = points.get_mut(upper) {
        point.output = output;
    }
}

/// Rec. 709 luma in the sampled display-RGB values. This is the neutral level the gray picker
/// should preserve when it removes a colour cast.
fn rgb_luma(rgb: [f32; 3]) -> f32 {
    rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
}

/// Find the master-curve input whose output is closest to `target`. The master curve is kept
/// intact by the gray picker, so channel curves must target its inverse to preserve final luma.
/// A non-monotone or flat curve may not have an exact inverse; the first nearest LUT entry gives
/// a deterministic best fit.
fn inverse_master_target(master: &[CurvePoint], target: f32) -> f32 {
    if photocraft_doc::adjust::is_identity_curve(master) {
        return target;
    }
    let lut = photocraft_compose::adjust::curve_lut(master);
    let Some(denominator) = lut.len().checked_sub(1).filter(|last| *last > 0) else { return target };
    let mut best_input = target;
    let mut best_error = f32::INFINITY;
    for (index, value) in lut.iter().enumerate() {
        let error = (*value - target).abs();
        if error < best_error {
            best_error = error;
            best_input = index as f32 / denominator as f32;
        }
    }
    best_input
}

/// The sample and colour modes both eyedroppers (Levels, Curves) support: RGB, and Grayscale
/// without the Neutral Gray picker.
fn check_eyedropper(cmd: &str, picker: CurvesEyedropper, sample: [f32; 3], mode: ColorMode) -> Result<()> {
    if sample.iter().any(|v| !v.is_finite()) {
        return Err(bad(cmd, "sample components must be finite"));
    }
    if !matches!(mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return Err(bad(cmd, "only RGB and Grayscale documents are supported"));
    }
    if mode == ColorMode::Grayscale && picker == CurvesEyedropper::NeutralGray {
        return Err(bad(cmd, "Neutral Gray is unavailable for Grayscale documents"));
    }
    Ok(())
}

/// The smallest input range Levels allows (two levels), as the parser enforces.
const LEVELS_MIN_RANGE: f32 = 2.0 / 255.0;

/// Moves the input black (or white) of `c` to `sample`, so the sample maps to the output black
/// (white). The other input end moves away if it would leave less than two levels between them.
fn set_levels_input(c: &mut LevelsChannel, sample: f32, black: bool) {
    if black {
        c.in_black = sample.clamp(0.0, 1.0 - LEVELS_MIN_RANGE);
        c.in_white = c.in_white.max(c.in_black + LEVELS_MIN_RANGE).min(1.0);
    } else {
        c.in_white = sample.clamp(LEVELS_MIN_RANGE, 1.0);
        c.in_black = c.in_black.min(c.in_white - LEVELS_MIN_RANGE).max(0.0);
    }
}

/// Sets the midtone gamma of `c` so `sample` maps to `target` (both 0..1). Bisection over the
/// gamma range on the compositor's own Levels curve (monotone in gamma), so the shadow toe is
/// accounted for. A sample at or outside the input range, or a target outside the output range,
/// can't be reached by gamma and leaves `c` unchanged.
fn set_levels_gamma(c: &mut LevelsChannel, sample: f32, target: f32) {
    let u = (sample - c.in_black) / (c.in_white - c.in_black).max(1e-6);
    let span = c.out_white - c.out_black;
    let t = (target - c.out_black) / span;
    if !(u > 0.0 && u < 1.0) || span.abs() < 1e-6 || !(t > 0.0 && t < 1.0) {
        return;
    }
    let reached = |gamma: f32| {
        let trial = LevelsChannel { gamma, ..c.clone() };
        (photocraft_compose::adjust::levels(&trial, sample) - c.out_black) / span
    };
    let (mut lo, mut hi) = (0.01f32.ln(), 9.99f32.ln());
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if reached(mid.exp()) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    c.gamma = ((lo + hi) / 2.0).exp().clamp(0.01, 9.99);
}

/// Photoshop's Levels eyedroppers on the Levels in `base` (params, as for `from_params`), from a
/// colour sample of the image the Levels adjust. RGB: Set Black/White Point move each channel's
/// input black/white to the sample's value in that channel, so the sample maps to the output
/// black/white; Set Neutral Gray Point sets each channel's midtone gamma so the channel maps to
/// the sample's luma, removing the cast while keeping its brightness. The composite record is
/// kept. Grayscale: Black/White move the composite input; there is no gray picker.
pub fn levels_eyedropper(base: &Value, picker: CurvesEyedropper, sample: [f32; 3], mode: ColorMode) -> Result<Adjustment> {
    const CMD: &str = "levels eyedropper";
    check_eyedropper(CMD, picker, sample, mode)?;
    let mut adjustment = from_params("levels", base, None, mode)?;
    let Adjustment::Levels { master, per_channel, space, .. } = &mut adjustment else {
        return Err(bad(CMD, "expected a Levels adjustment"));
    };
    if *space != ToneSpace::Rgb {
        return Err(bad(CMD, "only RGB channel Levels are supported"));
    }
    let sample = sample.map(|v| v.clamp(0.0, 1.0));
    if mode == ColorMode::Grayscale {
        set_levels_input(master, (sample[0] + sample[1] + sample[2]) / 3.0, picker == CurvesEyedropper::Black);
        return Ok(adjustment);
    }
    match picker {
        CurvesEyedropper::Black | CurvesEyedropper::White => {
            for (c, v) in per_channel.iter_mut().zip(sample) {
                set_levels_input(c, v, picker == CurvesEyedropper::Black);
            }
        }
        CurvesEyedropper::NeutralGray => {
            let target = rgb_luma(sample);
            for (c, v) in per_channel.iter_mut().zip(sample) {
                set_levels_gamma(c, v, target);
            }
        }
    }
    Ok(adjustment)
}

/// The eyedropper of adjustment `kind` ("levels" or "curves"): [`levels_eyedropper`] or
/// [`curves_eyedropper`].
pub fn adjustment_eyedropper(kind: &str, base: &Value, picker: CurvesEyedropper, sample: [f32; 3], mode: ColorMode) -> Result<Adjustment> {
    match kind {
        "levels" => levels_eyedropper(base, picker, sample, mode),
        "curves" => curves_eyedropper(base, picker, sample, mode),
        _ => Err(bad(EYEDROPPER, "`eyedropper` applies to Levels and Curves only")),
    }
}

/// Resolve a merged-composite colour sample into the black, luma-preserving neutral-gray or white target.
/// RGB uses the three channel curves. Grayscale uses the composite curve and has no gray picker.
/// Other colour models stay unsupported until their native channel semantics can be sampled.
pub fn curves_eyedropper(base: &Value, picker: CurvesEyedropper, sample: [f32; 3], mode: ColorMode) -> Result<Adjustment> {
    check_eyedropper("curves eyedropper", picker, sample, mode)?;
    // The pickers place points: a pencil-drawn curve is first fitted with points, as when
    // Photoshop leaves the pencil for point editing.
    let mut base = base.clone();
    if let Some(o) = base.as_object_mut() {
        o.remove("smooth");
        if o.get("pencil").and_then(Value::as_bool) == Some(true) {
            o.insert("pencil".into(), json!(false));
        }
    }
    let mut adjustment = from_params("curves", &base, None, mode)?;
    let Adjustment::Curves { master, per_channel, .. } = &mut adjustment else {
        return Err(bad("curves eyedropper", "expected a Curves adjustment"));
    };
    let sample = sample.map(|v| v.clamp(0.0, 1.0));
    if mode == ColorMode::Grayscale {
        let tone = (sample[0] + sample[1] + sample[2]) / 3.0;
        move_curve_endpoint(master, tone, picker == CurvesEyedropper::Black);
        return Ok(adjustment);
    }
    match picker {
        CurvesEyedropper::Black | CurvesEyedropper::White => {
            let black = picker == CurvesEyedropper::Black;
            for (curve, value) in per_channel.iter_mut().zip(sample) {
                move_curve_endpoint(curve, value, black);
            }
            if let Some(endpoint) = if black { master.first_mut() } else { master.last_mut() } {
                endpoint.output = if black { 0.0 } else { 1.0 };
            }
        }
        CurvesEyedropper::NeutralGray => {
            // Use sampled luma instead of a fixed mid-gray so removing a cast keeps the sample's
            // brightness. The per-channel curves precede the composite curve, so invert the
            // existing master curve while preserving it.
            let target = inverse_master_target(master, rgb_luma(sample));
            for (curve, value) in per_channel.iter_mut().zip(sample) {
                anchor_curve(curve, value, target);
            }
        }
    }
    Ok(adjustment)
}

/// The `eyedropper` param of the Levels and Curves commands (`kind`), so agents and the CLI can
/// drive the dialog's Set Black/Neutral Gray/White Point pickers:
/// `{"point":"black"|"gray"|"white","at":[x,y]}` samples the merged composite at document pixel
/// (x, y) (`"size"`: the Eyedropper's Sample Size, 1 = Point Sample), or `"color":[r,g,b]` (0–1)
/// gives the sample directly. The rest of `p` is the adjustment the picker edits. `None` without
/// the key.
pub fn eyedropper_from_params(s: &crate::Session, kind: &str, p: &Value) -> Result<Option<Adjustment>> {
    let Some(e) = p.get("eyedropper") else { return Ok(None) };
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let (picker, sample) = eyedropper_sample(&d.doc, None, e)?;
    let mut base = p.clone();
    if let Some(o) = base.as_object_mut() {
        o.remove("eyedropper");
    }
    adjustment_eyedropper(kind, &base, picker, sample, d.doc.mode).map(Some)
}

/// The `eyedropper` param of `layer.setAdjustment` on Levels or Curves adjustment layer `layer`:
/// the Properties panel's Set Black/Neutral Gray/White Point pickers. `at` samples the image the
/// layer adjusts (the composite with the layer hidden, like its Properties histogram), so clicking
/// the same pixel again maps it the same way. The picker edits the layer's adjustment with the
/// other keys of `p` merged over it. `None` without the key.
pub fn layer_eyedropper(s: &crate::Session, layer: photocraft_doc::LayerId, p: &Value) -> Result<Option<Adjustment>> {
    let Some(e) = p.get("eyedropper") else { return Ok(None) };
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let l = d.doc.layer(layer).ok_or(EngineError::NoLayer(layer))?;
    let (kind, adj) = match &l.content {
        photocraft_doc::LayerContent::Adjustment(adj @ Adjustment::Curves { .. }) => ("curves", adj),
        photocraft_doc::LayerContent::Adjustment(adj @ Adjustment::Levels { .. }) => ("levels", adj),
        _ => return Err(bad(EYEDROPPER, "`eyedropper` applies to Levels and Curves adjustment layers only")),
    };
    let mut rest = p.clone();
    if let Some(o) = rest.as_object_mut() {
        o.remove("eyedropper");
    }
    let current = if user_keys(&rest).is_empty() { adj.clone() } else { from_params(kind, &rest, Some(adj), d.doc.mode)? };
    let (picker, sample) = eyedropper_sample(&d.doc, Some(layer), e)?;
    adjustment_eyedropper(kind, &to_params(&current), picker, sample, d.doc.mode).map(Some)
}

const EYEDROPPER: &str = "eyedropper";

/// The picker and sample of an `eyedropper` param; `at` samples `doc` with layer `hide` hidden.
fn eyedropper_sample(doc: &photocraft_doc::Document, hide: Option<photocraft_doc::LayerId>, e: &Value) -> Result<(CurvesEyedropper, [f32; 3])> {
    const CMD: &str = EYEDROPPER;
    let picker = match e.get("point").and_then(Value::as_str) {
        Some("black") => CurvesEyedropper::Black,
        Some("gray") => CurvesEyedropper::NeutralGray,
        Some("white") => CurvesEyedropper::White,
        _ => return Err(bad(CMD, "`eyedropper.point` must be \"black\", \"gray\" or \"white\"")),
    };
    let triple = |v: &Value| -> Option<[f64; 3]> {
        let a = v.as_array().filter(|a| a.len() == 3)?;
        Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
    };
    let sample = if let Some(c) = e.get("color") {
        let c = triple(c).ok_or_else(|| bad(CMD, "`eyedropper.color` must be [r, g, b] in 0..1"))?;
        c.map(|v| v as f32)
    } else if let Some(at) = e.get("at") {
        let xy = at.as_array().filter(|a| a.len() == 2).and_then(|a| Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?]));
        let [x, y] = xy.filter(|xy| xy.iter().all(|v| v.is_finite())).ok_or_else(|| bad(CMD, "`eyedropper.at` must be [x, y] in document pixels"))?;
        let (w, h) = (f64::from(doc.size.width), f64::from(doc.size.height));
        if x < 0.0 || y < 0.0 || x >= w || y >= h {
            return Err(bad(CMD, "`eyedropper.at` is outside the image"));
        }
        let size = crate::sample_cmds::size_param(CMD, e)?;
        let all = crate::sample_cmds::SampleLayers::All;
        let [r, g, b, a] = match hide {
            Some(id) => {
                let mut below = doc.clone();
                if let Some(l) = below.layer_mut(id) {
                    l.visible = false;
                }
                crate::sample_cmds::sample_color(&below, None, x, y, size, all)
            }
            None => crate::sample_cmds::sample_color(doc, None, x, y, size, all),
        };
        if a <= 0.0 {
            return Err(bad(CMD, "the sampled pixel is transparent"));
        }
        [r, g, b]
    } else {
        return Err(bad(CMD, "`eyedropper` needs `at` [x, y] or `color` [r, g, b]"));
    };
    Ok((picker, sample))
}

/// `[[in, out], …]` in 0..255: sorted by input, duplicates collapsed, 2..=19 points.
pub fn parse_curve(cmd: &str, key: &str, v: &Value) -> Result<Vec<CurvePoint>> {
    let err = || bad(cmd, format!("`{key}` must be an array of 2..={MAX_CURVE_POINTS} [input, output] pairs in 0..255"));
    let a = v.as_array().ok_or_else(err)?;
    if a.len() < 2 || a.len() > 64 {
        return Err(err());
    }
    let mut pts = Vec::with_capacity(a.len());
    for q in a {
        let q = q.as_array().filter(|q| q.len() == 2).ok_or_else(err)?;
        // Like Photoshop, a point outside 0..255 is refused rather than moved onto the edge.
        let level = |v: Option<&Value>| v.and_then(Value::as_f64).filter(|v| (-1e-6..=255.0 + 1e-6).contains(v));
        let x = level(q.first()).ok_or_else(err)?;
        let y = level(q.get(1)).ok_or_else(err)?;
        pts.push(CurvePoint { input: (x as f32).clamp(0.0, 255.0) / 255.0, output: (y as f32).clamp(0.0, 255.0) / 255.0 });
    }
    pts.sort_by(|a, b| a.input.total_cmp(&b.input));
    pts.dedup_by(|later, earlier| (later.input - earlier.input).abs() < CURVE_POINT_DUPLICATE_DISTANCE);
    if pts.len() < 2 || pts.len() > MAX_CURVE_POINTS {
        return Err(err());
    }
    Ok(pts)
}

/// A pencil-mode map param: exactly [`CURVE_MAP_LEN`] outputs in 0..255.
fn parse_map(cmd: &str, key: &str, v: &Value) -> Result<Vec<CurvePoint>> {
    let err = || bad(cmd, format!("`{key}` must be {CURVE_MAP_LEN} output levels in 0..255 (a pencil-drawn map) or [input, output] pairs"));
    let a = v.as_array().filter(|a| a.len() == CURVE_MAP_LEN).ok_or_else(err)?;
    let outs =
        a.iter().map(|x| x.as_f64().filter(|x| x.is_finite()).map(|x| (x as f32).clamp(0.0, 255.0) / 255.0)).collect::<Option<Vec<_>>>().ok_or_else(err)?;
    Ok(curve_map(&outs))
}

/// A Curves channel param in the adjustment's mode: `[[in, out], …]` points (drawn into a map in
/// pencil mode) or a [`CURVE_MAP_LEN`]-level map (fitted with points outside pencil mode).
fn parse_curve_in(cmd: &str, key: &str, v: &Value, pencil: bool) -> Result<Vec<CurvePoint>> {
    if v.as_array().and_then(|a| a.first()).is_some_and(Value::is_number) {
        let map = parse_map(cmd, key, v)?;
        Ok(if pencil { map } else { map_to_points(&map) })
    } else {
        let pts = parse_curve(cmd, key, v)?;
        Ok(if pencil { points_to_map(&pts) } else { pts })
    }
}

fn curves(r: &Reader<'_>, base: Option<&Adjustment>, mode: ColorMode) -> Result<Adjustment> {
    let (mut master, mut chans, base_space, base_pencil) = match base {
        Some(Adjustment::Curves { master, per_channel, space, black, pencil }) => {
            (master.clone(), [per_channel[0].clone(), per_channel[1].clone(), per_channel[2].clone(), black.clone()], Some(*space), *pencil)
        }
        _ => (identity_curve(), std::array::from_fn(|_| identity_curve()), None, false),
    };
    let space = space_of(r, base_space, mode);
    if base_space.is_some_and(|b| b != space) {
        chans = std::array::from_fn(|_| identity_curve());
    }
    let pencil = r.boolean("pencil", base_pencil)?;
    // Every stored curve in the adjustment's mode: maps are drawn from the spline when switching
    // to the pencil, fitted with points when switching back, and filled out to full maps.
    let convert = |c: Vec<CurvePoint>| match (base_pencil, pencil) {
        (false, true) => points_to_map(&c),
        (true, false) => map_to_points(&c),
        (true, true) if c.len() != CURVE_MAP_LEN => curve_map(&curve_map_outputs(&c)),
        _ => c,
    };
    let [c0, c1, c2, k] = chans;
    let [m, c0, c1, c2, k] = [master, c0, c1, c2, k].map(convert);
    (master, chans) = (m, [c0, c1, c2, k]);
    let composite = r.get("points").or_else(|| r.get("gray"));
    if let Some(v) = composite {
        let c = parse_curve_in(r.cmd, "points", v, pencil)?;
        if space == ToneSpace::Lab {
            chans[0] = c;
        } else {
            master = c;
        }
    }
    for (i, key) in channel_keys(space).iter().enumerate() {
        if let Some(v) = r.get(key)
            && let Some(slot) = chans.get_mut(i)
        {
            *slot = parse_curve_in(r.cmd, key, v, pencil)?;
        }
    }
    let passes = r.num("smooth", 0.0, 0.0, MAX_SMOOTH_PASSES)?.round() as usize;
    if pencil {
        // Smoothed, then held at the params' precision so `to_params` round-trips exactly.
        let level = |o: f32| (o * 255.0 * CURVE_COORDINATE_PRECISION).round() / CURVE_COORDINATE_PRECISION / 255.0;
        for c in std::iter::once(&mut master).chain(chans.iter_mut()) {
            let mut outs = curve_map_outputs(c);
            for _ in 0..passes {
                smooth_curve_map(&mut outs);
            }
            *c = curve_map(&outs.into_iter().map(level).collect::<Vec<_>>());
        }
    }
    let [c0, c1, c2, black] = chans;
    let identity = if pencil { is_identity_map(&black) } else { photocraft_doc::adjust::is_identity_curve(&black) };
    let black = if space == ToneSpace::Cmyk && !identity { black } else { Vec::new() };
    Ok(Adjustment::Curves { master, per_channel: [c0, c1, c2], space, black, pencil })
}

fn levels_obj(c: &LevelsChannel) -> Value {
    let q = |v: f32| (v * 255.0 * 100.0).round() / 100.0;
    json!({"inBlack": q(c.in_black), "gamma": (c.gamma * 1000.0).round() / 1000.0, "inWhite": q(c.in_white), "outBlack": q(c.out_black), "outWhite": q(c.out_white)})
}

/// `[[location 0..1, colour], …]` (colour as for `color`), sorted by location.
fn gradient_stops(cmd: &str, v: &Value) -> Result<Vec<(f32, [f32; 3])>> {
    let err = || bad(cmd, "`stops` must be an array of 2..=64 [location 0..1, colour] pairs");
    let a = v.as_array().filter(|a| (2..=64).contains(&a.len())).ok_or_else(err)?;
    let mut out = Vec::with_capacity(a.len());
    for s in a {
        let (t, c) = match s {
            Value::Array(p) if p.len() == 2 => (p.first(), p.get(1)),
            Value::Object(o) => (o.get("location"), o.get("color")),
            _ => return Err(err()),
        };
        let t = t.and_then(Value::as_f64).filter(|t| t.is_finite()).ok_or_else(err)?;
        let c = color_of(cmd, "stops", c.ok_or_else(err)?)?;
        out.push(((t as f32).clamp(0.0, 1.0), c));
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(out)
}

/// Params with the private keys (`layer`, `coalesce`, `__kind`) removed.
pub fn user_keys(p: &Value) -> Map<String, Value> {
    p.as_object()
        .map(|o| {
            o.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "layer" | "coalesce" | "channel" | "targetChannel") && !k.starts_with("__"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
