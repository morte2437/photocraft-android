//! Raw development: linearize, subtract black, scale to white, white
//! balance, clip highlights, demosaic, camera → XYZ (D50) → linear ProPhoto,
//! exposure, Dehaze, Shadows / Highlights / Clarity / Texture and the black
//! point (`tone.rs`), the profile's tone curve, gamma 1.8 encode to 16 bits,
//! orientation.

use crate::color::{self, Mat3};
use crate::demosaic::{Demosaic, PAD, Padded, Pattern, demosaic, demosaic_cfa, demosaic_cfa_fused, demosaic_fused, for_bands};
use crate::error::{RawError, Result};
use crate::look::{CameraLook, ToneCurve};
use crate::sensor::Sensor;
use crate::tone::{Haze, Tone, ToneMap, black_level};
use crate::{Limits, RawFormat, par};

/// White balance choice.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum WhiteBalance {
    /// The camera's as-shot white balance when the file records one, else automatic.
    #[default]
    AsShot,
    /// Grey-world estimate from the image.
    Auto,
    /// Explicit R, G, B multipliers applied to the camera channels.
    Multipliers([f64; 3]),
}

/// Development settings.
#[derive(Debug, Clone, PartialEq)]
pub struct DevelopOptions {
    pub demosaic: Demosaic,
    pub white_balance: WhiteBalance,
    /// Exposure compensation in EV, added to the file's baseline exposure.
    pub exposure: f64,
    /// Apply the file's orientation (rotate / flip the pixels).
    pub orient: bool,
    /// Camera Raw's Shadows, Highlights, Clarity, Texture and Dehaze, applied in linear light
    /// before the tone curve.
    pub tone: Tone,
    pub limits: Limits,
    /// A colour rendering fitted to the camera's own JPEG ([`crate::fit_look`]). When set it
    /// replaces the file's colour calibration (or the neutral fallback) and adds its tone curve.
    pub camera_look: Option<CameraLook>,
}

impl Default for DevelopOptions {
    fn default() -> Self {
        DevelopOptions {
            demosaic: Demosaic::default(),
            white_balance: WhiteBalance::default(),
            exposure: 0.0,
            orient: true,
            tone: Tone::default(),
            limits: Limits::default(),
            camera_look: None,
        }
    }
}

/// What was developed and how.
#[derive(Debug, Clone, PartialEq)]
pub struct RawInfo {
    pub format: RawFormat,
    pub make: Option<String>,
    pub model: Option<String>,
    /// Sensor data size.
    pub sensor_width: usize,
    pub sensor_height: usize,
    /// CFA pattern at the crop origin: `"RGGB"` for Bayer; other patterns list
    /// their whole repeat row by row, rows separated by `/` (X-Trans:
    /// `"GGRGGB/GGBGGR/…"`); `None` for LinearRaw.
    pub cfa: Option<String>,
    /// White-balance multipliers used (R, G, B, normalized to a minimum of 1).
    pub wb_multipliers: [f64; 3],
    pub orientation: u16,
    pub baseline_exposure: f64,
}

/// A developed image: 16-bit RGB in ProPhoto RGB (ROMM primaries, D50, gamma 1.8).
#[derive(Debug, Clone)]
pub struct Developed {
    pub width: u32,
    pub height: u32,
    /// Interleaved RGB, `width * height * 3` samples.
    pub rgb: Vec<u16>,
    pub info: RawInfo,
    /// Notes about anything approximated or not applied.
    pub warnings: Vec<String>,
}

/// Linearization + black/white scaling for one sample (pre white balance), 0..1.
pub(crate) struct Scale<'a> {
    s: &'a Sensor,
    lin: Option<&'a [u16]>,
}

impl<'a> Scale<'a> {
    /// The scale for `s` (lazily: `lin` is the sensor's linearization table).
    pub(crate) fn new(s: &'a Sensor) -> Self {
        Scale { s, lin: s.linearization.as_deref() }
    }

    #[inline]
    pub(crate) fn value(&self, x: usize, y: usize, sample: usize) -> f32 {
        let s = self.s;
        let raw = s.data[(y * s.width + x) * s.samples + sample];
        let v = match self.lin {
            Some(l) => f32::from(l.get(usize::from(raw)).or_else(|| l.last()).copied().unwrap_or(raw)),
            None => f32::from(raw),
        };
        let black = s.black.at(x.saturating_sub(s.active.x), y.saturating_sub(s.active.y), sample, s.samples);
        let white = s.white[sample.min(2)];
        let range = white - black;
        if range.is_nan() || range < 1.0 {
            return 0.0;
        }
        let n = (v - black) / range;
        if n.is_finite() { n } else { 0.0 }
    }
}

/// Grey-world neutral (camera-space R, G, B) from unclipped mid-range samples.
fn grey_world(s: &Sensor, sc: &Scale) -> [f64; 3] {
    let c = s.crop;
    let step = (c.height / 512).max(1);
    let rows: Vec<usize> = (0..c.height).step_by(step).collect();
    let parts = par::map(rows.len(), |i| {
        let y = c.y + rows[i];
        let mut sum = [0.0f64; 3];
        let mut n = [0u64; 3];
        for x in c.x..c.x + c.width {
            for k in 0..s.samples.min(3) {
                let ch = match &s.cfa {
                    Some(cfa) => usize::from(cfa.color(x, y)),
                    None => k,
                };
                let v = sc.value(x, y, k);
                if v > 0.002 && v < 0.9 {
                    sum[ch] += f64::from(v);
                    n[ch] += 1;
                }
            }
        }
        (sum, n)
    });
    let mut sum = [0.0f64; 3];
    let mut n = [0u64; 3];
    for (s, k) in parts {
        for c in 0..3 {
            sum[c] += s[c];
            n[c] += k[c];
        }
    }
    let mean = [0, 1, 2].map(|c| if n[c] > 0 { sum[c] / n[c] as f64 } else { 0.0 });
    if mean.iter().any(|m| m.is_nan() || *m <= 1e-6) {
        return [1.0; 3];
    }
    let m = mean.iter().cloned().fold(f64::MIN, f64::max);
    mean.map(|v| v / m)
}

fn normalize(n: [f64; 3]) -> Option<[f64; 3]> {
    let m = n.iter().cloned().fold(f64::MIN, f64::max);
    (m > 0.0 && n.iter().all(|v| v.is_finite() && *v > 1e-4)).then(|| n.map(|v| v / m))
}

/// The camera-space neutral (max component 1) for `opts.white_balance`, as
/// [`develop_sensor`] uses it (the same white balance [`crate::fit_look`] balances with).
pub(crate) fn resolve_neutral(s: &Sensor, opts: &DevelopOptions, sc: &Scale, warnings: &mut Vec<String>) -> Result<[f64; 3]> {
    // Monochrome data has no colour to balance.
    if s.is_monochrome() {
        return Ok([1.0; 3]);
    }
    // White balance: camera-space neutral, max component 1.
    let as_shot = s
        .color
        .as_shot_neutral
        .or_else(|| s.color.as_shot_white_xy.and_then(|xy| s.color.xy_to_neutral(xy)))
        .or_else(|| s.camera_wb.map(|m| m.map(|v| 1.0 / v)))
        .and_then(normalize);
    Ok(match opts.white_balance {
        WhiteBalance::AsShot => match as_shot {
            Some(n) => n,
            None => {
                warnings.push("no as-shot white balance in the file; estimated automatically (grey world)".into());
                grey_world(s, sc)
            }
        },
        WhiteBalance::Auto => grey_world(s, sc),
        WhiteBalance::Multipliers(m) => {
            normalize(m.map(|v| 1.0 / v)).ok_or_else(|| RawError::Malformed("white-balance multipliers must be positive".into()))?
        }
    })
}

/// Develops sensor data.
pub fn develop_sensor(s: &Sensor, opts: &DevelopOptions) -> Result<Developed> {
    develop(s, opts, true)
}

/// [`develop_sensor`]; `allow_fused` false keeps the full-frame path (what the tests compare the
/// fused one against).
fn develop(s: &Sensor, opts: &DevelopOptions, allow_fused: bool) -> Result<Developed> {
    let c = s.crop;
    let samples = s.width.checked_mul(s.height).and_then(|n| n.checked_mul(s.samples));
    if c.is_empty()
        || c.x.checked_add(c.width).is_none_or(|x| x > s.width)
        || c.y.checked_add(c.height).is_none_or(|y| y > s.height)
        || samples.is_none_or(|n| s.data.len() < n)
    {
        return Err(RawError::malformed("crop lies outside the sensor data"));
    }
    // LinearRaw can be monochrome (one sample) or RGB (three); CFA data needs one.
    if s.samples != 1 && s.samples != 3 {
        return Err(RawError::unsupported(format!("{} samples per pixel", s.samples)));
    }
    if let Some(cfa) = &s.cfa
        && (s.samples != 1
            || cfa.width == 0
            || cfa.height == 0
            || cfa.width > 16
            || cfa.height > 16
            || cfa.colors.len() != cfa.width * cfa.height
            || cfa.colors.iter().any(|c| *c > 2))
    {
        return Err(RawError::malformed("invalid sensor CFA pattern"));
    }
    let bayer = s.cfa.as_ref().is_none_or(|cfa| cfa.is_bayer());
    // The default rendering (no Dehaze, no local tone, no black point) needs no statistic of
    // the whole frame: demosaic and encode band by band, with no full RGB frame (the same
    // condition under which `Haze::new` and `ToneMap::new` return `None`).
    let fused = allow_fused && opts.tone.is_neutral() && s.black_point <= 0.0;
    let orient_copy = opts.orient && (2..=8).contains(&s.orientation);
    let jobs = working_memory(c.width, c.height, s.cfa.is_some(), bayer, Shape { fused, orient_copy }, opts, par::workers())?;
    let mut warnings = s.warnings.clone();
    let sc = Scale::new(s);

    let neutral = resolve_neutral(s, opts, &sc, &mut warnings)?;
    let mult = neutral.map(|v| (1.0 / v) as f32);

    // Balanced camera → XYZ D50: a look fitted to the camera's JPEG, the file's calibration,
    // or the neutral fallback.
    // Monochrome data stays neutral grey: no camera look and no colour calibration.
    let mono = s.is_monochrome();
    let look = opts.camera_look.as_ref().filter(|l| l.is_usable() && !mono);
    if opts.camera_look.is_some() && look.is_none() && !mono {
        warnings.push("the supplied camera look is not usable (non-finite or singular); ignored".into());
    }
    let to_xyz: Mat3 = match (look, s.color.balanced_to_xyz_d50(neutral)) {
        (Some(l), _) => {
            warnings.push(l.note());
            l.to_xyz
        }
        (None, Some(m)) => m,
        (None, None) => {
            if s.format != RawFormat::Dng && !mono {
                warnings.push("no colour calibration for this camera; camera colours are treated as sRGB primaries".into());
            }
            color::srgb_to_xyz_d50()
        }
    };
    let gain = 2f64.powf(s.baseline_exposure + opts.exposure.clamp(-10.0, 10.0));
    let out_m = color::scale(&color::mul(&color::xyz_d50_to_prophoto(), &to_xyz), gain);
    let out_m: [[f32; 3]; 3] = out_m.map(|r| r.map(|v| v as f32));

    // Neutral grey through `out_m`: the gain alone (the matrix maps camera white to ProPhoto
    // white, up to rounding), so the three output channels stay exactly equal.
    let grey_gain = gain as f32;
    let (w, h) = (c.width, c.height);
    let band = par::band_rows(w);
    // Camera → ProPhoto, exposure, Dehaze, Shadows / Highlights / Clarity / Texture and the black
    // point, the look's (or the profile's) tone curve, gamma 1.8, 16 bits.
    let y_row = color::rgb_to_xyz(color::ROMM, color::D50_XY)[1].map(|v| v as f32);
    let lum = |v: &[f32; 3]| y_row[0] * v[0] + y_row[1] * v[1] + y_row[2] * v[2];
    // Monochrome: the one sample, grey; no colour matrix.
    let pro = |p: &[f32]| if mono { [grey_gain * p[0]; 3] } else { [0, 1, 2].map(|k| out_m[k][0] * p[0] + out_m[k][1] * p[1] + out_m[k][2] * p[2]) };
    let lut = match look {
        Some(l) => GammaLut::with_tone(l.tone.as_ref()),
        None => GammaLut::with_curve(&s.tone_curve),
    };
    // The default rendering: matrix then encode, in blocks so the encode's clamp and square root
    // run over whole slices.
    let encode_plain = |src: &[f32], dst: &mut [u16]| {
        let mut buf = [0.0f32; ENCODE_BLOCK];
        for (oc, sc) in dst.chunks_mut(ENCODE_BLOCK).zip(src.chunks(ENCODE_BLOCK)) {
            let n = sc.len();
            for (t, p) in buf[..n].as_chunks_mut::<3>().0.iter_mut().zip(sc.as_chunks::<3>().0) {
                *t = pro(p);
            }
            lut.encode_into(&buf[..n], oc);
        }
    };
    // Rows `y0..` of a LinearRaw (3-sample or monochrome) frame, balanced and clipped.
    let linear_rows = |y0: usize, chunk: &mut [f32]| {
        for (r, row) in chunk.chunks_exact_mut(w * 3).enumerate() {
            let y = c.y + y0 + r;
            for i in 0..w {
                for k in 0..3 {
                    // Monochrome: the one sample in all three channels.
                    let sample = k.min(s.samples.saturating_sub(1));
                    row[i * 3 + k] = (sc.value(c.x + i, y, sample) * mult[k]).clamp(0.0, 1.0);
                }
            }
            for k in 0..s.samples.min(3) {
                if let Some(g) = gain_row(s, y, c.x, w, k) {
                    for (i, g) in g.into_iter().enumerate() {
                        row[i * 3 + k] = (row[i * 3 + k] * g).clamp(0.0, 1.0);
                    }
                }
            }
        }
    };
    let mut rgb: Vec<f32> = Vec::new();
    // The encoded frame, when the fused path made it.
    let mut fused_out: Option<Vec<u16>> = None;
    let pattern: Option<String>;
    match &s.cfa {
        Some(cfa) => {
            // One normalized, balanced, clipped CFA row (image row `iy` of the crop).
            // Fast path: black varies at most per 2×2 position, so black and gain
            // repeat every `period` columns (the CFA width, made even).
            let bl = &s.black;
            let periodic = bl.delta_h.is_empty() && bl.delta_v.is_empty() && matches!(bl.rows, 1 | 2) && matches!(bl.cols, 1 | 2) && cfa.width <= 16;
            let period = if cfa.width % 2 == 0 { cfa.width.max(2) } else { cfa.width * 2 };
            let fill_row = |iy: usize, row: &mut [f32]| {
                let y = c.y + iy;
                if periodic {
                    // Per column of the period: black and the combined scale × white-balance gain.
                    let k: Vec<(f32, f32)> = (0..period)
                        .map(|p| {
                            let x = c.x + p;
                            let black = bl.at(x.saturating_sub(s.active.x), y.saturating_sub(s.active.y), 0, 1);
                            let range = s.white[0] - black;
                            // A non-finite black (a BlackLevel DOUBLE beyond f32) gives 0, as
                            // `Scale::value` does, instead of (raw - inf) * 0 = NaN.
                            if !(range >= 1.0 && black.is_finite()) {
                                return (0.0, 0.0);
                            }
                            (black, mult[usize::from(cfa.color(x, y)).min(2)] / range)
                        })
                        .collect();
                    let src = &s.data[y * s.width + c.x..y * s.width + c.x + w];
                    let ks = k.iter().cycle();
                    match &sc.lin {
                        None => {
                            for ((v, &raw), &(black, gain)) in row.iter_mut().zip(src).zip(ks) {
                                *v = ((f32::from(raw) - black) * gain).clamp(0.0, 1.0);
                            }
                        }
                        Some(l) => {
                            for ((v, &raw), &(black, gain)) in row.iter_mut().zip(src).zip(ks) {
                                let lin = f32::from(l.get(usize::from(raw)).or_else(|| l.last()).copied().unwrap_or(raw));
                                *v = ((lin - black) * gain).clamp(0.0, 1.0);
                            }
                        }
                    }
                } else {
                    for (i, v) in row.iter_mut().enumerate() {
                        let x = c.x + i;
                        let ch = usize::from(cfa.color(x, y)).min(2);
                        *v = (sc.value(x, y, 0) * mult[ch]).clamp(0.0, 1.0);
                    }
                }
                // Lens-shading gain maps, after the white-balance clip so clipped
                // highlights stay neutral.
                if let Some(g) = gain_row(s, y, c.x, w, 0) {
                    for (v, g) in row.iter_mut().zip(g) {
                        *v = (*v * g).clamp(0.0, 1.0);
                    }
                }
            };
            let names = |p: &[u8]| p.iter().map(|c| ['R', 'G', 'B'][usize::from(*c).min(2)]).collect::<String>();
            if bayer && fused {
                // No padded plane either: every band normalizes its own rows.
                let p = cfa.phase(c.x, c.y);
                pattern = Some(names(&p));
                let mut o = vec![0u16; w * h * 3];
                demosaic_fused(w, h, p, opts.demosaic, &to_xyz, jobs, &fill_row, &mut o, &encode_plain);
                fused_out = Some(o);
            } else {
                let mut plane = Padded::new(w, h);
                let stride = plane.stride;
                plane.fill_rows(band, |y0, chunk| {
                    for (r, prow) in chunk.chunks_exact_mut(stride).enumerate() {
                        fill_row(y0 + r, &mut prow[PAD..PAD + w]);
                    }
                });
                plane.fill_borders();
                if bayer {
                    let p = cfa.phase(c.x, c.y);
                    pattern = Some(names(&p));
                    rgb = demosaic(&plane, p, opts.demosaic, &to_xyz, jobs);
                } else {
                    let pat = Pattern::of(cfa, c.x, c.y);
                    pattern = Some(pat.colors.chunks(pat.w.max(1)).map(names).collect::<Vec<_>>().join("/"));
                    if fused {
                        let mut o = vec![0u16; w * h * 3];
                        demosaic_cfa_fused(&plane, &pat, jobs, &mut o, &encode_plain);
                        fused_out = Some(o);
                    } else {
                        rgb = demosaic_cfa(&plane, &pat);
                    }
                }
            }
        }
        None => {
            pattern = None;
            if fused {
                let mut o = vec![0u16; w * h * 3];
                for_bands(
                    &mut o,
                    band * w * 3,
                    h.div_ceil(band),
                    jobs,
                    || vec![0.0f32; band.min(h) * w * 3],
                    |b, chunk, buf| {
                        let buf = &mut buf[..chunk.len()];
                        linear_rows(b * band, buf);
                        encode_plain(buf, chunk);
                    },
                );
                fused_out = Some(o);
            } else {
                rgb = vec![0.0f32; w * h * 3];
                par::chunks_mut(&mut rgb, band * w * 3, |b, chunk| linear_rows(b * band, chunk));
            }
        }
    }

    let out = match fused_out {
        Some(out) => out,
        None => {
            let haze = Haze::new(w, h, |i| pro(&rgb[i * 3..i * 3 + 3]), opts.tone.dehaze);
            // Linear ProPhoto of pixel `i` after Dehaze.
            let linear = |i: usize, p: &[f32]| {
                let v = pro(p);
                haze.as_ref().map_or(v, |z| z.apply(i % w, i / w, v))
            };
            let mut map = ToneMap::new(w, h, |i| lum(&linear(i, &rgb[i * 3..i * 3 + 3])), opts.tone, s.black_point);
            if let (Some(m), Some(_)) = (&mut map, &haze) {
                // The black point belongs to the default rendering: Dehaze does not move it (taken after
                // Dehaze it fell to almost 0, and Dehaze darkened less than Photoshop's).
                m.black = black_level(w, h, |i| lum(&pro(&rgb[i * 3..i * 3 + 3])), s.black_point);
            }
            let mut out = vec![0u16; w * h * 3];
            let plain = map.is_none() && haze.is_none();
            par::chunks_mut(&mut out, band * w * 3, |b, chunk| {
                let start = b * band * w * 3;
                let src = &rgb[start..start + chunk.len()];
                if plain {
                    encode_plain(src, chunk);
                    return;
                }
                for (i, (o, p)) in chunk.as_chunks_mut::<3>().0.iter_mut().zip(src.as_chunks::<3>().0).enumerate() {
                    let mut v = linear(b * band * w + i, p);
                    if let Some(m) = &map {
                        let g = m.gain(i % w, b * band + i / w, lum(&v));
                        // The black point first, as Camera Raw: lifting before it would flatten the
                        // deepest shadows' contrast.
                        v = v.map(|c| m.unblack(c) * g);
                    }
                    for k in 0..3 {
                        o[k] = lut.encode(v[k]);
                    }
                }
            });
            rgb.clear();
            rgb.shrink_to_fit();
            out
        }
    };

    let (out, ow, oh) = if opts.orient { orient(out, w, h, s.orientation) } else { (out, w, h) };
    let nmax = mult.iter().cloned().fold(f32::MAX, f32::min).max(1e-6);
    Ok(Developed {
        width: ow as u32,
        height: oh as u32,
        rgb: out,
        info: RawInfo {
            format: s.format,
            make: s.make.clone(),
            model: s.model.clone(),
            sensor_width: s.width,
            sensor_height: s.height,
            cfa: pattern,
            wb_multipliers: mult.map(|m| f64::from(m / nmax)),
            orientation: s.orientation,
            baseline_exposure: s.baseline_exposure,
        },
        warnings,
    })
}

/// How the frame is developed, for [`working_memory`].
#[derive(Clone, Copy)]
struct Shape {
    /// Demosaic and encode band by band (no full RGB frame).
    fused: bool,
    /// The orientation copies the encoded frame.
    orient_copy: bool,
}

/// Bytes one worker of the fused path holds for a band of `band_rows` rows: the f32 RGB band
/// and, for a Bayer sensor, the padded CFA window it reads.
fn fused_band_bytes(w: usize, h: usize, band_rows: usize, window: bool) -> u64 {
    let rows = band_rows.min(h) as u64;
    let rgb = rows.saturating_mul(w as u64).saturating_mul(12);
    let window = if window { (rows + 2 * PAD as u64).saturating_mul((w + 2 * PAD) as u64).saturating_mul(4) } else { 0 };
    rgb.saturating_add(window)
}

/// Checks the peak of the two development phases. The CFA and non-Bayer green
/// planes are freed before the u16 output is allocated; the AHD scratch and the fused band
/// buffers are bounded by reducing simultaneous bands when needed. The fused path never holds
/// the f32 RGB frame: it has the u16 output (6 bytes a pixel) next to the CFA plane, plus a band
/// buffer and the scratch per simultaneous band, and an orientation copy of the output afterwards. Sensor storage belongs
/// to the caller and is checked separately by the decoder.
fn working_memory(w: usize, h: usize, cfa: bool, bayer: bool, shape: Shape, opts: &DevelopOptions, workers: usize) -> Result<usize> {
    let limits = &opts.limits;
    let Shape { fused, orient_copy } = shape;
    limits.check(w as u64, h as u64, if fused { 6 } else { 18 })?;
    let pixels = (w as u64).saturating_mul(h as u64);
    let table = (GAMMA_STEPS as u64 + 2) * 4;
    // Encode phase: the f32 RGB frame and the output, or (fused) the output and its oriented copy.
    let output = pixels.saturating_mul(match (fused, orient_copy) {
        (false, _) => 18,
        (true, false) => 6,
        (true, true) => 12,
    });
    let mut peak = output.saturating_add(table);
    let mut jobs = workers.max(1);
    // Fused: the u16 output and, per worker running a band, one band of f32 RGB.
    let band_of = |rows: usize, window: bool| if fused { fused_band_bytes(w, h, rows, window) } else { 0 };
    // Fewer bands at once when the bands of every worker don't fit next to `base` (at least one).
    let with_bands = |jobs: &mut usize, base: u64, rows: usize, band: u64| {
        let available = limits.max_alloc.saturating_sub(base) / band.max(1);
        *jobs = (*jobs).min(h.div_ceil(rows.max(1))).min(usize::try_from(available).unwrap_or(usize::MAX)).max(1);
        base.saturating_add(band.saturating_mul(*jobs as u64))
    };
    if cfa {
        let plane = (w as u64).saturating_add(2 * PAD as u64).saturating_mul((h as u64).saturating_add(2 * PAD as u64)).saturating_mul(4);
        // Bytes a pixel next to the plane: the f32 RGB frame (and the non-Bayer green plane), or
        // the u16 output (and the green plane, which the fused path keeps).
        let frame = match (fused, bayer) {
            (false, true) => 12,
            (false, false) => 16,
            (true, true) => 6,
            (true, false) => 10,
        };
        // The fused Bayer path has no whole plane: each worker's window is counted with its band.
        let plane = if fused && bayer { 0 } else { plane };
        let mut mosaic = pixels.saturating_mul(frame).saturating_add(plane).saturating_add(table);
        if bayer && opts.demosaic == Demosaic::Ahd {
            mosaic = mosaic.saturating_add(crate::demosaic::AHD_TABLE_BYTES);
            let scratch = crate::demosaic::ahd_scratch(w as u64, h as u64).saturating_add(band_of(crate::demosaic::AHD_BAND, true));
            // At least one band must fit, even when no parallelism is available.
            mosaic = with_bands(&mut jobs, mosaic, crate::demosaic::AHD_BAND, scratch);
        } else {
            let rows = if bayer { crate::demosaic::FUSED_ROWS } else { par::band_rows(w) };
            mosaic = with_bands(&mut jobs, mosaic, rows, band_of(rows, bayer));
        }
        peak = peak.max(mosaic);
    } else if fused {
        // LinearRaw: the output and the bands.
        let rows = par::band_rows(w);
        peak = peak.max(with_bands(&mut jobs, pixels.saturating_mul(6), rows, band_of(rows, false)));
    }
    if peak > limits.max_alloc || usize::try_from(peak).is_err() {
        return Err(RawError::LimitExceeded(format!("{peak} bytes of RAW working memory exceed {}", limits.max_alloc)));
    }
    Ok(jobs.max(1))
}

/// Gain-map factors (DNG OpcodeList2 GainMap) for data row `y`, columns
/// `x0..x0 + w`, sample `k`; `None` when no map touches the row.
fn gain_row(s: &Sensor, y: usize, x0: usize, w: usize, k: usize) -> Option<Vec<f32>> {
    let a = s.active;
    if s.gain_maps.is_empty() || y < a.y || a.width == 0 || a.height == 0 {
        return None;
    }
    let ay = y - a.y;
    // Map points are placed in coordinates relative to the (active) image.
    let rv = (ay as f64 + 0.5) / a.height as f64;
    let mut out: Option<Vec<f32>> = None;
    for m in &s.gain_maps {
        if k < m.plane || k >= m.plane.saturating_add(m.planes) || ay < m.top || ay >= m.bottom || !(ay - m.top).is_multiple_of(m.row_pitch) {
            continue;
        }
        let o = out.get_or_insert_with(|| vec![1.0; w]);
        for (i, f) in o.iter_mut().enumerate() {
            let x = x0 + i;
            if x >= a.x && m.covers(x - a.x, ay) {
                *f *= m.at_rv(rv, k - m.plane, ((x - a.x) as f64 + 0.5) / a.width as f64);
            }
        }
    }
    out
}

/// Linear 0..1 → gamma-1.8 16-bit code. Tabulated over `s = sqrt(v)`, where
/// the curve `s^(2/1.8)` is smooth down to 0, with linear interpolation
/// (error below 0.2 code values).
struct GammaLut {
    table: Vec<f32>,
    /// `[table[i], table[i + 1] - table[i]]`: the same interpolation from one load.
    pairs: Vec<[f32; 2]>,
}

const GAMMA_STEPS: usize = 4096;
/// Samples per block of [`GammaLut::encode_into`] (whole pixels).
const ENCODE_BLOCK: usize = 3 * 256;

impl GammaLut {
    #[cfg(test)]
    fn new() -> Self {
        Self::with_curve(&[])
    }

    /// The gamma-1.8 encoding after a tone curve (see [`Sensor::tone_curve`]; empty: none).
    fn with_curve(curve: &[[f32; 2]]) -> Self {
        let table = (0..=GAMMA_STEPS + 1)
            .map(|i| {
                let s = (i as f32 / GAMMA_STEPS as f32).min(1.0);
                tone(curve, s * s).clamp(0.0, 1.0).powf(1.0 / 1.8) * 65535.0
            })
            .collect();
        Self::from_table(table)
    }

    /// The gamma-1.8 encoding after a fitted [`ToneCurve`] (`None`: none, plain gamma 1.8).
    fn with_tone(tone: Option<&ToneCurve>) -> Self {
        let table = (0..=GAMMA_STEPS + 1)
            .map(|i| {
                let s = (i as f64 / GAMMA_STEPS as f64).min(1.0);
                match tone {
                    Some(t) => (t.eval(s * s).powf(1.0 / 1.8) * 65535.0) as f32,
                    None => (s as f32).powf(2.0 / 1.8) * 65535.0,
                }
            })
            .collect();
        Self::from_table(table)
    }

    fn from_table(table: Vec<f32>) -> Self {
        let pairs = table.windows(2).map(|p| [p[0], p[1] - p[0]]).collect();
        GammaLut { table, pairs }
    }

    /// [`Self::encode`] of every sample of `v` into `out` (at most [`ENCODE_BLOCK`] samples):
    /// the clamp and square root first over the whole block, then the table.
    #[inline]
    fn encode_into(&self, v: &[f32], out: &mut [u16]) {
        let mut xs = [0.0f32; ENCODE_BLOCK];
        let n = v.len().min(ENCODE_BLOCK).min(out.len());
        for (x, &v) in xs[..n].iter_mut().zip(&v[..n]) {
            let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
            *x = v.sqrt() * GAMMA_STEPS as f32;
        }
        for (o, &x) in out[..n].iter_mut().zip(&xs[..n]) {
            let i = (x as usize).min(GAMMA_STEPS);
            let f = x - i as f32;
            let [a, step] = self.pairs[i];
            *o = (a + step * f + 0.5) as u16;
        }
    }

    #[inline]
    fn encode(&self, v: f32) -> u16 {
        let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
        let x = v.sqrt() * GAMMA_STEPS as f32;
        let i = (x as usize).min(GAMMA_STEPS);
        let f = x - i as f32;
        let (a, b) = (self.table[i], self.table[i + 1]);
        (a + (b - a) * f + 0.5) as u16
    }
}

/// `curve` at linear `v`: linear interpolation between the points in log₂ of the input, a line
/// through 0 below the first point, the last output beyond the last.
pub(crate) fn tone(curve: &[[f32; 2]], v: f32) -> f32 {
    let (Some(first), Some(last)) = (curve.first(), curve.last()) else { return v };
    if v <= first[0] {
        return if first[0] > 0.0 { v * first[1] / first[0] } else { first[1] };
    }
    if v >= last[0] {
        return last[1];
    }
    let i = curve.partition_point(|p| p[0] <= v).clamp(1, curve.len() - 1);
    let ([x0, y0], [x1, y1]) = (curve[i - 1], curve[i]);
    // Logarithmic between knots; linear from a knot at black, where log₂ has no value.
    let f = if x0 > 0.0 { (v.log2() - x0.log2()) / (x1.log2() - x0.log2()) } else { (v - x0) / (x1 - x0) };
    y0 + (y1 - y0) * f
}

/// The source position in a `w × h` image of output pixel (`x2`, `y2`) after
/// applying TIFF orientation `o` (output in range when the input is).
pub(crate) fn source_xy(o: u16, w: usize, h: usize, x2: usize, y2: usize) -> (usize, usize) {
    let (wl, hl) = (w.saturating_sub(1), h.saturating_sub(1));
    match o {
        2 => (wl.saturating_sub(x2), y2),
        3 => (wl.saturating_sub(x2), hl.saturating_sub(y2)),
        4 => (x2, hl.saturating_sub(y2)),
        5 => (y2, x2),
        6 => (y2, hl.saturating_sub(x2)),
        7 => (wl.saturating_sub(y2), hl.saturating_sub(x2)),
        8 => (wl.saturating_sub(y2), x2),
        _ => (x2, y2),
    }
}

/// Applies a TIFF orientation (1–8) to interleaved RGB.
pub(crate) fn orient(src: Vec<u16>, w: usize, h: usize, o: u16) -> (Vec<u16>, usize, usize) {
    if !(2..=8).contains(&o) {
        return (src, w, h);
    }
    let swap = o >= 5;
    let (ow, oh) = if swap { (h, w) } else { (w, h) };
    let mut out = vec![0u16; src.len()];
    let band = par::band_rows(ow);
    par::chunks_mut(&mut out, band * ow * 3, |b, chunk| {
        for (r, row) in chunk.chunks_exact_mut(ow * 3).enumerate() {
            let y2 = b * band + r;
            for x2 in 0..ow {
                let (x, y) = match o {
                    2 => (w - 1 - x2, y2),
                    3 => (w - 1 - x2, h - 1 - y2),
                    4 => (x2, h - 1 - y2),
                    5 => (y2, x2),
                    6 => (y2, h - 1 - x2),
                    7 => (w - 1 - y2, h - 1 - x2),
                    _ => (w - 1 - y2, x2),
                };
                let i = (y * w + x) * 3;
                row[x2 * 3..x2 * 3 + 3].copy_from_slice(&src[i..i + 3]);
            }
        }
    });
    (out, ow, oh)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: Shape = Shape { fused: false, orient_copy: false };
    const FUSED: Shape = Shape { fused: true, orient_copy: false };

    /// A `w × h` sensor with a pseudo-random gradient, black 64, white 4000.
    fn test_sensor(w: usize, h: usize, cfa: Option<crate::Cfa>, samples: usize) -> Sensor {
        use crate::*;
        let data = (0..w * h * samples).map(|i| 64 + ((i * 7919 + i / w * 104_729 + i / 5 * 31) % 4100) as u16).collect();
        Sensor {
            format: RawFormat::Dng,
            make: None,
            model: None,
            width: w,
            height: h,
            samples,
            data,
            cfa,
            linearization: None,
            black: BlackLevels::uniform(64.0),
            white: [4000.0; 3],
            active: Rect::new(0, 0, w, h),
            crop: Rect::new(0, 0, w, h),
            color: ColorInfo::default(),
            camera_wb: None,
            orientation: 1,
            baseline_exposure: 0.0,
            gain_maps: Vec::new(),
            tone_curve: vec![[0.18, 0.3], [1.0, 1.0]],
            black_point: 0.0,
            warnings: Vec::new(),
        }
    }

    /// `f` on a pool of `n` threads (the browser builds have none).
    fn with_threads<R: Send>(n: usize, f: impl FnOnce() -> R + Send) -> R {
        #[cfg(not(target_arch = "wasm32"))]
        return rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap().install(f);
        #[cfg(target_arch = "wasm32")]
        {
            let _ = n;
            f()
        }
    }

    fn assert_fused_matches_full(s: &Sensor, opts: &DevelopOptions, what: &str) {
        let full = develop(s, opts, false).unwrap();
        for threads in [1, 3, 16] {
            let fused = with_threads(threads, || develop(s, opts, true).unwrap());
            assert_eq!((full.width, full.height), (fused.width, fused.height), "{what} {threads}");
            assert!(full.rgb == fused.rgb, "{what} {threads} threads: fused output differs");
        }
    }

    #[test]
    fn fused_development_matches_the_full_frame_path() {
        use crate::Cfa;
        // Odd sizes, tiny images, band borders (AHD bands are 32 rows) and several bands.
        let sizes = [(1, 1), (2, 3), (5, 3), (33, 31), (64, 64), (65, 97), (70, 131), (3, 300)];
        for (w, h) in sizes {
            for method in [Demosaic::Ahd, Demosaic::Bilinear, Demosaic::Mhc] {
                for orientation in [1, 6] {
                    for pattern in ["RGGB", "BGGR", "GRBG"] {
                        let mut s = test_sensor(w, h, Cfa::bayer(pattern), 1);
                        s.orientation = orientation;
                        let opts = DevelopOptions { demosaic: method, ..Default::default() };
                        assert_fused_matches_full(&s, &opts, &format!("{w}x{h} {method:?} o{orientation} {pattern}"));
                    }
                }
            }
        }
    }

    #[test]
    fn fused_development_matches_for_every_orientation_and_look() {
        use crate::Cfa;
        for orientation in 1..=8 {
            let mut s = test_sensor(67, 45, Cfa::bayer("RGGB"), 1);
            s.orientation = orientation;
            s.baseline_exposure = 0.5;
            s.linearization = Some((0..4100u32).map(|v| (v * 15 / 16) as u16).collect());
            assert_fused_matches_full(&s, &DevelopOptions::default(), &format!("orientation {orientation}"));
        }
    }

    #[test]
    fn fused_development_matches_for_xtrans_and_linear_raw() {
        use crate::Cfa;
        // 6 × 6 X-Trans repeat.
        let rows = ["GGRGGB", "GGBGGR", "BRGRBG", "GGBGGR", "GGRGGB", "RBGBRG"];
        let colors: Vec<u8> = rows
            .iter()
            .flat_map(|r| {
                r.bytes().map(|c| match c {
                    b'R' => 0,
                    b'G' => 1,
                    _ => 2,
                })
            })
            .collect();
        // 600 samples wide: bands of 109 rows, so 350 rows are four (the last short) and 50 rows
        // are less than one.
        for (w, h) in [(6, 6), (37, 41), (80, 150), (600, 350), (600, 50)] {
            let mut s = test_sensor(w, h, Some(Cfa { width: 6, height: 6, colors: colors.clone(), origin_x: 0, origin_y: 0 }), 1);
            s.orientation = 8;
            assert_fused_matches_full(&s, &DevelopOptions::default(), &format!("xtrans {w}x{h}"));
        }
        for samples in [1, 3] {
            for (w, h) in [(1, 1), (7, 5), (90, 111), (600, 350), (600, 50)] {
                let mut s = test_sensor(w, h, None, samples);
                s.orientation = 3;
                assert_fused_matches_full(&s, &DevelopOptions::default(), &format!("linear {samples} {w}x{h}"));
            }
        }
    }

    #[test]
    fn fused_only_where_no_whole_frame_statistic_is_needed() {
        use crate::Cfa;
        // The condition for the fused path is exactly "Haze and ToneMap are None".
        let px = |_: usize| [0.5f32; 3];
        for tone in [Tone::NEUTRAL, Tone { shadows: 10.0, ..Tone::NEUTRAL }, Tone { dehaze: 5.0, ..Tone::NEUTRAL }, Tone { texture: 1.0, ..Tone::NEUTRAL }] {
            for black_point in [0.0, 0.5] {
                let none = Haze::new(8, 8, px, tone.dehaze).is_none() && ToneMap::new(8, 8, |_| 0.5, tone, black_point).is_none();
                assert_eq!(none, tone.is_neutral() && black_point <= 0.0, "{tone:?} {black_point}");
            }
        }
        // A tone slider or a black point is still developed (differently from the plain rendering).
        let s = test_sensor(40, 40, Cfa::bayer("RGGB"), 1);
        let plain = develop_sensor(&s, &DevelopOptions::default()).unwrap();
        let toned = develop_sensor(&s, &DevelopOptions { tone: Tone { shadows: 50.0, ..Tone::NEUTRAL }, ..Default::default() }).unwrap();
        assert!(plain.rgb != toned.rgb);
    }

    #[test]
    fn block_encode_matches_per_sample_encode() {
        let lut = GammaLut::with_curve(&[[0.25, 0.5], [1.0, 1.0]]);
        let v: Vec<f32> = (0..2000).map(|i| (i as f32 - 300.0) / 1500.0).chain([f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, 1.0]).collect();
        for n in [0, 1, 3, 255, ENCODE_BLOCK, v.len().min(ENCODE_BLOCK)] {
            let mut out = vec![0u16; n];
            lut.encode_into(&v[..n], &mut out);
            assert_eq!(out, v[..n].iter().map(|&x| lut.encode(x)).collect::<Vec<_>>(), "{n}");
        }
    }

    #[test]
    fn hundred_megapixel_bayer_fits_without_raising_default_limits() {
        let mut opts = DevelopOptions::default();
        for method in [Demosaic::Bilinear, Demosaic::Mhc, Demosaic::Ahd] {
            opts.demosaic = method;
            for (fused, orient_copy) in [(false, false), (true, false), (true, true)] {
                let shape = Shape { fused, orient_copy };
                for workers in [1, 10, 32, 128] {
                    let jobs = working_memory(11648, 8736, true, true, shape, &opts, workers).unwrap();
                    assert!(jobs <= workers);
                    if method == Demosaic::Ahd && workers >= 32 && !fused {
                        assert!(jobs < workers);
                    }
                }
            }
        }
        opts.limits.max_alloc = 1 << 30;
        assert!(working_memory(11648, 8736, true, true, FULL, &opts, 1).is_err());
        // Fused: the u16 output and the CFA plane (+ one band) is about 10 bytes a pixel.
        opts.demosaic = Demosaic::Bilinear;
        assert!(working_memory(11648, 8736, true, true, FUSED, &opts, 1).is_ok());
    }

    /// The fused bilinear / MHC, X-Trans and LinearRaw paths run fewer bands at once when the bands
    /// of every worker don't fit next to the output, as AHD does. A very wide, short Bayer frame
    /// (262144 x 455: 15 bands of 32 rows, about 147 MB a band with its window) needs 2.92 GB
    /// with all 15 at once against the default 2 GiB, and develops with 9.
    #[test]
    fn fused_bands_at_once_are_capped_by_the_memory_limit() {
        let opts = DevelopOptions { demosaic: Demosaic::Bilinear, ..Default::default() };
        let table = (GAMMA_STEPS as u64 + 2) * 4;
        let (w, h) = (262144usize, 455usize);
        let band = 32 * 262144 * 12 + (32 + 2 * PAD as u64) * (262144 + 2 * PAD as u64) * 4;
        for (workers, want) in [(1, 1), (8, 8), (9, 9), (10, 9), (16, 9), (32, 9)] {
            assert_eq!(working_memory(w, h, true, true, FUSED, &opts, workers).unwrap(), want, "{workers} workers");
        }
        // Never more jobs than bands, and one when only one band fits.
        assert_eq!(working_memory(1000, 40, true, true, FUSED, &opts, 32).unwrap(), 2);
        let mut tight = opts.clone();
        tight.limits.max_alloc = 262144 * 455 * 6 + table + band;
        assert_eq!(working_memory(w, h, true, true, FUSED, &tight, 32).unwrap(), 1);
        tight.limits.max_alloc -= 1;
        assert!(working_memory(w, h, true, true, FUSED, &tight, 32).is_err());
        // X-Trans (bands of `band_rows(1000)` = 65 rows next to its planes) and LinearRaw likewise.
        let plane = (1000 + 2 * PAD as u64) * (2000 + 2 * PAD as u64) * 4;
        let mut x = opts.clone();
        x.limits.max_alloc = 1000 * 2000 * 10 + plane + table + 3 * 65 * 1000 * 12;
        assert_eq!(working_memory(1000, 2000, true, false, FUSED, &x, 32).unwrap(), 3);
        x.limits.max_alloc = 1000 * 2000 * 6 + 5 * 65 * 1000 * 12;
        assert_eq!(working_memory(1000, 2000, false, true, FUSED, &x, 32).unwrap(), 5);
    }

    #[test]
    fn working_memory_accounts_for_padding_and_at_least_one_ahd_band() {
        let mut opts = DevelopOptions { demosaic: Demosaic::Ahd, ..Default::default() };
        opts.limits.max_alloc = 100 * 100 * 22;
        assert!(working_memory(100, 100, true, true, FULL, &opts, 1).is_err());
        opts.demosaic = Demosaic::Mhc;
        assert!(working_memory(100, 100, true, true, FULL, &opts, 1).is_ok());
        opts.limits.max_alloc = 100 * 100 * 18;
        assert!(working_memory(100, 100, false, true, FULL, &opts, 1).is_err());
    }

    #[test]
    fn fused_working_memory_charges_the_output_the_window_and_the_bands() {
        let mut opts = DevelopOptions { demosaic: Demosaic::Mhc, ..Default::default() };
        let (w, h) = (1000u64, 1000u64);
        let table = (GAMMA_STEPS as u64 + 2) * 4;
        let plane = (w + 2 * PAD as u64) * (h + 2 * PAD as u64) * 4;
        let pad = PAD as u64;
        let exact = |opts: &mut DevelopOptions, cfa: bool, bayer: bool, shape: Shape, need: u64| {
            opts.limits.max_alloc = need - 1;
            assert!(working_memory(1000, 1000, cfa, bayer, shape, opts, 1).is_err(), "{need} - 1");
            opts.limits.max_alloc = need;
            assert!(working_memory(1000, 1000, cfa, bayer, shape, opts, 1).is_ok(), "{need}");
        };
        // Bayer: the u16 output and one band with its padded window, no whole plane.
        let band = 32 * w * 12 + (32 + 2 * pad) * (w + 2 * pad) * 4;
        exact(&mut opts, true, true, FUSED, w * h * 6 + table + band);
        // An oriented frame needs the output twice; a monochrome sensor has no plane.
        let mono_band = 65 * w * 12;
        exact(&mut opts, false, true, Shape { fused: true, orient_copy: true }, w * h * 12 + table);
        exact(&mut opts, false, true, FUSED, w * h * 6 + mono_band);
        // X-Trans keeps its plane and its green plane.
        exact(&mut opts, true, false, FUSED, w * h * 10 + plane + table + mono_band);
    }

    #[test]
    fn gamma_table_matches_powf() {
        let lut = GammaLut::new();
        for i in 0..=100_000 {
            let v = i as f32 / 100_000.0;
            let exact = v.powf(1.0 / 1.8) * 65535.0;
            assert!((f32::from(lut.encode(v)) - exact).abs() <= 1.0, "{v}: {} vs {exact}", lut.encode(v));
        }
        assert_eq!(lut.encode(-1.0), 0);
        assert_eq!(lut.encode(2.0), 65535);
        assert_eq!(lut.encode(f32::NAN), 0);
    }

    #[test]
    fn tone_curve_is_folded_into_the_gamma_table() {
        let code = |v: f32| v.powf(1.0 / 1.8) * 65535.0;
        let lut = GammaLut::with_curve(&[[0.25, 0.5], [1.0, 1.0]]);
        let near = |got: u16, want: f32| (f32::from(got) - want).abs() <= 2.0;
        // On a point, between points (linear in log2 of the input), on the line through 0 below the first.
        assert!(near(lut.encode(0.25), code(0.5)));
        assert!(near(lut.encode(0.5), code(0.75)), "{} vs {}", lut.encode(0.5), code(0.75));
        assert!(near(lut.encode(0.125), code(0.25)));
        assert_eq!(lut.encode(0.0), 0);
        assert_eq!(lut.encode(1.0), 65535);
        assert!((0..1000).map(|i| lut.encode(i as f32 / 999.0)).collect::<Vec<_>>().windows(2).all(|w| w[1] >= w[0]));
        // No curve: the plain gamma 1.8.
        assert!(near(GammaLut::with_curve(&[]).encode(0.25), code(0.25)));
    }

    #[test]
    fn orientations() {
        // 3×2 image, pixel value = index.
        let src: Vec<u16> = (0..6).flat_map(|i| [i, i, i]).collect();
        let px = |v: &[u16]| v.as_chunks::<3>().0.iter().map(|p| p[0]).collect::<Vec<_>>();
        assert_eq!(px(&orient(src.clone(), 3, 2, 1).0), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(px(&orient(src.clone(), 3, 2, 2).0), vec![2, 1, 0, 5, 4, 3]);
        assert_eq!(px(&orient(src.clone(), 3, 2, 3).0), vec![5, 4, 3, 2, 1, 0]);
        assert_eq!(px(&orient(src.clone(), 3, 2, 4).0), vec![3, 4, 5, 0, 1, 2]);
        let (v, w, h) = orient(src.clone(), 3, 2, 6);
        assert_eq!((w, h), (2, 3));
        // Rotated 90° clockwise: the left column (0, 3) becomes the top row (3, 0).
        assert_eq!(px(&v), vec![3, 0, 4, 1, 5, 2]);
        assert_eq!(px(&orient(src.clone(), 3, 2, 8).0), vec![2, 5, 1, 4, 0, 3]);
        assert_eq!(px(&orient(src.clone(), 3, 2, 5).0), vec![0, 3, 1, 4, 2, 5]);
        assert_eq!(px(&orient(src, 3, 2, 7).0), vec![5, 2, 4, 1, 3, 0]);
    }
}
